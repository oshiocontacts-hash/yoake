# 9. Replacing Item and Block Sprites at Runtime

In a `client-full` mod, you can replace an item icon or a block texture (as RGBA8 pixels) while the game is running. Only the changed part is updated, without rebuilding the whole atlas.

> [!NOTE]
> This only changes how the game looks on your own client (HUD, held items, dropped items, and blocks in the world). Other players' screens are not affected.

---

## 9-1. Available Host Services and SDK Helpers

`yoake_wasm_sdk::full::client::sprites` wraps the six `client-services` functions in natural argument order:

| SDK helper (`sprites::*`) | WIT service (`client_services::*`) | Role |
|---|---|---|
| `set_item(item_id, (width, height), rgba)` | `item_sprite_set(item_id, rgba, (width, height))` | Overwrites only that item's rectangle in the shared atlas with `rgba` (the original image is kept internally). |
| `reset_item(item_id)` | `item_sprite_reset(item_id)` | Restores the item's sprite from the current resource pack. |
| `set_block(texture_id, (width, height), rgba)` | `block_sprite_set(rgba, (width, height), texture_id)` | Overwrites a block model texture layer such as `"mymod:block/pan"` or `"block/stone"` (`"minecraft:block/stone"`), and regenerates all mipmap levels. |
| `reset_block(texture_id)` | `block_sprite_reset(texture_id)` | Restores the block texture layer from the current resource pack (including animations). |
| `item_pixels(item_id)` | `item_sprite_pixels(item_id)` | Returns the item's current sprite as `(width, height, rgba)` (after `set_item`, returns the override; animated sprites return frame 0). |
| `block_pixels(texture_id)` | `block_sprite_pixels(texture_id)` | Returns the block texture's current image as `(width, height, rgba)` (animated textures return frame 0; size is one layer of the texture array). |

- **`rgba` format**: **RGBA8**, 4 bytes per pixel (`[r, g, b, a, ...]`, top row first, left to right). The length must equal `width * height * 4`.
- **Size mismatch**: If `size` does not match the actual resolution of that sprite or block texture array layer (or is `0`, or the byte length is wrong), the call returns an error and leaves the atlas unchanged. There is no automatic resizing.
- **Namespaces**: You can override sprites from your own mod, vanilla, or another mod. If another mod overrides the same ID later, the later write wins. When your mod unloads or reloads, only the overrides still owned by your mod are reset automatically.
- **Resource pack reload (`F3+T`, etc.)**: Your overrides are kept and re-applied to the new pack. If the new pack changed that sprite's resolution, the engine logs a warning and skips that override instead of crashing.

---

## 9-2. Rust Example (`client-full`)

The full example is in [`crates/yoake-wasm-sdk/examples/sprite-demo/`](../../crates/yoake-wasm-sdk/examples/sprite-demo/src/lib.rs).

```rust
use yoake_wasm_sdk::full::client::sprites;

fn apply_tint_icon() -> Result<(), String> {
    // 16x16 RGBA8 solid color texture (16 * 16 * 4 = 1024 bytes)
    let mut rgba = Vec::with_capacity(16 * 16 * 4);
    for _ in 0..(16 * 16) {
        rgba.extend_from_slice(&[255, 80, 40, 255]);
    }
    sprites::set_item("minecraft:diamond", (16, 16), &rgba)?;
    sprites::set_block("block/stone", (16, 16), &rgba)?;
    Ok(())
}

fn restore_sprites() -> Result<(), String> {
    sprites::reset_item("minecraft:diamond")?;
    sprites::reset_block("block/stone")?;
    Ok(())
}
```

> [!TIP]
> You can also call `yoake_wasm_sdk::full::bindings::yoake::mods::client_services` directly, but its argument order is alphabetical per WIT parameter name. `full::client::sprites` keeps `(id, (width, height), rgba)`.

---

## 9-3. Things to Watch Out For

- **A block's item icon is a separate copy**: The icon shown in inventories lives in the item atlas, separate from the block texture array. To change both the placed block and its inventory icon, call both `set_block` and `set_item`.
- **Animated items**: While `set_item` is active, HUD atlas animation updates pause for that rectangle so your override stays visible. Calling `reset_item` (or unloading the mod) resumes animation from the current frame.
- **Animated block textures**: While `set_block` is active, tick animation updates pause for that layer. Calling `reset_block` restores the original frames and animation.
- **Per-stack overrides (by NBT or custom data)**: Dynamic per-stack runtime sprites (`item-stack-sprite`) are not implemented yet, because they would need dynamic slots in the shared atlas. To vary an item's look per stack, use vanilla item model definitions (`assets/<ns>/items/*.json`) with `minecraft:custom_model_data` or `minecraft:custom_data`.

---

## 9-4. Reading the Current Pixels (`item_pixels` / `block_pixels`)

- Reads come from the CPU copy of the atlases already prepared on the client. No GPU readback happens, and nothing extra is stored when mods do not call them. The intended pattern is to read once per item and cache the result.
- The returned bytes are `(width, height, rgba)` in the exact RGBA8 layout accepted by `set_item` / `set_block`. For blocks, `width` and `height` match one layer of the block texture array (`sprite_size`).
- Unknown IDs return an error.
