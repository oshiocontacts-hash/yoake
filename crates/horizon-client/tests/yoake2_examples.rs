//! MX3EX1: the guest SDK's example mods (crates/yoake-guest-sdk/examples), compiled to wasm32, packed as full-access
//! `.yak` files, installed and approved through a launcher `Control`, and loaded by the real loader
//! (`load_world_mods` with this launch's approved list), which installs the bridges. No game resources: the server
//! tick is driven the way `Game::yoake2_server_tick` drives it (`dispatch_root_id(HOOK_SERVER_TICK, ..)`) on a
//! stand-in root, and the observe-only hook on a standalone `Experience`. The same examples against a real `Game`
//! (item-use hooks, Game paths) are in `game/yoake2_examples_tests.rs` (ignored without HORIZON_TEST_RESOURCES).
//!
//!   cargo test --offline -j 4 -p horizon-client --features yoake2-slice --test yoake2_examples

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use horizon_client::entities::Experience;
use horizon_client::yoake2_mods::{ApprovedList, Control, LoadReport, Origin, install_package, load_world_mods};
use horizon_mods::yak::pack_directory;
use yoake_slice::activation::{Accessor, PathRoot, Status};
use yoake_slice::runtime;

#[path = "../../yoake-guest-sdk/tests/support/build.rs"]
mod build;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("yoake2_examples").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Builds `examples/<name>.rs` of the SDK to a wasm32 core module.
fn example(name: &str) -> Vec<u8> {
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("yoake2_examples").join(name);
    let rlib = build::build_sdk(&sdk, &tmp);
    build::build_guest(&rlib, &sdk.join(format!("examples/{name}.rs")), &tmp, name)
}

/// Packs `wasm` as a full-access mod `id`, installs it like the launcher does, approves it, and loads the game's mods
/// folder with this launch's approved list.
fn install_and_load(name: &str, id: &str, wasm: &[u8]) -> LoadReport {
    let base = scratch(name);
    let mods = base.join("game").join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let control = Control::create(&base.join("launcher").join("yoake-full-access"), &base.join("game")).unwrap();
    let src = base.join("src");
    std::fs::create_dir_all(src.join("bin/wasm")).unwrap();
    std::fs::write(
        src.join("yak.toml"),
        format!(
            "format = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = \"{id}\"\nauthors = [\"test\"]\ndescription = \"an SDK example\"\n\n[yoake]\nversion = \">=0.0.0\"\n\n[side]\nclient = false\nserver = true\nrequired_on_join = false\n\n[wasm]\ninterface = \"1.0\"\nserver = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"\n"
        ),
    )
    .unwrap();
    std::fs::write(src.join("bin/wasm/mod.wasm"), wasm).unwrap();
    let packed = base.join("packed.yak");
    pack_directory(&src, &packed, &BTreeMap::new()).unwrap();
    let bytes = std::fs::read(&packed).unwrap();
    install_package(&control, &mods, &format!("{id}.yak"), &bytes, Origin::Local, None).unwrap();
    control.approve(&bytes).unwrap();
    let file = control.approved_list(&mods).unwrap().write_for_launch(&base.join("run")).unwrap();
    load_world_mods(&mods, Some(&ApprovedList::read(&file).unwrap()))
}

/// A stand-in for the `Game` root (a real `Game` needs the game resources): no path resolves.
struct TickRoot;

impl PathRoot for TickRoot {
    fn resolve(&mut self, _: &[Accessor], _: &mut dyn FnMut(&mut dyn std::any::Any)) -> Result<(), Status> {
        Err(Status::Stale)
    }
}

fn server_tick() {
    runtime::dispatch_root_id(runtime::HOOK_SERVER_TICK, &mut TickRoot, &mut |_| 0);
}

fn slot(all: &[(i32, i64)], n: i32) -> Vec<i64> {
    all.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
}

/// What `engine_call` dispatches through in these tests: the entities glue (the `Experience` field getters).
fn entities_dispatch(handle: u64, id: u32, args: &[u8], out: &mut Vec<u8>) -> Result<(), yoake_slice::codec::CallError> {
    horizon_client::entities::yoake_glue::call(handle, id, args, out)
}

#[test]
fn tick_and_item_use_loads_reports_its_modes_counts_ticks_and_observes_experience() {
    runtime::uninstall().unwrap();
    // `Experience::give(30)` without any guest on this thread (the dispatcher of 43 may already be installed by
    // another test: with no guest it runs the original)
    let mut plain = Experience::default();
    let expected = plain.give(30);
    let expected_total = plain.total;

    runtime::set_engine_dispatch(Some(entities_dispatch));
    let report = install_and_load("tick_and_item_use", "example_tick", &example("tick_and_item_use"));
    assert_eq!(report.loaded.len(), 1, "{:?} {:?}", report.skipped, report.problems);
    assert!(report.unbridged.is_empty() && report.messages().is_empty(), "{:?}", report.messages());
    let init = runtime::take_reports().unwrap();
    // what hook_register said: server tick Full, start_using_item (103) Full, Experience::give (43) observe-only
    assert_eq!((slot(&init, 10), slot(&init, 11), slot(&init, 12)), (vec![0], vec![0], vec![1]));

    for _ in 0..3 {
        server_tick();
    }
    assert_eq!(slot(&runtime::take_reports().unwrap(), 20), [1, 2, 3], "it counted three server ticks");

    // hook 43 through the generated bridge: the guest reads `total` before and after `original`, cannot change the
    // result, and the real `give` runs exactly once
    let mut xp = Experience::default();
    assert_eq!(xp.give(30), expected, "the observe-only guest cannot change the result");
    assert_eq!(xp.total, expected_total, "the real give ran exactly once");
    let all = runtime::take_reports().unwrap();
    assert_eq!((slot(&all, 40), slot(&all, 41)), (vec![0], vec![i64::from(expected_total)]), "{all:?}");
    assert!(!runtime::stats().disabled, "{:?}", runtime::stats().trap);
    runtime::set_engine_dispatch(None);
    runtime::uninstall().unwrap();
}

#[test]
fn engine_call_game_path_loads_and_a_path_it_cannot_open_is_an_error_not_a_trap() {
    runtime::uninstall().unwrap();
    let report = install_and_load("engine_call_game_path", "example_path", &example("engine_call_game_path"));
    assert_eq!(report.loaded.len(), 1, "{:?} {:?}", report.skipped, report.problems);
    assert!(report.unbridged.is_empty() && report.messages().is_empty(), "{:?}", report.messages());
    server_tick();
    let all = runtime::take_reports().unwrap();
    // the stand-in root has no `dropped` item: the guest gets an error back from engine_call and handles it
    assert!(slot(&all, 1).is_empty() && slot(&all, 2).len() == 1, "{all:?}");
    let stats = runtime::stats();
    assert!(!stats.disabled, "{:?}", stats.trap);
    assert_eq!(stats.entries, 1);
    runtime::uninstall().unwrap();
}
