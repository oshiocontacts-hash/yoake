//! Register `/chestcount <x> <y> <z>` and total the items in that container.

use std::collections::BTreeMap;
use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, et, services, types};

#[derive(Default)]
pub struct ChestCountMod;

impl ServerMod for ChestCountMod {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::REGISTER_COMMANDS, hook::PERFORM_COMMAND]
    }

    fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
        commands.push(et::NativeCommand {
            usage: "chestcount <x> <y> <z>".to_owned(),
            level: 0,
        });
        commands
    }

    fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool {
        let line = command.trim_start_matches('/').trim();
        if line != "chestcount" && !line.starts_with("chestcount ") {
            return false;
        }
        let uuid = format_player_uuid(&player.uuid);
        let mut words = line.split_whitespace();
        let coordinates = (words.nth(1), words.next(), words.next());
        let (Some(x), Some(y), Some(z), None) = (coordinates.0, coordinates.1, coordinates.2, words.next()) else {
            let _ = services::tell("Usage: /chestcount <x> <y> <z>", &uuid);
            return true;
        };
        let (Ok(x), Ok(y), Ok(z)) = (x.parse::<i32>(), y.parse::<i32>(), z.parse::<i32>()) else {
            let _ = services::tell("Coordinates must be whole numbers.", &uuid);
            return true;
        };

        // Use the sender's dimension so the same coordinates work in every world.
        let position = types::BlockPos { x, y, z };
        match services::container_slots(&player.dimension, position) {
            Ok(slots) => {
                let mut totals = BTreeMap::<String, u64>::new();
                for stack in slots.into_iter().flatten() {
                    *totals.entry(stack.item).or_default() += u64::from(stack.count);
                }
                let reply = if totals.is_empty() {
                    format!("Container at {x} {y} {z} is empty.")
                } else {
                    let items = totals
                        .iter()
                        .map(|(item, count)| format!("{item} x{count}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("Container at {x} {y} {z}: {items}")
                };
                let _ = services::tell(&reply, &uuid);
            }
            Err(err) => {
                let _ = services::tell(
                    &format!("Could not read a container at {x} {y} {z}: {err}"),
                    &uuid,
                );
            }
        }
        true
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

export_server_full!(ChestCountMod);
