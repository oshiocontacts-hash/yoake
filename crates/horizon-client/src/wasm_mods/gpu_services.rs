use super::GameHost;
use crate::gfx::{mod_gpu as gpu, mod_gpu_resources as resources, mod_gpu_runtime as runtime};
use yoake_wasm::gpu_types as types;

pub(super) fn owner(id: &str) -> Option<gpu::ModId> {
    super::client_services::identity(id)
        .map(|(_, index)| gpu::ModId((1_u64 << 32) + (usize::MAX - index) as u64))
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
        types::PixelFormat::Rgba8 => gpu::Format::Rgba8Unorm,
        types::PixelFormat::Rgba8Srgb => gpu::Format::Rgba8Srgb,
        types::PixelFormat::Rgba16f => gpu::Format::Rgba16Float,
        types::PixelFormat::Rgba32f => gpu::Format::Rgba32Float,
        types::PixelFormat::R32f => gpu::Format::R32Float,
        types::PixelFormat::Rg16f => gpu::Format::Rg16Float,
        types::PixelFormat::Depth32f => gpu::Format::Depth32Float,
        types::PixelFormat::Bgra8 => gpu::Format::Bgra8Unorm,
        types::PixelFormat::Rgb10a2 => gpu::Format::Rgb10a2Unorm,
    }
}

fn pixel_format(value: gpu::Format) -> types::PixelFormat {
    match value {
        gpu::Format::Rgba8Unorm => types::PixelFormat::Rgba8,
        gpu::Format::Rgba8Srgb => types::PixelFormat::Rgba8Srgb,
        gpu::Format::Rgba16Float => types::PixelFormat::Rgba16f,
        gpu::Format::Rgba32Float => types::PixelFormat::Rgba32f,
        gpu::Format::R32Float => types::PixelFormat::R32f,
        gpu::Format::Rg16Float => types::PixelFormat::Rg16f,
        gpu::Format::Depth32Float => types::PixelFormat::Depth32f,
        gpu::Format::Bgra8Unorm => types::PixelFormat::Bgra8,
        gpu::Format::Rgb10a2Unorm => types::PixelFormat::Rgb10a2,
    }
}

fn shader_error(error: gpu::Error) -> types::ShaderError {
    match error {
        gpu::Error::Shader(error) => types::ShaderError {
            message: error.message,
            line: error.line,
            column: error.column,
        },
        other => types::ShaderError {
            message: other.to_string(),
            line: 0,
            column: 0,
        },
    }
}

impl GameHost {
    fn gpu_call<R>(
        &self,
        operation: impl FnOnce(
            &mut resources::Gpu,
            &wgpu::Device,
            &wgpu::Queue,
            &runtime::Frame,
        ) -> Result<R, gpu::Error>,
    ) -> Result<R, String> {
        self.gpu_call_error(operation)
            .map_err(|error| error.to_string())
    }

    fn gpu_call_error<R>(
        &self,
        operation: impl FnOnce(
            &mut resources::Gpu,
            &wgpu::Device,
            &wgpu::Queue,
            &runtime::Frame,
        ) -> Result<R, gpu::Error>,
    ) -> Result<R, gpu::Error> {
        self.gpu_call_timed(false, operation)
    }

    fn gpu_creation<R>(
        &self,
        operation: impl FnOnce(
            &mut resources::Gpu,
            &wgpu::Device,
            &wgpu::Queue,
            &runtime::Frame,
        ) -> Result<R, gpu::Error>,
    ) -> Result<R, String> {
        self.gpu_call_timed(true, operation)
            .map_err(|error| error.to_string())
    }

    fn gpu_call_timed<R>(
        &self,
        creation: bool,
        operation: impl FnOnce(
            &mut resources::Gpu,
            &wgpu::Device,
            &wgpu::Queue,
            &runtime::Frame,
        ) -> Result<R, gpu::Error>,
    ) -> Result<R, gpu::Error> {
        if !self.gpu_enabled {
            return Err(gpu::Error::Host(
                "service.gpu permission is required".to_owned(),
            ));
        }
        if !self.gpu_ready.load(std::sync::atomic::Ordering::Acquire) {
            return Err(gpu::Error::Host(
                "GPU services are unavailable during module initialization".to_owned(),
            ));
        }
        let owner = owner(&self.id)
            .ok_or_else(|| gpu::Error::Host("GPU identity limit reached".to_owned()))?;
        if creation {
            runtime::with_creation_error(owner, operation)
        } else {
            runtime::with_error(owner, operation)
        }
    }
}

impl yoake_wasm::GpuServices for GameHost {
    fn buffer(&mut self, size: u64, usage: u32) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_creation(|gpu, device, _, _| gpu.create_buffer(device, owner, size, usage))
    }

    fn texture(&mut self, desc: types::TextureDesc) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_creation(|gpu, device, _, _| {
            gpu.create_texture(
                device,
                owner,
                gpu::Texture {
                    width: desc.width,
                    height: desc.height,
                    depth_or_layers: desc.depth,
                    mips: desc.mips,
                    is_3d: desc.volume,
                    format: format(desc.format),
                },
                desc.usage,
            )
        })
    }

    fn sampler(&mut self, linear: bool, repeat: bool) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_creation(|gpu, device, _, _| gpu.create_sampler(device, owner, linear, repeat))
    }

    fn shader(&mut self, source: String) -> Result<u32, types::ShaderError> {
        self.gpu_call_timed(true, |gpu, device, _, _| {
            let owner = owner(&self.id)
                .ok_or_else(|| gpu::Error::Host("GPU identity limit reached".to_owned()))?;
            gpu.create_shader(device, owner, &source)
        })
        .map_err(shader_error)
    }

    fn shader_file(&mut self, path: String, source: String) -> Result<u32, types::ShaderError> {
        self.gpu_call_error(|_, _, _, _| Ok(()))
            .map_err(shader_error)?;
        let source = match &self.dev_shaders {
            Some(shaders) => shaders
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .source(&path, &source)
                .map_err(|error| shader_error(gpu::Error::Shader(error)))?,
            None => source,
        };
        self.shader(source)
    }

    fn compute(&mut self, entry: String, shader: u32) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_creation(|gpu, device, _, _| gpu.create_compute(device, owner, shader, &entry))
    }

    fn render(&mut self, desc: types::RenderDesc) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        let attributes = desc
            .attributes
            .into_iter()
            .map(|attribute| {
                Ok(resources::VertexAttribute {
                    location: attribute.location,
                    offset: attribute.offset,
                    format: match attribute.components {
                        1 => wgpu::VertexFormat::Float32,
                        2 => wgpu::VertexFormat::Float32x2,
                        3 => wgpu::VertexFormat::Float32x3,
                        4 => wgpu::VertexFormat::Float32x4,
                        _ => {
                            return Err("vertex components must be between one and four".to_owned());
                        }
                    },
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        self.gpu_creation(|gpu, device, _, _| {
            gpu.create_render(
                device,
                owner,
                desc.shader,
                resources::RenderDescriptor {
                    vertex_entry: desc.vertex_entry,
                    fragment_entry: desc.fragment_entry,
                    stride: desc.stride,
                    attributes,
                    color: format(desc.format),
                    depth: desc.depth,
                    depth_write: desc.depth_write,
                    topology: if desc.lines {
                        wgpu::PrimitiveTopology::LineList
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    cull: desc.cull.then_some(wgpu::Face::Back),
                    blend: desc.blend.then_some(wgpu::BlendState::ALPHA_BLENDING),
                },
            )
        })
    }

    fn bind_group(
        &mut self,
        bindings: Vec<types::Binding>,
        index: u32,
        pipeline: u32,
    ) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        let bindings: Vec<_> = bindings
            .into_iter()
            .map(|binding| match binding.kind {
                types::BindingKind::Buffer => resources::Binding::Buffer {
                    binding: binding.binding,
                    handle: binding.handle,
                    offset: binding.offset,
                    size: binding.size,
                },
                types::BindingKind::Texture => resources::Binding::Texture {
                    binding: binding.binding,
                    handle: binding.handle,
                },
                types::BindingKind::Sampler => resources::Binding::Sampler {
                    binding: binding.binding,
                    handle: binding.handle,
                },
            })
            .collect();
        self.gpu_creation(|gpu, device, _, _| {
            gpu.create_bind_group(device, owner, pipeline, index, &bindings)
        })
    }

    fn destroy(&mut self, handle: u32) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, _, _, _| gpu.destroy(owner, handle))
    }

    fn write_buffer(
        &mut self,
        bytes: Vec<u8>,
        handle: u32,
        offset: u64,
        at: types::Stage,
    ) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| {
            gpu.write_buffer(device, owner, stage(at), handle, offset, &bytes)
        })
    }

    fn copy_buffer(
        &mut self,
        destination: u32,
        destination_offset: u64,
        size: u64,
        source: u32,
        source_offset: u64,
        at: types::Stage,
    ) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, _, _, _| {
            gpu.copy_buffer(
                owner,
                stage(at),
                source,
                source_offset,
                destination,
                destination_offset,
                size,
            )
        })
    }

    fn write_texture(
        &mut self,
        bytes: Vec<u8>,
        extent: (u32, u32, u32),
        handle: u32,
        mip: u32,
        origin: (u32, u32, u32),
        at: types::Stage,
    ) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| {
            gpu.write_texture(
                device,
                owner,
                stage(at),
                handle,
                mip,
                [origin.0, origin.1, origin.2],
                [extent.0, extent.1, extent.2],
                &bytes,
            )
        })
    }

    fn dispatch(
        &mut self,
        groups: Vec<u32>,
        pipeline: u32,
        at: types::Stage,
        workgroups: (u32, u32, u32),
    ) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, _device, _, _| {
            #[cfg(test)]
            if let Some(fault) = self.gpu_recording_fault {
                fault(_device);
            }
            gpu.dispatch(
                owner,
                stage(at),
                pipeline,
                &groups,
                [workgroups.0, workgroups.1, workgroups.2],
            )
        })
    }

    fn draw(
        &mut self,
        desc: types::DrawDesc,
        at: types::Stage,
        target: types::DrawTarget,
    ) -> Result<(), String> {
        self.draw_region_call(
            desc,
            types::DrawRegion {
                viewport: None,
                scissor: None,
            },
            at,
            target,
        )
    }

    fn draw_region_call(
        &mut self,
        desc: types::DrawDesc,
        region: types::DrawRegion,
        at: types::Stage,
        target: types::DrawTarget,
    ) -> Result<(), String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, _, _, _| {
            gpu.draw_region(
                owner,
                stage(at),
                resources::Draw {
                    pipeline: desc.pipeline,
                    groups: desc.groups,
                    vertex: desc
                        .vertex
                        .map(|range| (range.handle, range.offset, range.size)),
                    index: desc.index.map(|range| resources::Index {
                        buffer: range.handle,
                        offset: range.offset,
                        size: range.size,
                        u32_indices: desc.index32,
                    }),
                    count: desc.count,
                    instances: desc.instances,
                },
                match target.color {
                    Some(color) => resources::Target::Texture {
                        color,
                        depth: target.depth,
                        clear: target
                            .clear
                            .map(|color| [color.0, color.1, color.2, color.3]),
                    },
                    None if target.depth.is_none() && target.clear.is_none() => {
                        resources::Target::Frame
                    }
                    None => return Err(gpu::Error::Descriptor),
                },
                resources::Region {
                    viewport: region.viewport.map(|rect| resources::Rect {
                        x: rect.x,
                        y: rect.y,
                        width: rect.width,
                        height: rect.height,
                    }),
                    scissor: region.scissor.map(|rect| resources::Rect {
                        x: rect.x,
                        y: rect.y,
                        width: rect.width,
                        height: rect.height,
                    }),
                },
            )
        })
    }

    fn read(
        &mut self,
        handle: u32,
        offset: u64,
        size: u64,
        at: types::Stage,
    ) -> Result<u32, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| {
            gpu.read_buffer(device, owner, stage(at), handle, offset, size)
        })
    }

    fn poll(&mut self, ticket: u32) -> Result<Option<Vec<u8>>, String> {
        let owner = owner(&self.id).ok_or("GPU identity limit reached")?;
        self.gpu_call(|gpu, device, _, _| {
            gpu.poll_readback(device, owner, ticket)
                .map(|value| match value {
                    resources::Readback::Pending => None,
                    resources::Readback::Ready(bytes) => Some(bytes),
                })
        })
    }

    fn frame(&mut self) -> Result<types::FrameInfo, String> {
        self.gpu_call(|_, _, _, frame| {
            Ok(types::FrameInfo {
                version: runtime::FRAME_VERSION,
                index: frame.index,
                width: frame.width,
                height: frame.height,
                view_projection: frame.view_projection.to_vec(),
                camera: (frame.camera[0], frame.camera[1], frame.camera[2]),
                delta_seconds: frame.delta_seconds,
                world_format: frame.world_format.map(pixel_format),
                hud_format: frame.hud_format.map(pixel_format),
            })
        })
    }
}
