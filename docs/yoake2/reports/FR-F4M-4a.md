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
| `climb` (`cost`) | implemented (follow-up 1) | the engine's spider climbing (`Flags.climber`) and `path.rs` | `a_climb_row_gives_the_engines_climber_physics_and_a_path_cost`, `the_navigator_routes_up_a_wall_only_for_a_mob_with_a_climb_row`, `the_climb_cost_decides_between_the_wall_and_the_way_round`, `a_climber_gets_up_the_wall_by_pressing_against_it` |
| `sleep_in_bed_like` (`when`, `state`, `block_tag`, `range`, `speed`, `reach`) | implemented (follow-up 2), new `Rest` goal (`MOVE`, `LOOK`, `JUMP`) | `nearest_block`, `nav_move_to`, the safe expression language | `a_mob_goes_home_at_night_sleeps_and_stays_put`, `a_mob_does_not_rest_by_day_and_wakes_when_day_comes`, `a_hurt_sleeper_wakes_and_does_not_lie_down_again_at_once`, `a_when_expression_replaces_the_night`, `the_rest_place_can_be_the_nearest_block_of_a_tag` |
| `open_door` (`door_tag`, `break`) | implemented (follow-up 4), new `OpenTaggedDoor`, `BreakDoor::tagged` in `doors.rs` | `DoorInteractGoal`'s `Interact` (given an optional door tag) | `open_door_opens_a_door_of_its_tag_on_the_path_and_closes_it_after`, `open_door_with_break_breaks_the_door_after_240_ticks_and_only_when_griefing` |
| `eat_block` (`blocks`, `interval`, `cooldown`, `duration`, `effect`, `trigger`) | implemented (follow-up 4), new `EatCustomBlock` (`MOVE`, `LOOK`, `JUMP`) | `MobEvent::BreakBlock` / `SetBlockName`, `ai::age_up` | `eat_block_replaces_the_block_heals_raises_its_event_and_waits_for_its_cooldown`, `eat_block_changes_nothing_without_griefing_but_still_has_its_effect` |
| `script` | **deferred, F4M-10** | | same |

**Schema decision for deferred goals: the schema keeps accepting them** (the vocabulary is the contract; a content pack written for 4b/4c loads).
The mob does not get the goal. `ai::support(kind)` says why, `ai::unsupported(&CustomMobFile)` lists them, and `build_one` logs one warning per
type and goal (`custom mob x: the goal "sit" is not run by this version (F4M-4c: ...)`). An unknown *type* is still a schema error as before.
The fixture fox therefore does not get `follow_owner`, `sit` and the script goal (test `the_fixture_fox_lists_the_goals_it_does_not_get`).
33 of the 41 types run, 8 are deferred (`every_schema_goal_type_has_a_status` pins both numbers; it was 29 and 12 before the follow-up round below).

## Follow-up round (reviewer decisions)

Each is its own commit.

1. **`climb`** (a goal row; the `climb` *navigator* is unchanged). A type with a `climb` row gets the engine's spider climbing: `Flags.climber` is set in
   `build::mob_type`, so `Mob.climbing`, the climbing velocity clamp and the fall reset are the engine's. `cost` (default 0.5, 0 to 32) is the extra
   path cost of one block of wall: `Navigation.climb_cost` / `PathMob.climb_cost` (a new field, `None` for every vanilla mob) let `Finder::walk_neighbors`
   add vertical edges (`climb_neighbors`): up a face that has a solid block beside the new node, and down it while hanging. While following such a step the
   mob aims past the wall (`Mob::climb_aim`) so it presses against it (that is what makes the engine's climbing move it). Limits: it climbs straight up
   and down a face and steps onto the top; it does not travel sideways along a wall.
2. **`sleep_in_bed_like`** (`ai/rest.rs`, `ai/expr.rs`). At night (`!daylight` in the overworld-like dimensions), or when its `when` expression is true,
   it walks to the rest place (the nearest block of `block_tag` within `range`, else the home of a `stay_near_home`/`patrol_around` row, else where it first
   thought of resting) and stays: it stops, the goal holds `MOVE|LOOK|JUMP`, and the tracked value `state` (default `sleep`, a bool, or an i32 as 1/0)
   is set; a type without such a value logs a warning and still rests. It wakes when hurt (health below what it had lying down, or hurt within 100
   ticks stops it resting again), at day, or when the condition turns false. `when` uses the safe expression language (`horizon-mobmodel`, new
   dependency of `horizon-client-lib`; `Cargo.lock` gained that edge): `query.health`, `max_health`, `is_on_ground`, `is_in_water`, `is_baby`,
   `hurt_time`, `has_target`, `is_riding`, `life_time` (seconds), `time_of_day` (day time over 24000) and `query.synced('name')`; a bad expression leaves
   the goal out with a warning.
3. **`Seen.custom`** (`Option<Arc<str>>`, the custom type id; `None` for vanilla). `Prey::Types` / `Avoid::Types` (vanilla species and custom ids) are used
   only when a list names a custom id; `Prey::Species` and `Avoid::Species` are as before, so vanilla matching is unchanged (the shared helper
   `seen_is_type` gives the same answer for a vanilla `Seen`). In a data row's `types` / `from` any id outside `minecraft:` is a custom type id (kept with a
   warning when no such type is loaded yet, since content may load in any order). **`Seen` is a bindgen-visible struct and got a field** (see Needs checking).
4. **`open_door`** (`door_tag` default `minecraft:wooden_doors`, `break` default false) and **`eat_block`**: `doors.rs`'s private `Interact` got an optional
   door tag (`None` keeps every old call exactly: the wooden-door and `_door` name tests are the same code paths), `BreakDoor::tagged` and the new
   `OpenTaggedDoor`. The goal also lets the mob's navigation open doors. Breaking is 240 ticks, any difficulty, only with mob griefing. `eat_block`
   `blocks` is `{ "<block>": "<replacement>" }` or a list (left as air), the block it stands in or the one below; `interval` (1 in N, default 100), `cooldown`
   (default 600 ticks), `duration` (default 40), `effect` = `{ "heal": 4 }`, `"grow_up"` or `{ "event": "name" }` / `"name"` (the event goes to
   `CustomState.events`, taken with `ai::blocks::take_events(&mut Mob)`), `trigger` (an animation). Blocks change only when mob griefing is on, the effect
   always happens. The sheep's `EatBlock` is untouched.
5. **Shared path budget** (`ai/budget.rs`), see the section below.

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
`a_search_to_an_unreachable_target_gives_up_in_bounded_work`.

**Shared per-tick budget** (follow-up 5, `ai/budget.rs`). All custom mobs together may visit `limit` nodes per game tick, default **8192**
(`DEFAULT_PATH_BUDGET`), the per-search cap of 1024 staying. `Mob::query_path_raw` asks `budget::request(id, game_time, cap)` for a custom mob only
(a vanilla mob returns before it) and charges the nodes the search really visited (`Finder::find_counted`). A request that does not fit gets no path this
tick (`None`; its goal asks again by its own rules) and the mob is first in line next tick; a mob that is not waiting may only search while enough is left
for every waiting mob that has not searched yet, and a mob served two ticks running gives way while anybody waits (round robin). The state is per thread
(the server tick is one thread; tests are isolated). `budget::stats()` reads the counters (`PathStats`: ticks, granted, waited, nodes, busiest tick,
this tick). **Server setting**: `server.properties` key `horizon-custom-mob-path-budget` (Horizon's own, as `horizon-lod-distance`), written by
`horizon-server` from `server.toml`'s `[server] custom_mob_path_budget = 8192` (1 to 1048576; `config.rs` `TYPED`, `settings.rs` range,
`docs/api/server-reference.md`, `docs/hosting.md`); `dedicated.rs` applies it with `budget::set_limit`. An integrated server uses the default.
Tests: `the_budget_defaults_to_8192_nodes_a_tick_and_reads_the_server_setting`, `a_search_that_does_not_fit_waits_for_the_next_tick`,
`mobs_take_turns_when_the_budget_cannot_serve_all_of_them`, `a_waiting_mob_that_never_comes_back_does_not_block_the_others_for_long`,
`the_engines_path_search_of_a_custom_mob_is_metered_and_a_vanilla_mobs_is_not`.

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

After the follow-up round and the two F4M-3 fixes (same commands):

* `-p horizon-client-lib --lib`: **1733 passed**, 0 failed, 48 ignored (+27 over 1706). The `mobs::custom` subset is 90 (with `mod_content`'s three egg tests and one row test run in the same filter: 134).
* `-p horizon-client --lib`: **1203 passed**, 0 failed, 40 ignored.
* `-p horizon-mods` lib: **58 passed** (+2: the Yoake water slowdown and the axolotls category).
* `-p horizon-server`: **117 passed**, 1 ignored (the reference doc test checks the new `custom_mob_path_budget` row).
* Not run, as before: `f4m_parity`, clippy, `regen.sh --check`.

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
* **`Seen` got a field** (`custom: Option<Arc<str>>`): a bindgen-visible struct, so `crates/yoake-bindgen/regen.sh` / `--check` must be run by the coordinator (not run here;
  the generated glue in the working tree was not touched or committed). `PathMob` (a new `climb_cost`), `Navigation`, `CustomState` (`events`), `Prey` and `Avoid`
  (a `Types` variant each) changed too; the last four are not annotated, `PathMob` is a plain engine struct.
* New dependency `horizon-mobmodel` for `horizon-client-lib` (the `when` expression); `Cargo.lock` has the one edge.
* `Finder::find` is now a wrapper of `find_counted` (same search, plus the visited count); vanilla mobs call `find` and never touch the budget.
* `BenchWorld` (`mob_hook_bench`, compiled under `cfg(test)`) got `set_stone`, `set_air`, `set_tag` helpers (all `yoake_hook(skip)`).
* The `open_door` goal and the climb path edges are tested on the flat world with stone as the only block (a "door" is a stone block of a test tag; a real
  door's path types and the `set_door_open` effect are not exercised); `eat_block`'s block events are checked as events, not applied. No in-game run of any of them.
* Server setting: `server.toml` `custom_mob_path_budget` and `horizon-custom-mob-path-budget`; `horizon-server`'s typed list grew to 17 and its reference doc has the row.
  `horizon-server` tests: 117 passed.
* A path request that does not fit returns no path (`None`) rather than a queued one; goals that treat "no path" as a failure (a chase) retry by their own timers.
* No in-game run.

## Open questions

* Should a data row be able to override a goal's control flags (for example a `melee_attack` that also takes `JUMP`)? Not done; the schema has no field for it.
* `damage_delay` as both the plain delay and the timeout of a named frame: fine, or should the timeout be a separate key when server animation arrives?
* (Answered by the reviewer and done in the follow-up round: `climb`, `sleep_in_bed_like`, the type id in `Seen`, generic `open_door` and `eat_block`, the per-tick path budget.)
* `sleep_in_bed_like` remembers its home in the goal, so a mob reloaded from a save rests where it is when it first thinks of resting, unless a `stay_near_home` /
  `patrol_around` row or a `block_tag` gives it a place. Should the first place be saved with the mob?
* The budget is per thread and counts nodes; a debug view that reads `budget::stats()` is not part of this item. Where should it show?

## Fixes from the reviewer's check against real 26.3 resources

Spawn eggs, the `entity-types.json` attribute set, `yoake:water_slowdown` and the `axolotls` category: see `FR-F4M-3.md` ("Fix notes"). They are F4M-3 code, fixed on this branch in
two commits after the follow-up round.
