# MX3L1: full-access mod loader and several mods at once

Branch `yoake2/slice` on top of c852d7ad8 (MX3K1), working tree. No git writes, no installs, no game launch. Everything is behind `yoake2-slice` or in `yoake-slice` / `yoake-guest-sdk` / `yoake-bindgen`; feature-off builds are unchanged. ABI notes: `docs/yoake2/ENGINE_CALL_ABI.md` (new section "Registration rules, several mods, the loader").

## Progress log (kept as work landed)

- [x] 1a `hook_register` only during `yoake_init`, cap 4096, finite init budget
- [x] 1b id 0 reserved for the legacy API
- [x] 1c guest allocator alignment and checked math
- [x] 1d SDK docs on re-entry
- [x] 1e tests: nested trap, `memory.grow` during nesting, align 64
- [x] 2 loader (manifest, policy, allow file, world-start call site)
- [x] 3 several mods per thread, priority chains, per-mod isolation and stats
- [x] 4 tests incl. the real `Game` with several mods
- [x] gate and baseline (see the test table)

## 1. Review should-fixes

- **`hook_register` is init-only.** `Host.in_init` is set only around the `yoake_init` call; elsewhere it returns `-1` (`register_status::NOT_IN_INIT`). At most `MAX_REGISTRATIONS = 4096` ids per mod (`-2`). `yoake_init` runs on a finite budget: `INIT_FUEL` = 50 million (fuel is now always on, so even an unmetered mod has one) and `INIT_EPOCH_TICKS` = 2000 when epoch is on; a guest that spins there fails `load` with `yoake_init failed: ...` and nothing stays loaded. Tests: from a hook (`-1`), id 0, the cap (5000 asked, 4096 granted, 904 refused), the spinning init.
- **Id 0 is reserved** for the legacy single-export guest (`-3` for a multiplexed guest); a legacy guest is registered as hook 0. S2 slot ids start at 1 (the ledger's first id is 1). Documented in `ENGINE_CALL_ABI.md`.
- **Guest allocator** (`yoake-guest-sdk/src/heap.rs`): every block of a size class is carved aligned to `min(size, 64 KiB)` and keeps that alignment on the free list; the class is at least the requested alignment, alignment above 64 KiB is refused (null). `start + size` is checked (`checked_add`), a failure is null, not a wrapped pointer. Test guest allocates `align(64)` boxes over three rounds with frees in between, a `Vec` of `align(256)` that reallocates 40 times, and an `align(4096)` box: no misalignment.
- **SDK docs**: a "Re-entry: never hold a borrow across `original`" section (RefCell guard, `static mut` reference: the nested activation of the same guest finds it taken, panics, and a panic is a trap).
- **New tests** (Rust guests, `guest_runtime.rs`, 7 now): the outer hook continues after a nested hook traps (its data intact, the engine body of each function ran exactly once, the mod is disabled and never entered again); `memory.grow` during nesting (the nested hook allocates and touches 3 MiB while the outer hook waits in `original`, memory grew by 48+ pages, the outer vector is intact and it allocates afterwards); align 64/256/4096.

## 2. The loader

- **Declaring a full-access mod** (no new table: the launcher builds `yak::Manifest` literals in its tests, a new field would break them): `[wasm] interface = "1.0"`, `server = "bin/wasm/<name>.wasm"`, `world = "full-access"`. `horizon-mods` validates it: server module only (no `client`), no `[wasm] permissions` (it is all-or-nothing), `side.server = true`. `Archive::full_access_module()` reads the module (bounded by `MAX_WASM_SIZE`). Any other manifest is unchanged and is the sandboxed path.
- **`crates/horizon-client/src/yoake2_mods.rs`** (feature `yoake2-slice`):
  - `Origin { LocalFolder, Catalog, Server, Friend }` and the policy `decide`: a local mod needs its switch (a signature does not replace it); a catalog mod needs a signer on the trust list or the switch; **a server's or a friend's package is never full-access, with or without a switch** (checked before the signature).
  - The record of the switches is `<mods folder>/full-access.toml` (`[allow]`, `<mod id> = true`): `AllowFile` parses, sets and saves it; a missing file allows nothing. The launcher UI is for later.
  - Signatures reuse `horizon-mods`' `Archive::verify_signature` against the compiled-in trust list (`trusted_keys()`); for a catalog package a signature that does not verify refuses it (`BadSignature`).
  - `load_package` / `load_dir` (files in name order, so load order is stable) and `load_world_mods(<game root>/mods)`, which replaces the thread's mods of a previous world. A non-full-access package is reported as `Refusal::Sandboxed` and left alone.
- **World start**: `native_lifecycle_world_ready` calls `self.yoake2_world_start_hook()` (no `cfg` in that file, a source test forbids it). With the feature, an integrated-server or dedicated world installs the `engine_call` table and loads the allowed mods of `<game root>/mods`; without it the method is empty. Client-side loading is not done (server first, as asked). `yoake2_world_start_hook` itself is not exercised by a test (it needs an installed game root); the tests call `load_dir` / `load_package` directly.

## 3. Several mods per thread

The runtime (`yoake-slice/src/runtime.rs`) keeps a list of mods per thread; each is its own `Store` with its own published-context cell (`active`), its own hook registrations and its own tickets. `ModId`, `load(wasm, cfg, name)`, `unload(id)`, `stats_of(id)`, `all_stats()`, `take_reports_of(id)`; the old `install` (replace everything with one mod), `stats()` and `take_reports()` keep their single-mod meaning, so the 21 older runtime tests and every engine test are unchanged.

- **Chains**: for one hook id the mods registered for it run by `(priority, load order)`, lowest first. The first is the outermost wrapper; its `original` runs the next mod; the last one's `original` runs the engine. The composition is the shim's own `original` closure: each `chain_step` wraps the rest of the chain as the activation's `original`, so locking, tickets, handles and the trap rule are the per-activation machinery that already existed. `dispatch_id`, `dispatch_root_id` and `dispatch2` are all chained.
- **Isolation**: a trap or engine panic disables only that mod (`Host::disable` also sets a shared flag the chain builder reads without borrowing the instance). A trapped mod that had not called `original` is replaced by "run the rest of the chain", so the other mods and the engine still run once; the next call builds a chain without it.
- **Re-entry across mods**: a mod's nested activation uses its own store's published context; another mod's store is borrowed fresh; a hook that fires again inside `original` finds the right store's context. Handles never cross mods.
- **Stats per mod** (entries, disabled, trap text, registered hooks, fuel) and per-mod reports.
- Cost note: building a chain allocates a small `Vec` when at least one mod is loaded; with no mods the hook entry is one thread-local read and an empty check.

## Tests

| Suite | Result |
|---|---|
| yoake-slice (`--features wasm`): unit 25, **`multi` 6 (new)**, runtime 21, doctest 1 | all passed |
| yoake-guest-sdk: unit 3, `guest_runtime` 7 (3 new), doctest ignored | all passed |
| horizon-mods | 44 passed (manifest change) |
| yoake-bindgen `mini_engine` 8, `policy` 3 | all passed |
| horizon-client-lib `yoake2_engine_call` 8, `yoake2_glue` 5, `yoake2_slice` 7 (+1 ignored bench) | all passed |
| **horizon-client `yoake2_loader` 7 (new)** | all passed |
| **real `Game`, feature on** (`yoake2_game_tests`, 2, ignored, `HORIZON_TEST_RESOURCES` set) | 2 passed (19 s) |
| yoake-bindgen `engine` gate (ignored, CI) | 1 passed (382 s; toolchain pin, regeneration diff, append-only ids against the MX3K1 lock, unsafe policy, stale files, `cargo check -p horizon-client --features yoake2-slice`). It first failed on two new feature-off `Game` stubs, which were regenerated (ids appended) |
| feature-off `cargo test -p horizon-client --lib` | **1193 passed, 0 failed, 40 ignored** |

What the new tests prove:
- `multi.rs` (WAT guests): outer and inner order by priority (the outer sees the count before anyone, the inner sees the outer's change, the outer sees everyone's effect after `original`, the engine ran once), the same with priorities swapped, ties by load order; a middle mod traps and the others plus the engine still run once and the trapped mod is never entered again; an inner mod that calls `original` and then traps leaves the outer mod running and the engine run once; `hook_register` rules and cap; a spinning `yoake_init`; hook 0 is not registered by a multiplexed guest; `unload`.
- `yoake2_loader.rs` (real `.yak` files, `pack_directory`, `sign_file`): an allowed local mod loads and its hook runs; a mod with its switch off or no file is not loaded; a sandboxed manifest comes back `Sandboxed` and nothing loads; server and friend origins are refused even with the switch on, and the whole policy table is checked; a catalog mod loads with a trusted signature and no switch, a signature by an unknown key is `BadSignature`, an unsigned one without a switch is `NotAllowed`; several mods load in file order and compose; the allow file round-trips.
- Real `Game`: three allowed Rust mods and one that is not allowed are in the mods folder. The disallowed one is never loaded. `a_outer` (priority 10) wraps `c_trap` (15) wraps `b_inner` (20). On the server tick the outer mod sees fire_ticks 0, the inner one sees 1 (the outer's write, although the trapper sat between them), both end at 21, the item in the `Game` has 21 (0 + 1 + 20), the trapping mod is disabled and the others are not. On the second tick the trapper is not entered (entries stays 1) and the item reaches 42. No activation or path handle is left open. The single-mod test of MX3K1 passes unchanged on the new runtime.

## Open

- **Client side**: mods load on the integrated-server/dedicated thread only; client hooks and the client thread loader come later.
- The launcher UI for the allow switches and the catalog download path (where `Origin::Catalog` and the trust list would be fed from) are not built; the loader takes the origin as an argument. Server and friend deliveries are refused by policy but nothing yet tags a package with those origins.
- `yoake2_world_start_hook` (the world-start call site) has no test of its own; reloading mods at every world start unloads the previous world's mods but does not stop them mid-tick.
- Priorities are the guest's own claim; nothing yet lets the player reorder mods or caps a mod to a priority band.
- Disabled mods stay in the thread's list (their stats stay readable) until `unload`/`uninstall`.
- Two bindings were added to the generated glue by the new empty feature-off stubs of `Game` (`yoake2_tick_hook`, `yoake2_world_start_hook`); they are `Game`-receiver functions, unreachable by guests, and the ids are append-only.
- Miri is still not available.
