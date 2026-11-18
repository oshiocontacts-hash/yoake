# MX3MERGE2: yoake2/merge, round 2 (launcher work merged, bridge fixes S1–S4, loader wiring, read-only arguments)

Date: 2026-11-19. Branch `yoake2/merge` after MX3MERGE (@39fb8fcbe, reviewed by Fable: merge OK, bridge OK for test
use). Built and tested on LENOVO, Windows, MSVC, **rustc 1.100.0** for everything (the bindgen gate pins 1.99.1 and
was not run, see Results), `-j 4`, own `CARGO_TARGET_DIR`. Generator work by agy (Gemini 4 Hydrogen), runtime and
SDK work by Kimi (K3.1), the shared bridge module, loader wiring, end-to-end tests, CI and review by Claude.

## 1. Merges

`yoake2/slice` @2d9b00366 (MX3L3 control files, MX3L4 launcher authority / `--full-access-approved`), @6b51435b9
(MX3U1 launcher backend, MX3U2 Mods page) and @10672e931 (MX3U2 fix-ups after Fable's review): all without
conflicts. `yoake2_mods.rs` gained only the bridge wiring below.

## 2. Review findings

### S1: `install` was not idempotent (fixed)

Every registration of a generated dispatcher goes through `yoake_slice::bridge::install_once(id, ..)`: a process-wide
map from hook id to the first outcome. A second call for the same id (another thread, a second mod, a second world,
a reload) returns that outcome and adds nothing, so a guest runs once per call. The priority order of guests stays
with the runtime chain; the dispatcher's own slot order is the priority of the first registration.

### S2: `yoake2-slice` turned on `yoake-hook-bench` (fixed)

`yoake2-slice` no longer includes `yoake-hook-bench`, so the typed `hook_bench::register_*` helpers are closed again
in slice builds. The slice tests that register directly (`yoake2_slice`, `yoake2_engine_call`) and the ones that need
the bench constructor `StandStep::for_hook_bench` (`yoake2_glue`, `yoake2_bridge`, `yoake2_loader`) name
`yoake-hook-bench` in their `required-features`; horizon-client forwards the feature. CI (`yoake-bindgen.yml`) passes
both features.

### S3: `install` mixed "not mine" with "not bridgeable" (fixed)

Each generated `hook_bridge` now has `BRIDGED` (sorted), `mode(id) -> Option<Mode>` and
`install(registered) -> InstallReport { installed, not_mine, not_bridgeable, failed }` (`yoake_slice::bridge`).
An id of another module or outside the ledger range is `not_mine`; an id of this module that cannot be bridged is
`not_bridgeable` with the `UNBRIDGED` reason; a registration the hook slot refused is `failed` with its error.

### S4: observe-only was invisible to the guest (fixed: the mode is returned at registration)

The loader sets a process-wide mode resolver (`yoake_slice::bridge::set_mode_resolver`, from the bridges' `mode`)
before any `yoake_init` runs. `hook_register` still registers, and returns `0` (Full: the guest's value is the result,
not calling `original` cancels), `1` (`OBSERVE_ONLY`: cannot replace or cancel; if no guest calls `original` it runs
once afterwards) or `2` (`NOT_CALLABLE`: this engine build has no bridge for the id; it will never run). Negative
statuses are unchanged (not registered). The guest SDK's `Registrar::hook` returns
`Result<HookMode, RegisterError>`. Without a resolver (a host without bridges) the status is `0` as before.
ENGINE_CALL_ABI.md, "Hook modes and results".

### Scalar results are range-checked (new)

The guest's `i64` is decoded with `yoake_slice::bridge::decode`: `bool` only `0`/`1`; integers must fit the type
(`try_from`); `u64` is the bit pattern; `f32` must be in `0..=u32::MAX` (its bits); `f64` is the bit pattern. Out of
range: the original's result is used (the last one if the guest called `original`, else the original runs now); the
event is counted (`bridge::out_of_range_count`) and printed once per hook id. Previously `as` truncated silently.

## 3. Loader wiring

`yoake2_mods::load_world_mods` sets the mode resolver, loads the approved mods, then `install_bridges(&loaded)`: for
each loaded mod, its registered ids go to every bridge module (entities, `game::item_use`); an id is unbridged for that
mod when no module installed it, with the reason (`not_bridgeable` / `failed`, or "no hook with this id in this engine
build"). `LoadReport::unbridged` holds `(mod id, [(hook id, reason)])` and `messages()` prints one line per mod.

## 4. End-to-end tests (all through the generated bridge, real engine functions, no game resources)

`crates/horizon-client/tests/yoake2_bridge.rs` (WAT guests unless noted):
- SDK guest on fn hook 71 and observe-only call site 215 (from MX3MERGE), now through `InstallReport`;
- **original not called**: a Full hook (71, `ArmorStand::tick_settle`) is cancelled: the stand keeps its velocity;
- **original called twice**: the settle step runs twice;
- **trap before original**: the mod is disabled, the original still runs once, the next call does not enter it;
- **trap after original**: the original's result is kept and it does not run again;
- **install twice**: no second dispatcher, the guest is entered once per call;
- **the three report buckets** and `mode` (71 Full, 215 observe-only, 1 none, 103 another module's), `BRIDGED` and
  `OBSERVE_ONLY` sorted;
- **a scalar result replaced and out of range** (hook 58, `ArmorStand::name_with_tag -> bool`, `false` for a stone):
  the guest's `1` makes it `true`; `5` without `original` runs the original (`false`); `-1` after `original` keeps
  `false` without a second run; both are counted.

`crates/horizon-client/tests/yoake2_loader.rs`, `a_loaded_mod_that_registers_a_bridged_hook_id_is_called_by_the_engine`:
a launcher-installed, approved `.yak` registering 71 and 215 (entities), 103 (`game::item_use`: **install from two
modules**) and 1 (not bridgeable). Over two world starts: `hook_register` returned `0, 2, 0, 1` for 71, 1, 103, 215;
the report names only `1 (arg0 is not &mut T)`; a real `tick_settle` enters the guest exactly once through the bridge.

## 5. Read-only hook arguments (`&T`)

Runtime part done (Kimi, reviewed and tested here); the bridges do not use it yet.

- `runtime::dispatch_id_ref<T>(hook_id, arg: &T, table: &'static RefFieldTable<T>, orig: &mut dyn FnMut(&T) -> i64)`:
  same chain, re-entry, original-call recording and trap rule as `dispatch_id`.
- `activation::RefFieldTable<T> { get: fn(&T, u32) -> Option<i64> }`: a separate read-only table. `FieldTable` is
  unchanged, so the generated glue and the bindgen regeneration gate are untouched. (A first version changed
  `FieldTable::get` to take a wrapper that panicked on mutable access through a shared argument; it broke the existing
  tables and was replaced.)
- The activation stores `ArgPtr::Mut(*mut ())` or `ArgPtr::Shared(*const ())`. A shared slot is read only through
  `get_ref_impl`, which makes `&T` and nothing else; `set` returns the new `Status::ReadOnly` (7; `engine_call`
  status -23), as do `with_arg`, `open_path` and `with_path`. No shared pointer is ever turned into a mutable one;
  the unsafe-function list in activation.rs names `get_ref_impl` and `run_cell_ref`.
- Tests: an activation unit test (get, set refused, typed access refused, original result recorded) and
  `crates/yoake-slice/tests/readonly.rs` with real WAT guests (get, set refused with the value unchanged, original
  called / not called, a chain of two mods, nested re-entry, traps before and after `original`).

Next for the 86 `&T` / `&self` hooks: the bindgen generator emits a `RefFieldTable` next to each `FieldTable` (only
the getters that read fields), and the bridge generator emits `dispatch_id_ref` dispatchers for `&T` arg0 with such a
table. Not started: it changes the bindgen output, so the regeneration gate (rustc 1.99.1) has to be re-run with it.

## 6. THIR CI job

`.github/workflows/yoake-callsite-verify.yml`:
- free-space precheck: the drive with the most free space among `RUNNER_TEMP`, `D:\`, `C:\` gets the dumps; under
  16 GiB the job fails at once with the numbers (the tool refuses below 10 GiB and kills rustc below 5 GiB);
- per-module filter: on a push or pull request only the call-site modules whose files or crate ledger shard changed
  are dumped and verified; everything when the hook tools, the shared ledger files, `Cargo.lock`, a toolchain file or
  the workflow change, when there is no usable base, by hand, and weekly (new `schedule`, Monday 03:17 UTC), which
  also covers a change elsewhere in a crate that alters what a shim's call resolves to;
- not possible: filtering the dump itself. rustc builds the whole crate's `-Zunpretty=thir-tree` as one string and
  writes it at once (on Windows a pipe fails with os error 1450 for that write), so the raw file is always the whole
  crate. Shrinking it needs a small rustc driver (`rustc_private`, pinned toolchain) that prints THIR only for the
  bodies of the call-site modules: worth it once the toolchain is pinned.

## 7. Results (rustc 1.100.0)

| suite | result |
| --- | --- |
| hook crates (ledger, macro, rt, codemod, thir, callsite-match), feature off | 128 passed |
| `horizon-client --lib` | off 1193 passed / 40 ignored; `yoake2-slice` 1194 / 42 |
| `horizon-client-lib --lib` | off and on: 1619 passed / 48 ignored |
| horizon-mods (control files) | 44 passed |
| yoake-wasm | 124 passed |
| horizon-launcher (own workspace, `--manifest-path launcher/Cargo.toml`) | 597 lib + 170 bin + 9 integration passed, 8 ignored (built on D:, `-j 1`: with `-j 4` the linker ran out of memory, LNK1102) |
| `codemod check --callsites`, `bridge --check` (entities, game::item_use) | clean |
| yoake2_glue / engine_call / slice / bridge (`yoake2-slice,yoake-hook-bench`) | 28 passed, 1 ignored |
| yoake2_loader (`yoake2-slice,yoake-hook-bench`) | 17 passed |
| yoake-slice (`wasm`; incl. new `modes`, `readonly`, `bridge` unit tests) | 69 passed |
| yoake-guest-sdk | 11 passed, 1 ignored |
| yoake-bindgen mini_engine | 8 passed |
| bindgen regeneration gate (`--ignored`) | not run: needs rustc 1.99.1, found 1.100.0 (nothing installed) |
| verify-callsites | not re-run: the call-site modules, the ledger and the THIR tools are unchanged since MX3MERGE (entities 58/58, item_use 41/41 there); the same rule as the CI module filter |

The first run of the new bridge tests failed twice through a test bug (the expected values were computed after the
guest was installed, so they went through the hook too); fixed by computing them first.

## 8. Notes

- Toolchain: not touched, as asked (one `rust-toolchain.toml` at 1.100.0 is pending the owner's decision).
- Disk on LENOVO: C: is down to about 5 GB free (two target folders of about 29 GB each). Only build units unused
  for 12 hours were removed (the owner's rule); the launcher was built on D:. Further large builds here go to D:.
- `yoake-guest-sdk` tests build their guests in one temp folder; on Windows a parallel run once failed with "failed to
  remove ...rcgu.o: access denied" and passed on the re-run (a file lock, not the code).

## 9. Fable's review of 3828f998b: fixes

**M1 (must-fix): the server tick was reported NOT_CALLABLE.** `yoake2_mods::ENGINE_DISPATCHED` lists the synthetic
ids the engine dispatches itself (`runtime::HOOK_SERVER_TICK`); `hook_mode` returns `Full` for them and
`install_bridges` leaves them out (no "will never run" line). Test `yoake2_loader::
a_loaded_mod_on_the_server_tick_is_told_full_is_not_unbridged_and_runs_on_tick`: a launcher-installed, approved mod
gets `0` from `hook_register(HOOK_SERVER_TICK)`, the report is empty, and it is entered on each tick (driven as
`Game::yoake2_server_tick` does, `dispatch_root_id(HOOK_SERVER_TICK, ..)`, on a stand-in root: a real `Game` needs the
game resources).

**S-a.** `observe_only_runs_the_original_exactly_once_whatever_the_guest_does`: call site 215 through original not
called, called twice, trap before, trap after, and a returned value (ignored): the item is hurt exactly once each
time. `two_mods_on_one_bridged_id_share_one_dispatcher_and_both_run_once`: two mods on 71 (priorities 10 and 20),
installed per mod (the second install reuses the dispatcher), each entered once, the original once; and with the outer
mod not calling `original`, the inner mod and the engine do not run.

**S-b.** Observe-only now runs the real original at most once per call: a later `original` that reaches it returns
`0` without running it (generated `if result.is_none() { result = Some(original(..)); }`). ENGINE_CALL_ABI.md and
HOOK-SLICE-BRIDGE.md say so.

**S-c.** Documented: an out-of-range fallback that runs the original directly skips the rest of the chain.

**S-d.** Documented: a shared argument cannot be used with `engine_call` (every binding takes `&mut` through
`with_arg`); a future `with_arg_ref` must hand out only `&T`, only to `&self` bindings. Test
`readonly::engine_call_on_a_shared_argument_is_refused_read_only_and_the_binding_does_not_run` (`-23`, the value
unchanged, not a trap).

**Nits.**
- Disjoint ids: `yoake2_mods::tests::bridge_modules_claim_disjoint_ids` checks that the two bridges' `BRIDGED` never
  overlap and that no bridge claims an engine-dispatched id (`install_once` keys by id alone).
- Full but the install failed: the guest was told `0`; the loader reports the id with the slot's error in
  `LoadReport::unbridged` and a message line.
- Pass-through: a value equal to the encoding of the original's own result is accepted and not counted (a `usize`
  above `i64::MAX`), generated `Some(r) if encode(r) == value => r`.
- activation.rs: the shared slot's getter now lives in `ArgPtr::Shared(ptr, get)`, so there is no unreachable
  `ok_or`; the `run_cell_ref` SAFETY comment says that only `&A` is made. New unit test: a shared handle is `Locked`
  while `original` runs and `Stale` after the scope.
