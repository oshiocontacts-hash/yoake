//! The server-tick guest of the end-to-end test: in the `Game`-root hook it finds the dropped item 4242 by id
//! through a Game path, reads its position, and writes a writable field. It registers only the server tick.
#![no_std]
extern crate alloc;

use yoake_guest_sdk::stubs::entities::{entities_itementity_fire_ticks_set, entities_itementity_position_get};
use yoake_guest_sdk::stubs::game_items::game_items_game_item_entity_handle;
use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks, report};

const ITEM: u32 = 4242;

fn init(r: &mut Registrar) {
    r.hook(hooks::SERVER_TICK, 0);
}

fn hook(id: u32, root: Handle) -> i64 {
    report(1, i64::from(id));
    let item = match game_items_game_item_entity_handle(root, ITEM) {
        Ok(h) => h,
        Err(e) => {
            report(2, match e {
                yoake_guest_sdk::Error::Handle(s) => -i64::from(s),
                _ => -99,
            });
            return 0;
        }
    };
    match entities_itementity_position_get(item) {
        Ok(p) => {
            report(3, p.x.to_bits() as i64);
            report(4, p.y.to_bits() as i64);
        }
        Err(_) => report(3, -1),
    }
    report(5, i64::from(entities_itementity_fire_ticks_set(item, 55).is_ok()));
    // a missing item is Stale, not a trap
    if let Err(e) = game_items_game_item_entity_handle(root, 1) {
        report(6, i64::from(e.is_stale()));
    }
    0
}

export_guest!(init, hook);
