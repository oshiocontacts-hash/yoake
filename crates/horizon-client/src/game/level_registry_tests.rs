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

/// The `mypack:moon` level: a data pack stem with a type of its own (128 high from y 0, a fixed
/// ambient light, its own sky and fog colours).
fn with_moon_game(test: impl FnOnce(&mut Game<'_>, &std::path::Path)) {
    let resources = PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"),
    );
    let world = std::env::temp_dir().join(format!("hz-f33-game-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    write(&world, "datapacks/moon/pack.mcmeta", r#"{"pack":{"pack_format":1,"description":"x"}}"#);
    write(
        &world,
        "datapacks/moon/data/mypack/dimension_type/moon_type.json",
        r##"{"min_y":0,"height":128,"logical_height":128,"has_skylight":true,"has_ceiling":false,
        "ambient_light":0.3,"coordinate_scale":1.0,"default_clock":"minecraft:overworld",
        "skybox":"overworld","cardinal_light":"default","timelines":"#minecraft:in_overworld",
        "attributes":{"minecraft:visual/sky_color":"#336699","minecraft:visual/fog_color":"#223344"}}"##,
    );
    // A Nether-like type of its own: no sky, a ceiling, fixed time, the Nether's rules.
    write(
        &world,
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
        &world,
        "datapacks/moon/data/mypack/dimension/hell.json",
        r#"{"type":"mypack:hell_type","generator":{"type":"minecraft:noise",
        "settings":"minecraft:overworld","biome_source":{"type":"yoake:grid","cell_size":16,
        "width":1,"depth":1,"cells":["minecraft:plains"],"outside":"minecraft:plains"}}}"#,
    );
    write(
        &world,
        "datapacks/moon/data/mypack/dimension/moon.json",
        r#"{"type":"mypack:moon_type","generator":{"type":"minecraft:noise",
        "settings":"minecraft:overworld","biome_source":{"type":"yoake:grid","cell_size":16,
        "width":2,"depth":1,"cells":["minecraft:plains","minecraft:desert"],
        "outside":"minecraft:plains"}}}"#,
    );
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
        assert!(facts.beds.sets_spawn && !facts.beds.anchor_works);
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
