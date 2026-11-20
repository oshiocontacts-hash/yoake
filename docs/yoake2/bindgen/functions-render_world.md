# `render_world`: every function (horizon-client-render/src/world.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `render_world::<DirtySections as Clone>::clone` | 15 | &self | derived (not counted) | field `0` of `SectionMask` is private to another module [return value] |
| `render_world::<DirtySections as Debug>::fmt` | 15 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `render_world::<DirtySections as Default>::default` | 15 | - | derived (not counted) | field `0` of `SectionMask` is private to another module [return value] |
| `render_world::<HealthRules as Clone>::clone` | 551 | &self | derived (not counted) | - |
| `render_world::<HealthRules as Debug>::fmt` | 551 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `render_world::<HealthRules as Default>::default` | 563 | - | bound (b, mechanical) | - |
| `render_world::<Weather as Clone>::clone` | 128 | &self | derived (not counted) | - |
| `render_world::<Weather as Debug>::fmt` | 128 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `render_world::<Weather as Default>::default` | 128 | - | derived (not counted) | - |
| `render_world::DirtySections::clear` | 43 | &mut self | bound (a) | - |
| `render_world::DirtySections::columns` | 40 | &self | excluded | `impl Trait` requires a listed instantiation |
| `render_world::DirtySections::contains` | 24 | &self | bound (b, mechanical) | - |
| `render_world::DirtySections::insert` | 20 | &mut self | bound (b, mechanical) | - |
| `render_world::DirtySections::is_empty` | 46 | &self | bound (a) | - |
| `render_world::DirtySections::len` | 50 | &self | bound (b, mechanical) | - |
| `render_world::DirtySections::retain_columns` | 36 | &mut self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |
| `render_world::DirtySections::take_column` | 32 | &mut self | handle-required | field `0` of `SectionMask` is private to another module [return value] |
| `render_world::Weather::after_sleep` | 139 | &mut self | bound (a) | - |
| `render_world::Weather::is_raining` | 201 | &self | bound (a) | - |
| `render_world::Weather::is_thundering` | 205 | &self | bound (a) | - |
| `render_world::Weather::levels` | 194 | &self | bound (b, mechanical) | - |
| `render_world::Weather::tick_levels` | 185 | &mut self | bound (a) | - |
| `render_world::Weather::tick_timers` | 150 | &mut self | handle-required | field `state` of `LegacyRandom` is private to another module [&mut argument] |
| `render_world::World::block` | 406 | &self | bound (b, mechanical) | - |
| `render_world::World::block_entity_tickers` | 447 | &mut self | bound (b, mechanical) | - |
| `render_world::World::block_lock` | 472 | &self | handle-required | a reference nested inside another type needs a handle [return value] |
| `render_world::World::bottom_y` | 308 | &self | bound (a) | - |
| `render_world::World::forget_block_entity_source` | 520 | &mut self | bound (b, mechanical) | - |
| `render_world::World::is_air` | 463 | &self | bound (b, mechanical) | - |
| `render_world::World::layout` | 304 | &self | handle-required | field `min_y` of `ColumnLayout` is private to another module [return value] |
| `render_world::World::light` | 458 | &self | bound (a) | - |
| `render_world::World::loaded` | 466 | &self | bound (a) | - |
| `render_world::World::may_hold` | 415 | &self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |
| `render_world::World::motion_blocking` | 400 | &self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |
| `render_world::World::new` | 210 | - | handle-required | field `entries` of `Attachments` is private to another module [return value] |
| `render_world::World::set` | 318 | &mut self | excluded | generic type alias `crate::Result` |
| `render_world::World::set_block_lock` | 486 | &mut self | bound (b, mechanical) | - |
| `render_world::World::test_scene` | 248 | - | excluded | a reference nested inside another type needs a handle [&argument]; field `choices` of `StateModels` is private to another module [&argument]; generic type alias `crate::Result` |
| `render_world::World::top_y` | 312 | &self | bound (a) | - |
| `render_world::World::with_chunk_extras` | 528 | &mut self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |
| `render_world::World::world_surface` | 391 | &self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |

## Fields

| field | status |
|---|---|
| `render_world::DirtySections.columns` | no accessor: field `0` of `SectionMask` is private to another module |
| `render_world::World.attachments` | no accessor: field `entries` of `Attachments` is private to another module |
| `render_world::World.attachment_syncs` | no accessor: `Holder` is #[non_exhaustive] in another crate |
| `render_world::World.shape` | get + set |
| `render_world::World.fast_lava` | get + set |
| `render_world::World.chunks` | no accessor: field `columns` of `ChunkMap` is private to another module |
| `render_world::World.random` | no accessor: field `state` of `LegacyRandom` is private to another module |
| `render_world::World.revision` | get + set |
| `render_world::World.health_rules` | get + set |
| `render_world::World.air` | get + set |
| `render_world::World.dirty_sections` | no accessor: field `0` of `SectionMask` is private to another module |
| `render_world::World.dirty_light` | get + set |
| `render_world::World.light_edits` | get + set |
| `render_world::World.loaded_entities` | get + set |
| `render_world::World.loaded_chunks` | get + set |
| `render_world::World.modified` | get + set |
| `render_world::World.block_entities` | no accessor: field `map` of `BlockEntities` is private to another module |
| `render_world::World.chest_openers` | get + set |
| `render_world::World.block_names` | get + set |
| `render_world::World.watched` | get + set |
| `render_world::World.watched_writes` | get + set |
| `render_world::World.inhabited` | get + set |
| `render_world::World.game_time` | get + set |
| `render_world::World.day_time` | get + set |
| `render_world::World.weather` | get + set |
| `render_world::World.block_log` | get + set |
| `render_world::World.path_writes` | get + set |
| `render_world::World.bulk_log` | get + set |
| `render_world::World.light_log` | no accessor: field `0` of `SectionMask` is private to another module |
| `render_world::World.pois` | no accessor: generic type `crate::poi_sections::PoiSections` requires a listed instantiation |
| `render_world::World.moving_piston_fallback` | get + set |
| `render_world::World.moving_pistons` | get + set |
| `render_world::World.shulkers` | get + set |
| `render_world::Weather.clear_time` | get + set |
| `render_world::Weather.rain_time` | get + set |
| `render_world::Weather.thunder_time` | get + set |
| `render_world::Weather.raining` | get + set |
| `render_world::Weather.thundering` | get + set |
| `render_world::Weather.rain_level` | get + set |
| `render_world::Weather.thunder_level` | get + set |
| `render_world::HealthRules.difficulty` | get + set |
| `render_world::HealthRules.natural_regeneration` | get + set |
| `render_world::HealthRules.drowning_damage` | get + set |
| `render_world::HealthRules.fall_damage` | get + set |
| `render_world::HealthRules.fire_damage` | get + set |
| `render_world::HealthRules.freeze_damage` | get + set |
