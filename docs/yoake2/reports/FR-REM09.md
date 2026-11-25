# FR-REM09: F4M-11 full-access custom-mob bindings, F4M-10 deferred items

Branch `yoake-free/rem09` (from `yoake-free/merge` 47d227de1). Two lanes, merged here: `rem09-full` (F4M-11) and
`rem09-def` (F4M-10 deferred). Code by Kimi (first part, stopped at its 5-hour limit) and agy Hydrogen; review,
fixes, regen.sh and tests by Claude on LENOVO. Lane details: `FR-REM09-FULL.md`, `FR-REM09-DEF.md` (where they
disagree with this file, this file is right: Claude's changes are listed below). Nothing here was run in a game.

## Against the request

| Item | State |
| --- | --- |
| F4M-11 bindgen target `custom_mobs` | Done. Bound surface `game/yoake2_custom_mobs.rs` (wire types + thin wrappers), state and adapters in `game/yoake2_custom_mobs_state{.rs,/runtime.rs,/stage.rs}` (not a target). S-route parity over `mobs::custom::script`: info, synced values, anim triggers, controller state (+hold/release), navigate/look/jump, target/attack, attributes, hitbox, data get/put, despawn, claim control, impulse, seats, memories, bone index/locator, textures; `mob_spawn_queued` (validated now, spawned right after `yoake2_server_tick`, because a `&mut Game` receiver gets no guest stub and the world is busy inside mob ticks). Goals and mob hooks: synthetic engine-dispatched hook ids (`SYNTHETIC_HOOK_BASE + 7..=29`, FR-REM12's pattern; `hook_register` takes numeric ids only), event payload by `mob_hook_event()`, answers by `mob_reply_*`; `FScriptRuntime` implements the same `script::Runtime` / `ClientRuntime` that `ScriptGoal` calls, so both routes share one mechanism. regen.sh run: `yoake_glue/custom_mobs.rs`, `yoake-guest-sdk/src/stubs/custom_mobs.rs`, `ids.lock.json` **appended only** (no line removed). |
| Guest SDK, `fox_ai_full` | Done. `yoake-guest-sdk/src/custom_mobs.rs` (typed `CustomMob`, goal/draw-stage registration helpers); `examples/fox_ai_full.rs` mirrors `fox_ai` (pounce goal, hit leap+bite, warmth on hurt); guest tests `tests/guests/custom_mob_stage.rs` and `yoake2_custom_mobs_state/guest_tests.rs` (`fox_ai_full_scripts_pounce_...`). |
| Renderer stage hooks | Done. `custom_mob_draw.rs`: `before_pose` (cancel skips the engine draw) -> pose -> `after_pose` (read/override bone transforms via `StagePose` in `render_state.rs`, cancel) -> vertices -> `after_vertices` (observe). With no F client module: one thread-local bool check; without `yoake2-slice` compiled out. |
| F4M-10 built-in attributes | Done for the engine's existing built-ins (`finalize::Attr`: max_health, movement_speed, follow_range, attack_damage, knockback_resistance, spawn_reinforcements, jump_strength, armor, burning_time, flying_speed), when the type's attribute map declares them; vanilla ranges clamp; lowering max health clamps health. Ids 272-275 and the F route both use `mobs::custom::attrs::{get,set_base,add_modifier,remove_modifier}`. |
| `skeleton`, `play-local-anim` | Engine side only: `TypeRender::draw_skeleton` / `render_state::draw_skeleton_model` / `Game::native_draw_skeleton`; client-only `MobRender::play_local_anim` queued by `script::queue_local_anim` and drained in `custom_mob_draw.rs` (no sync). **Not wired to WASM**: `model-command` is frozen in `abi-1.0-baseline.wit` (`baseline.rs` rejects appending variant cases), and `play-local-anim` would go in `abi-subset-client.json` / `wasm_mods/client_services.rs`, which are on the do-not-touch list. Needs a decision (see "Left"). |
| Per-`Game` bus | Done for the server side: `script::Session` owns its own bus (views, queue, `current`, drop counter) and runtime; several sessions may be open; `Game::bind_script_session` binds the session's thread-local scope around `tick_server` and world-ready. Still process-wide: `ClientRuntime`, `LOCATORS`, `mob_services::GRANTED`; `MobWorld` carries no session handle (the thread-local guard does it). |

## Claude's changes after the lanes

* **Reverted** new `finalize::Attr` variants (attack_knockback, armor_toughness, step_height, gravity, safe_fall_distance,
  scale, fall_damage_multiplier) and a max-health clamp added inside `finalize::refresh`: `Attr::ALL` and `refresh`
  run for every vanilla mob, so that would change vanilla. The clamp now runs only after a service call
  (`attrs::after_builtin`). `attribute_scale` and the scale branches in `script_apply.rs` are back to the original.
  `finalize::Modifier.id` is now a `String` (a guest names modifiers at run time); `add_modifier` takes `impl Into<String>`.
* Test fixes: the concurrent-session test reused a queue it had just filled to 4096 (emptied first now); Kimi's unused
  `wasm_mods/f_custom_mob*.rs` deleted; a `Vertex.position` -> `.pos`.
* agy's fix for the stage-hook test: `yoake_slice::runtime::set_thread_engine_dispatch` (a thread-local engine
  dispatch checked before the process-wide one), because parallel guest tests replaced each other's `DISPATCH`.
* regen.sh rerun after the merge (the DEF lane made `finalize::Attr` reachable from the `mobs` target): only `mobs`
  glue/stubs/docs and appended ids changed. Line-ending-only rewrites of the other glue files are not committed.

## Tests (LENOVO, merged branch)

horizon-client-lib --lib 1973 passed; horizon-client --lib --bins --tests 1300 and the integration tests passed;
horizon-client --lib --features yoake2-slice 1306 passed, 1 failed:
`launch_options::yoake_options_tests::guest_reads_game_options_and_w10_world_fields_and_setters_are_read_only`, which
fails the same way without this lane's F changes (the DEF-only branch) — likely the shared `DISPATCH` race the stage
test had; it can move to `set_thread_engine_dispatch`. `--test yoake2_examples` 2 passed, yoake-slice 33 passed.
yoake-guest-sdk `rustc_and_wasm_ld_retain_the_sdk_abi_section` and yoake-bindgen `mini_engine` (2) fail at **linking**
(`rust-lld` / `link.exe` 1181) on this laptop, also on the DEF-only branch: environment, not this change. Not run here:
anything with the game (render e2e is on GALLERIA).

## Left

* WASM wiring of `skeleton` (baseline policy: allow trailing variant cases, or a new command type) and
  `play-local-anim` (client service id, e.g. 299, catalog 1.10.0 -> 1.11.0, in files GALLERIA owns).
  `native_draw_skeleton` resolves textures in the GUI atlas: check custom mob textures there.
* Per-`Game` client runtime, locators and grants; built-ins beyond `finalize::Attr` (needs vanilla-safe growth).
* The `launch_options` guest test race above.
