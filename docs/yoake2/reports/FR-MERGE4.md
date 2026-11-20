# FR-MERGE4: F3-5, F3-6, F3-7 and the F1 hook-tooling waves into `yoake-free/merge`

Base `43b66a190` (MERGE3 follow-ups). Order and commits taken (all on vps at merge time), no push:

| Step | Branch | Commit | Conflict |
|---|---|---|---|
| 1 | `f3-5` | d7add1675 | none |
| 2 | `f3-6` | 4f6ecd2fe | `game.rs` mod list: both `mod level_manager;` and the `#[cfg(test)]` before `level_registry_tests` kept |
| 3 | `f3-7` | 376c9413d | none (it carries the same `cfg(test)` placement fix as my MERGE3 fix; the tree has one copy: `mod level_facts; mod level_manager; #[cfg(test)] mod level_registry_tests;`) |
| 4 | `f1-w1` | 080b2dd58 | none |
| 5 | `f1-w2` | 192aac27a | `hook-ledger/meta.json` (`next_id` 1710 vs 8475): took 8475 (W2's ledger really uses ids up to 8474), see below |
| 6 | `f1-w3` | 16e8000b3 | none |
| 7 | `f1-w9b` | 99c311ff9 | none |

## Id collision: the F4M-0 reserved hook id range

FR-F1 W2's committed ledger (`hook-ledger/horizon_client*.json`, `horizon_world.json`) uses ledger ids 1..=8474, including 710..=1709, the range F4M-0 reserved for
custom mob hooks (`custom_mob::reserved_ids::HOOK_FIRST..=HOOK_LAST`, test `the_reserved_id_ranges_are_in_the_locks` forbids ledger entries in it). No custom mob hook has an id yet.
Fix (commit "Move the custom-mob hook id reservation past the F1 ledger"): the reservation is now 8475..=9474 and `meta.json` `next_id` 9475 (ledger tool hands out from there).
The F1 ids were not renumbered (the committed field tables and stamp config key on them). F4M code that assigns custom mob hook ids by hand must use 8475..=9474.
Checked: 8474 ledger ids, no duplicates, `horizon-mods` test passes.

## No stamped source in the tree

Counted `#[yoake_hook` attributes in the diff against `e49fe6031`: 16 added lines, 3 are `#[yoake_hook(skip = "benchmark helper")]`, the rest are doc comments and test strings.
No `#[yoake_hook_macro::yoake_hook(id = ..)]` was added to any source file; the stamped build is overlay-only (`RUSTC_WORKSPACE_WRAPPER` + `YOAKE_STAMP=1`; the wrapper
passes everything through otherwise, `overlay.rs`). The repo configures no wrapper (`.cargo` has none). Plain unstamped builds and tests in this report (`cargo check/test/build` with neither variable)
left no overlay or stamp folder under `target/`.

## Generated files and ids

`regen.sh --check` found 9 files out of date after the merges (the glue of `entities`, `entity_query`, `game_items`, `mobs`, `motion`, `items`, and the reports); `regen.sh` rewrote them;
content changes were only in `yoake_glue/mobs.rs` (receivers `&mut X` became `&X` where the F1 waves made the method read-only; e.g. calls 2515, 2517-2527) and the reports.
Commit "Regenerate the engine glue after F3-5..7 and F1". Binding ids unchanged and unique (1533, max 2532). No catalog JSON or WIT changed; versions as before (client 1.5.0, worker 1.6.0).
I did not rerun `regen.sh --check` after the regeneration (disk); `yoake-bindgen` tests (3 + 8 + 3) pass.

## Tests (CARGO_INCREMENTAL=0, CARGO_PROFILE_DEV_DEBUG=0, one target dir)

On the final tree (after the glue regeneration):
* `f4m_parity` goldens A `golden_a_saves`, B `golden_b_network`, C `golden_c_render_types_and_identity`: all pass.
* `horizon-client-lib --lib` 1797 passed (48 ignored); `horizon-client --lib` 1228 passed (50 ignored); `cargo check -p horizon-client --features yoake2-slice` ok; `yoake-bindgen` all pass.

Before the glue regeneration (same sources otherwise): `level_registry_tests -- --ignored` **10 passed in parallel** (no `--test-threads=1` needed now); `horizon-world` all pass; `horizon-mods --features yak` 92 + 42;
`horizon-mod-api-gen` all pass (46); `yoake-wasm` all binaries pass (67, 6, 2, 11, 5, 1, 9, 7, 16, 2, 3, 5); `yoake-slice` 32 + 1; the hook crates `yoake-hook-ledger`, `-macro`, `-rt`, `-codemod` all pass
(33, 4, 15, 21, 4, 33, 11, 1, 5, 15 over their binaries). A first `yoake-wasm` run failed in `package_services` while the disk was down to 5 GB; the rerun passed.

End to end (real games, `--offscreen --no-focus --no-pause`, nothing else running, run before the glue regeneration; the regeneration changed receiver mutability only):
* `tools/levels_e2e.py` (PYTHONUTF8=1, dev binaries incl. `examples/level_probe`): setup, travel, stock (the protocol-level client), created, lifecycle: all pass.
* `tools/mob_sync_e2e.py`: quiet, fox, store (1 snapshot, 14 deltas, 0 dropped, 0 incompatible), late (first line a snapshot, known=1): all pass. The late client had no fox in its 40 block entity list this time
  (it is placed away from the fox; the check does not require it; last time it had one).
Not rerun after the regeneration: the two e2e scripts and `regen.sh --check` (disk was at 6 GB; the coordinator asked to stop game runs below 12 GB).

## Disk

Freed in `horizon-free/target`: `release`, `doc`, `tmp`, `sdk`, `wasm-guests`, the e2e output folders, `debug/incremental` (about 0.9 GB in total as measured; the shared disk moves by GBs between readings).
Free space during the run: 26 GB at the start, 5.3 GB at the lowest (while `yoake-wasm` rebuilt its guests beside other agents' builds), 23 GB at the end.
