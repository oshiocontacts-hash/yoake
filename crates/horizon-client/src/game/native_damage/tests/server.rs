use std::path::PathBuf;
use std::sync::atomic::AtomicI32;
use std::sync::mpsc::{self, Receiver};
use std::time::Instant;

use horizon_client::chunk::Chunk;
use horizon_client::crash::{NativeModCall, enter_native_mod};
use horizon_client::gamerules::Value;
use horizon_client::lan::guest::Guest;
use horizon_client::lan::hub::HubConfig;
use horizon_client::lan::{Joined, NetConfig, OnlineMode, Out, PlayerSender};
use horizon_client::mod_api::NativeAuthority;
use horizon_client::motion::PlayerMotion;
use horizon_protocol::buf::Reader;
use horizon_protocol::packets::configuration::ClientInformation;
use horizon_protocol::packets::play;

use crate::game::execution_role::ExecutionRole;
use crate::game::lan::{LanSettings, Owner};
use crate::game::lan_actions::GuestPlay;
use crate::game::native_explosions::NativeBlast;
use crate::game::{Audio, Game, Options, Progress, Settings, load_resources};

use super::*;

const OWNER: u128 = 11;
const GUEST: u128 = 22;
const OWNER_CONN: u32 = 1;
const GUEST_CONN: u32 = 2;
const OWNER_ID: u32 = 41;
const GUEST_ID: u32 = 42;

// CHECK: Run these ignored outcome tests with an externally supplied resource bundle.
fn with_server(test: impl FnOnce(&mut Game<'_>, Receiver<Out>, Receiver<Out>)) {
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
    options.execution_role = ExecutionRole::IntegratedServer;
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
    game.native_lifecycle
        .begin_open(NativeAuthority::IntegratedServer);
    game.native_lifecycle.world_open_ready();
    game.native_lifecycle.server_tick_start(true);
    game.world
        .chunks
        .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let shapes = game.level_shapes;
    game.lan_host(
        LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        },
        NetConfig {
            listen: false,
            online: OnlineMode::lan(),
            levels: shapes.clone(),
            owner_profile: None,
            ..NetConfig::lan(0, "native damage test".into(), 20)
        },
        HubConfig {
            allow_flight: true,
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 10,
            simulation_distance: 10,
            dimension: game.dim.id().to_owned(),
            seed: 0,
            max_players: 20,
            integrated: true,
            levels: shapes,
        },
    )
    .unwrap();
    let owner_packets = add_player(&mut game, OWNER_CONN, OWNER, OWNER_ID, "Owner");
    let guest_packets = add_player(&mut game, GUEST_CONN, GUEST, GUEST_ID, "Guest");
    let host = game.lan.as_mut().unwrap();
    host.hub.singleplayer_owner = Some(OWNER);
    host.owner = Some(Owner {
        uuid: OWNER,
        saved: None,
    });
    let _caller = enter_native_mod(NativeModCall {
        id: Some("native_damage_test"),
        index: 0,
        hook: "level_post",
    });
    test(&mut game, owner_packets, guest_packets);
}

fn add_player(game: &mut Game<'_>, conn: u32, uuid: u128, id: u32, name: &str) -> Receiver<Out> {
    let (sender, receiver) = mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn,
            uuid,
            name: name.into(),
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: "vanilla".into(),
            sender: PlayerSender::new(sender, Arc::default()),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
        },
        id as i32,
        [5.0, 64.0, 5.0],
        8,
    );
    guest.placed = true;
    guest.dimension = game.dim.id().to_owned();
    guest.load_timeout = 0;
    guest.motion = Some(Box::new(
        PlayerMotion::new(game.motion, Vec3::new(5.0, 64.0, 5.0)).unwrap(),
    ));
    guest.motion.as_mut().unwrap().entity_id = id as i32;
    let host = game.lan.as_mut().unwrap();
    host.play.insert(conn, GuestPlay::default());
    host.hub.guests.push(guest);
    receiver
}

fn player_state<Result>(
    game: &mut Game<'_>,
    conn: u32,
    read: impl FnOnce(&mut Game<'_>) -> Result,
) -> Result {
    let mut host = game.lan.take().unwrap();
    let state = game.as_guest(&mut host, conn, read).unwrap();
    game.lan = Some(host);
    state
}

fn health(game: &mut Game<'_>, conn: u32) -> f32 {
    player_state(game, conn, |game| {
        game.player.as_ref().unwrap().health.health()
    })
}

fn velocity(game: &mut Game<'_>, conn: u32) -> Vec3 {
    player_state(game, conn, |game| game.player.as_ref().unwrap().velocity)
}

fn motion_packets(receiver: &Receiver<Out>) -> Vec<(i32, [f64; 3])> {
    let mut motions = Vec::new();
    for outbound in receiver.try_iter() {
        let packets = match outbound {
            Out::Packet(packet) => vec![packet],
            Out::Packets(packets) => packets,
            _ => continue,
        };
        for bytes in packets {
            let mut reader = Reader::new(&bytes);
            let packet_id = reader.varint().unwrap();
            let packet = play::Clientbound::decode(packet_id, reader.rest()).unwrap();
            if let play::Clientbound::SetEntityMotion(packet) = packet {
                motions.push((packet.id, packet.velocity));
            }
        }
    }
    motions
}

fn close_vector(actual: [f64; 3], expected: [f64; 3]) {
    for (actual, expected) in actual.into_iter().zip(expected) {
        assert!((actual - expected).abs() < 0.001, "{actual} != {expected}");
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn player_damage_preserves_victim_killer_and_disconnected_actor_identity() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        let damage = NativeDamage::new(
            NativeDamageKind::Magic,
            NativeDamageActor::Player(OWNER),
            None,
        );
        assert_eq!(
            game.native_hurt_entity(&dimension, GUEST_ID, 100.0, &damage),
            Ok(true)
        );
        assert!(health(game, GUEST_CONN) <= 0.0);
        close(health(game, OWNER_CONN), 20.0);
        let kills = player_state(game, OWNER_CONN, |game| {
            game.stats.get("custom", "minecraft:player_kills")
        });
        assert_eq!(kills, 1);
        let last_blow = player_state(game, GUEST_CONN, |game| {
            game.combat
                .last_blow()
                .map(|(kind, attacker)| (kind.to_owned(), attacker))
        });
        assert_eq!(last_blow, Some(("minecraft:magic".into(), OWNER_ID)));
        let stale = NativeDamage::new(
            NativeDamageKind::Magic,
            NativeDamageActor::Player(999),
            None,
        );
        assert!(
            game.native_hurt_entity(&dimension, OWNER_ID, 4.0, &stale)
                .is_err()
        );
        close(health(game, OWNER_CONN), 20.0);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn pvp_and_friendly_fire_refuse_without_changing_the_victim() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        let damage = NativeDamage::new(
            NativeDamageKind::PlayerAttack,
            NativeDamageActor::Player(OWNER),
            None,
        );
        game.ui.rules.set("pvp", Value::Bool(false));
        assert_eq!(
            game.native_hurt_entity(&dimension, GUEST_ID, 4.0, &damage),
            Ok(false)
        );
        close(health(game, GUEST_CONN), 20.0);
        game.ui.rules.set("pvp", Value::Bool(true));
        game.score.board.add_team("red").allow_friendly_fire = false;
        game.score.board.add_to_team("Owner", "red");
        game.score.board.add_to_team("Guest", "red");
        assert_eq!(
            game.native_hurt_entity(&dimension, GUEST_ID, 4.0, &damage),
            Ok(false)
        );
        close(health(game, GUEST_CONN), 20.0);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn owner_and_guest_damage_and_push_have_equivalent_results() {
    with_server(|game, owner_packets, guest_packets| {
        let dimension = game.dim.id().to_owned();
        let impulse = [0.25, 0.5, -0.125];
        assert_eq!(
            game.native_push_server_player(&dimension, OWNER, impulse),
            Ok(true)
        );
        assert_eq!(
            game.native_push_server_player(&dimension, GUEST, impulse),
            Ok(true)
        );
        assert_eq!(velocity(game, OWNER_CONN), velocity(game, GUEST_CONN));
        let owner_motion = motion_packets(&owner_packets);
        let guest_motion = motion_packets(&guest_packets);
        assert!(!owner_motion.is_empty());
        assert!(!guest_motion.is_empty());
        for (id, packet) in owner_motion {
            assert_eq!(id, OWNER_ID as i32);
            close_vector(packet, impulse);
        }
        for (id, packet) in guest_motion {
            assert_eq!(id, GUEST_ID as i32);
            close_vector(packet, impulse);
        }
        let damage = NativeDamage::new(NativeDamageKind::Magic, NativeDamageActor::None, None);
        assert_eq!(
            game.native_hurt_entity(&dimension, OWNER_ID, 4.0, &damage),
            Ok(true)
        );
        assert_eq!(
            game.native_hurt_entity(&dimension, GUEST_ID, 4.0, &damage),
            Ok(true)
        );
        close(health(game, OWNER_CONN), health(game, GUEST_CONN));
        assert_eq!(velocity(game, OWNER_CONN), velocity(game, GUEST_CONN));
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn undamaged_push_sends_additive_motion_and_damage_push_is_not_doubled() {
    with_server(|game, _, guest_packets| {
        let dimension = game.dim.id().to_owned();
        let initial = Vec3::new(0.125, 0.0, -0.25);
        player_state(game, GUEST_CONN, |game| {
            game.player.as_mut().unwrap().velocity = initial;
        });
        let impulse = [0.25, 0.5, -0.125];
        assert_eq!(
            game.native_push_server_player(&dimension, GUEST, impulse),
            Ok(true)
        );
        let sent = motion_packets(&guest_packets);
        assert!(!sent.is_empty());
        close_vector(sent.last().unwrap().1, [0.375, 0.5, -0.375]);
        let damage = NativeDamage::new(
            NativeDamageKind::PlayerAttack,
            NativeDamageActor::Player(OWNER),
            Some([3.0, 64.0, 5.0]),
        );
        assert_eq!(
            game.native_hurt_entity(&dimension, GUEST_ID, 4.0, &damage),
            Ok(true)
        );
        let damage_motion = motion_packets(&guest_packets);
        assert!(!damage_motion.is_empty());
        let before_extra = velocity(game, GUEST_CONN);
        assert_eq!(
            before_extra,
            initial.add_vector(Vec3::new(impulse[0], impulse[1], impulse[2]))
        );
        let sent_velocity = damage_motion.last().unwrap().1;
        assert_ne!(
            sent_velocity,
            [before_extra.x, before_extra.y, before_extra.z]
        );
        assert_eq!(
            game.native_push_server_player(&dimension, GUEST, impulse),
            Ok(true)
        );
        assert_eq!(
            velocity(game, GUEST_CONN),
            before_extra.add_vector(Vec3::new(impulse[0], impulse[1], impulse[2]))
        );
        let extra_motion = motion_packets(&guest_packets);
        assert!(!extra_motion.is_empty());
        close_vector(extra_motion.last().unwrap().1, [0.625, 1.0, -0.5]);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn parked_host_and_invalid_requests_leave_all_players_unchanged() {
    with_server(|game, _, guest_packets| {
        let dimension = game.dim.id().to_owned();
        let damage = NativeDamage::new(NativeDamageKind::Magic, NativeDamageActor::None, None);
        player_state(game, OWNER_CONN, |game| {
            assert!(
                game.native_hurt_entity(&dimension, GUEST_ID, 4.0, &damage)
                    .is_err()
            );
            assert!(
                game.native_push_server_player(&dimension, GUEST, [1.0, 0.0, 0.0])
                    .is_err()
            );
        });
        assert!(
            game.native_hurt_entity(&dimension, GUEST_ID, f32::NAN, &damage)
                .is_err()
        );
        assert!(
            game.native_hurt_entity(&dimension, GUEST_ID, -1.0, &damage)
                .is_err()
        );
        assert!(
            game.native_hurt_entity("minecraft:the_end", GUEST_ID, 4.0, &damage)
                .is_err()
        );
        assert!(
            game.native_hurt_entity(&dimension, u32::MAX, 4.0, &damage)
                .is_err()
        );
        close(health(game, GUEST_CONN), 20.0);
        assert_eq!(velocity(game, GUEST_CONN), Vec3::ZERO);
        assert!(motion_packets(&guest_packets).is_empty());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn exhausted_operation_budget_does_not_add_motion() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        for _ in 0..MAX_OPERATIONS {
            assert_eq!(
                game.native_push_server_player(&dimension, GUEST, [0.0, 0.001, 0.0]),
                Ok(true)
            );
        }
        let accepted = velocity(game, GUEST_CONN);
        assert!(
            game.native_push_server_player(&dimension, GUEST, [1.0, 0.0, 0.0])
                .is_err()
        );
        assert_eq!(velocity(game, GUEST_CONN), accepted);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn guest_actor_receives_mob_kill_credit_loot_and_experience() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        let mut victim = mob(0.0);
        let id = victim.id;
        victim.position = Vec3::new(8.0, 64.0, 8.0);
        game.mobs.push(victim);
        let damage = NativeDamage::new(
            NativeDamageKind::Magic,
            NativeDamageActor::Player(GUEST),
            None,
        );
        assert_eq!(
            game.native_hurt_entity(&dimension, id, 100.0, &damage),
            Ok(true)
        );
        let victim = game.mobs.iter().find(|victim| victim.id == id).unwrap();
        assert!(!victim.alive());
        assert_eq!(victim.ai.last_hurt_by_player, 100);
        assert_eq!(victim.last_damage.unwrap().0.attacker, Some(Who::Player));
        assert_eq!(
            player_state(game, GUEST_CONN, |game| {
                game.stats.get("killed", "minecraft:pig")
            }),
            1
        );
        assert_eq!(
            player_state(game, OWNER_CONN, |game| {
                game.stats.get("killed", "minecraft:pig")
            }),
            0
        );
        assert!(game.dropped.iter().any(|drop| {
            drop.stack.item.as_ref() == "minecraft:porkchop" && drop.stack.count > 0
        }));
        assert!(
            game.orbs
                .iter()
                .map(|orb| orb.value * orb.count)
                .sum::<u32>()
                > 0
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn sourceless_mob_death_has_no_player_credit_or_experience() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        let mut victim = mob(0.0);
        let id = victim.id;
        victim.position = Vec3::new(8.0, 64.0, 8.0);
        game.mobs.push(victim);
        let damage = NativeDamage::new(NativeDamageKind::Magic, NativeDamageActor::None, None);
        assert_eq!(
            game.native_hurt_entity(&dimension, id, 100.0, &damage),
            Ok(true)
        );
        assert!(game.orbs.is_empty());
        for conn in [OWNER_CONN, GUEST_CONN] {
            assert_eq!(
                player_state(game, conn, |game| {
                    game.stats.get("killed", "minecraft:pig")
                }),
                0
            );
        }
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn explosion_modes_preserve_or_destroy_the_loaded_center_block() {
    for interaction in [NativeBlast::Keep, NativeBlast::Block, NativeBlast::Tnt] {
        with_server(|game, _, _| {
            let dimension = game.dim.id().to_owned();
            let dirt = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dirt")
                .unwrap();
            game.level_set([8, 64, 8], dirt).unwrap();
            assert!(game.native_explode(&dimension, [8.0, 64.0, 8.0], 2.0, false, interaction));
            let block = game.world.block(8, 64, 8);
            if interaction == NativeBlast::Keep {
                assert_eq!(block, dirt);
            } else {
                assert_eq!(block, game.world.air);
            }
        });
    }
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn invalid_and_over_budget_explosions_leave_blocks_unchanged() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        let center = [8.0, 64.0, 8.0];
        let dirt = game
            .pack
            .assets
            .registry
            .default_state("minecraft:dirt")
            .unwrap();
        game.level_set([8, 64, 8], dirt).unwrap();
        for power in [0.0, -1.0, 32.001, f32::NAN, f32::INFINITY] {
            assert!(!game.native_explode(&dimension, center, power, false, NativeBlast::Block));
        }
        for at in [[f64::NAN, 64.0, 8.0], [32.0, 64.0, 8.0]] {
            assert!(!game.native_explode(&dimension, at, 2.0, false, NativeBlast::Block));
        }
        assert_eq!(game.world.block(8, 64, 8), dirt);
        for _ in 0..MAX_EXPLOSIONS {
            assert!(game.native_explode(&dimension, center, 2.0, false, NativeBlast::Keep));
        }
        assert!(!game.native_explode(&dimension, center, 2.0, false, NativeBlast::Block));
        assert_eq!(game.world.block(8, 64, 8), dirt);
    });
}
