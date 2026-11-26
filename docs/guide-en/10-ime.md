# 10. Using IME (Input Method) on Mod Screens

A `client-full` mod can receive IME preedit (composition) text and committed text on its own mod screen (`Screen::Mod`). Services are also provided to place the OS candidate window next to your text box.

This chapter is for **sandboxed WebAssembly mods**. For full-access core modules, use the `hooks::IME_PREEDIT` / `hooks::IME_COMMIT` hooks and `stubs::f_services` (see [MOD-GUIDE.md](../yoake2/MOD-GUIDE.md)).

---

## 10-1. Subscriptions and Setup

Use these four hooks:

- `hook::SCREEN_INIT_POST`: Remember that the screen just opened (services cannot be called here).
- `hook::FRAME`: Enable IME and set the candidate window area on the next frame.
- `hook::IME_PREEDIT`: Receive composition text while the user is typing.
- `hook::IME_COMMIT`: Receive the final committed text.

> [!IMPORTANT]
> Client services require write access to the host. `screen_init_post` is read-only, so calling IME services inside it returns `Err`. Instead, set a flag in `screen_init_post` and call them in `frame` until they succeed.

```rust
use yoake_wasm_sdk::full::client::{ClientMod, et, ime_set_allowed, ime_set_cursor_area};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct TextScreen {
    composition: String,
    pending: bool,
}

impl ClientMod for TextScreen {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::SCREEN_INIT_POST, hook::FRAME, hook::IME_PREEDIT, hook::IME_COMMIT]
    }

    fn screen_init_post(&mut self, _screen: et::NativeScreenView) {
        // IME state resets each time a mod screen opens, so apply it again.
        self.pending = true;
    }

    fn frame(&mut self, _elapsed: u64) {
        if self.pending
            && ime_set_allowed(true).is_ok()
            && ime_set_cursor_area(32.0, 48.0, 180.0, 20.0).is_ok()
        {
            self.pending = false;
        }
    }

    fn ime_preedit(&mut self, text: String, cursor: Option<(u32, u32)>) {
        self.composition = text;
        println!("cursor: {cursor:?}");
    }

    fn ime_commit(&mut self, text: String) {
        self.composition.clear();
        println!("committed: {text}");
    }
}

yoake_wasm_sdk::export_client_full!(TextScreen);
```

A complete compiling example is in [`crates/yoake-wasm-sdk/examples/ime/`](../../crates/yoake-wasm-sdk/examples/ime/src/lib.rs). In a real screen, open your own mod screen via the screen API and draw `composition` inside your text box in `screen_render_post`.

IME allowance and the candidate area reset whenever a mod screen opens or closes. Call `ime_set_allowed(false)` when focus leaves your text box, and set `true` again when it regains focus.

**No impact on existing mods.** A mod screen that never calls `ime_set_allowed` or `ime_set_cursor_area` behaves just as before (IME enabled on mod screens opened from the title menu, disabled on in-game mod screens). Once either service is called, the screen follows your mod's setting.

---

## 10-2. How the Hooks Behave

### `ime_preedit(text, cursor)`

- `text` is the current composition string. An empty string means the composition was cleared. When the OS sends `Ime::Disabled`, the hook receives `""` and `None`.
- When `cursor` is `Some((start, end))`, it gives the selection range as **UTF-8 byte offsets** (not character counts or UTF-16 indices). `None` means no cursor range.
- The engine does not draw preedit text for you on a mod screen. Your mod draws it inside its own text box.

### `ime_commit(text)`

This hook fires once with the committed string before the existing `screen_char_typed_pre` hooks run. Afterward, per-character `screen_char_typed_pre` still fires as before.

> [!IMPORTANT]
> If you insert text inside `ime_commit`, do not also insert the same characters in `screen_char_typed_pre` on the same mod screen, or every character will be entered twice. Mods that only use `screen_char_typed_pre` continue to work unchanged.

These hooks fire **only when a mod screen is open and the mod subscribed to them**. IME events from vanilla chat or settings screens are never sent to mods, and non-subscribing mods are never called.

---

## 10-3. Services and Coordinates

### `ime_set_allowed(allowed)`

Turns the OS IME on or off for your own mod screen. Until you call it, the screen keeps the legacy behavior above. Changes take effect on the next IME sync for the window. Setting `false` also clears preedit and the candidate position on that screen.

### `ime_set_cursor_area(x, y, w, h)`

> [!WARNING]
> **Always use the `yoake_wasm_sdk::full::client::ime_set_cursor_area(x, y, w, h)` wrapper.** The raw generated WIT import `services::ime_set_cursor_area` sorts parameters alphabetically as **`(h, w, x, y)`**. Passing `(x, y, w, h)` to the raw import swaps the numbers and puts the candidate window in the wrong spot. The wrapper puts them back in order.

Sets the reference rectangle for the OS candidate window, usually your text box or caret area:

- `x`, `y`: Position from the top-left of the screen
- `w`, `h`: Width and height (`0` or greater)
- **Units**: **GUI-scaled coordinates**, the same as other screen APIs—not physical window pixels

The engine multiplies by the current GUI scale to convert to physical pixels, clamping width and height to at least 1 physical pixel. `NaN`, infinity, or negative width/height returns an error. Calling this every frame with the same rectangle only updates the OS window when the values change.

Both services require that **your own mod's screen is currently open**. Calling them when another mod's screen is open, or when no mod screen is open, returns `Err`.

`service.ime` is a catalog identifier only (service IDs **201** for `ime_set_allowed` and **202** for `ime_set_cursor_area`). Do not put `"service.ime"` in `[wasm] permissions`—it is not a manifest permission. These IDs are also separate from full-access hook IDs and `engine_call` binding IDs.
