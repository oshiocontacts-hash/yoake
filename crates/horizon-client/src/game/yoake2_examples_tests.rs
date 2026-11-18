//! MX3EX1 end-to-end tests for the Rust guest SDK examples (`crates/yoake-guest-sdk/examples/`).
//!
//! Each example is compiled to a wasm32 core module with `rustc` (`tests/support/build.rs`), packed as a
//! full-access `.yak`, installed and approved via `Control`, and loaded into a real server `Game` via
//! `crate::yoake2_mods::load_world_mods` (which sets the bridge mode resolver and installs the generated
//! hook bridges).
//!
//! Needs the game resources (same variable as `yoake2_game_tests.rs`):
//!
//!   HORIZON_TEST_RESOURCES=C:\Users\USER\NestgateHorizon\horizon\.local\horizon-client\26.3 \
//!   cargo test --offline -j 4 -p horizon-client --lib --features yoake2-slice yoake2_examples -- --ignored

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use horizon_client::entities::ItemEntity;
use horizon_client::first_person::Hand;
use horizon_client::items::ItemStack;
use horizon_client::mod_api::NativeAuthority;
use horizon_core::geometry::Vec3;
use yoake_slice::runtime;

use crate::game::{Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources};
use crate::yoake2_mods::{ApprovedList, Control, LoadReport, Origin, install_package, load_world_mods};

#[path = "../../../yoake-guest-sdk/tests/support/build.rs"]
mod guest_build;

const TARGET_ITEM_ID: u32 = 4242;

fn server_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"));
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
    let mut game = Game::new(&loaded.resources, loaded.pack, loaded.gui.unwrap(), options, settings, Audio::silent(&root), None).unwrap();
    game.native_lifecycle.begin_open(NativeAuthority::IntegratedServer);
    game.native_lifecycle.world_open_ready();
    test(&mut game);
}

fn item(id: u32, x: f64) -> ItemEntity {
    let at = Vec3::new(x, 64.0, 0.5);
    ItemEntity {
        join: Default::default(),
        no_gravity: false,
        id,
        thrower: None,
        target: None,
        stack: ItemStack { item: Arc::from("minecraft:stone"), count: 3, damage: 0, extra: None },
        health: 5,
        fire_ticks: 0,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        on_ground: true,
        vertical_collision: false,
        horizontal_collision: false,
        age: 0,
        pickup_delay: 0,
        bob_offset: 0.0,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
    }
}

fn slot(reports: &[(i32, i64)], n: i32) -> Vec<i64> {
    reports.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
}

fn pack_mod(control: &Control, dir: &Path, id: &str, wasm: &[u8]) -> PathBuf {
    let src = dir.join(format!("src_{id}"));
    std::fs::create_dir_all(src.join("bin/wasm")).unwrap();
    std::fs::write(
        src.join("yak.toml"),
        format!(
            "format = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = \"{id}\"\nauthors = [\"test\"]\ndescription = \"an example mod\"\n\n[yoake]\nversion = \">=0.0.0\"\n\n[side]\nclient = false\nserver = true\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.0\"\nserver = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"\n"
        ),
    )
    .unwrap();
    std::fs::write(src.join("bin/wasm/mod.wasm"), wasm).unwrap();
    let built = dir.join(format!("{id}.built"));
    horizon_mods::yak::pack_directory(&src, &built, &std::collections::BTreeMap::new()).unwrap();
    let bytes = std::fs::read(&built).unwrap();
    std::fs::remove_file(&built).unwrap();
    install_package(control, dir, &format!("{id}.yak"), &bytes, Origin::Local, None).unwrap();
    dir.join(format!("{id}.yak"))
}

fn load_example(scratch: &Path, name: &str) -> LoadReport {
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let _ = std::fs::remove_dir_all(scratch);
    std::fs::create_dir_all(scratch).unwrap();
    let rlib = guest_build::build_sdk(&sdk, scratch);
    let wasm = guest_build::build_guest(&rlib, &sdk.join(format!("examples/{name}.rs")), scratch, name);
    let mods = scratch.join("game").join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let control = Control::create(&scratch.join("launcher").join("yoake-full-access"), &scratch.join("game")).unwrap();
    let yak = pack_mod(&control, &mods, name, &wasm);
    control.approve(&std::fs::read(&yak).unwrap()).unwrap();
    let file = control.approved_list(&mods).unwrap().write_for_launch(&scratch.join("run")).unwrap();
    let list = ApprovedList::read(&file).unwrap();
    load_world_mods(&mods, Some(&list))
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES (the game resources)"]
fn tick_and_item_use_example_loads_reports_modes_and_runs_through_bridges() {
    let _ = runtime::uninstall();
    let scratch = std::env::temp_dir().join("yoake2_examples_tick_and_item_use");

    server_game(|game| {
        super::native_client_items::yoake2_install_dispatch();
        let report = load_example(&scratch, "tick_and_item_use");
        assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["tick_and_item_use"]);
        assert!(report.skipped.is_empty(), "unexpected refusals: {:?}", report.skipped);
        assert!(report.problems.is_empty(), "unexpected problems: {:?}", report.problems);
        assert!(report.unbridged.is_empty(), "unbridged hooks: {:?}", report.unbridged);
        assert!(report.legacy_notes.is_empty(), "unexpected legacy notes: {:?}", report.legacy_notes);
        assert!(yoake_slice::bridge::is_installed(103), "start_using_item (103) bridge should be installed");
        assert!(yoake_slice::bridge::is_installed(43), "Experience::give (43) bridge should be installed");

        let mod_id = report.loaded[0].module;
        let init_reports = runtime::take_reports_of(mod_id).unwrap();
        assert_eq!(slot(&init_reports, 10), [0], "SERVER_TICK reports Full (0)");
        assert_eq!(slot(&init_reports, 11), [0], "start_using_item reports Full (0)");
        assert_eq!(slot(&init_reports, 12), [1], "Experience::give reports ObserveOnly (1)");

        // Run three real server ticks.
        for _ in 0..3 {
            game.native_lifecycle_server_tick_start(true);
        }
        let tick_reports = runtime::take_reports_of(mod_id).unwrap();
        assert_eq!(slot(&tick_reports, 20), [1, 2, 3], "server ticks counted in guest");

        // Drive the Full `game::item_use` hook (`Game::start_using_item`, id 103).
        game.selected = 0;
        game.hand = Hand::Main;
        game.using = None;
        game.inventory[0] = Some(ItemStack::new("minecraft:bow", 1));
        assert!(game.start_using_item(), "start_using_item succeeds when guest passes original through");
        assert_eq!(game.using, Some((0, 0)));
        let item_use_reports = runtime::take_reports_of(mod_id).unwrap();
        assert_eq!(slot(&item_use_reports, 30), [1], "Full hook received original bool true (1)");

        // Drive the ObserveOnly hook (`Experience::give`, id 43): guest reads before/after `original`,
        // and its `0` return value is ignored while the caller still receives `Some(5)`.
        assert_eq!(game.experience.give(60), Some(5));
        assert_eq!(game.experience.total, 60);
        assert_eq!(game.experience.level, 5);
        let observe_reports = runtime::take_reports_of(mod_id).unwrap();
        assert_eq!(slot(&observe_reports, 40), [0], "ObserveOnly hook read total before original");
        assert_eq!(slot(&observe_reports, 41), [60], "ObserveOnly hook read total after original");

        let stats = runtime::stats_of(mod_id).unwrap();
        assert!(!stats.disabled, "{:?}", stats.trap);
        assert_eq!(stats.entries, 5, "3 server ticks + 1 start_using_item + 1 Experience::give");
        assert_eq!(yoake_slice::activation::live_activations(), 0);
        assert_eq!(yoake_slice::activation::live_path_handles(), 0);
        let _ = runtime::uninstall();
    });
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES (the game resources)"]
fn engine_call_game_path_example_reads_writes_and_handles_stale() {
    let _ = runtime::uninstall();
    let scratch = std::env::temp_dir().join("yoake2_examples_engine_call_game_path");

    server_game(|game| {
        super::native_client_items::yoake2_install_dispatch();
        game.dropped.push(item(TARGET_ITEM_ID, 12.5));

        let report = load_example(&scratch, "engine_call_game_path");
        assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["engine_call_game_path"]);
        assert!(report.skipped.is_empty(), "unexpected refusals: {:?}", report.skipped);
        assert!(report.problems.is_empty(), "unexpected problems: {:?}", report.problems);
        assert!(report.unbridged.is_empty(), "unbridged hooks: {:?}", report.unbridged);
        assert!(report.legacy_notes.is_empty(), "unexpected legacy notes: {:?}", report.legacy_notes);

        let mod_id = report.loaded[0].module;

        // Tick 1: item 4242 is present; guest reads position and fire_ticks, writes 55, and probes a missing id.
        game.native_lifecycle_server_tick_start(true);
        let r = runtime::take_reports_of(mod_id).unwrap();
        assert_eq!(slot(&r, 1), [1], "path handle opened for item 4242");
        assert_eq!(slot(&r, 2), Vec::<i64>::new(), "no error while item 4242 exists");
        assert_eq!(f64::from_bits(slot(&r, 3)[0] as u64), 12.5);
        assert_eq!(f64::from_bits(slot(&r, 4)[0] as u64), 64.0);
        assert_eq!(slot(&r, 5), [0], "fire_ticks before write");
        assert_eq!(slot(&r, 6), [1], "fire_ticks write succeeded");
        assert_eq!(slot(&r, 7), [55], "fire_ticks read back after write");
        assert_eq!(slot(&r, 8), [1], "missing entity id returned Stale, not a trap");
        assert_eq!(game.dropped.iter().find(|d| d.id == TARGET_ITEM_ID).unwrap().fire_ticks, 55);

        let stats = runtime::stats_of(mod_id).unwrap();
        assert!(!stats.disabled, "{:?}", stats.trap);
        assert_eq!(stats.entries, 1);

        // Tick 2: item 4242 is removed; the same guest handles `Stale` cleanly without trapping.
        game.dropped.retain(|d| d.id != TARGET_ITEM_ID);
        game.native_lifecycle_server_tick_start(true);
        let r = runtime::take_reports_of(mod_id).unwrap();
        assert_eq!(slot(&r, 2), [-1], "removed entity returns Error::Handle(1) = Stale");
        assert!(!runtime::stats_of(mod_id).unwrap().disabled, "stale handle must not disable mod");
        assert_eq!(yoake_slice::activation::live_activations(), 0);
        assert_eq!(yoake_slice::activation::live_path_handles(), 0);
        let _ = runtime::uninstall();
    });
}
