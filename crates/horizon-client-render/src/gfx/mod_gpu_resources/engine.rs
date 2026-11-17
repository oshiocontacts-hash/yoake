use super::*;
use crate::gfx::mod_gpu::Stage;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct EngineFrame {
    pub header: [u32; 4],
    pub view: [f32; 16],
    pub projection: [f32; 16],
    pub view_projection: [f32; 16],
    pub inverse_view_projection: [f32; 16],
    pub previous_view_projection: [f32; 16],
    pub jittered_view_projection: [f32; 16],
    pub inverse_jittered_view_projection: [f32; 16],
    pub camera_time: [f32; 4],
    pub viewport_delta: [f32; 4],
    pub sun: [f32; 4],
    pub moon: [f32; 4],
    pub fog_color: [f32; 4],
    pub fog_range: [f32; 4],
    pub jitter: [f32; 4],
}

impl Default for EngineFrame {
    fn default() -> Self {
        bytemuck::Zeroable::zeroed()
    }
}

pub struct EngineSources<'a> {
    pub frame: EngineFrame,
    pub atlas: Option<&'a wgpu::TextureView>,
    pub sampler: Option<&'a wgpu::Sampler>,
    pub depth: Option<&'a wgpu::Texture>,
    pub scene: Option<&'a wgpu::Texture>,
    pub lightmap: Option<&'a wgpu::TextureView>,
}

#[derive(Default)]
pub(super) struct Snapshots {
    depth: Option<wgpu::TextureView>,
    scene: Option<wgpu::TextureView>,
    uniform_written: bool,
    pub(super) groups: BTreeMap<(usize, u8), Vec<(u32, wgpu::BindGroup)>>,
}

fn snapshot(
    device: &wgpu::Device,
    encoder: &mut wgpu::CommandEncoder,
    source: &wgpu::Texture,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Yoake read-only engine snapshot"),
        size: source.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: source.format(),
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let aspect = if source.format() == wgpu::TextureFormat::Depth32Float {
        wgpu::TextureAspect::DepthOnly
    } else {
        wgpu::TextureAspect::All
    };
    encoder.copy_texture_to_texture(
        wgpu::TexelCopyTextureInfo {
            texture: source,
            mip_level: 0,
            origin: Default::default(),
            aspect,
        },
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Default::default(),
            aspect,
        },
        source.size(),
    );
    texture.create_view(&Default::default())
}

impl Gpu {
    pub(super) fn engine_groups(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        command: &commands::Command,
        stage: Stage,
        sources: Option<&EngineSources<'_>>,
        snapshots: &mut Snapshots,
    ) -> Result<Vec<(u32, wgpu::BindGroup)>, Error> {
        let (pipeline, groups) = match command {
            commands::Command::Dispatch {
                pipeline, groups, ..
            }
            | commands::Command::Draw {
                pipeline, groups, ..
            } => (pipeline, groups),
            _ => return Ok(Vec::new()),
        };
        let supplied = groups.iter().fold(0_u8, |mask, group| match group.value() {
            Value::BindGroup { index, .. } => mask | (1 << index),
            _ => mask,
        });
        let key = (Arc::as_ptr(pipeline) as usize, supplied);
        if let Some(cached) = snapshots.groups.get(&key) {
            return Ok(cached.clone());
        }
        let (mask, layout) = match pipeline.value() {
            Value::Compute {
                pipeline,
                engine_mask,
                ..
            } if *engine_mask != 0 => (*engine_mask, pipeline.get_bind_group_layout(3)),
            Value::Render {
                pipeline,
                engine_mask,
                ..
            } if *engine_mask != 0 => (*engine_mask, pipeline.get_bind_group_layout(3)),
            _ => return Ok(Vec::new()),
        };
        if mask & (1 << 3) != 0 && !matches!(stage, Stage::AfterWorld | Stage::WorldTranslucent)
            || mask & (1 << 4) != 0 && stage != Stage::AfterWorld
        {
            return Err(Error::EngineBinding);
        }
        let sources = sources.ok_or(Error::EngineBinding)?;
        if mask & 1 != 0 {
            let uniform = self.engine_uniform.get_or_insert_with(|| {
                device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Yoake versioned engine frame"),
                    size: std::mem::size_of::<EngineFrame>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                })
            });
            if !snapshots.uniform_written {
                queue.write_buffer(uniform, 0, bytemuck::bytes_of(&sources.frame));
                snapshots.uniform_written = true;
            }
        }
        if mask & (1 << 3) != 0 && snapshots.depth.is_none() {
            snapshots.depth = Some(snapshot(
                device,
                encoder,
                sources.depth.ok_or(Error::EngineBinding)?,
            ));
        }
        if mask & (1 << 4) != 0 && snapshots.scene.is_none() {
            snapshots.scene = Some(snapshot(
                device,
                encoder,
                sources.scene.ok_or(Error::EngineBinding)?,
            ));
        }
        let mut entries = Vec::new();
        for binding in 0..6 {
            if mask & (1 << binding) == 0 {
                continue;
            }
            let resource = match binding {
                0 => self
                    .engine_uniform
                    .as_ref()
                    .ok_or(Error::EngineBinding)?
                    .as_entire_binding(),
                1 => wgpu::BindingResource::TextureView(sources.atlas.ok_or(Error::EngineBinding)?),
                2 => wgpu::BindingResource::Sampler(sources.sampler.ok_or(Error::EngineBinding)?),
                3 => wgpu::BindingResource::TextureView(
                    snapshots.depth.as_ref().ok_or(Error::EngineBinding)?,
                ),
                4 => wgpu::BindingResource::TextureView(
                    snapshots.scene.as_ref().ok_or(Error::EngineBinding)?,
                ),
                5 => wgpu::BindingResource::TextureView(
                    sources.lightmap.ok_or(Error::EngineBinding)?,
                ),
                _ => return Err(Error::EngineBinding),
            };
            entries.push(wgpu::BindGroupEntry { binding, resource });
        }
        let engine = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Yoake read-only engine group"),
            layout: &layout,
            entries: &entries,
        });
        let mut result = Vec::new();
        for index in 0..3 {
            if groups.iter().any(|group| matches!(group.value(), Value::BindGroup { index: existing, .. } if *existing == index)) {
                continue;
            }
            let layout = match pipeline.value() {
                Value::Compute { pipeline, .. } => pipeline.get_bind_group_layout(index),
                Value::Render { pipeline, .. } => pipeline.get_bind_group_layout(index),
                _ => return Err(Error::Kind),
            };
            result.push((
                index,
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Yoake empty engine group gap"),
                    layout: &layout,
                    entries: &[],
                }),
            ));
        }
        result.push((3, engine));
        snapshots.groups.insert(key, result.clone());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_scene_reads_skip_without_disabling_other_work() {
        let (device, queue, _guard) = super::super::tests::gpu();
        for case in 0..4 {
            unavailable_scene_read_case(&device, &queue, case);
        }
    }

    fn unavailable_scene_read_case(device: &wgpu::Device, queue: &wgpu::Queue, case: usize) {
        let mut gpu = Gpu::default();
        gpu.set_test_time_allowance(std::time::Duration::from_secs(5));
        let owner = ModId(42);
        let output = gpu
            .create_buffer(
                &device,
                owner,
                8,
                (wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC).bits(),
            )
            .unwrap();
        let shader = gpu
            .create_shader(
                &device,
                owner,
                r#"
@group(0) @binding(0) var<storage, read_write> output: array<u32>;
@group(3) @binding(4) var scene: texture_2d<f32>;
@compute @workgroup_size(1) fn read_scene() {
    output[0] = u32(textureLoad(scene, vec2<i32>(0), 0).r * 255.0);
}
@compute @workgroup_size(1) fn plain() {
    output[1] = output[1] + 1u;
}
"#,
            )
            .unwrap();
        let scene_pipeline = gpu
            .create_compute(&device, owner, shader, "read_scene")
            .unwrap();
        gpu.collect_timings(&device);
        let plain_pipeline = gpu.create_compute(&device, owner, shader, "plain").unwrap();
        let mut groups = Vec::new();
        for pipeline in [scene_pipeline, plain_pipeline] {
            groups.push(
                gpu.create_bind_group(
                    &device,
                    owner,
                    pipeline,
                    0,
                    &[Binding::Buffer {
                        binding: 0,
                        handle: output,
                        offset: 0,
                        size: 8,
                    }],
                )
                .unwrap(),
            );
        }
        let texture = |copyable| {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | if copyable {
                        wgpu::TextureUsages::COPY_SRC
                    } else {
                        wgpu::TextureUsages::empty()
                    },
                view_formats: &[],
            });
            queue.write_texture(
                texture.as_image_copy(),
                &[255, 0, 0, 255],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                texture.size(),
            );
            texture
        };
        let uncopyable = texture(false);
        let copyable = texture(true);
        let sources = |scene| EngineSources {
            frame: EngineFrame::default(),
            atlas: None,
            sampler: None,
            depth: None,
            scene,
            lightmap: None,
        };
        let missing = sources(None);
        let uncopyable = sources(Some(&uncopyable));
        let copyable = sources(Some(&copyable));
        let (sources, expected_scene) = [
            (None, 0),
            (Some(&missing), 0),
            (Some(&uncopyable), 0),
            (Some(&copyable), 255),
        ]
        .into_iter()
        .nth(case)
        .unwrap();
        gpu.collect_timings(&device);
        gpu.begin_frame().unwrap();
        for (pipeline, group) in [scene_pipeline, plain_pipeline].into_iter().zip(&groups) {
            gpu.dispatch(owner, Stage::AfterWorld, pipeline, &[*group], [1; 3])
                .unwrap();
        }
        let ticket = gpu
            .read_buffer(&device, owner, Stage::AfterWorld, output, 0, 8)
            .unwrap();
        gpu.finish_frame().unwrap();
        assert!(gpu.reads_scene());
        assert!(
            gpu.submit_stage_with_engine(&device, &queue, Stage::AfterWorld, None, sources,)
                .is_empty()
        );
        assert!(gpu.disabled_reason(owner).is_none());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let Readback::Ready(bytes) = gpu.poll_readback(&device, owner, ticket).unwrap() else {
            panic!("readback remained pending");
        };
        assert_eq!(
            bytemuck::cast_slice::<u8, u32>(&bytes),
            &[expected_scene, 1]
        );
    }

    #[test]
    fn engine_snapshots_are_shared_and_groups_are_cached_per_batch() {
        let (device, queue, _guard) = super::super::tests::gpu();
        let mut gpu = Gpu::default();
        gpu.set_test_time_allowance(std::time::Duration::from_secs(5));
        gpu.begin_frame().unwrap();
        for owner in [ModId(40), ModId(41)] {
            let output = gpu
                .create_buffer(&device, owner, 8, wgpu::BufferUsages::STORAGE.bits())
                .unwrap();
            let shader = gpu.create_shader(&device, owner, "@group(0) @binding(0) var<storage, read_write> output: array<u32>; @group(3) @binding(3) var depth: texture_depth_2d; @group(3) @binding(4) var scene: texture_2d<f32>; @compute @workgroup_size(1) fn main() { output[0] = u32(textureLoad(depth, vec2<i32>(0), 0) * 100.0); output[1] = u32(textureLoad(scene, vec2<i32>(0), 0).r * 255.0); }").unwrap();
            let pipeline = gpu.create_compute(&device, owner, shader, "main").unwrap();
            let group = gpu
                .create_bind_group(
                    &device,
                    owner,
                    pipeline,
                    0,
                    &[Binding::Buffer {
                        binding: 0,
                        handle: output,
                        offset: 0,
                        size: 8,
                    }],
                )
                .unwrap();
            assert_eq!(
                gpu.dispatch(owner, Stage::WorldOpaque, pipeline, &[group], [1; 3]),
                Err(Error::EngineBinding)
            );
            assert!(gpu.disabled_reason(owner).is_none());
            gpu.dispatch(owner, Stage::AfterWorld, pipeline, &[group], [1; 3])
                .unwrap();
        }
        gpu.finish_frame().unwrap();
        let texture = |format| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width: 2,
                    height: 2,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let depth = texture(wgpu::TextureFormat::Depth32Float);
        let scene = texture(wgpu::TextureFormat::Rgba8Unorm);
        let sources = EngineSources {
            frame: EngineFrame::default(),
            atlas: None,
            sampler: None,
            depth: Some(&depth),
            scene: Some(&scene),
            lightmap: None,
        };
        let mut encoder = device.create_command_encoder(&Default::default());
        let mut snapshots = Snapshots::default();
        let mut shared = None;
        for recorded in std::mem::take(&mut gpu.pending) {
            snapshots.groups.clear();
            let first = gpu
                .engine_groups(
                    &device,
                    &queue,
                    &mut encoder,
                    &recorded.command,
                    Stage::AfterWorld,
                    Some(&sources),
                    &mut snapshots,
                )
                .unwrap();
            let second = gpu
                .engine_groups(
                    &device,
                    &queue,
                    &mut encoder,
                    &recorded.command,
                    Stage::AfterWorld,
                    Some(&sources),
                    &mut snapshots,
                )
                .unwrap();
            assert_eq!(first, second);
            assert_eq!(snapshots.groups.len(), 1);
            let views = (
                snapshots.depth.clone().unwrap(),
                snapshots.scene.clone().unwrap(),
            );
            if let Some(shared) = &shared {
                assert_eq!(&views, shared);
            } else {
                shared = Some(views);
            }
        }
        assert!(shared.is_some());
        queue.submit([encoder.finish()]);
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    }
}
