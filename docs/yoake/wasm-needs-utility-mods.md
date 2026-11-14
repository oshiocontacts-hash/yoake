# WASM needs from the utility-mod chat (2026-11-14, "Yoake便利mod作成")

Mods planned: vein-mine / tree-felling, chest-sort, door-link, look-info (WAILA-like), quests (FTB-like), charms (Curios-like), plus auto-refill, death chest, coords HUD, zoom.

Already in the full catalog: destroy_block, place_block, use_item(_on), interact_entity, container/menu hooks, inventory_slot_rules / inventory_screen_layout, tooltip hooks, mob_killed / living_death, player_changed_dimension, advancement hooks, key hooks, player_save_to_file. Server services draft: block / block_property / set_block_property, edit_blocks, block_entity_read / patch, inventory_apply, player_stacks, entities_in, clip, player_effect_give, world_storage_read / write, net_send_to_player, tell / action_bar / title, run_command.

Missing → W2 services:
1. break a block as a player with their tool (drops, durability, stats, events)
2. add attribute modifiers (only removal exists)
3. register key bindings (client)
4. client view raycast (block + entity) and registry id → owning mod + display name
5. entity health / max health query
6. give items to a player (or confirm inventory_apply covers it)
