# Join-time mod sync (Yoake 2.0)

Owner decision (2026-11-19): when a player joins a host and lacks mods the host has, the missing mods are fetched and
installed **fully automatically, with no confirmation, including full-access mods**. A launcher setting
("Get missing mods automatically when joining", default on) can turn it off.

## Guarantees kept without prompts
- **Hash pinned**: only bytes whose SHA-256 equals the host's advertised list are installed. Anything else is discarded.
- **Host scoped**: mods fetched from host H are stored under `<instance>/mods/remote/<host-key>/<sha256>.yak` with origin
  `Server` (plain address) or `Friend` (social/relay host), and are passed to the game **only for launches that join H**.
  Single player, other servers and the player's own mods never get them.
- **Visible and removable**: the launcher's Mods page lists them grouped by host ("from <host>"), with "Remove all from
  this host".
- **Catalog first**: if an advertised entry carries a catalog id and the signed Yoake catalog has that id+sha256, it is
  downloaded from the catalog (verified signature) instead of from the host.
- **Caps**: 64 MiB per mod, 256 MiB per join, 64 mods per join; the transfer stops on the first mismatch. A host's folder
  holds at most 512 MiB and 64 mods in total (MX3RF1): a request that would go over installs nothing and says so.
- **One automatic rejoin**: the launcher restarts the game once after a sync. A game it restarted itself that exits 75
  again installs nothing and stops with an error on the Mods and Play pages (the player can still join again by hand).
- **Per session, not per process** (MX3RF1): a host's mods run only while the connection of the session goes to exactly
  that host (`join_mods::session_remote_key`). Leaving for single player or another server in the same process runs none
  of them, and they are unloaded when the session ends.
- **Old games**: the launcher passes `--full-access-approved`, `--full-access-dev`, `--join-mod-sync` and `--remote-mods`
  only to a game whose `--launch-api` lists them (cached per executable); a 1.0 game gets none of them.

## Flow
1. On join, the host sends its yak list: `{id, version, sha256, size, full_access, side_client, catalog_id?}` for every
   mod `required_on_join` (plus all full-access mods that have a client module).
2. The client game compares with what it has loaded. Missing or different → instead of refusing:
   a. requests the missing sha256s on channel `yoake:mod_transfer`; the host streams each file in chunks (≤ 256 KiB),
      the client writes them to `<instance>/mods/remote/.staging/<sha256>.part`, verifies size + SHA-256, renames to
      `.yak`;
   b. writes `<instance>/mods/remote/.staging/join-request.json`:
      `{ "host": {"kind": "server"|"friend", "address": "...", "key": "<host-key>"}, "mods": [ {id, version, sha256,
      size, full_access, catalog_id?, file: "<sha256>.yak"} ], "rejoin": "<what --server / friend join needs>" }`
   c. shows "Getting mods from the host…" then exits with exit code **75** (`JOIN_NEEDS_MODS`).
3. The launcher sees exit code 75 for a game it started, reads join-request.json, and for each mod: catalog download if
   possible, else takes the staged file; checks SHA-256 against the request; installs it into
   `mods/remote/<host-key>/` with origin Server/Friend and the host key; auto-approves it for that host scope; then
   relaunches the game with the same instance joining the same host (`--server` or the friend-join arguments), passing
   the approved list for that host scope. The staging folder is emptied.
4. If the setting is off, the game refuses as today, listing the missing mods.

## Host key
`server:<lowercase host>:<port>` for direct joins, `friend:<lowercase portal user id>` for social/relay joins. Keys are
lowercase only (`valid_host_key`); the launcher requires the request's key to equal the key it recomputes from the address. Stored in the
origins sidecar next to the origin.
