# MX3HR1: dev-mode hot reload (game side)

Branch `yoake2/hot-reload` (based on bc0cd95f2 = yoake2/merge + client-hooks).

## Checklist

- [x] 1. `ApprovedList.generation` (optional top-level integer, absent = 0, parsed and serialized only when non-zero, old files read as 0), `DEV_ARG = "--full-access-dev"`. Pushed early: `9e4a13793` (field) and `6988cbc03` (serialization + unit tests; the first commit lacked the writer, use **6988cbc03** or later)
- [x] 2. `runtime::guest_running()` (any guest of the thread on the stack, or the mod list borrowed)
- [x] 3. `yoake2_mods::dev_reload_check` / `dev_reload_tick` / `dev_mode` / `launch_approved_list`
- [x] 4. Call sites: server thread (`ServerTickStart`, new `yoake2_dev_reload_hook`), client thread (`yoake2_client_tick_hook`); world and session starts load with `launch_approved_list()`
- [x] 5. Tests in `crates/horizon-client/tests/yoake2_loader.rs` (11 new) and `control.rs` unit test
- [x] 6. `ENGINE_CALL_ABI.md`, section "Dev reload (MX3HR1)"
- [x] 7. Suites (see "Test results")
- [x] 8. This report
- [x] push `git push vps yoake2/hot-reload`

## Design as built

- **Flag.** `dev_mode()` is true when the process command line has `--full-access-dev` (read once). Everything below is behind it. Without it `dev_reload_tick` is one `OnceLock` read and returns; `launch_approved_list()` returns `process_approved_list().cloned()`, the same list as before. Call sites in the engine add only: one call before `yoake2_tick_hook` (empty `#[inline(always)]` method without the `yoake2-slice` feature) and one call in the client tick hook. With no mods, vanilla behaviour is unchanged (no thread-local is touched beyond the flag).
- **Check.** `dev_reload_check(enabled, side, list_path, mods_dir, now)` (pure core, `dev_reload_tick` supplies the process values). Per thread and per side it keeps `in_session`, `loaded_generation`, `next_check`, `stamp` (mtime + size). At most once per second (`DEV_CHECK_INTERVAL`) it stats the file; an unchanged stamp ends it. A changed stamp: if a guest is running the check is postponed (stamp kept, logged); otherwise the file is read and parsed (`ApprovedList::read`, same 1 MiB limit). A read or parse error is logged ("keeping the loaded mods"), the stamp is remembered so it is not logged every second, and nothing changes. `generation <= loaded_generation`: nothing. Otherwise the reload runs.
- **Reload.** The new list's `protected` paths are registered with `guard`; the list is stored as the process-wide newest list (`DEV_LATEST`); then `load_mods_on(side, mods_dir, Some(&list))`, the same function the world/session start uses: `unload_previous` (drops this thread's runtime for that side), `load_dir_on` with the new list's allow/origin files (same trust rules, ABI check, signatures; server and friend packages never), `yoake_init`, `install_bridges`. Per-mod failures are `LoadReport` skips, announced like at startup; the other mods load. One line per reload: `yoake: dev reload (Server) generation N: K mod(s) loaded [ids], R refused` (refused = skipped except sandboxed / other-side).
- **Safe point.** The server check is at `native_lifecycle_server_tick_start`, immediately before `yoake2_tick_hook` (a separate method on purpose: `yoake2_tick_hook` is itself a hookable function with its own bridge, id 460, so a check inside it would run inside a guest's `original` and always be refused). The client check is in `yoake2_client_tick_hook` before `dispatch_client_tick`. `runtime::guest_running()` is the assertion: it looks at every mod slot's instance borrow, which is held for the whole activation (also during `original`), so a hook that wraps an enclosing engine call makes the reload wait, never run inside. `debug_assert!(!guest_running())` before the reload.
- **Sides.** Server and client keep separate state and reload on their own thread at their own check; the client only if `load_client_mods` ran and `unload_client_mods` did not end it (`in_session`); the server from its first world-start load. A newer list read by one thread is used by world/session starts on any thread (`launch_approved_list`).
- **Dispatchers.** Unchanged: `install_once` per id. A reload goes through `install_bridges`, so ids first registered after a reload are bridged then; ids already bridged are not installed again. The chains in the runtime are rebuilt (`unload`/`load` bump the thread's generation), so they point at the new instances; the old `ModSlot`s are dropped. Nothing in the runtime or the bridge assumed a single load per thread (the "leaked chains" note holds: bounded, no effect).
- **Placement.** New methods are in `game/yoake2_client.rs` (not scanned by yoake-bindgen); `native_lifecycle.rs` only gains one plain method call (no `cfg`).

## Tests

Added to `crates/horizon-client/tests/yoake2_loader.rs` (real `.yak` files, the real runtime, no game resources; a `DevLaunch` helper rewrites the list with a temp file + rename like the launcher):

- `the_generation_round_trips_and_a_file_without_one_is_generation_zero` (+ `control.rs` unit test: negative / non-integer generation invalid)
- `without_the_dev_flag_a_changed_list_is_never_looked_at` (`enabled = false`: old guest keeps running; the same file reloads with it on)
- `a_higher_generation_reloads_the_changed_guest_and_the_old_one_no_longer_runs` (only the new tag runs, one mod, not two)
- `the_same_generation_does_not_reload_even_when_the_file_changed` (equal and lower generations ignored, then higher reloads)
- `the_file_is_looked_at_once_per_second_and_only_when_it_changed`
- `a_broken_list_keeps_the_old_mods_and_a_later_good_one_still_reloads` (four kinds of broken file)
- `a_mod_that_traps_in_init_or_is_not_approved_is_reported_and_the_others_still_load`
- `a_reload_is_refused_while_a_guest_is_running_and_happens_at_the_next_check` (the check is called from inside `original`; the activation finishes undisturbed)
- `an_id_registered_for_the_first_time_after_a_reload_is_bridged_once` (hook 71 through the real armor-stand bridge: new guest entered once per call, also after a second reload)
- `the_client_and_the_server_runtimes_reload_independently` (two threads, one `both` mod)

Not tested (needs the process flag or a `Game`): `dev_mode()` reading the command line, `launch_approved_list()` preferring the newer list, the two call sites.

## Test results

- `yoake2_loader` (features `yoake2-slice,yoake-hook-bench`): 37 passed (27 old + 10 new `hr` tests; the generation round trip counts among them)
- `horizon-mods --features yak`: 73 + 41 passed; `yoake-slice`: 28 + 1 passed; `yoake2_examples` (feature `yoake2-slice`): 2 passed; no-feature `cargo check -p horizon-client`: clean (warnings only)

## 要確認

- **mtime granularity.** The check compares mtime and size only (as designed). Two rewrites inside one file-system timestamp tick with the same size (a generation of the same digit count and equal-length hashes) would be missed. The launcher's rewrites are seconds apart; the tests sleep 40 ms between rewrites. If the launcher can rewrite faster, add a content hash to the stamp (one read per check, cheap) or make it raise the size.
- **Reload drops all mods of the side**, also unchanged ones (fresh instances, state lost). A per-mod reload (keep mods whose bytes did not change) was not asked for and is not built.
- **A reload whose `unload` fails** (cannot happen after the `guest_running` check, but) logs the problem and keeps `loaded_generation` advanced, so it is not retried until the next generation.
- **The two `[wasm]` modules of a `both` mod** reload separately, each at its own thread's check; between the two checks the server may run the new build while the client runs the old one for up to about a second plus a tick.
- **Entering dev mode with no list path** (flag without `--full-access-approved`): nothing happens, no message.
