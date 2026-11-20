//! Example full-access mod: a hook on a stamped function that takes `&Game` (`Game::sleep_height(&self, head)`, ledger
//! hook 5380) reads the level (`Game.world`) and the settings (`Game.settings`) through the read-only `&Game` root,
//! then passes the call on. Nothing can be written through a shared root.
//!
//! Compiled as a `#![no_std]` wasm32 core module (see `tests/support/build.rs`), packed as a full-access `.yak`.
#![no_std]
extern crate alloc;

use yoake_guest_sdk::stubs::game_items::{game_items_game_settings_handle, game_items_game_world_handle};
use yoake_guest_sdk::stubs::render_settings::render_settings_settings_fov_get;
use yoake_guest_sdk::stubs::render_world::render_world_world_day_time_get;
use yoake_guest_sdk::{Handle, Registrar, export_guest, original, report};

/// `horizon_client::game::beds::Game::sleep_height` in hook-ledger/horizon_client.json.
const SLEEP_HEIGHT: u32 = 5380;

// Report slots read by the end-to-end test (`yoake2_examples_tests.rs`).
const SLOT_ENTERED: i32 = 1;
const SLOT_DAY_TIME: i32 = 2;
const SLOT_FOV_BITS: i32 = 3;

fn init(r: &mut Registrar) {
    r.hook(SLEEP_HEIGHT, 0).expect("register Game::sleep_height");
}

fn hook(id: u32, root: Handle) -> i64 {
    if id != SLEEP_HEIGHT {
        return original(root).unwrap_or(0);
    }
    report(SLOT_ENTERED, 1);
    // Argument 0 is the shared `&Game` root; argument 1 is the by-value-ish `[i32; 3]` head position (opaque).
    if let Ok(world) = game_items_game_world_handle(root) {
        if let Ok(day_time) = render_world_world_day_time_get(world) {
            report(SLOT_DAY_TIME, day_time);
        }
    }
    if let Ok(settings) = game_items_game_settings_handle(root) {
        if let Ok(fov) = render_settings_settings_fov_get(settings) {
            report(SLOT_FOV_BITS, i64::from(fov.to_bits()));
        }
    }
    original(root).unwrap_or(0)
}

export_guest!(init, hook);
