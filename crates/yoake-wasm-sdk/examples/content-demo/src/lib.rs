//! Content demo. The mod's blocks, items, mob effect, recipe, loot table, mob type, creative tab and sound
//! are plain data in `content/` (see docs/mods.md, "Mob types"); this server module only reacts to them:
//!
//! * `interact_entity`: right-clicking a `content_demo:gem_cow` plays the mod's own sound event
//!   (`content_demo:gem_cow.moo`, from `assets/content_demo/sounds.json`) and tells the player;
//! * `mob_killed`: killing one is counted per world (a data attachment on the level) and reported;
//! * `entity_tick_post`: every gem cow ticked is counted, and the module logs the first and then each 2000th.
//!
//! Build: `cargo build --release --target wasm32-wasip2`, then `bin/wasm/content-demo-server.wasm`.

use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::ext;
use yoake_wasm_sdk::full::server::{ServerMod, et, services};

const COW: &str = "content_demo:gem_cow";

#[derive(Default)]
struct Demo {
    cow_ticks: u64,
    moos: u64,
}

impl ServerMod for Demo {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::INTERACT_ENTITY, hook::MOB_KILLED, hook::ENTITY_TICK_POST]
    }

    fn init(&mut self) {
        println!("content_demo server: init");
        let _ = ext::attachment_define("content_demo:kills", "level", true, false, false);
    }

    fn entity_tick_post(&mut self, _dimension: String, _entity: u32, kind: String) {
        if kind != COW {
            return;
        }
        self.cow_ticks += 1;
        if self.cow_ticks == 1 || self.cow_ticks % 2000 == 0 {
            println!("content_demo server: gem cow ticks {}", self.cow_ticks);
        }
    }

    fn interact_entity(&mut self, entity: i32, kind: String, _stack: Option<et::ItemStack>) -> bool {
        if kind != COW {
            return false;
        }
        self.moos += 1;
        let player = services::acting_player();
        println!("content_demo server: interact gem cow {entity} (moo #{})", self.moos);
        if let Some(player) = player {
            let _ = services::play_sound_at("neutral", &player.dimension, 1.0, player.position, "content_demo:gem_cow.moo", 1.0);
            let _ = services::tell(&format!("content_demo: the gem cow moos (#{})", self.moos), &player.uuid);
        }
        false
    }

    fn mob_killed(&mut self, player: Vec<u8>, entity: u32, kind: String) {
        if kind != COW {
            return;
        }
        let kills = ext::attachment_get("content_demo:kills", "level", "")
            .ok()
            .flatten()
            .and_then(|b| b.first().copied())
            .unwrap_or(0)
            + 1;
        let _ = ext::attachment_set("content_demo:kills", "level", "", &[kills]);
        println!("content_demo server: gem cow {entity} killed by {} bytes of player id, {kills} in this world", player.len());
    }
}

export_server_full!(Demo);
