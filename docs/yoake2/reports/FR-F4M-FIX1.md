# FR-F4M-FIX1: the three defects of the in-game run of fully custom mobs

Branch `yoake-free/f4m-fix` (= `yoake-free/merge` @40bbb34be merged with `yoake-free/f4m-10` @faa5182a7). No push.
This laptop has no game resources, so nothing here was run in the real game: the honest status of each defect is
stated below, including that for all three **no break was found by tracing the code, and none of the three
reproduces in-process**. What changed is (a) the pieces of each path that can run without resources are now
pinned by tests that mirror the harness's fixture, (b) every step where the real game may lose the blow or the
body now says so in the log (`info`), so the in-game rerun names the broken step at once.

## Step 0

`cargo run -p xtask -- mod-api`, `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib`, then the same
without the variable: 46 passed twice. The regeneration changed only line endings (`git diff --ignore-cr-at-eol`
is empty), discarded; nothing to commit.

## 1. Strike damage

### What was traced
`anim_sync::tick` (first thing in `Mob::tick`) emits the server frame and calls `attacks::deliver_frame`; the
goal (`ChaseStrike::tick`, same tick, after the animation) takes the frame in `Timing::due`, checks `t.alive &&
in_reach` and calls `land` -> `ai::do_hurt_target` -> `MobEvent::AttackPlayer` / `AttackMob`; the game's
`tick_creature_entities` moves the player events of a mob that sees a guest to `lan_guest_mob_events`, hands the
rest to `handle_mob_events` -> `mob_hits_player` / `mob_hits_mob` -> `hurt_player_from` / `Mob::damage_aimed`.

### Result
* The fox of `tools/custom_mob_render_e2e.py` (swim + `melee_attack` priority 2, `interval` 600, `trigger`
  pounce, `damage_frame` hit, no `damage_delay`; `attack_damage` 2; `nearest_target` on chicken and player; the
  real fixture's animations and controllers) strikes through `Mob::tick`: on a player at 1.0 block
  (`AttackPlayer(2.0)`), on a chicken (`AttackMob`), and it picks the nearer of a chicken at 1 block and a
  player at 5. Tests `the_e2e_fox_picks_the_player_and_its_pounce_strikes`,
  `the_e2e_fox_picks_a_chicken_and_its_pounce_strikes`,
  `the_e2e_fox_prefers_the_nearer_of_a_chicken_and_a_player` (`anim_sync_tests.rs`).
  So the goal side produces the event with the harness's configuration, the consumer arms are the ones every
  vanilla melee mob uses, and I found no custom-specific code on the way (the only `kind.custom` tests in the
  game's mob loop are spawn/death/interact hooks).
* Not provable here: `Game` cannot be built without resources (every existing test that builds one needs
  `HORIZON_TEST_RESOURCES`), so the step from the `MobEvent` to the player's health, and the LAN side
  (`player_for` can make the fox see the guest, and then `AttackPlayer` goes to the guest's health, not the
  host's), are not run by any test.
* One observation of the harness that may explain part of the report: after the first blow the goal ends and
  starts again, and `start` resets `until`, so the `interval` of 600 does not hold the second strike back
  (also vanilla `MeleeAttackGoal`'s behaviour); the fox strikes again at once. That does not explain "no
  damage", but "one strike per run" is not what the engine does.

### Change (commit "custom-mob strikes log every step ...")
New `info` lines, all `Horizon custom-mob strike: mob N ...`: `begins on <who> tick T`, `lands on <who>`,
`dropped, target alive A in reach R` (the frame came, the blow was not delivered), `dropped, the goal stopped`
(a pending blow lost to a stop), and, from the game's consumer, `hit the player|mob K for X: hurt H` (whether
`mob_hits_player`/`mob_hits_mob` actually hurt). No vanilla path is touched (the logging is behind
`kind.custom` / a custom mob standing at the event's position).

### What the rerun should show
For a strike that works: `begins` -> (server frame) -> `lands` -> `hit the player for 2: hurt true`, and the
player's health 20 -> 18. Where it stops tells the break: no `begins` (the goal never started: target or reach),
`begins` without `lands` (a `dropped ...` line names why, or nothing: the goal is not ticked), `lands` without `hit`
(the event did not reach `handle_mob_events`: look at the guest binding), `hurt false` (the damage code refuses:
`refuses_damage`, a native `LivingIncomingDamage` hook, the player's hurt cooldown).

## 2. Naming custom mobs

### What was traced
`summon_with_nbt` -> `spawn_entity_from_nbt` -> `load_listed` (`Mob` claims every id in `mob_types`, custom ids
included) -> `load_mob` -> `entity_nbt::mob_from_nbt` (reads `CustomName`, `CustomNameVisible`; the custom
`yoake` compound is read after them) ; `/data merge entity` -> `apply_entity_nbt(Target::Mob)` -> the same
`mob_from_nbt`; the save (`mob_to_nbt`) writes both keys; the name tags (`names.rs`) have no custom-mob
special case but the tag height.

### Result
No custom exclusion exists on any of these paths. Two things in the harness's commands explain what was seen:
* `/data merge entity @e[type=...]` is refused by the parser ("selector yields multiple") for **every** type: an
  `entity` argument that is not a single-result selector fails vanilla's `EntityArgument.entity()`; the harness
  needs `limit=1`.
* `CustomName:'"Foxy"'` is the pre-1.21.5 JSON-string form. This game reads the name as a text component NBT
  (as the target version does): a string tag is literal text, so that form would show the quotes. The form that
  names is `{CustomName:"Foxy"}` (a string) or a component compound.
Not provable here: the part of `/summon` that needs a `Game` (`spawn_loaded_entities`), see 1.

### Change (commit "/summon NBT builds its compound in one function ...")
`summon_tag(kind, pos, nbt)` (the first half of `summon_with_nbt`, unchanged behaviour) so the exact compound the
command builds can be tested. Tests (`game::commands::data::tests`): `summoning_a_custom_mob_with_a_name_names_it`
(summon NBT -> custom mob with `custom_name_text` "Foxy", `command_name`, `name_visible`),
`merging_a_name_into_a_custom_mob_keeps_it_through_a_save` (saved fox + merged `CustomName`/`CustomNameVisible` ->
rebuilt with both -> saved again with both).

### What the rerun should show
`/summon sample_mobs:cinder_fox ~ ~ ~ {CustomName:"Foxy",CustomNameVisible:1b}` then a tag "Foxy" above the fox;
`/data merge entity @e[type=sample_mobs:cinder_fox,limit=1] {CustomName:"Cinder",CustomNameVisible:1b}` renames it.
If either still fails, the chat reply names the error (I could not see the "entity type not found" of the report;
if it is real it comes from `/summon` with NBT before `summon_tag`: check the `commands.summon.failed` key).

## 3. Death timing

### What was traced
The body is dropped by `Mob::tick` returning false (`ai::tick_inner`: `death_time >= creaking::death_limit(m)`,
which is `custom::removal_ticks` for a custom type) and announced (poof, entity event 60) by
`tick_creature_entities`' `death_time >= removal_ticks(m)`: both read the same value, `death_ticks()` = the
`death` clip's length clamped to 20..=60 ticks (20 for the fixture's 0.6 s). `death_time` starts at 0 and
`base_tick` adds 1 each tick while dead (`/kill` sets 1); the remote client's `death_time += 1`
(`remote.rs`) belongs to a guest's own copy, which is removed only by the server's `RemoveEntities`. The agent
`entities` list (what the harness samples) keeps dead bodies until `removed`.

### Result
The code removes at exactly `removal_ticks` and not before; `a_mob_killed_by_damage_is_removed_exactly_at_removal_ticks_and_announced`
drives it the way `/damage` does (a hit on a live mob, then ticks through `Mob::tick` with the game loop's
removal rule) for clips of 0.6 s, 1.5 s, 5 s and none: 20, 30, 60, 20 ticks; a drop that the game loop would not
announce (a silent removal) fails the test. I found no second place that removes a dead mob; the
despawn check (`custom::spawn::check_despawn`) can drop a mob at any time, dead or not, and is the only other
silent drop of a mob in its own tick (no sign of it in the run: persistent_by_default false, distance 6).
The "4 to 20 ticks" were measured by sampling every 2 ticks plus command latency; it is not shown that a body
went before tick 20.

### Change (commit "custom-mob deaths log ...")
`info` lines `Horizon custom-mob death: mob N dies, death_time T (limit L) tick G` (when the death is first
processed) and `... removed at death_time T (limit L) tick G` (when the body is dropped), so the rerun can read the
interval in game ticks directly instead of sampling. The new test above.

### What the rerun should show
`dies` and `removed` 20 ticks apart (tick numbers in the lines) for the fixture fox; `removed at death_time 20
(limit 20)`. A line `removed` with `death_time` below the limit would be impossible; a body that disappears
without a `removed` line was dropped by something else (despawn, a script `despawn`, the `kill`).

## Tests
`horizon-mod-api-gen --lib` 46; `horizon-client-lib --lib` and `horizon-client --lib`, `wasm_mods::mob_runtime`,
`horizon-mods`: see the summary of the commit message / hand-over (all green at the final commit). New tests: 4 in
`anim_sync_tests.rs` (3 strike, 1 death), 2 in `game::commands::data`.

## Open questions
1. Is the strike reaching the guest? In the harness the fox may see the guest (`player_for`); the harness reads
   the host's health only. The new log lines show the target and the victim.
2. Should `interval` hold after a goal restart (it does not, as in vanilla)? The e2e's "one strike" assumption
   needs `interval` to be a cooldown that survives `start`.
3. The harness's two syntax mistakes (`@e[...]` without `limit=1`, the JSON-string `CustomName`) should be fixed
   in `tools/custom_mob_render_e2e.py` and the death check should read the new log lines.
4. No test runs the `Game` path (see 1 and 2): a resource-free `Game` fixture would be the way to close it.
