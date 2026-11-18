//! Example full-access mod: reaching an engine entity through a `Game` path in `SERVER_TICK`,
//! reading and writing fields via generated `engine_call` stubs, and handling `Error` (`Stale`).
//!
//! Compiled as a `#![no_std]` wasm32 core module (see `tests/support/build.rs`), then packed into a
//! `.yak` with `[wasm] world = "full-access"`.
#![no_std]
extern crate alloc;

use yoake_guest_sdk::stubs::entities::{
    entities_itementity_fire_ticks_get, entities_itementity_fire_ticks_set, entities_itementity_position_get,
};
use yoake_guest_sdk::stubs::game_items::game_items_game_item_entity_handle;
use yoake_guest_sdk::{Error, Handle, Registrar, export_guest, hooks, original, report};

const TARGET_ITEM_ID: u32 = 4242;
const MISSING_ITEM_ID: u32 = 1;
const NEW_FIRE_TICKS: i32 = 55;

// Report slots read by the end-to-end test (`yoake2_examples_tests.rs`).
const SLOT_FOUND_HANDLE: i32 = 1;
const SLOT_MISSING_ITEM_ERR: i32 = 2;
const SLOT_POS_X: i32 = 3;
const SLOT_POS_Y: i32 = 4;
const SLOT_FIRE_TICKS_BEFORE: i32 = 5;
const SLOT_WRITE_OK: i32 = 6;
const SLOT_FIRE_TICKS_AFTER: i32 = 7;
const SLOT_MISSING_IS_STALE: i32 = 8;

fn init(r: &mut Registrar) {
    // Only `hooks::SERVER_TICK` receives the `Game` root handle; other hooks receive a typed argument handle.
    r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK");
}

fn hook(id: u32, root: Handle) -> i64 {
    if id != hooks::SERVER_TICK {
        return original(root).unwrap_or(0);
    }

    // Open a path handle into `Game.dropped` for entity id `TARGET_ITEM_ID`.
    // Path handles re-borrow from `Game` on every stub call and live only for this activation.
    let item = match game_items_game_item_entity_handle(root, TARGET_ITEM_ID) {
        Ok(handle) => {
            report(SLOT_FOUND_HANDLE, 1);
            handle
        }
        Err(err) => {
            // If the entity is not present (e.g. removed), `engine_call` returns `Error::Handle(1)` (`Stale`),
            // never a trap — the mod stays enabled and handles the error normally.
            report(
                SLOT_MISSING_ITEM_ERR,
                match err {
                    Error::Handle(code) => -i64::from(code),
                    _ => -99,
                },
            );
            let _ = original(root);
            return 0;
        }
    };

    // Read a compound plain-data field (`Vec3`).
    if let Ok(pos) = entities_itementity_position_get(item) {
        report(SLOT_POS_X, pos.x.to_bits() as i64);
        report(SLOT_POS_Y, pos.y.to_bits() as i64);
    }

    // Read a scalar field, update it through its generated setter, and read it back.
    if let Ok(before) = entities_itementity_fire_ticks_get(item) {
        report(SLOT_FIRE_TICKS_BEFORE, i64::from(before));
    }
    let write_ok = entities_itementity_fire_ticks_set(item, NEW_FIRE_TICKS).is_ok();
    report(SLOT_WRITE_OK, i64::from(write_ok));
    if let Ok(after) = entities_itementity_fire_ticks_get(item) {
        report(SLOT_FIRE_TICKS_AFTER, i64::from(after));
    }

    // Asking for a nonexistent entity id returns `Error::Handle(1)` (`e.is_stale() == true`).
    if let Err(err) = game_items_game_item_entity_handle(root, MISSING_ITEM_ID) {
        report(SLOT_MISSING_IS_STALE, i64::from(err.is_stale()));
    }

    // Run the rest of the `SERVER_TICK` chain. Note: `item` is `Locked` while `original` runs;
    // if you need it again after `original(root)`, request a fresh path handle.
    let _ = original(root);
    0
}

export_guest!(init, hook);
