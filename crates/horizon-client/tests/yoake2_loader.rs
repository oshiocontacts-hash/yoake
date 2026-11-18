//! MX3L1/MX3L2: the full-access mod loader. Packages are real `.yak` files (packed with `pack_directory`, signed with
//! `sign_file`), the runtime is the real slice runtime.
//!
//!   cargo test --offline -j 4 -p horizon-client --features yoake2-slice --test yoake2_loader

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use horizon_client::yoake2_mods::{AllowFile, ApprovedList, Control, Origin, OriginFile, OriginRecord, Refusal, decide, install_package, load_dir, load_package, load_world_mods, remove_package, sha256_hex};
use horizon_mods::yak::pack_directory;
use horizon_release_sign::SigningKey;
use yoake_slice::activation::FieldTable;
use yoake_slice::runtime::{self, ModConfig};

struct Thing {
    count: i32,
}

static TABLE: FieldTable<Thing> = FieldTable {
    get: |t, f| (f == 0).then(|| i64::from(t.count)),
    set: |t, f, v| {
        if f == 0 {
            t.count = v as i32;
            true
        } else {
            false
        }
    },
};

/// Hook 1: the engine body adds 300.
fn engine(t: &mut Thing) {
    runtime::dispatch_id(1, t, &TABLE, &mut |t| {
        t.count += 300;
        0
    });
}

/// A multiplexed guest for hook 1 that adds `add` to the count before `original` (text format; the loader
/// hands the module bytes to the runtime, which accepts text and binary).
fn module(priority: i32, add: i32) -> String {
    format!(
        r#"(module
  (@custom "yoake_abi" "\01\00\00\00")
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "get" (func $get (param i64 i32 i32) (result i32)))
  (import "env" "set" (func $set (param i64 i32 i64) (result i32)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (drop (call $register (i32.const 1) (i32.const {priority}))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (drop (call $get (local.get $h) (i32.const 0) (i32.const 32)))
    (drop (call $set (local.get $h) (i32.const 0) (i64.add (i64.load (i32.const 32)) (i64.const {add}))))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (i64.const 0)))"#
    )
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("yoake2_loader").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Packs a package; `world` is the `[wasm] world` line (empty for none) and `module_text` the module.
fn pack(dir: &Path, id: &str, wasm: &str, module_text: &str) -> PathBuf {
    pack_as(dir, &format!("{id}.yak"), id, wasm, module_text)
}

/// [`pack`] with the file name chosen (two files can then carry one id).
fn pack_as(dir: &Path, file_name: &str, id: &str, wasm: &str, module_text: &str) -> PathBuf {
    let src = dir.join(format!("src_{}", file_name.trim_end_matches(".yak")));
    std::fs::create_dir_all(src.join("bin/wasm")).unwrap();
    std::fs::write(
        src.join("yak.toml"),
        format!(
            r#"format = 1
id = "{id}"
version = "1.0.0"
name = "{id}"
authors = ["test"]
description = "a test mod"

[yoake]
version = ">=0.0.0"

[side]
client = false
server = true
required_on_join = false

[wasm]
interface = "1.0"
{wasm}
"#
        ),
    )
    .unwrap();
    std::fs::write(src.join("bin/wasm/mod.wasm"), module_text).unwrap();
    let out = dir.join(file_name);
    pack_directory(&src, &out, &BTreeMap::new()).unwrap();
    out
}


fn full_access(dir: &Path, id: &str, priority: i32, add: i32) -> PathBuf {
    pack(dir, id, "server = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"", &module(priority, add))
}

fn full_access_as(dir: &Path, file_name: &str, id: &str, priority: i32, add: i32) -> PathBuf {
    pack_as(dir, file_name, id, "server = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"", &module(priority, add))
}

/// The player approved exactly these package files.
fn allow(paths: &[&Path]) -> AllowFile {
    let mut file = AllowFile::default();
    for path in paths {
        file.allow_package(&std::fs::read(path).unwrap()).unwrap();
    }
    file
}

/// These package files arrived with `origin` recorded.
fn arrived(paths: &[&Path], origin: Origin) -> OriginFile {
    let mut file = OriginFile::default();
    for path in paths {
        let bytes = std::fs::read(path).unwrap();
        file.record(&sha256_hex(&bytes), OriginRecord { origin, id: String::new(), version: String::new(), signer: None });
    }
    file
}

/// A launcher-owned control folder outside the game root, and the game's mods folder: `<scratch>/launcher/yoake-full-access`
/// and `<scratch>/game/mods`.
fn world(name: &str) -> (Control, PathBuf) {
    let base = scratch(name);
    let mods = base.join("game").join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let control = Control::create(&base.join("launcher").join("yoake-full-access"), &base.join("game")).unwrap();
    (control, mods)
}

fn cfg() -> ModConfig {
    ModConfig::new("")
}

fn names() -> Vec<String> {
    runtime::all_stats().into_iter().map(|(_, name, _)| name).collect()
}

#[test]
fn an_allowed_local_mod_loads_and_its_hook_runs() {
    runtime::uninstall().unwrap();
    let dir = scratch("allowed");
    let path = full_access(&dir, "demo_one", 10, 7);
    let report = load_dir(&dir, &arrived(&[&path], Origin::Local), &allow(&[&path]), &[], &cfg());
    assert_eq!(report.loaded.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.loaded[0].id, "demo_one");
    assert_eq!(report.loaded[0].sha256, sha256_hex(&std::fs::read(&path).unwrap()));
    assert_eq!(names(), ["demo_one"]);
    assert_eq!(runtime::stats().registered, vec![(1, 10)]);
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 307, "the mod added 7, the engine 300");
    runtime::uninstall().unwrap();
}

#[test]
fn a_mod_nobody_allowed_is_not_loaded() {
    runtime::uninstall().unwrap();
    let dir = scratch("denied");
    let path = full_access(&dir, "demo_two", 10, 7);
    let report = load_dir(&dir, &arrived(&[&path], Origin::Local), &AllowFile::default(), &[], &cfg());
    assert!(report.loaded.is_empty());
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].1, Refusal::NotAllowed);
    assert!(names().is_empty(), "nothing was loaded");
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 300);
}

#[test]
fn a_manifest_without_full_access_stays_sandboxed() {
    runtime::uninstall().unwrap();
    let dir = scratch("sandboxed");
    // an ordinary sandboxed server module (no `world`): even approved and local, this loader leaves it alone
    let path = pack(&dir, "demo_plain", "server = \"bin/wasm/mod.wasm\"", "not even checked here");
    let report = load_dir(&dir, &arrived(&[&path], Origin::Local), &allow(&[&path]), &[], &cfg());
    assert!(report.loaded.is_empty());
    assert_eq!(report.skipped[0].1, Refusal::Sandboxed, "left to the existing WebAssembly path");
    assert!(names().is_empty());
    assert!(report.messages().is_empty(), "a sandboxed package is not worth a line");
}

#[test]
fn a_package_from_a_server_or_a_friend_is_never_full_access() {
    runtime::uninstall().unwrap();
    let dir = scratch("delivered");
    let path = full_access(&dir, "demo_three", 10, 7);
    let bytes = std::fs::read(&path).unwrap();
    let approved = allow(&[&path]);
    assert!(approved.is_allowed("demo_three", &sha256_hex(&bytes)), "its hash is on the allow list");
    for (origin, expected) in [(Origin::Server, Refusal::FromServer), (Origin::Friend, Refusal::FromFriend)] {
        let result = load_package(bytes.clone(), Some(origin), &approved, &[], cfg());
        assert_eq!(result.unwrap_err(), expected, "{origin:?}, even with the hash allowed");
        // and through the folder scan, with the origin read from the arrival record
        let report = load_dir(&dir, &arrived(&[&path], origin), &approved, &[], &cfg());
        assert!(report.loaded.is_empty());
        assert_eq!(report.skipped[0].1, expected);
    }
    assert!(names().is_empty());
    // the policy itself, for every combination
    for approved in [false, true] {
        for signed in [false, true] {
            assert_eq!(decide(Origin::Server, approved, signed), Err(Refusal::FromServer));
            assert_eq!(decide(Origin::Friend, approved, signed), Err(Refusal::FromFriend));
            assert_eq!(decide(Origin::Local, approved, signed).is_ok(), approved, "a local mod needs its approval, a signature does not replace it");
            assert_eq!(decide(Origin::Catalog, approved, signed).is_ok(), approved || signed);
        }
    }
}

#[test]
fn a_package_with_no_recorded_origin_is_never_full_access() {
    runtime::uninstall().unwrap();
    let dir = scratch("no_origin");
    let path = full_access(&dir, "demo_orphan", 10, 7);
    let bytes = std::fs::read(&path).unwrap();
    let approved = allow(&[&path]);
    // the direct call
    let error = load_package(bytes.clone(), None, &approved, &[], cfg()).unwrap_err();
    assert_eq!(error, Refusal::UnknownOrigin { sha256: sha256_hex(&bytes) });
    // the folder scan: the file sits in the mods folder and is approved, but nothing recorded its arrival
    let report = load_dir(&dir, &OriginFile::default(), &approved, &[], &cfg());
    assert!(report.loaded.is_empty());
    assert!(matches!(report.skipped[0].1, Refusal::UnknownOrigin { .. }), "{:?}", report.skipped);
    assert!(report.messages()[0].contains("no origin is recorded"), "{:?}", report.messages());
    // an origin written by a newer build (a word this build does not know) counts as no origin
    let text = format!("[[package]]\nsha256 = \"{}\"\nid = \"demo_orphan\"\nversion = \"1.0.0\"\norigin = \"usb-stick\"\n", sha256_hex(&bytes));
    let origins = OriginFile::parse(&text).unwrap();
    assert!(origins.is_empty());
    let report = load_dir(&dir, &origins, &approved, &[], &cfg());
    assert!(report.loaded.is_empty());
    // an unreadable record file is an error the loader reports, it never turns into trust
    assert!(OriginFile::parse("[[package]]\nsha256 = \"nothex\"\norigin = \"local\"\n").is_err());
    assert!(names().is_empty());
}

#[test]
fn an_allowed_package_whose_content_changes_is_not_loaded_until_approved_again() {
    runtime::uninstall().unwrap();
    let (control, mods) = world("changed");
    let v1 = std::fs::read(full_access(&scratch("changed_v1"), "demo_update", 10, 7)).unwrap();
    // the player installs it (origin local) and approves it, through the launcher's entry points
    install_package(&control, &mods, "demo_update.yak", &v1, Origin::Local, None).unwrap();
    let entry = control.approve(&v1).unwrap();
    assert_eq!((entry.id.as_str(), entry.version.as_str(), entry.sha256.as_str()), ("demo_update", "1.0.0", sha256_hex(&v1).as_str()));

    let load = |allow: &AllowFile| load_dir(&mods, &control.origins().unwrap(), allow, &[], &cfg());
    let report = load(&control.allow().unwrap());
    assert_eq!(report.loaded.len(), 1, "the approved content loads: {:?}", report.skipped);
    runtime::uninstall().unwrap();

    // installing over an existing file is refused: the approved file cannot be replaced by an install
    let v2 = std::fs::read(full_access(&scratch("changed_v2"), "demo_update", 10, 9)).unwrap();
    assert_ne!(v1, v2);
    let error = install_package(&control, &mods, "demo_update.yak", &v2, Origin::Local, None).unwrap_err();
    assert!(error.contains("never overwritten"), "{error}");
    assert_eq!(std::fs::read(mods.join("demo_update.yak")).unwrap(), v1, "the file is untouched");
    // an update: the old file is removed first, then the new one arrives
    remove_package(&mods, "demo_update.yak").unwrap();
    install_package(&control, &mods, "demo_update.yak", &v2, Origin::Local, None).unwrap();
    let report = load(&control.allow().unwrap());
    assert!(report.loaded.is_empty(), "not loaded: the old approval does not match the new content");
    assert!(names().is_empty());
    match &report.skipped[0].1 {
        Refusal::ContentChanged { id, allowed, actual } => {
            assert_eq!(id, "demo_update");
            assert_eq!(allowed, &[sha256_hex(&v1)]);
            assert_eq!(actual, &sha256_hex(&v2));
        }
        other => panic!("expected ContentChanged, got {other:?}"),
    }
    let message = &report.messages()[0];
    assert!(message.contains("changed since it was approved") && message.contains("approve it again"), "{message}");
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 300, "no mod ran");

    // approving the new content loads it (and the old content stops matching)
    control.approve(&v2).unwrap();
    assert!(!control.allow().unwrap().is_allowed("demo_update", &sha256_hex(&v1)));
    let report = load(&control.allow().unwrap());
    assert_eq!(report.loaded.len(), 1, "{:?}", report.skipped);
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 309, "the new content ran");
    runtime::uninstall().unwrap();
}

#[test]
fn a_second_package_with_the_same_id_disables_both() {
    runtime::uninstall().unwrap();
    let dir = scratch("duplicate");
    let first = full_access_as(&dir, "a_demo.yak", "demo_dup", 10, 7);
    let second = full_access_as(&dir, "b_demo_copy.yak", "demo_dup", 10, 9);
    let other = full_access_as(&dir, "c_other.yak", "demo_fine", 20, 1);
    // everything is local and approved: the duplicate id is the only problem
    let all = [first.as_path(), second.as_path(), other.as_path()];
    let report = load_dir(&dir, &arrived(&all, Origin::Local), &allow(&all), &[], &cfg());
    assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["demo_fine"], "the unrelated mod still loads");
    assert_eq!(report.skipped.len(), 2);
    for (path, refusal) in &report.skipped {
        let me = path.file_name().unwrap().to_string_lossy().into_owned();
        match refusal {
            Refusal::DuplicateId { id, others } => {
                assert_eq!(id, "demo_dup");
                assert_eq!(others.len(), 1);
                assert_ne!(others[0], me);
            }
            other => panic!("{me}: expected DuplicateId, got {other:?}"),
        }
    }
    assert_eq!(names(), ["demo_fine"]);
    let lines = report.messages();
    assert_eq!(lines.len(), 2, "one line for each of the two");
    assert!(lines.iter().all(|l| l.contains("same id 'demo_dup'") && l.contains("neither is loaded")), "{lines:?}");
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 301, "only demo_fine ran");
    runtime::uninstall().unwrap();
}

#[test]
fn server_and_friend_packages_live_apart_and_cannot_disturb_an_approved_mod() {
    runtime::uninstall().unwrap();
    let (control, mods) = world("remote");
    let local = std::fs::read(full_access(&scratch("remote_src_a"), "demo_mine", 10, 7)).unwrap();
    install_package(&control, &mods, "demo_mine.yak", &local, Origin::Local, None).unwrap();
    control.approve(&local).unwrap();
    let origins_before = std::fs::read(control.dir().join("yak-origins.toml")).unwrap();
    let allow_before = std::fs::read(control.dir().join("full-access.toml")).unwrap();

    // a server sends a package with the SAME id (other bytes) and a friend sends the SAME bytes
    let other_bytes = std::fs::read(full_access(&scratch("remote_src_b"), "demo_mine", 10, 99)).unwrap();
    let a = install_package(&control, &mods, "demo_mine.yak", &other_bytes, Origin::Server, None).unwrap();
    let b = install_package(&control, &mods, "demo_mine.yak", &local, Origin::Friend, None).unwrap();
    assert_eq!((a, b), (sha256_hex(&other_bytes), sha256_hex(&local)));
    // they are stored apart, under their own origin, named by their hash, and never in the mods folder proper
    let (hash_other, hash_local) = (sha256_hex(&other_bytes), sha256_hex(&local));
    assert_eq!(std::fs::read(mods.join("remote").join("server").join(format!("{hash_other}.yak"))).unwrap(), other_bytes);
    assert_eq!(std::fs::read(mods.join("remote").join("friend").join(format!("{hash_local}.yak"))).unwrap(), local);
    assert!(!mods.join("remote").join("server").join("demo_mine.yak").exists(), "the sender's file name is not used");
    assert_eq!(std::fs::read(mods.join("demo_mine.yak")).unwrap(), local, "the approved file is untouched (the same file name too)");
    // the control files did not change at all: no record was added, none changed, the same bytes were not demoted
    assert_eq!(std::fs::read(control.dir().join("yak-origins.toml")).unwrap(), origins_before);
    assert_eq!(std::fs::read(control.dir().join("full-access.toml")).unwrap(), allow_before);
    assert_eq!(control.origins().unwrap().origin_of(&sha256_hex(&local)), Some(Origin::Local));

    // the approved local mod still loads: no duplicate id, no disabling
    let report = load_dir(&mods, &control.origins().unwrap(), &control.allow().unwrap(), &[], &cfg());
    assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["demo_mine"], "{:?}", report.skipped);
    assert!(report.skipped.is_empty());
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 307);
    runtime::uninstall().unwrap();

    // the same package delivered twice is fine, and a file pre-placed under the SENDER's name cannot block a delivery
    install_package(&control, &mods, "demo_mine.yak", &other_bytes, Origin::Server, None).unwrap();
    std::fs::write(mods.join("remote").join("friend").join("demo_other.yak"), b"squatter").unwrap();
    install_package(&control, &mods, "demo_other.yak", &other_bytes, Origin::Friend, None).unwrap();
    // but a file of the right (hash) name with other bytes is never overwritten
    let squatted = mods.join("remote").join("server").join(format!("{}.yak", sha256_hex(&local)));
    std::fs::write(&squatted, b"squatter").unwrap();
    let error = install_package(&control, &mods, "demo_mine.yak", &local, Origin::Server, None).unwrap_err();
    assert!(error.contains("never overwritten"), "{error}");
    assert_eq!(std::fs::read(&squatted).unwrap(), b"squatter");
    for bad in ["../x.yak", "sub/x.yak", ".hidden.yak", "x.zip", "", "C:x.yak"] {
        assert!(install_package(&control, &mods, bad, &local, Origin::Local, None).is_err(), "{bad:?}");
        assert!(install_package(&control, &mods, bad, &local, Origin::Server, None).is_err(), "{bad:?}");
    }
    // catalog installs keep the signer, in the sealed record
    let signed = std::fs::read(full_access(&scratch("remote_src_c"), "demo_cat", 10, 7)).unwrap();
    let sha = install_package(&control, &mods, "demo_cat.yak", &signed, Origin::Catalog, Some("yoake-release-1")).unwrap();
    let back = control.origins().unwrap();
    assert_eq!(back.get(&sha).unwrap().signer.as_deref(), Some("yoake-release-1"));
    assert_eq!(OriginFile::parse(&back.to_text()).unwrap(), back);
    assert!(names().is_empty());
}

#[test]
fn the_launcher_entry_point_installs_a_local_package_and_a_hand_dropped_one_stays_unknown() {
    runtime::uninstall().unwrap();
    let (control, mods) = world("entry");
    let source_dir = scratch("entry_src");
    let chosen = full_access(&source_dir, "demo_chosen", 10, 7);
    let sha = control.install_local(&mods, &chosen).unwrap();
    assert_eq!(sha, sha256_hex(&std::fs::read(&chosen).unwrap()));
    assert!(mods.join("demo_chosen.yak").is_file());
    assert_eq!(control.origins().unwrap().origin_of(&sha), Some(Origin::Local));
    assert!(control.install_local(&mods, &chosen).unwrap_err().contains("never overwritten"));
    control.approve(&std::fs::read(&chosen).unwrap()).unwrap();
    // a file the player drops into the folder by hand, even approved by hash: no record, so not full-access
    let dropped = full_access(&mods, "demo_dropped", 10, 7);
    control.approve(&std::fs::read(&dropped).unwrap()).unwrap();
    let report = load_dir(&mods, &control.origins().unwrap(), &control.allow().unwrap(), &[], &cfg());
    assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["demo_chosen"]);
    assert!(report.skipped.iter().any(|(p, r)| p.ends_with("demo_dropped.yak") && matches!(r, Refusal::UnknownOrigin { .. })), "{:?}", report.skipped);
    runtime::uninstall().unwrap();
}

#[test]
fn the_game_loads_only_packages_that_match_the_launchers_approved_list() {
    runtime::uninstall().unwrap();
    let (control, mods) = world("worldstart");
    let bytes = std::fs::read(full_access(&scratch("worldstart_src"), "demo_world", 10, 7)).unwrap();
    install_package(&control, &mods, "demo_world.yak", &bytes, Origin::Local, None).unwrap();
    control.approve(&bytes).unwrap();
    // an installed, never approved package, and a hand-dropped one
    let unapproved = std::fs::read(full_access(&scratch("worldstart_src2"), "demo_other", 10, 1)).unwrap();
    install_package(&control, &mods, "demo_other.yak", &unapproved, Origin::Local, None).unwrap();
    full_access(&mods, "demo_dropped", 10, 2);

    // no list given: nothing is full-access, and the report says why
    let report = load_world_mods(&mods, None);
    assert!(report.loaded.is_empty());
    assert!(report.messages().iter().any(|l| l.contains("--full-access-approved")), "{:?}", report.messages());
    assert!(names().is_empty());

    // the launcher verifies the control files and builds this launch's list; the game receives it as a file
    let list = control.approved_list(&mods).unwrap();
    assert_eq!(list.entries().len(), 1, "only the approved package is on it");
    assert_eq!((list.entries()[0].id.as_str(), list.entries()[0].origin), ("demo_world", Origin::Local));
    assert!(list.protected().contains(&control.dir().to_path_buf()), "it names the control folder as protected");
    let file = list.write_for_launch(&scratch("worldstart_run")).unwrap();
    let list = ApprovedList::read(&file).unwrap();
    for _ in 0..2 {
        let report = load_world_mods(&mods, Some(&list));
        assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["demo_world"], "{:?} {:?}", report.skipped, report.problems);
        assert_eq!(names(), ["demo_world"], "loading again replaces the mods, never duplicates them");
        // the unapproved and the hand-dropped package are refused
        assert!(report.skipped.iter().any(|(p, r)| p.ends_with("demo_other.yak") && matches!(r, Refusal::UnknownOrigin { .. })));
        assert!(report.skipped.iter().any(|(p, r)| p.ends_with("demo_dropped.yak") && matches!(r, Refusal::UnknownOrigin { .. })));
    }

    // only matching bytes: the approved package is replaced by other bytes under the same id and name
    std::fs::remove_file(mods.join("demo_world.yak")).unwrap();
    std::fs::write(mods.join("demo_world.yak"), std::fs::read(full_access(&scratch("worldstart_src3"), "demo_world", 10, 99)).unwrap()).unwrap();
    let report = load_world_mods(&mods, Some(&list));
    assert!(report.loaded.is_empty(), "other bytes under the approved id do not load");
    assert!(names().is_empty());
    std::fs::write(mods.join("demo_world.yak"), &bytes).unwrap();

    // a list entry with origin server or friend does not even parse
    let text = list.to_text().replace("origin = \"local\"", "origin = \"server\"");
    assert!(ApprovedList::parse(&text).is_err());
    runtime::uninstall().unwrap();
}

#[test]
fn a_tampered_control_file_has_no_effect_on_the_current_launch_and_is_refused_for_the_next() {
    runtime::uninstall().unwrap();
    let (control, mods) = world("tamper");
    let bytes = std::fs::read(full_access(&scratch("tamper_src"), "demo_good", 10, 7)).unwrap();
    install_package(&control, &mods, "demo_good.yak", &bytes, Origin::Local, None).unwrap();
    control.approve(&bytes).unwrap();
    let evil = std::fs::read(full_access(&scratch("tamper_src2"), "demo_evil", 10, 1)).unwrap();
    std::fs::write(mods.join("demo_evil.yak"), &evil).unwrap();

    // this launch: the launcher's list, as the game got it
    let list = ApprovedList::read(&control.approved_list(&mods).unwrap().write_for_launch(&scratch("tamper_run")).unwrap()).unwrap();
    assert_eq!(load_world_mods(&mods, Some(&list)).loaded.len(), 1);

    // a running mod (or anything else) now rewrites BOTH control files and the checksum file: it approves itself
    let evil_hash = sha256_hex(&evil);
    let allow_path = control.dir().join("full-access.toml");
    let origins_path = control.dir().join("yak-origins.toml");
    let mut allow = std::fs::read_to_string(&allow_path).unwrap();
    allow.push_str(&format!("\n[[allow]]\nid = \"demo_evil\"\nversion = \"1.0.0\"\nsha256 = \"{evil_hash}\"\n"));
    std::fs::write(&allow_path, &allow).unwrap();
    let mut origins = std::fs::read_to_string(&origins_path).unwrap();
    origins.push_str(&format!("\n[[package]]\nsha256 = \"{evil_hash}\"\nid = \"demo_evil\"\nversion = \"1.0.0\"\norigin = \"local\"\n"));
    std::fs::write(&origins_path, &origins).unwrap();
    control.accept_current_files().unwrap(); // even a forged checksum file changes nothing for the running game

    // the running game never reads the folder: its world start (next world, same launch) loads the same set as before
    let report = load_world_mods(&mods, Some(&list));
    assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["demo_good"], "{:?}", report.skipped);
    assert!(report.skipped.iter().any(|(p, r)| p.ends_with("demo_evil.yak") && matches!(r, Refusal::UnknownOrigin { .. })));
    runtime::uninstall().unwrap();

    // the NEXT launch: the launcher sees the change in the new list and can show the diff before the player accepts it
    let next = control.approved_list(&mods).unwrap();
    let diff = list.diff(&next);
    assert_eq!(diff.added.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["demo_evil"]);
    assert!(diff.removed.is_empty() && diff.changed.is_empty());

    // and a file damaged without the checksum being updated is refused outright: no list at all
    std::fs::write(&allow_path, format!("{allow}\n# damaged\n")).unwrap();
    assert!(control.approved_list(&mods).is_err());
    assert!(load_world_mods(&mods, None).loaded.is_empty());
}

#[test]
fn nothing_inside_or_above_the_control_folder_can_be_reached_by_a_guarded_path() {
    let (control, mods) = world("guarded");
    // the control files sit outside the game root
    assert!(!control.dir().starts_with(mods.parent().unwrap()));
    let guard = horizon_client::yoake2_mods::guard;
    // inside the folder and at its checksum file
    for path in [control.dir().to_path_buf(), control.dir().join("full-access.toml"), control.dir().join("yak-origins.toml"), control.checksum_path()] {
        assert!(guard(&path).is_err(), "{path:?}");
    }
    // an ancestor (a storage root that contains the folder) and any `..`
    assert!(guard(control.dir().parent().unwrap()).is_err());
    assert!(guard(&PathBuf::from(format!("{}/..", mods.to_string_lossy().trim_start_matches(r"\\?\")))).is_err());
    assert!(guard(&mods.join("anything.yak")).is_ok());
}

#[test]
fn a_catalog_mod_needs_a_trusted_signer_or_the_approval() {
    runtime::uninstall().unwrap();
    let dir = scratch("catalog");
    let (trusted_key, _) = SigningKey::generate().unwrap();
    let (other_key, _) = SigningKey::generate().unwrap();
    let trusted = [trusted_key.public()];

    let signed = full_access(&dir, "demo_signed", 10, 7);
    horizon_release_sign::yak::sign_file(&signed, &trusted_key).unwrap();
    let loaded = load_package(std::fs::read(&signed).unwrap(), Some(Origin::Catalog), &AllowFile::default(), &trusted, cfg()).unwrap();
    assert_eq!(loaded.id, "demo_signed", "signed by a key on the trust list: no approval needed");

    let stranger = full_access(&dir, "demo_stranger", 10, 7);
    horizon_release_sign::yak::sign_file(&stranger, &other_key).unwrap();
    let error = load_package(std::fs::read(&stranger).unwrap(), Some(Origin::Catalog), &allow(&[&stranger]), &trusted, cfg()).unwrap_err();
    assert!(matches!(error, Refusal::BadSignature(_)), "{error:?}: a signature that does not verify refuses a catalog package");

    let unsigned = full_access(&dir, "demo_unsigned", 10, 7);
    let error = load_package(std::fs::read(&unsigned).unwrap(), Some(Origin::Catalog), &AllowFile::default(), &trusted, cfg()).unwrap_err();
    assert_eq!(error, Refusal::NotAllowed);
    runtime::uninstall().unwrap();
}

#[test]
fn several_mods_of_a_folder_load_in_file_order_and_compose_by_priority() {
    runtime::uninstall().unwrap();
    let dir = scratch("several");
    let a = full_access(&dir, "a_outer", 10, 1);
    let b = full_access(&dir, "b_inner", 20, 20);
    let report = load_dir(&dir, &arrived(&[&a, &b], Origin::Local), &allow(&[&a, &b]), &[], &cfg());
    assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["a_outer", "b_inner"], "file order");
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 321, "both mods ran, once each, around one engine call");
    assert!(runtime::all_stats().iter().all(|(_, _, s)| s.entries == 1 && !s.disabled));
    runtime::uninstall().unwrap();
}

#[test]
fn the_allow_file_round_trips_and_old_entries_approve_nothing() {
    let (control, _) = world("allowfile");
    assert!(control.allow().unwrap().entries().is_empty());
    let hash_one = "a".repeat(64);
    let hash_two = "b".repeat(64);
    let mut file = AllowFile::default();
    file.allow("one", "1.0.0", &hash_one);
    file.allow("two", "2.0.0", &hash_two);
    control.save_allow(&file).unwrap();
    let back = control.allow().unwrap();
    assert!(back.is_allowed("one", &hash_one) && back.is_allowed("two", &hash_two));
    assert!(!back.is_allowed("one", &hash_two), "an approval names its content");
    assert!(!back.is_allowed("three", &hash_one));
    assert_eq!(back, file);
    // approving an update replaces the old approval
    let mut file = back;
    file.allow("one", "1.1.0", &hash_two);
    assert!(!file.is_allowed("one", &hash_one) && file.is_allowed("one", &hash_two));
    file.revoke("two");
    assert!(file.approved_hashes("two").is_empty());
    // the MX3L1 form named only the id: it is read, listed, and approves nothing
    let old = AllowFile::parse("[allow]\nx = true\ny = false\n").unwrap();
    assert_eq!(old.legacy_ids(), ["x"]);
    assert!(!old.is_allowed("x", &hash_one) && old.entries().is_empty());
    assert!(AllowFile::parse("[allow]\nx = \"yes\"\n").is_err(), "an old switch is true or false");
    assert!(AllowFile::parse("[[allow]]\nid = \"x\"\nversion = \"1\"\nsha256 = \"short\"\n").is_err(), "a hash is 64 hex digits");
    assert!(AllowFile::parse("[[allow]]\nid = \"x\"\nversion = \"1\"\n").is_err(), "the hash is required");
}

#[test]
fn an_old_style_approval_does_not_load_and_the_report_says_so() {
    runtime::uninstall().unwrap();
    let dir = scratch("legacy");
    let path = full_access(&dir, "demo_old", 10, 7);
    let old = AllowFile::parse("[allow]\ndemo_old = true\n").unwrap();
    let report = load_dir(&dir, &arrived(&[&path], Origin::Local), &old, &[], &cfg());
    assert!(report.loaded.is_empty());
    assert_eq!(report.skipped[0].1, Refusal::NotAllowed);
    assert!(report.messages().iter().any(|l| l.contains("demo_old") && l.contains("without a package hash")), "{:?}", report.messages());
    assert!(names().is_empty());
}

/// A guest for ledger hooks 71 (`ArmorStand::tick_settle`, entities, bridged), 1 (entities, arg0 is not `&mut T`, so
/// not bridgeable), 103 (a `game::item_use` hook: the other bridge module) and 215 (an observe-only call site). In
/// `yoake_init` it reports what `hook_register` returned for each (slots 501..504); every hook entry reports
/// `(hook id, 1)` and runs `original`.
fn bridge_module() -> String {
    r#"(module
  (@custom "yoake_abi" "\01\00\00\00")
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init")
    (call $report (i32.const 501) (i64.extend_i32_s (call $register (i32.const 71) (i32.const 0))))
    (call $report (i32.const 502) (i64.extend_i32_s (call $register (i32.const 1) (i32.const 0))))
    (call $report (i32.const 503) (i64.extend_i32_s (call $register (i32.const 103) (i32.const 0))))
    (call $report (i32.const 504) (i64.extend_i32_s (call $register (i32.const 215) (i32.const 0)))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (call $report (local.get $id) (i64.const 1))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (i64.const 0)))"#
        .to_string()
}

#[test]
fn a_loaded_mod_that_registers_a_bridged_hook_id_is_called_by_the_engine() {
    use horizon_client::entities::{ArmorStand, StandMedium, StandStep};
    use horizon_core::geometry::Vec3;

    runtime::uninstall().unwrap();
    let (control, mods) = world("bridge");
    let fa = "server = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"";
    let bytes = std::fs::read(pack(&scratch("bridge_src"), "demo_bridge", fa, &bridge_module())).unwrap();
    install_package(&control, &mods, "demo_bridge.yak", &bytes, Origin::Local, None).unwrap();
    control.approve(&bytes).unwrap();
    let file = control.approved_list(&mods).unwrap().write_for_launch(&scratch("bridge_run")).unwrap();
    let list = ApprovedList::read(&file).unwrap();

    let step = StandStep::for_hook_bench(true, 0.6);
    let stand = || ArmorStand::new(1, Vec3::new(3.0, 64.0, 5.0), 30.0);
    let mut baseline = stand();
    baseline.tick_settle(StandMedium::Air, step);

    // two world starts: the second unloads and reloads the mod, and must not add a second dispatcher
    for round in 0..2 {
        let report = load_world_mods(&mods, Some(&list));
        assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["demo_bridge"], "{:?} {:?}", report.skipped, report.problems);
        assert_eq!(
            report.unbridged,
            vec![("demo_bridge".to_string(), vec![(1, "arg0 is not &T or &mut T".to_string())])],
            "71 and 215 (entities) and 103 (game::item_use) are bridged, 1 is not"
        );
        assert!(report.messages().iter().any(|l| l.contains("'demo_bridge'") && l.contains("1 (arg0 is not &T or &mut T)")), "{:?}", report.messages());
        // what hook_register told the guest (S4): Full = 0, not callable = 2, Full = 0, observe-only = 1
        assert_eq!(runtime::take_reports().unwrap(), [(501, 0), (502, 2), (503, 0), (504, 1)], "round {round}");

        let mut hooked = stand();
        hooked.tick_settle(StandMedium::Air, step);
        assert_eq!(hooked.velocity, baseline.velocity, "round {round}: the original ran once");
        assert_eq!(runtime::take_reports().unwrap(), [(71, 1)], "round {round}: the guest was entered once, through the bridge");
    }
    runtime::uninstall().unwrap();
}

/// A stand-in for the `Game` root: the server tick dispatches `runtime::dispatch_root_id(HOOK_SERVER_TICK, game, ..)`
/// (native_client_items.rs) and a real `Game` needs the game resources. No paths resolve.
struct TickRoot;

impl yoake_slice::activation::PathRoot for TickRoot {
    fn resolve(
        &mut self,
        _: &[yoake_slice::activation::Accessor],
        _: &mut dyn FnMut(&mut dyn std::any::Any),
    ) -> Result<(), yoake_slice::activation::Status> {
        Err(yoake_slice::activation::Status::Stale)
    }
}

#[test]
fn a_loaded_mod_on_the_server_tick_is_told_full_is_not_unbridged_and_runs_on_tick() {
    runtime::uninstall().unwrap();
    let (control, mods) = world("server_tick");
    let tick = runtime::HOOK_SERVER_TICK as i32;
    let wat = format!(
        r#"(module
  (@custom "yoake_abi" "\01\00\00\00")
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init")
    (call $report (i32.const 500) (i64.extend_i32_s (call $register (i32.const {tick}) (i32.const 0)))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (call $report (i32.const 1) (i64.extend_i32_u (local.get $id)))
    (i64.const 0)))"#
    );
    let fa = "server = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"";
    let bytes = std::fs::read(pack(&scratch("server_tick_src"), "demo_tick", fa, &wat)).unwrap();
    install_package(&control, &mods, "demo_tick.yak", &bytes, Origin::Local, None).unwrap();
    control.approve(&bytes).unwrap();
    let file = control.approved_list(&mods).unwrap().write_for_launch(&scratch("server_tick_run")).unwrap();
    let report = load_world_mods(&mods, Some(&ApprovedList::read(&file).unwrap()));
    assert_eq!(report.loaded.len(), 1, "{:?} {:?}", report.skipped, report.problems);
    assert!(report.unbridged.is_empty(), "the server tick needs no bridge: {:?}", report.unbridged);
    assert!(report.messages().is_empty(), "{:?}", report.messages());
    assert_eq!(runtime::take_reports().unwrap(), [(500, 0)], "hook_register said Full (0), not NOT_CALLABLE (2)");

    // what Game::yoake2_server_tick does every tick
    for _ in 0..2 {
        runtime::dispatch_root_id(runtime::HOOK_SERVER_TICK, &mut TickRoot, &mut |_| 0);
    }
    assert_eq!(runtime::take_reports().unwrap(), [(1, i64::from(runtime::HOOK_SERVER_TICK)); 2], "entered on each tick");
    runtime::uninstall().unwrap();
}

/// [`module`] with the `yoake_abi` section line replaced by `section` (empty: no section).
fn module_with_abi(section: &str) -> String {
    module(10, 1).replace(r#"(@custom "yoake_abi" "\01\00\00\00")"#, section)
}

#[test]
fn the_loader_checks_the_abi_version_before_any_guest_code_runs() {
    runtime::uninstall().unwrap();
    let fa = "server = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"";
    let cases = [
        // (name, section, loads)
        ("abi_same", r#"(@custom "yoake_abi" "\01\00\00\00")"#, true),
        ("abi_newer_minor", r#"(@custom "yoake_abi" "\01\00\05\00")"#, true),
        ("abi_major_2", r#"(@custom "yoake_abi" "\02\00\00\00")"#, false),
        ("abi_major_0", r#"(@custom "yoake_abi" "\00\00\09\00")"#, false),
        ("abi_missing", "", false),
        ("abi_short", r#"(@custom "yoake_abi" "\01\00\00")"#, false),
    ];
    for (name, section, loads) in cases {
        let (control, mods) = world(name);
        let bytes = std::fs::read(pack(&scratch(&format!("{name}_src")), name, fa, &module_with_abi(section))).unwrap();
        install_package(&control, &mods, &format!("{name}.yak"), &bytes, Origin::Local, None).unwrap();
        control.approve(&bytes).unwrap();
        let file = control.approved_list(&mods).unwrap().write_for_launch(&scratch(&format!("{name}_run"))).unwrap();
        let report = load_world_mods(&mods, Some(&ApprovedList::read(&file).unwrap()));
        if loads {
            assert_eq!(report.loaded.len(), 1, "{name}: {:?}", report.skipped);
            let newer = report.messages().iter().any(|l| l.contains("was built for ABI 1.5"));
            assert_eq!(newer, name == "abi_newer_minor", "{name}: {:?}", report.messages());
        } else {
            assert!(report.loaded.is_empty(), "{name}");
            assert!(matches!(&report.skipped[..], [(_, Refusal::Abi(_))]), "{name}: {:?}", report.skipped);
            assert!(names().is_empty(), "{name}: never instantiated");
            assert!(report.messages().iter().any(|l| l.contains("incompatible ABI")), "{name}: {:?}", report.messages());
        }
    }
    runtime::uninstall().unwrap();
}
