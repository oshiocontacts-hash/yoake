use super::*;
use crate::gfx::mod_gpu::Stage;

#[path = "../../../../yoake-wasm-sdk/examples/gpu-demo/src/workload.rs"]
mod workload;

const ENGINE_WGSL: &str = include_str!("../../../../yoake-wasm-sdk/src/engine.wgsl");

fn render_descriptor(stride: u64, attributes: Vec<VertexAttribute>) -> RenderDescriptor {
    RenderDescriptor {
        vertex_entry: "vertex".to_owned(),
        fragment_entry: "fragment".to_owned(),
        stride,
        attributes,
        color: Format::Rgba8Unorm,
        depth: true,
        depth_write: false,
        topology: wgpu::PrimitiveTopology::TriangleList,
        cull: None,
        blend: None,
    }
}

#[test]
fn shared_demo_mesh_and_compute_run_with_and_without_timestamps() {
    let instance = crate::gfx::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .expect("headless GPU adapter");
    let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
    println!(
        "gpu-demo adapter: {:?}; timestamps: {timestamps}",
        adapter.get_info()
    );
    for measured in [false, true] {
        if measured && !timestamps {
            continue;
        }
        let features = if measured {
            wgpu::Features::TIMESTAMP_QUERY
        } else {
            wgpu::Features::empty()
        };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features: features,
            ..Default::default()
        }))
        .unwrap();
        run_demo(&device, &queue, measured);
    }
}

fn run_demo(device: &wgpu::Device, queue: &wgpu::Queue, measured: bool) {
    let owner = ModId(81);
    let mut gpu = Gpu::default();
    gpu.configure_frame(256, 256, Some(Format::Rgba8Unorm), None);
    let mesh_shader = gpu
        .create_shader(
            device,
            owner,
            &format!("{ENGINE_WGSL}{}", workload::MESH_SHADER),
        )
        .unwrap();
    let mesh_pipeline = gpu
        .create_render(
            device,
            owner,
            mesh_shader,
            render_descriptor(
                workload::VERTEX_STRIDE,
                vec![
                    VertexAttribute {
                        location: 0,
                        offset: 0,
                        format: wgpu::VertexFormat::Float32x3,
                    },
                    VertexAttribute {
                        location: 1,
                        offset: 12,
                        format: wgpu::VertexFormat::Float32x3,
                    },
                    VertexAttribute {
                        location: 2,
                        offset: 24,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                ],
            ),
        )
        .unwrap();
    gpu.collect_timings(device);
    let compute_shader = gpu
        .create_shader(
            device,
            owner,
            &format!("{ENGINE_WGSL}{}", workload::COMPUTE_SHADER),
        )
        .unwrap();
    let compute_pipeline = gpu
        .create_compute(device, owner, compute_shader, "update")
        .unwrap();
    gpu.collect_timings(device);
    let particle_shader = gpu
        .create_shader(
            device,
            owner,
            &format!("{ENGINE_WGSL}{}", workload::PARTICLE_SHADER),
        )
        .unwrap();
    let particle_pipeline = gpu
        .create_render(
            device,
            owner,
            particle_shader,
            render_descriptor(0, Vec::new()),
        )
        .unwrap();
    let mesh_bytes = workload::mesh_vertices();
    let mesh = gpu
        .create_buffer(
            device,
            owner,
            mesh_bytes.len() as u64 * 4,
            (wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST).bits(),
        )
        .unwrap();
    let particle_bytes = workload::particle_positions();
    let particles = gpu
        .create_buffer(
            device,
            owner,
            particle_bytes.len() as u64 * 4,
            (wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC)
                .bits(),
        )
        .unwrap();
    let checker = gpu
        .create_texture(
            device,
            owner,
            Texture {
                width: 2,
                height: 2,
                depth_or_layers: 1,
                mips: 1,
                is_3d: false,
                format: Format::Rgba8Unorm,
            },
            (wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST).bits(),
        )
        .unwrap();
    let sampler = gpu.create_sampler(device, owner, false, true).unwrap();
    let mesh_group = gpu
        .create_bind_group(
            device,
            owner,
            mesh_pipeline,
            0,
            &[
                Binding::Texture {
                    binding: 0,
                    handle: checker,
                },
                Binding::Sampler {
                    binding: 1,
                    handle: sampler,
                },
            ],
        )
        .unwrap();
    let binding = Binding::Buffer {
        binding: 0,
        handle: particles,
        offset: 0,
        size: particle_bytes.len() as u64 * 4,
    };
    let compute_group = gpu
        .create_bind_group(device, owner, compute_pipeline, 0, &[binding.clone()])
        .unwrap();
    let particle_group = gpu
        .create_bind_group(device, owner, particle_pipeline, 0, &[binding])
        .unwrap();
    let color = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let color_view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let target = FrameTarget {
        color: &color_view,
        depth: Some(&depth_view),
        format: wgpu::TextureFormat::Rgba8Unorm,
    };
    let identity = glam::Mat4::IDENTITY.to_cols_array();
    let sources = EngineSources {
        frame: EngineFrame {
            view_projection: identity,
            inverse_view_projection: identity,
            viewport_delta: [256.0, 256.0, 1.0, 0.0],
            ..Default::default()
        },
        atlas: None,
        sampler: None,
        depth: None,
        scene: None,
        lightmap: None,
    };
    let mut final_ticket = None;
    for frame in 0..12 {
        gpu.collect_timings(device);
        gpu.begin_frame().unwrap();
        if frame == 0 {
            gpu.write_buffer(
                device,
                owner,
                Stage::AfterWorld,
                mesh,
                0,
                bytemuck::cast_slice(&mesh_bytes),
            )
            .unwrap();
            gpu.write_buffer(
                device,
                owner,
                Stage::AfterWorld,
                particles,
                0,
                bytemuck::cast_slice(&particle_bytes),
            )
            .unwrap();
            gpu.write_texture(
                device,
                owner,
                Stage::AfterWorld,
                checker,
                0,
                [0; 3],
                [2, 2, 1],
                &[255; 16],
            )
            .unwrap();
        }
        gpu.dispatch(
            owner,
            Stage::AfterWorld,
            compute_pipeline,
            &[compute_group],
            [1, 1, 1],
        )
        .unwrap();
        gpu.draw(
            owner,
            Stage::AfterWorld,
            Draw {
                pipeline: mesh_pipeline,
                groups: vec![mesh_group],
                vertex: Some((mesh, 0, mesh_bytes.len() as u64 * 4)),
                index: None,
                count: workload::MESH_VERTEX_COUNT,
                instances: 1,
            },
            Target::Frame,
        )
        .unwrap();
        gpu.draw_region(
            owner,
            Stage::AfterWorld,
            Draw {
                pipeline: particle_pipeline,
                groups: vec![particle_group],
                vertex: None,
                index: None,
                count: 6,
                instances: workload::PARTICLE_COUNT,
            },
            Target::Frame,
            Region {
                viewport: Some(Rect {
                    x: 0,
                    y: 0,
                    width: 32,
                    height: 64,
                }),
                scissor: None,
            },
        )
        .unwrap();
        assert_eq!(
            gpu.draw(
                owner,
                Stage::AfterWorld,
                Draw {
                    pipeline: particle_pipeline,
                    groups: vec![particle_group],
                    vertex: None,
                    index: None,
                    count: 3_000_000,
                    instances: 1
                },
                Target::Frame
            ),
            Ok(())
        );
        assert_eq!(
            gpu.dispatch(
                owner,
                Stage::AfterWorld,
                compute_pipeline,
                &[compute_group],
                [u32::MAX, 1, 1]
            ),
            Err(Error::Workgroups)
        );
        if frame == 11 {
            final_ticket = Some(
                gpu.read_buffer(device, owner, Stage::AfterWorld, particles, 0, 16)
                    .unwrap(),
            );
        }
        gpu.finish_frame().unwrap();
        assert_eq!(
            gpu.pending
                .iter()
                .filter(|command| matches!(
                    command.kind,
                    crate::gfx::mod_gpu::CommandKind::Render { .. }
                ))
                .count(),
            2
        );
        let mut encoder = device.create_command_encoder(&Default::default());
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        queue.submit([encoder.finish()]);
        let errors = gpu.submit_stage_with_engine(
            device,
            queue,
            Stage::AfterWorld,
            Some(&target),
            Some(&sources),
        );
        assert!(
            errors.is_empty(),
            "gpu-demo frame {frame}, measured={measured}: {errors:?}; disabled: {:?}",
            gpu.disabled_reason(owner)
        );
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
    gpu.collect_timings(device);
    assert!(gpu.disabled_reason(owner).is_none());
    assert_eq!(gpu.timestamps, measured);
    if !measured {
        assert_eq!(gpu.budget_divisor(owner, measured), 16);
    }
    let Readback::Ready(bytes) = gpu
        .poll_readback(device, owner, final_ticket.unwrap())
        .unwrap()
    else {
        panic!("completed particle readback")
    };
    let position: &[f32] = bytemuck::cast_slice(&bytes);
    assert!((position[1] - (-0.7 + 12.0 * 0.07)).abs() < 0.0001);
    let pixels = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: 256 * 256 * 4,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        color.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &pixels,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 4),
                rows_per_image: Some(256),
            },
        },
        wgpu::Extent3d {
            width: 256,
            height: 256,
            depth_or_array_layers: 1,
        },
    );
    queue.submit([encoder.finish()]);
    let (sender, receiver) = std::sync::mpsc::channel();
    pixels
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            sender.send(result).unwrap()
        });
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    receiver.recv().unwrap().unwrap();
    let mapped = pixels.slice(..).get_mapped_range().unwrap();
    let center = (128 * 256 + 128) * 4;
    assert!(
        mapped[center..center + 3]
            .iter()
            .all(|channel| *channel > 50)
    );
    drop(mapped);
    pixels.unmap();
    println!(
        "gpu-demo: 12 frames, 2048 mesh triangles + 64 compute particles; measured={measured}; divisor={}",
        gpu.budget_divisor(owner, measured)
    );
}
