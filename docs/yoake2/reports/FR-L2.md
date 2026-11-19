# FR-L2 Report: Runtime Item & Block Sprites

## Summary
Implemented runtime item and block sprite override and reset support for both WebAssembly (`client-services`) and native client mods, uploading only the changed sub-rectangle (for item sprites) or single mipmapped array layer (for block sprites) without rebuilding atlases. With no mod overrides active, atlases and per-frame rendering remain untouched (zero allocations and zero GPU copy work).

## ABI IDs Added (`crates/horizon-mod-api-gen/abi-subset-client.json`)
Reserved range **211–220**; 211–216 are used (217–220 free, 221–250 are Yoake's). `interface_version` of `abi-subset-client.json` is now **1.3.0** (same bump as L1):

| ID | Service | WIT Signature | Capability |
|---:|---|---|---|
| `211` | `item_sprite_set` | `(item-id: string, rgba: list<u8>, size: tuple<u32, u32>) -> result<_, string>` | `service.hud` |
| `212` | `item_sprite_reset` | `(item-id: string) -> result<_, string>` | `service.hud` |
| `213` | `block_sprite_set` | `(texture-id: string, rgba: list<u8>, size: tuple<u32, u32>) -> result<_, string>` | `service.world-draw` |
| `214` | `block_sprite_reset` | `(texture-id: string) -> result<_, string>` | `service.world-draw` |
| `215` | `item_sprite_pixels` | `(item-id: string) -> result<tuple<u32, u32, list<u8>>, string>` | `service.hud` |
| `216` | `block_sprite_pixels` | `(texture-id: string) -> result<tuple<u32, u32, list<u8>>, string>` | `service.world-draw` |

*(Generated files `yoake-full.wit`, `full_adapters.rs`, `full_adapters_report.txt`, `coverage.json` and `crates/yoake-wasm-sdk/src/full_*_generated.rs` were regenerated with `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib`, not hand-edited. The generator orders service arguments alphabetically, so the WIT import is `block-sprite-set(rgba, size, texture-id)`; the item services happen to keep their order.)*

## Files Created & Modified
- **Created**
  - `crates/horizon-client/src/game/native_sprites.rs` — `SpriteOverrideStore` (lazy original snapshots, validation, reload re-apply) + `Game::native_*` helpers.
  - `crates/horizon-client/src/game/native_sprites/tests.rs` — unit tests covering item sub-rect patching, cross-mod overwrite/ownership, reset, shared still block layers, wrong sizes, and pack reload re-apply/mismatch skip.
  - `tools/native-hook-fragments/sprites.json` — native `Host` trait fragment (`item_sprite_set`, `item_sprite_reset`, `block_sprite_set`, `block_sprite_reset`).
  - `docs/guide-ja/09-sprites.md` — Japanese user guide for WASM sprite overrides.
  - `docs/yoake2/reports/FR-L2.md` — this report.
- **Modified**
  - `crates/horizon-mod-api-gen/abi-subset-client.json` — added ABI IDs `211`–`216`, `interface_version` 1.3.0.
  - `crates/yoake-wasm/src/client_services.rs` — added the four methods to `ClientServices` and `client_services!`.
  - `crates/horizon-client-render/src/gfx/animate.rs` — added `overridden` layer set to `SpriteAnimator` so animated block layers pause while overridden and force frame redraw on reset.
  - `crates/horizon-client-render/src/gfx.rs` — added `Renderer::queue_block_sprite_layer` and `flush_block_sprite_uploads` (regenerating all mip levels with `mipmap::generate` and uploading via `Uploads::texture` + `PackAtlas::copy`).
  - `crates/horizon-client/src/game.rs` — added `native_sprites` and `sprite_patches` fields on `Game` and drained `sprite_patches` alongside `map_patches` in `Game::render`.
  - `crates/horizon-client/src/game/packs.rs` — cleared `sprite_patches` and called `self.reapply_native_sprites()` on `apply_staged_pack`.
  - `crates/horizon-client/src/wasm_mods/client_services.rs` — implemented the four `ClientServices` methods on `GameHost` and recorded per-mod ownership claims.
  - `crates/horizon-client/src/wasm_mods/resources.rs` — tracked claimed item/block sprite keys in `Registry` and auto-reset mod-owned entries on unload/reload + unit test.
  - `docs/guide-ja/README.md` & `docs/yoake-wasm-dev.md` — documented the new APIs.

## How the Atlas Writes Work
1. **Item sprites (`HudAtlas` / Shared 2D Atlas)**:
   - Resolves `item_id` (canonicalized with `minecraft:` default namespace) through `ItemRenderer::resolve(..., DisplayContext::Gui, ...)` quad materials (falling back to `item/<path>` / `block/<path>` if a sprite exists in `gui.atlas.sprites` without an item definition).
   - Computes exact sub-rectangle `(x, y, rect.w, rect.h)` in `gui.atlas`.
   - On first override of a sprite key, lazily snapshots the existing rectangle from `gui.atlas.pixels`. Writes the new RGBA8 pixels into `gui.atlas.pixels` and pushes a `font::Patch` to `Game::sprite_patches`, which is appended to `hud.patches` in `Game::render` and uploaded via `HudRenderer::upload_patches` without altering atlas layout or UVs.
   - Because UVs in `gui.atlas.sprites` remain unchanged, GUI slots, held items, dropped items, and `draw_gui_item` immediately sample the updated region.
2. **Block sprites (`Renderer::sprite_textures` 2D arrays)**:
   - Looks up `texture_id` (canonicalized) in `pack.render.sprite_layers`.
   - Calls `Renderer::queue_block_sprite_layer`, which validates pixel size against `sprite_size`, suppresses `SpriteAnimator` updates on that layer (`set_layer_overridden(..., true)`), regenerates the full mip chain via `crate::mipmap::generate` (preserving the layer's `mipmap::Settings` strategy and cutoff bias with updated transparency), and enqueues only that layer.
   - `Renderer::flush_block_sprite_uploads` runs before rendering passes (and in the occluded-frame upload fallback in `Renderer::render`), uploading each mip level via `Uploads::texture` and syncing `PackAtlas::copy` if a shader pack is active.
   - `block_sprite_reset` re-applies any remaining active override sharing the same layer (if `Sprites::still` deduplicated identical still textures onto one layer) or restores original pixels from `pack.render.pixels` and clears `SpriteAnimator` suppression.

## Policy & Design Decisions
- **Wrong `size` handling**: Refused cleanly (`Err(...)`). `size` must be non-zero, match `[rect.w, rect.h]` (items) or `[sprite_size, sprite_size]` (blocks), and `rgba.len()` must equal `w * h * 4`. No resizing or partial writes occur.
- **Namespace / cross-mod overwrite**: Allowed per owner decision (2026-11-20). Replacing vanilla or another mod's sprite succeeds and transfers ownership of that key in `wasm_mods::resources::Registry` to the latest caller. Unloading/reloading a WASM mod resets only keys currently owned by that unloading mod instance.
- **Resource-pack reload**: Active overrides are stored in `Game::native_sprites` and re-applied in `packs::apply_staged_pack` (`reapply_native_sprites`) at the new UV rects / layers. If a reloaded pack drops a sprite or changes its resolution (e.g., 16x16 override on a 32x32 pack), the mismatch is logged and skipped without panicking; the override stays stored in case a matching pack is selected later.
- **Per-stack variant (`item-stack-sprite`) decision**: **Not built.** `BakedItemModel`, `ItemRenderer::gui_drawn`, and GPU `SkinModels` key cached geometry and vertex buffers by static model sprite names and normalized atlas UVs without a per-item dynamic slot pool. Adding runtime per-stack sprite swapping without rebuilding `HudAtlas` would require a pre-allocated LRU sprite pool plus invalidation/keying hooks across GUI, hand, and entity item caches. Vanilla `minecraft:custom_model_data` / `minecraft:custom_data` item model selectors already cover per-stack model variants cleanly.

## Tests Added
Pure-Rust unit tests (no GPU adapter or `.local/` assets required):
- `horizon_client::game::native_sprites::tests::item_sprite_set_overwrite_and_reset_updates_only_subrect`
- `horizon_client::game::native_sprites::tests::wrong_size_and_invalid_inputs_are_refused_without_mutating_atlas`
- `horizon_client::game::native_sprites::tests::block_sprite_shared_layer_and_reset_restores_originals`
- `horizon_client::game::native_sprites::tests::resource_pack_reload_reapplies_at_new_rect_and_skips_mismatched_sizes`
- `horizon_client::wasm_mods::resources::tests::cleanup_resets_only_sprites_still_owned_by_unloading_mod`

## In-Game Checks for Yoake
Run a test client with a `client-full` WASM mod (or native mod) exercising `item_sprite_set/reset` and `block_sprite_set/reset`, and verify:
1. **Inventory icon**: Hotbar, creative tab, and inventory slots reflect the new item sprite immediately; neighboring atlas icons show no bleeding.
2. **Held item**: First-person hand and third-person held item show the updated sprite.
3. **Dropped item**: Throwing (`Q`) the overridden item renders the updated texture on the item entity in-world.
4. **Placed block**: Overriding e.g. `minecraft:block/stone` or a mod block texture (`mymod:block/...`) updates placed blocks in-world with crisp/smooth mipmaps as distance changes (no solid-color corruption at distant mip levels).
5. **After resource-pack reload**: Press `F3+T` (or toggle a resource pack); verify matching-size mod overrides remain active after reload (and check logs if switching between 16x and 32x packs).
6. **After `reset`**: Calling `item_sprite_reset` / `block_sprite_reset` restores the exact resource pack texture (including resuming animated block textures if overridden).
7. **After mod unload/reload**: Unloading/disabling the WASM mod resets only that mod's still-owned sprites without clearing sprites owned by another active mod.

## Open Questions / 要確認
- **Generated bindings & build verification**: Need Claude to run `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib` and workspace builds/tests (see `## Commands for Claude` in `.lane/TASK.md`).
- **Animated GUI items in `HudAtlas`**: Static items stay overridden across frames; if a mod overrides an item whose texture also animates inside `Renderer::hud.sprite_animations` (e.g., compass/clock frames in the HUD atlas), `hud.animate_sprites` may overwrite its sub-rect on later ticks because `hud.rs` animation suppression for individual `HudAtlas` rects was left untouched (only `SpriteAnimator` for block textures suppresses overridden layers).
- **WASM `#[cfg(test)]` guest coverage**: No WASM example mod guest was compiled in this pass; optionally add a sprite test call into `crates/yoake-wasm-sdk/examples/client-services-demo` once WIT bindings are regenerated if CI guest coverage is desired.

## Round 2: reading sprite pixels (`item-sprite-pixels` 215, `block-sprite-pixels` 216)

- **CPU copy, not GPU readback.** Items read the rect from `gui.atlas.pixels`; blocks read the layer from `pack.render.pixels` (both already exist for atlas and mip building, so nothing new is kept, allocated or done per frame; work happens only inside the call: one rect or layer copy). A readback would stall the GPU, need a staging buffer and could not see a pack's first frame anyway.
- Item: first resolved atlas sprite of the item (same resolver as `item-sprite-set`); returns that rect's size. Includes a mod override because `item-sprite-set` writes into `gui.atlas.pixels`. If the pack stores an animated item as its first frame in the atlas copy (as far as the code shows), that is what is returned. Items with several layers return the first layer only.
- Block: an override stored for this id (or for another id on the same dedup layer) is returned, else the pack's initial frame of the layer. Size is `sprite_size` x `sprite_size` (the array layer size; textures of other native resolutions are already scaled up), exactly what `block-sprite-set` accepts, so read, edit and write back is a round trip.
- Errors: unknown id, bad id, atlas or pack pixels not available (`finish_without_pixels`).
- Capabilities: `service.hud` for items, `service.world-draw` for blocks (same as the setters). Host helpers: `native_item_sprite_pixels` / `native_block_sprite_pixels` in `native_sprites.rs`, returning `([w, h], rgba)`; WASM adapters in `wasm_mods/client_services.rs`; fragment entries in `tools/native-hook-fragments/sprites.json`.
- Tests: `item_pixels_read_the_atlas_rect_including_a_mod_override`, `block_pixels_give_override_then_shared_layer_then_pack_first_frame`.

## Version bump (1.2.0 to 1.3.0)
`interface_version` in `abi-subset-client.json` is metadata: the generator does not read it, no generated file contains it, and `abi-1.0-baseline.wit` / `abi-1.0-ids.json` contain neither 199/200 nor 211-216 (the 1.0 snapshot is not touched). The WIT package stays `yoake:mods@1.0.0`. Nothing else was found that needs a change; `abi-subset-gpu.json` stays 1.2.0.

## Which texture do held, dropped and framed items use?
From the code: held items (`game/hand.rs`), item frames (`game/frames.rs`), `item_draw.rs` and armor-stand items all look sprites up in `self.gui.atlas.sprites`, i.e. the same HUD atlas texture that `item-sprite-set` patches. Dropped item entities go through `ItemRenderer` / `submit_skinned` with `gui.atlas.sprites` too. I did not trace the final GPU bind of every one of these paths, and I am not sure about **block items** (a block shown as an item): their model sprite is the `block/...` entry **in the HUD atlas**, so `item-sprite-set` on that item changes the item view only, and `block-sprite-set` changes the placed block only (the two are separate copies). Verify in game.

## Additional in-game checks for Yoake
- Held item in first person and third person; item in an item frame; dropped item on the ground.
- Enchanted glint on an overridden item (glint is an overlay on the same quads; it should still show over the new pixels).
- A block texture whose layer is shared with another texture (dedup case, e.g. two identical still textures): override one name, see both change; reset one while the other is still overridden, the layer must keep the other override.
- An animated block (water, fire): override it, the animation must stop; reset, it must resume.
- Resource-pack reload (F3+T or switching packs) with an override active: it must stay (same size) or log a skip (size changed).
- A block item (e.g. stone in inventory): which of `item-sprite-set` / `block-sprite-set` changes it (see above).
- `*-sprite-pixels` on an untouched item and block, then after a set (must return the new pixels), then after a reset (the original again); on an animated block (first frame).

## Additional open questions
- The generator must accept `result<tuple<u32, u32, list<u8>>, string>` and the helper return type `Result<([u32; 2], Vec<u8>), String>` in `sprites.json` (not tried: no generator run).
- Animated items in the HUD atlas: whether `gui.atlas.pixels` always holds the first frame is read from the code, not tested.
- `item-sprite-pixels` returns only the first layer of a multi-layer item.

## Results on LENOVO (2026-11-20, rustc 1.100.0)

Written by agy (Gemini 4 Hydrogen, round 1) and Sonnet (round 2: ids 215/216, version bump, report items) after agy's
quota ran out; builds and tests by Claude. Round 1 compiled and passed on the first try.

| suite | result |
| --- | --- |
| `horizon-mod-api-gen --lib` (after `YOAKE_UPDATE_WIT=1`) | 45 passed |
| `yoake-wasm --lib --tests` | 68 + 6 + 2 + 5 + 1 + 9 passed |
| `horizon-client-render --lib` | 1453 passed, 40 ignored |
| `horizon-client --lib` (incl. `native_sprites::tests`, resources cleanup) | 1203 passed, 40 ignored |

Nothing was rendered: every visual claim above is from reading the code and must be checked in game (list above).
