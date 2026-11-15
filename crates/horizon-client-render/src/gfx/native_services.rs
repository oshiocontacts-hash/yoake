use std::collections::{BTreeMap, VecDeque};
use std::ops::Range;

use bytemuck::{Pod, Zeroable};
use glam::{DVec3, Mat4, Vec3, Vec4};
use wgpu::util::DeviceExt;

use crate::hud::native_textures::NativeGuiTexture;

pub const MAX_WORLD_DRAWS: usize = 256;
const MAX_DISTANCE: f64 = 256.0;

enum Shape {
    Quad([[f64; 3]; 4], [f32; 4]),
    Line([f64; 3], [f64; 3], f32),
    Billboard([f64; 3], [f32; 2], [f32; 4]),
}

struct Draw {
    owner: String,
    shape: Shape,
    color: u32,
    texture: Option<NativeGuiTexture>,
}

#[derive(Default)]
pub struct WorldDraws {
    draws: Vec<Draw>,
    owner: String,
    disabled: BTreeMap<String, String>,
    cursor: usize,
    counts: BTreeMap<String, usize>,
}

impl WorldDraws {
    pub fn set_owner(&mut self, owner: &str) {
        self.owner = owner.into();
    }

    pub fn state(&self, owner: &str) -> Result<(), String> {
        self.disabled
            .get(owner)
            .map_or(Ok(()), |error| Err(error.clone()))
    }

    pub fn disable(&mut self, owner: &str, error: &str) {
        if !self.disabled.contains_key(owner) {
            eprintln!("mod world draws {owner} disabled: {error}");
            self.disabled.insert(owner.into(), error.into());
        }
    }

    pub fn failures(&self) -> BTreeMap<String, String> {
        self.disabled.clone()
    }

    pub fn restore_failures(&mut self, failures: &BTreeMap<String, String>) {
        self.disabled.clone_from(failures);
    }

    fn take_fair(&mut self) -> Vec<Draw> {
        let mut owners: BTreeMap<String, VecDeque<Draw>> = BTreeMap::new();
        for draw in std::mem::take(&mut self.draws) {
            if !self.disabled.contains_key(&draw.owner) {
                owners
                    .entry(draw.owner.clone())
                    .or_default()
                    .push_back(draw);
            }
        }
        let mut queues: Vec<_> = owners.into_values().collect();
        if queues.is_empty() {
            return Vec::new();
        }
        let count = queues.len();
        queues.rotate_left(self.cursor % count);
        self.cursor = (self.cursor + 1) % count;
        let mut selected = Vec::new();
        while selected.len() < MAX_WORLD_DRAWS {
            let before = selected.len();
            for queue in &mut queues {
                if selected.len() == MAX_WORLD_DRAWS {
                    break;
                }
                if let Some(draw) = queue.pop_front() {
                    selected.push(draw);
                }
            }
            if selected.len() == before {
                break;
            }
        }
        selected
    }

    pub fn clear(&mut self) {
        self.draws.clear();
        self.counts.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.draws.is_empty()
    }

    fn push(
        &mut self,
        shape: Shape,
        color: u32,
        texture: Option<&NativeGuiTexture>,
    ) -> Result<(), String> {
        self.state(&self.owner)?;
        let count = self.counts.entry(self.owner.clone()).or_default();
        if *count >= MAX_WORLD_DRAWS {
            return Err("world draw per-mod frame limit reached".into());
        }
        if texture.is_some_and(|texture| !texture.is_valid()) {
            return Err("world draw texture is invalid".into());
        }
        *count += 1;
        self.draws.push(Draw {
            owner: self.owner.clone(),
            shape,
            color,
            texture: texture.cloned(),
        });
        Ok(())
    }

    pub fn quad(&mut self, corners: [[f64; 3]; 4], color: u32) -> Result<(), String> {
        self.textured_quad_inner(corners, [0.0, 0.0, 1.0, 1.0], color, None)
    }

    pub fn textured_quad(
        &mut self,
        corners: [[f64; 3]; 4],
        uv: [f32; 4],
        color: u32,
        texture: &NativeGuiTexture,
    ) -> Result<(), String> {
        self.textured_quad_inner(corners, uv, color, Some(texture))
    }

    fn textured_quad_inner(
        &mut self,
        corners: [[f64; 3]; 4],
        uv: [f32; 4],
        color: u32,
        texture: Option<&NativeGuiTexture>,
    ) -> Result<(), String> {
        if !corners.iter().flatten().all(|value| value.is_finite())
            || !uv.iter().all(|value| value.is_finite())
        {
            return Err("world quad coordinates must be finite".into());
        }
        self.push(Shape::Quad(corners, uv), color, texture)
    }

    pub fn line(
        &mut self,
        start: [f64; 3],
        end: [f64; 3],
        color: u32,
        width: f32,
    ) -> Result<(), String> {
        if !start
            .iter()
            .chain(end.iter())
            .all(|value| value.is_finite())
            || start == end
            || !width.is_finite()
            || !(0.5..=4.0).contains(&width)
        {
            return Err(
                "world lines need finite distinct endpoints and width from 0.5 to 4".into(),
            );
        }
        self.push(Shape::Line(start, end, width), color, None)
    }

    pub fn billboard(
        &mut self,
        center: [f64; 3],
        size: [f32; 2],
        uv: [f32; 4],
        color: u32,
        texture: Option<&NativeGuiTexture>,
    ) -> Result<(), String> {
        if !center.iter().all(|value| value.is_finite())
            || !size
                .iter()
                .all(|value| value.is_finite() && *value > 0.0 && *value <= 256.0)
            || !uv.iter().all(|value| value.is_finite())
        {
            return Err("world billboard needs finite coordinates and size in (0, 256]".into());
        }
        self.push(Shape::Billboard(center, size, uv), color, texture)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    clip: [f32; 4],
    uv: [f32; 2],
    color: [f32; 4],
}

fn vertices(draw: &Draw, camera: DVec3, matrix: Mat4, size: [u32; 2]) -> Option<[Vertex; 6]> {
    let project = |point: [f64; 3]| {
        let relative = DVec3::from_array(point) - camera;
        (relative.length_squared() <= MAX_DISTANCE * MAX_DISTANCE)
            .then(|| matrix * relative.as_vec3().extend(1.0))
    };
    let (clips, uv) = match draw.shape {
        Shape::Quad(corners, uv) => {
            let clips = corners.map(project);
            ([clips[0]?, clips[1]?, clips[2]?, clips[3]?], uv)
        }
        Shape::Line(start, end, width) => {
            let mut start = project(start)?;
            let mut end = project(end)?;
            let epsilon = 0.0001;
            if start.w < epsilon && end.w < epsilon {
                return None;
            }
            if start.w < epsilon {
                start = start.lerp(end, (epsilon - start.w) / (end.w - start.w));
                start.w = epsilon;
            } else if end.w < epsilon {
                end = end.lerp(start, (epsilon - end.w) / (start.w - end.w));
                end.w = epsilon;
            }
            let pixels = glam::Vec2::new(size[0] as f32, size[1] as f32);
            let direction = (end.truncate().truncate() / end.w
                - start.truncate().truncate() / start.w)
                * pixels;
            let normal =
                glam::Vec2::new(-direction.y, direction.x).try_normalize()? * width / pixels;
            let offset = Vec4::new(normal.x, normal.y, 0.0, 0.0);
            (
                [
                    start + offset * start.w,
                    end + offset * end.w,
                    end - offset * end.w,
                    start - offset * start.w,
                ],
                [0.0, 0.0, 1.0, 1.0],
            )
        }
        Shape::Billboard(center, dimensions, uv) => {
            let inverse = matrix.inverse();
            let origin = inverse.project_point3(Vec3::new(0.0, 0.0, 0.5));
            let right = (inverse.project_point3(Vec3::new(1.0, 0.0, 0.5)) - origin)
                .try_normalize()?
                * dimensions[0]
                * 0.5;
            let up = (inverse.project_point3(Vec3::new(0.0, 1.0, 0.5)) - origin).try_normalize()?
                * dimensions[1]
                * 0.5;
            let corners = [-right + up, right + up, right - up, -right - up]
                .map(|offset| (DVec3::from_array(center) + offset.as_dvec3()).to_array());
            let clips = corners.map(project);
            ([clips[0]?, clips[1]?, clips[2]?, clips[3]?], uv)
        }
    };
    if !clips.iter().all(|clip| clip.is_finite()) {
        return None;
    }
    let [alpha, red, green, blue] = draw.color.to_be_bytes();
    let color = [red, green, blue, alpha].map(|value| f32::from(value) / 255.0);
    let coordinates = [
        [uv[0], uv[1]],
        [uv[2], uv[1]],
        [uv[2], uv[3]],
        [uv[0], uv[3]],
    ];
    Some([0, 1, 2, 0, 2, 3].map(|index| Vertex {
        clip: clips[index].to_array(),
        uv: coordinates[index],
        color,
    }))
}

pub(super) fn checked<Value>(
    device: &wgpu::Device,
    create: impl FnOnce() -> Value,
) -> Result<Value, String> {
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);
    let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let value = std::panic::catch_unwind(std::panic::AssertUnwindSafe(create));
    let errors = [
        pollster::block_on(memory.pop()),
        pollster::block_on(internal.pop()),
        pollster::block_on(validation.pop()),
    ];
    if let Some(error) = errors.into_iter().flatten().next() {
        Err(error.to_string())
    } else {
        value.map_err(|_| "mod GPU command panicked during validation".into())
    }
}

struct Pipeline {
    format: wgpu::TextureFormat,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    white: wgpu::Texture,
    sampler: wgpu::Sampler,
}

#[derive(Default)]
pub struct WorldServices {
    pub draws: WorldDraws,
    pub post: super::native_post_shader::PostShaders,
    pipeline: Option<Pipeline>,
    groups: BTreeMap<u64, (Option<NativeGuiTexture>, wgpu::BindGroup)>,
}

impl WorldServices {
    fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Result<(), String> {
        if self
            .pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.format == format)
        {
            return Ok(());
        }
        self.pipeline = Some(checked(device, || {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mod world draws"),
                source: wgpu::ShaderSource::Wgsl(include_str!("native_services.wgsl").into()),
            });
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mod world texture"),
                entries: &[
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
                ],
            });
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mod world layout"),
                bind_group_layouts: &[Some(&layout)],
                ..Default::default()
            });
            let attributes =
                wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x2, 2 => Float32x4];
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mod world draws"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &attributes,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::GreaterEqual),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            let white = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("mod world white"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                white.as_image_copy(),
                &[255; 4],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4),
                    rows_per_image: Some(1),
                },
                white.size(),
            );
            Pipeline {
                format,
                pipeline,
                layout,
                white,
                sampler: device.create_sampler(&wgpu::SamplerDescriptor::default()),
            }
        })?);
        Ok(())
    }

    pub(super) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: &wgpu::Texture,
        depth: &wgpu::Texture,
        matrix: Mat4,
        camera: DVec3,
        format: wgpu::TextureFormat,
    ) -> Result<Option<wgpu::CommandBuffer>, String> {
        let draws = self.draws.take_fair();
        if draws.is_empty() {
            return Ok(None);
        }
        let owners: std::collections::BTreeSet<_> =
            draws.iter().map(|draw| draw.owner.clone()).collect();
        let result = self.encode(device, queue, color, depth, matrix, camera, format, draws);
        if let Err(error) = &result {
            for owner in owners {
                self.draws.disable(&owner, error);
            }
        }
        result
    }

    fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: &wgpu::Texture,
        depth: &wgpu::Texture,
        matrix: Mat4,
        camera: DVec3,
        format: wgpu::TextureFormat,
        draws: Vec<Draw>,
    ) -> Result<Option<wgpu::CommandBuffer>, String> {
        if !matrix.is_finite() || !camera.is_finite() {
            return Err("world draw camera must be finite".into());
        }
        let color_texture = color;
        let depth_texture = depth;
        if !matches!(
            format,
            wgpu::TextureFormat::Rgba8Unorm
                | wgpu::TextureFormat::Bgra8Unorm
                | wgpu::TextureFormat::Rgba16Float
        ) || color_texture.format().remove_srgb_suffix() != format
            || depth_texture.format() != wgpu::TextureFormat::Depth32Float
            || color_texture.size() != depth_texture.size()
            || [color_texture, depth_texture].iter().any(|texture| {
                texture.dimension() != wgpu::TextureDimension::D2
                    || texture.sample_count() != 1
                    || texture.depth_or_array_layers() != 1
                    || texture.width() == 0
                    || texture.height() == 0
                    || !texture
                        .usage()
                        .contains(wgpu::TextureUsages::RENDER_ATTACHMENT)
            })
        {
            return Err(
                "world draws require matching single-sample 2D color/depth32 attachments".into(),
            );
        }
        if self
            .pipeline
            .as_ref()
            .is_some_and(|pipeline| pipeline.format != format)
        {
            self.groups.clear();
        }
        self.prepare(device, queue, format)?;
        let pipeline = self.pipeline.as_ref().ok_or("world pipeline unavailable")?;
        let size = [color.width(), color.height()];
        let mut data = Vec::new();
        let mut commands: Vec<(Range<u32>, u64)> = Vec::new();
        self.groups
            .retain(|_, (texture, _)| texture.as_ref().is_none_or(NativeGuiTexture::is_valid));
        checked(device, || {
            for draw in draws {
                if self.draws.disabled.contains_key(&draw.owner) {
                    continue;
                }
                if draw
                    .texture
                    .as_ref()
                    .is_some_and(|texture| !texture.is_valid())
                {
                    self.draws
                        .disable(&draw.owner, "world draw texture is invalid");
                    continue;
                }
                let Some(vertices) = vertices(&draw, camera, matrix, size) else {
                    continue;
                };
                let start = data.len() as u32;
                data.extend(vertices);
                let key = draw
                    .texture
                    .as_ref()
                    .map_or(0, NativeGuiTexture::registration);
                self.groups.entry(key).or_insert_with(|| {
                    let texture = draw
                        .texture
                        .as_ref()
                        .map_or(&pipeline.white, NativeGuiTexture::texture);
                    let view = texture.create_view(&Default::default());
                    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("mod world draw"),
                        layout: &pipeline.layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&pipeline.sampler),
                            },
                        ],
                    });
                    (draw.texture.clone(), group)
                });
                commands.push((start..data.len() as u32, key));
            }
            if data.is_empty() {
                return None;
            }
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mod world vertices"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::VERTEX,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("isolated mod world draws"),
            });
            let color_view = color.create_view(&wgpu::TextureViewDescriptor {
                format: Some(format),
                dimension: Some(wgpu::TextureViewDimension::D2),
                mip_level_count: Some(1),
                array_layer_count: Some(1),
                ..Default::default()
            });
            let depth_view = depth.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                mip_level_count: Some(1),
                array_layer_count: Some(1),
                ..Default::default()
            });
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("mod world draws"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &color_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    ..Default::default()
                });
                pass.set_pipeline(&pipeline.pipeline);
                pass.set_vertex_buffer(0, buffer.slice(..));
                for (range, key) in &commands {
                    pass.set_bind_group(0, &self.groups[key].1, &[]);
                    pass.draw(range.clone(), 0..1);
                }
            }
            Some(encoder.finish())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_services_are_inert() {
        let services = WorldServices::default();
        assert!(services.draws.is_empty());
        assert!(services.pipeline.is_none());
        assert!(!services.post.active());
    }

    #[test]
    fn native_gpu_world_texture_bind_groups_are_reused() {
        use super::super::native_services_tests::{gpu, target};
        use crate::hud::{HudRenderer, native_textures::NativeGuiTextureFilter};

        let Some((device, queue)) = gpu() else { return };
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let color = target(&device, 4, format);
        let depth = target(&device, 4, wgpu::TextureFormat::Depth32Float);
        let view = color.create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let hud = HudRenderer::new(&device, format, &view, &view, &sampler).unwrap();
        let texture = hud
            .native_gui_texture_create(&device, [1, 1], format, NativeGuiTextureFilter::Nearest)
            .unwrap();
        let mut services = WorldServices::default();
        let corners = [
            [-1.0, -1.0, 0.75],
            [1.0, -1.0, 0.75],
            [1.0, 1.0, 0.75],
            [-1.0, 1.0, 0.75],
        ];
        services
            .draws
            .textured_quad(corners, [0.0, 0.0, 1.0, 1.0], u32::MAX, &texture)
            .unwrap();
        queue.submit(
            services
                .render(
                    &device,
                    &queue,
                    &color,
                    &depth,
                    Mat4::IDENTITY,
                    DVec3::ZERO,
                    format,
                )
                .unwrap(),
        );
        let cached = services.groups[&texture.registration()].1.clone();
        for _ in 0..3 {
            services.draws.clear();
            for _ in 0..MAX_WORLD_DRAWS {
                services
                    .draws
                    .textured_quad(corners, [0.0, 0.0, 1.0, 1.0], u32::MAX, &texture)
                    .unwrap();
            }
            queue.submit(
                services
                    .render(
                        &device,
                        &queue,
                        &color,
                        &depth,
                        Mat4::IDENTITY,
                        DVec3::ZERO,
                        format,
                    )
                    .unwrap(),
            );
            assert_eq!(services.groups.len(), 1);
            assert_eq!(services.groups[&texture.registration()].1, cached);
        }
    }

    #[test]
    fn draws_are_bounded_validated_and_frame_local() {
        let mut draws = WorldDraws::default();
        assert!(draws.line([0.0; 3], [f64::NAN; 3], 0, 1.0).is_err());
        assert!(
            draws
                .billboard([0.0; 3], [0.0, 1.0], [0.0; 4], 0, None)
                .is_err()
        );
        for _ in 0..MAX_WORLD_DRAWS {
            draws.quad([[0.0; 3]; 4], 0).unwrap();
        }
        assert!(draws.line([0.0; 3], [1.0; 3], 0, 1.0).is_err());
        draws.clear();
        assert!(draws.is_empty());
    }

    #[test]
    fn world_vertices_preserve_depth_color_and_uv_and_cull_far_requests() {
        let mut draw = Draw {
            owner: "test".into(),
            shape: Shape::Quad(
                [
                    [0.0, 1.0, 0.5],
                    [1.0, 1.0, 0.5],
                    [1.0, 0.0, 0.5],
                    [0.0, 0.0, 0.5],
                ],
                [0.25, 0.5, 0.75, 1.0],
            ),
            color: 0x80ff0000,
            texture: None,
        };
        let output = vertices(&draw, DVec3::ZERO, Mat4::IDENTITY, [100, 100]).unwrap();
        assert_eq!(output[0].clip, [0.0, 1.0, 0.5, 1.0]);
        assert_eq!(output[0].uv, [0.25, 0.5]);
        assert_eq!(output[2].uv, [0.75, 1.0]);
        assert_eq!(output[0].color, [1.0, 0.0, 0.0, 128.0 / 255.0]);
        draw.shape = Shape::Quad([[257.0, 0.0, 0.0]; 4], [0.0; 4]);
        assert!(vertices(&draw, DVec3::ZERO, Mat4::IDENTITY, [100, 100]).is_none());
    }

    #[test]
    fn world_budget_is_shared_fairly_regardless_of_registration_order() {
        let mut draws = WorldDraws::default();
        for _ in 0..3 {
            draws.clear();
            for owner in ["first", "second", "third"] {
                draws.set_owner(owner);
                for _ in 0..MAX_WORLD_DRAWS {
                    draws.quad([[0.0; 3]; 4], 0).unwrap();
                }
                assert!(draws.quad([[0.0; 3]; 4], 0).is_err());
            }
            let selected = draws.take_fair();
            assert_eq!(selected.len(), MAX_WORLD_DRAWS);
            for owner in ["first", "second", "third"] {
                assert!(
                    (85..=86).contains(&selected.iter().filter(|draw| draw.owner == owner).count())
                );
            }
            assert!(draws.is_empty());
        }
    }

    #[test]
    fn failed_owner_stays_disabled_without_affecting_other_mods() {
        let mut draws = WorldDraws::default();
        draws.disable("first", "first error");
        draws.disable("first", "second error");
        draws.clear();
        draws.set_owner("first");
        assert_eq!(draws.quad([[0.0; 3]; 4], 0).unwrap_err(), "first error");
        draws.set_owner("second");
        draws.quad([[0.0; 3]; 4], 0).unwrap();
        let mut restored = WorldDraws::default();
        restored.restore_failures(&draws.failures());
        assert_eq!(restored.state("first"), Err("first error".into()));
        assert!(restored.state("second").is_ok());
    }

    #[test]
    fn lines_crossing_the_eye_plane_are_clipped_in_either_direction() {
        let matrix = Mat4::perspective_rh(1.0, 1.0, 0.1, 100.0);
        for (start, end) in [
            ([-0.5, 0.0, 1.0], [0.5, 0.5, -1.0]),
            ([0.5, 0.5, -1.0], [-0.5, 0.0, 1.0]),
        ] {
            let draw = Draw {
                owner: "test".into(),
                shape: Shape::Line(start, end, 2.0),
                color: u32::MAX,
                texture: None,
            };
            let output = vertices(&draw, DVec3::ZERO, matrix, [100, 100]).unwrap();
            assert!(
                output
                    .iter()
                    .all(|vertex| vertex.clip.iter().all(|value| value.is_finite())
                        && vertex.clip[3] >= 0.0001)
            );
        }
        let draw = Draw {
            owner: "test".into(),
            shape: Shape::Line([-0.5, 0.0, 1.0], [0.5, 0.5, 1.0], 2.0),
            color: u32::MAX,
            texture: None,
        };
        assert!(vertices(&draw, DVec3::ZERO, matrix, [100, 100]).is_none());
    }
}
