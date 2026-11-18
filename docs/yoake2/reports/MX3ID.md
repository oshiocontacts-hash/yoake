# MX3ID: one hook-ID ledger and a codemod

Date: 2026-11-18. Branch `yoake2/ids` on `yoake2/s2` @1321f14e9. Built and tested on LENOVO (Windows,
MSVC stable 1.100.0, `-j 4`). Ledger, runtime and macro by Kimi Code (k3.1-pro); codemod by agy
(gemini-4-hydrogen-high); spec, migration, runs and fixes by Claude. Shared spec: see "Spec" below.

## Summary

This summary describes round 1. Round 2 (next section) replaced the single ledger file with
`hook-ledger/` shards, the ledger-derived MAX_SLOTS with a fixed 65536-slot BSS array and per-crate key
tables, and the public `register` with typed `pub(crate)` registration.

- **Ledger** `crates/yoake-hook-rt/hook-ids.lock.json`, read through the new crate `yoake-hook-ledger`:
  append-only ids, `renamed_from` entries keep ids, tombstones are never reused, `kind` = `fn` |
  `callsite` | `store` (only `fn` is used now), deterministic JSON.
- **MAX_SLOTS from the ledger:** `yoake-hook-rt/build.rs` validates the ledger (the build fails on an invalid
  one) and generates `MAX_SLOTS = next_id`, `LEDGER_KEYS`, and 64 test slots on top. No fixed 4096.
- **Duplicates cannot compile:** every attribute carries `id` and `key`; the macro emits
  `const _: () = assert_ledger_key(id, key)`, so an id/key pair that is not in the ledger (a second fn on
  the same id included) is a compile error.
- **Codemod** `yoake-hook-codemod apply|check`: adds/fixes attributes from the ledger, appends new ids,
  converts trait impl methods to a delegating method plus an inherent `#[inline(always)]` hooked helper,
  excludes ineligible fns into the ledger with a reason, `--rename OLD=NEW`, `--tombstone-missing`,
  `--rustfmt`. `check` is the CI idempotency check.
- **Typed registration:** each hook emits `__yoake_register_<fn>(callback, order)` with the exact callback
  type, because a hand-written fn-pointer type with different binders was silently ignored (found in
  round 1). Debug builds also reject a second callback type on one slot.
- **`impl Game<'_>` methods are hookable:** S2 could not store a callback whose type mentions `Game<'x>` as
  `Any`. The new `self_ty = Game<'_>` argument (written by the codemod) rebinds impl lifetimes with
  `for<'..>`, so the callback type is `'static`.
- **Call-site proof:** `yoake_hook_rt::callsite!(test_id = K, callee, (args))` is one slot on one call.
  The test shows the empty slot unchanged, wrap, replace args and skip, and the other call to the same
  callee untouched.

## Round 2 (Fable review fixes, 2026-11-18)

The round-2 spec is summarised in the table. Kimi: ledger, runtime, macro. agy: codemod.
Claude: migration, crate wiring, CI, fixes, runs.

| Item | Fix | Proof |
| --- | --- | --- |
| **B1** two fns could claim one (id, key) | Every hook emits `#[used] #[unsafe(export_name = "__yoake_hook_slot_<id>")] static`; the macro also checks that the key's last segment is the fn name (`__yoake_trait_<trait>_<method>` helpers check `<method>`). | A probe with two fns on one id: `error: symbol __yoake_hook_test_slot_50 is already defined` (same crate; across crates the same symbol fails at link time). New CI workflow `.github/workflows/yoake-hooks.yml` runs `codemod check`; making it a *required* check is a repository setting for the owner. |
| **SF1** append-only not enforced | `check_append_only(base, current)` and `codemod check-history --base <dir> --current <dir>`: each base id exists, key unchanged or in `renamed_from`, tombstones stay, next_id never decreases, ids never change crate. CI compares against the PR base (or HEAD~1). | Ledger tests; real run against c0a3a2560's ledger: `check-history: ok`; the same ledger with one key edited: `history error: id 2 changed key without renamed_from`. |
| **SF2** `--tombstone-missing` too broad | Removed. `--tombstone KEY` (repeatable); a missing live key is an error naming the key and suggesting `--rename` or `--tombstone`. `live_id_including_renames`. | Codemod tests; the real run had to name all 20 tombstones and one rename explicitly. |
| **SF3** manual exclusions overwritten | A written `#[yoake_hook(skip = "...")]` stays as written and is recorded as `"manual": true`; manual ledger items are never overwritten. | Codemod test; real: the two benchmark helpers in `entities::hook_bench` are `"manual": true` after apply. |
| **SF4** key collisions | Always-on disambiguators: impl self-type generic args (`W<u32>`), trait generic args, `[cfg(...)]` from the fn, impl or enclosing mods; then the full trait path; then an error. | Tests for cfg(unix)/cfg(windows) twins and `W<u32>`/`W<u64>`; real: `StandStep::for_hook_bench` became `...for_hook_bench[cfg(feature="yoake-hook-bench")]` through `--rename`, id 57 kept. |
| **SF5** illegal trait conversions | Convert only for struct/enum/union declared in this crate; exclude `&Foo`, tuples, foreign types, and methods with body-changing attributes. | Codemod tests (`&Foo`, tuple, attribute). |
| **SF6** public `register` | `#[doc(hidden)] __register_typed` with the expected TypeId supplied by the hook side and checked in every build; typed `pub(crate) fn __yoake_register_<fn>`; no registration by number. Hookable needs a crate-level path to the self type. | Runtime tests. Real: the 20 `EntityFluidWorld` methods (private struct in `entities::fluids`) are tombstoned and excluded (`self type has no crate-level path`). `Game` is private in `game`; **engine change: `struct Game<'a>` -> `pub(crate) struct Game<'a>` (crates/horizon-client/src/game.rs:2504)** so the 23 `item_use` hooks keep a crate-level path. The release bench example lives outside the crate, so it now registers through `entities::hook_bench::register_noop_tick_settle()` (behind the `yoake-hook-bench` feature). |
| **SF7** a ledger change rebuilt everything | `hook-ledger/meta.json` (global next_id) plus one shard per crate (`horizon_client_lib.json`, `horizon_client.json`). yoake-hook-rt no longer reads the ledger: `SLOT_CAPACITY = 65536` zeroed BSS slots, test ids at the top. Each instrumented crate's build.rs calls `yoake_hook_ledger::build::emit_crate_table(crate)`, which watches only its own shard and writes `crate::__yoake_hook_keys::KEYS`. Adding a fn to one crate rebuilds only that crate. | horizon-client-lib got a build.rs, horizon-client's build.rs one call; both crate roots include `__yoake_hook_keys`. |
| **N1** non-atomic apply | Writes `<file>.yoake-tmp`, then renames all, ledger last; on error nothing is renamed and temps are removed. Claude fixed one bug: rustfmt ran on the temp file and resolved `mod fluids;` next to the `.yoake-tmp` name; rustfmt now formats through stdin. | Codemod test; real apply leaves no `.yoake-tmp`. |
| **N2** test code by name | Test code is detected by `#[cfg(test)]` only; `test_id` in scanned engine modules is a `check` error. | Codemod tests. |

After this round:

| | entities (`horizon_client_lib`) | game::item_use (`horizon_client`) |
| --- | --- | --- |
| hooked (live) | 82 (7 converted trait methods) | 23 |
| tombstoned | 20 (EntityFluidWorld) | 0 |
| excluded | 34: no crate-level path 28, `impl Trait` argument 2, destructuring 1, nested callable borrow 1, manual (benchmark helper) 2 | 0 |

meta: next_id 126. Ids 1..=71 from S2 still name the same functions except the 20 tombstones and the
one cfg rename.

Tests after the round: ledger 20, macro 10, rt 13 (mechanics 12 + callsite 1), codemod 28 (11 unit +
17 integration), all passed. `apply --rustfmt` then `check` clean for both modules, files rustfmt-clean.
`cargo test -p horizon-client --lib`: 1193 passed, 0 failed, 40 ignored (same as before).
`cargo test -p horizon-client-lib --lib`: 1619 passed, 0 failed, 48 ignored (same as before).

Release bench after round 2 (same example, two runs each): empty slots `tick_settle` 9.3-9.6 ns,
one no-op around hook 25.5-26.3 ns; the unhooked leaf `move_applies` was 10.0-10.8 ns, against 6.2 ns in
S2, so the laptop ran about 1.6x slower than during S2 (thermal/power state). These numbers are not
comparable with MX3S2 and do not show a regression or an improvement; a same-session A/B against
yoake2/s2 is needed before drawing conclusions about the typed-registration dispatch.

Still open after round 2: callsite and store entries are not written (see MX3CS on yoake2/callsite);
EntityFluidWorld could become `pub(crate)` if its fluid hooks are wanted; the required-check setting.

## Round 1 results

### Run on two modules

| | entities (crate `horizon_client_lib`, incl. `entities::fluids`) | game::item_use (crate `horizon_client`) |
| --- | --- | --- |
| hooked | 102 (71 kept from S2 with the same ids, 31 new) | 23 (all `impl Game<'_>` with `self_ty`) |
| trait methods converted | 27 | 0 |
| excluded (in the ledger with reasons) | 12: uses trait `Self::` item 5, destructuring argument 3, `impl Trait` argument 2, borrowed return 1, nested callable borrow 1 | 0 |
| call-site candidates counted (not written) | 535 (ambiguous receiver 315), store candidates 66 | 86 (ambiguous 66), store 9 |

Ledger after the run: next_id 126, 125 live, 0 tombstones, 12 excluded, all `kind: fn`.
The S2 ids 1..=71 were migrated first and still name the same functions (checked by script).

## Tests and checks

- `cargo test -p yoake-hook-ledger -p yoake-hook-macro -p yoake-hook-rt -p yoake-hook-codemod`: all passed
  (ledger 12, macro 10, rt mechanics 12 + callsite 1, codemod 8 unit + 10 integration).
  Codemod integration tests: fresh apply + idempotent re-apply/check; a fn added in the middle gets a new
  id and moves nothing; `--rename` keeps the id; a removed fn is an error without `--tombstone-missing`
  and a tombstone with it, and a later fn gets a fresh id; trait conversion and no double conversion;
  generic / `impl Trait` / const fns excluded; `#[cfg(test)]` ignored; bytes outside edits unchanged
  (UTF-8 Japanese comment, CRLF); duplicate key error; sig hash unchanged by reformatting; callsite
  ordinal counting; `apply --rustfmt` then `check` clean.
  Ledger tests: stable ids on add, rename keeps id, tombstone never reused, sig change keeps id, invalid
  ledgers (duplicate id, id 0, id >= next_id, duplicate live key, live and excluded) rejected, kind
  default and round trip.
- Real modules: `apply --rustfmt` then `check` is clean for both modules; the three touched files are
  rustfmt-clean.
- With no hook set, behaviour is unchanged: `cargo test -p horizon-client --lib` 1193 passed, 0 failed,
  40 ignored (identical to before the codemod); `cargo test -p horizon-client-lib --lib` 1619 passed,
  0 failed, 48 ignored. The release `yoake_hook_bench` example builds and runs with the typed registration.

## Fixes found during the rounds

1. Round 1: a hand-written callback type with different binder nesting registered fine and never fired
   (`Snapshot::next` skips other TypeIds). Fixed with typed registration functions plus a debug-build
   TypeId check. Release builds still skip silently if someone bypasses the typed function and calls
   `register` directly: make `register` private to the macro's generated code before 2.0.
2. Round 1: the signature hash included `,` tokens, so rustfmt's trailing commas changed 8 sigs.
   Fixed: the hash input is canonicalised (params and predicates joined without trailing punctuation).
3. Small fixes by Claude: a `format!` brace escape in a codemod test, a generic fixture in the sig test
   (generics are excluded, so it had no ledger entry), the Holder test's closure arity and expected value.

## Codemod design (agy)

### Design Overview

`crates/yoake-hook-codemod` is a span-based Rust codemod binary (`apply` / `check`) backed by `yoake-hook-ledger`. It parses target Rust modules with `syn` (`full`, `visit`, `extra-traits`) and `proc-macro2` (`span-locations`), maps `LineColumn` spans to UTF-8 byte offsets (`edit::LineIndex`), and applies non-overlapping edits from end to start so untouched code (comments, Japanese UTF-8 text, `\r\n` or `\n` line endings, whitespace) remains byte-for-byte unchanged.

Modules:
- `src/main.rs`: CLI parser (`apply` / `check`, `--crate-root`, `--crate-name`, `--ledger`, `--module`, `--rename`, `--tombstone-missing`, `--rustfmt`).
- `src/modules.rs`: Crate root discovery (`Cargo.toml` `[lib] path`, `src/lib.rs`, `[[bin]] path`, `src/main.rs`), `[package] edition` detection (falling back to `2024` when workspace-inherited), and module tree traversal.
- `src/key.rs`: Canonical key formatting (fn, callsite, store), `to_snake_case` trait helper naming, and canonical `syn::Signature` hashing via `Ledger::sig_hash`.
- `src/eligible.rs`: Mirrors `yoake-hook-macro`'s rejection rules plus codemod exclusions (type/const generics, generic impls, trait `Self::` dependencies, unimported trait method calls), producing short stable exclusion reasons.
- `src/attr.rs`: Custom `yoake_hook(...)` and `cfg_attr(..., yoake_hook(...))` parser/formatter supporting `self_ty = <Type>`, file-level import detection (`yoake_hook` vs `yoake_hook_macro::yoake_hook`), and `yoake-hook-baseline` `cfg_attr` wrapper preservation.
- `src/edit.rs`: `LineIndex` (char-column to byte-offset for UTF-8 and `\r\n`/`\n`), reverse-ordered `TextEdit` application, attribute insertion/replacement/removal, and rustfmt-style multi-line trait impl delegation + inherent helper emission.
- `src/run.rs`: Orchestrates `--rename`, `FileVisitor` (`syn::visit_mut::VisitMut`), source-order `assign(Kind::Fn, ...)`/`exclude`, duplicate key detection, already-converted trait helper pairing, missing-key reconciliation (`--tombstone-missing`, scoped to `Kind::Fn`), optional `--rustfmt` pass, and `apply`/`check` output + summary.

### Canonical Keys and Module Paths

1. **Crate prefix:** `--crate-name` with `-` replaced by `_` (e.g. `horizon-client` -> `horizon_client`).
2. **Module tree traversal:** Starts from the crate root file (`src/lib.rs`, `src/main.rs`, or `[lib] path = "..."` in `Cargo.toml`, e.g. `horizon-client-lib` -> `../horizon-client/src/client_lib.rs`). Follows `mod name;` (checking `<dir>/<name>.rs` or `<dir>/<name>/mod.rs`), inline `mod name { ... }`, and `#[path = "..."]` overrides on both inline and external `mod` items.
3. **Test filtering:** Skips `mod tests`, `#[cfg(test)]` modules (including `all(test, ...)` while keeping `not(test)`), `#[cfg(test)] impl` blocks, and `#[cfg(test)] fn` items.
4. **Item keys:**
   - Free fn: `<crate>::<mod>::<fn>`
   - Inherent method: `<crate>::<mod>::<SelfTy>::<fn>` where `<SelfTy>` is the last path segment of the impl's self type with generics/lifetimes dropped (`Game<'_>` -> `Game`).
   - Trait impl method: `<crate>::<mod>::<<SelfTy> as <Trait>>::fn` using the last segment of both self type and trait path.
   - Duplicate keys within a crate fail with both `file:line` locations.
5. **Signature canonicalisation:** `key::canonical_signature_tokens` builds a formatting-invariant string from `syn::Signature` rather than printing raw tokens:
   - Qualifiers (`const`, `async`, `unsafe`, `extern ...`), `fn <ident>`,
   - Generic params (attributes stripped) joined by `, ` with no trailing comma,
   - Inputs joined by `, ` with no trailing comma (`receiver` with attributes stripped, or `pattern: Type`, plus `...` if variadic),
   - Optional `-> ReturnType`,
   - Where-clause predicates joined by `, ` with no trailing comma.
   A `VisitMut` cleanup (`CanonicalSigCleanup`) also strips redundant trailing commas inside nested tuple types/patterns (when `elems.len() != 1`, preserving `(T,)` 1-tuples), bare `fn(...)` types, and generic argument lists before passing to `Ledger::sig_hash`. Multi-line signatures, rustfmt-inserted trailing commas, parameter attributes, and comments all yield the same hash.
6. **Impl lifetimes (`self_ty`):** When an `impl` self type contains a non-`'static` lifetime (such as `Game<'_>`), the codemod emits `self_ty = <impl self type>` inside `#[yoake_hook(method, self_ty = ..., id = ..., key = "...")]`.

### Formatting and Trait-Method Conversion

- **Rustfmt-style emission:** Trait delegations are emitted with multi-line bodies (`{\n    self.__yoake_trait_...(...)\n}`) and 4-space indentation; single-line helper bodies `{ expr }` are likewise expanded into multi-line `{ \n    expr\n}` blocks (empty `{}` and already multi-line blocks are preserved).
- **`--rustfmt` mode:** When `--rustfmt` is passed to `apply`, after writing modified `.rs` files the codemod reads the crate's `[package] edition` from `Cargo.toml` (defaulting to `2024` when `edition.workspace = true` or missing) and runs `rustfmt --edition <edition>` on the changed files. If `rustfmt` is missing from `PATH`, it prints a warning and continues without failing. Because attribute matching (`attr::matches_expected`), trait delegation detection, and `signature_hash` are all AST/canonical-token based, running `check` after `apply --rustfmt` is clean.
- Eligible trait impl methods are rewritten to a single delegating call (`self.__yoake_trait_<snake_trait>_<method>(...)`, or `Self::...` for associated functions without `self`), stripping `mut` from delegating parameter bindings so no `unused_mut` warning is produced.
- An inherent `impl` block with the trait impl's generics, `self_ty`, `where` clause, and `#[cfg]`/`#[allow]` attributes is inserted immediately after the trait impl (or updated in-place if a helper `impl` already exists). Each helper carries `#[yoake_hook(method, ...)]`, `#[inline(always)]`, copied `#[cfg]`/`#[allow]` attributes, the original signature (including `mut` parameter bindings), and the original body slice.
- Idempotency: on subsequent runs, a trait method whose body is a single delegating call to its expected `__yoake_trait_*` helper in an inherent `impl` on the same type is paired with that helper. The helper is not assigned a second key, and the signature hash is computed from the helper's signature (with the original method name restored) so `mut` parameter stripping does not cause false signature changes.

### `VisitMut` Pass Structure and Call-Site / Store Candidates

- `run::FileVisitor` implements `syn::visit_mut::VisitMut` with separate fn-level and body-level methods:
  - Fn-level: `visit_file_mut`, `visit_item_mod_mut`, `visit_item_trait_mut` (no-op), `visit_item_fn_mut`, `visit_item_impl_mut`, `visit_impl_item_fn_mut`.
  - Body-level: `visit_expr_call_mut`, `visit_expr_method_call_mut`, `visit_expr_assign_mut`.
- Nested items inside function bodies (`fn`, `impl`, `mod`) are skipped whenever `current_caller` is active.
- Paired converted trait methods do not scan their one-line delegating stub; instead, the matched `__yoake_trait_*` helper body is visited under the trait method's canonical key so candidate counts and ordinals stay identical before and after trait conversion.
- Body-level visitors traverse expressions in left-to-right source order (`receiver` before `.method` before `args`; `left` before `right`) and maintain per-caller 0-based ordinal counters:
  - `ExprCall`: `<caller>@call:<callee path>#<ordinal>` (path segments without turbofish; non-path calls such as field calls count as `?::<member>` / `?::<expr>` and are flagged ambiguous).
  - `ExprMethodCall`: `<caller>@call:?::<method>#<ordinal>`, flagged ambiguous because receiver types are unknown to `syn`.
  - `ExprAssign`: `<caller>@store:<field path>#<ordinal>` for field writes (`self.velocity`, `self.pos.x`, etc.); local variable assignments are ignored.
- Candidate counts are reported in the summary as `callsite_candidates=N (ambiguous=M), store_candidates=K`. Nothing body-level is written to source files or the ledger yet; fn hooks pass `Kind::Fn` to `Ledger::assign`.
- **Conversion limits:**
  - Generic trait impls (`type` or `const` params, bounded lifetimes, or lifetime-dependent `where` clauses) are excluded rather than converted.
  - Trait methods whose signature or body references `Self::Item` for any associated `type`, `const`, or `fn` declared in the trait impl are excluded (`uses trait Self:: item`).
  - Trait methods whose body calls `self.<other_trait_method>(...)` when the trait is not imported into module scope (multi-segment trait path without a matching `use`) are excluded (`calls trait method without trait in scope`).
  - Associated items inherited from a trait's default definitions (not listed inside the `impl` block) are not visible to AST-only scanning; if a body references `Self::DefaultAssocType`, `rustc` will catch it at build time.

### General Codemod Limits

- **Macro-generated functions:** `syn::parse_file` inspects unexpanded syntax, so `fn` items generated by `macro_rules!` or proc macros are not seen or instrumented.
- **`include!` files:** External files pulled in via `include!("...")` are not treated as Rust `mod`s and are not traversed.
- **`#[cfg(...)]` duplicate items:** Platform- or feature-mutually-exclusive items with the same canonical key (e.g., `#[cfg(unix)] fn f()` and `#[cfg(windows)] fn f()`) collide under the key rule and are rejected as duplicates with both locations reported.
- **Nested functions and trait definitions:** Functions nested inside another function body and default bodies inside `trait` definitions are intentionally not instrumented.
- **Unresolved type aliases:** Type aliases hiding non-`'static` lifetimes (e.g. `type Alias<'a> = &'a str` used through a non-lifetime alias `type MyRef = ...`) cannot be detected without full type resolution; `rustc` + `yoake-hook-macro` remain the final check.

## Ledger, runtime and macro (Kimi)

### Built

- Added `yoake-hook-ledger` with the specified API, serde model, deterministic two-space JSON,
  monotonic assignment, rename history, tombstones, exclusions, validation, and FNV-1a signature
  hashes. Unit tests cover ID stability and non-reuse, rename/tombstone behavior, signature changes,
  validation failures, canonical JSON round trips, exclusion replacement/re-admission, and hashing.
- Added `yoake-hook-rt/build.rs`. It validates the ledger beside `Cargo.toml`, emits its rerun
  dependency, and generates `MAX_SLOTS`, `TEST_ID_BASE`, `TEST_SLOTS`, and `LEDGER_KEYS` in `OUT_DIR`.
  The runtime array now includes the 64 test slots, registration accepts that full range, and
  `assert_ledger_key` performs byte-wise const validation with distinct range/tombstone/mismatch
  diagnostics.
- Extended the macro with `id` + `key`, `test_id`, and optional `self_ty`. Engine hooks emit the
  ledger assertion; test hooks use `TEST_ID_BASE + K`, reject `K >= 64`, and have no key check.
  Existing exclusions and macro validations remain in place, with new argument unit tests.
- Converted runtime tests and `inline_probe` to test IDs, removed the old source/lock-file scanner,
  and added a hooked `impl Holder<'_>` execution test. Updated the runtime README.

### Lifetime finding

S2 already generated higher-ranked callback pointers for borrowed function arguments, including
shared named argument lifetimes. It did **not** handle a lifetime introduced by the surrounding
`impl`, because `Self = Game<'x>` or `Holder<'x>` in the callback type prevents the `Any`-erased
pointer from being `'static`.

The `self_ty = Holder<'_>` option substitutes the supplied self type for `Self` in callback inputs
and owned results. Self-type lifetimes are normalized to fresh names and then rebound in the
callback, original-closure, and call-helper types. The original-body call is fully qualified with
the rebound self type.

### Round 2: type mismatch, kinds, and callsites

Round 1 testing showed that the Holder hook did not fire: its manually written callback type had a
different binder nesting from the macro's exact type, and `Snapshot::next` silently skipped it.
Every hook now emits `__yoake_register_<fn>(callback, order)` with the exact callback type. Free
functions also emit `__YoakeHook_<fn>` when all generic parameters occur in that type. A method
attribute cannot emit a module-level alias because its output remains inside the inherent impl,
while inherent associated type aliases are unstable; methods therefore expose the associated typed
registration function with the exact type inline. Tests, `inline_probe`, and the benchmark use these
functions. Debug registration also records one `TypeId` per slot and rejects a second, different
callback type instead of allowing a silent no-op.

Ledger entries and exclusions now have `Kind::{Fn, Callsite, Store}`. Missing JSON kinds deserialize
as `fn`, serialization always includes the kind, `assign` takes a kind, and live keys remain unique
across kinds. `exclude_kind` supports future kinds while the existing `exclude` defaults to `fn`.
Tests cover legacy defaults, kind round trips, and cross-kind duplicate rejection.

The runtime's `callsite!` proof performs one slot load and either invokes the callee directly or
runs an around chain with a tuple of arguments. Its test verifies an empty slot, wrapping, argument
replacement, and skipping, while a second call to the same callee remains unchanged. The proof
currently supports up to four `'static` arguments and a `'static` return; generalized borrowed
callsite arguments and codemod emission remain future work.

## Open problems

- `register` is still public; only the typed `__yoake_register_<fn>` is safe in release builds.
- Call-site and store hooks: the ledger kinds and the codemod's body-level `VisitMut` methods are in place
  and count candidates; 59-77 % of the call-site candidates (315 of 535, 66 of 86) are method calls whose receiver type syn cannot know (`?::method`), so
  callsite keys for method calls need type information (rustdoc JSON from C1, or rust-analyzer) or an
  explicit selector chosen by the mod author.
- The `callsite!` proof supports up to four `'static` arguments and a `'static` return.
- The 5 trait methods that use trait-only `Self::` items, generic impls, and fns generated by macros or
  `include!` stay unhookable.
- Keys use the module where the impl block is, not where the type is defined
  (`horizon_client::game::item_use::Game::...`); moving an impl block to another file is a rename.
- A CI job should run `yoake-hook-codemod check` for every instrumented module.

## Spec

The shared spec both AIs implemented against is reproduced here for review.

### yoake2/ids: one hook-ID ledger + codemod (shared spec for Kimi and agy)

Root: C:\Users\suke0\YoakeBuild\wt-y2ids (git worktree of the Horizon engine, branch yoake2/ids, based on
yoake2/s2 @1321f14e9). Read docs/yoake2/reports/MX3S2.md and crates/yoake-hook-rt/README.md first:
the S2 runtime (global SLOTS, around hooks, `#[yoake_hook(id = N)]`) is already here.

Neither AI can run shell commands. Claude builds, runs the codemod, tests, and appends results to the
TASK files. Two AIs work in this folder at the same time, so each owns separate files (see TASK-*.md).

### 1. Canonical key (one per hookable fn)

`<crate>::<module path>::<item>` with Rust-style `::` separators, no spaces, no generics/lifetimes:
- crate = the crate name with `-` turned into `_` (e.g. `horizon_client`);
- module path = the module tree from the crate root (`src/lib.rs` or `src/main.rs`), following `mod`
  declarations, including in-file `mod x { }` blocks. It is NOT the file path (a `#[path]` attribute
  is followed), and cfg attributes are ignored (an item under `#[cfg(...)]` gets its key anyway);
- free fn: `horizon_client::entities::move_applies`;
- inherent method: `horizon_client::entities::ArmorStand::tick_settle`
  (the self type's last path segment, generics and lifetimes dropped: `Game<'_>` -> `Game`);
- trait impl method: `horizon_client::entities::<ArmorStand as EntityRules>::tick`
  (trait path's last segment, generics dropped).
Two fns with the same key in one crate are an error (report both locations).

Signature hash `sig`: FNV-1a 64-bit over the fn signature tokens (`syn::Signature` printed with
`quote::ToTokens`, whitespace collapsed to single spaces, attributes excluded), lowercase 16 hex digits.

### 2. Ledger file: crates/yoake-hook-rt/hook-ids.lock.json (the single engine ledger)

```json
{
  "version": 1,
  "next_id": 72,
  "entries": [
    { "id": 1, "key": "horizon_client::entities::move_applies", "sig": "0123456789abcdef", "status": "live" },
    { "id": 2, "key": "horizon_client::entities::old_name", "sig": "...", "status": "tombstone" },
    { "id": 3, "key": "horizon_client::entities::new_name", "sig": "...", "status": "live",
      "renamed_from": ["horizon_client::entities::older_name"] }
  ],
  "excluded": [
    { "key": "horizon_client::entities::<StandClimbing as ClimbingWorld>::block", "reason": "..." }
  ]
}
```
Rules:
- ids start at 1; 0 is never used. `next_id` only grows. ids are never reused (a tombstone keeps its id).
- entries sorted by id, excluded sorted by key; pretty JSON, 2-space indent, `\n` line endings,
  trailing newline. Serialization is deterministic so running tools twice gives no diff.
- valid ledger: every id unique, 1 <= id < next_id, live keys unique, no key both live and excluded.
- a rename keeps the id: the entry's key changes and the old key is appended to `renamed_from`.
- a signature change keeps the id and updates `sig` (the tool reports it).
- MAX_SLOTS = next_id (slots 0..next_id-1). There is no fixed 4096 any more. Tests get a reserved
  range on top: `TEST_SLOTS = 64`, `TEST_ID_BASE = next_id`, array length = next_id + 64.

### 3. Crate yoake-hook-ledger (new, Kimi) API the codemod uses (agy codes against exactly this)

```rust
pub struct Ledger { /* version, next_id, entries, excluded */ }
pub struct Entry { pub id: u32, pub key: String, pub sig: String, pub status: Status, pub renamed_from: Vec<String> }
pub enum Status { Live, Tombstone }
pub struct Excluded { pub key: String, pub reason: String }
pub struct Assigned { pub id: u32, pub new: bool, pub sig_changed: bool }
pub enum LedgerError { /* Io, Json, Invalid(String) ... */ }  // impl Display + std::error::Error

impl Ledger {
    pub fn new() -> Ledger;                                   // version 1, next_id 1
    pub fn load(path: &std::path::Path) -> Result<Ledger, LedgerError>;   // parses + validate()
    pub fn save(&self, path: &std::path::Path) -> Result<(), LedgerError>; // to_json() bytes
    pub fn from_json(text: &str) -> Result<Ledger, LedgerError>;          // parses + validate()
    pub fn to_json(&self) -> String;                          // deterministic, see rules
    pub fn validate(&self) -> Result<(), LedgerError>;
    pub fn assign(&mut self, key: &str, sig: &str) -> Assigned; // existing live key: same id; else next_id++
    pub fn rename(&mut self, old_key: &str, new_key: &str) -> Result<u32, LedgerError>;
    pub fn tombstone(&mut self, key: &str) -> Result<u32, LedgerError>;
    pub fn exclude(&mut self, key: &str, reason: &str) -> Result<(), LedgerError>; // error if key is live; replaces reason
    pub fn unexclude(&mut self, key: &str);
    pub fn live_id(&self, key: &str) -> Option<u32>;
    pub fn live(&self) -> impl Iterator<Item = &Entry>;
    pub fn excluded(&self) -> &[Excluded];
    pub fn max_slots(&self) -> u32;                           // == next_id
    pub fn sig_hash(signature_tokens: &str) -> String;        // FNV-1a 64, whitespace collapsed first
}
```
Dependencies allowed (all in Cargo.lock, offline): serde (derive), serde_json. Nothing else.

### 4. Attribute form (written by the codemod, checked by the macro)

`#[yoake_hook(id = 17, key = "horizon_client::entities::ArmorStand::tick_settle")]`
with `method,` first inside impl blocks: `#[yoake_hook(method, id = 17, key = "...")]`.
entities.rs wraps it as `#[cfg_attr(not(feature = "yoake-hook-baseline"), yoake_hook(...))]`; keep that
wrapper when it is already there (the S2 bench uses it); new attributes in other modules are plain.
Tests inside yoake-hook-rt and yoake-hook-macro use `#[yoake_hook(test_id = K)]` (K < 64) which maps to
`::yoake_hook_rt::TEST_ID_BASE + K` and has no key check.

Compile-time check: yoake-hook-rt's build.rs loads the ledger, fails the build if it is invalid, and writes
OUT_DIR/ledger.rs with `pub const MAX_SLOTS: usize`, `pub const TEST_ID_BASE: usize`,
`pub const TEST_SLOTS: usize = 64` and `pub const LEDGER_KEYS: [&str; MAX_SLOTS]` ("" for 0 and for
tombstones). The macro emits `const _: () = ::yoake_hook_rt::assert_ledger_key(17, "key");`, a const fn
that panics at compile time unless `LEDGER_KEYS[17] == key`. So an attribute whose id/key pair is not in
the ledger, including a duplicate id on a second fn with another key, does not compile.

### 5. Codemod: crates/yoake-hook-codemod (new bin, agy)

`yoake-hook-codemod <apply|check> --crate-root <dir containing Cargo.toml> --crate-name <name>
 --ledger <path> --module <rust module path, e.g. entities or game::item_use> [--module ...]
 [--rename OLD_KEY=NEW_KEY]... [--tombstone-missing]`
- parses files with syn (full, visit) and edits the TEXT by byte ranges from proc-macro2 spans
  (`span-locations` feature), never re-printing whole files, so formatting is kept;
- for every fn in the given modules (free fns, inherent methods, trait impl methods; NOT fns inside
  `#[cfg(test)]` modules or `mod tests`, NOT nested fns inside fn bodies, NOT trait definitions):
  - eligible -> ensure exactly one `yoake_hook` attribute with the ledger id and key (adds, or fixes a
    wrong id/key, keeps a cfg_attr wrapper), `ledger.assign(key, sig)`;
  - not eligible -> no attribute (remove an old `yoake_hook(...)`/`skip` one) and `ledger.exclude(key, reason)`.
    Ineligible: generic type params or const generics, `impl Trait` in args or return, async, const,
    unsafe, extern ABI, variadic, by-value/typed self, never `!` return, borrowed return types,
    `#[track_caller]`, destructuring argument patterns, and anything else the macro rejects
    (read crates/yoake-hook-macro/src/lib.rs `expand` for the full list and mirror it);
- trait impl methods: move the body into an inherent helper and delegate:
  ```rust
  impl EntityRules for ArmorStand {
      fn tick(&mut self, world: &World) -> bool { self.__yoake_trait_entity_rules_tick(world) }
  }
  impl ArmorStand {
      #[yoake_hook(method, id = 12, key = "horizon_client::entities::<ArmorStand as EntityRules>::tick")]
      #[inline(always)]
      fn __yoake_trait_entity_rules_tick(&mut self, world: &World) -> bool { /* original body */ }
  }
  ```
  The helper impl goes right after the trait impl block. Keep the trait method's attributes on the trait
  method. Use `Self` paths unchanged (Self is the same type in both impls). Copy the impl's generics and
  where clause to the helper impl. If the impl is generic or the body cannot move (uses trait-only items
  such as associated types or consts of the trait via `Self::`), exclude with a reason instead.
  Idempotent: recognise an already-converted method (body is exactly one call to its helper) and
  do not convert again; just check the helper's attribute.
- `check`: does everything in memory and exits 1 with a list of what would change (files, ledger),
  exit 0 when nothing would change. CI runs `check` (that is the idempotency check).
- `--rename OLD=NEW`: before scanning, `ledger.rename(OLD, NEW)`.
- missing ledger keys in the scanned modules (live keys under a scanned module prefix not found in the
  sources): with `--tombstone-missing` -> `ledger.tombstone`; without it -> error listing them (so a
  rename is never silently a delete + add).
- prints a summary: hooked, newly assigned, excluded by reason, trait methods converted, sig changes.
Dependencies allowed (in Cargo.lock): syn 2 (features full, visit, extra-traits), quote, proc-macro2
(span-locations), serde_json, and the path dependency yoake-hook-ledger.

### 6. Run targets

`horizon-client` modules `entities` (crates/horizon-client/src/entities.rs, currently ids 1..=71 in the
S2 format) and `game::item_use` (crates/horizon-client/src/game/item_use.rs, `impl Game<'_>` methods).
Claude migrates the S2 lock file to the new ledger by running the codemod (existing ids 1..=71 keep their
numbers: Claude seeds the ledger from the S2 lock file in id order before the first run).

### 7. Addendum (2026-11-18, from the owner via Yoake): call-site and store hooks later

2.0 will also get call-site hooks (Java `@Redirect` / `@Inject` at an INVOKE: wrap, replace or cancel ONE
call inside ONE function) and later store hooks on field writes. Only fn hooks are implemented now, but the
ledger and codemod must support the others from the start:
- every ledger entry and excluded item has `"kind": "fn" | "callsite" | "store"` (serde default `fn` when
  missing; serialized always). `Ledger::assign` becomes `assign(kind, key, sig)`; keys are unique per kind
  (the same string may not be live twice even across kinds; keep it simple: keys unique overall).
- callsite key: `<caller canonical key>@call:<callee canonical path>#<ordinal>`, where the ordinal is the
  0-based index of that callee among calls to the same callee in the caller body in source order
  (e.g. `horizon_client::entities::ArmorStand::tick_settle@call:horizon_core::geometry::Vec3::add_vector#0`).
  Method calls whose receiver type is unknown to syn use `?::method` as the callee path
  (`...@call:?::add_vector#1`); the codemod reports these as ambiguous.
- store key: `<caller canonical key>@store:<field path>#<ordinal>` (for example `@store:self.velocity#0`).
- stability rules are identical: append-only ids, rename entries, tombstones, no reuse.
- LEDGER_KEYS covers every kind (one id space, one SLOTS array).
- codemod structure: one `syn::visit_mut::VisitMut` pass per fn body is the place where call expressions
  and assignments will be rewritten later. Keep fn-level work and body-level work as separate visitor
  methods (`visit_item_fn_mut` / `visit_impl_item_fn_mut` vs `visit_expr_call_mut` / `visit_expr_method_call_mut`
  / `visit_expr_assign_mut`), even though the body-level ones only count calls for now (record callsite
  candidates in the summary, do not write them to the ledger yet).
- proof test (cheap, runtime + macro only, no codemod): in yoake-hook-rt tests, one call-site slot in a
  test fn, written by hand the way the codemod will emit it later:
  `let y = ::yoake_hook_rt::callsite!(test_id = K, callee(x, 2))` or an equivalent macro/fn form: the empty
  slot calls `callee(x, 2)` unchanged; with an around callback registered, the callback receives
  `original` and the args and can wrap, replace or skip the single call; other calls to `callee` in the
  same fn are not affected.
