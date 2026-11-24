# `items`: every function (horizon-items/src/items.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `items::<BlockReduction as Clone>::clone` | 277 | &self | derived (not counted) | - |
| `items::<BlockReduction as Debug>::fmt` | 277 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::<BlocksAttacks as Clone>::clone` | 286 | &self | derived (not counted) | - |
| `items::<BlocksAttacks as Debug>::fmt` | 286 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::<ConsumeEffect as Clone>::clone` | 80 | &self | derived (not counted) | external type `serde_json::value::Value` (crate not loaded) [return value] |
| `items::<ConsumeEffect as Debug>::fmt` | 80 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::<ConsumeEffect as PartialEq>::eq` | 80 | &self | derived (not counted) | external type `serde_json::value::Value` (crate not loaded) [&argument] |
| `items::<ConsumeEffectSpec as Clone>::clone` | 73 | &self | derived (not counted) | - |
| `items::<ConsumeEffectSpec as Debug>::fmt` | 73 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::<ConsumeEffectSpec as PartialEq>::eq` | 73 | &self | derived (not counted) | - |
| `items::<Instrument as Clone>::clone` | 453 | &self | derived (not counted) | - |
| `items::<Instrument as Debug>::fmt` | 453 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::<ItemInfo as Clone>::clone` | 180 | &self | derived (not counted) | generic type `horizon_world::consumable::Consumable` requires a listed instantiation |
| `items::<ItemInfo as Debug>::fmt` | 180 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::<ItemStack as Clone>::clone` | 369 | &self | derived (not counted) | - |
| `items::<ItemStack as Debug>::fmt` | 369 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::<ItemStack as InjectSnapshot>::inject_snapshot` | 379 | &self | handle-required | external type `yoake_value::Value` (crate not loaded) [return value] |
| `items::<ItemStack as PartialEq>::eq` | 369 | &self | derived (not counted) | - |
| `items::<Song as Clone>::clone` | 439 | &self | derived (not counted) | external type `serde_json::value::Value` (crate not loaded) [return value] |
| `items::<Song as Debug>::fmt` | 439 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::<ToolRule as Clone>::clone` | 67 | &self | derived (not counted) | - |
| `items::<ToolRule as Debug>::fmt` | 67 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `items::BlocksAttacks::of_stack` | 301 | - | excluded | generic type `horizon_world::consumable::Consumable` requires a listed instantiation; lifetime-carrying type `std::borrow::Cow` |
| `items::BlocksAttacks::parse` | 317 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [&argument] |
| `items::Instrument::use_ticks` | 463 | &self | bound (a) | - |
| `items::Items::block_drops` | 1156 | &self | excluded | field `states` of `BlockRegistry` is private to another module [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation |
| `items::Items::block_drops_by_player` | 1206 | &self | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; field `states` of `BlockRegistry` is private to another module [&argument] |
| `items::Items::block_drops_exploded` | 1220 | &self | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; a reference nested inside another type needs a handle; field `states` of `BlockRegistry` is private to another module [&argument] |
| `items::Items::block_drops_full` | 1190 | &self | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; a reference nested inside another type needs a handle; field `states` of `BlockRegistry` is private to another module [&argument] |
| `items::Items::block_drops_in` | 1167 | &self | excluded | a reference nested inside another type needs a handle; field `states` of `BlockRegistry` is private to another module [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation |
| `items::Items::block_drops_raw` | 1248 | &self | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; a reference nested inside another type needs a handle; field `states` of `BlockRegistry` is private to another module [&argument] |
| `items::Items::block_drops_with` | 1179 | &self | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; field `states` of `BlockRegistry` is private to another module [&argument] |
| `items::Items::block_in_tag` | 1037 | &self | bound (a) | - |
| `items::Items::block_loot_sequence` | 1125 | &self | handle-required | field `states` of `BlockRegistry` is private to another module [&argument] |
| `items::Items::block_loot_table` | 1135 | &self | handle-required | field `states` of `BlockRegistry` is private to another module [&argument] |
| `items::Items::block_matches` | 1044 | &self | bound (a) | - |
| `items::Items::can_destroy_block` | 1051 | &self | bound (b, mechanical) | - |
| `items::Items::catalog` | 739 | - | excluded | generic type alias `Result`; lifetime-carrying type `PackOverlay`; std type `std::path::Path` has no codec yet [&argument] |
| `items::Items::correct_for_drops` | 1089 | &self | bound (b, mechanical) | - |
| `items::Items::entity_drops` | 1380 | &self | excluded | function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation |
| `items::Items::get` | 987 | &self | handle-required | a reference nested inside another type needs a handle [return value] |
| `items::Items::has_loot_table` | 1103 | &self | bound (a) | - |
| `items::Items::inline_loot_table` | 1107 | &self | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation |
| `items::Items::instrument` | 992 | &self | bound (b, mechanical) | - |
| `items::Items::interact_loot` | 1331 | &self | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; field `salt` of `RandomSequences` is private to another module [&mut argument]; field `states` of `BlockRegistry` is private to another module [&argument] |
| `items::Items::item_in_tag` | 1041 | &self | bound (a) | - |
| `items::Items::load` | 720 | - | excluded | field `states` of `BlockRegistry` is private to another module [&argument]; generic type alias `Result`; lifetime-carrying type `PackOverlay`; std type `std::path::Path` has no codec yet [&argument] |
| `items::Items::load_with` | 742 | - | excluded | a reference nested inside another type needs a handle [&argument]; generic type alias `Result`; lifetime-carrying type `PackOverlay`; std type `std::path::Path` has no codec yet [&argument] |
| `items::Items::loot_sequence` | 1323 | &self | bound (b, mechanical) | - |
| `items::Items::loot_table` | 1295 | &self | excluded | function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation |
| `items::Items::loot_table_from` | 1358 | &self | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation |
| `items::Items::mining_speed` | 1071 | &self | bound (b, mechanical) | - |
| `items::Items::resists_cactus` | 693 | &self | bound (b, mechanical) | - |
| `items::Items::resists_fire` | 670 | &self | bound (b, mechanical) | - |
| `items::Items::tags_only` | 1003 | - | excluded | generic type alias `Result`; lifetime-carrying type `PackOverlay`; std type `std::path::Path` has no codec yet [&argument] |
| `items::LootContext::choose` | 1772 | &self | excluded | external type `serde_json::value::Value` (crate not loaded); function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::condition` | 1527 | &self | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::conditions` | 1668 | &self | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::enchant_pool` | 1843 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [&argument] |
| `items::LootContext::enchantment_level` | 1448 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::evaluate` | 1407 | &self | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::expand` | 1681 | &self | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; external type `serde_json::value::Value` (crate not loaded) [&mut argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::int_number` | 1510 | - | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation |
| `items::LootContext::modifiers` | 1851 | &self | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::native_loot_context` | 1455 | &self | excluded | function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation; lifetime-carrying type `horizon_world::loot::NativeLootContext`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::nested` | 1811 | &self | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `items::LootContext::number` | 1478 | - | excluded | external type `serde_json::value::Value` (crate not loaded) [&argument]; function-level type parameter `R` requires a listed instantiation; generic type `R` requires a listed instantiation |
| `items::Song::finished` | 448 | &self | bound (a) | - |
| `items::apply_loot` | 2112 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [&argument] |
| `items::bucket_contents` | 2191 | - | handle-required | a reference nested inside another type needs a handle [return value] |
| `items::entity_spec` | 638 | - | bound (b, mechanical) | - |
| `items::fire_starter_pitch` | 59 | - | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation |
| `items::fire_starter_sound` | 48 | - | bound (b, mechanical) | - |
| `items::inline_instrument` | 482 | - | handle-required | a reference nested inside another type needs a handle [return value] |
| `items::instrument_id` | 470 | - | bound (b, mechanical) | - |
| `items::load_instruments` | 504 | - | handle-required | std type `std::path::Path` has no codec yet [&argument] |
| `items::load_songs` | 522 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [return value]; std type `std::path::Path` has no codec yet [&argument] |
| `items::load_tag_lists` | 565 | - | excluded | generic type alias `crate::Result`; lifetime-carrying type `horizon_world::loot::PackOverlay`; std type `std::path::Path` has no codec yet [&argument] |
| `items::load_tags` | 553 | - | excluded | generic type alias `crate::Result`; lifetime-carrying type `horizon_world::loot::PackOverlay`; std type `std::path::Path` has no codec yet [&argument] |
| `items::loot_of` | 2057 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [return value] |
| `items::mod_function` | 2172 | - | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built; external type `serde_json::value::Value` (crate not loaded) [&argument]; lifetime-carrying type `horizon_world::loot::NativeLootContext` |
| `items::parse_consume_effect` | 105 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [&argument]; external type `serde_json::value::Value` (crate not loaded) [return value] |
| `items::parse_instrument` | 493 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [&argument] |
| `items::publish_registry` | 28 | - | excluded | generic type `horizon_world::consumable::Consumable` requires a listed instantiation |
| `items::publish_registry_for_checks` | 24 | - | bound (a) | - |
| `items::published_stack_limit` | 41 | - | bound (b, mechanical) | - |
| `items::registry_order` | 390 | - | handle-required | std type `std::path::Path` has no codec yet [&argument] |
| `items::set_villager_food` | 649 | - | excluded | generic type `horizon_world::consumable::Consumable` requires a listed instantiation |
| `items::take_held_drop` | 2853 | - | bound (b, mechanical) | - |
| `items::villager_food` | 662 | - | bound (b, mechanical) | - |

## Fields

| field | status |
|---|---|
| `items::ToolRule.blocks` | get + set |
| `items::ToolRule.speed` | get + set |
| `items::ToolRule.correct` | get + set |
| `items::ConsumeEffectSpec.id` | get + set |
| `items::ConsumeEffectSpec.duration` | get + set |
| `items::ConsumeEffectSpec.amplifier` | get + set |
| `items::ItemInfo.fire_resistant` | get + set |
| `items::ItemInfo.id` | get + set |
| `items::ItemInfo.max_stack` | get + set |
| `items::ItemInfo.max_damage` | get + set |
| `items::ItemInfo.block` | get + set |
| `items::ItemInfo.tool` | get + set |
| `items::ItemInfo.tool_default_speed` | get + set |
| `items::ItemInfo.tool_damage_per_block` | get + set |
| `items::ItemInfo.can_destroy_blocks_in_creative` | get + set |
| `items::ItemInfo.food` | get + set |
| `items::ItemInfo.can_always_eat` | get + set |
| `items::ItemInfo.consumable` | no accessor: generic type `horizon_world::consumable::Consumable` requires a listed instantiation |
| `items::ItemInfo.use_remainder` | get + set |
| `items::ItemInfo.use_cooldown` | get + set |
| `items::ItemInfo.model` | get + set |
| `items::ItemInfo.attack_damage` | get + set |
| `items::ItemInfo.attack_speed` | get + set |
| `items::ItemInfo.fuel` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `items::ItemInfo.brewing_fuel` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `items::ItemInfo.compostable` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `items::ItemInfo.remainder` | get + set |
| `items::ItemInfo.equip_slot` | get + set |
| `items::ItemInfo.equip_asset` | get + set |
| `items::ItemInfo.modifiers` | get + set |
| `items::ItemInfo.blocks_attacks` | get + set |
| `items::ItemInfo.disable_blocking_seconds` | get + set |
| `items::ItemInfo.weapon_damage` | get + set |
| `items::ItemInfo.villager_food` | get + set |
| `items::ItemInfo.use_speed` | get + set |
| `items::ItemInfo.can_sprint` | get + set |
| `items::ItemInfo.interact_vibrations` | get + set |
| `items::ItemInfo.glider` | get + set |
| `items::ItemInfo.swappable` | get + set |
| `items::ItemInfo.equip_dispensable` | get + set |
| `items::ItemInfo.equip_damage_on_hurt` | get + set |
| `items::ItemInfo.equip_allowed` | get + set |
| `items::ItemInfo.equip_sound` | get + set |
| `items::ItemInfo.jukebox_song` | get + set |
| `items::ItemInfo.crafting_remainder` | get + set |
| `items::ItemInfo.dye` | get + set |
| `items::ItemInfo.cushion_color` | get + set |
| `items::ItemInfo.block_transformer` | get + set |
| `items::ItemInfo.bundle` | get + set |
| `items::ItemInfo.components` | get + set |
| `items::ItemInfo.custom_components` | get + set |
| `items::BlockReduction.horizontal_angle` | get + set |
| `items::BlockReduction.types` | get + set |
| `items::BlockReduction.base` | get + set |
| `items::BlockReduction.factor` | get + set |
| `items::BlocksAttacks.delay_seconds` | get + set |
| `items::BlocksAttacks.disable_scale` | get + set |
| `items::BlocksAttacks.reductions` | get + set |
| `items::BlocksAttacks.item_damage` | get + set |
| `items::BlocksAttacks.bypassed_by` | get + set |
| `items::BlocksAttacks.block_sound` | get + set |
| `items::BlocksAttacks.disabled_sound` | get + set |
| `items::ItemStack.item` | get + set |
| `items::ItemStack.count` | get + set |
| `items::ItemStack.damage` | get + set |
| `items::ItemStack.extra` | no accessor: field `0` of `Wire` is private to another module |
| `items::Items.items` | no accessor: generic type `horizon_world::consumable::Consumable` requires a listed instantiation |
| `items::Items.order` | get + set |
| `items::Items.registry_order` | get + set |
| `items::Items.block_tags` | get + set |
| `items::Items.item_tags` | get + set |
| `items::Items.item_tag_order` | get + set |
| `items::Items.loot` | no accessor: field `0` of `LootTables` is private to another module |
| `items::Items.songs` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `items::Items.instruments` | get + set |
| `items::Items.enchantments` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `items::Song.sound` | get + set |
| `items::Song.length_ticks` | get + set |
| `items::Song.comparator` | get + set |
| `items::Song.description` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `items::Instrument.sound` | get + set |
| `items::Instrument.use_duration` | get + set |
| `items::Instrument.range` | get + set |
| `items::LootContext.items` | no accessor: `LootContext` carries a lifetime (cannot be a handle type) |
| `items::LootContext.block` | no accessor: `LootContext` carries a lifetime (cannot be a handle type) |
| `items::LootContext.properties` | no accessor: `LootContext` carries a lifetime (cannot be a handle type) |
| `items::LootContext.held` | no accessor: `LootContext` carries a lifetime (cannot be a handle type) |
| `items::LootContext.killed_by_player` | no accessor: `LootContext` carries a lifetime (cannot be a handle type) |
| `items::LootContext.explosion_radius` | no accessor: `LootContext` carries a lifetime (cannot be a handle type) |
| `items::LootContext.this_entity` | no accessor: `LootContext` carries a lifetime (cannot be a handle type) |
