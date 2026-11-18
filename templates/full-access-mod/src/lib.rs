//! Starter full-access mod for Yoake 2.0.
//!
//! It registers one hook, the server tick, and on every tick it makes engine calls: it reads the experience
//! total of the `Game`. Then it lets the engine (and any other mod) run by calling `original`.
//!
//! This is a `#![no_std]` WebAssembly core module. Build it with `build.sh` / `build.ps1` (see README.md).
#![no_std]
extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};

use yoake_guest_sdk::stubs::entities::entities_experience_total_get;
use yoake_guest_sdk::stubs::game_items::game_items_game_experience_handle;
use yoake_guest_sdk::{Error, Handle, Registrar, export_guest, hooks, original, report};

// `report(slot, value)` is a test aid: the host keeps the pairs in memory and only tests read them. The real game
// never drains that list, so call it a few times at most (here: the first 3 ticks), never on every tick.
const SLOT_TICK: i32 = 1;
const SLOT_XP: i32 = 2;
const REPORT_FIRST_TICKS: u32 = 3;

static TICKS: AtomicU32 = AtomicU32::new(0);

/// Runs once when the mod is loaded. `hook_register` is accepted only here.
fn init(r: &mut Registrar) {
    // `Ok(HookMode::Full)` for the server tick. A refusal is a bug in the mod: panic, and the mod is not loaded.
    r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK");
}

/// What a failed engine call becomes in a report: a handle error is -100 - code (`Stale` is -101), a refused call
/// is its raw status (for example -5, unknown binding), and a result that did not decode is -50.
fn error_code(err: Error) -> i64 {
    match err {
        Error::Handle(code) => -100 - i64::from(code),
        Error::Status(status) => i64::from(status),
        Error::Decode(_) => -50,
    }
}

/// Called for every hook id registered in `init`. For the server tick, `root` is the `Game` root handle.
fn hook(id: u32, root: Handle) -> i64 {
    if id != hooks::SERVER_TICK {
        return original(root).unwrap_or(0);
    }

    let tick = TICKS.fetch_add(1, Ordering::Relaxed) + 1;

    // Engine calls on a path handle: `Game.experience#handle` gives a handle to the experience object, and
    // `Experience.total#get` reads a field of it. Do this BEFORE `original`: every handle is Locked while it runs.
    let total = game_items_game_experience_handle(root).and_then(entities_experience_total_get);

    if tick <= REPORT_FIRST_TICKS {
        report(SLOT_TICK, i64::from(tick));
        report(SLOT_XP, total.map_or_else(error_code, i64::from));
    }

    // Let the rest of the chain (other mods, then the engine's own tick) run. The server tick has no result.
    let _ = original(root);
    0
}

export_guest!(init, hook);
