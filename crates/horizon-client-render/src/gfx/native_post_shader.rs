use std::collections::BTreeMap;

use wgpu::util::DeviceExt;

use super::native_services::checked;

pub const MAX_POST_SHADERS: usize = 32;
pub const MAX_POST_SOURCE: usize = 65_536;
pub const MAX_POST_UNIFORMS: u32 = 4096;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativePostUniformLayout {
    pub size: u32,
}

fn loop_free(block: &naga::Block) -> bool {
    block.iter().all(|statement| match statement {
        naga::Statement::Loop { .. } => false,
        naga::Statement::Block(block) => loop_free(block),
        naga::Statement::If { accept, reject, .. } => loop_free(accept) && loop_free(reject),
        naga::Statement::Switch { cases, .. } => cases.iter().all(|case| loop_free(&case.body)),
        _ => true,
    })
}

fn validate(source: &str, layout: NativePostUniformLayout) -> Result<naga::Module, String> {
    if source.is_empty() || source.len() > MAX_POST_SOURCE {
        return Err("post shader source must be 1..=65536 bytes".into());
    }
    if layout.size > MAX_POST_UNIFORMS || !layout.size.is_multiple_of(16) {
        return Err("post uniforms size must be a multiple of 16, at most 4096 bytes".into());
    }
    let module =
        naga::front::wgsl::parse_str(source).map_err(|error| error.emit_to_string(source))?;
    naga::valid::Validator::new(
        naga::valid::ValidationFlags::all(),
        naga::valid::Capabilities::empty(),
    )
    .validate(&module)
    .map_err(|error| error.to_string())?;
    if module.entry_points.len() != 1
        || module.entry_points[0].stage != naga::ShaderStage::Fragment
        || module.entry_points[0].name != "fs_main"
    {
        return Err("post shader must have exactly one fragment entry named fs_main".into());
    }
    if module.functions.len() > 64
        || module
            .functions
            .iter()
            .any(|(_, function)| !loop_free(&function.body) || function.expressions.len() > 4096)
        || !loop_free(&module.entry_points[0].function.body)
        || module.entry_points[0].function.expressions.len() > 4096
    {
        return Err(
            "post shaders must be loop-free and within the function/expression limits".into(),
        );
    }
    let mut layouter = naga::proc::Layouter::default();
    layouter
        .update(module.to_ctx())
        .map_err(|error| error.to_string())?;
    let mut private_bytes = 0u64;
    for (_, variable) in module.global_variables.iter() {
        match variable.space {
            naga::AddressSpace::Private => {
                private_bytes += u64::from(layouter[variable.ty].size);
                if private_bytes > 16_384 {
                    return Err("post shader private memory exceeds 16384 bytes".into());
                }
            }
            naga::AddressSpace::Uniform | naga::AddressSpace::Handle => {
                let binding = variable
                    .binding
                    .as_ref()
                    .ok_or("post resources must have a binding")?;
                if binding.group != 0 {
                    return Err("post resources must be in group 0".into());
                }
                let valid = match binding.binding {
                    0 => {
                        variable.space == naga::AddressSpace::Handle
                            && matches!(
                                module.types[variable.ty].inner,
                                naga::TypeInner::Image {
                                    dim: naga::ImageDimension::D2,
                                    arrayed: false,
                                    class: naga::ImageClass::Sampled {
                                        kind: naga::ScalarKind::Float,
                                        multi: false
                                    }
                                }
                            )
                    }
                    1 => {
                        variable.space == naga::AddressSpace::Handle
                            && matches!(
                                module.types[variable.ty].inner,
                                naga::TypeInner::Sampler { comparison: false }
                            )
                    }
                    2 => {
                        variable.space == naga::AddressSpace::Uniform
                            && layout.size > 0
                            && layouter[variable.ty].size == layout.size
                    }
                    _ => false,
                };
                if !valid {
                    return Err("post binding must be 0: texture_2d<f32>, 1: sampler, or 2: matching uniform block".into());
                }
            }
            _ => {
                return Err(
                    "post shader storage, workgroup and other address spaces are forbidden".into(),
                );
            }
        }
    }
    for function in module
        .functions
        .iter()
        .map(|(_, function)| function)
        .chain(std::iter::once(&module.entry_points[0].function))
    {
        let bytes: u64 = function
            .local_variables
            .iter()
            .map(|(_, variable)| u64::from(layouter[variable.ty].size))
            .sum();
        if bytes > 16_384 {
            return Err("post shader function memory exceeds 16384 bytes".into());
        }
    }
    Ok(module)
}

struct Shader {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: Option<wgpu::Buffer>,
    size: u32,
    enabled: bool,
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    sampler: wgpu::Sampler,
}

#[derive(Default)]
pub struct PostShaders {
    shaders: BTreeMap<String, Shader>,
    target: Option<Target>,
}

const VERTEX: &str = "struct Output { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }; @vertex fn vs_main(@builtin(vertex_index) index: u32) -> Output { let point = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u)); var output: Output; output.position = vec4<f32>(point * 2.0 - 1.0, 0.0, 1.0); output.uv = vec2<f32>(point.x, 1.0 - point.y); return output; }";

impl PostShaders {
    pub fn active(&self) -> bool {
        self.shaders.values().any(|shader| shader.enabled)
    }

    pub fn register(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        id: &str,
        source: &str,
        uniforms_layout: NativePostUniformLayout,
    ) -> Result<(), String> {
        if id.is_empty()
            || id.len() > 256
            || self.shaders.contains_key(id)
            || self.shaders.len() >= MAX_POST_SHADERS
        {
            return Err("invalid, duplicate or excessive post shader id".into());
        }
        let module = validate(source, uniforms_layout)?;
        let shader = checked(device, || {
            let mut entries = vec![
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
            ];
            if uniforms_layout.size > 0 {
                entries.push(wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(u64::from(uniforms_layout.size)),
                    },
                    count: None,
                });
            }
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mod post layout"),
                entries: &entries,
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mod post pipeline layout"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let vertex = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mod post vertex"),
                source: wgpu::ShaderSource::Wgsl(VERTEX.into()),
            });
            let fragment = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mod post fragment"),
                source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mod post"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &vertex,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &fragment,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            let uniforms = (uniforms_layout.size > 0).then(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("mod post uniforms"),
                    contents: &vec![0; uniforms_layout.size as usize],
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            });
            Shader {
                format,
                pipeline,
                layout,
                uniforms,
                size: uniforms_layout.size,
                enabled: false,
            }
        })?;
        self.shaders.insert(id.into(), shader);
        Ok(())
    }

    pub fn set_uniforms(
        &mut self,
        queue: &wgpu::Queue,
        id: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let shader = self.shaders.get(id).ok_or("unknown post shader")?;
        if bytes.len() != shader.size as usize {
            return Err("post uniform byte length does not match layout".into());
        }
        if let Some(buffer) = &shader.uniforms {
            queue.write_buffer(buffer, 0, bytes);
        }
        Ok(())
    }

    pub fn enable(&mut self, id: &str, enabled: bool) -> Result<(), String> {
        self.shaders
            .get_mut(id)
            .ok_or("unknown post shader")?
            .enabled = enabled;
        Ok(())
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
    ) -> Result<(), String> {
        if !self.active() {
            return Ok(());
        }
        let texture = color.texture();
        if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return Err("post world target cannot be copied".into());
        }
        let size = texture.size();
        let format = texture.format().remove_srgb_suffix();
        if texture.dimension() != wgpu::TextureDimension::D2
            || texture.sample_count() != 1
            || size.depth_or_array_layers != 1
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err("post world target must be a single-sampled 2D color attachment".into());
        }
        if self
            .shaders
            .values()
            .any(|shader| shader.enabled && shader.format != format)
        {
            return Err("post shader color format does not match the world target".into());
        }
        if !self.target.as_ref().is_some_and(|target| {
            target.texture.size() == size && target.texture.format() == format
        }) {
            self.target = Some(checked(device, || {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("mod post source"),
                    size,
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                    label: Some("mod post sampler"),
                    ..Default::default()
                });
                Target {
                    texture,
                    view,
                    sampler,
                }
            })?);
        }
        let target = self.target.as_ref().unwrap();
        for shader in self.shaders.values().filter(|shader| shader.enabled) {
            encoder.copy_texture_to_texture(
                texture.as_image_copy(),
                target.texture.as_image_copy(),
                size,
            );
            let mut entries = vec![
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&target.view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&target.sampler),
                },
            ];
            if let Some(buffer) = &shader.uniforms {
                entries.push(wgpu::BindGroupEntry {
                    binding: 2,
                    resource: buffer.as_entire_binding(),
                });
            }
            let group = checked(device, || {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("mod post group"),
                    layout: &shader.layout,
                    entries: &entries,
                })
            })?;
            checked(device, || {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("mod post"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: color,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                pass.set_pipeline(&shader.pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.draw(0..3, 0..1);
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = "@group(0) @binding(0) var image: texture_2d<f32>; @group(0) @binding(1) var nearest: sampler; @fragment fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return textureSample(image, nearest, uv); }";

    #[test]
    fn validates_fullscreen_contract_and_uniforms() {
        validate(VALID, NativePostUniformLayout::default()).unwrap();
        let source = format!("@group(0) @binding(2) var<uniform> tint: vec4<f32>; {VALID}");
        validate(&source, NativePostUniformLayout { size: 16 }).unwrap();
        assert!(validate(&source, NativePostUniformLayout { size: 32 }).is_err());
        assert!(validate(&source, NativePostUniformLayout::default()).is_err());
    }

    #[test]
    fn rejects_invalid_sources_storage_loops_and_resource_excess() {
        for source in [
            "invalid wgsl",
            "",
            &" ".repeat(MAX_POST_SOURCE + 1),
            &format!("@group(0) @binding(3) var<storage, read> data: array<f32>; {VALID}"),
            &VALID.replace("return textureSample", "loop {} return textureSample"),
            &VALID.replace("@group(0)", "@group(1)"),
            &format!("var<private> data: array<vec4<f32>, 2048>; {VALID}"),
        ] {
            assert!(validate(source, NativePostUniformLayout::default()).is_err());
        }
        assert!(validate(VALID, NativePostUniformLayout { size: 15 }).is_err());
        assert!(
            validate(
                VALID,
                NativePostUniformLayout {
                    size: MAX_POST_UNIFORMS + 16
                }
            )
            .is_err()
        );
    }

    #[test]
    fn no_registration_means_no_pass_or_allocated_target() {
        let mut shaders = PostShaders::default();
        assert!(!shaders.active());
        assert!(shaders.target.is_none());
        assert!(shaders.enable("missing", true).is_err());
    }
}
