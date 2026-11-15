use glam::{DVec3, Mat4};

use super::native_post_shader::{NativePostUniformLayout, PostShaders};
use super::native_services::WorldServices;
use crate::hud::{HudRenderer, native_textures::NativeGuiTextureFilter};

const COPY: &str = "@group(0) @binding(0) var image: texture_2d<f32>; @group(0) @binding(1) var nearest: sampler; @fragment fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return textureSample(image, nearest, uv); }";
const TINT: &str = "@group(0) @binding(0) var image: texture_2d<f32>; @group(0) @binding(1) var nearest: sampler; @group(0) @binding(2) var<uniform> tint: vec4<f32>; @fragment fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { return textureSample(image, nearest, uv) * tint; }";

fn gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = super::test_gpu::instance();
    let adapter = pollster::block_on(instance.request_adapter(&Default::default()))
        .inspect_err(|error| eprintln!("GPU test skipped: {error}"))
        .ok()?;
    eprintln!("Client services GPU test adapter: {:?}", adapter.get_info());
    pollster::block_on(adapter.request_device(&Default::default()))
        .inspect_err(|error| eprintln!("GPU test skipped: {error}"))
        .ok()
}

fn target(device: &wgpu::Device, size: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn clear(
    encoder: &mut wgpu::CommandEncoder,
    color: &wgpu::TextureView,
    depth: Option<&wgpu::TextureView>,
) {
    let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: color,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLUE),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: depth.map(|view| wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load: wgpu::LoadOp::Clear(0.5),
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        ..Default::default()
    });
}

fn pixels(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mut encoder: wgpu::CommandEncoder,
    texture: &wgpu::Texture,
) -> Vec<[u8; 4]> {
    let stride = (texture.width() * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let output = device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(stride) * u64::from(texture.height()),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &output,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(stride),
                rows_per_image: None,
            },
        },
        texture.size(),
    );
    queue.submit([encoder.finish()]);
    output
        .slice(..)
        .map_async(wgpu::MapMode::Read, |result| result.unwrap());
    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
    let mapped = output.slice(..).get_mapped_range().unwrap();
    mapped
        .chunks_exact(stride as usize)
        .flat_map(|row| {
            row[..texture.width() as usize * 4]
                .chunks_exact(4)
                .map(|pixel| pixel.try_into().unwrap())
        })
        .collect()
}

#[test]
fn world_services_gpu_depth_textures_primitives_and_idle_pixels() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let color = target(&device, 16, wgpu::TextureFormat::Rgba8Unorm);
    let depth = device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: color.size(),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    let view = color.create_view(&Default::default());
    let depth_view = depth.create_view(&Default::default());
    let mut services = WorldServices::default();
    let render = |services: &mut WorldServices| {
        let mut encoder = device.create_command_encoder(&Default::default());
        clear(&mut encoder, &view, Some(&depth_view));
        services
            .render(
                &device,
                &queue,
                &mut encoder,
                &view,
                &depth_view,
                Mat4::IDENTITY,
                DVec3::ZERO,
                wgpu::TextureFormat::Rgba8Unorm,
            )
            .unwrap();
        pixels(&device, &queue, encoder, &color)
    };
    assert!(
        render(&mut services)
            .iter()
            .all(|pixel| *pixel == [0, 0, 255, 255])
    );
    let corners = |left: f64, right: f64, depth: f64| {
        [
            [left, 1.0, depth],
            [right, 1.0, depth],
            [right, -1.0, depth],
            [left, -1.0, depth],
        ]
    };
    services
        .draws
        .quad(corners(-1.0, 1.0, 0.25), 0xffff0000)
        .unwrap();
    assert!(
        render(&mut services)
            .iter()
            .all(|pixel| *pixel == [0, 0, 255, 255])
    );
    services
        .draws
        .quad(corners(-1.0, 0.0, 0.75), 0xffff0000)
        .unwrap();
    let atlas = target(&device, 1, wgpu::TextureFormat::Rgba8Unorm);
    let atlas_view = atlas.create_view(&Default::default());
    let sampler = device.create_sampler(&Default::default());
    let hud = HudRenderer::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        &atlas_view,
        &atlas_view,
        &sampler,
    )
    .unwrap();
    let texture = hud
        .native_gui_texture_create(
            &device,
            [1, 1],
            wgpu::TextureFormat::Rgba8Unorm,
            NativeGuiTextureFilter::Nearest,
        )
        .unwrap();
    hud.native_gui_texture_write(&queue, &texture, [0, 0], [1, 1], &[0, 255, 0, 255])
        .unwrap();
    services
        .draws
        .textured_quad(
            corners(0.0, 1.0, 0.75),
            [0.0, 0.0, 1.0, 1.0],
            0xffffffff,
            &texture,
        )
        .unwrap();
    let drawn = render(&mut services);
    assert_eq!(drawn[8 * 16 + 3], [255, 0, 0, 255]);
    assert_eq!(drawn[8 * 16 + 12], [0, 255, 0, 255]);
    services
        .draws
        .line([-1.0, 0.0, 0.75], [1.0, 0.0, 0.75], 0xffff0000, 2.0)
        .unwrap();
    assert_eq!(render(&mut services)[8 * 16 + 8], [255, 0, 0, 255]);
    services
        .draws
        .billboard(
            [0.0, 0.0, 0.75],
            [1.0, 1.0],
            [0.0, 0.0, 1.0, 1.0],
            0xffffffff,
            Some(&texture),
        )
        .unwrap();
    let drawn = render(&mut services);
    assert_eq!(drawn[8 * 16 + 8], [0, 255, 0, 255]);
    assert_eq!(drawn[0], [0, 0, 255, 255]);
    assert!(services.draws.is_empty());
    assert!(
        render(&mut services)
            .iter()
            .all(|pixel| *pixel == [0, 0, 255, 255])
    );
}

#[test]
fn post_services_gpu_validation_uniforms_chaining_resize_and_idle_pixels() {
    let Some((device, queue)) = gpu() else {
        return;
    };
    let mut post = PostShaders::default();
    let color = target(&device, 4, wgpu::TextureFormat::Rgba8Unorm);
    let render = |post: &mut PostShaders, color: &wgpu::Texture| {
        let view = color.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&Default::default());
        clear(&mut encoder, &view, None);
        post.render(&device, &mut encoder, &view).unwrap();
        pixels(&device, &queue, encoder, color)
    };
    let baseline = render(&mut post, &color);
    assert!(baseline.iter().all(|pixel| *pixel == [0, 0, 255, 255]));
    assert!(post.register(&device, wgpu::TextureFormat::Rgba8Unorm, "test:bad", "@fragment fn fs_main(@location(1) missing: vec4<f32>) -> @location(0) vec4<f32> { return missing; }", NativePostUniformLayout::default()).is_err());
    post.register(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        "test:copy",
        COPY,
        NativePostUniformLayout::default(),
    )
    .unwrap();
    post.register(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        "test:tint",
        TINT,
        NativePostUniformLayout { size: 16 },
    )
    .unwrap();
    assert_eq!(render(&mut post, &color), baseline);
    assert!(post.set_uniforms(&queue, "test:tint", &[0; 15]).is_err());
    assert!(post.enable("missing", true).is_err());
    assert!(post.set_uniforms(&queue, "missing", &[]).is_err());
    let tint = [1.0f32, 1.0, 0.5, 1.0];
    post.set_uniforms(&queue, "test:tint", bytemuck::cast_slice(&tint))
        .unwrap();
    post.enable("test:copy", true).unwrap();
    post.enable("test:tint", true).unwrap();
    let tinted = render(&mut post, &color);
    assert!(matches!(tinted[0], [0, 0, 127..=128, 255]));
    assert!(tinted.iter().all(|pixel| *pixel == tinted[0]));
    let resized = target(&device, 8, wgpu::TextureFormat::Rgba8Unorm);
    assert!(
        render(&mut post, &resized)
            .iter()
            .all(|pixel| *pixel == tinted[0])
    );
    let incompatible = target(&device, 1, wgpu::TextureFormat::Bgra8Unorm);
    let mut encoder = device.create_command_encoder(&Default::default());
    assert!(
        post.render(
            &device,
            &mut encoder,
            &incompatible.create_view(&Default::default())
        )
        .is_err()
    );
    queue.submit([encoder.finish()]);
    post.enable("test:copy", false).unwrap();
    post.enable("test:tint", false).unwrap();
    assert_eq!(render(&mut post, &color), baseline);
    assert!(
        post.register(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            "test:copy",
            COPY,
            NativePostUniformLayout::default()
        )
        .is_err()
    );
}
