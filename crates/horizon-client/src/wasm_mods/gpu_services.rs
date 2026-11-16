use super::GameHost;
use crate::gfx::{mod_gpu as gpu, mod_gpu_resources as resources, mod_gpu_runtime as runtime};
use yoake_wasm::gpu_types as types;

pub(super) fn owner(id: &str) -> Option<gpu::ModId> {
    super::client_services::identity(id)
        .map(|(_, index)| gpu::ModId((usize::MAX - index + 1) as u64))
}

fn stage(value: types::Stage) -> gpu::Stage {
    match value {
        types::Stage::WorldOpaque => gpu::Stage::WorldOpaque,
        types::Stage::WorldTranslucent => gpu::Stage::WorldTranslucent,
        types::Stage::AfterWorld => gpu::Stage::AfterWorld,
        types::Stage::Hud => gpu::Stage::Hud,
    }
}

fn format(value: types::PixelFormat) -> gpu::Format {
    match value {
        types::PixelFormat::Rgba8 => gpu::Format::Rgba8,
        types::PixelFormat::Rgba8Srgb => gpu::Format::Rgba8Srgb,
        types::PixelFormat::Bgra8 => gpu::Format::Bgra8,
        types::PixelFormat::Bgra8Srgb => gpu::Format::Bgra8Srgb,
        types::PixelFormat::Rgba16f => gpu::Format::Rgba16Float,
        types::PixelFormat::R32f => gpu::Format::R32Float,
        types::PixelFormat::Depth32f => gpu::Format::Depth32Float,
    }
}

impl GameHost {
    fn gpu_call<R>(&self, operation: impl FnOnce(&mut resources::Gpu, &wgpu::Device, &wgpu::Queue, &runtime::Frame) -> Result<R, gpu::Error>) -> Result<R, String> {
        if !self.gpu_enabled {
            return Err("service.gpu permission is required".to_owned());
        }
        runtime::with(owner(&self.id).ok_or_else(|| "GPU identity limit reached".to_owned())?, operation)
    }
}

impl yoake_wasm::GpuServices for GameHost {
    fn buffer(&mut self, size: u64, usage: u32) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.create_buffer(device, owner, size, usage))
    }

    fn texture(&mut self, desc: types::TextureDesc) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.create_texture(device, owner, gpu::Texture {
            width: desc.width, height: desc.height, depth_or_layers: desc.depth, mips: desc.mips,
            is_3d: desc.volume, format: format(desc.format),
        }, desc.usage))
    }

    fn sampler(&mut self, linear: bool, repeat: bool) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.create_sampler(device, owner, linear, repeat))
    }

    fn shader(&mut self, source: String) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.create_shader(device, owner, &source))
    }

    fn compute(&mut self, entry: String, shader: u32) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.create_compute(device, owner, shader, &entry))
    }

    fn render(&mut self, desc: types::RenderDesc) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        let attributes = desc.attributes.into_iter().map(|attribute| Ok(resources::VertexAttribute {
            location: attribute.location, offset: attribute.offset,
            format: match attribute.components {
                1 => wgpu::VertexFormat::Float32, 2 => wgpu::VertexFormat::Float32x2,
                3 => wgpu::VertexFormat::Float32x3, 4 => wgpu::VertexFormat::Float32x4,
                _ => return Err("vertex components must be between one and four".to_owned()),
            },
        })).collect::<Result<Vec<_>, String>>()?;
        self.gpu_call(|gpu, device, _, _| gpu.create_render(device, owner, desc.shader, resources::RenderDescriptor {
            vertex_entry: desc.vertex_entry, fragment_entry: desc.fragment_entry, stride: desc.stride,
            attributes, color: format(desc.format), depth: desc.depth, depth_write: desc.depth_write,
            topology: if desc.lines { wgpu::PrimitiveTopology::LineList } else { wgpu::PrimitiveTopology::TriangleList },
            cull: desc.cull.then_some(wgpu::Face::Back), blend: desc.blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
        }))
    }

    fn bind_group(&mut self, bindings: Vec<types::Binding>, index: u32, pipeline: u32) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        let bindings: Vec<_> = bindings.into_iter().map(|binding| match binding.kind {
            types::BindingKind::Buffer => resources::Binding::Buffer { binding: binding.binding, handle: binding.handle, offset: binding.offset, size: binding.size },
            types::BindingKind::Texture => resources::Binding::Texture { binding: binding.binding, handle: binding.handle },
            types::BindingKind::Sampler => resources::Binding::Sampler { binding: binding.binding, handle: binding.handle },
        }).collect();
        self.gpu_call(|gpu, device, _, _| gpu.create_bind_group(device, owner, pipeline, index, bindings))
    }

    fn destroy(&mut self, handle: u32) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, _, _, _| gpu.destroy(owner, handle))
    }

    fn write_buffer(&mut self, bytes: Vec<u8>, handle: u32, offset: u64) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.write_buffer(device, owner, handle, offset, &bytes))
    }

    fn copy_buffer(&mut self, destination: u32, destination_offset: u64, size: u64, source: u32, source_offset: u64) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, _, _, _| gpu.copy_buffer(owner, source, source_offset, destination, destination_offset, size))
    }

    fn write_texture(&mut self, bytes: Vec<u8>, extent: (u32, u32, u32), handle: u32, mip: u32, origin: (u32, u32, u32)) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.write_texture(device, owner, handle, mip, [origin.0, origin.1, origin.2], [extent.0, extent.1, extent.2], &bytes))
    }

    fn dispatch(&mut self, groups: Vec<u32>, pipeline: u32, at: types::Stage, workgroups: (u32, u32, u32)) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, _, _, _| gpu.dispatch(owner, stage(at), pipeline, groups, [workgroups.0, workgroups.1, workgroups.2]))
    }

    fn draw(&mut self, desc: types::DrawDesc, at: types::Stage, target: types::DrawTarget) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, _, _, _| gpu.draw(owner, stage(at), resources::Draw {
            pipeline: desc.pipeline, groups: desc.groups, vertex: desc.vertex.map(|range| (range.handle, range.offset, range.size)),
            index: desc.index.map(|range| resources::Index { buffer: range.handle, offset: range.offset, size: range.size, u32_indices: desc.index32 }),
            count: desc.count, instances: desc.instances,
        }, match target.color {
            Some(color) => resources::Target::Texture { color, depth: target.depth, clear: target.clear.map(|color| [color.0, color.1, color.2, color.3]) },
            None if target.depth.is_none() && target.clear.is_none() => resources::Target::Frame,
            None => return Err(gpu::Error::Descriptor),
        }))
    }

    fn read(&mut self, handle: u32, offset: u64, size: u64, at: types::Stage) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.read_buffer(device, owner, stage(at), handle, offset, size))
    }

    fn poll(&mut self, ticket: u32) -> Result<Option<Vec<u8>>, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| gpu.poll_readback(device, owner, ticket).map(|value| match value {
            resources::Readback::Pending => None, resources::Readback::Ready(bytes) => Some(bytes),
        }))
    }

    fn frame(&mut self) -> Result<types::FrameInfo, String> {
        self.gpu_call(|_, _, _, frame| Ok(types::FrameInfo {
            version: runtime::FRAME_VERSION, index: frame.index, width: frame.width, height: frame.height,
            view_projection: frame.view_projection.to_vec(), camera: (frame.camera[0], frame.camera[1], frame.camera[2]), delta_seconds: frame.delta_seconds,
        }))
    }
}
