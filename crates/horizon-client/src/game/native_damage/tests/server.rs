use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::AtomicI32;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use horizon_client::chunk::Chunk;
use horizon_client::crash::{NativeModCall, enter_native_mod};
use horizon_client::gamerules::Value;
use horizon_client::lan::guest::Guest;
use horizon_client::lan::hub::HubConfig;
use horizon_client::lan::{Joined, NetConfig, OnlineMode, Out, PlayerSender};
use horizon_client::mod_api::{NativeAuthority, NativeTeleportStatus};
use horizon_client::motion::PlayerMotion;
use horizon_protocol::buf::Reader;
use horizon_protocol::packets::configuration::ClientInformation;
use horizon_protocol::packets::play;
use horizon_world::nbt::{Compound, Tag};

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
pub(in crate::game) fn with_server(test: impl FnOnce(&mut Game<'_>, Receiver<Out>, Receiver<Out>)) {
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
    options.dedicated = Some(Vec::new());
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
            loopback_only: false,
            announce: true,
            max_players: horizon_client::lan::MAX_PLAYERS,
            game_mode: None,
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

fn load_move_chunks(game: &mut Game<'_>) {
    for chunk in [(0, -3), (0, -2)] {
        game.world
            .chunks
            .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
    }
}

fn assert_entity_saved(game: &mut Game<'_>, kind: &str, position: [f64; 3]) {
    let chunk = ((position[0].floor() as i32) >> 4, (position[2].floor() as i32) >> 4);
    let saved = game.entity_chunks(&HashSet::from([chunk]), false);
    let expected_position = Tag::List(
        position
            .into_iter()
            .map(|value| Tag::Double(value.to_bits()))
            .collect(),
    );
    assert!(saved.iter().any(|(key, entities)| {
        *key == chunk
            && entities.iter().any(|tag| {
                tag.get("id") == Some(&Tag::String(kind.into()))
                    && tag.get("Pos") == Some(&expected_position)
            })
    }));
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn moved_item_on_stone_floor_survives_server_ticks() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        load_move_chunks(game);
        for block_x in 7..=9 {
            for block_z in -35..=-30 {
                game.world.set(block_x, 99, block_z, stone).unwrap();
            }
        }
        game.spawn_item(
            ItemStack::new("minecraft:stone", 1),
            [8.0, 104.0, 8.0],
            [0.0; 3],
            0,
        );
        let entity_id = game.next_entity;
        game.update_chunk_tickets();
        let mut ticks = 0;
        for position in [
            [8.202, 100.0, -33.102],
            [8.202, 100.0, -32.308],
            [8.202, 100.0, -31.9],
        ] {
            assert!(
                game.native_item_entity_move(&dimension, entity_id, position, [0.0; 3])
                    .unwrap()
            );
            let item = game.dropped.iter().find(|item| item.id == entity_id).unwrap();
            assert_eq!(item.previous, item.position);
            for _ in 0..5 {
                game.tick_items();
                ticks += 1;
                let item = game
                    .dropped
                    .iter()
                    .find(|item| item.id == entity_id)
                    .expect("moved item must remain on the server");
                assert_eq!(item.age, ticks);
                assert_eq!(item.health, 5);
                assert_eq!(item.stack.count, 1);
                assert_eq!(item.pickup_delay, 0);
                assert!((item.position.y - 100.0).abs() < 1.0e-6);
                assert!(item.on_ground);
                let nearby = game
                    .native_item_entities_in(&dimension, position, 1.0)
                    .unwrap();
                assert!(nearby.iter().any(|item| item.0 == entity_id));
            }
            assert_entity_saved(game, "minecraft:item", position);
        }
        game.entity_chunks(&HashSet::from([(0, 0), (0, -3)]), true);
        assert!(game.dropped.iter().any(|item| item.id == entity_id));
        assert_entity_saved(game, "minecraft:item", [8.202, 100.0, -31.9]);
        assert_eq!(
            game.native_item_entity_move(&dimension, entity_id, [8.0, 100.0, -0.01], [1.0; 3]),
            Err("the destination chunk is not loaded".into())
        );
        let item = game.dropped.iter().find(|item| item.id == entity_id).unwrap();
        assert_eq!(item.position, Vec3::new(8.202, 100.0, -31.9));
        assert_eq!(item.age, ticks);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn moved_mob_survives_source_chunk_unload_and_saves_in_destination() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        load_move_chunks(game);
        game.mobs.push(mob(0.0));
        for position in [
            [8.202, 100.0, -33.102],
            [8.202, 100.0, -32.308],
            [8.202, 100.0, -31.9],
        ] {
            assert!(game.native_teleport_mob(&dimension, 71, position));
            let moved = game.mobs.iter().find(|mob| mob.id == 71).unwrap();
            assert_eq!(moved.position, Vec3::new(position[0], position[1], position[2]));
            assert_eq!(moved.previous, moved.position);
            assert!(moved.needs_sync);
            assert!(game.native_mob(&dimension, 71).is_some());
            assert_entity_saved(game, "minecraft:pig", position);
        }
        game.entity_chunks(&HashSet::from([(0, 0), (0, -3)]), true);
        assert!(game.native_mob(&dimension, 71).is_some());
        assert!(!game.native_teleport_mob(&dimension, 71, [8.0, 100.0, -0.01]));
        assert_entity_saved(game, "minecraft:pig", [8.202, 100.0, -31.9]);
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn moved_other_entities_survive_source_chunk_unload_and_save_in_destination() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        load_move_chunks(game);
        for kind in [
            "minecraft:marker",
            "minecraft:interaction",
            "minecraft:block_display",
            "minecraft:item_display",
            "minecraft:text_display",
        ] {
            let entity_id = game
                .native_summon(&dimension, kind, [8.0, 100.0, 8.0], Compound::new())
                .unwrap();
            for position in [
                [8.202, 100.0, -33.102],
                [8.202, 100.0, -32.308],
                [8.202, 100.0, -31.9],
            ] {
                assert!(game.native_move_entity(&dimension, entity_id, position, 45.0, 20.0));
                let moved = game.others.iter().find(|entity| entity.id == entity_id).unwrap();
                assert_eq!(moved.position, Vec3::new(position[0], position[1], position[2]));
                assert_eq!((moved.yaw, moved.pitch), (45.0, 20.0));
                assert_entity_saved(game, kind, position);
            }
            game.entity_chunks(&HashSet::from([(0, 0), (0, -3)]), true);
            assert!(game.others.iter().any(|entity| entity.id == entity_id));
            assert!(!game.native_move_entity(
                &dimension, entity_id, [8.0, 100.0, -0.01], 90.0, 30.0
            ));
            assert_entity_saved(game, kind, [8.202, 100.0, -31.9]);
            let unchanged = game.others.iter().find(|entity| entity.id == entity_id).unwrap();
            assert_eq!((unchanged.yaw, unchanged.pitch), (45.0, 20.0));
        }
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn player_teleports_load_destinations_five_hundred_blocks_away() {
    with_server(|game, owner_packets, guest_packets| {
        let dimension = game.dim.id().to_owned();
        game.lan.as_mut().unwrap().hub.config.view_distance = 0;
        for guest in &mut game.lan.as_mut().unwrap().hub.guests {
            guest.position = [5.5, 100.0, 5.5];
            guest.motion.as_mut().unwrap().position = Vec3::new(5.5, 100.0, 5.5);
        }
        game.streaming.paused = false;
        game.streaming.mesh = false;
        game.streaming.radius = -1;
        for (conn, uuid, packets, position) in [
            (OWNER_CONN, OWNER, &owner_packets, [505.5, 100.0, 5.5]),
            (GUEST_CONN, GUEST, &guest_packets, [-494.5, 100.0, 5.5]),
        ] {
            clientbound(packets);
            let destination_chunk = ((position[0] as i32) >> 4, 0);
            assert!(!game.world.chunks.contains_key(&destination_chunk));
            let ticket = game
                .native_player_teleport_impl(uuid, &dimension, position, 45.0, 20.0)
                .unwrap();
            assert_eq!(
                game.native_player_teleport_status_impl(ticket),
                NativeTeleportStatus::Done
            );
            game.tick_lan(true);
            let deadline = Instant::now() + Duration::from_secs(10);
            while !game.world.chunks.contains_key(&destination_chunk) {
                game.streaming.update(&mut game.world);
                assert!(Instant::now() < deadline, "teleport destination did not load");
                game.streaming.wait(Duration::from_millis(10));
            }
            player_state(game, conn, |game| {
                assert_eq!(
                    game.player.as_ref().unwrap().position,
                    Vec3::new(position[0], position[1], position[2])
                );
            });
            assert!(clientbound(packets).iter().any(|packet| {
                matches!(packet, play::Clientbound::PlayerPosition(packet)
                    if packet.position == position && packet.yaw == 45.0 && packet.pitch == 20.0)
            }));
        }
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn level_post_pickup_credits_the_integrated_owner_not_the_stand_in() {
    with_server(|game, _, _| {
        load_move_chunks(game);
        let dimension = game.dim.id().to_owned();
        game.level.player_uuid = Some(horizon_client::lan::playerdata::uuid_ints(OWNER));
        game.player =
            Some(PlayerMotion::new(game.motion, Vec3::new(5.5, 100.0, -29.5)).unwrap());
        game.spawn_item(
            ItemStack::new("minecraft:emerald", 4),
            [8.5, 100.0, -33.5],
            [0.0; 3],
            0,
        );
        let item_id = game.dropped.last().unwrap().id;
        assert!(game.native_item_entity_pickup(&dimension, item_id, OWNER).unwrap());
        assert!(!game.dropped.iter().any(|item| item.id == item_id));
        assert!(
            game.inventory
                .iter()
                .flatten()
                .all(|stack| stack.item.as_ref() != "minecraft:emerald")
        );
        assert_eq!(
            player_state(game, OWNER_CONN, |game| {
                game.inventory
                    .iter()
                    .flatten()
                    .filter(|stack| stack.item.as_ref() == "minecraft:emerald")
                    .map(|stack| stack.count)
                    .sum::<u32>()
            }),
            4
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn level_post_pickup_preserves_items_when_giving_fails() {
    with_server(|game, _, _| {
        let dimension = game.dim.id().to_owned();
        game.spawn_item(
            ItemStack::new("minecraft:emerald", 4),
            [5.0, 64.0, 5.0],
            [0.0; 3],
            0,
        );
        let item_id = game.dropped.last().unwrap().id;
        assert!(!game.native_item_entity_pickup(&dimension, item_id, 999).unwrap());
        player_state(game, OWNER_CONN, |game| {
            game.inventory = vec![Some(ItemStack::new("minecraft:stone", 64)); 36];
            game.offhand = Some(ItemStack::new("minecraft:stone", 64));
        });
        assert!(!game.native_item_entity_pickup(&dimension, item_id, OWNER).unwrap());
        assert_eq!(
            game.dropped
                .iter()
                .find(|item| item.id == item_id)
                .unwrap()
                .stack
                .count,
            4
        );
        player_state(game, OWNER_CONN, |game| {
            game.inventory[0] = Some(ItemStack::new("minecraft:emerald", 62));
        });
        assert!(game.native_item_entity_pickup(&dimension, item_id, OWNER).unwrap());
        assert_eq!(
            game.dropped
                .iter()
                .find(|item| item.id == item_id)
                .unwrap()
                .stack
                .count,
            2
        );
        player_state(game, OWNER_CONN, |game| game.inventory[1] = None);
        assert!(game.native_item_entity_pickup(&dimension, item_id, OWNER).unwrap());
        assert!(!game.dropped.iter().any(|item| item.id == item_id));
        assert_eq!(
            player_state(game, OWNER_CONN, |game| {
                game.inventory
                    .iter()
                    .flatten()
                    .filter(|stack| stack.item.as_ref() == "minecraft:emerald")
                    .map(|stack| stack.count)
                    .sum::<u32>()
            }),
            66
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn integrated_server_advances_rain_levels_before_sleep_resets_weather() {
    with_server(|game, _, _| {
        game.world.day_time = 14022;
        game.world.weather.raining = true;
        game.world.weather.rain_time = 12000;
        game.world.weather.thunder_time = 12000;
        game.world.weather.rain_level = (0.0, 0.0);
        for _ in 0..100 {
            game.tick_server(false).unwrap();
        }
        assert!(game.world.weather.levels(1.0).0 > 0.99);
        let host = game.lan.as_mut().unwrap();
        host.hub.guests.retain(|guest| guest.uuid == OWNER);
        let owner = &mut host.hub.guests[0];
        owner.bed = Some([5, 100, -30]);
        owner.sleep_timer = 100;
        game.tick_sleep();
        assert_eq!(game.world.day_time % 24000, 0);
        assert!(!game.world.weather.raining);
        assert_eq!(
            (game.world.weather.rain_time, game.world.weather.thunder_time),
            (0, 0)
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn owner_sneaking_uses_movement_state_without_a_guest_record() {
    with_server(|game, _, _| {
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .retain(|guest| guest.uuid != OWNER);
        game.lan_acting_uuid = Some(OWNER);
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(5.0, 64.0, 5.0)).unwrap());
        game.player.as_mut().unwrap().shift_key_down = true;
        assert!(game.native_player_sneaking(OWNER).unwrap());

        game.player.as_mut().unwrap().shift_key_down = false;
        game.keys.insert(winit::keyboard::KeyCode::ShiftLeft);
        assert!(!game.native_player_sneaking(OWNER).unwrap());
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn guest_entry_uses_latest_sneaking_and_sprinting_for_owner_and_guests() {
    with_server(|game, _, _| {
        for (conn, uuid) in [(OWNER_CONN, OWNER), (GUEST_CONN, GUEST)] {
            for pressed in [true, false] {
                let guest = game
                    .lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .iter_mut()
                    .find(|guest| guest.uuid == uuid)
                    .unwrap();
                guest.sneaking = pressed;
                guest.sprinting = pressed;
                let motion = guest.motion.as_mut().unwrap();
                motion.shift_key_down = !pressed;
                motion.sprinting = !pressed;

                let posture = player_state(game, conn, |game| {
                    let motion = game.player.as_ref().unwrap();
                    (
                        motion.shift_key_down,
                        motion.sprinting,
                        game.native_player_sneaking(uuid).unwrap(),
                    )
                });
                assert_eq!(posture, (pressed, pressed, pressed));
            }
        }
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn multiplayer_sneaking_updates_each_guests_pose_without_leaking() {
    with_server(|game, _, _| {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::new(12.0, 100.0, 12.0)).unwrap());
        for owner_sneaking in [true, false] {
            for guest in &mut game.lan.as_mut().unwrap().hub.guests {
                guest.loaded = true;
                guest.sneaking = if guest.uuid == OWNER {
                    owner_sneaking
                } else {
                    !owner_sneaking
                };
                guest.motion.as_mut().unwrap().shift_key_down = !guest.sneaking;
            }
            let host = game.lan.take().unwrap();
            game.lan = game.tick_guests(host);
            for guest in &game.lan.as_ref().unwrap().hub.guests {
                let motion = guest.motion.as_ref().unwrap();
                assert_eq!(motion.shift_key_down, guest.sneaking);
                assert_eq!(
                    motion.pose,
                    if guest.sneaking {
                        horizon_client::pose::Pose::Crouching
                    } else {
                        horizon_client::pose::Pose::Standing
                    }
                );
                assert_eq!(
                    guest.dimensions().height,
                    if guest.sneaking { 1.5 } else { 1.8 }
                );
            }
            let stand_in = game.player.as_ref().unwrap();
            assert!(!stand_in.shift_key_down);
            assert_eq!(stand_in.pose, horizon_client::pose::Pose::Standing);
        }
    });
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

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn player_appearance_is_available_for_placed_guest_and_owner_during_server_tick() {
    with_server(|game, _, _| {
        let owner = game.native_player_appearance(OWNER).unwrap();
        assert_eq!((owner.uuid, owner.profile_name.as_str()), (OWNER, "Owner"));
        let guest = game.native_player_appearance(GUEST).unwrap();
        assert_eq!((guest.uuid, guest.profile_name.as_str()), (GUEST, "Guest"));
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn player_appearance_is_unavailable_outside_the_attached_server_tick() {
    with_server(|game, _, _| {
        let host = game.lan.take().unwrap();
        assert!(game.native_player_appearance(OWNER).is_none());
        assert!(game.native_player_appearance(GUEST).is_none());
        game.lan = Some(host);
        game.native_lifecycle.server_tick_end(false);
        assert!(game.native_player_appearance(OWNER).is_none());
        assert!(game.native_player_appearance(GUEST).is_none());
    });
}

fn clientbound(receiver: &Receiver<Out>) -> Vec<play::Clientbound> {
    let mut packets = Vec::new();
    for outbound in receiver.try_iter() {
        let bodies = match outbound {
            Out::Packet(packet) => vec![packet],
            Out::Packets(packets) => packets,
            _ => continue,
        };
        for bytes in bodies {
            let mut reader = Reader::new(&bytes);
            let packet_id = reader.varint().unwrap();
            packets.push(play::Clientbound::decode(packet_id, reader.rest()).unwrap());
        }
    }
    packets
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_owner_has_a_server_motion_with_or_without_a_body_in_the_record() {
    with_server(|game, _, _| {
        let owner = game.native_server_player_motion_impl(OWNER).unwrap();
        assert_eq!(owner.position, [5.0, 64.0, 5.0]);
        // The record gives the body back only after a turn: the owner is still readable.
        let host = game.lan.as_mut().unwrap();
        let record = host
            .hub
            .guests
            .iter_mut()
            .find(|guest| guest.uuid == OWNER)
            .unwrap();
        record.motion = None;
        record.known_movement = [0.5, 0.0, -0.5];
        let owner = game.native_server_player_motion_impl(OWNER).unwrap();
        assert_eq!(owner.velocity, [0.5, 0.0, -0.5]);
        assert_eq!(
            game.native_set_server_player_velocity_impl(&game.dim.id().to_owned(), OWNER, [1.0; 3]),
            Ok(true)
        );
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn an_attack_hitbox_is_the_box_a_guest_is_attacked_in_and_reaches_horizon_clients() {
    with_server(|game, owner_packets, _| {
        let ordinary = {
            let host = game.lan.as_ref().unwrap();
            game.guest_target_box(host, 0, GUEST_ID as i32).unwrap()
        };
        // The owner's connection speaks for a Horizon client.
        game.lan.as_mut().unwrap().hub.guests[0].brand = "horizon".into();
        assert_eq!(
            game.native_set_player_attack_hitbox(GUEST, 6.0, 18.0),
            Ok(true)
        );
        let tall = {
            let host = game.lan.as_ref().unwrap();
            game.guest_target_box(host, 0, GUEST_ID as i32).unwrap()
        };
        assert_ne!(tall, ordinary);
        assert_eq!(tall.max_position().y - tall.min_position().y, 18.0);
        assert_eq!(tall.max_position().x - tall.min_position().x, 6.0);
        assert!(clientbound(&owner_packets).iter().any(|packet| matches!(
            packet,
            play::Clientbound::CustomPayload(p)
                if p.channel == horizon_client::lan::attack_hitbox::CHANNEL
        )));
        assert!(
            game.native_set_player_attack_hitbox(GUEST, 0.0, 18.0)
                .is_err()
        );
        assert_eq!(game.native_clear_player_attack_hitbox(GUEST), Ok(true));
        let host = game.lan.as_ref().unwrap();
        assert_eq!(game.guest_target_box(host, 0, GUEST_ID as i32).unwrap(), ordinary);
        assert_eq!(game.native_clear_player_attack_hitbox(404), Ok(false));
    });
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn the_world_border_is_read_and_set_with_vanillas_packets() {
    with_server(|game, _, guest_packets| {
        let dimension = game.dim.id().to_owned();
        // The first sync only records what the players were told.
        let mut host = game.lan.take().unwrap();
        game.lan_sync_border(&mut host);
        game.lan = Some(host);
        clientbound(&guest_packets);
        let before = game.native_world_border(&dimension).unwrap();
        assert_eq!(before.remaining_ticks, 0);
        assert!(game.native_world_border("minecraft:nowhere").is_err());
        assert_eq!(
            game.native_set_world_border(&dimension, [5.0, 6.0], 80.0, Some((20.0, 100))),
            Ok(())
        );
        let after = game.native_world_border(&dimension).unwrap();
        assert_eq!(
            (after.center, after.size, after.target_size, after.remaining_ticks),
            ([5.0, 6.0], 80.0, 20.0, 100)
        );
        let mut host = game.lan.take().unwrap();
        game.lan_sync_border(&mut host);
        game.lan = Some(host);
        let packets = clientbound(&guest_packets);
        assert!(packets.iter().any(|packet| matches!(
            packet,
            play::Clientbound::SetBorderCenter(p) if (p.x, p.z) == (5.0, 6.0)
        )));
        assert!(packets.iter().any(|packet| matches!(
            packet,
            play::Clientbound::SetBorderLerpSize(p)
                if (p.old_size, p.new_size, p.lerp_time) == (80.0, 20.0, 100)
        )));
        assert!(
            game.native_set_world_border(&dimension, [0.0, 0.0], 0.0, None)
                .is_err()
        );
        assert_eq!(game.native_world_border(&dimension).unwrap(), after);
    });
}
