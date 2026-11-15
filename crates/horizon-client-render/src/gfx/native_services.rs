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
    shape: Shape,
    color: u32,
    texture: Option<NativeGuiTexture>,
}

#[derive(Default)]
pub struct WorldDraws {
    draws: Vec<Draw>,
}

impl WorldDraws {
    pub fn clear(&mut self) {
        self.draws.clear();
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
        if self.draws.len() >= MAX_WORLD_DRAWS {
            return Err("world draw frame limit reached".into());
        }
        if texture.is_some_and(|texture| !texture.is_valid()) {
            return Err("world draw texture is invalid".into());
        }
        self.draws.push(Draw {
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
            let start = project(start)?;
            let end = project(end)?;
            if start.w <= 0.0 || end.w <= 0.0 {
                return None;
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
    let value = create();
    let errors = [
        pollster::block_on(memory.pop()),
        pollster::block_on(internal.pop()),
        pollster::block_on(validation.pop()),
    ];
    if let Some(error) = errors.into_iter().flatten().next() {
        Err(error.to_string())
    } else {
        Ok(value)
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
        encoder: &mut wgpu::CommandEncoder,
        color: &wgpu::TextureView,
        depth: &wgpu::TextureView,
        matrix: Mat4,
        camera: DVec3,
        format: wgpu::TextureFormat,
    ) -> Result<(), String> {
        let draws = std::mem::take(&mut self.draws.draws);
        if draws.is_empty() {
            return Ok(());
        }
        if !matrix.is_finite() || !camera.is_finite() {
            return Err("world draw camera must be finite".into());
        }
        self.prepare(device, queue, format)?;
        let pipeline = self.pipeline.as_ref().ok_or("world pipeline unavailable")?;
        let size = [color.texture().width(), color.texture().height()];
        let mut data = Vec::new();
        let mut commands: Vec<(Range<u32>, wgpu::BindGroup)> = Vec::new();
        for draw in draws {
            if draw
                .texture
                .as_ref()
                .is_some_and(|texture| !texture.is_valid())
            {
                continue;
            }
            let Some(vertices) = vertices(&draw, camera, matrix, size) else {
                continue;
            };
            let start = data.len() as u32;
            data.extend(vertices);
            let texture = draw
                .texture
                .as_ref()
                .map_or(&pipeline.white, |texture| texture.texture());
            let view = texture.create_view(&Default::default());
            let group = checked(device, || {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                })
            })?;
            commands.push((start..data.len() as u32, group));
        }
        if data.is_empty() {
            return Ok(());
        }
        let buffer = checked(device, || {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("mod world vertices"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::VERTEX,
            })
        })?;
        checked(device, || {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("mod world draws"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: color,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth,
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
            for (range, group) in &commands {
                pass.set_bind_group(0, group, &[]);
                pass.draw(range.clone(), 0..1);
            }
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
}
