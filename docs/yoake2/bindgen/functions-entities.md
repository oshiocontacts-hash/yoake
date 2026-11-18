# `entities`: every function (horizon-client/src/entities.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `entities::<ArmorStand as Clone>::clone` | 1442 | &self | derived (not counted) | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ArmorStand as Debug>::fmt` | 1442 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ArmorStand as EntityRules>::get_default_gravity` | 205 | &self | bound (a) | - |
| `entities::<ArmorStand as EntityRules>::is_no_gravity` | 197 | &self | bound (a) | - |
| `entities::<ArmorStand as EntityRules>::is_pushed_by_fluid` | 214 | &self | bound (a) | - |
| `entities::<ArmorStand as Joined>::join_seq` | 3656 | &self | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ArmorStand as Joined>::set_join_seq` | 3656 | &mut self | handle-required | field `0` of `JoinSeq` is private to another module |
| `entities::<Cushion as Clone>::clone` | 1348 | &self | derived (not counted) | - |
| `entities::<Cushion as Debug>::fmt` | 1348 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<Experience as Clone>::clone` | 1226 | &self | derived (not counted) | - |
| `entities::<Experience as Debug>::fmt` | 1226 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<Experience as Default>::default` | 1226 | - | derived (not counted) | - |
| `entities::<Experience as State>::fields` | 1225 | - | handle-required | external type `yoake_state::Field` (crate not loaded) |
| `entities::<Experience as State>::set` | 1225 | &mut self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `entities::<Experience as State>::snapshot` | 1225 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [return value] |
| `entities::<Experience as State>::type_name` | 1225 | - | bound (b, mechanical) | - |
| `entities::<Experience as State>::validate_set` | 1225 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `entities::<ExperienceOrb as Clone>::clone` | 910 | &self | derived (not counted) | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ExperienceOrb as Debug>::fmt` | 910 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ExperienceOrb as EntityRules>::get_default_gravity` | 188 | &self | bound (a) | - |
| `entities::<ExperienceOrb as EntityRules>::is_no_gravity` | 180 | &self | bound (a) | - |
| `entities::<ExperienceOrb as Joined>::join_seq` | 3656 | &self | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ExperienceOrb as Joined>::set_join_seq` | 3656 | &mut self | handle-required | field `0` of `JoinSeq` is private to another module |
| `entities::<ItemCells as Default>::default` | 779 | - | derived (not counted) | - |
| `entities::<ItemEntity as Clone>::clone` | 105 | &self | derived (not counted) | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ItemEntity as Debug>::fmt` | 105 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ItemEntity as EntityRules>::get_default_gravity` | 171 | &self | bound (a) | - |
| `entities::<ItemEntity as EntityRules>::is_no_gravity` | 163 | &self | bound (a) | - |
| `entities::<ItemEntity as Joined>::join_seq` | 3656 | &self | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ItemEntity as Joined>::set_join_seq` | 3656 | &mut self | handle-required | field `0` of `JoinSeq` is private to another module |
| `entities::<ItemEnvironmentTick as Debug>::fmt` | 143 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ItemFrame as Clone>::clone` | 1394 | &self | derived (not counted) | field `0` of `Wire` is private to another module [return value] |
| `entities::<ItemFrame as Debug>::fmt` | 1394 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ItemStep as Clone>::clone` | 220 | &self | derived (not counted) | - |
| `entities::<ObstacleGrid as Default>::default` | 711 | - | derived (not counted) | field `min` of `Aabb` is private to another module [return value] |
| `entities::<Painting as Clone>::clone` | 1282 | &self | derived (not counted) | - |
| `entities::<Painting as Debug>::fmt` | 1282 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<StandClimbing as ClimbingWorld>::block_position` | 1546 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::block_state` | 1560 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::in_block_state` | 1553 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::is_fall_flying` | 1539 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::is_spectator` | 1532 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::set_last_climbable_position` | 1570 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandMedium as Clone>::clone` | 1497 | &self | derived (not counted) | - |
| `entities::<StandMedium as Debug>::fmt` | 1497 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<StandMedium as PartialEq>::eq` | 1497 | &self | derived (not counted) | - |
| `entities::<StandStep as Clone>::clone` | 1574 | &self | derived (not counted) | - |
| `entities::ArmorStand::bounds` | 1798 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `entities::ArmorStand::environment_tick` | 1747 | &mut self | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument]; field `states` of `BlockRegistry` is private to another module [&argument] |
| `entities::ArmorStand::feet_are_leather_boots` | 1809 | &self | bound (a) | - |
| `entities::ArmorStand::filter_flag` | 1726 | - | bound (b, mechanical) | - |
| `entities::ArmorStand::interpolate` | 1655 | &mut self | bound (b, mechanical) | - |
| `entities::ArmorStand::is_disabled` | 1732 | &self | bound (b, mechanical) | - |
| `entities::ArmorStand::name_with_tag` | 1610 | &mut self | handle-required | field `0` of `Wire` is private to another module [&argument] |
| `entities::ArmorStand::new` | 1673 | - | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::ArmorStand::on_pos_legacy` | 1712 | &self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |
| `entities::ArmorStand::render_position` | 1638 | &self | bound (b, mechanical) | - |
| `entities::ArmorStand::render_yaw` | 1645 | &self | bound (a) | - |
| `entities::ArmorStand::snap` | 1666 | &mut self | bound (b, mechanical) | - |
| `entities::ArmorStand::swap_locked` | 1738 | &self | bound (b, mechanical) | - |
| `entities::ArmorStand::tick_move` | 1828 | &mut self | excluded | field `center` of `BorderGeometry` is private to another module [Option<&> argument]; lifetime-carrying type `crate::collisions::SceneCollisions`; lifetime-carrying type `horizon_world::climbing::ClimbingRegistry` |
| `entities::ArmorStand::tick_settle` | 1954 | &mut self | bound (b, mechanical) | - |
| `entities::Cushion::bounds` | 1363 | &self | bound (b, mechanical) | - |
| `entities::Cushion::color_name` | 1372 | &self | bound (b, mechanical) | - |
| `entities::Cushion::dismount` | 1388 | &self | bound (b, mechanical) | - |
| `entities::Cushion::item` | 1377 | &self | bound (a) | - |
| `entities::Cushion::seat` | 1383 | &self | bound (b, mechanical) | - |
| `entities::Experience::death_reward` | 1237 | &self | bound (a) | - |
| `entities::Experience::give` | 1264 | &mut self | bound (b, mechanical) | - |
| `entities::Experience::needed` | 1252 | - | bound (a) | - |
| `entities::Experience::respawn` | 1246 | &mut self | bound (a) | - |
| `entities::Experience::yoake_state_level` | 1225 | &self | bound (a) | - |
| `entities::Experience::yoake_state_progress` | 1225 | &self | bound (a) | - |
| `entities::Experience::yoake_state_total` | 1225 | &self | bound (a) | - |
| `entities::ExperienceOrb::bounds` | 949 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `entities::ExperienceOrb::box_at` | 941 | - | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `entities::ExperienceOrb::can_merge` | 997 | &self | bound (a) | - |
| `entities::ExperienceOrb::hurt` | 935 | &mut self | bound (a) | - |
| `entities::ExperienceOrb::icon` | 1010 | &self | bound (a) | - |
| `entities::ExperienceOrb::move_towards_closest_space` | 1157 | &mut self | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument] |
| `entities::ExperienceOrb::split` | 1002 | - | bound (a) | - |
| `entities::ExperienceOrb::tick` | 1026 | &mut self | excluded | `dyn Trait` values cannot cross the data ABI; external type `horizon_client_render::world::World` (crate not loaded) [&argument]; field `center` of `BorderGeometry` is private to another module [Option<&> argument]; field `states` of `FluidResources` is private to another module [&argument]; lifetime-carrying type `BlockCollisionRegistry` |
| `entities::ExperienceOrb::unstuck` | 963 | - | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |
| `entities::ItemCells::add` | 796 | &mut self | bound (b, mechanical) | - |
| `entities::ItemCells::cell` | 792 | - | bound (b, mechanical) | - |
| `entities::ItemCells::moved` | 806 | &mut self | bound (b, mechanical) | - |
| `entities::ItemCells::near` | 816 | &self | handle-required | field `min` of `Aabb` is private to another module |
| `entities::ItemCells::new` | 784 | - | handle-required | field `0` of `JoinSeq` is private to another module |
| `entities::ItemCells::remove` | 800 | &mut self | bound (b, mechanical) | - |
| `entities::ItemEntity::apply_honey_slide` | 651 | &mut self | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument] |
| `entities::ItemEntity::bounds` | 265 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `entities::ItemEntity::environment_tick` | 277 | &mut self | excluded | external type `horizon_client_render::world::World` (crate not loaded) [&argument]; field `0` of `Wire` is private to another module [return value]; field `states` of `BlockRegistry` is private to another module [&argument]; generic type `horizon_world::consumable::Consumable` requires a listed instantiation |
| `entities::ItemEntity::has_infinite_pickup_delay` | 694 | &self | bound (a) | - |
| `entities::ItemEntity::honey_block_inside` | 668 | &mut self | bound (a) | - |
| `entities::ItemEntity::hurt` | 339 | &mut self | handle-required | field `0` of `Wire` is private to another module [return value] |
| `entities::ItemEntity::inside_block` | 356 | &self | excluded | external type `horizon_client_render::world::World` (crate not loaded) [&argument]; lifetime-carrying type `BlockCollisionRegistry` |
| `entities::ItemEntity::mergeable` | 700 | &self | bound (a) | - |
| `entities::ItemEntity::move_towards_closest_space` | 370 | &mut self | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument] |
| `entities::ItemEntity::piston_slime_bounce` | 641 | &mut self | bound (b, mechanical) | - |
| `entities::ItemEntity::slime_step_on` | 618 | &mut self | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument] |
| `entities::ItemEntity::stuck_multiplier` | 594 | &self | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument] |
| `entities::ItemEntity::tick_move` | 390 | &mut self | excluded | external type `horizon_client_render::world::World` (crate not loaded) [&argument]; field `center` of `BorderGeometry` is private to another module [Option<&> argument]; field `min` of `Aabb` is private to another module; lifetime-carrying type `BlockCollisionRegistry` |
| `entities::ItemEntity::tick_settle` | 518 | &mut self | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument] |
| `entities::ItemEnvironmentTick::should_play_lava_hurt_sound` | 153 | &self | bound (a) | - |
| `entities::ItemFrame::bounds` | 1430 | &self | bound (b, mechanical) | - |
| `entities::ItemFrame::kind` | 1421 | &self | bound (b, mechanical) | - |
| `entities::ItemFrame::step` | 1410 | &self | bound (a) | - |
| `entities::ObstacleGrid::is_empty` | 744 | &self | bound (a) | - |
| `entities::ObstacleGrid::near` | 749 | &self | handle-required | field `min` of `Aabb` is private to another module; field `min` of `Aabb` is private to another module [&mut argument] |
| `entities::ObstacleGrid::new` | 718 | - | handle-required | field `min` of `Aabb` is private to another module; field `min` of `Aabb` is private to another module [return value] |
| `entities::ObstacleGrid::span` | 733 | - | handle-required | field `min` of `Aabb` is private to another module |
| `entities::Painting::bounds` | 1305 | &self | bound (b, mechanical) | - |
| `entities::Painting::left` | 1300 | &self | bound (a) | - |
| `entities::Painting::step` | 1295 | &self | bound (a) | - |
| `entities::StandMedium::from_heights` | 1508 | - | bound (b, mechanical) | - |
| `entities::dye_index` | 1338 | - | bound (a) | - |
| `entities::item_support_is_slime` | 78 | - | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument] |
| `entities::merge_with_neighbours` | 847 | - | excluded | `dyn Trait` values cannot cross the data ABI; field `0` of `JoinSeq` is private to another module |
| `entities::move_applies` | 56 | - | bound (b, mechanical) | - |
| `entities::slime_at` | 88 | - | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument] |
| `entities::slime_step_on_scale` | 64 | - | bound (b, mechanical) | - |
| `entities::slime_support_name` | 70 | - | bound (b, mechanical) | - |
| `entities::towards_closest_space` | 1174 | - | handle-required | external type `horizon_client_render::world::World` (crate not loaded) [&argument]; field `min` of `Aabb` is private to another module |
| `entities::vertical_collision_velocity` | 36 | - | bound (a) | - |

## Fields

| field | status |
|---|---|
| `entities::ItemEntity.join` | no accessor: field `0` of `JoinSeq` is private to another module |
| `entities::ItemEntity.health` | get + set |
| `entities::ItemEntity.fire_ticks` | get + set |
| `entities::ItemEntity.id` | get + set |
| `entities::ItemEntity.stack` | no accessor: field `0` of `Wire` is private to another module |
| `entities::ItemEntity.thrower` | get + set |
| `entities::ItemEntity.target` | get + set |
| `entities::ItemEntity.position` | get + set |
| `entities::ItemEntity.previous` | get + set |
| `entities::ItemEntity.velocity` | get + set |
| `entities::ItemEntity.on_ground` | get + set |
| `entities::ItemEntity.age` | get + set |
| `entities::ItemEntity.pickup_delay` | get + set |
| `entities::ItemEntity.bob_offset` | get + set |
| `entities::ItemEntity.ticks` | get + set |
| `entities::ItemEntity.needs_sync` | get + set |
| `entities::ItemEntity.stuck` | get + set |
| `entities::ItemEntity.no_gravity` | get + set |
| `entities::ItemEntity.vertical_collision` | get + set |
| `entities::ItemEntity.horizontal_collision` | get + set |
| `entities::ItemEnvironmentTick.alive` | get + set |
| `entities::ItemEnvironmentTick.lava_hurt` | get + set |
| `entities::ItemEnvironmentTick.spilled` | no accessor: field `0` of `Wire` is private to another module |
| `entities::ItemStep.old_velocity` | get + set |
| `entities::ItemStep.moved` | get + set |
| `entities::ObstacleGrid.cells` | get + set |
| `entities::ObstacleGrid.boxes` | no accessor: field `min` of `Aabb` is private to another module |
| `entities::ItemCells.cells` | get + set |
| `entities::ExperienceOrb.join` | no accessor: field `0` of `JoinSeq` is private to another module |
| `entities::ExperienceOrb.id` | get + set |
| `entities::ExperienceOrb.position` | get + set |
| `entities::ExperienceOrb.previous` | get + set |
| `entities::ExperienceOrb.velocity` | get + set |
| `entities::ExperienceOrb.value` | get + set |
| `entities::ExperienceOrb.age` | get + set |
| `entities::ExperienceOrb.on_ground` | get + set |
| `entities::ExperienceOrb.count` | get + set |
| `entities::ExperienceOrb.ticks` | get + set |
| `entities::ExperienceOrb.health` | get + set |
| `entities::ExperienceOrb.no_gravity` | get + set |
| `entities::Experience.level` | get + set |
| `entities::Experience.progress` | get + set |
| `entities::Experience.total` | get + set |
| `entities::Painting.id` | get + set |
| `entities::Painting.pos` | get + set |
| `entities::Painting.facing` | get + set |
| `entities::Painting.variant` | get + set |
| `entities::Painting.width` | get + set |
| `entities::Painting.height` | get + set |
| `entities::Painting.check_interval` | get + set |
| `entities::Cushion.id` | get + set |
| `entities::Cushion.position` | get + set |
| `entities::Cushion.pos` | get + set |
| `entities::Cushion.yaw` | get + set |
| `entities::Cushion.color` | get + set |
| `entities::Cushion.check_interval` | get + set |
| `entities::ItemFrame.id` | get + set |
| `entities::ItemFrame.pos` | get + set |
| `entities::ItemFrame.facing` | get + set |
| `entities::ItemFrame.glow` | get + set |
| `entities::ItemFrame.item` | no accessor: field `0` of `Wire` is private to another module |
| `entities::ItemFrame.rotation` | get + set |
| `entities::ItemFrame.drop_chance` | get + set |
| `entities::ItemFrame.fixed` | get + set |
| `entities::ItemFrame.invisible` | get + set |
| `entities::ItemFrame.check_interval` | get + set |
| `entities::ArmorStand.join` | no accessor: field `0` of `JoinSeq` is private to another module |
| `entities::ArmorStand.fire_ticks` | get + set |
| `entities::ArmorStand.health` | get + set |
| `entities::ArmorStand.id` | get + set |
| `entities::ArmorStand.position` | get + set |
| `entities::ArmorStand.previous` | get + set |
| `entities::ArmorStand.velocity` | get + set |
| `entities::ArmorStand.on_ground` | get + set |
| `entities::ArmorStand.yaw` | get + set |
| `entities::ArmorStand.previous_yaw` | get + set |
| `entities::ArmorStand.equipment` | no accessor: field `0` of `Wire` is private to another module |
| `entities::ArmorStand.small` | get + set |
| `entities::ArmorStand.show_arms` | get + set |
| `entities::ArmorStand.no_base_plate` | get + set |
| `entities::ArmorStand.invisible` | get + set |
| `entities::ArmorStand.marker` | get + set |
| `entities::ArmorStand.no_gravity` | get + set |
| `entities::ArmorStand.custom_name` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `entities::ArmorStand.name_visible` | get + set |
| `entities::ArmorStand.last_hit` | get + set |
| `entities::ArmorStand.pose` | get + set |
| `entities::ArmorStand.vehicle` | get + set |
| `entities::ArmorStand.disabled_slots` | get + set |
| `entities::ArmorStand.enchantment_block` | get + set |
| `entities::ArmorStand.enchantment_fall_distance` | get + set |
| `entities::ArmorStand.enchantment_equipment` | no accessor: field `0` of `Wire` is private to another module |
| `entities::ArmorStand.support` | get + set |
| `entities::StandClimbing.world` | no accessor: `StandClimbing` carries a lifetime (cannot be a handle type) |
| `entities::StandClimbing.position` | no accessor: `StandClimbing` carries a lifetime (cannot be a handle type) |
| `entities::StandStep.falling` | get + set |
| `entities::StandStep.friction` | get + set |
