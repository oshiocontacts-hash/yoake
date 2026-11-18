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

## Open problems

- `check --callsites --thir <stale file>` reports every rewritten site as `lost`. With a THIR dump taken after
  the rewrite it is clean. It should detect a stale dump (its spans no longer match the source) and fail with
  that message.
- Path calls (`Type::f(x)`, `f(x)`) are Phase 2; so are calls with closure arguments (the shim would need the
  closure's type) and the generic and `dyn` receivers.
- The codemod does not yet run the compile-then-exclude loop of the plan; item_use compiled after the two fixes
  above without needing it.
- Shared `CARGO_TARGET_DIR` across several git worktrees is unsafe: cargo gives workspace members the same
  artifact hash in every worktree and decides freshness from file times, so a build in one worktree can
  silently reuse another worktree's newer artifact. The first SLOT_CAPACITY measurement here was wrong for that
  reason and was redone after touching the source. CI is unaffected (one checkout per job).
- Next: entities (316 method calls) with the same pipeline.
