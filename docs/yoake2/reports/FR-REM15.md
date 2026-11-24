# FR-REM15: behaviours of a mod's own blocks, items, menus and recipe types; block entity data by position

Branch `yoake-free/rem15-behaviors` (from `yoake-free/merge` e53d51c4b). Audit rows 1, 2, 4, 13, 15 of GAP3 and remaining-work
item REM-15. Guide chapter: `MOD-GUIDE.md` / `MOD-GUIDE.ja.md` section 19.

## Design

The audit's gap was that the WASM adapter of `blocks`, `items`, `block_behaviors`, `item_behaviors`, `menu_behaviors` and
`recipe_types` "holds live engine objects" and applied nothing. The engine already has these as **traits** for native mods
(`BlockBehavior`, `ItemBehavior`, `MenuBehavior`, `NativeRecipeCodec`), looked up by id from fixed places. The change gives a WASM
package those same objects, built by the host from a registration, so no new engine call site was needed for use, ticks, contact,
items, menus and recipes:

1. A server module calls the new service `behavior-register(kind, id, hooks)` (292) from `init` (a package's own ids only, hook names
   checked, the table frozen once the game has read it; a world start re-running `init` repeats the same registration harmlessly).
2. `native_block_behaviors`, `native_item_behaviors`, `native_menu_behaviors` and `native_recipe_types` (the game's collectors) append
   `wasm_mods::behaviors::{block,item,menu}_behaviors()` / `recipe_types()` to the native mods' own. A game with no registration appends
   nothing, and the engine's tables stay `None` as before.
3. When the engine reaches one of those places for a registered id, the host object calls the package's `handle-call` export with the
   caller `yoake:engine` and the method `yoake:<kind>/<hook>` (the F3-8 chunk generator's pattern), under the guest's own per-call limits.
   The wire is a small binary format (documented at the top of `wasm_mods/behaviors.rs`, mirrored by the SDK's `behaviors` module); an
   answer that does not check changes nothing.
4. A hook with no game access of its own (level ticks, block observed changing, entity contact) answers **commands** (`set-property`,
   `schedule-tick`, `play-sound`; at most 64, within 32 blocks) which the host checks and applies: level ticks at once through
   `LevelWriteHost`, the others at the next tick of the same level (`native_level_tick_pre`). A hook with a `Host` (use, items, menus'
   create/closed/quick-move) uses the ordinary services.

## Done against the goal

| Goal | State |
| --- | --- |
| 1. Block behaviour for a mod block id: use, placed, broken/removed, neighbour changed, random / scheduled tick, entity inside / stepped on | Done. `use`, `neighbor-changed`, `random-tick` (the block then ticks at random), `scheduled-tick`, `entity-inside`, `step-on` are the engine's existing `BlockBehavior` calls; `placed` and `removed` are two new default-bodied trait methods fed by `native_block_changed` (a change of state alone is neither). Called only for the registered id; client-side calls return at once. |
| 2. Item behaviour: use, use on block, use on entity, finish using, inventory tick | Done through `ItemBehavior` (`use`, `use-on-block`, `use-on-entity`, `finish-using` with a replacement stack, `inventory-tick`). |
| 3. Menu behaviour: slot click, quick-move, may-place, on-close | Done. `create` (the guest gives the menu's size and first contents; the host keeps the storage), `may-place` (the storage's `may_place`), `still-valid`, `closed`, `clicked` (a new defaulted `ItemHandler::clicked`, called by `Menu::click` after the transaction stored; the guest may set slots) and `quick-move` (a new defaulted `MenuBehavior::quick_move`, called by `apply_data_quick_move`; true skips the engine's move). |
| 4. Block entity: position-addressed `block-entity-read` / `-patch`, mod block entities' data persisting | Done as services `block-entity-read-at` (293) and `block-entity-patch-at` (294): any block entity's type id and saved data as one network-NBT compound (at most 64 KiB), merged like `/data merge block` (replace for a mod block entity only), chunk marked saved, update tag synced. The old handle-based `block-entity-read` / `-patch-call` keep answering "unsupported" (ABI 1.0 baseline). A mod block entity's `nbt`, `items` and `data` already persisted with the chunk (`save.rs` round trip test); nothing new was needed for that, and the new service reads it. |
| 5. Recipe types: a mod-defined type whose match/assemble runs in the mod, usable by a mod menu | Done. A `recipe` object is registered with `describe` (version, category, icon), `matches`, `assemble` and optional `decode` and `display`; the host builds a `NativeRecipeType` whose codec calls the guest, so recipe files naming `"type": "<id>"` load, match and show in the recipe book as native types do. Menus use them through `recipe-match` (295, for a type of another package) and `recipe-data` (296: the loaded recipes of a type with their data, for the package that defines it). See "Re-entrancy" below for why there are two. |

## Re-entrancy (what the design cannot do)

The engine cannot call a package that is running a hook (its instance is borrowed by the call in progress). Consequences, all handled by
returning the object to its un-hooked behaviour and counting the drop once (`behaviors::invoke`, same rule as the catalog's hooks):
a guest hook whose own service causes another hook of the same package (it sets a block, the neighbour update returns to the same module)
has that second call dropped; `recipe-match` refuses a recipe type of the calling package with an error that points to `recipe-data`
(a guest matches its own recipes with its own code).

## Files

- Catalog: `crates/horizon-mod-api-gen/abi-subset-server.json` (services 292 to 296, types `block-entity-data`, `recipe-match`,
  `recipe-data`, server interface 1.7.0), regenerated `crates/yoake-wasm/wit/yoake-full.wit` and the committed generated files, the
  `server_services!` lines of `crates/yoake-wasm/src/full.rs`.
- Host: `crates/horizon-client/src/wasm_mods/behaviors.rs` (table, wire, block, item, menu, recipe objects; tests in
  `behaviors/tests.rs`), `server_services.rs` (the five services), `full.rs` (visibility only), `game/native_block_behaviors.rs`
  (`placed`, `removed`, `changed`), `game/native_data_menus.rs` (`MenuBehavior::quick_move`), `native_inventory.rs` and `menu.rs`
  (`ItemHandler::clicked`), `game/native_hooks.rs` (collectors, `drain`, `changed`), `game/native_block_entity_data.rs` (new),
  `game/native_recipes.rs` (`native_recipe_data`), fragments `tools/native-hook-fragments/block_entity_data.json` and `recipe_types.json`.
- Guest: `crates/yoake-wasm-sdk/src/full_behaviors.rs` (wire and the `Behaviors` trait), `full.rs` (`ext::register_behavior`,
  `block_entity_read/merge/replace`, `recipe_match`, `recipe_data`), the example `crates/yoake-wasm-sdk/examples/ruby-behaviors`
  (lamp, wand, vault menu, alloy recipe type, content files), `yoake-wasm/src/test_guests.rs` (`RUBY_BEHAVIORS`).
- Tools and docs: `tools/behaviors_e2e.py`, `docs/yoake2/MOD-GUIDE.md` and `.ja.md` section 19, `docs/native-mods.md`.

## Tests

- `horizon-mod-api-gen --lib`: **46 passed** (the generated WIT, SDK bindings, adapters and `abi_1_0_is_additive`; services 292 to 296 do not touch the
  1.0 ids). Regenerated with `YOAKE_UPDATE_WIT=1`.
- `horizon-client --lib --features yoake-hook-bench,yoake-wasm/test-guests`: **1280 passed** (the whole library, run before the block entity tests below were added; those 4
  passed on their own, 1284 in all), 0 failed, 53 ignored (need prepared resources); 112 s for the
  12 new tests in `wasm_mods::behaviors` alone (they build and run the real `ruby-behaviors` guest):
  the wire (`answers_are_checked_before_anything_is_applied`: version, result, extra bytes, reach, delay, count, NaN, long names),
  registration (`registrations_are_checked_and_stay_the_same_across_worlds`, `a_recipe_type_must_describe_itself_to_be_registered`),
  the objects the engine builds (`the_guest_registers_only_its_own_objects_and_the_engine_builds_their_behaviours`),
  block hooks (`block_hooks_reach_the_guest_for_its_own_block_and_apply_the_answers`: `use` flips `lit` through the services of the hook's host,
  a client's call does nothing, tick commands reach the level, contact commands wait for their level's tick and are applied once;
  `placed_and_removed_blocks_are_told_to_the_guest`), items (`item_hooks_reach_the_guest_and_the_answer_leaves_the_stack`: use, use on block,
  on entity, finish with a replacement stack, inventory tick), menus (`a_menu_the_guest_creates_stores_what_it_allows_and_reacts_to_clicks`:
  create, may-place, a click that crafts, closed; `a_menu_asks_the_recipe_type_in_a_hook_that_has_game_access`: shift-click uses `recipe-data`
  and the type's own matching), recipe types (`a_recipe_type_of_the_guest_decodes_matches_assembles_and_displays`,
  `recipe_match_refuses_the_package_that_defines_the_type_and_recipe_data_serves_it`) and the busy or missing guest
  (`a_busy_or_missing_guest_leaves_the_object_as_it_was`).
- Block entity data by position (`game/native_block_entity_data.rs`, 4 tests): a mod block entity reads as its type and its own data; a merge keeps
  what the patch does not name, keeps the slots, and persists through `encode_block_entity` / `decode_block_entity` (what the world saves is what the
  guest reads next); a replace is a mod block entity's only (a vanilla one is merged into) and keeps its type and place; data nested too deep is
  refused. The `Game` methods around them (`native_block_entity_data`, `native_block_entity_patch`) and the two services were not run in a game: building
  a `Game` needs prepared resources.
- `horizon-client-lib --lib`: **1966 passed**, 0 failed, 48 ignored.
- `cargo build -p horizon-client -p horizon-client-lib --bin horizon-client --bin horizon-ctl`: succeeded.
- E2E `tools/behaviors_e2e.py --bin-dir target/debug --wasm target/wasm32-wasip2/release/yoake_wasm_ruby_behaviors.wasm`, a real game (LAN host, offscreen,
  unfocused) with the `ruby` package installed: all four checks pass. `registered` (the module loaded, `ruby:lamp` places with `lit`), `use` (the
  engine's right-click reaches the guest, which flips `lit` through `block-property` / `set-block-property` and tells the player each time; the lines
  alternate and the block ends as the last says), `wand` (using the item in the air makes the guest tell the player), `vanilla` (a game without the
  package refuses `ruby:lamp` and a lever still works). The same package in a plain single-player window (no LAN) called nothing, as designed
  (`native_authoritative` is false there).
- Not run: `tools/pr_ready.py`, clippy, `hook-ledger` verification, any e2e of `placed`/`removed`, contact, the menu or the recipe type in a real game
  (covered by the host tests with the real guest only), the full `horizon-client` e2e suites.

## What is left

- Service ids 292 to 296 and interface 1.7.0 were taken from the next free numbers of this branch; a parallel branch that also adds
  services has to renumber one of them when merging.
- The hook ledger (`hook-ledger/`) and the callsite verification of CI were not run (CI-only); the new helper functions are plain `native_*`
  helpers of fragments.
- No behaviour for fluids or entities; no capability providers by hook (`capability_providers` of the adapter report stays unapplied);
  `blocks` / `items` registration by the guest (creating content that has no data file) is still data-only.
- A block with no vanilla template row (REM-16 left it too).
- `placed` / `removed` and entity contact cannot write the world in the same tick: their commands wait for that level's next tick.
- Menus: `clicked` is called without game access (the menu engine has none there), so a guest that needs services after a click does the
  work in `quick-move` or `still-valid`. The `Input` ordinal is passed as the click mode.
- A second client over a dedicated server was not run with these hooks: server-side hooks only, and the client is told by the usual syncs.
- In the e2e one `horizon-ctl interact` reached the guest **twice** (two toggles, four tells for two interacts). The harness presses and
  releases the right button inside one task (the button is held across a frame), so a second use is plausible, but this was not
  confirmed against the engine's own click path; if it is a real double call of `block_behavior_use` it affects native mods as well.
- Behaviours run only where server modules do (a LAN host's integrated server, a dedicated server): a plain single-player window calls none.
