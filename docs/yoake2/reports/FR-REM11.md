# FR-REM11: F1 completion (drift, call-site hooks, setters, Copy self), 2026-11-25

Branch `yoake-free/rem11-callsites` (from `yoake-free/merge` f7516323c). Machine: GALLERIA, shared with other sessions. No game was started.

## Part 0: drift (done)

* **Ledger.** `yoake-hook-codemod sync-ledger` and `apply --verify --overlay` now take `--rename OLD=NEW` and `--tombstone KEY`
  (`overlay.rs`, `verify.rs`, `main.rs`; each crate applies only the keys of its own prefix), so a moved or removed key is fixed through the
  overlay route: no source rewrite, no JSON hand edit. Test: `overlay_e2e::sync_ledger_renames_and_tombstones_moved_keys_without_touching_the_tree` (fast).
  * `horizon_client::game::Options::read` renamed to `horizon_client::game::launch_options::Options::read` (id 4997 kept).
  * `horizon_client::game::agent::low_point` tombstoned (id 12153).
  * `horizon_client_lib::mod_generator::<ModTerrain as ChunkGenerator>::{worker,proto,finish}` (12046..12048) **tombstoned, not renamed**: the new key
    `<ModTerrain<G> as ChunkGenerator>::...` is a generic impl whose parameter is not `'static`, so the codemod refuses it
    ("live in ledger but now ineligible (generic impl)"). Nothing referenced those ids.
  * New ids: the FR-REM17c native fns (`native_draw_model`, `native_item_model_select_v2`, `native_input_key_v2`, `native_input_pad_button`,
    `native_input_pad_axes`), the unassigned mobs fns and 150 other new fns (`next_id` 12210 -> 12361). `check-history` against the previous ledger: ok.
    The saved shards are re-sorted by the ledger writer (manual exclusions were appended at the end before), so the diff is larger than the change.
* **yoake-bindgen.** `crates/yoake-bindgen/regen.sh` run in full (nine rustdoc JSONs, then the generator). `mod_generator` loses `ModTerrain` (its ids stay
  in `ids.lock.json` as `unbound`) and gains `ModChunk::set_biome_at` and `ModProto.failed`; items/mobs/motion/entities rows move. `ids.lock.json` is
  append-only against HEAD (`--lock-base`, 2090 base ids), `yoake-bindgen --check` is clean.
* **Tests.**
  * `yoake2_loader::a_loaded_mod_that_registers_a_bridged_hook_id_is_called_by_the_engine`: id 1 became bridged with the n-ary form (W9), the test's
    "unbridged" example is now id 211 (a borrowed return, still unbridged). 43 pass.
  * `yoake-guest-sdk::rustc_and_wasm_ld_retain_the_sdk_abi_section`: `ENGINE_CALL_ABI.md` is right (engine 1.1; the Rust SDK declares 1.0 and an older
    minor loads, `AbiCheck::OkOlderMinor`). The test now says that. (The SDK bump to 1.1 is REM-19.)
* **Verification.** See "Checks" below.

## Part 1: call-site (INVOKE) hooks across the stamped crates (infrastructure done, no crate stamped)

What exists now (commits 1a, 1b):

* `yoake-hook-codemod callsite-patch --workspace-root . --crate <name> --thir <calls.json>`: runs the normal call-site `apply` on a scratch copy of the crate
  (sites go to the real ledger shard, the tree is untouched) and stores the difference as `hook-ledger/<crate>.callsites.patch`
  (`callsite_patch.rs`: own Myers line differ, LF-only hunks with context, CRLF-safe patcher that finds a hunk near its recorded line).
* The stamping overlay applies that patch before the fn stamps (`build_overlay`); a hunk that no longer fits is skipped with a `yoake-stamp:` warning
  (its site is not hooked in that build). The overlay cache key already hashes the whole ledger directory. No patch file: nothing changes.
* Tests: 4 unit tests (roundtrip, shifted file, broken hunk, CRLF) and `callsite::callsite_patch_replays_the_rewrite_in_the_overlay_without_touching_the_tree`
  (fixture: THIR file, callsite-patch, tree untouched, overlay contains the shim and the fn stamps, a broken hunk is reported and skipped).
* `yoake-hook-thir dump` fix: cargo prints `--check-cfg "cfg(feature, values(\"a\", \"b\"))"` and the tokenizer split on the escaped quotes (unit test).
* Design text: `CALLSITE-PLAN.md` section 8.

**What a real run showed (horizon-command, the smallest crate).** THIR dump 5.2 MB; `callsite-patch` gave 1,604 live sites in 15 files (138 hunks, a 771 KB patch).
A stamped `cargo check` of the crate then reported 2,189 errors: 1,675 of them are only my check command (`-p horizon-command --features yoake2-slice` does not
enable `yoake-slice/wasm`, so `yoake_slice::runtime` is missing; the real game build enables it), and **514 are real shim errors** in four classes:
E0521 (457, `a0: &str` passed to a `'static` parameter: the shim must keep the callee's lifetime), E0061 (26) and E0404 (26, free-function path calls such as
`partial(x)` that were rewritten like methods: `<String as crate::component>::partial(recv)`), E0502 (5, two-phase borrow). So about 32 % of the sites are rejected by
the compiler. The design (CALLSITE-PLAN section 1) says these are found by "rewrite, compile, exclude, repeat", and that loop does not exist for the patch route yet.
I therefore did **not** commit the horizon-command patch or its 1,604 ledger entries (the ledger and patch were reverted; the THIR/patch recipe is in the plan).

**Left for part 1 (in this order):**
1. A verify loop for `callsite-patch` (like `apply --verify --overlay`): stamped build with `yoake2-slice,yoake-hook-bench`-style features that enable `yoake-slice/wasm`,
   map each error to its `__yoake_cs_<id>` (the call line or the enclosing shim in the block), revert that site in the scratch copy, tombstone the entry and record a manual
   exclusion, repeat until clean. (`verify.rs` already has the build runner, the JSON error parser and the exclusion file; only `find_hook_attrs` skips `kind = callsite`.)
2. Fix the shim generator for free-function path calls (E0404/E0061) and for borrowed `&str`/`&T` arguments of `'static` parameters (E0521), so the loop excludes few sites.
3. A policy for how many sites per caller are worth having (command-tree builders such as `vanilla.rs` give hundreds per fn) and the measured hot-caller list (needs a hit-count run; the hit-count files of REM-03 are not in the tree).
4. Then run it for the 8 crates (THIR scratch 4 to 7 GB per large crate; the tool refuses below 10 GB free), commit one patch per crate, build level verify
   (`apply --verify --overlay --verify-arg build` equivalent), and the coordinator's benchmark.

## Part 2: setters in the field tables (done)

`yoake-bindgen --tables` now writes a setter arm for every scalar field except (a) `forbidden_writable` (segments `id`, `uuid`, `len`, `length`, `index`, `idx`, `seq`,
`sequence`, tuple field `.0`), (b) `TABLE_INVARIANT_SEGMENTS` (`hash`, `checksum`, `crc`, `generation`, `capacity`, `version`, `nonce`, `secret`, `password`, `token`, `salt`, `cipher`, `handle`),
(c) `table_readonly` of `bindgen.json` (`Type.field`, `Type.*`; empty for now), and types in files with `unsafe` (`frozen_files`, no table at all). Shared (`&T`) arguments
have only the `RefFieldTable`. All eight table trees were regenerated (rustdoc JSON of the regen): 773 files, 41 of them new modules. Table fields have no lock ids,
so `ids.lock.json` did not change for this. Tests: policy unit tests, `mini_engine::field_table_setters_follow_the_policy`, the core `Vec3` and client `Options`
table tests now check a write instead of a refusal.

## Part 3: by-value `Copy` self (done)

`classify_nary` accepts a by-value (`self`, `mut self`) receiver whose type the module derives `Copy` for (the macro side was done in W12); a by-value `Copy` argument
with a `RefFieldTable` gets a shared view of the engine's copy (`NaryKind::CopyShared`: the guest can read fields, `original` gets a copy back), without a table it is an
opaque handle. A non-`Copy` by-value self stays unbridged (`by-value self`). Test: `bridge::bridge_nary_takes_a_by_value_copy_self_and_copy_arguments`.

## Checks

Env: `CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0`, target `horizon-rem11	arget`, mingw on PATH, `--offline -j 6`.

| Check | Result |
|---|---|
| `tools/yoake-stamped-cargo.sh build --release -p horizon-client --bin horizon-client --features yoake2-slice` (cache deleted first) | OK, 76 min, **0 UNSTAMPED lines**, `horizon-client.exe` 268 MB (all eight crates stamped, with the regenerated tables including setters) |
| `test -p yoake-hook-codemod -p yoake-hook-ledger -p yoake-bindgen -p yoake-slice` | all pass: codemod unit 39, bridge 5, callsite 16, codemod_tests 21, overlay_e2e 1 (+2 ignored slow), shallow 4; ledger 36; slice 32 + doc test; bindgen lib 5, policy 3 (see note below for `mini_engine`) |
| `test -p horizon-client --features "yoake2-slice,yoake-hook-bench" --test yoake2_loader --test yoake2_examples --test yoake2_template` | loader 43 pass (the known failing one included), examples 2, template 2 |
| `test -p yoake-hook-thir -p yoake-hook-macro -p yoake-hook-rt -p yoake-guest-sdk` | all pass (guest-sdk ABI test included) |
| `cargo build -p horizon-client-lib -p horizon-client --lib` (the CI `build-hooked-crates` job) | OK |
| `yoake-bindgen --config crates/yoake-bindgen/bindgen.json --check` (same rustdoc JSON as the regen) | clean; `--lock-base` HEAD ledger: append-only |
| `yoake-hook-codemod check-history` old ledger -> new | ok |
| CI `check` per `modules.txt`: `entities`, `game::item_use` | clean |
| CI `check` per `modules.txt`: **`mobs` and `motion` (shallow)** | **fail, not fixed**: see below |

`mini_engine`: 7 of 9 pass in a target that also built `yoake-slice` with the `wasm` feature; `generated_glue_compiles_and_every_binding_works` and
`glue_module_forbids_unsafe_code` then fail at the link step (`newest_rlib` picks the wasm-feature rlib; documented in FR-REM04, not caused by this work). In a clean target (the first run of this work) all 9 passed.

**Open for CI `check` of `mobs` and `motion` (in-place modules).** These two lines of `modules.txt` want the stamps in the source files themselves. At the start of this work
`check` already failed for them: `mobs.rs` has 3 functions without a hook attribute (they now have ids from `sync-ledger`; `check` reports `newly_assigned=3`, before the sync they
were unassigned), `motion.rs` wants 2 trait-method conversions (`trait_methods_converted=2`). The fix is the in-place `apply --shallow ... --module mobs` and `--module motion`
(two source files). I did not run it: the brief said not to rewrite sources in place, and the coordinator should decide (or drop the two lines, since the overlay stamps those modules anyway).

Not run: the callsite-verify CI job (needs THIR dumps of the whole crates, 16 GB free), the benchmark, any game run.
