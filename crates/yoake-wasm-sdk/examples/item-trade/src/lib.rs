//! Trade exactly 16 cobblestone for one diamond with `/trade`.

use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, et, ext, services, types};

const INPUT_ITEM: &str = "minecraft:cobblestone";
const INPUT_COUNT: u32 = 16;
const OUTPUT_ITEM: &str = "minecraft:diamond";
const OUTPUT_COUNT: u32 = 1;

#[derive(Default)]
pub struct ItemTradeMod;

impl ServerMod for ItemTradeMod {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::REGISTER_COMMANDS, hook::PERFORM_COMMAND]
    }

    fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
        commands.push(et::NativeCommand {
            usage: "trade".to_owned(),
            level: 0,
        });
        commands
    }

    fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool {
        let line = command.trim_start_matches('/').trim();
        if line != "trade" && !line.starts_with("trade ") {
            return false;
        }
        let uuid = format_player_uuid(&player.uuid);
        if line != "trade" {
            let _ = services::tell("Usage: /trade", &uuid);
            return true;
        }

        // Give the reward first with inventory_apply's Give: all-or-nothing and
        // main slots only (services::give_items would also top up held and
        // off-hand stacks, but it never fills an empty off hand). If it does
        // not fit, nothing has changed yet.
        let reward = types::InventoryChange {
            operation: types::InventoryOperation::Give,
            item: OUTPUT_ITEM.to_owned(),
            count: OUTPUT_COUNT,
            components: None,
        };
        match services::inventory_apply(&reward, &uuid) {
            Ok(true) => {}
            Ok(false) => {
                let _ = services::tell("Your inventory is full; trade cancelled.", &uuid);
                return true;
            }
            Err(err) => {
                let _ = services::tell(&format!("Could not give the reward: {err}"), &uuid);
                return true;
            }
        }

        // Then take the input. ext::take matches any components and also
        // searches the off hand. If it fails, take the reward back.
        let reply = match ext::take(&uuid, INPUT_ITEM, INPUT_COUNT) {
            Ok(true) => {
                format!("Traded {INPUT_COUNT} {INPUT_ITEM} for {OUTPUT_COUNT} {OUTPUT_ITEM}.")
            }
            Ok(false) => undo_reward(&uuid, &format!("You need {INPUT_COUNT} {INPUT_ITEM} to trade")),
            Err(err) => undo_reward(&uuid, &format!("Could not take trade items: {err}")),
        };
        let _ = services::tell(&reply, &uuid);
        true
    }
}

/// Take the just-given reward back and describe the outcome.
fn undo_reward(uuid: &str, reason: &str) -> String {
    match ext::take_exact(uuid, &ext::item(OUTPUT_ITEM, OUTPUT_COUNT)) {
        Ok(true) => format!("{reason}; trade cancelled."),
        Ok(false) => format!(
            "{reason}; trade cancelled, but undo failed: the {OUTPUT_COUNT} {OUTPUT_ITEM} reward could not be taken back."
        ),
        Err(err) => format!(
            "{reason}; trade cancelled, but undo failed: the {OUTPUT_COUNT} {OUTPUT_ITEM} reward could not be taken back ({err})."
        ),
    }
}

fn format_player_uuid(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if hex.len() == 32 {
        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32]
        )
    } else {
        hex
    }
}

export_server_full!(ItemTradeMod);
