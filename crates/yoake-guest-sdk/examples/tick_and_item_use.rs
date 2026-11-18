//! Example full-access mod: registering hooks, checking `HookMode`, counting server ticks,
//! and handling **Full** vs **ObserveOnly** bridged hooks.
//!
//! Compiled as a `#![no_std]` wasm32 core module (see `tests/support/build.rs`), then packed into a
//! `.yak` with `[wasm] world = "full-access"`.
#![no_std]
extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

use yoake_guest_sdk::stubs::entities::entities_experience_total_get;
use yoake_guest_sdk::{Handle, HookMode, Registrar, export_guest, hooks, original, report};

/// Ledger id 103: `horizon_client::game::item_use::Game::start_using_item(&mut self) -> bool`.
/// Listed in `hook_bridge_game__item_use::BRIDGED` and not in `OBSERVE_ONLY`, so it runs in `HookMode::Full`.
const HOOK_START_USING_ITEM: u32 = 103;

/// Ledger id 43: `horizon_client_lib::entities::Experience::give(&mut self, points: i32) -> Option<i32>`.
/// Listed in `hook_bridge_entities::OBSERVE_ONLY` (all bridged `game::item_use` hooks currently return
/// `()` or `bool`, so `hook_bridge_game__item_use::OBSERVE_ONLY` is empty). `Option<i32>` does not
/// fit in an `i64`, so this hook runs in `HookMode::ObserveOnly`.
const HOOK_EXPERIENCE_GIVE: u32 = 43;

// Report slots read by the end-to-end test (`yoake2_examples_tests.rs`).
const SLOT_MODE_SERVER_TICK: i32 = 10;
const SLOT_MODE_START_USING_ITEM: i32 = 11;
const SLOT_MODE_EXPERIENCE_GIVE: i32 = 12;
const SLOT_TICKS: i32 = 20;
const SLOT_FULL_RESULT: i32 = 30;
const SLOT_OBSERVE_BEFORE: i32 = 40;
const SLOT_OBSERVE_AFTER: i32 = 41;

static SERVER_TICKS: AtomicU32 = AtomicU32::new(0);

fn mode_code(mode: HookMode) -> i64 {
    match mode {
        HookMode::Full => 0,
        HookMode::ObserveOnly => 1,
        HookMode::NotCallable => 2,
    }
}

/// Runs once at mod load (`yoake_init`). `hook_register` is only accepted here.
fn init(r: &mut Registrar) {
    // Check the `HookMode` returned for each hook:
    // - `Full` (0): guest return value is decoded as the hooked function's result; skipping `original` cancels it.
    // - `ObserveOnly` (1): guest return value is ignored and the call cannot be cancelled.
    // - `NotCallable` (2): registration succeeded, but this engine build has no bridge for the id.
    let tick_mode = r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK");
    let full_mode = r.hook(HOOK_START_USING_ITEM, 0).expect("register start_using_item");
    let observe_mode = r.hook(HOOK_EXPERIENCE_GIVE, 0).expect("register Experience::give");

    report(SLOT_MODE_SERVER_TICK, mode_code(tick_mode));
    report(SLOT_MODE_START_USING_ITEM, mode_code(full_mode));
    report(SLOT_MODE_EXPERIENCE_GIVE, mode_code(observe_mode));
}

/// Dispatched for every hook id registered in `init` (`yoake_hook`).
///
/// Re-entry rule: never hold a borrow of guest statics (like a `RefCell` guard) across `original(handle)`,
/// because `original` runs the engine and other hooks and may re-enter this same guest.
fn hook(id: u32, handle: Handle) -> i64 {
    match id {
        hooks::SERVER_TICK => {
            let count = SERVER_TICKS.fetch_add(1, Ordering::Relaxed) + 1;
            report(SLOT_TICKS, i64::from(count));
            // Pass through to the next mod in the `SERVER_TICK` chain (or the engine).
            let _ = original(handle);
            0
        }

        HOOK_START_USING_ITEM => {
            // **Full mode** (`HookMode::Full`):
            // - You decide the hooked function's return value (here `bool`: `0` = false, `1` = true).
            // - Call `original(handle)` to run the next mod in the chain or the engine body, then pass its
            //   value through or replace it with your own `i64`.
            // - Return without calling `original(handle)` to cancel the call and skip downstream mods.
            // - Out-of-range return values (e.g. `2` for `bool`) are rejected by the bridge, which falls back
            //   to the original's result.
            let result = original(handle).unwrap_or(0);
            report(SLOT_FULL_RESULT, result);
            result
        }

        HOOK_EXPERIENCE_GIVE => {
            // **ObserveOnly mode** (`HookMode::ObserveOnly`):
            // - The hooked function returns a type that does not fit in `i64` (here `Option<i32>`).
            // - You may read the receiver (`handle`) before and after `original(handle)` to observe state changes.
            // - Your `i64` return value is **ignored**; you cannot replace the engine result.
            // - You **cannot cancel** the call: even if you never call `original(handle)`, the bridge runs the
            //   real original once after the guest returns.
            let before = entities_experience_total_get(handle).unwrap_or(-1);
            report(SLOT_OBSERVE_BEFORE, i64::from(before));

            let _ = original(handle);

            // `handle` is `Locked` only while `original` is running; afterwards it is usable again.
            let after = entities_experience_total_get(handle).unwrap_or(-1);
            report(SLOT_OBSERVE_AFTER, i64::from(after));
            0
        }

        _ => original(handle).unwrap_or(0),
    }
}

export_guest!(init, hook);
