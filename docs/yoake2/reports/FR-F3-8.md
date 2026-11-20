# FR-F3-8: levels and chunk generators for WebAssembly mods

Branch `yoake-free/f3-8` on `yoake-free/f3-7`, with `yoake-free/merge` @ `ce2c94760` merged first (one trivial
conflict in `game.rs`: both sides declared `mod level_manager;`). No Minecraft files, source or prepared resources
committed. Nothing pushed.

## What landed

### Catalog 1.2 of `abi-subset-server.json` (additive)
Services 227 to 234 (the highest id after the merge was 226), all `kind: level`, capability `world-control`
(`server-read` for the list). `interface_version` 1.1.0 to 1.2.0. Regenerated with
`YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib`: `yoake-full.wit` gains 8 functions and nothing
existing changes; the generator's own tests (including the 1.0 baseline "additive only" test, the committed
adapters and the coverage tests) pass.

| id | service | when |
| --- | --- | --- |
| 227 | `level-register-type(id, type-json)` | `init` |
| 228 | `level-register(id, stem-json)` | `init` |
| 229 | `level-create(id, stem-json, persistent)` | a hook |
| 230 | `level-delete(id)` | a hook |
| 231 | `level-unload(id)` | a hook (done at the next tick) |
| 232 | `level-regenerate(id, seed: option<s64>)` | a hook |
| 233 | `level-list() -> list<string>` | a hook |
| 234 | `generator-register(id)` | `init` |

Wiring (the S1 recipe): rows in the catalog JSON; the `ServerHost` trait lines in `yoake-wasm/src/full.rs`; the
`GameServerHost` implementations in `wasm_mods/server_services.rs`; the helper fragment
`tools/native-hook-fragments/f3_levels.json` naming `Game::native_levels_*` in the new `game/native_levels.rs`
(named `native_levels_*` because a hook called `native_level_unload` already exists); SDK wrappers
`yoake_wasm_sdk::full::server::ext::{register_type, register_level, register_generator, create_level,
delete_level, unload_level, regenerate_level, levels}`.
* The registering ones run in `init`, when no world exists: they validate and **stage** (the F3-6
  `mod_generator` staging, now with staged dimension types and replacing registration, since `init` runs again at each
  world start), and the world's level registry takes the staged types, levels and generators when it is built. So a player
  saved in a WebAssembly mod's level reopens there. In a hook they reach the registry directly and are refused after the first tick.
* The others run on the server's hook through the F3-6/F3-7 level manager (create is immediate and announced to the
  hub; delete and regenerate are immediate and refused for occupied and non-created levels; unload is queued
  and runs at the start of the next server tick because saving a level needs it in place). At most 64 created levels.

### The generator callback (no new hook export)
For a `yoake:mod` level the engine calls the package's existing `handle-call` export with the caller
`yoake:engine` and the method `yoake:generate-chunk`, on the thread that generates the chunk (F3-5's `ModColumnSource`,
`wasm_mods/level_services.rs`). Request: JSON (`level`, `generator`, `x`, `z`, `min_y`, `height`, `seed`, `settings`).
Answer: a documented binary fill (sections from the bottom, each one block or a palette of up to 4096 and 4096
`u16` indices, one biome for the chunk). The SDK has `generator::{METHOD, Request, Fill}`.
* **Validation**: the whole answer is checked before any block is written (version, flags, section count and range,
  duplicates, unknown blocks, palette size and indices, leftover bytes). Any failure, a guest that did not answer, one
  over its per-call limit, or a module that is disabled leaves the chunk empty and is logged once per level.
* **Time budget**: the guest's per-call limit (25 ms) applies. The generator's calls are made exempt from the *aggregate*
  budget (`ModInstance::set_aggregate_exempt`, new): the first run of the sample showed the aggregate window
  (25 ms per 50 ms, 20 strikes then disabled) skipped the calls while a world's first chunks generated at once, which
  would have left permanent air chunks. A hook holding the module is waited for up to 2 s.
* **Thread safety**: the module sits behind its slot's mutex (a mod-call peer is reachable the same way); generating
  threads run it one at a time.

### Level ids in payloads
String payloads already carry the real id (loot drops, native sounds and particles since F3-6, the level services
here). The `level-dimension` enum of hooks is frozen by the 1.0 baseline and still says `overworld` for a level past the
vanilla three; a guest gets the real id from `level-list` and the hook's own strings. A new enum case would be a
breaking change; a `current-level` service is the additive follow-up (not added).

### Sample mod and docs
`crates/yoake-wasm-sdk/examples/level-generator` (a `yak.toml`, a `server-full` guest): `init` registers
`levelgen:terrace_type` (64 high from y 0, blue sky, ambient 0.2), the generator `levelgen:terraces` and the level
`levelgen:terraces` (`step` setting 8); `handle-call` answers terraced hills (bedrock, stone, grass, one biome) as a pure
function of chunk, seed and settings. `yoake_wasm::test_guests::LEVEL_GENERATOR` builds it. Docs: a section "Levels and
chunk generators" in `docs/yoake-wasm-dev.md`.

## Tests
* `-p horizon-client --lib`, ignored, needs resources and the wasm32-wasip2 target:
  `a_wasm_mod_generates_the_chunks_of_its_level_and_they_save`: the guest is loaded as a world start loads it (its `init` stages
  type, generator and level); the world that opens has the level with the guest's type (0..64, ambient 0.2); two chunks
  generated through the guest have bedrock, stone, a grass top at the surface the guest computes and air above, in the right
  biome; an edit is saved into `dimensions/levelgen/terraces/region`; a level naming a generator whose package is not running
  stays registered and does not open (message names the generator); the vanilla levels are unchanged.
* `wasm_mods::level_services::tests` (answers decode; the fuzz the plan asks for: every truncation, bytes left over, a palette
  index past the palette, wrong section count/range/duplicates, unknown block/version/flags/kind, palette of 0 or too many, 2000
  random byte strings: no panic) and `service_tests` (what a guest passes is validated; outside a hook the services say so).
* `yoake-wasm`: `exempt_calls_are_not_counted_against_the_aggregate_budget`.
* Generator tests: the ABI baseline, adapters and coverage tests pass with the new rows.
* Parity: the F3-5 vanilla chunk hashes (ignored `vanilla_chunks_hash_...`) and the hub/registry byte tests pass unchanged.

Results: `-p horizon-client --lib` 1231 passed, 0 failed, 51 ignored; `-p horizon-client-lib --lib` 1796 passed; `-p horizon-world --lib` 385 passed; `-p yoake-wasm` (lib and all integration tests, guests built: `YOAKE_REQUIRE_GUESTS=1`) all pass (68 lib); `-p horizon-mod-api-gen --lib` 46 passed (after `YOAKE_UPDATE_WIT=1`); ignored real-resource `level_registry_tests` 11 passed (incl. the vanilla chunk hashes and the wasm test). No offscreen game smoke run.

## Not done / needs checking
* **F3-8c, full-access (DLL) bindings**: `yoake-bindgen` bindings, `ids.lock.json`, the hook points
  `worldgen::generate_chunk` and `level::before_open/after_open/before_close`, the ABI 1.1 note in `ENGINE_CALL_ABI.md` and
  the `MOD-GUIDE.md` chapter are not done; `crates/yoake-bindgen/regen.sh` was not run (the new engine functions are
  `pub(super)` helpers, not engine bindings, and the run needs rustdoc JSON of six crates on a disk with about 12 GB free).
  Run `regen.sh --check` before merging.
* **F3-8a, the data route** (`horizon-mods/src/content.rs` validation, `docs/mods.md`): a pack-only mod cannot declare
  generators or levels beyond what a data pack's `dimension` files already do; not touched.
* **F3-8d, the cooking-duel e2e** (a 4x4 grid level, two bots, regenerate, delete) is not written; the sample mod and the
  game test cover register, generate and save, and F3-7's e2e covers create, regenerate and delete through `horizon-ctl`.
* The services were exercised through the game tests and unit tests, not through a guest calling `level-create` and the
  rest in a running server hook (needs a tick with a native mod call); `level-unload` is queued and the queue is run by
  `run_pending_level_ops`, tested only indirectly through the F3-7 `unload_level`.
* A WebAssembly server module's `init` runs at process start, before any world: a world already open when a mod is
  loaded later (developer reload) does not see its staged levels until the next world.
* A generator whose answer needs more than the per-call limit leaves empty chunks (and they are saved so); there is no
  retry. A level whose generator's package was removed does not open until the package returns or the stem changes.
* The server-side compile of the sample uses `wit-bindgen` offline from the cargo cache; a machine without it skips the
  test unless `YOAKE_REQUIRE_GUESTS=1`.
