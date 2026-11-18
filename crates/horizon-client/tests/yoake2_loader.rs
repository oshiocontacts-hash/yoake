//! MX3L1: the full-access mod loader. Packages are real `.yak` files (packed with `pack_directory`, signed with
//! `sign_file`), the runtime is the real slice runtime.
//!
//!   cargo test --offline -j 4 -p horizon-client --features yoake2-slice --test yoake2_loader

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use horizon_client::yoake2_mods::{AllowFile, Origin, Refusal, decide, load_dir, load_package};
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
    let src = dir.join(format!("src_{id}"));
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
    let out = dir.join(format!("{id}.yak"));
    pack_directory(&src, &out, &BTreeMap::new()).unwrap();
    out
}

fn full_access(dir: &Path, id: &str, priority: i32, add: i32) -> PathBuf {
    pack(dir, id, "server = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"", &module(priority, add))
}

fn allow(ids: &[&str]) -> AllowFile {
    let mut file = AllowFile::default();
    for id in ids {
        file.set(id, true);
    }
    file
}

fn names() -> Vec<String> {
    runtime::all_stats().into_iter().map(|(_, name, _)| name).collect()
}

#[test]
fn an_allowed_local_mod_loads_and_its_hook_runs() {
    runtime::uninstall().unwrap();
    let dir = scratch("allowed");
    full_access(&dir, "demo_one", 10, 7);
    let report = load_dir(&dir, Origin::LocalFolder, &allow(&["demo_one"]), &[], &ModConfig::new(""));
    assert_eq!(report.loaded.len(), 1, "{:?}", report.skipped);
    assert_eq!(report.loaded[0].id, "demo_one");
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
    full_access(&dir, "demo_two", 10, 7);
    for allow in [AllowFile::default(), {
        let mut off = AllowFile::default();
        off.set("demo_two", false);
        off
    }] {
        let report = load_dir(&dir, Origin::LocalFolder, &allow, &[], &ModConfig::new(""));
        assert!(report.loaded.is_empty());
        assert_eq!(report.skipped.len(), 1);
        assert_eq!(report.skipped[0].1, Refusal::NotAllowed);
    }
    assert!(names().is_empty(), "nothing was loaded");
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 300);
}

#[test]
fn a_manifest_without_full_access_stays_sandboxed() {
    runtime::uninstall().unwrap();
    let dir = scratch("sandboxed");
    // an ordinary sandboxed server module (no `world`): even with a switch on, this loader leaves it alone
    pack(&dir, "demo_plain", "server = \"bin/wasm/mod.wasm\"", "not even checked here");
    let report = load_dir(&dir, Origin::LocalFolder, &allow(&["demo_plain"]), &[], &ModConfig::new(""));
    assert!(report.loaded.is_empty());
    assert_eq!(report.skipped[0].1, Refusal::Sandboxed, "left to the existing WebAssembly path");
    assert!(names().is_empty());
}

#[test]
fn a_package_from_a_server_or_a_friend_is_never_full_access() {
    runtime::uninstall().unwrap();
    let dir = scratch("delivered");
    let path = full_access(&dir, "demo_three", 10, 7);
    let bytes = std::fs::read(path).unwrap();
    let switched_on = allow(&["demo_three"]);
    for (origin, expected) in [(Origin::Server, Refusal::FromServer), (Origin::Friend, Refusal::FromFriend)] {
        let result = load_package(bytes.clone(), origin, &switched_on, &[], ModConfig::new(""));
        assert_eq!(result.unwrap_err(), expected, "{origin:?}, even with the switch on");
    }
    assert!(names().is_empty());
    // the policy itself, for every combination
    for switched in [false, true] {
        for signed in [false, true] {
            assert_eq!(decide(Origin::Server, switched, signed), Err(Refusal::FromServer));
            assert_eq!(decide(Origin::Friend, switched, signed), Err(Refusal::FromFriend));
            assert_eq!(decide(Origin::LocalFolder, switched, signed).is_ok(), switched, "a local mod needs its switch, a signature does not replace it");
            assert_eq!(decide(Origin::Catalog, switched, signed).is_ok(), switched || signed);
        }
    }
}

#[test]
fn a_catalog_mod_needs_a_trusted_signer_or_the_switch() {
    runtime::uninstall().unwrap();
    let dir = scratch("catalog");
    let (trusted_key, _) = SigningKey::generate().unwrap();
    let (other_key, _) = SigningKey::generate().unwrap();
    let trusted = [trusted_key.public()];

    let signed = full_access(&dir, "demo_signed", 10, 7);
    horizon_release_sign::yak::sign_file(&signed, &trusted_key).unwrap();
    let loaded = load_package(std::fs::read(&signed).unwrap(), Origin::Catalog, &AllowFile::default(), &trusted, ModConfig::new("")).unwrap();
    assert_eq!(loaded.id, "demo_signed", "signed by a key on the trust list: no switch needed");

    let stranger = full_access(&dir, "demo_stranger", 10, 7);
    horizon_release_sign::yak::sign_file(&stranger, &other_key).unwrap();
    let error = load_package(std::fs::read(&stranger).unwrap(), Origin::Catalog, &allow(&["demo_stranger"]), &trusted, ModConfig::new("")).unwrap_err();
    assert!(matches!(error, Refusal::BadSignature(_)), "{error:?}: a signature that does not verify refuses a catalog package");

    let unsigned = full_access(&dir, "demo_unsigned", 10, 7);
    let error = load_package(std::fs::read(&unsigned).unwrap(), Origin::Catalog, &AllowFile::default(), &trusted, ModConfig::new("")).unwrap_err();
    assert_eq!(error, Refusal::NotAllowed);
    runtime::uninstall().unwrap();
}

#[test]
fn several_mods_of_a_folder_load_in_file_order_and_compose_by_priority() {
    runtime::uninstall().unwrap();
    let dir = scratch("several");
    full_access(&dir, "a_outer", 10, 1);
    full_access(&dir, "b_inner", 20, 20);
    let report = load_dir(&dir, Origin::LocalFolder, &allow(&["a_outer", "b_inner"]), &[], &ModConfig::new(""));
    assert_eq!(report.loaded.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["a_outer", "b_inner"], "file order");
    let mut t = Thing { count: 0 };
    engine(&mut t);
    assert_eq!(t.count, 321, "both mods ran, once each, around one engine call");
    assert!(runtime::all_stats().iter().all(|(_, _, s)| s.entries == 1 && !s.disabled));
    runtime::uninstall().unwrap();
}

#[test]
fn the_allow_file_round_trips_and_a_missing_file_allows_nothing() {
    let dir = scratch("allowfile");
    assert!(!AllowFile::load(&dir).unwrap().is_allowed("anything"));
    let mut file = AllowFile::default();
    file.set("one", true);
    file.set("two", false);
    file.save(&dir).unwrap();
    let back = AllowFile::load(&dir).unwrap();
    assert!(back.is_allowed("one") && !back.is_allowed("two") && !back.is_allowed("three"));
    assert_eq!(back, file);
    assert!(AllowFile::parse("[allow]\nx = \"yes\"\n").is_err(), "a switch is true or false");
}
