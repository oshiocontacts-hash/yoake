//! The calling half of the mod-call pair. Install `mod-call-responder` too.

use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, et, services};

const RESPONDER: &str = "mod-call-responder";

#[derive(Default)]
pub struct CallerMod;

impl ServerMod for CallerMod {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::REGISTER_COMMANDS, hook::PERFORM_COMMAND]
    }

    fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
        commands.push(et::NativeCommand {
            usage: "ping <text>".to_owned(),
            level: 0,
        });
        commands
    }

    fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool {
        let line = command.trim_start_matches('/').trim();
        if line != "ping" && !line.starts_with("ping ") {
            return false;
        }
        let uuid = format_player_uuid(&player.uuid);
        let Some(text) = line.strip_prefix("ping ").map(str::trim).filter(|s| !s.is_empty()) else {
            let _ = services::tell("Usage: /ping <text>", &uuid);
            return true;
        };

        // The target is the other package's id from its yak.toml. Payloads are
        // bytes; this example's private protocol is simply UTF-8 text.
        match services::mod_call("echo", text.as_bytes(), RESPONDER) {
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(answer) => {
                    let _ = services::tell(&format!("{RESPONDER} answered: {answer}"), &uuid);
                }
                Err(_) => {
                    let _ = services::tell("The responder returned invalid UTF-8.", &uuid);
                }
            },
            Err(err) => {
                let _ = services::tell(
                    &format!("Call failed: {err}. Is {RESPONDER} installed and enabled?"),
                    &uuid,
                );
            }
        }
        true
    }
}

/// Host services accept the canonical hyphenated form of the 16-byte UUID.
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

export_server_full!(CallerMod);
