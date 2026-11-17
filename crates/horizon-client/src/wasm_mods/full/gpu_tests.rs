use super::*;
use crate::gfx::{mod_gpu, mod_gpu_resources, mod_gpu_runtime};
use std::sync::atomic::AtomicBool;

struct GpuTestHost;

impl Host for GpuTestHost {
    fn window(&self) -> bool {
        true
    }
}

fn test_limits() -> yoake_wasm::TimeLimits {
    yoake_wasm::TimeLimits {
        call: std::time::Duration::from_secs(5),
        aggregate: std::time::Duration::from_secs(30),
    }
}

fn guest(id: &str, bytes: &[u8], allowed: bool) -> FullMod {
    let mut client = load_client(&ModuleSource {
        id,
        bytes,
        commands: false,
        gpu: allowed,
    })
    .unwrap();
    client.set_test_limits(test_limits());
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

fn wait_for_fov(module: &mut FullMod, device: &wgpu::Device, expected: f32) {
    let mut actual = 0.0;
    for _ in 0..60 {
        std::thread::sleep(yoake_wasm::AGGREGATE_WINDOW);
        mod_gpu_runtime::begin_frame(frame(), engine_frame());
        mod_gpu_runtime::submit_stage(mod_gpu::Stage::WorldOpaque, None, engine_sources());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        actual = fov(module);
        if actual == expected {
            break;
        }
    }
    assert_eq!(actual, expected);
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
    assert_ne!(verify_bytes, reload_bytes);
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
    let reload = Arc::new(Reload {
        test_limits: Some(test_limits()),
        ..Reload::default()
    });
    let mut verifier = guest("gpu-verify", &verify_bytes, true)
        .with_client_reload(Some(Arc::clone(&reload)))
        .with_gpu(true, "1", "gpu-invalid-v1");
    let verifier_owner = verifier.gpu_owner.unwrap();
    wait_for_fov(&mut verifier, &device, 91.0);
    for _ in 0..256 {
        mod_gpu_runtime::begin_frame(frame(), engine_frame());
        mod_gpu_runtime::submit_stage(mod_gpu::Stage::WorldOpaque, None, engine_sources());
    }
    let demo = guest("gpu-demo", &demo_bytes, true).with_gpu(true, "1", "gpu-demo-v1");
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
    let clear_target = || {
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
    };
    clear_target();
    let target = mod_gpu_resources::FrameTarget {
        color: &color_view,
        depth: Some(&depth_view),
        format: wgpu::TextureFormat::Rgba8Unorm,
    };
    for _ in 0..4 {
        std::thread::sleep(yoake_wasm::AGGREGATE_WINDOW);
        mod_gpu_runtime::begin_frame(frame(), engine_frame());
        for stage in [
            mod_gpu::Stage::WorldOpaque,
            mod_gpu::Stage::WorldTranslucent,
            mod_gpu::Stage::AfterWorld,
        ] {
            mod_gpu_runtime::submit_stage(stage, Some(&target), engine_sources());
        }
    }
    let mut failure = None;
    for _ in 0..60 {
        std::thread::sleep(yoake_wasm::AGGREGATE_WINDOW);
        clear_target();
        mod_gpu_runtime::begin_frame(frame(), engine_frame());
        let mut previous = pixels(&device, &queue, &color);
        failure = None;
        for stage in [
            mod_gpu::Stage::WorldOpaque,
            mod_gpu::Stage::WorldTranslucent,
            mod_gpu::Stage::AfterWorld,
        ] {
            if !mod_gpu_runtime::has_stage(stage) {
                failure = Some(format!("guest stage {stage:?} was not submitted"));
                break;
            }
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
            if current == previous {
                failure = Some(format!("guest stage {stage:?} did not change rendered pixels"));
                break;
            }
            previous = current;
        }
        if failure.is_none() {
            break;
        }
    }
    assert!(failure.is_none(), "{failure:?}");
    drop(demo);
    assert_eq!(
        mod_gpu_runtime::with(observer, |gpu, _, _, _| Ok(gpu.usage(demo_owner))).unwrap(),
        mod_gpu::Usage::default()
    );
    *reload.bytes.lock().unwrap() = Some(reload_bytes);
    reload.ready.store(true, Ordering::Release);
    assert_eq!(
        mod_gpu_runtime::with(observer, |gpu, _, _, _| {
            Ok(gpu.disabled_reason(verifier_owner).map(str::to_owned))
        })
        .unwrap(),
        None,
        "verifier must not be suspended before reload"
    );
    wait_for_fov(&mut verifier, &device, 92.0);
    let trap = guest("gpu-trap", &trap_bytes, true).with_gpu(true, "1", "gpu-trap-v1");
    let trap_owner = trap.gpu_owner.unwrap();
    for _ in 0..60 {
        std::thread::sleep(yoake_wasm::AGGREGATE_WINDOW);
        mod_gpu_runtime::begin_frame(frame(), engine_frame());
        if trap.client.lock().unwrap().module.as_ref().unwrap().is_disabled() {
            break;
        }
    }
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
