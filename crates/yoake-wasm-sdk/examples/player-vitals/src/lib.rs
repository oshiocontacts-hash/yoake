//! Report the sender's health and `minecraft:story/root` progress with `/vitals`.

use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, et, services};

const ADVANCEMENT: &str = "minecraft:story/root";

#[derive(Default)]
pub struct PlayerVitalsMod;

impl ServerMod for PlayerVitalsMod {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::REGISTER_COMMANDS, hook::PERFORM_COMMAND]
    }

    fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
        commands.push(et::NativeCommand {
            usage: "vitals".to_owned(),
            level: 0,
        });
        commands
    }

    fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool {
        let line = command.trim_start_matches('/').trim();
        if line != "vitals" && !line.starts_with("vitals ") {
            return false;
        }
        let uuid = format_player_uuid(&player.uuid);
        if line != "vitals" {
            let _ = services::tell("Usage: /vitals", &uuid);
            return true;
        }

        // The command hook's player record already contains current health.
        // advancement_progress also says whether this advancement is known.
        let health = match services::attribute_get("minecraft:max_health", &uuid) {
            Ok(max) => format!("health {:.1}/{:.1}", player.health, max.value),
            Err(_) => format!("health {:.1}", player.health),
        };
        let reply = match services::advancement_progress(ADVANCEMENT, &uuid) {
            Ok(state) => format!(
                "{}: {health}; {ADVANCEMENT} known={}, done={}.",
                player.name, state.known, state.done
            ),
            Err(err) => format!("{}: {health}; could not read {ADVANCEMENT}: {err}", player.name),
        };
        let _ = services::tell(&reply, &uuid);
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

export_server_full!(PlayerVitalsMod);
