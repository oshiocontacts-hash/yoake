use std::path::PathBuf;
use std::time::{Duration, Instant};

use horizon_protocol::connection::{Connection, Pipe};
use horizon_protocol::packets::{Empty, configuration as cfg, login, play};

use crate::game::execution_role::ExecutionRole;
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

fn with_remote(test: impl FnOnce(&mut Game<'_>, &mut Connection)) {
    let root = PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"),
    );
    let settings = Settings::default();
    let loaded = load_resources(
        root.clone(),
        None,
        &root,
        &settings,
        Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!("no menu handoff"),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = root.clone();
    options.execution_role = ExecutionRole::Client;
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&root),
        None,
    )
    .unwrap();
    let (client, server) = Pipe::pair();
    game.connect_local(client, "MouseTest").unwrap();
    let mut peer = Connection::local(server);
    peer.reader
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();

    peer.recv().unwrap();
    peer.recv().unwrap();
    peer.send(
        &login::Clientbound::LoginFinished(login::LoginFinished {
            profile: login::GameProfile {
                uuid: 1,
                name: "MouseTest".into(),
                properties: vec![],
            },
            session: 1,
        })
        .encode(),
    )
    .unwrap();
    peer.recv().unwrap();
    peer.recv().unwrap();
    peer.recv().unwrap();
    peer.send(&cfg::Clientbound::FinishConfiguration(Empty).encode())
        .unwrap();
    peer.recv().unwrap();

    let remote = game.remote.as_mut().unwrap();
    remote.loaded_sent = true;
    remote.carried = Some(game.selected);
    test(&mut game, &mut peer);
}

fn receive_action(peer: &mut Connection, expected: i32) -> play::PlayerAction {
    let (id, body) = peer.recv().unwrap();
    let play::Serverbound::PlayerAction(action) = play::Serverbound::decode(id, &body).unwrap()
    else {
        panic!("expected player action packet");
    };
    assert_eq!(action.action, expected);
    action
}

fn assert_no_packet(peer: &mut Connection) {
    peer.reader
        .set_read_timeout(Some(Duration::from_millis(100)))
        .unwrap();
    assert!(peer.recv().unwrap_err().is_timed_out());
}

#[test]
#[ignore = "requires prepared resource bundle"]
fn cancelling_attack_aborts_destroy_once() {
    with_remote(|game, peer| {
        game.left = true;
        let remote = game.remote.as_mut().unwrap();
        remote.is_destroying = true;
        remote.destroying = Some(([1, 64, 2], 1, 0.6));

        game.mouse_buttons_state.poll(true, false, true, |buttons| {
            buttons.cancel_vanilla_attack = true;
        });
        game.remote_handle_input();
        let action = receive_action(peer, 2);
        assert_eq!(action.pos, [1, 64, 2]);
        let remote = game.remote.as_ref().unwrap();
        assert!(!remote.is_destroying);
        assert_eq!(remote.destroying, Some(([1, 64, 2], 1, 0.0)));

        game.mouse_buttons_state.poll(true, false, true, |buttons| {
            buttons.cancel_vanilla_attack = true;
        });
        game.remote_handle_input();
        assert_no_packet(peer);
    });
}

#[test]
#[ignore = "requires prepared resource bundle"]
fn cancelling_use_releases_item_once() {
    with_remote(|game, peer| {
        game.right = true;
        game.using = Some((game.selected, 4));
        game.mouse_buttons_state.poll(false, true, true, |buttons| {
            buttons.cancel_vanilla_use = true;
        });
        game.remote_handle_input();
        receive_action(peer, 6);
        assert!(game.using.is_none());

        game.using = Some((game.selected, 4));
        game.mouse_buttons_state.poll(false, true, true, |buttons| {
            buttons.cancel_vanilla_use = true;
        });
        game.remote_handle_input();
        assert!(game.using.is_some());
        assert_no_packet(peer);

        game.right = false;
        game.mouse_buttons_state
            .poll(false, false, true, |buttons| {
                buttons.cancel_vanilla_use = true;
            });
        game.remote_handle_input();
        receive_action(peer, 6);
        assert!(game.using.is_none());
    });
}

#[test]
#[ignore = "requires prepared resource bundle"]
fn no_listener_keeps_held_use_without_release_packet() {
    with_remote(|game, peer| {
        game.right = true;
        game.using = Some((game.selected, 4));
        game.mouse_buttons_state
            .poll(false, true, false, |_| panic!("no listener"));
        game.remote_handle_input();
        assert!(game.using.is_some());
        assert_no_packet(peer);
    });
}
