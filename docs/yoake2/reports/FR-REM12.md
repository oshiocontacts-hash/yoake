# FR-REM12: client services for full-access (F) modules: drawable HUD, audio, IME, GPU

Branch `yoake-free/rem12-fullclient` (from `yoake-free/merge` `ff160e9f9`). Nothing here was run in a real game window.

## What was added

A full-access module (route F, core WASM) that runs on the client thread now has what a sandboxed `client-full`
module (route S) has: it draws from `HUD_RENDER`, plays sounds and feeds PCM streams, reads the GUI size, takes the
input method and uses the GPU. They are engine-call bindings of a new generated target `f_services`
(`crates/yoake-bindgen/bindgen.json`), guest stubs `yoake_guest_sdk::stubs::f_services` (ids 2894..=3089 in
`ids.lock.json`, append-only: 196 entries added, none changed).

| group | bindings | how it reaches the game |
|---|---|---|
| HUD | `hud_draw(Vec<FHud>)`, `hud_gui_size()`, `hud_gui_scale()` | `FHud` is the S route's `hud-command` set (text, rect, gradient, quad, sprite, item, texture, highlight, clip/scale/blend runs). `hud_draw` is accepted only inside `HUD_RENDER`: the commands are queued, and `Game::hud` applies them to the frame's `HudBatch` when the hook returned, with the S route's own `wasm_mods::full::apply_hud` (`apply_f_hud` converts to the S wire commands, so the same limits and skips hold). At most 8192 commands a frame; a bad call queues none of its commands. |
| audio | `audio_play_sound`, `audio_play_sound_at`, `audio_stream_open/push/queued/dropped/move/close` | sounds go to `Game::native_queue_local_sound` (same validation as the S route); PCM streams to `native_audio_stream_*`. All of it is queued while the hook runs and applied when it returns (a stream is opened by the game then; a refusal shows as an error of the next call naming the stream). 64 sounds waiting, 16 streams, about a million samples waiting per stream, samples clamped, NaN is silence. |
| IME | `ime_set_allowed(bool)`, `ime_set_cursor_area(x, y, w, h)`, `ime_event()`; hooks `IME_PREEDIT`, `IME_COMMIT` | FR-L1's "not done" list for F, item 1-4: new synthetic hooks (no ledger entry needed: engine-dispatched, observe-only) whose event (preedit text and UTF-8 byte cursor range, or commit text) the guest reads with `ime_event()`, which avoids a string hook argument; `ime_set_allowed(true)` takes the input method: `text_field_focused` is true, the engine's own composition overlay and text fields step aside (`ime_preedit`, `ime_commit`, `flush_ime`), and the candidate window goes to the module's rectangle (GUI pixels, scaled like the S route's). `ime_set_allowed(false)` hands it back. |
| GPU | `gpu_shader`, `gpu_shader_file`, `gpu_buffer`, `gpu_texture`, `gpu_sampler`, `gpu_compute`, `gpu_render`, `gpu_bind_group`, `gpu_destroy`, `gpu_write_buffer`, `gpu_copy_buffer`, `gpu_write_texture`, `gpu_dispatch`, `gpu_draw`, `gpu_draw_region`, `gpu_read`, `gpu_poll`, `gpu_frame` (types `F...` mirror `gpu-types`) | `wasm_mods/f_route.rs`: one `GameHost` with the GPU on (the S route's `GpuServices` code, quotas and ownership) under the GPU identity `yoake-f-client` for all F client modules. A module that registers the new `GPU_FRAME` hook gets the renderer's per-frame callback (`register_f_gpu` at session start, `release_f_gpu` at session end). |

New synthetic hooks (observe-only, no handle; `yoake_slice::runtime`, `yoake_guest_sdk::hooks`, `ENGINE_DISPATCHED`):
`GPU_FRAME = 0x8000_0004`, `IME_PREEDIT = 0x8000_0005`, `IME_COMMIT = 0x8000_0006`. `HUD_RENDER` keeps its dead
argument and its observe-only mode: drawing needs no handle because the bindings are free functions.

Design notes:

* The bindings are free functions with per-thread state, not `Game` methods, so they work in every client hook without
  a root handle and without `unsafe` (the crate denies it). What needs the `Game` is queued and applied by the game
  after the hook (`game/yoake2_client.rs`: `yoake2_hud_render_hook`, `yoake2_f_flush`).
* `yoake-bindgen` binds every function of a target file, whatever its visibility, so the bound surface is
  `game/yoake2_f_services.rs` (wire types and thin wrappers) and everything else (state, limits, flush, helpers) is
  in `game/yoake2_f_state.rs`, which is not a target.
* All F client modules of a session share this state and the one GPU identity: the services do not know which module
  is calling. Per-module GPU/audio ownership would need the runtime to name the calling module to the dispatcher.
* Nothing runs without a module that calls it: the flush points return at once when no client mod is loaded
  (`client_mods_loaded()`), and without the `yoake2-slice` feature the hooks are empty functions.

## Generated files

`crates/yoake-bindgen/regen.sh` was run (rustdoc JSON of the nine crates with the feature off, then the generator).
Its output for the *other* targets differs from what is committed (for example `mod_generator`: `ModTerrain` and its
bindings are gone from the glue, `ModChunk::set_biome_at` and `ModProto.failed` are new; items, mobs, coverage
rows move): the committed glue is not what this tree regenerates, independent of this change. Those files were put
back to the committed state and only the `f_services` outputs were kept: `yoake_glue/f_services.rs`,
`stubs/f_services.rs`, `functions-f_services.md`, the new `f_services` rows of `coverage.md` and `summary.json`, and
the 196 new entries of `ids.lock.json` (the others stay exactly as committed). A full `regen.sh --check` therefore
still reports that drift; `f_services` itself is current. Whoever regenerates the other targets next will get the
`mod_generator` ids 3090.. (the regen assigned them after ours).

## Tests

Host tests (`game/yoake2_f_state/tests.rs`, 11): HUD refused outside the hook / queued inside / 8192-command limit /
bad call queues nothing / a new frame drops an unfinished one; sounds validated and capped; a PCM stream opens, takes
samples (clamped, NaN silent), reports queued/dropped, moves and closes at the flush with a fake audio engine; an
engine that takes only part of the samples keeps the rest; a refused stream is dead with the reason and not retried;
stream argument and quota checks; shutdown closes the game's streams, forgets the state and never reuses a stream id;
IME allowed/area/events (UTF-8 byte ranges, text capped at a character); the GPU services refuse without a device and
do not panic. `wasm_mods/full.rs`: `f_hud_commands_become_the_same_wire_commands_and_are_drawn_through_the_host_services`
(FakeHost: rect, text, gradient strips, highlight, clip/scale/blend; no host draws nothing; over-long text draws nothing).

Guest test (`game/yoake2_f_state/guest_tests.rs`, 1): `crates/yoake-guest-sdk/examples/f_client_services.rs` compiled to
wasm32 with the SDK, installed in the runtime with the generated glue as the engine dispatcher: from `CLIENT_TICK` it
plays a sound, opens and feeds a stream, takes the IME, and `hud_draw` outside `HUD_RENDER` is refused; from
`HUD_RENDER` it reads the GUI size and draws a rect and a text (the test sees both commands); the IME hooks deliver
the event; the hook modes of the new ids are observe-only.

Results and commands: see the end of this file.

## Not done / not verified

* No live window: the HUD batch, the audio engine, the window's IME and the renderer's GPU callback were run only
  through fakes and the guest harness. The GPU services were checked to refuse cleanly without a device; a real
  device run (a module creating a buffer and drawing from `GPU_FRAME`) is not done. If the renderer calls its package
  callbacks on a thread other than the client thread, `GPU_FRAME` finds no runtime and does nothing; not verified.
* No e2e script hosts an F client module (the existing ones load server-side F mods or S packages), so none was run.
* `hud_text_width`, `hud_place` and item/block sprite services of the S route have no F binding yet; `draw_world_*`,
  camera, post shaders, `mob_*` and `client_mod_call` client services are not in this item.
* Per-module isolation (see design notes).
* `docs/yoake2/MOD-GUIDE*.md` not edited (as asked); `ENGINE_CALL_ABI.md` documents the hooks and services.

## Results (all with `hz-cargo.cmd`, `--offline -j 6`, disk 38 GB free before)

| command | result |
|---|---|
| `test -p horizon-client --lib --features "yoake2-slice,yoake-hook-bench" yoake2_f` | 12 passed (11 host + the guest test) |
| `test -p horizon-client --lib --features "yoake2-slice,yoake-hook-bench" f_hud_commands` | 1 passed |
| `test -p horizon-client --features "yoake2-slice,yoake-hook-bench" --test yoake2_loader` | 42 passed, 1 failed: the known `a_loaded_mod_that_registers_a_bridged_hook_id_is_called_by_the_engine` (expects `[(1, "arg0 is not &T or &mut T")]`, gets `[]`); not in this change's path, left |
| `--test yoake2_examples --test yoake2_template` | 2 + 2 passed |
| `test -p yoake-slice` | all passed (32 + 1) |
| `test -p yoake-bindgen` | lib 0, policy 3, mini_engine 8 passed; `engine` (ignored, slow) not run |
| `test -p yoake-guest-sdk` | 5 + 7 passed, 1 failed: `rustc_and_wasm_ld_retain_the_sdk_abi_section` expects the SDK's section to be 1.0, the runtime constant is 1.1 (documented in ENGINE_CALL_ABI.md); present before this change, not touched |
| `build -p horizon-client -p horizon-client-lib --bin horizon-client --bin horizon-ctl` (default features) | ok |

The regeneration that produced the `f_services` files is `crates/yoake-bindgen/regen.sh` (rustdoc JSON with
`RUSTC_BOOTSTRAP=1 RUSTDOCFLAGS=` for horizon-client-lib, horizon-client, horizon-items, horizon-core, horizon-world,
horizon-client-core, horizon-client-render, horizon-protocol, horizon-command; then `yoake-bindgen --config
crates/yoake-bindgen/bindgen.json`); see "Generated files" for what was kept.
