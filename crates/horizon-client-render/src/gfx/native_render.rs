//! Render hook points for native mods (docs/native-mods.md): empty functions the renderer calls at
//! fixed places of a frame, which a mod's mixins hook like those of
//! `horizon-client/src/game/native_hooks.rs`, and listed with them in `native-hooks.json`. API
//! 58.

use super::Vertex;
use super::pack_draw::EntitySpan;
use glam::{DVec3, Mat4, Vec3};
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, Ordering},
};

mod generated {
    include!(concat!(env!("OUT_DIR"), "/native_render_generated.rs"));
}
pub use generated::{
    HandDrawnEvent, OverlayDrawnEvent, RenderDeviceLostEvent, RenderDeviceReadyEvent, RenderHooks,
    RenderNeedsEvent, WorldDrawnEvent, WorldPrePostEvent,
};
pub use wgpu;

static RENDERERS: OnceLock<Box<[&'static dyn RenderHooks]>> = OnceLock::new();

/// Install trait mod renderers in dispatch order, once per process.
pub fn install(
    renderers: Box<[&'static dyn RenderHooks]>,
) -> Result<(), Box<[&'static dyn RenderHooks]>> {
    RENDERERS.set(renderers)
}

static MODS_BUILT_IN: AtomicBool = AtomicBool::new(false);

/// The game was built with native mods (the game sets it at start): one of them may draw in
/// [`native_world_drawn`], so the first-person hand keeps its own pass after it.
pub fn set_mods_built_in(on: bool) {
    MODS_BUILT_IN.store(on, Ordering::Relaxed);
}
/// Whether [`native_world_drawn`] may draw ([`set_mods_built_in`]).
pub fn world_drawn_hooked() -> bool {
    MODS_BUILT_IN.load(Ordering::Relaxed)
        || RENDERERS
            .get()
            .is_some_and(|renderers| !renderers.is_empty())
}

/// What a mod can draw with once the world is drawn: after the sky, terrain, entities,
/// particles, weather and the world post pass, before the first-person hand ([`HandDrawn`]),
/// the HUD and the screens. `color` holds the world's picture (copy it out before reading it: it is the target
/// drawn into), `depth` the world's depth, reversed (0 is the sky).
#[non_exhaustive]
pub struct WorldDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The frame's target and a view of it in `color_format`.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    /// The size of `color`.
    pub size: [u32; 2],
    /// The world's depth (Depth32Float, sampleable), and its size: smaller than `size` when the
    /// world draws at a lower resolution. It holds the frame's depth only when a mod asked for
    /// it ([`RenderNeeds::world_depth`]) or something else kept it.
    pub depth: &'a wgpu::TextureView,
    pub depth_size: [u32; 2],
    /// Camera-relative world to clip space, without TAA's jitter, and with it (what the world
    /// was drawn with).
    pub view_proj: Mat4,
    pub jittered: Mat4,
    /// The camera's position in the world.
    pub camera: DVec3,
    /// The camera-relative entity geometry of the frame (`Frame::entities`), quads of 4
    /// vertices, and its runs by entity when a mod asked for them
    /// ([`RenderNeeds::entity_spans`]) or a shader pack draws them.
    pub entities: &'a [Vertex],
    pub spans: &'a [EntitySpan],
    /// Which entity each run of `entities` is, recorded every frame (`Frame::entity_motion`):
    /// the run's first vertex and the entity, `(1 << 32) | id` for a mob or a player and
    /// `(2 << 32) | id` for a dropped item (their entity ids), 0 for anything else. A run lasts
    /// until the next. Unlike `spans`, whose ids name a type, these follow one entity from
    /// frame to frame (API @API:world_drawn_motion@).
    pub motion: &'a [(u32, u64)],
    /// The shared atlas entity UVs point into.
    pub atlas: Option<&'a wgpu::Texture>,
    /// Quad indices (0 1 2 0 2 3 for each 4 vertices), for as many quads as `entities` holds.
    pub quad_indices: &'a wgpu::Buffer,
    /// The block textures the terrain samples, for drawing blocks with the player's own
    /// textures (`native_block_sprites` says where a block's are). API @API:native_block_sprites@.
    pub block_sprites: BlockSpriteArrays<'a>,
}

/// The block sprite arrays ([`WorldDrawn::block_sprites`]): each texture a whole square layer,
/// its UVs 0 to 1 across it, as the world uses them (gamma-encoded, animated layers at the frame
/// shown).
#[non_exhaustive]
pub struct BlockSpriteArrays<'a> {
    /// The arrays as `D2Array` views, terrain.wgsl's eight (`sprites`, `sprites_1`...): a 1-layer
    /// stand-in past the ones in use, each with the mip levels the Mipmap Levels option leaves.
    pub views: &'a [wgpu::TextureView],
    /// Layers per array on this device (2048 on most; fewer where the GPU holds fewer, and
    /// OpenGL pads each array past it): a sprite's `array` and `layer` are cut by it.
    pub split: u32,
    /// A layer's width and height in texels.
    pub size: u32,
    /// Nearest, repeating, the nearest mip level: texels drawn sharp.
    pub nearest: &'a wgpu::Sampler,
    /// The terrain's own (linear, clamped, anisotropic under that Texture Filtering).
    pub filtered: &'a wgpu::Sampler,
}

/// What a mod can draw with once the first-person hand is drawn ([`native_hand_drawn`]): after
/// the world and the hand, before entity outlines, post effects, screen effects and the HUD.
/// `color` holds the frame's picture with the hand in it (copy it out before reading it).
#[non_exhaustive]
pub struct HandDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The frame's target and a view of it in `color_format`.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    /// The size of `color`.
    pub size: [u32; 2],
    /// The hand and held item just drawn, camera-relative quads of 4 vertices: empty when no
    /// hand drew (third person, F1, a sleeping or spectating player) or a shader pack drew it.
    pub hand: &'a [Vertex],
    /// Camera-relative world to clip space for the hand: its own field of view
    /// (`hand_fov_degrees`) and the view bob, without TAA's jitter (the hand has none). Its
    /// depth is the hand's own: the hand's pass clears depth and keeps none.
    pub view_proj: Mat4,
    /// The camera's position in the world.
    pub camera: DVec3,
    /// The shared atlas the hand's UVs point into.
    pub atlas: Option<&'a wgpu::Texture>,
    /// Quad indices (0 1 2 0 2 3 for each 4 vertices).
    pub quad_indices: &'a wgpu::Buffer,
}

/// The world before any finishing passes ([`native_world_pre_post`]), after Improved
/// Transparency, before the first-person hand. Draws enter the shader pack's bloom/post chain
/// or the built-in world post pass and TAA. There is no motion-vector target: built-in TAA
/// reprojects with camera motion only, including geometry a mod draws here.
#[non_exhaustive]
pub struct WorldPrePost<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The world colour the next finishing pass reads, not a pack's material/MRT targets.
    /// Copy it out before sampling it while drawing into it. Its format is the renderer's
    /// working format without sRGB, also when scaled or HDR output is enabled.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    /// The world colour and depth size, smaller than the window when scaled.
    pub size: [u32; 2],
    /// The world's stored Depth32Float depth, reversed Z (0 is sky), with
    /// RENDER_ATTACHMENT, TEXTURE_BINDING and COPY_SRC usage. Attach `depth_view` with Load
    /// and Store to test/write against the world; use the world's jittered clip matrix.
    pub depth: &'a wgpu::Texture,
    pub depth_view: &'a wgpu::TextureView,
    /// Camera-relative world to clip space, without TAA's jitter, and with it.
    pub view_proj: Mat4,
    pub jittered: Mat4,
    /// Last consecutive requested world frame's unjittered clip matrix and camera position.
    /// The matrix is relative to `previous_camera`, not this frame's camera. Both are None
    /// on the first call, after a title/unrequested frame, or after device recreation.
    pub previous_view_proj: Option<Mat4>,
    pub previous_camera: Option<DVec3>,
    pub camera: DVec3,
    /// Requested world frames on this renderer, starting at 0 and wrapping at u64::MAX.
    pub frame_index: u64,
    /// The world uniform's sun direction, in world space.
    pub sun_direction: Vec3,
    /// This frame's existing sky lighting and colours. The moon is available as
    /// `Sky::moon_angle` in degrees; the world uniform retains no moon direction vector.
    pub sky: &'a crate::environment::Sky,
    /// The exact world uniform fog colour and distances: environmental start/end, then
    /// render-distance start/end. These include underwater and environmental overrides.
    pub fog_color: [f32; 4],
    pub fog: [f32; 4],
}

/// The final overlay target ([`native_overlay_drawn`]), after HUD, screens, menus, toasts and
/// tooltips. Draws precede screenshot readback, HDR output conversion and presentation.
#[non_exhaustive]
pub struct OverlayDrawn<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    /// The overlay's actual target: normally the swapchain image, but an intermediate for
    /// HDR, frame generation, offscreen rendering or screenshot fallback. COPY_SRC is not
    /// guaranteed: check `color.usage()` before copying. Never sample it as an attachment.
    /// With HDR output this is still gamma-encoded working colour, before HDR10/scRGB
    /// conversion, not the HDR swapchain's output colour space.
    pub color: &'a wgpu::Texture,
    pub color_view: &'a wgpu::TextureView,
    pub color_format: wgpu::TextureFormat,
    pub size: [u32; 2],
    /// Physical pixels per GUI unit, as used by the HUD batch for this frame.
    pub gui_scale: f32,
}

/// What the mods need from a frame that the game doesn't make for itself. Each mod sets what it
/// needs and leaves the rest, so several can ask.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RenderNeeds {
    /// Record `Frame::entity_spans` without a shader pack: entity runs get a stable id from
    /// their type's name, block entity runs their block state.
    pub entity_spans: bool,
    /// Keep the world pass's depth for [`native_world_drawn`] when nothing else does.
    pub world_depth: bool,
    /// Call [`native_world_pre_post`] and retain the world depth for its attachments.
    pub pre_post: bool,
    /// Call [`native_overlay_drawn`] after the final HUD/screen batch, including title frames.
    pub overlay: bool,
}

/// The world is drawn (see [`WorldDrawn`]). Not called on the title screen's panorama.
#[inline(never)]
pub fn native_world_drawn(world: &mut WorldDrawn<'_>) {
    generated::dispatch_world_drawn(RENDERERS.get().map_or(&[], |renderers| renderers), world);
}

/// The first-person hand is drawn (see [`HandDrawn`]), on every frame [`native_world_drawn`]
/// runs, with no hand too. API @API:hand_drawn@.
#[inline(never)]
pub fn native_hand_drawn(hand: &mut HandDrawn<'_>) {
    generated::dispatch_hand_drawn(RENDERERS.get().map_or(&[], |renderers| renderers), hand);
}

/// The world is drawn, before every finishing branch (see [`WorldPrePost`]). Only called
/// when [`RenderNeeds::pre_post`] is set, never on the title panorama. API @API:world_pre_post@.
#[inline(never)]
pub fn native_world_pre_post(world: &mut WorldPrePost<'_>) {
    generated::dispatch_world_pre_post(RENDERERS.get().map_or(&[], |renderers| renderers), world);
}

/// The final HUD/screen batch is drawn (see [`OverlayDrawn`]). Only called when
/// [`RenderNeeds::overlay`] is set, on real and generated overlay frames. API @API:overlay_drawn@.
#[inline(never)]
pub fn native_overlay_drawn(overlay: &mut OverlayDrawn<'_>) {
    generated::dispatch_overlay_drawn(RENDERERS.get().map_or(&[], |renderers| renderers), overlay);
}

/// A device was lost, not deliberately destroyed. Runs synchronously on wgpu's device-lost
/// callback thread, which need not be the render thread. Drop persistent GPU resources using
/// thread-safe state; do not draw here. Horizon attempts recovery, which can fail. References
/// are valid only for this call. API @API:render_device_lost@.
#[inline(never)]
pub fn native_render_device_lost(reason: &wgpu::DeviceLostReason, message: &str) {
    generated::dispatch_render_device_lost(
        RENDERERS.get().map_or(&[], |renderers| renderers),
        reason,
        message,
    );
}

/// A newly opened GPU device is ready, on the device-opening thread, before the renderer is
/// constructed. Runs at initial device creation and successful device-loss recovery, not
/// surface reconfiguration or device reuse. References are valid only for this call; retain
/// cloned handles if needed. API @API:render_device_ready@.
#[inline(never)]
pub fn native_render_device_ready(device: &wgpu::Device, queue: &wgpu::Queue) {
    generated::dispatch_render_device_ready(
        RENDERERS.get().map_or(&[], |renderers| renderers),
        device,
        queue,
    );
}

/// Asked as a frame is built and drawn: set what your mod needs.
#[inline(never)]
pub fn native_render_needs(needs: &mut RenderNeeds) {
    generated::dispatch_render_needs(RENDERERS.get().map_or(&[], |renderers| renderers), needs);
}

/// What the mods need this frame ([`native_render_needs`]).
pub fn render_needs() -> RenderNeeds {
    let mut needs = RenderNeeds::default();
    native_render_needs(&mut needs);
    needs
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Depth;
    impl RenderHooks for Depth {
        fn render_needs(&self, event: &mut RenderNeedsEvent<'_>) {
            event.needs.world_depth = true;
        }
    }
    struct Spans;
    impl RenderHooks for Spans {
        fn render_needs(&self, event: &mut RenderNeedsEvent<'_>) {
            event.needs.entity_spans = event.needs.world_depth;
        }
    }

    struct PrePost;
    impl RenderHooks for PrePost {
        fn render_needs(&self, event: &mut RenderNeedsEvent<'_>) {
            event.needs.pre_post = true;
        }
    }
    struct Overlay;
    impl RenderHooks for Overlay {
        fn render_needs(&self, event: &mut RenderNeedsEvent<'_>) {
            event.needs.overlay = true;
        }
    }

    #[test]
    fn no_mod_needs_no_extra_render_work() {
        let mut needs = RenderNeeds::default();
        generated::dispatch_render_needs(&[], &mut needs);
        assert!(!needs.pre_post);
        assert!(!needs.overlay);
        assert!(!needs.world_depth);
        assert!(!needs.entity_spans);
    }

    #[test]
    fn pre_post_does_not_request_overlay() {
        let mut needs = RenderNeeds::default();
        generated::dispatch_render_needs(&[&PrePost], &mut needs);
        assert_eq!(
            needs,
            RenderNeeds {
                pre_post: true,
                ..RenderNeeds::default()
            }
        );
    }

    #[test]
    fn overlay_does_not_request_pre_post() {
        let mut needs = RenderNeeds::default();
        generated::dispatch_render_needs(&[&Overlay], &mut needs);
        assert_eq!(
            needs,
            RenderNeeds {
                overlay: true,
                ..RenderNeeds::default()
            }
        );
    }

    #[test]
    fn render_hook_needs_accumulate() {
        let mut needs = RenderNeeds::default();
        generated::dispatch_render_needs(&[&PrePost, &Overlay], &mut needs);
        assert_eq!(
            needs,
            RenderNeeds {
                pre_post: true,
                overlay: true,
                ..RenderNeeds::default()
            }
        );
    }

    /// A mod samples `BlockSpriteArrays` as the renderer hands them over: a layer of the second
    /// array through its view, texels sharp and repeating with the nearest sampler.
    #[test]
    fn a_pass_samples_a_sprite_layer_sharp() {
        let instance = crate::gfx::test_gpu::instance();
        let Some((device, queue)) =
            pollster::block_on(instance.request_adapter(&Default::default()))
                .ok()
                .and_then(|adapter| {
                    pollster::block_on(adapter.request_device(&Default::default())).ok()
                })
        else {
            return;
        };
        let gl = device.adapter_info().backend == wgpu::Backend::Gl;
        // two arrays of two 2 by 2 layers; array 1's layer 1 red, green / blue, white
        let layers = super::super::array_layers(2, gl);
        let quad = [
            [255, 0, 0, 255],
            [0, 255, 0, 255],
            [0, 0, 255, 255],
            [255; 4],
        ];
        let textures: Vec<wgpu::Texture> = (0..2)
            .map(|array| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: 2,
                        height: 2,
                        depth_or_array_layers: layers,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                });
                let mut texels = vec![0u8; 16 * layers as usize];
                if array == 1 {
                    texels[16..32].copy_from_slice(quad.as_flattened());
                }
                queue.write_texture(
                    texture.as_image_copy(),
                    &texels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(8),
                        rows_per_image: Some(2),
                    },
                    texture.size(),
                );
                texture
            })
            .collect();
        let unused = textures[0].create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let views = super::super::sprite_views(&textures, &unused, Some(1));
        let nearest = device.create_sampler(&super::super::SPRITE_NEAREST);
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                "@group(0) @binding(0) var sprites: texture_2d_array<f32>;
                 @group(0) @binding(1) var sprites_1: texture_2d_array<f32>;
                 @group(0) @binding(2) var nearest: sampler;
                 @vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
                     return vec4(f32(i & 1u) * 4.0 - 1.0, f32(i >> 1u) * 4.0 - 1.0, 0.0, 1.0);
                 }
                 @fragment fn fs(@builtin(position) at: vec4<f32>) -> @location(0) vec4<f32> {
                     // inside each texel, and past the edge (repeating); the last reads array 0
                     let uv = array(vec2(1.3, 0.2), vec2(0.7, 0.45), vec2(-0.8, 0.9), vec2(0.6, 0.6));
                     let x = u32(at.x);
                     if x == 4u {
                         return textureSampleLevel(sprites, nearest, vec2(0.3), 1, 0.0);
                     }
                     return textureSampleLevel(sprites_1, nearest, uv[x], 1, 0.0);
                 }"
                .into(),
            ),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&nearest),
                },
            ],
        });
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: None,
            size: wgpu::Extent3d {
                width: 5,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let out = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: u64::from(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        {
            let view = target.create_view(&Default::default());
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &out,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                    rows_per_image: None,
                },
            },
            target.size(),
        );
        queue.submit([encoder.finish()]);
        out.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        let texels = out.slice(..).get_mapped_range().unwrap()[..20].to_vec();
        let mut expected = quad.as_flattened().to_vec();
        expected.extend([0, 0, 0, 0]);
        assert_eq!(texels, expected);
    }

    #[test]
    fn needs_accumulate_in_mod_order() {
        let mut needs = RenderNeeds::default();
        generated::dispatch_render_needs(&[&Depth, &Spans], &mut needs);
        assert_eq!(
            needs,
            RenderNeeds {
                world_depth: true,
                entity_spans: true,
                ..RenderNeeds::default()
            }
        );
    }
}
