//! Minimal `client-full` WebAssembly example that renders a HUD banner whose
//! text and colour come from two `[config]` fields in `yak.toml` (`text` and
//! `color`), refreshed whenever [`ClientMod::config_changed`] fires.
//!
//! Note: Calling `services::config_get` requires `permissions = ["service.config"]`
//! under `[wasm]` in `yak.toml`.

use yoake_wasm_sdk::export_client_full;
use yoake_wasm_sdk::full::client::{ClientMod, et, services, types};
use yoake_wasm_sdk::full::hook;

/// Cached HUD state backed by the package's `[config]` settings.
pub struct ConfigDemoMod {
    text: String,
    color: u32,
}

impl Default for ConfigDemoMod {
    fn default() -> Self {
        Self {
            text: "Config Demo HUD".to_owned(),
            color: color_from_preset("gold"),
        }
    }
}

impl ConfigDemoMod {
    /// Update cached HUD state from a typed [`types::ConfigValue`].
    fn apply_config(&mut self, key: &str, value: &types::ConfigValue) {
        match (key, value.kind) {
            ("text", types::ConfigValueKind::String) => {
                self.text.clone_from(&value.string_value);
            }
            ("color", types::ConfigValueKind::Enum | types::ConfigValueKind::String) => {
                self.color = color_from_preset(&value.string_value);
            }
            _ => {}
        }
    }
}

impl ClientMod for ConfigDemoMod {
    /// Subscribe to HUD rendering and config-change notifications.
    fn subscriptions() -> &'static [&'static str] {
        &[hook::HUD, hook::CONFIG_CHANGED]
    }

    /// Read the initial resolved config values when the component starts.
    fn init(&mut self) {
        for key in ["text", "color"] {
            if let Ok(value) = services::config_get(key) {
                self.apply_config(key, &value);
            }
        }
    }

    /// Called by the host when a setting in `[config]` changes (for example via
    /// the in-game Configure screen).
    fn config_changed(&mut self, key: String, value: types::ConfigValue) {
        self.apply_config(&key, &value);
    }

    /// Draw the HUD panel and the configured label each frame.
    fn hud(&mut self) -> et::HudOutcome {
        let panel_x = 8.0;
        let panel_y = 8.0;

        let background = et::HudCommand::Rect(et::HudRect {
            x: panel_x,
            y: panel_y,
            width: 160.0,
            height: 20.0,
            color: 0xAA10_141C,
        });

        let label = et::HudCommand::Text(et::HudText {
            text: json_text_component(&self.text),
            x: panel_x + 6.0,
            y: panel_y + 6.0,
            color: self.color,
            shadow: true,
        });

        et::HudOutcome {
            batch_commands: vec![background, label],
            corners_commands: Vec::new(),
        }
    }
}

/// Map the declared `[config.color]` enum choice to an ARGB colour.
fn color_from_preset(preset: &str) -> u32 {
    match preset {
        "gold" => 0xFFFF_AA00,
        "aqua" => 0xFF55_FFFF,
        "lime" => 0xFF55_FF55,
        "white" => 0xFFFF_FFFF,
        _ => 0xFFFF_FFFF,
    }
}

/// Encode a plain string as a minimal Minecraft JSON text component.
fn json_text_component(raw: &str) -> String {
    let mut escaped = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            c if c.is_control() => {}
            c => escaped.push(c),
        }
    }
    format!("{{\"text\":\"{escaped}\"}}")
}

export_client_full!(ConfigDemoMod);
