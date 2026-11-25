# FR-REM11b: call-site (INVOKE) hooks across the stamped crates, compile/exclude loop and real patches

Branch `yoake-free/rem11b-callsites` (from `yoake-free/merge`). Part 1 of FR-REM11 built the `callsite-patch` route; a real run on
`horizon-command` showed 514 compile errors in four classes and no loop to remove them. This report covers the generator fixes, the loop,
the per-crate results and what is left.

## Generator fixes (each has a unit or fixture test)

| Class | Fix | Test |
| --- | --- | --- |
| E0521: `&str` passed to a `'static` parameter (457 on horizon-command) | the shim takes `'static` from the callee's declared parameter type (`apply_static_from_param_ty`) | `static_str_parameter_preserves_static_lifetime_in_shim` |
| E0061 / E0404: free-function path calls rewritten like methods (26 + 26) | path calls are rewritten as path calls, `Type::method(recv, ..)` for UFCS | `free_function_path_call_rewrites_as_path_call_not_method` |
| E0502 borrow errors (5) | left to the loop: the site is excluded with the compiler's message | verify tests |
| Callee returns `&'static T` / `Option<&'static T>` / `Foo<'static>` | the callee's return type is read as well; `'static` in it is kept, including as a lifetime argument of a generic path | `static_return_type_is_kept_in_the_shim`, `static_lifetime_argument_of_the_callee_return_is_kept` |
| Callee returns a named lifetime (`-> &'r T`, impl or fn generics) | excluded by rule, `named lifetime in callee return` (the shim cannot name it) | `named_lifetime_in_callee_return_is_excluded` |
| E0603 `module X is private` (1,030 on horizon-world): THIR prints definition paths through private modules | `publicize_paths` drops the private module's segment from shim signatures and callee paths unless the caller is inside the module's parent (`placement::context::Err` becomes `placement::Err`) | `private_path_tests` (3) |
| Generic type aliases (`type Result<T> = ...`) were used as non-generic aliases and broke the `'static` mapping | aliases with generics are ignored | covered by the world run |
| `callsite-patch` also rewrote the in-place stamped modules `mobs` and `motion` (`modules.txt`), so the overlay stamped their trait wrappers twice (E0592) | every module named in `modules.txt` for the crate is skipped by the patch | client_lib run |
| `include_bytes!` assets (PNG) made the patch diff fail with "not valid UTF-8" | binary files of the source closure are skipped | client_lib run |
| Receiver `Index::index` deref counted as an extra deref | `yoake-hook-thir` receiver tree fix | `overloaded_index_receiver_does_not_add_extra_deref` |
| Re-run lost exact unchanged duplicate sites | `patch_reidentify` keeps them confirmed | codemod tests |

`yoake-hook-thir dump` got `--files-from LIST` because 1,203 `--file` arguments exceed the Windows command line limit (horizon-client-lib).

## The compile/exclude loop

`yoake-hook-codemod callsite-patch --crate X --thir f.json --verify [--verify-rounds N] [--verify-max-withdraw N]`:

1. generate the sites and the patch (scratch copy, ledger shard, `hook-ledger/X.callsites.patch`);
2. run the stamped overlay check of the crate (`check -p <pkg> --lib --features yoake2-slice[,yoake-slice/wasm]`); a stamped build that prints
   `is built UNSTAMPED` aborts the loop;
3. map every compile error to its `__yoake_cs_<id>` shim (primary span, then secondary spans, then error locations, then a shim name in the
   message), tombstone those sites and record them as manual exclusions with the reason `compile error E....: <message>`;
4. regenerate and repeat (8 rounds by default). More than `--verify-max-withdraw` withdrawals in one round (default 300) is refused as a likely
   systematic error: that is how the private-module class was found (1,063 on horizon-world), and the generator was fixed instead.

Errors that point at the use of a shim result and not at the shim (lifetime errors in a struct literal) cannot be mapped; the loop stops and
names them. That is how the `'static` return and generic alias problems were found. They are fixed in the generator, not worked around.

## Hot callers

`hook-ledger/callsite-hot-callers.txt` has 55 entries: the older hand list (entities, 18) plus every live fn of `reports/hit_counts-rem03b.txt`
with at least 5,000 calls, mapped id to key through `hook-ledger/*.json` (37, all in `horizon_client_lib::mobs`, `motion`, `entities`). I rechecked
this against the 668 fn ids with 5,000 or more calls: 41 are live, all are on the list except `entities::fluids::update_entity_fluids` and the two
`<ItemEntity as EntityRules>` fns, which are in-place (`entities`, `callsites=true`) and keep their committed sites. The other hot ids (beardifier,
nbt, climate, shape join, hud) are tombstoned, so their callers fall under `caller not in fn ledger` and carry no call site.

## Per-crate results (live sites in the patch, exclusions by reason)

"compile errors" are the exclusions the loop produced from real rustc errors; they are included in the total.

| Crate | patch files / hunks | live sites | excluded | main reasons | compile errors |
| --- | --- | --- | --- | --- | --- |
| horizon_command | 14 / 145 | 1,563 | 2,417 | caller not in fn ledger 927, path call 839, non-engine callee 416, inside macro 176 | 15 (13 E0502, 2 E0503); the 514 first errors were generator classes |
| horizon_core | 20 / 78 | 126 | 3,919 | caller not in fn ledger 2,097, path call 1,271, non-engine callee 368, inside macro 77, track-caller 41, dyn receiver 22 | 31 (21 E0603, 8 E0614, 2 E0502); found before the private-module fix |
| horizon_items | 5 / 33 | 58 | 2,707 | caller not in fn ledger 1,471, non-engine callee 657, path call 538 | 0 |
| horizon_protocol | 27 / 235 | 1,146 | 4,638 | path call 1,890, caller not in fn ledger 1,744, non-engine callee 840 | 72 (68 E0502, 4 E0433) |
| horizon_world | 145 / 861 | 2,977 | 44,155 | caller not in fn ledger 24,708, path call 12,455, non-engine callee 6,075, named lifetime 64 | 26 (19 E0433, 4 E0502/E0503, 3 lifetime) |
| horizon_client_render | 135 / 1,254 | 2,470 | 41,056 | caller not in fn ledger 14,339, non-engine callee 13,547, path call 12,035, named lifetime 37 | 33 |
| horizon_client_lib | 128 / 1,587 | 3,802 | 33,909 | non-engine callee 11,314, caller not in fn ledger 10,900, path call 10,253 | 52 (20 E0433, the rest borrow and lifetime errors) |
| horizon_client | no patch | n/a | n/a | see "Left" | n/a |

(`horizon_client_lib` counts include the 58 in-place `entities` sites and `horizon_client` keeps its 41 in-place `game::item_use` sites.)
"path call" is the THIR kind rule (constructors, enum variants, plain fns are not rewritten); UFCS method calls are rewritten as path calls.

Each patch was committed only after its stamped overlay check was clean, one commit per crate.

## Results

| Check | Result |
| --- | --- |
| `test -j 6 -p yoake-hook-codemod -p yoake-callsite-match -p yoake-hook-ledger -p yoake-hook-thir` | all pass (codemod: 44 bin unit, 5 + 21 + 21 integration incl. the new fixture tests) |
| CI `callsite-verify` for `horizon_client_lib` module `entities` (THIR dump of the 3 module files, `verify-callsites`) | `verified 58 of 58` |
| CI `callsite-verify` for `horizon_client` module `game::item_use` | not run (see Left) |
| stamped check `tools/yoake-stamped-cargo.sh check -p horizon-client --bin horizon-client --features yoake2-slice` (all 7 patches applied) | exit 0, 0 `UNSTAMPED` lines |
| stamped release build `build --release -p horizon-client --bin horizon-client --features yoake2-slice` | **not completed**: stopped on purpose (disk) |

## Left

1. **The stamped release build was not run to the end.** C: had 12 to 17 GB free (other worktrees on the machine hold 45 GB of targets and I may only
   delete my own). A first attempt dropped free space from 16 GB to 5.9 GB in a few minutes while compiling dependencies, so I stopped it (rule: stop
   under 12 GB). The checks above are a smaller stand-in: the same stamped wrapper, the same patches, the whole dependency chain of
   `horizon-client`, 0 UNSTAMPED. The release build still has to be run on a disk with about 30 GB free:
   `bash tools/yoake-stamped-cargo.sh build --release -p horizon-client --bin horizon-client --features yoake2-slice`.
2. **`horizon_client` has no patch.** Its THIR dump (`yoake-hook-thir dump`, whole-crate rustc output) needs more than 14 GB of scratch; the tool
   killed rustc twice at the 5 GB limit. The crate keeps its 41 in-place `game::item_use` sites. Needs a disk with free space, then
   `callsite-patch --crate horizon_client --thir ... --verify`. The CI `callsite-verify` step for `game::item_use` has the same need.
3. The patches were made in this order and the generator got fixes in between (`'static` returns, private modules, named lifetimes). The first
   four patches (command, core, items, protocol) were generated before those fixes. They compile, but re-running `callsite-patch` on them with
   the current generator would also recover the sites the older generator could not name (for example `-> &'static T`) and may drop a few that it
   excludes now. A re-run of the four is cheap for items and command and takes about an hour for protocol and core; worth doing before 2.0.
4. `callsite-bench-gate` (no-mod overhead with `yoake-callsite-baseline`) was not run: it needs the release build.
5. Errors that point at a use of a shim result (lifetime errors in a struct literal) cannot be mapped to a shim; the loop reports them and stops.
   None remain in the committed patches.
