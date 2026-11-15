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
and a `tellraw @a "..."` of the server reached client 1; `say` from the console did **not** reach the client's chat
log (use `tellraw`).

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
* Time spent inside a server service (the game breaking a block, reading a container, another package's
  `handle-call`) is the game's, not the guest's: it extends the call's deadline (by at most 400 ms in all)
  and does not count against the aggregate budget; the guest's own work stays bounded by fuel and the
  clock, and the native helpers allow 512 to 1024 calls per mod and server tick.
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
feature, `client` and `server`).

### What the host applies / does not

* HUD commands: text (plain), rect, gradient (strips), flat quads, sprites, items, clip/scale/blend runs.
  Skipped: `texture`, `highlight`, textured quads.
* Model commands: push/pop/translate/rotate/scale/light/keep, quad, block. Skipped: cube, model, item.
* Corner reservations: applied. Frame-stage commands (`world-drawn`, `hand-drawn`, ...): not applied yet.
* Server services: all 84 of `server-services` (`crates/horizon-mod-api-gen/abi-subset-server.json`) answer,
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
  | `world-storage-read/write`, `config-read/write` (per package folder `yoake-wasm/<id>`, keys of letters, digits `_ . -`, 1 MiB a value, 256 keys, 32 MiB), `asset-read` (`data/<key>` of the game's resources), `qa-mode-enabled` (false), `qa-record` (debug log) | forwarded (WASM layer) |
  | `play-sound-at` | new (WASM layer: every player in earshot) |
  | `break-block-as` (the game's own player break: drops with Fortune/Silk Touch, tool damage, XP, stats, mod events), `player-sneaking` (guests, the LAN host and single player's owner), `container-slots`, `container-set-slots` (same items, totals verified; plain containers, both halves of a double chest), `containers-in`, `player-set-slots` (36 main slots, totals verified), `block-tags`, `item-tags`, `block-tool-requirement`, `can-harvest`, `attribute-get`, `item-entities-in`, `item-entity-move`, `item-entity-pickup`, `give-items` (remainder returned), `inventory-apply` (exact: id + count + components, all or nothing; `give` needs room for all), `advancement-progress`, `advancement-grant`, `entity-vitals-of` | new |
  | `mod-call` | new: `target-mod`, `method`, `payload` (1 MiB); the callee exports `handle-call(caller, method, payload)` of the `server-full` world; no call into a caller that is waiting (it is busy), at most 4 nested, never to itself |
  | `block-entity-read`, `block-entity-patch-call` | still missing: a block-entity hook returns its new state directly (no native service to forward) |
  | `range-override-set/clear` | deferred (Nestgate voice, W5) |

  `handle-call` is a new export of the `server-full` world: a module built before it cannot be loaded by
  this host (rebuild with the SDK). A service called from a read-only hook (the hook has no `&mut` host)
  answers `Err("unsupported: ...")`; the services that change the game need a hook that may.
* A guest that subscribes to a hook with no adapter is refused at load (`UNWIRED` in the generated code;
  `crates/horizon-client/src/wasm_mods/full_adapters_report.txt` lists them and the partial ones).
* The adapters (`full_adapters.rs`) are generated by `horizon-mod-api-gen` and **committed**, like the WIT.
  After a catalog or generator change run `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib`
  (a test fails when the committed file is out of date). The game's build does not generate them.
* A package takes part in the render pass only when its client module subscribed to a render-stage hook.
* The guest examples used by the tests are built by `yoake_wasm::test_guests` (feature `test-guests`) into
  `YOAKE_WASM_TARGET_DIR`, else a `target-wasm` folder next to the repository, else `target/wasm-guests`.
  With `YOAKE_CI=1` (or `CI`) a guest that cannot be built fails its test instead of skipping.
