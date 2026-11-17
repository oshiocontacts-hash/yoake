use super::*;
use crate::gfx::mod_gpu::Stage;

pub(super) fn gpu() -> (wgpu::Device, wgpu::Queue, std::sync::MutexGuard<'static, ()>) {
    let guard = crate::gfx::mod_gpu_runtime::test_guard();
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("headless GPU adapter");
    let (device, queue) =
        pollster::block_on(adapter.request_device(&Default::default())).expect("headless GPU device");
    (device, queue, guard)
}

#[test]
fn empty_frames_do_not_allocate_or_submit() {
    let mut gpu = Gpu::default();
    for _frame in 0..3 {
        gpu.begin_frame().unwrap();
        gpu.finish_frame().unwrap();
    }
    assert_eq!(gpu.total_bytes(), 0);
    for stage in [
        Stage::WorldOpaque,
        Stage::WorldTranslucent,
        Stage::AfterWorld,
        Stage::Hud,
    ] {
        assert!(!gpu.has_stage(stage));
    }
}

#[test]
fn exhausted_upload_budget_is_checked_before_staging_allocations() {
    let (device, _queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    let owner = ModId(1);
    let buffer = gpu
        .create_buffer(&device, owner, 16, wgpu::BufferUsages::COPY_DST.bits())
        .unwrap();
    let texture = gpu
        .create_texture(
            &device,
            owner,
            Texture {
                width: 2,
                height: 2,
                depth_or_layers: 1,
                mips: 1,
                is_3d: false,
                format: Format::Rgba8Unorm,
            },
            wgpu::TextureUsages::COPY_DST.bits(),
        )
        .unwrap();
    gpu.begin_frame().unwrap();
    let resource = gpu.resources.get(owner, buffer).unwrap();
    gpu.recorder
        .record(
            owner,
            Stage::Hud,
            crate::gfx::mod_gpu::CommandKind::Upload(crate::gfx::mod_gpu::MAX_UPLOAD_BYTES),
            || commands::Command::Copy {
                source: resource.clone(),
                destination: resource,
                source_offset: 0,
                destination_offset: 0,
                size: 0,
            },
        )
        .unwrap();
    let remaining = crate::gfx::mod_gpu::MAX_MOD_BYTES - gpu.usage(owner).bytes;
    gpu.resources
        .create(owner, Allocation::Buffer(remaining), || {
            Ok(Value::Sampler(device.create_sampler(&Default::default())))
        })
        .unwrap();
    let usage = gpu.usage(owner);
    assert_eq!(
        gpu.write_buffer(&device, owner, Stage::Hud, buffer, 0, &[0; 16]),
        Err(Error::Upload)
    );
    assert_eq!(
        gpu.write_texture(
            &device,
            owner,
            Stage::Hud,
            texture,
            0,
            [0; 3],
            [2, 2, 1],
            &[0; 16],
        ),
        Err(Error::Upload)
    );
    assert_eq!(gpu.usage(owner), usage);
    assert_eq!(gpu.disabled_reason(owner), None);
}

#[test]
fn engine_uncaptured_errors_keep_the_default_behavior_before_and_after_mod_creation() {
    for create_mod_objects in [false, true] {
        let (device, _queue, _guard) = gpu();
        let mut gpu = Gpu::default();
        let owner = ModId(1);
        if create_mod_objects {
            gpu.create_buffer(&device, owner, 16, wgpu::BufferUsages::COPY_DST.bits())
                .unwrap();
        }
        let usage = gpu.usage(owner);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("engine validation regression"),
                size: device.limits().max_buffer_size + 4,
                usage: wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        }));
        assert!(result.is_err());
        assert_eq!(gpu.usage(owner), usage);
        assert_eq!(gpu.disabled_reason(owner), None);
    }
}

#[test]
fn mod_scoped_errors_are_returned_without_replacing_the_engine_handler() {
    let (device, _queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    let owner = ModId(1);
    let handle = gpu
        .create_buffer(&device, owner, 16, wgpu::BufferUsages::COPY_DST.bits())
        .unwrap();
    let usage = gpu.usage(owner);
    let result = gpu.resources.create(owner, Allocation::Buffer(16), || {
        checked(&device, || Value::Buffer {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mod validation regression"),
                size: device.limits().max_buffer_size + 4,
                usage: wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            size: 16,
            usage: wgpu::BufferUsages::COPY_DST,
        })
    });
    assert!(matches!(result, Err(Error::Host(_))));
    assert_eq!(gpu.usage(owner), usage);
    assert!(gpu.resources.get(owner, handle).is_ok());
    assert_eq!(gpu.disabled_reason(owner), None);
}

#[test]
fn outer_scoped_validation_and_timed_panics_disable_only_their_owner() {
    let (device, _queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    gpu.set_test_time_allowance(Duration::from_secs(5));
    let healthy = ModId(81);
    let healthy_buffer = gpu
        .create_buffer(&device, healthy, 16, wgpu::BufferUsages::COPY_DST.bits())
        .unwrap();
    let healthy_usage = gpu.usage(healthy);
    for (owner, panic) in [(ModId(82), false), (ModId(83), true)] {
        gpu.create_buffer(&device, owner, 16, wgpu::BufferUsages::COPY_DST.bits())
            .unwrap();
        let now = Instant::now();
        let result =
            crate::gfx::mod_gpu_runtime::checked_operation(&mut gpu, owner, &device, |gpu| {
                crate::gfx::mod_gpu_runtime::timed_operation(
                    gpu,
                    owner,
                    false,
                    || now,
                    |_| {
                        assert!(!panic, "timed GPU host panic");
                        let _ = device.create_buffer(&wgpu::BufferDescriptor {
                            label: None,
                            size: device.limits().max_buffer_size + 4,
                            usage: wgpu::BufferUsages::COPY_DST,
                            mapped_at_creation: false,
                        });
                        Ok(())
                    },
                )
            });
        assert!(matches!(result, Err(Error::Host(_))));
        assert!(gpu.disabled_reason(owner).is_some());
        assert_eq!(gpu.usage(owner), crate::gfx::mod_gpu::Usage::default());
        assert_eq!(gpu.disabled_reason(healthy), None);
        assert_eq!(gpu.usage(healthy), healthy_usage);
        assert!(gpu.resources.get(healthy, healthy_buffer).is_ok());
    }
}

#[test]
fn creation_limits_are_per_owner_and_survive_resource_reset() {
    let mut gpu = Gpu::default();
    let owner = ModId(1);
    gpu.creation(owner).unwrap();
    gpu.creation(owner).unwrap();
    assert_eq!(gpu.creation(owner), Err(Error::CreationRate));
    assert!(gpu.creation(ModId(2)).is_ok());
    gpu.reset_resources(owner);
    assert_eq!(gpu.creation(owner), Err(Error::CreationRate));
    let now = Instant::now();
    let budget = gpu.creations.get_mut(&owner).unwrap();
    for frame in 1..4 {
        budget.charge(now, frame).unwrap();
        budget.charge(now, frame).unwrap();
    }
    assert_eq!(budget.charge(now, 4), Err(Error::CreationRate));
    assert!(budget.charge(now + Duration::from_secs(60), 5).is_ok());
    gpu.disable(owner, "test suspension".to_owned());
    gpu.reset_resources(owner);
    let mut recreated = Gpu::default();
    recreated.restore_suspended(gpu.suspended());
    assert_eq!(recreated.disabled_reason(owner), Some("test suspension"));
}

#[test]
fn compute_dispatch_validates_device_dimensions_separately_from_work_budget() {
    let (device, queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    gpu.set_test_time_allowance(Duration::from_secs(5));
    let owner = ModId(1);
    let shader = gpu
        .create_shader(&device, owner, "@compute @workgroup_size(1) fn main() {}")
        .unwrap();
    let pipeline = gpu.create_compute(&device, owner, shader, "main").unwrap();
    let limit = device.limits().max_compute_workgroups_per_dimension;
    let excessive = limit.checked_add(1).unwrap();
    gpu.begin_frame().unwrap();
    for workgroups in [
        [0, 1, 1],
        [1, 0, 1],
        [1, 1, 0],
        [excessive, 1, 1],
        [1, excessive, 1],
        [1, 1, excessive],
    ] {
        assert_eq!(
            gpu.dispatch(owner, Stage::AfterWorld, pipeline, &[], workgroups),
            Err(Error::Workgroups),
            "{workgroups:?}"
        );
    }
    for workgroups in [[limit, 1, 1], [1, limit, 1], [1, 1, limit], [256, 257, 1]] {
        gpu.dispatch(owner, Stage::AfterWorld, pipeline, &[], workgroups)
            .unwrap();
    }
    gpu.finish_frame().unwrap();
    assert!(
        gpu.submit_stage(&device, &queue, Stage::AfterWorld, None)
            .is_empty()
    );
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    assert_eq!(gpu.disabled_reason(owner), None);
}

#[test]
fn compute_upload_copy_readback_and_ownership() {
    let (device, queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    gpu.set_test_time_allowance(Duration::from_secs(5));
    let owner = ModId(1);
    let usage =
        (wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST)
            .bits();
    let data = gpu.create_buffer(&device, owner, 16, usage).unwrap();
    let output = gpu.create_buffer(&device, owner, 16, usage).unwrap();
    let shader = gpu.create_shader(&device, owner, "@group(0) @binding(0) var<storage, read_write> values: array<u32>; @compute @workgroup_size(1) fn main(@builtin(global_invocation_id) id: vec3<u32>) { values[id.x] += 7u; }").unwrap();
    let pipeline = gpu.create_compute(&device, owner, shader, "main").unwrap();
    let group = gpu
        .create_bind_group(
            &device,
            owner,
            pipeline,
            0,
            &[Binding::Buffer {
                binding: 0,
                handle: data,
                offset: 0,
                size: 16,
            }],
        )
        .unwrap();
    gpu.begin_frame().unwrap();
    assert!(
        gpu.write_buffer(&device, ModId(2), Stage::AfterWorld, data, 0, &[0; 16])
            .is_err()
    );
    assert!(
        gpu.write_buffer(&device, owner, Stage::AfterWorld, data, 4, &[0; 16])
            .is_err()
    );
    gpu.write_buffer(
        &device,
        owner,
        Stage::AfterWorld,
        data,
        0,
        bytemuck::cast_slice(&[1u32, 2, 3, 4]),
    )
    .unwrap();
    gpu.dispatch(owner, Stage::AfterWorld, pipeline, &[group], [4, 1, 1])
        .unwrap();
    gpu.copy_buffer(owner, Stage::AfterWorld, data, 0, output, 0, 16)
        .unwrap();
    let ticket = gpu
        .read_buffer(&device, owner, Stage::AfterWorld, output, 0, 16)
        .unwrap();
    gpu.finish_frame().unwrap();
    gpu.destroy(owner, shader).unwrap();
    gpu.destroy(owner, group).unwrap();
    assert!(
        gpu.submit_stage(&device, &queue, Stage::AfterWorld, None)
            .is_empty()
    );
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let Readback::Ready(bytes) = gpu.poll_readback(&device, owner, ticket).unwrap() else {
        panic!("readback pending after wait")
    };
    assert_eq!(bytemuck::cast_slice::<u8, u32>(&bytes), &[8, 9, 10, 11]);
    assert!(gpu.poll_readback(&device, owner, ticket).is_err());
    gpu.remove_owner(owner);
    assert_eq!(gpu.total_bytes(), 0);
}

#[test]
fn texture_bounds_and_device_loss_are_isolated() {
    let (device, _queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    gpu.set_test_time_allowance(Duration::from_secs(5));
    let owner = ModId(3);
    let texture = gpu
        .create_texture(
            &device,
            owner,
            Texture {
                width: 2,
                height: 2,
                depth_or_layers: 1,
                mips: 1,
                is_3d: false,
                format: Format::Rgba8Unorm,
            },
            wgpu::TextureUsages::COPY_DST.bits(),
        )
        .unwrap();
    gpu.begin_frame().unwrap();
    assert!(
        gpu.write_texture(
            &device,
            owner,
            Stage::Hud,
            texture,
            0,
            [1, 0, 0],
            [2, 2, 1],
            &[0; 16]
        )
        .is_err()
    );
    assert!(
        gpu.write_texture(
            &device,
            owner,
            Stage::Hud,
            texture,
            1,
            [0; 3],
            [1; 3],
            &[0; 4]
        )
        .is_err()
    );
    gpu.write_texture(
        &device,
        owner,
        Stage::Hud,
        texture,
        0,
        [0; 3],
        [2, 2, 1],
        &[255; 16],
    )
    .unwrap();
    gpu.finish_frame().unwrap();
    gpu.disable(owner, "test".to_owned());
    assert!(!gpu.has_stage(Stage::Hud));
    assert_eq!(gpu.total_bytes(), 0);
    assert!(matches!(
        gpu.create_sampler(&device, owner, false, false),
        Err(Error::Disabled)
    ));
    assert!(gpu.create_sampler(&device, ModId(4), false, false).is_ok());
    gpu.device_lost();
    assert_eq!(gpu.total_bytes(), 0);
    assert!(matches!(
        gpu.create_sampler(&device, ModId(4), false, false),
        Err(Error::Disabled)
    ));
}

fn render_pipeline(
    gpu: &mut Gpu,
    device: &wgpu::Device,
    owner: ModId,
    topology: wgpu::PrimitiveTopology,
) -> u32 {
    let shader = gpu.create_shader(device, owner, "@vertex fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> { let positions = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0)); return vec4(positions[index % 3u], 0.0, 1.0); } @fragment fn fragment() -> @location(0) vec4<f32> { return vec4(1.0, 0.0, 0.0, 1.0); }").unwrap();
    gpu.create_render(
        device,
        owner,
        shader,
        RenderDescriptor {
            vertex_entry: "vertex".to_owned(),
            fragment_entry: "fragment".to_owned(),
            stride: 0,
            attributes: Vec::new(),
            color: Format::Rgba8Unorm,
            depth: false,
            depth_write: false,
            topology,
            cull: None,
            blend: None,
        },
    )
    .unwrap()
}

fn draw(pipeline: u32, count: u32) -> Draw {
    Draw {
        pipeline,
        groups: Vec::new(),
        vertex: None,
        index: None,
        count,
        instances: 1,
    }
}

#[test]
fn consecutive_draws_share_passes_and_reject_incompatible_frames() {
    let (device, queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    gpu.set_test_time_allowance(Duration::from_secs(5));
    let owner = ModId(7);
    gpu.configure_frame(64, 64, Some(Format::Rgba8Unorm), Some(Format::Bgra8Unorm));
    let pipeline = render_pipeline(
        &mut gpu,
        &device,
        owner,
        wgpu::PrimitiveTopology::TriangleList,
    );
    let target = gpu
        .create_texture(
            &device,
            owner,
            Texture {
                width: 2,
                height: 2,
                depth_or_layers: 1,
                mips: 1,
                is_3d: false,
                format: Format::Rgba8Unorm,
            },
            (wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC).bits(),
        )
        .unwrap();
    gpu.begin_frame().unwrap();
    assert_eq!(
        gpu.draw(owner, Stage::Hud, draw(pipeline, 3), Target::Frame),
        Err(Error::Descriptor)
    );
    assert!(gpu.disabled_reason(owner).is_none());
    for _draw in 0..128 {
        gpu.draw(
            owner,
            Stage::WorldOpaque,
            draw(pipeline, 3),
            Target::Texture {
                color: target,
                depth: None,
                clear: None,
            },
        )
        .unwrap();
    }
    for _pass in 1..64 {
        gpu.draw(
            owner,
            Stage::WorldOpaque,
            draw(pipeline, 3),
            Target::Texture {
                color: target,
                depth: None,
                clear: Some([0.0; 4]),
            },
        )
        .unwrap();
    }
    assert_eq!(
        gpu.draw(
            owner,
            Stage::WorldOpaque,
            draw(pipeline, 3),
            Target::Texture {
                color: target,
                depth: None,
                clear: Some([0.0; 4]),
            }
        ),
        Err(Error::Passes)
    );
    gpu.finish_frame().unwrap();
    assert!(
        gpu.submit_stage(&device, &queue, Stage::WorldOpaque, None)
            .is_empty()
    );
    let color = gpu.resources.get(owner, target).unwrap();
    let Value::Texture { texture, .. } = color.value() else {
        panic!("texture target")
    };
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 512,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(2),
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    readback.map_async(wgpu::MapMode::Read, .., move |result| {
        sender.send(result).unwrap()
    });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receiver.recv().unwrap().unwrap();
    {
        let bytes = readback.get_mapped_range(..).unwrap();
        assert_eq!(&bytes[..4], &[255, 0, 0, 255]);
        assert_eq!(&bytes[256..260], &[255, 0, 0, 255]);
    }
    readback.unmap();
}

#[test]
fn fragment_budget_counts_primitives_for_every_supported_topology() {
    let (device, _queue, _guard) = gpu();
    for (owner, topology, vertices) in [
        (ModId(10), wgpu::PrimitiveTopology::PointList, 1),
        (ModId(11), wgpu::PrimitiveTopology::LineList, 2),
        (ModId(12), wgpu::PrimitiveTopology::TriangleList, 3),
    ] {
        let mut gpu = Gpu::default();
        gpu.set_test_time_allowance(Duration::from_secs(5));
        gpu.configure_frame(1, 1, Some(Format::Rgba8Unorm), Some(Format::Rgba8Unorm));
        let pipeline = render_pipeline(&mut gpu, &device, owner, topology);
        for (primitives, pixels) in [
            (2048, Some(1)),
            (2049, Some(2)),
            (8192, Some(4)),
            (8193, None),
        ] {
            gpu.begin_frame().unwrap();
            let result = gpu.draw(
                owner,
                Stage::Hud,
                draw(pipeline, primitives * vertices),
                Target::Frame,
            );
            if let Some(expected) = pixels {
                result.unwrap();
                gpu.finish_frame().unwrap();
                assert!(
                    matches!(gpu.pending[0].kind, crate::gfx::mod_gpu::CommandKind::Render { fragment_work, .. } if fragment_work.div_ceil(2048) == expected)
                );
            } else {
                result.unwrap();
                gpu.finish_frame().unwrap();
                assert!(gpu.pending.is_empty());
            }
        }
        assert!(gpu.disabled_reason(owner).is_none());
    }
}

#[test]
fn frame_fragment_budget_rounds_once_across_small_mesh_draws() {
    let (device, _queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    gpu.set_test_time_allowance(Duration::from_secs(5));
    let owner = ModId(16);
    gpu.configure_frame(
        1280,
        720,
        Some(Format::Rgba8Unorm),
        Some(Format::Rgba8Unorm),
    );
    let pipeline = render_pipeline(
        &mut gpu,
        &device,
        owner,
        wgpu::PrimitiveTopology::TriangleList,
    );
    gpu.begin_frame().unwrap();
    for _ in 0..200 {
        gpu.draw_region(
            owner,
            Stage::Hud,
            draw(pipeline, 500 * 3),
            Target::Frame,
            Region {
                viewport: Some(Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                }),
                scissor: None,
            },
        )
        .unwrap();
    }
    gpu.finish_frame().unwrap();
    assert_eq!(gpu.pending.len(), 200);
    let fragment_work: u64 = gpu
        .pending
        .iter()
        .map(|command| match command.kind {
            crate::gfx::mod_gpu::CommandKind::Render { fragment_work, .. } => fragment_work,
            _ => panic!("expected mesh draws"),
        })
        .sum();
    assert_eq!(fragment_work, 100_000);
    assert_eq!(fragment_work.div_ceil(2048), 49);
    gpu.begin_frame().unwrap();
    assert_eq!(
        (0..500).try_for_each(|_| {
            gpu.draw(owner, Stage::Hud, draw(pipeline, 2000 * 3), Target::Frame)
        }),
        Ok(())
    );
    gpu.finish_frame().unwrap();
    assert_eq!(gpu.pending.len(), 4);
    assert!(gpu.disabled_reason(owner).is_none());
}

#[test]
fn new_mod_fragment_budget_admits_meshes_and_small_instanced_quads() {
    let (device, _queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    gpu.set_test_time_allowance(Duration::from_secs(5));
    let owner = ModId(15);
    gpu.configure_frame(100, 100, Some(Format::Rgba8Unorm), Some(Format::Rgba8Unorm));
    let pipeline = render_pipeline(
        &mut gpu,
        &device,
        owner,
        wgpu::PrimitiveTopology::TriangleList,
    );
    gpu.begin_frame().unwrap();
    gpu.draw(owner, Stage::Hud, draw(pipeline, 2048 * 3), Target::Frame)
        .unwrap();
    gpu.finish_frame().unwrap();
    gpu.begin_frame().unwrap();
    assert_eq!(
        gpu.draw(
            owner,
            Stage::Hud,
            draw(pipeline, 1_000_000 * 3),
            Target::Frame
        ),
        Ok(())
    );
    gpu.finish_frame().unwrap();
    assert!(gpu.pending.is_empty());
    for instances in [10_000, 65_536] {
        gpu.begin_frame().unwrap();
        let mut particles = draw(pipeline, 6);
        particles.instances = instances;
        gpu.draw_region(
            owner,
            Stage::Hud,
            particles,
            Target::Frame,
            Region {
                viewport: Some(Rect {
                    x: 0,
                    y: 0,
                    width: 10,
                    height: 10,
                }),
                scissor: None,
            },
        )
        .unwrap();
        gpu.finish_frame().unwrap();
        assert_eq!(gpu.pending.len(), 1);
    }
    gpu.begin_frame().unwrap();
    let mut vertex_heavy = draw(pipeline, 6);
    vertex_heavy.instances = 200_000;
    assert_eq!(
        gpu.draw_region(
            owner,
            Stage::Hud,
            vertex_heavy,
            Target::Frame,
            Region {
                viewport: Some(Rect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1
                }),
                scissor: None,
            }
        ),
        Err(Error::Vertices)
    );
    assert!(gpu.disabled_reason(owner).is_none());
}

#[test]
fn slow_creation_is_exempt_and_failed_cpu_charge_reclaims_successful_results() {
    let (device, _queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    let owner = ModId(94);
    let started = Instant::now();
    let mut clock = [started, started + Duration::from_secs(10)].into_iter();
    let handle = crate::gfx::mod_gpu_runtime::timed_operation(
        &mut gpu,
        owner,
        true,
        || clock.next().unwrap(),
        |gpu| gpu.create_buffer(&device, owner, 4, wgpu::BufferUsages::COPY_DST.bits()),
    )
    .unwrap();
    assert!(gpu.disabled_reason(owner).is_none());
    assert!(gpu.resources.get(owner, handle).is_ok());
    for frame in 1..=3 {
        gpu.frame_index = frame;
        let mut clock = [started, started + Duration::from_secs(1)].into_iter();
        let result = crate::gfx::mod_gpu_runtime::timed_operation(
            &mut gpu,
            owner,
            false,
            || clock.next().unwrap(),
            |gpu| gpu.create_buffer(&device, owner, 4, wgpu::BufferUsages::COPY_DST.bits()),
        );
        assert_eq!(result.is_err(), frame == 3);
    }
    assert!(gpu.disabled_reason(owner).is_some());
    assert_eq!(gpu.total_bytes(), 0);
    assert!(gpu.resources.get(owner, handle).is_err());
}

#[test]
fn a_skipped_stage_is_discarded_without_wedging_other_owners() {
    let (device, queue, _guard) = gpu();
    let mut gpu = Gpu::default();
    gpu.set_test_time_allowance(Duration::from_secs(5));
    let usage = (wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST).bits();
    let stale = gpu.create_buffer(&device, ModId(20), 4, usage).unwrap();
    let current = gpu.create_buffer(&device, ModId(21), 4, usage).unwrap();
    gpu.begin_frame().unwrap();
    gpu.write_buffer(&device, ModId(20), Stage::AfterWorld, stale, 0, &[99; 4])
        .unwrap();
    gpu.finish_frame().unwrap();
    gpu.begin_frame().unwrap();
    gpu.write_buffer(&device, ModId(21), Stage::AfterWorld, current, 0, &[7; 4])
        .unwrap();
    let stale_ticket = gpu
        .read_buffer(&device, ModId(20), Stage::AfterWorld, stale, 0, 4)
        .unwrap();
    let current_ticket = gpu
        .read_buffer(&device, ModId(21), Stage::AfterWorld, current, 0, 4)
        .unwrap();
    gpu.finish_frame().unwrap();
    assert!(
        gpu.submit_stage(&device, &queue, Stage::AfterWorld, None)
            .is_empty()
    );
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let Readback::Ready(stale_bytes) = gpu.poll_readback(&device, ModId(20), stale_ticket).unwrap()
    else {
        panic!("stale readback pending")
    };
    let Readback::Ready(current_bytes) = gpu
        .poll_readback(&device, ModId(21), current_ticket)
        .unwrap()
    else {
        panic!("current readback pending")
    };
    assert_eq!(stale_bytes, [0; 4]);
    assert_eq!(current_bytes, [7; 4]);
}
