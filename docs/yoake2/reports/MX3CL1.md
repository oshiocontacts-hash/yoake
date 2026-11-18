# MX3CL1: full-access mods on the client thread

Branch `yoake2/client-hooks` (based on 3828f998b, merged with `vps/yoake2/merge` 18c35c022).

## Checklist

- [x] 1. Manifest: `[wasm] side = "server" | "client" | "both"` (full-access only, default `server`), validated against `[side]`; tests in `crates/horizon-mods/tests/yak/manifest.rs`
- [x] 2. Client runtime: `load_client_mods` / `unload_client_mods` (per-thread runtime, same approved list and trust rules), called from `native_session_ready` / `native_session_left` for client games
- [x] 3. Synthetic hooks `HOOK_CLIENT_TICK = 0x8000_0002` (`tick_remote`), `HOOK_HUD_RENDER = 0x8000_0003` (`Game::hud`); SDK `hooks::CLIENT_TICK` / `hooks::HUD_RENDER`; rows in `ENGINE_DISPATCHED` (Full, Full, ObserveOnly)
- [x] 4. Tests (`crates/horizon-client/tests/yoake2_loader.rs`, no game resources)
- [x] 5. `docs/yoake2/ENGINE_CALL_ABI.md`, section "Sides and threads (MX3CL1)"
- [ ] 6. Suites green (see "Test results")
- [x] 7. This report
- [ ] push `git push vps yoake2/client-hooks`

## Design choices

- **Manifest.** `Wasm.side: Option<FullAccessSide>` (`server` default, `client`, `both`); an error when set without `world = "full-access"`. The old rule "a full-access mod needs `[side] server = true`" became per side: a mod that runs on the server needs `side.server`, one that runs on the client needs `side.client`. Existing packages (no `wasm.side`) behave exactly as before.
- **Loader.** `ThreadSide { Server, Client }`; `load_package_on`, `load_dir_on`, `load_client_mods`, `unload_client_mods`, `client_mods_loaded`. The old `load_package` / `load_dir` / `load_world_mods` are the server variants unchanged. A package of the other side gives `Refusal::OtherSide`, which is silent (no message, no "no list" note) and checked before the origin/approval policy, so each side judges and reports only its own packages. The thread-local `LOADED` list is kept per side.
- **Where the client loads.** `native_session_ready` (once per session after login; `native_session.ready()` fires once) for games whose `execution_role` is `Client`; unload in `native_session_left`. This is the window's thread in both single player (it is a remote client of the integrated server) and multiplayer. The server-side load (`yoake2_world_start_hook`) is untouched.
- **Hooks.** `CLIENT_TICK` at `tick_remote` right after the client mods' own tick (the client's per-tick path, also while waiting for chunks), `HUD_RENDER` at the end of `Game::hud` (after the layers, dialogs, debug overlay). Both dispatch through `yoake2_mods::dispatch_client_tick` / `dispatch_hud_render`, which are plain functions (testable without a `Game`). `HUD_RENDER` is dispatched with an empty `FieldTable<()>`: argument 0 exists but nothing can be read from it.
- **Cost with no mod.** Same gate as `SERVER_TICK`: the call sites are empty `#[inline(always)]` methods without `yoake2-slice`; with it, one `execution_role` compare plus the runtime's thread-local read (no mod on the thread: no allocation, `orig` runs).
- **Placement of the code.** New methods live in `crates/horizon-client/src/game/yoake2_client.rs`, not in `native_client_items.rs`, because yoake-bindgen scans that file (new methods there would change the generated glue / ids).
- **ENGINE_DISPATCHED.** Became `&[(u32, Mode)]` (the other lane had `&[u32]`, Full only) because `HUD_RENDER` is observe-only; `hook_mode` consults it first and `install_bridges` filters it out; their disjoint-bridges test now also checks the mode.
- **Threads.** The runtime keeps everything in thread-locals (`RT`, `ACTS`, `PATHS`); the process-wide parts (`DISPATCH` mutex, mode resolver, `install_once`) are read-only after setup or idempotent, so nothing assumed one thread. `yoake2_install_dispatch()` is called again at the client session start (idempotent).

## Test results

(filled in below)

## 要確認 (not guessed)

1. **Module field for client mods.** A `side = "client"` mod still declares its module as `wasm.server` (the manifest rule "full-access = `wasm.server`, no `wasm.client`" is unchanged). Should a client-only mod use `wasm.client` instead?
2. **`[side]` table vs `[wasm] side`.** Two similar keys now exist. I made them agree by validation instead of deriving one from the other. Alternative: derive the thread from `[side]` and drop `wasm.side`.
3. **One thread, two sides.** If a build ever ran the client `Game` and the integrated server on the same thread, both sides would share one runtime (hooks would mix). The loader keeps separate unload lists per side, but cannot separate the runtime. Real builds use two threads; not verified with a real game here (no GUI/launch allowed).
4. **`HUD_RENDER` argument.** Argument 0 is a handle to nothing rather than "no handle". Changing that needs a runtime entry without arguments.
5. **`CLIENT_TICK` and the replay / paused-reconfiguration states.** It runs from `tick_remote`, so it also runs while the level is loading and during reconfiguration; replays use the same path. Whether mods want it suppressed there is open.
6. **Real-game check.** The client hooks' call sites compile but were not exercised in a running client (the game-resource tests in `game/yoake2_game_tests.rs` need resources and a `Game`; not run).
