# FR-F1-FINAL: Stamped release client build fix (field tables + call-site patch), 2026-11-26

Branch `yoake-free/f1-final` (from `yoake-free/merge` `83673ff389`).

## What was stale and why

The stamped release build (`tools/yoake-stamped-cargo.sh build --release -j 6 -p horizon-client --bin horizon-client --features yoake2-slice`) failed in `horizon-client-lib` with 16 errors:

1. **Stale field tables (`yoake-bindgen --tables`).** `crates/horizon-client/src/yoake_glue/tables/horizon_client_lib/mobs__custom__script.rs` (lines 390 and 401) still read `t.id` on `crate::mobs::custom::script::Session`. Commit `ed78530627` (REM-09 F4M-10 deferred) moved `id` into `SessionInner` (`Session` now holds `inner: Arc<SessionInner>`) after the tables were generated in `af19fc7caa`. Other structs across `horizon_client_lib`, `horizon_client`, `horizon_world`, and `horizon_command` had also gained or shifted fields in subsequent commits (e.g. `4c0741f546`).
2. **Drifted call-site patch (`hook-ledger/horizon_client_lib.callsites.patch`).** Edits in `crates/horizon-client/src/merchant.rs` (around line 1186) and `crates/horizon-client/src/lan/hub.rs` (around line 2877) caused their patch hunks to no longer fit. Because `apply_patch_text` previously applied fitting hunks even when another hunk in the same file failed, the call replacements in `merchant.rs` were applied while the `__yoake_cs_<id>` shim block at the end of the file was skipped, producing 14 `E0425` errors (`cannot find function __yoake_cs_23190`, ...). In addition, refactoring `ServerEntity::pairing` -> `pairing_with` (`lan/entity.rs`) and `entity_type` -> `resolve_entity_type` (`lan/hub.rs`) left 5 live callsite entries in `hook-ledger/horizon_client_lib.json` (`22292..22294`, `22331..22332`) whose original caller no longer contained those calls.
3. **`sync-ledger` dropping callsite exclusions.** In `crates/yoake-hook-codemod/src/run.rs`, `reconcile_missing_keys` filtered `missing_live` by `Kind::Fn` but pruned `stale_excluded` without checking `ex.kind == Kind::Fn`. Because `run::execute` only populates `seen_keys` with function keys when `callsites` is false, `sync-ledger` removed all non-manual `Kind::Callsite` exclusions in scanned modules.

## What was fixed and regenerated

1. **Codemod safety & `sync-ledger` (`e1499c51ad`):**
   - `crates/yoake-hook-codemod/src/run.rs`: restricted `stale_excluded` in `reconcile_missing_keys` to `ex.kind == Kind::Fn`, keeping non-manual `Kind::Callsite` and `Kind::Store` exclusions it does not own. Added unit test `reconcile_keeps_non_manual_callsite_and_store_exclusions_while_dropping_stale_fn_exclusions`.
   - `crates/yoake-hook-codemod/src/callsite_patch.rs`: changed `apply_patch_text` to buffer hunks per file in memory and skip all hunks for a file (leaving it untouched on disk) if any hunk in that file no longer fits, while other files in the patch still apply. Updated `a_broken_hunk_skips_its_file_and_other_files_still_apply` and added `when_any_hunk_for_a_file_does_not_fit_all_hunks_for_that_file_are_skipped`.
2. **Field table regeneration (`53070759d6`):**
   - `af19fc7caa` produced its rustdoc JSON inputs via `crates/yoake-bindgen/regen.sh`: `RUSTC_BOOTSTRAP=1 CARGO_INCREMENTAL=0 RUSTDOCFLAGS=`, running `cargo rustdoc --offline -j 4 -p "$krate" --lib -- -Z unstable-options --output-format json --document-private-items` across all 9 crates (`horizon-client-lib`, `horizon-client`, `horizon-items`, `horizon-core`, `horizon-world`, `horizon-client-core`, `horizon-client-render`, `horizon-protocol`, `horizon-command`), then running `yoake-bindgen --config crates/yoake-bindgen/bindgen.json`.
   - Ran `regen.sh` with `--tables horizon_client_lib --tables-dir crates/horizon-client/src/yoake_glue/tables/horizon_client_lib`, then ran `target\release\yoake-bindgen.exe --config crates/yoake-bindgen/bindgen.json --tables <CRATE> --tables-dir <DIR>` for the remaining 7 stamped crates.
   - Updated 9 table files across `horizon_client_lib` (`lan__entity.rs`, `mobs__custom__render_state.rs`, `mobs__custom__script.rs`), `horizon_client` (`entry__wasm_mods__command_arguments.rs`, `game__native_server.rs`, `game__yoake2_custom_mobs.rs`, `yak_mods.rs`), `horizon_command` (`vanilla.rs`), and `horizon_world` (`fluid__custom.rs`).
3. **Call-site patch CLI & `horizon_client_lib` patch regeneration (`5a81ee6f8e`, `3c01475ea3`):**
   - `crates/yoake-hook-codemod/src/callsite_patch.rs`, `crates/yoake-hook-codemod/src/main.rs`: added `--rename-site` and `--tombstone-site` flags to `callsite-patch`, consumed on round 1 of `generate_and_verify_with`.
   - Dumped THIR for `horizon-client-lib` with `yoake-hook-thir dump --crate-pkg horizon-client-lib --crate-name horizon_client_lib --workspace . --out target/hcl.calls.json --files-from target/hcl-files.txt --allow-shared-target` using the `crates/horizon-client-lib/../horizon-client/src/...` paths matching rustc's `[lib] path` spans (`48,854,143` bytes).
   - Ran `yoake-hook-codemod callsite-patch --workspace-root . --crate horizon_client_lib --thir target/hcl.calls.json` with `--tombstone-site` for the 5 moved callsite keys (`fn:2138@call:horizon_client_lib::lan::entity::Riding::passengers`, `set_passengers`, `set_passengers#1`, `fn:2157@call:horizon_protocol::registry::Builtin::get`, `Ids::id`) and `--verify --stamp-target C:\Users\USER\NestgateHorizon\horizon-free\target-stamped`.
   - Verified clean in 1 round with 0 withdrawals (`3,798` live callsites, `34,070` excluded; `129` files, `1,586` hunks in `hook-ledger/horizon_client_lib.callsites.patch`; `hook-ledger/meta.json` `next_id` `24677 -> 24678`).

## Verification

| Check | Result |
| --- | --- |
| `.\hz-cargo.cmd test -j 6 -p yoake-hook-codemod -p yoake-callsite-match -p yoake-hook-ledger` | Pass: `yoake-callsite-match` 11; `yoake-hook-codemod` 46 unit, 5 `bridge`, 21 `callsite`, 21 `codemod_tests`, 1 `overlay_e2e` (+ 2 ignored slow), 4 `shallow`; `yoake-hook-ledger` 36 |
| `.\hz-cargo.cmd check -j 6 -p horizon-client --features yoake2-slice` | Pass (exit 0) |
| `.\hz-bash.cmd tools/yoake-stamped-cargo.sh build --release -j 6 -p horizon-client --bin horizon-client --features yoake2-slice` | Pass (exit 0 in 76m 00s), 0 `UNSTAMPED` lines, 0 `yoake-stamp` warnings; `horizon-client.exe` `321,225,595` bytes |
| `target-tools\release\yoake-hook-codemod.exe check-history --base target\old-hook-ledger --current hook-ledger` | `check-history: ok` |

## What is left

- `horizon_client` still has no `.callsites.patch` (keeps its in-place `game::item_use` sites; left from FR-REM11b).
- Re-running `callsite-patch` on the 4 earlier crates (`horizon_command`, `horizon_core`, `horizon_items`, `horizon_protocol`) with the newer generator fixes (left from FR-REM11b).
- `callsite-bench-gate`, `yoake-bindgen` full `--check` (for non-table glue / `ids.lock.json`), and game / benchmark runs were not run in this task.
