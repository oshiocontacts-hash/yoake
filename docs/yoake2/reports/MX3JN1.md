# MX3JN1: join-time mod sync, game and protocol side

Branch `yoake2/join-mods` on a764f314f. Contract: `docs/yoake2/JOIN-MODS.md` (owner decision: fully automatic, no confirmation).
Built: everything up to and including step 2c of the contract, the host side of the transfer, and the loader/argument
changes the launcher needs. **No launcher code, no game launch, no GUI.**

## Progress checklist

- [x] read the contract, the capability exchange (`mod_list/native.rs`), the join path (`lan/conn.rs` host, `net.rs` client), the yak loaders
- [x] 1 protocol: `yoake:mod_transfer` (`horizon-protocol/src/mod_transfer.rs`), offer fields, caps on both sides
- [x] 2 host side: offer from the host's own `mods/*.yak`, lazy file source, one transfer at a time (`join_mods.rs`, `lan/conn.rs`)
- [x] 3 client side: sync state machine, staging sink, `join-request.json`, status text, exit code 75 (`join_mods.rs`, `net.rs`)
- [x] 4 loading on the next launch: `--remote-mods`, full-access via the approved list scoped to a host, sandboxed via `yak_mods::remote_installed`
- [x] 5 tests (table below)
- [x] 6 runs of the requested suites (table below)
- [x] report, commit, push

## Protocol as built

Channel `yoake:mod_transfer`, configuration phase, **used only when both peers speak it**:

1. The host sends `[0, 1]` (capabilities, version 1) **only if it has something to offer**, then a `Ping(4)`. A host with nothing
   to offer, an in-memory (single player) connection and a non-Horizon client never see this.
2. A client that understands answers `[0, 1]` (the net thread answers any host capabilities message it gets from a Horizon host).
   An old client ignores the unknown payload and answers the ping; the host then does nothing more on this channel and the join
   goes on exactly as before (native/legacy mod comparison included).
3. The host sends the offer (kind 1). The client replies `done` (kind 5) when it needs nothing (everything present, setting off, no
   host identity) or `request` (kind 2) for the first missing SHA-256. The host serves **one SHA-256 at a time**:
   `chunk` messages (kind 3: SHA-256, offset, last flag, at most 256 KiB), the last one marked. A refusal (kind 4: SHA-256, reason
   NotOffered / Busy / Unavailable / Limit) ends it. The client asks for the next only after the previous one is verified.
4. Offer entry: `id, version, sha256, size, full_access, side_client, catalog_id?` (strings 1..=64 chars, no control characters,
   size 1..=64 MiB, no duplicate id or SHA-256). Flags byte: bit0 full_access, bit1 side_client, bit2 catalog_id present; other bits are an error.

Caps, both sides: 64 MiB per mod, 64 mods per offer and per join, 256 MiB per join. The client refuses a plan over 64 mods or
256 MiB **before asking for any byte**; the host refuses requests after 64 requests or 256 MiB promised on one connection, refuses
a hash that is not in its own offer without touching the disk, and a request while a transfer runs (Busy). A request carries only a
SHA-256: no path, no name. The host opens the file again when a transfer starts and re-hashes it; a file that changed since the offer is refused (Unavailable).

The wire format is versioned like the existing channels: the capability message carries the version (`[0, 1]`), and the offer a
list version byte (1). An unknown version is "not supported" (no offer sent) or a decode error (client stops syncing and says why).

## Game side, step by step

`join_mods::ClientJoin` (one per configuration phase, in `net.rs configure`; idle unless the host sends the channel):

- host capabilities -> answer capabilities. Nothing else is read before that.
- offer -> `ClientSync::start`: for every entry with `side_client` and no local copy (`has_local`: a `.yak` of that SHA-256 in
  `mods/` or in this host's remote folder, or, for a mod that is not full-access, an installed package with the same id and version)
  a request. With the setting off (no `--join-mod-sync`), or no host identity, the answer is just `done` and **nothing is written, not even a folder**.
- chunks -> `<game dir>/mods/remote/.staging/<sha256>.part`; on the last chunk size and SHA-256 are checked, the file is renamed to
  `<sha256>.yak`. Status text on the connecting screen: `ホストから MOD を取得中… / Getting mods from the host… (n/total)`.
- all staged -> `join-request.json` (atomic write) -> status -> **exit code 75** (`join_mods::exit_with`; the testable part is `finish_join`, which writes the file and returns 75).
- any mismatch (bad hash, wrong offset, too much data, refusal, bad message, write error, connection lost): the partial file is
  deleted, a stale `join-request.json` is deleted, the join stops with `Could not get the host's mods: ...`. Files already verified in
  staging stay (the launcher empties the folder); **no request is left**.

`join-request.json`, exactly:

```json
{ "host": {"kind": "server"|"friend", "address": "play.example.com:25570"|"<portal user id>", "key": "server:play.example.com:25570"|"friend:<portal user id>"},
  "mods": [ {"id": "...", "version": "...", "sha256": "<64 hex>", "size": 123, "full_access": true, "catalog_id": "...", "file": "<sha256>.yak"} ],
  "rejoin": "play.example.com:25570"|"friend:<portal user id>" }
```

`catalog_id` is omitted when the host did not name one. `mods` holds only what was fetched. `rejoin` for a server is the value to pass to `--server`
(host lowercased, port always present); for a friend it is `friend:<portal user id>` (the launcher must make a fresh social grant itself).

Host identity: a direct join is `server:<lowercase host>:<port>` (default port 25565, IPv6 literal in brackets); a social/relay join
is `friend:<host principal of the grant>`. No identity (single player, replay, an old `friend:K7QM-...` invite code, an address with odd characters) means no sync. The key is
valid when it is `server:`/`friend:` + `[A-Za-z0-9:._\-\[\]]`, no `..`, at most 200 characters.

## Loading on the next launch

- **Folder name.** A colon is not allowed in a Windows folder name, so `mods/remote/<host-key>/` is written with `:` as `+`:
  `mods/remote/server+play.example.com+25570/`, `mods/remote/friend+<id>/` (`control::host_dir_name`, `control::host_remote_dir`).
  Files in it are named `<sha256>.yak`.
- **`--remote-mods <host key>`** (the key with colons, not the folder name; `--remote-mods=<key>` also works). Without it nothing remote is read.
- **Full-access** (needs the launcher's approved list, `--full-access-approved`): the list gets a top-level `host = "<host key>"`
  (`ApprovedList::with_host`) and entries with `origin = "server"` or `"friend"` (the kind must match the host). `ApprovedList::parse`
  refuses such an entry in a list with no host or with a host of the other kind. The client thread loads
  `mods/remote/<host dir>/` through `load_client_mods` **only if** `--remote-mods` equals the list's `host`; the plain `mods/` folder never sees these entries
  (`ApprovedList::to_files` returns local/catalog only; `to_remote_files(key)` returns the host's, and nothing for any other key). The server thread (single player, the player's own world)
  never reads a remote folder. A remote mod with the id of a local mod that already runs is unloaded again (the local one wins) and reported.
- **MX3L2 rule, changed minimally.** `decide(Server|Friend, approved, signed)` is now Ok exactly when `approved`; the early refusal
  in `load_package_on` is gone (the origin goes through `decide` like the others). Everything else keeps the old refusal: no list, another host's list, a list without a host scope, a hand-dropped file in the plain folder
  (no origin), a remote file not named in the list, a file whose bytes changed (hash no longer matches), a signature alone. The signature is never a substitute.
- **Sandboxed** `.yak` (not full-access): `yak_mods::remote_installed` reads `mods/remote/<host dir>/<sha256>.yak` (name must be the file's SHA-256, at most 64 MiB, client side, compatible with this build, not full-access), unpacks each once into
  `<game dir>/yoake-remote/<host dir>/<sha256>/` and adds them to the installed packages after the player's own (an id the player has stays theirs). They then go through
  the unchanged sandboxed loader, which still **requires a trusted signature** (unless the developer override). This also works when the launch has no `yoake-mods.txt`.
- `game.rs` now accepts `--join-mod-sync`, `--full-access-dev`, `--remote-mods <v>` and `--full-access-approved <v>` (the last two were not listed in the argument parser; the loader reads them from the process arguments).

## What the launcher must do

1. Pass `--join-mod-sync` to every game it starts, when the setting "Get missing mods automatically when joining" is on.
2. On exit code **75** of a game it started: read `<instance>/mods/remote/.staging/join-request.json`. For each `mods[]` entry: catalog download if `catalog_id` + the signed catalog has that id+SHA-256,
   else take `.staging/<file>`; check the SHA-256 against the request; install with `horizon_mods::yak::control::install_remote_package(mods_dir, host.key, bytes, sha256)` (new, writes
   `mods/remote/<host dir>/<sha256>.yak`, never overwrites; it does **not** use `install_package`, whose Server/Friend folder is the old per-origin one);
   record/show it as "from <host>"; empty `.staging`.
3. Relaunch the same instance joining the same host: `--server <host.rejoin>` for `kind = server`; for `kind = friend` make a fresh social grant for `host.address` (the portal user id) and use
   the usual `--yoake-social-handoff`. Add **`--remote-mods <host.key>`** and keep `--join-mod-sync`.
4. Full-access mods: auto-approve (owner decision) by writing an approved list for this launch with `ApprovedList::new(entries, protected).with_host(host.key)`; the host's entries have `origin` `Server` or `Friend`
   (kind of the host), `id`/`version`/`sha256` from the request. Local/catalog entries of the player go in the same list as before. Pass it with `--full-access-approved <path>`.
   Keep the entries for later joins of the same host if you want them to load without a new request (the request only exists after a sync).
5. Every later launch that joins that host (the player already has the mods, so no 75) must also pass `--remote-mods <key>` and a list scoped to it; launches that do not join it pass neither.
6. Files to read: `join-request.json` only. The game writes nothing else outside `mods/remote/.staging/` and `yoake-remote/` (unpacked sandboxed packages, safe to delete).

## Where the code is

- `crates/horizon-protocol/src/mod_transfer.rs` (+ `mod_transfer/tests.rs`): messages, `Offer`, `HostSession` (host state machine, `Source` trait), `ClientSync` (client state machine, `Sink` trait), `Negotiation`.
- `crates/horizon-client/src/join_mods.rs` (+ `join_mods/tests.rs`), compiled in **horizon-client-lib** (that crate holds `net.rs` and `lan/conn.rs`): args, host identity, staging sink, request file, host offer and file source, `ClientJoin`. The game crate registers what it runs with `join_mods::set_providers` (in `yak_mods::initialize`).
- `crates/horizon-client/src/lan/conn.rs` (`offer_mods`, host) and `net.rs` (`configure`, client): the wiring. `game/remote.rs` and `game/social_relay.rs` name the host (`set_host`) before a connection.
- Loader: `horizon-mods/src/yak/control.rs` (`ApprovedList::host`, `with_host`, `to_files` local/catalog only, `to_remote_files`, `valid_host_key`, `host_dir_name`, `host_remote_dir`, `install_remote_package`), `horizon-mods/src/yak.rs` (`Archive::copy_file`), `yoake2_mods.rs` (`decide`, `load_client_mods_scoped`, `server_runs_package`), `yak_mods.rs` (`remote_installed`).

## Tests

| Suite | Result |
|---|---|
| horizon-protocol (all) | 220 passed (11 new in `mod_transfer`) |
| horizon-mods, `--features yak` | lib 76 passed (3 new: host-scoped list, host keys, `install_remote_package`), `tests/yak.rs` 41 passed |
| horizon-client-lib lib, filter `join_mods` | 13 passed (all new) |
| horizon-client lib, features `yoake2-slice,yoake-hook-bench`, filter `yak_mods` | 21 passed (1 new: `a_hosts_sandboxed_packages_are_installed_for_that_host_only`) |
| `yoake2_loader` (features `yoake2-slice,yoake-hook-bench`) | 32 passed (1 rewritten, 5 new) |
| yoake-slice, `--features wasm` | all passed (6, 6, 7, 21, 1) |
| `cargo check -p horizon-client` (no features) | clean |

The full horizon-client lib suite (about 1200 tests) was **not** run: only the filtered tests above, to save disk and time.

Requested tests and where they are:

- capability negotiation, old peer gives old behaviour: `mod_transfer::tests::capabilities_gate_the_offer_and_old_peers_hear_nothing`; client side `join_mods::tests::the_client_ignores_the_channel_until_the_host_advertised_and_after_it_is_done`; `without_the_flag_or_a_host_identity_...` (setting off: only `done`, no folder written).
- happy path: `a_transfer_happy_path_streams_chunks_with_a_final_marker` (3 chunks, 256 KiB, last marker) and `join_mods::tests::a_join_with_missing_mods_stages_them_writes_the_request_and_exits_75` (real `.yak` files, host and client in memory).
- sha256 mismatch, abort and delete: `a_sha256_mismatch_aborts_and_deletes_the_partial_file`, `a_changed_byte_on_the_wire_aborts_the_join_deletes_the_file_and_leaves_no_request`, `a_file_changed_after_the_offer_is_not_served`.
- non-advertised sha256 refused by the host: `the_host_refuses_a_sha256_it_did_not_offer` (the source is never opened), plus a foreign hash in `the_host_offers_only_what_it_runs_...`.
- caps: `the_caps_hold_on_both_sides` (64 mods, 64 MiB, 256 MiB plan refused before any request, host `Limit`), `one_transfer_at_a_time_and_the_per_join_limits` (Busy, 64 requests).
- request file and exit code as functions: `the_request_file_has_exactly_the_contracts_fields`, `finishing_a_join_writes_the_request_and_gives_exit_code_75` (`finish_join` returns 75; `exit_with` is the only part that ends the process and is not called in tests).
- `--remote-mods` scoping: `remote_key_from` tests; `a_hosts_full_access_package_loads_only_for_the_launch_that_joins_that_host` (other key, no key, invalid key, server thread), `a_remote_folder_of_another_host_is_not_loaded_...`, `a_hosts_sandboxed_packages_are_installed_for_that_host_only`.
- loader allows Server/Friend only via the approved list: `a_package_from_a_server_or_a_friend_is_not_full_access_unless_approved_for_its_host` (policy table), `a_remote_package_that_the_list_does_not_name_or_that_changed_is_refused`, `an_entry_of_a_host_scope_does_not_make_a_hand_dropped_file_in_the_plain_folder_run`, `a_remote_mod_with_the_id_of_a_local_one_does_not_replace_it`, and `control::tests::a_servers_or_friends_package_is_valid_only_in_a_list_scoped_to_its_host`.

## 要確認

1. **Folder name.** The contract says `mods/remote/<host-key>/`; `server:host:port` has colons that Windows folder names cannot hold. I wrote the folder with `:` replaced by `+` (`server+play.example.com+25570`). The launcher must use `host_dir_name` / `host_remote_dir` (exported from `horizon_mods::yak::control`). Confirm or choose another encoding.
2. **Portal user id.** For social joins I use the `host_principal` of the guest grant as the "portal user id" (`friend:<host_principal>`). Confirm that is the id the launcher and portal know the host by. Old invite-code friend joins (`friend:K7QM-...`) and anything without a known host identity do not sync (the secret code must not become a folder or key).
3. **Where the approved list carries the host scope.** `ApprovedEntry` is unchanged (the launcher builds it with a struct literal); the scope is a new top-level `host = "<key>"` in the approved list, and entries with origin `server`/`friend` are valid only in a list that has it. Confirm the launcher may write it this way.
4. **`--full-access-approved` and `--full-access-dev` in `game.rs`.** I could not find them in the option parser (which rejects unknown options), so the game would not start with them; I added accepting arms (the loaders read them from the process arguments). Please check this was not handled somewhere I missed.
5. **What counts as "already has it".** Full-access: a `.yak` with the same SHA-256 in `mods/` (the player's own, even if not approved) or in this host's folder. Sandboxed: an installed package with the same id and version (an installed tree has no package SHA-256, so a same-version different-bytes mod is not noticed). Entries with `side_client = false` are never fetched.
6. **Host offer contents.** Offered: top-level `mods/*.yak` that the host actually runs (full-access: loaded on the server thread, hash recorded by `server_runs_package`, and with a client module; sandboxed: `required_on_join` and an installed package with the same id and version). `catalog_id` is always absent for now (the manifest has no such field; the launcher knows the catalog). A sandboxed package the host only has as an unpacked tree (no `.yak` file) cannot be served.
7. **Read timeout.** Host reads in the configuration phase have a 10 s timeout (`LOGIN_TIMEOUT`). The host waits for the client's first reply after the offer while the client hashes candidate local files (size filter first, so normally milliseconds). A very slow disk with many large same-size files could exceed it. Not changed.
8. **Verified files stay in staging on a later failure.** On a failure the partial file and the request are deleted; mods already verified earlier in the same join stay in `.staging` for the launcher to clear. Say if they should be deleted too.
9. **Exit 75 from the net thread.** The game quits with `std::process::exit(75)` from the connection thread after a 400 ms pause for the status text (no world is open at that point). If a quieter shutdown is wanted, the game loop could take `Reaction::exit` instead.
10. **Remote sandboxed mods are unpacked by the game** into `<game>/yoake-remote/<host dir>/<sha256>/` (new folder, safe to delete). The launcher might prefer to unpack them itself like other sandboxed packages (`yoake-mods/` + `yoake-mods.txt`); both work. Unsigned remote packages still do not run (the sandboxed loader requires a trusted signature).
11. **`install_package` for Server/Friend** still writes to the old `mods/remote/<origin>/`; the launcher must use `install_remote_package` instead. `install_package` is unchanged (its tests assert that layout).
12. **Line endings.** Several files were CRLF in the working tree and are now LF; git normalises them (the diff is small).
