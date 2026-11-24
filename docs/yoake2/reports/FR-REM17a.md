# FR-REM17a: client draw lists, HUD highlight, item model select, client attachment/mod-call, input v2

Branch `yoake-free/rem17-client`, from `yoake-free/merge` (f244f7ce6).

## Done vs list

| Item | Status |
| --- | --- |
| 0. Template `loot.json` vanilla `conditions` form | Done (commit c9d809acaa). Template now uses `"conditions": [{"condition": "minecraft:survives_explosion"}]`. The loot loader (`horizon-items/src/items.rs`, `condition()`) only read `"type"` on object conditions, so it now reads `"condition"` too (falling back to `"type"`). Test `vanilla_pool_conditions_array_is_honoured`: passes. |
| 1. `ModelCommand::Cube` / `Item` / `Model` host rendering | Not done. The WIT variants already exist (`crates/yoake-wasm/wit/yoake-full.wit`, `model-command`), but `apply_model` in `crates/horizon-client/src/wasm_mods/full.rs` (~line 1101) ignores them. The renderer helpers exist in `crates/horizon-client/src/game/native_block_entity_draw.rs` (`native_draw_cube`, `native_draw_model`, `native_draw_item`). Blocker: the `Host` trait that `apply_model` calls (`host.draw_quad`, `host.draw_block`) is generated from the catalog into `OUT_DIR/mod_api_generated_host.rs` (`crates/horizon-client/src/mod_api.rs`), and I did not find the game's implementation of those methods. Need to trace it before wiring Cube/Model/Item. |
| 2. `HudCommand::Highlight` | Not done. Variant exists (`hud-highlight` in WIT); `full.rs` ~line 943 is a no-op `{}`. |
| 3. `item_model_select` service/hook | Not started. Needs catalog JSON in `crates/horizon-mod-api-gen` plus generator, WIT/SDK/adapters regen. |
| 4. Client `attachment-get` and `mod-call` | Not started. Needs the same generator path; server versions to mirror in `wasm_mods/server_services.rs`. |
| 5. `input_key` v2 (up/down/repeat, modifiers, gamepad) | Not started. `crates/horizon-mod-api-gen/wasm-adapter-denylist.json` marks `input_key` as needing a key-event mapping fix. Gamepad layer exists in `horizon-client-input` (`gamepad.rs`), not checked yet. |

## Tests

* `.\hz-cargo.cmd test -j 6 -p horizon-items --lib vanilla_pool`: 1 passed.
* `.\hz-cargo.cmd test -j 6 -p horizon-items --lib loot`: 6 passed.
* Not run: `horizon-mod-api-gen --lib` (no catalog changes yet), client build, e2e.

## Left

Items 1-5 as above. Items 3-5 each need the catalog/generator/WIT/SDK path and were not started. Item 1 needs the `Host` draw-method implementation located first.

## Update: model commands and HUD highlight (items 1 and 2)

The `Host` trait (generated into `OUT_DIR/mod_api_generated_host.rs` from the catalog and the `native_*` methods of `Game`) already has `draw_quad`, `draw_cube`, `draw_block` and `draw_item`, implemented by `impl Host for Game` (`OUT_DIR/mod_api_host_generated.rs`) over `native_block_entity_draw.rs`. Only `native_draw_model` has no catalog helper, so `Host` has no `draw_model`.

| Item | Status |
| --- | --- |
| `ModelCommand::Cube` | Done: `cube_from` (non-finite corners, a UV list that is not four finite numbers, more than six faces: draws nothing) then `Host::draw_cube` with the current pose, tint masked to RGB. |
| `ModelCommand::Item` | Done: `Host::draw_item` with the stack, display context, finite owner yaw and seed. |
| `ModelCommand::Model` | Partial: no `draw_model` helper exists, and adding one needs a catalog row or fragment (not touched). The id is drawn as a block (`draw_block`, default state) and, if that is no block, as an item (`draw_item`, `Fixed`). A model that is neither (a standalone model named in `standalone-models`) draws nothing. Needs a helper row for `native_draw_model` to be complete. |
| `HudCommand::Highlight` | Done: the variant has only a screen rect and `invert_text` (no world-block form). Tinted fill plus a one-pixel outline; with `invert_text` the fill is the inverting blend. Empty rect: nothing. |

All four go through `apply_model`/`apply_hud`, so mob, block entity, player and item draw lists get them. The first applied command of each kind logs `wasm: model command cube|item|model applied`.

Tests (`wasm_mods/full.rs`): `cube_item_and_model_commands_reach_the_draw_services`, `malformed_cubes_and_stacks_draw_nothing_and_no_host_is_a_no_op`, `a_highlight_is_a_tint_with_a_one_pixel_outline`. `horizon-client --lib --features yoake-hook-bench,yoake-wasm/test-guests wasm_mods::full`: 31 passed.

Example: `fox_texture` now also listens to `player-render-listened` (mob layers are only asked for while a mod does) and `mob-layer`, drawing a gold cube and an apple over the Cinder Fox. E2E: new `model_commands` check (log lines for cube and item, screenshot `f4m6-model-commands.png`); with `--wasm`/`--client-wasm` all checks pass.
