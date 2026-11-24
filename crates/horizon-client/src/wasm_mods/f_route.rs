//! REM-12: the GPU of full-access (route F) modules. One `GameHost` with the GPU on, for all F modules of the client
//! session, backs `game::yoake2_f_services`' `gpu_*` bindings: the same `yoake_wasm::GpuServices` code the S route
//! runs, with the same quotas, ownership and recording, under one GPU identity ([`F_ID`]). The renderer calls the
//! registered callback once per frame, and that callback enters the F `GPU_FRAME` hook (`yoake2_mods`).

use super::{GameHost, gpu_services};
use crate::game::yoake2_f_services as f;
use std::cell::RefCell;
use std::sync::Arc;
use yoake_wasm::gpu_types as t;
use yoake_wasm::GpuServices;

/// The GPU identity (and the package name the renderer logs) of every F client module.
pub(crate) const F_ID: &str = "yoake-f-client";

thread_local! {
    static HOST: RefCell<Option<FGpuHost>> = const { RefCell::new(None) };
}

/// The F modules' GPU: a `GameHost` (the S route's backend) taking the F wire types.
pub(crate) struct FGpuHost {
    host: GameHost,
}

fn pixel(format: f::FPixelFormat) -> t::PixelFormat {
    match format {
        f::FPixelFormat::Rgba8 => t::PixelFormat::Rgba8,
        f::FPixelFormat::Rgba8Srgb => t::PixelFormat::Rgba8Srgb,
        f::FPixelFormat::Rgba16f => t::PixelFormat::Rgba16f,
        f::FPixelFormat::Rgba32f => t::PixelFormat::Rgba32f,
        f::FPixelFormat::R32f => t::PixelFormat::R32f,
        f::FPixelFormat::Rg16f => t::PixelFormat::Rg16f,
        f::FPixelFormat::Depth32f => t::PixelFormat::Depth32f,
        f::FPixelFormat::Bgra8 => t::PixelFormat::Bgra8,
        f::FPixelFormat::Rgb10a2 => t::PixelFormat::Rgb10a2,
    }
}

fn pixel_back(format: t::PixelFormat) -> f::FPixelFormat {
    match format {
        t::PixelFormat::Rgba8 => f::FPixelFormat::Rgba8,
        t::PixelFormat::Rgba8Srgb => f::FPixelFormat::Rgba8Srgb,
        t::PixelFormat::Rgba16f => f::FPixelFormat::Rgba16f,
        t::PixelFormat::Rgba32f => f::FPixelFormat::Rgba32f,
        t::PixelFormat::R32f => f::FPixelFormat::R32f,
        t::PixelFormat::Rg16f => f::FPixelFormat::Rg16f,
        t::PixelFormat::Depth32f => f::FPixelFormat::Depth32f,
        t::PixelFormat::Bgra8 => f::FPixelFormat::Bgra8,
        t::PixelFormat::Rgb10a2 => f::FPixelFormat::Rgb10a2,
    }
}

fn stage(stage: f::FStage) -> t::Stage {
    match stage {
        f::FStage::WorldOpaque => t::Stage::WorldOpaque,
        f::FStage::WorldTranslucent => t::Stage::WorldTranslucent,
        f::FStage::AfterWorld => t::Stage::AfterWorld,
        f::FStage::Hud => t::Stage::Hud,
    }
}

fn range(range: Option<f::FBufferRange>) -> Option<t::BufferRange> {
    range.map(|r| t::BufferRange { handle: r.handle, offset: r.offset, size: r.size })
}

fn rect(rect: Option<f::FGpuRect>) -> Option<t::GpuRect> {
    rect.map(|r| t::GpuRect { height: r.height, width: r.width, x: r.x, y: r.y })
}

fn draw_desc(desc: f::FDrawDesc) -> t::DrawDesc {
    t::DrawDesc {
        count: desc.count,
        groups: desc.groups,
        index: range(desc.index),
        index32: desc.index32,
        instances: desc.instances,
        pipeline: desc.pipeline,
        vertex: range(desc.vertex),
    }
}

fn target(target: f::FDrawTarget) -> t::DrawTarget {
    t::DrawTarget { clear: target.clear, color: target.color, depth: target.depth }
}

fn shader_error(error: t::ShaderError) -> f::FShaderError {
    f::FShaderError { column: error.column, line: error.line, message: error.message }
}

impl FGpuHost {
    pub(crate) fn shader(&mut self, source: String) -> Result<u32, f::FShaderError> {
        GpuServices::shader(&mut self.host, source).map_err(shader_error)
    }

    pub(crate) fn shader_file(&mut self, path: String, source: String) -> Result<u32, f::FShaderError> {
        GpuServices::shader_file(&mut self.host, path, source).map_err(shader_error)
    }

    pub(crate) fn buffer(&mut self, size: u64, usage: u32) -> Result<u32, String> {
        GpuServices::buffer(&mut self.host, size, usage)
    }

    pub(crate) fn texture(&mut self, desc: f::FTextureDesc) -> Result<u32, String> {
        GpuServices::texture(
            &mut self.host,
            t::TextureDesc {
                depth: desc.depth,
                format: pixel(desc.format),
                height: desc.height,
                mips: desc.mips,
                usage: desc.usage,
                volume: desc.volume,
                width: desc.width,
            },
        )
    }

    pub(crate) fn sampler(&mut self, linear: bool, repeat: bool) -> Result<u32, String> {
        GpuServices::sampler(&mut self.host, linear, repeat)
    }

    pub(crate) fn compute(&mut self, entry: String, shader: u32) -> Result<u32, String> {
        GpuServices::compute(&mut self.host, entry, shader)
    }

    pub(crate) fn render(&mut self, desc: f::FRenderDesc) -> Result<u32, String> {
        GpuServices::render(
            &mut self.host,
            t::RenderDesc {
                attributes: desc.attributes.into_iter().map(|a| t::Attribute { components: a.components, location: a.location, offset: a.offset }).collect(),
                blend: desc.blend,
                cull: desc.cull,
                depth: desc.depth,
                depth_write: desc.depth_write,
                format: pixel(desc.format),
                fragment_entry: desc.fragment_entry,
                lines: desc.lines,
                shader: desc.shader,
                stride: desc.stride,
                vertex_entry: desc.vertex_entry,
            },
        )
    }

    pub(crate) fn bind_group(&mut self, bindings: Vec<f::FBinding>, index: u32, pipeline: u32) -> Result<u32, String> {
        let bindings = bindings
            .into_iter()
            .map(|b| t::Binding {
                binding: b.binding,
                handle: b.handle,
                kind: match b.kind {
                    f::FBindingKind::Buffer => t::BindingKind::Buffer,
                    f::FBindingKind::Texture => t::BindingKind::Texture,
                    f::FBindingKind::Sampler => t::BindingKind::Sampler,
                },
                offset: b.offset,
                size: b.size,
            })
            .collect();
        GpuServices::bind_group(&mut self.host, bindings, index, pipeline)
    }

    pub(crate) fn destroy(&mut self, handle: u32) -> Result<(), String> {
        GpuServices::destroy(&mut self.host, handle)
    }

    pub(crate) fn write_buffer(&mut self, bytes: Vec<u8>, handle: u32, offset: u64, at: f::FStage) -> Result<(), String> {
        GpuServices::write_buffer(&mut self.host, bytes, handle, offset, stage(at))
    }

    pub(crate) fn copy_buffer(&mut self, destination: u32, destination_offset: u64, size: u64, source: u32, source_offset: u64, at: f::FStage) -> Result<(), String> {
        GpuServices::copy_buffer(&mut self.host, destination, destination_offset, size, source, source_offset, stage(at))
    }

    pub(crate) fn write_texture(&mut self, bytes: Vec<u8>, extent: (u32, u32, u32), handle: u32, mip: u32, origin: (u32, u32, u32), at: f::FStage) -> Result<(), String> {
        GpuServices::write_texture(&mut self.host, bytes, extent, handle, mip, origin, stage(at))
    }

    pub(crate) fn dispatch(&mut self, groups: Vec<u32>, pipeline: u32, at: f::FStage, workgroups: (u32, u32, u32)) -> Result<(), String> {
        GpuServices::dispatch(&mut self.host, groups, pipeline, stage(at), workgroups)
    }

    pub(crate) fn draw(&mut self, desc: f::FDrawDesc, at: f::FStage, to: f::FDrawTarget) -> Result<(), String> {
        GpuServices::draw(&mut self.host, draw_desc(desc), stage(at), target(to))
    }

    pub(crate) fn draw_region(&mut self, desc: f::FDrawDesc, region: f::FDrawRegion, at: f::FStage, to: f::FDrawTarget) -> Result<(), String> {
        let region = t::DrawRegion { scissor: rect(region.scissor), viewport: rect(region.viewport) };
        GpuServices::draw_region_call(&mut self.host, draw_desc(desc), region, stage(at), target(to))
    }

    pub(crate) fn read(&mut self, handle: u32, offset: u64, size: u64, at: f::FStage) -> Result<u32, String> {
        GpuServices::read(&mut self.host, handle, offset, size, stage(at))
    }

    pub(crate) fn poll(&mut self, ticket: u32) -> Result<Option<Vec<u8>>, String> {
        GpuServices::poll(&mut self.host, ticket)
    }

    pub(crate) fn frame(&mut self) -> Result<f::FFrameInfo, String> {
        let info = GpuServices::frame(&mut self.host)?;
        Ok(f::FFrameInfo {
            camera: info.camera,
            delta_seconds: info.delta_seconds,
            height: info.height,
            hud_format: info.hud_format.map(pixel_back),
            index: info.index,
            version: info.version,
            view_projection: info.view_projection,
            width: info.width,
            world_format: info.world_format.map(pixel_back),
        })
    }
}

/// Runs `operation` on the session's F GPU, made on first use. An error of the service itself is the `Err`.
pub(crate) fn with_f_gpu<R>(operation: impl FnOnce(&mut FGpuHost) -> Result<R, String>) -> Result<R, String> {
    HOST.with(|slot| {
        let mut slot = slot.try_borrow_mut().map_err(|_| "GPU services cannot be re-entered".to_owned())?;
        let host = slot.get_or_insert_with(|| FGpuHost { host: GameHost::new(F_ID).with_gpu(true) });
        operation(host)
    })
}

/// Registers the F package with the renderer: from now on `callback` runs once per frame, on the render thread, and
/// the renderer owns the GPU resources of [`F_ID`]. Idempotent per call (a later call replaces the callback).
pub(crate) fn register_f_gpu(fingerprint: &str, callback: Arc<dyn Fn() + Send + Sync>) -> bool {
    let Some(owner) = gpu_services::owner(F_ID) else {
        return false;
    };
    crate::gfx::mod_gpu_runtime::register_package(owner, F_ID, fingerprint, callback);
    true
}

/// Session end: the renderer forgets the package and every GPU resource of it, and the host is dropped.
pub(crate) fn release_f_gpu() {
    if let Some(owner) = gpu_services::owner(F_ID) {
        crate::gfx::mod_gpu_runtime::remove(owner);
    }
    let host = HOST.with(|slot| slot.borrow_mut().take());
    if let Some(mut host) = host {
        yoake_wasm::ClientHost::release_resources(&mut host.host);
    }
}
