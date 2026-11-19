# `motion`: every function (horizon-client/src/motion.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `motion::<LocationEffects as Clone>::clone` | 149 | &self | derived (not counted) | - |
| `motion::<LocationEffects as Debug>::fmt` | 149 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `motion::<LocationEffects as Default>::default` | 149 | - | derived (not counted) | - |
| `motion::<MotionEvent as Clone>::clone` | 166 | &self | derived (not counted) | a reference nested inside another type needs a handle [return value] |
| `motion::<MotionEvent as Debug>::fmt` | 166 | &self | derived (not counted) | lifetime-carrying type `$crate::fmt::Formatter`; std type `core::fmt::Result` has no codec yet [return value] |
| `motion::<MotionWorld as AutoJumpWorld>::collision_boxes` | 2194 | &mut self | excluded | field `min` of `Aabb` is private to another module; generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as AutoJumpWorld>::collision_shape` | 2188 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::block_position` | 2167 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::block_state` | 2176 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::in_block_state` | 2170 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::is_fall_flying` | 2173 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::is_spectator` | 2164 | &mut self | excluded | associated-type projection requires a listed instantiation; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as ClimbingWorld>::set_last_climbable_position` | 2182 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EdgeBackoffWorld>::max_up_step` | 2260 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EdgeBackoffWorld>::no_collision` | 2269 | &mut self | excluded | field `min` of `Aabb` is private to another module; generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EdgeBackoffWorld>::on_ground` | 2266 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EdgeBackoffWorld>::staying_on_ground_surface` | 2263 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::apply_movement_emission` | 2483 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::back_off_edge` | 2319 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::block_speed_factor` | 2492 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::block_state` | 2412 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::can_simulate` | 2454 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::check_fall_damage` | 2418 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::client_side` | 2470 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::collide` | 2332 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::collision_flags` | 2291 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::fall_distance` | 2347 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::fall_reset_clip` | 2350 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::legacy_on_position` | 2409 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::limit_piston_movement` | 2300 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::locally_authoritative` | 2374 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::minor_horizontal_collision` | 2402 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::movement_emission` | 2473 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::movement_history` | 2371 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle; returns `&mut`: needs a child handle that locks its parent |
| `motion::<MotionWorld as EntityMoveWorld>::no_physics` | 2275 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::on_ground` | 2415 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::passenger` | 2476 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::position` | 2278 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::record_movement` | 2285 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::removed` | 2451 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::reset_fall_distance` | 2367 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::restitute` | 2457 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_collision_flags` | 2294 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_on_ground_with_movement` | 2377 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_position` | 2281 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_stuck_multiplier` | 2309 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::set_velocity` | 2315 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::stuck_multiplier` | 2306 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as EntityMoveWorld>::velocity` | 2312 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::air_drag_modifier` | 2850 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::block_friction` | 2757 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::can_walk_powder_snow` | 2826 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::discard_friction` | 2847 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::effective_gravity` | 2838 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::flying_speed` | 2771 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::friction_modifier` | 2765 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::has_chunk_at` | 2835 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::horizontal_collision` | 2817 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::in_scaffolding` | 2799 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::is_client_side` | 2832 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::is_player_class` | 2811 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::jumping` | 2820 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::levitation_amplifier` | 2829 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::minimum_y` | 2844 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::move_self` | 2814 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::movement_support_position` | 2751 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::omnidirectional_air_mover` | 2853 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::on_climbable` | 2792 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::on_ground` | 2754 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::reset_fall_distance` | 2795 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::set_velocity` | 2788 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::speed` | 2768 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::suppressing_ladder_slide` | 2808 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::velocity` | 2785 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::was_in_powder_snow` | 2823 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::y` | 2841 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingAirWorld>::yaw` | 2782 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::air_drag_modifier` | 2915 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::block_jump_factor` | 2869 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::controlling_passenger_is_player` | 2906 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::gravity_attribute` | 2900 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::has_jump_boost` | 2879 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::has_slow_falling` | 2903 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::jump_boost_amplifier` | 2882 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::jump_strength` | 2866 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::mark_needs_sync` | 2894 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::no_gravity` | 2897 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::omnidirectional_air_mover` | 2912 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::set_velocity` | 2862 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::speed` | 2909 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::sprinting` | 2888 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::velocity` | 2859 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as LivingMotionWorld>::yaw` | 2891 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementBlockProvider>::movement_block_state` | 2254 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle; std type `core::io::error::Result` has no codec yet [return value] |
| `motion::<MotionWorld as MovementEmissionWorld>::block_state` | 2525 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::climbable_tag` | 2540 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::crouching` | 2561 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::distances` | 2510 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::entity_event` | 2666 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::flapping` | 2669 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::in_water` | 2570 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::is_air` | 2528 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::is_powder_snow` | 2549 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::movement_emission` | 2677 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::next_step` | 2649 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::on_flap` | 2672 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::on_ground` | 2558 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::on_position` | 2522 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::on_rails` | 2564 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::set_flight_distance` | 2516 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::set_movement_distance` | 2513 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::set_next_step` | 2519 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::step_event` | 2646 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::swimming` | 2567 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::walking_step_sound` | 2576 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementEmissionWorld>::water_swim_sound` | 2654 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::award` | 2238 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::crouching` | 2232 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::exhaust` | 2247 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::eyes_in_water` | 2217 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::fall_flying` | 2235 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::in_water` | 2220 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::on_climbable` | 2223 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::on_ground` | 2226 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::passenger` | 2211 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::sprinting` | 2229 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as MovementStatisticsWorld>::swimming` | 2214 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::air_drag` | 2711 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::block_bounce_restitution` | 2703 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::block_suppresses_bounce` | 2700 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::bounce_event` | 2738 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::bounce_on` | 2717 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::effective_gravity` | 2697 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::entity_bounciness` | 2691 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::mark_position_for_sync` | 2741 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::movement_support_position` | 2714 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::set_velocity` | 2744 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::suppressing_bounce` | 2688 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<MotionWorld as RestitutionWorld>::velocity` | 2694 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::<PlayerMotion as Clone>::clone` | 273 | &self | derived (not counted) | field `records` of `MovementHistory` is private to another module [return value] |
| `motion::<PlayerMotion as State>::fields` | 272 | - | handle-required | external type `yoake_state::Field` (crate not loaded) |
| `motion::<PlayerMotion as State>::set` | 272 | &mut self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `motion::<PlayerMotion as State>::snapshot` | 272 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [return value] |
| `motion::<PlayerMotion as State>::type_name` | 272 | - | bound (b, mechanical) | - |
| `motion::<PlayerMotion as State>::validate_set` | 272 | &self | handle-required | external type `yoake_state::Value` (crate not loaded) [&argument] |
| `motion::MotionResources::load` | 80 | - | excluded | external type `horizon_client_render::assets::Assets` (crate not loaded) [&argument]; generic type alias `Result`; std type `std::path::Path` has no codec yet [&argument] |
| `motion::MotionResources::load_with_facts` | 90 | - | excluded | external type `horizon_client_render::assets::Assets` (crate not loaded) [&argument]; generic type alias `Result`; std type `std::path::Path` has no codec yet [&argument] |
| `motion::MotionWorld::apply_effects_from_blocks` | 1479 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::block` | 1039 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::check_auto_spin_attack` | 1447 | &mut self | excluded | field `min` of `Aabb` is private to another module; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::check_inside_blocks` | 1658 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::check_movement_statistics` | 1303 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::climbable` | 1065 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::collisions` | 1009 | &self | excluded | lifetime-carrying type `SceneCollisions`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::context` | 998 | &self | excluded | lifetime-carrying type `CollisionContext`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::controlled_simulation_tick` | 1241 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::controlled_simulation_tick_with_sprint_window` | 1245 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::dry_tick` | 1306 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::effects_from_blocks` | 1505 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::freeze_damage` | 1459 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::honey_block_inside` | 1839 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::jump_from_ground` | 2010 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::lava_cauldron_inside` | 1897 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::layered_cauldron_inside` | 1906 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::local_tick` | 1271 | &mut self | excluded | `impl Trait` requires a listed instantiation; closure argument (`impl Fn*`): guest callback handles are not built; generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::location` | 1055 | &self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::make_stuck_in_block` | 1887 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::move_self` | 2119 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::movement_collisions` | 1016 | &self | excluded | lifetime-carrying type `crate::collisions::EntityCollisions`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::movement_tick` | 1080 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::no_player_collision` | 1027 | &mut self | excluded | field `min` of `Aabb` is private to another module; generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::on_soul_speed_block` | 1367 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::player_movement_tick` | 1086 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::player_movement_tick_with_fluids` | 1090 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::player_travel` | 1377 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::powder_snow_inside` | 1964 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::powder_snow_snowflake` | 1983 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::replayed_tick` | 1257 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::server_block_damage` | 1487 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::server_travel` | 1422 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::shulker_box_push` | 2114 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::simulation_tick` | 1223 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::simulation_tick_with_fluids` | 1228 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::support` | 1042 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::sweet_berry_bush_damage` | 1932 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::sweet_berry_bush_inside` | 1927 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::travel_fall_flying` | 2026 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::travel_in_air` | 1072 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::update_auto_jump` | 2127 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::update_soul_speed_block` | 1362 | &mut self | excluded | generic type alias `Result`; receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::MotionWorld::web_inside` | 1916 | &mut self | excluded | receiver type carries a lifetime: it cannot be a hook-argument handle |
| `motion::PlayerMotion::apply_saved_attributes` | 829 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::attribute` | 727 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::bounds` | 724 | &self | handle-required | field `min` of `Aabb` is private to another module [return value] |
| `motion::PlayerMotion::can_freeze` | 683 | &self | bound (a) | - |
| `motion::PlayerMotion::carried_attributes` | 786 | &self | bound (b, mechanical) | - |
| `motion::PlayerMotion::carry_client_movement` | 481 | &mut self | bound (b, mechanical) | - |
| `motion::PlayerMotion::drop_tiny_velocity` | 662 | &mut self | bound (a) | - |
| `motion::PlayerMotion::has_sprint_modifier` | 909 | &self | bound (a) | - |
| `motion::PlayerMotion::ignite_for_ticks` | 531 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::is_fully_frozen` | 679 | &self | bound (a) | - |
| `motion::PlayerMotion::is_sweep_attack` | 487 | &self | bound (a) | - |
| `motion::PlayerMotion::load_saved_state_once` | 498 | &mut self | excluded | field `0` of `Wire` is private to another module [&argument]; generic type alias `Result` |
| `motion::PlayerMotion::new` | 538 | - | excluded | field `attributes` of `Catalog` is private to another module [&argument]; generic type alias `Result` |
| `motion::PlayerMotion::packed_attributes` | 789 | &self | bound (b, mechanical) | - |
| `motion::PlayerMotion::percent_frozen` | 674 | &self | bound (a) | - |
| `motion::PlayerMotion::powder_snow_subject` | 687 | &self | handle-required | external type `horizon_client_render::collisions::PowderSnowSubject` (crate not loaded) [return value] |
| `motion::PlayerMotion::remove_frost` | 962 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::saved_attributes` | 777 | &self | bound (b, mechanical) | - |
| `motion::PlayerMotion::set_modifier` | 739 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::set_remaining_fire_ticks` | 524 | &mut self | bound (a) | - |
| `motion::PlayerMotion::set_sprint_modifier` | 882 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::set_sprinting` | 874 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::sprint_input_tick` | 925 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::sprint_input_tick_with_window` | 929 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::try_add_frost` | 697 | &mut self | excluded | generic type alias `Result` |
| `motion::PlayerMotion::with_attribute_identities` | 555 | - | excluded | `dyn Trait` values cannot cross the data ABI; field `attributes` of `Catalog` is private to another module [&argument]; generic type alias `Result` |
| `motion::PlayerMotion::yoake_state_air` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_fall_distance` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_fall_fly_ticks` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_fall_flying` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_fire_ticks` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_flying` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_flying_speed` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_glider` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_head_yaw` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_invulnerable` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_is_passenger` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_jump_trigger_time` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_jumping` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_may_fly` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_mobility_restricted` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_no_jump_delay` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_pitch` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_respiration` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_scale` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_shift_key_down` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_sleeping` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_spectator` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_speed` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_sprinting` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_swimming` | 272 | &self | bound (a) | - |
| `motion::PlayerMotion::yoake_state_yaw` | 272 | &self | bound (a) | - |
| `motion::attribute_error` | 472 | - | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; std type `core::io::error::Error` has no codec yet [return value] |
| `motion::fly_into_wall_damage` | 2939 | - | bound (a) | - |
| `motion::horizontal_collision_minor` | 2921 | - | bound (b, mechanical) | - |
| `motion::native_living_jump` | 48 | - | bound (a) | - |

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
| `motion::PlayerMotion.status` | no accessor: external type `horizon_client_render::effects::StatusFlags` (crate not loaded) |
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
