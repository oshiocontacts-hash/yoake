# `motion`: every function (horizon-client/src/motion.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `motion::<LocationEffects as Clone>::clone` | 152 | &self | derived (not counted) | - |
| `motion::<LocationEffects as Debug>::fmt` | 152 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `motion::<LocationEffects as Default>::default` | 152 | - | derived (not counted) | - |
| `motion::<MotionEvent as Clone>::clone` | 169 | &self | derived (not counted) | a reference nested inside another type needs a handle [return value] |
| `motion::<MotionEvent as Debug>::fmt` | 169 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `motion::<MotionWorld as AutoJumpWorld>::collision_boxes` | 2267 | &mut self | excluded | field `min` of `Aabb` is private to another module; generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as AutoJumpWorld>::collision_shape` | 2264 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::block_position` | 2236 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::block_state` | 2245 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::in_block_state` | 2239 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::is_fall_flying` | 2242 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::is_spectator` | 2233 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::set_last_climbable_position` | 2251 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EdgeBackoffWorld>::max_up_step` | 2412 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EdgeBackoffWorld>::no_collision` | 2421 | &mut self | excluded | field `min` of `Aabb` is private to another module; generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EdgeBackoffWorld>::on_ground` | 2418 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EdgeBackoffWorld>::staying_on_ground_surface` | 2415 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::apply_movement_emission` | 2559 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::back_off_edge` | 2485 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::block_speed_factor` | 2568 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::block_state` | 2522 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::can_simulate` | 2540 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::check_fall_damage` | 2528 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::client_side` | 2546 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::collide` | 2488 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::collision_flags` | 2461 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::fall_distance` | 2491 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::fall_reset_clip` | 2494 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::legacy_on_position` | 2519 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::limit_piston_movement` | 2470 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::locally_authoritative` | 2503 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::minor_horizontal_collision` | 2516 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::movement_emission` | 2549 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::movement_history` | 2500 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle; returns `&mut`: needs a child handle that locks its parent |
| `motion::<MotionWorld as EntityMoveWorld>::no_physics` | 2449 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::on_ground` | 2525 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::passenger` | 2552 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::position` | 2452 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::record_movement` | 2458 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::removed` | 2537 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::reset_fall_distance` | 2497 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::restitute` | 2543 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_collision_flags` | 2464 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_on_ground_with_movement` | 2506 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_position` | 2455 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_stuck_multiplier` | 2476 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_velocity` | 2482 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::stuck_multiplier` | 2473 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::velocity` | 2479 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::air_drag_modifier` | 3346 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::block_friction` | 3274 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::can_walk_powder_snow` | 3322 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::discard_friction` | 3343 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::effective_gravity` | 3334 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::flying_speed` | 3283 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::friction_modifier` | 3277 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::has_chunk_at` | 3331 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::horizontal_collision` | 3313 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::in_scaffolding` | 3301 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::is_client_side` | 3328 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::is_player_class` | 3307 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::jumping` | 3316 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::levitation_amplifier` | 3325 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::minimum_y` | 3340 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::move_self` | 3310 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::movement_support_position` | 3268 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::omnidirectional_air_mover` | 3349 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::on_climbable` | 3295 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::on_ground` | 3271 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::reset_fall_distance` | 3298 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::set_velocity` | 3292 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::speed` | 3280 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::suppressing_ladder_slide` | 3304 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::velocity` | 3289 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::was_in_powder_snow` | 3319 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::y` | 3337 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::yaw` | 3286 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::air_drag_modifier` | 3563 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::block_jump_factor` | 3527 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::controlling_passenger_is_player` | 3554 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::gravity_attribute` | 3548 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::has_jump_boost` | 3530 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::has_slow_falling` | 3551 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::jump_boost_amplifier` | 3533 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::jump_strength` | 3524 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::mark_needs_sync` | 3542 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::no_gravity` | 3545 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::omnidirectional_air_mover` | 3560 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::set_velocity` | 3521 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::speed` | 3557 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::sprinting` | 3536 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::velocity` | 3518 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::yaw` | 3539 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementBlockProvider>::movement_block_state` | 2399 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle; std type `core::io::error::Result` has no codec yet [return value] |
| `motion::<MotionWorld as MovementEmissionWorld>::block_state` | 2873 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::climbable_tag` | 2879 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::crouching` | 2888 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::distances` | 2858 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::entity_event` | 2917 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::flapping` | 2920 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::in_water` | 2897 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::is_air` | 2876 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::is_powder_snow` | 2882 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::movement_emission` | 2928 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::next_step` | 2909 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::on_flap` | 2923 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::on_ground` | 2885 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::on_position` | 2870 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::on_rails` | 2891 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::set_flight_distance` | 2864 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::set_movement_distance` | 2861 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::set_next_step` | 2867 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::step_event` | 2906 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::swimming` | 2894 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::walking_step_sound` | 2903 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::water_swim_sound` | 2914 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::award` | 2326 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::crouching` | 2320 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::exhaust` | 2329 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::eyes_in_water` | 2305 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::fall_flying` | 2323 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::in_water` | 2308 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::on_climbable` | 2311 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::on_ground` | 2314 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::passenger` | 2299 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::sprinting` | 2317 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::swimming` | 2302 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::air_drag` | 3156 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::block_bounce_restitution` | 3153 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::block_suppresses_bounce` | 3150 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::bounce_event` | 3170 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::bounce_on` | 3162 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::effective_gravity` | 3147 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::entity_bounciness` | 3141 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::mark_position_for_sync` | 3173 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::movement_support_position` | 3159 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::set_velocity` | 3176 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::suppressing_bounce` | 3138 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::velocity` | 3144 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<PlayerMotion as Clone>::clone` | 276 | &self | derived (not counted) | field `records` of `MovementHistory` is private to another module [return value] |
| `motion::<PlayerMotion as State>::fields` | 275 | - | handle-required | external type `yoake_state::Field` (crate not loaded) |
| `motion::<PlayerMotion as State>::set` | 275 | &mut self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `motion::<PlayerMotion as State>::snapshot` | 275 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [return value] |
| `motion::<PlayerMotion as State>::type_name` | 275 | - | bound (b, mechanical) | - |
| `motion::<PlayerMotion as State>::validate_set` | 275 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `motion::MotionResources::load` | 81 | - | excluded | a reference nested inside another type needs a handle [&argument]; generic type alias `Result`; std type `std::path::Path` has no codec yet [&argument] |
| `motion::MotionResources::load_with_facts` | 92 | - | excluded | a reference nested inside another type needs a handle [&argument]; generic type alias `Result`; std type `std::path::Path` has no codec yet [&argument] |
| `motion::MotionWorld::__yoake_trait_auto_jump_world_collision_boxes` | 2280 | &mut self | excluded | field `min` of `Aabb` is private to another module; generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_auto_jump_world_collision_shape` | 2272 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_climbing_world_set_last_climbable_position` | 2256 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_edge_backoff_world_max_up_step` | 2426 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_edge_backoff_world_no_collision` | 2441 | &mut self | excluded | field `min` of `Aabb` is private to another module; generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_edge_backoff_world_on_ground` | 2436 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_edge_backoff_world_staying_on_ground_surface` | 2431 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_apply_movement_emission` | 2827 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_back_off_edge` | 2636 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_block_speed_factor` | 2838 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_block_state` | 2742 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_can_simulate` | 2792 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_check_fall_damage` | 2752 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_client_side` | 2812 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_collide` | 2651 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_collision_flags` | 2597 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_fall_distance` | 2668 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_fall_reset_clip` | 2673 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_legacy_on_position` | 2737 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_limit_piston_movement` | 2607 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_locally_authoritative` | 2698 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_minor_horizontal_collision` | 2728 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_movement_emission` | 2817 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_no_physics` | 2573 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_on_ground` | 2747 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_passenger` | 2822 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_position` | 2578 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_record_movement` | 2589 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_removed` | 2787 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_reset_fall_distance` | 2692 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_restitute` | 2797 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_set_collision_flags` | 2602 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_set_on_ground_with_movement` | 2703 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_set_position` | 2583 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_set_stuck_multiplier` | 2620 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_set_velocity` | 2630 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_stuck_multiplier` | 2615 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_entity_move_world_velocity` | 2625 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_air_drag_modifier` | 3505 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_block_friction` | 3364 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_can_walk_powder_snow` | 3465 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_discard_friction` | 3500 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_effective_gravity` | 3485 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_flying_speed` | 3384 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_friction_modifier` | 3374 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_has_chunk_at` | 3480 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_horizontal_collision` | 3450 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_in_scaffolding` | 3424 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_is_client_side` | 3475 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_is_player_class` | 3440 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_jumping` | 3455 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_levitation_amplifier` | 3470 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_minimum_y` | 3495 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_move_self` | 3445 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_movement_support_position` | 3354 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_omnidirectional_air_mover` | 3510 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_on_climbable` | 3413 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_on_ground` | 3359 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_reset_fall_distance` | 3418 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_set_velocity` | 3407 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_speed` | 3379 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_suppressing_ladder_slide` | 3435 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_velocity` | 3402 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_was_in_powder_snow` | 3460 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_y` | 3490 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_air_world_yaw` | 3397 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_air_drag_modifier` | 3654 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_block_jump_factor` | 3584 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_controlling_passenger_is_player` | 3639 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_gravity_attribute` | 3629 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_has_jump_boost` | 3596 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_has_slow_falling` | 3634 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_jump_boost_amplifier` | 3601 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_jump_strength` | 3579 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_mark_needs_sync` | 3619 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_no_gravity` | 3624 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_omnidirectional_air_mover` | 3649 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_set_velocity` | 3573 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_speed` | 3644 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_sprinting` | 3609 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_velocity` | 3568 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_living_motion_world_yaw` | 3614 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_block_provider_movement_block_state` | 2404 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle; std type `core::io::error::Result` has no codec yet [return value] |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_block_state` | 2958 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_climbable_tag` | 2977 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_crouching` | 3004 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_distances` | 2933 | &self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_flapping` | 3115 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_in_water` | 3019 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_is_air` | 2963 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_is_powder_snow` | 2988 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_movement_emission` | 3125 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_next_step` | 3096 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_on_flap` | 3120 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_on_ground` | 2999 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_on_position` | 2953 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_on_rails` | 3009 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_set_flight_distance` | 2943 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_set_movement_distance` | 2938 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_set_next_step` | 2948 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_swimming` | 3014 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_walking_step_sound` | 3024 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_emission_world_water_swim_sound` | 3101 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_award` | 2379 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_crouching` | 2369 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_exhaust` | 2390 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_eyes_in_water` | 2344 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_fall_flying` | 2374 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_in_water` | 2349 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_on_climbable` | 2354 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_on_ground` | 2359 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_passenger` | 2334 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_sprinting` | 2364 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_movement_statistics_world_swimming` | 2339 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_air_drag` | 3216 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_block_bounce_restitution` | 3206 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_block_suppresses_bounce` | 3201 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_bounce_event` | 3249 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_bounce_on` | 3226 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_effective_gravity` | 3196 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_entity_bounciness` | 3186 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_mark_position_for_sync` | 3254 | &mut self | excluded | hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_movement_support_position` | 3221 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_set_velocity` | 3259 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_suppressing_bounce` | 3181 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::__yoake_trait_restitution_world_velocity` | 3191 | &mut self | excluded | generic type alias `Result`; hook machinery (`__yoake_*`): not an engine function; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::apply_effects_from_blocks` | 1530 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::block` | 1069 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::check_auto_spin_attack` | 1496 | &mut self | excluded | field `min` of `Aabb` is private to another module; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::check_inside_blocks` | 1712 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::check_movement_statistics` | 1346 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::climbable` | 1098 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::collisions` | 1038 | &self | excluded | lifetime-carrying type `SceneCollisions`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::context` | 1026 | &self | excluded | lifetime-carrying type `CollisionContext`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::controlled_simulation_tick` | 1281 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::controlled_simulation_tick_with_sprint_window` | 1286 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::dry_tick` | 1350 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::effects_from_blocks` | 1558 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::freeze_damage` | 1509 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::honey_block_inside` | 1894 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::jump_from_ground` | 2074 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::lava_cauldron_inside` | 1954 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::layered_cauldron_inside` | 1964 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::local_tick` | 1314 | &mut self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built; generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::location` | 1087 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::make_stuck_in_block` | 1943 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::move_self` | 2186 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::movement_collisions` | 1045 | &self | excluded | lifetime-carrying type `crate::collisions::EntityCollisions`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::movement_tick` | 1115 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::no_player_collision` | 1056 | &mut self | excluded | field `min` of `Aabb` is private to another module; generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::on_soul_speed_block` | 1413 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::player_movement_tick` | 1122 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::player_movement_tick_with_fluids` | 1127 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::player_travel` | 1424 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::powder_snow_inside` | 2026 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::powder_snow_snowflake` | 2046 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::replayed_tick` | 1299 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::server_block_damage` | 1539 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::server_travel` | 1470 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::shulker_box_push` | 2180 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::simulation_tick` | 1261 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::simulation_tick_with_fluids` | 1267 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::support` | 1073 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::sweet_berry_bush_damage` | 1993 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::sweet_berry_bush_inside` | 1987 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::travel_fall_flying` | 2091 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::travel_in_air` | 1106 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::update_auto_jump` | 2195 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::update_soul_speed_block` | 1407 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::web_inside` | 1975 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::PlayerMotion::apply_saved_attributes` | 851 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::attribute` | 744 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::bounds` | 740 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `motion::PlayerMotion::can_freeze` | 696 | &self | bound (a) | - |
| `motion::PlayerMotion::carried_attributes` | 806 | &self | bound (b, mechanical) | - |
| `motion::PlayerMotion::carry_client_movement` | 484 | &mut self | bound (b, mechanical) | - |
| `motion::PlayerMotion::drop_tiny_velocity` | 672 | &mut self | bound (a) | - |
| `motion::PlayerMotion::has_sprint_modifier` | 933 | &self | bound (a) | - |
| `motion::PlayerMotion::ignite_for_ticks` | 538 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::is_fully_frozen` | 691 | &self | bound (a) | - |
| `motion::PlayerMotion::is_sweep_attack` | 491 | &self | bound (a) | - |
| `motion::PlayerMotion::load_saved_state_once` | 503 | &mut self | excluded | field `0` of `Wire` is private to another module [&argument]; generic type alias `Result` |
| `motion::PlayerMotion::new` | 546 | - | excluded | field `attributes` of `Catalog` is private to another module [&argument]; generic type alias `Result` |
| `motion::PlayerMotion::packed_attributes` | 810 | &self | bound (b, mechanical) | - |
| `motion::PlayerMotion::percent_frozen` | 685 | &self | bound (a) | - |
| `motion::PlayerMotion::powder_snow_subject` | 701 | &self | bound (b, mechanical) | - |
| `motion::PlayerMotion::remove_frost` | 989 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::saved_attributes` | 796 | &self | bound (b, mechanical) | - |
| `motion::PlayerMotion::set_modifier` | 757 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::set_remaining_fire_ticks` | 530 | &mut self | bound (a) | - |
| `motion::PlayerMotion::set_sprint_modifier` | 905 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::set_sprinting` | 897 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::sprint_input_tick` | 950 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::sprint_input_tick_with_window` | 955 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::try_add_frost` | 712 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::with_attribute_identities` | 564 | - | excluded | `dyn Trait` values cannot cross the data ABI; field `attributes` of `Catalog` is private to another module [&argument]; generic type alias `Result` |
| `motion::PlayerMotion::yoake_state_air` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_fall_distance` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_fall_fly_ticks` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_fall_flying` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_fire_ticks` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_flying` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_flying_speed` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_glider` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_head_yaw` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_invulnerable` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_is_passenger` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_jump_trigger_time` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_jumping` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_may_fly` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_mobility_restricted` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_no_jump_delay` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_pitch` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_respiration` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_scale` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_shift_key_down` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_sleeping` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_spectator` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_speed` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_sprinting` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_swimming` | 275 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_yaw` | 275 | &self | bound (a) | - |
| `motion::attribute_error` | 475 | - | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; std type `core::io::error::Error` has no codec yet [return value] |
| `motion::fly_into_wall_damage` | 3681 | - | bound (a) | - |
| `motion::horizontal_collision_minor` | 3662 | - | bound (b, mechanical) | - |
| `motion::native_living_jump` | 47 | - | bound (a) | - |

## Fields

| field | status |
|---|---|
| `motion::MotionResources.attributes` | no accessor: field `attributes` of `Catalog` is private to another module |
| `motion::MotionResources.blocks` | no accessor: field `states` of `BoundMovementProperties` is private to another module |
| `motion::MotionResources.climbable` | get + set |
| `motion::MotionResources.soul_speed_blocks` | get + set |
| `motion::MotionResources.climbing` | no accessor: lifetime-carrying type `horizon_world::ClimbingRegistry` |
| `motion::MotionResources.fall_reset` | get + set |
| `motion::MotionResources.inside_step_sound` | get + set |
| `motion::MotionResources.combination_step_sound` | get + set |
| `motion::MotionResources.crystal_sound` | get + set |
| `motion::MotionResources.landing` | no accessor: field `states` of `BlockLandingRegistry` is private to another module |
| `motion::MotionResources.health` | no accessor: field `definitions` of `Catalog` is private to another module |
| `motion::MotionResources.fluids` | no accessor: field `states` of `FluidResources` is private to another module |
| `motion::LocationEffects.on_soul_speed_block` | get + set |
| `motion::LocationEffects.block` | get + set |
| `motion::LocationEffects.landed` | get + set |
| `motion::LocationEffects.boots_level` | get + set |
| `motion::LocationEffects.applied_level` | get + set |
| `motion::PlayerMotion.swimming` | get + set |
| `motion::PlayerMotion.is_passenger` | get + set |
| `motion::PlayerMotion.fire_ticks` | get + set |
| `motion::PlayerMotion.air` | get + set |
| `motion::PlayerMotion.respiration` | get + set |
| `motion::PlayerMotion.position` | get + set |
| `motion::PlayerMotion.velocity` | get + set |
| `motion::PlayerMotion.ground` | get + set |
| `motion::PlayerMotion.collision` | get + set |
| `motion::PlayerMotion.history` | no accessor: field `records` of `MovementHistory` is private to another module |
| `motion::PlayerMotion.fall_distance` | get + set |
| `motion::PlayerMotion.block_landing` | get + set |
| `motion::PlayerMotion.stuck_multiplier` | get + set |
| `motion::PlayerMotion.piston` | get + set |
| `motion::PlayerMotion.yaw` | get + set |
| `motion::PlayerMotion.head_yaw` | get + set |
| `motion::PlayerMotion.jumping` | get + set |
| `motion::PlayerMotion.no_jump_delay` | get + set |
| `motion::PlayerMotion.jump_trigger_time` | get + set |
| `motion::PlayerMotion.may_fly` | get + set |
| `motion::PlayerMotion.mobility_restricted` | get + set |
| `motion::PlayerMotion.invulnerable` | get + set |
| `motion::PlayerMotion.spectator` | get + set |
| `motion::PlayerMotion.flying` | get + set |
| `motion::PlayerMotion.flying_speed` | get + set |
| `motion::PlayerMotion.was_jumping` | get + set |
| `motion::PlayerMotion.auto_jump_enabled` | get + set |
| `motion::PlayerMotion.auto_jump_time` | get + set |
| `motion::PlayerMotion.was_auto_jump` | get + set |
| `motion::PlayerMotion.move_vector` | get + set |
| `motion::PlayerMotion.applied_input` | get + set |
| `motion::PlayerMotion.glider` | get + set |
| `motion::PlayerMotion.pitch` | get + set |
| `motion::PlayerMotion.fall_flying` | get + set |
| `motion::PlayerMotion.sleeping` | get + set |
| `motion::PlayerMotion.fall_fly_ticks` | get + set |
| `motion::PlayerMotion.glide_ending` | get + set |
| `motion::PlayerMotion.server_owns_glide` | get + set |
| `motion::PlayerMotion.glider_wear` | get + set |
| `motion::PlayerMotion.location_effects` | get + set |
| `motion::PlayerMotion.auto_spin_attack_ticks` | get + set |
| `motion::PlayerMotion.auto_spin_attack_dmg` | get + set |
| `motion::PlayerMotion.spin_targets` | no accessor: field `min` of `Aabb` is private to another module |
| `motion::PlayerMotion.spin_hit` | get + set |
| `motion::PlayerMotion.sprinting` | get + set |
| `motion::PlayerMotion.sprint_input` | get + set |
| `motion::PlayerMotion.shift_key_down` | get + set |
| `motion::PlayerMotion.local_crouching` | get + set |
| `motion::PlayerMotion.pose` | get + set |
| `motion::PlayerMotion.dimensions` | get + set |
| `motion::PlayerMotion.scale` | get + set |
| `motion::PlayerMotion.speed` | get + set |
| `motion::PlayerMotion.item_use_speed` | get + set |
| `motion::PlayerMotion.item_use_slows_sprint` | get + set |
| `motion::PlayerMotion.nudge` | get + set |
| `motion::PlayerMotion.last_climbable` | get + set |
| `motion::PlayerMotion.steps` | get + set |
| `motion::PlayerMotion.crystal_sound_intensity` | get + set |
| `motion::PlayerMotion.last_crystal_sound_play_tick` | get + set |
| `motion::PlayerMotion.needs_sync` | get + set |
| `motion::PlayerMotion.sync_position` | get + set |
| `motion::PlayerMotion.teleported` | get + set |
| `motion::PlayerMotion.events` | no accessor: a reference nested inside another type needs a handle |
| `motion::PlayerMotion.attributes` | no accessor: generic type `horizon_world::live::LiveAttributes` requires a listed instantiation |
| `motion::PlayerMotion.impulse` | get + set |
| `motion::PlayerMotion.spawn_extra_particles_on_fall` | get + set |
| `motion::PlayerMotion.health` | no accessor: generic type `crate::synced_values::SyncedValues` requires a listed instantiation |
| `motion::PlayerMotion.status` | get + set |
| `motion::PlayerMotion.fluids` | no accessor: field `trackers` of `EntityFluidInteraction` is private to another module |
| `motion::PlayerMotion.in_powder_snow` | get + set |
| `motion::PlayerMotion.was_in_powder_snow` | get + set |
| `motion::PlayerMotion.ticks_frozen` | get + set |
| `motion::PlayerMotion.leather_boots` | get + set |
| `motion::PlayerMotion.freeze_immune` | get + set |
| `motion::PlayerMotion.burn_from_stepping_immune` | get + set |
| `motion::PlayerMotion.old_position` | get + set |
| `motion::PlayerMotion.server_copy` | get + set |
| `motion::PlayerMotion.replaying` | get + set |
| `motion::PlayerMotion.wall_hit_events` | no accessor: std type `core::ops::range::Range` has no codec yet |
| `motion::PlayerMotion.client_side` | get + set |
| `motion::PlayerMotion.entity_id` | get + set |
| `motion::PlayerMotion.client_loaded` | get + set |
| `motion::PlayerMotion.block_effects` | get + set |
| `motion::PlayerMotion.powder_snow_entered` | get + set |
| `motion::PlayerMotion.hurt_log` | get + set |
| `motion::PlayerMotion.attack_log` | get + set |
| `motion::PlayerMotion.border` | no accessor: field `center` of `BorderGeometry` is private to another module |
| `motion::MotionWorld.entities` | no accessor: `MotionWorld` carries a lifetime (cannot be a handle type) |
| `motion::MotionWorld.player` | no accessor: `MotionWorld` carries a lifetime (cannot be a handle type) |
| `motion::MotionWorld.scene` | no accessor: `MotionWorld` carries a lifetime (cannot be a handle type) |
| `motion::MotionWorld.assets` | no accessor: `MotionWorld` carries a lifetime (cannot be a handle type) |
| `motion::MotionWorld.collision_binding` | no accessor: `MotionWorld` carries a lifetime (cannot be a handle type) |
| `motion::MotionWorld.resources` | no accessor: `MotionWorld` carries a lifetime (cannot be a handle type) |
