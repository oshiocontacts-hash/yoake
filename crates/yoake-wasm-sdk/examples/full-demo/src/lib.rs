//! Full-catalog demo. One crate, two worlds, chosen by a Cargo feature:
//!
//! * `client` (`client-full`): the `hud` hook returns a HUD command list (a panel and a line of text)
//!   and the `gui_layer_pre` hook cancels the vanilla `hotbar` layer;
//! * `server` (`server-full`): `register_commands` adds `/wasmping` and `/wasmsvc`, `perform_command`
//!   answers them through the `tell` server service. `/wasmsvc` walks through the W2 services (storage,
//!   give and exact take, attribute add and read, break-block-as, a chest read and rearranged, tags, vitals,
//!   advancements, sound, mod-call) and tells the player one line per step;
//! * `peer` (`server-full`, a second package `full-demo-peer`): answers `mod-call` through `handle_call`;
//! * `client-peer` (`client-full`, package `full-demo-client-peer`): the same on the client side (`client::call`).

#[cfg(any(
    all(feature = "client", feature = "server"),
    all(feature = "client", feature = "peer"),
    all(feature = "server", feature = "peer"),
    all(feature = "client-peer", any(feature = "client", feature = "server", feature = "peer"))
))]
compile_error!("build the client, server and peer modules separately: one world per component");

#[cfg(feature = "client")]
mod client {
    use yoake_wasm_sdk::export_client_full;
    use yoake_wasm_sdk::full::client::{ClientMod, et};
    use yoake_wasm_sdk::full::hook;

    #[derive(Default)]
    struct Demo {
        frames: u64,
        layers_hidden: u64,
    }

    impl ClientMod for Demo {
        fn subscriptions() -> &'static [&'static str] {
            &[hook::HUD, hook::GUI_LAYER_PRE, hook::TOOLTIP, hook::COMPUTE_FOV]
        }

        fn init(&mut self) {
            println!("full-demo client: init");
        }

        fn hud(&mut self) -> et::HudOutcome {
            self.frames += 1;
            let text = format!("{{\"text\":\"full-demo: HUD commands from WASM (frame {}, hotbar hidden {}x)\"}}", self.frames, self.layers_hidden);
            et::HudOutcome {
                corners_commands: Vec::new(),
                batch_commands: vec![
                    et::HudCommand::Rect(et::HudRect { x: 4.0, y: 4.0, width: 250.0, height: 14.0, color: 0xC0102040 }),
                    et::HudCommand::Gradient(et::HudGradient { x: 4.0, y: 20.0, width: 250.0, height: 6.0, top: 0xFFFF8800, bottom: 0xFF880000 }),
                    et::HudCommand::Text(et::HudText { text, x: 7.0, y: 7.0, color: 0xFFFFFF, shadow: true }),
                ],
            }
        }

        /// Collect: the stack comes in, the lines come back with one more.
        fn tooltip(&mut self, stack: et::ItemStack, mut lines: Vec<(String, u32)>) -> Vec<(String, u32)> {
            lines.push((format!("{{\"text\":\"full-demo saw {} x{}\"}}", stack.id, stack.count), 0));
            lines
        }

        /// Mutate: widens the field of view by 10 degrees.
        fn compute_fov(&mut self, _partial: f32, _camera: Vec<f64>, degrees: f32) -> et::ComputeFovOutcome {
            et::ComputeFovOutcome { degrees: Some(degrees + 10.0) }
        }

        fn gui_layer_pre(&mut self, layer: String) -> et::GuiLayerPreOutcome {
            let cancel = layer == "hotbar" || layer.ends_with(":hotbar");
            if cancel {
                self.layers_hidden += 1;
                if self.layers_hidden == 1 {
                    println!("full-demo client: hiding the hotbar layer");
                }
            }
            et::GuiLayerPreOutcome { cancel, commands: Vec::new() }
        }
    }

    export_client_full!(Demo);
}

#[cfg(feature = "server")]
mod server {
    use yoake_wasm_sdk::export_server_full;
    use yoake_wasm_sdk::full::hook;
    use yoake_wasm_sdk::full::server::ext;
    use yoake_wasm_sdk::full::server::{ServerMod, et, services, types};

    #[derive(Default)]
    struct Demo {
        pings: u64,
        ticks: u64,
        /// `/wasmsvc` ran: the player (UUID, dimension) whose tick-time services run at the next tick.
        pending: Option<(String, String)>,
    }

    impl ServerMod for Demo {
        fn subscriptions() -> &'static [&'static str] {
            &[hook::REGISTER_COMMANDS, hook::PERFORM_COMMAND, hook::SERVER_TICK]
        }

        fn init(&mut self) {
            println!("full-demo server: init");
        }

        fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
            commands.push(et::NativeCommand { usage: "wasmping".to_owned(), level: 0 });
            commands.push(et::NativeCommand { usage: "wasmsvc".to_owned(), level: 0 });
            commands
        }

        fn server_tick(&mut self, _context: et::NativeLifecycleContext, _phase: et::ServerTickPhase) {
            self.ticks += 1;
            if self.ticks == 1 {
                println!("full-demo server: first server tick");
            }
            if let (Some((uuid, dimension)), et::ServerTickPhase::ServerTickStart(_)) = (self.pending.clone(), &_phase) {
                self.pending = None;
                for report in walk_tick(&uuid, &dimension) {
                    println!("full-demo server: {report}");
                    let _ = services::tell(&report, &uuid);
                }
            }
        }

        fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool {
            let line = command.trim_start_matches('/');
            if line == "wasmsvc" {
                let uuid = services_uuid(&player);
                for report in walk_hook(&uuid, &player.dimension) {
                    println!("full-demo server: {report}");
                    let _ = services::tell(&report, &uuid);
                }
                self.pending = Some((uuid, player.dimension.clone()));
                return true;
            }
            if line != "wasmping" {
                return false;
            }
            self.pings += 1;
            let reply = format!("pong #{} from the full-demo WASM server module", self.pings);
            println!("full-demo server: /wasmping from {} ({})", player.name, player.dimension);
            if let Err(error) = services::tell(&reply, &services_uuid(&player)) {
                println!("full-demo server: tell failed: {error}");
            }
            true
        }
    }

    fn step<T: core::fmt::Debug>(name: &str, result: Result<T, String>) -> String {
        match result {
            Ok(value) => format!("svc {name}: ok {value:?}"),
            Err(error) => format!("svc {name}: ERR {error}"),
        }
    }

    fn names(slots: &[Option<types::ItemStack>]) -> Vec<String> {
        slots.iter().flatten().map(|s| format!("{} x{}", s.item, s.count)).collect()
    }

    fn player_slot_names(slots: &[types::PlayerInventorySlot]) -> Vec<String> {
        slots
            .iter()
            .filter_map(|slot| slot.stack.as_ref().map(|stack| format!("{} [{}] x{}", stack.item, slot.id, stack.count)))
            .collect()
    }

    /// The services that work inside a player's hook (the player's own state is in place): one report
    /// line each.
    fn walk_hook(uuid: &str, dimension: &str) -> Vec<String> {
        let mut out = Vec::new();
        out.push(step("player-sneaking", services::player_sneaking(uuid)));
        // Per-world storage: a visit counter that survives between calls.
        let visits = ext::store_get("visits").ok().flatten().and_then(|b| b.first().copied()).unwrap_or(0) + 1;
        out.push(step("world-storage-write", ext::store_put("visits", &[visits])));
        out.push(step("world-storage-read", ext::store_get("visits")));
        // Data attachments: a persistent, synced, copy-on-death value on the player, and one on the level.
        out.push(step("attachment-define player", ext::attachment_define("full-demo:coins", "player", true, true, true)));
        out.push(step("attachment-get player before", ext::attachment_get("full-demo:coins", "player", uuid)));
        out.push(step("attachment-set player", ext::attachment_set("full-demo:coins", "player", uuid, &[visits, 7])));
        out.push(step("attachment-get player after", ext::attachment_get("full-demo:coins", "player", uuid)));
        out.push(step("attachment-define level", ext::attachment_define("full-demo:epoch", "level", true, false, false)));
        out.push(step("attachment-set level", ext::attachment_set("full-demo:epoch", "level", "", b"w2b")));
        out.push(step("attachment-get level", ext::attachment_get("full-demo:epoch", "level", "")));
        out.push(step("attachment-define wrong owner (must be ERR)", ext::attachment_define("other:x", "player", true, false, false)));
        // Give, then take exactly (all or nothing).
        out.push(step("give-items cobblestone x5 (left over)", ext::give(uuid, "minecraft:cobblestone", 5)));
        out.push(step("inventory-apply take 60 (all or nothing, must be false)", ext::take(uuid, "minecraft:cobblestone", 60)));
        out.push(step("inventory-apply take 2", ext::take(uuid, "minecraft:cobblestone", 2)));
        out.push(step("player-stacks (cobblestone left: 3)", services::player_stacks(uuid).map(|slots| player_slot_names(&slots))));
        // Attributes.
        out.push(step("player-attribute-modifier-add", ext::add_attribute(uuid, "minecraft:max_health", "full_demo:bonus", 4.0)));
        out.push(step("attribute-get max_health (base, value)", services::attribute_get("minecraft:max_health", uuid).map(|v| (v.base, v.value))));
        let _ = services::player_attribute_modifier_remove("minecraft:max_health", "full_demo:bonus", uuid);
        // Tags and tools.
        out.push(step("block-tags stone (count)", services::block_tags("minecraft:stone").map(|t| t.len())));
        out.push(step("item-tags stone_pickaxe (count)", services::item_tags("minecraft:stone_pickaxe").map(|t| t.len())));
        out.push(step("block-tool-requirement obsidian", services::block_tool_requirement("minecraft:obsidian").map(|r| (r.requires_tool, r.tier))));
        out.push(step("can-harvest obsidian by hand", services::can_harvest("minecraft:obsidian", None)));
        let player = services::server_players().ok().and_then(|ps| ps.into_iter().find(|p| p.uuid == uuid));
        let Some(player) = player else {
            out.push("svc server-players: player not found".to_owned());
            return out;
        };
        out.push(format!("svc server-players: sneaking={} position=({:.1}, {:.1}, {:.1})", player.sneaking, player.position.x, player.position.y, player.position.z));
        let (px, py, pz) = (player.position.x.floor() as i32, player.position.y.floor() as i32, player.position.z.floor() as i32);
        let here = ext::vec3(player.position.x, player.position.y, player.position.z);
        // Break the block under the player as the player; its drops appear as items we pick up.
        let below = (px, py - 1, pz);
        out.push(step("block below", services::block(ext::block_pos(below.0, below.1, below.2))));
        out.push(step("break-block-as", ext::break_block_as(dimension, uuid, below)));
        out.push(step("block below after", services::block(ext::block_pos(below.0, below.1, below.2))));
        let drops = services::item_entities_in(here, dimension, 4.0);
        out.push(step("item-entities-in", drops.clone().map(|d| d.iter().map(|e| format!("{} x{}", e.stack.item, e.stack.count)).collect::<Vec<_>>())));
        if let Ok(drops) = drops {
            if let Some(drop) = drops.first() {
                out.push(step("item-entity-move", services::item_entity_move(dimension, drop.handle, ext::vec3(player.position.x, player.position.y + 1.0, player.position.z), ext::vec3(0.0, 0.0, 0.0))));
                out.push(step("item-entity-pickup", services::item_entity_pickup(dimension, drop.handle, uuid)));
            }
        }
        out.push(step("advancement-progress story/root (known, done)", services::advancement_progress("minecraft:story/root", uuid).map(|a| (a.known, a.done))));
        out.push(step("play-sound-at (players reached)", services::play_sound_at("player", dimension, 1.0, here, "minecraft:entity.experience_orb.pickup", 1.0)));
        // Another package: the peer answers through handle-call.
        out.push(step("mod-call echo", ext::call("full-demo-peer", "echo", b"hello peer").map(|b| String::from_utf8_lossy(&b).into_owned())));
        out.push(step("mod-call count", ext::call("full-demo-peer", "count", b"").map(|b| String::from_utf8_lossy(&b).into_owned())));
        out.push(step("mod-call to nobody (must fail)", ext::call("no-such-package", "echo", b"")));
        out.push(step("break-block-as unloaded (must fail)", ext::break_block_as(dimension, uuid, (px + 100_000, py, pz))));
        out
    }

    /// The services that need no player's state swapped in (entity queries, the world border, commands, the
    /// chest): run from the server tick after `/wasmsvc`.
    fn walk_tick(uuid: &str, dimension: &str) -> Vec<String> {
        let mut out = vec!["svc -- tick part (no player state in place) --".to_owned()];
        let Some(player) = services::server_players().ok().and_then(|ps| ps.into_iter().find(|p| p.uuid == uuid)) else {
            out.push("svc server-players (tick): player not found".to_owned());
            return out;
        };
        let (px, py, pz) = (player.position.x.floor() as i32, player.position.y.floor() as i32, player.position.z.floor() as i32);
        let here = ext::vec3(player.position.x, player.position.y, player.position.z);
        // A chest next to the player, filled by a command, read and rearranged.
        let chest = (px + 2, py, pz);
        let edit = types::BlockEdit {
            palette: vec!["minecraft:chest".to_owned()],
            filters: Vec::new(),
            runs: vec![types::BlockRun { x: chest.0, y: chest.1, z: chest.2, height: 1, surface: None, block: 0, only: None }],
        };
        out.push(step("edit-blocks chest", services::edit_blocks(dimension, &edit)));
        out.push(step("run-command item replace", services::run_command(&format!("item replace block {} {} {} container.0 with minecraft:cobblestone 5", chest.0, chest.1, chest.2))));
        out.push(step("containers-in (kind, slots)", services::containers_in(here, dimension, 6.0).map(|c| c.iter().map(|r| (r.kind.clone(), r.slots)).collect::<Vec<_>>())));
        let slots = ext::container(dimension, chest);
        out.push(step("container-slots", slots.clone().map(|s| names(&s))));
        if let Ok(mut slots) = slots {
            if slots.len() > 5 {
                slots.swap(0, 5);
            }
            out.push(step("container-set-slots (moved slot 0 to 5)", ext::set_container(dimension, chest, &slots)));
            let mut forged = slots.clone();
            if forged.len() > 1 {
                forged[1] = Some(ext::item("minecraft:diamond", 3));
            }
            out.push(step("container-set-slots (forged diamonds, must fail)", ext::set_container(dimension, chest, &forged)));
            out.push(step("container-slots again: first filled slot (5)", ext::container(dimension, chest).map(|s| s.iter().position(|i| i.is_some()))));
        }
        // Vitals of the player's own entity, the world border, the attack hit box.
        let me = services::entities_in(here, dimension, 3.0);
        out.push(step("entities-in (kinds)", me.clone().map(|es| es.iter().map(|e| e.kind.clone()).collect::<Vec<_>>())));
        match me.ok().and_then(|es| es.into_iter().find(|e| e.player_uuid.as_deref() == Some(uuid))) {
            Some(me) => {
                out.push(step("entity-vitals-of (health, max, armor)", services::entity_vitals_of(dimension, me.handle).map(|v| (v.health, v.max_health, v.armor))));
                out.push(step("hurt-entity 1.0 (magic)", services::hurt_entity(1.0, None, types::DamageKind::Magic, dimension, me.handle)));
            }
            None => out.push("svc entity-vitals-of: own entity not in entities-in".to_owned()),
        }
        out.push(step("world-border (size, remaining ticks)", services::world_border(dimension).map(|b| (b.size, b.remaining_ticks))));
        out.push(step("set-player-attack-hitbox 1.2 x 2.4", services::set_player_attack_hitbox(2.4, uuid, 1.2)));
        out.push(step("clear-player-attack-hitbox", services::clear_player_attack_hitbox(uuid)));
        out.push(step("server-player-motion (on ground)", services::server_player_motion(uuid).map(|m| m.on_ground)));
        out
    }

    /// A player's canonical UUID text, from the hook's 16 big-endian bytes.
    fn services_uuid(player: &et::NativeServerPlayer) -> String {
        let hex: String = player.uuid.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
    }

    export_server_full!(Demo);
}

#[cfg(feature = "peer")]
mod peer {
    use yoake_wasm_sdk::export_server_full;
    use yoake_wasm_sdk::full::server::ServerMod;

    /// A package that does nothing but answer another package's `mod_call`.
    #[derive(Default)]
    struct Peer {
        calls: u64,
    }

    impl ServerMod for Peer {
        fn handle_call(&mut self, caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
            self.calls += 1;
            match method.as_str() {
                "echo" => {
                    let mut answer = format!("{caller} said: ").into_bytes();
                    answer.extend(payload);
                    Ok(answer)
                }
                "count" => Ok(format!("{} call(s) so far", self.calls).into_bytes()),
                _ => Err(format!("unknown method {method}")),
            }
        }
    }

    export_server_full!(Peer);
}

#[cfg(feature = "client-peer")]
mod client_peer {
    use yoake_wasm_sdk::export_client_full;
    use yoake_wasm_sdk::full::client::ClientMod;

    /// A client package that does nothing but answer another client package's `client::call`.
    /// `relay` calls `target` (given as the payload) so the tests can nest calls.
    #[derive(Default)]
    struct ClientPeer {
        calls: u64,
    }

    impl ClientMod for ClientPeer {
        fn handle_call(&mut self, caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
            self.calls += 1;
            match method.as_str() {
                "echo" => {
                    let mut answer = format!("{caller} said: ").into_bytes();
                    answer.extend(payload);
                    Ok(answer)
                }
                "count" => Ok(format!("{} call(s) so far", self.calls).into_bytes()),
                "relay" => {
                    let target = String::from_utf8(payload).map_err(|_| "relay target must be text".to_owned())?;
                    yoake_wasm_sdk::full::client::call(&target, "echo", b"relayed")
                }
                _ => Err(format!("unknown method {method}")),
            }
        }
    }

    export_client_full!(ClientPeer);
}
