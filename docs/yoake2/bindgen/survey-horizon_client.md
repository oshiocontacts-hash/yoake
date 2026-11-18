# Survey of crate `horizon_client`: the classifier over every file module

Generated; nothing is bound by this. Each module is classified as if its own glue were included in it (private items of other modules are not reachable). Derived trait impls are left out of the denominator.

- modules: 408
- functions: 9265
- bound now: 4394 (47.4%): 2122 basic + 2272 mechanical
- handle-required: 3306 (35.7%)
- excluded: 1565 (16.9%)
- fields with accessors: 2290 of 4060

## Handle-required: most frequent reasons (names generalised)

| reason | occurrences |
|---|---:|
| external type `_` (crate not loaded) [return value] | 852 |
| external type `_` (crate not loaded) | 668 |
| external type `_` (crate not loaded) [&argument] | 565 |
| field `_` of `_` is private to another module [&argument] | 512 |
| field `_` of `_` is private to another module [&mut argument] | 320 |
| external type `_` (crate not loaded) [&mut argument] | 316 |
| field `_` of `_` is private to another module | 253 |
| field `_` of `_` is private to another module [return value] | 249 |
| a reference nested inside another type needs a handle [return value] | 197 |
| std type `_` has no codec yet [&argument] | 123 |
| std type `_` has no codec yet [return value] | 89 |
| field `_` of `_` is private to another module [Option<&> argument] | 61 |
| a reference nested inside another type needs a handle | 60 |
| std type `_` has no codec yet | 49 |
| returns `_`: needs a child handle that locks its parent | 39 |
| external type `_` (crate not loaded) [Option<&> argument] | 19 |
| std type `_` has no codec yet [&mut argument] | 18 |
| a reference nested inside another type needs a handle [&argument] | 17 |
| external type `_` (crate not loaded) [returned reference] | 16 |
| `_` is #[non_exhaustive] in another crate | 10 |
| a reference nested inside another type needs a handle [&mut argument] | 8 |
| field `_` of `_` is private to another module [returned reference] | 8 |
| std type `_` has no codec yet [Option<&> argument] | 7 |
| external type `_` (crate not loaded) [by-value self] | 4 |
| `_` is #[non_exhaustive] in another crate [return value] | 3 |

## Excluded: most frequent reasons (names generalised)

| reason | occurrences |
|---|---:|
| receiver type carries a lifetime: it cannot be a hook-argument handle | 516 |
| generic type alias `_` | 396 |
| `_` requires a listed instantiation | 244 |
| lifetime-carrying type `_` | 230 |
| external type `_` (crate not loaded) | 199 |
| closure argument (`_`): guest callback handles are not built | 169 |
| `_` values cannot cross the data ABI | 162 |
| external type `_` (crate not loaded) [&argument] | 101 |
| external type `_` (crate not loaded) [return value] | 99 |
| generic type `_` requires a listed instantiation | 95 |
| field `_` of `_` is private to another module | 68 |
| function-level type parameter `_` requires a listed instantiation | 60 |
| field `_` of `_` is private to another module [&mut argument] | 55 |
| std type `_` has no codec yet [&argument] | 55 |
| field `_` of `_` is private to another module [&argument] | 54 |
| std type `_` has no codec yet [return value] | 48 |
| `_` argument requires a listed instantiation | 46 |
| std type `_` has no codec yet | 40 |
| a reference nested inside another type needs a handle [return value] | 39 |
| a reference nested inside another type needs a handle | 38 |
| unsafe functions require an explicit safety contract | 29 |
| external type `_` (crate not loaded) [&mut argument] | 23 |
| impl-level type parameter `_` requires a listed instantiation | 23 |
| field `_` of `_` is private to another module [return value] | 22 |
| impl on a type that is not defined in a loaded engine crate | 20 |

## Per module

| module | functions | bound | handle-required | excluded |
|---|---:|---:|---:|---:|
| `game::native_hooks` | 724 | 457 | 175 | 92 |
| `game` | 501 | 184 | 146 | 171 |
| `game::remote` | 261 | 126 | 82 | 53 |
| `game::entity` | 215 | 131 | 39 | 45 |
| `entry::wasm_mods::full` | 203 | 32 | 122 | 49 |
| `game::dragon` | 136 | 99 | 30 | 7 |
| `game::creatures` | 121 | 60 | 34 | 27 |
| `entry::wasm_mods::server_services` | 119 | 61 | 56 | 2 |
| `game::lan_actions` | 104 | 20 | 74 | 10 |
| `game::commands` | 103 | 42 | 59 | 2 |
| `game::advancements` | 96 | 49 | 36 | 11 |
| `game::sculk` | 93 | 62 | 19 | 12 |
| `game::stations` | 90 | 51 | 35 | 4 |
| `game::persist` | 89 | 78 | 8 | 3 |
| `game::minecarts` | 88 | 51 | 34 | 3 |
| `game::raids` | 86 | 47 | 6 | 33 |
| `entry::wasm_mods` | 85 | 10 | 44 | 31 |
| `game::dispenser` | 78 | 29 | 30 | 19 |
| `game::hand` | 69 | 22 | 10 | 37 |
| `game::game_tests` | 68 | 29 | 37 | 2 |
| `game::maps` | 67 | 38 | 27 | 2 |
| `game::chat` | 64 | 35 | 28 | 1 |
| `game::natural_spawning` | 64 | 6 | 7 | 51 |
| `game::recipe_book` | 64 | 40 | 24 | 0 |
| `game::end` | 63 | 30 | 16 | 17 |
| `game::potions` | 62 | 23 | 35 | 4 |
| `game::lan` | 60 | 24 | 27 | 9 |
| `entry::dll_mods` | 59 | 5 | 9 | 45 |
| `game::repro` | 59 | 15 | 27 | 17 |
| `game::boats` | 58 | 40 | 17 | 1 |
| `game::dedicated` | 58 | 16 | 29 | 13 |
| `game::machines` | 57 | 18 | 24 | 15 |
| `game::pad_menu` | 54 | 19 | 14 | 21 |
| `game::books` | 53 | 32 | 17 | 4 |
| `game::agent` | 52 | 18 | 13 | 21 |
| `game::fishing` | 51 | 32 | 17 | 2 |
| `game::nether_portal` | 51 | 43 | 8 | 0 |
| `game::integrated` | 50 | 31 | 6 | 13 |
| `game::native_server` | 49 | 23 | 23 | 3 |
| `game::scoreboard` | 49 | 17 | 22 | 10 |
| `game::command_blocks` | 48 | 22 | 24 | 2 |
| `game::native_player_ops` | 48 | 37 | 7 | 4 |
| `entry::wasm_mods::client_services` | 47 | 19 | 25 | 3 |
| `game::lan_screen` | 46 | 27 | 18 | 1 |
| `game::commands::extra` | 45 | 21 | 22 | 2 |
| `game::perspective` | 45 | 15 | 22 | 8 |
| `game::debug_keys` | 42 | 23 | 13 | 6 |
| `game::armor_stands` | 40 | 9 | 27 | 4 |
| `game::item_draw` | 38 | 4 | 17 | 17 |
| `game::explosion` | 37 | 16 | 5 | 16 |
| `game::enchant_effects` | 35 | 22 | 12 | 1 |
| `game::commands::execute` | 34 | 15 | 18 | 1 |
| `game::respawn_anchor` | 34 | 21 | 4 | 9 |
| `game::combat` | 33 | 15 | 18 | 0 |
| `game::consume` | 33 | 8 | 0 | 25 |
| `game::native_block_entity_draw` | 33 | 12 | 18 | 3 |
| `game::mount_menu` | 32 | 25 | 7 | 0 |
| `game::mounts` | 32 | 21 | 5 | 6 |
| `game::split::windows` | 32 | 1 | 2 | 29 |
| `game::fireworks` | 31 | 12 | 17 | 2 |
| `game::native_client_map` | 31 | 26 | 2 | 3 |
| `game::native_panel` | 31 | 19 | 10 | 2 |
| `game::functions` | 30 | 13 | 16 | 1 |
| `game::native_wasm_services` | 30 | 12 | 17 | 1 |
| `game::shader_packs` | 30 | 19 | 11 | 0 |
| `game::sign_edit` | 30 | 20 | 9 | 1 |
| `game::startup_menu` | 30 | 9 | 10 | 11 |
| `game::native_mob_player` | 29 | 12 | 9 | 8 |
| `game::server_travel` | 29 | 15 | 12 | 2 |
| `game::block_entity_anims` | 28 | 23 | 5 | 0 |
| `game::bossbars` | 28 | 13 | 13 | 2 |
| `game::dialogs` | 28 | 13 | 11 | 4 |
| `game::native_session` | 28 | 17 | 1 | 10 |
| `game::options` | 28 | 14 | 7 | 7 |
| `entry::wasm_mods::gpu_services` | 27 | 5 | 18 | 4 |
| `game::beacon` | 27 | 18 | 9 | 0 |
| `game::commands::world_cmds` | 27 | 10 | 16 | 1 |
| `game::entity_culling` | 26 | 8 | 18 | 0 |
| `game::lan_particles` | 26 | 9 | 15 | 2 |
| `game::mod_api_host` | 26 | 3 | 8 | 15 |
| `game::stats` | 26 | 19 | 4 | 3 |
| `game::storage` | 26 | 21 | 4 | 1 |
| `game::structure_blocks` | 26 | 21 | 5 | 0 |
| `game::test_instances` | 26 | 15 | 11 | 0 |
| `game::trading` | 26 | 13 | 11 | 2 |
| `game::villagers` | 26 | 22 | 4 | 0 |
| `yak_mods` | 26 | 9 | 17 | 0 |
| `game::commands::data` | 25 | 8 | 17 | 0 |
| `game::leads` | 25 | 15 | 8 | 2 |
| `game::levels` | 25 | 10 | 6 | 9 |
| `game::remote_projectiles` | 25 | 13 | 7 | 5 |
| `game::tweaks` | 25 | 18 | 7 | 0 |
| `game::lan_stations` | 24 | 0 | 16 | 8 |
| `game::packs` | 24 | 9 | 12 | 3 |
| `game::pad::hardware` | 24 | 7 | 15 | 2 |
| `game::server_links` | 24 | 12 | 10 | 2 |
| `game::server_packs` | 24 | 12 | 9 | 3 |
| `game::cushions` | 23 | 14 | 6 | 3 |
| `game::item_use` | 23 | 10 | 12 | 1 |
| `game::native_data_menus` | 23 | 10 | 7 | 6 |
| `game::native_lifecycle` | 23 | 12 | 11 | 0 |
| `game::remote_chat` | 23 | 13 | 8 | 2 |
| `mod_api` | 23 | 13 | 4 | 6 |
| `game::extras` | 22 | 12 | 9 | 1 |
| `game::mace` | 22 | 17 | 5 | 0 |
| `game::narrator` | 22 | 9 | 13 | 0 |
| `game::piglins` | 22 | 13 | 9 | 0 |
| `game::spill` | 22 | 4 | 5 | 13 |
| `game::split` | 22 | 0 | 1 | 21 |
| `game::boat_leads` | 21 | 9 | 12 | 0 |
| `game::jukebox` | 21 | 15 | 5 | 1 |
| `game::level_particles` | 21 | 21 | 0 | 0 |
| `game::native_recipes` | 21 | 14 | 7 | 0 |
| `game::native_streams` | 21 | 8 | 12 | 1 |
| `game::screens` | 21 | 6 | 7 | 8 |
| `game::test_instance_edit` | 21 | 9 | 12 | 0 |
| `game::advancements_screen` | 20 | 17 | 3 | 0 |
| `game::block_editor` | 20 | 9 | 6 | 5 |
| `game::control` | 20 | 4 | 6 | 10 |
| `game::lan_mobs` | 20 | 4 | 12 | 4 |
| `game::native_deferred` | 20 | 18 | 0 | 2 |
| `game::native_entities` | 20 | 18 | 0 | 2 |
| `game::native_entity_query` | 20 | 7 | 4 | 9 |
| `game::native_inventory_screen` | 20 | 9 | 5 | 6 |
| `game::spawner` | 20 | 13 | 2 | 5 |
| `game::split::pads` | 20 | 2 | 1 | 17 |
| `game::text_displays` | 20 | 9 | 11 | 0 |
| `game::tutorial` | 20 | 17 | 3 | 0 |
| `game::boat_passengers` | 19 | 11 | 8 | 0 |
| `game::commands::test_runs` | 19 | 10 | 9 | 0 |
| `game::creaking` | 19 | 7 | 2 | 10 |
| `game::debug_lines` | 19 | 11 | 8 | 0 |
| `game::dispense_fixture` | 19 | 7 | 3 | 9 |
| `game::initial_spawn` | 19 | 5 | 2 | 12 |
| `game::input_trace` | 19 | 15 | 3 | 1 |
| `game::lan_crafting` | 19 | 1 | 13 | 5 |
| `game::commands::items` | 18 | 3 | 14 | 1 |
| `game::debug_options` | 18 | 13 | 5 | 0 |
| `game::frames` | 18 | 11 | 7 | 0 |
| `game::native_capabilities` | 18 | 9 | 2 | 7 |
| `game::stats_screen` | 18 | 15 | 3 | 0 |
| `game::trident` | 18 | 9 | 8 | 1 |
| `game::fov` | 17 | 12 | 5 | 0 |
| `game::lightning` | 17 | 13 | 4 | 0 |
| `game::native_claimed_items` | 17 | 16 | 1 | 0 |
| `game::native_menus` | 17 | 4 | 10 | 3 |
| `game::native_panel_recipes` | 17 | 8 | 8 | 1 |
| `game::native_world_draw` | 17 | 10 | 7 | 0 |
| `game::passengers` | 17 | 16 | 1 | 0 |
| `game::potent_sulfur` | 17 | 3 | 12 | 2 |
| `game::remote_vehicles` | 17 | 11 | 6 | 0 |
| `game::world_loading` | 17 | 1 | 10 | 6 |
| `game::commands::profiles` | 16 | 1 | 15 | 0 |
| `game::lan_pvp` | 16 | 5 | 10 | 1 |
| `game::mods` | 16 | 6 | 8 | 2 |
| `game::native_damage` | 16 | 12 | 4 | 0 |
| `game::pad` | 16 | 5 | 4 | 7 |
| `game::piston_push` | 16 | 11 | 3 | 2 |
| `game::render_tick` | 16 | 6 | 10 | 0 |
| `game::social_relay` | 16 | 8 | 6 | 2 |
| `game::split::players` | 16 | 1 | 0 | 15 |
| `game::trial_spawner` | 16 | 3 | 1 | 12 |
| `game::trial_visuals` | 16 | 12 | 4 | 0 |
| `yak_mods::shaders` | 16 | 1 | 15 | 0 |
| `entry::wasm_mods::resources` | 15 | 8 | 4 | 3 |
| `game::beehives` | 15 | 6 | 1 | 8 |
| `game::commands::terrain_cmds` | 15 | 5 | 9 | 1 |
| `game::key_focus` | 15 | 0 | 15 | 0 |
| `game::loom` | 15 | 12 | 3 | 0 |
| `game::native_audio` | 15 | 4 | 11 | 0 |
| `game::native_server_physics` | 15 | 11 | 3 | 1 |
| `game::commands::players` | 14 | 8 | 4 | 2 |
| `game::crossbow` | 14 | 2 | 12 | 0 |
| `game::machines::crafter` | 14 | 8 | 5 | 1 |
| `game::machines::native_fluids` | 14 | 13 | 1 | 0 |
| `game::mob_blocks` | 14 | 1 | 13 | 0 |
| `game::native_mobs` | 14 | 12 | 2 | 0 |
| `game::native_title_screen` | 14 | 10 | 4 | 0 |
| `game::paintings` | 14 | 7 | 7 | 0 |
| `game::remote::native_snapshot` | 14 | 0 | 6 | 8 |
| `game::remote_chunks` | 14 | 0 | 14 | 0 |
| `game::spear` | 14 | 9 | 5 | 0 |
| `game::tnt` | 14 | 6 | 4 | 4 |
| `game::wind` | 14 | 6 | 3 | 5 |
| `game::game_mode` | 13 | 10 | 3 | 0 |
| `game::golems` | 13 | 7 | 1 | 5 |
| `game::jigsaw_edit` | 13 | 5 | 8 | 0 |
| `game::lan_entities` | 13 | 5 | 8 | 0 |
| `game::lan_vehicles` | 13 | 7 | 6 | 0 |
| `game::music` | 13 | 11 | 2 | 0 |
| `game::native_client_state` | 13 | 10 | 3 | 0 |
| `game::native_frames` | 13 | 6 | 5 | 2 |
| `game::sculk_fixture` | 13 | 6 | 2 | 5 |
| `game::social` | 13 | 8 | 5 | 0 |
| `game::structure_edit` | 13 | 6 | 7 | 0 |
| `game::beds` | 12 | 9 | 1 | 2 |
| `game::brush` | 12 | 11 | 1 | 0 |
| `game::deflect` | 12 | 6 | 6 | 0 |
| `game::lan_trading` | 12 | 1 | 10 | 1 |
| `game::names` | 12 | 2 | 10 | 0 |
| `game::native_block_behaviors` | 12 | 6 | 3 | 3 |
| `game::native_crosshair` | 12 | 12 | 0 | 0 |
| `game::native_item_behaviors` | 12 | 2 | 9 | 1 |
| `game::native_server_ops` | 12 | 8 | 0 | 4 |
| `game::test_blocks` | 12 | 6 | 6 | 0 |
| `game::test_environments` | 12 | 10 | 1 | 1 |
| `entry::wasm_mods::package` | 11 | 2 | 7 | 2 |
| `game::block_edits` | 11 | 5 | 0 | 6 |
| `game::dedicated::management` | 11 | 0 | 8 | 3 |
| `game::dedicated::report` | 11 | 3 | 8 | 0 |
| `game::friends` | 11 | 8 | 1 | 2 |
| `game::lan_attributes` | 11 | 5 | 5 | 1 |
| `game::mod_api_host::budget` | 11 | 1 | 5 | 5 |
| `game::native_entity_motion` | 11 | 1 | 10 | 0 |
| `game::native_inventories` | 11 | 3 | 8 | 0 |
| `game::native_item_energy` | 11 | 2 | 9 | 0 |
| `game::native_item_fluids` | 11 | 2 | 9 | 0 |
| `game::sulfur_cubes` | 11 | 7 | 4 | 0 |
| `game::wandering_trader` | 11 | 10 | 1 | 0 |
| `yoake_test` | 11 | 5 | 5 | 1 |
| `game::cartography` | 10 | 2 | 7 | 1 |
| `game::commands::pregen` | 10 | 4 | 6 | 0 |
| `game::commands::test_locate` | 10 | 6 | 4 | 0 |
| `game::copper_golems` | 10 | 6 | 4 | 0 |
| `game::events` | 10 | 3 | 4 | 3 |
| `game::fangs` | 10 | 8 | 2 | 0 |
| `game::machines::native_energy` | 10 | 7 | 3 | 0 |
| `game::native_entity_refs` | 10 | 1 | 9 | 0 |
| `game::native_item_container` | 10 | 1 | 9 | 0 |
| `game::native_item_render` | 10 | 4 | 3 | 3 |
| `game::native_mannequin` | 10 | 8 | 1 | 1 |
| `game::native_mob_draw` | 10 | 1 | 4 | 5 |
| `game::native_net` | 10 | 8 | 2 | 0 |
| `game::ominous_item_spawner` | 10 | 7 | 3 | 0 |
| `game::remote_camera` | 10 | 7 | 3 | 0 |
| `game::screen_info` | 10 | 1 | 4 | 5 |
| `game::vault` | 10 | 1 | 4 | 5 |
| `game::wither` | 10 | 4 | 4 | 2 |
| `game::zombie_villagers` | 10 | 6 | 3 | 1 |
| `game::agent::navigation` | 9 | 6 | 0 | 3 |
| `game::clipboard` | 9 | 5 | 4 | 0 |
| `game::command_minecarts` | 9 | 4 | 5 | 0 |
| `game::commands::compute` | 9 | 1 | 8 | 0 |
| `game::commands::text_commands` | 9 | 1 | 7 | 1 |
| `game::composter` | 9 | 3 | 6 | 0 |
| `game::cosmetics` | 9 | 2 | 7 | 0 |
| `game::lan_mount_menu` | 9 | 0 | 6 | 3 |
| `game::machines::native_items` | 9 | 3 | 6 | 0 |
| `game::mob_gear` | 9 | 8 | 1 | 0 |
| `game::native_account` | 9 | 2 | 3 | 4 |
| `game::native_avatar_equipment` | 9 | 5 | 2 | 2 |
| `game::native_level_refs` | 9 | 0 | 9 | 0 |
| `game::native_mouse_buttons` | 9 | 5 | 0 | 4 |
| `game::native_projectiles` | 9 | 4 | 5 | 0 |
| `game::native_ticks` | 9 | 4 | 5 | 0 |
| `game::remote_objects` | 9 | 4 | 4 | 1 |
| `game::agent_settings` | 8 | 6 | 1 | 1 |
| `game::chat_filter` | 8 | 0 | 8 | 0 |
| `game::code_of_conduct` | 8 | 4 | 2 | 2 |
| `game::debug_renderers` | 8 | 5 | 1 | 2 |
| `game::debug_stick` | 8 | 2 | 3 | 3 |
| `game::locator_bar` | 8 | 6 | 2 | 0 |
| `game::memory` | 8 | 1 | 3 | 4 |
| `game::native_living_refs` | 8 | 2 | 6 | 0 |
| `game::native_mob_navigation` | 8 | 0 | 8 | 0 |
| `game::no_resources` | 8 | 2 | 4 | 2 |
| `game::pickups` | 8 | 4 | 4 | 0 |
| `game::remote_mob_data` | 8 | 0 | 8 | 0 |
| `game::spectator` | 8 | 5 | 3 | 0 |
| `game::split::hud` | 8 | 1 | 4 | 3 |
| `game::test_block_edit` | 8 | 2 | 6 | 0 |
| `game::totem` | 8 | 6 | 2 | 0 |
| `game::block_entity_ticks` | 7 | 2 | 2 | 3 |
| `game::block_items` | 7 | 3 | 3 | 1 |
| `game::commands::effects` | 7 | 1 | 6 | 0 |
| `game::commands::place` | 7 | 1 | 5 | 1 |
| `game::interactions` | 7 | 3 | 4 | 0 |
| `game::jigsaw_blocks` | 7 | 2 | 5 | 0 |
| `game::lan_camera` | 7 | 1 | 6 | 0 |
| `game::native_attack_hitbox` | 7 | 5 | 1 | 1 |
| `game::native_cooldowns` | 7 | 3 | 1 | 3 |
| `game::native_data_maps` | 7 | 1 | 6 | 0 |
| `game::native_inventory_rules` | 7 | 2 | 2 | 3 |
| `game::native_mounts` | 7 | 7 | 0 | 0 |
| `game::native_post_shader` | 7 | 5 | 2 | 0 |
| `game::native_screen_slots` | 7 | 2 | 4 | 1 |
| `game::native_world_read` | 7 | 0 | 7 | 0 |
| `game::shield` | 7 | 1 | 2 | 4 |
| `game::splash_water` | 7 | 2 | 5 | 0 |
| `game::test_entities` | 7 | 7 | 0 | 0 |
| `game::transient_blocks` | 7 | 5 | 2 | 0 |
| `game::ui_control` | 7 | 2 | 3 | 2 |
| `game::win` | 7 | 3 | 1 | 3 |
| `entry` | 6 | 2 | 0 | 4 |
| `entry::wasm_mods::dev_shaders` | 6 | 1 | 4 | 1 |
| `game::bundles` | 6 | 6 | 0 | 0 |
| `game::command_harness` | 6 | 0 | 1 | 5 |
| `game::commands::misc` | 6 | 2 | 4 | 0 |
| `game::lan_chat` | 6 | 1 | 5 | 0 |
| `game::machines::native_wasm_containers` | 6 | 3 | 3 | 0 |
| `game::moving_pistons` | 6 | 4 | 2 | 0 |
| `game::native_entity_data` | 6 | 0 | 6 | 0 |
| `game::native_lan` | 6 | 6 | 0 | 0 |
| `game::native_registry_names` | 6 | 4 | 0 | 2 |
| `game::remote_stations` | 6 | 5 | 1 | 0 |
| `game::shoulders` | 6 | 6 | 0 | 0 |
| `game::titles` | 6 | 1 | 5 | 0 |
| `entry::wasm_mods::package_ui` | 5 | 0 | 5 | 0 |
| `entry::wasm_mods::test_driver` | 5 | 1 | 3 | 1 |
| `game::allays` | 5 | 4 | 1 | 0 |
| `game::campfire` | 5 | 1 | 2 | 2 |
| `game::cat_spawner` | 5 | 5 | 0 | 0 |
| `game::client_ambient` | 5 | 3 | 2 | 0 |
| `game::ghasts` | 5 | 3 | 2 | 0 |
| `game::ime` | 5 | 4 | 1 | 0 |
| `game::item_particles` | 5 | 2 | 3 | 0 |
| `game::mob_looting` | 5 | 1 | 4 | 0 |
| `game::native_criteria` | 5 | 1 | 0 | 4 |
| `game::native_explosions` | 5 | 2 | 2 | 1 |
| `game::native_gui_item` | 5 | 2 | 2 | 1 |
| `game::native_key_bindings` | 5 | 5 | 0 | 0 |
| `game::native_sky_tint` | 5 | 3 | 2 | 0 |
| `game::native_visuals` | 5 | 1 | 3 | 1 |
| `game::native_world_border` | 5 | 3 | 2 | 0 |
| `game::pandas` | 5 | 5 | 0 | 0 |
| `game::player_equip` | 5 | 0 | 5 | 0 |
| `game::powder_snow` | 5 | 3 | 1 | 1 |
| `game::station_control` | 5 | 2 | 1 | 2 |
| `mod_guard` | 5 | 0 | 5 | 0 |
| `game::atlas_slots` | 4 | 0 | 0 | 4 |
| `game::attachment_sync` | 4 | 1 | 2 | 1 |
| `game::banners` | 4 | 2 | 2 | 0 |
| `game::blockctl` | 4 | 1 | 0 | 3 |
| `game::chiseled_bookshelf` | 4 | 2 | 2 | 0 |
| `game::dedicated::events` | 4 | 0 | 4 | 0 |
| `game::dedicated::status` | 4 | 1 | 3 | 0 |
| `game::dynamic_lights` | 4 | 0 | 4 | 0 |
| `game::foxes` | 4 | 4 | 0 | 0 |
| `game::glowing` | 4 | 1 | 3 | 0 |
| `game::growth` | 4 | 1 | 1 | 2 |
| `game::headless` | 4 | 2 | 2 | 0 |
| `game::lan_quiet` | 4 | 3 | 1 | 0 |
| `game::llamas` | 4 | 4 | 0 | 0 |
| `game::mob_index` | 4 | 0 | 3 | 1 |
| `game::native_block_sprites` | 4 | 1 | 2 | 1 |
| `game::native_chat` | 4 | 1 | 1 | 2 |
| `game::native_components` | 4 | 0 | 3 | 1 |
| `game::native_gui_textures` | 4 | 0 | 4 | 0 |
| `game::native_item_slot` | 4 | 1 | 2 | 1 |
| `game::native_name_lines` | 4 | 1 | 1 | 2 |
| `game::native_quick_move` | 4 | 1 | 3 | 0 |
| `game::nautili` | 4 | 4 | 0 | 0 |
| `game::pick_entity` | 4 | 3 | 1 | 0 |
| `game::ram_limit` | 4 | 4 | 0 | 0 |
| `game::saving` | 4 | 0 | 0 | 4 |
| `game::shelf` | 4 | 3 | 1 | 0 |
| `game::split::setup` | 4 | 0 | 0 | 4 |
| `game::campfire_douse` | 3 | 0 | 2 | 1 |
| `game::creative_hotbar` | 3 | 3 | 0 | 0 |
| `game::decorated_pot` | 3 | 1 | 2 | 0 |
| `game::event_loop` | 3 | 0 | 1 | 2 |
| `game::falling_damage` | 3 | 1 | 2 | 0 |
| `game::gamemode_switcher` | 3 | 2 | 1 | 0 |
| `game::lan_far` | 3 | 0 | 3 | 0 |
| `game::lan_waypoints` | 3 | 1 | 2 | 0 |
| `game::launch_api` | 3 | 1 | 1 | 1 |
| `game::mob_sounds` | 3 | 1 | 2 | 0 |
| `game::native_client_items` | 3 | 3 | 0 | 0 |
| `game::native_entity_moves` | 3 | 1 | 2 | 0 |
| `game::native_falling` | 3 | 3 | 0 | 0 |
| `game::native_hud_layout` | 3 | 1 | 2 | 0 |
| `game::native_hud_visibility` | 3 | 3 | 0 | 0 |
| `game::native_motion` | 3 | 3 | 0 | 0 |
| `game::native_pvp` | 3 | 3 | 0 | 0 |
| `game::native_shaders` | 3 | 0 | 3 | 0 |
| `game::native_world_columns` | 3 | 2 | 1 | 0 |
| `game::quick_actions` | 3 | 3 | 0 | 0 |
| `game::remote_mob_particles` | 3 | 3 | 0 | 0 |
| `game::render_control` | 3 | 0 | 0 | 3 |
| `game::sniffers` | 3 | 2 | 1 | 0 |
| `game::structures` | 3 | 2 | 0 | 1 |
| `game::yoake_tests` | 3 | 2 | 1 | 0 |
| `game::zombie_reinforcements` | 3 | 1 | 2 | 0 |
| `game::discord` | 2 | 1 | 1 | 0 |
| `game::execution_role` | 2 | 2 | 0 | 0 |
| `game::interaction_shapes` | 2 | 2 | 0 | 0 |
| `game::native_local_player` | 2 | 1 | 1 | 0 |
| `game::native_team` | 2 | 1 | 0 | 1 |
| `game::native_world_edit` | 2 | 2 | 0 | 0 |
| `game::prediction` | 2 | 0 | 2 | 0 |
| `game::random_textures` | 2 | 1 | 1 | 0 |
| `game::remote_mob_use` | 2 | 2 | 0 | 0 |
| `game::saves_control` | 2 | 0 | 0 | 2 |
| `game::subtitle_hud` | 2 | 1 | 1 | 0 |
| `game::boat_authority_fixture` | 1 | 0 | 0 | 1 |
| `game::breezes` | 1 | 1 | 0 | 0 |
| `game::commands::raid` | 1 | 0 | 1 | 0 |
| `game::native_biome_colors` | 1 | 1 | 0 | 0 |
| `game::native_camera` | 1 | 0 | 0 | 1 |
| `game::native_fluids` | 1 | 0 | 1 | 0 |
| `game::native_mix` | 1 | 1 | 0 | 0 |
| `game::native_perms` | 1 | 1 | 0 | 0 |
| `game::net_harness` | 1 | 0 | 0 | 1 |
| `game::postprocess` | 1 | 0 | 0 | 1 |
| `game::pumpkin` | 1 | 0 | 0 | 1 |
| `game::split::frames` | 1 | 0 | 0 | 1 |
