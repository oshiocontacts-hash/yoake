//! F3-3: a world with a data pack level opens that level with its own save folder.
//! Needs the prepared game resources (`HORIZON_TEST_RESOURCES`), so it is `#[ignore]`d:
//! `cargo test -p horizon-client --lib level_registry_tests -- --ignored`.
use super::world_loading::WorldRequest;
use super::{Audio, Dim, Game, Options, Progress, Settings, load_resources};
use std::path::PathBuf;
use std::time::Instant;

fn write(root: &std::path::Path, path: &str, text: &str) {
    let p = root.join(path);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// The pack with the `mypack:moon` and `mypack:hell` levels.
fn write_moon_pack(world: &std::path::Path) {
    write(world, "datapacks/moon/pack.mcmeta", r#"{"pack":{"pack_format":1,"description":"x"}}"#);
    write(
        world,
        "datapacks/moon/data/mypack/dimension_type/moon_type.json",
        r##"{"min_y":0,"height":128,"logical_height":128,"has_skylight":true,"has_ceiling":false,
        "ambient_light":0.3,"coordinate_scale":1.0,"default_clock":"minecraft:overworld",
        "skybox":"overworld","cardinal_light":"default","timelines":"#minecraft:in_overworld",
        "attributes":{"minecraft:visual/sky_color":"#336699","minecraft:visual/fog_color":"#223344"}}"##,
    );
    // A Nether-like type of its own: no sky, a ceiling, fixed time, the Nether's rules.
    write(
        world,
        "datapacks/moon/data/mypack/dimension_type/hell_type.json",
        r##"{"min_y":0,"height":128,"logical_height":128,"has_skylight":false,"has_ceiling":true,
        "has_fixed_time":true,"ambient_light":0.1,"coordinate_scale":8.0,"skybox":"none",
        "cardinal_light":"nether","monster_spawn_block_light_limit":15,"monster_spawn_light_level":7,
        "infiniburn":"#minecraft:infiniburn_nether",
        "attributes":{"minecraft:gameplay/fast_lava":true,"minecraft:gameplay/water_evaporates":true,
        "minecraft:gameplay/piglins_zombify":false,"minecraft:gameplay/snow_golem_melts":true,
        "minecraft:gameplay/respawn_anchor_works":true,
        "minecraft:gameplay/bed_rule":{"can_set_spawn":"never","can_sleep":"never","destroy_on_use":true},
        "minecraft:gameplay/straw_bed_rule":{"can_set_spawn":"never","can_sleep":"never","destroy_on_use":true}}}"##,
    );
    write(
        world,
        "datapacks/moon/data/mypack/dimension/hell.json",
        r#"{"type":"mypack:hell_type","generator":{"type":"minecraft:noise",
        "settings":"minecraft:overworld","biome_source":{"type":"yoake:grid","cell_size":16,
        "width":1,"depth":1,"cells":["minecraft:plains"],"outside":"minecraft:plains"}}}"#,
    );
    write(
        world,
        "datapacks/moon/data/mypack/dimension/moon.json",
        r#"{"type":"mypack:moon_type","generator":{"type":"minecraft:noise",
        "settings":"minecraft:overworld","biome_source":{"type":"yoake:grid","cell_size":16,
        "width":2,"depth":1,"cells":["minecraft:plains","minecraft:desert"],
        "outside":"minecraft:plains"}}}"#,
    );
}

/// The `mypack:moon` and `mypack:hell` levels: data pack stems with a type of its own (128 high from y 0, a fixed
/// ambient light, its own sky and fog colours).
fn with_moon_game(test: impl FnOnce(&mut Game<'_>, &std::path::Path)) {
    with_world(write_moon_pack, test)
}

/// A world with the pack `write_pack` makes in it, opened as an integrated server.
fn with_world(
    write_pack: impl FnOnce(&std::path::Path),
    test: impl FnOnce(&mut Game<'_>, &std::path::Path),
) {
    let resources = PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"),
    );
    let world = std::env::temp_dir().join(format!("hz-f33-game-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    write_pack(&world);
    let settings = Settings::default();
    let loaded = load_resources(
        resources.clone(),
        None,
        &resources,
        &settings,
        Instant::now(),
        false,
        true,
        &Progress::new(),
        &mut |_, _| unreachable!("no menu handoff"),
    )
    .unwrap();
    let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
    options.resources = resources.clone();
    options.execution_role = super::execution_role::ExecutionRole::IntegratedServer;
    options.dedicated = Some(Vec::new());
    let mut game = Game::new(
        &loaded.resources,
        loaded.pack,
        loaded.gui.unwrap(),
        options,
        settings,
        Audio::silent(&resources),
        None,
    )
    .unwrap();
    game.load_world(WorldRequest {
        save_root: world.clone(),
        folder: "f33".into(),
        name: Some("f33".into()),
        seed: Some(5),
        creative: true,
        create: None,
    })
    .unwrap();
    test(&mut game, &world);
    let _ = std::fs::remove_dir_all(&world);
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES"]
fn a_pack_level_opens_with_its_own_save_folder_and_unknown_levels_do_not() {
    with_moon_game(|game, world| {
        let ids: Vec<_> = game.level_registry.ids().map(str::to_owned).collect();
        assert_eq!(ids[..3], ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end"]);
        assert_eq!(ids.len(), 5, "{ids:?}");
        assert!(ids.contains(&"mypack:moon".to_owned()) && ids.contains(&"mypack:hell".to_owned()));
        assert!(game.level_registry.problems().is_empty(), "{:?}", game.level_registry.problems());
        // The vanilla visual attributes came from the prepared bundle.
        assert!(
            game.level_registry.get("minecraft:overworld").unwrap().dimension_type.attributes.keys().any(|k| k.contains("visual")),
        );
        let moon = game.level_dim("mypack:moon").expect("registered");
        assert_ne!(moon, Dim::Overworld);
        game.enter_level(None, moon).unwrap();
        assert_eq!(game.dim, moon);
        assert_eq!(game.world.shape, game.level_registry.shape("mypack:moon").unwrap());
        game.save_level(true).unwrap();
        assert_eq!(game.level_registry.save_dir(world, "mypack:moon"), world.join("dimensions/mypack/moon"));
        game.enter_level(None, Dim::Overworld).unwrap();
        // A level the world does not have never opens, and is not another level's folder.
        let stray = Dim::intern("f33test:stray");
        assert_eq!(game.level_dim("f33test:stray"), None);
        assert!(game.enter_level(None, stray).is_err());
        assert_eq!(game.dim, Dim::Overworld);
    });
}

/// F3-4: the sky, fog, light and weather of a pack level follow its dimension type, and a client
/// told that type (the registry the hub sends) reads the same shape and the same facts.
#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES"]
fn a_pack_level_follows_its_type_and_a_client_reads_the_same_type() {
    use super::level_facts::{LevelFacts, SkyKind};
    use horizon_client::chunk::LevelShape;
    use horizon_client::lan::hub::HubConfig;
    use horizon_client::lan::{NetConfig, OnlineMode};
    use horizon_protocol::registry::Dynamic;
    with_moon_game(|game, _| {
        let shapes = game.level_shapes;
        game.lan_host(
            super::lan::LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
                loopback_only: false,
                announce: false,
                max_players: horizon_client::lan::MAX_PLAYERS,
                game_mode: None,
            },
            NetConfig {
                listen: false,
                online: OnlineMode::lan(),
                levels: shapes,
                owner_profile: None,
                announce: false,
                ..NetConfig::lan(0, "f34".into(), 4)
            },
            HubConfig {
                allow_flight: true,
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 4,
                simulation_distance: 4,
                dimension: game.dim.id().to_owned(),
                seed: 0,
                max_players: 4,
                integrated: true,
                levels: shapes,
            },
        )
        .unwrap();
        let moon = game.level_dim("mypack:moon").expect("registered");

        // The server side: the level in place is the moon, with its type's facts.
        game.enter_level(None, moon).unwrap();
        let facts = game.level_facts();
        assert_eq!(facts.sky, SkyKind::Overworld);
        assert!(facts.weather, "open sky under an overworld sky has weather");
        assert!((facts.ambient - 0.3).abs() < 1e-6);
        assert_eq!(facts.top, 127);
        assert_ne!(facts, LevelFacts::OVERWORLD);
        // Its sky colour is the pack's (#336699), not the Overworld's, and its own
        // timelines (`#minecraft:in_overworld`) darken it at night.
        let sky = game.dim_environment().sample(6000, None, None);
        for (got, want) in sky.sky_color.iter().zip([0.2, 0.4, 0.6]) {
            assert!((got - want).abs() < 0.01, "{:?}", sky.sky_color);
        }
        let night = game.dim_environment().sample(18000, None, None);
        assert!(night.sky_color.iter().sum::<f32>() < sky.sky_color.iter().sum::<f32>() * 0.5);
        assert!(facts.day_cycle && facts.rules.day_cycle);
        assert!(!facts.rules.fast_lava && !facts.fast_lava);
        assert!(facts.beds.sets_spawn == super::level_facts::SetSpawn::Always && !facts.beds.anchor_works);
        // The vanilla levels keep their fixed rules.
        game.enter_level(None, Dim::Nether).unwrap();
        assert_eq!(game.level_facts(), LevelFacts::NETHER);
        game.enter_level(None, Dim::Overworld).unwrap();
        assert_eq!(game.level_facts(), LevelFacts::OVERWORLD);

        // The wire: the registry the hub sends carries the pack's type after the vanilla ones.
        let sync = game.lan.as_ref().unwrap().hub.sync.clone();
        let mut client = Dynamic::default();
        for packet in sync.registry_packets(true, &shapes) {
            client.add(packet);
        }
        let type_id = sync.level_type_id("mypack:moon");
        let level = super::remote::remote_level_type(&client, type_id);
        assert_eq!(level.dimension.name, "mypack:moon_type");
        assert_eq!(level.shape, game.level_registry.shape("mypack:moon").unwrap());
        assert_eq!((level.shape.min_y, level.shape.height), (0, 128));
        let (name, json) = level.sent;
        assert_eq!(name, "mypack:moon_type");
        let sent = horizon_world::dimension::DimensionType::from_value(&json.expect("sent with its data"))
            .unwrap();
        assert_eq!(LevelFacts::of_type(&sent), facts, "the client reads the facts the server has");
        // The vanilla levels: known to every client, so no data, and the vanilla shapes.
        for (id, shape) in [
            ("minecraft:overworld", LevelShape::OVERWORLD),
            ("minecraft:the_nether", LevelShape::NETHER),
            ("minecraft:the_end", LevelShape::END),
        ] {
            let level = super::remote::remote_level_type(&client, sync.level_type_id(id));
            assert_eq!(level.shape, shape, "{id}");
            assert_eq!(level.sent.0, id);
            assert!(level.sent.1.is_none(), "{id}");
            assert_eq!(shapes.get(id), shape);
        }
    });
}

/// F3-4b: a Nether-like level of a pack behaves as the Nether does for lava, water, beds and
/// anchors, mobs and light, while the vanilla levels keep their rules.
#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES"]
fn a_nether_like_pack_level_follows_its_rules() {
    use super::level_facts::{LevelFacts, SkyKind, Sleep};
    with_moon_game(|game, _| {
        let hell = game.level_dim("mypack:hell").expect("registered");
        game.enter_level(None, hell).unwrap();
        let f = game.level_facts();
        assert_eq!(f.sky, SkyKind::None);
        assert!(f.fast_lava && f.rules.fast_lava && f.rules.water_evaporates && f.ultrawarm);
        assert!(f.nether_light && !f.weather && !f.day_cycle && !f.rules.has_skylight);
        assert_eq!(f.beds.sleep, Sleep::Never);
        // Beds explode, anchors work (the Nether's rules), as the game asks them.
        assert!(game.anchor_works());
        assert!(game.beds_destroy_on_use());
        let c = f.custom.unwrap();
        assert!(!c.piglins_zombify && c.snow_golem_melts && c.fast_lava && !c.has_skylight);
        assert_eq!(c.spawn_light.block_limit, 15);
        assert_eq!(game.level_registry.coordinate_scale("mypack:hell"), 8.0);
        // The world and the player's lava follow the type (the world's own flag is the registry's).
        assert!(game.world.fast_lava);
        // The vanilla three still answer with their constants.
        game.enter_level(None, Dim::Nether).unwrap();
        assert_eq!(game.level_facts(), LevelFacts::NETHER);
        assert!(game.anchor_works() && game.beds_destroy_on_use());
        game.enter_level(None, Dim::End).unwrap();
        assert_eq!(game.level_facts(), LevelFacts::END);
        assert!(!game.anchor_works() && game.beds_destroy_on_use());
        game.enter_level(None, Dim::Overworld).unwrap();
        assert_eq!(game.level_facts(), LevelFacts::OVERWORLD);
        assert!(!game.anchor_works() && !game.beds_destroy_on_use());
    });
}

/// Streams the level in place until the final chunk `(cx, cz)` is in, and hashes its blocks and
/// biomes (FNV-1a over every block id and a biome sample per 4x4x4 cell).
fn generated_chunk_hash(game: &mut Game<'_>, cx: i32, cz: i32) -> u64 {
    let chunk = generated_chunk(game, cx, cz);
    hash_chunk(game, &chunk)
}

/// Streams the level in place until the final chunk (cx, cz) is in.
fn generated_chunk(game: &mut Game<'_>, cx: i32, cz: i32) -> std::sync::Arc<horizon_client::chunk::Chunk> {
    game.streaming.forced.insert((cx, cz));
    game.streaming.center = (cx, cz);
    game.streaming.radius = 2;
    let started = Instant::now();
    while !game.world.chunks.contains_key(&(cx, cz)) {
        let _ = game.streaming.update(&mut game.world);
        assert!(started.elapsed().as_secs() < 240, "chunk {cx} {cz} never finished generating");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    game.world.chunks.get(&(cx, cz)).unwrap().clone()
}

fn hash_chunk(game: &Game<'_>, chunk: &horizon_client::chunk::Chunk) -> u64 {
    let shape = game.world.shape;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut put = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for y in shape.min_y..shape.min_y + shape.height {
        for z in 0..16 {
            for x in 0..16 {
                put(u64::from(chunk.get(x, y, z)));
                if x % 4 == 0 && y % 4 == 0 && z % 4 == 0 {
                    put(u64::from(chunk.biome(x, y, z)) + 1);
                }
            }
        }
    }
    h
}

/// Vanilla generation is unchanged by F3-5: the same fixed seed gives the same chunks in the three
/// vanilla levels (hashes recorded before the generator work).
#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES"]
fn vanilla_chunks_hash_as_they_did_before_the_generator_work() {
    with_moon_game(|game, _| {
        let mut got = Vec::new();
        for dim in [Dim::Overworld, Dim::Nether, Dim::End] {
            game.enter_level(None, dim).unwrap();
            got.push((dim.id(), generated_chunk_hash(game, 3, -2)));
            game.enter_level(None, Dim::Overworld).unwrap();
        }
        eprintln!("VANILLA_HASHES {got:?}");
        const EXPECTED: [(&str, u64); 3] = [
            ("minecraft:overworld", 10844162340323160155),
            ("minecraft:the_nether", 3948111228871227996),
            ("minecraft:the_end", 2685422101413284537),
        ];
        assert_eq!(got, EXPECTED);
    });
}

/// Superflat and checkerboard-biome pack levels, and the level of a mod's generator (F3-5).
fn write_generator_pack(world: &std::path::Path) {
    write(world, "datapacks/gen/pack.mcmeta", r#"{"pack":{"pack_format":1,"description":"x"}}"#);
    write(
        world,
        "datapacks/gen/data/mypack/dimension/plain.json",
        r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:flat","settings":{
        "biome":"minecraft:plains","features":false,"lakes":false,
        "layers":[{"block":"minecraft:bedrock","height":1},{"block":"minecraft:stone","height":2},
        {"block":"minecraft:dirt","height":1},{"block":"minecraft:grass_block","height":1}]}}}"#,
    );
    write(
        world,
        "datapacks/gen/data/mypack/dimension/checker.json",
        r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise","settings":"minecraft:overworld",
        "biome_source":{"type":"minecraft:checkerboard","biomes":["minecraft:plains","minecraft:desert"],"scale":0}}}"#,
    );
    write(
        world,
        "datapacks/gen/data/mypack/dimension/void.json",
        r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}}"#,
    );
}

/// A generator a test mod registers: `depth` layers of stone from the level's floor.
struct FloorFactory;
struct Floor {
    stone: u32,
    depth: i32,
}
impl horizon_client::mod_generator::ModGeneratorFactory for FloorFactory {
    fn create(
        &self,
        c: &horizon_client::mod_generator::ModGeneratorContext,
    ) -> Result<std::sync::Arc<dyn horizon_client::mod_generator::ModColumnSource>, String> {
        let depth = c.settings["depth"].as_i64().unwrap_or(1) as i32;
        let stone = c.block("minecraft:stone").ok_or("no stone")?;
        Ok(std::sync::Arc::new(Floor { stone, depth }))
    }
}
impl horizon_client::mod_generator::ModColumnSource for Floor {
    fn generate(&self, chunk: &mut horizon_client::mod_generator::ModChunk<'_>) {
        let floor = chunk.shape.min_y;
        for y in floor..floor + self.depth {
            chunk.fill_layer(y, self.stone);
        }
        chunk.set_biome("minecraft:desert");
    }
}

/// F3-5: a superflat level, a checkerboard-biome level and a debug level of a data pack open,
/// generate their own terrain and save under their own folders.
#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES"]
fn flat_checkerboard_and_debug_pack_levels_generate_and_save() {
    with_world(write_generator_pack, |game, world| {
        assert!(game.level_registry.problems().is_empty(), "{:?}", game.level_registry.problems());
        let id = |name: &str| game.pack.assets.registry.default_state(name).unwrap().0;
        let (bedrock, stone, dirt, grass) = (
            id("minecraft:bedrock"),
            id("minecraft:stone"),
            id("minecraft:dirt"),
            id("minecraft:grass_block"),
        );
        let barrier = id("minecraft:barrier");
        let plains = game.res.biomes().id("minecraft:plains");
        let desert = game.res.biomes().id("minecraft:desert");
        // Superflat: the layers from the level's floor, air above, the layer biome.
        let plain = game.level_dim("mypack:plain").expect("registered");
        game.enter_level(None, plain).unwrap();
        let chunk = generated_chunk(game, 0, 0);
        let floor = game.world.shape.min_y;
        for (i, want) in [bedrock, stone, stone, dirt, grass, 0].into_iter().enumerate() {
            assert_eq!(chunk.get(5, floor + i as i32, 9), want, "layer {i}");
        }
        assert_eq!(chunk.biome(3, floor, 3), plains);
        // An edit saves under the level's own folder.
        game.level_set([1, floor + 5, 1], horizon_world::registry::BlockStateId(stone)).unwrap();
        game.save_level(true).unwrap();
        let region = game.level_registry.save_dir(world, "mypack:plain").join("region");
        let saved = std::fs::read_dir(&region)
            .unwrap_or_else(|e| panic!("{}: {e}", region.display()))
            .count();
        assert!(saved > 0, "no region file in {}", region.display());
        game.enter_level(None, Dim::Overworld).unwrap();

        // Checkerboard: the noise terrain with the two biomes in alternating quarts.
        let checker = game.level_dim("mypack:checker").expect("registered");
        game.enter_level(None, checker).unwrap();
        let chunk = generated_chunk(game, 0, 0);
        let y = game.world.shape.min_y + 100;
        // `CheckerboardColumnBiomeSource`: a cell is `1 << (scale + 2)` quarts, so scale 0 is a
        // chunk; neighbouring chunks hold the two biomes in turn.
        let next = generated_chunk(game, 1, 0);
        let (a, b) = (chunk.biome(0, y, 0), next.biome(0, y, 0));
        assert_eq!([a, b].into_iter().collect::<std::collections::BTreeSet<_>>(), [plains, desert].into_iter().collect());
        for (x, z) in [(0, 0), (4, 12), (12, 4), (15, 15)] {
            assert_eq!(chunk.biome(x, y, z), a, "one biome in a chunk");
            assert_eq!(next.biome(x, y, z), b, "one biome in a chunk");
        }
        game.enter_level(None, Dim::Overworld).unwrap();

        // Debug: the barrier floor.
        let debug = game.level_dim("mypack:void").expect("registered");
        game.enter_level(None, debug).unwrap();
        let chunk = generated_chunk(game, 0, 0);
        assert_eq!(chunk.get(3, 60, 3), barrier);
        game.enter_level(None, Dim::Overworld).unwrap();
    });
}

/// F3-5: a level whose stem names a mod's generator opens through the registered factory, and
/// without the registration it does not open.
#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES"]
fn a_mod_generator_makes_the_chunks_of_its_level() {
    with_moon_game(|game, _| {
        let stem = serde_json::json!({
            "type": "minecraft:overworld",
            "generator": {"type": "yoake:mod", "id": "testmod:floor", "settings": {"depth": 3}}
        });
        let dim = game.register_mod_level("mypack:floor", stem).expect("the stem is valid");
        // No factory yet: the level is registered but cannot open, and says why.
        let error = game.enter_level(None, dim).unwrap_err().to_string();
        assert!(error.contains("testmod:floor") && error.contains("no mod registered"), "{error}");
        assert_eq!(game.dim, Dim::Overworld);
        assert!(game.register_mod_generator("nocolon", std::sync::Arc::new(FloorFactory)).is_err());
        game.register_mod_generator("testmod:floor", std::sync::Arc::new(FloorFactory)).unwrap();
        assert!(game.register_mod_generator("testmod:floor", std::sync::Arc::new(FloorFactory)).is_err());
        game.enter_level(None, dim).unwrap();
        let chunk = generated_chunk(game, 2, -1);
        let floor = game.world.shape.min_y;
        let stone = game.pack.assets.registry.default_state("minecraft:stone").unwrap().0;
        for y in floor..floor + 3 {
            assert_eq!(chunk.get(7, y, 7), stone, "y {y}");
        }
        assert_eq!(chunk.get(7, floor + 3, 7), 0);
        assert_eq!(chunk.biome(0, floor, 0), game.res.biomes().id("minecraft:desert"));
        // Vanilla levels are not touched by a mod's generator.
        game.enter_level(None, Dim::Overworld).unwrap();
        assert_eq!(game.level_facts(), super::level_facts::LevelFacts::OVERWORLD);
    });
}
