# FR-REM17c: client mod-call, standalone models, item model select, input v2

Branch `yoake-free/rem17c-client`, worktree `horizon-rem17c`, from `yoake-free/merge` 9fd49c7ce. Follows FR-REM17a/b.

| Item | Status | Commit |
| --- | --- | --- |
| 1. Client `mod-call` | Done | `FR-REM17c: client mod-call` |
| 2. `draw_model` helper, standalone models in `ModelCommand::Model` | Done | `FR-REM17c: standalone models draw through the draw_model helper` |
| 3. `item_model_select` | Done as `item_model_select_v2` (additive) | `FR-REM17c: item_model_select_v2` |
| 4. `input_key` v2 | Done as `input_key_v2` + `input_pad_button` + `input_pad_axes` (additive) | `FR-REM17c: input_key_v2, input_pad_button and input_pad_axes hooks` |

Nothing generated was edited by hand: `cargo run -p xtask -- mod-api` (fills the placeholder body of a new native hook) and
`YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib` (WIT, SDK, adapters, hook names, coverage).

## 1. Client `mod-call`

* `client-full` now exports `handle-call` like `server-full` (`wit_full.rs`); the SDK `ClientMod` trait has `handle_call` (default: refuses)
  and `export_client_full!` wires it (`sdk_full.rs`). Hand-written: `ClientFullMod::handle_call` (`yoake-wasm/src/full.rs`).
* Service `client_mod_call(method, payload, target-mod) -> result<list<u8>, string>`, id **298**, capability `mod-call`
  (the server's name for it), client catalog **1.9.0 -> 1.10.0**. SDK: `client::call(target, method, payload)`.
  (The `client_` prefix: service names are unique across catalogs.)
* Host (`wasm_mods/client_services.rs`): a registry of client module slots by package id (filled in `FullMod::new`) and
  `call_client_mod`, which has the server's guards: valid package id, method 1..128 plain characters, payload and answer at
  most 1 MiB, nesting at most 4 (`MAX_CALL_DEPTH`, shared with the server), no call to oneself, an unknown package, a callee
  that is busy (its lock is held: waiting on this very call, directly or through others) is refused, not waited for. A callee
  that traps or runs out of limits gives an error result. The callee runs with the caller hook's game access
  (read stays read, write stays write), inside its own invocation scope.
* **Compatibility note:** a client-full component built before this change has no `handle-call` export and no longer
  instantiates (`invalid client-full exports`); rebuild with the SDK (the macro generates it). The server world has always
  required it. The 1.0 baseline test (`abi_1_0_is_additive`) passes, because it compares function by function and a new
  world export is not an item it tracks, but this is a real change for already-built client-full mods.
* Test guest: `full-demo` feature `client-peer` (`FULL_DEMO_CLIENT_PEER`): answers `echo`, `count`, and `relay` (calls
  another client package from inside its handler, to test nesting).
* Test: `wasm_mods::full::tests::client_mod_call_reaches_the_peers_handle_call_with_the_servers_guards`: echo, count,
  the callee's own error, bad id, empty method, 1 MiB + 1 payload, self call, unknown package, a nested relay b -> c, a busy
  callee, a busy callee reached through a relay.

## 2. Standalone models

* `native-hooks.json`: helper `draw_model` -> `Game::native_draw_model`, so the generated `Host` has
  `draw_model(draw, model, tint) -> bool`.
* `apply_model`: `ModelCommand::Model` asks `draw_model` first (tint masked to RGB; **a guest's tint 0 is black** on the
  model's tinted faces, pass `0xffffff` for none), then falls back to `draw_block`, then `draw_item` (`Fixed`). An invalid id
  draws nothing and asks nobody.
* Test: `cube_item_and_model_commands_reach_the_draw_services` (extended; the fake host's `draw_model` accepts `mymod:block/...`).
* e2e: **not extended.** A standalone model needs a model JSON named in a content mod's `standalone-models` plus fixture
  plumbing in the Cinder Fox pack; not cheap. Not run for item 2 beyond the host test.

## 3. `item_model_select_v2`

The old `item_model_select` hook took an opaque handle (`value: option<u64>`: `Value` was imported from a path that is not
`serde_json`, so the type table did not map it), so no guest could answer it and the load refuses it. Changing its signature
fails the frozen 1.0 baseline, so it is additive, like `menu_button_v2`:

* fragment `tools/native-hook-fragments/item_model_select_v2.json`, `Game::native_item_model_select_v2(property, stack, components, context, value: &mut Option<serde_json::Value>)`;
  WIT `item-model-select-v2 ... value: option<json> -> option<option<json>>` (collect: Some changes, None keeps).
* `item_draw.rs`: the resolver asks v1 (native mods), then v2 (WASM mods), which sees v1's answer.
* v1 stays unwired (`UNWIRED` still lists it); its report line is unchanged.
* Test: `a_guest_chooses_an_item_models_select_case`: the case reaches the resolver as a JSON string; an unanswered or
  foreign property keeps the value v1 gave.
* Not done: a pack `select` model naming the mod's property end to end in a running game.

## 4. Input v2

`input_key` (`winit::event::KeyEvent`) has no guest form, and the 1.0 baseline forbids changing it, so three additive hooks
(fragment `input_v2.json`), all `listen` (no cancel; `key_pre` is the cancellable in-world one):

* `input_key_v2(key, text, pressed, repeat, modifiers)`: every real window key event, down/up/repeat; `key` is the vanilla
  name of the physical key (`key.keyboard.z`, empty for a key with no name), `text` what it typed, `modifiers` GLFW bits.
  Called from `window_event` next to `native_input_key`.
* `input_pad_button(button, pressed)`: the controller layer exists (`horizon-client-input` `gamepad.rs`, `game/pad.rs`);
  called for every button edge of the merged state (triggers count past their threshold), `button` is the name used in
  `horizon-controller.txt` (`south`, `lb`, `dpad_up`...).
* `input_pad_axes(left_x, left_y, right_x, right_y, left_trigger, right_trigger)`: when any of them moved more than 0.01
  since the last call, or came back to exact rest.
* The denylist reason of `input_key` now says why it stays refused and what replaces it. `yoake-cli`'s `FULL_UNWIRED` and the
  `unwired-client` validation fixture (which subscribes to `input_key`) are unchanged and still correct.
* Tests: `a_guest_hears_keys_with_state_and_modifiers_and_pad_buttons_and_axes` (down, repeat, up with modifiers, no-name key,
  pad buttons, axes; the guest echoes events as local sound ids), `game::pad::axes_tests`.
* Not done: a live key/pad press in a running game.

## Tests run

All with `.\hz-cargo.cmd ... -j 6`; C: had 35 GB free.

* `test -p horizon-mod-api-gen --lib` (also with `YOAKE_UPDATE_WIT=1` after each generator change): 46 passed (incl. `abi_1_0_is_additive`,
  committed WIT/adapter/SDK match tests).
* `test -p horizon-client --lib --features yoake-hook-bench,yoake-wasm/test-guests -- wasm_mods::full catalog_matches_hook_points a_guest_hears axes_are_sent`: 36 passed
  (the new tests above plus the existing `wasm_mods::full` suite and `catalog_matches_hook_points`).
* `test -p yoake-wasm -p yoake --features yoake-wasm/test-guests`: all passed (35 + 3 + 73 + 6 + 2 + 11 + 5 + 1 + 9 + 7 + 16 + 2 + 3 + 5).
* `build -p horizon-client -p horizon-client-lib --bin horizon-client --bin horizon-ctl`: ok.
* e2e `tools/custom_mob_render_e2e.py --bin-dir target\debug --wasm yoake_wasm_fox_ai.wasm --client-wasm yoake_wasm_fox_texture.wasm`
  (offscreen, no focus, through horizon-ctl): all checks pass (idle, glow, pounce, lod, name, brain, strike, runtime_texture,
  model_commands, death). It does not exercise items 1, 3 or 4 in a live game, nor a standalone model.

## Left

* Item 2 e2e (standalone model in the Cinder Fox pack).
* `hook-ledger` ids for the four new `native_*` functions (`sync-ledger`; not run here, ids are the stamp tool's).
* `docs/native-mods.md` has no section for the new native hooks (`@API:input_v2@` and `@API:item_presentation@` tokens are left as the other hooks have them).
* Gamepad buttons beyond edges (per-pad ids, DualSense touchpad/gyro) are not exposed.
