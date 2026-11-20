# `entities`: every function (horizon-client/src/entities.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `entities::<ArmorStand as Clone>::clone` | 1813 | &self | derived (not counted) | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ArmorStand as Debug>::fmt` | 1813 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ArmorStand as EntityRules>::get_default_gravity` | 261 | &self | bound (a) | - |
| `entities::<ArmorStand as EntityRules>::is_no_gravity` | 257 | &self | bound (a) | - |
| `entities::<ArmorStand as EntityRules>::is_pushed_by_fluid` | 266 | &self | bound (a) | - |
| `entities::<ArmorStand as Joined>::join_seq` | 4113 | &self | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ArmorStand as Joined>::set_join_seq` | 4113 | &mut self | handle-required | field `0` of `JoinSeq` is private to another module |
| `entities::<Cushion as Clone>::clone` | 1683 | &self | derived (not counted) | - |
| `entities::<Cushion as Debug>::fmt` | 1683 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<Experience as Clone>::clone` | 1517 | &self | derived (not counted) | - |
| `entities::<Experience as Debug>::fmt` | 1517 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<Experience as Default>::default` | 1517 | - | derived (not counted) | - |
| `entities::<Experience as State>::fields` | 1516 | - | handle-required | external type `yoake_state::Field` (crate not loaded) |
| `entities::<Experience as State>::set` | 1516 | &mut self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `entities::<Experience as State>::snapshot` | 1516 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [return value] |
| `entities::<Experience as State>::type_name` | 1516 | - | bound (b, mechanical) | - |
| `entities::<Experience as State>::validate_set` | 1516 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `entities::<ExperienceOrb as Clone>::clone` | 1141 | &self | derived (not counted) | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ExperienceOrb as Debug>::fmt` | 1141 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ExperienceOrb as EntityRules>::get_default_gravity` | 226 | &self | bound (a) | - |
| `entities::<ExperienceOrb as EntityRules>::is_no_gravity` | 222 | &self | bound (a) | - |
| `entities::<ExperienceOrb as Joined>::join_seq` | 4113 | &self | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ExperienceOrb as Joined>::set_join_seq` | 4113 | &mut self | handle-required | field `0` of `JoinSeq` is private to another module |
| `entities::<ItemCells as Default>::default` | 988 | - | derived (not counted) | - |
| `entities::<ItemEntity as Clone>::clone` | 126 | &self | derived (not counted) | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ItemEntity as Debug>::fmt` | 126 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ItemEntity as EntityRules>::get_default_gravity` | 191 | &self | bound (a) | - |
| `entities::<ItemEntity as EntityRules>::is_no_gravity` | 187 | &self | bound (a) | - |
| `entities::<ItemEntity as Joined>::join_seq` | 4113 | &self | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::<ItemEntity as Joined>::set_join_seq` | 4113 | &mut self | handle-required | field `0` of `JoinSeq` is private to another module |
| `entities::<ItemEnvironmentTick as Debug>::fmt` | 164 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ItemFrame as Clone>::clone` | 1752 | &self | derived (not counted) | field `0` of `Wire` is private to another module [return value] |
| `entities::<ItemFrame as Debug>::fmt` | 1752 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<ItemStep as Clone>::clone` | 310 | &self | derived (not counted) | - |
| `entities::<ObstacleGrid as Default>::default` | 892 | - | derived (not counted) | field `min` of `Aabb` is private to another module [return value] |
| `entities::<Painting as Clone>::clone` | 1601 | &self | derived (not counted) | - |
| `entities::<Painting as Debug>::fmt` | 1601 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<StandClimbing as ClimbingWorld>::block_position` | 1912 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::block_state` | 1918 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::in_block_state` | 1915 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::is_fall_flying` | 1909 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::is_spectator` | 1906 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandClimbing as ClimbingWorld>::set_last_climbable_position` | 1924 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `entities::<StandMedium as Clone>::clone` | 1868 | &self | derived (not counted) | - |
| `entities::<StandMedium as Debug>::fmt` | 1868 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `entities::<StandMedium as PartialEq>::eq` | 1868 | &self | derived (not counted) | - |
| `entities::<StandStep as Clone>::clone` | 1928 | &self | derived (not counted) | - |
| `entities::ArmorStand::__yoake_trait_entity_rules_get_default_gravity` | 285 | &self | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::ArmorStand::__yoake_trait_entity_rules_is_no_gravity` | 273 | &self | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::ArmorStand::__yoake_trait_entity_rules_is_pushed_by_fluid` | 297 | &self | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::ArmorStand::bounds` | 2227 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `entities::ArmorStand::environment_tick` | 2165 | &mut self | handle-required | field `entries` of `Attachments` is private to another module [&argument]; field `states` of `BlockRegistry` is private to another module [&argument] |
| `entities::ArmorStand::feet_are_leather_boots` | 2245 | &self | bound (a) | - |
| `entities::ArmorStand::filter_flag` | 2123 | - | bound (b, mechanical) | - |
| `entities::ArmorStand::interpolate` | 2039 | &mut self | bound (b, mechanical) | - |
| `entities::ArmorStand::is_disabled` | 2136 | &self | bound (b, mechanical) | - |
| `entities::ArmorStand::name_with_tag` | 1973 | &mut self | handle-required | field `0` of `Wire` is private to another module [&argument] |
| `entities::ArmorStand::new` | 2071 | - | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `entities::ArmorStand::on_pos_legacy` | 2107 | &self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |
| `entities::ArmorStand::render_position` | 2008 | &self | bound (b, mechanical) | - |
| `entities::ArmorStand::render_yaw` | 2022 | &self | bound (a) | - |
| `entities::ArmorStand::snap` | 2057 | &mut self | bound (b, mechanical) | - |
| `entities::ArmorStand::swap_locked` | 2149 | &self | bound (b, mechanical) | - |
| `entities::ArmorStand::tick_move` | 2271 | &mut self | excluded | field `center` of `BorderGeometry` is private to another module [Option<&> argument]; lifetime-carrying type `crate::collisions::SceneCollisions`; lifetime-carrying type `horizon_world::climbing::ClimbingRegistry` |
| `entities::ArmorStand::tick_settle` | 2406 | &mut self | bound (b, mechanical) | - |
| `entities::Cushion::bounds` | 1700 | &self | bound (b, mechanical) | - |
| `entities::Cushion::color_name` | 1712 | &self | bound (b, mechanical) | - |
| `entities::Cushion::dismount` | 1741 | &self | bound (b, mechanical) | - |
| `entities::Cushion::item` | 1724 | &self | bound (a) | - |
| `entities::Cushion::seat` | 1733 | &self | bound (b, mechanical) | - |
| `entities::Experience::death_reward` | 1530 | &self | bound (a) | - |
| `entities::Experience::give` | 1578 | &mut self | bound (b, mechanical) | - |
| `entities::Experience::needed` | 1559 | - | bound (a) | - |
| `entities::Experience::respawn` | 1546 | &mut self | bound (a) | - |
| `entities::Experience::yoake_state_level` | 1516 | &self | bound (a) | - |
| `entities::Experience::yoake_state_progress` | 1516 | &self | bound (a) | - |
| `entities::Experience::yoake_state_total` | 1516 | &self | bound (a) | - |
| `entities::ExperienceOrb::__yoake_trait_entity_rules_get_default_gravity` | 245 | &self | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::ExperienceOrb::__yoake_trait_entity_rules_is_no_gravity` | 233 | &self | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::ExperienceOrb::bounds` | 1196 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `entities::ExperienceOrb::box_at` | 1181 | - | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `entities::ExperienceOrb::can_merge` | 1247 | &self | bound (a) | - |
| `entities::ExperienceOrb::hurt` | 1168 | &mut self | bound (a) | - |
| `entities::ExperienceOrb::icon` | 1274 | &self | bound (a) | - |
| `entities::ExperienceOrb::move_towards_closest_space` | 1440 | &mut self | handle-required | field `entries` of `Attachments` is private to another module [&argument] |
| `entities::ExperienceOrb::split` | 1259 | - | bound (a) | - |
| `entities::ExperienceOrb::tick` | 1297 | &mut self | excluded | `dyn Trait` values cannot cross the data ABI; field `center` of `BorderGeometry` is private to another module [Option<&> argument]; field `entries` of `Attachments` is private to another module [&argument]; field `states` of `FluidResources` is private to another module [&argument]; lifetime-carrying type `BlockCollisionRegistry` |
| `entities::ExperienceOrb::unstuck` | 1211 | - | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built |
| `entities::ItemCells::add` | 1013 | &mut self | bound (b, mechanical) | - |
| `entities::ItemCells::cell` | 1006 | - | bound (b, mechanical) | - |
| `entities::ItemCells::moved` | 1033 | &mut self | bound (b, mechanical) | - |
| `entities::ItemCells::near` | 1050 | &self | handle-required | field `min` of `Aabb` is private to another module |
| `entities::ItemCells::new` | 995 | - | handle-required | field `0` of `JoinSeq` is private to another module |
| `entities::ItemCells::remove` | 1020 | &mut self | bound (b, mechanical) | - |
| `entities::ItemEntity::__yoake_trait_entity_rules_get_default_gravity` | 210 | &self | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::ItemEntity::__yoake_trait_entity_rules_is_no_gravity` | 198 | &self | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::ItemEntity::apply_honey_slide` | 804 | &mut self | handle-required | field `entries` of `Attachments` is private to another module [&argument] |
| `entities::ItemEntity::bounds` | 357 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `entities::ItemEntity::environment_tick` | 376 | &mut self | excluded | field `0` of `Wire` is private to another module [return value]; field `entries` of `Attachments` is private to another module [&argument]; field `states` of `BlockRegistry` is private to another module [&argument]; generic type `horizon_world::consumable::Consumable` requires a listed instantiation |
| `entities::ItemEntity::has_infinite_pickup_delay` | 863 | &self | bound (a) | - |
| `entities::ItemEntity::honey_block_inside` | 830 | &mut self | bound (a) | - |
| `entities::ItemEntity::hurt` | 447 | &mut self | handle-required | field `0` of `Wire` is private to another module [return value] |
| `entities::ItemEntity::inside_block` | 471 | &self | excluded | field `entries` of `Attachments` is private to another module [&argument]; lifetime-carrying type `BlockCollisionRegistry` |
| `entities::ItemEntity::mergeable` | 876 | &self | bound (a) | - |
| `entities::ItemEntity::move_towards_closest_space` | 492 | &mut self | handle-required | field `entries` of `Attachments` is private to another module [&argument] |
| `entities::ItemEntity::piston_slime_bounce` | 787 | &mut self | bound (b, mechanical) | - |
| `entities::ItemEntity::slime_step_on` | 757 | &mut self | handle-required | field `entries` of `Attachments` is private to another module [&argument] |
| `entities::ItemEntity::stuck_multiplier` | 726 | &self | handle-required | field `entries` of `Attachments` is private to another module [&argument] |
| `entities::ItemEntity::tick_move` | 513 | &mut self | excluded | field `center` of `BorderGeometry` is private to another module [Option<&> argument]; field `entries` of `Attachments` is private to another module [&argument]; field `min` of `Aabb` is private to another module; lifetime-carrying type `BlockCollisionRegistry` |
| `entities::ItemEntity::tick_settle` | 643 | &mut self | handle-required | field `entries` of `Attachments` is private to another module [&argument] |
| `entities::ItemEnvironmentTick::should_play_lava_hurt_sound` | 176 | &self | bound (a) | - |
| `entities::ItemFrame::bounds` | 1796 | &self | bound (b, mechanical) | - |
| `entities::ItemFrame::kind` | 1784 | &self | bound (b, mechanical) | - |
| `entities::ItemFrame::step` | 1770 | &self | bound (a) | - |
| `entities::ObstacleGrid::is_empty` | 941 | &self | bound (a) | - |
| `entities::ObstacleGrid::near` | 953 | &self | handle-required | field `min` of `Aabb` is private to another module; field `min` of `Aabb` is private to another module [&mut argument] |
| `entities::ObstacleGrid::new` | 901 | - | handle-required | field `min` of `Aabb` is private to another module; field `min` of `Aabb` is private to another module [return value] |
| `entities::ObstacleGrid::span` | 923 | - | handle-required | field `min` of `Aabb` is private to another module |
| `entities::Painting::bounds` | 1632 | &self | bound (b, mechanical) | - |
| `entities::Painting::left` | 1624 | &self | bound (a) | - |
| `entities::Painting::step` | 1616 | &self | bound (a) | - |
| `entities::StandMedium::from_heights` | 1881 | - | bound (b, mechanical) | - |
| `entities::__yoake_cs_207` | 4168 | - | excluded | field `entries` of `Attachments` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_211` | 4178 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_212` | 4186 | - | excluded | field `0` of `Wire` is private to another module [&argument]; generic type `horizon_world::consumable::Consumable` requires a listed instantiation; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_213` | 4195 | - | excluded | field `0` of `Wire` is private to another module [&argument]; generic type `horizon_world::consumable::Consumable` requires a listed instantiation; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_215` | 4204 | - | excluded | field `0` of `JoinSeq` is private to another module [&mut argument]; field `0` of `Wire` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_218` | 4212 | - | excluded | field `0` of `Wire` is private to another module [&argument]; generic type `horizon_world::consumable::Consumable` requires a listed instantiation; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_220` | 4221 | - | excluded | field `0` of `JoinSeq` is private to another module [&mut argument]; field `0` of `Wire` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_222` | 4229 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_224` | 4234 | - | excluded | field `0` of `Wire` is private to another module [&argument]; generic type `horizon_world::consumable::Consumable` requires a listed instantiation; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_226` | 4243 | - | excluded | field `0` of `JoinSeq` is private to another module [&mut argument]; field `0` of `Wire` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_228` | 4251 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_229` | 4256 | - | excluded | field `0` of `Wire` is private to another module [&argument]; generic type `horizon_world::consumable::Consumable` requires a listed instantiation; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_231` | 4264 | - | excluded | field `0` of `JoinSeq` is private to another module [&mut argument]; field `0` of `Wire` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_236` | 4272 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_266` | 4277 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_267` | 4282 | - | excluded | field `min` of `Aabb` is private to another module; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_268` | 4290 | - | excluded | field `min` of `Aabb` is private to another module; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_269` | 4295 | - | excluded | field `min` of `Aabb` is private to another module; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_276` | 4300 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_277` | 4308 | - | excluded | field `entries` of `Attachments` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_278` | 4318 | - | excluded | field `0` of `JoinSeq` is private to another module [&mut argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_281` | 4323 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_303` | 4328 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_322` | 4337 | - | excluded | field `entries` of `Attachments` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_326` | 4347 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_327` | 4352 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_329` | 4360 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_330` | 4365 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_333` | 4373 | - | excluded | field `min` of `Aabb` is private to another module; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_334` | 4381 | - | excluded | field `0` of `JoinSeq` is private to another module [&mut argument]; field `entries` of `Attachments` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_337` | 4391 | - | excluded | field `entries` of `Attachments` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_341` | 4401 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_342` | 4409 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_343` | 4414 | - | excluded | field `min` of `Aabb` is private to another module; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_344` | 4419 | - | excluded | field `min` of `Aabb` is private to another module; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_348` | 4424 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_349` | 4432 | - | excluded | field `entries` of `Attachments` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_350` | 4442 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_356` | 4447 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_357` | 4452 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_362` | 4457 | - | excluded | field `0` of `Wire` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_363` | 4462 | - | excluded | a reference nested inside another type needs a handle [return value]; field `0` of `Wire` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_367` | 4469 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_368` | 4474 | - | excluded | field `min` of `Aabb` is private to another module; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_369` | 4479 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_370` | 4484 | - | excluded | field `min` of `Aabb` is private to another module; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_377` | 4489 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_378` | 4497 | - | excluded | field `entries` of `Attachments` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_380` | 4507 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_382` | 4512 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_387` | 4517 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_388` | 4522 | - | excluded | field `entries` of `Attachments` is private to another module [&argument]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_392` | 4532 | - | excluded | hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_397` | 4540 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::__yoake_cs_398` | 4545 | - | excluded | field `0` of `JoinSeq` is private to another module [&argument]; field `min` of `Aabb` is private to another module [return value]; hook machinery (`__yoake_*`): not an engine function |
| `entities::dye_index` | 1672 | - | bound (a) | - |
| `entities::item_support_is_slime` | 95 | - | handle-required | field `entries` of `Attachments` is private to another module [&argument] |
| `entities::merge_with_neighbours` | 1078 | - | excluded | `dyn Trait` values cannot cross the data ABI; field `0` of `JoinSeq` is private to another module |
| `entities::move_applies` | 64 | - | bound (b, mechanical) | - |
| `entities::slime_at` | 108 | - | handle-required | field `entries` of `Attachments` is private to another module [&argument] |
| `entities::slime_step_on_scale` | 75 | - | bound (b, mechanical) | - |
| `entities::slime_support_name` | 84 | - | bound (b, mechanical) | - |
| `entities::towards_closest_space` | 1464 | - | handle-required | field `entries` of `Attachments` is private to another module [&argument]; field `min` of `Aabb` is private to another module |
| `entities::vertical_collision_velocity` | 38 | - | bound (a) | - |

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
