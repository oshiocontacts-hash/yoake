use super::{Allocation, Error, Gpu, ModId, Owned, Value, checked, texture_format};
use crate::gfx::mod_gpu::Format;

#[derive(Clone, Debug)]
pub struct VertexAttribute {
    pub location: u32,
    pub offset: u64,
    pub format: wgpu::VertexFormat,
}

#[derive(Clone, Debug)]
pub struct RenderDescriptor {
    pub vertex_entry: String,
    pub fragment_entry: String,
    pub stride: u64,
    pub attributes: Vec<VertexAttribute>,
    pub color: Format,
    pub depth: bool,
    pub depth_write: bool,
    pub topology: wgpu::PrimitiveTopology,
    pub cull: Option<wgpu::Face>,
    pub blend: Option<wgpu::BlendState>,
}

#[derive(Clone, Debug)]
pub enum Binding {
    Buffer {
        binding: u32,
        handle: u32,
        offset: u64,
        size: u64,
    },
    Texture {
        binding: u32,
        handle: u32,
    },
    Sampler {
        binding: u32,
        handle: u32,
    },
}

impl Binding {
    fn number(&self) -> u32 {
        match self {
            Self::Buffer { binding, .. }
            | Self::Texture { binding, .. }
            | Self::Sampler { binding, .. } => *binding,
        }
    }

    fn handle(&self) -> u32 {
        match self {
            Self::Buffer { handle, .. }
            | Self::Texture { handle, .. }
            | Self::Sampler { handle, .. } => *handle,
        }
    }
}

impl Gpu {
    pub fn create_compute(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        shader: u32,
        entry: &str,
    ) -> Result<u32, Error> {
        self.available(owner)?;
        self.timestamps = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        self.creation(owner)?;
        let shader = self
            .resources
            .get_kind(owner, shader, Allocation::Shader(0))?;
        let Value::Shader {
            module,
            engine_masks,
            costs,
        } = shader.value()
        else {
            return Err(Error::Kind);
        };
        if entry.is_empty() || entry.len() > 128 {
            return Err(Error::Descriptor);
        }
        let cost = *costs.get(entry).ok_or(Error::Descriptor)?;
        self.resources
            .create(owner, Allocation::ComputePipeline, || {
                checked(device, || Value::Compute {
                    engine_mask: *engine_masks.get(entry).unwrap_or(&0),
                    cost,
                    pipeline: device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                        label: Some("Yoake mod compute pipeline"),
                        layout: None,
                        module,
                        entry_point: Some(entry),
                        compilation_options: Default::default(),
                        cache: None,
                    }),
                    _shader: shader.clone(),
                })
            })
    }

    pub fn create_render(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        shader: u32,
        descriptor: RenderDescriptor,
    ) -> Result<u32, Error> {
        self.available(owner)?;
        self.timestamps = device.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        self.creation(owner)?;
        if descriptor.attributes.len() > 16
            || descriptor.stride > 2048
            || descriptor.stride % 4 != 0
            || descriptor.color == Format::Depth32Float
            || descriptor.vertex_entry.is_empty()
            || descriptor.fragment_entry.is_empty()
            || descriptor.vertex_entry.len() > 128
            || descriptor.fragment_entry.len() > 128
            || descriptor.depth_write && !descriptor.depth
            || matches!(
                descriptor.topology,
                wgpu::PrimitiveTopology::LineStrip | wgpu::PrimitiveTopology::TriangleStrip
            )
        {
            return Err(Error::Descriptor);
        }
        let mut locations = std::collections::BTreeSet::new();
        for attribute in &descriptor.attributes {
            if !locations.insert(attribute.location)
                || attribute.location >= 16
                || attribute
                    .offset
                    .checked_add(attribute.format.size())
                    .is_none_or(|end| end > descriptor.stride)
            {
                return Err(Error::Descriptor);
            }
        }
        let shader = self
            .resources
            .get_kind(owner, shader, Allocation::Shader(0))?;
        let Value::Shader {
            module,
            engine_masks,
            ..
        } = shader.value()
        else {
            return Err(Error::Kind);
        };
        let attributes: Vec<_> = descriptor
            .attributes
            .iter()
            .map(|attribute| wgpu::VertexAttribute {
                format: attribute.format,
                offset: attribute.offset,
                shader_location: attribute.location,
            })
            .collect();
        let vertex_buffer = wgpu::VertexBufferLayout {
            array_stride: descriptor.stride,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        };
        let buffers = if descriptor.stride == 0 {
            vec![]
        } else {
            vec![Some(vertex_buffer)]
        };
        self.resources
            .create(owner, Allocation::RenderPipeline, || {
                checked(device, || Value::Render {
                    engine_mask: engine_masks
                        .get(&descriptor.vertex_entry)
                        .copied()
                        .unwrap_or(0)
                        | engine_masks
                            .get(&descriptor.fragment_entry)
                            .copied()
                            .unwrap_or(0),
                    pipeline: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("Yoake mod render pipeline"),
                        layout: None,
                        vertex: wgpu::VertexState {
                            module,
                            entry_point: Some(&descriptor.vertex_entry),
                            compilation_options: Default::default(),
                            buffers: &buffers,
                        },
                        fragment: Some(wgpu::FragmentState {
                            module,
                            entry_point: Some(&descriptor.fragment_entry),
                            compilation_options: Default::default(),
                            targets: &[Some(wgpu::ColorTargetState {
                                format: texture_format(descriptor.color),
                                blend: descriptor.blend,
                                write_mask: wgpu::ColorWrites::ALL,
                            })],
                        }),
                        primitive: wgpu::PrimitiveState {
                            topology: descriptor.topology,
                            cull_mode: descriptor.cull,
                            ..Default::default()
                        },
                        depth_stencil: descriptor.depth.then_some(wgpu::DepthStencilState {
                            format: wgpu::TextureFormat::Depth32Float,
                            depth_write_enabled: Some(descriptor.depth_write),
                            depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: Default::default(),
                        multiview_mask: None,
                        cache: None,
                    }),
                    descriptor: descriptor.clone(),
                    _shader: shader.clone(),
                })
            })
    }

    pub fn create_bind_group(
        &mut self,
        device: &wgpu::Device,
        owner: ModId,
        pipeline: u32,
        index: u32,
        bindings: &[Binding],
    ) -> Result<u32, Error> {
        self.available(owner)?;
        if index >= 3 || bindings.len() > 32 {
            return Err(Error::Descriptor);
        }
        let mut numbers = std::collections::BTreeSet::new();
        if bindings
            .iter()
            .any(|binding| !numbers.insert(binding.number()))
        {
            return Err(Error::Descriptor);
        }
        let pipeline = self.resources.get(owner, pipeline)?;
        let resources: Vec<Owned> = bindings
            .iter()
            .map(|binding| self.resources.get(owner, binding.handle()))
            .collect::<Result<_, _>>()?;
        let mut entries = Vec::with_capacity(bindings.len());
        for (binding, resource) in bindings.iter().zip(&resources) {
            let value = match (binding, resource.value()) {
                (
                    Binding::Buffer { offset, size, .. },
                    Value::Buffer {
                        buffer,
                        size: length,
                        usage,
                    },
                ) => {
                    if *size == 0
                        || offset.checked_add(*size).is_none_or(|end| end > *length)
                        || !usage
                            .intersects(wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::STORAGE)
                    {
                        return Err(Error::Descriptor);
                    }
                    wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer,
                        offset: *offset,
                        size: std::num::NonZeroU64::new(*size),
                    })
                }
                (Binding::Texture { .. }, Value::Texture { view, usage, .. }) => {
                    if !usage.intersects(
                        wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
                    ) {
                        return Err(Error::Descriptor);
                    }
                    wgpu::BindingResource::TextureView(view)
                }
                (Binding::Sampler { .. }, Value::Sampler(sampler)) => {
                    wgpu::BindingResource::Sampler(sampler)
                }
                _ => return Err(Error::Kind),
            };
            entries.push(wgpu::BindGroupEntry {
                binding: binding.number(),
                resource: value,
            });
        }
        self.resources.create(owner, Allocation::BindGroup, || {
            checked(device, || {
                let layout = match pipeline.value() {
                    Value::Compute { pipeline, .. } => pipeline.get_bind_group_layout(index),
                    Value::Render { pipeline, .. } => pipeline.get_bind_group_layout(index),
                    _ => return None,
                };
                Some(Value::BindGroup {
                    group: device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("Yoake mod bind group"),
                        layout: &layout,
                        entries: &entries,
                    }),
                    pipeline: pipeline.clone(),
                    index,
                    _resources: resources.clone(),
                })
            })?
            .ok_or(Error::Kind)
        })
    }
}
