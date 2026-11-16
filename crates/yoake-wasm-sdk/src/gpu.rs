use super::{gpu_raw as raw, gpu_types as types};
use crate::prelude::*;

pub use types::{Attribute, FrameInfo, PixelFormat, ShaderError, Stage};

macro_rules! usage {
    ($name:ident { $($flag:ident = $value:expr),* $(,)? }) => {
        #[derive(Clone, Copy, Debug, Default)]
        pub struct $name(u32);
        impl $name { $(pub const $flag: Self = Self($value);)* }
        impl core::ops::BitOr for $name {
            type Output = Self;
            fn bitor(self, other: Self) -> Self { Self(self.0 | other.0) }
        }
    };
}

usage!(BufferUsage {
    COPY_SRC = 4, COPY_DST = 8, INDEX = 16, VERTEX = 32, UNIFORM = 64,
    STORAGE = 128,
});
usage!(TextureUsage {
    COPY_SRC = 1, COPY_DST = 2, SAMPLED = 4, STORAGE = 8, RENDER = 16,
});

macro_rules! resource {
    ($($name:ident),* $(,)?) => {$(
        #[derive(Debug)]
        pub struct $name { handle: u32 }
        impl Drop for $name {
            fn drop(&mut self) { let _ = raw::destroy(self.handle); }
        }
    )*};
}

resource!(
    Buffer,
    Texture,
    Sampler,
    Shader,
    ComputePipeline,
    RenderPipeline,
    BindGroup
);

impl Buffer {
    pub fn create(size: u64, usage: BufferUsage) -> Result<Self, String> {
        raw::buffer(size, usage.0).map(|handle| Self { handle })
    }

    pub fn write(&self, stage: Stage, offset: u64, bytes: &[u8]) -> Result<(), String> {
        raw::write_buffer(bytes, self.handle, offset, stage)
    }

    pub fn copy_to(
        &self,
        stage: Stage,
        offset: u64,
        destination: &Self,
        destination_offset: u64,
        size: u64,
    ) -> Result<(), String> {
        raw::copy_buffer(
            destination.handle,
            destination_offset,
            size,
            self.handle,
            offset,
            stage,
        )
    }

    pub fn range(&self, offset: u64, size: u64) -> BufferRange<'_> {
        BufferRange {
            buffer: self,
            offset,
            size,
        }
    }

    pub fn read(&self, stage: Stage, offset: u64, size: u64) -> Result<Readback, String> {
        raw::read(self.handle, offset, size, stage).map(|handle| Readback {
            handle: Some(handle),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct BufferRange<'a> {
    buffer: &'a Buffer,
    pub offset: u64,
    pub size: u64,
}

impl BufferRange<'_> {
    fn raw(self) -> types::BufferRange {
        types::BufferRange {
            handle: self.buffer.handle,
            offset: self.offset,
            size: self.size,
        }
    }
}

pub struct TextureDescriptor {
    pub width: u32,
    pub height: u32,
    pub depth: u32,
    pub mips: u32,
    pub volume: bool,
    pub format: PixelFormat,
    pub usage: TextureUsage,
}

impl Texture {
    pub fn create(desc: &TextureDescriptor) -> Result<Self, String> {
        raw::texture(types::TextureDesc {
            width: desc.width,
            height: desc.height,
            depth: desc.depth,
            mips: desc.mips,
            volume: desc.volume,
            format: desc.format,
            usage: desc.usage.0,
        })
        .map(|handle| Self { handle })
    }

    pub fn write(
        &self,
        stage: Stage,
        mip: u32,
        origin: (u32, u32, u32),
        extent: (u32, u32, u32),
        bytes: &[u8],
    ) -> Result<(), String> {
        raw::write_texture(bytes, extent, self.handle, mip, origin, stage)
    }
}

impl Sampler {
    pub fn create(linear: bool, repeat: bool) -> Result<Self, String> {
        raw::sampler(linear, repeat).map(|handle| Self { handle })
    }
}

impl Shader {
    /// Compiles validated WGSL. Host diagnostics include the source line and column.
    pub fn create(wgsl: &str) -> Result<Self, ShaderError> {
        raw::shader(wgsl).map(|handle| Self { handle })
    }

    /// Uses a confined, watched WGSL file in unsigned development mode, or the embedded source otherwise.
    pub fn from_file(path: &str, embedded_wgsl: &str) -> Result<Self, ShaderError> {
        raw::shader_file(path, embedded_wgsl).map(|handle| Self { handle })
    }
}

impl ComputePipeline {
    pub fn create(shader: &Shader, entry: &str) -> Result<Self, String> {
        raw::compute(entry, shader.handle).map(|handle| Self { handle })
    }

    pub fn dispatch(
        &self,
        stage: Stage,
        groups: &[&BindGroup],
        workgroups: (u32, u32, u32),
    ) -> Result<(), String> {
        raw::dispatch(
            &groups.iter().map(|group| group.handle).collect::<Vec<_>>(),
            self.handle,
            stage,
            workgroups,
        )
    }
}

pub struct RenderDescriptor<'a> {
    pub shader: &'a Shader,
    pub vertex_entry: &'a str,
    pub fragment_entry: &'a str,
    pub stride: u64,
    pub attributes: &'a [Attribute],
    pub format: PixelFormat,
    pub depth: bool,
    pub depth_write: bool,
    pub blend: bool,
    pub cull: bool,
    pub lines: bool,
}

impl RenderPipeline {
    pub fn create(desc: &RenderDescriptor<'_>) -> Result<Self, String> {
        raw::render(&types::RenderDesc {
            shader: desc.shader.handle,
            vertex_entry: desc.vertex_entry.into(),
            fragment_entry: desc.fragment_entry.into(),
            stride: desc.stride,
            attributes: desc.attributes.to_vec(),
            format: desc.format,
            depth: desc.depth,
            depth_write: desc.depth_write,
            blend: desc.blend,
            cull: desc.cull,
            lines: desc.lines,
        })
        .map(|handle| Self { handle })
    }

    pub fn draw(&self, stage: Stage, draw: &Draw<'_>, target: &Target<'_>) -> Result<(), String> {
        raw::draw(
            &types::DrawDesc {
                pipeline: self.handle,
                count: draw.count,
                instances: draw.instances,
                vertex: draw.vertex.map(BufferRange::raw),
                index: draw.index.map(BufferRange::raw),
                index32: draw.index32,
                groups: draw.groups.iter().map(|group| group.handle).collect(),
            },
            stage,
            types::DrawTarget {
                color: target.color.map(|texture| texture.handle),
                depth: target.depth.map(|texture| texture.handle),
                clear: target.clear,
            },
        )
    }
}

pub enum Pipeline<'a> {
    Compute(&'a ComputePipeline),
    Render(&'a RenderPipeline),
}
impl<'a> From<&'a ComputePipeline> for Pipeline<'a> {
    fn from(pipeline: &'a ComputePipeline) -> Self {
        Self::Compute(pipeline)
    }
}
impl<'a> From<&'a RenderPipeline> for Pipeline<'a> {
    fn from(pipeline: &'a RenderPipeline) -> Self {
        Self::Render(pipeline)
    }
}

pub enum Binding<'a> {
    Buffer {
        binding: u32,
        range: BufferRange<'a>,
    },
    Texture {
        binding: u32,
        texture: &'a Texture,
    },
    Sampler {
        binding: u32,
        sampler: &'a Sampler,
    },
}

impl BindGroup {
    pub fn create<'a>(
        pipeline: impl Into<Pipeline<'a>>,
        index: u32,
        bindings: &[Binding<'_>],
    ) -> Result<Self, String> {
        let pipeline = match pipeline.into() {
            Pipeline::Compute(pipeline) => pipeline.handle,
            Pipeline::Render(pipeline) => pipeline.handle,
        };
        let bindings = bindings
            .iter()
            .map(|binding| match binding {
                Binding::Buffer { binding, range } => types::Binding {
                    binding: *binding,
                    handle: range.buffer.handle,
                    offset: range.offset,
                    size: range.size,
                    kind: types::BindingKind::Buffer,
                },
                Binding::Texture { binding, texture } => types::Binding {
                    binding: *binding,
                    handle: texture.handle,
                    offset: 0,
                    size: 0,
                    kind: types::BindingKind::Texture,
                },
                Binding::Sampler { binding, sampler } => types::Binding {
                    binding: *binding,
                    handle: sampler.handle,
                    offset: 0,
                    size: 0,
                    kind: types::BindingKind::Sampler,
                },
            })
            .collect::<Vec<_>>();
        raw::bind_group(&bindings, index, pipeline).map(|handle| Self { handle })
    }
}

pub struct Draw<'a> {
    pub count: u32,
    pub instances: u32,
    pub vertex: Option<BufferRange<'a>>,
    pub index: Option<BufferRange<'a>>,
    pub index32: bool,
    pub groups: &'a [&'a BindGroup],
}

/// `None` attachments select the current engine stage. Explicit textures render offscreen.
#[derive(Default)]
pub struct Target<'a> {
    pub color: Option<&'a Texture>,
    pub depth: Option<&'a Texture>,
    pub clear: Option<(f64, f64, f64, f64)>,
}

/// Poll on later frames; a completed ticket is consumed exactly once.
#[derive(Debug)]
pub struct Readback {
    handle: Option<u32>,
}

impl Readback {
    pub fn poll(&mut self) -> Result<Option<Vec<u8>>, String> {
        let handle = self
            .handle
            .ok_or_else(|| String::from("readback already consumed"))?;
        let result = raw::poll(handle);
        if !matches!(result, Ok(None)) {
            self.handle = None;
        }
        result
    }
}

impl Drop for Readback {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = raw::destroy(handle);
        }
    }
}

pub fn frame() -> Result<FrameInfo, String> {
    raw::frame()
}

/// Group 3, binding 0: ABI v1, exactly 576 bytes. Append resource declarations as needed.
pub const ENGINE_WGSL: &str = r#"
struct EngineFrame {
    header: vec4<u32>,
    view: mat4x4<f32>,
    projection: mat4x4<f32>,
    view_projection: mat4x4<f32>,
    inverse_view_projection: mat4x4<f32>,
    previous_view_projection: mat4x4<f32>,
    jittered_view_projection: mat4x4<f32>,
    inverse_jittered_view_projection: mat4x4<f32>,
    camera_time: vec4<f32>,
    viewport_delta: vec4<f32>,
    sun: vec4<f32>,
    moon: vec4<f32>,
    fog_color: vec4<f32>,
    fog_range: vec4<f32>,
    jitter: vec4<f32>,
}
@group(3) @binding(0) var<uniform> engine: EngineFrame;
"#;
