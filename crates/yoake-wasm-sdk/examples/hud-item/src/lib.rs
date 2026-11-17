//! Minimal `client-full` WebAssembly example that renders an item icon and a
//! line of text on the in-game HUD every frame.
//!
//! To keep per-frame overhead low, Yoake only dispatches hooks listed in
//! [`ClientMod::subscriptions`]. Here we subscribe solely to [`hook::HUD`].

use yoake_wasm_sdk::export_client_full;
use yoake_wasm_sdk::full::client::{ClientMod, et};
use yoake_wasm_sdk::full::hook;

/// State for our client HUD mod.
///
/// `export_client_full!` instantiates this type with [`Default::default`] when
/// the WebAssembly component is loaded by the client.
#[derive(Default)]
pub struct HudItemMod;

impl ClientMod for HudItemMod {
    /// Declare which hooks the host should call on this mod.
    fn subscriptions() -> &'static [&'static str] {
        &[hook::HUD]
    }

    /// Called once per frame while the in-game HUD is visible.
    ///
    /// Instead of drawing pixels directly, a `client-full` mod returns a list
    /// of [`et::HudCommand`] primitives (`Rect`, `Item`, `Text`) that the host
    /// batches and renders in a single pass.
    fn hud(&mut self) -> et::HudOutcome {
        // Screen coordinates are in GUI-scaled pixels from the top-left corner.
        let panel_x = 8.0;
        let panel_y = 8.0;

        // 1. Semi-transparent dark background box (ARGB: 0xAA10141C).
        let background = et::HudCommand::Rect(et::HudRect {
            x: panel_x,
            y: panel_y,
            width: 132.0,
            height: 24.0,
            color: 0xAA10_141C,
        });

        // 2. A 16x16 item icon for "minecraft:diamond".
        //    `components` holds optional binary item component data; an empty
        //    Vec means a default stack with no extra components. The host
        //    currently ignores `decorations`, so no count or durability overlay is drawn.
        let icon = et::HudCommand::Item(et::HudItem {
            stack: et::ItemStack {
                id: "minecraft:diamond".to_owned(),
                count: 1,
                components: Vec::new(),
            },
            x: panel_x + 4.0,
            y: panel_y + 4.0,
            decorations: true,
        });

        // 3. A text label next to the icon. The host flattens the JSON text
        //    component to plain text and drops the colour's alpha channel.
        let label = et::HudCommand::Text(et::HudText {
            text: "{\"text\":\"Diamond HUD\"}".to_owned(),
            x: panel_x + 24.0,
            y: panel_y + 8.0,
            color: 0xFFFF_FFFF,
            shadow: true,
        });

        et::HudOutcome {
            batch_commands: vec![background, icon, label],
            corners_commands: Vec::new(),
        }
    }
}

export_client_full!(HudItemMod);
