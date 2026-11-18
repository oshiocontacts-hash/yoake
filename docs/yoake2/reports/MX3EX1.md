# MX3EX1

## Example mods

Two `#![no_std]` full-access guest examples in `crates/yoake-guest-sdk/examples/` (`autoexamples = false` in `crates/yoake-guest-sdk/Cargo.toml`, because they compile to `wasm32-wasip2` core modules via `tests/support/build.rs` rather than host cargo examples):

1. **`crates/yoake-guest-sdk/examples/tick_and_item_use.rs`** — registers `hooks::SERVER_TICK` (`0x8000_0001`, `HookMode::Full`), `103` (`horizon_client::game::item_use::Game::start_using_item(&mut self) -> bool`, in `hook_bridge_game__item_use::BRIDGED`, `HookMode::Full`), and `43` (`horizon_client_lib::entities::Experience::give(&mut self, points: i32) -> Option<i32>`, in `hook_bridge_entities::OBSERVE_ONLY`, `HookMode::ObserveOnly`).
   - Note on `OBSERVE_ONLY` selection: `hook_bridge_game__item_use::OBSERVE_ONLY` is currently empty (`&[]`, because all 29 bridged `game::item_use` hooks return `()` or `bool` and run in `Full` mode), so the observe-only hook is taken from `hook_bridge_entities::OBSERVE_ONLY` (installed by the same `load_world_mods` / `install_bridges` path).
   - Reports the `HookMode` returned by `Registrar::hook` (`0 = Full`, `1 = ObserveOnly`, `2 = NotCallable`), counts server ticks with an `AtomicU32`, demonstrates `Full` mode pass-through in `start_using_item` (and documents replacing/cancelling results and the re-entry borrow rule), and demonstrates `ObserveOnly` mode in `Experience::give` (reading `entities_experience_total_get` before and after `original`, while documenting that the returned `i64` is ignored and the call cannot be cancelled).
2. **`crates/yoake-guest-sdk/examples/engine_call_game_path.rs`** — registers `hooks::SERVER_TICK`, opens a `Game` path handle to `ItemEntity` `4242` via `game_items_game_item_entity_handle`, reads `position` and `fire_ticks`, writes `fire_ticks = 55`, reads `fire_ticks` back, probes a missing entity id (`1`) to verify `Error::is_stale()` (`Error::Handle(1)`), and handles a removed entity without trapping.

### End-to-end tests (`crates/horizon-client/src/game/yoake2_examples_tests.rs`)

Mirrors `crates/horizon-client/src/game/yoake2_game_tests.rs` (two `#[ignore = "needs HORIZON_TEST_RESOURCES (the game resources)"]` tests using `server_game`, `Control`, `ApprovedList`, and `crate::yoake2_mods::load_world_mods`):

- `tick_and_item_use_example_loads_reports_modes_and_runs_through_bridges`: verifies clean `LoadReport` (`loaded`, no `skipped`/`problems`/`unbridged`/`legacy_notes`), bridge installation for `103` and `43`, `HookMode` reports (`0`, `0`, `1`), 3 server ticks (`[1, 2, 3]`), driving `Game::start_using_item()` (`Full`, holding `minecraft:bow` in main hand, `game.using == Some((0, 0))`), driving `game.experience.give(60)` (`ObserveOnly`, observing `total` `0 -> 60` while caller still receives `Some(5)`), and clean runtime stats (`!disabled`, `entries == 5`, 0 live activations/path handles).
- `engine_call_game_path_example_reads_writes_and_handles_stale`: verifies clean `LoadReport`, server tick reads/writes on item `4242` (`position` `(12.5, 64.0)`, `fire_ticks` `0 -> 55`, missing id `1` is `Stale`), and clean `Stale` handling (`Error::Handle(1)`) on the next tick after item `4242` is removed.

### Runnable end-to-end (`crates/horizon-client/tests/yoake2_examples.rs`, no game resources)

Both examples are compiled to wasm32 by the tests, packed as full-access `.yak` files, installed and approved through a
launcher `Control`, and loaded by the real loader (`load_world_mods` with this launch's approved list):
- `tick_and_item_use`: the report is clean (no refusal, nothing unbridged); `hook_register` returned `0, 0, 1`
  (server tick Full, 103 Full, 43 observe-only); three server ticks (driven as `Game::yoake2_server_tick` does,
  `dispatch_root_id(HOOK_SERVER_TICK, ..)`, on a stand-in root) report `1, 2, 3`; `Experience::give(30)` on a plain
  `Experience` goes through the generated bridge: the guest reads `total` before (`0`) and after `original`, the result
  equals the result without the guest and `give` ran exactly once.
- `engine_call_game_path`: loads cleanly; on a stand-in root no `Game` path opens, the guest gets an error back from
  `engine_call`, handles it, and is not disabled. The real path read/write is the ignored `Game` test above.

## ABI version negotiation

Decision: a **custom section** `yoake_abi` (payload exactly 4 bytes: `major: u16` LE, `minor: u16` LE), not an exported
function, so the loader (and later the launcher) reads it from the module bytes **without running guest code**.
- Engine: `yoake_slice::runtime::ABI_VERSION = 1.0`; `module_abi_version(wasm_or_wat)` parses the sections by hand
  (WAT through the `wat` crate already in Cargo.lock); `check_abi`.
- Guest SDK: `export_guest!` emits `#[used] #[unsafe(link_section = "yoake_abi")]` with `ABI_MAJOR`/`ABI_MINOR`; a test
  builds a real SDK guest and reads 1.0 back (rustc + wasm-ld keep the section). WAT guests write
  `(@custom "yoake_abi" "\01\00\00\00")`.
- Loader (`yoake2_mods::load_package`), before `runtime::load`: no section, a malformed or duplicated one, or another
  major → `Refusal::Abi` (never instantiated); a newer minor loads with a message ("engine calls it makes that this
  engine does not know are refused"); an older minor loads silently. `Loaded::abi` records the version.
- Rules for changing it (ENGINE_CALL_ABI.md, "ABI version"): a new import, binding or status is a minor change;
  changing or removing an import, a status meaning or the wire format is a major one.
- Tests: `yoake-slice/tests/abi.rs` (binary and WAT, 1.0 / 1.5 / 2.0 / 0.9, missing, 3-byte payload, two sections, other
  custom sections around it); `yoake2_loader::the_loader_checks_the_abi_version_before_any_guest_code_runs` (same,
  newer minor, major 2, major 0, missing, short: loaded or `Refusal::Abi` with nothing instantiated).

## Results (rustc 1.100.0, LENOVO)

| suite | result |
| --- | --- |
| yoake2_examples (new) | 2 passed |
| yoake2_loader (incl. ABI and server-tick tests) | 19 passed |
| yoake2_glue / engine_call / slice / bridge | 30 passed, 1 ignored |
| yoake-slice (`wasm`, incl. `abi`, `modes`, `readonly`) | 77 passed |
| yoake-guest-sdk (incl. the section-retention build) | 13 passed, 1 ignored |
| `horizon-client --lib` with `yoake2-slice` (incl. the ignored real-`Game` example tests) | 1195 passed, 44 ignored |
| hook crates, `horizon-client(-lib) --lib` feature off, horizon-mods, yoake-wasm, codemod checks | as in MX3MERGE2, green |

Work split: examples and the `Game` tests by agy (Gemini 4 Hydrogen), the ABI section, parser and SDK macro by Kimi
(K3.1), the loader check, the runnable end-to-end tests and review by Claude.
