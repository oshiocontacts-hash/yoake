# 15. Embedded Browser and Media Display (MEDIA1)

A `client-full` mod can open an installed **Edge or Chrome** browser in the background, draw its frames onto a `hud::GuiTexture` in the HUD or the world, forward mouse/keyboard/JavaScript input, and capture tab audio as PCM.

> [!NOTE]
> This feature is for `client-full` client modules only. You must declare `permissions = ["service.browser"]` under `[wasm]` in `yak.toml`, or every browser call returns a permission error.

---

## 15-1. `yak.toml` Setup

Excerpt from [`crates/yoake-wasm-sdk/examples/browser-demo/yak.toml`](../../crates/yoake-wasm-sdk/examples/browser-demo/yak.toml):

```toml
[side]
client = true
server = false
required_on_join = false

[wasm]
interface = "1.7"
world = "client-full"
client = "bin/wasm/browser-demo.wasm"
permissions = ["service.browser"]
```

---

## 15-2. Using `Browser` (`browser-demo`)

[`crates/yoake-wasm-sdk/examples/browser-demo/src/lib.rs`](../../crates/yoake-wasm-sdk/examples/browser-demo/src/lib.rs) toggles an embedded browser with `B` and paints its latest frame onto a HUD `GuiTexture`:

```rust
use yoake_wasm_sdk::full::client::bindings::yoake::mods::types::{TextureFilter, TextureFormat};
use yoake_wasm_sdk::full::client::media::{Browser, BrowserOptions};
use yoake_wasm_sdk::full::client::{self, et, hud, ClientMod};
use yoake_wasm_sdk::full::hook;

#[derive(Default)]
struct BrowserDemo {
    view: Option<(Browser, hud::GuiTexture, String)>,
}

impl ClientMod for BrowserDemo {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::HUD, hook::KEY]
    }

    fn key(&mut self, key: String, pressed: bool) {
        if !pressed || !key.eq_ignore_ascii_case("b") {
            return;
        }
        if self.view.is_some() {
            self.view = None; // Dropping `Browser` closes it automatically.
            return;
        }
        let options = BrowserOptions {
            allow: Vec::new(),
            audio: false,
            jpeg_quality: 80,
            max_message: 64 * 1024,
            transparent: false,
            window: false,
        };
        let Ok(browser) = Browser::open("https://example.com", 640, 360, 1.0, &options) else {
            return;
        };
        let Ok(tex) =
            hud::GuiTexture::create(640, 360, TextureFormat::Rgba8, TextureFilter::Linear)
        else {
            return;
        };
        if let Ok(id) = client::gui_texture_id(&tex) {
            self.view = Some((browser, tex, id));
        }
    }

    fn hud(&mut self) -> et::HudOutcome {
        let mut batch_commands = Vec::new();
        if let Some((browser, tex, id)) = &self.view {
            let _ = browser.frame_to_texture(tex);
            batch_commands.push(et::HudCommand::Texture(et::HudTexture {
                id: id.clone(),
                x: 16.0,
                y: 16.0,
                width: 320.0,
                height: 180.0,
                color: 0xffff_ffff,
            }));
        }
        et::HudOutcome {
            batch_commands,
            corners_commands: Vec::new(),
        }
    }
}

yoake_wasm_sdk::export_client_full!(BrowserDemo);
```

To draw in the 3D world instead of the HUD, pass the same `GuiTexture` to `ext::draw_world_textured_quad` or `ext::draw_world_billboard`.

---

## 15-3. Main Methods (`yoake_wasm_sdk::full::client::media`)

| Method / function | What it does |
|---|---|
| `Browser::open(url, width, height, scale, &options)` | Opens a browser instance (closed automatically on `Drop`). |
| `browser.frame_to_texture(&tex)` | Writes the newest frame as RGBA8 into `tex` (returns `Ok(None)` if unchanged). |
| `browser.frame_jpeg(quality)` / `media::jpeg_to_texture(&bytes, &tex)` | Encodes a frame as JPEG for streaming, and decodes received JPEG bytes on the host (up to 4 MiB, 3840×2160). |
| `browser.audio_pcm()` | Captures tab audio (48 kHz, 2-channel `Vec<i16>`). Requires `window: true` and `audio: true`. Feed it into `ext::AudioStream::push_i16`. |
| `browser.navigate(url)` / `back()` / `forward()` / `reload()` / `set_visible(bool)` | Navigation, history, and pausing rendering while hidden. |
| `browser.input(&event)` | Sends mouse, wheel, key, text, or paste input. |
| `browser.eval(js)` / `Browser::eval_result(ticket)` | Runs JavaScript on the page and polls the JSON result (expression up to 64 KiB, result up to 1 MiB). |
| `browser.post(json)` / `browser.messages()` | Sends and receives messages with the page. |

---

## 15-4. Safety Rules and Limits

- **Web pages only**: Only `http://`, `https://`, `data:`, `blob:`, and `about:blank` are allowed. `file://`, `chrome://`, `edge://`, and `javascript:` are refused, and file downloads are denied.
- **Per-package profile**: Stored under `<game>/media-browser/<mod id>/<slot>`, isolated from other mods and your personal browser profile.
- **Limits**: Up to **8** open browsers per package, max **3840×2160** pixels, **8 KiB** URLs, **64** `allow` prefixes, and **1 MiB** messages.
- **Cleanup**: Unloading or reloading the mod closes all of its open browsers.
- `"service.screen-share"` is reserved for a future screen-share phase and is not wired yet.
