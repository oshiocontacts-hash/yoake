# FR-MERGE5: F4M-4c, F1 W10, F5 guide, media browser and F3-8b into `yoake-free/merge`

Base `fed161c18` (MERGE4). Merged in this order, no push:

| Step | Branch / commit | Conflicts |
|---|---|---|
| 1 | `f4m-4c` 6b106b7e5 (contains f4m-robust 7ce41f690), then fast-forward 040ce9eb0 on top | none |
| 2 | `f1-w10` 728f56372 | glue/stubs/reports (took ours, regenerated), `hook-ledger/meta.json`, `ids.lock.json` (see below) |
| 3 | `f5-guide` e9454b4dd, then 032a64d65 on top | `game.rs` mod list (kept `mod native_levels;`) |
| 4 | `media-browser` 7a2dd210a | none |
| 5 | `f3-8b` 59fa115ec | `native_client_items.rs` (dispatch list now 7 modules: glue, entities, mobs, motion, render world, render settings, mod_generator), `bindgen.json` (kept both modules), `ids.lock.json` |

`f5-guide` is not docs-only: its history holds the earlier F3-8 (level generator services) next to the guide chapters; it merged cleanly with f3-8b.

## F4M-4c stage: goldens A/B/C

`f4m_parity -- --ignored` on the tree right after f4m-4c: at 6b106b7e5 and again at 040ce9eb0 all three pass (A `golden_a_saves`, B `golden_b_network`, C `golden_c_render_types_and_identity`).
`horizon-mobmodel` (the `file: &str` API change) passes; the engine call sites were already fixed on f4m-4c. Final tree: all three pass again.

## Id collisions and how they were resolved

1. **Custom-mob hook reservation (again).** F1 W10/W12 ledger ids run 1..=10754 and used 8475..=9474, the range I had reserved in MERGE4. Moved again: `reserved_ids::HOOK_FIRST..=HOOK_LAST` = 10755..=11754,
   `hook-ledger/meta.json` `next_id` 11755. Every F1 wave moves `next_id`, so this can recur with the next wave; a lasting fix would reserve a range far above the ledger (and the test would then check `next_id` stays below it).
2. **Binding ids.** F1 W10 branched before F4M-0's reserved binding range, so its 261 new bindings had ids 1515..1775, inside the reserved `custom_mobs` range 1515..=2514. The merged `ids.lock.json` keeps the F4M lock
   as it was and appends W10's bindings renumbered 2533..2793 (order kept), then F3-8b's 25 (`mod_generator`) as 2794..2818; the regeneration added the rest. Final lock: 1825 ids, unique, max 2824; the sentinel at 2514 is untouched.
   The glue, stubs and reports were regenerated from the merged lock (`regen.sh`, now with 7 rustdoc crates incl. `horizon-client-render`); `regen.sh --check` passes on the final tree.
3. **Catalog ids.** `horizon-mod-api-gen` (46 tests incl. `abi_1_0_is_additive`) passes with media services 240-254 (client 1.6.0), F3-8 server services 227-235 and the hook catalog; `YOAKE_UPDATE_WIT=1` produced no
   content change in the WIT, coverage, adapters or `catalog.rs` (only line endings, discarded).

## A real defect found: two enums named `Hand`

F4M-4c added `mobs::custom::ai::interact::Hand`. The WIT generator resolves Rust types by their name, so with two `Hand` enums `native-player-render`'s `swing-hand` and `using-item` became opaque `u64`
handles: `abi_1_0_is_additive` failed ("breaking 1.0 WIT changes: native-player-render"). Fixed by renaming the new enum to `ClickHand` (`interact.rs`, `tests_4c.rs`, `game/custom_mob_interact.rs`). Worth a rule for later waves:
type names that appear in the catalog must stay unique across the engine crates.

## F3-8b's hook bridge

`hook_bridge_mod_generator.rs` was not generated. The committed bridges exist only for `entities`, `game::item_use`, `mobs` and `motion` (hand-listed); every other module gets its bridge from the overlay at
stamp time (`YOAKE_STAMP=1`), so `mod_generator` is covered there and nothing is committed for it. No stamped source appeared in the tree (the diff adds only `yoake_hook(test_id = ..)` lines of the macro's own tests and
skip annotations). Plain builds and tests created no overlay or stamp folder under `target/`.

## Tests on the final tree (CARGO_INCREMENTAL=0, CARGO_PROFILE_DEV_DEBUG=0, one target dir)

| Suite | Result |
|---|---|
| `f4m_parity` A / B / C | pass / pass / pass |
| `horizon-client-lib --lib` | 1830 passed (48 ignored) |
| `horizon-client --lib` | 1243 passed (52 ignored) |
| `level_registry_tests -- --ignored` | 11 passed (parallel) |
| `horizon-command` | 36 passed |
| `horizon-world` | 385 + integration tests |
| `horizon-mods --features yak` | 94 + 43 |
| `horizon-mod-api-gen` | 46 |
| `yoake-wasm` | all binaries (68, 6, 2, 11, 5, 1, 9, 7, 16, 2, 3, 5) |
| `yoake-slice` | 32 + 1 |
| `horizon-mobmodel` | all (60, 13, 18, ..., goldens) |
| hook crates (ledger, macro, rt, codemod) | all pass |
| `yoake-bindgen` | 3 + 8 + 3 |
| `cargo check -p horizon-client --features yoake2-slice` | ok |
| `regen.sh --check` | ok |

End to end (real games, `--offscreen --no-focus --no-pause`, PYTHONUTF8=1, dev binaries, no other horizon-client running):
* `levels_e2e.py`: setup, travel, stock (protocol-level client), created, lifecycle: all pass.
* `mob_sync_e2e.py`: quiet, fox, store (1 snapshot, 4 deltas, 0 dropped, 0 incompatible), late (first line a snapshot, known=1): all pass.
* `levelgen_wasm_e2e.py` (the `level-generator` sample): level, travel (terraces generated by the guest), back: all pass.

## Disk

Free space between 23 GB at the start and 13 GB at the end; never below 12 GB, so no pause was needed. Freed at the end: `target/doc`, `target/release`, the e2e outputs and `target/tmp` (about 0.2 GB measured).
