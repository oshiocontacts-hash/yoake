# MX3MERGE: yoake2/merge (yoake2/callsite + yoake2/slice) and the ledger -> runtime bridge

Date: 2026-11-19. Branch `yoake2/merge`: `yoake2/callsite` @eae792d4f, then `yoake2/slice` @b6bd2bec3 (MX3L1) and
@8d635e946 (MX3L2) merged in. Built and tested on LENOVO (Windows, MSVC stable 1.100.0, `-j 4`) in its own
`CARGO_TARGET_DIR` (see docs/yoake2/BUILDING-WORKTREES.md). Not touched beyond the merge: `yoake2_mods.rs`,
`yak_mods.rs`, `crates/horizon-mods`.

## Merge

Textual conflicts, all resolved by keeping both sides:
- workspace `Cargo.toml` members (hook ledger/codemod/thir/callsite-match + yoake-slice/bindgen/guest-sdk);
- `crates/horizon-client/Cargo.toml` features (`yoake-callsite-baseline` + `yoake2-slice`);
- `crates/horizon-client/src/entities.rs` tail: the `yoake2-slice` glue `include!` first, then `hook_bench` and the
  codemod's call-site shim block (the codemod manages only its own block);
- `Cargo.lock`: yoake2/slice's lock plus the hook crates (offline).
MX3L2 merged on top without conflicts.

Semantic conflict: the slice tests (`yoake2_slice.rs`, `yoake2_engine_call.rs`) called the public
`yoake_hook_rt::register`, which yoake2/ids round 2 (SF6) made the hidden `__register_typed` with crate-private typed
registration. They now register through feature-gated helpers in `entities::hook_bench` (same behaviour); the bench
example already did.

## The bridge (docs/yoake2/HOOK-SLICE-BRIDGE.md, now implemented)

`yoake-hook-codemod bridge --crate-root .. --crate-name .. --ledger hook-ledger --module .. --glue ..` generates, per
module, `src/yoake_glue/hook_bridge_<module>.rs`, `include!`d inside that module under
`#[cfg(feature = "yoake2-slice")]` (so it reaches private child modules' `pub(crate)` registration fns):
- a dispatcher per bridgeable ledger id (fn or call site): arg0 `&mut T` with a generated `FieldTable` goes to
  `runtime::dispatch_id(id, arg0, &FIELDS_.., orig)`, a `&mut Game` arg0 to `dispatch_root_id`; other params are passed
  to `original` unchanged inside the closure; `()`, integers, `bool`, `f32`/`f64` results round-trip through `i64`;
- **observe-only** for other result types (`OBSERVE_ONLY`): the guest sees the hook and may call `original`, but cannot
  replace the result; if it never calls `original`, the original runs once afterwards (no cancel in this mode);
- `pub fn install(registered: &[(u32, i32)]) -> Vec<u32>`: installs the dispatcher of each id a guest registered
  (`runtime::stats().registered`, ids below `SYNTHETIC_HOOK_BASE`) through the typed registration with the guest's
  priority, and returns the ids it could not bridge;
- `UNBRIDGED`: every other live id with its reason; `hook-ledger/bridge-exclude.txt` can add more by hand;
- `bridge --check` fails on a stale or missing bridge file.

Coverage now: entities 26 bridged (114 unbridged), item_use 29 bridged (35 unbridged). Most unbridged hooks take
`&T`/`&self` as arg0 (the runtime's `dispatch_id` needs `&mut T`), or have no FieldTable for arg0.

## End to end (crates/horizon-client/tests/yoake2_bridge.rs, feature `yoake2-slice`)

A Rust guest built with `yoake-guest-sdk` (crates/yoake-guest-sdk/tests/guests/bridge.rs) exports `yoake_hook` and
`yoake_init` and registers **fn hook 71** (`ArmorStand::tick_settle`) and **call-site hook 215** (the
`ItemEntity::hurt` call inside `ItemEntity::environment_tick`, observe-only). The test installs it,
calls `entities::hook_bridge::install(&runtime::stats().registered)` (nothing unbridged), runs a real `ArmorStand`
settle step and a real `ItemEntity::environment_tick` in an empty world (no game resources), and checks: each handler
ran exactly once, both originals ran (velocity as without the guest; the item lost exactly one health point), no
unregistered hook was entered, the guest was entered twice in total, and it was not disabled.

## Results on yoake2/merge

| suite | feature off | feature `yoake2-slice` |
| --- | --- | --- |
| `horizon-client --lib` | 1193 passed, 40 ignored | 1194 passed, 42 ignored |
| `horizon-client-lib --lib` | 1619 passed, 48 ignored | 1619 passed, 48 ignored |
| yoake2_glue / engine_call / slice / bridge tests | - | 21 passed, 1 ignored |
| yoake2_loader | - | 12 passed |
| yoake-slice (`--features wasm`) | 59 passed | |
| yoake-guest-sdk | 10 passed, 1 ignored | |
| yoake-bindgen mini_engine | 8 passed | |
| hook crates (ledger, macro, rt, codemod, thir, callsite-match) | 127 passed | |
| `codemod check --callsites`, `bridge --check` | clean (both modules) | |
| verify-callsites after a THIR dump | entities 58 of 58, item_use 41 of 41 | |
| bindgen regeneration gate | not run: it requires rustc 1.99.1 (crates/yoake-bindgen/toolchain.txt); LENOVO has 1.100.0 and nothing was installed | |

Fixes made during the merge:
- `crates/yoake-bindgen/tests/mini_engine.rs` assumed `libyoake_slice-*.rlib` next to the test binary (classic
  `deps/` layout). This cargo uses a per-unit layout (`<profile>/build/<crate>/<hash>/out`); the test now finds the
  rlib there too and passes the `-L` directories through a rustc `@argfile` (too many for a Windows command line).
- A codemod bridge test checked `(5, ` and matched `dispatch_id(5, ..`; it now checks the UNBRIDGED tuple form.
- `callsite-hot-callers.txt` keys of another crate are ignored silently.

## Notes for the coordinator

- **Two pinned toolchains:** the bindgen gate pins rustc 1.99.1, while the call-site THIR tooling was built and
  verified with 1.100.0 (THIR output is toolchain-specific). One pinned toolchain (a `rust-toolchain.toml`) should
  cover both before CI relies on either.
- **THIR dump size:** with the fn hooks and call-site shims expanded, a whole-crate dump of horizon_client_lib no longer
  fits next to LENOVO's build directories (the raw THIR tree passed 3.8 GB and the 5 GB watchdog stopped it with 13 GB free); it was written to another
  drive. CI runners need the free-disk check of the verify job, and a per-module THIR filter (or `-Zdump-mir` with a
  filter) would make it much smaller.
- Loader wiring: nothing calls the bridge's `install` outside the test yet. The loader (MX3L2's `yoake2_mods.rs`) should
  call `horizon_client_lib::entities::hook_bridge::install` and `horizon_client::game::item_use::hook_bridge::install`
  after installing the guests; left for you, since that file is being reviewed.
