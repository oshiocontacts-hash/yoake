//! FR-F4-MOB vanilla parity goldens (F4M-0, docs/yoake2/FR-F4-MOBS.md section 9).
//!
//! The goldens are SHA-256 hashes of what the engine produces for every vanilla mob type with no
//! mod loaded, kept in `tools/fixtures/mob_parity/`. They were captured before any F4M engine
//! change and must keep passing through every F4M item:
//!
//! * golden A (saves): the entity NBT of every mob type, and the same after a load and save.
//! * golden B (network): the configuration-phase packets (registry sync) and the `AddEntity` /
//!   `SetEntityData` / `UpdateAttributes` packets of a hosted world, to a stock-brand and to a
//!   Horizon-brand client, plus the custom-payload channels each of them is sent.
//! * golden C (render): the vertices of every mob type posed twice, the type facts, and the
//!   content identity and `registries.json` with zero mods.
//!
//! No game data is stored, only hashes. Needs the prepared resources, so the tests are ignored
//! by default:
//!
//! ```text
//! HORIZON_TEST_RESOURCES=<...>/.local/horizon-client/26.3 cargo test -p horizon-client \
//!     --test f4m_parity -- --ignored
//! ```
//!
//! `F4M_PARITY_WRITE=1` rewrites the golden files instead of comparing (only when the vanilla
//! behaviour is meant to change).
extern crate horizon_client_lib as horizon_client;
use horizon_client::{
    assets::Assets,
    blocks::BlockFacts,
    chunk::{Chunk, MIN_Y},
    entity_nbt,
    lan::{
        Lan, NetConfig,
        chunks::ChunkEncoder,
        data::SyncData,
        entity::EntityState,
        hub::{HostEvent, Hub, HubConfig, TickCtx},
    },
    mobs::{Mob, MobData, MobTypes},
    world::World,
};
use horizon_protocol::{
    PROTOCOL_VERSION,
    buf::Writer,
    connection::Connection,
    crypto::offline_uuid,
    packets::{
        Empty,
        configuration::{self as cfg, ClientInformation},
        handshake, login,
        play::{self, Clientbound as C, Serverbound as S},
    },
};
use horizon_world::registry::BlockStateId;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

type Golden = BTreeMap<String, String>;

fn resources() -> PathBuf {
    let root = std::env::var_os("HORIZON_TEST_RESOURCES")
        .map(PathBuf::from)
        .expect("set HORIZON_TEST_RESOURCES to the prepared resources (.../26.3)");
    assert!(root.join("entity-models.json").exists(), "{root:?}");
    root
}

fn golden_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tools/fixtures/mob_parity")
        .join(name)
}

fn hex(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    bytes.as_ref().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn sha(bytes: &[u8]) -> String {
    hex(Sha256::digest(bytes))
}

/// Compares `now` with the golden file (or rewrites it with `F4M_PARITY_WRITE=1`).
fn check(file: &str, now: &Golden) {
    let path = golden_path(file);
    if std::env::var_os("F4M_PARITY_WRITE").is_some() {
        let mut text = serde_json::to_string_pretty(now).unwrap();
        text.push('\n');
        std::fs::write(&path, text).unwrap();
        return;
    }
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
    let golden: Golden = serde_json::from_str(&text).unwrap();
    let mut differences = Vec::new();
    for key in golden
        .keys()
        .chain(now.keys())
        .collect::<std::collections::BTreeSet<_>>()
    {
        if golden.get(key) != now.get(key) {
            differences.push(format!(
                "{key}: golden {:?}, now {:?}",
                golden.get(key),
                now.get(key)
            ));
        }
    }
    assert!(
        differences.is_empty(),
        "{file}: {} differences from the golden\n{}",
        differences.len(),
        differences.join("\n")
    );
}

struct Loaded {
    types: MobTypes,
    data: MobData,
    registry: &'static horizon_world::registry::BlockRegistry,
}

fn load(root: &Path) -> Loaded {
    let assets = Assets::load(root).unwrap();
    let registry = assets.registry;
    let facts = BlockFacts::load(root, registry).unwrap();
    Loaded {
        types: MobTypes::load(root).unwrap(),
        data: MobData::load(root, &facts),
        registry,
    }
}

/// The scalar facts of a type and its table-driven behaviour, everything but the model trees.
fn type_facts(t: &horizon_client::mobs::MobType) -> String {
    let attributes: BTreeMap<_, _> = t.attributes.iter().collect();
    let layers: std::collections::BTreeSet<_> = t.layers.keys().collect();
    format!(
        "{} {:?} w{} h{} e{} {} hp{} sp{} fsp{} ad{} akb{} fr{} st{} ar{} at{} kr{} jump{} {attributes:?} g{} sfd{} fdm{} we{} ws{} as{} bs{} hrs{} mhx{} mhy{} ai{} {:?} layers{layers:?} tex{} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        t.id,
        t.species,
        t.width,
        t.height,
        t.eye_height,
        t.category,
        t.max_health,
        t.speed,
        t.flying_speed,
        t.attack_damage,
        t.attack_knockback,
        t.follow_range,
        t.step_height,
        t.armor,
        t.armor_toughness,
        t.knockback_resistance,
        t.jump_strength,
        t.gravity,
        t.safe_fall_distance,
        t.fall_damage_multiplier,
        t.water_efficiency,
        t.water_slowdown,
        t.air_speed,
        t.baby_scale,
        t.head_rot_speed,
        t.max_head_x_rot,
        t.max_head_y_rot,
        t.ambient_interval,
        t.flags,
        t.texture,
        t.brain,
        t.anim,
        t.sounds,
        t.step_sound,
        t.food,
        t.tempt,
        t.layers.len(),
    )
}

/// A fixed atlas: every texture name maps to its own small rectangle.
fn atlas(name: &str) -> Option<[f32; 4]> {
    let h = Sha256::digest(name.as_bytes());
    let u = f32::from(h[0]) / 512.0;
    let v = f32::from(h[1]) / 512.0;
    Some([u, v, u + 0.0625, v + 0.0625])
}

fn vertex_hash(out: &[horizon_client::mesher::Vertex]) -> String {
    let mut hash = Sha256::new();
    hash.update((out.len() as u64).to_le_bytes());
    hash.update(bytemuck::cast_slice::<_, u8>(out));
    hex(hash.finalize())
}

fn nbt_hash(tag: &horizon_world::nbt::Tag) -> String {
    sha(&horizon_world::nbt::write_network(tag, horizon_world::nbt::Limits::saved()).unwrap())
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn golden_c_render_types_and_identity() {
    let root = resources();
    let loaded = load(&root);
    let mut now = Golden::new();
    now.insert(
        "identity".into(),
        hex(horizon_client::content_identity::identity(&root).unwrap()),
    );
    now.insert(
        "registries.json".into(),
        sha(&std::fs::read(root.join("registries.json")).unwrap()),
    );
    let mut ids: Vec<_> = loaded.types.types.keys().cloned().collect();
    ids.sort();
    now.insert("mob_type_count".into(), ids.len().to_string());
    for id in &ids {
        let kind = &loaded.types.types[id];
        now.insert(format!("type {id}"), sha(type_facts(kind).as_bytes()));
        let kind = Arc::new(kind.clone());
        let mut mob = Mob::new(
            7,
            kind,
            horizon_core::geometry::Vec3::new(1.5, 64.0, -2.25),
            33.0,
        );
        let mut out = Vec::new();
        horizon_client::mobs::render::render(
            &mob,
            0.5,
            glam::DVec3::new(0.25, 63.0, 0.0),
            &atlas,
            0xF0F0,
            &loaded.data,
            &mut out,
        );
        now.insert(format!("render rest {id}"), vertex_hash(&out));
        // A second pose: moving limbs, an older mob, the head turned.
        mob.age = 41;
        mob.limb_position = 3.7;
        mob.limb_speed = 0.8;
        mob.head_yaw = 52.0;
        mob.previous_head_yaw = 48.0;
        mob.head_pitch = -14.0;
        mob.previous_head_pitch = -12.0;
        out.clear();
        horizon_client::mobs::render::render(
            &mob,
            0.25,
            glam::DVec3::new(0.25, 63.0, 0.0),
            &atlas,
            0xF0F0,
            &loaded.data,
            &mut out,
        );
        now.insert(format!("render walk {id}"), vertex_hash(&out));
    }
    check("golden_c_render.json", &now);
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn golden_a_saves() {
    let root = resources();
    let loaded = load(&root);
    let by_id: std::collections::HashMap<String, Arc<horizon_client::mobs::MobType>> = loaded
        .types
        .types
        .iter()
        .map(|(k, v)| (k.clone(), Arc::new(v.clone())))
        .collect();
    let mut ids: Vec<_> = by_id.keys().cloned().collect();
    ids.sort();
    let mut now = Golden::new();
    now.insert("mob_type_count".into(), ids.len().to_string());
    for id in &ids {
        let mob = Mob::new(
            9,
            by_id[id].clone(),
            horizon_core::geometry::Vec3::new(-3.5, 70.0, 11.25),
            121.0,
        );
        let uuid = [0x1111_i32, 0x2222, 0x3333, 0x4444];
        let tag = entity_nbt::mob_to_nbt(&mob, uuid, None, Some(loaded.registry));
        now.insert(format!("nbt {id}"), nbt_hash(&tag));
        // Saved, loaded, saved again: the second compound is the one a world keeps.
        match entity_nbt::mob_from_nbt(&tag, &by_id, 9, Some(loaded.registry)) {
            Some(back) => {
                let again = entity_nbt::mob_to_nbt(&back, uuid, Some(&tag), Some(loaded.registry));
                now.insert(format!("nbt reload {id}"), nbt_hash(&again));
            }
            None => {
                now.insert(format!("nbt reload {id}"), "none".into());
            }
        }
    }
    check("golden_a_saves.json", &now);
}

// ---- golden B: a hosted world, a bot of each brand ----

fn flat_world() -> World {
    let mut world = World::new(BlockStateId(0), 0);
    for cz in -4..=4 {
        for cx in -4..=4 {
            let mut chunk = Chunk::empty(cx, cz);
            for z in 0..16 {
                for x in 0..16 {
                    chunk.set(x, MIN_Y + 4, z, 1);
                }
            }
            chunk.lit = true;
            world.chunks.insert((cx, cz), Arc::new(chunk));
        }
    }
    world
}

struct Server {
    hub: Hub,
    world: World,
    next_entity: u32,
}
impl Server {
    fn tick(&mut self, entities: &[EntityState]) {
        let mut ctx = TickCtx {
            world: &mut self.world,
            entities,
            next_entity: &mut self.next_entity,
            spawn: [8.5, f64::from(MIN_Y + 5), 8.5],
            difficulty: 2,
            difficulty_locked: false,
            clock_rate: 1.0,
            tick_rate: 20.0,
            tick_frozen: false,
            tick_steps: Default::default(),
            collision: None,
            border: &horizon_client::worldborder::WorldBorder::default(),
            streaming: None,
        };
        let events = self.hub.tick(&mut ctx);
        for event in events {
            if let HostEvent::Joined(conn) = event {
                self.hub.place_new_player(conn, &mut ctx);
            }
        }
    }
}

fn server(root: &Path) -> Server {
    let sync = Arc::new(
        SyncData::load(root, &horizon_world::loot::PackOverlay::NONE).expect("sync data"),
    );
    let lan = Lan::start_with(sync.clone(), NetConfig::lan(0, "Parity - World".into(), 8)).unwrap();
    let biomes: Vec<String> = sync
        .dynamic
        .registries
        .get("minecraft:worldgen/biome")
        .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
        .unwrap_or_default();
    let states = vec![
        (Arc::<str>::from("minecraft:air"), false),
        (Arc::<str>::from("minecraft:stone"), false),
        (Arc::<str>::from("minecraft:chest"), false),
    ];
    let encoder = ChunkEncoder::new(states, &sync.builtin, &biomes, &biomes);
    let hub = Hub::new(
        lan,
        sync,
        encoder,
        HubConfig {
            allow_flight: true,
            default_game_mode: 1,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 3,
            simulation_distance: 3,
            integrated: true,
            dimension: "minecraft:overworld".into(),
            seed: 1234,
            max_players: 8,
            levels: Default::default(),
        },
    );
    Server {
        hub,
        world: flat_world(),
        next_entity: 100,
    }
}

fn entity(id: i32, kind: &str, x: f64, z: f64) -> EntityState {
    EntityState {
        id,
        uuid: 0x0F4D_0000_0000_0000_0000_0000_0000_0000 + id as u128,
        kind: kind.into(),
        position: [x, f64::from(MIN_Y + 5), z],
        yaw: 45.0,
        pitch: 0.0,
        head_yaw: 45.0,
        on_ground: true,
        velocity: [0.0; 3],
        minecart_steps: None,
        data: 0,
        meta: Vec::new(),
        defaults: Vec::new(),
        equipment: Vec::new(),
        attributes: Vec::new(),
        precise: false,
        vertical_collision: false,
        horizontal_collision: false,
        needs_sync: false,
        leash: None,
        spectator: false,
        watching: false,
        hidden: None,
    }
}

/// What a bot of one brand was sent.
struct Seen {
    config: String,
    config_channels: Vec<String>,
    play_channels: Vec<String>,
    entities: BTreeMap<i32, String>,
}

fn bot(port: u16, brand: &str, wanted: usize) -> Result<Seen, String> {
    let e = |e: horizon_protocol::Error| e.to_string();
    let mut c = Connection::connect("127.0.0.1", port, Duration::from_secs(5)).map_err(e)?;
    c.reader
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(e)?;
    let name = format!("Parity{brand}");
    c.send(
        &handshake::Serverbound::Intention(handshake::Intention {
            protocol: PROTOCOL_VERSION,
            host: "127.0.0.1".into(),
            port,
            intent: handshake::LOGIN,
        })
        .encode(),
    )
    .map_err(e)?;
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: name.clone(),
            uuid: offline_uuid(&name),
        })
        .encode(),
    )
    .map_err(e)?;
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        match login::Clientbound::decode(id, &payload).map_err(e)? {
            login::Clientbound::Compression(p) => c.set_compression(p.threshold),
            login::Clientbound::LoginFinished(_) => {
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())
                    .map_err(e)?;
                break;
            }
            other => return Err(format!("unexpected login packet {other:?}")),
        }
    }
    c.send(&cfg::Serverbound::ClientInformation(ClientInformation::new(4)).encode())
        .map_err(e)?;
    let mut brand_payload = Writer::new();
    brand_payload.string(brand);
    c.send(
        &cfg::Serverbound::CustomPayload(cfg::CustomPayload {
            channel: "minecraft:brand".into(),
            data: brand_payload.data,
        })
        .encode(),
    )
    .map_err(e)?;
    let mut config = Sha256::new();
    let mut config_channels = Vec::new();
    loop {
        let (id, payload) = c.recv().map_err(e)?;
        config.update(id.to_le_bytes());
        config.update((payload.len() as u64).to_le_bytes());
        config.update(&payload);
        match cfg::Clientbound::decode(id, &payload).map_err(e)? {
            cfg::Clientbound::SelectKnownPacks(_) => {
                c.send(
                    &cfg::Serverbound::SelectKnownPacks(cfg::SelectKnownPacks {
                        packs: Vec::new(),
                    })
                    .encode(),
                )
                .map_err(e)?;
            }
            cfg::Clientbound::CustomPayload(p) => config_channels.push(p.channel),
            cfg::Clientbound::FinishConfiguration(_) => {
                c.send(&cfg::Serverbound::FinishConfiguration(Empty).encode())
                    .map_err(e)?;
                break;
            }
            _ => {}
        }
    }
    c.reader
        .set_read_timeout(Some(Duration::from_millis(200)))
        .map_err(e)?;
    let mut seen: BTreeMap<(i32, u8), String> = BTreeMap::new();
    let mut add_entities = std::collections::BTreeSet::new();
    let mut play_channels = Vec::new();
    let mut at: Option<[f64; 3]> = None;
    let mut reminder = Instant::now();
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline && add_entities.len() < wanted {
        if let Some([x, y, z]) = at
            && reminder.elapsed() >= Duration::from_secs(1)
        {
            c.send(&S::MovePlayerPos(play::MovePlayerPos { x, y, z, flags: 1 }).encode())
                .map_err(e)?;
            c.send(&S::ClientTickEnd(Empty).encode()).map_err(e)?;
            reminder = Instant::now();
        }
        let Ok((id, payload)) = c.recv() else {
            continue;
        };
        let Ok(packet) = C::decode(id, &payload) else {
            continue;
        };
        let digest = || sha(&[&id.to_le_bytes()[..], &payload].concat());
        match packet {
            C::PlayerPosition(p) => {
                at = Some(p.position);
                c.send(
                    &S::AcceptTeleportation(play::AcceptTeleportation {
                        id: p.teleport,
                        x: p.position[0],
                        y: p.position[1],
                        z: p.position[2],
                        yaw: 0.0,
                        pitch: 0.0,
                    })
                    .encode(),
                )
                .map_err(e)?;
            }
            C::AddEntity(a) if a.id >= 1000 => {
                add_entities.insert(a.id);
                seen.entry((a.id, 0)).or_insert_with(digest);
            }
            C::SetEntityData(d) if d.id >= 1000 => {
                seen.entry((d.id, 1)).or_insert_with(digest);
            }
            C::UpdateAttributes(d) if d.id >= 1000 => {
                seen.entry((d.id, 2)).or_insert_with(digest);
            }
            C::CustomPayload(p) => play_channels.push(p.channel),
            _ => {}
        }
    }
    if add_entities.len() < wanted {
        return Err(format!(
            "only {} of {wanted} entities arrived",
            add_entities.len()
        ));
    }
    let mut entities: BTreeMap<i32, String> = BTreeMap::new();
    for ((id, kind), digest) in seen {
        let entry = entities.entry(id).or_default();
        entry.push_str(&format!("{kind}:{digest} "));
    }
    config_channels.sort();
    play_channels.sort();
    play_channels.dedup();
    Ok(Seen {
        config: hex(config.finalize()),
        config_channels,
        play_channels,
        entities,
    })
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
fn golden_b_network() {
    let root = resources();
    let loaded = load(&root);
    let mut ids: Vec<_> = loaded.types.types.keys().cloned().collect();
    ids.sort();
    // Entities in a ring around the spawn, all within the view of a player there.
    let states: Vec<EntityState> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| {
            let angle = i as f64 * 0.37;
            let ring = 3.0 + (i % 5) as f64 * 1.2;
            entity(
                1000 + i as i32,
                id,
                8.5 + angle.cos() * ring,
                8.5 + angle.sin() * ring,
            )
        })
        .collect();
    let mut now = Golden::new();
    now.insert("mob_type_count".into(), ids.len().to_string());
    for brand in ["vanilla", "horizon"] {
        let mut s = server(&root);
        let port = s.hub.lan.port;
        let wanted = states.len();
        let name = brand.to_owned();
        let handle = thread::spawn(move || bot(port, &name, wanted));
        let start = Instant::now();
        while !handle.is_finished() {
            s.tick(&states);
            thread::sleep(Duration::from_millis(10));
            assert!(start.elapsed() < Duration::from_secs(120), "timed out");
        }
        let seen = handle.join().unwrap().unwrap();
        now.insert(format!("{brand} config packets"), seen.config);
        now.insert(
            format!("{brand} config channels"),
            seen.config_channels.join(","),
        );
        now.insert(
            format!("{brand} play channels"),
            seen.play_channels.join(","),
        );
        for (i, id) in ids.iter().enumerate() {
            let entity = 1000 + i as i32;
            now.insert(
                format!("{brand} entity {id}"),
                seen.entities
                    .get(&entity)
                    .cloned()
                    .unwrap_or_else(|| "missing".into()),
            );
        }
    }
    check("golden_b_network.json", &now);
}
