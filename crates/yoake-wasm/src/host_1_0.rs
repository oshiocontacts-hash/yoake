//! Host trait bridges for frozen `yoake:mods@1.0.0` (`wit-mod-1.0`) `client-mod` components.
//!
//! Types are shared with `crate::bindings` (`1.7.0`) via `bindgen!(..., with: { "yoake:mods/types@1.0.0": ... })`;
//! each 1.0 interface forwards straight to the 1.7 `ClientContext` implementation so behavior is identical.

use wasmtime::component::Resource;

use crate::bindings::yoake::mods::{
    accounts as a17, client_world as cw17, feedback as f17, hud as h17, instance as i17,
    logging as l17, network as n17, player as p17, screen as sc17, settings as se17, types,
};
use crate::bindings_1_0::yoake::mods::{
    accounts as a10, client_world as cw10, feedback as f10, hud as h10, instance as i10,
    logging as l10, network as n10, player as p10, screen as sc10, settings as se10,
};
use crate::host::ClientContext;
use types::{
    AccountSnapshot, GuiNineSlice, HostError, PlayerSnapshot, Point, Rect, SendError,
    TextureFilter, TextureFormat, WorldKey, WorldSnapshot,
};

impl l10::Host for ClientContext {
    fn log(&mut self, target: String, message: String) -> Result<(), HostError> {
        l17::Host::log(self, target, message)
    }
}

impl f10::Host for ClientContext {
    fn feedback(&mut self, text: String) -> Result<(), HostError> {
        f17::Host::feedback(self, text)
    }
}

impl h10::Host for ClientContext {
    fn text(&mut self, text: String, at: Point, rgb: u32, shadow: bool) -> Result<(), HostError> {
        h17::Host::text(self, text, at, rgb, shadow)
    }

    fn fill(&mut self, bounds: Rect, argb: u32) -> Result<(), HostError> {
        h17::Host::fill(self, bounds, argb)
    }

    fn text_width(&mut self, text: String) -> Result<Option<f32>, HostError> {
        h17::Host::text_width(self, text)
    }

    fn batch_len(&mut self) -> Result<u64, HostError> {
        h17::Host::batch_len(self)
    }

    fn batch_scale_since(
        &mut self,
        start: u64,
        origin: Point,
        scale: Point,
    ) -> Result<(), HostError> {
        h17::Host::batch_scale_since(self, start, origin, scale)
    }

    fn batch_clip_since(&mut self, start: u64, bounds: Rect) -> Result<(), HostError> {
        h17::Host::batch_clip_since(self, start, bounds)
    }

    fn batch_tint_since(&mut self, start: u64, alpha: f32) -> Result<(), HostError> {
        h17::Host::batch_tint_since(self, start, alpha)
    }
}

impl h10::HostGuiTexture for ClientContext {
    fn create(
        &mut self,
        width: u32,
        height: u32,
        format: TextureFormat,
        filter: TextureFilter,
    ) -> Result<Resource<h10::GuiTexture>, HostError> {
        h17::HostGuiTexture::create(self, width, height, format, filter)
            .map(|r| Resource::new_own(r.rep()))
    }

    fn draw(
        &mut self,
        texture: Resource<h10::GuiTexture>,
        bounds: Rect,
        uv: Rect,
        argb: u32,
        nine_slice: Option<GuiNineSlice>,
    ) -> Result<bool, HostError> {
        h17::HostGuiTexture::draw(
            self,
            Resource::new_own(texture.rep()),
            bounds,
            uv,
            argb,
            nine_slice,
        )
    }

    fn drop(&mut self, texture: Resource<h10::GuiTexture>) -> wasmtime::Result<()> {
        h17::HostGuiTexture::drop(self, Resource::new_own(texture.rep()))
    }
}

impl n10::Host for ClientContext {
    fn net_send_to_server(&mut self, channel: String, data: Vec<u8>) -> Result<(), SendError> {
        n17::Host::net_send_to_server(self, channel, data)
    }
}

impl se10::Host for ClientContext {
    fn setting(&mut self, key: String) -> Result<Option<String>, HostError> {
        se17::Host::setting(self, key)
    }

    fn set_setting(&mut self, key: String, value: String) -> Result<(), HostError> {
        se17::Host::set_setting(self, key, value)
    }
}

impl cw10::Host for ClientContext {
    fn client_world_key(&mut self) -> Result<Option<WorldKey>, HostError> {
        cw17::Host::client_world_key(self)
    }

    fn snapshot_client_world(&mut self) -> Result<WorldSnapshot, HostError> {
        cw17::Host::snapshot_client_world(self)
    }

    fn playing(&mut self) -> Result<bool, HostError> {
        cw17::Host::playing(self)
    }
}

impl p10::Host for ClientContext {
    fn local_player_snapshot(&mut self) -> Result<Option<PlayerSnapshot>, HostError> {
        p17::Host::local_player_snapshot(self)
    }
}

impl i10::Host for ClientContext {
    fn instance_dir(&mut self) -> Result<String, HostError> {
        i17::Host::instance_dir(self)
    }
}

impl a10::Host for ClientContext {
    fn account(&mut self) -> Result<Option<AccountSnapshot>, HostError> {
        a17::Host::account(self)
    }
}

impl sc10::Host for ClientContext {
    fn mod_screen_open(&mut self, mod_id: String) -> Result<bool, HostError> {
        sc17::Host::mod_screen_open(self, mod_id)
    }

    fn mod_screen_close(&mut self, mod_id: String) -> Result<(), HostError> {
        sc17::Host::mod_screen_close(self, mod_id)
    }

    fn mod_screen(&mut self) -> Result<Option<String>, HostError> {
        sc17::Host::mod_screen(self)
    }

    fn mod_screen_open_anywhere(&mut self, mod_id: String) -> Result<bool, HostError> {
        sc17::Host::mod_screen_open_anywhere(self, mod_id)
    }
}
