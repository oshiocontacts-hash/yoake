# Chapter 6: Testing Your Mod Inside the Game Engine

In the previous chapters, you learned how to create, build, and package a WebAssembly (Wasm) mod. This chapter explains how to **automatically test world-manipulation scenarios (and `mod_call` invocations into server-side mods) inside the real game engine (`horizon-client`)**. Note that because a test module runs as a separate component that only drives the server-side world, **it cannot test client-only mod behavior (such as HUD rendering)**.

When you run Yoake's test runner (`yoake test`), the game launches offscreen without opening a window (`--offscreen`), generates a fresh superflat world for each test, and runs your test functions. You can also test world-interaction scenarios quickly against a local mock driver without starting the game at all.

---

## 1. Preparing to Write Tests

### How Test Modules Work

In Yoake, tests are compiled as a separate WebAssembly component called a **test module** (using the `test-mod` WIT world defined in `crates/yoake-wasm/wit/yoake-test.wit`), distinct from your mod itself (for example a `client-mod`):

- A normal build produces your mod binary (`bin/wasm/<id>.wasm`).
- Building with the Cargo `tests` feature enabled produces the test module (`bin/wasm/<id>-tests.wasm`) from the same crate.
- The test module is not listed in `yak.toml` and is never bundled into distributable `.yak` packages, so test code can never accidentally slip into a release package.

### Configuring `Cargo.toml` (the `tests` Feature)

Projects created with `yoake new` already include this, but if you are adding it by hand, define `tests = ["yoake-wasm-sdk/test"]` in `Cargo.toml` (source: `docs/yoake-wasm-dev.md` / `crates/yoake-cli/src/scaffold.rs`):

```toml
[features]
tests = ["yoake-wasm-sdk/test"]
```

This enables the SDK's `test` feature (`yoake_wasm_sdk::testing::engine` and the `export_tests!` macro) only during test builds.

### Registering Tests with `#[yoake_test]` and `export_tests!`

Each test is a function that takes `&mut TestWorld<'_>` and returns `TestResult` (`Result<(), String>`). Returning `Ok(())` passes the test, while returning `Err(message)` fails it (you can use the `?` operator to propagate driver errors directly).

Here is the setup from `docs/yoake-wasm-dev.md` (replace `Starter` with your mod's struct name):

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

As in the `yoake new` template (`crates/yoake-cli/src/scaffold.rs`), you can also annotate test functions with `#[yoake_wasm_sdk::yoake_test]` (or `#[yoake_test]` after `use yoake_wasm_sdk::yoake_test;`):

```rust
#[yoake_wasm_sdk::yoake_test]
fn example_world_test(world: &mut TestWorld<'_>) -> TestResult {
    let position = BlockPos { x: 0, y: 64, z: 0 };
    world.place_block(position, "minecraft:stone")?;
    world.run_ticks(1)?;
    world.assert_block(position, "minecraft:stone")
}

pub const YOAKE_TESTS: &[TestCase] = &[TestCase { name: "example_world_test", run: example_world_test }];
```

Note: The `yoake new` starter template uses `y: 64` (high up in the air), whereas if you want to place blocks right on the ground surface in the superflat test world, use `y: -60` as in the earlier example.

- **`#[yoake_test]`**: A marker attribute indicating a test function (it takes no arguments; passing arguments fails compilation with `"yoake_test does not accept arguments"`). Because this attribute alone does not auto-register the function, you must still list every test in the `YOAKE_TESTS` slice (`&[TestCase]`).
- **`export_tests!(YOAKE_TESTS)`**: Exports `YOAKE_TESTS` as the test module's `tests` and `run-test` exports when compiled with `feature = "tests"`.
- **Test naming rules**: Each name must be 1 to 128 characters long, unique within the module, with at most 1,024 tests per module. Because test names are passed as command-line arguments, a test name cannot start with `-`.

---

## 2. Complete `TestWorld` and `TestDriver` Reference

The `TestWorld` passed to your test function forwards every action to an underlying `TestDriver` trait implementation (`engine::HostDriver` inside the real game, or your own mock struct during `cargo test`). Inside the game, all operations run between ticks on the integrated server thread.

### Public `TestWorld` Methods (`crates/yoake-wasm-sdk/src/testing.rs`)

| Function | Behavior | Example |
| --- | --- | --- |
| `TestWorld::new(driver)` | Creates a new `TestWorld` wrapping a mutable reference to a `TestDriver` | `let mut world = TestWorld::new(&mut driver);` |
| `place_block(position, block)` | Places a block at the given Overworld coordinates (`/setblock x y z <block>`) | `world.place_block(ORIGIN, "minecraft:stone")?;` |
| `block_at(position)` | Returns the block ID string at the given coordinates (`namespace:id` without block state properties) | `let id = world.block_at(ORIGIN)?;` |
| `spawn_entity(kind, position)` | Summons an entity at the given coordinates and returns its entity ID (`u64`) | `let pig = world.spawn_entity("minecraft:pig", ORIGIN)?;` |
| `run_ticks(ticks)` | Advances the server by `ticks` ticks (1–72000) immediately without sleeping | `world.run_ticks(3)?;` |
| `break_block(position)` | Breaks the block at `position` with drops as if broken by a player (`/setblock ... destroy`) | `world.break_block(ORIGIN)?;` |
| `spawn_mannequin(name, position)` | Summons a non-player mannequin with a deterministic UUID derived from `name` and returns its ID | `let player = world.spawn_mannequin("Tester", ORIGIN)?;` |
| `run_command(command)` | Executes a command as the server console and returns whether it succeeded (`bool`) | `let ok = world.run_command("say hi")?;` |
| `assert_command(command)` | Runs `run_command` and fails the test if it returns `false` | `world.assert_command("say hi")?;` |
| `mod_call(target, method, payload)` | Calls `handle-call` on a loaded server module (either the mod under test or one added via `--yoake-test-with`) | `let echoed = world.mod_call("demo", "echo", b"ping")?;` |
| `entities_near(position, radius)` | Returns the entity type IDs (up to 512) within `radius` blocks of `position` | `let kinds = world.entities_near(ORIGIN, 4.0)?;` |
| `player_names()` | Returns the list of player names on the server (with the world owner first) | `let names = world.player_names()?;` |
| `tick()` | Returns the number of server ticks the world has advanced since the test started | `let t = world.tick();` |
| `log(message)` | Prints one line to the test runner's standard error (the test log) | `world.log("logged");` |
| `assert_entity_count(kind, position, radius, count)` | Asserts that the number of `kind` entities within `radius` of `position` equals `count` | `world.assert_entity_count("minecraft:pig", ORIGIN, 4.0, 1)?;` |
| `assert_block(position, expected)` | Asserts that the block ID at `position` equals `expected` | `world.assert_block(ORIGIN, "minecraft:stone")?;` |

### `TestDriver` Trait and Helper Functions (`testing.rs` / `testing/engine.rs`)

When writing a mock for `cargo test`, implement the `TestDriver` trait. The first 4 methods are required, while the remaining methods have defaults (`tick` returns `0`, `log` is a no-op, and the rest return `Err("unsupported: …")`), so you only need to implement what your test uses. Note that `engine::client_count` and `engine::run_with` are available only when the SDK's `test` feature (`yoake-wasm-sdk/test`) is enabled.

| Function / Method | Behavior | Example |
| --- | --- | --- |
| `TestDriver::place_block(position, block)` | **[Required]** Implements placing a block | `driver.place_block(ORIGIN, "minecraft:stone")?;` |
| `TestDriver::block_at(position)` | **[Required]** Implements reading a block ID | `let id = driver.block_at(ORIGIN)?;` |
| `TestDriver::spawn_entity(kind, position)` | **[Required]** Implements spawning an entity | `let id = driver.spawn_entity("minecraft:pig", ORIGIN)?;` |
| `TestDriver::run_ticks(ticks)` | **[Required]** Implements advancing server ticks | `driver.run_ticks(2)?;` |
| `TestDriver::break_block(position)` | **[Optional]** Breaks a block (default: `Err("unsupported: break_block")`) | `driver.break_block(ORIGIN)?;` |
| `TestDriver::spawn_mannequin(name, position)` | **[Optional]** Spawns a mannequin (default: `Err("unsupported: spawn_mannequin")`) | `let id = driver.spawn_mannequin("Tester", ORIGIN)?;` |
| `TestDriver::run_command(command)` | **[Optional]** Runs a command (default: `Err("unsupported: run_command")`) | `let ok = driver.run_command("say hi")?;` |
| `TestDriver::mod_call(target, method, payload)` | **[Optional]** Calls a server mod (default: `Err("unsupported: mod_call")`) | `let res = driver.mod_call("demo", "echo", b"ping")?;` |
| `TestDriver::entities_near(position, radius)` | **[Optional]** Queries nearby entities (default: `Err("unsupported: entities_near")`) | `let list = driver.entities_near(ORIGIN, 4.0)?;` |
| `TestDriver::player_names()` | **[Optional]** Lists player names (default: `Err("unsupported: player_names")`) | `let names = driver.player_names()?;` |
| `TestDriver::tick()` | **[Optional]** Returns elapsed ticks (default: `0`) | `let t = driver.tick();` |
| `TestDriver::log(message)` | **[Optional]** Logs a message (default: no-op) | `driver.log("logged");` |
| `TestCase::run(&self, driver)` | Runs a single `TestCase` against `driver` | `case.run(&mut driver)?;` |
| `engine::client_count()` | Returns the number of connected clients (`1`, or `2` in two-client mode; requires SDK feature `test`) | `let n = yoake_wasm_sdk::testing::engine::client_count();` |
| `engine::run_with(tests, name, driver)` | Finds the test matching `name` in `tests` and runs it with `driver` (requires SDK feature `test`) | `yoake_wasm_sdk::testing::engine::run_with(YOAKE_TESTS, "places_a_block", &mut driver)?;` |

---

## 3. Running Tests

### Running with `yoake test`

Running `yoake test` inside your project folder builds your mod, runs `yoake check`, compiles the test module with `--features tests`, and launches the game to execute your tests (source: `docs/yoake-wasm-dev.md`):

```sh
export YOAKE_GAME=/path/to/horizon-client      # the game executable (Yoake build)
yoake test                                      # in the project folder: build, check, run the tests
yoake test --filter places --timeout-ms 60000 -- --resources /path/to/prepared/resources
```

Main `yoake test` options (`crates/yoake-cli/src/engine_test.rs`):
- `--game EXE`: Path to the game executable (defaults to the `YOAKE_GAME` environment variable)
- `--filter TEXT`: Runs only tests whose name contains `TEXT`
- `--timeout-ms N`: Per-test timeout in milliseconds (forwarded to the game's `--yoake-test-timeout-ms`; defaults to `30000`)
- `--module WASM`: Explicitly specifies the path to the test module
- `--keep`: Keeps the temporary test folders on disk instead of deleting them
- `-- GAME-ARGS...`: Forwards any arguments after `--` (such as `--resources <PATH>`, GPU-free `--headless`, or `--yoake-test-startup-seconds <N>`) directly to the game executable

**Output**:
- Standard error (`stderr`) prints per-test progress lines (`yoake test: ok   <name> (<ms> ms)` or `yoake test: FAIL <name> (<ms> ms)`) and error diagnostics.
- Standard output (`stdout`) prints **a single line of JSON** at the end (exit code `0` if all tests pass, or `1` otherwise), which also includes `game` and `wasm_needs` (`crates/yoake-cli/src/engine_test.rs`):

```json
{"ok": false, "built": true, "checked": true, "id": "my-mod", "version": "0.1.0",
 "tests": [{"name": "places_a_block", "ok": true, "ms": 412},
           {"name": "breaks_it", "ok": false, "ms": 380, "error": "block at BlockPos { .. }: expected ..."}],
 "errors": ["test breaks_it failed: block at BlockPos { .. }: expected ..."], "notes": []}
```

You can also run `yoake dev --test` to execute your in-engine tests and populate the `tests` array in its output (`yoake dev` without `--test` does not start the game and leaves `tests` empty).

### Running `horizon-client --yoake-test` Directly

The game executable's direct test interface is `horizon-client --yoake-test <project folder | .yak> [options]` (`--yoake-test` **must be the very first argument**).

| Option | Meaning |
| --- | --- |
| `--yoake-test-json FILE` | Writes the JSON result report to `FILE` in addition to standard output |
| `--yoake-test-module FILE` | Path to the test module (defaults to `bin/wasm/<id>-tests.wasm` for a project folder; required when testing a `.yak` because `.yak` archives do not bundle tests) |
| `--yoake-test-with PATH` | Additional mod package (folder or `.yak`) to load alongside the mod under test; can be passed multiple times |
| `--yoake-test-filter TEXT` | Runs only tests whose name contains `TEXT` |
| `--yoake-test-timeout-ms N` | Per-test timeout in milliseconds (default: `30000`) |
| `--yoake-test-startup-seconds N` | Grace period from worker launch until the test starts (default: `240` seconds) |
| `--yoake-test-keep` | Preserves temporary folders (game directory, world, logs) and records their path under `"kept"` in the JSON output |
| `--yoake-test-clients=1\|2` | Number of participating clients (`2` is available on `yoake/p10-test-runner` @ `bdc976398` and newer; see Section 5) |

**Output**:
- Prints `yoake test: ok  ` / `FAIL` lines for each test to standard error (`stderr`), and a single-line JSON report `{"ok": ..., "id": ..., "version": ..., "clients": 1, "module": ..., "declared": ..., "tests": [...], "errors": [...]}` to standard output (`stdout`).
- Exit code is `0` (all tests passed), `1` (one or more tests failed), or `2` (tests did not run due to invalid CLI arguments, a missing module, etc.).

---

## 4. Superflat World Assumptions and Execution Rules

The world where your tests run follows these rules (source: `docs/yoake-wasm-dev.md` and verified in a real run (2026-11-16)):

1. **Superflat layers and coordinates (verified in a real run (2026-11-16))**:
   - The world is a superflat world with seed `0` and structures turned off.
   - Its ground layers from bottom to top are: **`y = -64` is bedrock, `y = -63` and `y = -62` are dirt, `y = -61` is grass block, and `y = -60` and above are air**. Examples place blocks at `y = -60` because that is the first air layer right above the grass.
2. **Initial state and working near spawn**:
   - The world owner joins near spawn (`0, 0`) in Creative mode (`--creative`), with time set to `6000` (`--time 6000`), clear weather (`--weather clear`), and a render distance of `4` chunks (`--render-distance 4`). Place blocks and entities used by your tests near `(0, 0)`.
   - The owner joins with an automatically generated name matching **`PlayerNNNNN`** (verified in a real run (2026-11-16)).
3. **The world is frozen and advances only when you call `run_ticks`**:
   - After the owner joins, the world runs for 2 seconds of real time so chunks can finish loading (warmup). Because of this warmup, **the world has already advanced by roughly 87–103 ticks when your test function starts** (do not assume `world.tick()` starts at `0`; verified in a real run (2026-11-16)).
   - Right after warmup, **the world is frozen**. While frozen, server ticks never advance on their own; they advance by the exact number of ticks you request via `world.run_ticks(n)` (skipping the normal 50 ms per-tick sleep to run as fast as possible).
4. **Each test spawns a fresh process and world, which takes time**:
   - To completely isolate tests from leftover state or hangs, **every single test starts a brand-new worker process and temporary world**.
   - While the test body itself finishes in a few to a few hundred milliseconds, starting the worker and preparing the world takes time (measured at 90–100 seconds per test on a Windows debug build without a `horizon-data.bin` cache; faster with a release build and warm cache).
   - Keep the number of separate test cases small by grouping related assertions into one test, and use `yoake test --filter <name>` during development to run one test at a time.

---

## 5. Two-Client Multiplayer Tests

Note: This feature requires commit `bdc976398` or newer on `yoake/p10-test-runner` (the SDK on this branch is older, so update to `bdc976398` or later to use it).

Passing `yoake test --clients 2` or `horizon-client --yoake-test <target> --yoake-test-clients=2` (`--yoake-test-clients 2`) runs multiplayer tests with 1 server and 2 clients. Client `0` is the world owner (the worker itself), and client `1` is a second game process (player name `TestB`) that joins over loopback (`127.0.0.1`).

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

### `world.client_count()` and `world.client(1)` (`ClientView`) Methods (`testing.rs` @ `bdc976398`)

| Function / Signature | Behavior |
| --- | --- |
| `world.client_count(&mut self) -> u32` | Returns the number of participating clients (normally `1`, or `2` with `--yoake-test-clients=2`) |
| `world.client(&mut self, index: u32) -> ClientView<'_, 'driver>` | Returns a `ClientView` handle for client `index` (`1` is the second game process) |
| `ClientView::call(&mut self, command: &str) -> Result<String, String>` | Sends one line of the game's control protocol (same format as `horizon-ctl`) and returns the JSON response |
| `ClientView::block_at(&mut self, position: BlockPos) -> Result<String, String>` | Returns the block ID at `position` in that client's own local world view |
| `ClientView::wait_block(&mut self, position: BlockPos, expected: &str, timeout_ms: u32) -> TestResult` | Waits in real time (up to `timeout_ms` ms) until `position` on that client matches `expected` |
| `ClientView::chat(&mut self, text: &str) -> TestResult` | Sends a plain chat message `text` (without a leading `/`) from that client |
| `ClientView::wait_chat(&mut self, text: &str, timeout_ms: u32) -> TestResult` | Waits in real time (up to `timeout_ms` ms) until `text` appears in that client's last 30 chat lines |
| `ClientView::player_names(&mut self) -> Result<Vec<String>, String>` | Returns the player names in the world that client is connected to (the owner and `TestB`) |

(The `TestDriver` trait for mocks also includes `client_count`, `client_call`, `client_wait`, and `client_player_names` with default implementations.)

- **Wait functions (`wait_block` / `wait_chat`) and real time**: While the server world is frozen between `run_ticks` calls, the second client process is not frozen. `wait_*` polls every 50 ms in **real time** up to `timeout_ms` milliseconds. Because the server only broadcasts changes when ticks advance, always call `world.run_ticks(...)` before waiting on client `1`.
- **Known caveat with the `say` console command (under investigation)**: Running `say` from the server console does not reach the second client's chat log (under investigation). To broadcast a message from the server so that client `1` receives it, use `tellraw @a "..."` instead.

---

## 6. Common Failures and How to Fix Them

| Error Message / Symptom | Cause and Fix |
| --- | --- |
| `yoake test needs the game: pass --game <executable> or set YOAKE_GAME` | No path to the game executable was given. Set the `YOAKE_GAME` environment variable to your `horizon-client` path or pass `yoake test --game <path>`. |
| `the project has no tests: add a tests Cargo feature that calls export_tests!, or pass --module` | `Cargo.toml` is missing `[features] tests = ["yoake-wasm-sdk/test"]` or no test module was built. Add the feature and `export_tests!(YOAKE_TESTS);`. |
| `the test module ... does not exist: run yoake build (or give --yoake-test-module)` / `the <side> module ... is missing: run yoake build first` | You ran `horizon-client --yoake-test` directly before building the `.wasm` files into `bin/wasm/`. Run `yoake build` first or pass `--yoake-test-module`. |
| `a .yak carries no tests: pass the test module with --yoake-test-module` | `.yak` archives do not include test modules. When testing a `.yak` directly, also pass `--yoake-test-module bin/wasm/<id>-tests.wasm`. |
| `... is a link or reparse point; move the project to a normal folder without links or reparse points` | Your project sits under a symbolic link, Windows junction, or synced cloud folder. Move it to a regular directory. |
| `<option> cannot be used with --yoake-test: each test gets its own world and game folder` | Options like `--world`, `--seed`, `--game-dir`, `--dedicated`, and `--fullscreen` are managed by the test runner and cannot be overridden. To point at prepared vanilla assets, pass `-- --resources <PATH>`. |
| A typo in `--filter` matches 0 tests, yet the command exits with code `0` and `"ok": true` | Matching 0 tests is not treated as an error; it exits cleanly with `"notes": ["no test matches the filter"]` (or `"note": "no test matches the filter"` when invoked directly; `coordinator.rs`). Check that the `tests` array in the JSON output is not empty. |
| `block at ...: expected "minecraft:oak_stairs[facing=north]", got "minecraft:oak_stairs"` | `block_at` and `assert_block` return only the base block ID (`minecraft:oak_stairs`) without state properties. Pass only the block ID (such as `"minecraft:oak_stairs"`) to `assert_block`. |
| `not a block state: ...` / `not a registry id (namespace:path): ...` / `not a player name (1 to 16 letters, digits, _): ...` | Argument format error. Use lowercase `namespace:path` IDs like `"minecraft:pig"` for entity kinds, and 1–16 ASCII letters, digits, or `_` for mannequin names. |
| `the test module trapped or ran out of time: ...` / `the worker did not finish within ... s and was killed` | The test hit a `panic!`, entered an infinite loop, or timed out. On slower machines, increase startup grace time via `yoake test -- --yoake-test-startup-seconds 600` (after `--`), or raise `--timeout-ms` for long tests. |
| Calling `entities_near` or `spawn_entity` many times in a row fails midway | While the world is frozen without calling `run_ticks`, every call shares the same server tick's host-call budget (512–1,024 calls). Call `world.run_ticks(1)?;` to advance to a fresh tick. |

---

## 7. Features Not Yet Implemented

The following features are not implemented yet (source: `docs/yoake-wasm-dev.md`, "Limits and what is not covered"):

- **Driving client-only hooks (HUD rendering, key input, etc.) or asserting on rendered frames (not implemented)**:
  - Tests run on the server side of a single-player world, so key input, screenshots, and rendered frames cannot be checked, and there is no way for the test driver to trigger client-only hooks such as HUD rendering (not implemented).
- **Operating on dimensions other than the Overworld (not implemented)**:
  - The test driver can only operate on the Overworld (not implemented).
- **Running tests for native (DLL) mod packages (not implemented)**:
  - The test runner only loads data packages and Wasm packages; native (DLL) packages are not executed (`native (DLL) packages are not run by the test runner`).
