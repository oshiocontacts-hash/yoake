# MX3CL1: full-access mods on the client thread

Branch `yoake2/client-hooks` (based on 3828f998b, merged with `vps/yoake2/merge` 18c35c022).

## Checklist

- [x] 1. Manifest: the side comes from the declared modules (`wasm.server` = server thread, `wasm.client` = client thread, both allowed), validated against `[side]` (revised: the first draft had a `[wasm] side` key, now removed); tests in `crates/horizon-mods/tests/yak/manifest.rs`
- [x] 2. Client runtime: `load_client_mods` / `unload_client_mods` (per-thread runtime, same approved list and trust rules), called from `native_session_ready` / `native_session_left` for client games
- [x] 3. Synthetic hooks `HOOK_CLIENT_TICK = 0x8000_0002` (`tick_remote`), `HOOK_HUD_RENDER = 0x8000_0003` (`Game::hud`); SDK `hooks::CLIENT_TICK` / `hooks::HUD_RENDER`; rows in `ENGINE_DISPATCHED` (Full, Full, ObserveOnly)
- [x] 4. Tests (`crates/horizon-client/tests/yoake2_loader.rs`, no game resources)
- [x] 5. `docs/yoake2/ENGINE_CALL_ABI.md`, section "Sides and threads (MX3CL1)"
- [x] 6. Suites green except the SDK-guest example harness (see "Test results")
- [x] 7. This report
- [ ] push `git push vps yoake2/client-hooks`

## Design choices

- **Manifest.** No `side` key. A full-access mod declares `wasm.server` and/or `wasm.client`; `[side]` must agree exactly (`side.server` iff `wasm.server`, `side.client` iff `wasm.client`). Existing server-only packages are unchanged. A mod with both modules gets two separate instances (one per thread).
- **Loader.** `ThreadSide { Server, Client }`; `load_package_on`, `load_dir_on`, `load_client_mods`, `unload_client_mods`, `client_mods_loaded`. The old `load_package` / `load_dir` / `load_world_mods` are the server variants unchanged. A package that declares no module for the thread gives `Refusal::OtherSide`, which is silent (no message, no "no list" note) and checked before the origin/approval policy, so each side judges and reports only its own packages. The thread-local `LOADED` list is kept per side.
- **Where the client loads.** `native_session_ready` (once per session after login; `native_session.ready()` fires once) for games whose `execution_role` is `Client`; unload in `native_session_left`. This is the window's thread in both single player (it is a remote client of the integrated server) and multiplayer. The server-side load (`yoake2_world_start_hook`) is untouched.
- **Hooks.** `CLIENT_TICK` at `tick_remote` right after the client mods' own tick (the client's per-tick path, also while waiting for chunks), `HUD_RENDER` at the end of `Game::hud` (after the layers, dialogs, debug overlay). Both dispatch through `yoake2_mods::dispatch_client_tick` / `dispatch_hud_render`, which are plain functions (testable without a `Game`). `HUD_RENDER` is dispatched with an empty `FieldTable<()>`: argument 0 exists but nothing can be read from it.
- **Cost with no mod.** Same gate as `SERVER_TICK`: the call sites are empty `#[inline(always)]` methods without `yoake2-slice`; with it, one `execution_role` compare plus the runtime's thread-local read (no mod on the thread: no allocation, `orig` runs).
- **Placement of the code.** New methods live in `crates/horizon-client/src/game/yoake2_client.rs`, not in `native_client_items.rs`, because yoake-bindgen scans that file (new methods there would change the generated glue / ids).
- **ENGINE_DISPATCHED.** Became `&[(u32, Mode)]` (the other lane had `&[u32]`, Full only) because `HUD_RENDER` is observe-only; `hook_mode` consults it first and `install_bridges` filters it out; their disjoint-bridges test now also checks the mode.
- **Threads.** The runtime keeps everything in thread-locals (`RT`, `ACTS`, `PATHS`); the process-wide parts (`DISPATCH` mutex, mode resolver, `install_once`) are read-only after setup or idempotent, so nothing assumed one thread. `yoake2_install_dispatch()` is called again at the client session start (idempotent).

## Revision 2 (coordinator decisions)

- `[wasm] side` removed (parser, `FullAccessSide`, tests, ABI doc). `Archive::full_access_module(client: bool)` reads `wasm.client` or `wasm.server`.
- Guard: loading a client runtime on a thread that holds server-side mods (or the reverse) gives `Refusal::SharedThread { loading, holds }`, printed and logged. It is checked on the per-thread loaded list that `load_world_mods` / `load_client_mods` keep. Test `a_client_runtime_is_refused_on_a_thread_that_holds_a_server_runtime_and_the_reverse`. Note: a runtime filled directly through `runtime::load` / `load_dir` (not the two entry points) is not tracked by the guard.
- HUD_RENDER argument 0 documented as a dead handle. **Difference from the decision text:** `get`/`set` on it return status 3 (`BadField`), not `Stale` (the activation is live, there is just nothing behind the handle); verified by test `the_hud_render_argument_is_a_dead_handle`. Making it literally `Stale` would need a runtime entry without arguments.
- CLIENT_TICK during level loading, reconfiguration and replays: documented, behaviour unchanged.
- Examples/test manifests: the example manifests are server-only and still valid; the loader test helper now writes `[side]` from the declared modules.

## Test results

Run with `cargo +1.100.0 ... -j 4`, features `yoake2-slice,yoake-hook-bench`.

| suite | result |
|---|---|
| `horizon-mods --features yak` | 41 passed (new: `full_access_modules_decide_the_threads_and_side_must_agree`) |
| `yoake-slice` | 28 + 1 doc test passed (73 more in its other target earlier) |
| `horizon-client --test yoake2_loader` | 27 passed (new: side loading, client ticks, server-only not on client, both = two instances, client trap, no-mod no-op, shared-thread guard, dead HUD handle) |
| `horizon-client --test yoake2_examples` | 2 passed |
| `cargo check -p horizon-client` without `yoake2-slice` | ok |

## 要確認

1. `HUD_RENDER` argument 0 gives `BadField`, not `Stale` (see above); is that acceptable, or should it be made `Stale`?
2. The shared-thread guard relies on the two loader entry points; direct `runtime::load` use on a thread is not seen.
3. Not exercised in a running client (no game launch): the call sites in `tick_remote`, `Game::hud`, `native_session_ready/left`.
