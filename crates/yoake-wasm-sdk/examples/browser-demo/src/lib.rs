//! MEDIA1 browser example: toggles an embedded browser instance on `B` and paints its
//! latest frame onto a HUD `GuiTexture`.
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
            self.view = None;
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
