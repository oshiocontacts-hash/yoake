use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use super::mod_gpu::{Error, ModId, Stage};
use super::mod_gpu_resources::{FrameTarget, Gpu};

pub const FRAME_VERSION: u32 = 1;

#[derive(Clone, Default)]
pub struct Frame {
    pub index: u64,
    pub width: u32,
    pub height: u32,
    pub view_projection: [f32; 16],
    pub camera: [f64; 3],
    pub delta_seconds: f32,
}

type Callback = Arc<dyn Fn() + Send + Sync>;

struct Runtime {
    device: wgpu::Device,
    queue: wgpu::Queue,
    lost: Arc<AtomicBool>,
    gpu: Gpu,
    frame: Frame,
}

fn runtime() -> &'static Mutex<Option<Runtime>> {
    static RUNTIME: OnceLock<Mutex<Option<Runtime>>> = OnceLock::new();
    RUNTIME.get_or_init(Default::default)
}

fn callbacks() -> &'static Mutex<BTreeMap<ModId, Callback>> {
    static CALLBACKS: OnceLock<Mutex<BTreeMap<ModId, Callback>>> = OnceLock::new();
    CALLBACKS.get_or_init(Default::default)
}

pub fn configure(device: &wgpu::Device, queue: &wgpu::Queue, lost: Arc<AtomicBool>) {
    let mut guard = runtime().lock().unwrap_or_else(|error| error.into_inner());
    *guard = Some(Runtime {
        device: device.clone(),
        queue: queue.clone(),
        lost,
        gpu: Gpu::default(),
        frame: Frame::default(),
    });
}

pub fn register(owner: ModId, callback: Callback) {
    callbacks().lock().unwrap_or_else(|error| error.into_inner()).insert(owner, callback);
}

pub fn remove(owner: ModId) {
    callbacks().lock().unwrap_or_else(|error| error.into_inner()).remove(&owner);
    if let Some(runtime) = runtime().lock().unwrap_or_else(|error| error.into_inner()).as_mut() {
        runtime.gpu.remove_owner(owner);
    }
}

pub fn with<R>(
    owner: ModId,
    operation: impl FnOnce(&mut Gpu, &wgpu::Device, &wgpu::Queue, &Frame) -> Result<R, Error>,
) -> Result<R, String> {
    let mut guard = runtime().lock().map_err(|_| "GPU host unavailable".to_owned())?;
    let runtime = guard.as_mut().ok_or_else(|| "GPU device is not ready".to_owned())?;
    if runtime.lost.load(Ordering::Relaxed) {
        runtime.gpu.device_lost();
    }
    if let Some(reason) = runtime.gpu.disabled_reason(owner) {
        return Err(format!("GPU disabled: {reason}"));
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        operation(&mut runtime.gpu, &runtime.device, &runtime.queue, &runtime.frame)
    }));
    match result {
        Ok(result) => result.map_err(|error| error.to_string()),
        Err(_) => {
            runtime.gpu.disable(owner, "GPU host panic".to_owned());
            Err("GPU disabled after a host failure".to_owned())
        }
    }
}

pub fn begin_frame(frame: Frame) {
    let callbacks: Vec<_> = callbacks().lock().unwrap_or_else(|error| error.into_inner())
        .iter().map(|(owner, callback)| (*owner, callback.clone())).collect();
    if callbacks.is_empty() {
        return;
    }
    {
        let mut guard = runtime().lock().unwrap_or_else(|error| error.into_inner());
        let Some(runtime) = guard.as_mut() else { return };
        if runtime.lost.load(Ordering::Relaxed) {
            runtime.gpu.device_lost();
            return;
        }
        runtime.frame = frame;
        runtime.gpu.begin_frame();
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
    if let Some(runtime) = runtime().lock().unwrap_or_else(|error| error.into_inner()).as_mut() {
        runtime.gpu.finish_frame();
    }
}

pub fn has_stage(stage: Stage) -> bool {
    runtime().lock().unwrap_or_else(|error| error.into_inner()).as_ref()
        .is_some_and(|runtime| !runtime.lost.load(Ordering::Relaxed) && runtime.gpu.has_stage(stage))
}

pub fn submit_stage(stage: Stage, frame: Option<&FrameTarget<'_>>) {
    let mut guard = runtime().lock().unwrap_or_else(|error| error.into_inner());
    let Some(runtime) = guard.as_mut() else { return };
    if runtime.lost.load(Ordering::Relaxed) {
        runtime.gpu.device_lost();
        return;
    }
    for (owner, error) in runtime.gpu.submit_stage(&runtime.device, &runtime.queue, stage, frame) {
        log::warn!("WASM GPU mod {} disabled: {error}", owner.0);
    }
}
