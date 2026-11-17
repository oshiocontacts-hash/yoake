use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use super::mod_gpu::{Error, Format, ModId, Stage};
use super::mod_gpu_resources::{EngineFrame, EngineSources, FrameTarget, Gpu};

mod recovery;

pub const FRAME_VERSION: u32 = 1;
static ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Default)]
pub struct Frame {
    pub index: u64,
    pub width: u32,
    pub height: u32,
    pub view_projection: [f32; 16],
    pub camera: [f64; 3],
    pub delta_seconds: f32,
    pub world_format: Option<Format>,
    pub hud_format: Option<Format>,
}

type Callback = Arc<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub(super) struct SceneReads {
    remaining: u32,
}

impl SceneReads {
    pub fn required(&self) -> bool {
        self.remaining > 0
    }

    pub fn observe(&mut self, reads_scene: bool) {
        self.remaining = if reads_scene {
            120
        } else {
            self.remaining.saturating_sub(1)
        };
    }
}

struct Runtime {
    device: wgpu::Device,
    queue: wgpu::Queue,
    lost: Arc<AtomicBool>,
    gpu: Gpu,
    frame: Frame,
    engine_frame: EngineFrame,
    last_frame: Option<Instant>,
    started: Instant,
    scene_reads: SceneReads,
}

fn runtime() -> &'static Mutex<Option<Runtime>> {
    static RUNTIME: OnceLock<Mutex<Option<Runtime>>> = OnceLock::new();
    RUNTIME.get_or_init(Default::default)
}

pub(super) fn submission_timeline(
    queue: &wgpu::Queue,
) -> Option<super::mod_gpu_resources::SubmissionTimeline> {
    let guard = runtime()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .as_ref()
        .filter(|runtime| runtime.queue == *queue)
        .map(|runtime| runtime.gpu.submission_timeline.clone())
}

fn callbacks() -> &'static Mutex<BTreeMap<ModId, Callback>> {
    static CALLBACKS: OnceLock<Mutex<BTreeMap<ModId, Callback>>> = OnceLock::new();
    CALLBACKS.get_or_init(Default::default)
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_recovery_path_for_tests(path: std::path::PathBuf) -> Result<(), String> {
    recovery::configure(path)
}

pub fn configure(device: &wgpu::Device, queue: &wgpu::Queue, lost: Arc<AtomicBool>) {
    let mut gpu = Gpu::default();
    let mut guard = runtime().lock().unwrap_or_else(|error| error.into_inner());
    if let Some(previous) = guard.as_mut() {
        if previous.lost.load(Ordering::Acquire) {
            recovery::device_lost();
            previous.gpu.device_lost();
        }
        gpu.restore_suspended(previous.gpu.suspended());
    }
    recovery::install(&mut gpu, lost.clone());
    gpu.submission_timeline.set_engine_tracking(
        active() && !device.features().contains(wgpu::Features::TIMESTAMP_QUERY),
    );
    *guard = Some(Runtime {
        device: device.clone(),
        queue: queue.clone(),
        lost,
        gpu,
        frame: Frame::default(),
        engine_frame: EngineFrame::default(),
        last_frame: None,
        started: Instant::now(),
        scene_reads: SceneReads::default(),
    });
}

pub fn register(owner: ModId, identity: &str, callback: Callback) {
    register_package(owner, identity, identity, callback);
}

pub fn register_package(owner: ModId, identity: &str, fingerprint: &str, callback: Callback) {
    callbacks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(owner, callback);
    let result = recovery::register(owner, identity, fingerprint);
    if let Some(runtime) = runtime()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_mut()
    {
        recovery::install(&mut runtime.gpu, runtime.lost.clone());
        runtime.gpu.submission_timeline.set_engine_tracking(
            !runtime
                .device
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY),
        );
        runtime.scene_reads = SceneReads::default();
        if let Err(error) = &result {
            runtime.gpu.disable(owner, error.clone());
        }
    }
    if let Err(error) = result {
        log::warn!("WASM GPU disabled for {identity}: {error}");
    }
    ACTIVE.store(true, Ordering::Release);
}

pub fn reset_resources(owner: ModId) -> Result<(), String> {
    with(owner, |gpu, _, _, _| {
        gpu.reset_resources(owner);
        Ok(())
    })
}

pub fn replace_package(owner: ModId, fingerprint: &str) -> Result<(), String> {
    let mut guard = runtime()
        .lock()
        .map_err(|_| "GPU runtime lock unavailable".to_owned())?;
    let runtime = guard.as_mut().ok_or("GPU runtime unavailable")?;
    if runtime.lost.load(Ordering::Acquire) {
        return Err("GPU device was lost; restart before replacing mods".to_owned());
    }
    recovery::replace_package(owner, fingerprint)?;
    runtime.gpu.remove_owner(owner);
    runtime.scene_reads = SceneReads::default();
    Ok(())
}

pub fn reenable(identity: &str) -> Result<(), String> {
    let mut guard = runtime()
        .lock()
        .map_err(|_| "GPU runtime lock unavailable".to_owned())?;
    if guard
        .as_ref()
        .is_some_and(|runtime| runtime.lost.load(Ordering::Acquire))
    {
        return Err("GPU device was lost; restart before re-enabling mods".to_owned());
    }
    let owners = recovery::reenable(identity)?;
    if let Some(runtime) = guard.as_mut() {
        for owner in owners {
            runtime.gpu.remove_owner(owner);
        }
    }
    Ok(())
}

pub fn disable(owner: ModId, reason: &str) {
    if let Some(runtime) = runtime()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_mut()
    {
        runtime.gpu.disable(owner, reason.to_owned());
    }
}

pub fn remove(owner: ModId) {
    let mut callbacks = callbacks()
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    callbacks.remove(&owner);
    ACTIVE.store(!callbacks.is_empty(), Ordering::Release);
    drop(callbacks);
    if let Some(runtime) = runtime()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_mut()
    {
        runtime.gpu.remove_owner(owner);
        runtime.gpu.submission_timeline.set_engine_tracking(
            active()
                && !runtime
                    .device
                    .features()
                    .contains(wgpu::Features::TIMESTAMP_QUERY),
        );
    }
}

pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

pub fn with<R>(
    owner: ModId,
    operation: impl FnOnce(&mut Gpu, &wgpu::Device, &wgpu::Queue, &Frame) -> Result<R, Error>,
) -> Result<R, String> {
    with_error(owner, operation).map_err(|error| error.to_string())
}

pub fn with_error<R>(
    owner: ModId,
    operation: impl FnOnce(&mut Gpu, &wgpu::Device, &wgpu::Queue, &Frame) -> Result<R, Error>,
) -> Result<R, Error> {
    with_timing(owner, false, operation)
}

pub fn with_creation_error<R>(
    owner: ModId,
    operation: impl FnOnce(&mut Gpu, &wgpu::Device, &wgpu::Queue, &Frame) -> Result<R, Error>,
) -> Result<R, Error> {
    with_timing(owner, true, operation)
}

fn with_timing<R>(
    owner: ModId,
    creation: bool,
    operation: impl FnOnce(&mut Gpu, &wgpu::Device, &wgpu::Queue, &Frame) -> Result<R, Error>,
) -> Result<R, Error> {
    let mut guard = runtime()
        .lock()
        .map_err(|_| Error::Host("GPU host unavailable".to_owned()))?;
    let runtime = guard
        .as_mut()
        .ok_or_else(|| Error::Host("GPU device is not ready".to_owned()))?;
    if runtime.lost.load(Ordering::Relaxed) {
        recovery::device_lost();
        runtime.gpu.device_lost();
    }
    if let Some(reason) = runtime.gpu.disabled_reason(owner) {
        return Err(Error::Host(format!("GPU disabled: {reason}")));
    }
    checked_operation(&mut runtime.gpu, owner, &runtime.device, |gpu| {
        timed_operation(gpu, owner, creation, Instant::now, |gpu| {
            operation(gpu, &runtime.device, &runtime.queue, &runtime.frame)
        })
    })
}

pub(super) fn checked_operation<R>(
    gpu: &mut Gpu,
    owner: ModId,
    device: &wgpu::Device,
    operation: impl FnOnce(&mut Gpu) -> Result<R, Error>,
) -> Result<R, Error> {
    let result = super::native_services::checked(device, || operation(gpu))
        .map_err(Error::Host)
        .and_then(|result| result);
    if let Err(Error::Host(error)) = &result {
        gpu.disable(owner, error.to_owned());
        log::warn!("WASM GPU mod {} disabled: {error}", owner.0);
    }
    result
}

pub(super) fn timed_operation<R>(
    gpu: &mut Gpu,
    owner: ModId,
    creation: bool,
    mut clock: impl FnMut() -> Instant,
    operation: impl FnOnce(&mut Gpu) -> Result<R, Error>,
) -> Result<R, Error> {
    let started = clock();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(gpu)));
    let elapsed = clock().saturating_duration_since(started);
    match result {
        Ok(result) => {
            if !creation {
                gpu.charge_cpu(owner, elapsed)?;
            }
            result
        }
        Err(_) => {
            gpu.disable(owner, "GPU host panic".to_owned());
            Err(Error::Host("GPU disabled after a host failure".to_owned()))
        }
    }
}

pub fn begin_frame(mut frame: Frame, mut engine_frame: EngineFrame) {
    if !active() {
        return;
    }
    let callbacks: Vec<_> = callbacks()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .map(|(owner, callback)| (*owner, callback.clone()))
        .collect();
    if callbacks.is_empty() {
        return;
    }
    {
        let mut guard = runtime().lock().unwrap_or_else(|error| error.into_inner());
        let Some(runtime) = guard.as_mut() else {
            return;
        };
        if runtime.lost.load(Ordering::Relaxed) {
            recovery::device_lost();
            runtime.gpu.device_lost();
            return;
        }
        let now = Instant::now();
        let ticks = frame.index;
        frame.index = runtime.frame.index.saturating_add(1);
        frame.delta_seconds = runtime
            .last_frame
            .map_or(0.0, |previous| now.duration_since(previous).as_secs_f32());
        engine_frame.header = [
            FRAME_VERSION,
            std::mem::size_of::<EngineFrame>() as u32,
            ticks as u32,
            (ticks >> 32) as u32,
        ];
        engine_frame.previous_view_projection = if runtime.last_frame.is_some() {
            runtime.engine_frame.view_projection
        } else {
            engine_frame.view_projection
        };
        engine_frame.camera_time[3] = now.duration_since(runtime.started).as_secs_f32();
        engine_frame.viewport_delta = [
            frame.width as f32,
            frame.height as f32,
            frame.delta_seconds,
            ticks as f32,
        ];
        runtime.last_frame = Some(now);
        runtime.engine_frame = engine_frame;
        runtime.gpu.configure_frame(
            frame.width,
            frame.height,
            frame.world_format,
            frame.hud_format,
        );
        runtime.frame = frame;
        recovery::begin_frame();
        runtime.gpu.collect_timings(&runtime.device);
        if let Err(error) = runtime.gpu.begin_frame() {
            log::warn!("WASM GPU frame rejected: {error}");
            return;
        }
    }
    for (owner, callback) in callbacks {
        if with(owner, |_, _, _, _| Ok(())).is_err() {
            continue;
        }
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback())).is_err() {
            let _ = with(owner, |gpu, _, _, _| {
                gpu.disable(owner, "GPU callback panic".to_owned());
                Ok(())
            });
        }
    }
    if let Some(runtime) = runtime()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_mut()
    {
        if let Err(error) = runtime.gpu.finish_frame() {
            log::warn!("WASM GPU frame rejected: {error}");
        }
        runtime.scene_reads.observe(runtime.gpu.reads_scene());
    }
}

pub fn has_stage(stage: Stage) -> bool {
    if !active() {
        return false;
    }
    runtime()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .is_some_and(|runtime| {
            !runtime.lost.load(Ordering::Relaxed) && runtime.gpu.has_stage(stage)
        })
}

pub fn reads_scene() -> bool {
    active()
        && runtime()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .is_some_and(|runtime| runtime.gpu.reads_scene())
}

pub fn scene_target_required() -> bool {
    active()
        && runtime()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .is_some_and(|runtime| {
                !runtime.lost.load(Ordering::Acquire) && runtime.scene_reads.required()
            })
}

pub fn clean_exit() {
    recovery::clean_exit();
}

pub fn device_lost() {
    recovery::device_lost();
}

pub fn submit_stage(stage: Stage, frame: Option<&FrameTarget<'_>>, mut sources: EngineSources<'_>) {
    let mut guard = runtime().lock().unwrap_or_else(|error| error.into_inner());
    let Some(runtime) = guard.as_mut() else {
        return;
    };
    if runtime.lost.load(Ordering::Relaxed) {
        recovery::device_lost();
        runtime.gpu.device_lost();
        return;
    }
    sources.frame = runtime.engine_frame;
    for (owner, error) in runtime.gpu.submit_stage_with_engine(
        &runtime.device,
        &runtime.queue,
        stage,
        frame,
        Some(&sources),
    ) {
        log::warn!("WASM GPU mod {} disabled: {error}", owner.0);
    }
}
