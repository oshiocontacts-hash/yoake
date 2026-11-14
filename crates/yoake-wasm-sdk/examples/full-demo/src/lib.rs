//! Full-catalog demo. One crate, two worlds, chosen by a Cargo feature:
//!
//! * `client` (`client-full`): the `hud` hook returns a HUD command list (a panel and a line of text)
//!   and the `gui_layer_pre` hook cancels the vanilla `hotbar` layer;
//! * `server` (`server-full`): `register_commands` adds `/wasmping`, `perform_command` answers it
//!   through the `tell` server service.

#[cfg(all(feature = "client", feature = "server"))]
compile_error!("build the client and the server module separately: one world per component");

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
    use yoake_wasm_sdk::full::server::{ServerMod, et, services};

    #[derive(Default)]
    struct Demo {
        pings: u64,
        ticks: u64,
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
            commands
        }

        fn server_tick(&mut self, _context: et::NativeLifecycleContext, _phase: et::ServerTickPhase) {
            self.ticks += 1;
            if self.ticks == 1 {
                println!("full-demo server: first server tick");
            }
        }

        fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool {
            let line = command.trim_start_matches('/');
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

    /// A player's canonical UUID text, from the hook's 16 big-endian bytes.
    fn services_uuid(player: &et::NativeServerPlayer) -> String {
        let hex: String = player.uuid.iter().map(|byte| format!("{byte:02x}")).collect();
        format!("{}-{}-{}-{}-{}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
    }

    export_server_full!(Demo);
}
