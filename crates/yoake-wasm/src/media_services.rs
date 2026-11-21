//! FR-MEDIA1: `media-services`, the browser (`service.browser`) a client-full package drives by handle.
//!
//! The package never sees the browser process or its DevTools socket: the host opens an installed Edge or
//! Chrome with a profile of the package's own, and the package asks for frames, sound and page state.

use wasmtime::component::Resource;

use crate::bindings::yoake::mods::hud::GuiTexture;
use crate::full_bindings::client::yoake::mods::media_services;
use crate::host::ClientContext;

pub use crate::full_bindings::client::yoake::mods::media_types;
use media_types::*;

/// The error a package without `service.browser` gets from every browser service.
pub const BROWSER_PERMISSION: &str = "service.browser permission is required";

macro_rules! media_services {
    ($($name:ident($($argument:ident: $kind:ty),*) -> $result:ty;)*) => {
        /// The engine's side of `media-services`. Textures arrive as the host's texture handle (see
        /// [`ClientContext`]); a package's own `gui-texture` is checked before the call reaches this.
        pub trait MediaServices: Send {
            $(fn $name(&mut self, $($argument: $kind),*) -> Result<$result, String>;)*
            fn browser_frame_to_texture(&mut self, handle: u32, texture: u64) -> Result<Option<u64>, String>;
            fn jpeg_to_texture(&mut self, bytes: Vec<u8>, texture: u64) -> Result<(u32, u32), String>;
        }

        impl media_services::Host for ClientContext {
            $(fn $name(&mut self, $($argument: $kind),*) -> Result<$result, String> {
                self.with_media_services(|host| host.$name($($argument),*))
            })*

            fn browser_frame_to_texture(&mut self, handle: u32, texture: Resource<GuiTexture>) -> Result<Option<u64>, String> {
                let texture = self.media_texture(&texture)?;
                self.with_media_services(|host| host.browser_frame_to_texture(handle, texture))
            }

            fn jpeg_to_texture(&mut self, bytes: Vec<u8>, texture: Resource<GuiTexture>) -> Result<(u32, u32), String> {
                let texture = self.media_texture(&texture)?;
                self.with_media_services(|host| host.jpeg_to_texture(bytes, texture))
            }
        }
    };
}

impl ClientContext {
    fn media_texture(&mut self, texture: &Resource<GuiTexture>) -> Result<u64, String> {
        // The permission comes first, so a package without it learns nothing about its textures here.
        if self.host.media_services().is_none() {
            return Err(BROWSER_PERMISSION.to_owned());
        }
        self.client_texture(texture)
    }

    fn with_media_services<T>(
        &mut self,
        operation: impl FnOnce(&mut dyn MediaServices) -> Result<T, String>,
    ) -> Result<T, String> {
        let began = std::time::Instant::now();
        let cpu_began = crate::thread_cpu_time();
        let result = match self.host.media_services() {
            Some(host) => operation(host),
            None => Err(BROWSER_PERMISSION.to_owned()),
        };
        // Decoding and copying frames is the host's work: it extends the call's deadline like the other
        // client services instead of using up the package's own budget.
        crate::add_host_time(began.elapsed(), crate::cpu_elapsed(cpu_began));
        result
    }
}

impl media_types::Host for ClientContext {}

media_services! {
    browser_open(height: u32, options: BrowserOptions, scale: f32, url: String, width: u32) -> u32;
    browser_close(handle: u32) -> ();
    browser_set_visible(handle: u32, visible: bool) -> ();
    browser_navigate(handle: u32, url: String) -> ();
    browser_history(action: HistoryAction, handle: u32) -> ();
    browser_input(event: InputEvent, handle: u32) -> ();
    browser_state(handle: u32) -> PageState;
    browser_frame_jpeg(handle: u32, quality: u8) -> Option<(u64, Vec<u8>)>;
    browser_audio_pcm(handle: u32) -> Vec<i16>;
    browser_eval(expression: String, handle: u32) -> u64;
    browser_eval_result(ticket: u64) -> EvalAnswer;
    browser_post(handle: u32, json: String) -> ();
    browser_messages(handle: u32) -> Vec<String>;
}
