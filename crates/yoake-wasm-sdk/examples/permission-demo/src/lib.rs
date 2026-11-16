//! Minimal `server-full` WebAssembly example that registers a `/secret`
//! command and replies differently depending on whether the player has the
//! `"secret.access"` permission node checked via [`services::has_permission`].
//!
//! Note: Using `services::has_permission` requires:
//! 1. `permissions = ["service.permissions"]` under `[wasm]` in `yak.toml`, and
//! 2. declaring the node under `[permissions."secret.access"]` in `yak.toml`.

use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, et, services};

/// Permission node declared in `yak.toml` under `[permissions."secret.access"]`.
const SECRET_NODE: &str = "secret.access";

/// State for our server permission demo mod.
#[derive(Default)]
pub struct PermissionDemoMod;

impl ServerMod for PermissionDemoMod {
    /// Subscribe only to command registration and command execution hooks.
    fn subscriptions() -> &'static [&'static str] {
        &[hook::REGISTER_COMMANDS, hook::PERFORM_COMMAND]
    }

    /// Register `/secret` with `level: 0` so any player can invoke the command
    /// and reach [`ServerMod::perform_command`], where `services::has_permission`
    /// performs the fine-grained permission node check (including per-world
    /// `permissions.toml` overrides and `default_op_level` fallback).
    fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
        commands.push(et::NativeCommand {
            usage: "secret".to_owned(),
            level: 0,
        });
        commands
    }

    /// Handle `/secret` by checking whether the player has `SECRET_NODE`.
    fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool {
        let line = command.trim_start_matches('/').trim();
        if line != "secret" && !line.starts_with("secret ") {
            return false;
        }

        let uuid = format_player_uuid(&player.uuid);
        let reply = match services::has_permission(SECRET_NODE, &uuid) {
            Ok(true) => format!(
                "Welcome, {}! You have '{}' — the secret code is YOAKE-DAWN.",
                player.name, SECRET_NODE
            ),
            Ok(false) => format!(
                "Access denied, {}: you need the '{}' permission node to use /secret.",
                player.name, SECRET_NODE
            ),
            Err(err) => format!("Failed to check permission '{SECRET_NODE}': {err}"),
        };

        let _ = services::tell(&reply, &uuid);
        true
    }
}

/// Convert the 16-byte big-endian player UUID into a canonical hyphenated
/// string accepted by host services such as `services::has_permission` and
/// `services::tell`.
fn format_player_uuid(bytes: &[u8]) -> String {
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    if hex.len() == 32 {
        format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        )
    } else {
        hex
    }
}

export_server_full!(PermissionDemoMod);
