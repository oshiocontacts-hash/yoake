# Writing a full-access mod for Yoake 2.0

This guide is for someone who wants to write a **full-access mod**: a WebAssembly module, packed in a `.yak` file, that
calls engine functions and hooks them, the way a Java mod would. A working starter is in
[`templates/full-access-mod`](../../templates/full-access-mod/README.md). The Japanese version of this guide is
[MOD-GUIDE.ja.md](MOD-GUIDE.ja.md).

Everything below is checked against the code of this tree (commit `4b707d438` and what the guide's own commit adds).
Where something could not be checked it is listed under "要確認" at the end instead of being guessed.

The reference documents are [ENGINE_CALL_ABI.md](ENGINE_CALL_ABI.md) (the ABI) and the SDK
(`crates/yoake-guest-sdk`). This guide does not repeat every status code; it tells you what you need to write a mod
and what goes wrong.

## 1. What a full-access mod is, and the trust model

A full-access mod is a WebAssembly **core module** (not a component) that the engine loads next to the game. It has two
entry points, `yoake_init` and `yoake_hook`, and one import module, `env`. It can:

- **call engine functions** through generated bindings (`engine_call`), on handles the engine gives it: a hook's
  argument, or a path from the `Game` root;
- **hook engine functions**: it registers hook ids in `yoake_init`, and the engine calls `yoake_hook` when one of
  those functions runs. A hook can run the original (`original`), replace its result, or skip it;
- **run on synthetic ticks** the engine provides: the server tick (the tick of the simulated world). (The client tick
  and the HUD render hook exist on another branch; see section 6.)

It is meant to feel like a Java mod: you drop the file in and play. The price is the same as with a Java mod. **A
full-access mod has the power of the engine itself.** The launcher's own confirmation says it plainly: the mod gets the
same power as the game, including changing your worlds and settings, reading and writing files on the PC and using the
network, and "Yoake cannot check what it does".

Who decides, in plain words:

- Only the **engine executable is signed**. A full-access mod is not.
- The **launcher is the authority.** The player installs the `.yak` (Add a .yak file…, or drag and drop), and then
  switches it to **Allowed** on the Mods page, after a confirmation.
- **Only those exact bytes run.** The approval records the mod id, the version and the SHA-256 of the `.yak` file. If
  you rebuild and the bytes change, the player has to allow it again. The launcher shows such a file as "The file
  changed since you allowed it · will not run until allowed again".
- **Packages from servers or friends are never full-access**, even with a matching hash. Neither is a file that was put
  into the `mods` folder by hand (it has no install record). Only a package the player installed from their own PC
  (`local`) or from the Yoake catalog (`catalog`, approved or signed by a trusted key) can run.
- The game never reads the launcher's control files. The launcher hands the game a list for each launch; a mod that
  rewrites the control files changes nothing in the running game (the change shows up at the next launch as a diff the
  player must accept).

So as an author: a player trusts **you** and **this file**. Keep your build reproducible if you can, and say what
your mod does.

## 2. Setup: toolchain and target

You need the Yoake source checkout (the SDK and the pack tool live in it) and the toolchain it pins.

- `rust-toolchain.toml` at the checkout root pins **rustc 1.100.0** with the targets `wasm32-unknown-unknown` and
  `wasm32-wasip2`. Inside the checkout rustup picks it up by itself. Outside it, call `rustc +1.100.0` (the template's
  scripts do, `YOAKE_TOOLCHAIN` changes it).
- The mod is built for **`wasm32-wasip2`** with the plain `rustc` (no Cargo needed, no component wrapper): the output
  is a core module with no WASI imports.
- **Windows with the GNU toolchain**: building the pack tool (`cargo-horizon-mod`) compiles crates that need `dlltool`
  and an assembler on `PATH`. The mod itself does not (it only needs `rustc` and the bundled `rust-lld`). The SDK
  build and the module build in the template need no native tools.

Nothing else is installed: no `wasm-tools`, no `cargo-component`, no `wasm-opt`.

## 3. Project layout

The starter template is the smallest layout that works:

```text
my-mod/
  yak.toml              the package manifest
  src/lib.rs            the mod (one file is enough; a no_std crate root)
  build.sh / build.ps1  compile the SDK, compile the mod, pack the .yak
  bin/wasm/<name>.wasm  build output (the path is named in yak.toml)
  dist/<id>-<version>.yak  the package
```

The module is a `#![no_std]` cdylib crate root compiled against the SDK `rlib`:

```rust
#![no_std]
extern crate alloc;

use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks, original};

fn init(r: &mut Registrar) { r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK"); }
fn hook(id: u32, root: Handle) -> i64 { let _ = original(root); 0 }

export_guest!(init, hook);
```

`export_guest!(init, hook)` emits the two exports (`yoake_init`, `yoake_hook`) and the ABI custom section
(section 13). The SDK provides the allocator (`alloc` works) and a panic handler that traps. There is no `std`.

A Cargo project also works if you set the same flags (`crate-type = ["cdylib"]`, `panic = "abort"`, the linker
arguments of section 8) and depend on `crates/yoake-guest-sdk` by path; nothing in this tree builds a mod that way, so
the template does not.

## 4. Writing `yoake_init` and hooks

### The two functions

`export_guest!(init, hook)` takes two functions:

| function | called | what it does |
|---|---|---|
| `init(r: &mut Registrar)` | once, when the mod is loaded (`yoake_init`) | registers the hook ids you want with `r.hook(id, priority)` |
| `hook(id: u32, handle: Handle) -> i64` | for every registered hook, every time it fires (`yoake_hook`) | does the work; `match id` to pick the handler; the result is the hooked function's result |

Hook ids: small numbers are **ledger ids** of engine functions and call sites; ids from `0x8000_0000` up are
engine-synthetic. In this tree `hooks::SERVER_TICK` (`0x8000_0001`) is the synthetic one. The ledger is in
`hook-ledger/` (`horizon_client.json`, `horizon_client_lib.json`): each entry has an `id`, the `key` (the Rust path of
the function), a `status` (`live` or `tombstone`) and a `kind` (`fn` or `callsite`). Register `live` ids only.
Which ids the engine can actually enter is decided by the generated bridges
(`crates/horizon-client/src/yoake_glue/hook_bridge_*.rs`: `BRIDGED`, `OBSERVE_ONLY`, `UNBRIDGED`); at this commit 201
ids are bridged (156 function hooks, 45 call-site hooks; 67 of them are `ObserveOnly`), in four modules: `entities`
81, `game::item_use` 29, `mobs` 68, `motion` 23. `hook_register` tells you at load time (see modes below).

### `Registrar::hook` and modes

`r.hook(id, priority)` returns `Result<HookMode, RegisterError>`:

| `HookMode` | meaning |
|---|---|
| `Full` | your return value is the hooked function's result; not calling `original` cancels the call |
| `ObserveOnly` | the function's result type does not fit in an `i64`; your return value is ignored and you cannot cancel the call |
| `NotCallable` | registered, but this engine build has no way to call the id: the hook will never run (the loader says so) |

| `RegisterError` | meaning |
|---|---|
| `NotInInit` | `hook_register` works only while `yoake_init` runs |
| `TooMany` | at most 4096 ids per mod |
| `ReservedId` | id 0 is reserved |
| `Unknown(n)` | any other status |

`priority`: lower runs first. For one hook id, the mods that registered it run in order of `(priority, load order)`;
the first is outermost and its `original` runs the next mod, and the last mod's `original` runs the engine.

An example that registers three hooks and reports their modes (from
`crates/yoake-guest-sdk/examples/tick_and_item_use.rs`):

```rust
fn init(r: &mut Registrar) {
    let tick_mode = r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK");
    let full_mode = r.hook(HOOK_START_USING_ITEM, 0).expect("register start_using_item");
    let observe_mode = r.hook(HOOK_EXPERIENCE_GIVE, 0).expect("register Experience::give");
    // ...
}
```

### `original` and results

`original(handle) -> Result<i64, Error>` runs the next mod in the chain, or the engine's function when you are last.
**Every handle of this activation is `Locked` while it runs**, so read what you need before, or ask for a fresh path
handle after.

In a **Full** hook:

- call `original`, then return its value (pass it through) or your own `i64` (replace it);
- return without calling `original` to **cancel** the call and skip the rest of the chain; your value is the result;
- the value must fit the result type: `()` is `0`; `bool` is `0` or `1`; integers narrower than 64 bits use their
  numeric value; `u64` uses its two's-complement pattern; `f32` its bits as a value in `0..=u32::MAX`; `f64` its
  64-bit pattern. A value out of range (for example `2` for a `bool`) is rejected: the engine uses the original's result
  instead and prints `yoake: a mod returned N for hook ID, out of range for its result type; the original result is
  used`;
- calling `original` twice normally runs the engine's function twice, **except** in the hooks whose function takes an
  argument by value that the bridge has to move (ids 16, 61, 62, 130, 168, 172, 175, 433, 434, 473, 484, 501, 506,
  515 in this build): there the function runs at most once and a second `original` returns `0`. The mode does not
  show this (they are `Full`); the list with the function names is in
  [ENGINE_CALL_ABI.md, "Hooks whose original runs at most once"](ENGINE_CALL_ABI.md#hooks-whose-original-runs-at-most-once).
  If you need the result twice, keep the first one instead of calling `original` again.

```rust
HOOK_START_USING_ITEM => {
    // Full mode: pass the original's value through, or replace it with your own i64.
    let result = original(handle).unwrap_or(0);
    result
}
```

In an **ObserveOnly** hook you can read before and after, but your return value is ignored and the call cannot be
cancelled; if you never call `original`, the engine runs the real function once after you return. Each guest may call
`original` any number of times, but the real original runs **at most once** per call: later calls return `0` and the
first result is kept.

```rust
HOOK_EXPERIENCE_GIVE => {
    let before = entities_experience_total_get(handle).unwrap_or(-1);
    let _ = original(handle);
    // `handle` is Locked only while `original` is running; afterwards it is usable again.
    let after = entities_experience_total_get(handle).unwrap_or(-1);
    0
}
```

### Traps and rules that cost mods

- **A trap disables the mod.** A panic, an out-of-bounds access, `unreachable`, an engine panic caused by your
  arguments: the guest traps and the mod is dropped from every chain for the rest of the session. The mods around it
  and the engine still run, once.
- **Never hold a borrow across `original`.** The engine may call a hook that enters your guest again (a nested
  activation of the same instance). A `RefCell` guard or a `static mut` reference held over `original` is borrowed
  when the nested hook runs; it panics, and a panic is a trap. Copy what you need out, then call.
- **Keep state in atomics** (the examples use `AtomicU32`). The guest is single-threaded, but hooks can nest, so avoid
  anything that needs a borrow guard.
- **`hook_register` only inside `yoake_init`.**
- **Handle ids are per mod and per activation.** Do not store a `Handle` in a static; it is `Stale` the next time.
- **Do not call `report` on every tick.** `report(slot, value)` is a test aid: the host appends the pair to an
  in-memory list that only tests read (`runtime::take_reports`); nothing in the game drains it. The template reports
  the first three ticks only.
- **`yoake_init` is finite**: 50 million fuel and 2000 epoch ticks. A spinning init fails the load. Do the real work in hooks.
- **Hot paths cost.** A WebAssembly hook adds on the order of 110-150 ns per call (and more with arguments); do not
  hook something that runs hundreds of thousands of times per tick (the engine team's rule of thumb in
  `reports/MX3S2.md`).

## 5. Calling engine functions

### Typed stubs

You rarely call `engine_call` yourself. `yoake_guest_sdk::stubs` has generated typed functions, one per binding:

| module | contents |
|---|---|
| `stubs::entities` | getters and setters of entity fields and their methods, e.g. `entities_experience_total_get`, `entities_itementity_fire_ticks_set`, `entities_itementity_position_get` |
| `stubs::items` | item data types and item functions |
| `stubs::game_items` | the `Game.<name>#handle` functions that open a path handle from the `Game` root |

The names follow `<module>_<type>_<field>_get` / `_set` for fields and `<module>_<type>_<method>` for methods. A list of
what is bound is in `docs/yoake2/bindgen/` (`functions-*.md`, `coverage.md`) and the ids are in
`crates/yoake-bindgen/ids.lock.json` (append-only, never reused). A stub returns `Result<T, Error>`.

```rust
// from examples/engine_call_game_path.rs
let item = game_items_game_item_entity_handle(root, 4242)?;          // a path handle
let pos = entities_itementity_position_get(item)?;                   // a compound field (Vec3)
let ok = entities_itementity_fire_ticks_set(item, 55).is_ok();       // a setter
```

`Error` is:

| `Error` | meaning |
|---|---|
| `Handle(code)` | the handle is not usable: 1 `Stale`, 2 `Locked`, 3 `BadField`, 4 `NoSuchArg`, 5 `WrongType`, 6 `LimitReached`, 7 `ReadOnly`. Nothing ran |
| `Status(n)` | the call was refused or failed: the raw negative status (-1 args over 1 MiB, -3 malformed arguments, -4 setter of a read-only field, -5 unknown binding id, -6 no dispatcher, -8 result lost, ...). See ENGINE_CALL_ABI.md |
| `Decode(_)` | the result bytes did not decode as the stub's type |

`err.is_stale()` is true for `Handle(1)`. An error is **not** a trap: handle it and carry on.

### Handles and `Game` paths

A handle is the receiver of a call. There are two kinds:

- **A hook argument**: the first argument (argument 0) of the hooked function, given to `yoake_hook` as `handle`.
  Typed getters and setters of that type work on it.
- **A path handle**: only in the hook whose argument 0 is the `Game` root, which is `hooks::SERVER_TICK`. You call a
  `Game.<name>#handle` binding with the root and get a handle:

  | binding (SDK function) | opens |
  |---|---|
  | `game_items_game_experience_handle(root)` | the experience object |
  | `game_items_game_item_entity_handle(root, id)` | a dropped item by entity id |
  | `game_items_game_armor_stand_handle(root, id)` | an armor stand by id |
  | `game_items_game_cushion_handle(root, id)`, `..._experience_orb_handle`, `..._item_frame_handle`, `..._painting_handle` | the same for those types |

  A path handle is a **path** (`Game`, field, id) that is re-borrowed from the root on every call, never a reference.
  It is valid only inside this activation; it is `Stale` when the object is gone (an item picked up or removed), `Locked`
  while `original` or another call of this activation runs, and `WrongType` for a method of another type. At most 256
  per activation. Inside any other hook there is no root, and asking for a path is `WrongType`.

### `OUT_TOO_SMALL` and tickets

You normally never see this, because the SDK's `call` handles it. It is worth knowing why:

- The raw import is `engine_call(handle, id, ptr, len, out_ptr, out_cap) -> i32`. A non-negative result is the number of
  result bytes written.
- If the result **did not fit** `out_cap`, the call **has already run** and its result is kept under a **ticket**. The
  return value is `-(ticket + 256)` or lower, and the needed size is written as a little-endian `u32` at `out_ptr`.
  Fetch the result with `engine_take(ticket, out_ptr, out_cap)`.
- **Never call again to retry.** A second call would run a setter, a spawn or a counter a second time.
- The SDK does this for you: `call` first tries a 256-byte buffer, and on `OUT_TOO_SMALL` allocates exactly the needed
  size and takes the ticket. A ticket is valid only inside its activation. An activation holds at most 8 kept results
  or 16 MiB; past that the call returns status `-8` (`Error::Status(-8)`): **the call ran and its result is lost.**
  Only a hand-written guest that keeps several big results at once can reach this.

## 6. Client vs server modules

A full-access package declares a `[wasm] server` module, a `[wasm] client` module, or both (MX3CL1, report
`docs/yoake2/reports/MX3CL1.md`):

| declared | runs on |
|---|---|
| `[wasm] server = "bin/wasm/x.wasm"` | the simulation thread, at world start |
| `[wasm] client = "bin/wasm/y.wasm"` | the window's thread, when a client session becomes ready |
| both | both threads, as two separate instances (no shared memory or globals) |

`[side]` must agree exactly with the declared modules (`side.server` iff `wasm.server`, `side.client` iff
`wasm.client`), and the client side takes the same approved list and the same rules. The synthetic hooks are
`hooks::SERVER_TICK` (`0x8000_0001`, every server tick, argument 0 the server's `Game` root), `hooks::CLIENT_TICK`
(`0x8000_0002`, once per client tick, argument 0 the client's `Game`) and `hooks::HUD_RENDER` (`0x8000_0003`, once per
frame after the HUD, observe-only, argument 0 a dead handle: it cannot draw into the HUD yet). `hook_register`
returns `0` (Full) for the two ticks and `1` (observe-only) for `HUD_RENDER`.

## 7. A complete small mod

This is the template's `src/lib.rs` (shortened). It registers the server tick, makes engine calls on the experience
object, reports the first three ticks, and lets the tick run.

```rust
#![no_std]
extern crate alloc;

use core::sync::atomic::{AtomicU32, Ordering};
use yoake_guest_sdk::stubs::entities::entities_experience_total_get;
use yoake_guest_sdk::stubs::game_items::game_items_game_experience_handle;
use yoake_guest_sdk::{Error, Handle, Registrar, export_guest, hooks, original, report};

static TICKS: AtomicU32 = AtomicU32::new(0);

fn init(r: &mut Registrar) {
    r.hook(hooks::SERVER_TICK, 0).expect("register SERVER_TICK");
}

fn hook(id: u32, root: Handle) -> i64 {
    if id != hooks::SERVER_TICK {
        return original(root).unwrap_or(0);
    }
    let tick = TICKS.fetch_add(1, Ordering::Relaxed) + 1;
    // two engine calls, BEFORE `original`: handles are Locked while it runs
    let total = game_items_game_experience_handle(root).and_then(entities_experience_total_get);
    if tick <= 3 {
        report(1, i64::from(tick));
        report(2, total.map_or(-1, i64::from));
    }
    let _ = original(root);
    0
}

export_guest!(init, hook);
```

## 8. Building to wasm

Two `rustc` invocations, taken from `crates/yoake-guest-sdk/tests/support/build.rs` (what the engine's own tests use):

```sh
COMMON="--edition 2024 --target wasm32-wasip2 -C panic=abort -C opt-level=1"

# 1. the SDK as an rlib
rustc +1.100.0 $COMMON --crate-type rlib --crate-name yoake_guest_sdk \
  --out-dir build/sdk  <checkout>/crates/yoake-guest-sdk/src/lib.rs

# 2. the mod as a core module (no component wrapper)
rustc +1.100.0 $COMMON --crate-type cdylib --crate-name my_full_access_mod \
  -C linker=rust-lld -C linker-flavor=wasm-ld -C strip=symbols \
  -C link-arg=--no-entry -C link-arg=--export-memory \
  -C link-arg=--export=yoake_hook -C link-arg=--export=yoake_init \
  -L dependency=build/sdk --extern yoake_guest_sdk=build/sdk/libyoake_guest_sdk.rlib \
  -o bin/wasm/my_full_access_mod.wasm  src/lib.rs
```

The flags that matter: `wasm32-wasip2` with `-C linker=rust-lld -C linker-flavor=wasm-ld` skips the component
wrapper; `--no-entry` because there is no `main`; `--export-memory` and the two `--export=` flags make the exports the
loader needs; `-C panic=abort` because panics trap. The module has no WASI imports. `build.sh` / `build.ps1` in the
template do exactly this, then pack.

## 9. Packing a `.yak`

### `yak.toml`

```toml
format = 1
id = "my_full_access_mod"
version = "0.1.0"
name = "My Full-Access Mod"
authors = ["Your Name"]
description = "What it does."

[yoake]
version = ">=0.0.0"

[side]
client = false
server = true
required_on_join = false

[wasm]
interface = "1.0"
server = "bin/wasm/my_full_access_mod.wasm"
world = "full-access"
```

Rules the manifest checker enforces for a full-access mod:

| field | rule |
|---|---|
| `format` | `1` |
| `id` | 3 to 48 characters of `a-z`, `0-9`, `_`, `-` |
| `version` | semantic version |
| `authors` | 1 to 128 names |
| `[yoake] version` | a version requirement the launcher checks against the installed Yoake (`>=0.0.0` always matches) |
| `[wasm] world` | `"full-access"` |
| `[wasm] interface` | `"1.0"` (major must be 1; a minor of 1 or higher needs another world, so use `1.0`) |
| `[wasm] server` | a file **directly** under `bin/wasm/` ending in `.wasm`; required in this tree; no `wasm.client` (section 6) |
| `[wasm] permissions` | not allowed: a full-access mod is all-or-nothing |
| `[wasm] worker` | optional: a component directly under `bin/wasm/` (section 14) |
| `[wasm] host_filesystem` | optional boolean: `/host/<letter>` preopens (section 14) |
| `[side]` | `server = true` |

Optional keys include `name_ja`, `license`, `homepage`, `dependencies`, `optional`, `conflicts`. A `README.md`,
`LICENSE`, `content/`, `assets/`, `shaders/` and `config/` next to `yak.toml` are packed too.

### Pack it

There is a pack tool in the checkout: `cargo-horizon-mod`. From the checkout:

```sh
cargo run -p cargo-horizon-mod -- pack path/to/my-mod
```

It reads `yak.toml`, collects the declared files and writes an **unsigned** `<id>-<version>.yak` into the project
folder. It refuses to overwrite an existing file (the template's scripts delete the old one and move the new one to
`dist/`). It prints a summary line that says "data-only package": that wording refers to native binaries (`[bin]`), and
a package with a WebAssembly module is still labelled that way. To check the result:

```sh
cargo run -p cargo-horizon-mod -- yak verify path/to/my_full_access_mod-0.1.0.yak
```

Do not use `yoake build`/`pack`/`check` (the `yoake` CLI, `crates/yoake-cli`) for a full-access mod: it builds
components with Cargo for the sandboxed WebAssembly path and checks server modules as sandboxed components.

## 10. Installing and allowing it in the launcher

1. Open the **Mods page** of the Yoake instance in the launcher.
2. **Add a .yak file…** and choose your `.yak`, or drag the file onto the page ("Drop a .yak file here"). The launcher
   copies it into `<instance>/mods/` and records that it came from this PC (`local`). It is **not allowed yet**.
3. A **Full-access mods** section appears with a row for your mod, tagged "Full access". It reads "Not allowed · will
   not run".
4. Switch it on. A confirmation shows the mod's name, version, the first 12 hex digits of the file's SHA-256 and the
   origin, and warns that the mod gets the power of the game. Choose **Allow**.
5. The row reads **Allowed**. The launcher builds the approved list when it **starts the game**, and the game reads it
   once at the first world start, so start (or restart) the game from the launcher after allowing.

Updating your mod: a different version of the same id is refused ("already installed in another version; remove it
first"), so **Remove** the row, add the new file and allow it again. The same file name with new bytes is "changed since
you allowed it". Allowing is also what the player does on every rebuild during development.

If a row says "Allowed · runs after you accept the changes above", the launcher is showing a panel "Full-access changes
since you last accepted"; the player reviews the list (Added / Changed / Removed) and chooses **Accept these changes**.
This is how changes that did not come from the player's own click (a control file changed, a file was dropped into
`mods`) are held back.

## 11. Debugging

There is no `print` for a guest. The tools you have:

- **The engine's own lines.** Everything the loader and the runtime have to say about your mod is a line starting
  `yoake:` (printed to the game process's stderr; the loader's messages also go to the `log` crate at `warn` level).
- **Return values.** A stub returns `Result`: look at the `Error`. In a test, map errors to numbers and `report` them
  (see the template's `error_code`).
- **Run it in a harness without the game.** `crates/horizon-client/tests/yoake2_template.rs` builds the template, packs
  it, installs and approves it, loads it with the real loader and drives the server tick on a stand-in `Game` root; copy
  it for your mod. `report` values come back from `runtime::take_reports()`.

  ```sh
  cargo test --offline -j 4 -p horizon-client --features yoake2-slice,yoake-hook-bench --test yoake2_template
  ```

### Lines you can see

| line | meaning |
|---|---|
| `yoake: full-access mod <file> not loaded: <reason>` | the loader refused a package; the reasons are in the next table |
| `yoake: full-access mod '<id>' was built for ABI M.m, this engine speaks M.n: engine calls it makes that this engine does not know are refused` | your guest's minor version is newer than the engine's |
| `yoake: full-access mod '<id>' registered hooks this engine build cannot call, they will never run: <id> (<why>)` | `NotCallable` hooks: the id has no bridge in this build (for example `arg0 is &self, needs &mut`) |
| `yoake: mod <id>: a hook activation has run N ms of guest time (watchdog: 50 ms); it is not stopped` | one activation used more than 50 ms of your guest's own time; it keeps running, nothing is disabled |
| `yoake: engine state may be inconsistent: binding N panicked on guest-supplied arguments (...); the mod is disabled` | an engine function panicked on arguments you gave; your mod trapped and is disabled |
| `yoake: a mod returned N for hook ID, out of range for its result type; the original result is used` | a Full hook's value does not fit the result type |

### Why a package is not loaded

| the loader says (`Refusal`) | the launcher row says | what to do |
|---|---|---|
| nobody approved it | "Not allowed · will not run" | switch it to Allowed |
| the package changed since it was approved (approved X, this file is Y) | "The file changed since you allowed it · will not run until allowed again" | allow it again |
| no origin is recorded: a package of unknown origin is never full-access | "Cannot run as full-access: This file was put into the mods folder by hand ..." | Remove it, then add it with Add a .yak file… |
| delivered by a server / by a friend: never full-access | "A mod delivered by a server or a friend cannot be full-access." | it cannot be allowed; install a copy from your own PC |
| another package has the same id | "Two files have this mod id · neither will run" | remove the one you do not want |
| signature does not verify | (catalog packages only) | not for local installs |
| malformed package | "This is not a valid .yak file." | repack; run `yak verify` |
| could not start: ... | (loaded, then failed) | the module did not instantiate, or `yoake_init` trapped, ran out of its 50 M fuel, or `hook_register` was refused: read the text after "could not start" |
| incompatible ABI: ... | (loaded, then refused) | the `yoake_abi` section is missing, malformed, duplicated, or of another major version (section 13) |
| the "Full-access settings failed their check" banner | "Waits for the settings check above" | the launcher's control files do not match their checksum; the player must review the shown diff |

## 12. Limits

| limit | value |
|---|---|
| `yoake_init` budget | 50 million fuel and 2000 epoch ticks; failing it fails the load |
| per-activation fuel | none (a full tank each activation) |
| watchdog | logs after 50 ms of guest time in one activation; **never traps** |
| nesting depth | 16 nested activations **per mod** |
| `hook_register` | at most 4096 ids per mod; id 0 reserved; only inside `yoake_init` |
| `engine_call` arguments | at most 1 MiB |
| kept results (tickets) | at most 8 per activation and 16 MiB together |
| path handles | at most 256 per activation |
| module size | a `.wasm` entry over 32 MiB is refused by the package reader |
| trap | the mod is disabled for the session; others and the engine continue |
| shared (`&self`) arguments | read-only: `get` works, `set` returns handle status 7 (`ReadOnly`); a typed mutable call on a shared argument is refused as `-23` and does not run |

A runaway guest cannot be stopped by the watchdog: the remaining stoppers are the process and the player. That is the
price of full access, so keep hooks short.

## 13. Versioning: the ABI section

Every full-access guest declares its ABI in a custom section named `yoake_abi`: exactly four bytes, the major version
as a little-endian `u16` and then the minor version as a little-endian `u16`. The SDK and engine are at **1.0**
(`01 00 00 00`); `export_guest!` emits it (`ABI_MAJOR`, `ABI_MINOR` in the SDK). The loader reads it from the module
bytes **before any guest code runs**:

| guest section | result |
|---|---|
| missing, wrong length, duplicated or malformed | refused |
| another major version | refused as incompatible |
| same major and minor | loaded |
| same major, newer guest minor | loaded; a line says the mod expects a newer engine and calls the engine refuses with "unknown binding" may fail |
| same major, older guest minor | loaded (minor additions are backwards compatible) |

Adding an import, binding or status is a minor change; changing or removing an existing import or binding, changing a
status's meaning or the wire format is a major change. Binding ids are append-only and never reused, so a built mod keeps
working across engine updates of the same major version.

A mod's own `version` in `yak.toml` is separate: bump it for each release, and expect the player to allow each new file.

## 14. IME for mod screens (sandboxed `client-full`, not full-access)

The sandboxed component API has IME support for a mod-owned screen. It is not exposed to full-access core modules:
there are no ledger hook ids or `engine_call` bindings for it in this tree. Do not register service ids 201/202 as
full-access hook ids; those ids belong only to the sandboxed client-service catalog.

A `client-full` mod subscribes to `hook::SCREEN_INIT_POST`, `hook::FRAME`, `hook::IME_PREEDIT`, and
`hook::IME_COMMIT`. The services need write access to the host, which `screen_init_post` does not have (the calls
return `Err` there), so the mod notes the screen opening and calls them from `frame`, retrying until they succeed. On
its own open mod screen, it enables input and places the candidate window:

```rust
use yoake_wasm_sdk::full::client::{ime_set_allowed, ime_set_cursor_area};

fn screen_init_post(&mut self, _screen: et::NativeScreenView) {
    self.pending = true; // IME state resets each time a mod screen opens
}

fn frame(&mut self, _elapsed: u64) {
    if self.pending
        && ime_set_allowed(true).is_ok()
        && ime_set_cursor_area(32.0, 48.0, 180.0, 20.0).is_ok()
    {
        self.pending = false;
    }
}

fn ime_preedit(&mut self, text: String, cursor: Option<(u32, u32)>) {
    // Draw `text` in the mod's text field. Cursor offsets are UTF-8 bytes.
}

fn ime_commit(&mut self, text: String) {
    // Insert the committed text once.
}
```

`ime_set_allowed` is service **201**, and `ime_set_cursor_area` is **202**. The rectangle is `(x, y, w, h)` in
GUI-scaled coordinates, not physical window pixels. Coordinates must be finite and the size nonnegative. The services
return `Err` unless the caller owns the currently open mod screen. The state resets whenever a mod screen opens or
closes. A mod screen whose mod never calls either service behaves exactly as before (IME on for a title-menu mod
screen, off for an in-game one); once it calls one, the screen follows the mod. No `[wasm] permissions` entry is
needed or accepted for `service.ime`.

**Argument order trap:** the generator sorts service arguments alphabetically, so the raw WIT import
`services::ime_set_cursor_area` takes `(h, w, x, y)`. Always call the SDK wrapper
`yoake_wasm_sdk::full::client::ime_set_cursor_area(x, y, w, h)`, which reorders them.

Preedit and commit hooks run only on an open mod screen and only for subscribers. Empty preedit clears the composition;
`Ime::Disabled` sends `("", None)`. Commit arrives before the existing per-character `screen_char_typed_pre` hooks, which
still run: use one route for insertion, not both. See `docs/guide-ja/10-ime.md` and
`crates/yoake-wasm-sdk/examples/ime/` for the complete sandboxed example.

## 15. Files, network and workers (FR-F2)

Yoake mods are used inside one group, so a mod may do what a Forge or Fabric mod can: use files and the network. Two
rules stay: with no mod installed nothing changes, and **the game thread never waits for a mod**. Files are short and
synchronous, so a mod may use them on the game thread. Network can wait for seconds, so it goes through a **worker**.

This section holds for both kinds of mod: full-access core modules (this guide) and sandboxed components
(`docs/yoake-wasm-dev.md`).

### Folders

| guest path | host folder | access |
|---|---|---|
| `/data` | `<game dir>/mod-data/<mod id>/` (created on first start) | read and write |
| `/game` | the instance game directory | read and write |
| `/host/<letter>` | every local drive root, for example `/host/c` (`/host/root` on Unix) | read and write; **only** with `[wasm] host_filesystem = true` |

Persistent storage is just files under `/data`; there is no separate storage API.

Full-access mods are built with Rust `std` for **`wasm32-wasip1`** to use `std::fs`:

```sh
rustup target add wasm32-wasip1
rustc --edition 2024 --target wasm32-wasip1 --crate-type cdylib -C panic=abort -o my_mod.wasm src/lib.rs
```

The guest still exports `yoake_init` and `yoake_hook` and imports the usual `env` functions. A `no_std` guest that
imports nothing from `wasi_snapshot_preview1` loads exactly as before. What the WASI imports give you:

- `println!` and `eprintln!` become log lines `wasm mod <id> [stdout]: ...` and `[stderr]: ...` (the same lines the
  sandboxed route writes).
- Wall clock, monotonic clock and random work.
- **Waiting is limited.** `thread::sleep` (WASI `poll_oneoff`) with a wait over 2 ms traps the mod; a shorter wait
  returns at once. File calls count against the hook's time (the watchdog logs a slow hook). Copy big files in a worker.

### Workers

A mod may declare a worker, a WebAssembly **component** that the engine runs on its own OS thread:

```toml
[wasm]
interface = "1.0"
world = "full-access"
server = "bin/wasm/my_mod.wasm"
worker = "bin/wasm/my_worker.wasm"   # a component, world `worker-mod`
host_filesystem = false               # optional
```

The worker has full WASI preview 2: files as above, `wasi:sockets` (TCP, UDP, name lookup), `wasi:http` outgoing
requests, blocking waits and real clocks. It has **no engine handles**. It talks to its mod only through byte messages.
The WIT is `crates/yoake-wasm/wit-worker/yoake-worker.wit` (package `yoake:mods@1.6.0`, an additive minor: new
interfaces only, nothing of 1.0 to 1.2 changed):

| side | calls |
|---|---|
| mod, full-access core module | `yoake_worker_send(ptr, len) -> i32`, `yoake_worker_recv(ptr, cap) -> i32` (module `env`); SDK: `yoake_guest_sdk::worker::{send, recv}` |
| mod, sandboxed component | `worker-send(list<u8>) -> result<_, string>`, `worker-recv() -> option<list<u8>>` (interface `yoake:mods/worker@1.6.0`) |
| worker component | imports `host-send`, `host-recv` (blocks until a message or until the worker is stopped), `host-try-recv` (interface `yoake:mods/worker-host@1.6.0`); exports `run: func()` (interface `yoake:mods/worker-run@1.6.0`) |

Rules:

- Each direction has a queue of at most **1024 messages and 16 MiB**. When it is full, the send returns an error
  (`queue-full`, or `-1` in the core ABI); it never blocks. A message over 16 MiB is `too-large` (`-3`).
- `yoake_worker_send` returns `0` ok, `-1` queue full, `-2` no worker (none declared, or it stopped), `-3` too large,
  `-4` bad pointer. `yoake_worker_recv` returns the length (> 0), `0` for none, `-n` when the next message needs `n`
  bytes (it stays queued), `i32::MIN` for no worker, `i32::MIN + 1` for a bad pointer. The core ABI treats an empty
  message as "none".
- `run` is called once on the worker thread; when it returns, the worker ends. A trap, a panic or `exit` ends only the
  worker: the line `wasm mod <id> worker: ended, the worker is disabled: ...` is logged, the mod goes on and its sends
  return `closed` / `-2`.
- The worker stops when the mod is unloaded (hot reload, leaving the world) or the game exits: its queues are closed, a
  blocked `host-recv` returns `none`, and guest code is interrupted within about 10 ms. A worker blocked inside a
  socket call is let go after 0.5 s.
- A package with both a client and a server module gets one worker per instance.
- The worker's stdout and stderr go to the log as `wasm mod <id> worker [stdout]: ...`.

Example (worker side, Rust with `wit-bindgen`): `crates/yoake-wasm-sdk/examples/f2-io` (`--features worker`).

### Network in the main instance

A sandboxed component's main instance links `wasi:sockets` and `wasi:http`, but the context denies them: a connect or a
request returns an access-denied error. A call that could wait would stop the game thread. Do network in the worker.
(`Sandbox::set_main_network(true)` lifts this for tools and tests; it is not used by the game.)

### Services are no longer gated

The service calls of a sandboxed mod (`config`, `permissions`, `storage`) are granted without `[wasm] permissions`. The
field is still parsed so older mods load; `service.commands`, `service.chat` and `service.gpu` keep their own meaning.
`service.http` was accepted by the manifest but no transport was ever connected to it; use `wasi:http` in a worker.

## 16. Levels and world generation (F3-8b)

Four injected hook points (all `side = "server"`, ids in `inject-sites.json`; every one is a no-op unless a mod subscribes):

| Id | Arguments | Runs |
| --- | --- | --- |
| `level::before_open` | `&mut Game`, level id | before a level that is not open is read from its folder |
| `level::after_open` | `&mut Game`, level id | after it opened and mods heard `LevelEvent.Load` |
| `level::before_close` | `&mut Game`, level id | before an unload, delete or regenerate drops it, while chunks and files are still there |
| `worldgen::generate_chunk` | level id, `cx`, `cz`, `&mut ModChunk` | once per chunk of a level made by a mod generator (`yoake:mod`), before the generator fills it |

`worldgen::generate_chunk` is not called for the vanilla three levels or for flat, debug and noise levels, so vanilla terrain is untouched. A hook may write into the empty chunk through the `ModChunk` calls (`set_block`, `fill_layer`, `block`, `set_biome`); the level's generator then fills it, so a hook that wants to replace the terrain should be the generator itself (the `generator_register` service in a WebAssembly mod). It runs on a chunk-generating worker thread, so keep it short: a hook that takes longer than the module's call limit loses the chunk to the safety rule below.

The WebAssembly services stay on the frozen enum-free side: `current_level(player)` (service 235) gives the real id of the level a player is in, whatever level it is.

Generator safety. A mod generator that does not answer (module busy, trapped, over its per-call limit) is tried once more about a tick later. If that fails too, the chunk is left as the generator wrote it, marked not generated, and never saved: it is generated again the next time it loads. The first block anyone writes into it clears the mark, so a build in such a chunk is kept.

## 17. Custom mobs and the brain option (F4M-4b)

Custom mobs (`data/<id>/horizon/entity_type/<name>.json`, `"template": "yoake:mob"`) can declare plain `ai.goals` /
`ai.targets`, or add the **brain option** (`ai.memories`, `ai.sensors`, `ai.activities`, `ai.schedule`,
`ai.default_activity`). Mobs with none of those brain fields run plain goals unchanged.

### Memories (`ai.memories`)

Up to 64 typed memory slots per type:

| `type` | JSON `default` | Notes |
|---|---|---|
| `bool` | `true` / `false` | |
| `i32` | integer | |
| `f32` | finite float | |
| `string` | string | max 256 bytes |
| `vec3` | `[x, y, z]` | finite floats |
| `block_pos` | `[x, y, z]` | integer block coordinates |
| `entity` | — | runtime mob id or player UUID; no JSON literal |

Each entry accepts `"default"`, `"persistent": true|false` (saved under `yoake.memories.<name>` in entity NBT; unknown
keys from other mod versions are preserved), and `"ttl": 1..=72000` (default expiry in ticks when written).

### Sensors (`ai.sensors`)

Up to 32 declarative sensors per type (`interval` 1..=2400 ticks, default 20; optional `memory` and `ttl`):

- `nearest_players` (`range`, `must_see`, `ignore_creative`) -> writes `entity` (default key `nearest_player`).
- `nearest_living` (`types`, `range`, `must_see`) -> writes `entity` (default key `nearest_living`).
- `hurt_by` (`max_age`) -> writes `entity` or `bool` (default key `hurt_by`).
- `blocks` (`tag` and/or `blocks`, `radius` 1..=32, `vertical_radius` 0..=16) -> writes `block_pos` or `vec3` (default key `nearest_block`).
- `light` (`kind`: `combined` | `sky` | `block`) -> writes `i32`, `f32`, or `bool` (default key `light_level`).

### Activities and schedule (`ai.activities`, `ai.schedule`, `ai.default_activity`)

- An activity (`priority` 0..=32, default 5, **lower number wins**; `core`: boolean) holds `when` predicates plus
  `goals` and `targets` (or top-level goals can set `"activity": "<name>"`).
- Every `core: true` activity whose `when` passes runs alongside the active non-core activity.
- Predicates in `when` test one memory (`present: bool`, `eq`, `ne`, and numeric `gt`/`gte`/`lt`/`lte` on `i32`/`f32`).
- At the same priority, a `when`-triggered activity wins over a schedule-only activity (`when` empty).
- `ai.schedule` lists `{"time": 0..24000, "activity": "<name>"}` entries (evaluated over `day_time % 24000`); when no
  gated or scheduled activity matches, `ai.default_activity` (else `"idle"` if present) is chosen.

### Script access (`service.mobs`)

With `[wasm] permissions = ["service.mobs"]` (`crates/yoake-wasm-sdk`, `full::server::mobs`), server modules read and
write a mob's declared brain memories:

- `mobs::memory_get(entity, key) -> Result<Option<MobMemoryValue>, String>` (service 287: `mob-memory-get`).
- `mobs::memory_set(entity, key, value: Option<MobMemoryValue>, ttl_ticks: Option<u32>) -> Result<(), String>` (service 288: `mob-memory-set`; `value = None` erases, `ttl_ticks = None` uses the memory's default `ttl` or never expires).
- Value constructors: `mobs::memory_bool`, `memory_i32`, `memory_f32`, `memory_string`, `memory_vec3`, `memory_block_pos`, `memory_entity_mob`, `memory_entity_player`.
- See `crates/yoake-wasm-sdk/examples/fox_ai` and `tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json`.

### Runtime entity textures (`service.mobs`, F4M-7)

A client module in the `client-full` world (`[wasm] world = "client-full"`, `client = "bin/wasm/..."`,
`permissions = ["service.mobs"]`) can allocate, paint, and bind RGBA8 textures to custom mobs at run time via
`yoake_wasm_sdk::full::client`. Everything is local to each client: nothing is sent over the network, and each
client renders its own bindings (commonly driven by `hook::SYNCED_CHANGED_CLIENT`).

- `client::mob_texture_create(width, height) -> Result<u32, String>` (service 289: `mob-texture-create`): allocates a
  `width` x `height` (`1..=1024` on each side) slot inside the reserved `1024x1024` `horizon/mob-textures` region of the
  entity atlas. Each mod may keep at most **64** live textures at once.
- `client::mob_texture_write(id, origin, size, rgba) -> Result<(), String>` (service 290: `mob-texture-write`): uploads
  RGBA8 pixels into the sub-rectangle `[origin.0, origin.1, size.0, size.1]` of texture `id` (full or partial upload,
  at most **4 MiB** per call).
- `client::mob_texture_bind(entity, slot, id: Option<u32>) -> Result<(), String>` (service 291: `mob-texture-bind`):
  binds texture `id` (or `None` to restore the pack texture) to `slot` on custom mob `entity`:
  - `""` or `"default"`: replaces the model's `default` texture (and any layer in `model.layers` that inherits it).
  - `<layer_name>`: replaces the named layer in `model.layers`.
  - `"glowmask"`: replaces the automatic `<texture>_glowmask` emissive pass.
  - `<bone_name>` or `"bone:<bone_name>"`: replaces only that bone's quads on the body pass.
- **Argument order trap:** the generator sorts WIT arguments alphabetically (`services::mob_texture_create(height, width)`,
  `services::mob_texture_write(id, origin, rgba, size)`, `services::mob_texture_bind(entity, id, slot)`). Always call
  the `yoake_wasm_sdk::full::client::mob_texture_*` wrappers, which keep `(width, height)`, `(id, origin, size, rgba)`,
  and `(entity, slot, id)`.
- **Lifetime and reloads:** active textures and bindings persist across resource-pack reloads (atlas rebuilds) and
  shader-pack toggles. Unloading or reloading the mod automatically frees its textures and clears its bindings.
- Example: `crates/yoake-wasm-sdk/examples/fox_texture` (updates the Cinder Fox skin when `fox_ai` updates `warmth` in
  `mob_hurt`).

### Debug view and hot reload of custom mobs (F4M-12, F4M-13)

**Debug view.** It is the player's own client: nothing is sent to the server, and it costs nothing while it is off.

- `/yoake mobdebug [on|off] [hitboxes] [bones] [anim] [ai]`: with no words it toggles everything; `on` alone switches all
  four parts on, `off` switches the view off, and part names alone switch exactly those parts on. F3+M is the same toggle
  (only in a world that has custom mob types).
  - `hitboxes`: every hit and pick part of the type's `hitboxes` as a box with its name and damage multiplier. Green is
    hurt and pick, red hurt only, blue pick only, grey a part that is switched off (`set_hitbox_enabled`).
  - `bones`: the skeleton as lines from each bone's origin to its children (drawn over the body, in the pose the mob is
    drawn in), the bone names, the `locators` as small cyan boxes with their names, and the `seats` as orange boxes.
  - `anim`: per controller layer the state and the time of its first clock, and the tracked (`synced`) values.
  - `ai`: an arrow to the mob's target, the navigator's path from its next node, the running goals and target goals, and
    the brain's current activity.
  - Every mob in view (the 48 nearest within 48 blocks) also gets a header line: type id, mob id and health.
- `/yoake mobs`: the loaded custom types with their live and drawn mobs. The first call starts a cost meter; run it again
  after a few seconds to see the mean microseconds a frame spends posing and building vertices per type. The path-budget
  lines of `/horizon custommobs stats` follow. `/yoake mobs stop` ends the meter. (The server tick cost per type is not
  in this report yet: the path searches are the only per-type server cost counted today.)
- The commands are answered by your own game on any server, for every player who types them in the chat. With Horizon's
  own commands on (`--pregen`) they are also in the command tree for game masters (tab completion, and the control
  socket's `command`).

**Hot reload (development only).** It needs developer mode (`YOAKE_WASM_UNSIGNED=1`) and a package folder that has no
`SIGNATURE` (a `--dev-mod` folder, or an unsigned installed package). A signed package never reloads.

- **Assets (R1)**: the game looks at the files of each custom type once a second: its JSON, the geo or bbmodel, the
  animation files, the controllers, the textures and the glow mask. A change counts once the files have stayed the same for
  a whole second (your editor or exporter may still be writing). The type is then read again through the same loaders as
  at start, and every running mob of it switches to the new model, animations and layers at the next game tick (between two
  frames). **The animation phase is kept**: it is carried by the clocks the server anchored, so a walking fox keeps
  walking from the same point of its stride. A texture change reloads the resources (like F3+T) instead.
- **Definitions (R2)**: `/yoake mob reload [type]` reads the named type (every reloadable type without a name) now,
  without waiting. Live mobs get the new numbers (`max_health`, `movement_speed`, attributes: a value you did not change on
  the mob follows the file, a hurt mob stays hurt), a rebuilt goal list when the `ai` section changed, and the new hitbox
  parts. A mob spawned a moment after the reload is brought over within a tick.
- **A broken file keeps the old version**: the log gets one warning that names the file and why
  (`Horizon custom-mob reload: <type> not reloaded, the old version keeps running ...`), and the game goes on with the
  last good type. Fix the file and save again.
- **Not reloadable while the game runs**: a change to the `synced` values, or to the controllers' layers, states or
  triggers (what the network format depends on). The reload is refused with a message; restart the game. Deleting a type
  and adding a new one also needs a restart.
- **Server modules (R3)**: with `YOAKE_WASM_SERVER_RELOAD=1` as well, the `.wasm` file of an unsigned server module is
  watched and a new build is swapped in at a server tick. The engine owns the mobs, so they keep their state; the guest's
  own memory (anything it kept in variables) starts afresh. A build that does not load keeps the old instance running with
  one warning. Without that switch server modules load once, as before.

## 18. Data content kinds: enchantments, damage types, game rules, fluids and integer block states (REM-16)

A `.yak` (or a `.hmod`) with capability `registries` adds these without any module code, in `data/<id>/horizon/<kind>/<path>.json`
(the id is `<id>:<path>`; the full field lists are in [docs/mods.md](../mods.md#enchantments-damage-types-game-rules-and-fluids)).
A mod with none of the files changes nothing in the game.

| Kind | File | What it becomes |
| --- | --- | --- |
| `enchantment` | vanilla's enchantment JSON | `data/<id>/enchantment/<path>.json` of the content root: the registry (synced to clients with its data), `/enchant`, the enchanting table, the anvil, loot functions, item `enchantments` components |
| `damage_type` | vanilla's damage type JSON | `data/<id>/damage_type/<path>.json`, the damage catalog, the commands' id lists (`/damage`) |
| `game_rule` | `{"type": "bool"/"int", "default", "min", "max", "category"}` | the rule `<id>:<path>` for `/gamerule`, the rules screen, the management API, saved with the world |
| `fluid` | `flowing_id`, `still_block`, `flowing_block`, `bucket`, `still_texture`, `flowing_texture`, `tint`, `viscosity`, `tick_rate`, `light`, `can_drown`, `motion` | one family in the world's fluid registry, as a module's `fluids` registration |

```json
// data/ruby/horizon/enchantment/keen.json
{ "description": { "translate": "enchantment.ruby.keen" },
  "supported_items": "#minecraft:enchantable/sharp_weapon",
  "weight": 5, "max_level": 3,
  "min_cost": { "base": 5, "per_level_above_first": 8 },
  "max_cost": { "base": 25, "per_level_above_first": 8 },
  "anvil_cost": 2, "slots": ["mainhand"] }

// data/ruby/horizon/damage_type/shatter.json
{ "message_id": "shatter", "scaling": "when_caused_by_living_non_player", "exhaustion": 0.1, "effects": "freezing" }

// data/ruby/horizon/game_rule/gem_rate.json
{ "type": "int", "default": 3, "min": 0, "max": 10, "category": "misc" }

// data/ruby/horizon/fluid/oil.json   (blocks ruby:oil and ruby:flowing_oil, item ruby:oil_bucket, textures block/oil_still.png and block/oil_flow.png)
{ "tint": 4473924, "viscosity": 4, "tick_rate": 10, "motion": "water" }
```

From a module: a `set-game-rule` change of `world-control` takes the namespaced name (`ruby:gem_rate`) of a rule the game
registered (a name it does not know, or `minecraft:...`, is refused as before); `/damage` works through `execute-command`
with the damage type id. Rules and fluids are registered at start, so adding or removing one needs a restart. A fluid
reload updates an existing family's `motion`, `tick_rate` and `viscosity`; its other field changes also need a restart.
Fluids from files and from a module's `fluids` registration join one registry (125 families at most), and a name that
two families bind is refused with the fluid named.

Blocks gain an `int` state property next to `boolean` and `enum`: `"state_properties": { "age": { "type": "int", "min": 0, "max": 7 } }`
(vanilla's `IntegerProperty`: `min` 0 or more and below `max`, at most 256 values, ascending). Its values are decimal text in
`default_state` and `random_ticks` (`"default_state": { "age": "0" }`), and the model schema lists it as an integer range, so
a blockstate file can select on `age=3` as it does for a vanilla crop. A block is still a copy of a vanilla *template*, but
`"template": "yoake:cube"` gives a plain full cube (stone's single state, whose stats and files the block's own `properties`,
blockstate, model and loot table replace); a block with no vanilla row at all is not available (report FR-REM16).

## 19. Behaviours of the mod's own blocks, items, menus and recipe types (REM-15)

Forge mods subclass `Block`, `Item` and `MenuType`; a server module does the same with **behaviours**. From `init` it
tells the game which of the package's own content objects it acts on and which hooks it implements; from then on the
game calls the module's `handle-call` export for *those objects only*. Every other block, item and menu costs exactly
what it did (a lookup that finds nothing; nothing at all while no module registered anything), and a game with no such mod
behaves as before.

```rust
use yoake_wasm_sdk::full::behaviors::{self, Behaviors, BlockAnswer, BlockUse, Interaction, block};
use yoake_wasm_sdk::full::server::{ServerMod, ext, services, types::BlockPos};

#[derive(Default)]
struct Guest;

impl Behaviors for Guest {
    fn on_block_use(&mut self, call: &BlockUse) -> BlockAnswer {
        // the services of the hook work here: flip `lit` of the block that was clicked
        let pos = BlockPos { x: call.block.pos[0], y: call.block.pos[1], z: call.block.pos[2] };
        // ... block_property / set_block_property ...
        BlockAnswer::result(Interaction::Success)
    }
}

impl ServerMod for Guest {
    fn subscriptions() -> &'static [&'static str] { &[] }
    fn init(&mut self) {
        ext::register_behavior("block", "ruby:lamp", &[block::USE]).ok();
    }
    fn handle_call(&mut self, _caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        behaviors::handle(self, &method, &payload).unwrap_or_else(|| Err(format!("unknown method {method}")))
    }
}
yoake_wasm_sdk::export_server_full!(Guest);
```

`ext::register_behavior(kind, id, hooks)` is the service `behavior-register` (292). The sample is
`crates/yoake-wasm-sdk/examples/ruby-behaviors` (a lamp that toggles on use, a wand, a vault menu and an alloy recipe type,
with its content files); its host tests are `crates/horizon-client/src/wasm_mods/behaviors/tests.rs`.

**Rules of registration.** `id` must be one of the package's own objects (`<package id>:<path>`); `kind` is `block`, `item`,
`menu` or `recipe`; the hook names are those below; the same object may not be taken by two packages. The game reads the
table once, when it starts, and a world start runs `init` again: the same registration is accepted again, a new object or a
different hook set after the game has started is refused with a message (restart the game to change them). At most 256
objects of a kind per package.

| Kind | Hooks | The engine calls it | Game access of the hook |
| --- | --- | --- | --- |
| `block` | `use` | a player right-clicks the block (server side; the answer is `Pass`, `Success`, `Consume` or `Fail`, as `InteractionResult`) | the services (`block-property`, `set-block-property`, `give-items`, ...) |
| | `placed`, `removed` | a block of the id arrived or left through `Level.setBlock` (a change of state alone is neither) | none: answer commands |
| | `neighbor-changed`, `random-tick`, `scheduled-tick` | the level's own ticks (`random-tick` makes the block tick at random) | none: answer commands, applied at once |
| | `entity-inside`, `step-on` | an entity touches the block | none: answer commands |
| `item` | `use`, `use-on-block`, `use-on-entity` | a use of the item in the air, on a block (when the block did not take the click), on an entity | the services |
| | `finish-using` | eating or drinking ended; the answer may leave another stack | the services |
| | `inventory-tick` | every tick the item is in a player's inventory | the services |
| `menu` | `create` | a player opens the menu type (`open_menu`): answers its size and first contents | the services |
| | `may-place`, `clicked` | a stack is offered to a slot; a click on a slot was applied (slot, button, click mode, the contents after it; the answer may set slots) | none |
| | `quick-move` | shift-click: the answer says whether the guest took the move over and may set slots | the services |
| | `still-valid`, `closed` | every tick the menu is open; it closed (the contents then) | the services |
| `recipe` | `describe`, `decode`, `matches`, `assemble`, `display` | see below | none |

A hook that has no game access of its own (a tick the level runs, a block observed changing, an entity touching a block) cannot call
the write services; it **answers commands** instead, which the engine checks and applies: `set-property` (a block's property at
a position), `schedule-tick` and `play-sound`; at most 64, within 32 blocks of the object. Commands of the level's ticks are
applied in that tick; those of `placed`, `removed`, `entity-inside` and `step-on` wait for the next tick of that level and can
set properties only.

**Where they run.** On a server, as every server module hook does: a dedicated server, or a game that opened its world to LAN
(its integrated server). A client's prediction (a game that joined a server) calls nothing, and neither does a plain single-player
window that has not opened its world to LAN (it holds no server): the server decides. A hook that is still busy
because one of its own services caused it (it set a block, and the block's neighbour update came back to the same module) is
dropped, and counted and logged once; the object then does what it does without the hook. A guest over its per-call limits is
disabled as for any hook, and a call that fails or answers something that does not check changes nothing. The calls count in
the module's usual limits.

**Block entities, by position.** The services `block-entity-read-at` (293) and `block-entity-patch-at` (294), `ext::block_entity_read`,
`block_entity_merge` and `block_entity_replace` in the SDK, read and change the saved data of the block entity at a position, a vanilla
chest or a mod block entity alike, as `/data get block` and `/data merge block` do: the data is one network-encoded NBT compound
(`yoake_wasm_sdk::nbt::Compound`), at most 64 KiB, merged into the entity's data (or, for a mod block entity only, replacing it).
The chunk is saved next time and clients receive the update tag. A mod block entity's data (`nbt`, `items`, `data`) was already
saved and loaded with the chunk; this service is how a guest outside its tick hooks reaches it.

**Recipe types.** A `recipe` object is a recipe type that recipe files can name (`"type": "ruby:alloy", "version": 1`). The
guest answers `describe` (the version its recipes carry and a recipe book category, whose id and icon are its own), may answer
`decode` (turn a recipe's JSON into the data the game keeps, at most 64 KiB; without it the data is the JSON text), and answers `matches`
(a crafting grid), `assemble` (what it makes) and, for the recipe book, `display`. `behaviors::json_fields` and `json_strings` read a recipe's
JSON without a JSON library. Two services let a hook with game access use the types: `recipe-match` (295, `ext::recipe_match`) asks the game what a
crafting grid makes with a type, and `recipe-data` (296, `ext::recipe_data`) lists the loaded recipes of a type with the data it kept for each.
A package cannot be called while it runs a hook, so `recipe-match` refuses the package that defines the type (with an error that says so):
a menu of the same package takes the data from `recipe-data` and matches with its own `on_recipe_matches` (the vault of the sample does, in `quick-move`), and
a menu of another package uses `recipe-match`. Neither crafts or removes anything.

| Limits | |
| --- | --- |
| objects per kind and package | 256 |
| commands per answer, reach | 64, 32 blocks |
| block entity data, recipe data | 64 KiB |
| a menu | at most 256 slots; `create` is asked once per opening and only on the server |

Not in this chapter: a block with no vanilla template row (section 18), entity behaviours and custom fluid behaviour
beyond the built-in flow and buckets, and capabilities by hook (`capability_providers`: energy, item and fluid handlers). The catalog's global hooks (`random-tick`
for every block, `use-item-on`, ...) stay as they were, for a mod that wants every block, not its own. See the report FR-REM15.

## 要確認 (not verified)

1. **Client modules** are merged (MX3CL1) and tested through the loader, but not in a running game client: no client
   module was run against a real window or HUD.
2. **Where the player finds the `yoake:` lines.** They are written to stderr and to the `log` crate; this guide did not
   find where the launcher stores the game's stderr, so it does not say which file to open.
3. **Running in the real game.** Nothing here launched the game. The template and the examples were run through the
   real loader with a stand-in `Game` root (a real `Experience` at the path `Game.experience`). The same SDK examples
   against a real `Game` are in `crates/horizon-client/src/game/yoake2_examples_tests.rs` and are ignored without
   `HORIZON_TEST_RESOURCES`.
4. **"Start or restart the game after allowing."** Derived from the code (the launcher writes the list before it spawns
   the game and the game reads it once at the first world start); not observed in a running launcher.
5. **Call-site hooks** (`kind = callsite` in the ledger): the ledger and the bridges list them like function hooks, and
   `docs/yoake2/HOOK-SLICE-BRIDGE.md` says the guest sees argument 0 as a handle and the other arguments are not
   visible. No call-site hook was run for this guide.
6. **`yoake` CLI for full-access mods**: "do not use it" is from reading `crates/yoake-cli/src/main.rs` (it builds
   components with Cargo and checks a server module as a sandboxed component); it was not run on a full-access package.
7. **`build.sh` was run in Git Bash on Windows only**; Linux and macOS were not tried. On this Windows (GNU toolchain)
   host the pack tool needed a MinGW (`dlltool`, `as`) directory on `PATH` to compile.
8. **Hook ids are not a stable public list.** The ledger has `tombstone` entries and ids that are not bridged; the
   count of bridged hooks (201) is true at this commit only.
9. **The 110-150 ns hook cost** is from the S2 spike report (`MX3S2.md`), measured on one machine; treat it as an order
   of magnitude.
