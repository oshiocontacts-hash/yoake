//! The same mod as `hello`, written as a normal Rust `std` crate.

use std::collections::HashMap;
use std::time::Instant;

use yoake_wasm_sdk::{Mod, export_mod, hook, hud};

struct HelloStd {
    started: Instant,
    counts: HashMap<&'static str, u64>,
}

impl Default for HelloStd {
    fn default() -> Self {
        Self { started: Instant::now(), counts: HashMap::new() }
    }
}

impl Mod for HelloStd {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::TICK, hook::KEY, hook::HUD]
    }

    fn init(&mut self) {
        println!("hello-std: println works, args={}, env={}", std::env::args().count(), std::env::vars().count());
        eprintln!("hello-std: stderr works");
        // No directory is preopened, so no path can be opened.
        println!("hello-std: fs denied={}", std::fs::read_dir("/").is_err() && std::fs::File::open("/etc/hosts").is_err());
        let mut sorted: Vec<_> = self.counts.keys().collect();
        sorted.sort();
        println!("hello-std: map ok={} elapsed_ok={}", sorted.is_empty(), self.started.elapsed().as_secs() < 5);
        let _ = yoake_wasm_sdk::log("wasm-hello-std", "hello from a std WebAssembly mod");
    }

    fn key(&mut self, key: &str, pressed: bool) {
        if pressed && key == "EXIT" {
            std::process::exit(3);
        }
        // The sandbox answers a clock wait of more than a couple of milliseconds with a trap.
        if pressed && key == "SLEEP" {
            std::thread::sleep(std::time::Duration::from_secs(10));
        }
        if pressed && key == "NAP" {
            std::thread::sleep(std::time::Duration::from_micros(500));
        }
    }

    fn tick(&mut self) {
        *self.counts.entry("ticks").or_default() += 1;
    }

    fn hud(&mut self) {
        let ticks = self.counts.get("ticks").copied().unwrap_or(0);
        let _ = hud::text(
            &format!("wasm-hello-std #{ticks} {}ms", self.started.elapsed().as_millis()),
            (10.0, 11.0),
            0xffffff,
            true,
        );
    }
}

export_mod!(HelloStd);
