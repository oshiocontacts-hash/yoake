//! MX3K1 end to end with a real `Game`: a Rust guest, compiled to a wasm32 core module with the SDK, registers
//! the server-tick hook, and in `Game::yoake2_server_tick` (called from the integrated-server tick start) finds
//! a dropped item by id through a Game path, reads its position and writes a writable field; the change is
//! visible in the `Game`. A guest that did not register the hook is not entered.
//!
//! Needs the game resources (the other ignored tests in this crate use the same variable):
//!
//!   HORIZON_TEST_RESOURCES=C:\Users\USER\NestgateHorizon\horizon\.local\horizon-client\26.3 \
//!   cargo test --offline -j 4 -p horizon-client --lib --features yoake2-slice yoake2_game -- --ignored

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use horizon_client::entities::ItemEntity;
use horizon_client::items::ItemStack;
use horizon_client::mod_api::NativeAuthority;
use horizon_core::geometry::Vec3;
use yoake_slice::runtime::{self, ModConfig};

use crate::game::{Audio, Game, Options, Progress, Settings, execution_role::ExecutionRole, load_resources};

#[path = "../../../yoake-guest-sdk/tests/support/build.rs"]
mod guest_build;

const ITEM: u32 = 4242;

fn server_game(test: impl FnOnce(&mut Game<'_>)) {
    let root = PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").expect("set HORIZON_TEST_RESOURCES"));
    let settings = Settings::default();
    let loaded = load_resources(root.clone(), None, &root, &settings, Instant::now(), false, true, &Progress::new(), &mut |_, _| unreachable!("no menu handoff")).unwrap();
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

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES (the game resources)"]
fn a_rust_guest_in_the_server_tick_hook_edits_a_dropped_item_of_a_real_game() {
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let scratch = std::env::temp_dir().join("yoake2_guest_e2e");
    let rlib = guest_build::build_sdk(&sdk, &scratch);
    let guest = guest_build::build_guest(&rlib, &sdk.join("tests/guests/game_tick.rs"), &scratch, "game_tick");
    let other = guest_build::build_guest(&rlib, &sdk.join("tests/guests/only_outer.rs"), &scratch, "only_outer");

    server_game(|game| {
        super::native_client_items::yoake2_install_dispatch();
        game.dropped.push(item(ITEM, 12.5));
        game.dropped.push(item(7, 99.0));

        // 1. no mod installed: the tick works and costs nothing
        let _ = runtime::uninstall();
        game.native_lifecycle_server_tick_start(true);
        assert_eq!(game.dropped.iter().find(|d| d.id == ITEM).unwrap().fire_ticks, 0);

        // 2. a guest that registered a different hook is never entered
        runtime::install(&other, ModConfig::new("")).unwrap();
        game.native_lifecycle_server_tick_start(true);
        assert_eq!(runtime::stats().entries, 0, "the server tick is not hook 1");
        assert!(runtime::take_reports().unwrap().is_empty());
        assert_eq!(game.dropped.iter().find(|d| d.id == ITEM).unwrap().fire_ticks, 0);

        // 3. the server-tick guest: finds item 4242 by id, reads its position, writes fire_ticks
        runtime::install(&guest, ModConfig::new("")).unwrap();
        assert_eq!(runtime::stats().registered, vec![(runtime::HOOK_SERVER_TICK, 0)]);
        game.native_lifecycle_server_tick_start(true);
        let r = runtime::take_reports().unwrap();
        assert_eq!(slot(&r, 1), [i64::from(runtime::HOOK_SERVER_TICK)], "the guest ran in the server-tick hook");
        assert_eq!(slot(&r, 2), Vec::<i64>::new(), "the handle was made");
        assert_eq!(f64::from_bits(slot(&r, 3)[0] as u64), 12.5, "the position of item 4242");
        assert_eq!(f64::from_bits(slot(&r, 4)[0] as u64), 64.0);
        assert_eq!(slot(&r, 5), [1], "fire_ticks is writable");
        assert_eq!(slot(&r, 6), [1], "an item that does not exist is Stale, not a trap");
        let edited = game.dropped.iter().find(|d| d.id == ITEM).unwrap();
        assert_eq!(edited.fire_ticks, 55, "the write is visible in the Game");
        assert_eq!(game.dropped.iter().find(|d| d.id == 7).unwrap().fire_ticks, 0, "the other item is untouched");
        let stats = runtime::stats();
        assert!(!stats.disabled, "{:?}", stats.trap);
        assert_eq!(stats.entries, 1);
        assert_eq!(yoake_slice::activation::live_activations(), 0);
        assert_eq!(yoake_slice::activation::live_path_handles(), 0);

        // 4. the item is gone: the same guest gets Stale for it and the game goes on
        game.dropped.retain(|d| d.id != ITEM);
        game.native_lifecycle_server_tick_start(true);
        let r = runtime::take_reports().unwrap();
        assert_eq!(slot(&r, 2).len(), 1, "no handle for a removed item");
        assert!(!runtime::stats().disabled);
        let _ = runtime::uninstall();
    });
}

/// Packs a full-access package around a compiled guest.
fn pack_mod(control: &crate::yoake2_mods::Control, dir: &Path, id: &str, wasm: &[u8]) -> PathBuf {
    let src = dir.join(format!("src_{id}"));
    std::fs::create_dir_all(src.join("bin/wasm")).unwrap();
    std::fs::write(
        src.join("yak.toml"),
        format!(
            "format = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = \"{id}\"\nauthors = [\"test\"]\ndescription = \"a test mod\"\n\n[yoake]\nversion = \">=0.0.0\"\n\n[side]\nclient = false\nserver = true\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.0\"\nserver = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"\n"
        ),
    )
    .unwrap();
    std::fs::write(src.join("bin/wasm/mod.wasm"), wasm).unwrap();
    let built = dir.join(format!("{id}.built"));
    horizon_mods::yak::pack_directory(&src, &built, &std::collections::BTreeMap::new()).unwrap();
    // arrives like a player's own install: the file lands in the folder and its origin is recorded
    let bytes = std::fs::read(&built).unwrap();
    std::fs::remove_file(&built).unwrap();
    crate::yoake2_mods::install_package(control, dir, &format!("{id}.yak"), &bytes, crate::yoake2_mods::Origin::Local, None).unwrap();
    dir.join(format!("{id}.yak"))
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES (the game resources)"]
fn two_rust_mods_compose_in_the_server_tick_of_a_real_game_and_a_trap_leaves_the_others_working() {
    use crate::yoake2_mods::{Control, Refusal, load_dir};

    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let scratch = std::env::temp_dir().join("yoake2_two_mods");
    let _ = std::fs::remove_dir_all(&scratch);
    let rlib = guest_build::build_sdk(&sdk, &scratch);
    let build = |name: &str| guest_build::build_guest(&rlib, &sdk.join(format!("tests/guests/{name}.rs")), &scratch, name);
    let (a, b, trap) = (build("tick_a"), build("tick_b"), build("tick_trap"));
    let mods = scratch.join("game").join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    // the control files live in a launcher-owned folder outside the game root
    let control = Control::create(&scratch.join("launcher").join("yoake-full-access"), &scratch.join("game")).unwrap();
    // load order is file order: a_outer, b_inner, c_trap; d_denied is a full-access package nobody switched on
    pack_mod(&control, &mods, "a_outer", &a);
    pack_mod(&control, &mods, "b_inner", &b);
    pack_mod(&control, &mods, "c_trap", &trap);
    pack_mod(&control, &mods, "d_denied", &a);
    for id in ["a_outer", "b_inner", "c_trap"] {
        control.approve(&std::fs::read(mods.join(format!("{id}.yak"))).unwrap()).unwrap();
    }

    server_game(|game| {
        super::native_client_items::yoake2_install_dispatch();
        game.dropped.push(item(ITEM, 12.5));
        let _ = runtime::uninstall();
        let report = {
            // the launcher verifies the control files and hands the game this launch's approved list (as a file)
            let run = scratch.join("run");
            let file = control.approved_list(&mods).unwrap().write_for_launch(&run).unwrap();
            let list = crate::yoake2_mods::ApprovedList::read(&file).unwrap();
            let (allow, origins) = list.to_files();
            load_dir(&mods, &origins, &allow, &[], &ModConfig::new(""))
        };
        assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["a_outer", "b_inner", "c_trap"]);
        assert_eq!(report.skipped.len(), 1);
        assert!(matches!(report.skipped[0].1, Refusal::UnknownOrigin { .. }), "d_denied is not on the approved list, so it never loaded: {:?}", report.skipped[0].1);
        let (ma, mb, mc) = (report.loaded[0].module, report.loaded[1].module, report.loaded[2].module);
        assert_eq!(runtime::all_stats().len(), 3);

        // tick 1: a (priority 10) wraps the trapping mod (15), which wraps b (20), which wraps the engine
        game.native_lifecycle_server_tick_start(true);
        let (ra, rb, rc) = (runtime::take_reports_of(ma).unwrap(), runtime::take_reports_of(mb).unwrap(), runtime::take_reports_of(mc).unwrap());
        assert_eq!(slot(&ra, 1), [0], "the outermost mod sees the item before anyone");
        assert_eq!(slot(&rb, 11), [1], "b sees a's change although the mod between them trapped");
        assert_eq!(slot(&ra, 2), [21], "a sees b's effect after `original`");
        assert_eq!(slot(&rb, 12), [21]);
        assert!(rc.is_empty());
        assert_eq!(game.dropped.iter().find(|d| d.id == ITEM).unwrap().fire_ticks, 21, "both mods' writes are in the Game: 0 + 1 + 20");
        assert!(runtime::stats_of(mc).unwrap().disabled, "the trapping mod is out");
        assert!(!runtime::stats_of(ma).unwrap().disabled);
        assert!(!runtime::stats_of(mb).unwrap().disabled);

        // tick 2: the trapped mod is not entered, the other two go on from what the first tick left
        game.native_lifecycle_server_tick_start(true);
        let (ra, rb) = (runtime::take_reports_of(ma).unwrap(), runtime::take_reports_of(mb).unwrap());
        assert_eq!(slot(&ra, 1), [21]);
        assert_eq!(slot(&rb, 11), [22]);
        assert_eq!(game.dropped.iter().find(|d| d.id == ITEM).unwrap().fire_ticks, 42);
        assert_eq!(runtime::stats_of(mc).unwrap().entries, 1, "never entered again");
        assert_eq!(runtime::stats_of(ma).unwrap().entries, 2);
        assert_eq!(yoake_slice::activation::live_activations(), 0);
        assert_eq!(yoake_slice::activation::live_path_handles(), 0);
        let _ = runtime::uninstall();
    });
}
