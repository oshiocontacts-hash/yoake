# engine_call ABI notes (SDK-facing)

For guests (core WASM modules) that call generated engine bindings. Imports are in module `env`. Read this before
writing a retry loop: **there is no retry**.

## ABI version

Every full-access guest declares its ABI in a WASM custom section named `yoake_abi`. Its payload is exactly four bytes:
the major version as a little-endian `u16`, then the minor version as a little-endian `u16`. The current engine and Rust
SDK version is **1.0**, encoded as `01 00 00 00`; this includes the hook modes and read-only arguments described below.
A WAT guest writes `(@custom "yoake_abi" "\01\00\00\00")`, and the Rust SDK's `export_guest!` emits the section.

The loader reads this section directly from the module bytes, without running guest code, and checks it before
`yoake_init` runs:

- No section, a payload of the wrong length, duplicate `yoake_abi` sections, or a malformed section: refuse the module.
- A different major version: refuse it as incompatible.
- The same major and minor version: load it.
- The same major and a newer guest minor: load it, but log that the mod expects a newer engine. It may use calls that
  this engine refuses with the unknown-binding status.
- The same major and an older guest minor: load it; minor additions are backwards compatible. Log at most an info line.

Adding a new import, binding, or status is a **minor** change. Changing or removing an existing import or binding,
changing a status's meaning, or changing the wire format is a **major** change.

## `engine_call(handle: i64, id: i32, ptr: i32, len: i32, out_ptr: i32, out_cap: i32) -> i32`

Runs binding `id` (ids come from `crates/yoake-bindgen/ids.lock.json`, append-only, never reused).

- `handle`: the handle of the receiver: argument 0 of the hook (`Root::HookArg`), or a *path handle* (below). Free
  functions ignore it.
- `ptr`, `len`: the encoded arguments (codec: little-endian numbers, 32-bit lengths, tags for `Option`/`Result`,
  `usize` as 64 bits). At most 1 MiB; more is refused without reading guest memory.
- `out_ptr`, `out_cap`: where the encoded result goes.

Return value:

| value | meaning |
|---|---|
| `>= 0` | the call ran; this many result bytes were written at `out_ptr` |
| `<= -257` | the call ran, the result did not fit `out_cap`. The result is kept; the **ticket** is `-ret - 256`. The size was written as a little-endian `u32` at `out_ptr` if `out_cap >= 4` |
| `-1` | arguments over 1 MiB (nothing ran) |
| `-3` | malformed arguments (nothing ran) |
| `-4` | the binding is a setter of a read-only field (nothing ran) |
| `-5` | unknown id |
| `-6` | the host registered no dispatcher |
| `-8` | the result did not fit and this activation already holds 8 kept results or 16 MiB: **the call ran, its result is lost**. Size the buffer first |
| `-17 … -23` | handle errors: Stale, Locked, BadField, NoSuchArg, WrongType, LimitReached, ReadOnly (nothing ran) |

An engine panic caused by the arguments is not a status: the guest traps, the mod is disabled (the trap rule), and the
host logs `engine state may be inconsistent` with the binding id.

### Never call again to retry

By the time the buffer turns out too small the engine call has **already run**. Calling again would run it a second
time (a setter, a spawn, a counter). So the result is kept under a ticket and you fetch it:

## `engine_take(ticket: i32, out_ptr: i32, out_cap: i32) -> i32`

Copies the kept result without running anything. Returns the byte count (the ticket is then spent), or `-2` (still too
small: the size is written at `out_ptr` when `out_cap >= 4`, the ticket stays), or `-7` (no such ticket).

A ticket is valid only inside the activation that got it and is dropped when that activation ends. If you do not
want to deal with tickets, size `out_cap` for the biggest result (the getters of one type are at most a few hundred
bytes; lists are bounded by the type).

## Path handles (`Root::Game`)

In a hook whose argument 0 is the `Game` root (the `yoake2_server_tick` hook; no other hook has one) the guest calls a
generated `Game.<name>#handle` binding with the root handle, for example `Game.item_entity#handle(id: u32) -> u64`, and
gets a path handle. Every bound method and field accessor of the type then works on it exactly as on a hook argument.

- The handle is a path (`Game`, field, id), re-borrowed from the root on **every** call, never a reference.
- It is valid only inside this activation (`Stale` after it). It is `Stale` when the object is gone (removed from the
  level), `Locked` while `original` or another call of this activation runs, `WrongType` for a method of another type.
- An activation holds at most 256 path handles.
- Inside any other hook (`ArmorStand::tick_settle`, ...) there is no root: the caller holds an outer `&mut`. Asking for a
  path there is `WrongType`.

## Hook multiplexing (MX3K1)

One guest, many hooks, thousands of ids: the guest exports **one** entry point and registers the ids it wants.

- `yoake_hook(hook_id: i32, handle: i64) -> i64`: called by the host for every registered hook; the guest picks the
  handler with a `match` on `hook_id` (a jump table in the guest; no name lookup, no per-hook export, so a guest with
  thousands of hooks has the same two exports as one with one hook). The result is the hooked function's result.
- `yoake_init() -> ()` (optional): called once at install. It calls the host import
  `hook_register(id: i32, priority: i32) -> i32` for each hook id it wants (the Rust SDK: `Registrar::hook(id, priority)`).
- The host keeps `id -> priority` and dispatches by id. A hook id the guest did not register costs one hash lookup and
  nothing else: no context is published, no activation opened, the guest is not entered, the original just runs.
- A guest with no `yoake_hook` export is the single-hook guest of the slice (one export named in `ModConfig`, hook id 0).
- Hook ids: S2 slot ids (the codemod ledger) are the small numbers; engine-synthetic hooks are at and above
  `0x8000_0000`. `0x8000_0001` is `Game::yoake2_server_tick`, the hook whose argument 0 is the `Game` root.
  `priority` is recorded and reported (`runtime::stats().registered`); ordering several mods by it is the loader's job.

## Hook modes and results

`hook_register` returns `0` for a **Full** hook (and also when the host has no bridge mode resolver), `1` for
**ObserveOnly**, and `2` for **NotCallable**. All three statuses mean that the id was registered. Negative statuses
mean that it was not registered: `-1` outside `yoake_init`, `-2` over the registration cap, and `-3` for reserved id
0. The Rust SDK exposes the non-negative outcomes as `Result<HookMode, RegisterError>` from `Registrar::hook`.

- **Full**: the guest's `i64` return value is decoded as the hooked function's result. Calling `original` runs the
  next guest in the chain, or the real original when this guest is last. Not calling it skips the rest of the chain
  and cancels the real original; the guest's return value replaces that result.
- **ObserveOnly**: the hooked function's result type cannot round-trip through `i64`, so the guest's return value is
  ignored and it cannot replace the result or cancel the call. Each guest may call `original` any number of times.
  An upstream guest's `original` runs the next guest, not necessarily the real original: the real original runs only
  when that call reaches the end of the live chain. If an upstream guest does not call `original`, downstream guests
  do not run during that activation. After the outermost guest returns, the bridge runs the real original directly if
  no chain call reached it; this fallback does not run skipped downstream guests. The real original runs **at most
  once** per call: when a later `original` reaches it again (the same guest calling `original` twice, or two guests of
  the chain), it is not run again and that call returns `0`; the first result is kept. A side effect of the original
  (call site 215 hurts an item) happens once.
- **NotCallable**: registration is harmless and is included in the loader's report, but this engine build has no
  callable bridge for the id, so its hook will not be entered.

For a Full hook, the original's result is encoded for `original`, and the guest's returned `i64` is decoded, as
follows: `()` is `0`; `bool` is `0` or `1`; signed integers and unsigned integers narrower than 64 bits use their
numeric value and must fit the result type; `u64` uses its two's-complement 64-bit pattern (every `i64` is a valid
`u64`); `usize` is passed to `original` as that pattern but a guest result must be a value in `0..=i64::MAX`; `f32` uses
its bits as a value in `0..=u32::MAX`; and `f64` uses its 64-bit pattern. A guest result outside the decodable range
(for example `2` for `bool`, a negative `u8`, or a negative `f32` bit pattern) is rejected. The bridge then uses the
original's result, running the original if it has not run, and counts the rejection in `bridge::out_of_range_count`.
That direct run skips the rest of the chain: when the guest that returned the bad value did not call `original`, the
guests after it do not run for this call. A guest that passes the original's own result through unchanged is never
out of range (a `usize` above `i64::MAX` comes back as the same negative `i64`; it is accepted and not counted).

## Read-only hook arguments

A hook whose argument 0 is `&T` / `&self` uses `runtime::dispatch_id_ref` with a `RefFieldTable<T>` (`get: fn(&T, ...)`),
emitted next to the mutable `FieldTable<T>` for types whose getters only read fields. Its handle supports `get`, and its
`original` receives the same shared argument; chain order, re-entry, the lending lock, original-call recording, and
trap fallback are the same as for mutable dispatch. `set` returns handle status `7` (`ReadOnly`) without changing the
argument. The direct `get` / `set` imports return `0` on success or the positive handle status on failure. A typed
mutable receiver requested by `engine_call` on a shared argument is refused as `-23` (`HANDLE_BASE - 7`), and the
binding does not run. This is distinct from `engine_call` status `-4`, which means a setter targets a field that is
read-only in the binding schema. A shared activation never turns its `&T` into `&mut T`.

So today a shared argument cannot be used with `engine_call` at all: every generated binding takes its receiver
through `with_arg` (`&mut T`), and that is refused. This is deliberate. A later `with_arg_ref` for `&self` bindings
must hand out only `&T` (never build a `&mut T` from the shared pointer), and only to bindings whose receiver is
`&self`.

## Sides and threads (MX3CL1)

The runtime is per thread: every thread that loads mods has its own set of instances, each in its own store. There are
two such threads, and a full-access mod says where it runs with `[wasm] side` (only valid with
`world = "full-access"`):

| `side` | loaded on |
|---|---|
| `server` (the default, also when the key is absent) | the simulation thread: integrated-server or dedicated-server world start (`load_world_mods`) |
| `client` | the window's thread, when a client session becomes ready (`load_client_mods`) |
| `both` | both threads, as two separate instances |

- The module is still declared as `wasm.server` (one core module for any side). `[side]` of the manifest must agree:
  `side.server = true` when the mod runs on the server, `side.client = true` when it runs on the client.
- **Same trust, same bytes.** The client side takes the same launcher-approved list (`--full-access-approved`) and the
  same rules as the server side: only a package whose bytes hash to an approved `local`/`catalog` entry loads; a server's or
  a friend's package never does. A package for the other side is skipped without a message (`Refusal::OtherSide`); it is
  judged (and reported) once, by the loader of its own side.
- **Lifetime.** Client mods load at `native_session_ready` (a client game only, once per session, after login) and are
  dropped at `native_session_left`; a new session loads them again from the approved list. `yoake_init` of a client
  instance runs on the client thread.
- **No shared state.** A `both` mod's two instances share no memory, no globals, no registrations, and a trap or a
  budget breach disables only the instance on that thread. To talk across threads a mod uses the game's own channels
  (packets, files); there is no cross-thread call.
- **Engine-dispatched hooks** (ids >= `0x8000_0000`, no generated bridge; `hook_register` says Full, or observe-only
  where noted, and the loader never lists them as unbridged):

  | id | name | thread | when | argument 0 | mode |
  |---|---|---|---|---|---|
  | `0x8000_0001` | `SERVER_TICK` | server | once per server tick | the server's `Game` root | Full |
  | `0x8000_0002` | `CLIENT_TICK` | client | once per client tick (`tick_remote`, after the client mods' own tick) | the client's `Game` root | Full |
  | `0x8000_0003` | `HUD_RENDER` | client | once per frame, after the vanilla HUD was built (`Game::hud`) | none yet | observe-only |

  A mod that registers a hook on a thread that never dispatches it (a server-side mod registering `CLIENT_TICK`) is not
  an error and is never entered. `CLIENT_TICK` has path handles like `SERVER_TICK` (generated `Game.<name>#handle`
  bindings resolve against the client's `Game`; mind that a client `Game` mirrors a server's world, it does not own it).
  `HUD_RENDER` carries a handle to nothing: it only gives the guest a point in the frame from which to call engine
  functions; it cannot draw into the HUD yet. The SDK constants are `hooks::CLIENT_TICK` and `hooks::HUD_RENDER`.
- **Cost with no mod.** Each dispatch is one thread-local read (the chain lookup finds no mod loaded on the thread) and
  the engine's original behaviour; without the `yoake2-slice` feature the call sites are empty functions.

## The Rust guest SDK

`crates/yoake-guest-sdk` (`no_std`, wasm32): `call` (with the ticket handling above, never a retry), `original`, `get`,
`set`, `report`, `Registrar`, the wire codec, a size-class allocator over `memory.grow` and a trapping panic handler, plus
`stubs::{entities, items, game_items}` generated by yoake-bindgen (typed functions with the engine's data types as guest
structs and enums, the same ids and wire bytes as the host glue).

It builds to a **core module** with the installed `wasm32-wasip2` target and no component wrapper:
`rustc --target wasm32-wasip2 --crate-type cdylib -C panic=abort -C linker=rust-lld -C linker-flavor=wasm-ld
-C link-arg=--no-entry -C link-arg=--export-memory -C link-arg=--export=yoake_hook -C link-arg=--export=yoake_init`
(see `crates/yoake-guest-sdk/tests/support/build.rs`). The module has no WASI imports.

## Registration rules, several mods, the loader (MX3L1)

- **`hook_register` works only while `yoake_init` runs.** Anywhere else it returns `-1`. A mod may register at most 4096
  ids (`-2` past that), and **hook id 0 is reserved** for the legacy single-export guest (`-3`): S2 slot ids start at 1,
  hook ids at and above `0x8000_0000` are engine-synthetic. `yoake_init` runs with a finite budget (50 million fuel,
  2000 epoch ticks when epoch is on); a guest that spins there fails to load and nothing is left loaded.
- **Several mods per thread.** Each full-access mod is its own store. For one hook id the registered mods run in order
  of `(priority, load order)`, lowest first, and each wraps the next: the first is outermost, its `original` runs the next
  mod, the last mod's `original` runs the engine. A mod that traps is disabled and drops out of every chain; the mods
  around it and the engine still run, once. `runtime::stats_of(id)` / `all_stats()` give per-mod stats,
  `take_reports_of(id)` per-mod reports.
- **Handles of one mod are its own.** A path handle or hook-argument handle belongs to the activation of one mod; another
  mod's activation of the same hook has its own handles.
- **Full-access packages** are `.yak` files with `[wasm] interface = "1.0"`, `server = "bin/wasm/<name>.wasm"` and
  `world = "full-access"` (a core module, no `client`, no `[wasm] permissions`, `side.server = true`). A manifest
  without that world is the sandboxed WebAssembly path, untouched; that path skips full-access packages explicitly
  (MX3L2, below). When a package may run is in "Loader hardening" below.
- **Never hold a guest borrow across `original`** (a `RefCell` guard, a `static mut` reference): the engine may call a hook
  that enters this guest again, the nested hook finds the borrow taken, panics, and a panic is a trap.

## Loader hardening (MX3L2)

**Permission is bound to content; the origin is metadata recorded on arrival.**

- The player's approval is an entry of `full-access.toml` (in the control folder, MX3L3 below):
  `[[allow]] id = "..." version = "..." sha256 = "<64 hex of the .yak file>"`. The loader hashes the package file; a
  package whose bytes differ from the approved ones (an update, a replaced file) is **not loaded** and reported as
  `ContentChanged` ("changed since it was approved ... approve it again"). Approving again replaces the old entry. The
  MX3L1 form (`[allow] id = true`) names no content: it is read, listed in the report and approves nothing.
- The origin of a package is **not** inferred from the folder. It is the record written when the package arrived,
  `yak-origins.toml` (control folder): `[[package]] sha256 = ... id = ... version = ... origin = "local" | "catalog" |
  "server" | "friend"` (+ `signer` for the catalog). `yoake2_mods::install_package` stores a file and records its origin;
  the player's own installs use `local`, catalog downloads `catalog` (with the signer), deliveries from a server or a
  friend `server` / `friend`. Nothing in this tree delivers full-access packages yet (the launcher extracts sandboxed
  packages into `yoake-mods/<id>/<version>/`, which this loader never reads), so a future delivery path **must** call
  `install_package` with its origin. The record is keyed by hash and distrust is sticky: bytes that arrived from a
  server or a friend stay `server`/`friend` when the same bytes are installed locally later (delete the record to start
  over). Only the game and the launcher write the file; a mod cannot.
- **No record, an unreadable record, or an origin word this build does not know is an unknown origin: never
  full-access** (`UnknownOrigin`). A `server` or `friend` package is never full-access either, even with its hash
  approved (checked before the signature and before the approval). Only `local` (approved) and `catalog` (approved, or
  signed by a key of the trust list) can run.
- **Duplicates**: when two `.yak` files of the folder carry the same manifest id, every full-access one of them is
  refused (`DuplicateId`, naming the other file); other mods load. Same bytes under two names count too.
- `load_world_mods` unloads the previous world's mods first; if one cannot be unloaded (its guest is running) it keeps
  it, loads nothing new and reports the error.

**The sandboxed path skips full-access packages.** `yak_mods::wasm_mods_from` (both sides) skips a manifest with
`world = "full-access"` with a log line, and `wasm_preflight` refuses it if asked directly, so the component loader
never sees the core module.

**Fuel policy of a full-access mod** (design 4b: no hard per-call limit, a watchdog only logs). `ModConfig::full_access()`:

| | tests' `ModConfig::new` | `ModConfig::full_access()` (the loader's) |
|---|---|---|
| fuel per activation | 1,000,000 (traps when spent) | **none**: an activation starts from a full tank (`INITIAL_FUEL`, refilled after every top-level activation) |
| trapping epoch (`epoch_ticks`) | off (configurable) | off |
| watchdog (`watchdog_ms`) | off (configurable) | **50 ms** of the guest's own time per activation (time inside `original` is not counted) |
| `yoake_init` | 50 M fuel, 2000 epoch ticks | the same: **still finite**; a spinning init fails the load |

The watchdog is an epoch deadline with a callback: when an activation's guest time passes the threshold, one
`yoake: mod <id>: a hook activation has run N ms of guest time ... it is not stopped` line goes to stderr, `Stats::slow_activations`
and `slowest_ms` count it, and the guest runs on with a fresh deadline; it **never traps** (the mod is not disabled). It
logs once per activation. Its ticker thread wakes every 5 ms (a trapping `epoch_ticks` budget keeps the 1 ms ticker);
`epoch_ticks`, if set, wins and traps as before. The remaining stoppers of a runaway guest are the process and the
player (a mod can hang the server thread; that is the price of full access).

**Depth cap**: `MAX_DEPTH` (16 nested activations) is **per mod**, each store counts its own; with several mods in one
chain the total nesting can be deeper than 16 (each level still costs native stack).

**Chains are cached per hook id** (`chain_for`): built on first use, dropped whenever a mod is loaded, unloaded or dies
(a generation counter), so the hot path is one hash lookup and an `Rc` clone. A mod's registrations change only
inside its `yoake_init`, before it is in the list.

## The launcher is the authority (MX3L3, MX3L4)

**The game never reads the control files.** `full-access.toml` (approvals: id + version + SHA-256 of the `.yak`) and
`yak-origins.toml` (arrival records) belong to the launcher and live in `<launcher data dir>/yoake-full-access/`
(`horizon_mods::yak::control::control_dir_in(backend::data_dir())`), outside the game root. At each launch the launcher
reads and checks them (`Control::approved_list(mods_dir)`) and hands the game **this launch's approved list**. A full-access
mod that rewrites the control files therefore changes nothing in the running game; the change appears at the next
launch, where the launcher compares the new list with the last one the player accepted (`ApprovedList::diff`) and shows
the difference before approving anything.

### What the launcher must pass to the game

| | |
|---|---|
| **What** | the approved list for this launch: one `[[package]]` per `.yak` in `<game root>/mods` whose SHA-256 is approved under its id **and** whose origin record is `local` or `catalog`. `Control::approved_list` builds it; `ApprovedList::write_for_launch(run_dir)` writes it |
| **Format** | TOML, UTF-8, at most 1 MiB: `format = 1`; `protected = ["<absolute path>", ...]` (the control folder and its `.checksum` file); `[[package]]` with `id`, `version`, `sha256` (64 lowercase hex of the `.yak` file), `origin` (`"local"` or `"catalog"`; any other value invalidates the whole file) |
| **Path** | a fresh file of this launch, outside the game folder, e.g. `<launcher data>/run/<launch id>/approved.toml` (`write_for_launch` creates the folder and refuses to overwrite). Passed on the game's command line as `--full-access-approved <absolute path>` (or `--full-access-approved=<path>`); the environment variable `YOAKE_FULL_ACCESS_APPROVED` is the same and the option wins. A command-line path is the simplest robust way on Windows: no inherited handle to keep alive across `CreateProcess`, and the launcher already builds the game command line |
| **Lifetime** | one launch. The file must exist and stay unchanged from process start until the game's first world start (the game reads it once there, keeps it in memory, and never reads it again). The launcher deletes the file and its folder when the game process exits and writes a new one next time |
| **No list** | no option, no variable, an unreadable or invalid file: nothing is full-access (a line is logged only when a full-access package is present) |
| **Game side** | loads a package only if the SHA-256 of its bytes equals an entry's `sha256` and its manifest id equals the entry's `id`, with the entry's origin; catalog signatures are still verified; duplicate ids still refuse both. The game registers the list file and every `protected` path with `control::protect` |

There is no `--full-access-dir` any more. `control.rs` is the launcher's library: `AllowFile`, `OriginFile`,
`Control::{create, open, approve, install_local, approved_list, accept_current_files}`, `install_package`, `ApprovedList`.

### The checksum file is a corruption check, not tamper protection

Each save of a control file is atomic (temp file, fsync best effort, rename) and then recorded in
`<control folder>.checksum` (the SHA-256 of both files; this was the "seal"). The launcher refuses a file that does not
match, has no recorded checksum, or is missing although one was recorded, and builds no list from it. Anything that can
write the control files can write the checksum too: it catches damage, half-written files and hand edits, not an
attacker. What protects the approvals is that the game never reads the folder and cannot reach it
(`control::guard`, below). `Control::accept_current_files()` (formerly `reseal`) records whatever is on disk; **the launcher
calls it only after the player confirmed a diff it showed** (for example the files changed behind its back), never to
make an error go away.

### Protected paths (`control::guard`)

`Control::open` and the game's list reader register the control folder, its checksum file and the list file as protected.
`guard(path)` refuses a path that is **inside** a protected path, **an ancestor** of one (a storage root that contains the
folder would reach it), or has a **`..` component** (refused outright, never resolved; verbatim `\?\` paths, where `..`
is an ordinary name, too). `yoake-wasm`'s `DiskStorage` canonicalizes its trusted base (the OS resolves links and `..`
there), checks the resolved base, the storage directory, the legacy import directories and every file it writes. The current
generated bindings take no paths; **the first binding that opens a file must call `guard` on the path it opens.**

### Remote packages

`install_package` puts `Origin::Server` / `Friend` packages into `<mods>/remote/<origin>/<sha256>.yak`, **named by their
hash** (the sender's name is not used, so a file pre-placed under it cannot block a delivery; the same package delivered
twice is fine; a file of that name with other bytes is refused). The folder is never scanned by the loader, so these take no
part in duplicate-id detection, and the control files are not touched. Local and catalog installs never overwrite an
existing file (an update removes the old one with `remove_package` first).

### Launcher entry points

`Control::install_local(mods_dir, source)` for "install a local .yak" (copies, records origin `local`),
`Control::approve(bytes)` for the player's approval, `install_package(.., Origin::Catalog, signer)` for catalog downloads.
A `.yak` dropped into the mods folder by hand has no record and stays not-full-access.

`chain_for` returns at once when no mod is loaded (no lookup, no allocation).
