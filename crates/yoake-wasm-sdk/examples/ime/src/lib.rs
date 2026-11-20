use yoake_wasm_sdk::full::client::{ClientMod, et, ime_set_allowed, ime_set_cursor_area};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct ImeDemo {
    composition: String,
    /// The services only work from hooks with write access (`frame`), not
    /// from `screen_init_post`, so the screen asks and the next frame calls.
    pending: bool,
}

impl ClientMod for ImeDemo {
    fn subscriptions() -> &'static [&'static str] {
        &[
            hook::SCREEN_INIT_POST,
            hook::FRAME,
            hook::IME_PREEDIT,
            hook::IME_COMMIT,
        ]
    }

    fn screen_init_post(&mut self, _screen: et::NativeScreenView) {
        // The IME state resets whenever a mod screen opens: set it up again.
        self.pending = true;
    }

    fn frame(&mut self, _elapsed: u64) {
        if !self.pending {
            return;
        }
        // Both calls fail while no screen of this mod is open: keep `pending`
        // and try again next frame. Use the wrapper `ime_set_cursor_area(x, y, w, h)`,
        // not the raw import `services::ime_set_cursor_area(h, w, x, y)`.
        if ime_set_allowed(true).is_ok() && ime_set_cursor_area(32.0, 48.0, 180.0, 20.0).is_ok() {
            self.pending = false;
        }
    }

    fn ime_preedit(&mut self, text: String, cursor: Option<(u32, u32)>) {
        self.composition = text;
        println!("IME cursor: {cursor:?}");
    }

    fn ime_commit(&mut self, text: String) {
        self.composition.clear();
        println!("IME committed: {text}");
    }
}

yoake_wasm_sdk::export_client_full!(ImeDemo);
