//! MX3L1/MX3L2: the full-access mod loader. Packages are real `.yak` files (packed with `pack_directory`, signed with
//! `sign_file`), the runtime is the real slice runtime.
//!
//!   cargo test --offline -j 4 -p horizon-client --features yoake2-slice --test yoake2_loader

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use horizon_client::yoake2_mods::{ThreadSide, dispatch_client_tick, dispatch_hud_render, load_client_mods, unload_client_mods, AllowFile, ApprovedList, Control, Origin, OriginFile, OriginRecord, Refusal, decide, install_package, load_dir, load_package, load_world_mods, remove_package, sha256_hex};
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
client = {client}
server = {server}
required_on_join = false

[wasm]
interface = "1.0"
{wasm}
"#,
            // MX3CL1: [side] follows the declared modules (`client = ...` is the client thread, `server = ...` the server)
            client = wasm.contains("client = "),
            server = wasm.contains("server = ") || !wasm.contains("client = "),
        ),
    )
    .unwrap();
    std::fs::write(src.join("bin/wasm/mod.wasm"), module_text).unwrap();
    if wasm.contains("bin/wasm/client.wasm") {
        std::fs::write(src.join("bin/wasm/client.wasm"), module_text).unwrap();
    }
    let out = dir.join(file_name);
    let _ = std::fs::remove_file(&out); // a package is packed again with other content (the dev reload tests)
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
fn a_package_from_a_server_or_a_friend_is_not_full_access_unless_approved_for_its_host() {
    runtime::uninstall().unwrap();
    let dir = scratch("delivered");
    let path = full_access(&dir, "demo_three", 10, 7);
    let bytes = std::fs::read(&path).unwrap();
    // not approved (an empty allow list): refused, whatever the origin record says, and the same through the folder scan
    for (origin, expected) in [(Origin::Server, Refusal::FromServer), (Origin::Friend, Refusal::FromFriend)] {
        let result = load_package(bytes.clone(), Some(origin), &AllowFile::default(), &[], cfg());
        assert_eq!(result.unwrap_err(), expected, "{origin:?}, not approved");
        let report = load_dir(&dir, &arrived(&[&path], origin), &AllowFile::default(), &[], &cfg());
        assert!(report.loaded.is_empty());
        assert_eq!(report.skipped[0].1, expected);
    }
    assert!(names().is_empty());
    // MX3JN1: the policy itself, for every combination: a server's or friend's package needs the approval (a signature
    // does not replace it), a local one too, a catalog one has the signature as an alternative
    for approved in [false, true] {
        for signed in [false, true] {
            assert_eq!(decide(Origin::Server, approved, signed).is_ok(), approved);
            assert_eq!(decide(Origin::Friend, approved, signed).is_ok(), approved);
            if !approved {
                assert_eq!(decide(Origin::Server, approved, signed), Err(Refusal::FromServer));
                assert_eq!(decide(Origin::Friend, approved, signed), Err(Refusal::FromFriend));
            }
            assert_eq!(decide(Origin::Local, approved, signed).is_ok(), approved, "a local mod needs its approval, a signature does not replace it");
            assert_eq!(decide(Origin::Catalog, approved, signed).is_ok(), approved || signed);
        }
    }
}

/// A game folder with the packages of one host in `mods/remote/<host dir>/` (stored the way the launcher does) and the
/// approved list of a launch that joins it: entries with `origin`, list scoped to `list_host`. Returns the mods folder and
/// the list file.
fn remote_world(name: &str, folder_host: &str, list_host: &str, origin: Origin, packs: &[(&str, &str, String)]) -> (PathBuf, PathBuf) {
    let base = scratch(name);
    let mods = base.join("game").join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let src = scratch(&format!("{name}_src"));
    let mut entries = Vec::new();
    for (id, side, text) in packs {
        let declared = match *side {
            "client" => "client = \"bin/wasm/mod.wasm\"",
            "both" => "server = \"bin/wasm/mod.wasm\"\nclient = \"bin/wasm/client.wasm\"",
            _ => "server = \"bin/wasm/mod.wasm\"",
        };
        let bytes = std::fs::read(pack(&src, id, &format!("{declared}\nworld = \"full-access\""), text)).unwrap();
        let hash = sha256_hex(&bytes);
        horizon_mods::yak::control::install_remote_package(&mods, folder_host, &bytes, &hash).unwrap();
        entries.push(horizon_mods::yak::control::ApprovedEntry { id: (*id).into(), version: "1.0.0".into(), sha256: hash, origin });
    }
    let list = ApprovedList::new(entries, vec![]).with_host(list_host);
    let file = list.write_for_launch(&scratch(&format!("{name}_run"))).unwrap();
    (mods, file)
}

fn client_loads(mods: &Path, file: &Path, key: Option<&str>) -> (Vec<String>, Vec<Refusal>, Vec<String>) {
    let (mods, file, key) = (mods.to_path_buf(), file.to_path_buf(), key.map(str::to_owned));
    std::thread::spawn(move || {
        let report = horizon_client::yoake2_mods::load_client_mods_scoped(&mods, Some(&ApprovedList::read(&file).unwrap()), key.as_deref());
        let out = (loaded_ids(&report), report.skipped.iter().map(|(_, refusal)| refusal.clone()).collect(), report.messages());
        unload_client_mods();
        out
    })
    .join()
    .unwrap()
}

#[test]
fn a_hosts_full_access_package_loads_only_for_the_launch_that_joins_that_host() {
    for (origin, key, refusal) in [(Origin::Server, "server:play.example.com:25565", Refusal::FromServer), (Origin::Friend, "friend:user_7", Refusal::FromFriend)] {
        let name = format!("remote_{}", origin.as_str());
        let packs = [("demo_host", "client", counting_guest(&ALL_THREE, None))];
        let (mods, file) = remote_world(&name, key, key, origin, &packs);
        // the launch joins this host: it runs, on the client thread
        let (loaded, skipped, messages) = client_loads(&mods, &file, Some(key));
        assert_eq!(loaded, ["demo_host"], "{origin:?}: {skipped:?} {messages:?}");
        // another host's launch (the list is scoped to the first host): nothing remote is loaded
        let (loaded, _, _) = client_loads(&mods, &file, Some("server:other.example.com:25565"));
        assert!(loaded.is_empty(), "another host's key does not read this host's folder");
        // no key (single player, a plain launch): nothing remote is read
        let (loaded, _, _) = client_loads(&mods, &file, None);
        assert!(loaded.is_empty());
        // a key that is not a host key is no key
        let (loaded, _, _) = client_loads(&mods, &file, Some("../../x"));
        assert!(loaded.is_empty());
        // the server thread (single player, the host's own world) never loads a remote folder, whatever the key
        let (m, f) = (mods.clone(), file.clone());
        let server = std::thread::spawn(move || {
            let report = load_world_mods(&m, Some(&ApprovedList::read(&f).unwrap()));
            let ids = loaded_ids(&report);
            runtime::uninstall().unwrap();
            ids
        })
        .join()
        .unwrap();
        assert!(server.is_empty());
        let _ = refusal;
    }
}

#[test]
fn a_remote_folder_of_another_host_is_not_loaded_even_with_a_list_for_this_one() {
    // the list names host A, the launch joins host A, but the package sits in host B's folder
    let a = "server:a.example.com:25565";
    let b = "server:b.example.com:25565";
    let packs = [("demo_host", "client", counting_guest(&ALL_THREE, None))];
    let (mods, file) = remote_world("remote_wrong_folder", b, a, Origin::Server, &packs);
    let (loaded, _, _) = client_loads(&mods, &file, Some(a));
    assert!(loaded.is_empty(), "host A's folder is empty");
    let (loaded, _, _) = client_loads(&mods, &file, Some(b));
    assert!(loaded.is_empty(), "the list is for host A");
}

#[test]
fn a_remote_package_that_the_list_does_not_name_or_that_changed_is_refused() {
    let key = "server:play.example.com:25565";
    let packs = [("demo_host", "client", counting_guest(&ALL_THREE, None))];
    let (mods, file) = remote_world("remote_unlisted", key, key, Origin::Server, &packs);
    // a second file dropped into the host's folder after the launcher approved: not in the list
    let src = scratch("remote_unlisted_extra");
    let extra = std::fs::read(pack(&src, "demo_extra", "client = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"", &counting_guest(&ALL_THREE, None))).unwrap();
    let folder = horizon_mods::yak::control::host_remote_dir(&mods, key).unwrap();
    std::fs::write(folder.join(format!("{}.yak", sha256_hex(&extra))), &extra).unwrap();
    let (loaded, skipped, messages) = client_loads(&mods, &file, Some(key));
    assert_eq!(loaded, ["demo_host"], "only the approved one runs");
    assert!(skipped.contains(&Refusal::UnknownOrigin { sha256: sha256_hex(&extra) }), "{skipped:?}");
    assert!(messages.iter().any(|line| line.contains("demo_extra") || line.contains(&sha256_hex(&extra)[..12])), "{messages:?}");
    // the approved file's bytes replaced under its own name: no entry matches any more
    let approved = std::fs::read_dir(&folder).unwrap().flatten().map(|item| item.path()).find(|path| path.file_name().unwrap().to_string_lossy() != format!("{}.yak", sha256_hex(&extra))).unwrap();
    std::fs::write(&approved, &extra[..extra.len() - 1]).unwrap();
    let (loaded, _, _) = client_loads(&mods, &file, Some(key));
    assert!(loaded.is_empty(), "changed bytes are not the approved package");
}

#[test]
fn an_entry_of_a_host_scope_does_not_make_a_hand_dropped_file_in_the_plain_folder_run() {
    let key = "server:play.example.com:25565";
    let packs = [("demo_host", "client", counting_guest(&ALL_THREE, None))];
    let (mods, file) = remote_world("remote_plain_folder", key, key, Origin::Server, &packs);
    // the same approved bytes, copied by hand into the plain mods folder: the plain scan has no origin for them
    let folder = horizon_mods::yak::control::host_remote_dir(&mods, key).unwrap();
    let remote = std::fs::read_dir(&folder).unwrap().flatten().next().unwrap().path();
    std::fs::copy(&remote, mods.join("hand_dropped.yak")).unwrap();
    std::fs::remove_file(&remote).unwrap();
    let (loaded, skipped, _) = client_loads(&mods, &file, Some(key));
    assert!(loaded.is_empty(), "{skipped:?}");
    assert!(matches!(skipped.as_slice(), [Refusal::UnknownOrigin { .. }]), "{skipped:?}");
}

#[test]
fn a_remote_mod_with_the_id_of_a_local_one_does_not_replace_it() {
    let key = "friend:user_7";
    let guest = counting_guest(&ALL_THREE, None);
    let (mods, file) = remote_world("remote_same_id", key, key, Origin::Friend, &[("demo_same", "client", guest.clone())]);
    // the player has their own approved package of that id; the same list carries both entries
    let src = scratch("remote_same_id_local");
    let local = std::fs::read(pack(&src, "demo_same", "client = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"", &counting_guest(&ALL_THREE, Some(900)))).unwrap();
    std::fs::write(mods.join("demo_same.yak"), &local).unwrap();
    let mut list = ApprovedList::read(&file).unwrap();
    let mut entries = list.entries().to_vec();
    entries.push(horizon_mods::yak::control::ApprovedEntry { id: "demo_same".into(), version: "1.0.0".into(), sha256: sha256_hex(&local), origin: Origin::Local });
    list = ApprovedList::new(entries, vec![]).with_host(key);
    let file = list.write_for_launch(&scratch("remote_same_id_run2")).unwrap();
    let (mods2, file2) = (mods.clone(), file.clone());
    let (loaded, problems) = std::thread::spawn(move || {
        let report = horizon_client::yoake2_mods::load_client_mods_scoped(&mods2, Some(&ApprovedList::read(&file2).unwrap()), Some(key));
        let out = (report.loaded.iter().map(|l| l.sha256.clone()).collect::<Vec<_>>(), report.problems.clone());
        assert_eq!(names().len(), 1, "one mod of that id runs");
        unload_client_mods();
        out
    })
    .join()
    .unwrap();
    assert_eq!(loaded, [sha256_hex(&local)], "the player's own package wins");
    assert!(problems.iter().any(|line| line.contains("has the id of one of your own mods")), "{problems:?}");
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
            vec![("demo_bridge".to_string(), vec![(1, "arg0 is not &mut T".to_string())])],
            "71 and 215 (entities) and 103 (game::item_use) are bridged, 1 is not"
        );
        assert!(report.messages().iter().any(|l| l.contains("'demo_bridge'") && l.contains("1 (arg0 is not &mut T)")), "{:?}", report.messages());
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

// ---------------------------------------------------------------------------------------------------------------
// MX3CL1: the client side
// ---------------------------------------------------------------------------------------------------------------

/// A guest that registers `ids` and counts its entries in a wasm global (its private state). Each entry reports
/// `(id - SYNTHETIC_HOOK_BASE, n)`; `yoake_init` reports the registration statuses as `(500 + index, status)`. When
/// `trap_on` is `Some(id)` the guest executes `unreachable` instead, for that id.
fn counting_guest(ids: &[u32], trap_on: Option<u32>) -> String {
    let register: String = ids
        .iter()
        .enumerate()
        .map(|(i, id)| format!("(call $report (i32.const {}) (i64.extend_i32_s (call $register (i32.const {}) (i32.const 0))))\n", 500 + i, *id as i32))
        .collect();
    let trap = trap_on.map_or(String::new(), |id| format!("(if (i32.eq (local.get $id) (i32.const {})) (then unreachable))", id as i32));
    format!(
        r#"(module
  (@custom "yoake_abi" "\01\00\00\00")
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (global $n (mut i64) (i64.const 0))
  (func (export "yoake_init") {register})
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    {trap}
    (global.set $n (i64.add (global.get $n) (i64.const 1)))
    (call $report (i32.sub (local.get $id) (i32.const {base})) (global.get $n))
    (i64.const 0)))"#,
        base = runtime::SYNTHETIC_HOOK_BASE as i32
    )
}

const ALL_THREE: [u32; 3] = [runtime::HOOK_SERVER_TICK, runtime::HOOK_CLIENT_TICK, runtime::HOOK_HUD_RENDER];

/// Installs and approves packages `(id, "server" | "client" | "both" (the modules it declares), module text)` the way the launcher does; returns the mods folder
/// and the approved list file (each thread of a test reads its own copy: the list is the launch's, not the thread's).
fn approved_world(name: &str, packs: &[(&str, &str, String)]) -> (PathBuf, PathBuf) {
    let (control, mods) = world(name);
    let src = scratch(&format!("{name}_src"));
    for (id, side, text) in packs {
        // which modules the package declares decides where it runs: `server` = wasm.server, `client` = wasm.client, `both` = both
        let declared = match *side {
            "client" => "client = \"bin/wasm/mod.wasm\"",
            "both" => "server = \"bin/wasm/mod.wasm\"\nclient = \"bin/wasm/client.wasm\"",
            _ => "server = \"bin/wasm/mod.wasm\"",
        };
        let fa = format!("{declared}\nworld = \"full-access\"");
        let bytes = std::fs::read(pack(&src, id, &fa, text)).unwrap();
        install_package(&control, &mods, &format!("{id}.yak"), &bytes, Origin::Local, None).unwrap();
        control.approve(&bytes).unwrap();
    }
    let file = control.approved_list(&mods).unwrap().write_for_launch(&scratch(&format!("{name}_run"))).unwrap();
    (mods, file)
}

fn loaded_ids(report: &horizon_client::yoake2_mods::LoadReport) -> Vec<String> {
    report.loaded.iter().map(|l| l.id.clone()).collect()
}

fn other_side_count(report: &horizon_client::yoake2_mods::LoadReport) -> usize {
    report.skipped.iter().filter(|(_, r)| *r == Refusal::OtherSide).count()
}

#[test]
fn side_decides_which_thread_loads_a_full_access_package_and_the_other_side_is_not_an_error() {
    let guest = counting_guest(&ALL_THREE, None);
    let (mods, file) = approved_world(
        "sides",
        &[("demo_srv", "server", guest.clone()), ("demo_cli", "client", guest.clone()), ("demo_both", "both", guest.clone()), ("demo_exp", "server", guest)],
    );
    // each side on its own thread: the runtime is per thread
    let (m, f) = (mods.clone(), file.clone());
    let server = std::thread::spawn(move || {
        let report = load_world_mods(&m, Some(&ApprovedList::read(&f).unwrap()));
        let out = (loaded_ids(&report), report.messages(), other_side_count(&report));
        runtime::uninstall().unwrap();
        out
    });
    let client = std::thread::spawn(move || {
        let report = load_client_mods(&mods, Some(&ApprovedList::read(&file).unwrap()));
        let out = (loaded_ids(&report), report.messages(), other_side_count(&report));
        assert_eq!(names().len(), 2);
        assert!(unload_client_mods().is_empty());
        assert!(names().is_empty(), "the session end drops them");
        out
    });
    let (loaded, messages, other) = server.join().unwrap();
    assert_eq!(loaded, ["demo_both", "demo_exp", "demo_srv"], "packages with wasm.server");
    assert!(messages.is_empty(), "{messages:?}");
    assert_eq!(other, 1);
    let (loaded, messages, other) = client.join().unwrap();
    assert_eq!(loaded, ["demo_both", "demo_cli"]);
    assert!(messages.is_empty(), "a package for the other side is silent: {messages:?}");
    assert_eq!(other, 2);
}

#[test]
fn a_client_mod_gets_client_tick_and_hud_render_and_never_the_server_tick() {
    let (mods, file) = approved_world("client_ticks", &[("demo_cli", "client", counting_guest(&ALL_THREE, None))]);
    std::thread::spawn(move || {
        let report = load_client_mods(&mods, Some(&ApprovedList::read(&file).unwrap()));
        assert_eq!(loaded_ids(&report), ["demo_cli"], "{:?} {:?}", report.skipped, report.problems);
        assert!(report.unbridged.is_empty(), "synthetic client hooks need no bridge: {:?}", report.unbridged);
        assert!(report.messages().is_empty(), "{:?}", report.messages());
        // hook_register: server tick Full, client tick Full, HUD render observe-only
        assert_eq!(runtime::take_reports().unwrap(), [(500, 0), (501, 0), (502, 1)]);
        // the engine dispatches the server tick on the server thread only: nothing on this thread does
        for _ in 0..3 {
            dispatch_client_tick(&mut TickRoot);
        }
        dispatch_hud_render();
        dispatch_hud_render();
        assert_eq!(runtime::take_reports().unwrap(), [(2, 1), (2, 2), (2, 3), (3, 4), (3, 5)], "no (1, _) entry: no server tick");
        assert_eq!(runtime::all_stats().len(), 1);
        assert!(unload_client_mods().is_empty());
        dispatch_client_tick(&mut TickRoot);
        assert!(runtime::take_reports().unwrap().is_empty(), "unloaded at the session's end");
    })
    .join()
    .unwrap();
}

#[test]
fn a_server_only_mod_is_not_loaded_on_the_client_and_its_hooks_never_run_there() {
    let (mods, file) = approved_world("server_only", &[("demo_srv", "server", counting_guest(&ALL_THREE, None))]);
    std::thread::spawn(move || {
        let report = load_client_mods(&mods, Some(&ApprovedList::read(&file).unwrap()));
        assert!(report.loaded.is_empty());
        assert!(report.messages().is_empty(), "{:?}", report.messages());
        assert!(names().is_empty());
        dispatch_client_tick(&mut TickRoot);
        dispatch_hud_render();
        assert!(runtime::take_reports().unwrap().is_empty());
        // without a list nothing is loaded either (same trust rules)
        assert!(load_client_mods(&mods, None).loaded.is_empty());
    })
    .join()
    .unwrap();
}

#[test]
fn a_both_mod_has_one_instance_per_thread_and_they_share_no_state() {
    let (mods, file) = approved_world("both", &[("demo_both", "both", counting_guest(&ALL_THREE, None))]);
    let (m, f) = (mods.clone(), file.clone());
    let server = std::thread::spawn(move || {
        let report = load_world_mods(&m, Some(&ApprovedList::read(&f).unwrap()));
        assert_eq!(loaded_ids(&report), ["demo_both"]);
        runtime::take_reports().unwrap();
        runtime::dispatch_root_id(runtime::HOOK_SERVER_TICK, &mut TickRoot, &mut |_| 0);
        let r = runtime::take_reports().unwrap();
        runtime::uninstall().unwrap();
        r
    });
    let client = std::thread::spawn(move || {
        let report = load_client_mods(&mods, Some(&ApprovedList::read(&file).unwrap()));
        assert_eq!(loaded_ids(&report), ["demo_both"]);
        runtime::take_reports().unwrap();
        for _ in 0..3 {
            dispatch_client_tick(&mut TickRoot);
        }
        let r = runtime::take_reports().unwrap();
        unload_client_mods();
        r
    });
    assert_eq!(server.join().unwrap(), [(1, 1)], "the server instance counted one entry, whatever the client did");
    assert_eq!(client.join().unwrap(), [(2, 1), (2, 2), (2, 3)], "the client instance started from zero");
}

#[test]
fn a_trap_on_the_client_thread_disables_the_mod_there_and_the_original_still_runs() {
    let guest = counting_guest(&ALL_THREE, Some(runtime::HOOK_CLIENT_TICK));
    let (mods, file) = approved_world("client_trap", &[("demo_both", "both", guest)]);
    let (m, f) = (mods.clone(), file.clone());
    let client = std::thread::spawn(move || {
        let report = load_client_mods(&m, Some(&ApprovedList::read(&f).unwrap()));
        assert_eq!(loaded_ids(&report), ["demo_both"]);
        let module = report.loaded[0].module;
        let mut original = 0;
        for _ in 0..2 {
            runtime::dispatch_root_id(runtime::HOOK_CLIENT_TICK, &mut TickRoot, &mut |_| {
                original += 1;
                0
            });
        }
        assert_eq!(original, 2, "the engine's own tick ran both times, the trapping one included");
        assert!(runtime::stats_of(module).unwrap().disabled, "disabled on the client thread");
        assert_eq!(runtime::stats_of(module).unwrap().entries, 1, "not entered again after the trap");
        // the dead mod's other hooks are gone too, and nothing panics
        dispatch_client_tick(&mut TickRoot);
        dispatch_hud_render();
        assert_eq!(yoake_slice::activation::live_activations(), 0);
        unload_client_mods();
    });
    // the server instance of the same package is a different instance: the client's trap does not touch it
    let server = std::thread::spawn(move || {
        let report = load_world_mods(&mods, Some(&ApprovedList::read(&file).unwrap()));
        assert_eq!(loaded_ids(&report), ["demo_both"]);
        runtime::take_reports().unwrap();
        runtime::dispatch_root_id(runtime::HOOK_SERVER_TICK, &mut TickRoot, &mut |_| 0);
        assert_eq!(runtime::take_reports().unwrap(), [(1, 1)]);
        assert!(!runtime::stats().disabled);
        runtime::uninstall().unwrap();
    });
    client.join().unwrap();
    server.join().unwrap();
}

#[test]
fn with_no_client_mod_the_client_hooks_are_no_ops_and_the_original_runs() {
    std::thread::spawn(|| {
        for _ in 0..3 {
            dispatch_client_tick(&mut TickRoot);
            dispatch_hud_render();
        }
        let mut original = 0;
        let mut count = |_: &mut TickRoot| {
            original += 1;
            0
        };
        assert_eq!(runtime::dispatch_root_id(runtime::HOOK_CLIENT_TICK, &mut TickRoot, &mut count), 0);
        assert_eq!(original, 1);
        assert!(runtime::all_stats().is_empty());
        assert!(!horizon_client::yoake2_mods::client_mods_loaded());
        assert!(unload_client_mods().is_empty(), "ending a session that never loaded anything is fine");
        // a client load of a folder with nothing in it
        let (_, mods) = world("client_empty");
        assert!(load_client_mods(&mods, None).loaded.is_empty());
        assert!(runtime::all_stats().is_empty());
    })
    .join()
    .unwrap();
}

#[test]
fn a_client_runtime_is_refused_on_a_thread_that_holds_a_server_runtime_and_the_reverse() {
    let (mods, file) = approved_world("shared_thread", &[("demo_both", "both", counting_guest(&ALL_THREE, None))]);
    let (mods2, file2) = (mods.clone(), file.clone());
    std::thread::spawn(move || {
        let list = ApprovedList::read(&file2).unwrap();
        let mods = mods2;
        // server first: the client load on the same thread is refused, and nothing of it is loaded
        let report = load_world_mods(&mods, Some(&list));
        assert_eq!(loaded_ids(&report), ["demo_both"]);
        let refused = load_client_mods(&mods, Some(&list));
        assert!(refused.loaded.is_empty());
        assert!(
            matches!(&refused.skipped[..], [(_, Refusal::SharedThread { loading: ThreadSide::Client, holds: ThreadSide::Server })]),
            "{:?}",
            refused.skipped
        );
        assert!(refused.messages().iter().any(|l| l.contains("share its runtime")), "{:?}", refused.messages());
        assert_eq!(names().len(), 1, "the server instance is untouched");
        assert!(!horizon_client::yoake2_mods::client_mods_loaded());
        runtime::uninstall().unwrap();
    })
    .join()
    .unwrap();
    // and the reverse, on a fresh thread: the server load on a thread holding the client runtime
    std::thread::spawn(move || {
        let list = ApprovedList::read(&file).unwrap();
        let report = load_client_mods(&mods, Some(&list));
        assert_eq!(loaded_ids(&report), ["demo_both"]);
        let refused = load_world_mods(&mods, Some(&list));
        assert!(refused.loaded.is_empty());
        assert!(matches!(&refused.skipped[..], [(_, Refusal::SharedThread { loading: ThreadSide::Server, holds: ThreadSide::Client })]), "{:?}", refused.skipped);
        assert_eq!(names().len(), 1);
        unload_client_mods();
        // once the client mods are gone the server may load again
        assert_eq!(loaded_ids(&load_world_mods(&mods, Some(&list))), ["demo_both"]);
        runtime::uninstall().unwrap();
    })
    .join()
    .unwrap();
}

#[test]
fn the_hud_render_argument_is_a_dead_handle() {
    let hud = runtime::HOOK_HUD_RENDER as i32;
    let wat = format!(
        r#"(module
  (@custom "yoake_abi" "\01\00\00\00")
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "get" (func $get (param i64 i32 i32) (result i32)))
  (import "env" "set" (func $set (param i64 i32 i64) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (drop (call $register (i32.const {hud}) (i32.const 0))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (call $report (i32.const 1) (i64.extend_i32_s (call $get (local.get $h) (i32.const 0) (i32.const 64))))
    (call $report (i32.const 2) (i64.extend_i32_s (call $set (local.get $h) (i32.const 0) (i64.const 1))))
    (i64.const 0)))"#
    );
    let (mods, file) = approved_world("hud_dead", &[("demo_hud", "client", wat)]);
    std::thread::spawn(move || {
        let report = load_client_mods(&mods, Some(&ApprovedList::read(&file).unwrap()));
        assert_eq!(loaded_ids(&report), ["demo_hud"], "{:?}", report.skipped);
        dispatch_hud_render();
        let r = runtime::take_reports().unwrap();
        assert_eq!(r, [(1, 3), (2, 3)], "get and set on the HUD handle fail with status 3 (BadField): there is nothing behind it");
        unload_client_mods();
    })
    .join()
    .unwrap();
}

// ---------------------------------------------------------------------------------------------------------------
// MX3HR1: dev-mode hot reload
// ---------------------------------------------------------------------------------------------------------------

use horizon_client::yoake2_mods::dev_reload_check;
use std::time::{Duration, Instant};

/// A guest that registers `ids` and reports `(tag, id - SYNTHETIC_HOOK_BASE or id)` on each entry: which build of the
/// guest ran. With `trap_in_init` its `yoake_init` executes `unreachable`.
fn tagged_guest(tag: i32, ids: &[u32], trap_in_init: bool) -> String {
    let register: String = ids.iter().map(|id| format!("(drop (call $register (i32.const {}) (i32.const 0)))\n", *id as i32)).collect();
    let trap = if trap_in_init { "unreachable" } else { "" };
    format!(
        r#"(module
  (@custom "yoake_abi" "\01\00\00\00")
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init") {register} {trap})
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (call $report (i32.const {tag}) (i64.extend_i32_u (i32.and (local.get $id) (i32.const 0x7fffffff))))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (i64.const 0)))"#
    )
}

const SERVER_TICK: u32 = runtime::HOOK_SERVER_TICK;
const CLIENT_TICK: u32 = runtime::HOOK_CLIENT_TICK;

/// A launch the way the launcher keeps it in dev mode: a control folder, the mods folder and the one approved-list file
/// that is rewritten (temp file + rename) with a rising generation.
struct DevLaunch {
    control: Control,
    mods: PathBuf,
    list: PathBuf,
    src: PathBuf,
    t0: Instant,
}

impl DevLaunch {
    fn new(name: &str) -> Self {
        let (control, mods) = world(name);
        let run = scratch(&format!("{name}_run"));
        Self { control, mods, list: run.join("approved.toml"), src: scratch(&format!("{name}_src")), t0: Instant::now() }
    }

    /// Installs (replacing any earlier file of that id) and approves one package: `side` is "server" | "client" | "both".
    fn publish(&self, id: &str, side: &str, text: &str) {
        let declared = match side {
            "client" => "client = \"bin/wasm/mod.wasm\"",
            "both" => "server = \"bin/wasm/mod.wasm\"\nclient = \"bin/wasm/client.wasm\"",
            _ => "server = \"bin/wasm/mod.wasm\"",
        };
        let packed = pack(&self.src, id, &format!("{declared}\nworld = \"full-access\""), text);
        let bytes = std::fs::read(&packed).unwrap();
        let _ = std::fs::remove_file(self.mods.join(format!("{id}.yak")));
        install_package(&self.control, &self.mods, &format!("{id}.yak"), &bytes, Origin::Local, None).unwrap();
        self.control.approve(&bytes).unwrap();
    }

    /// The launcher's rewrite: the list of what is in the mods folder now, `generation`, a temp file renamed over the list.
    fn rewrite(&self, generation: u64) {
        let list = self.control.approved_list(&self.mods).unwrap().with_generation(generation);
        std::fs::create_dir_all(self.list.parent().unwrap()).unwrap();
        let tmp = self.list.with_extension("tmp");
        std::fs::write(&tmp, list.to_text()).unwrap();
        std::fs::rename(&tmp, &self.list).unwrap();
        // a file system's mtime is coarse: two rewrites must not share one stamp (the launcher's are seconds apart)
        std::thread::sleep(Duration::from_millis(40));
    }

    fn read(&self) -> ApprovedList {
        ApprovedList::read(&self.list).unwrap()
    }

    fn at(&self, secs: u64) -> Instant {
        self.t0 + Duration::from_secs(secs)
    }

    fn check(&self, enabled: bool, side: ThreadSide, secs: u64) -> Option<horizon_client::yoake2_mods::DevReload> {
        dev_reload_check(enabled, side, &self.list, &self.mods, self.at(secs))
    }
}

fn server_tick() -> Vec<(i32, i64)> {
    runtime::dispatch_root_id(SERVER_TICK, &mut TickRoot, &mut |_| 0);
    runtime::take_reports().unwrap()
}

const TICK: i64 = 1;

#[test]
fn the_generation_round_trips_and_a_file_without_one_is_generation_zero() {
    let (control, mods) = world("hr_generation");
    let bytes = std::fs::read(full_access(&scratch("hr_generation_src"), "demo_gen", 0, 1)).unwrap();
    install_package(&control, &mods, "demo_gen.yak", &bytes, Origin::Local, None).unwrap();
    control.approve(&bytes).unwrap();
    let list = control.approved_list(&mods).unwrap();
    assert_eq!(list.generation(), 0);
    let old_style = list.write_for_launch(&scratch("hr_generation_run")).unwrap();
    assert!(!std::fs::read_to_string(&old_style).unwrap().contains("generation ="), "generation 0 is not written: old readers and files stay as they were");
    assert_eq!(ApprovedList::read(&old_style).unwrap().generation(), 0);
    let raised = list.with_generation(41);
    let text = raised.to_text();
    assert!(text.contains("generation = 41"));
    assert_eq!(ApprovedList::parse(&text).unwrap().generation(), 41);
    assert_eq!(ApprovedList::parse(&text).unwrap(), raised);
}

#[test]
fn without_the_dev_flag_a_changed_list_is_never_looked_at() {
    runtime::uninstall().unwrap();
    let dev = DevLaunch::new("hr_no_flag");
    dev.publish("demo_hr", "server", &tagged_guest(10, &[SERVER_TICK], false));
    dev.rewrite(1);
    assert_eq!(loaded_ids(&load_world_mods(&dev.mods, Some(&dev.read()))), ["demo_hr"]);
    runtime::take_reports().unwrap();

    dev.publish("demo_hr", "server", &tagged_guest(20, &[SERVER_TICK], false));
    dev.rewrite(2);
    for secs in [2, 4, 6] {
        assert!(dev.check(false, ThreadSide::Server, secs).is_none(), "dev flag off");
    }
    assert_eq!(server_tick(), [(10, TICK)], "still the old guest");
    // the flag on: the same file now reloads
    assert!(dev.check(true, ThreadSide::Server, 8).is_some());
    assert_eq!(server_tick(), [(20, TICK)]);
    runtime::uninstall().unwrap();
}

#[test]
fn a_higher_generation_reloads_the_changed_guest_and_the_old_one_no_longer_runs() {
    runtime::uninstall().unwrap();
    let dev = DevLaunch::new("hr_reload");
    dev.publish("demo_hr", "server", &tagged_guest(10, &[SERVER_TICK], false));
    dev.rewrite(1);
    load_world_mods(&dev.mods, Some(&dev.read()));
    runtime::take_reports().unwrap();
    assert_eq!(server_tick(), [(10, TICK)]);
    let before = runtime::all_stats();

    dev.publish("demo_hr", "server", &tagged_guest(20, &[SERVER_TICK], false));
    dev.rewrite(2);
    let reload = dev.check(true, ThreadSide::Server, 10).expect("generation 2 > 1");
    assert_eq!(reload.generation, 2);
    assert_eq!(reload.report.loaded.len(), 1, "{:?} {:?}", reload.report.skipped, reload.report.problems);
    assert!(reload.report.messages().is_empty(), "{:?}", reload.report.messages());
    runtime::take_reports().unwrap();
    assert_eq!(server_tick(), [(20, TICK)], "the chain points at the new instance; the old guest is gone");
    assert_eq!(runtime::all_stats().len(), before.len(), "one mod, not two");
    runtime::uninstall().unwrap();
}

#[test]
fn the_same_generation_does_not_reload_even_when_the_file_changed() {
    runtime::uninstall().unwrap();
    let dev = DevLaunch::new("hr_same_generation");
    dev.publish("demo_hr", "server", &tagged_guest(10, &[SERVER_TICK], false));
    dev.rewrite(5);
    load_world_mods(&dev.mods, Some(&dev.read()));
    runtime::take_reports().unwrap();
    dev.publish("demo_hr", "server", &tagged_guest(20, &[SERVER_TICK], false));
    dev.rewrite(5);
    assert!(dev.check(true, ThreadSide::Server, 2).is_none(), "5 is not above 5");
    dev.rewrite(4);
    assert!(dev.check(true, ThreadSide::Server, 4).is_none(), "a lower generation is ignored too");
    assert_eq!(server_tick(), [(10, TICK)]);
    dev.rewrite(6);
    assert!(dev.check(true, ThreadSide::Server, 6).is_some());
    assert_eq!(server_tick(), [(20, TICK)]);
    runtime::uninstall().unwrap();
}

#[test]
fn the_file_is_looked_at_once_per_second_and_only_when_it_changed() {
    runtime::uninstall().unwrap();
    let dev = DevLaunch::new("hr_throttle");
    dev.publish("demo_hr", "server", &tagged_guest(10, &[SERVER_TICK], false));
    dev.rewrite(1);
    load_world_mods(&dev.mods, Some(&dev.read()));
    assert!(dev.check(true, ThreadSide::Server, 10).is_none(), "generation 1 is what is loaded");
    dev.publish("demo_hr", "server", &tagged_guest(20, &[SERVER_TICK], false));
    dev.rewrite(2);
    // 10.0 s was the last look; the file changes, but 10.5 s is too soon
    let soon = dev_reload_check(true, ThreadSide::Server, &dev.list, &dev.mods, dev.at(10) + Duration::from_millis(500));
    assert!(soon.is_none(), "at most once per second");
    assert!(dev.check(true, ThreadSide::Server, 11).is_some());
    assert!(dev.check(true, ThreadSide::Server, 13).is_none(), "unchanged file: nothing to do");
    runtime::uninstall().unwrap();
}

#[test]
fn a_broken_list_keeps_the_old_mods_and_a_later_good_one_still_reloads() {
    runtime::uninstall().unwrap();
    let dev = DevLaunch::new("hr_broken");
    dev.publish("demo_hr", "server", &tagged_guest(10, &[SERVER_TICK], false));
    dev.rewrite(1);
    load_world_mods(&dev.mods, Some(&dev.read()));
    runtime::take_reports().unwrap();

    for (n, broken) in ["format = 1\ngeneration = 9\n[[package]\n", "", "format = 7\ngeneration = 9\n", "format = 1\ngeneration = \"nine\"\n"].into_iter().enumerate() {
        std::fs::write(&dev.list, broken).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        assert!(dev.check(true, ThreadSide::Server, 2 + 2 * n as u64).is_none(), "{broken:?}");
        assert_eq!(server_tick(), [(10, TICK)], "the old mods keep running after {broken:?}");
    }
    dev.publish("demo_hr", "server", &tagged_guest(20, &[SERVER_TICK], false));
    dev.rewrite(2);
    assert!(dev.check(true, ThreadSide::Server, 20).is_some());
    assert_eq!(server_tick(), [(20, TICK)]);
    runtime::uninstall().unwrap();
}

#[test]
fn a_mod_that_traps_in_init_or_is_not_approved_is_reported_and_the_others_still_load() {
    runtime::uninstall().unwrap();
    let dev = DevLaunch::new("hr_partial");
    dev.publish("demo_a", "server", &tagged_guest(10, &[SERVER_TICK], false));
    dev.publish("demo_b", "server", &tagged_guest(11, &[SERVER_TICK], false));
    dev.publish("demo_c", "server", &tagged_guest(12, &[SERVER_TICK], false));
    dev.rewrite(1);
    load_world_mods(&dev.mods, Some(&dev.read()));
    runtime::take_reports().unwrap();
    assert_eq!(server_tick().len(), 3);

    dev.publish("demo_a", "server", &tagged_guest(30, &[SERVER_TICK], false));
    dev.publish("demo_b", "server", &tagged_guest(31, &[SERVER_TICK], true)); // traps in yoake_init
    // demo_c is replaced by a file nobody approved: the rewrite leaves it out of the list
    let unapproved = std::fs::read(pack(&dev.src, "demo_c", "server = \"bin/wasm/mod.wasm\"\nworld = \"full-access\"", &tagged_guest(32, &[SERVER_TICK], false))).unwrap();
    std::fs::remove_file(dev.mods.join("demo_c.yak")).unwrap();
    install_package(&dev.control, &dev.mods, "demo_c.yak", &unapproved, Origin::Local, None).unwrap();
    dev.rewrite(2);

    let reload = dev.check(true, ThreadSide::Server, 5).expect("reloaded");
    assert_eq!(loaded_ids(&reload.report), ["demo_a"]);
    let messages = reload.report.messages().join("\n");
    assert!(messages.contains("demo_b.yak") && messages.contains("could not start"), "{messages}");
    assert!(messages.contains("demo_c.yak"), "{messages}");
    runtime::take_reports().unwrap();
    assert_eq!(server_tick(), [(30, TICK)], "only the mod that loaded runs");
    runtime::uninstall().unwrap();
}

#[test]
fn a_reload_is_refused_while_a_guest_is_running_and_happens_at_the_next_check() {
    runtime::uninstall().unwrap();
    assert!(!runtime::guest_running());
    let dev = DevLaunch::new("hr_mid_activation");
    dev.publish("demo_hr", "server", &module(0, 1));
    dev.rewrite(1);
    load_world_mods(&dev.mods, Some(&dev.read()));
    dev.publish("demo_hr", "server", &module(0, 5));
    dev.rewrite(2);

    let mut thing = Thing { count: 0 };
    let mut inside = None;
    let mut running = false;
    runtime::dispatch_id(1, &mut thing, &TABLE, &mut |t| {
        // this is `original`: the guest's frame is on the stack
        running = runtime::guest_running();
        inside = Some(dev.check(true, ThreadSide::Server, 3).is_some());
        t.count += 300;
        0
    });
    assert!(running, "the activation is visible");
    assert_eq!(inside, Some(false), "refused mid-activation");
    assert_eq!(thing.count, 1 + 300, "the first guest finished its activation undisturbed");
    assert!(!runtime::guest_running());
    // between ticks the same change is still pending
    assert!(dev.check(true, ThreadSide::Server, 5).is_some());
    let mut thing = Thing { count: 0 };
    engine(&mut thing);
    assert_eq!(thing.count, 5 + 300, "the new guest");
    runtime::uninstall().unwrap();
}

#[test]
fn an_id_registered_for_the_first_time_after_a_reload_is_bridged_once() {
    use horizon_client::entities::{ArmorStand, StandMedium, StandStep};
    use horizon_core::geometry::Vec3;

    runtime::uninstall().unwrap();
    let dev = DevLaunch::new("hr_bridge");
    dev.publish("demo_hr", "server", &tagged_guest(10, &[SERVER_TICK], false));
    dev.rewrite(1);
    let first = load_world_mods(&dev.mods, Some(&dev.read()));
    assert!(first.unbridged.is_empty());

    let step = StandStep::for_hook_bench(true, 0.6);
    let stand = || ArmorStand::new(1, Vec3::new(3.0, 64.0, 5.0), 30.0);
    let mut baseline = stand();
    baseline.tick_settle(StandMedium::Air, step);
    runtime::take_reports().unwrap();

    // generation 2 registers hook 71 (an armor stand's tick), which no mod of this thread had before
    dev.publish("demo_hr", "server", &tagged_guest(20, &[SERVER_TICK, 71], false));
    dev.rewrite(2);
    let reload = dev.check(true, ThreadSide::Server, 3).expect("reloaded");
    assert!(reload.report.unbridged.is_empty(), "{:?}", reload.report.unbridged);
    assert!(yoake_slice::bridge::is_installed(71), "bridged by the reload (or earlier by another test: install_once)");
    for round in 0..2 {
        let mut hooked = stand();
        hooked.tick_settle(StandMedium::Air, step);
        assert_eq!(hooked.velocity, baseline.velocity, "round {round}: the original ran once");
        assert_eq!(runtime::take_reports().unwrap(), [(20, 71)], "round {round}: the new guest, entered once");
    }
    // generation 3: the id is already bridged; no second dispatcher, so still one entry per call
    dev.publish("demo_hr", "server", &tagged_guest(30, &[SERVER_TICK, 71], false));
    dev.rewrite(3);
    assert!(dev.check(true, ThreadSide::Server, 6).is_some());
    let mut hooked = stand();
    hooked.tick_settle(StandMedium::Air, step);
    assert_eq!(runtime::take_reports().unwrap(), [(30, 71)], "a reload never installs a dispatcher twice");
    runtime::uninstall().unwrap();
}

#[test]
fn the_client_and_the_server_runtimes_reload_independently() {
    use std::sync::mpsc::channel;
    enum Cmd {
        Tick,
        Check(u64),
    }
    let dev = std::sync::Arc::new(DevLaunch::new("hr_two_sides"));
    dev.publish("demo_both", "both", &tagged_guest(10, &[SERVER_TICK, CLIENT_TICK], false));
    dev.rewrite(1);

    // each side on its own thread: load, then answer commands: Tick (what runs) or Check (did it reload)
    let spawn_side = |side: ThreadSide| {
        let (ready_tx, ready_rx) = channel::<()>();
        let (cmd_tx, cmd_rx) = channel::<Cmd>();
        let (out_tx, out_rx) = channel::<(bool, Vec<(i32, i64)>)>();
        let dev = dev.clone();
        let handle = std::thread::spawn(move || {
            let list = dev.read();
            let report = match side {
                ThreadSide::Server => load_world_mods(&dev.mods, Some(&list)),
                ThreadSide::Client => load_client_mods(&dev.mods, Some(&list)),
            };
            assert_eq!(loaded_ids(&report), ["demo_both"]);
            runtime::take_reports().unwrap();
            ready_tx.send(()).unwrap();
            while let Ok(cmd) = cmd_rx.recv() {
                let answer = match cmd {
                    Cmd::Check(secs) => (dev.check(true, side, secs).is_some(), Vec::new()),
                    Cmd::Tick => {
                        match side {
                            ThreadSide::Server => drop(runtime::dispatch_root_id(SERVER_TICK, &mut TickRoot, &mut |_| 0)),
                            ThreadSide::Client => dispatch_client_tick(&mut TickRoot),
                        };
                        (false, runtime::take_reports().unwrap())
                    }
                };
                out_tx.send(answer).unwrap();
            }
            match side {
                ThreadSide::Server => runtime::uninstall().unwrap(),
                ThreadSide::Client => assert!(unload_client_mods().is_empty()),
            }
        });
        ready_rx.recv().unwrap();
        let ask = move |cmd: Cmd| {
            cmd_tx.send(cmd).unwrap();
            out_rx.recv().unwrap()
        };
        (ask, handle)
    };
    let (server, server_thread) = spawn_side(ThreadSide::Server);
    let (client, client_thread) = spawn_side(ThreadSide::Client);

    dev.publish("demo_both", "both", &tagged_guest(20, &[SERVER_TICK, CLIENT_TICK], false));
    dev.rewrite(2);

    // only the server looks: it reloads, and the client still runs its old instance
    assert!(server(Cmd::Check(5)).0);
    assert_eq!(server(Cmd::Tick).1, [(20, 1)]);
    assert_eq!(client(Cmd::Tick).1, [(10, 2)], "the client runtime was not touched by the server's reload");
    // now the client looks (its own throttle and stamp): it reloads on its own
    assert!(client(Cmd::Check(5)).0);
    assert_eq!(client(Cmd::Tick).1, [(20, 2)]);
    // a second look changes nothing on either side
    assert!(!server(Cmd::Check(8)).0);
    assert!(!client(Cmd::Check(8)).0);
    drop((server, client));
    server_thread.join().unwrap();
    client_thread.join().unwrap();
}
