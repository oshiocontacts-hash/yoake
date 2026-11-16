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
            queue.write_buffer(uniform, 0, bytemuck::bytes_of(&sources.frame));
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
        Ok(result)
    }
}
