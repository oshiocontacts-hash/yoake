# MX3CS2: call-site hooks, Phase 1 (game::item_use)

Date: 2026-11-19. Branch `yoake2/callsite` (includes yoake2/ids @476883750). LENOVO (Windows, MSVC stable
1.100.0, `-j 4`). Plan: docs/yoake2/CALLSITE-PLAN.md. Kimi Code wrote yoake-hook-thir and the ledger, macro
and runtime parts until it hit its 5-hour quota; agy wrote yoake-callsite-match and the codemod's callsite
pass; Claude fixed, ran and measured everything.

## Result

- **58 + 3 = 61 of the 66 written method calls in `game::item_use` are rewritten into ledger-backed shims**
  (`__yoake_cs_<id>`, `kind: callsite`). The other 5 take a closure argument whose type cannot be written and
  are excluded with that reason. The 18 path calls are excluded as `path call` (Phase 2). The two `Some(..)`
  constructors are syn-only (THIR has no Call for them) and are listed as unjoined.
- **THIR verification after the rewrite:** a fresh THIR dump of the rewritten crate shows the call inside every
  one of the 61 shims resolving to exactly the callee recorded in the ledger (61 match, 0 mismatch, 0 missing).
- **Behaviour unchanged with empty slots:** `cargo test -p horizon-client --lib` 1193 passed, 0 failed,
  40 ignored (same as before); `cargo test -p horizon-client-lib --lib` 1619 passed, 0 failed, 48 ignored.
- `codemod check --callsites` (syn only, as CI runs it) is clean; with a fresh THIR it is clean too.
  item_use.rs is rustfmt-clean.
- Ledger shard `horizon_client.json`: 23 fn + 61 callsite live entries, 23 callsite exclusions; meta
  next_id 187.

## The pieces

| piece | what it does | tests |
| --- | --- | --- |
| `crates/yoake-hook-thir` (new) | `dump`: gets cargo's rustc command (`cargo rustc -vv`), re-runs it alone with `RUSTC_BOOTSTRAP=1 -Zunpretty=thir-tree` from an `@argfile`, stdout to a file with a disk check, stream-parses it, deletes the dump. `parse`: the same on an existing dump. Output: every Call of the listed files with span, method-name span, syntax context, kind, caller, normalised callee, return and argument types (null when not writable), receiver autoref/autoderef. | 3, on `tests/fixtures/thir-fixture.txt`, the real rustc output for `spikes/cs-thir/fixture/lib.rs` |
| `crates/yoake-callsite-match` (new) | The matcher with Fable's fixes: only exact matches and identical, LCS-aligned statements are `confirmed`; unique-anchor moves are `moved`, replacement gaps are `changed`; `fragile`, `ambiguous`, `lost` are errors. | 11, including Fable's two examples |
| codemod `--callsites` | Joins THIR calls to syn expressions 1:1 by span; picks eligible method calls; rewrites `RECV.m(A..)` to `__yoake_cs_<id>(ADJ(RECV), A..)` and appends one generated shim block per file; ledger `kind: callsite` entries with a `site` object; `--accept-site KEY@FILE:LINE:COL`, `--rename-site OLD=NEW` (a real key change), `--tombstone-site KEY`. | 6 integration tests + the existing 29 |
| ledger / macro / runtime | `site` object on entries; `#[yoake_hook(..., kind = callsite)]`; `SLOT_CAPACITY = 1 << 20`. | ledger 25, macro 11, rt 13 |

Example shim (generated):

```rust
#[yoake_hook_macro::yoake_hook(id = 126, key = "fn:103@call:horizon_client::game::Game::held", kind = callsite)]
#[inline(always)]
fn __yoake_cs_126<'r>(
    recv: &'r crate::game::Game<'_>,
) -> std::option::Option<&'r horizon_client_lib::items::ItemStack> {
    recv.held()
}
```

The site `self.held()` became `__yoake_cs_126(&*self)` (THIR reported autoref shared and one deref).

## Fixes made while running it

In Kimi's yoake-hook-thir (Kimi stopped before it could build anything):
- the span splitter dropped the first line number (`file:L1:C1: L2:C2` was read as file `file:L1`);
- calls expanded from macros can have arguments without their own span: they stay in the list with the whole
  call's span instead of failing the parse;
- a whitespace reset the "path starts here" state, so `&'_ Stack` did not get `crate::`;
- the `arg:` line under a THIR `Borrow` overwrote `borrow_kind`, so every receiver showed `borrow: none`;
- Calls whose type is not an FnDef (fn pointers, closure values) are kept as `<indirect>` with resolution `dyn`
  instead of failing;
- the `@argfile` wrapped arguments that contain spaces in quotes, which rustc reads literally
  (`cfg(feature, values(..))`); rustc's stderr is now kept on failure;
- the test fixture was a hand-written snippet with one span; it is now the real rustc output.

In the codemod and macro:
- **borrowed returns:** a shim like `fn s(recv: &Game<'_>) -> Option<&ItemStack>` does not compile (two input
  lifetimes). A borrowed return on a borrowed receiver now names the receiver's outer reference `'r` and uses
  it for the return, which is exactly the method elision rule. If a callee really ties its result to another
  argument, the shim fails to borrow-check: a compile error, never a silent change. This turned the 3
  "borrowed return with several borrowed inputs" exclusions into rewrites.
- the macro elided a named lifetime that appears in one input and in the result (it only kept lifetimes
  shared by two inputs), which broke such shims;
- `--rename-site OLD=NEW` could not express "accept this moved candidate" (the key does not change when a site
  moves): agy added `--accept-site KEY@FILE:LINE:COL`, and every non-confirmed site is printed with the flag
  to pass.

## SLOT_CAPACITY 1,048,576

The bench example (`yoake_hook_bench`, release) with 65,536 slots (yoake2/ids) and with 1,048,576 slots
(this branch):

| | 65,536 slots | 1,048,576 slots |
| --- | --- | --- |
| exe file size | 270,336 B | 270,336 B (the slots are zero-initialised, not stored) |
| `.data` virtual size | 525,008 B | 8,389,328 B |
| private bytes (commit) | 1.8 MB | 9.5 MB (+7.7 MB) |
| peak working set, second run | 4.5 MB | 4.5 MB |
| `tick_settle`, empty slots | 6.29-6.38 ns | 6.24-6.35 ns |

The 8 MiB are committed but untouched pages stay out of physical memory. The first run of the large build
peaked at 13.4 MB working set; the second run did not, and this was not investigated further.

## Phase 1b: entities (2026-11-19)

Same pipeline on `entities` + `entities::fluids` (crate `horizon_client_lib`, `callsites=true` in
hook-ledger/modules.txt).

- THIR: 536 written calls (343 method: 341 concrete, 2 dyn; 193 path), 4 min for the dump.
- **224 call sites rewritten.** Excluded with reasons (312): path call 193, caller not in the fn ledger 92
  (callers that are not hooked themselves, such as the 20 tombstoned `EntityFluidWorld` methods and the
  codemod's trait delegations), unwritable argument type 19 (closures), unwritable return type 3 (the never
  type `!`), unparseable return type 2, dyn receiver 1, inside a macro invocation 1, and 1 manual exclusion:
  `fn:37@call:fn:81` (`ExperienceOrb::update_fluids`), whose argument type
  `crate::motion::fluids::FluidResources` is only nameable through a private module. 31 syn calls have no THIR
  call: tuple-struct and enum constructors (`Ok`, `Some`, ...) and code under the inactive
  `yoake-hook-bench` feature.
- **THIR re-verify after the rewrite: 224 of 224 shims call exactly the recorded callee** (0 mismatch, 0
  missing).
- Empty slots: `horizon-client-lib --lib` 1619 passed, `horizon-client --lib` 1193 passed (both unchanged).
  `codemod check --callsites` is clean for both modules; entities.rs, fluids.rs and item_use.rs are
  rustfmt-clean. Ledger: horizon_client_lib 82 fn + 224 callsite live, 20 fn tombstones; horizon_client
  23 fn + 61 callsite live; meta next_id 411.

The compile-then-exclude round found five systematic problems, all fixed in the tools rather than excluded:
- `Vec<T, std::alloc::Global>`: the default allocator argument needs the unstable `allocator_api` to be
  written, so the normaliser drops `, std::alloc::Global>`;
- the never type `!` inside a type (`Result<T, E<!>>`) is unstable to write: such types are unwritable;
- an unsizing coercion (`DYE_NAMES.iter()`, array to slice) is a THIR `Pointer { cast: Unsize }` node, now
  transparent when computing the receiver adjustment (the call site coerces the same way);
- a borrowed return with one borrowed input that is not a borrowed receiver (`Iter<'_, T>` by value ->
  `Enumerate<Iter<'_, T>>`) ties that input's and the result's elided lifetimes to one named `'r`;
- manual exclusions (`"manual": true`, SF3) were not applied to call-site candidates; they now are.

### Coordinator's two additions

- **Stale THIR:** `yoake-hook-thir` records each listed file's FNV-1a hash and length and a
  `source_snapshot`; `apply`/`check` with `--thir` fail first with
  `THIR is stale: <file> changed since the dump (dump sha .., now ..); re-run yoake-hook-thir dump`.
  Checked on the real tree: passing the pre-rewrite entities dump gives that error and no `lost` site.
- **Shared target dirs:** `yoake-hook-thir dump` writes `<target>/.yoake-workspace-root` and refuses a target
  dir whose marker names another worktree (`--allow-shared-target` overrides). Checked on the real tree: a dump
  from wt-y2ids wrote the marker and the next dump from wt-y2cs was refused. Notes in
  docs/yoake2/BUILDING-WORKTREES.md, linked from CALLSITE-PLAN.md.
- CI: `.github/workflows/yoake-hooks.yml` now passes `--callsites` for modules flagged in modules.txt.

Tooling note: horizon-client-lib's sources appear in THIR as `crates/horizon-client-lib/../horizon-client/src/...`,
so `--file` must be given in that form; the parser does not yet normalise `..`.

## Phase 1c: Fable's should-fixes (2026-11-19)

| item | change | result on the two modules |
| --- | --- | --- |
| **S3** pin the callee | Shim bodies are fully qualified: `crate::game::Game::held(recv)`, `<T as path::Trait>::m(recv, ..)`, generic owners as `<OwnerWithArgs>::m(..)`. `check` reports old method-syntax bodies and `apply` rewrites them in place (same id and key). New `yoake-hook-codemod verify-callsites`; new workflow `.github/workflows/yoake-callsite-verify.yml` (workflow_dispatch, and pushes/PRs touching hook-ledger/, the call-site modules or the hook crates) dumps THIR per `callsites=true` crate and runs it. | All shims rewritten to the qualified form. **verify-callsites after a fresh THIR dump: entities 103 of 103, item_use 41 of 41.** |
| **S5** narrow the default | Only engine callees (first path segment `crate`, `horizon_*`, `yoake_*`, or `--engine-crates`) are eligible; anything else is `non-engine callee` unless listed in `hook-ledger/callsite-allow.txt` (committed empty). `--retire-ineligible` un-rewrites such sites, deletes their shims and tombstones their ids. | **141 sites retired** (entities 121, item_use 20); ids stay reserved as tombstones. Rewritten now: entities 103, item_use 41 (144). |
| **S1** `#[track_caller]` | Excluded with reason `track-caller`: a built-in list of 92 std/core/alloc entries (not exhaustive), plus `hook-ledger/track-caller.txt` from `yoake-hook-codemod track-caller-scan --workspace .`. | The scan found **no `#[track_caller]` fn in any of the 57 workspace members** (the 7 occurrences are in tool tests). **None of the 285 Phase 1/1b sites had a track_caller callee** (their unwrap family was `unwrap_or`, `unwrap_or_default`, `get_or_insert`). |
| **S2** one call per shim | `check`/`apply` fail unless every `__yoake_cs_<id>` is referenced exactly once outside its definition. | `check --callsites` clean for both modules. |
| **S4** compile scaling | `assert_ledger_key` is a const binary search over the id-sorted KEYS table (the build asserts the order), then a const string compare; tests with a 10,000-entry table. | See the measurements below. |
| nits | THIR dumps record `rustc` and `cargo_lock_sha`; CALLSITE-PLAN section 3 says `#n` is only an id-allocation number (no ordinal meaning; mods target sites by id). | |

Two tool fixes found while verifying: the macro moves a shim's body into `__yoake_orig___yoake_cs_<id>`, so
verify looks there; a fully qualified call to an inherent method is a THIR path call naming `{impl#k}`, so path
callees with `{impl#k}` take their owner from the first argument (right for shims; Phase 2 path call sites must
take it from the written path), and a trait path call (`Trait::m`) is accepted as the same item as the ledger's
`<Self as Trait>::m`.

Behaviour with empty slots: `horizon-client-lib --lib` 1619 passed, `horizon-client --lib` 1193 passed
(unchanged). Hook crates: 113 tests passed.

### Compile time and size (debug, horizon-client-lib crate rebuilt after touching entities.rs)

| build | crate rebuild | rlib |
| --- | --- | --- |
| yoake2/ids (fn hooks only, no call sites; own target dir) | 40 s | 181.7 MB |
| call sites before Phase 1c (285 sites, linear key lookup) | 36 s | 184.5 MB |
| after Phase 1c (144 sites, binary search) | 35 s | 183.1 MB |

The key lookup was not measurable at this size: run-to-run noise (several seconds) is larger than any
difference. The rlib grows by about 1.4 MB for 144 call sites.

### A/B bench, empty slots (release `yoake_hook_bench`, separate target dirs, three runs each)

| | `tick_settle` per call | 1024-entity slice, per entity | `move_applies` leaf |
| --- | --- | --- | --- |
| yoake2/ids (no call sites) | 6.72 / 8.07 / 6.72 ns | 15.21 / 16.06 / 15.29 ns | 7.59 / 7.77 / 7.17 ns |
| yoake2/callsite after Phase 1c | 8.92 / 8.39 / 7.54 ns | 19.17 / 19.14 / 17.10 ns | 7.52 / 8.56 / 7.33 ns |

The entity hot path pays for its empty call-site slots: `tick_settle` has 11 rewritten call sites,
`render_position` 1 and `move_applies` 2, and the slice (settle + interpolate + render_position per entity) is
about 2-4 ns (12-25 %) slower, roughly 0.1-0.3 ns per empty site. A tight per-entity loop is the worst case.
Recommendation: before more modules, mark hot-leaf callers such as `tick_settle`, `render_position` and
`move_applies` `no_hook` for call sites (or exclude call sites inside fns that are themselves hot leaves),
and add this slice to the CI benchmark gate of MX3S2.

## Open problems

- (fixed in Phase 1b) `check --callsites --thir <stale file>` used to report every rewritten site as `lost`.
- Path calls (`Type::f(x)`, `f(x)`) are Phase 2; so are calls with closure arguments (the shim would need the
  closure's type) and the generic and `dyn` receivers.
- The codemod does not yet run the compile-then-exclude loop of the plan; item_use compiled after the two fixes
  above without needing it.
- Shared `CARGO_TARGET_DIR` across several git worktrees is unsafe: cargo gives workspace members the same
  artifact hash in every worktree and decides freshness from file times, so a build in one worktree can
  silently reuse another worktree's newer artifact. The first SLOT_CAPACITY measurement here was wrong for that
  reason and was redone after touching the source. CI is unaffected (one checkout per job).
- `--file` paths are not normalised (`..`); the compile-then-exclude loop is still manual (one manual exclusion in
  entities).
