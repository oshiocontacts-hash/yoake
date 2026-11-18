//! MX3L1/MX3L2: the full-access mod loader. Packages are real `.yak` files (packed with `pack_directory`, signed with
//! `sign_file`), the runtime is the real slice runtime.
//!
//!   cargo test --offline -j 4 -p horizon-client --features yoake2-slice --test yoake2_loader

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use horizon_client::yoake2_mods::{AllowFile, Origin, OriginFile, OriginRecord, Refusal, decide, install_package, load_dir, load_package, sha256_hex};
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
    let dir = scratch("changed");
    let v1 = std::fs::read(full_access(&scratch("changed_v1"), "demo_update", 10, 7)).unwrap();
    // the player installs it and approves it: both go on record
    install_package(&dir, "demo_update.yak", &v1, Origin::Local, None).unwrap();
    let mut approved = AllowFile::default();
    let entry = approved.allow_package(&v1).unwrap();
    assert_eq!((entry.id.as_str(), entry.version.as_str(), entry.sha256.as_str()), ("demo_update", "1.0.0", sha256_hex(&v1).as_str()));
    approved.save(&dir).unwrap();

    let load = |approved: &AllowFile| load_dir(&dir, &OriginFile::load(&dir).unwrap(), approved, &[], &cfg());
    let report = load(&AllowFile::load(&dir).unwrap());
    assert_eq!(report.loaded.len(), 1, "the approved content loads: {:?}", report.skipped);
    runtime::uninstall().unwrap();

    // an update (or a replaced file) with the same id arrives locally: other bytes
    let v2 = std::fs::read(full_access(&scratch("changed_v2"), "demo_update", 10, 9)).unwrap();
    assert_ne!(v1, v2);
    install_package(&dir, "demo_update.yak", &v2, Origin::Local, None).unwrap();
    let report = load(&AllowFile::load(&dir).unwrap());
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
    let mut reapproved = AllowFile::load(&dir).unwrap();
    reapproved.allow_package(&v2).unwrap();
    assert!(!reapproved.is_allowed("demo_update", &sha256_hex(&v1)));
    let report = load(&reapproved);
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
fn delivered_packages_are_recorded_with_their_origin_and_distrust_is_sticky() {
    runtime::uninstall().unwrap();
    let dir = scratch("install");
    let bytes = std::fs::read(full_access(&scratch("install_src"), "demo_gift", 10, 7)).unwrap();
    let sha = install_package(&dir, "demo_gift.yak", &bytes, Origin::Friend, None).unwrap();
    assert_eq!(sha, sha256_hex(&bytes));
    assert!(dir.join("demo_gift.yak").is_file());
    let origins = OriginFile::load(&dir).unwrap();
    assert_eq!(origins.origin_of(&sha), Some(Origin::Friend));
    assert_eq!(origins.get(&sha).unwrap().id, "demo_gift");
    // the player approves it (the allow list is a separate file): a friend's package still never runs
    let mut approved = AllowFile::default();
    approved.allow_package(&bytes).unwrap();
    let report = load_dir(&dir, &origins, &approved, &[], &cfg());
    assert!(report.loaded.is_empty());
    assert_eq!(report.skipped[0].1, Refusal::FromFriend);
    // the same bytes installed "locally" afterwards do not lift the record
    install_package(&dir, "demo_gift_again.yak", &bytes, Origin::Local, None).unwrap();
    assert_eq!(OriginFile::load(&dir).unwrap().origin_of(&sha), Some(Origin::Friend));
    // the other way round: local bytes that a server sends too become server
    let local = std::fs::read(full_access(&scratch("install_src2"), "demo_mine", 10, 7)).unwrap();
    let sha = install_package(&dir, "demo_mine.yak", &local, Origin::Local, None).unwrap();
    assert_eq!(OriginFile::load(&dir).unwrap().origin_of(&sha), Some(Origin::Local));
    install_package(&dir, "demo_mine_from_server.yak", &local, Origin::Server, None).unwrap();
    assert_eq!(OriginFile::load(&dir).unwrap().origin_of(&sha), Some(Origin::Server));
    // catalog records keep the signer, and the file round-trips
    let signed = std::fs::read(full_access(&scratch("install_src3"), "demo_cat", 10, 7)).unwrap();
    let sha = install_package(&dir, "demo_cat.yak", &signed, Origin::Catalog, Some("yoake-release-1")).unwrap();
    let back = OriginFile::load(&dir).unwrap();
    assert_eq!(back.get(&sha).unwrap().signer.as_deref(), Some("yoake-release-1"));
    assert_eq!(OriginFile::parse(&back.to_text()).unwrap(), back);
    // names that could leave the folder or hide are refused
    for bad in ["../x.yak", "sub/x.yak", ".hidden.yak", "x.zip", ""] {
        assert!(install_package(&dir, bad, &bytes, Origin::Local, None).is_err(), "{bad:?}");
    }
    assert!(names().is_empty());
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
    let dir = scratch("allowfile");
    assert!(AllowFile::load(&dir).unwrap().entries().is_empty());
    let hash_one = "a".repeat(64);
    let hash_two = "b".repeat(64);
    let mut file = AllowFile::default();
    file.allow("one", "1.0.0", &hash_one);
    file.allow("two", "2.0.0", &hash_two);
    file.save(&dir).unwrap();
    let back = AllowFile::load(&dir).unwrap();
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
