# Survey of crate `horizon_client_lib`: the classifier over every file module

Generated; nothing is bound by this. Each module is classified as if its own glue were included in it (private items of other modules are not reachable). Derived trait impls are left out of the denominator.

- modules: 275
- functions: 8530
- bound now: 2557 (30.0%): 1344 basic + 1213 mechanical
- handle-required: 2553 (29.9%)
- excluded: 3420 (40.1%)
- fields with accessors: 3218 of 4637

## Handle-required: most frequent reasons (names generalised)

| reason | occurrences |
|---|---:|
| field `_` of `_` is private to another module [&argument] | 649 |
| field `_` of `_` is private to another module [&mut argument] | 610 |
| external type `_` (crate not loaded) [&argument] | 455 |
| external type `_` (crate not loaded) [return value] | 374 |
| external type `_` (crate not loaded) | 239 |
| std type `_` has no codec yet [&argument] | 215 |
| a reference nested inside another type needs a handle [return value] | 174 |
| field `_` of `_` is private to another module [return value] | 170 |
| external type `_` (crate not loaded) [&mut argument] | 153 |
| std type `_` has no codec yet [return value] | 130 |
| std type `_` has no codec yet | 100 |
| a reference nested inside another type needs a handle [&mut argument] | 84 |
| field `_` of `_` is private to another module | 81 |
| a reference nested inside another type needs a handle | 50 |
| std type `_` has no codec yet [&mut argument] | 27 |
| a reference nested inside another type needs a handle [&argument] | 26 |
| external type `_` (crate not loaded) [Option<&> argument] | 16 |
| field `_` of `_` is private to another module [Option<&> argument] | 12 |
| returns `_`: needs a child handle that locks its parent | 10 |
| a reference nested inside another type needs a handle [by-value self] | 7 |
| external type `_` (crate not loaded) [by-value self] | 4 |
| std type `_` has no codec yet [by-value self] | 4 |
| a reference nested inside another type needs a handle [Option<&> argument] | 3 |
| an unsized slice outside a reference [return value] | 3 |
| a reference nested inside another type needs a handle [returned reference] | 2 |

## Excluded: most frequent reasons (names generalised)

| reason | occurrences |
|---|---:|
| lifetime-carrying type `_` | 1651 |
| field `_` of `_` is private to another module [&mut argument] | 1145 |
| receiver type carries a lifetime: it cannot be a hook-argument handle | 812 |
| generic type alias `_` | 390 |
| `_` values cannot cross the data ABI | 375 |
| `_` requires a listed instantiation | 271 |
| field `_` of `_` is private to another module [&argument] | 269 |
| generic type `_` requires a listed instantiation | 255 |
| impl-level type parameter `_` requires a listed instantiation | 201 |
| external type `_` (crate not loaded) [&argument] | 181 |
| a reference nested inside another type needs a handle [&mut argument] | 127 |
| receiver type is generic: bind a concrete instantiation first | 125 |
| `_` argument requires a listed instantiation | 124 |
| closure argument (`_`): guest callback handles are not built | 115 |
| function-level type parameter `_` requires a listed instantiation | 114 |
| std type `_` has no codec yet [return value] | 107 |
| std type `_` has no codec yet [&argument] | 103 |
| external type `_` (crate not loaded) [return value] | 88 |
| field `_` of `_` is private to another module | 88 |
| external type `_` (crate not loaded) | 86 |
| field `_` of `_` is private to another module [return value] | 69 |
| impl on a type that is not defined in a loaded engine crate | 65 |
| a reference nested inside another type needs a handle [return value] | 55 |
| external type `_` (crate not loaded) [&mut argument] | 55 |
| function pointers and closures cannot cross the data ABI (guest callbacks are not built) | 51 |

## Per module

| module | functions | bound | handle-required | excluded |
|---|---:|---:|---:|---:|
| `mobs::goals` | 242 | 62 | 10 | 170 |
| `motion` | 241 | 41 | 6 | 194 |
| `save` | 187 | 48 | 68 | 71 |
| `mobs::tame` | 170 | 28 | 35 | 107 |
| `mobs::piglin` | 154 | 18 | 30 | 106 |
| `mobs::villager_brain` | 149 | 25 | 43 | 81 |
| `lan::hub` | 145 | 77 | 43 | 25 |
| `mobs::bee` | 134 | 27 | 35 | 72 |
| `mobs` | 130 | 92 | 29 | 9 |
| `mobs::panda` | 124 | 17 | 50 | 57 |
| `title` | 123 | 63 | 55 | 5 |
| `health` | 113 | 16 | 8 | 89 |
| `arrows` | 106 | 20 | 7 | 79 |
| `mobs::finalize` | 105 | 26 | 40 | 39 |
| `mobs::fox` | 105 | 13 | 23 | 69 |
| `entities` | 101 | 55 | 32 | 14 |
| `menu` | 99 | 14 | 20 | 65 |
| `mobs::animal_brain` | 98 | 18 | 28 | 52 |
| `mobs::undead` | 98 | 24 | 7 | 67 |
| `mobs::living` | 97 | 34 | 4 | 59 |
| `raid` | 87 | 68 | 10 | 9 |
| `entity_nbt` | 78 | 27 | 47 | 4 |
| `mobs::render` | 76 | 14 | 45 | 17 |
| `mobs::ai` | 73 | 12 | 25 | 36 |
| `mobs::frog` | 73 | 11 | 15 | 47 |
| `mod_content` | 68 | 7 | 22 | 39 |
| `mobs::path` | 67 | 15 | 2 | 50 |
| `mobs::spear_use` | 67 | 11 | 18 | 38 |
| `mobs::raider` | 65 | 17 | 17 | 31 |
| `landing` | 63 | 3 | 2 | 58 |
| `mobs::aquatic` | 62 | 16 | 19 | 27 |
| `mobs::armadillo` | 62 | 10 | 20 | 32 |
| `motion::fluids` | 62 | 10 | 2 | 50 |
| `mobs::monster_brain` | 61 | 12 | 16 | 33 |
| `clock` | 60 | 53 | 6 | 1 |
| `mobs::equine` | 59 | 14 | 26 | 19 |
| `mobs::warden` | 59 | 15 | 29 | 15 |
| `net` | 59 | 25 | 32 | 2 |
| `projectiles` | 59 | 31 | 14 | 14 |
| `vanillagen` | 59 | 14 | 24 | 21 |
| `creative` | 57 | 29 | 20 | 8 |
| `mobs::spawn_selection` | 57 | 25 | 8 | 24 |
| `mobs::villager` | 57 | 17 | 29 | 11 |
| `multiplayer` | 56 | 23 | 31 | 2 |
| `mobs::turtle` | 55 | 12 | 9 | 34 |
| `lan::conn` | 52 | 13 | 34 | 5 |
| `mobs::dolphin` | 52 | 13 | 5 | 34 |
| `advancements` | 51 | 9 | 13 | 29 |
| `lan::chunks` | 51 | 8 | 34 | 9 |
| `item_render` | 50 | 18 | 20 | 12 |
| `mobs::happy_ghast` | 49 | 9 | 21 | 19 |
| `recipe_book` | 49 | 23 | 18 | 8 |
| `mobs::sulfur_cube` | 48 | 12 | 28 | 8 |
| `crafting` | 47 | 7 | 23 | 17 |
| `mobs::shulker` | 47 | 7 | 15 | 25 |
| `mobs::transport_items` | 47 | 3 | 15 | 29 |
| `mobs::illager` | 46 | 7 | 16 | 23 |
| `mobs::nav` | 46 | 23 | 0 | 23 |
| `music` | 46 | 18 | 15 | 13 |
| `lan` | 45 | 22 | 20 | 3 |
| `mobs::anim` | 45 | 34 | 11 | 0 |
| `mobs::warden_brain` | 44 | 15 | 10 | 19 |
| `merchant` | 41 | 16 | 9 | 16 |
| `mobs::camel` | 41 | 4 | 26 | 11 |
| `mobs::spawn_rules` | 41 | 5 | 1 | 35 |
| `title::flat` | 41 | 19 | 21 | 1 |
| `lan::containers` | 40 | 20 | 17 | 3 |
| `lan::entity` | 40 | 19 | 17 | 4 |
| `mobs::nautilus` | 40 | 5 | 24 | 11 |
| `mobs::golem` | 39 | 6 | 8 | 25 |
| `item_tooltip` | 38 | 4 | 5 | 29 |
| `mobs::cube` | 38 | 6 | 16 | 16 |
| `mobs::trial_spawner` | 38 | 11 | 2 | 25 |
| `game_rules_screen` | 37 | 18 | 18 | 1 |
| `mobs::doors` | 36 | 11 | 2 | 23 |
| `mobs::rabbit` | 36 | 6 | 14 | 16 |
| `first_person` | 35 | 19 | 8 | 8 |
| `lan::guest` | 35 | 17 | 16 | 2 |
| `mobs::goat` | 35 | 5 | 12 | 18 |
| `mobs::hostile` | 35 | 9 | 5 | 21 |
| `mobs::phantom` | 35 | 8 | 6 | 21 |
| `animate_tick` | 34 | 6 | 8 | 20 |
| `control` | 34 | 12 | 21 | 1 |
| `mobs::enderman` | 34 | 4 | 7 | 23 |
| `mobs::trader` | 34 | 8 | 5 | 21 |
| `debug_chart` | 33 | 23 | 10 | 0 |
| `mobs::body_anim` | 33 | 3 | 30 | 0 |
| `mobs::nether` | 33 | 8 | 6 | 19 |
| `mobs::allay` | 32 | 6 | 10 | 16 |
| `title::create_world` | 32 | 18 | 14 | 0 |
| `mobs::spawner` | 31 | 8 | 0 | 23 |
| `lan::waypoints` | 30 | 9 | 3 | 18 |
| `scoreboard_cmd` | 30 | 1 | 11 | 18 |
| `spear` | 30 | 20 | 8 | 2 |
| `dialog` | 29 | 16 | 11 | 2 |
| `item_render::special` | 29 | 11 | 6 | 12 |
| `mobs::creaking` | 29 | 1 | 23 | 5 |
| `screen_narrator` | 29 | 13 | 3 | 13 |
| `ui_theme` | 29 | 1 | 12 | 16 |
| `menu::place` | 28 | 6 | 7 | 15 |
| `mobs::world` | 28 | 0 | 0 | 28 |
| `locator_bar` | 27 | 13 | 6 | 8 |
| `stations` | 27 | 2 | 10 | 15 |
| `attribute_updates` | 26 | 2 | 0 | 24 |
| `debug_overlay` | 26 | 18 | 2 | 6 |
| `entities::fluids` | 26 | 2 | 2 | 22 |
| `lan::menu` | 26 | 3 | 6 | 17 |
| `mobs::vex` | 26 | 4 | 7 | 15 |
| `debug_entries` | 25 | 17 | 8 | 0 |
| `lan::hub::chat` | 25 | 12 | 12 | 1 |
| `mobs::creaking_heart` | 25 | 3 | 3 | 19 |
| `server_packs` | 25 | 18 | 7 | 0 |
| `item_render::gui` | 24 | 1 | 17 | 6 |
| `mobs::landing` | 24 | 0 | 0 | 24 |
| `mobs::llama` | 24 | 2 | 14 | 8 |
| `gossip` | 23 | 19 | 2 | 2 |
| `mobs::drops` | 23 | 9 | 12 | 2 |
| `mobs::witch` | 23 | 4 | 5 | 14 |
| `mobs::wither` | 23 | 4 | 12 | 7 |
| `lan::anticheat` | 22 | 11 | 9 | 2 |
| `mobs::cem` | 22 | 11 | 10 | 1 |
| `mobs::guardian` | 22 | 2 | 11 | 9 |
| `motion::environment` | 22 | 1 | 2 | 19 |
| `mobs::leash` | 21 | 11 | 7 | 3 |
| `toasts` | 21 | 8 | 11 | 2 |
| `discord_presence` | 20 | 4 | 10 | 6 |
| `lan::data` | 20 | 2 | 13 | 5 |
| `lan::playerdata` | 20 | 4 | 15 | 1 |
| `mobs::caravan` | 20 | 4 | 3 | 13 |
| `mobs::control` | 20 | 17 | 0 | 3 |
| `spectator_menu` | 20 | 9 | 10 | 1 |
| `ticket_levels` | 20 | 17 | 2 | 1 |
| `vanillagen::lod` | 20 | 6 | 11 | 3 |
| `vanillagen::structure_mobs` | 20 | 1 | 4 | 15 |
| `block_entity_item` | 19 | 9 | 9 | 1 |
| `mobs::breeze` | 19 | 1 | 12 | 6 |
| `mobs::gear` | 19 | 6 | 13 | 0 |
| `mobs::riders` | 19 | 3 | 13 | 3 |
| `mobs::cem_anim` | 18 | 7 | 3 | 8 |
| `mobs::polar_bear` | 18 | 1 | 8 | 9 |
| `pose` | 18 | 8 | 1 | 9 |
| `ram_limit` | 18 | 14 | 4 | 0 |
| `recipe_book_screen` | 18 | 5 | 6 | 7 |
| `title::buffet` | 18 | 8 | 10 | 0 |
| `beehive` | 17 | 4 | 0 | 13 |
| `lan::prediction` | 17 | 8 | 5 | 4 |
| `mod_content::biomes` | 17 | 3 | 9 | 5 |
| `social_screen` | 17 | 11 | 5 | 1 |
| `gizmos` | 16 | 15 | 1 | 0 |
| `lan::advancements` | 16 | 2 | 11 | 3 |
| `mobs::illusioner` | 16 | 3 | 8 | 5 |
| `old_world` | 16 | 5 | 5 | 6 |
| `structure_blocks` | 16 | 3 | 7 | 6 |
| `ambience` | 15 | 3 | 5 | 7 |
| `banner` | 15 | 4 | 9 | 2 |
| `benchmark` | 15 | 2 | 11 | 2 |
| `join_seq` | 15 | 3 | 0 | 12 |
| `mobs::brain` | 15 | 9 | 4 | 2 |
| `mobs::native_control` | 15 | 12 | 1 | 2 |
| `mobs::skin` | 15 | 2 | 7 | 6 |
| `mobs::sniffer` | 15 | 1 | 14 | 0 |
| `mobs::species` | 15 | 11 | 4 | 0 |
| `mobs::strider` | 15 | 3 | 3 | 9 |
| `native_recipe_types` | 15 | 4 | 8 | 3 |
| `server_packs::download` | 15 | 2 | 12 | 1 |
| `text_display` | 15 | 4 | 11 | 0 |
| `win_screen` | 15 | 6 | 8 | 1 |
| `dialog_screen` | 14 | 4 | 8 | 2 |
| `lan::discovery` | 14 | 10 | 4 | 0 |
| `title::onboarding` | 14 | 4 | 10 | 0 |
| `chunk_io` | 13 | 5 | 5 | 3 |
| `confirm_screen` | 13 | 8 | 5 | 0 |
| `falling_block` | 13 | 7 | 4 | 2 |
| `lan::far` | 13 | 10 | 2 | 1 |
| `lod_net` | 13 | 8 | 5 | 0 |
| `lod_net::client` | 13 | 6 | 5 | 2 |
| `mobs::equip` | 13 | 12 | 1 | 0 |
| `boss_overlay` | 12 | 10 | 2 | 0 |
| `data_cmd` | 12 | 3 | 9 | 0 |
| `item_sync` | 12 | 1 | 10 | 1 |
| `lan::hub::chat_filter` | 12 | 6 | 6 | 0 |
| `lod_net::server` | 12 | 3 | 2 | 7 |
| `mobs::keyframes` | 12 | 8 | 4 | 0 |
| `mobs::parrot` | 12 | 0 | 7 | 5 |
| `native_client_hooks` | 12 | 6 | 4 | 2 |
| `title::world_management` | 12 | 0 | 2 | 10 |
| `flatgen` | 11 | 3 | 4 | 4 |
| `hunger` | 11 | 2 | 0 | 9 |
| `lan::recipes` | 11 | 3 | 4 | 4 |
| `mobs::zombie_villager` | 11 | 2 | 7 | 2 |
| `osk` | 11 | 7 | 3 | 1 |
| `title::experiments` | 11 | 3 | 8 | 0 |
| `world_spawn` | 11 | 8 | 0 | 3 |
| `item_render::cit` | 10 | 2 | 7 | 1 |
| `lan::attack_hitbox` | 10 | 6 | 4 | 0 |
| `lan::attributes` | 10 | 3 | 3 | 4 |
| `mobs::copper_golem` | 10 | 2 | 6 | 2 |
| `mobs::data` | 10 | 6 | 4 | 0 |
| `random_tick_sections` | 10 | 4 | 3 | 3 |
| `reconnect` | 10 | 4 | 6 | 0 |
| `stopwatches` | 10 | 7 | 3 | 0 |
| `block_transform` | 9 | 0 | 5 | 4 |
| `cauldron` | 9 | 0 | 6 | 3 |
| `container_screen` | 9 | 6 | 0 | 3 |
| `friends_screen` | 9 | 4 | 5 | 0 |
| `lan::commands` | 9 | 1 | 8 | 0 |
| `mobs::bat` | 9 | 1 | 7 | 1 |
| `mobs::ravager` | 9 | 0 | 7 | 2 |
| `mobs::spawning` | 9 | 5 | 3 | 1 |
| `patrol` | 9 | 3 | 1 | 5 |
| `pregen` | 9 | 7 | 1 | 1 |
| `attack_strength` | 8 | 8 | 0 | 0 |
| `block_contact` | 8 | 1 | 0 | 7 |
| `block_pattern` | 8 | 6 | 0 | 2 |
| `end_gateway` | 8 | 5 | 1 | 2 |
| `mobs::axolotl` | 8 | 3 | 3 | 2 |
| `mobs::mod_mob` | 8 | 3 | 5 | 0 |
| `mobs::random_textures` | 8 | 1 | 7 | 0 |
| `title::confirm_experiments` | 8 | 4 | 4 | 0 |
| `vanillagen::lod::vegetation` | 8 | 2 | 6 | 0 |
| `command_storage` | 7 | 3 | 4 | 0 |
| `debugworld` | 7 | 4 | 3 | 0 |
| `tick_chunk_membership` | 7 | 5 | 1 | 1 |
| `content_identity` | 6 | 1 | 4 | 1 |
| `entity_culling` | 6 | 1 | 2 | 3 |
| `extras_hud` | 6 | 3 | 3 | 0 |
| `furnace` | 6 | 1 | 1 | 4 |
| `hotbar` | 6 | 0 | 1 | 5 |
| `item_activation` | 6 | 2 | 3 | 1 |
| `mobs::farm` | 6 | 0 | 4 | 2 |
| `mobs::seen_grid` | 6 | 1 | 4 | 1 |
| `native_inventory` | 6 | 2 | 0 | 4 |
| `no_resources` | 6 | 1 | 1 | 4 |
| `shadows` | 6 | 2 | 2 | 2 |
| `sprint` | 6 | 5 | 0 | 1 |
| `breathing` | 5 | 5 | 0 | 0 |
| `offline_name` | 5 | 4 | 1 | 0 |
| `vanillagen::lod::islands` | 5 | 2 | 3 | 0 |
| `auto_jump` | 4 | 2 | 1 | 1 |
| `game_mode_switcher` | 4 | 3 | 1 | 0 |
| `lan::code_of_conduct` | 4 | 2 | 2 | 0 |
| `lan::forwarding` | 4 | 3 | 1 | 0 |
| `mobs::sync` | 4 | 0 | 4 | 0 |
| `screen_layout` | 4 | 1 | 2 | 1 |
| `cushion` | 3 | 0 | 0 | 3 |
| `debug_keys` | 3 | 1 | 0 | 2 |
| `heartbeat` | 3 | 3 | 0 | 0 |
| `inv_sort` | 3 | 0 | 0 | 3 |
| `lan::border` | 3 | 0 | 3 | 0 |
| `lan::scoreboard` | 3 | 0 | 3 | 0 |
| `menu::bundle` | 3 | 0 | 0 | 3 |
| `mobs::armor` | 3 | 0 | 3 | 0 |
| `mobs::item_break` | 3 | 2 | 1 | 0 |
| `nether_portal` | 3 | 3 | 0 | 0 |
| `portal_sections` | 3 | 2 | 0 | 1 |
| `end_crystal` | 2 | 1 | 1 | 0 |
| `item_render::gui::depth` | 2 | 0 | 2 | 0 |
| `lan::handoff` | 2 | 0 | 2 | 0 |
| `mobs::ambient` | 2 | 0 | 1 | 1 |
| `mobs::combat` | 2 | 2 | 0 | 0 |
| `mobs::villager_hats` | 2 | 0 | 2 | 0 |
| `perf_overlay` | 2 | 1 | 1 | 0 |
| `pick_item` | 2 | 0 | 2 | 0 |
| `player_sync` | 2 | 0 | 2 | 0 |
| `save::gateway` | 2 | 0 | 0 | 2 |
| `fire_render` | 1 | 0 | 1 | 0 |
| `lan::tracking` | 1 | 1 | 0 | 0 |
| `mobs::conversion` | 1 | 0 | 1 | 0 |
| `mobs::dragon_contacts` | 1 | 0 | 1 | 0 |
| `native_data` | 1 | 0 | 1 | 0 |
| `yoake_state_catalog` | 1 | 0 | 1 | 0 |
