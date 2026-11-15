use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use wgpu::util::DeviceExt;

use super::native_services::checked;

pub const MAX_POST_SHADERS: usize = 4;
pub const MAX_POST_SOURCE: usize = 65_536;
pub const MAX_POST_UNIFORMS: u32 = 4096;
const MAX_INVOCATION_COST: u64 = 8192;
const MAX_IMAGE_SAMPLES: u64 = 64;
const MAX_CALL_DEPTH: u32 = 16;

#[derive(Clone, Copy, Default)]
struct Cost {
    expressions: u64,
    samples: u64,
    depth: u32,
}

fn calls(
    block: &naga::Block,
    iterations: u64,
    multiplier: u64,
    output: &mut Vec<(naga::Handle<naga::Function>, u64)>,
) -> u64 {
    let mut maximum = multiplier;
    for statement in block.iter() {
        let nested = match statement {
            naga::Statement::Call { function, .. } => {
                output.push((*function, multiplier));
                multiplier
            }
            naga::Statement::Block(block) => calls(block, iterations, multiplier, output),
            naga::Statement::If { accept, reject, .. } => {
                calls(accept, iterations, multiplier, output)
                    .max(calls(reject, iterations, multiplier, output))
            }
            naga::Statement::Switch { cases, .. } => cases
                .iter()
                .map(|case| calls(&case.body, iterations, multiplier, output))
                .max()
                .unwrap_or(multiplier),
            naga::Statement::Loop {
                body, continuing, ..
            } => {
                let repeated = multiplier.saturating_mul(iterations);
                calls(body, iterations, repeated, output)
                    .max(calls(continuing, iterations, repeated, output))
            }
            _ => multiplier,
        };
        maximum = maximum.max(nested);
    }
    maximum
}

fn invocation_cost(
    module: &naga::Module,
    function: &naga::Function,
    cache: &mut BTreeMap<usize, Cost>,
    ancestors: u32,
    iterations: u64,
) -> Result<Cost, String> {
    if ancestors >= MAX_CALL_DEPTH {
        return Err("post shader call depth exceeds 16".into());
    }
    let mut cost = Cost {
        expressions: function.expressions.len() as u64,
        samples: function
            .expressions
            .iter()
            .filter(|(_, expression)| {
                matches!(
                    expression,
                    naga::Expression::ImageSample { .. } | naga::Expression::ImageLoad { .. }
                )
            })
            .count() as u64,
        depth: 1,
    };
    let mut callees = Vec::new();
    let multiplier = calls(&function.body, iterations, 1, &mut callees);
    cost.expressions = cost.expressions.saturating_mul(multiplier);
    cost.samples = cost.samples.saturating_mul(multiplier);
    for (callee, repetitions) in callees {
        let nested = if let Some(cached) = cache.get(&callee.index()) {
            *cached
        } else {
            let nested = invocation_cost(
                module,
                &module.functions[callee],
                cache,
                ancestors + 1,
                iterations,
            )?;
            cache.insert(callee.index(), nested);
            nested
        };
        cost.expressions = cost
            .expressions
            .saturating_add(nested.expressions.saturating_mul(repetitions));
        cost.samples = cost
            .samples
            .saturating_add(nested.samples.saturating_mul(repetitions));
        cost.depth = cost.depth.max(nested.depth.saturating_add(1));
    }
    if cost.expressions > MAX_INVOCATION_COST
        || cost.samples > MAX_IMAGE_SAMPLES
        || cost.depth > MAX_CALL_DEPTH
    {
        return Err(
            "post shader exceeds 8192 expression-equivalents, 64 image fetches, or call depth 16"
                .into(),
        );
    }
    Ok(cost)
}

pub(super) fn validate_cost(
    module: &naga::Module,
    function: &naga::Function,
    iterations: u32,
) -> Result<(), String> {
    invocation_cost(
        module,
        function,
        &mut BTreeMap::new(),
        0,
        u64::from(iterations),
    )
    .map(|_| ())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativePostUniformLayout {
    pub size: u32,
}

struct SavedShader {
    source: String,
    layout: NativePostUniformLayout,
    uniforms: Vec<u8>,
    state: Result<bool, String>,
}

#[derive(Default)]
struct ModBudget {
    failed: u32,
    since: Option<Instant>,
    updates: u32,
}

impl ModBudget {
    fn registration(&mut self, now: Instant) -> Result<(), String> {
        if self
            .since
            .is_none_or(|since| now.duration_since(since) >= Duration::from_secs(60))
        {
            self.since = Some(now);
            self.failed = 0;
        }
        if self.failed >= 8 {
            return Err("post shader failed registration limit: 8 per mod per minute".into());
        }
        Ok(())
    }

    fn uniform_update(&mut self) -> Result<(), String> {
        if self.updates >= 16 {
            return Err("post uniform update limit: 16 per mod per frame".into());
        }
        self.updates += 1;
        Ok(())
    }
}

#[derive(Default)]
pub struct PostShaderRegistry {
    shaders: BTreeMap<String, SavedShader>,
    budgets: BTreeMap<String, ModBudget>,
}

impl PostShaderRegistry {
    pub fn begin_frame(&mut self) {
        for budget in self.budgets.values_mut() {
            budget.updates = 0;
        }
    }

    pub fn register(
        &mut self,
        post: &mut PostShaders,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        id: &str,
        source: &str,
        layout: NativePostUniformLayout,
    ) -> Result<(), String> {
        if id.is_empty() || id.len() > 256 || !id.contains(':') {
            return Err("post shader id must be a namespaced id of at most 256 bytes".into());
        }
        let budget = self
            .budgets
            .entry(id.split(':').next().unwrap().into())
            .or_default();
        budget.registration(Instant::now())?;
        let result = if self.shaders.contains_key(id) {
            Err("post shader already registered".into())
        } else if self
            .shaders
            .keys()
            .filter(|registered| registered.split(':').next() == id.split(':').next())
            .count()
            >= MAX_POST_SHADERS
        {
            Err("post shader limit: 4 per mod".into())
        } else {
            post.register(device, format, id, source, layout)
        };
        if result.is_err() {
            budget.failed += 1;
        }
        result?;
        self.shaders.insert(
            id.into(),
            SavedShader {
                source: source.into(),
                layout,
                uniforms: vec![0; layout.size as usize],
                state: Ok(false),
            },
        );
        Ok(())
    }

    pub fn set_uniforms(
        &mut self,
        post: &mut PostShaders,
        queue: &wgpu::Queue,
        id: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        let saved = self.shaders.get_mut(id).ok_or("unknown post shader")?;
        self.budgets
            .entry(id.split(':').next().unwrap().into())
            .or_default()
            .uniform_update()?;
        post.set_uniforms(queue, id, bytes)?;
        saved.uniforms.copy_from_slice(bytes);
        Ok(())
    }

    pub fn enable(
        &mut self,
        post: &mut PostShaders,
        id: &str,
        enabled: bool,
    ) -> Result<(), String> {
        let saved = self.shaders.get_mut(id).ok_or("unknown post shader")?;
        saved.state.as_ref().map_err(Clone::clone)?;
        post.enable(id, enabled)?;
        saved.state = Ok(enabled);
        Ok(())
    }

    pub fn state(&self, post: &PostShaders, id: &str) -> Result<bool, String> {
        let saved = self.shaders.get(id).ok_or("unknown post shader")?;
        saved.state.as_ref().map_err(Clone::clone)?;
        post.state(id)
    }

    pub fn unregister(&mut self, post: &mut PostShaders, id: &str) -> Result<(), String> {
        self.shaders.remove(id).ok_or("unknown post shader")?;
        if post.shaders.contains_key(id) {
            post.unregister(id)?;
        }
        Ok(())
    }

    pub fn capture(&mut self, post: &PostShaders) {
        for (id, saved) in &mut self.shaders {
            if saved.state.is_ok() {
                saved.state = post.state(id);
            }
        }
    }

    pub fn restore(
        &mut self,
        post: &mut PostShaders,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) {
        for (id, saved) in &mut self.shaders {
            let Ok(enabled) = saved.state else { continue };
            let result = post
                .register(device, format, id, &saved.source, saved.layout)
                .and_then(|()| post.set_uniforms(queue, id, &saved.uniforms))
                .and_then(|()| post.enable(id, enabled));
            if let Err(error) = result {
                post.disable(id, error.clone());
                saved.state = Err(error);
            }
        }
    }
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
    if module
        .types
        .iter()
        .any(|(handle, _)| layouter[handle].size > 16_384)
    {
        return Err("post shader type size exceeds 16384 bytes".into());
    }
    validate_cost(&module, &module.entry_points[0].function, 1)?;
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
    failure: Option<String>,
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
            || self
                .shaders
                .keys()
                .filter(|existing| existing.split(':').next() == id.split(':').next())
                .count()
                >= MAX_POST_SHADERS
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
                failure: None,
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
        let shader = self.shaders.get_mut(id).ok_or("unknown post shader")?;
        if enabled && let Some(error) = &shader.failure {
            return Err(error.clone());
        }
        shader.enabled = enabled;
        Ok(())
    }

    pub fn unregister(&mut self, id: &str) -> Result<(), String> {
        self.shaders
            .remove(id)
            .map(|_| ())
            .ok_or("unknown post shader".into())
    }

    pub fn state(&self, id: &str) -> Result<bool, String> {
        let shader = self.shaders.get(id).ok_or("unknown post shader")?;
        match &shader.failure {
            Some(error) => Err(error.clone()),
            None => Ok(shader.enabled),
        }
    }

    fn disable(&mut self, id: &str, error: String) {
        if let Some(shader) = self.shaders.get_mut(id) {
            if shader.failure.is_none() {
                eprintln!("mod post shader {id}: {error}");
                shader.failure = Some(error);
            }
            shader.enabled = false;
        }
    }

    pub fn render(
        &mut self,
        device: &wgpu::Device,
        color: &wgpu::Texture,
    ) -> Vec<wgpu::CommandBuffer> {
        let ids: Vec<_> = self
            .shaders
            .iter()
            .filter(|(_, shader)| shader.enabled)
            .map(|(id, _)| id.clone())
            .collect();
        let mut commands = Vec::new();
        for id in ids {
            match self.encode(device, color, &id) {
                Ok(buffer) => commands.push(buffer),
                Err(error) => self.disable(&id, error),
            }
        }
        commands
    }

    fn encode(
        &mut self,
        device: &wgpu::Device,
        color: &wgpu::Texture,
        id: &str,
    ) -> Result<wgpu::CommandBuffer, String> {
        let texture = color;
        if !texture.usage().contains(wgpu::TextureUsages::COPY_SRC) {
            return Err("post world target cannot be copied".into());
        }
        let size = texture.size();
        let format = texture.format().remove_srgb_suffix();
        if texture.dimension() != wgpu::TextureDimension::D2
            || texture.sample_count() != 1
            || size.depth_or_array_layers != 1
            || size.width == 0
            || size.height == 0
            || !matches!(
                format,
                wgpu::TextureFormat::Rgba8Unorm
                    | wgpu::TextureFormat::Bgra8Unorm
                    | wgpu::TextureFormat::Rgba16Float
            )
            || !texture
                .usage()
                .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
        {
            return Err("post world target must be a single-sampled 2D color attachment".into());
        }
        if self.shaders[id].format != format {
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
        let shader = &self.shaders[id];
        checked(device, || {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("isolated mod post shader"),
            });
            let color_view = color.create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                dimension: Some(wgpu::TextureViewDimension::D2),
                mip_level_count: Some(1),
                array_layer_count: Some(1),
                ..Default::default()
            });
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
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("mod post group"),
                layout: &shader.layout,
                entries: &entries,
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("mod post"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
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
            }
            encoder.finish()
        })
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

    fn call_chain(levels: usize, fanout: usize, sampled: bool) -> String {
        let leaf = if sampled {
            "textureSample(image, nearest, uv)"
        } else {
            "vec4<f32>(uv, uv.x + uv.y, 1.0)"
        };
        let mut source = VALID.replace(
            "textureSample(image, nearest, uv)",
            &format!("level{}(uv)", levels - 1),
        );
        source.push_str(&format!(
            "fn level0(uv: vec2<f32>) -> vec4<f32> {{ return {leaf}; }}"
        ));
        for level in 1..levels {
            let expression = vec![format!("level{}(uv)", level - 1); fanout].join(" + ");
            source.push_str(&format!(
                "fn level{level}(uv: vec2<f32>) -> vec4<f32> {{ return {expression}; }}"
            ));
        }
        source
    }

    #[test]
    fn counts_every_call_in_sample_and_expression_dags() {
        validate(&call_chain(3, 8, true), NativePostUniformLayout::default()).unwrap();
        let samples =
            validate(&call_chain(13, 8, true), NativePostUniformLayout::default()).unwrap_err();
        assert!(samples.contains("64 image fetches"), "{samples}");
        let expressions =
            validate(&call_chain(7, 8, false), NativePostUniformLayout::default()).unwrap_err();
        assert!(
            expressions.contains("8192 expression-equivalents"),
            "{expressions}"
        );
    }

    #[test]
    fn bounds_call_depth_even_without_fanout() {
        validate(
            &call_chain(15, 1, false),
            NativePostUniformLayout::default(),
        )
        .unwrap();
        let error = validate(
            &call_chain(16, 1, false),
            NativePostUniformLayout::default(),
        )
        .unwrap_err();
        assert!(error.contains("depth"), "{error}");
    }

    #[test]
    fn rejects_oversized_constant_argument_and_return_types_without_variables() {
        for declaration in [
            "const huge: array<vec4<f32>, 1025> = array<vec4<f32>, 1025>();",
            "fn huge_arg(value: array<vec4<f32>, 1025>) -> f32 { return value[0].x; }",
            "fn huge_return() -> array<vec4<f32>, 1025> { return array<vec4<f32>, 1025>(); }",
        ] {
            let source = format!("{declaration} {VALID}");
            let module = naga::front::wgsl::parse_str(&source).unwrap();
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::empty(),
            )
            .validate(&module)
            .unwrap();
            let error = validate(&source, NativePostUniformLayout::default()).unwrap_err();
            assert!(error.contains("type size"), "{error}");
        }
    }

    #[test]
    fn failed_registration_and_uniform_budgets_are_bounded_and_reset() {
        let now = Instant::now();
        let mut budget = ModBudget::default();
        for _ in 0..8 {
            budget.registration(now).unwrap();
            budget.failed += 1;
        }
        assert!(budget.registration(now + Duration::from_secs(59)).is_err());
        budget.registration(now + Duration::from_secs(60)).unwrap();
        assert_eq!(budget.failed, 0);
        for _ in 0..16 {
            budget.uniform_update().unwrap();
        }
        assert!(budget.uniform_update().is_err());
        let mut registry = PostShaderRegistry::default();
        registry.budgets.insert("first".into(), budget);
        registry
            .budgets
            .entry("second".into())
            .or_default()
            .uniform_update()
            .unwrap();
        registry.begin_frame();
        registry
            .budgets
            .get_mut("first")
            .unwrap()
            .uniform_update()
            .unwrap();
    }
}
