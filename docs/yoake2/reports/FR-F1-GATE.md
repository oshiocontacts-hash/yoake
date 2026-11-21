# FR-F1 gate: release gate, W6 cost gate, `&Game` bridge test

Branch `yoake-free/f1-gate` (from `vps/yoake-free/merge` @40bbb34be). Machine: 16 jobs, Windows, GNU toolchain 1.100.0 (`C:\Users\USER\NestgateHorizon\toolchain\mingw64\bin` on PATH or `ring` fails looking for gcc), fast profile, debug info off, `-p horizon-client --features yoake2-slice`.

**Status of the 1% gate: NOT PASSED YET.** After the first exclusion round (363 hot functions) the stamped build costs **+2.73 % frame p50, 95 % CI [-1.43, +6.88]** (10 interleaved pairs, scene `village`). Before any exclusion it was +5.9 % [+1.5, +10.3]. A second round (lower threshold) and a 20-pair rerun are next; this file is updated after them.

## 1. Release gate

| | unstamped | stamped |
|---|---|---|
| clean build, fast profile | 1,091 s (18m09s) | 2,118 s (35m16s): +94 % (the release codemod builds in 95 s on top) |
| rebuild after the ledger changed (363 more exclusions) | | 2,300 s (38m17s) |
| binary | 211,794,979 B | 353,822,844 B (+67 %); 354,347,926 B after the exclusions |
| incremental, leaf file of horizon-client-lib | 43 s | 210 s |
| incremental, `horizon-core` lib.rs | 61 s | 220 s |
| incremental, leaf file again | 52 s | 220 s |

The stamped incremental builds are dominated by the stamped crates being recompiled from `horizon-core` up (see section 3); an unstamped leaf edit rebuilds only the library.

Release/LTO was not built (no time; the fast profile is the one developers use).

No-mod stamped runs: every one of the 20 benchmark runs per variant exited 0; the `generated_count` / `meshed_count` of the `village` flight vary run to run in BOTH variants (worker timing: unstamped {625, 626, 637}, stamped {626, 637}), so these are not an exact equality check. A deterministic output comparison of a stamped run against an unstamped one was not done (my first smoke command used flags that never reach a world; both variants then failed the same way, `frame-limited run timed out`, which says nothing).

## 2. W6 cost gate

What did not exist and now does:
* `yoake-hook-rt` feature `count`: `Slot` gets a counter bumped in `load()`; `counts()` / `reset_counts()`. Test `crates/yoake-hook-rt/tests/count.rs`.
* `horizon-client(-lib)` feature `hook-count`: `--benchmark` resets the counters before each flight and writes `benchmarks/hook-counts.<scene>.txt` (`# frames N`, then `<slot id> <entries>`).
* `tools/hook_counts.py --counts DIR --per-frame T [--write]`: joins the counts with the ledger, lists hot functions, writes `hook-ledger/hot-exclusions.txt`. Hand-written hooks (ids written in the sources) are listed but not excluded: they are in the unstamped build too.
* The codemod reads `hot-exclusions.txt` like `stamp-exclude.txt` (only keys of the crate being stamped; `sync-ledger` then tombstones the ids and records manual exclusions).
* `tools/hook_cost_gate.py RUNS_DIR SCENE [--limit 1.0] [--metric p50|cpu|avg|p99]`: stamped against unstamped, delta-method 95 % CI, exit 1 above the limit. This is the CI step.

Pinned run: the game's own `--benchmark <scene> --offscreen --no-focus` (world seed 1234, fixed flight, warm-up settle 2 s, flight 12 s), prepared resources from `horizon\.local\horizon-client\26.3`, one game folder per run, runs interleaved A B B A ... with the stamped warm-up run discarded.

Hot list (one `standard` run = 4 scenes, 29,800 frames, count build): 2,360 hooked functions entered, 74,606 slot entries per frame; 394 functions above 2 entries/frame carry 99.2 % of all entries; 31 of them are hand-written hooks (kept), 363 stamped ones excluded. Top: `FxHasher::write_i32` 8,104/frame, `GenBlockStates::heightmap_state` 5,940, `FxHasher::finish` 3,988, `beardifier::beard` 3,492, `PerlinNoise::gradients_at` 2,936, `Mob::species` (hand-written) 2,723, `PackedStorage::write` 2,721, `occlusion::Graph::index` 2,557, `mesher::hidden` 2,513. Entries per frame left after the 2/frame cut: about 566 (0.8 % of the unexcluded rate); with a 0.5 cut about 86.

Frame time, scene `village` (ms; mean of 10 runs per variant):

| | unstamped | stamped | diff | 95 % CI |
|---|---|---|---|---|
| before exclusions: frame p50 | 1.260 | 1.335 | +5.90 % | [+1.47, +10.32] |
| before: cpu p50 | 1.201 | 1.271 | +5.81 % | [+1.29, +10.34] |
| before: chunk load (generate + mesh) | 4,770 | 5,246 | +10.0 % | [-8.4, +28.4] |
| **after 363 exclusions: frame p50** | 1.280 | 1.315 | **+2.73 %** | [-1.43, +6.88] |
| after: cpu p50 | 1.221 | 1.252 | +2.54 % | [-1.67, +6.75] |
| after: mean frame (1/avg fps) | 1.463 | 1.491 | +1.95 % | [-3.24, +7.15] |
| after: frame p99 | 3.919 | 3.953 | +0.87 % | [-7.23, +8.96] |
| after: chunk load (world generation + meshing of the flight start) | 5,118 | 5,236 | +2.31 % | [-16.05, +20.67] |
| after: world open (`startup.world_ms`) | 19,323 | 18,723 | -3.10 % | [-8.90, +2.69] |

Reading: the exclusion round halved the point estimate, but the interval is about ±4 points wide, so with 10 pairs a 1 % limit cannot be decided either way; the point estimate is above it. Part of what remains is not the empty slot check: the stamped binary is 67 % larger (code layout, instruction cache). The world-generation numbers have intervals of about ±18 points and show no effect at that resolution.

## 3. Defects found in the stamping wrapper (and what they were)

1. **Lost bridge `include!` (first incremental stamped build after a clean one failed in horizon-world: `loot::__yoake_bridge_install` not found).** Reproduced (a no-change stamped build after the rebuild failed the same way). Cause: `horizon-client` has `include_str!("../../../horizon-world/src/loot.rs")` in a test; the overlay closure of horizon-client copies that file into the SHARED overlay root, over horizon-world's stamped `loot.rs`. horizon-world's state file still says fresh, so its next build takes the fast path and compiles the plain `loot.rs` against a `lib.rs` that still calls the bridge chain. Fixes (both in `overlay.rs`): (a) a foreign `.rs` file reached only through `include_str!` is copied only when absent; (b) the fast path now also compares a snapshot of the crate's overlay files (size and time) written at the end of its own overlay run, so any file another crate's overlay replaced forces a regeneration.
2. **Same class, second form** (`yoake_glue_tables` not found in `motion` in horizon-client-lib after a no-change build): horizon-client-lib and horizon-client both own files under `crates/horizon-client/src`; their overlays share one root, so each can replace the other's stamped copy. The snapshot check in (b) covers it; a per-crate overlay root would remove the sharing, but verify's path translation and the e2e tests assume one root (`yoake-stamp/root`), so I did not change that.
3. **`horizon-core` is recompiled by every stamped build, even with nothing changed.** (Reason: see the final section after the rerun; not yet known when this paragraph was written.)

## 4. `&Game` wasm guest test (W9c gap)

Written, not yet run: `crates/yoake-guest-sdk/examples/engine_call_game_ref.rs` (hooks `Game::sleep_height(&self, head)`, ledger id 5380, reads `Game.world.day_time` and `Game.settings.fov` through the shared root, then calls original) and the ignored test `game_ref_example_reads_world_and_settings_through_a_stamped_shared_bridge` in `crates/horizon-client/src/game/yoake2_examples_tests.rs`. Run:

```
HORIZON_TEST_RESOURCES=C:\Users\USER\NestgateHorizon\horizon\.local\horizon-client\26.3 \
tools/yoake-stamped-cargo.sh test --offline --profile fast -p horizon-client --lib --features yoake2-slice yoake2_examples -- --ignored
```

## 5. Not done yet

Second exclusion round, 20-pair rerun (village and a generation-heavy scene), `apply --verify --overlay --verify-arg build`, the `&Game` guest test run, final deletion of targets and `gate-run`.
