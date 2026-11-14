//! The full-catalog worlds with the `full-demo` guest (built per world by `yoake_wasm::test_guests`).

use std::sync::{Arc, Mutex};

use yoake_wasm::full_bindings::client::yoake::mods::engine_types as et;
use yoake_wasm::{ClientFullMod, ClientHost, FullWorld, HookSide, Sandbox, ServerFullMod, ServerHost, Subscriptions};

#[derive(Default, Clone)]
struct Log(Arc<Mutex<Vec<String>>>);

struct Server(Log);
impl ServerHost for Server {
    fn log(&mut self, target: &str, message: &str) -> Result<(), yoake_wasm::bindings::yoake::mods::types::HostError> {
        self.0.0.lock().unwrap().push(format!("{target}:{message}"));
        Ok(())
    }
    fn tell(&mut self, message: String, player: String) -> Result<(), String> {
        self.0.0.lock().unwrap().push(format!("tell {player}: {message}"));
        Ok(())
    }
}

struct Client(Log);
impl ClientHost for Client {
    fn log(&mut self, target: &str, message: &str) -> Result<(), yoake_wasm::bindings::yoake::mods::types::HostError> {
        self.0.0.lock().unwrap().push(format!("{target}:{message}"));
        Ok(())
    }
}

fn read(name: &str) -> Option<Vec<u8>> {
    use yoake_wasm::test_guests::{FULL_DEMO_CLIENT, FULL_DEMO_SERVER, load};
    load(if name.contains("server") { &FULL_DEMO_SERVER } else { &FULL_DEMO_CLIENT })
}

#[test]
fn server_guest_subscribes_registers_a_command_and_answers_perform_command() {
    let Some(bytes) = read("full_demo_server.wasm") else { return };
    let log = Log::default();
    let mut guest = ServerFullMod::instantiate(&mut Sandbox::new(), "demo", &bytes, Box::new(Server(log.clone())), &[]).unwrap();
    assert!(guest.subscribed("perform_command") && guest.subscribed("register_commands"));
    assert!(guest.subscribed("server_tick"), "the demo subscribes to server_tick");
    assert!(!guest.subscribed("level_tick_pre"));
    // Unsubscribed hooks are never called.
    assert_eq!(guest.dispatch("level_tick_pre", |_, _| Ok(())), None);
    let commands = guest
        .dispatch("register_commands", |e, s| {
            e.yoake_mods_server_hooks().call_register_commands(s, &[])
        })
        .unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].usage, "wasmping");
    let player = et::NativeServerPlayer {
        uuid: vec![0x11; 16], name: "Guest".into(), permission: 0, dimension: "minecraft:overworld".into(),
        position: vec![0.0; 3], game_mode: 1, health: 20.0, yaw: 0.0, pitch: 0.0,
    };
    let taken = guest
        .dispatch("perform_command", |e, s| e.yoake_mods_server_hooks().call_perform_command(s, &player, "wasmping"))
        .unwrap();
    assert!(taken);
    let other = guest
        .dispatch("perform_command", |e, s| e.yoake_mods_server_hooks().call_perform_command(s, &player, "other"))
        .unwrap();
    assert!(!other);
    let lines = log.0.lock().unwrap();
    assert!(lines.iter().any(|l| l.contains("tell 11111111-1111-1111-1111-111111111111: pong #1")), "{lines:?}");
}

#[test]
fn client_guest_returns_hud_commands_and_cancels_a_layer() {
    let Some(bytes) = read("full_demo_client.wasm") else { return };
    let log = Log::default();
    let mut guest = ClientFullMod::instantiate(&mut Sandbox::new(), "demo", &bytes, Box::new(Client(log)), &[]).unwrap();
    let outcome = guest.dispatch("hud", |e, s| e.yoake_mods_client_hooks().call_hud(s)).unwrap();
    assert_eq!(outcome.batch_commands.len(), 3);
    assert!(matches!(outcome.batch_commands[0], et::HudCommand::Rect(_)));
    let hide = guest.dispatch("gui_layer_pre", |e, s| e.yoake_mods_client_hooks().call_gui_layer_pre(s, "hotbar")).unwrap();
    assert!(hide.cancel);
    let keep = guest.dispatch("gui_layer_pre", |e, s| e.yoake_mods_client_hooks().call_gui_layer_pre(s, "chat")).unwrap();
    assert!(!keep.cancel);
}

#[test]
fn a_module_cannot_subscribe_to_the_other_sides_hooks_or_unwired_ones() {
    assert!(Subscriptions::for_full_world(FullWorld::Server, ["server_about_to_start"], &[]).is_ok());
    assert!(Subscriptions::for_full_world(FullWorld::Client, ["server_about_to_start"], &[]).is_err());
    assert!(Subscriptions::for_full_world(FullWorld::Client, ["tooltip"], &["tooltip"]).is_err());
    assert!(Subscriptions::for_full_world(FullWorld::Client, ["nope"], &[]).is_err());
    assert!(yoake_wasm::FULL_HOOK_LIST.len() > 400);
    assert!(yoake_wasm::FULL_HOOK_LIST.iter().any(|(n, s)| *n == "server_tick" && *s == HookSide::Both));
}
