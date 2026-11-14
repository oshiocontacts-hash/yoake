#![no_std]

use yoake_wasm_sdk::prelude::*;
use yoake_wasm_sdk::{Mod, export_mod, hook, hud};

#[derive(Default)]
struct Hello {
    ticks: u64,
}

impl Mod for Hello {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::TICK, hook::HUD, hook::SETTINGS]
    }

    fn init(&mut self) {
        let _ = yoake_wasm_sdk::log("wasm-hello", "hello from WebAssembly");
    }

    fn tick(&mut self) {
        self.ticks += 1;
    }

    fn hud(&mut self) {
        let _ = hud::rect([6.0, 6.0, 200.0, 18.0], 0x78000000);
        let _ = hud::text(&format!("wasm-hello #{}", self.ticks), (10.0, 11.0), 0xffffff, true);
    }

    fn settings(&mut self, _mod_id: &str) -> Vec<String> {
        vec!["Show greeting".to_string()]
    }
}

export_mod!(Hello);
