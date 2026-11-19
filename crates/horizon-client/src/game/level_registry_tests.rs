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

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES"]
fn a_pack_level_opens_with_its_own_save_folder_and_unknown_levels_do_not() {
    let resources = PathBuf::from(
        std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"),
    );
    let world = std::env::temp_dir().join(format!("hz-f33-game-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&world);
    write(&world, "datapacks/moon/pack.mcmeta", r#"{"pack":{"pack_format":1,"description":"x"}}"#);
    write(
        &world,
        "datapacks/moon/data/mypack/dimension/moon.json",
        r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise",
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
    let ids: Vec<_> = game.level_registry.ids().map(str::to_owned).collect();
    assert_eq!(
        ids,
        ["minecraft:overworld", "minecraft:the_nether", "minecraft:the_end", "mypack:moon"]
    );
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
    assert_eq!(game.level_registry.save_dir(&world, "mypack:moon"), world.join("dimensions/mypack/moon"));
    game.enter_level(None, Dim::Overworld).unwrap();
    // A level the world does not have never opens, and is not another level's folder.
    let stray = Dim::intern("f33test:stray");
    assert_eq!(game.level_dim("f33test:stray"), None);
    assert!(game.enter_level(None, stray).is_err());
    assert_eq!(game.dim, Dim::Overworld);
    let _ = std::fs::remove_dir_all(&world);
}
