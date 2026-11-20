# FR-MERGE1: integration of L1, S1, L2b and F2 into `yoake-free/merge`

Base: `160c39a83` (FR-L2 sprites + FR-F3-0). Merged in this order, no push.

| Step | Branch | Merge commit |
|---|---|---|
| 1 | `vps/yoake-free/l1-ime` (365c2cf7f, contains L2 already; no conflicts) | see `git log` "Merge FR-L1" |
| 2 | `yoake-free/s1` | "Merge FR-S1" (419e23a41) |
| 3 | `vps/yoake-free/l2b` (3f7dd242c) | "Merge FR-L2b" (1cac1bb3b) |
| 4 | `yoake-free/f2` (995aee4f4) | "Merge FR-F2" (2dcbc5a25) |

## Conflicts and resolution

* S1: `abi-subset-client.json` (kept 201-202, 211-216 and 221-225; `interface_version` set to 1.5.0),
  `horizon-client/.../client_services.rs`, `yoake-wasm/src/client_services.rs`, `yoake-wasm-sdk/src/full.rs`
  (both sides' functions kept), `docs/guide-ja/README.md`, `docs/yoake-wasm-dev.md` (merged text, ids 221-225 and 226 added).
  Generated files (`yoake-full.wit`, `coverage.json`, `full_adapters.rs`, `full_client_generated.rs`) took the L1 side and were regenerated.
* L2b: `docs/yoake-wasm-dev.md` only (kept the L2b sentences and the merged id list). No catalog change.
* F2: `docs/guide-ja/README.md`, `docs/yoake2/MOD-GUIDE.md` (kept both; F2's chapter renumbered 15, IME stays 14).
* Guides: S1 `09-small-services.md` -> `11-small-services.md` (ja and en, heading and READMEs fixed);
  F2 `09-files-network.md` -> `12-files-network.md`. ja guide order: 09 sprites, 10 ime, 11 small services, 12 files/network.
* `@API:...@` tags (`fr_s1_services`, `key_pre`, `ime`, ...) stay unstamped: the repo stamps them on main with `tools/stamp_native_hooks.py`, as L1/L2 left theirs.
* Post-merge fix (S1 bug exposed by the merge): `native_key_pre` sat after the `Server-level helpers` marker in
  `native_hooks.rs`, so `catalog_matches_hook_points` did not see it. Moved next to `native_key` (pure move).

## Regenerated

`YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib` after the S1 merge: `yoake-full.wit`, `coverage.json`,
`full_adapters.rs`, `full_adapters_report.txt`, `full_client_generated.rs`, `full_hooks_generated.rs`, `full_hook_names.rs`
(hook ordinals now include `key-pre`). Re-run after the F2 merge and the 1.6.0 change: no further diff.

## Versions and ids

| Block | Version | Ids / notes |
|---|---|---|
| L2 sprites | client 1.3.0 | 211-216 |
| L1 IME | client 1.4.0 | 201-202 |
| S1 services | client 1.5.0 | client 221-225, server 226 (server catalog stays 1.1.0), hook `key-pre` |
| F2 worker WIT | `yoake:mods@1.6.0` | package, `worker`, `worker-host`, `worker-run`, `worker-user` (was 1.3.0 on F2's branch; changed in the generator `wit_inject.rs`, `yoake-mod-io`, `yoake-wasm`, example `f2-io`, MOD-GUIDE) |

All ids are unique; `abi_1_0_is_additive` passes. 1.6.0 is not an entry in a catalog JSON (the worker WIT is a separate package), so the catalog `interface_version` stays 1.5.0.

## Tests

Run in `horizon-free` only (single `target/`; the SDK crate with `CARGO_TARGET_DIR=target/sdk`). C: filled up twice during the run, which
produced spurious failures (package_services x3, std_io, yoake-slice link, one `a_guest_cancels_a_layer...` panic); every
affected suite was rerun after freeing space and passed. Flaky otherwise: none observed.

| Suite | Result |
|---|---|
| `horizon-mod-api-gen` (lib) | 46 passed (incl. `abi_1_0_is_additive`, WIT/adapter match) |
| `yoake-mod-io` | 5 passed |
| `yoake-wasm` (all binaries, incl. `f2_io` 11, `package_services` 7) | 67 + 6 + 2 + 11 + 5 + 1 + 9 + 7 + 16 + 2 + 3 + 5 passed |
| `yoake-guest-sdk` (incl. `std_io` 4) | 5 + 8 + 4 passed |
| `yoake-slice` | 28 + 1 passed |
| `horizon-mods --features yak` | 78 + 42 passed |
| `horizon-client --lib` (full, after the `native_key_pre` move) | 1222 passed, 40 ignored |
| `horizon-client-lib --lib` | 1639 passed, 48 ignored |
| `horizon-client-render --lib` (incl. L2b's 2 new HUD tests) | 1454 passed, 40 ignored |
| `yoake-wasm-sdk` (standalone crate) tests | 10 + 1 passed; `cargo check --target wasm32-wasip2 --features full` ok |
| `cargo check -p horizon-client --features yoake2-slice` | ok |
| `cargo check -p yoake -p yoake-test-runner -p cargo-horizon-mod` | ok |

Not run: the game, any Minecraft-dependent test.
