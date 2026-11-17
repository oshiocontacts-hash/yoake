use horizon_shader_api::check::CheckedStage;

use super::native_material::TerrainShape;
use super::native_post::{NativeFrame, present_constants};
use super::pack_draw::EntitySpan;
use super::wgsl_prefix::prefixed;

const ENTRIES: &str = r#"
struct HorizonEntityOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) light: vec2<f32>,
    @location(3) lightmap: vec3<f32>,
    @location(4) position: vec3<f32>,
    @location(5) normal: vec3<f32>,
    @location(6) @interpolate(flat) overlay: u32,
    @location(7) overlay_color: vec4<f32>,
    @location(8) glint_uv: vec2<f32>,
    @location(9) @interpolate(flat) cull: u32,
    @location(10) spherical: f32,
    @location(11) cylindrical: f32,
    @location(12) @interpolate(flat) kind: u32,
    @location(13) @interpolate(flat) wrap: vec2<f32>,
}
@vertex fn horizon_entity_vs(
    @location(0) position: vec3<f32>, @location(1) uv: vec2<f32>,
    @location(2) overlay: u32, @location(3) color: vec4<f32>,
    @location(4) light: vec4<u32>, @location(5) normal: vec4<f32>,
    @location(6) glint_uv: vec2<f32>, @builtin(instance_index) kind: u32
) -> HorizonEntityOut {
    let vanilla = horizon_entity_entity_vertex(position, uv, overlay & 0xffffu, color, light, normal);
    var out: HorizonEntityOut;
    out.clip = vanilla.clip;
    out.uv = uv;
    out.tint = color;
    out.light = min(vec2(max(f32(light.y), horizon_entity_dynamic_block_light(position)), f32(light.x)) / 240.0, vec2(1.0));
    out.lightmap = vanilla.light_color.rgb;
    out.position = position;
    out.normal = normal.xyz;
    out.overlay = vanilla.overlay;
    out.overlay_color = vanilla.overlay_color;
    out.glint_uv = glint_uv;
    out.cull = vanilla.cull;
    out.spherical = vanilla.spherical;
    out.cylindrical = vanilla.cylindrical;
    out.kind = kind;
    out.wrap = vanilla.wrap;
    return out;
}
@fragment fn horizon_entity_fs(in: HorizonEntityOut, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var uv = in.uv;
    if in.overlay == 6u { uv.y = in.wrap.x + fract(uv.y) * in.wrap.y; }
    if in.overlay == 11u || in.overlay == 12u {
        uv = horizon_entity_crumbling_uv(in.uv, in.wrap, horizon_entity_globals.crumbling_sprite.zw, textureDimensions(horizon_entity_atlas));
    }
    let texel = textureSample(horizon_entity_atlas, horizon_entity_atlas_sampler, uv);
    var tint = in.tint;
    if in.overlay == 38u {
        if tint.a < textureSampleLevel(horizon_entity_atlas, horizon_entity_atlas_sampler, in.glint_uv, 0.0).a { discard; }
        tint.a = 1.0;
    }
    let face_normal = cross(dpdx(in.position), dpdy(in.position));
    var normal = in.normal;
    if dot(normal, normal) < 0.0001 { normal = face_normal; }
    normal = normal / max(length(normal), 0.0001);
    var input: EntityMaterialInput;
    input.position = in.position;
    input.normal = select(-normal, normal, front);
    input.albedo = texel * tint;
    input.light = in.light;
    input.lightmap = in.lightmap;
    input.kind = in.kind;
    var color = entity_material(input).color;
    if color.a < 0.1 || (in.cull != 0u && !front) { discard; }
    color = vec4(mix(in.overlay_color.rgb, color.rgb, in.overlay_color.a), color.a);
    if in.overlay == 7u || in.overlay == 8u || in.overlay == 33u {
        color = horizon_entity_glint_over(color, textureSample(horizon_entity_glint_tex, horizon_entity_glint_sampler, in.glint_uv));
    } else if in.overlay == 36u {
        color = horizon_entity_glint_over(color, textureSample(horizon_entity_glint_tex, horizon_entity_glint_sampler, horizon_entity_armor_glint_uv(in.glint_uv)));
    }
    let fog = horizon_entity_globals.fog;
    let amount = max(horizon_entity_linear_fog(in.spherical, fog.x, fog.y), horizon_entity_linear_fog(in.cylindrical, fog.z, fog.w));
    return vec4(mix(color.rgb, horizon_entity_globals.fog_color.rgb, amount), color.a);
}
"#;

fn shader_source(stage: &CheckedStage) -> String {
    format!(
        "{}\n{}\n{ENTRIES}",
        prefixed(&super::entity_shader(), "horizon_entity_"),
        stage.source
    )
}

pub(super) fn draw_ranges<'a>(
    spans: &'a [EntitySpan],
    runs: &'a [(usize, usize)],
) -> impl Iterator<Item = (std::ops::Range<u32>, Option<u32>)> + 'a {
    let mut destination = 0;
    runs.iter().flat_map(move |&(first, count)| {
        let source_start = first as u32 * 4;
        let source_end = (first + count) as u32 * 4;
        let base = destination;
        destination += count as u32 * 6;
        let prefix_end = spans
            .first()
            .map_or(source_end, |span| span.start.min(source_end));
        let prefix = (prefix_end > source_start).then_some((
            base..base + prefix_end.saturating_sub(source_start) / 4 * 6,
            None,
        ));
        let first_span = spans
            .partition_point(|span| span.start <= source_start)
            .saturating_sub(1);
        prefix.into_iter().chain(
            spans[first_span..]
                .iter()
                .enumerate()
                .take_while(move |(_, span)| span.start < source_end)
                .filter_map(move |(index, span)| {
                    let end = spans
                        .get(first_span + index + 1)
                        .map_or(source_end, |next| next.start.min(source_end));
                    let start = span.start.max(source_start).min(end);
                    (!span.hidden && end > start).then_some((
                        base + (start - source_start) / 4 * 6..base + (end - source_start) / 4 * 6,
                        span.native_kind,
                    ))
                }),
        )
    })
}

pub struct NativeEntity {
    pub pipeline: wgpu::RenderPipeline,
    pub translucent_pipeline: wgpu::RenderPipeline,
}

impl NativeEntity {
    pub fn new(
        device: &wgpu::Device,
        shape: &TerrainShape<'_>,
        frame: &NativeFrame,
        stage: &CheckedStage,
        constants: &[(String, f64)],
    ) -> Result<Self, String> {
        let source = shader_source(stage);
        let module = naga::front::wgsl::parse_str(&source)
            .map_err(|error| format!("{}: {}", stage.file, error.emit_to_string(&source)))?;
        let constants = present_constants(&module, constants);
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(&stage.label),
            source: wgpu::ShaderSource::Naga(std::borrow::Cow::Owned(module)),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(&stage.label),
            bind_group_layouts: &[Some(shape.entities), None, None, Some(&frame.layout)],
            ..Default::default()
        });
        let attributes = wgpu::vertex_attr_array![0=>Float32x3, 1=>Float32x2, 2=>Uint32, 3=>Unorm8x4, 4=>Uint8x4, 5=>Snorm8x4, 6=>Float32x2];
        let vertices = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<super::Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &attributes,
        };
        let options = wgpu::PipelineCompilationOptions {
            constants: &constants,
            ..Default::default()
        };
        let create_pipeline = |depth_write_enabled| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(&stage.label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some("horizon_entity_vs"),
                    compilation_options: options.clone(),
                    buffers: &[Some(vertices.clone())],
                },
                primitive: wgpu::PrimitiveState {
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(depth_write_enabled),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: wgpu::DepthBiasState {
                        constant: 128,
                        slope_scale: 0.0,
                        clamp: 0.0,
                    },
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some("horizon_entity_fs"),
                    compilation_options: options.clone(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: shape.format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = create_pipeline(true);
        let translucent_pipeline = create_pipeline(false);
        if let Some(error) = pollster::block_on(scope.pop()) {
            return Err(format!("{}: {error}", stage.file));
        }
        Ok(Self {
            pipeline,
            translucent_pipeline,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_shader_api::{Pack, check::check};

    #[test]
    fn entity_ranges_preserve_kinds_and_hide_shadow_only_geometry() {
        let spans = [
            EntitySpan {
                start: 4,
                native_kind: Some(42),
                ..Default::default()
            },
            EntitySpan {
                start: 8,
                hidden: true,
                ..Default::default()
            },
            EntitySpan {
                start: 12,
                native_kind: Some(0x80000001),
                ..Default::default()
            },
            EntitySpan {
                start: 16,
                ..Default::default()
            },
        ];
        assert_eq!(
            draw_ranges(&spans, &[(0, 5)]).collect::<Vec<_>>(),
            vec![
                (0..6, None),
                (6..12, Some(42)),
                (18..24, Some(0x80000001)),
                (24..30, None),
            ]
        );
        assert_eq!(
            draw_ranges(&[], &[(0, 2)]).collect::<Vec<_>>(),
            vec![(0..12, None)]
        );
    }

    #[test]
    fn entity_ranges_follow_material_compaction() {
        let spans = [
            EntitySpan {
                start: 0,
                native_kind: Some(42),
                ..Default::default()
            },
            EntitySpan {
                start: 12,
                native_kind: Some(43),
                ..Default::default()
            },
            EntitySpan {
                start: 24,
                hidden: true,
                ..Default::default()
            },
        ];
        assert_eq!(
            draw_ranges(&spans, &[(0, 1), (2, 3), (6, 1)]).collect::<Vec<_>>(),
            vec![(0..6, Some(42)), (6..12, Some(42)), (12..24, Some(43)),]
        );
    }

    #[test]
    fn native_entity_adapter_validates() {
        let files = [
            (horizon_shader_api::MANIFEST, "schema=1\nid=\"entity_test\"\nname=\"Test\"\nversion=\"1\"\napi=\"0.1\"\n[entity_material]\nfile=\"entity.wgsl\"\n"),
            ("entity.wgsl", "fn entity_material(input: EntityMaterialInput) -> Surface { return Surface(vec4(input.albedo.rgb * input.lightmap, input.albedo.a)); }"),
        ].map(|(name, source)| (name.to_owned(), source.as_bytes().to_vec())).into();
        let report = check(&Pack::from_files(files).unwrap()).unwrap();
        let source = shader_source(&report.stages[0]);
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|error| panic!("{}", error.emit_to_string(&source)));
    }
}
