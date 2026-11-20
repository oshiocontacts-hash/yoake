# FR-F4M-4a: goals, navigators and attacks for fully custom mobs (data route)

Branch `yoake-free/f4m-4a`, from `yoake-free/f4m-3` (`174c502f8`). Not pushed.

## What landed

* `mobs/custom/ai/` (new): `mod.rs` (the catalogue status, `attach`, the goal row reader), `nav_select.rs`, `goals.rs`, `attacks.rs`,
  `targets.rs`, `test_support.rs` and `tests.rs` (test only). `build::attach_goals` now only calls `ai::attach`.
* Reviewer additions, as the first commits: `is_monster` follows the category; reach checks use hitbox parts.
* `horizon-mods/src/custom_mob.rs`: one new goal row, `leap_attack` (`GOAL_TYPES` is 41 long), plus one test. Nothing else of the schema changed
  (parameters of a goal are free keys of its row, as before).
* `mobs.rs`: `mob_hook_bench` (the synthetic resource-free world that the hook bench uses) is now also compiled for `cfg(test)`, and its `kind()`
  got the `custom: None` field it was missing (the `yoake-hook-bench` feature did not compile since F4M-3). That module is what the tests run on.
* `mobs/nav.rs`: one line in `query_path_raw`, the path budget cap (below). `mobs/goals.rs`: `within_melee_range` also tests hitbox parts.

## Goal catalogue (FR-F4-MOBS.md 6.1)

Which array a row is in does not matter: goals go to the goal selector and target selectors to the target selector by their **type**.

| Goal type(s) | Status | Engine goal reused | Test |
|---|---|---|---|
| `float`, `swim`, `float_in_water` | implemented | `Float` | `swim_floats_a_mob_that_is_in_lava_or_deep_water` |
| `wander`, `stroll` | implemented | `Stroll::avoiding_water` (`speed`, `interval`) | `wander_walks_somewhere`, `stroll_is_the_same_goal_under_its_new_name` |
| `random_swim` | implemented | `Stroll::swimming` (`speed`, `interval`) | attach and name only (no water in the test world) |
| `fly_wander` | implemented | `Stroll::flying` | `random_swim_and_fly_wander_...` (it starts on a flying mob) |
| `look_at_player` (`range`), `random_look` | implemented | `LookAtPlayer`, `RandomLookAround` | `look_at_player_and_random_look_start_by_chance`, `look_at_player_respects_its_range` |
| `look_at_target` (`range`) | implemented, new small goal (`LOOK`) | none generic | `look_at_target_looks_while_the_target_is_near` |
| `panic` | implemented | `Panic` | `panic_runs_from_a_recent_hit` |
| `tempt` (`items`, `can_scare`) | implemented | `Tempt` | `tempt_follows_a_player_holding_one_of_its_items` |
| `flee`, `avoid` (`from`, `distance`, `walk_speed`, `sprint_speed`) | implemented | `AvoidEntity` | `avoid_keeps_away_from_the_player` |
| `retreat_when_hurt` (`health_below`, `distance`) | implemented, new (`MOVE`) | `default_random_pos_away`, `create_path_to` | `retreat_when_hurt_waits_for_low_health` |
| `go_to_block` (`blocks`, `range`, `vertical_range`, `interval`) | implemented, new wrapper | `MoveToBlock` | `go_to_block_walks_to_a_listed_block` |
| `stay_near_home` (`radius`), `patrol_around` (`radius`, `interval`) | implemented | `Mob.ai.home` restriction, `default_random_pos_towards`, `Stroll` | `stay_near_home_walks_back_and_patrol_stays_around` |
| `leap_at_target` (`yd`) | implemented | `LeapAtTarget` | `leap_at_target_jumps_at_a_target_in_range` |
| `leap_attack` | implemented, new (`MOVE`, `JUMP`) | `do_hurt_target`, `within_melee_range` | `leap_attack_winds_up_jumps_and_strikes_once`, `leap_attack_strikes_at_its_damage_frame` |
| `melee_attack` | implemented, wraps `MeleeAttack` | `MeleeAttack` (chase) + `do_hurt_target` | `a_plain_melee_attack_is_the_engines_goal_tick_for_tick` and the frame tests |
| `ranged_attack` | implemented | engine projectiles (`Projectile`, `own_random`) | `ranged_attack_shoots_its_projectile_at_the_target`, `a_ranged_attack_shoots_at_its_damage_frame` |
| `area_attack` | implemented, wraps `MeleeAttack` | `MeleeAttack` (chase), the `AttackPlayer`/`AttackMob`/`Knockback` events | `area_attack_hits_what_is_in_its_shape`, `a_cone_area_attack_only_hits_in_front` |
| `target_player` | implemented | `NearestTarget::player` | `target_player_picks_the_player_in_sight` |
| `nearest_target`, `nearest_attackable` | implemented | `NearestTarget` | `nearest_attackable_takes_listed_types_in_range` |
| `hurt_by_target`, `hurt_by` | implemented | `HurtByTarget` | `hurt_by_takes_the_attacker_and_call_for_help_alerts_its_kind` |
| `call_for_help` | implemented (a target selector) | `HurtByTarget::alert` | same |
| `follow_owner`, `sit`, `owner_hurt_target`, `owner_hurt_by`, `owner_target` | **deferred, F4M-4c** | the engine's tame goals are private to `tame.rs` and Wolf-specific | listed in `every_schema_goal_type_has_a_status` |
| `breed`, `follow_parent` | **deferred, F4M-4c** | needs the `breedable` data and offspring creation | same |
| `eat_block` | **deferred** | the engine's `EatBlock` is the sheep's (hard-coded blocks and `Mob.ate`); a data version needs a generic goal | same |
| `open_door` | **deferred** | the door goals in `doors.rs` are private to raiders (`RaiderOpenDoor` needs a raid) | same |
| `sleep_in_bed_like`, `climb` | **deferred, unclear** | the schema row does not say what they do (`climb` may just be the `climb` navigator) | same |
| `script` | **deferred, F4M-10** | | same |

**Schema decision for deferred goals: the schema keeps accepting them** (the vocabulary is the contract; a content pack written for 4b/4c loads).
The mob does not get the goal. `ai::support(kind)` says why, `ai::unsupported(&CustomMobFile)` lists them, and `build_one` logs one warning per
type and goal (`custom mob x: the goal "sit" is not run by this version (F4M-4c: ...)`). An unknown *type* is still a schema error as before.
The fixture fox therefore does not get `follow_owner`, `sit` and the script goal (test `the_fixture_fox_lists_the_goals_it_does_not_get`).
29 of the 41 types run, 12 are deferred (`every_schema_goal_type_has_a_status` pins both numbers).

Parameters of a row not named above: `priority` and `speed` are the schema's own; a missing or non-finite number takes the engine's default.

## Navigator selection (`ai/nav_select.rs`)

| `navigator` | `NavKind` | Move control | Look control | Also |
|---|---|---|---|---|
| `ground` | `Ground` | `Ground` | default | nothing (the engine's default for every mob) |
| `climb` | `WallClimber` | `Ground` | default | |
| `swim` | `Water { breach: false }` | `SmoothSwimming(85, 10, 0.02, 0.1, gravity)` (dolphin) | `SmoothSwimming(10)` | water path malus 0 |
| `fly` | `Flying` | `Flying { max_turn: 10, hovers: false }` (parrot) | default | `can_float`, no door opening |
| `amphibious` | `Amphibious { prefers_shallow: false }` | `SmoothSwimming(85, 10, 0.1, 0.5, no gravity)` (axolotl) | `SmoothSwimming(20)` | water path malus 0 |

The swimming controls multiply `movement_speed` by their own factors, as the vanilla swimmers' do (dolphin 1.2, frog 1.0): a `swim` or `amphibious`
mob wants a `movement_speed` of that order. Tests: `each_navigator_picks_the_engines_navigation_and_controls`,
`a_path_is_found_for_the_ground_and_the_flying_navigator`.

**Path budget.** A search of a custom mob visits at most `nav_select::MAX_VISITED_NODES` = 1024 nodes (the engine's number is `follow_range * 16`, so
a follow range above 64 is capped; 16 gives 256 as in vanilla). The cap is applied in `Mob::query_path_raw` through `nav_select::max_visited_nodes`,
which returns the engine's number untouched for a vanilla mob. Tests: `a_custom_mobs_path_search_is_capped_and_a_vanilla_mobs_is_not`,
`a_search_to_an_unreachable_target_gives_up_in_bounded_work`. There is **no per-tick budget across mobs** (the design's 5 ms server budget is a
property of the whole scheduler; nothing in this item meters it), the existing engine path scheduling is unchanged.

## Attacks and the damage-frame wiring (`ai/attacks.rs`)

Every attack row takes `trigger` (animation trigger name), `damage_frame` (server timeline event name) and `damage_delay` (ticks).

* `trigger`: the goal only **requests** the animation (pushes the name to `CustomState.attack`, an outbox of at most 8). F4M-5b drains it with
  `attacks::take_triggers(&mut Mob) -> Vec<String>` once per tick and plays each name on the mob's synced trigger clock.
* `damage_frame`: the F4M-9 engine part calls `attacks::deliver_frame(&mut Mob, name) -> bool` for every server `Custom` event it gets from the mob's rig
  (the `Custom { name, .. }` events of animations flagged in `model.server_events`). The goal that waits for that name applies its blow at that moment
  (`do_hurt_target` / area damage / the projectile / the leap's strike). Frames are kept (at most 8) until a goal takes them; an attack that begins
  forgets a stale frame of its own name first.
* `damage_delay`: the **fallback**. If the frame has not come this many ticks after the attack began (`game_time`), the blow lands anyway. Without
  `damage_frame` it is a plain delay and defaults to 0 (the blow lands at once, as vanilla's goals do, tick for tick). With `damage_frame` and no delay
  it defaults to `DEFAULT_FRAME_DELAY` = 10 ticks. **Server animations are not running yet, so today the fallback is the whole mechanism**; set
  `damage_delay` to the keyframe time. When the server animation exists, the frame wins if it comes first. An author who sets a delay shorter than the
  keyframe makes the fallback win every time.
* A blow that is pending when its goal stops is dropped. `melee_attack` and `area_attack` stay running while a blow is pending even if the chase
  has arrived (the wrapped `MeleeAttack` ends then), unless the target died. When the blow lands the target must still be alive and in reach, else it is a miss.

Goals:

* `melee_attack` (`speed`, `interval` default 20, `follow_unseen`): wraps the engine's `MeleeAttack` for chasing and looking; its own strike is held off
  (its attack timer is pinned high each tick) and the goal strikes with `do_hurt_target` after `swing()`. With none of `trigger`, `damage_frame`,
  `damage_delay` it equals the engine's goal tick for tick (test compares event counts and the final position against a mob with a plain `MeleeAttack`).
* `area_attack` (`radius` 3, `range` = radius, `damage` = `attack_damage`, `knockback` 0.5, `shape` `sphere`|`cone`, `angle` 90, `types`, `interval` 40): chases like
  melee; when the target is within `range` it begins, and the blow damages the player (default victim), the listed vanilla species and the goal's own
  target, whichever are in the shape (sphere around the mob's centre, or a cone around its facing), with the `AttackPlayer`/`AttackMob` events and a
  `Knockback` event away from the mob.
* `ranged_attack` (`projectile` `arrow`|`snowball`|`egg`|`llama_spit`|`small_fireball`|`large_fireball`|`wind_charge`, `projectile_speed`, `spread`, `arc`,
  `damage` for an arrow's base damage, `interval_min` 20, `interval_max` 60, `radius` 15): `RangedAttackGoal`'s timing and movement (in the engine it takes
  a plain function for the shot, which cannot carry a row's numbers, so the timing is written out here; this is the one place the engine's goal is copied
  rather than wrapped) with the shot a `MobEvent::Projectile` of the existing projectile code, aimed at a third of the target's height with the arc as
  skeletons and snow golems do. An unknown projectile name logs a warning and leaves the goal out. Custom projectile entities are F4M-17.
* `leap_attack` (`windup` ticks, `impulse`, `forward`, `min_range`, `max_range`, `cooldown`, `chance`): faces the target for `windup` ticks (the trigger plays at
  the start), jumps (`LeapAtTargetGoal`'s impulse), and strikes once, at the damage frame or fallback delay when there is one, else the first tick the
  target is in reach in the air. `MOVE | JUMP`.
* `leap_at_target` is the engine's `LeapAtTarget` unchanged (`yd`).

Hit reach (reviewer item): `Mob::within_melee_range` also tests the attacker's pick/hurt parts (inflated like the box), and `Game::within_entity_range` measures a
player's reach to the nearest of a custom mob's boxes (`custom::hitbox::distance_sqr`). A mob without parts, and every vanilla mob, take the old code.
The **target's** parts are not used: the engine's `TargetInfo`/`Seen` carry the main box only. Tests in `hitbox.rs` and
`a_part_that_sticks_out_lets_a_melee_attack_reach_further`. `Game::within_entity_range` itself needs a `Game`; its distance function is tested.

## Targets (`ai/targets.rs`)

* `nearest_attackable` / `nearest_target` `{ types, must_see, range, interval }`: `NearestTarget`. `types` are `minecraft:player` and vanilla mob ids. Both
  together make two selectors at the same priority. No usable `types` is the player. `range` is turned into the engine's follow-range scale at attach time
  (`range / follow_range` of the type), so a later change of the follow range attribute rescales it. A type the engine cannot tell apart (another mod's, or a
  **custom** type: the `Seen` snapshot has a species, not a type id) is left out with a warning. Mob-versus-custom-mob targeting needs a type id in `Seen`.
* `target_player`, `hurt_by_target`/`hurt_by` (`alert`, `ignore`), `call_for_help` (alert on): the engine's selectors.
* `owner_target`: deferred with the owner state (F4M-4c).

## Control flags and conflicts

Each goal reports its own flags; the engine's `GoalSelector` resolves them: `MOVE`, `LOOK`, `JUMP`, `TARGET`. New goals: `look_at_target` `LOOK`; `retreat_when_hurt`,
`go_to_block`, `stay_near_home` `MOVE`; `melee_attack`, `area_attack`, `ranged_attack` `MOVE|LOOK`; `leap_attack` `MOVE|JUMP`. A data row cannot change a goal's flags.
Tests: `a_lower_priority_number_takes_over_a_flag_and_equal_ones_do_not`, `goals_with_disjoint_flags_run_together`, `float_takes_the_jump_flag_from_a_leap`,
`a_target_selector_with_a_lower_number_replaces_a_running_one`.

## Reviewer items

* `is_monster` follows the category: `ai::mob_is_monster(&Mob)` and `drops::mob_is_monster(&Mob)` (a custom mob is one when its category is `monster`, else the species
  tests as before). Switched at the call sites that have the Mob: xp reward, experience and loot rule, swim, splash and fall sounds, no-action time, the bed
  check. The rabbit and panda fear lists are species lists with no Mob and stay as they were. Test `a_custom_mob_is_a_monster_by_its_category`
  (custom monster, custom creature, five vanilla species compared with the species-based function, swim sound, experience and loot of a baby).
* Custom-mob sounds with full namespaced ids are **not** in F4M-4a (F4M-9 engine side / F4M-6); the `&'static str` vanilla path is untouched.

## Tests

The tests run on `mob_hook_bench::BenchWorld`, the synthetic flat world (a stone floor at `y = 0`, no game resources), so every goal has a real
deterministic test here. `with_built_test_world`-style tests (which need `.local/horizon-client`) are not used.

Final tree, all green (through `cargo-locked.sh`, offline, `-j 4`):

* `-p horizon-client-lib --lib`: **1706 passed**, 0 failed, 48 ignored (F4M-3 had 1663: +43 = 1 monster test, 2 reach tests, 40 in `custom/ai/tests.rs`).
  The `mobs::custom` subset is 67 (27 before the AI module).
* `-p horizon-client --lib`: **1203 passed**, 0 failed, 40 ignored (unchanged count; it compiles `game/pick_entity.rs`).
* `-p horizon-mods`: lib **56 passed** (+1: `every_goal_type_is_accepted_with_its_own_parameters`).
* Not run: `f4m_parity` (needs game resources), clippy, `regen.sh --check`.

The 40 AI tests: catalogue and deferral (4), movement and idle goals (14), target selectors (4), control flags (4), navigators and path budget (4), attacks and
damage frames (10). Commit (b) alone does not compile (it declares `ai` whose files are in (c)); the final tree was the one tested.

## Needs checking

* Yoake: the parity goldens (`f4m_parity`, saves, network, render). Nothing runs differently without a custom mob (`MobType.custom` is `None`): the
  `mob_is_monster` call sites give the species answer, `within_melee_range` takes the same path, `query_path_raw` gets the engine's number, and
  `mob_hook_bench` is only compiled in addition for tests.
* `CustomState` got a field (`attack: AttackState`), and `Extra.custom` holds it as `Option<Box<CustomState>>`. Bindgen saw `Extra.custom` already (F4M-3);
  the new field is behind the box and not `#[yoake_hook]`'d, but `crates/yoake-bindgen/regen.sh --check` should be run by the coordinator (not run here).
* `mod_hook_bench` now also compiles under `cfg(test)`: if the bindgen/hook reports list that module for non-test builds nothing changes, but the `custom: None`
  in its `kind()` is a source change of a `yoake_hook`-annotated module.
* The swimming move controls' speed factors are the vanilla swimmers' and were not tried in game; a `swim` or `amphibious` mob may need a different `movement_speed`.
* `Game::within_entity_range` (`game/pick_entity.rs`) compiles in `horizon-client` only; no `Game` test.
* No in-game run.

## Open questions

* `climb` and `sleep_in_bed_like(light)` have no definition in the design doc (`climb` may just be the navigator): what should they do?
* Mob-versus-custom-mob targeting and avoidance need a type id in `Seen` (a bindgen-visible struct). Wanted?
* `open_door`, `eat_block`: the engine's goals are the raiders' and the sheep's; should 4a add generic ones (small edits of `doors.rs` and `goals.rs`)?
* A per-tick path creation budget across all custom mobs (design section 8) needs a scheduler hook in the server tick: F4M-14?
* Should a data row be able to override a goal's control flags (for example a `melee_attack` that also takes `JUMP`)? Not done; the schema has no field for it.
* `damage_delay` as both the plain delay and the timeout of a named frame: fine, or should the timeout be a separate key when server animation arrives?
