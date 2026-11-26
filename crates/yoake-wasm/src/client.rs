use std::fmt;

use wasmtime::Store;
use wasmtime::component::HasSelf;

use crate::bindings::yoake::mods::types::{ReceivedFrame, ScreenView};
use crate::host::ClientContext;
use crate::{ClientHost, HostState, ModInstance, Sandbox, Subscriptions, bindings, bindings_1_0};

const LEGACY_HOOKS_EXPORT: &str = "yoake:mods/hooks@1.0.0";

#[derive(Debug)]
pub struct LoadError(String);

impl LoadError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for LoadError {}

enum ClientExports {
    Current(bindings::ClientMod),
    Legacy10(bindings_1_0::ClientMod),
}

macro_rules! call_hooks {
    ($exports:expr, $store:expr, $method:ident $(, $arg:expr)*) => {
        match $exports {
            ClientExports::Current(e) => e.yoake_mods_hooks().$method($store $(, $arg)*),
            ClientExports::Legacy10(e) => e.yoake_mods_hooks().$method($store $(, $arg)*),
        }
    };
}

pub struct ClientMod {
    raw: ModInstance<ClientContext>,
    exports: ClientExports,
    subscriptions: Subscriptions,
}

impl ClientMod {
    pub fn validate_component(
        runtime: &mut Sandbox,
        id: impl Into<String>,
        component_bytes: &[u8],
        host: Box<dyn ClientHost>,
    ) -> Result<(), LoadError> {
        Self::link(runtime, id.into(), component_bytes, host).map(|_| ())
    }

    pub fn instantiate(
        runtime: &mut Sandbox,
        id: impl Into<String>,
        component_bytes: &[u8],
        host: Box<dyn ClientHost>,
    ) -> Result<Self, LoadError> {
        let (mut raw, exports) = Self::link(runtime, id.into(), component_bytes, host)?;
        let names = raw.call_init(crate::TimeLimits::CLIENT, |store, _| match &exports {
            ClientExports::Current(e) => e.call_init(store).map_err(|error| error.root_cause().to_string()),
            ClientExports::Legacy10(e) => e.call_init(store).map_err(|error| error.root_cause().to_string()),
        });
        flush_output(&mut raw);
        let names = names.ok_or_else(|| LoadError("mod init trapped or timed out".into()))?;
        let subscriptions = Subscriptions::from_hook_names(names).map_err(LoadError)?;
        Ok(Self { raw, exports, subscriptions })
    }

    fn link(
        runtime: &mut Sandbox,
        id: String,
        component_bytes: &[u8],
        host: Box<dyn ClientHost>,
    ) -> Result<(ModInstance<ClientContext>, ClientExports), LoadError> {
        let texture_count = runtime.texture_count(&id);
        let compiled = runtime.compile(component_bytes).map_err(LoadError)?;
        let legacy_1_0 = compiled.exports_interface(LEGACY_HOOKS_EXPORT);
        let mut raw = compiled
            .instantiate(id.clone(), ClientContext::new(id, host, texture_count), |linker| {
                if legacy_1_0 {
                    bindings_1_0::ClientMod::add_to_linker::<HostState<ClientContext>, HasSelf<ClientContext>>(
                        linker,
                        |state| &mut state.host,
                    )
                } else {
                    bindings::ClientMod::add_to_linker::<HostState<ClientContext>, HasSelf<ClientContext>>(
                        linker,
                        |state| &mut state.host,
                    )
                }
                .map_err(|error| format!("{error:#}"))
            })
            .map_err(LoadError)?;
        let exports = if legacy_1_0 {
            ClientExports::Legacy10(
                bindings_1_0::ClientMod::new(&mut raw.store, &raw.instance)
                    .map_err(|error| LoadError(format!("invalid client-mod exports: {error:#}")))?,
            )
        } else {
            ClientExports::Current(
                bindings::ClientMod::new(&mut raw.store, &raw.instance)
                    .map_err(|error| LoadError(format!("invalid client-mod exports: {error:#}")))?,
            )
        };
        Ok((raw, exports))
    }

    pub fn subscribed(&self, hook: &str) -> bool {
        self.subscriptions.subscribed(hook)
    }

    pub fn is_disabled(&self) -> bool {
        self.raw.is_disabled()
    }

    pub fn tick(&mut self, reserved: u32) -> Option<()> {
        self.dispatch("tick", |exports, store| {
            call_hooks!(exports, store, call_tick, reserved)
        })
    }

    pub fn key(&mut self, key: &str, pressed: bool) -> Option<()> {
        self.dispatch("key", |exports, store| {
            call_hooks!(exports, store, call_key, key, pressed)
        })
    }

    pub fn hud(&mut self) -> Option<()> {
        self.dispatch("hud", |exports, store| {
            call_hooks!(exports, store, call_hud)
        })
    }

    pub fn net_from_server(&mut self, channel: &str, data: &[u8]) -> Option<()> {
        self.dispatch("net_from_server", |exports, store| {
            call_hooks!(exports, store, call_net_from_server, channel, data)
        })
    }

    pub fn frame(&mut self, elapsed_ns: u64) -> Option<()> {
        self.dispatch("frame", |exports, store| {
            call_hooks!(exports, store, call_frame, elapsed_ns)
        })
    }

    pub fn local_player_tick_end(&mut self, reserved: u32) -> Option<()> {
        self.dispatch("local_player_tick_end", |exports, store| {
            call_hooks!(exports, store, call_local_player_tick_end, reserved)
        })
    }

    pub fn net_frames_listened(&mut self, listened: bool) -> Option<bool> {
        self.dispatch("net_frames_listened", |exports, store| {
            call_hooks!(exports, store, call_net_frames_listened, listened)
        })
        .map(|guest_listened| listened || guest_listened)
    }

    pub fn net_frame_received(&mut self, frame: &ReceivedFrame) -> Option<()> {
        self.dispatch("net_frame_received", |exports, store| {
            call_hooks!(exports, store, call_net_frame_received, frame)
        })
    }

    pub fn net_session_ended(&mut self, session: u64, reason: &str) -> Option<()> {
        self.dispatch("net_session_ended", |exports, store| {
            call_hooks!(exports, store, call_net_session_ended, session, reason)
        })
    }

    pub fn gui_layer_pre(&mut self, layer: &str) -> Option<bool> {
        self.dispatch("gui_layer_pre", |exports, store| {
            call_hooks!(exports, store, call_gui_layer_pre, layer)
        })
    }

    pub fn gui_layer_post(&mut self, layer: &str) -> Option<()> {
        self.dispatch("gui_layer_post", |exports, store| {
            call_hooks!(exports, store, call_gui_layer_post, layer)
        })
    }

    pub fn screen_render_post(&mut self, screen: &ScreenView) -> Option<()> {
        self.dispatch("screen_render_post", |exports, store| {
            call_hooks!(exports, store, call_screen_render_post, screen)
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
            call_hooks!(exports, store, call_screen_key_pressed_pre, screen, key, repeat, modifiers)
        })
    }

    pub fn screen_char_typed_pre(
        &mut self,
        screen: &ScreenView,
        character: u32,
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_char_typed_pre", |exports, store| {
            call_hooks!(exports, store, call_screen_char_typed_pre, screen, character, modifiers)
        })
    }

    pub fn screen_mouse_button_pressed_pre(
        &mut self,
        screen: &ScreenView,
        button: i32,
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_mouse_button_pressed_pre", |exports, store| {
            call_hooks!(exports, store, call_screen_mouse_button_pressed_pre, screen, button, modifiers)
        })
    }

    pub fn screen_mouse_button_released_pre(
        &mut self,
        screen: &ScreenView,
        button: i32,
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_mouse_button_released_pre", |exports, store| {
            call_hooks!(exports, store, call_screen_mouse_button_released_pre, screen, button, modifiers)
        })
    }

    pub fn screen_mouse_scrolled_pre(
        &mut self,
        screen: &ScreenView,
        scroll: (f64, f64),
        modifiers: u32,
    ) -> Option<bool> {
        self.dispatch("screen_mouse_scrolled_pre", |exports, store| {
            call_hooks!(exports, store, call_screen_mouse_scrolled_pre, screen, scroll, modifiers)
        })
    }

    pub fn settings(&mut self) -> Option<Vec<String>> {
        let id = self.raw.id.clone();
        self.settings_for(&id)
    }

    /// The `settings` hook for the settings page of `mod_id` (any mod's page, as for a DLL mod).
    pub fn settings_for(&mut self, mod_id: &str) -> Option<Vec<String>> {
        self.dispatch("settings", |exports, store| {
            call_hooks!(exports, store, call_settings, mod_id)
        })
        .map(limit_settings)
    }

    pub fn settings_click(&mut self, button: u64) -> Option<()> {
        let id = self.raw.id.clone();
        self.settings_click_for(&id, button)
    }

    pub fn settings_click_for(&mut self, mod_id: &str, button: u64) -> Option<()> {
        self.dispatch("settings_click", |exports, store| {
            call_hooks!(exports, store, call_settings_click, mod_id, button)
        })
    }

    fn dispatch<R>(
        &mut self,
        hook: &str,
        call: impl FnOnce(
            &ClientExports,
            &mut Store<HostState<ClientContext>>,
        ) -> wasmtime::Result<R>,
    ) -> Option<R> {
        if !self.subscribed(hook) {
            return None;
        }
        let exports = &self.exports;
        let result = self.raw.call(hook, |store, _| {
            call(exports, store).map_err(|error| format!("{error:#}"))
        });
        if self.raw.is_disabled() {
            self.raw.store.data_mut().host.release_resources();
        }
        flush_output(&mut self.raw);
        result
    }
}

/// Forwards what the guest printed through WASI stdout/stderr to the host log, line by line.
fn flush_output(raw: &mut ModInstance<ClientContext>) {
    for line in raw.drain_output() {
        raw.store.data_mut().host.log_output(&line);
    }
}

fn limit_settings(entries: Vec<String>) -> Vec<String> {
    entries
        .into_iter()
        .take(64)
        .map(|entry| entry.chars().take(256).collect())
        .collect()
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
