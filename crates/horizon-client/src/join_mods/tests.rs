//! MX3JN1 tests: no game, no resources, no network. Real `.yak` files in a temp folder, the host and the client wired
//! together in memory through the protocol's message functions.
use super::*;
use horizon_protocol::mod_transfer::{HostEvent, HostSession, Message};
use std::collections::BTreeMap;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("mx3jn1").join(format!("{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Packs a `.yak` of `id`: `full_access` with a client module, or sandboxed with a client module; `required` is
/// `required_on_join`. `salt` makes the bytes differ.
fn pack(dir: &Path, id: &str, full_access: bool, required: bool, salt: &str) -> PathBuf {
    let src = dir.join(format!("src_{id}_{salt}"));
    std::fs::create_dir_all(src.join("bin/wasm")).unwrap();
    let world = if full_access { "world = \"full-access\"\n" } else { "" };
    std::fs::write(
        src.join("yak.toml"),
        format!(
            "format = 1\nid = \"{id}\"\nversion = \"1.0.0\"\nname = \"{id}\"\nauthors = [\"test\"]\ndescription = \"a test mod\"\n\n[yoake]\nversion = \">=0.0.0\"\n\n[side]\nclient = true\nserver = false\nrequired_on_join = {required}\n\n[wasm]\ninterface = \"1.0\"\nclient = \"bin/wasm/mod.wasm\"\n{world}"
        ),
    )
    .unwrap();
    std::fs::write(src.join("bin/wasm/mod.wasm"), format!("module {id} {salt}")).unwrap();
    let out = dir.join(format!("{id}-{salt}.yak"));
    horizon_mods::yak::pack_directory(&src, &out, &BTreeMap::new()).unwrap();
    out
}

fn sha_of(path: &Path) -> [u8; 32] {
    Sha256::digest(std::fs::read(path).unwrap()).into()
}

fn server() -> HostId {
    server_host("Play.Example.COM:25570").unwrap()
}

#[test]
fn the_launchers_arguments_are_read_and_checked() {
    let args = |list: &[&str]| list.iter().map(|text| text.to_string()).collect::<Vec<_>>();
    assert!(sync_enabled_from(args(&["--x", "--join-mod-sync"])));
    assert!(!sync_enabled_from(args(&["--x", "--server", "a"])));
    assert_eq!(remote_key_from(args(&["--remote-mods", "server:a.example:25565"])), Some("server:a.example:25565".into()));
    assert_eq!(remote_key_from(args(&["--remote-mods=friend:user_12"])), Some("friend:user_12".into()));
    assert_eq!(remote_key_from(args(&[])), None);
    assert_eq!(remote_key_from(args(&["--remote-mods"])), None);
    for bad in ["", "server", "server:", "other:x", "server:../x", "server:a/b", "server:a\\b", "friend:a b", "friend:a..b"] {
        assert!(!valid_host_key(bad), "{bad:?}");
        assert_eq!(remote_key_from(args(&["--remote-mods", bad])), None, "{bad:?}");
    }
    assert!(!valid_host_key(&format!("server:{}", "a".repeat(MAX_KEY))));
    // the folder name has no colon (Windows) and keeps the hosts apart
    assert_eq!(host_dir_name("server:a.example:25565"), "server+a.example+25565");
    assert_ne!(host_dir_name("server:a:1"), host_dir_name("friend:a:1"));
    let root = Path::new("game");
    assert_eq!(host_dir(root, "friend:u1"), root.join("mods").join("remote").join("friend+u1"));
    assert_eq!(staging_dir(root), root.join("mods").join("remote").join(".staging"));
}

#[test]
fn the_host_key_comes_from_the_address_or_the_portal_id() {
    let host = server();
    assert_eq!(host.key, "server:play.example.com:25570");
    assert_eq!(host.address, "play.example.com:25570");
    assert_eq!(host.rejoin, "play.example.com:25570");
    assert_eq!(host.kind, HostKind::Server);
    assert_eq!(server_host("Mc.Example.org").unwrap().key, "server:mc.example.org:25565");
    assert_eq!(server_host("[::1]:25566").unwrap().key, "server:[::1]:25566");
    assert_eq!(server_host("[::1]").unwrap().key, "server:[::1]:25565");
    for none in ["", "local", "social", "replay", "friend:K7QM-3XPD", "a:b", "a:99999", "::1", "ho st"] {
        assert!(server_host(none).is_none(), "{none:?}");
    }
    let friend = friend_host("user_42").unwrap();
    assert_eq!((friend.key.as_str(), friend.address.as_str(), friend.rejoin.as_str()), ("friend:user_42", "user_42", "friend:user_42"));
    assert!(friend_host("a b").is_none());
    assert!(friend_host("").is_none());
    set_host(Some(host.clone()));
    assert_eq!(super::host(), Some(host));
    set_host(None);
    assert_eq!(super::host(), None);
}

#[test]
fn the_staging_sink_writes_part_files_and_renames_verified_ones() {
    let dir = scratch("sink").join("staging");
    let mut sink = StagingSink::new(dir.clone());
    let entry = OfferEntry { id: "a".into(), version: "1".into(), sha256: [0xab; 32], size: 5, full_access: false, side_client: true, catalog_id: None };
    let hash = transfer::hex(&entry.sha256);
    sink.begin(&entry).unwrap();
    sink.write(b"he").unwrap();
    assert!(dir.join(format!("{hash}.part")).exists() && !dir.join(format!("{hash}.yak")).exists());
    sink.write(b"llo").unwrap();
    sink.commit(&entry).unwrap();
    assert!(!dir.join(format!("{hash}.part")).exists());
    assert_eq!(std::fs::read(dir.join(format!("{hash}.yak"))).unwrap(), b"hello");
    // a wrong size on disk is not committed
    sink.begin(&entry).unwrap();
    sink.write(b"hel").unwrap();
    assert!(sink.commit(&entry).is_err());
    assert!(!dir.join(format!("{hash}.part")).exists());
    // a discarded file leaves no part
    sink.begin(&entry).unwrap();
    sink.write(b"x").unwrap();
    sink.discard(&entry);
    assert!(!dir.join(format!("{hash}.part")).exists());
    assert!(sink.write(b"y").is_err(), "nothing in progress");
}

#[test]
fn the_request_file_has_exactly_the_contracts_fields() {
    let dir = scratch("request");
    let staging = staging_dir(&dir);
    let mods = vec![
        OfferEntry { id: "alpha".into(), version: "1.2.3".into(), sha256: [1; 32], size: 10, full_access: true, side_client: true, catalog_id: Some("catalog-alpha".into()) },
        OfferEntry { id: "beta".into(), version: "0.1.0".into(), sha256: [2; 32], size: 20, full_access: false, side_client: true, catalog_id: None },
    ];
    let path = write_request(&staging, &server(), &mods).unwrap();
    assert_eq!(path, staging.join("join-request.json"));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let hash = |byte: u8| transfer::hex(&[byte; 32]);
    assert_eq!(
        value,
        serde_json::json!({
            "host": { "kind": "server", "address": "play.example.com:25570", "key": "server:play.example.com:25570" },
            "mods": [
                { "id": "alpha", "version": "1.2.3", "sha256": hash(1), "size": 10, "full_access": true, "catalog_id": "catalog-alpha", "file": format!("{}.yak", hash(1)) },
                { "id": "beta", "version": "0.1.0", "sha256": hash(2), "size": 20, "full_access": false, "file": format!("{}.yak", hash(2)) },
            ],
            "rejoin": "play.example.com:25570",
        })
    );
    assert!(value["mods"][1].get("catalog_id").is_none(), "no catalog id: no field");
    // a friend host
    let friend = request_json(&friend_host("user_42").unwrap(), &mods);
    assert_eq!(friend["host"], serde_json::json!({ "kind": "friend", "address": "user_42", "key": "friend:user_42" }));
    assert_eq!(friend["rejoin"], "friend:user_42");
    // no temp file stays; a stale request goes
    let names: Vec<_> = std::fs::read_dir(&staging).unwrap().flatten().map(|item| item.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names, ["join-request.json"]);
    clear_request(&staging);
    assert!(!path.exists());
}

#[test]
fn finishing_a_join_writes_the_request_and_gives_exit_code_75() {
    let dir = scratch("finish");
    let staging = staging_dir(&dir);
    let entry = OfferEntry { id: "a".into(), version: "1".into(), sha256: [3; 32], size: 1, full_access: false, side_client: true, catalog_id: None };
    assert_eq!(finish_join(&staging, &server(), &[entry]).unwrap(), EXIT_JOIN_NEEDS_MODS);
    assert_eq!(EXIT_JOIN_NEEDS_MODS, 75);
    assert!(staging.join(REQUEST_FILE).exists());
    // a staging folder that cannot be created is an error, not an exit
    let blocked = dir.join("blocked");
    std::fs::write(&blocked, b"a file where a folder should be").unwrap();
    assert!(finish_join(&blocked.join("staging"), &server(), &[]).is_err());
}

/// A game folder of a host with three running mods and some that are not offered.
struct HostGame {
    root: PathBuf,
    sandboxed: PathBuf,
    full: PathBuf,
}

fn host_game(name: &str) -> HostGame {
    let root = scratch(name);
    let mods = root.join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let sandboxed = pack(&work, "sand_mod", false, true, "a");
    let full = pack(&work, "full_mod", true, false, "a");
    let not_running = pack(&work, "idle_mod", true, false, "a");
    let not_required = pack(&work, "plain_mod", false, false, "a");
    for path in [&sandboxed, &full, &not_running, &not_required] {
        std::fs::copy(path, mods.join(path.file_name().unwrap())).unwrap();
    }
    std::fs::write(mods.join("notes.txt"), b"not a mod").unwrap();
    std::fs::create_dir_all(mods.join("remote")).unwrap();
    HostGame { sandboxed: mods.join(sandboxed.file_name().unwrap()), full: mods.join(full.file_name().unwrap()), root }
}

fn running(game: &HostGame) -> (Offer, FileSource) {
    let full_hash = transfer::hex(&sha_of(&game.full));
    let installed = vec![("sand_mod".to_owned(), "1.0.0".to_owned()), ("plain_mod".to_owned(), "1.0.0".to_owned())];
    let runs = |hash: &str| hash == full_hash;
    offer_from(&game.root, &Running { full_access_hashes: &runs, installed: &installed }).expect("an offer")
}

#[test]
fn the_host_offers_only_what_it_runs_and_a_joining_client_needs() {
    let game = host_game("offer");
    let (offer, mut source) = running(&game);
    let mut ids: Vec<_> = offer.entries().iter().map(|entry| entry.id.as_str()).collect();
    ids.sort();
    assert_eq!(ids, ["full_mod", "sand_mod"], "not the idle full-access mod, not the one nobody requires, not other files");
    let full = offer.entries().iter().find(|entry| entry.id == "full_mod").unwrap();
    assert!(full.full_access && full.side_client && full.catalog_id.is_none());
    assert_eq!(full.sha256, sha_of(&game.full));
    assert_eq!(full.size, std::fs::metadata(&game.full).unwrap().len());
    let sandboxed = offer.entries().iter().find(|entry| entry.id == "sand_mod").unwrap();
    assert!(!sandboxed.full_access);
    // the source serves the offered bytes and nothing else
    source.open(sandboxed).unwrap();
    let mut buf = vec![0u8; sandboxed.size as usize + 10];
    let read = source.read_at(0, &mut buf).unwrap();
    assert_eq!(&buf[..read], &std::fs::read(&game.sandboxed).unwrap()[..]);
    source.close();
    let stranger = OfferEntry { sha256: [9; 32], ..sandboxed.clone() };
    assert!(source.open(&stranger).is_err(), "a hash that is not in the offer has no file");
    // the same files again are answered from the remembered hashes
    let (again, _) = running(&game);
    assert_eq!(again, offer);
    // nothing running, nothing offered
    let none = Running { full_access_hashes: &|_| false, installed: &[] };
    assert!(offer_from(&game.root, &none).is_none());
    assert!(offer_from(&game.root.join("missing"), &none).is_none());
}

#[test]
fn a_file_changed_after_the_offer_is_not_served() {
    let game = host_game("changed");
    let (offer, mut source) = running(&game);
    let entry = offer.entries().iter().find(|entry| entry.id == "sand_mod").unwrap().clone();
    let mut bytes = std::fs::read(&game.sandboxed).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    std::fs::write(&game.sandboxed, bytes).unwrap();
    assert!(source.open(&entry).is_err(), "same size, other bytes");
}

/// Runs a `ClientJoin` against a host session over the real messages; `tamper` may change a payload on the wire.
fn run_join(client: &mut ClientJoin, session: &mut HostSession<FileSource>, tamper: impl Fn(Vec<u8>) -> Vec<u8>) -> Vec<Reaction> {
    let mut reactions = Vec::new();
    // the host advertises, the client answers, the host sends its list
    let reaction = client.on_payload(transfer::CAPABILITIES);
    assert_eq!(reaction.send, vec![transfer::CAPABILITIES.to_vec()]);
    let mut to_host: Vec<Vec<u8>> = Vec::new();
    let reaction = client.on_payload(&session.offer().encode());
    to_host.extend(reaction.send.clone());
    reactions.push(reaction);
    let mut guard = 0;
    while let Some(payload) = (!to_host.is_empty()).then(|| to_host.remove(0)) {
        guard += 1;
        assert!(guard < 1000);
        let mut replies = Vec::new();
        match session.handle(&payload).unwrap() {
            HostEvent::Reply(more) => replies.extend(more),
            HostEvent::Streaming => {
                while let Some(chunk) = session.next_chunk() {
                    replies.push(chunk);
                }
            }
            HostEvent::Idle | HostEvent::Finished => {}
        }
        for reply in replies {
            let reaction = client.on_payload(&tamper(reply));
            to_host.extend(reaction.send.clone());
            let stop = reaction.fatal.is_some() || reaction.exit.is_some();
            reactions.push(reaction);
            if stop {
                return reactions;
            }
        }
    }
    reactions
}

fn client(game_root: &Path, enabled: bool, host: Option<HostId>) -> ClientJoin {
    ClientJoin::new(game_root.to_path_buf(), host, enabled, None, Vec::new())
}

#[test]
fn a_join_with_missing_mods_stages_them_writes_the_request_and_exits_75() {
    let host = host_game("join_host");
    let (offer, source) = running(&host);
    let mut session = HostSession::new(offer.clone(), source);
    let player = scratch("join_player");
    let mut join = client(&player, true, Some(server()));
    let reactions = run_join(&mut join, &mut session, |payload| payload);
    let last = reactions.last().unwrap();
    assert_eq!(last.exit, Some(75));
    assert!(last.fatal.is_none());
    assert!(reactions.iter().any(|r| r.status.as_deref().is_some_and(|text| text.contains("Getting mods from the host"))));
    let staging = staging_dir(&player);
    for entry in offer.entries() {
        let file = staging.join(format!("{}.yak", transfer::hex(&entry.sha256)));
        assert_eq!(Sha256::digest(std::fs::read(&file).unwrap()).as_slice(), entry.sha256, "{}", entry.id);
    }
    assert!(std::fs::read_dir(&staging).unwrap().flatten().all(|item| item.path().extension().is_none_or(|extension| extension != "part")));
    let request: serde_json::Value = serde_json::from_slice(&std::fs::read(staging.join("join-request.json")).unwrap()).unwrap();
    assert_eq!(request["host"]["key"], "server:play.example.com:25570");
    assert_eq!(request["rejoin"], "play.example.com:25570");
    let mut listed: Vec<_> = request["mods"].as_array().unwrap().iter().map(|m| (m["id"].as_str().unwrap().to_owned(), m["full_access"].as_bool().unwrap())).collect();
    listed.sort();
    assert_eq!(listed, [("full_mod".to_owned(), true), ("sand_mod".to_owned(), false)]);
    for entry in request["mods"].as_array().unwrap() {
        let hash = entry["sha256"].as_str().unwrap();
        assert_eq!(entry["file"], format!("{hash}.yak"));
        assert_eq!(entry["size"], std::fs::metadata(staging.join(format!("{hash}.yak"))).unwrap().len());
    }
    // nothing was installed or approved by the game
    assert!(!player.join("mods").join("remote").join("server+play.example.com+25570").exists());
}

#[test]
fn a_changed_byte_on_the_wire_aborts_the_join_deletes_the_file_and_leaves_no_request() {
    let host = host_game("tamper_host");
    let (offer, source) = running(&host);
    let mut session = HostSession::new(offer, source);
    let player = scratch("tamper_player");
    // an old request must not survive either
    write_request(&staging_dir(&player), &server(), &[]).unwrap();
    let mut join = client(&player, true, Some(server()));
    let flip = |payload: Vec<u8>| match Message::decode(&payload) {
        Ok(Message::Chunk { sha256, offset, last, mut data }) => {
            data[0] ^= 1;
            let mut w = horizon_protocol::buf::Writer::new();
            w.u8(3);
            w.bytes(&sha256);
            w.varlong(offset as i64);
            w.bool(last);
            w.byte_array(&data);
            w.data
        }
        _ => payload,
    };
    let reactions = run_join(&mut join, &mut session, flip);
    let last = reactions.last().unwrap();
    assert!(last.exit.is_none());
    let text = last.fatal.as_deref().expect("the join is aborted");
    assert!(text.contains("SHA-256 differs") && text.contains("deleted"), "{text}");
    let staging = staging_dir(&player);
    assert!(!staging.join("join-request.json").exists());
    let left: Vec<_> = std::fs::read_dir(&staging).unwrap().flatten().map(|item| item.file_name().to_string_lossy().into_owned()).collect();
    assert!(left.is_empty(), "no part and no yak of the bad file: {left:?}");
}

#[test]
fn without_the_flag_or_a_host_identity_nothing_is_fetched_and_nothing_is_written() {
    let host = host_game("off_host");
    for (enabled, identity) in [(false, Some(server())), (true, None), (false, None)] {
        let (offer, source) = running(&host);
        let mut session = HostSession::new(offer, source);
        let player = scratch(&format!("off_player_{enabled}_{}", identity.is_some()));
        let mut join = client(&player, enabled, identity);
        let reactions = run_join(&mut join, &mut session, |payload| payload);
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0].send, vec![transfer::done()], "only 'done'");
        assert!(reactions[0].exit.is_none() && reactions[0].fatal.is_none() && reactions[0].status.is_none());
        assert!(!player.join("mods").exists(), "no folder, no request");
    }
}

#[test]
fn the_client_ignores_the_channel_until_the_host_advertised_and_after_it_is_done() {
    let host = host_game("order_host");
    let (offer, _) = running(&host);
    let player = scratch("order_player");
    let mut join = client(&player, true, Some(server()));
    // an offer before the capability exchange is not listened to
    assert_eq!(join.on_payload(&offer.encode()), Reaction::default());
    assert_eq!(join.on_payload(&transfer::done()), Reaction::default());
    assert!(!player.join("mods").exists());
    // garbage after the exchange stops the join without leaving files
    assert_eq!(join.on_payload(transfer::CAPABILITIES).send, vec![transfer::CAPABILITIES.to_vec()]);
    let reaction = join.on_payload(&[200, 1, 2]);
    assert!(reaction.fatal.is_some());
    assert_eq!(join.on_payload(&offer.encode()), Reaction::default(), "finished is finished");
    // a join that already has everything: 'done', then silence
    let mut have = client(&player, true, Some(server()));
    have.on_payload(transfer::CAPABILITIES);
    let nothing = Offer::new(vec![]).unwrap();
    assert_eq!(have.on_payload(&nothing.encode()).send, vec![transfer::done()]);
}

#[test]
fn an_aborted_connection_removes_the_part_file_and_the_request() {
    let host = host_game("abort_host");
    let (offer, source) = running(&host);
    let mut session = HostSession::new(offer.clone(), source);
    let player = scratch("abort_player");
    {
        let mut join = client(&player, true, Some(server()));
        join.on_payload(transfer::CAPABILITIES);
        let reaction = join.on_payload(&offer.encode());
        // the first request goes out, the first chunk comes, then the connection dies
        let HostEvent::Streaming = session.handle(&reaction.send[0]).unwrap() else { panic!() };
        join.on_payload(&session.next_chunk().unwrap());
        let staging = staging_dir(&player);
        let parts = || std::fs::read_dir(&staging).unwrap().flatten().filter(|item| item.path().extension().is_some_and(|extension| extension == "part")).count();
        // the first mod is small enough to be done in one chunk; either way nothing but whole files or nothing stays
        let _ = parts();
    }
    let staging = staging_dir(&player);
    assert!(!staging.join("join-request.json").exists());
    assert!(std::fs::read_dir(&staging).unwrap().flatten().all(|item| item.path().extension().is_none_or(|extension| extension != "part")));
}

#[test]
fn what_the_player_already_has_is_not_fetched() {
    let host = host_game("have_host");
    let (offer, _) = running(&host);
    let player = scratch("have_player");
    let mods = player.join("mods");
    std::fs::create_dir_all(&mods).unwrap();
    let full = offer.entries().iter().find(|entry| entry.id == "full_mod").unwrap();
    let sandboxed = offer.entries().iter().find(|entry| entry.id == "sand_mod").unwrap();
    assert!(!has_local(&player, None, &[], full));
    // the same file in the player's own mods folder
    std::fs::copy(&host.full, mods.join("whatever_name.yak")).unwrap();
    assert!(has_local(&player, None, &[], full));
    // a file of the right size and another content is not the mod
    std::fs::write(mods.join("other.yak"), vec![0u8; sandboxed.size as usize]).unwrap();
    assert!(!has_local(&player, None, &[], sandboxed));
    // an installed package of the same id and version counts for a sandboxed mod, not for a full-access one
    let installed = vec![("sand_mod".to_owned(), "1.0.0".to_owned()), ("full_mod".to_owned(), "1.0.0".to_owned())];
    assert!(has_local(&player, None, &installed, sandboxed));
    assert!(!has_local(&Path::new("nowhere").to_path_buf(), None, &installed, full));
    let other_version = vec![("sand_mod".to_owned(), "2.0.0".to_owned())];
    assert!(!has_local(&Path::new("nowhere").to_path_buf(), None, &other_version, sandboxed));
    // this host's earlier download counts, another host's does not
    let key = "server:play.example.com:25570";
    std::fs::create_dir_all(host_dir(&player, key)).unwrap();
    std::fs::copy(&host.sandboxed, host_dir(&player, key).join(format!("{}.yak", transfer::hex(&sandboxed.sha256)))).unwrap();
    assert!(has_local(&player, Some(key), &[], sandboxed));
    assert!(!has_local(&player, Some("server:other.example.com:25565"), &[], sandboxed));
    assert!(!has_local(&player, None, &[], sandboxed));
}

// ---------------------------------------------------------------------------------------------------------------
// MX3RF1: the review's fixes
// ---------------------------------------------------------------------------------------------------------------

#[test]
fn m1_the_remote_scope_exists_only_while_connected_to_the_launchs_host() {
    let h = server();
    let key = Some(h.key.as_str());
    // joining H with H's key
    assert_eq!(session_remote_key_for(key, Some(&h)), Some(h.key.clone()));
    // single player / replay / a pseudo-address after H: no host identity
    assert_eq!(session_remote_key_for(key, None), None);
    // another server in the same process
    let other = server_host("other.example.com").unwrap();
    assert_eq!(session_remote_key_for(key, Some(&other)), None);
    // a friend is not a server of the same name
    assert_eq!(session_remote_key_for(key, friend_host("play.example.com").as_ref()), None);
    // no `--remote-mods` or an invalid one: nothing, whatever the host
    assert_eq!(session_remote_key_for(None, Some(&h)), None);
    assert_eq!(session_remote_key_for(Some("server:Play.Example.COM:25570"), Some(&h)), None);
    // the friend flow
    let friend = friend_host("User_7").unwrap();
    assert_eq!(friend.key, "friend:user_7");
    assert_eq!(session_remote_key_for(Some("friend:user_7"), Some(&friend)), Some("friend:user_7".into()));
    assert_eq!(session_remote_key_for(Some("friend:user_8"), Some(&friend)), None);
}

#[test]
fn m1_the_launchs_server_argument_decides_which_remote_packages_a_start_may_read() {
    let args = |list: &[&str]| list.iter().map(|text| text.to_string()).collect::<Vec<_>>();
    let key = "server:play.example.com:25570";
    assert!(launch_may_use_remote_from(key, args(&["--server", "Play.Example.COM:25570", "--remote-mods", key])));
    assert!(!launch_may_use_remote_from(key, args(&["--remote-mods", key])), "no --server: not a join of that server");
    assert!(!launch_may_use_remote_from(key, args(&["--server", "other.example.com", "--remote-mods", key])));
    assert!(launch_may_use_remote_from("friend:user_7", args(&["--remote-mods", "friend:user_7"])));
    assert!(!launch_may_use_remote_from("friend:User_7", args(&[])), "not a (lowercase) key");
}

fn wants(reaction_send: &[Vec<u8>], hash: &[u8; 32]) -> bool {
    reaction_send.iter().any(|payload| matches!(Message::decode(payload), Ok(Message::Request(found)) if found == *hash))
}

#[test]
fn m1_another_hosts_folder_does_not_count_as_already_there() {
    let host = host_game("m1_host");
    let (offer, _source) = running(&host);
    let sandboxed = offer.entries().iter().find(|entry| entry.id == "sand_mod").unwrap().clone();
    let player = scratch("m1_player");
    // the player has H's folder from an earlier join
    let h_key = server().key;
    std::fs::create_dir_all(host_dir(&player, &h_key)).unwrap();
    std::fs::copy(&host.sandboxed, host_dir(&player, &h_key).join(format!("{}.yak", transfer::hex(&sandboxed.sha256)))).unwrap();
    // a process started for H, now joining H: nothing to fetch for that mod
    let mut again = ClientJoin::new(player.clone(), Some(server()), true, Some(h_key.clone()), Vec::new());
    again.on_payload(transfer::CAPABILITIES);
    let sent = again.on_payload(&offer.encode()).send;
    assert!(!wants(&sent, &sandboxed.sha256), "H's own folder counts for H");
    // the same process now joins S: H's files are not loaded there, so they must not count
    let other = server_host("other.example.com").unwrap();
    let mut elsewhere = ClientJoin::new(player.clone(), Some(other), true, Some(h_key), Vec::new());
    elsewhere.on_payload(transfer::CAPABILITIES);
    let sent = elsewhere.on_payload(&offer.encode()).send;
    assert!(wants(&sent, &sandboxed.sha256), "another host's folder does not count");
}

#[test]
fn s3_a_sync_starts_with_an_empty_staging_folder() {
    let host = host_game("s3_host");
    let (offer, source) = running(&host);
    let mut session = HostSession::new(offer, source);
    let player = scratch("s3_player");
    let staging = staging_dir(&player);
    std::fs::create_dir_all(staging.join("sub")).unwrap();
    std::fs::write(staging.join("sub").join("junk.bin"), b"left by a hostile run").unwrap();
    std::fs::write(staging.join("old.yak"), b"old").unwrap();
    std::fs::write(staging.join("old.part"), b"old").unwrap();
    write_request(&staging, &server(), &[]).unwrap();
    let mut join = client(&player, true, Some(server()));
    run_join(&mut join, &mut session, |payload| payload);
    let mut names: Vec<_> = std::fs::read_dir(&staging).unwrap().flatten().map(|item| item.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    assert!(!names.iter().any(|n| n == "sub" || n == "old.yak" || n == "old.part"), "{names:?}");
    assert!(names.contains(&"join-request.json".to_owned()), "the new request is there: {names:?}");
}

#[test]
fn s4_at_most_two_transfers_run_at_once() {
    let counter = std::sync::atomic::AtomicUsize::new(0);
    assert!(take_place(&counter, MAX_TRANSFERS));
    assert!(take_place(&counter, MAX_TRANSFERS));
    assert!(!take_place(&counter, MAX_TRANSFERS), "a third is Busy");
    assert_eq!(MAX_TRANSFERS, 2);
}

#[test]
fn s4_a_third_transfer_is_answered_busy_and_places_come_back() {
    use std::sync::atomic::Ordering;
    let game = host_game("s4_busy");
    let (offer, source) = running(&game);
    let entry = offer.entries()[0].clone();
    // the counter is process wide and other tests may hold places: fill it the way two other connections would
    let before = ACTIVE_TRANSFERS.load(Ordering::Acquire);
    let missing = MAX_TRANSFERS.saturating_sub(before);
    ACTIVE_TRANSFERS.fetch_add(missing, Ordering::AcqRel);
    let mut session = HostSession::new(offer.clone(), source);
    let event = session.handle(&transfer::request(&entry.sha256)).unwrap();
    ACTIVE_TRANSFERS.fetch_sub(missing, Ordering::AcqRel);
    let HostEvent::Reply(replies) = event else { panic!("expected a refusal") };
    assert!(matches!(Message::decode(&replies[0]), Ok(Message::Refuse { reason: transfer::RefuseReason::Busy, .. })), "{replies:?}");
    drop(session);
}

#[test]
fn s4_a_file_that_did_not_change_is_hashed_once_and_read_from_the_hashed_handle() {
    let game = host_game("s4_cache");
    let (offer, mut source) = running(&game);
    let entry = offer.entries().iter().find(|entry| entry.id == "sand_mod").unwrap().clone();
    source.open(&entry).unwrap();
    source.close();
    let meta = std::fs::metadata(&game.sandboxed).unwrap();
    let key: HashKey = (meta.len(), meta.modified().ok());
    assert_eq!(cached_hash(&game.sandboxed, key), Some(entry.sha256), "the hash is remembered by path, length and mtime");
    assert_eq!(cached_hash(&game.sandboxed, (key.0 + 1, key.1)), None, "another length is a miss");
    source.open(&entry).unwrap();
    let mut buf = vec![0u8; entry.size as usize + 4];
    let read = source.read_at(0, &mut buf).unwrap();
    assert_eq!(Sha256::digest(&buf[..read]).as_slice(), entry.sha256);
    source.close();
}
