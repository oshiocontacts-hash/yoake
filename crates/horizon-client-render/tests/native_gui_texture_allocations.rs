#![cfg(feature = "test-support")]

use horizon_client_render::hud::{
    Blend, HudBatch, HudRenderer, HudTestUploads, Rect,
    native_textures::{NativeGuiNineSlice, NativeGuiTexture, NativeGuiTextureFilter},
};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    ops::Range,
};

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct CountingAllocator;

impl CountingAllocator {
    fn record() {
        if COUNTING.try_with(Cell::get).unwrap_or(false) {
            let _ = ALLOCATIONS.try_with(|allocations| allocations.set(allocations.get() + 1));
        }
    }
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        CountingAllocator::record();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        CountingAllocator::record();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        CountingAllocator::record();
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

struct CountingScope;

impl Drop for CountingScope {
    fn drop(&mut self) {
        COUNTING.with(|counting| counting.set(false));
    }
}

fn allocations(operation: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|allocations| allocations.set(0));
    COUNTING.with(|counting| assert!(!counting.replace(true)));
    let scope = CountingScope;
    operation();
    drop(scope);
    ALLOCATIONS.with(Cell::get)
}

#[test]
fn counter_observes_allocations_and_stops_between_scopes() {
    assert_eq!(allocations(|| {}), 0);
    assert_eq!(
        allocations(|| {
            std::hint::black_box(Box::new([0_u8; 64]));
        }),
        1
    );
    assert_eq!(allocations(|| {}), 0);
}

#[derive(Clone, Copy, Debug)]
enum DrawCase {
    Rectangle,
    NineSlice,
    BlendSplit,
    BlurSplit,
    Alternating,
}

struct FrameAllocations {
    append: usize,
    prepare: usize,
    draw: usize,
    submit: usize,
    recycle: usize,
}

fn atlas_rect(batch: &mut HudBatch, rect: [f32; 4]) {
    let [x, y, width, height] = rect;
    batch.rect_uv(
        x,
        y,
        width,
        height,
        Rect {
            u0: 0.0,
            v0: 0.0,
            u1: 1.0,
            v1: 1.0,
            w: 3,
            h: 3,
        },
        u32::MAX,
    );
}

fn native_rect(batch: &mut HudBatch, texture: &NativeGuiTexture, nine_slice: bool) {
    assert!(batch.native_gui_texture(
        texture,
        [4.0, 4.0, 8.0, 8.0],
        [0.0, 0.0, 1.0, 1.0],
        u32::MAX,
        nine_slice.then_some(NativeGuiNineSlice {
            source_size: [3, 3],
            border: [1; 4],
        }),
    ));
}

struct FrameHarness {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: HudRenderer,
    uploads: HudTestUploads,
    target: wgpu::TextureView,
    depth: wgpu::TextureView,
    texture: NativeGuiTexture,
}

impl FrameHarness {
    fn new() -> Self {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&Default::default())).unwrap();
        let (device, queue) =
            pollster::block_on(adapter.request_device(&Default::default())).unwrap();
        let atlas = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("allocation test atlas"),
            size: wgpu::Extent3d {
                width: 3,
                height: 3,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let atlas_view = atlas.create_view(&Default::default());
        let sampler = device.create_sampler(&Default::default());
        let renderer = HudRenderer::new(
            &device,
            wgpu::TextureFormat::Rgba8Unorm,
            &atlas_view,
            &atlas_view,
            &sampler,
        )
        .unwrap();
        let texture = renderer
            .native_gui_texture_create(
                &device,
                [3, 3],
                wgpu::TextureFormat::Rgba8Unorm,
                NativeGuiTextureFilter::Nearest,
            )
            .unwrap();
        let descriptor = wgpu::TextureDescriptor {
            label: Some("allocation test target"),
            size: wgpu::Extent3d {
                width: 32,
                height: 32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        };
        let target = device
            .create_texture(&descriptor)
            .create_view(&Default::default());
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("allocation test depth"),
                format: wgpu::TextureFormat::Depth32Float,
                ..descriptor
            })
            .create_view(&Default::default());
        let uploads = HudTestUploads::new(&device);
        Self {
            device,
            queue,
            renderer,
            uploads,
            target,
            depth,
            texture,
        }
    }

    fn frame(&mut self, case: DrawCase, native: bool) -> FrameAllocations {
        let size = [32, 32];
        let mut batch = HudBatch::new(size);
        batch.vertices.reserve(128);
        atlas_rect(&mut batch, [0.0, 0.0, 4.0, 4.0]);
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let attachments = [Some(wgpu::RenderPassColorAttachment {
                view: &self.target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })];
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &attachments,
                ..Default::default()
            });
        }
        let append = allocations(|| {
            if native {
                native_rect(
                    &mut batch,
                    &self.texture,
                    matches!(case, DrawCase::NineSlice),
                );
            }
            atlas_rect(&mut batch, [16.0, 16.0, 4.0, 4.0]);
            if matches!(case, DrawCase::BlendSplit) {
                batch.blend_since(6, Blend::Additive);
            }
            if matches!(case, DrawCase::BlurSplit) {
                batch.blur_at = 6;
            }
            if matches!(case, DrawCase::Alternating) {
                if native {
                    native_rect(&mut batch, &self.texture, false);
                }
                atlas_rect(&mut batch, [24.0, 24.0, 4.0, 4.0]);
            }
        });
        let prepare = allocations(|| {
            self.renderer
                .prepare(&self.device, &mut self.uploads, &mut encoder, &batch, size);
        });
        let draw = allocations(|| {
            let end = batch.vertices.len() as u32;
            let ranges = if matches!(case, DrawCase::BlurSplit) {
                [0..batch.blur_at, batch.blur_at..end]
            } else {
                [0..end, Range::default()]
            };
            for range in ranges {
                self.renderer.draw_range(
                    &mut encoder,
                    &self.target,
                    &self.depth,
                    &batch,
                    size,
                    range,
                    None,
                );
            }
        });
        let recycle = allocations(|| drop(batch));
        let submit = allocations(|| {
            self.uploads.finish(&encoder);
            self.queue.submit([encoder.finish()]);
        });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .unwrap();
        FrameAllocations {
            append,
            prepare,
            draw,
            submit,
            recycle,
        }
    }
}

#[test]
#[ignore = "requires a host GPU; run with --ignored --nocapture"]
fn native_gui_texture_frames_allocate_no_more_than_atlas_only_frames() {
    let mut harness = FrameHarness::new();
    for case in [
        DrawCase::Rectangle,
        DrawCase::NineSlice,
        DrawCase::BlendSplit,
        DrawCase::BlurSplit,
        DrawCase::Alternating,
    ] {
        for _ in 0..8 {
            harness.frame(case, false);
        }
        harness.frame(case, true);
        for frame in 0..8 {
            let baseline = harness.frame(case, false);
            let native = harness.frame(case, true);
            assert!(
                native.append <= baseline.append,
                "{case:?}, frame {frame}: append {} > {}",
                native.append,
                baseline.append
            );
            if !matches!(case, DrawCase::BlendSplit) {
                assert_eq!(native.append, 0, "{case:?}, frame {frame}: append");
                assert_eq!(baseline.append, 0, "{case:?}, frame {frame}: baseline");
            }
            assert_eq!(native.recycle, 0, "{case:?}, frame {frame}: recycle");
            assert!(
                native.prepare <= baseline.prepare,
                "{case:?}, frame {frame}: prepare {} > {}",
                native.prepare,
                baseline.prepare
            );
            assert!(
                native.draw <= baseline.draw,
                "{case:?}, frame {frame}: draw {} > {}",
                native.draw,
                baseline.draw
            );
            assert!(
                native.submit <= baseline.submit,
                "{case:?}, frame {frame}: submit {} > {}",
                native.submit,
                baseline.submit
            );
        }
    }
}
