//! MX3CL1: the client-thread hooks of full-access mods (feature `yoake2-slice`; without it every method here is empty,
//! so the call sites cost nothing and vanilla behaviour is unchanged).
//!
//! Kept out of `native_client_items.rs` on purpose: `yoake-bindgen` scans that file for generated `Game` bindings.
//!
//! * session start/end (`native_session_ready` / `native_session_left`, client games only): the approved full-access
//!   mods whose `[wasm] side` is `client` or `both` load into this thread's own runtime and are dropped again;
//! * `HOOK_CLIENT_TICK`: once per client tick (`tick_remote`, after the client mods' own tick), argument 0 is the `Game`;
//! * `HOOK_HUD_RENDER`: once per frame in `Game::hud`, after the vanilla HUD, observe-only.
//!
//! All three are no-ops unless a mod loaded here registered for them: the runtime is per thread and its chain lookup
//! is one thread-local read when no mod is loaded.

use super::Game;

#[cfg(feature = "yoake2-slice")]
impl Game<'_> {
    pub(super) fn yoake2_client_session_start_hook(&mut self) {
        if !self.options.execution_role.is_client() {
            return;
        }
        if let Some(root) = crate::yak_mods::game_root() {
            super::native_client_items::yoake2_install_dispatch();
            crate::yoake2_mods::load_client_mods(&root.join("mods"), crate::yoake2_mods::process_approved_list());
        }
    }

    pub(super) fn yoake2_client_session_end_hook(&mut self) {
        if self.options.execution_role.is_client() && crate::yoake2_mods::client_mods_loaded() {
            crate::yoake2_mods::unload_client_mods();
        }
    }

    pub(super) fn yoake2_client_tick_hook(&mut self) {
        if self.options.execution_role.is_client() {
            crate::yoake2_mods::dispatch_client_tick(self);
        }
    }

    pub(super) fn yoake2_hud_render_hook(&self) {
        if self.options.execution_role.is_client() {
            crate::yoake2_mods::dispatch_hud_render();
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
    pub(super) fn yoake2_hud_render_hook(&self) {}
}
