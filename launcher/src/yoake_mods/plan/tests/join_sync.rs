//! MX3JN2: join-time mod sync in the launcher. The game's exit code 75 and its `join-request.json` are turned into
//! installed, host-scoped, approved mods; later launches of the same host get them, other launches never do.
use super::fixtures::Package;
use super::*;
use crate::full_access::{self as fa, Rejoin, DevSync};
use horizon_mods::yak::control::{self, ApprovedList, Origin, APPROVED_ARG};
use serde_json::json;
use std::process::Command;
use std::time::{Duration, Instant};

const SERVER: &str = "server:play.example.com:25565";
const SERVER_DIR: &str = "server+play.example.com+25565";
const OTHER: &str = "server:other.example.net:25565";

fn data(fixture: &Fixture) -> PathBuf {
    fixture.client.data.clone()
}

fn remote(fixture: &Fixture) -> PathBuf {
    fixture.home.join("mods").join("remote")
}

fn staging(fixture: &Fixture) -> PathBuf {
    remote(fixture).join(".staging")
}

fn sha(package: &Package) -> String {
    horizon_release_sign::sha256_hex(&package.bytes)
}

/// What the game leaves behind: the staged `<sha>.yak` files and the request that names them.
struct Staged {
    mods: Vec<serde_json::Value>,
}

impl Staged {
    fn new(fixture: &Fixture) -> Self {
        std::fs::create_dir_all(staging(fixture)).unwrap();
        Self { mods: Vec::new() }
    }

    /// A mod as the game would stage it: the file under its SHA-256, the entry in the request.
    fn add(&mut self, fixture: &Fixture, id: &str, version: &str, full_access: bool) -> Package {
        let manifest = if full_access { fixture.full_access(id, version) } else { fixture.manifest(id, version) };
        let package = fixture.package(manifest, false);
        let hash = sha(&package);
        std::fs::write(staging(fixture).join(format!("{hash}.yak")), &package.bytes).unwrap();
        self.mods.push(json!({
            "id": id, "version": version, "sha256": hash, "size": package.bytes.len(),
            "full_access": full_access, "file": format!("{hash}.yak"),
        }));
        package
    }

    fn request(&self, host: serde_json::Value, rejoin: &str) -> serde_json::Value {
        json!({ "host": host, "mods": self.mods, "rejoin": rejoin })
    }

    fn write(&self, fixture: &Fixture, request: &serde_json::Value) {
        std::fs::write(staging(fixture).join("join-request.json"), serde_json::to_string_pretty(request).unwrap()).unwrap();
    }

    fn write_server(&self, fixture: &Fixture) {
        self.write(fixture, &self.request(json!({"kind": "server", "address": "play.example.com:25565", "key": SERVER}), "play.example.com:25565"));
    }
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
    names.sort();
    names
}

fn args(prepared: &fa::Prepared) -> Vec<String> {
    prepared.args().iter().map(|a| a.to_string_lossy().into_owned()).collect()
}

fn list_of(prepared: &fa::Prepared) -> ApprovedList {
    ApprovedList::read(prepared.run.as_ref().expect("a list was written").approved_path()).unwrap()
}

/// A player's own accepted, approved full-access mod (it must ride along in a host launch).
fn own_mod(fixture: &Fixture) -> String {
    let package = fixture.package(fixture.full_access("my-own", "1.0.0"), false);
    fixture.apply(fixture.local(&package.path).unwrap(), true).unwrap();
    fa::approve_in(&data(fixture), &fixture.home, "my-own", &sha(&package)).unwrap();
    sha(&package)
}

fn nothing_installed(fixture: &Fixture) {
    let leftovers: Vec<String> = names(&remote(fixture)).into_iter().filter(|n| n != ".staging").collect();
    assert!(leftovers.is_empty(), "nothing may be installed: {leftovers:?}");
    assert!(fa::remote_hosts_in(&data(fixture), &fixture.home).is_empty());
    assert!(names(&staging(fixture)).is_empty(), "the staging folder is emptied: {:?}", names(&staging(fixture)));
    let status = fa::last_join_sync_for(&fixture.home).expect("the failure is surfaced");
    assert_eq!(status.installed, 0);
    assert!(status.error.is_some());
}

#[test]
fn a_valid_request_installs_approves_for_the_host_and_builds_the_relaunch() {
    let fixture = Fixture::new();
    let own = own_mod(&fixture);
    let mut staged = Staged::new(&fixture);
    let full = staged.add(&fixture, "host-fa", "2.0.0", true);
    let plain = staged.add(&fixture, "host-plain", "1.0.0", false);
    staged.write_server(&fixture);

    let done = fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();
    assert_eq!((done.host_key.as_str(), done.installed), (SERVER, 2));
    assert_eq!(done.rejoin, Rejoin::Server("play.example.com:25565".into()));
    assert_eq!(done.rejoin.quick_play(), Some(("--server", "play.example.com:25565".to_owned())));
    // the files are in the host's folder under their hashes, byte for byte; the staging folder is empty
    let dir = remote(&fixture).join(SERVER_DIR);
    assert_eq!(std::fs::read(dir.join(format!("{}.yak", sha(&full)))).unwrap(), full.bytes);
    assert_eq!(std::fs::read(dir.join(format!("{}.yak", sha(&plain)))).unwrap(), plain.bytes);
    assert!(names(&staging(&fixture)).is_empty());
    let status = fa::last_join_sync_for(&fixture.home).unwrap();
    assert!(fa::last_join_sync().is_some());
    assert_eq!((status.host_display.as_str(), status.installed, status.error), ("play.example.com:25565", 2, None));
    // the player's own control files know nothing of them: the host's mods are no approval of the player's
    let ctl = control::Control::create(&control::control_dir_in(&data(&fixture)), &fixture.home).unwrap();
    assert!(ctl.origins().unwrap().origin_of(&sha(&full)).is_none());
    assert!(!ctl.allow().unwrap().is_allowed("host-fa", &sha(&full)));
    // no top-level copy either
    assert_eq!(fa::list_in(&data(&fixture), &fixture.home).len(), 1, "only the player's own mod is a top-level full-access mod");

    // the relaunch: the same flags the later launches get
    let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, Some(&done.host_key), true);
    assert!(prepared.problem.is_none(), "{:?}", prepared.problem);
    let args = args(&prepared);
    assert_eq!(args.iter().filter(|a| *a == "--join-mod-sync").count(), 1, "{args:?}");
    let at = args.iter().position(|a| a == "--remote-mods").expect("--remote-mods");
    assert_eq!(args[at + 1], SERVER);
    let at = args.iter().position(|a| a == APPROVED_ARG).expect("the list");
    let list = ApprovedList::read(Path::new(&args[at + 1])).unwrap();
    assert_eq!(list.host(), Some(SERVER));
    // the player's own accepted entry plus the host's full-access mod; the host's sandboxed mod is not in the list
    let mut seen: Vec<(String, Origin)> = list.entries().iter().map(|e| (e.id.clone(), e.origin)).collect();
    seen.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(seen, vec![("host-fa".to_owned(), Origin::Server), ("my-own".to_owned(), Origin::Local)]);
    assert!(list.entries().iter().any(|e| e.sha256 == own));
    let (allow, origins) = list.to_remote_files(SERVER);
    assert!(allow.is_allowed("host-fa", &sha(&full)) && origins.origin_of(&sha(&full)) == Some(Origin::Server));
    assert!(!list.to_files().0.is_allowed("host-fa", &sha(&full)), "the plain mods folder never sees the host's entry");

    // a friend host: the same, with origin friend
    let mut staged = Staged::new(&fixture);
    let theirs = staged.add(&fixture, "pal-fa", "1.0.0", true);
    staged.write(&fixture, &staged.request(json!({"kind": "friend", "address": "user_7", "key": "friend:user_7"}), "friend:user_7"));
    let done = fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();
    assert_eq!(done.rejoin, Rejoin::Friend("user_7".into()));
    assert_eq!(done.rejoin.quick_play(), None);
    let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, Some("friend:user_7"), true);
    let list = list_of(&prepared);
    assert_eq!(list.host(), Some("friend:user_7"));
    assert!(list.entries().iter().any(|e| e.sha256 == sha(&theirs) && e.origin == Origin::Friend));
    assert!(!list.entries().iter().any(|e| e.id == "host-fa"), "another host's mod is not in this host's list");
}

#[test]
fn a_changed_file_of_the_host_is_not_passed_and_a_newer_version_replaces_the_old_one() {
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    let one = staged.add(&fixture, "host-fa", "1.0.0", true);
    staged.write_server(&fixture);
    fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();
    let file = remote(&fixture).join(SERVER_DIR).join(format!("{}.yak", sha(&one)));
    // bytes changed behind the launcher's back: not in the list any more
    std::fs::write(&file, b"changed").unwrap();
    let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, Some(SERVER), true);
    assert!(list_of(&prepared).entries().is_empty());
    std::fs::write(&file, &one.bytes).unwrap();
    // the host updates the mod: the new version replaces the old file and record
    let mut staged = Staged::new(&fixture);
    let two = staged.add(&fixture, "host-fa", "1.1.0", true);
    staged.write_server(&fixture);
    fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();
    assert_eq!(names(&remote(&fixture).join(SERVER_DIR)), vec![format!("{}.yak", sha(&two))]);
    let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, Some(SERVER), true);
    let list = list_of(&prepared);
    assert_eq!(list.entries().len(), 1);
    assert_eq!(list.entries()[0].sha256, sha(&two));
}

#[test]
fn a_bad_request_installs_nothing_and_clears_the_staging_folder() {
    let server = || json!({"kind": "server", "address": "play.example.com:25565", "key": SERVER});

    // a SHA-256 that does not match the staged bytes (the first mod is fine: still nothing is installed)
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "good", "1.0.0", true);
    let other = fixture.package(fixture.full_access("evil", "1.0.0"), false);
    let lie = "ab".repeat(32);
    std::fs::write(staging(&fixture).join(format!("{lie}.yak")), &other.bytes).unwrap();
    staged.mods.push(json!({"id": "evil", "version": "1.0.0", "sha256": lie, "size": other.bytes.len(), "full_access": true, "file": format!("{lie}.yak")}));
    staged.write_server(&fixture);
    let error = fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap_err();
    assert!(error.contains("SHA-256"), "{error}");
    nothing_installed(&fixture);

    // a path traversal in the file name, in either separator, and a name that is not the hash
    for file in ["../../../evil.yak", "..\\..\\evil.yak", "sub/dir.yak", "evil.yak", "/etc/passwd"] {
        let fixture = Fixture::new();
        let mut staged = Staged::new(&fixture);
        let package = staged.add(&fixture, "trav", "1.0.0", true);
        staged.mods[0]["file"] = json!(file);
        // a file at the traversed place would be installed if the name were followed
        std::fs::write(fixture.home.join("evil.yak"), &package.bytes).unwrap();
        staged.write_server(&fixture);
        let error = fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap_err();
        assert!(error.contains("<sha256>.yak"), "{file}: {error}");
        nothing_installed(&fixture);
    }

    // too big: announced over 64 MiB; announced size not the staged size; more than 64 mods
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "big", "1.0.0", true);
    staged.mods[0]["size"] = json!(64 * 1024 * 1024 + 1);
    staged.write_server(&fixture);
    assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap_err().contains("size"));
    nothing_installed(&fixture);
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "short", "1.0.0", true);
    let announced = staged.mods[0]["size"].as_u64().unwrap();
    staged.mods[0]["size"] = json!(announced - 1);
    staged.write_server(&fixture);
    assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap_err().contains("bytes"));
    nothing_installed(&fixture);
    let fixture = Fixture::new();
    let staged = Staged::new(&fixture);
    let many: Vec<_> = (0..65).map(|n| json!({"id": format!("m{n}"), "version": "1.0.0", "sha256": format!("{n:064x}"), "size": 10, "full_access": false, "file": format!("{n:064x}.yak")})).collect();
    staged.write(&fixture, &json!({"host": server(), "mods": many, "rejoin": "play.example.com:25565"}));
    assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).is_err());
    nothing_installed(&fixture);

    // bad host keys (and a rejoin that disagrees with the host)
    for (host, rejoin) in [
        (json!({"kind": "server", "address": "x", "key": "server:../etc"}), "x"),
        (json!({"kind": "server", "address": "play.example.com:25565", "key": "friend:play.example.com:25565"}), "play.example.com:25565"),
        (json!({"kind": "server", "address": "a b", "key": "server:a b"}), "a b"),
        (json!({"kind": "friend", "address": "user_7", "key": "friend:user_8"}), "friend:user_7"),
        (json!({"kind": "server", "address": "play.example.com:25565", "key": SERVER}), "other.example.net:25565"),
        (json!({"kind": "lan", "address": "x", "key": "lan:x"}), "x"),
    ] {
        let fixture = Fixture::new();
        let mut staged = Staged::new(&fixture);
        staged.add(&fixture, "key-mod", "1.0.0", true);
        staged.write(&fixture, &staged.request(host.clone(), rejoin));
        assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).is_err(), "{host}");
        nothing_installed(&fixture);
    }

    // a request that is not JSON, one that is missing, and a package that is not what the request says
    let fixture = Fixture::new();
    Staged::new(&fixture);
    std::fs::write(staging(&fixture).join("join-request.json"), "not json").unwrap();
    assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).is_err());
    nothing_installed(&fixture);
    let fixture = Fixture::new();
    Staged::new(&fixture);
    assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).is_err());
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "claims-plain", "1.0.0", true);
    staged.mods[0]["full_access"] = json!(false);
    staged.write_server(&fixture);
    assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap_err().contains("full_access"));
    nothing_installed(&fixture);
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "real-name", "1.0.0", true);
    staged.mods[0]["id"] = json!("other-name");
    staged.write_server(&fixture);
    assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).is_err());
    nothing_installed(&fixture);
    // bytes that are no package at all
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    let junk = b"this is not a yak".to_vec();
    let hash = horizon_release_sign::sha256_hex(&junk);
    std::fs::write(staging(&fixture).join(format!("{hash}.yak")), &junk).unwrap();
    staged.mods.push(json!({"id": "junk", "version": "1.0.0", "sha256": hash, "size": junk.len(), "full_access": true, "file": format!("{hash}.yak")}));
    staged.write_server(&fixture);
    assert!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).is_err());
    nothing_installed(&fixture);
}

#[test]
fn only_the_host_that_was_joined_gets_the_remote_mods() {
    let fixture = Fixture::new();
    own_mod(&fixture);
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "host-fa", "1.0.0", true);
    staged.write_server(&fixture);
    fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();

    // another server, a friend, and no host at all: neither the flag nor the scope nor the entries
    for host in [Some(OTHER), Some("friend:user_7"), Some("server:play.example.com:25566"), None] {
        let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, host, true);
        let args = args(&prepared);
        assert!(!args.iter().any(|a| a == "--remote-mods"), "{host:?}: {args:?}");
        assert_eq!(prepared.remote_host, None);
        let list = list_of(&prepared);
        assert_eq!(list.host(), None, "{host:?}");
        assert!(list.entries().iter().all(|e| e.origin.can_be_full_access()), "{host:?}");
        assert!(list.entries().iter().all(|e| e.id != "host-fa"));
    }
    // the plain prepare of the game (single player, a world) is the same
    let prepared = fa::prepare_in(&data(&fixture), &fixture.home);
    assert!(!args(&prepared).iter().any(|a| a == "--remote-mods" || a == "--join-mod-sync"));
    assert_eq!(list_of(&prepared).host(), None);
    // an invalid key is no host
    assert_eq!(fa::prepare_for(&data(&fixture), &fixture.home, None, Some("server:../x"), true).remote_host, None);
    // a host that was joined but has no remote folder: nothing remote (the setting still shows)
    let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, Some(OTHER), true);
    assert!(args(&prepared).iter().any(|a| a == "--join-mod-sync"));
}

#[test]
fn the_setting_is_on_by_default_and_off_means_no_sync_flag() {
    assert!(crate::backend::Settings::default().join_mod_sync);
    assert!(crate::backend::Settings::parse("{}").join_mod_sync, "a file from before the setting");
    assert!(crate::backend::Settings::parse(r#"{"join_mod_sync":true}"#).join_mod_sync);
    let off = crate::backend::Settings::parse(r#"{"join_mod_sync":false}"#);
    assert!(!off.join_mod_sync);
    // serialized like the others: it round-trips
    let text = serde_json::to_string(&off).unwrap();
    assert!(text.contains("\"join_mod_sync\":false"));
    assert!(!crate::backend::Settings::parse(&text).join_mod_sync);
    assert!(crate::backend::Settings::parse(&serde_json::to_string(&crate::backend::Settings::default()).unwrap()).join_mod_sync);

    let fixture = Fixture::new();
    let on = fa::prepare_for(&data(&fixture), &fixture.home, None, None, true);
    assert_eq!(args(&on).iter().filter(|a| *a == "--join-mod-sync").count(), 1);
    let off = fa::prepare_for(&data(&fixture), &fixture.home, None, None, off_setting(&fixture));
    assert!(!args(&off).iter().any(|a| a == "--join-mod-sync"), "{:?}", args(&off));
}

fn off_setting(_: &Fixture) -> bool {
    crate::backend::Settings::parse(r#"{"join_mod_sync":false}"#).join_mod_sync
}

#[test]
fn remote_hosts_lists_the_mods_by_host_and_remove_deletes_everything_of_one_host() {
    let fixture = Fixture::new();
    own_mod(&fixture);
    let mut staged = Staged::new(&fixture);
    let full = staged.add(&fixture, "host-fa", "1.0.0", true);
    let plain = staged.add(&fixture, "host-plain", "0.5.0", false);
    staged.write_server(&fixture);
    fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();
    let mut staged = Staged::new(&fixture);
    let pal = staged.add(&fixture, "pal-fa", "3.0.0", true);
    staged.write(&fixture, &staged.request(json!({"kind": "friend", "address": "user_7", "key": "friend:user_7"}), "friend:user_7"));
    fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();

    let hosts = fa::remote_hosts_in(&data(&fixture), &fixture.home);
    assert_eq!(hosts.iter().map(|h| h.key.as_str()).collect::<Vec<_>>(), vec!["friend:user_7", SERVER]);
    assert_eq!(hosts[0].display, "user_7");
    assert_eq!(hosts[1].display, "play.example.com:25565");
    assert_eq!(
        hosts[1].mods,
        vec![
            fa::RemoteMod { id: "host-fa".into(), version: "1.0.0".into(), sha256_short: sha(&full)[..12].into(), full_access: true },
            fa::RemoteMod { id: "host-plain".into(), version: "0.5.0".into(), sha256_short: sha(&plain)[..12].into(), full_access: false },
        ]
    );
    assert_eq!(hosts[0].mods.len(), 1);
    assert_eq!(hosts[0].mods[0].sha256_short, sha(&pal)[..12]);
    // the staging folder and the old per-origin folders are not hosts
    std::fs::create_dir_all(remote(&fixture).join("server")).unwrap();
    assert_eq!(fa::remote_hosts_in(&data(&fixture), &fixture.home).len(), 2);

    // remove: folder, record, approval; the other host and the player's own mod stay
    fa::remove_remote_host_in(&data(&fixture), &fixture.home, SERVER).unwrap();
    assert!(!remote(&fixture).join(SERVER_DIR).exists());
    let hosts = fa::remote_hosts_in(&data(&fixture), &fixture.home);
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].key, "friend:user_7");
    let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, Some(SERVER), true);
    assert_eq!(prepared.remote_host, None, "no folder, no flag");
    assert!(list_of(&prepared).entries().iter().all(|e| e.id != "host-fa"));
    assert_eq!(fa::list_in(&data(&fixture), &fixture.home).len(), 1, "the player's own mod is untouched");
    // a record left over is gone too: the same hash arriving again is a fresh install, nothing is passed from the old one
    let record_dir = control::control_dir_in(&data(&fixture)).join("remote-hosts");
    let records: Vec<_> = walk(&record_dir).into_iter().filter(|p| p.to_string_lossy().contains(SERVER_DIR)).collect();
    assert!(records.is_empty(), "{records:?}");
    // errors: twice, a bad key, a key with a folder part
    assert!(fa::remove_remote_host_in(&data(&fixture), &fixture.home, SERVER).is_err());
    assert!(fa::remove_remote_host_in(&data(&fixture), &fixture.home, "server:../..").is_err());
    assert!(fa::remove_remote_host_in(&data(&fixture), &fixture.home, "nope").is_err());
    assert!(remote(&fixture).join("friend+user_7").exists());
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

#[test]
fn a_dev_rewrite_keeps_the_host_scope_of_the_run() {
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    let theirs = staged.add(&fixture, "host-fa", "1.0.0", true);
    staged.write_server(&fixture);
    fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();
    let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, Some(SERVER), false);
    let approved = prepared.run.as_ref().unwrap().approved_path().to_path_buf();
    let folder = fixture.home.parent().unwrap().join("dev-folder");
    std::fs::create_dir_all(&folder).unwrap();
    let mut sync = DevSync::new(&data(&fixture), &fixture.home, &folder, &approved);
    let mine = fixture.package(fixture.full_access("dev-mod", "1.0.0"), false);
    std::fs::write(folder.join("dev-mod.yak"), &mine.bytes).unwrap();
    let mut now = Instant::now();
    sync.tick(now);
    now += Duration::from_millis(700);
    sync.tick(now);
    let list = ApprovedList::read(&approved).unwrap();
    assert_eq!(list.generation(), 1);
    assert_eq!(list.host(), Some(SERVER), "the host scope survives the rewrite");
    assert!(list.entries().iter().any(|e| e.id == "dev-mod" && e.origin == Origin::Dev));
    assert!(list.entries().iter().any(|e| e.sha256 == sha(&theirs) && e.origin == Origin::Server));
}

#[test]
fn request_parsing_host_keys_and_the_exit_text() {
    assert_eq!(fa::server_host_key("Play.Example.COM"), Some("server:play.example.com:25565".into()));
    assert_eq!(fa::server_host_key(" play.example.com:25570 "), Some("server:play.example.com:25570".into()));
    assert_eq!(fa::server_host_key("[::1]"), Some("server:[::1]:25565".into()));
    assert_eq!(fa::server_host_key("[::1]:1234"), Some("server:[::1]:1234".into()));
    for none in ["", "local", "social", "replay", "a:b:c", "host:notaport", "bad host", "host:99999", "[::1]x"] {
        assert_eq!(fa::server_host_key(none), None, "{none:?}");
    }
    assert_eq!(fa::friend_host_key("user_7"), Some("friend:user_7".into()));
    assert_eq!(fa::friend_host_key("a/b"), None);
    assert_eq!(fa::friend_host_key(""), None);

    assert!(fa::is_join_exit_text("Exited with exit code: 75"));
    assert!(fa::is_join_exit_text("Exited with exit status: 75"));
    for no in ["Exited with exit code: 1", "Exited with exit code: 175", "Exited with exit code: 7", "75", "exit code: 75"] {
        assert!(!fa::is_join_exit_text(no), "{no}");
    }
    assert_eq!(fa::JOIN_EXIT_CODE, 75);

    // the shape the game writes (MX3JN1) parses; catalog_id is carried but not used
    let hash = "a".repeat(64);
    let request = json!({
        "host": {"kind": "server", "address": "play.example.com:25565", "key": SERVER},
        "mods": [{"id": "x", "version": "1.0.0", "sha256": hash, "size": 5, "full_access": false, "file": format!("{hash}.yak"), "catalog_id": "cat-x"}],
        "rejoin": "play.example.com:25565"
    });
    let parsed = fa::parse_join_request(&request.to_string()).unwrap();
    assert_eq!(parsed.mods[0].catalog_id.as_deref(), Some("cat-x"));
    // uppercase hash, duplicate id, zero size, no mods
    for mutate in [
        |r: &mut serde_json::Value| r["mods"][0]["sha256"] = json!("A".repeat(64)),
        |r: &mut serde_json::Value| r["mods"][0]["size"] = json!(0),
        |r: &mut serde_json::Value| r["mods"] = json!([]),
        |r: &mut serde_json::Value| r["mods"][0]["id"] = json!("bad\nid"),
        |r: &mut serde_json::Value| r["mods"][0]["full_access"] = json!("yes"),
    ] {
        let mut broken = request.clone();
        mutate(&mut broken);
        assert!(fa::parse_join_request(&broken.to_string()).is_err(), "{broken}");
    }
    let mut twice = request.clone();
    let again = twice["mods"][0].clone();
    twice["mods"].as_array_mut().unwrap().push(again);
    assert!(fa::parse_join_request(&twice.to_string()).is_err());
}

// ---------------------------------------------------------------------------------------------------------------
// MX3RF1: the review's fixes
// ---------------------------------------------------------------------------------------------------------------

/// M2: fills the host's folder with fake `.yak` files (never opened by the installer's cap check) and returns its path.
/// The first file has `first_bytes` bytes (sparse), the others one byte.
fn fill_host_folder(fixture: &Fixture, count: usize, first_bytes: u64) -> PathBuf {
    let dir = remote(fixture).join(SERVER_DIR);
    std::fs::create_dir_all(&dir).unwrap();
    for n in 0..count {
        let name = format!("{:064x}.yak", n + 1);
        let file = std::fs::File::create(dir.join(name)).unwrap();
        file.set_len(if n == 0 { first_bytes } else { 1 }).unwrap();
    }
    dir
}

#[test]
fn m2_a_host_folder_is_capped_at_64_mods() {
    assert_eq!((fa::MAX_HOST_MODS, fa::MAX_HOST_BYTES), (64, 512 * 1024 * 1024));
    let fixture = Fixture::new();
    let dir = fill_host_folder(&fixture, fa::MAX_HOST_MODS, 1);
    let before = names(&dir);
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "one-too-many", "1.0.0", false);
    staged.write_server(&fixture);
    let error = fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap_err();
    assert!(error.contains("nothing was installed") && error.contains("64"), "{error}");
    assert_eq!(names(&dir), before, "nothing from the request was installed");
    assert!(names(&staging(&fixture)).is_empty());
    assert!(fa::last_join_sync_for(&fixture.home).unwrap().error.is_some());
    // one place left: the same request fits
    std::fs::remove_file(dir.join(&before[0])).unwrap();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "fits", "1.0.0", false);
    staged.write_server(&fixture);
    assert_eq!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap().installed, 1);
}

#[test]
fn m2_a_host_folder_is_capped_at_512_mib() {
    let fixture = Fixture::new();
    // a (sparse) file that leaves less room than the request needs
    let dir = fill_host_folder(&fixture, 1, fa::MAX_HOST_BYTES - 16);
    let before = names(&dir);
    let mut staged = Staged::new(&fixture);
    let package = staged.add(&fixture, "too-big-together", "1.0.0", false);
    assert!(package.bytes.len() > 16);
    staged.write_server(&fixture);
    let error = fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap_err();
    assert!(error.contains("nothing was installed") && error.contains("512"), "{error}");
    assert_eq!(names(&dir), before);
    // the same bytes already in the folder cost nothing more
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "again", "1.0.0", false);
    staged.write_server(&fixture);
    fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "again", "1.0.0", false);
    staged.write_server(&fixture);
    assert_eq!(fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap().installed, 1);
}

#[test]
fn m2_a_game_the_launcher_restarted_that_asks_again_installs_nothing() {
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "loop-mod", "1.0.0", true);
    staged.write_server(&fixture);
    let error = fa::refuse_repeated_join_exit(&fixture.home).unwrap_err();
    assert!(error.contains("play.example.com:25565") && error.contains("asked for mods again"), "{error}");
    nothing_installed(&fixture);
    let status = fa::last_join_sync_for(&fixture.home).unwrap();
    assert_eq!(status.host_display, "play.example.com:25565");
    assert!(status.error.unwrap().contains("asked for mods again"));
    // without any request it still refuses and says so
    assert!(fa::refuse_repeated_join_exit(&fixture.home).is_err());
}

#[test]
fn s1_an_old_game_gets_none_of_the_new_arguments() {
    let fixture = Fixture::new();
    let mut staged = Staged::new(&fixture);
    staged.add(&fixture, "host-fa", "2.0.0", true);
    staged.write_server(&fixture);
    let done = fa::handle_join_exit_in(&data(&fixture), &fixture.home).unwrap();
    let prepared = fa::prepare_for(&data(&fixture), &fixture.home, None, Some(&done.host_key), true);
    assert!(prepared.run.is_some() && prepared.join_mod_sync && prepared.remote_host.is_some());
    let strings = |support: fa::GameSupport| prepared.args_for(support).iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>();
    // a 1.0 game: no `--launch-api` options at all
    assert!(strings(fa::GameSupport::NONE).is_empty());
    assert_eq!(fa::GameSupport::from_reply("{\"launch_api\":\"0.1.0\",\"options\":[{\"name\":\"--width\",\"value\":\"N\"},{\"name\":\"--server\",\"value\":\"X\"}]}"), fa::GameSupport::NONE);
    assert_eq!(fa::GameSupport::from_reply("not json"), fa::GameSupport::NONE);
    let mut cmd = Command::new("game");
    prepared.apply_for(&mut cmd, fa::GameSupport::NONE);
    assert_eq!(cmd.get_args().count(), 0);
    // a new game lists all four
    let reply = "{\"launch_api\":\"0.1.0\",\"options\":[{\"name\":\"--full-access-approved\",\"value\":\"FILE\"},{\"name\":\"--full-access-dev\",\"value\":null},{\"name\":\"--join-mod-sync\",\"value\":null},{\"name\":\"--remote-mods\",\"value\":\"KEY\"}]}";
    let support = fa::GameSupport::from_reply(reply);
    assert_eq!(support, fa::GameSupport::ALL);
    let all = strings(support);
    for flag in ["--full-access-approved", "--join-mod-sync", "--remote-mods"] {
        assert!(all.iter().any(|a| a == flag), "{flag} in {all:?}");
    }
    // a game that knows only some of them gets only those
    let partial = fa::GameSupport::from_reply("{\"options\":[{\"name\":\"--full-access-approved\",\"value\":\"FILE\"}]}");
    let some = strings(partial);
    assert_eq!(some.len(), 2, "{some:?}");
    assert!(!some.iter().any(|a| a == "--join-mod-sync" || a == "--remote-mods"));
    // a missing executable cannot be asked: no new arguments
    assert_eq!(fa::GameSupport::detect(Path::new("no-such-game.exe")), fa::GameSupport::NONE);
}

fn sample_mod() -> serde_json::Value {
    let hash = "a".repeat(64);
    json!({"id": "x", "version": "1.0.0", "sha256": hash, "size": 10, "full_access": false, "file": format!("{hash}.yak")})
}

#[test]
fn s2_a_host_key_is_lowercase_and_must_be_the_one_computed_from_the_address() {
    for bad in ["server:Play.Example.com:25565", "friend:User_7"] {
        assert!(!control::valid_host_key(bad), "{bad}");
    }
    assert_eq!(fa::friend_host_key("User_7"), Some("friend:user_7".into()));
    let ok = |host: serde_json::Value, rejoin: &str| {
        let staged = Staged { mods: vec![sample_mod()] };
        fa::parse_join_request(&staged.request(host, rejoin).to_string())
    };
    // the canonical forms pass; a friend's address keeps its spelling, its key is the lowercase of it
    assert!(ok(json!({"kind": "server", "address": "play.example.com:25565", "key": SERVER}), "play.example.com:25565").is_ok());
    assert!(ok(json!({"kind": "friend", "address": "User_7", "key": "friend:user_7"}), "friend:user_7").is_ok());
    // a key with capitals, a host that spells its own key, an address that is not the canonical `host:port`
    assert!(ok(json!({"kind": "server", "address": "Play.Example.com:25565", "key": "server:Play.Example.com:25565"}), "Play.Example.com:25565").is_err());
    assert!(ok(json!({"kind": "server", "address": "Play.Example.com:25565", "key": SERVER}), "Play.Example.com:25565").is_err());
    assert!(ok(json!({"kind": "server", "address": "play.example.com", "key": SERVER}), "play.example.com").is_err(), "the port is part of the address");
    assert!(ok(json!({"kind": "friend", "address": "User_7", "key": "friend:User_7"}), "friend:User_7").is_err());
    assert!(ok(json!({"kind": "friend", "address": "user_7", "key": "friend:user_8"}), "friend:user_7").is_err());
}
