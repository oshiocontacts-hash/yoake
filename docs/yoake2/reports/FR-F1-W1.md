# FR-F1 wave 1: W3, W2, W5, W1 (codemod and macro hardening for stamping at scale)

Branch `yoake-free/f1-w1` (cut from `vps/yoake-free/merge` @d84aea8e3), worktree `C:\Users\USER\NestgateHorizon\horizon-f1`. Not pushed.
Inputs: `docs/yoake2/reports/FR-F1-SPIKE.md` (the spike) and the decision to stamp through a build-time overlay (W7). The stamped tree of
the spike (8941a4ae6) was **not** brought over; only fixes. Builds ran with `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0`, one
target dir (deleted at the end), `df` stayed between 28 and 36 GB free (rule: at least 30 GB before the full verify: it was 35 GB at the start of the run, 28 GB at the end).

| Item | Commit | State |
|---|---|---|
| W3 codemod fixes with tests | `W3: codemod fixes ...` (cherry-pick of 40c01c8a2 plus `#[path]` fix and inventory example) | done |
| W2 macro lifetime / dyn fixes | `W2: macro fixes ...` | done; the 40 fns of the spike compile when stamped (shown by W1 verify) |
| W5 glue hygiene, per-crate stamp config, size rule | `W5: stamp config ...` | done |
| W1 compile-and-exclude loop, `apply --verify` | two `W1:` commits | done; full three-crate verify: **clean in round 1, 0 stamps withdrawn** |

Tests: `cargo test -p yoake-hook-ledger -p yoake-hook-macro -p yoake-hook-rt -p yoake-hook-codemod`: **134 passed, 0 failed** (codemod bin 28,
bridge 3, callsite 15, codemod_tests 20, shallow 4; rt mechanics 14; macro 11; ledger and rt others). CI `check` over the four `modules.txt` lines: clean.

## W3: codemod fixes (all found by the spike)

Files: `crates/yoake-hook-codemod/src/{key.rs,eligible.rs,modules.rs}`, `examples/f1_inventory.rs`.

* `key.rs`: the helper name of a converted trait method now carries a hash of the trait's generic arguments when it has any
  (`__yoake_trait_partial_eq_g1a2b3c4d_eq`). `impl PartialEq<A> for T` and `impl PartialEq<B> for T` no longer collide
  (about 290 of the 513 spike errors in horizon-world). Plain traits keep their old names, so already converted code is unchanged.
* `eligible.rs`, `check_trait_method_movable`: refuses the conversion when the body calls a trait method on **any** receiver or as
  a path call (`e.fmt(f)`, `u16::from_inject_value(v)`) and the trait is not in scope outside its own impl, and when the signature
  uses `Self::Item`-style associated types (supertrait items). `Self::Variant` in bodies is still allowed (tested: it would otherwise block most enum impls).
* `modules.rs`: a `#[path]` module declared in a non-`mod.rs` file is resolved relative to that file's directory, as rustc does
  (the codemod stopped with "resolved to missing file" on `mod_content`).
* Tests: 4 trait-move unit tests, 1 helper-name test, 1 `#[path]` fixture test.

## W2: macro and codemod fixes for the 40 un-stampable fns

Files: `crates/yoake-hook-macro/src/lib.rs`, `crates/yoake-hook-codemod/src/eligible.rs`, tests in `crates/yoake-hook-rt/tests/mechanics.rs`.

1. **Named impl lifetimes shared by an argument and the result** (`impl<'r> Reg<'r> { fn new(src: &'r X) -> io::Result<Self> }`, E0581/E0582, 204 of the
   spike errors, about 34 fns). Cause: `Self` was replaced by a type with fresh lifetimes while the argument kept `'r`, so the result lifetime was constrained by no input.
   Fix: the impl's named lifetimes are mapped to the same fresh names in arguments and result (`RenameNamedLifetimes`), and for a fn
   without receiver the original is called as `<Reg<'fn_0>>::__yoake_orig_new` at the call fn's lifetimes instead of the impl's fixed `'r`.
2. **`-> &'static dyn Trait`** (E0310/E0716 and "ambiguous `+`", `worldgen::blending::shift_noise`): a result that ends in a trait object is
   parenthesized inside `dyn FnMut(..) -> (&'static dyn Shape) + 'orig`.
3. **Attributes on parameters** (`#[cfg(feature = "capture")] fences: &Fences` in `net.rs`, E0061/E0425): the generated shim dropped the cfg.
   The codemod now refuses such fns (`attribute on parameter`), with a test.
* Tests (all in `mechanics.rs`): a constructor and a two-argument fn of `impl<'r>` that replaces an argument through the hook; a `-> &'static dyn Shape` fn and a `&dyn Shape` argument with a hook that changes the argument.
* The spike also listed E0425/E0061 (3 + 2 errors): that is item 3.

## W5: glue hygiene, per-crate stamp config, size rule as a setting

Files: `crates/yoake-hook-codemod/src/{main.rs,run.rs,modules.rs}`, `crates/yoake-hook-macro/src/lib.rs`, `hook-ledger/stamp-config.txt`, `hook-ledger/stamp-allow.txt`, tests in `tests/codemod_tests.rs`.

* Macro: `#[allow(non_snake_case)]` on the generated dispatch, call and original fns (the 772 spike warnings per crate came from names such as `__yoake_dispatch_yoake_cs_179`).
* Codemod flags: `--module '*'` (every top-level `mod` of the crate root, not `cfg(test)`, not `__yoake*`), `--exclude-module NAME`, `--min-stmts N`,
  `--stamp-config FILE`.
* **Size rule as a setting:** a fn with at most N statements gets no hook (reason `small body`), unless its key is in `hook-ledger/stamp-allow.txt`
  or already live in the ledger (a live hook is never retired by the rule: tested).
* `hook-ledger/stamp-config.txt`: one line per crate `<root> <crate> <module|*> [exclude=a,b] [min_stmts=N]`. Today:
  `horizon-client-lib`, `horizon-client`, `horizon-world`, all `min_stmts=3`, test-support modules excluded. The CI `check` job still reads `modules.txt` only, so these lines do not require stamps in the repository (W7 reads them).
* Tests: star plus exclude, size rule with allow list and live key, config file.

## W1: `codemod apply --verify` (compile-and-exclude loop)

Files: `crates/yoake-hook-codemod/src/verify.rs` (new), `main.rs`, `run.rs`.

`apply --stamp-config F --crate-root R --crate-name N --ledger L --verify [--verify-arg A]... [--verify-dir D] [--verify-rounds 8] [--verify-max-withdraw 300]`.
One round: `execute` (stamp), then `cargo <verify-args> --message-format=json` (default `check --offline`; pass `-p`, `--features` as `--verify-arg`).
Each compiler error is mapped to the hook attribute it belongs to: the attribute just above the error line (counting fn headers so an error in the next, unstamped fn is
not blamed on it), or the one just below it for the wrapper of a converted trait method; macro-expansion call sites count too. The keys go to
`hook-ledger/stamp-exclude.txt` with `compile error E0xxx: message`; the next `execute` retires a live id (tombstone, never reused) and records a manual
exclusion in the ledger, so a later plain `apply` keeps the fn out and the result is deterministic. The request file is deleted after a clean build.
The loop stops when the build is clean, when a round attributes no new key (error outside the scanned modules, link or disk failure: prints the unattributed errors and the cargo tail), at the round limit, or when one round
would withdraw more than 300 stamps (systematic cause; nothing is withdrawn). This guard exists because my first full run, on a tree that lacked the horizon-world
hook dependencies, withdrew all 656 stamps for `E0433 cannot find yoake_hook_macro`; that run was discarded.

Tests (unit, injected build runner): attribute scan, error-to-key attribution (own item, helper below a trait wrapper, unattributed fn), cargo JSON parsing, exclude-file round trip, the full loop on a fixture crate
(rejected stamp removed from the source, id tombstoned, manual exclusion in the ledger, request file removed, plain re-apply is a no-op), and the withdraw cap.

### Full three-crate verify (run from this branch's codemod, `cargo check`, dev profile)

Order world, client-lib, client (each depends on the previous). Config = `hook-ledger/stamp-config.txt` (all modules, `min_stmts=3`).

| Crate | verify time | rounds | stamps withdrawn | hooks now (incl. earlier ones) | new ids | excluded (top reasons) |
|---|---|---|---|---|---|---|
| horizon-world | 47 s | 1 | 0 | 656 | 656 | small body 1,434; generic impl 1,285; no crate-level path 895; generic type params 554 |
| horizon-client-lib | 3 m 22 s | 1 | 0 | 2,406 | 2,026 | small body 3,504; no crate-level path 1,399; impl Trait arg 193; destructuring 175 |
| horizon-client (`--features yoake2-slice`) | 9 m 07 s | 1 | 0 | 2,849 | 2,826 | small body 4,010; no crate-level path 1,824; borrowed return 186; impl Trait arg 138 |
| **total** | 13 m 16 s | | **0** | **5,911** | 5,508 | |

* Source change of this stamping: 800 files, +9,127 / -649 lines; ledger `next_id` 6,218, three shards 3.7 MB (the spike without the size rule: 14,743 hooks, +17,993 lines, 4.1 MB).
* `check` with the same config on the stamped tree: **clean** for all three crates (idempotent). The four `modules.txt` lines: clean.
* **No errors at all** in round 1: W3 plus W2 remove every compile error the spike hit (513 in world alone), including the 40 fns that had to be un-stamped by script. The
  loop itself was therefore exercised by the unit tests and by the discarded first run, not by a real rejected stamp in this tree.
* The stamped tree was reverted after the run (not committed, per the brief). It can be regenerated with the three commands above (about 15 min including cargo check of dependencies).

## Findings and what is not covered

* `cargo check` does not run monomorphization or codegen: errors that only appear there (rustc OOM seen once in the spike on `horizon-client`, link errors) are not found by `--verify`. Use `--verify-arg build` for a release gate (disk).
* The attribution heuristic is textual (nearest hook attribute, fn-header counting). A nested `fn` inside a body or an error in an item between two attributes is reported as unattributed and stops the loop with a message; it never withdraws the wrong stamp silently except in the wrapper-above-helper case, which is tested.
* horizon-world stamping needs the hook infra (deps, `build.rs`, `__yoake_hook_keys`): committed here (it was missing from the first full run). Other crates need the same three edits (W10).
* "Self type has no crate-level path" still removes 1.4k + 1.8k + 0.9k fns (W4).
* `min_stmts=3` is a setting, not measured: W6 should replace it by a measured hot list. Counting build and pinned benchmark are still missing.
* No full build or smoke run of the verified tree in this wave (disk rule); the spike's smoke and size numbers were for the 14.7 k-hook tree.

## Next

W4 (no crate-level path), W6 (cost gate), W7 (overlay) can start; W7 should run `apply --stamp-config ... --verify` in CI once to produce the committed ledger and then only `check` plus the overlay in builds.
