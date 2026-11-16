use std::collections::BTreeMap;
use std::sync::Arc;

use super::mod_gpu::{Allocation, Error, Format, ModId, Resource, Resources, Texture, Usage};

mod commands;
mod pipelines;

pub use commands::{Draw, FrameTarget, Index, Readback, Target};
pub use pipelines::{Binding, RenderDescriptor, VertexAttribute};

type Owned = Arc<Resource<Value>>;

enum Value {
    Buffer {
        buffer: wgpu::Buffer,
        size: u64,
        usage: wgpu::BufferUsages,
    },
    Texture {
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        descriptor: Texture,
        usage: wgpu::TextureUsages,
    },
    Sampler(wgpu::Sampler),
    Shader(wgpu::ShaderModule),
    Compute {
        pipeline: wgpu::ComputePipeline,
        _shader: Owned,
    },
    Render {
        pipeline: wgpu::RenderPipeline,
        descriptor: RenderDescriptor,
        _shader: Owned,
    },
    BindGroup {
        group: wgpu::BindGroup,
        pipeline: Owned,
        index: u32,
        _resources: Vec<Owned>,
    },
    Readback(commands::Ticket),
}

#[derive(Default)]
pub struct Gpu {
    resources: Resources<Value>,
    disabled: BTreeMap<ModId, String>,
    lost: bool,
    recorder: super::mod_gpu::Recorder<commands::Command>,
    pending: Vec<super::mod_gpu::Recorded<commands::Command>>,
    tickets: BTreeMap<ModId, std::collections::BTreeSet<u32>>,
}

impl Gpu {
    fn available(&self, owner: ModId) -> Result<(), Error> {
        if self.lost || self.disabled.contains_key(&owner) {
            Err(Error::Disabled)
        } else {
            Ok(())
        }
    }

    pub fn usage(&self, owner: ModId) -> Usage {
        self.resources.usage(owner)
    }

    pub fn total_bytes(&self) -> u64 {
        self.resources.total_bytes()
    }

    pub fn disabled_reason(&self, owner: ModId) -> Option<&str> {
        if self.lost {
            Some("GPU device lost")
        } else {
            self.disabled.get(&owner).map(String::as_str)
        }
    }

    pub fn disable(&mut self, owner: ModId, reason: String) {
        self.resources.remove_owner(owner);
        self.recorder.remove_owner(owner);
        self.pending.retain(|command| command.owner != owner);
        self.tickets.remove(&owner);
        self.disabled.entry(owner).or_insert(reason);
    }

    pub fn device_lost(&mut self) {
        self.lost = true;
        self.recorder = Default::default();
        self.pending.clear();
        self.tickets.clear();
        self.resources = Default::default();
    }

    pub fn remove_owner(&mut self, owner: ModId) {
        self.recorder.remove_owner(owner);
        self.pending.retain(|command| command.owner != owner);
        self.tickets.remove(&owner);
        self.resources.remove_owner(owner);
        self.disabled.remove(&owner);
    }

    pub fn destroy(&mut self, owner: ModId, handle: u32) -> Result<(), Error> {
        self.available(owner)?;
        self.resources.destroy(owner, handle)?;
        if let Some(tickets) = self.tickets.get_mut(&owner) {
            tickets.remove(&handle);
        }
        Ok(())
    }

    pub fn create_buffer(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        size: u64,
        usage: u32,
    ) -> Result<u32, Error> {
        self.available(owner)?;
        let usage = wgpu::BufferUsages::from_bits(usage).ok_or(Error::Descriptor)?;
        let allowed = wgpu::BufferUsages::VERTEX
            | wgpu::BufferUsages::INDEX
            | wgpu::BufferUsages::UNIFORM
            | wgpu::BufferUsages::STORAGE
            | wgpu::BufferUsages::COPY_SRC
            | wgpu::BufferUsages::COPY_DST;
        if usage.is_empty() || !allowed.contains(usage) || size % 4 != 0 {
            return Err(Error::Descriptor);
        }
        self.resources.create(owner, Allocation::Buffer(size), || {
            checked(device, || Value::Buffer {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Yoake mod buffer"),
                    size,
                    usage,
                    mapped_at_creation: false,
                }),
                size,
                usage,
            })
        })
    }

    pub fn create_texture(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        descriptor: Texture,
        usage: u32,
    ) -> Result<u32, Error> {
        self.available(owner)?;
        let usage = wgpu::TextureUsages::from_bits(usage).ok_or(Error::Descriptor)?;
        let allowed = wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::STORAGE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST;
        if usage.is_empty() || !allowed.contains(usage) {
            return Err(Error::Descriptor);
        }
        self.resources
            .create(owner, Allocation::Texture(descriptor), || {
                checked(device, || {
                    let texture = device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("Yoake mod texture"),
                        size: wgpu::Extent3d {
                            width: descriptor.width,
                            height: descriptor.height,
                            depth_or_array_layers: descriptor.depth_or_layers,
                        },
                        mip_level_count: descriptor.mips,
                        sample_count: 1,
                        dimension: if descriptor.is_3d {
                            wgpu::TextureDimension::D3
                        } else {
                            wgpu::TextureDimension::D2
                        },
                        format: texture_format(descriptor.format),
                        usage,
                        view_formats: &[],
                    });
                    let view = texture.create_view(&Default::default());
                    Value::Texture {
                        texture,
                        view,
                        descriptor,
                        usage,
                    }
                })
            })
    }

    pub fn create_sampler(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        linear: bool,
        repeat: bool,
    ) -> Result<u32, Error> {
        self.available(owner)?;
        let filter = if linear {
            wgpu::FilterMode::Linear
        } else {
            wgpu::FilterMode::Nearest
        };
        let address = if repeat {
            wgpu::AddressMode::Repeat
        } else {
            wgpu::AddressMode::ClampToEdge
        };
        self.resources.create(owner, Allocation::Sampler, || {
            checked(device, || {
                Value::Sampler(device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("Yoake mod sampler"),
                    address_mode_u: address,
                    address_mode_v: address,
                    address_mode_w: address,
                    mag_filter: filter,
                    min_filter: filter,
                    mipmap_filter: if linear {
                        wgpu::MipmapFilterMode::Linear
                    } else {
                        wgpu::MipmapFilterMode::Nearest
                    },
                    ..Default::default()
                }))
            })
        })
    }

    pub fn create_shader(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        source: &str,
    ) -> Result<u32, Error> {
        self.available(owner)?;
        self.resources
            .create(owner, Allocation::Shader(source.len()), || {
                let shader = super::mod_gpu_shader::validate(source).map_err(|error| {
                    Error::Host(format!(
                        "{}:{}: {}",
                        error.line, error.column, error.message
                    ))
                })?;
                checked(device, || {
                    Value::Shader(device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("Yoake guarded mod shader"),
                        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(
                            shader.into_module(),
                        )),
                    }))
                })
            })
    }
}

fn checked<Value>(device: &wgpu::Device, create: impl FnOnce() -> Value) -> Result<Value, Error> {
    super::native_services::checked(device, create).map_err(Error::Host)
}

pub fn texture_format(format: Format) -> wgpu::TextureFormat {
    match format {
        Format::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        Format::Rgba8Srgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        Format::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        Format::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        Format::R32Float => wgpu::TextureFormat::R32Float,
        Format::Rg16Float => wgpu::TextureFormat::Rg16Float,
        Format::Depth32Float => wgpu::TextureFormat::Depth32Float,
    }
}

#[cfg(test)]
mod tests;
