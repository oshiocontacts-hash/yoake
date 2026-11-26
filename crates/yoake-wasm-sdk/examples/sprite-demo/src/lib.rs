//! L2 standalone sprite override example: tints `minecraft:apple` and `block/stone`
//! on first tick and restores the original pack sprites on `R`.
use yoake_wasm_sdk::full::client::{sprites, ClientMod};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct SpriteDemo {
    applied: bool,
}

fn tint_sprite(rgba: &mut [u8], r_mul: u8, g_mul: u8, b_mul: u8) {
    for px in rgba.chunks_exact_mut(4) {
        px[0] = ((u16::from(px[0]) * u16::from(r_mul)) / 255) as u8;
        px[1] = ((u16::from(px[1]) * u16::from(g_mul)) / 255) as u8;
        px[2] = ((u16::from(px[2]) * u16::from(b_mul)) / 255) as u8;
    }
}

impl ClientMod for SpriteDemo {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::TICK, hook::KEY]
    }

    fn tick(&mut self) {
        if self.applied {
            return;
        }
        self.applied = true;
        if let Ok((w, h, mut px)) = sprites::item_pixels("minecraft:apple") {
            tint_sprite(&mut px, 120, 220, 255);
            let _ = sprites::set_item("minecraft:apple", (w, h), &px);
        }
        if let Ok((w, h, mut px)) = sprites::block_pixels("block/stone") {
            tint_sprite(&mut px, 255, 180, 120);
            let _ = sprites::set_block("block/stone", (w, h), &px);
        }
    }

    fn key(&mut self, key: String, pressed: bool) {
        if pressed && key.eq_ignore_ascii_case("r") {
            let _ = sprites::reset_item("minecraft:apple");
            let _ = sprites::reset_block("block/stone");
        }
    }
}

yoake_wasm_sdk::export_client_full!(SpriteDemo);
