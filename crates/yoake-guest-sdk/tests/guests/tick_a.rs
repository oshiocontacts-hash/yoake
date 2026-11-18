//! A server-tick mod of the two-mod test: priority 10. Before `original` it reads the item's fire_ticks (what the
//! mods outside it have done), adds 1, then runs the next mod (and finally the engine) and reads it again.
#![no_std]
extern crate alloc;

use yoake_guest_sdk::stubs::entities::{entities_itementity_fire_ticks_get, entities_itementity_fire_ticks_set};
use yoake_guest_sdk::stubs::game_items::game_items_game_item_entity_handle;
use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks, original, report};

const ITEM: u32 = 4242;
const BASE: i32 = 1;

fn init(r: &mut Registrar) {
    r.hook(hooks::SERVER_TICK, 10).unwrap();
}

fn hook(_id: u32, root: Handle) -> i64 {
    let Ok(item) = game_items_game_item_entity_handle(root, ITEM) else {
        report(BASE + 5, 1);
        return 0;
    };
    let before = entities_itementity_fire_ticks_get(item).unwrap_or(-1);
    report(BASE, i64::from(before));
    let _ = entities_itementity_fire_ticks_set(item, before + 1);
    // the item handle is Locked while `original` runs; ask again afterwards
    let _ = original(root);
    let after = game_items_game_item_entity_handle(root, ITEM)
        .and_then(entities_itementity_fire_ticks_get)
        .unwrap_or(-1);
    report(BASE + 1, i64::from(after));
    0
}

export_guest!(init, hook);
