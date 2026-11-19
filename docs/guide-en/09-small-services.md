# 9. Screen, inventory, sound and food services

This chapter gathers a few small services of `client-full` and `server-full`.
None of them changes anything unless a mod uses it. The full rules are in "FR-S1" of `docs/yoake-wasm-dev.md`.

---

## 9-1. Sending from screen input hooks

`net-send-to-server` can be called from the screen key and mouse hooks too:
`screen-key-pressed-pre`, `screen-char-typed-pre`, `screen-mouse-*` and `screen-click`.
At most 32 messages go out in a tick.

## 9-2. Drawing turned textures

A `hud-command::quad` with a `sprite` pastes that sprite on the quad.
`sprite` is a GUI sprite name, or `texture:<id>` for a GUI texture.

1. Get the `id` with `client::gui_texture_id(&texture)`.
2. Make the turned corners with `client::rotated_corners(x, y, w, h, degrees, pivot)`.
3. Return `HudQuad { corners, sprite: Some(format!("texture:{id}")), uv: None, color }`.

`hud-command::texture` draws with the same `id`.
It also works in the commands `item-decorations` returns.

## 9-3. Reading inventories

- `client::screen_carried()` is the stack on the cursor while a screen is open.
- `client::local_player_stacks()` is the local player's stacks: hotbar, main inventory, armor, then the off hand. Empty slots are left out.

## 9-4. Client-only sound

- `client::play_local_sound("namespace:path", volume, pitch)` is a UI sound with no position.
- `client::play_local_sound_at(position, "namespace:path", volume, pitch)` is a sound in the world.

Volume is 0 to 16 and pitch 0 to 8. The sound plays in the next frame.

## 9-5. Setting food

A server mod can call `server::ext::set_food(player, food, saturation)`.
Food is 0 to 20. Saturation never goes past food.

## 9-6. Changing a block entity

The `entity` that `block-entity-tick` and its siblings return is written by the engine.
Call `block-entity-sync` when clients should see the change.

## 9-7. Consuming a key

`key-pre` gets a key while playing, before the key bindings.
Return `true` and the key is gone.
If you consume a press, consume its release too. Escape is never offered.
