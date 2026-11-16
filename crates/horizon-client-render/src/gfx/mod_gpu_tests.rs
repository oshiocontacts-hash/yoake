use super::*;

fn texture() -> Texture {
    Texture {
        width: 4,
        height: 2,
        depth_or_layers: 1,
        mips: 1,
        is_3d: false,
        format: Format::Rgba8Unorm,
    }
}

#[test]
fn handles_enforce_owner_kind_and_lifetime() {
    let mut resources = Resources::default();
    let first = resources
        .create(ModId(1), Allocation::Buffer(4), || Ok(42))
        .unwrap();
    let foreign = resources
        .create(ModId(2), Allocation::Sampler, || Ok(17))
        .unwrap();
    assert_eq!(*resources.get(ModId(1), first).unwrap().value(), 42);
    for handle in [0, foreign, u32::MAX] {
        assert_eq!(resources.get(ModId(1), handle).err(), Some(Error::Handle));
        assert_eq!(resources.destroy(ModId(1), handle), Err(Error::Handle));
    }
    assert_eq!(
        resources
            .get_kind(ModId(1), first, Allocation::Sampler)
            .err(),
        Some(Error::Kind)
    );
    resources.destroy(ModId(1), first).unwrap();
    assert_eq!(resources.get(ModId(1), first).err(), Some(Error::Handle));
    assert_eq!(resources.destroy(ModId(1), first), Err(Error::Handle));
    let next = resources
        .create(ModId(1), Allocation::Buffer(4), || Ok(1))
        .unwrap();
    assert_ne!(next, first);
    resources.remove_owner(ModId(1));
    assert_eq!(resources.get(ModId(1), next).err(), Some(Error::Handle));
    assert_eq!(*resources.get(ModId(2), foreign).unwrap().value(), 17);
}

#[test]
fn retained_commands_and_bindings_keep_memory_charged() {
    let mut resources = Resources::default();
    let handle = resources
        .create(ModId(1), Allocation::Buffer(268_435_456), || Ok(()))
        .unwrap();
    let retained = resources.get(ModId(1), handle).unwrap();
    resources.destroy(ModId(1), handle).unwrap();
    assert_eq!(
        resources.usage(ModId(1)),
        Usage {
            bytes: 268_435_456,
            handles: 1
        }
    );
    assert_eq!(
        resources.create(ModId(1), Allocation::Buffer(4), || Ok(())),
        Err(Error::Memory)
    );
    resources.remove_owner(ModId(1));
    assert_eq!(resources.total_bytes(), 268_435_456);
    drop(retained);
    assert_eq!(resources.usage(ModId(1)), Usage::default());
    assert_eq!(resources.total_bytes(), 0);
    resources
        .create(ModId(1), Allocation::Buffer(4), || Ok(()))
        .unwrap();
}

#[test]
fn reservation_failure_and_unwind_release_budgets() {
    let mut resources = Resources::<()>::default();
    assert_eq!(
        resources.create(ModId(1), Allocation::Buffer(0), || panic!(
            "invalid descriptors must not reach the GPU"
        )),
        Err(Error::BufferSize)
    );
    assert_eq!(
        resources.create(ModId(1), Allocation::Buffer(4), || Err(Error::Host(
            "validation".into()
        ))),
        Err(Error::Host("validation".into()))
    );
    assert_eq!(resources.total_bytes(), 0);
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = resources.create(ModId(1), Allocation::Buffer(4), || {
                panic!("simulated driver failure")
            });
        }))
        .is_err()
    );
    assert_eq!(resources.usage(ModId(1)), Usage::default());
    assert_eq!(resources.total_bytes(), 0);
    let handle = resources
        .create(ModId(1), Allocation::Buffer(4), || Ok(()))
        .unwrap();
    assert!(handle > 2);
}

#[test]
fn memory_caps_apply_per_mod_and_across_mods() {
    let mut resources = Resources::default();
    for owner in 1..=4 {
        resources
            .create(ModId(owner), Allocation::Buffer(268_435_456), || Ok(()))
            .unwrap();
    }
    assert_eq!(resources.total_bytes(), 1_073_741_824);
    assert_eq!(
        resources.create(ModId(5), Allocation::Buffer(4), || panic!(
            "global budget exceeded"
        )),
        Err(Error::Memory)
    );
    resources.remove_owner(ModId(2));
    resources
        .create(ModId(5), Allocation::Buffer(268_435_456), || Ok(()))
        .unwrap();
    assert_eq!(
        resources.create(ModId(5), Allocation::Sampler, || Ok(())),
        Err(Error::Memory)
    );
}

#[test]
fn handles_are_bounded_and_reused_after_numeric_wrap() {
    let mut resources = Resources::default();
    for _ in 0..4096 {
        resources
            .create(ModId(1), Allocation::Sampler, || Ok(()))
            .unwrap();
    }
    assert_eq!(
        resources.create(ModId(1), Allocation::Sampler, || panic!(
            "handle budget exceeded"
        )),
        Err(Error::Handles)
    );
    resources.remove_owner(ModId(1));
    resources.ledger.lock().unwrap().next_handle = u64::from(u32::MAX) - 1;
    assert_eq!(
        resources.create(ModId(2), Allocation::Sampler, || Ok(())),
        Ok(u32::MAX)
    );
    assert_eq!(
        resources.create(ModId(2), Allocation::Sampler, || Ok(())),
        Ok(1)
    );
    assert_eq!(
        resources.usage(ModId(2)),
        Usage {
            bytes: 131_072,
            handles: 2
        }
    );
}

#[test]
fn allocation_sizes_include_mips_layers_and_format() {
    assert_eq!(Allocation::Buffer(268_435_456).bytes(), Ok(268_435_456));
    for size in [0, 268_435_457, u64::MAX] {
        assert_eq!(Allocation::Buffer(size).bytes(), Err(Error::BufferSize));
    }
    assert_eq!(Allocation::Buffer(65_537).bytes(), Ok(131_072));
    let mut description = texture();
    description.depth_or_layers = 3;
    description.mips = 3;
    assert_eq!(description.bytes(), Ok(132));
    description.is_3d = true;
    description.depth_or_layers = 4;
    assert_eq!(description.bytes(), Ok(148));
    description.format = Format::Rgba32Float;
    assert_eq!(description.bytes(), Ok(592));
    for (format, expected) in [
        (Format::Rgba8Unorm, 32),
        (Format::Rgba8Srgb, 32),
        (Format::Rgba16Float, 64),
        (Format::Rgba32Float, 128),
        (Format::R32Float, 32),
        (Format::Rg16Float, 32),
        (Format::Depth32Float, 32),
    ] {
        assert_eq!(
            Texture {
                format,
                ..texture()
            }
            .bytes(),
            Ok(expected)
        );
    }
    for allocation in [
        Allocation::Sampler,
        Allocation::RenderPipeline,
        Allocation::ComputePipeline,
        Allocation::BindGroup,
        Allocation::Shader(1),
    ] {
        assert_eq!(allocation.bytes(), Ok(65_536));
    }
    assert_eq!(Allocation::Shader(262_144).bytes(), Ok(262_144));
    assert_eq!(Allocation::Shader(262_145).bytes(), Err(Error::Descriptor));
}

#[test]
fn texture_dimensions_and_mip_counts_are_bounded() {
    let maximum = Texture {
        width: 8192,
        height: 8192,
        ..texture()
    };
    assert_eq!(maximum.bytes(), Ok(268_435_456));
    for invalid in [
        Texture {
            width: 0,
            ..maximum
        },
        Texture {
            height: 0,
            ..maximum
        },
        Texture {
            depth_or_layers: 0,
            ..maximum
        },
        Texture {
            width: 8193,
            ..maximum
        },
        Texture {
            height: 8193,
            ..maximum
        },
        Texture {
            depth_or_layers: 257,
            ..maximum
        },
    ] {
        assert_eq!(invalid.bytes(), Err(Error::TextureSize));
    }
    let volume = Texture {
        width: 512,
        height: 512,
        depth_or_layers: 512,
        is_3d: true,
        ..texture()
    };
    assert_eq!(volume.bytes(), Ok(536_870_912));
    for invalid in [
        Texture {
            width: 513,
            ..volume
        },
        Texture {
            height: 513,
            ..volume
        },
        Texture {
            depth_or_layers: 513,
            ..volume
        },
    ] {
        assert_eq!(invalid.bytes(), Err(Error::TextureSize));
    }
    for mips in [0, 15, u32::MAX] {
        assert_eq!(Texture { mips, ..maximum }.bytes(), Err(Error::Descriptor));
    }
    let mut resources = Resources::<()>::default();
    assert_eq!(
        resources.create(ModId(1), Allocation::Texture(volume), || panic!(
            "legal dimensions still require memory budget"
        )),
        Err(Error::Memory)
    );
}

#[test]
fn frame_limits_are_transactional_and_include_instances() {
    let empty = FrameUsage::default();
    let full = empty
        .charge(CommandKind::Draw {
            count: 4096,
            instances: 4096,
        })
        .unwrap();
    assert_eq!(full.vertices, 16_777_216);
    assert_eq!(
        full.charge(CommandKind::Draw {
            count: 1,
            instances: 1
        }),
        Err(Error::Vertices)
    );
    assert_eq!(
        empty.charge(CommandKind::Draw {
            count: u32::MAX,
            instances: u32::MAX
        }),
        Err(Error::Vertices)
    );
    let compute = empty.charge(CommandKind::Dispatch([256, 256, 1])).unwrap();
    assert_eq!(compute.workgroups, 65_536);
    assert_eq!(
        compute.charge(CommandKind::Dispatch([1, 1, 1])),
        Err(Error::Workgroups)
    );
    assert_eq!(
        empty.charge(CommandKind::Dispatch([u32::MAX; 3])),
        Err(Error::Workgroups)
    );
    let upload = empty.charge(CommandKind::Upload(16_777_216)).unwrap();
    assert_eq!(upload.charge(CommandKind::Upload(1)), Err(Error::Upload));
    assert_eq!(
        upload.charge(CommandKind::Upload(u64::MAX)),
        Err(Error::Upload)
    );
    let read = empty.charge(CommandKind::Readback(1_048_576)).unwrap();
    assert_eq!(read.charge(CommandKind::Readback(1)), Err(Error::Readback));
    assert_eq!(
        read.charge(CommandKind::Readback(u64::MAX)),
        Err(Error::Readback)
    );
    assert_eq!(empty, FrameUsage::default());
}

#[test]
fn raster_copy_and_compute_cost_limits_are_transactional() {
    let empty = FrameUsage::default();
    assert!(
        empty
            .charge(CommandKind::Draw {
                count: 1,
                instances: MAX_INSTANCES
            })
            .is_ok()
    );
    assert_eq!(
        empty.charge(CommandKind::Draw {
            count: 1,
            instances: MAX_INSTANCES + 1
        }),
        Err(Error::Vertices)
    );
    let render = |fragment_work, new_pass| CommandKind::Render {
        count: 3,
        instances: 1,
        fragment_work,
        pixel_limit: 4096,
        vertex_limit: MAX_VERTICES,
        new_pass,
    };
    let raster = empty.charge(render(4096 * 2048, true)).unwrap();
    assert_eq!(raster.charge(render(1, false)), Err(Error::Pixels));
    let mut merged = empty;
    for index in 0..128 {
        merged = merged.charge(render(1, index == 0)).unwrap();
    }
    assert_eq!(merged.passes, 1);
    let mut separate = empty;
    for _ in 0..MAX_PASSES {
        separate = separate.charge(render(1, true)).unwrap();
    }
    assert_eq!(separate.charge(render(1, true)), Err(Error::Passes));
    let copy = empty.charge(CommandKind::Copy(MAX_COPY_BYTES)).unwrap();
    assert_eq!(copy.charge(CommandKind::Copy(1)), Err(Error::Copy));
    assert_eq!(copy.charge(CommandKind::Copy(u64::MAX)), Err(Error::Copy));
    assert!(
        empty
            .charge(CommandKind::Compute {
                groups: [256, 1, 1],
                cost: 256,
                work_limit: MAX_WORKGROUPS,
            })
            .is_ok()
    );
    assert_eq!(
        empty.charge(CommandKind::Compute {
            groups: [257, 1, 1],
            cost: 256,
            work_limit: MAX_WORKGROUPS,
        }),
        Err(Error::Workgroups)
    );
    assert_eq!(empty, FrameUsage::default());
}

#[test]
fn record_only_in_frame_and_reject_before_constructing_commands() {
    let mut recorder = Recorder::default();
    assert_eq!(
        recorder.record(ModId(1), Stage::Hud, CommandKind::Upload(1), || ()),
        Err(Error::Frame)
    );
    assert_eq!(recorder.finish_frame().err(), Some(Error::Frame));
    recorder.begin_frame().unwrap();
    recorder
        .record(ModId(1), Stage::Hud, CommandKind::Upload(16_777_216), || ())
        .unwrap();
    assert_eq!(
        recorder.record(ModId(1), Stage::Hud, CommandKind::Upload(1), || panic!(
            "budget failure must precede command construction"
        )),
        Err(Error::Upload)
    );
    assert_eq!(recorder.begin_frame(), Ok(()));
    assert!(recorder.finish_frame().unwrap().is_empty());
    recorder.begin_frame().unwrap();
    recorder
        .record(ModId(1), Stage::Hud, CommandKind::Upload(16_777_216), || ())
        .unwrap();
    assert_eq!(recorder.finish_frame().unwrap().len(), 1);
}

#[test]
fn draw_and_command_counts_are_bounded_independently_per_mod() {
    let mut recorder = Recorder::default();
    recorder.begin_frame().unwrap();
    for _ in 0..4096 {
        recorder
            .record(
                ModId(1),
                Stage::WorldOpaque,
                CommandKind::Draw {
                    count: 1,
                    instances: 1,
                },
                || (),
            )
            .unwrap();
        recorder
            .record(ModId(2), Stage::AfterWorld, CommandKind::Upload(0), || ())
            .unwrap();
    }
    assert_eq!(
        recorder.record(
            ModId(1),
            Stage::Hud,
            CommandKind::Draw {
                count: 1,
                instances: 1
            },
            || ()
        ),
        Err(Error::Draws)
    );
    assert_eq!(
        recorder.record(ModId(2), Stage::Hud, CommandKind::Upload(0), || ()),
        Err(Error::Commands)
    );
    assert_eq!(recorder.finish_frame().unwrap().len(), 8192);
}

#[test]
fn commands_follow_stage_then_mod_load_then_submission_order() {
    let mut recorder = Recorder::default();
    recorder.begin_frame().unwrap();
    for (owner, stage, label) in [
        (2, Stage::Hud, "hud"),
        (2, Stage::WorldOpaque, "second mod"),
        (1, Stage::WorldOpaque, "first"),
        (1, Stage::WorldOpaque, "next"),
        (1, Stage::AfterWorld, "effect"),
        (1, Stage::WorldTranslucent, "particles"),
    ] {
        recorder
            .record(
                ModId(owner),
                stage,
                CommandKind::Draw {
                    count: 1,
                    instances: 1,
                },
                || label,
            )
            .unwrap();
    }
    let labels: Vec<_> = recorder
        .finish_frame()
        .unwrap()
        .into_iter()
        .map(|recorded| recorded.command)
        .collect();
    assert_eq!(
        labels,
        ["first", "next", "second mod", "particles", "effect", "hud"]
    );
}

#[test]
fn engine_resources_are_read_only_and_screen_inputs_are_stage_restricted() {
    for stage in [
        Stage::WorldOpaque,
        Stage::WorldTranslucent,
        Stage::AfterWorld,
        Stage::Hud,
    ] {
        for resource in [
            EngineResource::Frame,
            EngineResource::BlockAtlas,
            EngineResource::BlockAtlasSampler,
            EngineResource::WorldDepth,
            EngineResource::SceneColor,
            EngineResource::Lightmap,
        ] {
            assert_eq!(
                engine_binding(stage, resource, true),
                Err(Error::EngineBinding)
            );
        }
        for resource in [EngineResource::WorldDepth, EngineResource::SceneColor] {
            assert_eq!(
                engine_binding(stage, resource, false),
                if stage == Stage::AfterWorld {
                    Ok(())
                } else {
                    Err(Error::EngineBinding)
                }
            );
        }
        assert_eq!(engine_binding(stage, EngineResource::Frame, false), Ok(()));
        assert_eq!(
            engine_binding(stage, EngineResource::BlockAtlas, false),
            Ok(())
        );
    }
}

#[test]
fn pending_readback_storage_is_bounded_and_retained() {
    let mut resources = Resources::default();
    for size in [0, 1_048_577, u64::MAX] {
        assert_eq!(
            resources.create(ModId(1), Allocation::Readback(size), || Ok(())),
            Err(Error::Readback)
        );
    }
    let handle = resources
        .create(ModId(1), Allocation::Readback(1_048_576), || Ok(()))
        .unwrap();
    let pending = resources.get(ModId(1), handle).unwrap();
    resources.destroy(ModId(1), handle).unwrap();
    assert_eq!(
        resources.usage(ModId(1)),
        Usage {
            bytes: 2_097_152,
            handles: 1
        }
    );
    drop(pending);
    assert_eq!(resources.total_bytes(), 0);
}

#[test]
fn idle_recorder_does_not_allocate_or_emit_commands() {
    let mut recorder = Recorder::<()>::default();
    for _ in 0..3 {
        recorder.begin_frame().unwrap();
        let commands = recorder.finish_frame().unwrap();
        assert!(commands.is_empty());
        assert_eq!(commands.capacity(), 0);
        assert!(recorder.usage.is_empty());
        assert_eq!(recorder.commands.capacity(), 0);
    }
}

#[test]
fn guarded_compute_dispatch_reads_back_exact_iteration_bound() {
    use std::borrow::Cow;

    use super::super::native_services::checked;

    let Some((device, queue)) = super::super::native_services_tests::gpu() else {
        return;
    };
    for (source, expected) in [
        (
            "@group(0) @binding(0) var<storage, read_write> output: array<u32>; @compute @workgroup_size(1) fn main() { loop { output[0] += 1u; } }",
            super::super::mod_gpu_shader::LOOP_ITERATIONS,
        ),
        (
            "@group(0) @binding(0) var<storage, read_write> output: array<u32>; var<private> counter: u32; fn helper() { loop { counter += 1u; continue; } } @compute @workgroup_size(1) fn main() { helper(); helper(); output[0] = counter; }",
            super::super::mod_gpu_shader::LOOP_ITERATIONS * 2,
        ),
        (
            "@group(0) @binding(0) var<storage, read_write> output: array<u32>; @compute @workgroup_size(1) fn main() { for (var index = 0u; index < 3u; index++) { output[0] += 1u; } }",
            3,
        ),
    ] {
        let shader = super::super::mod_gpu_shader::validate(source).unwrap();
        let (pipeline, output, readback, group) = checked(&device, || {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Sandbox loop guard compute test"),
                source: wgpu::ShaderSource::Naga(Cow::Owned(shader.into_module())),
            });
            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: None,
                layout: None,
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            let output = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 4,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 4,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: output.as_entire_binding(),
                }],
            });
            (pipeline, output, readback, group)
        })
        .unwrap();
        let commands = checked(&device, || {
            let mut encoder = device.create_command_encoder(&Default::default());
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                pass.set_pipeline(&pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }
            encoder.copy_buffer_to_buffer(&output, 0, &readback, 0, 4);
            encoder.finish()
        })
        .unwrap();
        queue.submit([commands]);
        readback
            .slice(..)
            .map_async(wgpu::MapMode::Read, |result| result.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let mapped = readback.slice(..).get_mapped_range().unwrap();
        assert_eq!(
            u32::from_le_bytes(mapped[..4].try_into().unwrap()),
            expected,
            "{source}"
        );
    }
}
