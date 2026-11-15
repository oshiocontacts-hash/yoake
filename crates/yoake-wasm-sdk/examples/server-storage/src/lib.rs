//! Minimal `server-full` WebAssembly example that registers a `/visits`
//! command, increments a persistent per-player counter via the `storage`
//! host service, and replies to the player with the updated count.
//!
//! Note: Using `services::storage_get` and `services::storage_put` requires
//! `permissions = ["service.storage"]` under `[wasm]` in `yak.toml`.

use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, et, services, types};

/// State for our server mod.
///
/// Because the visit counts are persisted in world-scoped host storage, the
/// mod struct itself does not need to hold any in-memory fields.
#[derive(Default)]
pub struct ServerStorageMod;

impl ServerMod for ServerStorageMod {
    /// Subscribe only to command registration and command execution hooks.
    fn subscriptions() -> &'static [&'static str] {
        &[hook::REGISTER_COMMANDS, hook::PERFORM_COMMAND]
    }

    /// Called when the server builds its command list.
    ///
    /// Adding `"visits"` with `level: 0` makes `/visits` available to all
    /// players without requiring operator permissions.
    fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
        commands.push(et::NativeCommand {
            usage: "visits".to_owned(),
            level: 0,
        });
        commands
    }

    /// Called when a player runs a slash command.
    ///
    /// Return `true` if this mod handled the command, or `false` to let the
    /// server continue normal command dispatch.
    fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool {
        let line = command.trim_start_matches('/').trim();
        if line != "visits" && !line.starts_with("visits ") {
            return false;
        }

        let uuid = format_player_uuid(&player.uuid);
        // Storage keys are logical names scoped to this mod (no path separators
        // or colons needed). We store one 8-byte little-endian u64 per player.
        let key = format!("visits.{uuid}");

        let current = match services::storage_get(&key, types::StorageScope::World) {
            Ok(Some(bytes)) => decode_u64(&bytes),
            Ok(None) => 0,
            Err(err) => {
                let _ = services::tell(&format!("Failed to read visits: {err}"), &uuid);
                return true;
            }
        };

        let next = current.saturating_add(1);
        if let Err(err) =
            services::storage_put(&key, types::StorageScope::World, &next.to_le_bytes())
        {
            let _ = services::tell(&format!("Failed to save visits: {err}"), &uuid);
            return true;
        }

        let reply = format!(
            "Hello, {}! You have used /visits {} time(s) in this world.",
            player.name, next
        );
        let _ = services::tell(&reply, &uuid);
        true
    }
}

/// Convert the 16-byte big-endian player UUID into a canonical hyphenated
/// string accepted by host services such as `services::tell`.
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

/// Decode an 8-byte little-endian `u64` from storage, returning `0` if the
/// stored slice has an unexpected length.
fn decode_u64(bytes: &[u8]) -> u64 {
    bytes
        .get(..8)
        .and_then(|slice| <[u8; 8]>::try_from(slice).ok())
        .map_or(0, u64::from_le_bytes)
}

export_server_full!(ServerStorageMod);
