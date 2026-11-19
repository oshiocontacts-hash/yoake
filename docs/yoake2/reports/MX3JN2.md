# MX3JN2: join-time mod sync, launcher side

Branch `yoake2/join-mods`. Contract: `docs/yoake2/JOIN-MODS.md`, game side: `MX3JN1.md`. No UI was built, and no game or launcher GUI was started.

## Checklist
- [x] Step 0: merged `vps/yoake2/dev-folder` (conflicts in `control.rs` and `yoake2_mods.rs` resolved, both features kept)
- [x] horizon-mods `--features yak`: 77 + 41 passed (1 new test: host and generation together, and the error text names all five origins)
- [x] `yoake2_loader` (`yoake2-slice,yoake-hook-bench`): 42 passed; `cargo check -p horizon-client`: clean
- [x] 1 the setting `join_mod_sync` (default true) and `--join-mod-sync`
- [x] 2 exit code 75: strict request validation, hash check, `install_remote_package`, host-scoped approval, all or nothing, staging cleared, worker thread under the control lock
- [x] 3 relaunch (server: `--server`; friend: room join with a fresh grant, fallback "Join again")
- [x] 4 later launches of the same host get `--remote-mods` and the scoped list; other launches get neither
- [x] 5 `remote_hosts` / `remove_remote_host`
- [x] 6 tests (8 new) and the full launcher suite: lib 612, bin 170, integration 6 + 1 + 2, all passed
- [x] report, commit, push

## Step 0 decisions
- `ApprovedList`: `to_text` writes `host` and `generation`; `parse` reads both; `with_host` and `with_generation` both exist.
- Origin in a list: local, catalog and dev are always valid; server and friend only when the top-level host has that kind. Error text: "must be local, catalog, dev, server or friend".
- `decide()` was already right: Local and Dev run when approved, Catalog when approved or signed, Server and Friend only when approved (approval exists only in a host-scoped list).
- The dev reload (`dev_reload_check`) now loads through `load_mods_scoped(.., list.host())`. When the process has `--remote-mods`, only that same host is used (never another host's folder).
- Found and fixed in the launcher: the dev watcher's list rewrite built the list from the player's own entries only, so the first dev change would have dropped the host scope and the host's entries. `dev.rs rewrite` now keeps the previous list's host and its server/friend entries (test `a_dev_rewrite_keeps_the_host_scope_of_the_run`).

## API (launcher lib, `horizon_launcher::full_access`)
- Setting: `Settings::join_mod_sync: bool` (serde default true, so an old file reads as on; `Settings` now has a hand-written `Default`).
- Launch: `prepare_joining(instance, host: Option<&str>, join_mod_sync: bool) -> Prepared` and `prepare_for(data, instance, dev_folder, host, sync)`. `Prepared` gains `join_mod_sync` and `remote_host`; `apply` and `args` add `--join-mod-sync`, `--remote-mods <key>` and `--full-access-approved <list>`. `prepare`, `prepare_in` and `prepare_with` behave as before (no sync, no host).
- Host keys: `server_host_key(address)` (the same rule as the game's `server_host`), `friend_host_key(portal_id)`.
- Exit: `JOIN_EXIT_CODE` (75), `is_join_exit_text(&str)`, `handle_join_exit(instance) -> Result<JoinSyncDone { host_key, host_display, installed, rejoin: Rejoin::{Server(addr), Friend(principal)} }, String>` (`handle_join_exit_in(data, instance)` for tests), `Rejoin::quick_play()`, `parse_join_request`.
- Status: `last_join_sync() -> Option<JoinSyncStatus { host_display, installed, error }>` (latest of any instance) and `last_join_sync_for(instance)`.
- Mods page: `remote_hosts(instance) -> Vec<RemoteHost { key, display, mods: Vec<RemoteMod { id, version, sha256_short, full_access }> }>` and `remove_remote_host(instance, key) -> Result<(), String>` (`_in` variants take the data dir).
- App (main.rs, for the UI): `join_again_host() -> Option<&str>` and `join_again()` for the "Join again" button (see 要確認 1); `Msg::JoinSync`.

## How it works
- `full_access/join.rs`. Request validation: a regular file of at most 1 MiB; host kind, address, key and rejoin agree and use plain host-key characters; 1 to 64 mods; id and version plain text of 1 to 64 characters; sha256 64 lowercase hex digits; size 1 to 64 MiB and at most 256 MiB in total; `file == <sha256>.yak`; no duplicate id or hash. The staging folder must be a real folder (not a link or junction).
- Each staged file must be a regular file (no symlink) of the exact size, with the SHA-256 of the request, a valid `.yak`, a manifest id and version equal to the request, and a `full_access` flag equal to the package's. Everything is verified before anything is installed; a failure while installing deletes the files that request created. `.staging` is emptied in every case, and the error is in `last_join_sync`.
- Approval record: `<control folder>/remote-hosts/<instance>/<host dir>.json` (id, version, sha256, full_access per mod). It is **not** in `full-access.toml` or `yak-origins.toml`: those are keyed by hash only, and a server origin there would make the player's own copy of the same bytes distrusted for good. The control folder is in every list's `protected` paths. A launch passes a recorded full-access mod only if its file is still in the host folder and still hashes to the record. A newer version of the same id from the host replaces the old file and record.
- Catalog first: `catalog_bytes()` is the hook. It returns `None` (no `catalog_id` is sent yet), so the staged file is used.
- Launcher (main.rs): `Msg::Exit` of a Client that was started with the flag and exited 75 is not a crash. A worker thread runs `handle_join_exit`, then `Msg::JoinSync` arrives: for a server it calls `quick_play("--server", addr)`; for a friend it looks up the room of that host principal in the launcher's room list and calls the normal `friends_join_room` (fresh grant); if no such room is listed, `join_again` is set. `friends_join_room` also records the host key so the guest game gets that host's remote mods. The CLI `play --server` passes `--remote-mods` and the list for a host with a folder, and never `--join-mod-sync`.

## Tests (`launcher/src/yoake_mods/plan/tests/join_sync.rs`)
- A valid request (server and friend) installs, approves for the host only, and builds the relaunch args and list.
- A changed file is not passed; a newer version replaces the old one.
- Bad requests install nothing, clear staging and surface the error: sha mismatch (with a good first mod), 5 path-traversal names, oversize, wrong size, 65 mods, 6 bad keys or rejoins, bad JSON, no request, wrong full_access flag, wrong id, junk bytes.
- Another server, another port, a friend, no host and an invalid key get no `--remote-mods`, no scope and no host entries.
- Setting default, parse and round trip; no flag when off.
- `remote_hosts` and `remove_remote_host`.
- A dev rewrite keeps the host scope.
- Host key rules and the exit text.

## 要確認
1. The friend rejoin is automatic only when the host's room is in the launcher's cached room list (`friends.rooms`); otherwise `join_again_host()` / `join_again()` apply. The UI should show "Join again" and refresh the rooms.
2. The relaunch uses `quick_play` and the currently selected instance, as Play does; if the player changes the selected instance between the exit and the relaunch, the other instance starts.
3. `host_display` is the server address, or the friend's portal user id; the page may want to map an id to a friend name.
4. `last_join_sync()` takes no argument (latest of any instance); I added `last_join_sync_for(instance)`.
5. The CLI does not run the sync loop (it never passes `--join-mod-sync`).
6. `Settings` lost `#[derive(Default)]` for a manual `Default` (needed for default true).
7. Files in a host folder that are not in the record (hand-dropped) are listed by `remote_hosts` but never passed in a list.
8. Not run: the horizon-client lib suite (about 1200 tests). No game was launched, so the real exit-75 round trip is untested end to end.
