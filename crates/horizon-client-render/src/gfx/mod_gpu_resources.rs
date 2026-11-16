use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::mod_gpu::{Allocation, Error, Format, ModId, Resource, Resources, Texture, Usage};

mod commands;
mod engine;
mod pipelines;
mod timing;

pub use commands::{Draw, FrameTarget, Index, Readback, Rect, Region, Target};
pub use engine::{EngineFrame, EngineSources};
pub use pipelines::{Binding, RenderDescriptor, VertexAttribute};

#[derive(Clone)]
pub struct SubmissionHooks {
    pub before: Arc<dyn Fn(ModId) -> Result<(), String> + Send + Sync>,
    pub complete: Arc<dyn Fn(ModId) + Send + Sync>,
}

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
    Shader {
        module: wgpu::ShaderModule,
        engine_masks: BTreeMap<String, u32>,
        costs: BTreeMap<String, u64>,
    },
    Compute {
        pipeline: wgpu::ComputePipeline,
        engine_mask: u32,
        cost: u64,
        _shader: Owned,
    },
    Render {
        pipeline: wgpu::RenderPipeline,
        descriptor: RenderDescriptor,
        engine_mask: u32,
        _shader: Owned,
    },
    BindGroup {
        group: wgpu::BindGroup,
        pipeline: Owned,
        index: u32,
        _resources: Vec<Owned>,
    },
    Readback(commands::Ticket),
    Timing(timing::Ticket),
}

#[derive(Default)]
pub struct Gpu {
    resources: Resources<Value>,
    disabled: BTreeMap<ModId, String>,
    lost: bool,
    recorder: super::mod_gpu::Recorder<commands::Command>,
    pending: Vec<super::mod_gpu::Recorded<commands::Command>>,
    tickets: BTreeMap<ModId, std::collections::BTreeSet<u32>>,
    engine_uniform: Option<wgpu::Buffer>,
    timings: Vec<(ModId, Owned)>,
    budgets: BTreeMap<ModId, timing::Budget>,
    cpu_budgets: BTreeMap<ModId, timing::Budget>,
    timestamps: bool,
    frame_index: u64,
    submission_hooks: Option<SubmissionHooks>,
    creations: BTreeMap<ModId, CreationBudget>,
    frame_limits: FrameLimits,
}

struct CreationBudget {
    recent: std::collections::VecDeque<Instant>,
    frame: u64,
    count: u32,
}

impl CreationBudget {
    fn charge(&mut self, now: Instant, frame: u64) -> Result<(), Error> {
        while self
            .recent
            .front()
            .is_some_and(|time| now.duration_since(*time) >= Duration::from_secs(60))
        {
            self.recent.pop_front();
        }
        if self.frame != frame {
            self.frame = frame;
            self.count = 0;
        }
        if self.count >= 2 || self.recent.len() >= 8 {
            return Err(Error::CreationRate);
        }
        self.count += 1;
        self.recent.push_back(now);
        Ok(())
    }
}

struct FrameLimits {
    width: u32,
    height: u32,
    world: Option<Format>,
    hud: Option<Format>,
}

impl Default for FrameLimits {
    fn default() -> Self {
        Self {
            width: 1024,
            height: 1024,
            world: None,
            hud: None,
        }
    }
}

impl Gpu {
    pub fn configure_frame(
        &mut self,
        width: u32,
        height: u32,
        world: Option<Format>,
        hud: Option<Format>,
    ) {
        self.frame_limits = FrameLimits {
            width,
            height,
            world,
            hud,
        };
    }

    fn budget_divisor(&self, owner: ModId, timestamps: bool) -> u64 {
        if timestamps {
            self.budgets.get(&owner).map_or(16, timing::Budget::divisor)
        } else {
            4
        }
    }

    fn pixel_limit(&self, owner: ModId, timestamps: bool) -> u64 {
        u64::from(self.frame_limits.width)
            .saturating_mul(u64::from(self.frame_limits.height))
            .max(1)
            .saturating_mul(64)
            / self.budget_divisor(owner, timestamps)
    }

    fn vertex_limit(&self, owner: ModId, timestamps: bool) -> u64 {
        super::mod_gpu::MAX_VERTICES / self.budget_divisor(owner, timestamps)
    }

    fn compute_limit(&self, owner: ModId, timestamps: bool) -> u64 {
        (64 * 1024 * 1024 / self.budget_divisor(owner, timestamps)).max(256 * 8192)
    }

    fn creation(&mut self, owner: ModId) -> Result<(), Error> {
        let now = Instant::now();
        self.creations
            .entry(owner)
            .or_insert_with(|| CreationBudget {
                recent: Default::default(),
                frame: self.frame_index,
                count: 0,
            })
            .charge(now, self.frame_index)
    }

    pub fn suspended(&self) -> BTreeMap<ModId, String> {
        self.disabled.clone()
    }

    pub fn restore_suspended(&mut self, suspended: BTreeMap<ModId, String>) {
        self.disabled.extend(suspended);
    }

    pub fn reads_scene(&self) -> bool {
        self.pending
            .iter()
            .any(|recorded| recorded.command.reads_scene())
    }

    pub fn set_submission_hooks(&mut self, hooks: SubmissionHooks) {
        self.submission_hooks = Some(hooks);
    }

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
        self.timings.retain(|(identity, _)| *identity != owner);
        self.budgets.remove(&owner);
        self.cpu_budgets.remove(&owner);
        self.disabled.entry(owner).or_insert(reason);
    }

    pub fn device_lost(&mut self) {
        for owner in self.creations.keys() {
            self.disabled
                .entry(*owner)
                .or_insert_with(|| "GPU device lost".to_owned());
        }
        self.lost = true;
        self.recorder = Default::default();
        self.pending.clear();
        self.tickets.clear();
        self.timings.clear();
        self.budgets.clear();
        self.cpu_budgets.clear();
        self.resources = Default::default();
    }

    pub fn reset_resources(&mut self, owner: ModId) {
        self.recorder.remove_owner(owner);
        self.pending.retain(|command| command.owner != owner);
        self.tickets.remove(&owner);
        self.timings.retain(|(identity, _)| *identity != owner);
        self.resources.remove_owner(owner);
    }

    pub fn remove_owner(&mut self, owner: ModId) {
        self.reset_resources(owner);
        self.budgets.remove(&owner);
        self.cpu_budgets.remove(&owner);
        self.disabled.remove(&owner);
        self.creations.remove(&owner);
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
        self.creation(owner)?;
        self.resources
            .create(owner, Allocation::Shader(source.len()), || {
                let shader = super::mod_gpu_shader::validate(source).map_err(Error::Shader)?;
                let engine_masks = shader.engine_masks.clone();
                let costs = shader.costs.clone();
                checked(device, || Value::Shader {
                    module: device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some("Yoake guarded mod shader"),
                        source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(
                            shader.into_module(),
                        )),
                    }),
                    engine_masks,
                    costs,
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
        Format::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
        Format::Rgb10a2Unorm => wgpu::TextureFormat::Rgb10a2Unorm,
    }
}

pub fn frame_format(format: wgpu::TextureFormat) -> Option<Format> {
    match format.remove_srgb_suffix() {
        wgpu::TextureFormat::Rgba8Unorm => Some(Format::Rgba8Unorm),
        wgpu::TextureFormat::Bgra8Unorm => Some(Format::Bgra8Unorm),
        wgpu::TextureFormat::Rgb10a2Unorm => Some(Format::Rgb10a2Unorm),
        wgpu::TextureFormat::Rgba16Float => Some(Format::Rgba16Float),
        _ => None,
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod workload_tests;
