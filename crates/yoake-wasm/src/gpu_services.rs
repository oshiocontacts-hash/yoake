use crate::full_bindings::client::yoake::mods::gpu_services;
pub use crate::full_bindings::client::yoake::mods::gpu_types;
use crate::host::ClientContext;
use gpu_types::*;

macro_rules! services {
    ($($name:ident($($argument:ident: $kind:ty),*) -> $result:ty;)*) => {
        pub trait GpuServices: Send {
            fn shader(&mut self, source: String) -> Result<u32, ShaderError>;
            fn shader_file(&mut self, path: String, source: String) -> Result<u32, ShaderError>;
            $(fn $name(&mut self, $($argument: $kind),*) -> Result<$result, String>;)*
        }

        impl gpu_services::Host for ClientContext {
            fn shader(&mut self, source: String) -> Result<u32, ShaderError> {
                let started = std::time::Instant::now();
                let cpu_started = crate::thread_cpu_time();
                let result = match self.host.gpu_services() {
                    Some(host) => host.shader(source),
                    None => Err(ShaderError { message: "service.gpu permission is required".to_owned(), line: 0, column: 0 }),
                };
                crate::add_counted_host_time(started.elapsed(), crate::cpu_elapsed(cpu_started));
                result
            }
            fn shader_file(&mut self, path: String, source: String) -> Result<u32, ShaderError> {
                let started = std::time::Instant::now();
                let cpu_started = crate::thread_cpu_time();
                let result = match self.host.gpu_services() {
                    Some(host) => host.shader_file(path, source),
                    None => Err(ShaderError { message: "service.gpu permission is required".to_owned(), line: 0, column: 0 }),
                };
                crate::add_counted_host_time(started.elapsed(), crate::cpu_elapsed(cpu_started));
                result
            }
            $(fn $name(&mut self, $($argument: $kind),*) -> Result<$result, String> {
                let started = std::time::Instant::now();
                let cpu_started = crate::thread_cpu_time();
                let result = match self.host.gpu_services() {
                    Some(host) => host.$name($($argument),*),
                    None => Err("service.gpu permission is required".to_owned()),
                };
                crate::add_counted_host_time(started.elapsed(), crate::cpu_elapsed(cpu_started));
                result
            })*
        }
    };
}

impl gpu_types::Host for ClientContext {}

services! {
    buffer(size: u64, usage: u32) -> u32;
    texture(desc: TextureDesc) -> u32;
    sampler(linear: bool, repeat: bool) -> u32;
    compute(entry: String, shader: u32) -> u32;
    render(desc: RenderDesc) -> u32;
    bind_group(bindings: Vec<Binding>, index: u32, pipeline: u32) -> u32;
    destroy(handle: u32) -> ();
    write_buffer(bytes: Vec<u8>, handle: u32, offset: u64, stage: Stage) -> ();
    copy_buffer(destination: u32, destination_offset: u64, size: u64, source: u32, source_offset: u64, stage: Stage) -> ();
    write_texture(bytes: Vec<u8>, extent: (u32, u32, u32), handle: u32, mip: u32, origin: (u32, u32, u32), stage: Stage) -> ();
    dispatch(groups: Vec<u32>, pipeline: u32, stage: Stage, workgroups: (u32, u32, u32)) -> ();
    draw(desc: DrawDesc, stage: Stage, target: DrawTarget) -> ();
    draw_region_call(desc: DrawDesc, region: DrawRegion, stage: Stage, target: DrawTarget) -> ();
    read(handle: u32, offset: u64, size: u64, stage: Stage) -> u32;
    poll(ticket: u32) -> Option<Vec<u8>>;
    frame() -> FrameInfo;
}
