//! MX3CL1: the client-thread hooks of full-access mods (feature `yoake2-slice`; without it every method here is empty,
//! so the call sites cost nothing and vanilla behaviour is unchanged).
//!
//! Kept out of `native_client_items.rs` on purpose: `yoake-bindgen` scans that file for generated `Game` bindings.
//!
//! * session start/end (`native_session_ready` / `native_session_left`, client games only): the approved full-access
//!   mods whose `[wasm] side` is `client` or `both` load into this thread's own runtime and are dropped again;
//! * `HOOK_CLIENT_TICK`: once per client tick (`tick_remote`, after the client mods' own tick), argument 0 is the `Game`;
//! * `HOOK_HUD_RENDER`: once per frame in `Game::hud`, after the vanilla HUD. REM-12: a module draws from it with
//!   the `hud_draw` service: the commands are queued while the hook runs and applied to the frame's `HudBatch` here,
//!   through the S route's HUD code, when it returned;
//! * REM-12: `HOOK_GPU_FRAME` (the renderer's GPU pass, through the F package the session start registers),
//!   `HOOK_IME_PREEDIT` / `HOOK_IME_COMMIT` (`ime.rs`), and the flush of the client services' queues (sounds, PCM
//!   streams) after every client hook of the frame (`game/yoake2_f_services.rs`).
//!
//! All three are no-ops unless a mod loaded here registered for them: the runtime is per thread and its chain lookup
//! is one thread-local read when no mod is loaded.

use super::Game;
#[cfg(feature = "yoake2-slice")]
use super::yoake2_f_state as f_services;

#[cfg(feature = "yoake2-slice")]
impl Game<'_> {
    pub(super) fn yoake2_client_session_start_hook(&mut self) {
        if !self.options.execution_role.is_client() {
            return;
        }
        if let Some(root) = crate::yak_mods::game_root() {
            super::native_client_items::yoake2_install_dispatch();
            let report = crate::yoake2_mods::load_client_mods(&root.join("mods"), crate::yoake2_mods::launch_approved_list().as_ref());
            // REM-12: a module that registered `GPU_FRAME` gets the renderer's per-frame callback
            if crate::yoake2_mods::client_registered(yoake_slice::runtime::HOOK_GPU_FRAME) {
                let fingerprint = report.loaded.iter().map(|l| format!("{}@{}", l.id, l.version)).collect::<Vec<_>>().join(",");
                crate::entry::wasm_mods::register_f_gpu(&format!("f:{fingerprint}"), std::sync::Arc::new(crate::yoake2_mods::dispatch_gpu_frame));
            }
        }
    }

    pub(super) fn yoake2_client_session_end_hook(&mut self) {
        if self.options.execution_role.is_client() && crate::yoake2_mods::client_mods_loaded() {
            f_services::shutdown(&mut GameStreams(self));
            crate::entry::wasm_mods::release_f_gpu();
            crate::yoake2_mods::unload_client_mods();
        }
    }

    pub(super) fn yoake2_client_tick_hook(&mut self) {
        if self.options.execution_role.is_client() {
            crate::yoake2_mods::dev_reload_tick(crate::yoake2_mods::ThreadSide::Client);
            crate::yoake2_mods::dispatch_client_tick(self);
            self.yoake2_f_flush();
        }
    }

    /// REM-12: applies what the client hooks queued through the F services: the sounds, then the PCM streams.
    pub(super) fn yoake2_f_flush(&mut self) {
        for sound in f_services::take_sounds() {
            self.native_queue_local_sound(&sound.sound, sound.at, sound.volume, sound.pitch);
        }
        f_services::flush_streams(&mut GameStreams(self));
    }

    /// MX3HR1: the server thread's dev-reload point, called at `ServerTickStart` just before `yoake2_tick_hook` (that
    /// method is itself a hookable function, so a reload inside it would run inside a guest's `original`). Does nothing
    /// without `--full-access-dev`.
    pub(super) fn yoake2_dev_reload_hook(&mut self) {
        use super::execution_role::ExecutionRole;
        let serves = self.options.dedicated.is_some() || matches!(self.options.execution_role, ExecutionRole::IntegratedServer);
        if serves {
            crate::yoake2_mods::dev_reload_tick(crate::yoake2_mods::ThreadSide::Server);
        }
    }

    /// Once per frame after the vanilla HUD: the module's `HUD_RENDER` hook, then its HUD commands onto `batch`.
    pub(super) fn yoake2_hud_render_hook(&mut self, batch: &mut horizon_client::hud::HudBatch) {
        if !self.options.execution_role.is_client() || !crate::yoake2_mods::client_mods_loaded() {
            return;
        }
        let [width, height] = self.native_gui_size();
        f_services::hud_begin((width, height), batch.scale);
        crate::yoake2_mods::dispatch_hud_render();
        let commands = f_services::hud_end();
        if !commands.is_empty() {
            crate::entry::wasm_mods::apply_f_hud(Some(&*self), batch, &commands);
        }
        self.yoake2_f_flush();
    }

    /// `Ime::Preedit` / `Ime::Disabled`: the modules' `IME_PREEDIT` hook. True when a module took the input method
    /// (`ime_set_allowed(true)`): the engine's own composition handling then steps aside.
    pub(super) fn yoake2_ime_preedit_hook(&mut self, text: &str, cursor: Option<(usize, usize)>) -> bool {
        if self.options.execution_role.is_client() && crate::yoake2_mods::client_mods_loaded() {
            f_services::set_ime_event(Some(f_services::preedit_event(text, cursor)));
            crate::yoake2_mods::dispatch_ime_preedit();
            f_services::set_ime_event(None);
            self.yoake2_f_flush();
        }
        f_services::ime_claimed()
    }

    /// `Ime::Commit`: the modules' `IME_COMMIT` hook; true when a module took the input method.
    pub(super) fn yoake2_ime_commit_hook(&mut self, text: &str) -> bool {
        if self.options.execution_role.is_client() && crate::yoake2_mods::client_mods_loaded() {
            f_services::set_ime_event(Some(f_services::commit_event(text)));
            crate::yoake2_mods::dispatch_ime_commit();
            f_services::set_ime_event(None);
            self.yoake2_f_flush();
        }
        f_services::ime_claimed()
    }
}

#[cfg(feature = "yoake2-slice")]
thread_local! {
    /// The engine's handle of each PCM stream the F services opened, by the services' own local id.
    static HOST_STREAMS: std::cell::RefCell<std::collections::HashMap<u32, crate::mod_api::NativeAudioStreamId>> = Default::default();
}

/// The game's audio engine as the F services' stream flush sees it.
#[cfg(feature = "yoake2-slice")]
struct GameStreams<'a, 'g>(&'a mut Game<'g>);

#[cfg(feature = "yoake2-slice")]
fn stream_anchor(anchor: &f_services::FAnchor) -> crate::mod_api::NativeStreamAnchor {
    match *anchor {
        f_services::FAnchor::Listener => crate::mod_api::NativeStreamAnchor::Listener,
        f_services::FAnchor::Entity(id) => crate::mod_api::NativeStreamAnchor::Entity(id),
        f_services::FAnchor::Position { x, y, z } => crate::mod_api::NativeStreamAnchor::Position([x, y, z]),
    }
}

#[cfg(feature = "yoake2-slice")]
fn stream_category(name: &str) -> Result<super::native_client_state::NativeSoundCategory, String> {
    use super::native_client_state::NativeSoundCategory as C;
    Ok(match name {
        "master" => C::Master,
        "music" => C::Music,
        "record" | "records" => C::Records,
        "weather" => C::Weather,
        "block" | "blocks" => C::Blocks,
        "hostile" => C::Hostile,
        "neutral" => C::Neutral,
        "player" | "players" => C::Players,
        "ambient" => C::Ambient,
        "voice" => C::Voice,
        "ui" => C::Ui,
        other => return Err(format!("unknown sound category {other:.32}")),
    })
}

#[cfg(feature = "yoake2-slice")]
impl f_services::StreamBackend for GameStreams<'_, '_> {
    fn open(&mut self, local: u32, anchor: &f_services::FAnchor, category: &str, channels: u16, sample_rate: u32) -> Result<(), String> {
        let id = self.0.native_audio_stream_open(stream_anchor(anchor), sample_rate, channels, stream_category(category)?).map_err(|e| format!("{e:?}"))?;
        HOST_STREAMS.with(|streams| streams.borrow_mut().insert(local, id));
        Ok(())
    }

    fn push(&mut self, local: u32, samples: &[f32]) -> Result<usize, String> {
        let id = HOST_STREAMS.with(|streams| streams.borrow().get(&local).copied()).ok_or("stream is not open")?;
        self.0.native_audio_stream_push(id, samples).map_err(|e| format!("{e:?}"))
    }

    fn queued(&mut self, local: u32) -> Option<usize> {
        let id = HOST_STREAMS.with(|streams| streams.borrow().get(&local).copied())?;
        self.0.native_audio_stream_queued(id)
    }

    fn dropped(&mut self, local: u32) -> Option<u64> {
        let id = HOST_STREAMS.with(|streams| streams.borrow().get(&local).copied())?;
        self.0.native_audio_stream_dropped(id)
    }

    fn move_to(&mut self, local: u32, anchor: &f_services::FAnchor) -> Result<(), String> {
        let id = HOST_STREAMS.with(|streams| streams.borrow().get(&local).copied()).ok_or("stream is not open")?;
        self.0.native_audio_stream_move(id, stream_anchor(anchor)).map_err(|e| format!("{e:?}"))
    }

    fn close(&mut self, local: u32) -> bool {
        match HOST_STREAMS.with(|streams| streams.borrow_mut().remove(&local)) {
            Some(id) => self.0.native_audio_stream_close(id),
            None => false,
        }
    }
}

#[cfg(not(feature = "yoake2-slice"))]
impl Game<'_> {
    #[inline(always)]
    pub(super) fn yoake2_client_session_start_hook(&mut self) {}
    #[inline(always)]
    pub(super) fn yoake2_client_session_end_hook(&mut self) {}
    #[inline(always)]
    pub(super) fn yoake2_client_tick_hook(&mut self) {}
    #[inline(always)]
    pub(super) fn yoake2_dev_reload_hook(&mut self) {}
    #[inline(always)]
    pub(super) fn yoake2_hud_render_hook(&mut self, _batch: &mut horizon_client::hud::HudBatch) {}
    #[inline(always)]
    pub(super) fn yoake2_ime_preedit_hook(&mut self, _text: &str, _cursor: Option<(usize, usize)>) -> bool {
        false
    }
    #[inline(always)]
    pub(super) fn yoake2_ime_commit_hook(&mut self, _text: &str) -> bool {
        false
    }
}
