# FR-REM04: field tables for the five W10 crates and `Game.options`

Branch: `yoake-free/rem04-tables` (from `yoake-free/merge` `c01031597`).

Env used for all builds/tests: `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_TARGET_DIR=C:\Users\USER\NestgateHorizon\horizon-rem04\target`, `PATH` prepended with `C:\Users\USER\NestgateHorizon\toolchain\mingw64\bin`, `-j 6`. Disk before building: `39G` free on `/c` (`C: 476G 438G 39G 92% /c`). Game binary was not started.

## 1. Field tables for the five W10 crates

`yoake-bindgen --tables` was run with rustdoc JSON (`cargo rustdoc --offline -j 6 -p <crate> --lib -- -Z unstable-options --output-format json --document-private-items`) for the five W10 crates, placing each table tree where `yoake-hook-codemod`'s `overlay_bridge.rs` includes it (`crates/<crate>/src/yoake_glue/tables/<crate_ident>/`):

| crate | directory | files | structs (`FieldTable` / `RefFieldTable` pairs) | scalar field getters | setters |
|---|---|---:|---:|---:|---:|
| `horizon-client-render` | `crates/horizon-client-render/src/yoake_glue/tables/horizon_client_render/` | 159 | 493 | 1,787 | 0 (read-only) |
| `horizon-core` | `crates/horizon-core/src/yoake_glue/tables/horizon_core/` | 30 | 55 | 106 | 0 (read-only) |
| `horizon-protocol` | `crates/horizon-protocol/src/yoake_glue/tables/horizon_protocol/` | 22 | 203 | 451 | 0 (read-only) |
| `horizon-items` | `crates/horizon-items/src/yoake_glue/tables/horizon_items/` | 8 | 23 | 86 | 0 (read-only) |
| `horizon-command` | `crates/horizon-command/src/yoake_glue/tables/horizon_command/` | 4 | 7 | 10 | 0 (read-only) |
| **Total (5 W10 crates)** | | **223** | **781** | **2,440** | **0 (read-only)** |

Existing table directories synced:
- `horizon-client` (`crates/horizon-client/src/yoake_glue/tables/horizon_client/`): **178** files, **355** structs, **1,166** scalar fields (`game.rs` regenerated without inline `Options`; added `game__launch_options.rs` with 1 struct / 14 scalar field getters).
- `horizon-client-lib` (`crates/horizon-client/src/yoake_glue/tables/horizon_client_lib/`): **188** files, **478** structs, **1,930** scalar fields (unchanged).
- `horizon-world` (`crates/horizon-world/src/yoake_glue/tables/horizon_world/`): **141** files, **262** structs, **660** scalar fields (unchanged).

To allow rustdoc universe loading for all W10 crates and future checks:
- Added `target/doc/horizon_protocol.json` and `target/doc/horizon_command.json` to `crates/yoake-bindgen/bindgen.json` `"docs"`.
- Added `"horizon-protocol/src/crypto/blocks.rs"` to `"frozen_files"` in `crates/yoake-bindgen/bindgen.json` (policy gate for `yoake-bindgen`).
- Updated `crates/yoake-bindgen/regen.sh` and `crates/yoake-bindgen/tests/engine.rs` to generate rustdoc JSON for `horizon-client-render`, `horizon-protocol`, and `horizon-command` alongside the existing crates.

## 2. `Game.options` reachability and dependent regeneration

- Extracted `Options` (`pub(super) struct Options` and its methods `player_name`, `read`, `parse`) from `crates/horizon-client/src/game.rs` into `crates/horizon-client/src/game/launch_options.rs`, with `#[cfg(feature = "yoake2-slice")] include!("../yoake_glue/launch_options.rs");`.
- Registered `super::launch_options::yoake_glue::call` in `crates/horizon-client/src/game/native_client_items.rs` (`yoake2_dispatch`: `[_; 7]` -> `[_; 8]`).
- Added `{ "name": "options", "ty": "horizon_client::game::launch_options::Options", "path": ["options"] }` to `game_items.game_paths` in `crates/yoake-bindgen/bindgen.json` (appended at `Game` path index `11`, `Accessor::Field(11)`), leaving all prior indices `0..=10` untouched.
- Added the `launch_options` target in `crates/yoake-bindgen/bindgen.json` and `pub mod launch_options;` in `crates/yoake-guest-sdk/src/stubs/mod.rs`.
- Generated:
  - `crates/horizon-client/src/yoake_glue/launch_options.rs`
  - `crates/yoake-guest-sdk/src/stubs/launch_options.rs`
  - `docs/yoake2/bindgen/functions-launch_options.md`
  - Updated `crates/horizon-client/src/yoake_glue/{entities,entity_query,game_items,mobs,mod_generator,motion}.rs`, `crates/horizon-client-render/src/yoake_glue/{settings,world}.rs`, `crates/horizon-items/src/yoake_glue/items.rs`, guest stubs, `docs/yoake2/bindgen/coverage.md`, `docs/yoake2/bindgen/summary.json`, and `docs/yoake2/bindgen/functions-*.md`.

Coverage & lock summary from `yoake-bindgen` (`--check` verified):
- `game_items`: 5 functions, 5 bound (including `Game.options#handle` at ID `2825`).
- `launch_options`: 3 functions, **1 bound** (`a 1 + b 0`), 2 excluded, 1 derived (`0 bound`); **32 / 42** scalar/Option/Vec fields bound as getters (`Options.distance#get` = ID `2834`, `Options.distance#set` = ID `2835` `ReadOnly`, etc.; 66 entries in `ids.lock.json` across `launch_options::*` and `Game.options#handle`: 34 `bound`, 32 `ReadOnly`).
- Side benefit of loading `horizon_protocol.json` and `horizon_command.json` into `"docs"`: `mobs` fields bound increased from 369 to 370 / 453 (`Profile`) and `render_settings` functions bound increased from 11 to 12 / 26 (`ClientInformation`).
- `crates/yoake-bindgen/ids.lock.json`: verified strictly append-only against the 1,825 base entries; total entries now **1,894** (`1,172` `bound`, `721` `ReadOnly`, `1` `unbound`; max ID `2,893`).

### Note on setters

Setters remain disabled (`writable: {}` in `bindgen.json`, `set: |t, f, v| match f { _ => false }` in `--tables`), following the same rule as `render_world`, `render_settings`, `entity_query`, `mod_generator`, and `--tables` (`FR-F1-W9b.md`, `FR-F1-W10.md`). All `#set` lock IDs on `Options` and existing read-only structs are reserved with status `"ReadOnly"` so adding opt-in setters later under `"writable"` (or full access-transformer setters in REM-11) preserves lock IDs cleanly.

## 3. Tests added and commands run

New tests added:
- `crates/horizon-core/src/geometry/mod.rs` (`#[cfg(all(test, feature = "yoake2-slice"))] mod yoake_table_tests`): includes generated `yoake_glue/tables/horizon_core/geometry.rs` and tests `FIELDS_crate__geometry__Vec3` and `REF_FIELDS_crate__geometry__Vec3` getters and read-only setter refusal.
- `crates/horizon-client/src/game/launch_options.rs` (`#[cfg(all(test, feature = "yoake2-slice"))] mod yoake_options_tests`): compiles a real `#![no_std]` WASM guest via `yoake-guest-sdk` and verifies `Game.options#handle` (`id 2825`), `Options.{distance,creative,seed,execution_role}#get`, `Options.distance#set` -> `ReadOnly`, W10 `horizon-client-render` (`Game.world#handle`, `World.day_time#get` -> `id 2747`, `World.day_time#set` -> `ReadOnly`), and generated `FIELDS_crate__game__launch_options__Options` across both mutable (`HOOK_SERVER_TICK`) and shared (`HOOK_CLIENT_TICK`) roots.

Commands run after final code edits (with the standard build env and `--offline -j 6`):

| Command | Result |
|---|---|
| `cargo test --offline -j 6 -p horizon-client --lib --features yoake2-slice launch_options` | **1 passed, 0 failed** (guest WASM test of `Game.options` + W10 `World` fields, `FIELDS_..._Options`) |
| `cargo test --offline -j 6 -p horizon-client --lib launch_api` | **6 passed, 0 failed** |
| `cargo test --offline -j 6 -p horizon-core --features yoake2-slice --lib yoake_table_tests` | **1 passed, 0 failed** (`FIELDS_crate__geometry__Vec3` / `REF_FIELDS_...`) |
| `cargo test --offline -j 6 -p horizon-client-render --features yoake2-slice --test yoake_game_ref` | **1 passed, 0 failed** |
| `cargo test --offline -j 6 -p yoake-guest-sdk --lib` | **5 passed, 0 failed** |
| `cargo test --offline -j 6 -p horizon-client-lib --features "yoake2-slice,yoake-hook-bench" --test yoake2_glue --test yoake2_engine_call --test yoake2_bridge --test yoake2_slice` | **yoake2_bridge: 12 passed; yoake2_engine_call: 8 passed; yoake2_glue: 5 passed; yoake2_slice: 7 passed, 1 ignored** (`bench_round_trip`) |
| `cargo test --offline -j 6 -p horizon-client --features "yoake2-slice,yoake-hook-bench" --test yoake2_examples --test yoake2_template` | **yoake2_examples: 2 passed; yoake2_template: 2 passed** |
| `cargo check --offline -j 6 -p horizon-client` | **ok** (exit 0) |
| `cargo check --offline -j 6 -p horizon-client --features yoake2-slice` | **ok** (exit 0) |
| `cargo run --offline --release -j 6 -p yoake-bindgen -- --config crates/yoake-bindgen/bindgen.json --check` | **ok** (exit 0) |
| `cargo test --offline -j 6 -p yoake-bindgen` (run *after* `yoake2-slice` built `target/debug/build/yoake-slice/*[wasm]`) | **lib: 5 passed; engine: 1 ignored; policy: 3 passed; mini_engine: 6 passed, 2 failed** (`generated_glue_compiles_and_every_binding_works`, `glue_module_forbids_unsafe_code` failed in `rustc` link step with `cannot find -lwindows.0.52.0` because `mini_engine.rs`'s `newest_rlib` picked the newer `yoake-slice` rlib built with `wasm` feature without `-L native=`). Earlier clean run before `yoake2-slice`: **16 passed, 0 failed, 1 ignored** (`engine::checked_in_glue_is_current_and_compiles_in_the_engine`). |
| `cargo test --offline -j 6 -p yoake-bindgen --lib --test policy --test engine` and `--test mini_engine -- --skip generated_glue_compiles_and_every_binding_works --skip glue_module_forbids_unsafe_code` | **lib: 5 passed; engine: 1 ignored; policy: 3 passed; mini_engine (non-rustc tests): 6 passed** |
| `python tools/check_public_tree.py` | **Public tree check passed.** (exit 0) |

Earlier pre-test-edit run (unchanged code since, not re-run):
- `cargo test --offline -j 6 -p horizon-client --features "yoake2-slice,yoake-hook-bench" --test yoake2_loader`: **42 passed, 1 pre-existing failure** (`a_loaded_mod_that_registers_a_bridged_hook_id_is_called_by_the_engine` expected `[(1, "arg0 is not &T or &mut T")]`, got `[]`).

## 4. What is left / not done

- Full `YOAKE_STAMP=1` overlay `cargo check`/`build` integrating the 223 new table files into the stamped overlay was not run in this task.
- Ignored slow tests were not run: `yoake-bindgen --test engine` (`checked_in_glue_is_current_and_compiles_in_the_engine`), `yoake2_slice::bench_round_trip`, and `HORIZON_TEST_RESOURCES`-gated tests.
- Arbitrary AT-style field setters across engine structs / `--tables` remain deferred to `REM-11`.
- `mini_engine.rs`'s `newest_rlib` behavior when sharing `CARGO_TARGET_DIR` with a `yoake2-slice` (`wasm`) build was observed and left untouched as pre-existing outside REM-04 scope.
