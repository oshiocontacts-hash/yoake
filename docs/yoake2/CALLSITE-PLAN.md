# Call-site hooks: production plan

Status: draft, 2026-11-18. It builds on MX3CS (THIR resolves 99-100 % of written method calls) and on
Fable's review of it, with the coordinator's decisions folded in. Phase 1 is being implemented on
`yoake2/callsite`. Store hooks stay counted only.

## 1. Shape

A chosen call is rewritten once, in the source, into a call to a generated private **shim fn**:

```rust
// before, in horizon_client::game::item_use, inside Game::start_using_item
let held = self.held();
// after
let held = __yoake_cs_131(self);

#[yoake_hook(id = 131, key = "fn:103@call:fn:2207", kind = callsite)]
#[inline(always)]
fn __yoake_cs_131<'r>(recv: &'r crate::game::Game<'_>) -> Option<&'r ItemStack> {
    recv.held()
}
```

- **The identity lives in the source.** After the rewrite, no matcher is needed to find the site again.
  Duplicate ids fail on the existing per-id claim symbol, and the key is checked against the ledger as for
  fn hooks.
- **Types are spelled out.** The shim's parameters are the THIR-typed receiver (after autoref and autoderef)
  and arguments. A later change of the receiver or argument types is a compile error, not a silent
  retarget. After every rewrite, THIR is run again to check that the call inside the shim resolves to the
  same callee DefId as the original.
- **Non-`'static` arguments work** through the existing higher-ranked typed registration. There is no arity
  limit: the shim has as many parameters as the call.
- **Not rewritten (excluded with a reason):** calls whose argument or receiver type cannot be written
  (closures, `impl Trait`, opaque or unnameable types, or types not visible from the caller's module);
  a borrowed return when the shim would have more than one borrowed input (the lifetime cannot be elided and
  the callee's lifetime relation is not in THIR); calls inside macro arguments (`assert!(self.ok(x))`);
  calls in an inactive cfg (`excluded(cfg-inactive)`, frozen); generic or `dyn` receivers; explicit
  two-phase borrows that would no longer borrow-check (`v.push(v.len())`).
  These are found by trying: rewrite, compile, then exclude each site the compiler rejects, and repeat.

## 2. Type source in CI: THIR

Per crate listed with `callsites = true` in `hook-ledger/modules.txt`:
1. `cargo rustc -vv -p <crate> --lib -- --cfg yoake_thir` gives cargo's exact rustc command line and
   environment for that crate. The `--cfg` rebuilds only that crate.
2. That rustc process runs alone with `RUSTC_BOOTSTRAP=1` (that process only, never the whole cargo run,
   which would rebuild every dependency) and `-Zunpretty=thir-tree`. Its arguments come from an `@argfile`
   (Windows' 32767-character limit), and its output goes to a file, because writing to a pipe fails with
   os error 1450. Before it starts, the free disk is checked (at least 10 GB); a watchdog kills it below
   5 GB. The file is deleted after parsing.
3. A streaming parser, versioned together with the pinned toolchain, reads the Call nodes of the listed
   modules. Sanity checks: it must see Call nodes with `fn_span`; per module, its count of written calls
   (span context `#0`) must match the syn count within 1 %. On a toolchain bump, the parser is re-run on a
   committed fixture.
4. **Span join, 1:1:** every THIR `#0` call is joined to exactly one syn call expression by its span.
   Every unjoined call on either side gets a reason. Calls inside macro arguments are excluded.
5. **Worktree target-dir guard and staleness:** `yoake-hook-thir dump` refuses a `CARGO_TARGET_DIR`
   whose `.yoake-workspace-root` marker belongs to another worktree unless `--allow-shared-target` is
   passed (see [BUILDING-WORKTREES.md](BUILDING-WORKTREES.md)). It also records per-file FNV-1a 64 hashes,
   byte lengths, and a `source_snapshot` so `yoake-hook-codemod --thir` rejects stale dumps before
   running matching.

## 3. Ledger entries (`kind: callsite`)

```json
{ "id": 131, "kind": "callsite", "status": "live",
  "key": "fn:103@call:fn:2207",
  "caller": 103, "callee": { "fn": 2207 },
  "callee_path": "horizon_client::game::Game::held",
  "anchor": "5f3a...", "stmt_path": ["9c1e...", "02ab..."], "context": ["77d0...", "0000..."],
  "dup_rank": 0, "shim": "__yoake_cs_131", "sig": "..." }
```

- The caller is the fn-ledger id of the enclosing fn. The callee is a fn-ledger id when it is an engine fn,
  otherwise its normalised canonical path (lifetimes erased, local crate prefixed, impl type arguments kept
  as data, not in the key). Renaming a fn then does not make all its sites `lost`.
- `anchor`, `stmt_path`, `context` and `dup_rank` are kept for the matcher. The matcher only runs when a
  site is first added, and when code around a candidate that has not been rewritten yet changes.
- cfg: the canonical configuration is Windows with default features.

## 4. Matcher (adding sites; Fable's B1 and B2 fixed)

- **Confirmed** (applied automatically): (a) the anchor, statement path and context are all equal; or the
  site's statement is identical (same hash) and aligned by the LCS of the caller's statements, and it is the
  only site of that callee in that statement.
- **moved**: the anchor is unique but the statement moved (old pass b). Example: delete `f(1)` in one branch
  and add `f(1)` in another. **changed**: a replacement gap in the alignment (old pass c's 1:1 gap). Example:
  `let n = items.len()` becomes `let total = other.len()`. Both need a human `--rename-site OLD=NEW` (or
  `--tombstone-site`); they are never bound automatically.
- **fragile** (identical duplicates resolved by rank), **ambiguous** and **lost** are errors.
- `apply` and `check` both fail until every non-confirmed item is resolved with `--rename-site` or
  `--tombstone-site`. The output names each site with its file, line and both candidate texts.

## 5. Scope and capacity

- Opt-in per module (`callsites = true`): entities and item_use first, then a curated list of gameplay
  modules. Hot loops stay out through `no_hook`.
- `SLOT_CAPACITY` becomes 1,048,576 (8 MiB of zeroed BSS, touched only when a slot is used). Phase 1 measures
  the resident memory of the bench example before and after the change.

## 6. CI time budget

| step | budget |
| --- | --- |
| THIR dump, horizon_client_lib | about 70 s, 4 GB temporary |
| THIR dump, horizon_client | about 130 s, 6.5 GB temporary |
| parse + join + verify (both) | under 2 min |
| codemod `check` (syn only, no THIR) | seconds |

The THIR job runs on pushes to `yoake2/**` and on pull requests that touch listed modules or the ledger; it
is never part of a local build. A normal `check` does not need THIR, because rewritten sites carry their
identity and types in the source.

## 7. Phases

1. The matcher fixes (B1, B2, fragile as an error), the shim rewrite for `game::item_use` (66 method calls),
   a test that empty callsite slots leave behaviour unchanged, and `horizon-client --lib` unchanged.
   SLOT_CAPACITY 1,048,576 with a memory check. Report: MX3CS2.md.
2. entities (316 method calls, 27 codemod trait delegations excluded).
3. The CI job, the curated module list, and the store-hook design.
