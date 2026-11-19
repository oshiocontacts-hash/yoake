# FR-S1: small client/server services and permission fixes for sandboxed WASM mods

Branch `yoake-free/s1` (from `yoake-free/merge`), worktree `horizon-s1`. Seven items from the
cooking-duel mod. Every "missing" claim was checked in the code first. Nothing launches the game;
nothing changes for a game without a mod that uses these.

New ABI service ids (the 221-250 block): **221** `screen_carried`, **222** `local_player_stacks`,
**223** `play_local_sound`, **224** `play_local_sound_at`, **225** `gui_texture_id` (client,
`abi-subset-client.json`), **226** `player_set_food` (server, `abi-subset-server.json`). New hook:
`key-pre` (client). 227-250 are free. The 1.0 baseline test (`abi_1_0_is_additive`) passes: only new
functions and one new hook were added, no existing record, enum, variant or function changed.

How a service is added here (the precedent of 199/200): a row in the ABI catalog JSON, a
`native_*` helper on `Game` named in a fragment (`tools/native-hook-fragments/fr_s1_services.json`,
which becomes a `Host` method), the trait line in `yoake-wasm/src/client_services.rs` or `full.rs`,
the `GameHost` / `GameServerHost` implementation, then
`YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib` regenerates the WIT, the adapters, the
hook names and the guest SDK bindings.

## 1. net-send-to-server from screen input hooks

* **Existed:** `network::net-send-to-server` took `host.write()`; the screen key and mouse hooks
  (`screen-key-pressed-pre`, `screen-char-typed-pre`, `screen-mouse-*`, `screen-click`) run with
  `HostAccess::Read`, so it answered `unsupported` there. Confirmed.
* **Added:** a `net_send_to_server_shared(&self, ...)` helper (`game/native_net.rs`) with the same checks
  and the same 32-a-tick cap (the counter is now an `AtomicU32`; `native_net_send_to_server` calls it).
  `GameHost::net_send_to_server` uses the write host when it has one and falls back to the shared path
  for any hook that has the client host read-only (so also render hooks, not only screen hooks).
  The send itself already took `&self` (`Game::send`).
* **Not done:** the DLL path (`host_net_send`) keeps `write()`; "other client-side non-world-mutating
  sends" were limited to this one, since it is the only one that was refused for that reason.
* **Tests:** `wasm_mods::full::tests::net_send_to_server_works_from_read_only_hooks`.

## 2. Textured, rotated GUI quads, and gui-texture commands that draw

* **Existed:** `hud-command::quad` already has `corners`, `sprite`, `uv`, `color`, but the host skipped
  any quad with a sprite. `hud-command::texture` (`id`, x, y, w, h, color) was dropped
  (`HudCommand::Texture(_) => {}`). Returned commands are applied in `apply_hud` after the hook
  returns, outside the call scope, so a returned command cannot hold a texture handle. Confirmed.
* **Added (no new variant: a variant case would change an existing type):**
  * `apply_hud` now draws a sprite quad: `sprite` = a GUI atlas sprite name (new helper `sprite_quad`
    in `game/native_s1.rs`, four corners, four corner UVs, ARGB tint) or `texture:<id>` (a GUI texture).
  * `hud-command::texture` draws when `id` names a texture.
  * Service **225** `gui_texture_id(texture) -> string` gives a texture's id (`<mod>#<handle>`) and
    registers it in a process-wide table; the entry goes when the texture is dropped or the mod's
    resources are released. Unknown ids draw nothing.
  * A textured quad is drawn upright into a 1024-pixel square with the UV rectangle between the first and
    third UV, then its vertices are moved onto the four corners (bilinear), so a turned, sheared or
    squeezed quad keeps the texture mapped.
  * Rotation about a pivot is done by the caller turning the corners: SDK
    `full::client::rotated_corners(x, y, w, h, degrees, pivot)`.
* **Tests:** `quad_points_follow_a_turned_square` (the mapping),
  `a_sprite_quad_goes_to_the_host_with_its_uvs_and_an_unknown_texture_draws_nothing` (sprite quads reach
  the host with their UVs; an unknown texture id draws nothing).
* **Needs checking:** no GPU test of the textured-quad path (needs a wgpu device); nothing was rendered.

## 3. Client inventory reads

* **Existed:** `Host::screen_carried_stack` (native) had no WASM service. The local inventory had nothing.
* **Added:** **221** `screen_carried() -> option<item-stack>` (`service.screen`) and **222**
  `local_player_stacks() -> list<item-stack>` (`service.client-view`): main inventory (hotbar first),
  armor, off hand, no empty slots, no cursor, at most 41 entries. New helper `local_player_stacks`.
* **Note:** the list has no slot numbers (as specified). A slot-addressed read is a follow-up if the mod needs it.
* **Tests:** `client_inventory_reads_and_sounds_go_through_the_host`.

## 4. Block-entity write-back

* **Existed:** `block-entity-tick`, `menu-button`, `menu-button-v2`, `menu-quick-move` returned an `entity`
  (`mod-block-entity-fields`: id, nbt, items, data) that the generated adapter read and discarded
  (`let _ = r;`), listed as `partial ... not applied` in `full_adapters_report.txt`. Confirmed.
* **Added:** the adapter generator (`horizon-mod-api-gen/src/abi/wasm_adapters.rs`, `apply_extra`) now
  writes the answer back into the `&mut ModBlockEntity` through `DerefMut`, only when it differs (so an
  unchanged answer does not bump the revision). The four `partial` lines are gone from the report. The
  engine already puts the entity back with `setChanged` semantics (modified chunk, container-changed
  notification) in `tick_mod_block_entity` / `mod_menu_button`.
* **Sync:** unchanged and explicit: call the existing `block-entity-sync` (58); the update tag sent with
  chunks and block updates is unchanged. No automatic per-tick sync (the engine doc says not to).
* **Not done:** `block-entity-read` / `block-entity-patch-call` (59/60) stay `unsupported`. Their
  `block-entity: u32` is a handle only the DLL ABI hook receives; a full-world mod is never given one.
  A position-addressed read/patch would be a new service pair (not added). Reading is possible through the
  hook's `entity` argument and writing through the return value.
* **Tests:** the generated adapters are compared with the generator (`committed_wasm_adapters_match_the_catalog`)
  and compile. **No test runs a guest through the write-back** (needs a wasm test guest build).
  Needs checking in game.

## 5. Client-only sound

* **Existed:** `Host::play_local_sound` (positional, delayed, needs `&mut`); no WASM service.
* **Added:** **223** `play_local_sound(pitch, sound-id, volume)` (UI sound, relative to the listener, no
  attenuation) and **224** `play_local_sound_at(pitch, position, sound-id, volume)`, `service.audio`.
  Hooks with a read-only host cannot play audio directly, so a helper (`queue_local_sound`) queues the sound
  (64 at most, id `namespace:path`, volume 0-16, pitch 0-8, window client only) and
  `native_frame` plays the queue first thing next frame (`Audio::play_unshared`, so replay capture does
  not hear it). Returns an error when refused.
* **Tests:** the id and value validation in `game/native_s1.rs`; the service path in
  `client_inventory_reads_and_sounds_go_through_the_host`. Needs checking: that the sound is audible.

## 6. Food

* **Existed:** the `food-level-changed` hook (mutate/cancel on a change); no setter.
* **Added:** **226** server `player_set_food(food: s32, player, saturation: f32)` (`player-write`):
  food 0-20, saturation 0 up to food; goes through `for_native_player` like the other player ops and the
  per-mod-per-tick call cap. The hook hears it as any other change. SDK `server::ext::set_food`.
* **Not added:** the optional per-item `item-food-values` hook (a new hook that touches the eating path;
  left for a follow-up).
* **Tests:** argument validation only; the `for_native_player` path needs a running server tick. Needs
  checking in game (including a guest player's HUD sync).

## 7. Cancellable pre-key hook in the world

* **Existed:** `key` (listen, name + pressed, repeats left out) and `input-key` (listen, after handling).
  `input-key` is `unsupported` because its frozen WIT parameter is the ABI `key-event` (`str-view` +
  `pressed`), not the winit event.
* **Added:** hook `key-pre(key, pressed, repeat, modifiers) -> bool` (fragment `key_pre.json`,
  `native_key_pre`, cancel contract). It runs in the generic keyboard arm of `handle_window_event`
  before `native_key_event` and the key bindings, only while playing with nothing over the world
  (`screen == Playing`, no title). Escape is never offered. A cancelled key reaches nothing below it.
* **Not done:** `input-key` mapping fix. It is not trivial: changing the type of an existing function breaks
  the 1.0 baseline; a new `input-key-v2` function would be needed. `key-pre` plus `key` cover the use.
* **Tests:** the hook is covered by the generator's catalog tests (names, WIT, coverage, adapters).
  No test fires a real key event.

## Tests run

* `cargo test -p horizon-mod-api-gen --lib` (`YOAKE_UPDATE_WIT=1` once to regenerate, then plain): 45 passed, including `abi_1_0_is_additive`.
* `cargo check -p horizon-client --lib`: clean.
* `cargo test -p horizon-client --lib` with filters (`native_s1`, `wasm_mods::full::tests` quad, net, inventory, hud, gui_size, hud_place_v2): 11 passed. The full 1242-test suite was not run.
* `cargo check --features full --target wasm32-wasip2` in `crates/yoake-wasm-sdk`: clean.

## Docs

`docs/yoake-wasm-dev.md` (new Japanese and English section "FR-S1"), `docs/guide-ja/09-small-services.md`,
`docs/guide-en/09-small-services.md`, both READMEs.

## Needs checking

* In game: textured quads (rotation, UV), the sound being heard, food on a guest, block-entity write-back
  plus `block-entity-sync`, `key-pre` (consume a key; release handling).
* Hook ordinals in `full_adapters.rs` are renumbered by the new hook (generated); merging with other
  branches means regenerating it again.
* `@API:fr_s1_services@` and `@API:key_pre@` are unstamped fragment tags (the stamp tool replaces them).
* A texture id is not secret: a mod that guesses another mod's `<mod>#<n>` can draw (not write) its texture.
