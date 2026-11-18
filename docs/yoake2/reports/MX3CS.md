# MX3CS: feasibility of typed, stable call-site hooks (THIR + content identity)

Date: 2026-11-18. Branch `yoake2/callsite` on `yoake2/ids` @c5fe804a7. LENOVO (Windows, MSVC stable
1.100.0). Fable's redesign asked for (a) a body-level type source and (b) a stable call-site identity.
Nothing is written to the ledger yet; no engine source is changed on this branch.
Plan A (rust-analyzer SCIP) was not tried: rust-analyzer is not installed here (only the rustup proxy),
and plan B works without installing anything.

## Summary

- **(a) Type source: THIR from the pinned stable rustc with `RUSTC_BOOTSTRAP=1` and `-Zunpretty=thir-tree`.**
  Every written method call in both modules resolves to a callee DefId, with the receiver type and
  source spans, exactly as rustc typed it:

  | module (crate) | written method calls | resolved to a concrete callee | `dyn` (run-time dispatch) | generic receiver | earlier syn result |
  | --- | --- | --- | --- | --- | --- |
  | entities + entities::fluids (`horizon_client_lib`) | 316 | 314 (99.4 %) | 2 | 0 | 315 of 535 candidates had no receiver type |
  | game::item_use (`horizon_client`) | 66 | 66 (100 %) | 0 | 0 | 66 of 86 had no receiver type |

  The totals agree with the earlier syn count: entities 316 method + 193 path + 27 codemod trait
  delegations = 536 written calls (syn: 535); item_use 66 + 18 = 84 (syn: 86).
- **(b) Stable identity: content anchors plus statement alignment, never ordinals.** The prototype
  `spikes/cs-identity` (agy) keeps ids when an earlier call to the same callee is inserted, when statements
  are reordered, and when arguments change; deletions and ambiguous cases are reported, never retargeted.
  9 tests pass.
- **Cost:** one THIR dump per instrumented crate. `horizon_client_lib`: 68 s, 3.85 GB of text;
  `horizon_client`: 130 s, 6.36 GB. About 55 s of the first is the normal type check; the rest is printing.
  Feasible for a codegen step in CI, too heavy for every local build (see "Build-time cost").

## (a) THIR as the type source

How it is run (`spikes/cs-thir/`):
1. `cargo rustc -vv -p <crate> --lib -- --cfg yoake_thir_probeN` once, to get cargo's exact rustc command
   line and environment for that crate (the `--cfg` only forces that one crate to rebuild).
2. Re-run that rustc process alone with `RUSTC_BOOTSTRAP=1` and `-Zunpretty=thir-tree`, the arguments
   from an `@argfile` (the horizon-client command line is longer than Windows' 32767-character limit),
   stdout to a file (`thir_to_file.py`, with a disk watchdog).
3. Stream-parse the file (`thir_calls.py`): every `Call` node whose `fn_span` is in the target files, with
   the callee `FnDef(DefId(.. ~ path))`, generic arguments, `from_hir_call`, the method-name span and the
   first argument's type and span. `thir_resolve.py` then builds canonical callee keys and statistics.
   The per-call results are in `spikes/cs-thir/results/`.

Findings about the format:
- `from_hir_call: false` marks compiler-inserted calls (overloaded operators, autoderef `deref_mut`), not
  method syntax. Written method syntax is recognised by the receiver span starting before the method-name
  span. Spans end in a syntax context `(#N)`: `#0` is written code; non-zero contexts are macro expansions
  (format!, assert!, and the `#[yoake_hook]` expansions), which a call-site hook must not target.
- Inherent methods print as `crate::path::{impl#k}::method`; the owner type comes from the receiver type
  (`horizon_core::geometry::Vec3::length_squared`). Trait methods print as the trait item with `Self` as the
  first generic argument, which gives `<Self as path::Trait>::method`.
- Keys need normalising before they go into a ledger: drop erased lifetimes (`Game<'{erased}>`), prefix
  the local crate name (THIR prints local paths without it), and keep the callee's own path without the
  impl's concrete generic arguments (`Option<(usize, u32)>::is_none` becomes `std::option::Option::is_none`,
  with the arguments kept as data).

Problems met, all solved without installs:
- Piping THIR stdout fails: on Windows rustc panics with `failed printing to stdout ... (os error 1450)`
  when it writes very large chunks to a pipe. Writing to a file works.
- Setting `RUSTC_BOOTSTRAP=1` for a whole `cargo` run changes every dependency's fingerprint, rebuilds them
  under new hashes, and once filled the disk. Now only the final crate's rustc process gets it.
- The dump is large, so a watchdog kills rustc if free space drops under 15 GB. Both dumps were deleted
  after parsing.

### Stability across rustc versions

`-Zunpretty=thir-tree` is a Debug-style dump of compiler internals, not a stable interface. Field names
change between releases (this toolchain prints `temp_scope_id`), and the layout can change without
notice. It is acceptable because the toolchain is pinned. Required guard rails:
- the parser is versioned together with the pinned toolchain, and a toolchain bump re-runs it on a fixture;
- the parser fails loudly when it sees no `Call` nodes with `fn_span`, or when its written-call count for a
  module differs from the syn count by more than a few (the cross-check above: 536 vs 535);
- `RUSTC_BOOTSTRAP=1` is used only in this codegen step, never for the shipped build.

### Build-time cost

| crate | wall time | of which type check (time to first output) | THIR text |
| --- | --- | --- | --- |
| horizon_client_lib | 68 s | about 55 s | 3.85 GB |
| horizon_client | 130 s | not measured separately | 6.36 GB |

Parsing the 3.85 GB dump for two files took 43 s. So a callsite codegen pass costs roughly one extra
`cargo check` of each instrumented crate, a few GB of temporary disk, and under a minute of parsing.
Options to shrink it, not tried here: `-Zdump-mir` with a function-name filter (per-function files; MIR
terminators carry the same resolved callees), or a small rustc driver (needs the rustc-dev component, which
is an install and nightly-only). In production, run THIR only in the codemod's `apply` step and in CI, and
keep `check` on the ledger plus the stored anchors (part b), so ordinary builds never pay for it.

## (b) Stable call-site identity

Ordinal keys (`#0`, `#1`) silently retarget when an earlier call to the same callee is inserted. The
prototype replaces them with content identity and explicit failure (`spikes/cs-identity`, agy):
- each site records the callee (THIR-resolved in production, syntactic in the prototype), an anchor
  (FNV-1a hash of the call expression's tokens), the hashes of its statement path inside the body, the
  hashes of the neighbouring statements, and a rank among identical duplicates;
- re-identification works per callee in passes: exact anchor + statement + context; unique anchor; LCS
  alignment of the caller's statement hashes; identical duplicates by rank, marked `fragile`; anything else
  is `lost` (old) or `new` (new id), and one old record matching two new sites equally well is `ambiguous`.

`cargo test` in `spikes/cs-identity`: 9 passed. They show:
- inserting `work(0)` before `work(1)` gives the new call a new id and keeps the old ids (the ordinal
  scheme would have moved `#0`);
- swapping `f(1)` and `f(2)` keeps each id on its call;
- changing the arguments of a call with unchanged neighbours is matched by alignment;
- a deleted call is reported lost; a call moved to another fn is lost in the old caller;
- `f(1); f(1);` plus one more inserted at the top is reported fragile;
- comments and whitespace change nothing;
- nested blocks, closures and chained calls keep their order;
- one old record with two equal new candidates is reported ambiguous.

## Recommendation

Feasible. Proposed production shape:
1. `yoake-hook-codemod apply --callsites` runs the THIR step for the instrumented crates, joins THIR calls
   (callee, receiver type, span) with syn call expressions by span, and keeps only `#0` (written) calls with
   a concrete or trait-resolved callee; `dyn` and generic receivers are excluded with a reason.
2. Ledger `kind: callsite` entries store the id, caller key, normalised callee key, anchor, statement path
   hashes, context, and the caller's body statement hashes. `check` re-identifies with syn alone (no THIR)
   and fails on lost, ambiguous or fragile sites until `--rename-callsite` or `--tombstone` resolves them.
3. The rewrite emits the slot around the call expression. That part is still to be built: the `callsite!`
   proof on yoake2/ids covers only the runtime side.

## Side note for yoake2/ids

Making `struct Game` `pub(crate)` (round 2, SF6) turned 7 existing signatures into `private_interfaces`
warnings (for example `Game::attack_effects` exposes `Spared`, and `Game::handle_window_event` exposes
`InputOrigin` and `EventLoopCtl`). They are warnings, not errors. Options: allow the lint on those items, or
give the typed registration functions `pub(in crate::game)` visibility for hooks inside `game` and keep
`Game` private.

## Appendix: identity prototype report (agy) (`spikes/cs-identity`)

### 1. Identity Scheme

Ordinal keys (`<caller>@call:<callee>#0`, `#1`) silently retarget when an earlier call to the same callee is inserted or statements are reordered. `cs-identity` replaces ordinal keys with structural extraction and a multi-pass matcher that either proves a match or surfaces ambiguity/loss explicitly.

### Extraction (`extract` / `extract_caller`)
For a target function or method (`caller`), we walk its body `Block` in source order (including calls inside closures, flagged with `in_closure: bool`, while excluding nested `fn` items) and record each `ExprCall` and `ExprMethodCall`:
- `callee`: syntactic callee text (`path::to::f` or `.method`); production integration replaces this with the THIR-resolved callee key.
- `anchor`: FNV-1a 64-bit hash of the call expression's tokens with all whitespace removed (comments are stripped by tokenization).
- `stmt_index`, `stmt_hash`: top-level body statement index and token hash.
- `stmt_path`, `stmt_path_hashes`: statement index path from the body root (e.g. `[1, 0]`) and token hash at every nesting level.
- `context`: `(prev_hash, next_hash)` of sibling statements in the immediately enclosing block (`0` at block boundaries).
- `dup_rank`: 0-based index among calls in the same caller sharing `(callee, anchor)`.
- `CallerSnapshot { caller, body_stmt_hashes }`: top-level statement hashes of the caller body (including statements without calls) for diff alignment.

### Multi-Pass Re-identification (`reidentify` / `reidentify_with_snapshots`)
Matching runs per `(caller, callee)` among still-unmatched records and sites:
1. **Pass (a) — Exact structural match**: matches 1-to-1 `(anchor, stmt_path_hashes, context)`, skipping adjacent identical-statement duplicate runs whose multiplicity changed (`old_count != new_count`).
2. **Pass (b) — Unique anchor**: matches 1-to-1 `anchor` within the caller/callee when context or statement paths shifted.
3. **Pass (c) — LCS diff alignment**: computes unique optimal LCS anchors over `body_stmt_hashes` (plus 1-to-1 replacement gaps bounded by real LCS anchors). If an aligned old/new statement pair contains exactly one unmatched old record and one unmatched new site of the same callee, they match.
4. **Pass (d) — Identical duplicates fallback**: when `old` already had `>= 2` identical calls with equal anchors (`f(); f();`), pairs remaining sites in `dup_rank` order and records their IDs in `Outcome::fragile` so tooling warns.
5. **Pass (e) — Explicit reporting**:
   - One unmatched old record with `>= 2` equally plausible new sites -> `Outcome::ambiguous` (no match, no new ID assigned to candidates).
   - Remaining unmatched old records -> `Outcome::lost`.
   - Remaining unmatched new sites -> assigned fresh IDs from `next_id` in `Outcome::new`.

### 2. What Each Test Shows (`tests/identity.rs`)

- `insert_earlier_call_keeps_old_ids`: inserting `work(0)` before `prep(); work(1); done();` preserves the IDs of `prep`, `work(1)`, and `done` via Pass (a) and assigns a fresh ID to `work(0)`. An ordinal `#0` scheme would have silently retargeted `work(1)` to `work(0)`.
- `swap_statements_follows_calls`: swapping `f(1);` and `f(2);` breaks `(context)` so Pass (a) skips them, and Pass (b) follows their anchors so IDs track the calls rather than their positions.
- `changed_args_matched_by_alignment`: changing `f(10, 20)` to `f(99, 100)` with unchanged neighbors (`let a = 1; ... let b = 2;`) changes both `anchor` and `stmt_hash`; Pass (c) aligns the surrounding statements and matches the 1-to-1 gap.
- `deleted_call_reported_lost`: deleting `f(1)` reports its ID in `Outcome::lost` instead of binding it to another site.
- `identical_duplicates_insert_at_top_is_fragile`: starting from `f(1); f(1);` and inserting a third `f(1);` at the top cannot be disambiguated structurally; Pass (d) pairs by `dup_rank` and flags the matched IDs in `Outcome::fragile`.
- `moved_to_another_fn_is_lost_in_old_caller`: moving `f(1)` from `first` to `second` reports it `lost` in `first` (caller-level matching is out of scope).
- `comments_and_whitespace_change_nothing`: adding comments and reformatting across lines leaves `Site` fields byte-for-byte identical and matches everything in Pass (a).
- `nested_blocks_closures_and_chained_method_order`: verifies `stmt_path` (`[1, 0]`), `in_closure: true`, and left-to-right callee span ordering on `items.iter().map(...)`.
- `one_old_two_new_same_anchor_is_ambiguous`: cloning `if c { f(1); }` into `if c { f(1); } else { f(1); }` reports `ambiguous` rather than picking a branch silently.

### 3. Known Failure Modes

- **Identical duplicates (`f(1); f(1);`)**: inserting/removing a copy inside a run of identical statements shifts `dup_rank`, so Pass (d) can bind an existing ID to the newly inserted copy. Mitigated by `Outcome::fragile` warnings; long-term fix requires explicit source pins or THIR/block disambiguation.
- **Large rewrites**: rewriting a function body so few top-level statements remain in common drops LCS anchors below uniqueness, causing modified calls to appear as `lost` + `new` and requiring manual `--rename-callsite` / `--tombstone`.
- **Macro-generated calls**: syntactic `syn` extraction only sees the macro invocation tokens (`foo!(...)`), not expanded `ExprCall`s inside it. Production integration must either expand macros first or match against THIR/HIR spans tied back to macro call sites.
- **Calls inside closures**: recorded with `in_closure: true` and nested `stmt_path_hashes`, but if a closure is extracted into a helper function or moved between outer statements, its enclosing statement hash changes and requires Pass (b) (unique anchor) or manual rename.
- **Code moved between functions**: per-caller matching reports moved calls as `lost` in the old caller and `new` in the destination caller; cross-function moves require `--rename-callsite` or a separate whole-crate pass.

### 4. Ledger Entry Schema & CLI Flow

### Ledger Entry Additions for `Kind::Callsite`
In `crates/yoake-hook-ledger`, a callsite `Entry` (plus per-caller snapshot table) stores:
- `id: u32` — immutable hook slot ID.
- `key: String` (or structured fields): stable logical key / `caller` key + `callee` key (THIR-resolved path in production).
- `anchor: String` (hex FNV-1a 64) — call expression token hash.
- `stmt_path_hashes: Vec<String>` — enclosing statement hash chain from body root.
- `context: (String, String)` — sibling statement hashes `(prev, next)`.
- `dup_rank: u32` — tie-breaker rank for identical duplicates.
- Per-caller `CallerSnapshot { caller: String, body_stmt_hashes: Vec<String> }` stored alongside entries in the ledger shard so Pass (c) works even across call-free statements.

### CLI Flow (`yoake-hook-codemod`)
1. `check`: runs extraction + `reidentify_with_snapshots` against the ledger shard.
   - Fails non-zero if any callsite is `lost` or `ambiguous`, printing candidate indices/contexts for `ambiguous` and old IDs/callers for `lost`.
   - Emits warnings for any `fragile` matches (identical duplicates matched only via `dup_rank`).
2. `apply`: fails if `lost` or `ambiguous` remain unresolved; otherwise updates `anchor`, `stmt_path_hashes`, `context`, `dup_rank`, and `CallerSnapshot` for `kept` entries, and allocates new IDs for `new` callsites.
3. Explicit overrides (matching existing fn `--rename` / `--tombstone` flags):
   - `--rename-callsite OLD=NEW`: binds a `lost` or `ambiguous` callsite ID/key to an explicit target site/key and records `renamed_from`.
   - `--tombstone KEY`: marks a deliberately deleted callsite as `Status::Tombstone` so `check` passes cleanly without reusing its `id`.
