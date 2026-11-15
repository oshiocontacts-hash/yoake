//! Trade exactly 16 cobblestone for one diamond with `/trade`.

use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, et, ext, services};

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

        // ext::take uses inventory_apply with components: None, so any
        // components match and the off hand is searched too. give_items fills
        // only the 36 main slots. The clear below is exact and all-or-nothing.
        match ext::take(&uuid, INPUT_ITEM, INPUT_COUNT) {
            Ok(true) => {}
            Ok(false) => {
                let _ = services::tell(
                    &format!("You need {INPUT_COUNT} {INPUT_ITEM} to trade."),
                    &uuid,
                );
                return true;
            }
            Err(err) => {
                let _ = services::tell(&format!("Could not take trade items: {err}"), &uuid);
                return true;
            }
        }

        // Never keep the input when the complete reward was not delivered.
        let reply = match services::give_items(&[ext::item(OUTPUT_ITEM, OUTPUT_COUNT)], &uuid) {
            Ok(leftovers) => {
                let left: u32 = leftovers.iter().map(|stack| stack.count).sum();
                if left == 0 {
                    format!("Traded {INPUT_COUNT} {INPUT_ITEM} for {OUTPUT_COUNT} {OUTPUT_ITEM}.")
                } else {
                    undo_trade(&uuid, OUTPUT_COUNT.saturating_sub(left), "Inventory full")
                }
            }
            Err(err) => undo_trade(&uuid, 0, &format!("Reward failed: {err}")),
        };
        let _ = services::tell(&reply, &uuid);
        true
    }
}

/// Remove a partially given reward, return the input, and describe any failure.
fn undo_trade(uuid: &str, reward_given: u32, reason: &str) -> String {
    if reward_given > 0 {
        match ext::take(uuid, OUTPUT_ITEM, reward_given) {
            Ok(true) => {}
            Ok(false) => return format!(
                "{reason}; trade cancelled, but undo failed: you kept the {reward_given} {OUTPUT_ITEM} reward and none of the {INPUT_COUNT} {INPUT_ITEM} were returned."
            ),
            Err(err) => return format!(
                "{reason}; trade cancelled, but undo failed: you kept the {reward_given} {OUTPUT_ITEM} reward and none of the {INPUT_COUNT} {INPUT_ITEM} were returned ({err})."
            ),
        }
    }

    match services::give_items(&[ext::item(INPUT_ITEM, INPUT_COUNT)], uuid) {
        Ok(leftovers) => {
            let missing: u32 = leftovers.iter().map(|stack| stack.count).sum();
            if missing == 0 {
                format!("{reason}; trade cancelled; your {INPUT_ITEM} were returned.")
            } else {
                let returned = INPUT_COUNT.saturating_sub(missing);
                format!(
                    "{reason}; trade cancelled, but undo was incomplete: no trade reward remains, {returned} of {INPUT_COUNT} {INPUT_ITEM} were returned, and {missing} were not."
                )
            }
        }
        Err(err) => format!(
            "{reason}; trade cancelled, but undo failed: no trade reward remains and none of the {INPUT_COUNT} {INPUT_ITEM} were returned ({err})."
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
