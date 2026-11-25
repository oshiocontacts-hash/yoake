# REM-17d lane YAK: dependency resolution during package loading

## What changed

- `yak::resolve()` now uses manifest discovery order to break ties between packages that are ready to load. A set with no dependencies therefore keeps its previous order instead of being sorted by id.
- Sandboxed YAK startup discovery resolves the combined local and host-synced package set after manifest/build checks. Compatible packages are reordered into dependency-first resolution order.
- Missing dependencies, version mismatches, conflicts, duplicates and cycles produce the existing startup warning and consumable mod toast. The packages directly involved are skipped; required dependents of a skipped package are skipped transitively. Healthy packages still load. Both members of a conflict are refused.
- Empty discovery returns before resolution, and an independent set retains the same packages and order as before.
- Install needed no new code: `launcher/src/yoake_mods/plan/resolver.rs` already calls `yak::resolve()` for candidate install sets, and `plan/transaction.rs` uses it when enabling, disabling and removing packages. Invalid installs are already refused through `Error::Resolution`.

## Ids, versions and WIT

No ABI ids, interface versions, WIT items, generated adapters or stubs changed. No regeneration is needed for this lane.

## Files

- `crates/horizon-mods/src/yak/compatibility.rs`: stable discovery-order topological resolution.
- `crates/horizon-client/src/yak_mods.rs`: startup resolution, selective refusal, notices and resolved load order.
- `crates/horizon-client/src/yak_mods/tests.rs`: host-side dependency fixtures and tests.
- `docs/yoake2/reports/FR-REM17d.md`: this report.
- `.lane/TASK.md`: findings and verification commands.

## Tests

Added host tests for a missing dependency with a message, an incompatible dependency version, dependency-before-dependent ordering, and healthy-package survival. The existing `list_comments_and_unsigned_content_preserve_load_order` test pins unchanged no-dependency order. Existing resolver tests cover conflicts, duplicates, cycles and absent optional dependencies.

Kimi did not run builds or tests because this lane forbids shell commands. Commands for Claude are in `.lane/TASK.md`.

## Left / boundaries

- Full-access packages remain on the separate approved-loader path in `yoake2_mods.rs`; the launcher intentionally splits them out before sandboxed resolution. This lane does not resolve dependencies among full-access packages or between full-access and sandboxed packages. Supporting that requires a follow-up that resolves the approved local and host-scoped full-access sets before any module executes.
- `GAP3*` was not present under `docs/` in this worktree; `GAP2.md` records dependency enforcement as unverified.
