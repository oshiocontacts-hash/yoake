//! The mod-author starter template (templates/full-access-mod): its `src/lib.rs` is compiled to a wasm32 core
//! module with the same rustc flags as the template's build scripts, packed with its own `yak.toml` as a
//! full-access `.yak`, installed and approved through a launcher `Control`, and loaded by the real loader.
//! No game resources: the server tick is driven the way `Game::yoake2_server_tick` drives it
//! (`dispatch_root_id(HOOK_SERVER_TICK, ..)`) on a stand-in root that holds a real `Experience` at the path
//! `Game.experience`, which is what `Game.experience#handle` opens.
//!
//!   cargo test --offline -j 4 -p horizon-client --features yoake2-slice,yoake-hook-bench --test yoake2_template

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use horizon_client::entities::Experience;
use horizon_client::yoake2_mods::{ApprovedList, Control, Origin, install_package, load_world_mods};
use horizon_mods::yak::{Manifest, pack_directory};
use yoake_slice::activation::{self, Accessor, PathRoot, Status};
use yoake_slice::codec::{CallError, Encode};
use yoake_slice::runtime;

#[path = "../../yoake-guest-sdk/tests/support/build.rs"]
mod build;

fn template_dir() -> PathBuf {
    // no `..` component: the manifest reader refuses paths that have one
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("the workspace root").join("templates/full-access-mod")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("yoake2_template").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `Game.experience#handle` (binding 455) and the `Game` field it opens; ids are append-only, and the test checks
/// the lock file so a change cannot go unnoticed.
const GAME_EXPERIENCE_HANDLE: u32 = 455;
const GAME_EXPERIENCE_FIELD: u32 = 6;

fn check_binding_id_against_the_lock_file() {
    let lock = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-bindgen/ids.lock.json")).unwrap();
    let at = lock.find("\"game_items::Game.experience#handle\"").expect("binding in ids.lock.json");
    let before = &lock[..at];
    let id_line = before.rsplit("\"id\":").next().unwrap();
    assert_eq!(id_line.trim().trim_end_matches(',').trim().split(',').next().unwrap().trim(), GAME_EXPERIENCE_HANDLE.to_string());
}

/// A stand-in for the `Game` root with the one path the template uses.
struct World {
    experience: Experience,
}

impl PathRoot for World {
    fn resolve(&mut self, path: &[Accessor], f: &mut dyn FnMut(&mut dyn std::any::Any)) -> Result<(), Status> {
        match path {
            [Accessor::Field(GAME_EXPERIENCE_FIELD)] => {
                f(&mut self.experience);
                Ok(())
            }
            _ => Err(Status::BadField),
        }
    }
}

/// What `engine_call` dispatches through: the hand-written twin of the generated `call_455` (that glue lives in
/// `Game`'s module and is private), then the entities glue (`Experience.total#get`).
fn dispatch(handle: u64, id: u32, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {
    if id == GAME_EXPERIENCE_HANDLE {
        assert!(args.is_empty());
        let path = activation::open_path(handle, vec![Accessor::Field(GAME_EXPERIENCE_FIELD)])?;
        path.encode(out);
        return Ok(());
    }
    horizon_client::entities::yoake_glue::call(handle, id, args, out)
}

/// The engine dispatcher is process-wide: tests that install it run one at a time.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn slot(all: &[(i32, i64)], n: i32) -> Vec<i64> {
    all.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
}

/// Builds the template's guest, packs it, installs it as the launcher does (origin `local`), approves it and loads
/// the mods folder with this launch's approved list.
fn install_and_load(name: &str) -> horizon_client::yoake2_mods::LoadReport {
    let template = template_dir();
    let manifest = Manifest::read(&template.join("yak.toml")).expect("the template's yak.toml is a valid manifest");
    let wasm_path = manifest.wasm.as_ref().and_then(|w| w.server.clone()).expect("the template declares wasm.server");
    assert!(manifest.wasm.as_ref().unwrap().full_access());

    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("yoake2_template").join(name);
    let rlib = build::build_sdk(&sdk, &tmp);
    let module_name = Path::new(&wasm_path).file_stem().unwrap().to_str().unwrap().to_owned();
    let wasm = build::build_guest(&rlib, &template.join("src/lib.rs"), &tmp, &module_name);

    let base = scratch(name);
    let mods = base.join("game").join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let control = Control::create(&base.join("launcher").join("yoake-full-access"), &base.join("game")).unwrap();
    let src = base.join("src");
    std::fs::create_dir_all(src.join("bin/wasm")).unwrap();
    std::fs::copy(template.join("yak.toml"), src.join("yak.toml")).unwrap();
    std::fs::write(src.join(&wasm_path), &wasm).unwrap();
    let packed = base.join("packed.yak");
    pack_directory(&src, &packed, &BTreeMap::new()).unwrap();
    let bytes = std::fs::read(&packed).unwrap();
    install_package(&control, &mods, &format!("{}.yak", manifest.id), &bytes, Origin::Local, None).unwrap();
    control.approve(&bytes).unwrap();
    let file = control.approved_list(&mods).unwrap().write_for_launch(&base.join("run")).unwrap();
    load_world_mods(&mods, Some(&ApprovedList::read(&file).unwrap()))
}

#[test]
fn the_template_builds_loads_counts_ticks_and_reads_the_experience_total() {
    let _serial = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    check_binding_id_against_the_lock_file();
    runtime::uninstall().unwrap();
    runtime::set_engine_dispatch(Some(dispatch));

    let report = install_and_load("main");
    assert_eq!(report.loaded.len(), 1, "{:?} {:?}", report.skipped, report.problems);
    assert!(report.unbridged.is_empty() && report.messages().is_empty(), "{:?}", report.messages());

    let mut world = World { experience: Experience::default() };
    world.experience.give(30);
    let total = world.experience.total;
    assert!(total > 0, "the stand-in Experience holds something to read");

    let mut original_runs = 0;
    for _ in 0..5 {
        runtime::dispatch_root_id(runtime::HOOK_SERVER_TICK, &mut world, &mut |_| {
            original_runs += 1;
            0
        });
    }
    assert_eq!(original_runs, 5, "the template calls `original` on every tick, so the engine's own tick ran each time");

    let all = runtime::take_reports().unwrap();
    // the template reports the first three ticks only
    assert_eq!(slot(&all, 1), [1, 2, 3], "{all:?}");
    assert_eq!(slot(&all, 2), [i64::from(total); 3], "it read the experience total through the engine: {all:?}");
    let stats = runtime::stats();
    assert!(!stats.disabled, "{:?}", stats.trap);
    assert_eq!(stats.entries, 5);

    runtime::set_engine_dispatch(None);
    runtime::uninstall().unwrap();
}

#[test]
fn the_template_gets_an_error_not_a_trap_when_the_game_has_nothing_at_the_path() {
    let _serial = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    runtime::uninstall().unwrap();
    runtime::set_engine_dispatch(Some(dispatch));
    let report = install_and_load("no_path");
    assert_eq!(report.loaded.len(), 1, "{:?} {:?}", report.skipped, report.problems);

    struct Empty;
    impl PathRoot for Empty {
        fn resolve(&mut self, _: &[Accessor], _: &mut dyn FnMut(&mut dyn std::any::Any)) -> Result<(), Status> {
            Err(Status::Stale)
        }
    }
    runtime::dispatch_root_id(runtime::HOOK_SERVER_TICK, &mut Empty, &mut |_| 0);
    let all = runtime::take_reports().unwrap();
    // -101 is a Stale handle (the template's `error_code`); the mod is still enabled
    assert_eq!(slot(&all, 2), [-101], "{all:?}");
    assert!(!runtime::stats().disabled);

    runtime::set_engine_dispatch(None);
    runtime::uninstall().unwrap();
}
