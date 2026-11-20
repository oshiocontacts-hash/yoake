# `mobs`: every function (horizon-client/src/mobs.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `mobs::<Aim as Clone>::clone` | 681 | &self | derived (not counted) | - |
| `mobs::<Aim as Debug>::fmt` | 681 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Aim as PartialEq>::eq` | 681 | &self | derived (not counted) | - |
| `mobs::<Attrs as Clone>::clone` | 871 | &self | derived (not counted) | - |
| `mobs::<Attrs as Debug>::fmt` | 871 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Brain as Clone>::clone` | 326 | &self | derived (not counted) | - |
| `mobs::<Brain as Debug>::fmt` | 326 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Brain as PartialEq>::eq` | 326 | &self | derived (not counted) | - |
| `mobs::<Damage as Clone>::clone` | 688 | &self | derived (not counted) | - |
| `mobs::<Damage as Debug>::fmt` | 688 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<DamageKind as Clone>::clone` | 597 | &self | derived (not counted) | - |
| `mobs::<DamageKind as Debug>::fmt` | 597 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<DamageKind as PartialEq>::eq` | 597 | &self | derived (not counted) | - |
| `mobs::<ElytraAnimation as Clone>::clone` | 3985 | &self | derived (not counted) | - |
| `mobs::<ElytraAnimation as Debug>::fmt` | 3985 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<ElytraAnimation as Default>::default` | 3985 | - | derived (not counted) | - |
| `mobs::<Extra as Clone>::clone` | 896 | &self | derived (not counted) | field `catalog` of `EntityAttributes` is private to another module [return value] |
| `mobs::<Extra as Debug>::fmt` | 896 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Extra as Default>::default` | 896 | - | derived (not counted) | field `catalog` of `EntityAttributes` is private to another module [return value] |
| `mobs::<Flags as Clone>::clone` | 340 | &self | derived (not counted) | - |
| `mobs::<Flags as Debug>::fmt` | 340 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Flags as Default>::default` | 340 | - | derived (not counted) | - |
| `mobs::<MannequinDescription as Clone>::clone` | 1368 | &self | derived (not counted) | - |
| `mobs::<MannequinDescription as Debug>::fmt` | 1368 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<MannequinDescription as Default>::default` | 1368 | - | derived (not counted) | - |
| `mobs::<MannequinDescription as PartialEq>::eq` | 1368 | &self | derived (not counted) | - |
| `mobs::<Mob as Clone>::clone` | 1217 | &self | derived (not counted) | field `0` of `JoinSeq` is private to another module [return value] |
| `mobs::<Mob as Debug>::fmt` | 1217 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Mob as EntityRules>::is_no_gravity` | 1355 | &self | bound (a) | - |
| `mobs::<Mob as Joined>::join_seq` | 5057 | &self | handle-required | field `0` of `JoinSeq` is private to another module [return value] |
| `mobs::<Mob as Joined>::set_join_seq` | 5057 | &mut self | handle-required | field `0` of `JoinSeq` is private to another module |
| `mobs::<Mob as State>::fields` | 1216 | - | handle-required | external type `yoake_state::Field` (crate not loaded) |
| `mobs::<Mob as State>::set` | 1216 | &mut self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `mobs::<Mob as State>::snapshot` | 1216 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [return value] |
| `mobs::<Mob as State>::type_name` | 1216 | - | bound (b, mechanical) | - |
| `mobs::<Mob as State>::validate_set` | 1216 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `mobs::<MobEvent as Clone>::clone` | 1399 | &self | derived (not counted) | a reference nested inside another type needs a handle [return value] |
| `mobs::<MobEvent as Debug>::fmt` | 1399 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<MobType as Clone>::clone` | 365 | &self | derived (not counted) | a reference nested inside another type needs a handle [return value] |
| `mobs::<MobType as Debug>::fmt` | 365 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<MobWorld as Clone>::clone` | 2686 | &self | derived (not counted) | lifetime-carrying type `MobWorld`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `mobs::<ModelNormals as Clone>::clone` | 149 | &self | derived (not counted) | external type `glam::f32::mat3::Mat3` (crate not loaded) [return value] |
| `mobs::<Part as Clone>::clone` | 232 | &self | derived (not counted) | - |
| `mobs::<Part as Debug>::fmt` | 232 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<PendingPushes as Default>::default` | 2478 | - | derived (not counted) | - |
| `mobs::<PlayerView as Clone>::clone` | 2608 | &self | derived (not counted) | field `min` of `Aabb` is private to another module [return value] |
| `mobs::<PlayerView as Debug>::fmt` | 2608 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Polygon as Clone>::clone` | 120 | &self | derived (not counted) | - |
| `mobs::<Polygon as Debug>::fmt` | 120 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Seen as Clone>::clone` | 2128 | &self | derived (not counted) | field `min` of `Aabb` is private to another module [return value] |
| `mobs::<Seen as Debug>::fmt` | 2128 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<SpawnSetup as Clone>::clone` | 2111 | &self | derived (not counted) | - |
| `mobs::<SpawnSetup as Debug>::fmt` | 2111 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<SpawnSetup as Default>::default` | 2111 | - | derived (not counted) | - |
| `mobs::<SwimAnimation as Clone>::clone` | 4032 | &self | derived (not counted) | - |
| `mobs::<SwimAnimation as Debug>::fmt` | 4032 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<SwimAnimation as Default>::default` | 4032 | - | derived (not counted) | - |
| `mobs::<Teams as Clone>::clone` | 2759 | &self | derived (not counted) | lifetime-carrying type `Teams`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `mobs::<Teams as Debug>::fmt` | 2759 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; receiver type carries a lifetime: it cannot be a hook-argument handle; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::<Teams as Default>::default` | 2759 | - | derived (not counted) | lifetime-carrying type `Teams` |
| `mobs::<Vehicle as Clone>::clone` | 2660 | &self | derived (not counted) | field `min` of `Aabb` is private to another module [return value] |
| `mobs::<Vehicle as Debug>::fmt` | 2660 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `mobs::Damage::bypasses_armor` | 791 | &self | bound (a) | - |
| `mobs::Damage::causes_panic` | 768 | &self | bound (a) | - |
| `mobs::Damage::fall` | 695 | - | bound (b, mechanical) | - |
| `mobs::Damage::fireworks` | 751 | - | bound (b, mechanical) | - |
| `mobs::Damage::indirect_magic` | 740 | - | bound (b, mechanical) | - |
| `mobs::Damage::is_fire` | 813 | &self | bound (a) | - |
| `mobs::Damage::is_projectile` | 858 | &self | bound (a) | - |
| `mobs::Damage::mace` | 720 | - | bound (b, mechanical) | - |
| `mobs::Damage::mob` | 759 | - | bound (b, mechanical) | - |
| `mobs::Damage::no_knockback` | 827 | &self | bound (a) | - |
| `mobs::Damage::of` | 703 | - | bound (b, mechanical) | - |
| `mobs::Damage::player` | 711 | - | bound (b, mechanical) | - |
| `mobs::Damage::spear` | 729 | - | bound (b, mechanical) | - |
| `mobs::DamageKind::bypasses_effects` | 671 | self | bound (b, mechanical) | - |
| `mobs::DamageKind::bypasses_invulnerability` | 667 | self | bound (b, mechanical) | - |
| `mobs::DamageKind::bypasses_resistance` | 675 | self | bound (b, mechanical) | - |
| `mobs::ElytraAnimation::angles` | 4024 | &self | bound (a) | - |
| `mobs::ElytraAnimation::tick` | 3997 | &mut self | bound (b, mechanical) | - |
| `mobs::EntityModelDocument::keys` | 3764 | &self | excluded | `impl Trait` requires a listed instantiation |
| `mobs::EntityModelDocument::load` | 3752 | - | excluded | generic type alias `crate::Result`; std type `std::path::Path` has no codec yet [&argument] |
| `mobs::EntityModelDocument::model` | 3759 | &self | bound (b, mechanical) | - |
| `mobs::Extra::main_hand_stack` | 1204 | &self | handle-required | a reference nested inside another type needs a handle [return value] |
| `mobs::Extra::set_main_hand` | 1194 | &mut self | bound (b, mechanical) | - |
| `mobs::Extra::set_main_hand_stack` | 1199 | &mut self | bound (b, mechanical) | - |
| `mobs::Extra::take_main_hand` | 1209 | &mut self | bound (b, mechanical) | - |
| `mobs::MannequinDescription::component` | 1377 | &self | bound (b, mechanical) | - |
| `mobs::MannequinDescription::from_component` | 1389 | - | bound (b, mechanical) | - |
| `mobs::Mob::__yoake_trait_entity_rules_is_no_gravity` | 1360 | &self | excluded | hook machinery (`__yoake_*`): not an engine function |
| `mobs::Mob::add_effect` | 3647 | &mut self | excluded | `&'static` argument: the engine keeps it, a value decoded for the call cannot be |
| `mobs::Mob::add_effect_with` | 3535 | &mut self | excluded | `&'static` argument: the engine keeps it, a value decoded for the call cannot be |
| `mobs::Mob::alive` | 2933 | &self | bound (a) | - |
| `mobs::Mob::ambient_interval` | 3694 | &self | bound (a) | - |
| `mobs::Mob::apply_heal_or_harm` | 3634 | &mut self | bound (a) | - |
| `mobs::Mob::bounds` | 2904 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `mobs::Mob::bounds_at` | 2909 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `mobs::Mob::burning_time` | 2301 | &self | bound (a) | - |
| `mobs::Mob::can_pick_up_loot` | 2896 | &self | bound (a) | - |
| `mobs::Mob::category` | 2429 | &self | bound (b, mechanical) | - |
| `mobs::Mob::center` | 3679 | &self | bound (a) | - |
| `mobs::Mob::command_name` | 2231 | &self | handle-required | external type `serde_json::value::Value` (crate not loaded) [return value] |
| `mobs::Mob::damage` | 3019 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::damage_aimed` | 2991 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::damage_projectile` | 3006 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::damage_with` | 3034 | &mut self | handle-required | a reference nested inside another type needs a handle |
| `mobs::Mob::distance_sqr` | 3488 | &self | bound (b, mechanical) | - |
| `mobs::Mob::distance_to` | 3479 | &self | bound (a) | - |
| `mobs::Mob::equipment_stacks` | 2252 | &self | bound (b, mechanical) | - |
| `mobs::Mob::extinguish_fire` | 2327 | &mut self | bound (a) | - |
| `mobs::Mob::has_effect` | 3515 | &self | bound (a) | - |
| `mobs::Mob::heal` | 3623 | &mut self | bound (a) | - |
| `mobs::Mob::hostile` | 2937 | &self | bound (a) | - |
| `mobs::Mob::hurt` | 2952 | &mut self | handle-required | field `state` of `LegacyRandom` is private to another module [&mut argument] |
| `mobs::Mob::hurt_wolf_armor` | 3343 | &mut self | bound (a) | - |
| `mobs::Mob::ignite_for_ticks` | 2316 | &mut self | bound (a) | - |
| `mobs::Mob::is_currently_glowing` | 3521 | &self | bound (a) | - |
| `mobs::Mob::is_fully_frozen` | 2342 | &self | bound (a) | - |
| `mobs::Mob::is_invisible` | 3527 | &self | bound (a) | - |
| `mobs::Mob::is_on_fire` | 2312 | &self | bound (a) | - |
| `mobs::Mob::knockback` | 3379 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::knockback_by` | 3396 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::knockback_effect` | 3386 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::knockback_resistance` | 3391 | &self | bound (a) | - |
| `mobs::Mob::main_arm_left` | 2947 | &self | bound (a) | - |
| `mobs::Mob::movement_speed` | 3496 | &self | bound (a) | - |
| `mobs::Mob::new` | 2773 | - | handle-required | a reference nested inside another type needs a handle; field `0` of `JoinSeq` is private to another module [return value] |
| `mobs::Mob::potion_harm` | 3310 | &self | bound (a) | - |
| `mobs::Mob::record_damage` | 2923 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::refuses_damage` | 3025 | &self | bound (b, mechanical) | - |
| `mobs::Mob::saddle_item` | 3850 | &self | bound (b, mechanical) | - |
| `mobs::Mob::seen` | 2349 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `mobs::Mob::set_saddle` | 3860 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::soak_absorption` | 3577 | &mut self | bound (a) | - |
| `mobs::Mob::sound` | 3687 | &self | excluded | `&'static` argument: the engine keeps it, a value decoded for the call cannot be; a reference nested inside another type needs a handle [&mut argument] |
| `mobs::Mob::species` | 2941 | &self | bound (b, mechanical) | - |
| `mobs::Mob::speed_bonus` | 3675 | &self | bound (a) | - |
| `mobs::Mob::tick` | 3447 | &mut self | excluded | a reference nested inside another type needs a handle [&mut argument]; field `state` of `LegacyRandom` is private to another module [&mut argument]; lifetime-carrying type `MobWorld` |
| `mobs::Mob::tick_avatar_pose` | 4065 | &mut self | bound (a) | - |
| `mobs::Mob::track_section` | 3466 | &mut self | bound (a) | - |
| `mobs::Mob::use_death_protection` | 3586 | &mut self | bound (b, mechanical) | - |
| `mobs::Mob::voice` | 3726 | &mut self | excluded | `&'static` argument: the engine keeps it, a value decoded for the call cannot be; a reference nested inside another type needs a handle [&mut argument] |
| `mobs::Mob::voice_pitch` | 3704 | &mut self | bound (a) | - |
| `mobs::Mob::wolf_armor_absorbs` | 3321 | &self | bound (b, mechanical) | - |
| `mobs::Mob::worn_armor` | 2968 | &self | bound (b, mechanical) | - |
| `mobs::Mob::yoake_state_absorption` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_air` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_fire_ticks` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_glowing` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_health` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_invulnerable_flag` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_no_ai` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_no_gravity` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_removed` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_silent` | 1216 | &self | bound (a) | - |
| `mobs::Mob::yoake_state_ticks_frozen` | 1216 | &self | bound (a) | - |
| `mobs::MobType::layer` | 418 | &self | handle-required | a reference nested inside another type needs a handle [return value] |
| `mobs::MobTypes::load` | 455 | - | excluded | generic type alias `crate::Result`; std type `std::path::Path` has no codec yet [&argument] |
| `mobs::MobTypes::load_for_server` | 463 | - | excluded | generic type alias `crate::Result`; std type `std::path::Path` has no codec yet [&argument] |
| `mobs::MobTypes::load_with_models` | 468 | - | excluded | generic type alias `crate::Result`; std type `std::path::Path` has no codec yet [&argument] |
| `mobs::ModelNormals::new` | 157 | - | handle-required | external type `glam::f32::mat3::Mat3` (crate not loaded) [return value]; external type `glam::f32::sse2::mat4::Mat4` (crate not loaded) |
| `mobs::ModelNormals::part` | 217 | self | handle-required | external type `glam::f32::mat3::Mat3` (crate not loaded) [by-value self]; external type `glam::f32::mat3::Mat3` (crate not loaded) [return value] |
| `mobs::ModelNormals::rotation` | 171 | - | handle-required | external type `glam::f32::mat3::Mat3` (crate not loaded) [return value] |
| `mobs::Part::parse` | 251 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [&argument] |
| `mobs::Part::set_cull_back` | 242 | &mut self | bound (a) | - |
| `mobs::PendingPushes::push` | 2485 | &mut self | bound (b, mechanical) | - |
| `mobs::PlayerView::has_effect` | 2681 | &self | bound (a) | - |
| `mobs::PlayerView::visibility_percent` | 2669 | &self | bound (a) | - |
| `mobs::Polygon::vertex_normal` | 129 | &self | handle-required | external type `glam::f32::mat3::Mat3` (crate not loaded) |
| `mobs::Seen::has_effect` | 2224 | &self | bound (a) | - |
| `mobs::SwimAnimation::amount` | 4055 | &self | bound (a) | - |
| `mobs::SwimAnimation::tick` | 4043 | &mut self | bound (a) | - |
| `mobs::Teams::of` | 2765 | &self | excluded | a reference nested inside another type needs a handle [return value]; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `mobs::anger_left` | 887 | - | bound (a) | - |
| `mobs::apply_brain_events` | 2553 | - | handle-required | a reference nested inside another type needs a handle [&mut argument]; field `0` of `JoinSeq` is private to another module |
| `mobs::apply_mob_pushes` | 2528 | - | handle-required | field `0` of `JoinSeq` is private to another module [&mut argument] |
| `mobs::blocks_changed` | 2501 | - | handle-required | field `0` of `JoinSeq` is private to another module [&mut argument] |
| `mobs::collision_changes` | 2515 | - | excluded | lifetime-carrying type `MobWorld` |
| `mobs::culls_back_faces` | 446 | - | bound (a) | - |
| `mobs::exact_attributes` | 424 | - | bound (b, mechanical) | - |
| `mobs::load_model` | 3774 | - | handle-required | std type `std::path::Path` has no codec yet [&argument] |
| `mobs::mob_can_use_portal` | 2452 | - | handle-required | field `0` of `JoinSeq` is private to another module [&argument] |
| `mobs::render` | 3736 | - | handle-required | external type `glam::f64::dvec3::DVec3` (crate not loaded); external type `horizon_client_render::mesher::Vertex` (crate not loaded) [&mut argument]; field `0` of `JoinSeq` is private to another module [&argument] |
| `mobs::render_model` | 3779 | - | excluded | `dyn Trait` values cannot cross the data ABI; external type `glam::f32::sse2::mat4::Mat4` (crate not loaded); external type `horizon_client_render::mesher::Vertex` (crate not loaded) [&mut argument] |
| `mobs::take_mob_pushes` | 2456 | - | handle-required | a reference nested inside another type needs a handle [&mut argument] |

## Fields

| field | status |
|---|---|
| `mobs::Polygon.normal` | get + set |
| `mobs::Polygon.vertices` | get + set |
| `mobs::Polygon.cull_back` | get + set |
| `mobs::ModelNormals.matrix` | no accessor: external type `glam::f32::mat3::Mat3` (crate not loaded) |
| `mobs::ModelNormals.trusted` | get + set |
| `mobs::Part.name` | get + set |
| `mobs::Part.pose` | get + set |
| `mobs::Part.polygons` | get + set |
| `mobs::Part.children` | get + set |
| `mobs::Flags.aquatic_swimmer` | get + set |
| `mobs::Flags.fall_immune` | get + set |
| `mobs::Flags.flying` | get + set |
| `mobs::Flags.climber` | get + set |
| `mobs::Flags.lava_walker` | get + set |
| `mobs::Flags.omnidirectional` | get + set |
| `mobs::Flags.fire_immune` | get + set |
| `mobs::Flags.burns_in_day` | get + set |
| `mobs::Flags.undead` | get + set |
| `mobs::Flags.breathes_water` | get + set |
| `mobs::Flags.needs_water` | get + set |
| `mobs::Flags.water_sensitive` | get + set |
| `mobs::Flags.animal` | get + set |
| `mobs::MobType.id` | get + set |
| `mobs::MobType.species` | get + set |
| `mobs::MobType.width` | get + set |
| `mobs::MobType.height` | get + set |
| `mobs::MobType.eye_height` | get + set |
| `mobs::MobType.category` | get + set |
| `mobs::MobType.max_health` | get + set |
| `mobs::MobType.speed` | get + set |
| `mobs::MobType.flying_speed` | get + set |
| `mobs::MobType.attack_damage` | get + set |
| `mobs::MobType.attack_knockback` | get + set |
| `mobs::MobType.follow_range` | get + set |
| `mobs::MobType.step_height` | get + set |
| `mobs::MobType.armor` | get + set |
| `mobs::MobType.armor_toughness` | get + set |
| `mobs::MobType.knockback_resistance` | get + set |
| `mobs::MobType.jump_strength` | get + set |
| `mobs::MobType.attributes` | get + set |
| `mobs::MobType.gravity` | get + set |
| `mobs::MobType.safe_fall_distance` | get + set |
| `mobs::MobType.fall_damage_multiplier` | get + set |
| `mobs::MobType.water_efficiency` | get + set |
| `mobs::MobType.water_slowdown` | get + set |
| `mobs::MobType.air_speed` | get + set |
| `mobs::MobType.baby_scale` | get + set |
| `mobs::MobType.head_rot_speed` | get + set |
| `mobs::MobType.max_head_x_rot` | get + set |
| `mobs::MobType.max_head_y_rot` | get + set |
| `mobs::MobType.ambient_interval` | get + set |
| `mobs::MobType.flags` | get + set |
| `mobs::MobType.model` | get + set |
| `mobs::MobType.layers` | get + set |
| `mobs::MobType.texture` | get + set |
| `mobs::MobType.brain` | get + set |
| `mobs::MobType.anim` | get + set |
| `mobs::MobType.sounds` | no accessor: a reference nested inside another type needs a handle |
| `mobs::MobType.step_sound` | no accessor: a reference nested inside another type needs a handle |
| `mobs::MobType.food` | get + set |
| `mobs::MobType.tempt` | get + set |
| `mobs::MobType.custom` | no accessor: external type `horizon_mods::custom_mob::CustomMobFile` (crate not loaded) |
| `mobs::MobTypes.types` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Aim.eye` | get + set |
| `mobs::Aim.look` | get + set |
| `mobs::Aim.feet` | get + set |
| `mobs::Damage.kind` | get + set |
| `mobs::Damage.attacker` | get + set |
| `mobs::Damage.from` | get + set |
| `mobs::Attrs.speed` | get + set |
| `mobs::Attrs.attack_damage` | get + set |
| `mobs::Attrs.attack_knockback` | get + set |
| `mobs::Attrs.max_health` | get + set |
| `mobs::Attrs.follow_range` | get + set |
| `mobs::Attrs.knockback_resistance` | get + set |
| `mobs::Attrs.spawn_reinforcements` | get + set |
| `mobs::Attrs.jump_strength` | get + set |
| `mobs::Extra.from_bucket` | get + set |
| `mobs::Extra.armor_attributes` | no accessor: field `catalog` of `EntityAttributes` is private to another module |
| `mobs::Extra.death_loot_table` | get + set |
| `mobs::Extra.death_loot_seed` | get + set |
| `mobs::Extra.more_carrot_ticks` | get + set |
| `mobs::Extra.trusting` | get + set |
| `mobs::Extra.immovable` | get + set |
| `mobs::Extra.description` | get + set |
| `mobs::Extra.model_scale` | get + set |
| `mobs::Extra.crouching` | get + set |
| `mobs::Extra.avatar_pose` | get + set |
| `mobs::Extra.sprinting` | get + set |
| `mobs::Extra.passenger` | get + set |
| `mobs::Extra.age` | get + set |
| `mobs::Extra.age_locked` | get + set |
| `mobs::Extra.age_lock_particle_timer` | get + set |
| `mobs::Extra.forced_age` | get + set |
| `mobs::Extra.forced_age_timer` | get + set |
| `mobs::Extra.in_love` | get + set |
| `mobs::Extra.love_player` | get + set |
| `mobs::Extra.variant` | get + set |
| `mobs::Extra.variant2` | get + set |
| `mobs::Extra.variant3` | get + set |
| `mobs::Extra.stew_effects` | get + set |
| `mobs::Extra.color` | get + set |
| `mobs::Extra.trusted` | get + set |
| `mobs::Extra.love_cause` | get + set |
| `mobs::Extra.sheared` | get + set |
| `mobs::Extra.pick_up_loot` | get + set |
| `mobs::Extra.saddled` | get + set |
| `mobs::Extra.body_damage` | get + set |
| `mobs::Extra.body_extra` | no accessor: field `0` of `Wire` is private to another module |
| `mobs::Extra.relax_one` | get + set |
| `mobs::Extra.on_sleeper` | get + set |
| `mobs::Extra.saddle_stack` | get + set |
| `mobs::Extra.chested` | get + set |
| `mobs::Extra.items` | get + set |
| `mobs::Extra.inventory` | get + set |
| `mobs::Extra.tame` | get + set |
| `mobs::Extra.sitting` | get + set |
| `mobs::Extra.party_jukebox` | get + set |
| `mobs::Extra.owner` | get + set |
| `mobs::Extra.size` | get + set |
| `mobs::Extra.anim_ticks` | get + set |
| `mobs::Extra.egg_time` | get + set |
| `mobs::Extra.anger` | get + set |
| `mobs::Extra.anger_end` | get + set |
| `mobs::Extra.angry_at` | get + set |
| `mobs::Extra.player_created` | get + set |
| `mobs::Extra.carried` | get + set |
| `mobs::Extra.charged` | get + set |
| `mobs::Extra.main_hand` | get + set |
| `mobs::Extra.main_hand_stack` | get + set |
| `mobs::Extra.off_hand` | get + set |
| `mobs::Extra.head` | get + set |
| `mobs::Extra.body` | get + set |
| `mobs::Extra.chest` | get + set |
| `mobs::Extra.legs` | get + set |
| `mobs::Extra.feet` | get + set |
| `mobs::Extra.armor` | get + set |
| `mobs::Extra.using_item` | get + set |
| `mobs::Extra.using_off_hand` | get + set |
| `mobs::Extra.stabbed` | get + set |
| `mobs::Extra.blocking` | get + set |
| `mobs::Extra.fall_flying` | get + set |
| `mobs::Extra.fall_fly_ticks` | get + set |
| `mobs::Extra.previous_velocity` | get + set |
| `mobs::Extra.synced_pose` | get + set |
| `mobs::Extra.auto_spin_attack` | get + set |
| `mobs::Extra.arrow_count` | get + set |
| `mobs::Extra.stinger_count` | get + set |
| `mobs::Extra.shoulder_parrots` | get + set |
| `mobs::Extra.main_hand_left` | get + set |
| `mobs::Extra.hidden_model_parts` | get + set |
| `mobs::Extra.spectator` | get + set |
| `mobs::Extra.elytra` | get + set |
| `mobs::Extra.swim` | get + set |
| `mobs::Extra.avatar` | no accessor: external type `horizon_client_render::camera::AvatarBob` (crate not loaded) |
| `mobs::Extra.swing_right_arm` | get + set |
| `mobs::Extra.swing_stab` | get + set |
| `mobs::Extra.flag` | get + set |
| `mobs::Extra.flag2` | get + set |
| `mobs::Extra.bred` | get + set |
| `mobs::Extra.stand_sliding` | get + set |
| `mobs::Extra.ints` | get + set |
| `mobs::Extra.counter` | get + set |
| `mobs::Extra.counter2` | get + set |
| `mobs::Extra.home` | get + set |
| `mobs::Extra.opened_chest` | get + set |
| `mobs::Extra.point` | get + set |
| `mobs::Extra.repellent` | get + set |
| `mobs::Extra.vector` | get + set |
| `mobs::Extra.effects` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Extra.hidden_effects` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Extra.ambient_effects` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Extra.effect_particles` | no accessor: external type `horizon_client_render::particles::ParticleOptions` (crate not loaded) |
| `mobs::Extra.effect_ambient` | get + set |
| `mobs::Extra.illusions` | get + set |
| `mobs::Extra.bed` | get + set |
| `mobs::Extra.meeting` | get + set |
| `mobs::Extra.trade` | get + set |
| `mobs::Extra.leash` | get + set |
| `mobs::Extra.leash_pending` | get + set |
| `mobs::Extra.leash_player` | get + set |
| `mobs::Extra.leash_spin` | get + set |
| `mobs::Extra.scratch` | get + set |
| `mobs::Extra.positions` | get + set |
| `mobs::Extra.warden` | no accessor: field `suspects` of `AngerManagement` is private to another module |
| `mobs::Extra.allay` | no accessor: external type `horizon_client_render::chunk::block_entity::Data` (crate not loaded) |
| `mobs::Extra.raider` | get + set |
| `mobs::Extra.villager` | no accessor: field `map` of `Gossips` is private to another module |
| `mobs::Extra.animal_brain` | no accessor: generic type `Slot` requires a listed instantiation |
| `mobs::Extra.axolotl` | no accessor: field `old` of `Animation` is private to another module |
| `mobs::Extra.mount` | get + set |
| `mobs::Extra.rider` | get + set |
| `mobs::Extra.boarding_cooldown` | get + set |
| `mobs::Extra.jockey` | get + set |
| `mobs::Extra.trap` | get + set |
| `mobs::Extra.trap_time` | get + set |
| `mobs::Extra.gear` | get + set |
| `mobs::Extra.drop_chances` | get + set |
| `mobs::Extra.left_handed` | get + set |
| `mobs::Extra.sound_variant` | get + set |
| `mobs::Extra.can_break_doors` | get + set |
| `mobs::Extra.screaming` | get + set |
| `mobs::Extra.lost_horns` | get + set |
| `mobs::Extra.goat` | no accessor: field `lower_head_tick` of `State` is private to another module |
| `mobs::Extra.attributes` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Extra.memories` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Extra.pose_change_tick` | get + set |
| `mobs::Extra.camel_sitting` | get + set |
| `mobs::Extra.pose` | get + set |
| `mobs::Extra.leader` | get + set |
| `mobs::Extra.lone` | get + set |
| `mobs::Extra.followers` | get + set |
| `mobs::Extra.section` | get + set |
| `mobs::Extra.section_seq` | get + set |
| `mobs::Extra.brain` | no accessor: generic type `super::Slot` requires a listed instantiation |
| `mobs::Extra.bee` | get + set |
| `mobs::Extra.armadillo` | no accessor: field `danger` of `State` is private to another module |
| `mobs::Extra.frog` | no accessor: field `croak` of `State` is private to another module |
| `mobs::Extra.panda` | no accessor: field `got_bamboo` of `State` is private to another module |
| `mobs::Extra.piglin` | no accessor: field `0` of `Ttl` is private to another module |
| `mobs::Extra.equipment_heard` | get + set |
| `mobs::Extra.custom` | no accessor: field `list` of `AttrSet` is private to another module |
| `mobs::Mob.join` | no accessor: field `0` of `JoinSeq` is private to another module |
| `mobs::Mob.id` | get + set |
| `mobs::Mob.kind` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Mob.position` | get + set |
| `mobs::Mob.previous` | get + set |
| `mobs::Mob.velocity` | get + set |
| `mobs::Mob.yaw` | get + set |
| `mobs::Mob.previous_yaw` | get + set |
| `mobs::Mob.body_yaw` | get + set |
| `mobs::Mob.previous_body_yaw` | get + set |
| `mobs::Mob.head_yaw` | get + set |
| `mobs::Mob.previous_head_yaw` | get + set |
| `mobs::Mob.head_pitch` | get + set |
| `mobs::Mob.previous_head_pitch` | get + set |
| `mobs::Mob.on_ground` | get + set |
| `mobs::Mob.horizontal_collision` | get + set |
| `mobs::Mob.vertical_collision_below` | get + set |
| `mobs::Mob.needs_sync` | get + set |
| `mobs::Mob.gear_knockback_resistance` | get + set |
| `mobs::Mob.health` | get + set |
| `mobs::Mob.hurt_time` | get + set |
| `mobs::Mob.invulnerable` | get + set |
| `mobs::Mob.death_time` | get + set |
| `mobs::Mob.age` | get + set |
| `mobs::Mob.limb_position` | get + set |
| `mobs::Mob.limb_speed` | get + set |
| `mobs::Mob.previous_limb_speed` | get + set |
| `mobs::Mob.swell` | get + set |
| `mobs::Mob.previous_swell` | get + set |
| `mobs::Mob.max_swell` | get + set |
| `mobs::Mob.ambient_timer` | get + set |
| `mobs::Mob.fall_distance` | get + set |
| `mobs::Mob.stuck_multiplier` | get + set |
| `mobs::Mob.piston` | get + set |
| `mobs::Mob.persistent` | get + set |
| `mobs::Mob.custom_name` | get + set |
| `mobs::Mob.custom_name_text` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `mobs::Mob.name_visible` | get + set |
| `mobs::Mob.invisible` | get + set |
| `mobs::Mob.random` | no accessor: field `state` of `LegacyRandom` is private to another module |
| `mobs::Mob.ai` | no accessor: field `goals` of `GoalSelector` is private to another module |
| `mobs::Mob.native_control` | get + set |
| `mobs::Mob.extra` | no accessor: field `catalog` of `EntityAttributes` is private to another module |
| `mobs::Mob.fluids` | get + set |
| `mobs::Mob.was_eye_in_water` | get + set |
| `mobs::Mob.first_tick` | get + set |
| `mobs::Mob.no_gravity` | get + set |
| `mobs::Mob.supporting` | get + set |
| `mobs::Mob.standing_on` | get + set |
| `mobs::Mob.no_ai` | get + set |
| `mobs::Mob.silent` | get + set |
| `mobs::Mob.invulnerable_flag` | get + set |
| `mobs::Mob.native_damage_immune` | get + set |
| `mobs::Mob.attacker_aim` | get + set |
| `mobs::Mob.projectile_motion` | get + set |
| `mobs::Mob.climbing` | get + set |
| `mobs::Mob.fire_ticks` | get + set |
| `mobs::Mob.ticks_frozen` | get + set |
| `mobs::Mob.glowing` | get + set |
| `mobs::Mob.air` | get + set |
| `mobs::Mob.last_hurt` | get + set |
| `mobs::Mob.absorption` | get + set |
| `mobs::Mob.last_damage` | get + set |
| `mobs::Mob.native_last_player_attacker` | get + set |
| `mobs::Mob.combat` | no accessor: field `last_damage_time` of `Tracker` is private to another module |
| `mobs::Mob.removed` | get + set |
| `mobs::Mob.anim` | get + set |
| `mobs::Mob.attrs` | get + set |
| `mobs::Mob.rider` | get + set |
| `mobs::Mob.outbox` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Mob.skin` | get + set |
| `mobs::Mob.slim` | get + set |
| `mobs::Mob.cape` | get + set |
| `mobs::Mob.profile` | no accessor: external type `horizon_protocol::entity_data::Profile` (crate not loaded) |
| `mobs::Mob.random_textures` | no accessor: field `seen` of `Choice` is private to another module |
| `mobs::SpawnSetup.baby` | get + set |
| `mobs::SpawnSetup.size` | get + set |
| `mobs::SpawnSetup.yaw` | get + set |
| `mobs::SpawnSetup.velocity` | get + set |
| `mobs::SpawnSetup.health` | get + set |
| `mobs::SpawnSetup.persistent` | get + set |
| `mobs::SpawnSetup.variant` | get + set |
| `mobs::SpawnSetup.nausea` | get + set |
| `mobs::SpawnSetup.owner_mob` | get + set |
| `mobs::SpawnSetup.home` | get + set |
| `mobs::SpawnSetup.life` | get + set |
| `mobs::Seen.in_water` | get + set |
| `mobs::Seen.playing_dead` | get + set |
| `mobs::Seen.last_damage` | get + set |
| `mobs::Seen.id` | get + set |
| `mobs::Seen.species` | get + set |
| `mobs::Seen.custom` | get + set |
| `mobs::Seen.position` | get + set |
| `mobs::Seen.bounds` | no accessor: field `min` of `Aabb` is private to another module |
| `mobs::Seen.eye_y` | get + set |
| `mobs::Seen.alive` | get + set |
| `mobs::Seen.baby` | get + set |
| `mobs::Seen.in_love` | get + set |
| `mobs::Seen.tame` | get + set |
| `mobs::Seen.bred` | get + set |
| `mobs::Seen.owner` | get + set |
| `mobs::Seen.hostile` | get + set |
| `mobs::Seen.target` | get + set |
| `mobs::Seen.health` | get + set |
| `mobs::Seen.variant` | get + set |
| `mobs::Seen.size` | get + set |
| `mobs::Seen.color` | get + set |
| `mobs::Seen.job` | get + set |
| `mobs::Seen.bed` | get + set |
| `mobs::Seen.yaw` | get + set |
| `mobs::Seen.velocity` | get + set |
| `mobs::Seen.known_speed` | get + set |
| `mobs::Seen.leash` | get + set |
| `mobs::Seen.caravan` | get + set |
| `mobs::Seen.school` | get + set |
| `mobs::Seen.section` | get + set |
| `mobs::Seen.delay` | get + set |
| `mobs::Seen.hurt_by` | get + set |
| `mobs::Seen.active_raid` | get + set |
| `mobs::Seen.opened_chest` | get + set |
| `mobs::Seen.passenger` | get + set |
| `mobs::Seen.vehicle` | get + set |
| `mobs::Seen.player_rep` | get + set |
| `mobs::Seen.pushable` | get + set |
| `mobs::Seen.cold_strider` | get + set |
| `mobs::Seen.category` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Seen.potential_job` | get + set |
| `mobs::Seen.meeting` | get + set |
| `mobs::Seen.profession` | get + set |
| `mobs::Seen.xp` | get + set |
| `mobs::Seen.villager` | no accessor: field `map` of `Gossips` is private to another module |
| `mobs::Seen.stats` | get + set |
| `mobs::Seen.sitting` | get + set |
| `mobs::Seen.silent` | get + set |
| `mobs::Seen.lying` | get + set |
| `mobs::Seen.ridden` | get + set |
| `mobs::Seen.is_vehicle` | get + set |
| `mobs::Seen.knockback_resistance` | get + set |
| `mobs::Seen.effects` | no accessor: a reference nested inside another type needs a handle |
| `mobs::Seen.hunted_recently` | get + set |
| `mobs::Seen.hurt_recently` | get + set |
| `mobs::PendingPushes.list` | get + set |
| `mobs::PendingPushes.by_mob` | get + set |
| `mobs::PlayerView.position` | get + set |
| `mobs::PlayerView.bounds` | no accessor: field `min` of `Aabb` is private to another module |
| `mobs::PlayerView.eye` | get + set |
| `mobs::PlayerView.alive` | get + set |
| `mobs::PlayerView.creative` | get + set |
| `mobs::PlayerView.sneaking` | get + set |
| `mobs::PlayerView.swimming` | get + set |
| `mobs::PlayerView.look` | get + set |
| `mobs::PlayerView.main_hand` | get + set |
| `mobs::PlayerView.off_hand` | get + set |
| `mobs::PlayerView.head` | get + set |
| `mobs::PlayerView.armor_cover` | get + set |
| `mobs::PlayerView.invisible` | get + set |
| `mobs::PlayerView.spectator` | get + set |
| `mobs::PlayerView.gold_armor` | get + set |
| `mobs::PlayerView.hurt_by` | get + set |
| `mobs::PlayerView.last_hurt` | get + set |
| `mobs::PlayerView.velocity` | get + set |
| `mobs::PlayerView.yaw` | get + set |
| `mobs::PlayerView.sprinting` | get + set |
| `mobs::PlayerView.riding` | get + set |
| `mobs::PlayerView.hero` | get + set |
| `mobs::PlayerView.effects` | no accessor: a reference nested inside another type needs a handle |
| `mobs::PlayerView.uuid` | get + set |
| `mobs::PlayerView.allies` | get + set |
| `mobs::PlayerView.remote` | get + set |
| `mobs::PlayerView.sleeping` | get + set |
| `mobs::PlayerView.sleep_timer` | get + set |
| `mobs::PlayerView.passenger` | get + set |
| `mobs::PlayerView.vehicle` | no accessor: field `min` of `Aabb` is private to another module |
| `mobs::Vehicle.nautilus` | get + set |
| `mobs::Vehicle.bounds` | no accessor: field `min` of `Aabb` is private to another module |
| `mobs::Vehicle.velocity` | get + set |
| `mobs::MobWorld.world` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.registry` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.facts` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.fluid_tables` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.movement` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.collision` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.data` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.player` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.nearest_player` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.others` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.dragon` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.leash_boats` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.others_grid` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.others_index` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.daylight` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.monsters_burn` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.game_time` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.day_time` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.raining` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.thundering` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.nether` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.block_tags` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.landing` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.biomes` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.mob_griefing` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.universal_anger` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.mob_drops` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.max_entity_cramming` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.difficulty` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.end` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.world_seed` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.sea_level` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.teams` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.items` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.lid_openers` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.mod_effects` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::MobWorld.border` | no accessor: `MobWorld` carries a lifetime (cannot be a handle type) |
| `mobs::Teams.player` | no accessor: `Teams` carries a lifetime (cannot be a handle type) |
| `mobs::Teams.mobs` | no accessor: `Teams` carries a lifetime (cannot be a handle type) |
| `mobs::EntityModelDocument.0` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `mobs::ElytraAnimation.old` | get + set |
| `mobs::ElytraAnimation.angles` | get + set |
| `mobs::SwimAnimation.old` | get + set |
| `mobs::SwimAnimation.amount` | get + set |
| `mobs::SwimAnimation.visually` | get + set |
| `mobs::SwimAnimation.in_water` | get + set |
