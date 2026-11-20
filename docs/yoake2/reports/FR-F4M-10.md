# FR-F4M-10: sandboxed WASM API for fully custom mobs

Branch `yoake-free/f4m-10` (base `yoake-free/f4m-6` @88509ce78, merged with `origin/yoake-free/merge` @05c4d6bae). Design: `FR-F4-MOBS.md` 3.3, 6.2, 7, 11.

## Step 0: death removal 20 to 60 ticks

`TypeAnim::death_ticks` is clamped to `MIN_DEATH_TICKS..=MAX_DEATH_TICKS` = 20..=60 (`custom::removal_ticks`, `creaking::death_limit` follow). A death clip shorter than 20 ticks holds its last pose
(`death_clock` was already held) while XP, drops and the other `death_time` timing run on vanilla's 20 ticks. Test `a_dying_mob_stays_until_its_death_animation_ends_with_a_range_of_20_to_60_ticks`
(0.6 s -> 20, 1.5 s -> 30, 5 s -> 60, through real `Mob::tick`); FR-F4M-6.md updated.

## How it is built

* **WIT is generated** from the catalogs. Imports are rows of the `services` arrays (`abi-subset-server.json`, `abi-subset-client.json`); exports are native hooks, one fragment
  `tools/native-hook-fragments/f4m_mobs.json` with 20 hooks whose placeholder functions live in `game/native_hooks.rs` (`native_mob_goal_can_use` ... `native_synced_changed_client`).
  Regenerated with `cargo run -p xtask -- mod-api` and `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib`; nothing generated was edited by hand.
* **The engine does not call these exports through the `dispatch_*` of `mod_api::mods()`**: a mob is ticked as `&mut Mob` with no `Game` in reach, so the game installs a `mobs::custom::script::Runtime`
  (`wasm_mods/mob_runtime.rs`, `WasmRuntime`) that calls every server module's export directly (`ServerFullMod::dispatch`, so the module's own subscriptions and the sandbox limits apply).
  The native hook functions exist to name each export and its types for the generator (they are `dead_code` and never dispatched). This is the same pattern as `interact::set_hook` (F4M-4c), which
  `WasmRuntime` also implements (`mob-interact`).
* **Services write a command bus** (`script.rs`): a service reads a `MobView` published before the call and queues a `Command`; `script::call` applies the commands of the mob whose hook is running
  as soon as the guest returns (before the engine goes on), and any other mob's at the start of its next tick (`script::pre_tick`, called from `Mob::tick`). Commands that need a world or an event queue
  they have not got are put back for the next tick. All checking of values happens in `script_apply.rs` (bad names and non-finite numbers are ignored with a debug log).
* **Control claim**: `claim-control` sets `ScriptState.claimed`; `sync_claim` puts MOVE|LOOK|JUMP on `GoalSelector.disabled` at the start of each tick (the selector is taken out of the mob while goals run,
  so a flag set from inside a goal would be lost).

## API table

Status: wired = engine + test; see the deferred list for the rest. Ids are service ids (ABI catalog), permission `service.mobs` for all services (manifest `WASM_PERMISSIONS`, gated per package by `mob_services::granted`).

| Kind | WIT function | ABI id | Status |
|---|---|---|---|
| import | `mob-spawn(dimension, options, position, type-id) -> result<u32>` | 260 | wired (`Host::summon`; only from a hook that runs inside a native mod call, else Err) |
| import | `mob-info-call(entity) -> result<option<mob-info>>` (named `-call`: the generator avoids the clash with the `mob-info` record) | 261 | wired (snapshot; includes target position and sight) |
| import | `mob-set-synced`, `mob-get-synced` | 262, 263 | wired |
| import | `mob-trigger-anim(controller, entity, name, speed)` | 264 | wired (controller ignored, triggers are found by name) |
| import | `mob-set-controller-state` | 265 | wired (enters the state with its fade; the controller's own transitions may leave it) |
| import | `mob-navigate-to`, `mob-navigate-stop`, `mob-look-at`, `mob-jump` | 266, 267, 268, 269 | wired |
| import | `mob-set-target`, `mob-attack` | 270, 271 | wired (a mob by handle, or the player: any non-empty UUID means the engine's one player) |
| import | `mob-attribute-get/-set-base/-add-modifier/-remove-modifier` | 272-275 | wired for the type's custom attributes only |
| import | `mob-set-hitbox-enabled` | 276 | wired |
| import | `mob-data-get`, `mob-data-put` | 277, 278 | wired (opaque bytes, at most 64 KiB, saved in `yoake.script`) |
| import | `mob-despawn` | 279 | wired (no drops, at the latest next tick) |
| import | `mob-claim-control` | 280 | wired |
| import | `mob-impulse(entity, velocity)` (extension: the leap) | 281 | wired |
| client import | `mob-bone-index(type-id, name)`, `mob-locator(entity, name)` | 282, 283 | wired (locator: last drawn pose, only for `procedural_script` types) |
| export | `mob-goal-can-use/-can-continue/-start/-tick/-stop` | hooks | wired (ScriptGoal) |
| export | `mob-tick` | hook | wired (`pre_tick`) |
| export | `mob-spawn-check(kind, dimension, pos)` | hook | wired (`placement: custom`, false without a guest) |
| export | `custom-mob-spawned(mob, kind, reason)` (the old `mob-spawned` hook is unchanged) | hook | wired for natural/command spawns (`spawn_mob_for`); egg/breeding use the same function |
| export | `mob-hurt` (amount may change, cancel) | hook | wired (`Mob::damage_aimed`, after the part multiplier) |
| export | `mob-died` | hook | wired (`mob_death_canceled`, cannot cancel) |
| export | `mob-interact` (reply `consume`/`success`) | hook | wired (`interact::dispatch_global` hook) |
| export | `mob-anim-event` (server `Custom` timeline events) | hook | wired |
| export | `mob-load` (once, with the saved script data), `mob-save` | hooks | wired (`persist.rs`) |
| export | `mob-tamed`, `mob-bred` | hooks | wired (drains the 4c notices) |
| export | `synced-changed-server` | hook | wired (every real change of a tracked value, newest 16) |
| client export | `mob-pose(mob, kind, frame, writes) -> writes` | hook | wired (`custom_mob_draw.rs`, only `model.procedural_script`, only when the LOD evaluates the pose; at most 256 writes) |
| client export | `mob-anim-event-client`, `synced-changed-client` | hooks | wired (`GameSink::custom`; the 5b `take_changes` queue is drained, which it was not before) |

New schema field: `model.procedural_script: bool`. `look_at: owner` is filled for the local player's own mobs (`PoseCtx.owner`).

## ids and versions

* Catalog service ids used: **260-283** (server 260-281, client 282-283). Nothing in 201-250 was touched.
* `interface_version`: server catalog **1.2.0 -> 1.3.0**, client catalog **1.6.0 -> 1.7.0** (relative to MERGE5; the branch was built on 1.1.0 / 1.5.0 and merged).
* No hook ledger ids were needed (the exports are catalog hooks, not mixin sites). Fragment hooks get their API numbers from the stamp tool.
* `WASM_PERMISSIONS` gained `service.mobs` (10 entries after the merge).
* New WIT types: `mob-value(-kind)`, `mob-spawn-options`, `mob-info`, `mob-transform` (catalog), `script-vec3`, `mob-script-value`, `mob-bone-write`, `mob-pose-frame`, `mob-*-outcome` (generated).

## Budgets

* At most **32 script goal calls per mob per tick** (`MAX_EVALUATIONS_PER_MOB_TICK`; `can-use`, `can-continue`, `start`, `tick`, and the `mob-*` calls of `pre_tick` count; `stop` does not). A spent budget: `can-use` false, `can-continue` true, `start`/`tick` skipped.
* `interval` (1 to 200): `can-use` is asked only every that many ticks.
* Per call: the sandbox's own limits (server 25 ms wall clock, 10 M fuel, 25 ms per 50 ms window in aggregate); a trap or overrun disables the module (existing behaviour of `ServerFullMod`).
* Commands waiting for mobs that are not ticking: 4096 (oldest dropped, counted); script data 64 KiB; pose writes 256.

## ScriptGoal semantics

`ai/script_goal.rs`: `Goal` with the flags of the `ai.script_goals` row, priority of the goal row, `every_tick`. `can-use`: false without a live runtime, within `interval`, or without budget. `can-continue`: false when
the runtime is not live (a trapped or timed-out module), so the goal stops. `stop` is always forwarded. Several modules: any module answering yes wins `can-use`/`can-continue`; start/tick/stop go to all that subscribed
(a module ignores goal ids it does not know).

## fox_ai

`crates/yoake-wasm-sdk/examples/fox_ai` (about 70 lines, `wasm32-wasip2`, features `trap` and `spin` only for tests): `can-use` when the fox is on the ground, sees its target and the target is 3 to 6 blocks away;
`start` stops navigation, looks at the target and triggers `pounce`; on the server timeline event `hit` (0.35 s) it applies a leap impulse and bites (3 damage, knockback 0.4) when the target is within 4 blocks;
the goal lasts 16 ticks. The fixture `tools/fixtures/cinder_fox` is a Horizon mod folder without a `.yak` or a `[wasm]` section, so it has no place for the module: not wired (see open questions).

## Tests

* `horizon-mod-api-gen --lib`: **46 passed** (WIT/SDK/adapter conformance, `abi_1_0_is_additive`, full-hook-names, coverage).
* `horizon-client-lib --lib mobs::custom`: **195 passed** (was 179 at step 0; new `tests_script.rs` with 13 tests: interval, start/tick/stop, commands applied before the engine continues, 32-call budget, running goal not cut off, no runtime, dead guest, tick hook order, command checks, requeue, hurt/died/interact/save, bone writes, uuid; plus `mob_pose_writes_of_a_script...` in `render_state`).
* `horizon-mods`: 62 passed; `yoake-wasm --lib --tests`: 68 + integration all green; `horizon-client --lib`: 1243 passed (incl. `catalog_matches_hook_points`), 52 ignored (resources).
* `horizon-client --lib --features yoake-hook-bench,yoake-wasm/test-guests wasm_mods::mob_runtime`: **4 passed** with the real `fox_ai` guest (pounce at 4 blocks; not at 2 or 8; leap and bite on `hit`; ends in 20 ticks; no `service.mobs` -> never starts and the service says so; a trapping guest is disabled; a spinning guest is cut off).

## Deferred

* `memory-get/set` (needs F4M-4b brain memories) and `seat-set` (riding seats exist from 4c but no service is wired; F4M-4d or a follow-up).
* Built-in attributes (`max_health`, `movement_speed`) in the attribute services: custom attributes only.
* `mob-spawn` from inside a mob goal or mob hook (the world is busy): Err; use it from an ordinary server hook.
* `mob-hurt`'s `source` is the engine `DamageKind` name (`PlayerAttack`), not a damage-type id.
* `look_at: owner` for owners that are not the client's own player (needs a client-side entity lookup by UUID).
* `skeleton` `model-command` case (guest-to-host variant growth) and `play-local-anim`, `mob-think` batch: not done.
* The 5b `synced-changed` mirror on the client for the local integrated host reads through `LocalMirror::take_changes`.

## Needs checking

* The generated native hooks are never dispatched through `mod_api::dispatch_*` (direct runtime instead); a native (Rust) mod hooking them gets nothing.
* `mob-pose` frame fields `time`, `ground_speed` are derived on the client from game time and velocity.
* A real-game run of a script goal (no game resources on this laptop).
* `native_custom_mob_spawned` is `dead_code` in the build (warning only).

## Open questions

1. Should the fixture get a `.yak`-style packaging slot for a `[wasm]` section (the fox_ai guest would ride in `tools/fixtures/cinder_fox`)?
2. Should `mob-set-controller-state` hold the state (suppress the controller's transitions for N ticks) rather than only enter it?
3. Should the engine also dispatch the generated hooks through `mod_api::mods()` so native mods and the generic hook-bench see them?
4. The generator treated two enums named `Hand` as ambiguous: F4M-4c's `interact::Hand` is renamed `ClickHand` (as on the merge branch).
