
use std::fmt;

use wasmtime::component::HasSelf;
use wasmtime::Store;

use crate::bindings::yoake::mods::types::{ReceivedFrame, ScreenView};
use crate::host::ClientContext;
use crate::{bindings, ClientHost, HostState, ModInstance, Sandbox, Subscriptions};

#[derive(Debug)]
pub struct LoadError(String);

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for LoadError {}

pub struct ClientMod {
    raw: ModInstance<ClientContext>,
    exports: bindings::ClientMod,
    subscriptions: Subscriptions,
}

impl ClientMod {
    pub fn instantiate(
        runtime: &mut Sandbox,
        id: impl Into<String>,
        component_bytes: &[u8],
        host: Box<dyn ClientHost>,
    ) -> Result<Self, LoadError> {
        let id = id.into();
        let texture_count = runtime.texture_count(&id);
        let mut raw = runtime
            .compile(component_bytes)
            .map_err(LoadError)?
            .instantiate(id.clone(), ClientContext::new(id, host, texture_count), |linker| {
                bindings::ClientMod::add_to_linker::<HostState<ClientContext>, HasSelf<ClientContext>>(
                    linker,
                    |state| &mut state.host,
                )
                .map_err(|error| error.to_string())
            })
            .map_err(LoadError)?;
        let exports = bindings::ClientMod::new(&mut raw.store, &raw.instance)
            .map_err(|error| LoadError(error.to_string()))?;
        let names = raw
            .call("init", |store, _| exports.call_init(store).map_err(|error| error.to_string()))
            .ok_or_else(|| LoadError("mod init trapped or timed out".into()))?;
        let subscriptions = Subscriptions::from_hook_names(names).map_err(LoadError)?;
        Ok(Self { raw, exports, subscriptions })
    }

    pub fn subscribed(&self, hook: &str) -> bool {
        self.subscriptions.subscribed(hook)
    }

    pub fn is_disabled(&self) -> bool {
        self.raw.is_disabled()
    }

    pub fn tick(&mut self, reserved: u32) -> Option<()> {
        self.dispatch("tick", |exports, store| {
            exports.yoake_mods_hooks().call_tick(store, reserved)
        })
    }

    pub fn key(&mut self, key: &str, pressed: bool) -> Option<()> {
        self.dispatch("key", |exports, store| {
            exports.yoake_mods_hooks().call_key(store, key, pressed)
        })
    }

    pub fn hud(&mut self) -> Option<()> {
        self.dispatch("hud", |exports, store| exports.yoake_mods_hooks().call_hud(store))
    }

    pub fn net_from_server(&mut self, channel: &str, data: &[u8]) -> Option<()> {
        self.dispatch("net_from_server", |exports, store| {
            exports.yoake_mods_hooks().call_net_from_server(store, channel, data)
        })
    }

    pub fn frame(&mut self, elapsed_ns: u64) -> Option<()> {
        self.dispatch("frame", |exports, store| {
            exports.yoake_mods_hooks().call_frame(store, elapsed_ns)
        })
    }

    pub fn local_player_tick_end(&mut self, reserved: u32) -> Option<()> {
        self.dispatch("local_player_tick_end", |exports, store| {
            exports.yoake_mods_hooks().call_local_player_tick_end(store, reserved)
        })
    }

    pub fn net_frames_listened(&mut self, listened: bool) -> Option<bool> {
        self.dispatch("net_frames_listened", |exports, store| {
            exports.yoake_mods_hooks().call_net_frames_listened(store, listened)
        })
        .map(|guest_listened| listened || guest_listened)
    }

    pub fn net_frame_received(&mut self, frame: &ReceivedFrame) -> Option<()> {
        self.dispatch("net_frame_received", |exports, store| {
            exports.yoake_mods_hooks().call_net_frame_received(store, frame)
        })
    }

    pub fn net_session_ended(&mut self, session: u64, reason: &str) -> Option<()> {
        self.dispatch("net_session_ended", |exports, store| {
            exports.yoake_mods_hooks().call_net_session_ended(store, session, reason)
        })
    }

    pub fn gui_layer_pre(&mut self, layer: &str) -> Option<bool> {
        self.dispatch("gui_layer_pre", |exports, store| {
            exports.yoake_mods_hooks().call_gui_layer_pre(store, layer)
        })
    }

    pub fn gui_layer_post(&mut self, layer: &str) -> Option<()> {
        self.dispatch("gui_layer_post", |exports, store| {
            exports.yoake_mods_hooks().call_gui_layer_post(store, layer)
        })
    }

    pub fn screen_render_post(&mut self, screen: &ScreenView) -> Option<()> {
        self.dispatch("screen_render_post", |exports, store| {
            exports.yoake_mods_hooks().call_screen_render_post(store, screen)
        })
    }

    pub fn screen_key_pressed_pre(
        &mut self,
        screen: &ScreenView,
        key: &str,
        repeat: bool,
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_key_pressed_pre", |exports, store| {
            exports.yoake_mods_hooks().call_screen_key_pressed_pre(store, screen, key, repeat, modifiers)
        })
    }

    pub fn screen_char_typed_pre(
        &mut self,
        screen: &ScreenView,
        character: u32,
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_char_typed_pre", |exports, store| {
            exports.yoake_mods_hooks().call_screen_char_typed_pre(store, screen, character, modifiers)
        })
    }

    pub fn screen_mouse_button_pressed_pre(
        &mut self,
        screen: &ScreenView,
        button: i32,
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_mouse_button_pressed_pre", |exports, store| {
            exports.yoake_mods_hooks().call_screen_mouse_button_pressed_pre(store, screen, button, modifiers)
        })
    }

    pub fn screen_mouse_button_released_pre(
        &mut self,
        screen: &ScreenView,
        button: i32,
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_mouse_button_released_pre", |exports, store| {
            exports.yoake_mods_hooks().call_screen_mouse_button_released_pre(store, screen, button, modifiers)
        })
    }

    pub fn screen_mouse_scrolled_pre(
        &mut self,
        screen: &ScreenView,
        scroll: (f64, f64),
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_mouse_scrolled_pre", |exports, store| {
            exports.yoake_mods_hooks().call_screen_mouse_scrolled_pre(store, screen, scroll, modifiers)
        })
    }

    pub fn settings(&mut self) -> Option<Vec<String>> {
        let id = self.raw.id.clone();
        self.dispatch("settings", |exports, store| {
            exports.yoake_mods_hooks().call_settings(store, &id)
        })
        .map(limit_settings)
    }

    pub fn settings_click(&mut self, button: u64) -> Option<()> {
        let id = self.raw.id.clone();
        self.dispatch("settings_click", |exports, store| {
            exports.yoake_mods_hooks().call_settings_click(store, &id, button)
        })
    }

    fn dispatch<R>(
        &mut self,
        hook: &str,
        call: impl FnOnce(&bindings::ClientMod, &mut Store<HostState<ClientContext>>) -> wasmtime::Result<R>,
    ) -> Option<R> {
        if !self.subscribed(hook) {
            return None;
        }
        let exports = &self.exports;
        self.raw.call(hook, |store, _| call(exports, store).map_err(|error| error.to_string()))
    }
}

fn limit_settings(entries: Vec<String>) -> Vec<String> {
    entries.into_iter().take(64).map(|entry| entry.chars().take(256).collect()).collect()
}

#[cfg(test)]
mod tests {
    use super::limit_settings;

    #[test]
    fn settings_are_limited_by_entry_and_unicode_character_count() {
        let entries = vec!["界".repeat(257); 70];
        let limited = limit_settings(entries);
        assert_eq!(limited.len(), 64);
        assert!(limited.iter().all(|entry| entry.chars().count() == 256));
    }
}
