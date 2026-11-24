# FR-REM16: data content kinds (enchantment, damage_type, game_rule, fluid), integer block states

Branch `yoake-free/rem16-content` (from `yoake-free/merge` 8cdf2847e). Audit rows 1, 17, 25, 26 of GAP3 and remaining-work item REM-16.

## Done against the goal

| Goal | State |
| --- | --- |
| 1. `enchantment` and `damage_type` kinds | Done. `data/<id>/horizon/{enchantment,damage_type}/<path>.json` in vanilla's data-pack JSON, checked at load (vanilla's field ranges, unknown keys refused). The content root writes them to `data/<id>/enchantment/` and `data/<id>/damage_type/`, which every reader now reads beside `data/minecraft/`: the enchantment table (`horizon-items` `Enchantments::load_with`, with the mod's `en_us.json` for the name), the loot loader (`LootData`), the commands' id lists (`command-registries.json`, `/enchant`, `/damage`), the resource-id lists of the chat, and the configuration-phase registry sync (`lan/data.rs`: the entries and their data are always sent, in the loader's order). A damage type also joins `damage-catalog.json`, the table the health code reads. |
| 2. `game_rule` | Done. `data/<id>/horizon/game_rule/<path>.json` (`bool` or bounded `int`, default, category). Registered once at start in `horizon-client-core` `gamerules` (`register_mod_rules`) under `<id>:<path>`; `/gamerule` (the command tree has the namespaced literal), the rules screen, the dedicated management API, the `GameRuleValues` packet and the world save use it. A WASM `world-control` `set-game-rule` accepts a registered namespaced name. Saved in `game_rules` saved data only when it differs from the default. |
| 3. `fluid` | Done. `data/<id>/horizon/fluid/<path>.json` becomes a `horizon_world::fluid::custom::FluidDefinition` (what the WASM `fluids` registration gives). Blocks, bucket and textures must be the mod's own files and declarations. The fluid registry now takes several sources: `install` extends it instead of refusing a second call, so native mods' fluids and the files' share one registry. |
| 4. Int block properties, non-template block | Int done: `{"type": "int", "min", "max"}` (`IntegerProperty`: 0 or more, below `max`, at most 256 values), listed as an integer range in the model schema. Non-template block: **partly**. `"template": "yoake:cube"` is an alias of stone's single state (a plain full cube whose stats and files the block's own properties replace). A block with no vanilla row at all is not done: the engine's block tables (`facts.json`, collision, occlusion, behaviours, 15 more) are indexed by vanilla's states and every block's rows are copies of a template's. |
| 5. Tests | See below. |
| 6. Docs | `docs/mods.md` (kinds, int properties, plain cube), `MOD-GUIDE.md` and `MOD-GUIDE.ja.md` section 18. |

## Files

- `crates/horizon-mods/src/content_kinds.rs` (new): the four kinds' types, parsing and checks. `content.rs`: folder dispatch, cross-checks of a fluid against the mod's blocks, items and texture files, `StateProperty::Int`, `PLAIN_CUBE`.
- `crates/horizon-client/src/mod_content_kinds.rs` (new) and `mod_content.rs`: writing the files, the damage catalog, the command id lists, the model schema for int properties; registration data for rules and fluids.
- `crates/horizon-client/src/game/mods.rs`: `register_game_rules`, `register_fluids` at content-root time.
- `crates/horizon-client-core/src/gamerules.rs`, `crates/horizon-command/src/vanilla.rs`, `save.rs`, `game_rules_screen.rs`, `commands.rs`, `lan_actions.rs`, `dedicated/management.rs`, `wasm_mods/server_services.rs`: mod rules by their own names.
- `crates/horizon-items/src/enchant.rs`, `crates/horizon-world/src/loot.rs`, `crates/horizon-client/src/game/chat.rs`, `lan/data.rs`: read the mods' namespaces.
- `crates/horizon-world/src/fluid/custom.rs`: a registry that several sources extend.
- `tools/content_kinds_e2e.py` (new): one real game with a fixture mod using every kind.

## Tests

- `horizon-mods` (73 tests): per-kind parse and error cases (`content_kinds.rs`: enchantment ranges, slots, holder sets, unknown keys; damage type enums and exhaustion; rule types, bounds, categories, names; fluid defaults and limits), the folder dispatch with a whole fixture mod (`registry_kinds_are_read_from_the_horizon_folder`: missing block, bucket or texture named in the error, subfolder ids), integer properties (`blocks_declare_integer_state_properties`) and the plain cube.
- `horizon-client-core` `gamerules` (3): mod rules register once, vanilla's list is unchanged before and after, bounds and types, the NBT key is the rule's own id and round-trips.
- `horizon-world` `fluid::custom` (5): two sources join one registry, sorted, and a shared name is refused.
- `horizon-client-lib` `mod_content` (50, plus the 24 ignored ones, all passing, run against the real prepared resources through a `.local` junction): `build()` over a small base writes the data-pack files, the damage catalog entry (read back by `damage_source::Catalog`), the command id lists, and leaves a root with no such mod identical; the world-control line refuses an unregistered namespaced rule.
- `horizon-command` (38), `horizon-items` (63): unchanged and passing.
- E2E `tools/content_kinds_e2e.py` (one game with a fixture mod, then one without), all six checks pass: `/gamerule ruby:gem_rate` set, query, past max refused; `/enchant @s ruby:keen 2` on a held sword shows `Keen II` and the component; `/damage @s 1 ruby:shatter`; `/setblock ruby:crop[age=3]` keeps the integer property and `age=9` is refused; `ruby:oil` (a fluid's block) places; the game without the mod refuses every one of them.

## What is left (not done, or not verified)

- A block with no vanilla row at all (goal 4, second half). The engine's block tables are indexed by vanilla's states and each mod block's rows are copies of a template's. `yoake:cube` (stone's state) covers the plain full cube; a real templateless block needs the table writers (`facts`, collision, occlusion, behaviours, `survival-*`, `block-properties.nbt`, ...) to take a synthetic row.
- Damage type and enchantment **tag files** of a mod (`data/<id>/tags/damage_type/`, and enchantment tags of a mod namespace, which `Enchantments::load_with` reads from `data/minecraft/tags/enchantment` only): the damage catalog reads the prepared `damage-tags.json` only, so a mod's damage type is in no tag (nothing like `#minecraft:is_fire` can include it) until the tag merge covers that bundle. An `exclusive_set` naming a tag therefore needs a vanilla tag or an explicit list of ids.
- A mod's `data-pack-files` use of the registries from **world** data packs (the first goal line's "if the engine already reads world data packs ... reuse that path"): it does not. The engine reads world packs for dialogs and clocks only (`PACK_REGISTRIES`), so mods use the content-root path instead.
- Creative inventory: enchanted books for a mod's enchantments are not added to `creative-inventory.json`.
- Multiplayer of a **second** client was not run: the registry sync sends the mods' entries with their data and the joiner needs the same mods (the existing content identity check); the command tree a dedicated server sends (`commands.json`, vanilla's report) has no literal for a mod rule, so a client's tab completion of `/gamerule ruby:...` is missing even though the server runs it.
- Rules and fluids are registered once from the startup root: a pack reload that changes them warns to restart (as mob effects do).
- Fluid physics beyond what the WASM `fluids` registration already gave (the blocks of a data fluid are plain template blocks of `minecraft:water`): a fluid's buckets and its spread were not played in the e2e, only its block placed.
- `capability_providers` (GAP3 row 17's energy part) is untouched.

