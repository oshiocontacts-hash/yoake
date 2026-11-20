# Writing a Yoake WebAssembly mod

A Yoake wasm mod is a WebAssembly **component** (`wasm32-wasip2`) for the WIT world
`yoake:mods/client-mod@1.0.0` (`crates/yoake-wasm/wit/yoake-mod.wit`). The guest SDK is
`crates/yoake-wasm-sdk`; a complete example is `crates/yoake-wasm-sdk/examples/hello`.
The frozen world stays unchanged. The extended `client-full` and `server-full` worlds use
`yoake:mods@1.0.0` in `yoake-full.wit`; the GPU interface belongs to `client-full` only.

## The 1.0 ABI freeze

`crates/yoake-wasm/wit/abi-1.0-baseline.wit` freezes the current `yoake:mods@1.0.0`
interfaces and worlds, including `native-screen-view.kind`. The snapshot is comment-enveloped
WIT (remove its three-line header and each remaining line's `// ` prefix to parse it separately).
This keeps the snapshot inert when guest and host bindings load the live `wit/` directory.
`abi-1.0-ids.json` freezes the service/hook ID tables and capability names from all four active
ABI catalogs, and separately preserves the package manifest's accepted `WASM_PERMISSIONS`.

Run `cargo test --offline -j 4 -p horizon-mod-api-gen` to check compatibility. Existing record
fields (including their order and types), enum/variant/flag cases, aliases, function parameters
and results, world bindings, IDs and capability names cannot change or disappear. Independent
new types, functions with unused IDs, hooks, interfaces and capability names are additions;
they do not require a baseline refresh. In particular, W2DN's reserved service IDs **127–145**
and hook IDs **43–44** are additions, not permission to alter existing entries. The separate
`yoake:mods@1.1.0` package in `wit-inject/` is not part of this 1.0 snapshot.

Before the initial 1.0 release only, the owner may deliberately refresh both baseline files
after reviewing the entire ABI diff (PowerShell):

```powershell
$env:YOAKE_UPDATE_ABI_BASELINE = "1"
cargo test --offline -j 4 -p horizon-mod-api-gen abi_1_0_is_additive
Remove-Item Env:YOAKE_UPDATE_ABI_BASELINE
```

Review and commit both snapshots together, then run the full generator tests without the flag.
`YOAKE_UPDATE_WIT=1` regenerates live bindings, never the compatibility baseline. Once 1.0 is
released and signed by the owner, do not refresh that baseline or change existing 1.0 items;
put incompatible shapes in new types/interfaces instead. Signing remains an owner operation.

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
  return at once), the filesystem has the preopens `/data` (the mod's own folder `mod-data/<id>`), `/game` and, with `[wasm] host_filesystem = true`, `/host/<letter>` (FR-F2), and the main instance may not use sockets or `wasi:http` (they are linked, but the context denies them; use a worker, `docs/yoake2/MOD-GUIDE.md` section 14). A component that imports any interface outside `yoake:mods/*`, `yoake:mods/worker@1.3.0` and the list above is rejected at load. The example is
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

In a mod project, `yoake check` validates the component's imports and exports against the same
world and host linker used by the game, including its supported WASI subset, then calls `init`
and validates its subscriptions against the hooks wired for that world and side.
It does not rebuild: if `yak.toml`, `Cargo.toml`, or any file under `src/` is newer than a
declared Wasm module, the check fails as stale and asks you to run `yoake build`. Freshness uses
file modification times; equal timestamps are accepted. `yoake check --help` summarizes this rule.

## Packaging in a `.yak`

Put the component at `bin/wasm/<name>.wasm` and declare it in `yak.toml`:

```toml
[wasm]
client = "bin/wasm/hello.wasm"
interface = "1.0"
```

`interface = "1.0"` is the package's WASM compatibility declaration, checked separately from
the extended WIT package version. Keep it at `1.0` for this host, including full-world mods
built against WIT `1.0.0`. The host rejects imports outside `yoake:mods/*` and the WASI subset above.

## Developer mode: hot reload

With `YOAKE_WASM_UNSIGNED=1`, the game can load unsigned WASM modules from `.yak` packages whose
manifest declares `[wasm]` and has no `[bin]`, when the package has no `SIGNATURE` file. If a
`SIGNATURE` file is present, it must still be valid and match the installed files. It polls each
unsigned installed module once a second and, when the file changed (and stayed unchanged for a whole
poll), builds a fresh instance and swaps it in at the next frame or tick. A new build that fails to
load keeps the old instance running (one warning in the log). `YOAKE_WASM_DEV_DIR=<folder>`
additionally loads every `<id>.wasm` in the folder as a mod (no `.yak` needed) and reloads it the
same way. Those modules receive a `dev-` id prefix and are skipped if that id belongs to an installed
mod, keeping their settings and storage separate. Hot-reloaded modules do not pin a
`required_on_join` digest while their bytes can change. Both features are off unless the WASM switch
or legacy DLL switch is exactly `1`. WASM remains sandboxed, but it still receives permissions
declared by its mod; for example, a server module may have `service.commands`. The WASM switch does
not enable or load DLL/native code. The launcher developer option
**自分の WASM MOD をテスト / Test your own WASM mods** asks for confirmation and sets this switch
for the launched game.

The older `YOAKE_DLL_MODS_UNSIGNED=1` remains a compatibility superset: it still permits unsigned
native DLL mods and also enables these WASM development features. Use it only for native DLL work.
The startup log has one line saying `wasm: no mods (engine not started)` or
`wasm: N mod(s), engine started`.

## CLI and test scaffolding

The `yoake` workspace binary needs Rust 1.95 or later and the `wasm32-wasip2` target; no
`wasm-tools` installation is required.
From a directory where a new project may be created:

```sh
yoake new hud-only my-mod
cd my-mod
yoake build
yoake check
yoake pack
yoake dev
```

The `examples/hello` and `examples/full-demo` projects under `crates/yoake-wasm-sdk/` also
include manifests. Run `yoake build` in either example before `yoake check`: a raw Cargo build
alone does not place the components at the paths declared by `yak.toml`.

`new` also accepts `minigame`, `custom-mob`, `utility` and `script`. These are starter projects,
not complete game modes; mob data, models and script files are placeholders. The CLI embeds the
SDK and WIT definitions in each generated project. Optionally set `YOAKE_SDK_PATH` when creating a
project to use a local SDK checkout instead; that override stores its absolute path in the manifest.

`build` compiles declared modules into `bin/wasm/` and checks them; it does not pack them. Cargo
uses `CARGO_TARGET_DIR` when set, or the project's `target/` directory otherwise. For a package
declaring both a client and a server module, its Cargo project must define `client` and `server`
features that select the respective worlds, as in `full-demo`. The CLI builds once with each
feature in addition to the project's default features. Projects using another arrangement can
build components separately and run `yoake check`, but must adapt their features for `yoake build`,
`yoake dev` or `yoake pack`.
`pack` builds declared Wasm modules first, checks the package, and writes the unsigned
`dist/<id>-<version>.yak`, replacing the previous package of that version. Packages without `[wasm]`
skip component validation and can be checked and packed without a Wasm build. Signing remains an
owner operation.

`check` validates the manifest, permissions and module sizes, then uses the game's Wasmtime
configuration and host linker to instantiate each declared component against its world. It calls
`init` with stub hosts and checks the returned hook names using the game's subscription rules.
A JSON summary reports the checked modules and nonempty lines from `WASM-NEEDS.md`, if present.
Stub hosts do not supply live game state, so initialization that depends on game services can fail
or behave differently, and successful validation does not prove that hooks work in a running game.

Every command emits exactly one JSON result line on stdout, including on failure; diagnostics go to
stderr. `dev` includes `built`, `checked`, `installed`, `errors`, `notes`, `tests` and `wasm_needs`.
For a `client-mod` with `YOAKE_WASM_DEV_DIR` set, it builds, checks and installs `<id>.wasm` there,
creating the directory if needed. For `client-full`, server-only packages, or an unset directory,
it reports a successful build and check with `installed: false` and a note instead.
The engine discovers files at startup, so restart it if the file was not present then. The owner
must explicitly enable unsigned developer mode, and installation does not confirm the engine
accepted the reload. `yoake dev --test` also runs the project's tests in the game and fills `tests` (see
"Testing a mod in the engine"); plain `yoake dev` never starts the game and leaves `tests` empty.

Projects beneath a link or Windows reparse point, including junctions and some synced folders, are
rejected. The error identifies the affected path; move the project to a normal folder without links
or reparse points before using the CLI.

`#[yoake_test]`, `TestCase`, `TestWorld` and `TestDriver` are the test interface a test is written against. Run
natively with a mock driver (`cargo test`) they check the logic of a scenario; compiled as a *test module*
(below) the same code runs against a real world in the game. The marker `#[yoake_test]` only documents a
test function; the explicit `YOAKE_TESTS` registry is what is exported. See the [recipes](yoake-recipes.md)
and [API index](yoake-api-index.md); the latter can be checked with
`python tools/generate_yoake_api_index.py --check`.

## Testing a mod in the engine

`yoake test` runs the tests of a project in the game itself: every test in a fresh, temporary, superflat
world, in an offscreen game process that never shows a window or takes the focus. This is the engine side of
`yoake dev`'s `tests` array (it needs the game, so it is a separate step from `yoake check`, which only loads
the component against stub hosts).

### Declaring tests: a test module

A test is a function over a `TestWorld`; the tests of a project are a list, exported by a **test module**: a
second component built from the same crate, of the WIT world `test-mod`
(`crates/yoake-wasm/wit/yoake-test.wit`: exports `tests` and `run-test`, imports `test-driver`). It is
built with the Cargo feature `tests`, placed at `bin/wasm/<id>-tests.wasm`, and is **not** declared in
`yak.toml` and **not** packed into the `.yak`. `yoake new` projects are set up this way:

```toml
[features]
tests = ["yoake-wasm-sdk/test"]
```

```rust
use yoake_wasm_sdk::testing::{BlockPos, TestCase, TestResult, TestWorld};

fn places_a_block(world: &mut TestWorld<'_>) -> TestResult {
    let at = BlockPos { x: 0, y: -60, z: 0 };
    world.place_block(at, "minecraft:stone")?;
    world.run_ticks(2)?;                       // lets the server run 2 ticks
    world.assert_block(at, "minecraft:stone")
}

pub const YOAKE_TESTS: &[TestCase] = &[TestCase { name: "places_a_block", run: places_a_block }];

#[cfg(all(target_arch = "wasm32", not(feature = "tests")))]
yoake_wasm_sdk::export_mod!(Starter);                // the mod
#[cfg(all(target_arch = "wasm32", feature = "tests"))]
yoake_wasm_sdk::export_tests!(YOAKE_TESTS);          // the test module
```

A test passes by returning `Ok(())` and fails by returning `Err(message)` (the `?` operator passes the
driver's errors on; `assert_*` build messages). Names are 1 to 128 characters, unique, at most 1024 a module.
A test that traps (panic, `unreachable`, out of fuel) or runs past its time limit fails with the trap as its
message; the rest of that test's world is thrown away with it. A complete example is
`crates/yoake-wasm-sdk/examples/test-demo`.

Why a separate component rather than hooks in the mod: the mod's worlds (`client-mod`, `client-full`,
`server-full`) stay exactly as they are, so nothing a mod can import or export changes, and the test code
cannot ship inside a mod by accident.

### The driver

`TestWorld` forwards to the `TestDriver` the engine provides (`HostDriver`, SDK feature `test`):

| `TestWorld` | does | in the engine |
| --- | --- | --- |
| `run_ticks(n)` | the server runs `n` more ticks (1 to 72000), as fast as it can, then the call returns | the bridge counts server ticks; the server loop skips its 50 ms wait meanwhile, as `/tick sprint` does |
| `place_block(pos, "minecraft:oak_stairs[facing=north]")` | sets the block (overworld) | `/setblock x y z <block>` |
| `break_block(pos)` | breaks it, drops included | `/setblock x y z minecraft:air destroy` |
| `block_at(pos)`, `assert_block(pos, text)` | the block's id as text (`minecraft:oak_stairs`: measured, no properties) | the `block` server service (what a `server-full` mod gets) |
| `spawn_entity(kind, pos)` | spawns, returns the entity id | `/summon` then `entities-in` (the newest of that kind there) |
| `spawn_mannequin(name, pos)` | a mannequin entity (not a player: it is not listed by `player_names`) for a stable UUID derived from `name`, returns its entity id | the `spawn-mannequin` service |
| `entities_near(pos, radius)`, `assert_entity_count(kind, pos, radius, n)` | the kinds of the living entities near, at most 512 | the `entities-in` service |
| `player_names()` | the players on the server, the world's owner first | the `server-players` service |
| `run_command(line)`, `assert_command(line)` | runs a server command | the `run-command` service with the same denylist a mod has (`stop`, `op`, `ban`, `publish`, `transfer`, ... refuse) |
| `mod_call(target, method, payload)` | calls another loaded server module's `handle-call` (the mod under test, or one added with `--yoake-test-with`) | the `mod-call` service, with the caller id `yoake-test` |
| `tick()`, `log(text)`, `client_count()`, `client(1)` | ticks run since the test began; a line on stderr of the runner; 1 or 2; the second game (below) | |

Every operation runs on the integrated server thread between two ticks, with the game's own server
services, so a test sees the world the way a `server-full` mod does. The driver is test-mode only: its WIT
interface `yoake:mods/test-driver` is linked for test modules only, and the loader of every mod refuses a
component that imports it. Test mode itself starts only from the command line of the game process
(`--yoake-test*`, below); no mod, server, save or network message can turn it on, and without those flags
the server loop does one empty check per tick and nothing else changes (no new file, no new behaviour).

### Running

```sh
export YOAKE_GAME=/path/to/horizon-client      # the game executable (Yoake build)
yoake test                                      # in the project folder: build, check, run the tests
yoake test --filter places --timeout-ms 60000 -- --resources /path/to/prepared/resources
```

`yoake test` builds the mod and the test module (`cargo build --features tests`), checks the mod as
`yoake check` does, runs the game and prints one JSON line:

```json
{"ok": false, "built": true, "checked": true, "id": "my-mod", "version": "0.1.0",
 "tests": [{"name": "places_a_block", "ok": true, "ms": 412},
           {"name": "breaks_it", "ok": false, "ms": 380, "error": "block at BlockPos { .. }: expected ..."}],
 "errors": ["test breaks_it failed: block at BlockPos { .. }: expected ..."], "notes": []}
```

Exit code 0 when every test passed, 1 otherwise. `yoake dev --test` fills the same `tests` array (and fails
with the failures as `errors`); it adds a note and leaves `tests` empty when the project has no test module
or `YOAKE_GAME` is unset. Without `--test`, `yoake dev` does not start the game. Extra arguments after `--` go to the game unchanged, which is how the game
finds its prepared vanilla data (`--resources`); `--headless` runs without even a GPU.

The game's own interface is `horizon-client --yoake-test <project folder | .yak> [options]` (`--yoake-test` must
be the first argument; the game's own `--world`, `--seed`, `--game-dir`, `--home`, `--server`, `--dedicated`,
`--fullscreen`, `--frames`, `--world-frames`, `--timeout-seconds`, `--test-terrain` and the like are refused, because
each test gets its own world and folders):

| option | meaning |
| --- | --- |
| `--yoake-test-json FILE` | also write the report to `FILE` (it is always printed on stdout) |
| `--yoake-test-module FILE` | the test module; default `bin/wasm/<id>-tests.wasm` of a project folder (a `.yak` carries no tests, so it needs this) |
| `--yoake-test-with PATH` | install one more package (folder or `.yak`) next to the mod under test; repeatable |
| `--yoake-test-filter TEXT` | only the tests whose name contains `TEXT` |
| `--yoake-test-timeout-ms N` | per test, default 30000 |
| `--yoake-test-startup-seconds N` | per worker, from process start to the test starting, default 240 |
| `--yoake-test-keep` | keep the temporary folders (game folder, world, log) and report their path |
| `--yoake-test-clients=1\|2` | clients that take part; 2 starts a second game that joins the world (see below) |

Its exit code is 0 (all passed), 1 (a test failed) or 2 (nothing ran: bad command line, unreadable project,
missing module). Any other argument is handed to every worker.

### How a run works

The process started with `--yoake-test` is the *coordinator*. It never starts the game itself: it loads the
test module once (to list the tests, calling no driver function), then for each test, in order:

1. makes a temporary folder with a game folder, a `home` for logs, and `saves/yoake-test` holding the
   generator settings of a **superflat world with seed 0, structures off**;
2. installs the project (or `.yak`) and the `--yoake-test-with` packages into that game folder exactly as the
   game reads installed packages (`yoake-mods.txt`, `yoake-mods/<id>/<version>/`); the test module is not
   installed there;
3. starts the same executable as a *worker*: `--offscreen --no-focus --no-pause --world yoake-test --seed 0
   --creative --time 6000 --weather clear --render-distance 4 --yoake-test-run <test> ...` with
   its own `--game-dir` and `--home`, and a stripped environment: only the variables the system and the graphics
   driver need are inherited (`PATH`, `SystemRoot`, `TEMP`, `VK_*`, `WGPU_*`, ...), plus `HORIZON_NO_AUDIO=1`, `HORIZON_HOME`
   and a random token. No `HORIZON_*` or `YOAKE_*` switch of yours (session file, command script, `YOAKE_DLL_MODS`,
   `YOAKE_DLL_MODS_UNSIGNED`, `YOAKE_WASM_DEV_DIR`, ...) reaches it. All paths are absolute (the game changes its
   working directory);
4. the worker checks it is a worker: the token in its environment equals the token file next to its result, it is
   `--offscreen`, its world is `yoake-test` and its game folder is the `game` folder next to the result;
   otherwise it refuses to start. A verified worker loads no DLL mod and accepts the unsigned package under test
   (WebAssembly and data only: a separate switch that never reaches the DLL loader). It arms a watchdog
   (`startup + timeout`) at once. It opens the world as a single-player game, and when its integrated server is up,
   starts the test thread: it waits for the world's owner to join, lets the world run 2 s (chunks load), then
   **freezes** it: from then on the server only services its connections and the test's calls, and the world ticks
   exactly as often as `run_ticks` says (deterministic; a call costs what it costs, not a 50 ms tick). The test runs,
   `result.json` is written and the worker exits;
5. the coordinator waits for the worker. It kills it when it passes `startup + timeout + 10 s`, 5 s after the result
   appears if it has not exited, or at once on Ctrl+C. Workers also die with the coordinator (a Windows job object with
   kill-on-close; elsewhere the worker watches its parent). The result is read at most 1 MiB, error texts are cut at
   16 KiB, and only the last 8 KiB of a worker's output are kept. The temporary folder (an unpredictable new name
   under the system temp folder) is deleted and the next test starts.

A test name never starts with `-` (it travels on a command line). A worker that dies before it writes a result fails its test with the exit code and the tail of its output.
One game start per test is the price of a guaranteed fresh world and of isolation: a test cannot spoil the
next one, a hung test is just killed. It costs a game start and a world's worth of chunks per test, so keep
tests few and each one focused.

World facts a test may rely on: the owner is in creative mode at the world spawn; the time is 6000 and the
weather clear at the start; the render distance is 4 chunks. Tests should build what they use near the spawn
(0, 0): the superflat layers are bedrock, two dirt and grass, so the ground is the layer above the world's
bottom (the exact height is not pinned here; read it with `block_at` if a test depends on it).

### Measured (Windows, debug build, RTX 2060, prepared data without `horizon-data.bin`)

* A worker takes 90 to 100 s from start to the first driver call (loading the prepared data, the world and the
  owner). Do one test per call of `yoake test --filter` while developing, and expect a full run to cost that per
  test; a release build and a prepared folder that has its `horizon-data.bin` (a normal game start writes it) are
  faster. A test itself takes milliseconds: a place, an advance of 3 ticks and a read took 150 ms the first time,
  1000 `block_at` calls 96 ms, an advance of 20 ticks plus two spawns 1.5 s.
* A worker never writes into the prepared folder (the game's own `horizon-data.bin` cache is skipped), reads it in
  place, and leaves nothing outside its temporary folder.
* The server's entity queries (`entities_near`, `spawn_entity`'s lookup) have the game's budget per server tick
  and mod; the world is frozen, so every query of a test between two `run_ticks` shares one tick's budget (512
  to 1024 calls). Advance a tick to start a new budget.

### Two clients (`--yoake-test-clients=2`, `yoake test --clients 2`)

A multiplayer test runs one server and two clients: client 0 is the world's owner (the worker, which runs the test),
client 1 is a second game that joins over the loopback. In the test module, `world.client_count()` is 2 and
`world.client(1)` is the second game:

```rust
fn a_change_reaches_the_second_client(world: &mut TestWorld<'_>) -> TestResult {
    let at = BlockPos { x: 3, y: -60, z: 3 };
    world.place_block(at, "minecraft:stone")?;
    world.run_ticks(2)?;                                       // the server sends the change
    world.client(1).wait_block(at, "minecraft:stone", 20_000)?; // real time, up to 20 s
    world.client(1).chat("hello")?;                             // client -> server -> everyone
    world.run_ticks(2)?;
    world.client(1).wait_chat("hello", 20_000)
}
```

| `world.client(1)` | does |
| --- | --- |
| `call(line)` | sends one command line of the game's control protocol (the one `horizon-ctl` speaks) and returns the JSON reply. Open to tests: `getblock`, `chat`, `chat-log`, `wait-for`, `status`, `events`, `key`, `release`, `look`, `turn`, `hotbar`, `mouse`, `tp`, `hello`; the commands that end the game, change its connection or write files are refused |
| `block_at(pos)`, `wait_block(pos, id, timeout_ms)` | the block the client has at `pos` (its own copy of the world, as the server sent it); wait until it is `id` |
| `chat(text)`, `wait_chat(text, timeout_ms)` | the client says `text` (plain text); wait until `text` is among its last 30 chat lines |
| `player_names()` | the players on the world the client joined, as the server lists them (the owner and `TestB`) |
| `world.client_count()` | 1, or 2 with `--yoake-test-clients=2` |

The waits repeat the command every 50 ms in real time (the second game is not frozen: the server world is, between
`run_ticks`). The server sends block changes when it ticks, so `run_ticks` before waiting. Measured on the real
game (debug build): a placed and a broken block both reached client 1, a chat line went client to server to client,
and the server's `say` and `tellraw @a "..."` both reached client 1. (The console's `say` used to reach no one in a published world, because the command only wrote to the server's own chat; it now goes to every player, as the dedicated console's does; `me` likewise, and `msg`/`tell`/`w` from the server reach the players they name.)

How it works: with `--yoake-test-clients=2` the worker also gets `--yoake-test-clients 2` and the path
`client1/control.json` beside its result. When its owner has joined, the worker opens its world to `127.0.0.1`
only (`Game::publish`, loopback, no announcement, a port the system picks), writes the port to a `port` file beside
its result and waits until the server lists two players and the second game's status shows its player (up to 240 s).
The coordinator sees the `port` file and starts the second game: `--offscreen --no-focus --no-pause --server
127.0.0.1:<port> --name TestB --control-socket client1/control.json` with its own `--game-dir`, `--home` and the
same packages installed, the same stripped environment and token. The second game proves itself as the worker does
(token file in its folder, `--offscreen`, a loopback server address only, its control endpoint in its folder; no
world, no `--dedicated`), loads no DLL mod and accepts the unsigned package under test, and ends with a lifetime
limit and when the coordinator is gone. It is in the same Windows job object, so it dies with the coordinator.
Its output tail goes into a failure, as the worker's does.

### Limits and what is not covered

* Tests run on the server side of a single-player world; client-only hooks (HUD, keys) cannot be driven by
  the driver (there is no input or screenshot call yet).
* The driver talks to the overworld only.
* A project with a native (DLL) package is not run (the test runner only starts data and Wasm packages).
* The result of a test is only what the driver can read; there is no assertion on a rendered frame.

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
`yoake-mods.txt` of its `--game-dir` (a server-only package is fine there). Development hot reload covers
both client worlds, swapping a fresh instance at the next frame; server modules are not hot-reloaded.
The server runs one world at a time
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

The client-only `gpu` imports additionally require `service.gpu`. They are not imported by
`server-full` or the frozen `client-mod` world. A GPU error does not grant another capability.

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

GPU resources are different from borrowed engine-object handles: the safe `full::client::gpu`
wrapper owns persistent `u32` resources and destroys them on drop. They belong to one client
instance, not to the server or another package, and are invalidated when that instance is replaced.

### GUI サイズと HUD の上中央配置

`client-full` の `full::client::gui_size()` は、現在のスケーリング済み GUI の幅と高さを
GUI ピクセル単位の `Vec2 { x, y }` で返します。`service.screen` が必要です。
`health_hud` に依存せず、クライアントホストが利用できる `frame`、`tick`、`hud`、
`gui_layer_pre`、`screen_opening`、`screen_init_post`、`screen_closing` などのフック内で使用できます。
クリエイティブ、スペクテイター、タイトル画面やポーズ画面でも同じ値を読み、リサイズ後の値も
反映します。スケール係数自体は返しません。`gpu_frame` にはクライアントホストがないため、
この関数は使用できません。代わりに `full::client::gpu::frame()` から取得できる幅と高さは、
スケーリング前のフレームバッファのピクセル数であり、GUI ピクセル数とは異なります。

`full::client::hud_place_v2(HudCornerV2::TopCenter, size)` は `service.hud` を必要とし、
`hud` フック内でのみ使用できます。各ラベルは水平中央に配置され、前の予約から 2 GUI ピクセル
離して下向きに積み重なります。予約は同じフレームのすべての mod で共有されます。
表示中のボスバーの下から開始し、mod が `boss_overlay` レイヤーをキャンセルしてもボスバーの
領域は確保されます。タイトルやサブタイトルを避けるため、配置の下端は画面中央の 48 ピクセル上を
越えません。小さい画面で多数のボスバーが表示される場合など、収まる領域がないときやサイズが不正な
ときは `Ok(None)` が返るため、そのラベルを描画しないでください。`hud` 以外ではエラーになります。
タブリストは後から描画されるため、上中央のラベルを覆うことがあります。左右のコーナーやその他の
任意のオーバーレイとの重なりも防ぎません。既存の `services::hud_place` は変更されません。

### GUI size and top-center HUD placement

`client-full`'s `full::client::gui_size()` returns the current scaled GUI width and height as
`Vec2 { x, y }` in GUI pixels. It requires `service.screen` and does not depend on `health_hud`.
It works in hooks with a client host, including `frame`, `tick`, `hud`, `gui_layer_pre`,
`screen_opening`, `screen_init_post` and `screen_closing`. It works in creative, spectator, title
and pause screens and reflects resizes; it does not return the scale factor. `gpu_frame` has no
client host, so it cannot call `gui_size`. Its `full::client::gpu::frame()` alternative provides
unscaled framebuffer width and height in physical pixels, not scaled GUI dimensions.

`full::client::hud_place_v2(HudCornerV2::TopCenter, size)` requires `service.hud` and is available only
within the `hud` hook. Labels are horizontally centered and stack downwards with a 2-GUI-pixel gap.
Reservations are shared by all mods in the same frame. The stack starts below visible boss bars and
reserves their space even when a mod cancels the `boss_overlay` layer. The bottom of a placement
cannot extend past 48 pixels above the screen center, leaving the title and subtitle area clear. If
there is no room, such as on a small screen with several boss bars, or if the size is invalid, it
returns `Ok(None)` and the caller should not draw that label. Calling outside `hud` returns an error.
The tab list draws later and can cover top-center labels; side corners and arbitrary overlays are
not reserved either. The original `services::hud_place` remains unchanged.

### 画面入力・インベントリ・音・食料・ブロックエンティティ（FR-S1）

ここは `client-full` / `server-full` の小さなサービスです。どれも、使う MOD がなければ何も変わりません。

* **画面の入力フックから送信する。** `net-send-to-server` は、クライアントのホストを持つフックならどこからでも呼べます。
  `screen-key-pressed-pre`、`screen-char-typed-pre`、`screen-mouse-*`、`screen-click` も含みます。
  以前は読み取り専用のフックで `unsupported` でした。確認と上限は変わりません（1 ティック 32 通、サーバーが MOD メッセージを扱うこと、参加済みであること）。
* **回転できるテクスチャー付き四角形。** `hud-command::quad` の `sprite` を指定すると、いまは描かれます。
  `sprite` は GUI アトラスのスプライト名か、`texture:<id>` です。`corners` は左上から順に回る 4 点、`uv` は各角の UV（0〜1）です。
  回転は角を回して渡します。SDK の `client::rotated_corners(x, y, w, h, degrees, pivot)` が使えます。
  `hud-command::texture` も、`id` が `client::gui_texture_id(&texture)`（サービス 225）の値なら描かれます。
  `id` は GUI テクスチャーを捨てるか、MOD のリソースを解放するまで有効です。
  知らない `id` は何も描きません。`item-decorations` などの戻り値のコマンドでも使えます。
  テクスチャー四角形の UV は、1 つ目と 3 つ目の角の長方形として使います。
* **クライアントのインベントリ読み取り。** `screen_carried()`（221、`service.screen`）は、開いている画面のカーソル上のスタックを返します。
  `local_player_stacks()`（222、`service.client-view`）は、ローカルプレイヤーの持ち物を返します。
  メインインベントリ（ホットバーが先）、防具、オフハンドの順で、空きスロットとカーソルは含みません。
* **ブロックエンティティの書き戻し。** `block-entity-tick`、`menu-button`、`menu-button-v2`、`menu-quick-move` が返す `entity`（`nbt`、`items`、`data`）を、エンジンが適用します。
  中身が変わったときだけ書き込み、ブロックエンティティは「変更あり」になります。保存と、コンテナの変化の通知は、既存の経路のままです。
  クライアントへの反映は、既存の `block-entity-sync`（58）で行います。チャンクの送信やブロックの更新で出る更新タグも、これまでどおりです。
  `block-entity-read` と `block-entity-patch-call`（59・60）は、これまでどおり `unsupported` です。
  フルワールドの MOD には、それが指すハンドルを渡す所がありません。読み取りは `entity` 引数、書き込みは戻り値を使ってください。
* **クライアントだけの音。** `play_local_sound(sound_id, volume, pitch)`（223）は、位置を持たない UI 音です。
  `play_local_sound_at(position, sound_id, volume, pitch)`（224）は、ワールド内の位置の音です。どちらも `service.audio` が必要です。
  音は次のフレームの頭に鳴るので、読み取り専用のフックからも呼べます。ID は `namespace:path`、音量は 0〜16、ピッチは 0〜8 です。
  待てるのは 64 個までです。
* **食料。** サーバーの `player_set_food(player, food, saturation)`（226、`player-write`）は、食料レベル（0〜20）と満腹度（0 から食料レベルまで）を設定します。
  `food-level-changed` フックには、ほかの変化と同じく聞こえます。
  アイテムごとの食料値を決めるフックは、まだありません。
* **ワールドの事前キーフック。** `key-pre(key, pressed, repeat, modifiers) -> bool` は、プレイ中のキーを、キーバインドより前に受け取ります。
  `true` を返すとキーは消費され、キーバインドも `key` フックも見ません。Escape は渡されないので、ポーズメニューは必ず開きます。
  押下を消費したら、そのキーの離しも消費してください。そうしないと、押下で始まったバインドが押されたままになることがあります。
  画面、タイトル画面、プロンプトの上では呼ばれません。
  `input-key` は、固定された WIT の型が ABI の `key-event` なので、そのままです。キーの内容が欲しいときは `key-pre` と `key` を使ってください。

### Screen input, inventory, sound, food and block entities (FR-S1)

Small services of `client-full` / `server-full`. None of them changes anything unless a mod uses it.

* **Sending from screen input hooks.** `net-send-to-server` may now be called from any hook that has the client host,
  including `screen-key-pressed-pre`, `screen-char-typed-pre`, `screen-mouse-*` and `screen-click`. It used to answer
  `unsupported` in read-only hooks. The checks and caps are unchanged (32 messages a tick, the server must carry mod
  messages, the player must have joined).
* **Textured, turnable GUI quads.** A `hud-command::quad` with a `sprite` is now drawn (it used to be skipped).
  `sprite` is a GUI atlas sprite name or `texture:<id>`. `corners` are four points round the shape from the top left and
  `uv` the UVs of those corners (0 to 1). To turn a quad, turn its corners: the SDK has
  `client::rotated_corners(x, y, w, h, degrees, pivot)`. `hud-command::texture` is drawn too when its `id` is the value of
  `client::gui_texture_id(&texture)` (service 225). The id stays valid until the texture is dropped or the mod's resources
  are released; an unknown id draws nothing. Both work in the commands a hook returns (`item-decorations`, `hud`, `screen-render-post`
  and so on). A texture quad uses the rectangle between its first and third UV.
* **Client inventory reads.** `screen_carried()` (221, `service.screen`) is the stack on the cursor of the open screen.
  `local_player_stacks()` (222, `service.client-view`) lists the local player's stacks as the client sees them: the main
  inventory (hotbar first), the armor and the off hand, without empty slots and without the cursor.
* **Block entity write-back.** The `entity` that `block-entity-tick`, `menu-button`, `menu-button-v2` and
  `menu-quick-move` return (`nbt`, `items`, `data`) is now applied. It is written only when it differs, which marks the block
  entity changed; saving and container-change notifications go the usual way. To show the change to clients call the existing
  `block-entity-sync` (58); the update tag sent with chunks and block updates is unchanged. `block-entity-read` and
  `block-entity-patch-call` (59, 60) still answer `unsupported`: a full-world mod has no place to get the handle they take.
  Read through the `entity` argument and write through the return value.
* **Client-only sound.** `play_local_sound(sound_id, volume, pitch)` (223) is a UI sound with no position;
  `play_local_sound_at(position, sound_id, volume, pitch)` (224) is a sound at a place in the world. Both need `service.audio`.
  The sound plays at the start of the next frame, so read-only hooks may call them. The id is `namespace:path`, volume 0 to 16,
  pitch 0 to 8, and at most 64 sounds wait.
* **Food.** The server's `player_set_food(player, food, saturation)` (226, `player-write`) sets the food level (0 to 20)
  and saturation (0 up to the food level). The `food-level-changed` hook hears it like any other change. There is no
  per-item food value hook yet.
* **Cancellable key hook in the world.** `key-pre(key, pressed, repeat, modifiers) -> bool` gets a key while playing, before
  the key bindings. Returning `true` consumes it: no binding and no `key` hook see it. Escape is never offered, so the pause
  menu always opens. If you consume a press, consume its release too, or a binding it started may stay held. It is not
  called over a screen, the title screen or a prompt. `input-key` stays unwired: its frozen WIT type is the ABI `key-event`;
  use `key-pre` and `key` for key data.

### What the host applies / does not

* HUD commands: text (plain), rect, gradient (strips), flat quads, sprites, items, clip/scale/blend runs.
  Skipped: `texture`, `highlight`, textured quads.
* Model commands: push/pop/translate/rotate/scale/light/keep, quad, block. Skipped: cube, model, item.
* Corner reservations and world-stage draws: applied. Client imports forward key bindings, view hits,
  display names and namespace ownership, GUI items, screen kind and slot rectangles, world quads,
  lines and billboards (including `world-pre-post`), post shaders, map tiles and colors, players and
  entities, camera overrides, writable GUI textures, PCM streams, mob-as-player, HUD placement, screen-carried / local-player-stacks reads, client-only local sounds, `gui-texture-id` (ABI IDs 221–225; the server catalog adds 226, `interface_version` 1.5.0), mod-screen IME enablement/candidate areas (ABI IDs 201–202), and
  runtime item/block sprite overrides (`item-sprite-set`, `item-sprite-reset`, `block-sprite-set`,
  `block-sprite-reset` — ABI IDs 211–214; plus read-only `item-sprite-pixels` / `block-sprite-pixels` — 215–216, `interface_version` 1.3.0). Runtime sprite overrides patch only the changed item
  sub-rectangle in the shared atlas or the changed mipmapped block texture layer, refuse mismatched
  pixel dimensions/byte lengths, allow replacing vanilla or other mods' sprites, re-apply across
  resource-pack reloads (skipping mismatched pack resolutions cleanly with a warning), and reset
  entries still owned by an unloading WASM mod. A block's item icon (the block shown in an inventory)
  is a separate copy in the item atlas: to change both the placed block and its icon, call both
  `block-sprite-set` and `item-sprite-set`. While an item sprite is overridden, the HUD atlas
  animation skips that rect (an animated item stays on the mod's image until `item-sprite-reset` or
  the mod's unload, then resumes).
  The catalog is `crates/horizon-mod-api-gen/abi-subset-client.json` (ABI IDs 163–197, 199–202,
  211–216 and 221–225; the server catalog adds 226). `*-sprite-pixels` return `(width, height, rgba8)` of the current sprite (a mod override
  included, an animated texture as its first frame) from the CPU copies of the atlases, so no GPU
  readback and no extra memory; block pixels are at the array layer size (`sprite_size`).
  Native services run under the calling WASM package's stable identity, so namespace validation and
  native per-mod limits also apply to WASM. Texture resources are instance-owned and capped at 64;
  failed creation does not consume a slot. Unsupported hook context returns an error, not a dummy
  success. The `client-services-demo` guest exercises real key, line, post-shader, bounded
  texture-write and IME imports, including error propagation, in the runtime integration tests.
* Server services: `server-services` (`crates/horizon-mod-api-gen/abi-subset-server.json`) answer,
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
  and 41 cursor. `entities-in` reads the level currently in place. `level-tick-pre` and
  `level-tick-post`, mob/entity tick pre/post hooks, and item tick pre/post hooks can query that
  level's entities. The post hooks see the restored entity stores. Server lifecycle hooks do not
  promise a per-level context. Client execution, a parked guest/host context and a different dimension
  return an error. While other dimensions are
  parked, potion and eye-of-ender transients are omitted because those stores do not record a dimension.
* A guest that subscribes to a hook with no adapter is refused at load (`UNWIRED` in the generated code;
  `crates/horizon-client/src/wasm_mods/full_adapters_report.txt` lists them and the partial ones).
* The adapters (`full_adapters.rs`) are generated by `horizon-mod-api-gen` and **committed**, like the WIT.
  After a catalog or generator change run `YOAKE_UPDATE_WIT=1 cargo test --offline -j 4 -p horizon-mod-api-gen`
  (a test fails when the committed file is out of date). The game's build does not generate them.
* Legacy draw-command adapters require a render-stage subscription. GPU commands recorded during a
  frame are queued for their specified stage; a subscribed frame hook can prepare them before that stage.
* The guest examples used by the tests are built by `yoake_wasm::test_guests` (feature `test-guests`) into
  `CARGO_TARGET_DIR/wasm-guests`, else `YOAKE_WASM_TARGET_DIR`, else a `target-wasm` folder next to the
  repository, else `target/wasm-guests`. Guest builds use offline Cargo and four jobs.
  With `YOAKE_REQUIRE_GUESTS=1`, `YOAKE_CI=1` or `CI`, a guest that cannot be built fails its test
  instead of skipping. `YOAKE_REQUIRE_GUESTS=1` also requires a usable adapter for the GPU guest test.
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

## Client GPU API

The GPU catalog is `crates/horizon-mod-api-gen/abi-subset-gpu.json` (ABI IDs 146–163).
WIT types and generated host adapters are committed. Use `yoake_wasm_sdk::full::client::gpu`
for safe ownership; the generated raw imports are available when a lower-level interface is needed.
Creation is rejected during `init`: create lazily in a subscribed client hook. Recording uploads,
copies, dispatches, draws and reads is allowed only while the host has an active frame.

| SDK operation | Contract |
| --- | --- |
| `Buffer::create`, `write`, `copy_to`, `range`, `read` | Explicit size and usage; checked ranges and copy alignment; reads return asynchronous tickets |
| `Texture::create`, `write` | Typed format, dimensions, mip count, usage and 2D-array/3D choice; tightly packed uploads with checked origins and extents |
| `Sampler::create` | Nearest/linear filtering and clamp/repeat addressing |
| `Shader::create`, `Shader::from_file` | Bounded WGSL validated and rewritten through Naga; structured error with line/column when available |
| `ComputePipeline::create`, `dispatch` | Validated entry point, bind groups and workgroup counts |
| `RenderPipeline::create`, `draw` | Typed vertex layout, color/depth format, depth write, blend, cull and line mode; optional vertex/index ranges and instancing |
| `BindGroup::create` | Pipeline-derived layout and explicit borrowed bindings, with retained host dependencies |
| `Readback::poll` | `Ok(None)` while pending, bytes once ready, error on failure; consumed after completion/error, or destroyed on drop |
| `frame` | Version, monotonic GPU frame index, tick, delta and viewport size |

Buffer usage flags are `COPY_SRC`, `COPY_DST`, `INDEX`, `VERTEX`, `UNIFORM` and `STORAGE`.
Texture flags are `COPY_SRC`, `COPY_DST`, `SAMPLED`, `STORAGE` and `RENDER`. Supported formats are
RGBA8 unorm/sRGB, RGBA16/32 float, R32 float, RG16 float, Depth32 float, BGRA8 unorm and RGB10A2 unorm.
Usage, format, binding and attachment mismatches return errors rather than bypassing wgpu validation.
`Draw` supports indexed/non-indexed and instanced draws. `Target::default()` selects the engine
stage target; explicit color/depth textures select an offscreen target, with optional color clear.

### Stages and read-only engine bindings

Commands execute in stage order, then mod load order, then each mod's recording order.
Engine-target draws preserve the vanilla attachment contents; offscreen targets do not replace them.
Engine depth uses reverse Z (`GreaterEqual`). No-mod operation records no GPU commands and creates
none of the mod-only frame resources or screen copies.

| `Stage` | Placement | Screen inputs |
| --- | --- | --- |
| `WorldOpaque` | After vanilla opaque drawing | Neither world depth nor scene color |
| `WorldTranslucent` | After vanilla translucent drawing | Read-only copied world depth |
| `AfterWorld` | After world rendering and screen-input copies | Read-only copied world depth and scene color |
| `Hud` | After the HUD, before menus | Neither world depth nor scene color |

Bind groups 0–2 are mod-owned. Group 3 is reserved for the engine and cannot be supplied by a mod:

| Binding | Read-only resource |
| --- | --- |
| 0 | Versioned `EngineFrame` uniform |
| 1 | Block/item atlas texture |
| 2 | Atlas sampler |
| 3 | World depth texture, restricted to `WorldTranslucent` and `AfterWorld` |
| 4 | Scene color texture, restricted to `AfterWorld` |
| 5 | Lightmap texture |

`ENGINE_WGSL` declares the `EngineFrame` struct and `@group(3) @binding(0)` uniform named `engine`.
Append the texture/sampler declarations your shader needs. The host checks their binding types and
stage availability, rejects engine writes, and supplies copies instead of sampling a live attachment.

### Frame ABI version 1

The uniform is 576 bytes, using column-major matrices and 16-byte-aligned fields. Check
`engine.header.x` for version 1 and `engine.header.y` for byte size. `header.zw` carry the low/high
tick words. The hook's `Frame.index` is the tick; `gpu::frame().index` is the GPU frame counter.

| Offset | WGSL field / type | Meaning |
| --- | --- | --- |
| 0 | `header: vec4<u32>` | Version, size, tick low, tick high |
| 16 | `view: mat4x4<f32>` | Current view |
| 80 | `projection: mat4x4<f32>` | Current unjittered projection |
| 144 | `view_projection: mat4x4<f32>` | Current unjittered view-projection |
| 208 | `inverse_view_projection: mat4x4<f32>` | Its inverse |
| 272 | `previous_view_projection: mat4x4<f32>` | Previous unjittered view-projection; current on the first frame |
| 336 | `jittered_view_projection: mat4x4<f32>` | Current jittered view-projection |
| 400 | `inverse_jittered_view_projection: mat4x4<f32>` | Its inverse |
| 464 | `camera_time: vec4<f32>` | Camera XYZ, elapsed seconds |
| 480 | `viewport_delta: vec4<f32>` | Width, height, delta seconds, tick as float |
| 496 | `sun: vec4<f32>` | Sun direction XYZ, W = 1 |
| 512 | `moon: vec4<f32>` | Opposite direction XYZ, W = 1 |
| 528 | `fog_color: vec4<f32>` | Fog color |
| 544 | `fog_range: vec4<f32>` | Fog range parameters |
| 560 | `jitter: vec4<f32>` | Jitter parameters |

### Limits and failure isolation

Handles are nonzero, owner-checked and kind-checked; on counter wrap the registry skips live IDs.
Destroying a handle
does not release memory still retained by a command, bind group, pipeline or readback. Accounting is
logical allocation accounting rounded to 64 KiB, **not measured driver VRAM**: 256 MiB per mod,
1 GiB shared, and 4,096 handles per instance. Allocation failure unwinds its reservation.
Buffers are 1 byte–256 MiB; textures are at most 8,192 pixels on either 2D axis, 256 array layers,
or 512 on a 3D axis, with validated mip counts. WGSL source is at most 256 KiB.

Each mod/frame permits 4,096 commands/draws, 65,536 instances per draw, 16 MiB uploads,
256 MiB buffer copies and 1 MiB requested readback bytes. A read is 1 byte–1 MiB; its retained
source and staging storage remain charged while pending. Upload budgets are checked before
allocating staging buffers or padded texture data.

Static safety caps estimate fragment work rather than charging every triangle as a full-screen
draw. A new mod, and every mod on an adapter without timestamps, receives 8,192 fullscreen-triangle
equivalents per frame (4 screen-units at the 2,048 overdraw depth). A fully ramped mod receives
2,097,152 fullscreen-triangle equivalents (1,024 screen-units) and 2^24 vertices multiplied by
instances. Fragment work is the viewport/scissor intersection's
covered pixels multiplied by the primitive count, accumulated across all draws of a mod/frame,
then divided by 2,048 once, rounding up at the end. Topology determines the primitive count,
including all instances. This assumes meshes have an overdraw depth of 2,048 primitives,
rather than charging a whole viewport or a separate rounding penalty per draw or instance.
Draws are admitted in submission order until the current fragment or vertex-instance allowance
would be exceeded. That draw and the owner's remaining draws are silently skipped for that
frame: exceeding this adaptive allowance is not an API error and does not disable the mod.
`draw_region` narrows that rectangle, while `draw` uses the whole target. Compute charges total
invocations multiplied by the shader's static instruction, texture and loop cost, with a
64 Mi-unit cap. Its allowance always admits at
least one maximum-size workgroup (256 invocations at the maximum static cost of 8,192).
Each dispatch dimension must also be nonzero and at most the device's
`max_compute_workgroups_per_dimension`, independently of that adaptive work allowance.
Invalid dimensions return `Workgroups` before the dispatch is recorded or submitted.
Consecutive draws to the same target share a render pass, with at most 64 passes per mod/frame.
Shader and pipeline creation attempts remain limited to two per frame and eight per minute,
including failed attempts and reloads. Depth and scene snapshots are shared once per stage
across mods; engine bind groups are cached per pipeline within each owner's batch.

At `after-world`, scene colour may intentionally be copied into a mod-owned texture and read back
to the CPU; the same 1 MiB per-mod/frame readback limit applies. HUD-stage GPU commands run only on
the real frame when frame generation is enabled, never on interpolated frames. Mods that never
read scene colour do not force a scene-read target. Scene-read targets stay allocated until
120 consecutive frames without a read. If a read is requested after that target has lapsed,
or the source lacks `COPY_SRC`, scene-reading commands are skipped for that frame without an
error or suspension; other commands still run, and the request makes the target sticky again
for subsequent frames. The frame descriptor reports the actual render target's dimensions and format.

The Naga IR guard caps loops at 256 iterations per function activation, including helper functions
and continuing blocks. Static limits include 8,192 instructions, 64 texture fetches, call depth 16,
64 helpers, 16 entry points, 4,096 expressions, 256 workgroup invocations and 16 KiB combined private
storage and function locals.
Workgroup-size overrides, barriers, subgroup/ray operations and workgroup storage are rejected.
Only the validated, rewritten Naga module is compiled; robustness checks are not relaxed.

With `TIMESTAMP_QUERY`, pass timestamps enforce at most 4 ms per frame averaged over a rolling
two-second window. Three frames over 50 ms within 60 seconds suspend that owner's GPU API for
the session without suspending its other services; older strikes expire. Each peak immediately
pauses new submissions for at least 50 ms. These measured-budget violations do not persist a
fingerprint quarantine.
Without timestamps, each mod's command batch is submitted separately. Its fallback sample is
the time between completion of the immediately preceding submission (engine or previous mod)
and its own completion, not the latency from submission through queued engine work. Only peaks
more than 200 ms above the rolling two-second baseline of engine-only submission intervals
count as strikes; three struck frames in 60 seconds suspend that owner's GPU API for the
session only. There is no fallback average-time limit or peak-triggered throttle. Callback
delivery timing can still affect these approximate measurements.
Each owner can have up to three outstanding frames. A fourth frame waits, and new submissions
also wait when the oldest outstanding submission is more than 100 ms old. A GPU one or two
frames behind does not by itself skip a mod's batch.
New mods start at 4 screen-units, 2^20 vertex-instances and 4 Mi compute-cost units, with the
same minimum workgroup allowance. On timestamp-capable adapters, eight distinct safe measured
frames double the vertex-instance and compute allowances and quadruple the fragment allowance,
reaching the full caps after 32 qualifying frames. Each qualifying frame must measure at least
0.25 ms and at most 4 ms, and use at least one quarter of its current fragment, vertex-instance
or compute allowance, aggregated across all measured stages. A frame that hits the adaptive
draw allowance satisfies that usage threshold even when later draws are skipped. The ramp is
per mod, not per pipeline, and slow frames reset it; idle, zero-cost, wholly skipped, unmeasured
and underused frames do not grow it. Without timestamps, each mod permanently receives the
initial allowance. There is no CPU-timing-based GPU ramp on those adapters.

Host recording/encoding/submission CPU time has a separate rolling two-second 4 ms/frame
budget and expiring peak strikes. Shader, pipeline and other resource creation calls are
excluded from that per-frame CPU budget, so compilation latency does not suspend legitimate
mods. They still count against the WASM aggregate host/guest budget and creation rate limits.
CPU timing is **not a substitute for GPU timing**. Pending timing samples are limited per
owner (32); each query set stays within wgpu's 4,096-query ceiling, and timing storage is charged.
Suspension survives renderer recreation. The engine's default uncaptured-error handler is
unchanged; mod resource creation errors are captured in mod scopes. These limits reduce risk,
but actual driver/TDR behavior still needs platform tests.

Accepted residual risk: a static estimate cannot both admit large meshes and reject every
pathological shader workload. On adapters with or without timestamps, a hostile mod can cause
very slow work before completion is observed, with up to three frames outstanding before the
backlog gate stops newer frames. Timestamp average/peak violations and fallback peak violations
disable GPU work for the session only. Already submitted work cannot be preempted: one frame
may include several stage batches, and its duration has no hard upper bound. A driver hang or
OS TDR remains possible even on the first frame with the 8,192-fullscreen-triangle-equivalent
allowance. A device loss or abnormal exit uses the persistent recovery marker below instead.

Before GPU submission, the host persists registered owner identities in
`<HORIZON_HOME>/state/wasm-gpu-inflight.json` only when the identity set changes, without fsync on
the frame path. An identity remains while work is outstanding, then for 120 idle frames (or until
clean exit); unchanged active sets are not rewritten. Failed writes are retried after 120 frames,
with warnings limited to once per minute, and are not attributed to the mod. Device loss blocks all in-flight identities in memory and in
the recovery file, including across renderer recreation. The next start quarantines those
identities and logs the marker path. Malformed recovery state fails closed. Abnormal
termination can also leave a marker: it is **not proof** that a listed mod caused a driver fault.
Quarantine does not expire through clean launches in which the blocked mod cannot run GPU work.
Only device loss/TDR recovery, including an outstanding marker after an abnormal exit, creates
persistent quarantine; measured-budget suspension alone never does. Fault counts persist per package-version/module-SHA-256
fingerprint, including across package changes and explicit re-enables. After a fingerprint's first
fault, changing the package can
automatically clear quarantine. After its second fault, only an explicit owner action can clear
quarantine, even if the package changes. Repeated observations of the same outstanding fault
do not increment its count.
`mod_gpu_runtime::reenable(identity)` explicitly clears quarantine and the live GPU suspension;
it rejects identities with outstanding work and persists the change before enabling them,
without erasing fault history.
This is the API for a future launcher/developer-screen re-enable control; no UI is added here.

### Development shaders and demo

With **both** `YOAKE_DLL_MODS_UNSIGNED=1` and `YOAKE_WASM_DEV_DIR=<folder>`, `Shader::from_file`
can override its embedded source from `<folder>/<mod-id>/<relative-path>.wgsl`. A missing file
initially uses the embedded source. Paths are relative WGSL files, canonicalized inside that mod's
directory; traversal, absolute paths, alternate streams and escapes are rejected. Limits are
64 tracked files per instance, 512 bytes per path and 256 KiB per file.

After a valid edit stays unchanged across two one-second polls, the host loads a fresh client
instance and GPU resources at the next frame. Invalid source or a failed component load leaves the
old instance running with a diagnostic; deleting an override also keeps the old instance. A valid
WGSL edit with a pipeline-interface mismatch can still fail when the new instance creates its
pipeline, disabling that mod. Override files must include `ENGINE_WGSL` when they use `engine`.

`crates/yoake-wasm-sdk/examples/gpu-demo` contains a 2,048-triangle textured mesh,
64 compute-updated particles drawn in a bounded region, and a depth-based after-world tint.
Its mesh, compute and particle workload is shared with a headless renderer regression; the
effect embeds `effect.wgsl`. Build from that directory, without running the game:

```sh
cargo build --offline -j 4 --release --target wasm32-wasip2
cargo build --offline -j 4 --release --target wasm32-wasip2 --features verify
```

The `verify` feature additionally checks GPU readback and rejected bounds/workgroup/allocation
requests; `reload` and `trap` variants exercise replacement and trap cleanup in host tests.
`docs/yoake-gpu-coverage.md` maps implementation and tests, including the strict guest test commands
and the remaining platform-validation obligations.
