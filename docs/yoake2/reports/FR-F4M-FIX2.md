# F4M-FIX2: custom-mob target loss and interrupted strikes

## Cause

The Cinder Fox does not declare `follow_range`. `custom::build::mob_type` correctly put the default 16 in `MobType.follow_range`, but its `MobType.attributes` map contained only built-ins explicitly present in the JSON.

Summon finalization exposed the mismatch. `Game::spawn_mob_reason` calls `spawning::finalize` (`crates/horizon-client/src/game/creatures.rs:441-449`). Vanilla's normal `mob_spawn` adds a random follow-range modifier (`crates/horizon-client/src/mobs/finalize.rs:663-673`). Adding it refreshes the per-mob value through `base_value`, which reads the attributes map and otherwise falls back to zero (`crates/horizon-client/src/mobs/finalize.rs:498-509,595-603`). The fox consequently had zero follow range after finalization.

That produces both reported symptoms:

- Player acquisition allows a minimum 2-block distance (`crates/horizon-client/src/mobs/goals.rs:771-775`), so the data-goal fox one block away could acquire and begin a blow. `target_continue` then measured against the actual zero follow range and stopped the selector, which stopped the attack before `hit`.
- The real-WASM fox, with players 3.2 and 4 blocks away, was outside both acquisition's 2-block minimum and mob search range, so `script::view` never had a target.

The game path itself was confirmed: it builds a shared `MobWorld`, selects the host or guest per mob, and calls `Mob::tick` at `crates/horizon-client/src/game/creatures.rs:1053-1128`. An absent player, stale creative state, line of sight, or the two same-priority selectors is not needed to explain the evidence.

## Fix

- `crates/horizon-client/src/mobs/custom/build.rs`: insert the default `minecraft:follow_range` into the custom type's attribute map. Finalization now applies the random bonus to 16 rather than zero. An explicitly configured value still wins. Vanilla types and their random draws are untouched.
- `crates/horizon-client/src/mobs/goals.rs`: custom-only `info` diagnostics report target picks, goal/target stops and their reasons, and the precise `target_continue` failure (including distance and unseen count). The selector's evaluation order and short-circuiting are unchanged.
- `crates/horizon-client/src/mobs/custom/ai/attacks.rs`: a started chase strike records its `Who`, player UUID, timing and strike kind. If the goal stops, the pending blow moves to `CustomState.attack` instead of being cancelled. A restart cannot begin another blow while one is saved.
- `crates/horizon-client/src/mobs.rs`: `Mob::tick` resolves a saved blow after animation events and before the AI tick. At impact it rechecks the recorded target's identity, life, enemy flag, creative/spectator state, spared status and reach. Failure and mob death drop the blow with a strike log.
- `crates/horizon-client/src/game/lan_mobs.rs`: the LAN player binding stays on the saved blow's player until resolution. The UUID check is a second guard against ever redirecting the blow to another player.
- `tools/custom_mob_render_e2e.py`: `STRIKE_LINE` now also collects the new target and goal diagnostics. No other harness logic changed.

## Tests written

No build or test was run in this worktree; commands are listed in `.lane/TASK.md`.

- `custom/build.rs`: spawn finalization preserves at least the default 16-block follow range.
- `the_finalized_fixture_fox_keeps_its_target_and_lands_the_server_frame`: the real fixture, its actual player+chicken target list, spawn finalization, a player, `Mob::tick`, `ChaseStrike`, and the real `hit` server frame produce exactly one player hit.
- `a_stopped_goals_blow_lands_on_its_recorded_target`: forcibly stopping the selector mid-wind-up and clearing `m.ai.target` does not cancel the blow.
- `a_saved_blow_still_drops_for_an_invalid_target_or_a_dead_mob`: creative, dead, out-of-reach and different-player targets all miss; a dead attacker also drops its saved blow.
- `a_goal_restart_cannot_begin_a_second_blow_while_one_is_saved`: the attack does not restart while its first blow is pending and the first blow lands once.
- `the_fixture_targets_a_player_beyond_two_blocks_and_script_view_publishes_it`: the finalized real target list acquires a player four blocks away and the published `script::view` exposes the player and position.
- `custom/ai/test_support.rs`: `run_server` drives the same `Mob::tick` entry point with a cloned `MobWorld` and advancing game time, without prepared game resources.

## Still needs the real game

Run both `custom_mob_render_e2e.py` variants listed in `.lane/TASK.md`.

- Data goal: the fox keeps its target, `hit` lands once, and exactly one survival player loses 2 health. The chicken strike must land too.
- Real WASM guest: the gate changes from `(None, false)` to a player target with a position, the pounce starts, and one player takes the expected damage.
- Inspect the collected target/goal lines for the precise stop reasons and confirm there is no unchanged per-tick logging.
- Run `pr_ready.py --quick` for formatting, lint and the touched crate suites. The real host/guest run remains the authority for LAN routing and animation/event timing.
