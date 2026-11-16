use super::*;
use crate::gfx::{mod_gpu, mod_gpu_resources, mod_gpu_runtime};
use std::sync::atomic::AtomicBool;

struct GpuTestHost;

impl Host for GpuTestHost {
    fn window(&self) -> bool {
        true
    }
}

fn guest(id: &str, bytes: &[u8], allowed: bool) -> FullMod {
    let client = load_client(&ModuleSource {
        id,
        bytes,
        commands: false,
        gpu: allowed,
    })
    .unwrap();
    FullMod::new(id.to_owned(), Some(client), None)
}

fn fov(module: &mut FullMod) -> f32 {
    let mut event = ComputeFovEvent {
        partial: 0.0,
        camera: [0.0; 3],
        degrees: 70.0,
    };
    module.compute_fov(&GpuTestHost, &mut event);
    event.degrees
}

fn frame() -> mod_gpu_runtime::Frame {
    mod_gpu_runtime::Frame {
        width: 256,
        height: 256,
        world_format: Some(mod_gpu::Format::Rgba8Unorm),
        ..Default::default()
    }
}

fn engine_frame() -> mod_gpu_resources::EngineFrame {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    mod_gpu_resources::EngineFrame {
        view: identity,
        projection: identity,
        view_projection: identity,
        inverse_view_projection: identity,
        previous_view_projection: identity,
        jittered_view_projection: identity,
        inverse_jittered_view_projection: identity,
        ..Default::default()
    }
}

fn engine_sources() -> mod_gpu_resources::EngineSources<'static> {
    mod_gpu_resources::EngineSources {
        frame: engine_frame(),
        atlas: None,
        sampler: None,
        depth: None,
        scene: None,
        lightmap: None,
    }
}

fn pixels(device: &wgpu::Device, queue: &wgpu::Queue, texture: &wgpu::Texture) -> Vec<u8> {
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("WASM GPU test pixels"),
        size: 256 * 256 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(1024),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    staging.slice(..).map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let result = staging.slice(..).get_mapped_range().unwrap().to_vec();
    staging.unmap();
    result
}

#[test]
fn gpu_guests_render_compute_reload_trap_and_device_loss() {
    use yoake_wasm::test_guests::{self, GPU_DEMO, GPU_RELOAD, GPU_TRAP, GPU_VERIFY};

    let Some(demo_bytes) = test_guests::load(&GPU_DEMO) else {
        return;
    };
    let Some(verify_bytes) = test_guests::load(&GPU_VERIFY) else {
        return;
    };
    let Some(reload_bytes) = test_guests::load(&GPU_RELOAD) else {
        return;
    };
    let Some(trap_bytes) = test_guests::load(&GPU_TRAP) else {
        return;
    };
    let scratch = std::env::temp_dir().join(format!(
        "yoake-gpu-guests-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    let recovery = scratch.join("recovery.json");
    mod_gpu_runtime::set_recovery_path_for_tests(recovery.clone()).unwrap();
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
    let (device, queue) = pollster::block_on(adapter.request_device(&Default::default())).unwrap();
    let lost = Arc::new(AtomicBool::new(false));
    mod_gpu_runtime::configure(&device, &queue, Arc::clone(&lost));
    let observer = mod_gpu::ModId(1);
    mod_gpu_runtime::register(observer, "gpu-observer", Arc::new(|| {}));
    mod_gpu_runtime::with(observer, |gpu, device, _, _| {
        gpu.create_buffer(device, observer, 16, wgpu::BufferUsages::COPY_DST.bits())
    })
    .unwrap();
    let observer_usage =
        mod_gpu_runtime::with(observer, |gpu, _, _, _| Ok(gpu.usage(observer))).unwrap();
    let reload = Arc::new(Reload::default());
    let mut verifier = guest("gpu-verify", &verify_bytes, true)
        .with_client_reload(Some(Arc::clone(&reload)))
        .with_gpu(true);
    let verifier_owner = verifier.gpu_owner.unwrap();
    for _ in 0..8 {
        mod_gpu_runtime::begin_frame(frame(), engine_frame());
        mod_gpu_runtime::submit_stage(mod_gpu::Stage::WorldOpaque, None, engine_sources());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        if fov(&mut verifier) == 91.0 {
            break;
        }
    }
    assert_eq!(fov(&mut verifier), 91.0);
    let demo = guest("gpu-demo", &demo_bytes, true).with_gpu(true);
    let demo_owner = demo.gpu_owner.unwrap();
    let make_texture = |format| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("WASM GPU test target"),
            size: wgpu::Extent3d {
                width: 256,
                height: 256,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let color = make_texture(wgpu::TextureFormat::Rgba8Unorm);
    let depth = make_texture(wgpu::TextureFormat::Depth32Float);
    let color_view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let mut encoder = device.create_command_encoder(&Default::default());
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.1,
                        g: 0.2,
                        b: 0.3,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });
    }
    queue.submit([encoder.finish()]);
    let mut previous = pixels(&device, &queue, &color);
    mod_gpu_runtime::begin_frame(frame(), engine_frame());
    let target = mod_gpu_resources::FrameTarget {
        color: &color_view,
        depth: Some(&depth_view),
        format: wgpu::TextureFormat::Rgba8Unorm,
    };
    for stage in [
        mod_gpu::Stage::WorldOpaque,
        mod_gpu::Stage::WorldTranslucent,
        mod_gpu::Stage::AfterWorld,
    ] {
        assert!(mod_gpu_runtime::has_stage(stage));
        mod_gpu_runtime::submit_stage(
            stage,
            Some(&target),
            mod_gpu_resources::EngineSources {
                depth: Some(&depth),
                scene: Some(&color),
                ..engine_sources()
            },
        );
        let disabled = mod_gpu_runtime::with(observer, |gpu, _, _, _| {
            Ok(gpu.disabled_reason(demo_owner).map(str::to_owned))
        })
        .unwrap();
        assert!(
            disabled.is_none(),
            "guest stage {stage:?} disabled the mod: {disabled:?}"
        );
        let current = pixels(&device, &queue, &color);
        assert!(
            current != previous,
            "guest stage {stage:?} must change rendered pixels"
        );
        previous = current;
    }
    drop(demo);
    assert_eq!(
        mod_gpu_runtime::with(observer, |gpu, _, _, _| Ok(gpu.usage(demo_owner))).unwrap(),
        mod_gpu::Usage::default()
    );
    *reload.bytes.lock().unwrap() = Some(reload_bytes);
    reload.ready.store(true, Ordering::Release);
    for _ in 0..8 {
        mod_gpu_runtime::begin_frame(frame(), engine_frame());
        mod_gpu_runtime::submit_stage(mod_gpu::Stage::WorldOpaque, None, engine_sources());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        if fov(&mut verifier) == 92.0 {
            break;
        }
    }
    assert_eq!(fov(&mut verifier), 92.0);
    let trap = guest("gpu-trap", &trap_bytes, true).with_gpu(true);
    let trap_owner = trap.gpu_owner.unwrap();
    mod_gpu_runtime::begin_frame(frame(), engine_frame());
    assert!(
        trap.client
            .lock()
            .unwrap()
            .module
            .as_ref()
            .unwrap()
            .is_disabled()
    );
    assert!(
        trap.masks[Side::Client as usize]
            .iter()
            .all(|mask| mask.load(Ordering::Relaxed) == 0)
    );
    assert_eq!(
        mod_gpu_runtime::with(observer, |gpu, _, _, _| Ok(gpu.usage(trap_owner))).unwrap(),
        mod_gpu::Usage::default()
    );
    assert_eq!(
        mod_gpu_runtime::with(observer, |gpu, _, _, _| Ok(gpu.usage(observer))).unwrap(),
        observer_usage
    );
    drop(trap);
    drop(verifier);
    assert_eq!(
        mod_gpu_runtime::with(observer, |gpu, _, _, _| Ok(gpu.usage(verifier_owner))).unwrap(),
        mod_gpu::Usage::default()
    );
    let denied = guest("gpu-denied", &verify_bytes, false);
    let _ = denied.with_client(
        0,
        HostAccess::None,
        None,
        ClientExtras::default(),
        |module| {
            module.gpu_frame();
            None::<()>
        },
    );
    assert!(
        denied
            .client
            .lock()
            .unwrap()
            .module
            .as_ref()
            .unwrap()
            .is_disabled()
    );
    lost.store(true, Ordering::Release);
    assert!(mod_gpu_runtime::with(observer, |_, _, _, _| Ok(())).is_err());
    mod_gpu_runtime::configure(&device, &queue, Arc::new(AtomicBool::new(false)));
    assert_eq!(
        mod_gpu_runtime::with(observer, |gpu, _, _, _| Ok(gpu.usage(observer))).unwrap(),
        mod_gpu::Usage::default()
    );
    mod_gpu_runtime::remove(observer);
    if recovery.exists() {
        std::fs::remove_file(&recovery).unwrap();
    }
    std::fs::remove_dir(&scratch).unwrap();
}
