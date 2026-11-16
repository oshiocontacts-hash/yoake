# Writing a Yoake WebAssembly mod

A Yoake wasm mod is a WebAssembly **component** (`wasm32-wasip2`) for the WIT world
`yoake:mods/client-mod@1.0.0` (`crates/yoake-wasm/wit/yoake-mod.wit`). The guest SDK is
`crates/yoake-wasm-sdk`; a complete example is `crates/yoake-wasm-sdk/examples/hello`.

## Toolchain

```sh
rustup target add wasm32-wasip2
```

## Build

```sh
cd crates/yoake-wasm-sdk/examples/hello
cargo build --release --target wasm32-wasip2
# -> target/wasm32-wasip2/release/yoake_wasm_hello.wasm  (about 18 KB)
```

Use `crate-type = ["cdylib"]` and, for small output, `opt-level = "s"`, `lto = true`,
`strip = true`, `panic = "abort"` in `[profile.release]`. The SDK crates are excluded from the
host workspace; each has its own `[workspace]` table.

## Mod code

```rust
#![no_std]
use yoake_wasm_sdk::prelude::*;
use yoake_wasm_sdk::{Mod, export_mod, hook, hud};

#[derive(Default)]
struct Hello;

impl Mod for Hello {
    fn subscriptions() -> &'static [&'static str] { &[hook::HUD] }
    fn hud(&mut self) { let _ = hud::text("hi", (4.0, 4.0), 0xffffff, true); }
}
export_mod!(Hello);
```

- Implement `Mod` (all hooks default to no-ops) and list every hook you override in
  `subscriptions()` using the `hook::*` constants. The host only calls subscribed hooks.
- **Normal `std` works** (SDK feature `std`, on by default): `println!`/`eprintln!` go to the mod's
  log (one line per newline, long lines cut, bounded per call), `std::time::{Instant, SystemTime}`,
  `HashMap`, threads-free code, random (`HashMap`'s hasher) all work. The host links exactly this
  WASI preview 2 subset: `wasi:cli/{stdin,stdout,stderr,environment,exit,terminal-*}`,
  `wasi:clocks/{monotonic-clock,wall-clock}`, `wasi:random/*`, `wasi:io/*` and
  `wasi:filesystem/{types,preopens}`. stdin is empty, the environment and the arguments are empty,
  `std::process::exit` traps the mod (only that mod is disabled), the clock never blocks the host:
  `thread::sleep` and `pollable.block()` on a clock wait of more than 2 ms trap the mod (shorter waits
  return at once), the filesystem has **no
  preopened directory** (every `std::fs` call fails; the interfaces are linked only because `std`
  imports them) and there are **no sockets** (a component that imports `wasi:sockets/*`, or any
  other interface outside `yoake:mods/*` and the list above, is rejected at load). The example is
  `crates/yoake-wasm-sdk/examples/hello-std`.
- For the smallest modules turn `std` off (`default-features = false, features = ["runtime"]`) and
  write `#![no_std]`: the SDK ships its own allocator and panic handler (feature `runtime`) and
  `yoake_wasm_sdk::prelude` re-exports `String`, `Vec`, `Box`, `format!`, `vec!`. A panic traps
  the instance. See `examples/hello`.
- Host calls: `log`, `feedback`, `hud::{text, rect, text_width}`, `settings::{get, set}`,
  `net_send`, `local_player`, `playing`, `world_key`, `snapshot_world`, `instance_dir`,
  `account`, `screen::*`. All return `Result<_, HostError>` (`net_send`: `SendError`).
- The generated bindings come from `build.rs` (wit-bindgen 0.57.1's proc macro does not escape the
  package name `mod`).

Check the imports of a build (`yoake:mods/*` and, for `std` mods, the WASI subset):
`wasm-tools component wit target/wasm32-wasip2/release/yoake_wasm_hello.wasm`.

## Packaging in a `.yak`

Put the component at `bin/wasm/<name>.wasm` and declare it in `yak.toml`:

```toml
[wasm]
client = "bin/wasm/hello.wasm"
interface = "1.0"
```

`interface = "1.0"` is the `yoake:mods` major.minor the component was built against; the host
rejects components that import anything outside `yoake:mods/*` and the WASI subset above.

## Developer mode: hot reload

With `YOAKE_DLL_MODS_UNSIGNED=1` the game polls every unsigned wasm module of an installed `.yak`
once a second and, when the file changed (and stayed unchanged for a whole poll), builds a fresh
instance and swaps it in at the next frame or tick. A new build that fails to load keeps the old
instance running (one warning in the log). `YOAKE_WASM_DEV_DIR=<folder>` additionally loads every
`<id>.wasm` in the folder as a mod (no `.yak` needed) and reloads it the same way. Both are off
without the flag. The startup log has one line saying `wasm: no mods (engine not started)` or
`wasm: N mod(s), engine started`.

## Full-catalog mods (`client-full`, `server-full`)

The frozen `client-mod` world has 19 hooks. Two more worlds of `crates/yoake-wasm/wit/yoake-full.wit` cover
every hook of the native catalog (`coverage.json` says how typed each one is):

| world | exports | runs |
|---|---|---|
| `client-full` | every client-side and shared hook | in the client (the render thread) |
| `server-full` | every server-side and shared hook + `server-services` imports | in the integrated server (host) and in the dedicated server |

A hook of the game's shared code (`both` in the catalog) goes to the client module when it runs on the
client thread and to the server module when it runs on the server thread.

### Choosing the world in `yak.toml`

```toml
[wasm]
interface = "1.0"
client = "bin/wasm/demo-client.wasm"   # world client-mod (default) ...
world  = "client-full"                  # ... or the full client world
server = "bin/wasm/demo-server.wasm"   # always server-full
```

Both modules of a package share one identity (id, version, sha256 of the client module, else of the
server module). A package with `required_on_join = true` must match on host and guest.
A dedicated server (`--dedicated`) loads only server modules, from the packages listed in the
`yoake-mods.txt` of its `--game-dir` (a server-only package is fine there). Hot reload covers
`client-mod` modules only: a full module is loaded once per run. The server runs one world at a time
and a guest keeps its state in its instance, so each world's server starts with a fresh instance of the
server module (it runs `init` again when `server_about_to_start` fires for a later world). A package whose
client module fails to load does not run its server module either.

#### Limits every module runs under

* Memory 64 MiB, 1 memory, 8 tables of at most 65,536 elements, 16 instances; no threads.
* A hook call may run 20 ms (client) or 25 ms (server) of wall-clock time, checked every millisecond
  (a trap, fuel or time, disables the module; the limit is not tighter because a busy machine can pause
  a thread for a scheduler quantum, and a tight wall-clock limit disabled healthy mods); a call also has 10 million units of fuel, which bounds its CPU time first. All the calls of
  one module in a 50 ms window may use 10 ms (client) or 25 ms (server) of it;
  further calls in the window are skipped (a skipped hook answers its fallback) and counted, and a module
  that overruns in 20 windows in a row is disabled. `init` has 250 ms.
* Time spent inside a server service (the game breaking a block, reading a container, another
  package's `handle-call`) extends the call's deadline by at most 400 ms in all, including config and
  storage I/O. Config and storage I/O still count against the aggregate budget; other server-service
  time is the game's, not the guest's, and is excluded. The guest's own work stays bounded by fuel
  and the clock, and the native helpers allow 512 to 1024 calls per mod and server tick.
* A hook that finds its module busy (re-entered from the module's own service call) answers its fallback
  and is counted; the first drop is logged.
* A guest's answers are checked before they touch the game: floats must be finite, strings at most
  64 KiB, lists at most 65,536 elements, item stacks need a registered item id and a count between 1 and
  the item's stack limit, block states must exist, an NBT argument that cannot be encoded skips the hook,
  and the strings the engine must keep for the whole run (command names) are limited to 4096 strings of
  256 bytes and 256 KiB in total.

#### Permissions

`[wasm] permissions = ["service.commands"]` lets the server module call `run_command` (the package
installer/launcher should show it; the load log also says it). Without it `run_command` answers an
error. Even with it a command line goes to the game's own commands only (never to another mod's command
hook) and `transfer`, `fetchprofile`, `publish`, `unpublish`, `debug`, `jfr`, `perf`, `stop`, `save-off`,
`save-on`, `op`, `deop`, `whitelist`, `ban`, `ban-ip`, `pardon`, `pardon-ip`, `setidletimeout`, `test`,
`compute` and `pregen` are refused, also after `run` in `execute` and `return`.

### Writing one (SDK feature `full`)

```toml
yoake-wasm-sdk = { path = "...", features = ["full"] }
```

```rust
use yoake_wasm_sdk::full::{hook, server::{ServerMod, et, services}};
#[derive(Default)] struct M;
impl ServerMod for M {
    fn subscriptions() -> &'static [&'static str] { &[hook::PERFORM_COMMAND] }
    fn perform_command(&mut self, player: et::NativeServerPlayer, command: String) -> bool { /* true = take it */ false }
}
yoake_wasm_sdk::export_server_full!(M);
```

`yoake_wasm_sdk::full::client::ClientMod` / `export_client_full!` are the client twins. Contracts: cancel returns
`bool` (or `cancel` in an outcome record); replace returns an option; collect gets the aggregate and returns
it; `&mut` draw objects (`HudBatch`, `BlockEntityDraw`, `Corners`) become command lists you return; other
`&mut` values come back as `option` fields of the outcome. Engine objects that cannot be copied are `u64`
handles, valid only during the call. Example: `crates/yoake-wasm-sdk/examples/full-demo` (build once per
feature, `client` and `server`). A `.yak` with data content (`content/`: blocks, items, a mob effect, a recipe, a loot
table, a data-defined mob, a sound and a creative tab, see mods.md) and a small server module that reacts to the mob:
`crates/yoake-wasm-sdk/examples/content-demo` (`cargo build --release --target wasm32-wasip2`; the package id is
`content_demo`, a namespace, so `[a-z0-9_]` only).

### What the host applies / does not

* HUD commands: text (plain), rect, gradient (strips), flat quads, sprites, items, clip/scale/blend runs.
  Skipped: `texture`, `highlight`, textured quads.
* Model commands: push/pop/translate/rotate/scale/light/keep, quad, block. Skipped: cube, model, item.
* Corner reservations: applied. Frame-stage commands (`world-drawn`, `hand-drawn`, ...): not applied yet.
* Server services: `server-services` (`crates/horizon-mod-api-gen/abi-subset-server.json`) answers,
  except four (below). Each validates its arguments (finite numbers, sizes, registry ids, UUIDs) and fails
  closed (an `Err` changes nothing). Mods are trusted inside the game: they may change other players'
  inventories, break blocks and move items. The one hard line is the PC: storage is a per-package folder, commands
  are gated and denylisted, paths are relative to the game folder. `status` is `forwarded` (a native host
  service existed), `new` (the native host service was added for this; native mods may use it too: fragment
  `tools/native-hook-fragments/wasm_services.json`, `docs/native-mods.md` "Server helpers for WebAssembly services")
  or `deferred`. The SDK's `full::server::ext` has plain-argument wrappers for the common ones.

  | Services | Status |
  | --- | --- |
  | `acting-player`, `server-players` (now with `sneaking`), `integrated-owner`, `held` (whole stack), `player-stacks`, `player-teleport`, `player-set-health`, `player-set-gamemode`, `player-clear-inventory`, `player-effect-give/clear`, `player-attribute-modifier-remove`, `tell`, `action-bar`, `player-title`, `player-clear-title`, `player-play-sound`, `bossbar-*` (7), `block`, `block-property`, `set-block-property`, `block-collision`, `edit-blocks`, `block-entity-sync`, `spawn-mannequin`, `remove-spawned-entity`, `net-send-to-player`, `world-dir`, `config-dir`, `entities-in`, `clip`, `run-command` (`service.commands`, denylist) | forwarded |
  | `player-attribute-modifier-add`, `hurt-entity`, `server-player-motion`, `set-server-player-velocity`, `set-player-attack-hitbox`, `clear-player-attack-hitbox`, `world-border`, `set-world-border`, `open-menu`, `menu-set-items`, `menu-close` | forwarded (native service of the engine) |
  | `world-control` (time, game rule, force-load), `team-apply`, `particle-spawn` | forwarded as typed changes written as the game's own commands (every word validated; no `service.commands` needed) |
  | `world-storage-read/write`, `config-read/write` (legacy raw blobs: world/global confined stores below, require `service.storage`), `asset-read` (`data/<key>` of the game's resources), `qa-mode-enabled` (false), `qa-record` (debug log) | forwarded (WASM layer) |
  | `config-get/set`, `has-permission`, `storage-get/put/delete`, `storage-blob-begin/write/commit/abort/read` | new: typed, capability-gated package services (below) |
  | `play-sound-at` | new (WASM layer: every player in earshot) |
  | `break-block-as` (the game's own player break: drops with Fortune/Silk Touch, tool damage, XP, stats, mod events), `player-sneaking` (guests, the LAN host and single player's owner), `container-slots`, `container-set-slots` (same items, totals verified; plain containers, both halves of a double chest), `containers-in`, `player-set-slots` (36 main slots, totals verified), `player-remove-slot` (exact item, count and components; all-or-nothing across main, armor, off hand and cursor), `block-tags`, `item-tags`, `block-tool-requirement`, `can-harvest`, `attribute-get`, `item-entities-in`, `item-entity-move`, `item-entity-pickup`, `give-items` (remainder returned), `inventory-apply` (exact: id + count + components, all or nothing; `give` needs room for all), `advancement-progress`, `advancement-grant`, `entity-vitals-of` | new |
  | `mod-call` | new: `target-mod`, `method`, `payload` (1 MiB); the callee exports `handle-call(caller, method, payload)` of the `server-full` world; no call into a caller that is waiting (it is busy), at most 4 nested, never to itself |
  | `attachment-define`, `attachment-get`, `attachment-set`, `attachment-remove` (W2-B: opaque bytes, 64 KiB, on a player, entity, block entity, chunk or the level; persistent / synced / copy-on-death are the engine's own attachment policies, ids `<package id>:<name>`, see `docs/mods.md`) | new (server-full only; a client module cannot read synced values yet) |
  | `block-entity-read`, `block-entity-patch-call` | still missing: a block-entity hook returns its new state directly (no native service to forward) |
  | `range-override-set/clear` | deferred (Nestgate voice, W5) |

  `handle-call` is a new export of the `server-full` world: a module built before it cannot be loaded by
  this host (rebuild with the SDK). A service called from a read-only hook (the hook has no `&mut` host)
  answers `Err("unsupported: ...")` for gameplay services that need a mutable host. Package config and
  storage imports use the package service handle, so they also work from read-only hooks.
  `player-stacks` returns slot records: 0-35 main inventory, 36-39 armor (head to feet), 40 off hand,
  and 41 cursor. `entities-in` reads the level currently in place. Per-level hooks such as
  `level-tick-pre` and `level-tick-post` run with their dimension in place and include mobs after
  their tick. Server lifecycle hooks do not promise a per-level context. While other dimensions are
  parked, potion and eye-of-ender transients are omitted because those stores do not record a dimension.
* A guest that subscribes to a hook with no adapter is refused at load (`UNWIRED` in the generated code;
  `crates/horizon-client/src/wasm_mods/full_adapters_report.txt` lists them and the partial ones).
* The adapters (`full_adapters.rs`) are generated by `horizon-mod-api-gen` and **committed**, like the WIT.
  After a catalog or generator change run `YOAKE_UPDATE_WIT=1 cargo test --offline -p horizon-mod-api-gen -j 4`
  (a test fails when the committed file is out of date). The game's build does not generate them.
* A package takes part in the render pass only when its client module subscribed to a render-stage hook.
* The guest examples used by the tests are built by `yoake_wasm::test_guests` (feature `test-guests`) into
  `CARGO_TARGET_DIR/wasm-guests`, else `YOAKE_WASM_TARGET_DIR`, else a `target-wasm` folder next to the
  repository, else `target/wasm-guests`.
  With `YOAKE_CI=1` (or `CI`) a guest that cannot be built fails its test instead of skipping.

## Package config, permissions and storage

The generator consumes these declarations from `abi-subset-server.json`: ABI IDs 116–126 add the
services below, and hook ID 42 adds `config-changed(key, value)`. Config and storage are imported by
both `client-full` and `server-full`; `has-permission(node, player)` is server-only. The historical
`abi-subset-w2d-services.json` remains an unconsumed proposal, not the runtime contract.

Declare capabilities in `yak.toml`; every guest import checks them before touching config or disk:

| Capability | Authorizes |
| --- | --- |
| `service.config` | `config-get`, `config-set`, `config-dir`, and receiving synced config |
| `service.permissions` | `has-permission` for this package's declared permission nodes |
| `service.storage` | All `storage-*` imports and legacy `world-storage-*` / raw `config-read/write` |

Missing capability, invalid input, unavailable scope, or I/O failure returns the import's error result,
not a panic. The host-owned config screen is a user action, not a guest import, and does not require
granting the guest `service.config`.

For example:

```toml
[wasm]
permissions = ["service.config", "service.permissions", "service.storage"]

[config.enabled]
type = "bool"
side = "server"
default = true
label.en_us = "Enabled"
label.ja_jp = "有効"

[config.radius]
type = "int"
side = "synced"
default = 8
min = 1
max = 16

[config.mode]
type = "enum"
side = "client"
default = "simple"
values = ["simple", "advanced"]

[config.layers]
type = "list"
element = "enum"
side = "server"
default = ["surface"]
values = ["surface", "deep"]

[permissions."example.edit"]
default_op_level = 2
```

Types are `bool`, `int`, `float`, `string`, `enum` and non-nested `list`. Numeric ranges use the same
type as the value, and enum values must be declared. `comment` and `restart` are optional; restart is
advisory and the screen labels it. Names contain dot-separated alphanumeric, `_` or `-` segments.
There are at most 256 config keys, 128 bytes per key, 64 KiB per string and 4,096 list elements;
resolved values/defaults and each config document are bounded to 1 MiB in aggregate.

`ConfigValue` is a kind-tagged record, with separate bool/int/float/text/list payload fields; use the
matching payload and leave other fields empty/default. `config-set` checks the manifest type, range,
enum membership and side. Client modules can write `side = "client"`; server modules can write
`server` and `synced`. Client reads of `synced` values use the server snapshot, never a local override.
Persisted local settings live in `<game>/config/<mod-id>.toml`, with `[client]` and `[server]` tables.
Writes lock the config file, re-read its latest values to preserve the other side, and replace it
atomically. A concurrent writer gets an error instead of silently overwriting another process.
Loading defaults does not create a file. Unknown/removed keys are dropped with a warning and invalid
values fall back to their manifest defaults. An unreadable or malformed document is cached as a
failure and logged once; it does not disable storage or permissions, or other mods' snapshots.

The mod list's Configure button opens a generic screen for packages declaring `[config]`: bool
toggles, numbers with bounds, text, enum cycles and lists edited as TOML array contents (for example,
`"a,b", "quoted \"text\"", ""`). A package with native settings and `[config]` gets separate entries
for both screens. Labels use the current language's `label`, then `en_us`, then the key. Done validates
all values before saving; a bad field leaves the page and its draft open with an error, so it can be
corrected without losing other edits;
server/synced rows are read-only when connected to another server. There is no file-watcher: guest
writes and config-screen saves produce coalesced `config-changed` notifications at dispatch boundaries.
Writes made inside the hook are deferred to the next boundary rather than recursively dispatching.

Servers send only `side = "synced"` keys over `yoake:config_sync`, on join and after changes, to Horizon
clients. Snapshots are at most 1 MiB per mod, 256 mods / 8 MiB total; packets carry at most 28 KiB of
snapshot data and the sender pumps four packets per connection per tick. The receiver requires ordered,
complete packets and validates the entire snapshot before changing any values. Received values are
memory-only and reset to defaults on disconnect; config changes also reach subscribed client hooks.
Snapshots cache their revision and serialized bytes, and are rebuilt only when a synced value changes.
A precomputed eligible-mod flag skips per-tick sync entirely when no installed server module has
synced config. With no eligible mod, no config payload or config/storage file is created.

`has-permission(node, player)` requires an online player's UUID, a node declared by this package, and
an open world. It falls back to the node's `default_op_level` (0–4). Undeclared nodes return false.
`<world>/permissions.toml` can override defaults without modifying mods, for example:

```toml
[mods.example.players.'01234567-89ab-cdef-0123-456789abcdef']
'example.edit' = true

[mods.example.groups.builders]
'example.edit' = true

[mods.example.members]
'01234567-89ab-cdef-0123-456789abcdef' = ["builders"]
```

An explicit player rule wins; otherwise any matching group denial wins over group grants, then the
op-level default applies. UUIDs accept compact or hyphenated forms in either case. Parsed overrides
are cached; modification time and length are checked at most once per second and a change reloads the
bounded file. Missing files mean defaults; malformed files return an error rather than granting access.

`StorageScope` selects separate stores: `world` and `global` are server-only; `client` is client-only.
All calls are confined to the current package's store:

| Scope | Directory |
| --- | --- |
| world | `<world>/yoake-data/<mod-id>/` |
| global | `<game>/yoake-data/<mod-id>/global/` |
| client | `<game>/yoake-data/<mod-id>/client/` |

Each store has a 256 MiB quota, including serialized metadata and reserved pending uploads, and at most
65,536 keys. Entries use host-generated filenames and one atomic file per key, so a mutation writes
only that value and its metadata, not the whole store. Only keys and lengths stay resident; values and
blob slices are read on demand. Encoded writes, including config writes, share a per-mod token bucket
across scopes: 8 MiB per second sustained, with a 32 MiB burst. Storage operations charge at least
4 KiB each; larger reads also charge their requested available bytes. At most 64 storage operations
are allowed per 50 ms hook window. Exceeding either budget returns an error and counts toward
aggregate throttling; repeated over-budget windows disable the guest.
Config and storage I/O extend the bounded per-call deadline but still count against the aggregate
guest time budget. Store opening and migration happen when the package or world is bound, outside
guest calls.
Inline reads/writes and blob chunks are capped at 1 MiB. `storage-blob-begin` reserves quota and returns
a package-owned upload ID (at most 64 outstanding), with a separate **32 MiB aggregate pending-blob
limit per mod** across all scopes. Buffers grow lazily on write, not at begin. Writes may arrive out of order, but
commit requires complete coverage. A failed commit retains the upload for completion or abort.
`storage-blob-read` reads bounded slices of committed data. Uploads are not persisted and world uploads
are discarded when changing worlds or creating, dropping, disabling or trapping a module instance.
World storage first becomes available in `world_lifecycle(WorldOpenReady)`, stays bound through
shutdown callbacks and saving, and unbinds after explicit open-failure, final-close or close-failure
phases. It is unavailable in `server_about_to_start` and the initial level/chunk-load hooks before
`WorldOpenReady`; those hooks can use global storage instead. Starting another world first clears the previous binding.
Ordinary hooks and config notifications never change it.
Put, delete and successful blob commit persist before
reporting success. A store holds an exclusive lock for its lifetime; another process gets a clear
error. Writes use a synced temporary file and rename; stale temporary files are removed on open.
On Unix, renames also sync the containing directory; a sync failure after a successful rename is
logged without reverting the visible in-memory update. Loading a package with `service.storage`
creates its global or client directory, `store.lock` and migration marker; binding a world creates
the corresponding world directory and files. Failed global/client store opens retry on a later call
after a one-second backoff. With no mod installed, none of these directories or files are created.
Opening settings does not create directories. An unchanged config write creates no lock file;
changed config writes briefly retry a busy in-process lock.

At bind time, existing `store.bin` data and legacy `<world>/yoake-wasm/<mod-id>/<key>.bin`
or `<game>/config/yoake-wasm/<mod-id>/<key>.bin` entries are imported within quota, without replacing
newer keys. The `.bin` suffix is stripped in both cases; client stores import neither legacy source.
Corrupt and over-quota migration records are skipped with warnings, continuing with later records
when their boundaries are recoverable. A marker records completion only if no retryable I/O failure
occurred. Failed migrations retry when the store is reopened. Corrupt `entry-*.bin` files are renamed
to `*.bin.corrupt`, retaining at most 16 per store; excess quarantined or corrupt files are deleted.
Legacy files and `store.bin` are deliberately retained as recovery backups;
the marker prevents repeated imports. Legacy `world-storage-read/write`
and raw `config-read/write` now require `service.storage`; `config-dir` requires `service.config`.
Those imports use the same quota, locking and no-follow rules as new stores.

Keys are logical names, never paths. Traversal, absolute paths, Windows reserved names (including
reserved superscript digits), symlinks and Windows reparse points are rejected. The disk layer holds
directory handles on Windows and uses no-follow relative opens on Unix. The trusted game/world base
is canonicalized before checks below it, so a linked/junctioned base or `..` in `--game-dir` is allowed.
Unix ancestors use search-only handles (`O_PATH`/`O_SEARCH`). Permissions/config paths below the
trusted base are also checked for links/reparse points.

HTTP, general networking, chunk force-loading and the other W2D proposal entries are not connected by
this work. `HttpsPolicy`, transfer accounting, `ChunkTickets` and `RequestTickets` remain standalone
helpers, not transport implementations.
