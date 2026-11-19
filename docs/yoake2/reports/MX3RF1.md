# MX3RF1: fixes of the Yoake 2.0 security review

## Checklist
- [x] M1 host scoping per session (game: loader, sandboxed side, dev reload, unload)
- [x] M2 at most one automatic rejoin per join; host folder caps 512 MiB / 64 mods
- [x] S1 new arguments only for games that list them in `--launch-api`
- [x] S2 lowercase host keys, launcher recomputes the canonical key
- [x] S3 whole `.staging` emptied at sync start
- [x] S4 hash cache, 2 concurrent transfers (Busy), idle timeout, same-handle read
- [x] S5 dev-origin approvals only with an active dev folder
- [x] S6 dev reload remote check = session key
- [x] S7 failed unload: retry and SERVED update
- [x] Nits: `replaced` never deletes a file of the same request; no `bytes.clone()`
- [x] R1 (smoke test) game root known without `yoake-mods.txt`
- [x] R2 (smoke test) log lines for loaded full-access mods

## Findings and fixes
- **M1.** `join_mods::session_remote_key_for(process_key, host)` / `session_remote_key()`: the process `--remote-mods` key counts only while the connection's host (`join_mods::host()`) has the same key. `load_client_mods` uses it at every client session start (so single player, replay and another server load nothing remote; `load_client_mods_for_host` is the testable form). `ClientJoin::new` uses it for `has_local`, so another host's folder no longer counts as "already there". Remote mods are in the client `LOADED` list, so `unload_client_mods` at session end removes them (test checks `client_mods_loaded()` is false afterwards). Sandboxed side: packages are read once at process start, before any connection, so they cannot be unloaded per session; `add_remote` now additionally requires that the launch's `--server` equals the key (`launch_may_use_remote_from`; friend keys have no address on the command line, the key only). See 要確認 1.
- **M2.** Launcher: the start that follows a sync is marked automatic (`auto_rejoin_for`, valid 60 s, matched to the host key); if that game exits 75 again, `refuse_repeated_exit` empties staging, installs nothing and sets the status (Mods page) and `client_error` (Play page). A join the player starts (or "Join again") is a new chain. Host caps `MAX_HOST_BYTES` = 512 MiB, `MAX_HOST_MODS` = 64 count the files already in the folder (minus files this request replaces) plus the new ones; over the cap installs nothing.
- **S1.** `launcher/src/full_access/support.rs`: `GameSupport::detect(program)` runs `<game> --launch-api` (prints JSON, no window), caches per (path, size, mtime), 10 s timeout; a non-zero exit (old game) or failure gives no new arguments. `Prepared::args_for/apply_for` pass each argument only if listed; `--join-mod-sync` also decides whether a 75 exit is expected. The four options were added to `launch_api::OPTIONS`; `docs/api/launch-reference.md` regenerated.
- **S2.** `valid_host_key` accepts lowercase only; game `friend_host` and launcher `friend_host_key` lowercase the id; `parse_request` requires the key to equal the key recomputed from the address (server: canonical `host:port`; friend: lowercased id).
- **S3.** `join_mods::clear_staging` empties the whole folder when a sync starts.
- **S4.** Hash cache keyed by (path, length, mtime) (also fed from the offer scan); `Source::admit` (protocol) with a process-wide limit of `MAX_TRANSFERS` = 2, a refusal is `Busy`, places are returned on finish, close and drop; `offer_mods` ends the exchange after 60 s without a word on the channel (previously other packets kept it alive); the file is hashed and read from the same handle.
- **S5.** `prepare_for` filters dev-origin entries unless a dev folder is given, and rewrites the list if the dev start then fails.
- **S6.** `dev_remote_key(list_host, session_key)`: the list host is kept only if it equals the session key (no key = none).
- **S7.** After a dev reload the side's `loaded_generation` must equal the list's; otherwise the stamp is put back so the next check retries. `unload_previous` updates `SERVED` to what still runs after a partial failure (`LOADED` now keeps the hash).
- **Nits.** `replaced` skips hashes of the same request; `Cursor::new(bytes.as_slice())`.
- **R1.** `yak_mods::startup_runtime`: without `yoake-mods.txt` and without a remote key the runtime is now created with only `game_root` (before, `initialize` returned and `game_root()` stayed None).
- **R2.** `announce_loaded`: per loaded mod `yoake: full-access mod 'id' 1.0.0 loaded (Client side, N hook(s) registered)` and a summary line, `log::info!` plus stderr. `Loaded` gained `version`.

## Tests
Run and green: launcher `cargo test -j 4 --no-fail-fast` (618 lib incl. new `m2_*`, `s1_*`, `s2_*`, dev S5 test; all targets ok); `horizon-mods --features yak` (78 + 41); `horizon-protocol`; `horizon-client` with `yoake2-slice,yoake-hook-bench`: `--test yoake2_loader` (43, incl. `m1_a_hosts_mods_load_only_in_sessions_connected_to_that_host`: H loads, single player none, other server none, friend none, H again loads, unload ok), `yoake2_examples`, `yoake2_template`; lib `launch_api`, `yoake2_mods`, `yak_mods::the_game_root_is_known_without_a_yoake_mods_list`; `horizon-client-lib --lib join_mods` (20, incl. M1/S3/S4); `cargo check -p horizon-client`. Not run: the whole horizon-client lib suite (by instruction).

## 要確認
1. Sandboxed packages and content registered at process start (content packs, DLL natives) cannot be un-registered per session; only the WASM/full-access loading is per session. The start-time gate (`--server` must equal the key) limits it to a launch for that host. Full per-session sandboxed scoping would need a loader change.
2. S7 retry has no automated test (`runtime::unload` cannot be made to fail from a test); the stamp/generation logic is simple but unexercised.
3. R1 test covers the game root (`startup_runtime`) and the loader tests already use a mods folder with no `yoake-mods.txt`; no test starts the real game.
4. The auto-rejoin flag is a 60 s window matched by host key; a user join of the same host within it right after a sync would be treated as automatic once.
5. `GameSupport::detect` runs the game executable with `--launch-api` on the launcher's UI thread at launch (first time per build, <1 s expected).
6. Friend portal ids are lowercased in the key; if the portal ever has case-sensitive ids that differ only by case they would share a folder.
