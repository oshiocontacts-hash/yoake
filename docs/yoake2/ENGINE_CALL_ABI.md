# engine_call ABI notes (SDK-facing)

For guests (core WASM modules) that call generated engine bindings. Imports are in module `env`. Read this before
writing a retry loop: **there is no retry**.

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
| `-17 … -21` | handle errors: Stale, Locked, BadField, NoSuchArg, WrongType (nothing ran) |

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
