# FR-REM02: F1 runtime gate (REM-02 + REM-03), 2026-11-22..24, GALLERIA

Branch `yoake-free/rem02-f1gate` (from `yoake-free/merge` c01031597). Work: agy (REM-02a/c), coordinator (REM-03), Haiku (hit-count feature, bench stabilisation).

## REM-02
- **a, build-level verify:** `apply --verify --overlay --verify-arg build` passed for all 8 stamped crates in one round, 0 stamps withdrawn (16529d718).
- **b, release stamped build:** `tools/yoake-stamped-cargo.sh build --release -p horizon-client --features yoake2-slice` builds and a zero-mod game starts. Sizes: unstamped 146 MB; stamped 274 MB before exclusions, 263 MB after.
- **c, CI:** `.github/workflows/yoake-windows.yml` builds the stamped release (80b683bc1).
- **Bug found:** the stamp overlay cache (`target-stamped/yoake-stamp`) can be stale after ledger changes. Stale entries made `horizon_items` / `horizon_command` miss `__yoake_install_bridges` / `__yoake_bridge_mode`. Deleting the cache fixed it. The cache key (`overlay.rs` state file) should include every ledger shard's contents. Open.

## REM-03, <1% gate
- **Hit counts:** opt-in features `yoake-hook-rt/hit-count` and `horizon-client/yoake-hook-hit-count`. With `YOAKE_HOOK_HITS=<file>` the game dumps per-slot call counts at exit (c8a3c73a9). Two flights recorded: `reports/hit_counts-rem03b.txt` (second flight).
- **Exclusions:** manual exclusions in the ledger (status `tombstone` + `excluded` entries with `manual: true`, reason `hot: N calls ...`):
  - Round 1: 222 unbridged fns, 98% of calls in flight 1 (051a59dcf).
  - Round 2: 729 unbridged fns, 99.5% of unbridged calls in flight 2 (10eda416e).
  - Hooks with a committed bridge (481 ids, e.g. `Mob::species`, `Mob::bounds`) stay live, because a mod can really override them. They make 41% of the remaining calls.
  - About 9,950 live fn hooks remain.
- **Benchmark tool** (`tools/yoake-hook-bench-gate.py --mode game`):
  - Interleaved A/B runs.
  - The world is pregenerated once and copied into every run.
  - Warm-up runs are discarded.
  - Before each run the tool waits until no build or game is running.
  - The gate is the paired median of per-run ratios, with a bootstrap 95% interval.
  - Earlier runs, without the pregenerated world, drifted 6→12 ms within a session and had 40 ms outliers. They are not usable.

## Results
| Run | Server tick, paired median (95% CI) | Client frame p50, paired median (95% CI) |
|---|---|---|
| Before exclusions (5+5, old tool) | +8.86% | +6.54% |
| A/A, same exe (9 runs, new tool) | +3.86% [-6.8, +5.6] | +0.43% [-0.6, +7.4] |
| A/B after round 2 (21 runs, 40 s flights, new tool) | **+1.56%** [-3.1, +16.5] | **+0.56%** [-0.8, +4.6] |

- **Client frame time:** under 1%.
- **Server tick time:** +1.6%. This is within this PC's noise: the A/A interval alone is about ±6%, so a 1% difference cannot be resolved for tick time here.
- **Decision (coordinator):** keep the bridged mob hooks live (freedom first), accept the result as meeting F1 for frames, and record tick as "within noise, about 1.5%".

## Left
- Fix the stale overlay cache key.
- Measure tick time on a quieter or dedicated machine, or with a server-only tick benchmark.
- Excluding the hottest bridged getters would remove the last of the overhead, but mods could then no longer override those functions. That is the owner's call.
