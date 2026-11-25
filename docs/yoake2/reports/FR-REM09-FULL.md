# FR-REM09-FULL: F4M-11 full-access (route F) custom-mob bindings and renderer stage hooks

## Summary

Implemented Route F bindings, runtime adapters, guest SDK wrappers, example (`fox_ai_full`), renderer draw stage hooks (`before_pose`, `after_pose`, `after_vertices`), and resource-free unit/guest tests for fully custom mobs (`F4M-11`, design `docs/yoake2/FR-F4-MOBS.md` §6.3), following FR-REM12's `f_services` pattern:

1. **Bindgen target `custom_mobs`** (`crates/horizon-client/src/game/yoake2_custom_mobs.rs` + `crates/yoake-bindgen/bindgen.json`):
   - Wire types (`FVec3`, `FVec3f`, `FQuat`, `FMobValue`, `FMobInfo`, `FAttributeValue`, `FAttributeOperation`, `FMobMemoryValue`, `FMobSpawnOptions`, `FMobTransform`, `FMobBoneWrite`, `FMobPoseFrame`, `FBoneTransform`, `FInteractReply`, `FMobHookEvent`, `FMobDrawStage`, `FMobLodMode`, `FMobDrawEvent`).
   - Full S-route parity services over `mobs::custom::script` and `mobs::custom::textures`: `mob_info`, `mob_get_synced`, `mob_set_synced`, `mob_trigger_anim`, `mob_set_controller_state`, `mob_set_controller_state_hold`, `mob_release_controller_state`, `mob_navigate_to`, `mob_navigate_stop`, `mob_look_at`, `mob_jump`, `mob_set_target`, `mob_attack`, `mob_attribute_get`, `mob_attribute_set_base`, `mob_attribute_add_modifier`, `mob_attribute_remove_modifier`, `mob_set_hitbox_enabled`, `mob_data_get`, `mob_data_put`, `mob_despawn`, `mob_claim_control`, `mob_impulse`, `mob_seat_set`, `mob_memory_get`, `mob_memory_set`, `mob_spawn_queued`, `mob_bone_index`, `mob_locator`, `mob_texture_create`, `mob_texture_write`, `mob_texture_bind`.
   - Script hook & goal bridge bindings: `mob_goal_register`, `mob_hook_event`, `mob_reply_bool`, `mob_reply_hurt`, `mob_reply_interact`, `mob_reply_save`, `mob_reply_pose`.
   - Renderer stage hook bindings: `mob_draw_event`, `mob_draw_cancel`, `mob_pose_bone_count`, `mob_pose_bone_name`, `mob_pose_bone_index`, `mob_pose_get`, `mob_pose_get_all`, `mob_pose_set`, `mob_pose_write`.
2. **State, adapters, and stage hooks** (`crates/horizon-client/src/game/yoake2_custom_mobs_state.rs` + `runtime.rs`, `stage.rs`):
   - `FScriptRuntime` implements both `script::Runtime` (server hooks / `ScriptGoal`) and `script::ClientRuntime` (`mob_pose`, `mob_anim_event`, `synced_changed`) via `yoake2_mods::dispatch_custom_mob_hook`.
   - Integrated into `wasm_mods::mob_runtime::{world_runtime, install_client}` and F-only world/client session startup (`native_client_items.rs`, `yoake2_client.rs`, `native_lifecycle.rs`) so F-only worlds open `script::Session` at world start and release F-owned mob textures (`yoake-f-client`) on client disconnect.
   - Avoids editing `mobs/custom/ai/script*.rs` and `wasm_mods/mob_services.rs` (concurrently modified by `rem09-def`).
3. **Renderer stage hooks on `game/custom_mob_draw.rs`** + `StagePose` in `mobs/custom/render_state.rs`:
   - `before_pose` -> `mob_pose` / `state.update` -> `after_pose` -> `tr.draw` / `custom_mob_items` -> `after_vertices`.
   - When no F client module is loaded (`!crate::yoake2_mods::client_mods_loaded()`), the hot path pays one thread-local bool check and skips all stage setup/pose copies. Without `yoake2-slice` it compiles out completely.
   - `after_pose` lets F modules read/override bone transforms (`StagePose` calling `apply_bone_writes_with_visibility` and `bone_matrices_into`) and `before_pose` / `after_pose` support `mob_draw_cancel()`.
4. **Guest SDK & examples**:
   - `crates/yoake-guest-sdk/src/custom_mobs.rs` (typed `CustomMob`, `CallError`, `register_goal_hooks`, `register_draw_stage_hooks`, typed wrappers) exported from `crates/yoake-guest-sdk/src/lib.rs`.
   - `crates/yoake-guest-sdk/examples/fox_ai_full.rs` mirroring S-route `fox_ai` (`pounce` `ScriptGoal`, `"hit"` leap+bite, `"warmth"` on hurt).
   - `crates/yoake-guest-sdk/tests/guests/custom_mob_stage.rs` testing renderer stage order, pose override, and `before_pose` draw cancellation.

## Files Created / Modified

Created:
- `crates/horizon-client/src/game/yoake2_custom_mobs.rs`
- `crates/horizon-client/src/game/yoake2_custom_mobs_state.rs`
- `crates/horizon-client/src/game/yoake2_custom_mobs_state/runtime.rs`
- `crates/horizon-client/src/game/yoake2_custom_mobs_state/stage.rs`
- `crates/horizon-client/src/game/yoake2_custom_mobs_state/tests.rs`
- `crates/horizon-client/src/game/yoake2_custom_mobs_state/guest_tests.rs`
- `crates/yoake-guest-sdk/src/custom_mobs.rs`
- `crates/yoake-guest-sdk/examples/fox_ai_full.rs`
- `crates/yoake-guest-sdk/tests/guests/custom_mob_stage.rs`
- `docs/yoake2/reports/FR-REM09-FULL.md`

Modified:
- `crates/yoake-bindgen/bindgen.json`
- `crates/yoake-slice/src/runtime.rs`
- `crates/yoake-guest-sdk/src/lib.rs`
- `crates/yoake-guest-sdk/src/stubs/mod.rs`
- `crates/horizon-client/src/yoake2_mods.rs`
- `crates/horizon-client/src/game.rs`
- `crates/horizon-client/src/game/native_client_items.rs`
- `crates/horizon-client/src/game/native_lifecycle.rs`
- `crates/horizon-client/src/game/yoake2_client.rs`
- `crates/horizon-client/src/game/custom_mob_draw.rs`
- `crates/horizon-client/src/game/custom_mob_textures.rs`
- `crates/horizon-client/src/mobs/custom/render_state.rs`
- `crates/horizon-client/src/wasm_mods/mob_runtime.rs`
- `.lane/TASK.md`

## Synthetic Hook IDs Added

In `crates/yoake-slice/src/runtime.rs` (`SYNTHETIC_HOOK_BASE = 0x8000_0000`) and `yoake_guest_sdk::hooks` + `horizon_client::yoake2_mods::ENGINE_DISPATCHED`:

| Offset | Name | Mode | Purpose |
|---:|---|---|---|
| `+7` | `HOOK_CUSTOM_MOB_BEFORE_POSE` | `Full` | Custom-mob draw stage before pose evaluation (cancellable) |
| `+8` | `HOOK_CUSTOM_MOB_AFTER_POSE` | `Full` | Custom-mob draw stage after pose evaluation (pose read/override, cancellable) |
| `+9` | `HOOK_CUSTOM_MOB_AFTER_VERTICES` | `ObserveOnly` | Custom-mob draw stage after vertex emission |
| `+10` | `HOOK_MOB_GOAL_CAN_USE` | `Full` | `ScriptGoal::can_use` |
| `+11` | `HOOK_MOB_GOAL_CAN_CONTINUE` | `Full` | `ScriptGoal::can_continue` |
| `+12` | `HOOK_MOB_GOAL_START` | `ObserveOnly` | `ScriptGoal::start` |
| `+13` | `HOOK_MOB_GOAL_TICK` | `ObserveOnly` | `ScriptGoal::tick` |
| `+14` | `HOOK_MOB_GOAL_STOP` | `ObserveOnly` | `ScriptGoal::stop` |
| `+15` | `HOOK_MOB_TICK` | `ObserveOnly` | Server `mob_tick` |
| `+16` | `HOOK_MOB_SPAWN_CHECK` | `Full` | Server `mob_spawn_check` |
| `+17` | `HOOK_MOB_SPAWNED` | `ObserveOnly` | Server `mob_spawned` |
| `+18` | `HOOK_MOB_HURT` | `Full` | Server `mob_hurt` |
| `+19` | `HOOK_MOB_DIED` | `ObserveOnly` | Server `mob_died` |
| `+20` | `HOOK_MOB_INTERACT` | `Full` | Server `mob_interact` |
| `+21` | `HOOK_MOB_ANIM_EVENT` | `ObserveOnly` | Server `mob_anim_event` |
| `+22` | `HOOK_MOB_LOAD` | `ObserveOnly` | Server `mob_load` |
| `+23` | `HOOK_MOB_SAVE` | `Full` | Server `mob_save` |
| `+24` | `HOOK_MOB_TAMED` | `ObserveOnly` | Server `mob_tamed` |
| `+25` | `HOOK_MOB_BRED` | `ObserveOnly` | Server `mob_bred` |
| `+26` | `HOOK_MOB_SYNCED_CHANGED_SERVER` | `ObserveOnly` | Server `synced_changed` |
| `+27` | `HOOK_MOB_POSE` | `Full` | Client `mob_pose` |
| `+28` | `HOOK_MOB_ANIM_EVENT_CLIENT` | `ObserveOnly` | Client `mob_anim_event` |
| `+29` | `HOOK_MOB_SYNCED_CHANGED_CLIENT` | `ObserveOnly` | Client `synced_changed` |

*(Binding IDs in `crates/yoake-bindgen/ids.lock.json` and `yoake_glue/custom_mobs.rs` are assigned when Claude runs `crates/yoake-bindgen/regen.sh`.)*

## Tests

Added resource-free tests in `crates/horizon-client/src/game/yoake2_custom_mobs_state/`:
- `tests::server_services_validate_arguments_and_queue_engine_commands`: validates arguments and checks queued engine `script::Command`s against Cinder Fox fixture.
- `tests::client_texture_store_and_bone_helpers_round_trip`: tests `bone_index`, `texture_create`/`write`/`bind`, and `release_f_textures`.
- `tests::reply_and_stage_pose_helpers_refuse_outside_their_hook`: verifies hook-context gating on `reply_*`, `draw_cancel`, and `pose_*`.
- `tests::draw_stage_hooks_are_noop_when_no_f_module_is_loaded`: verifies zero interference when no F client module is active.
- `guest_tests::draw_stage_hooks_run_in_order_edit_pose_and_cancel_in_before_pose`: compiles `custom_mob_stage.rs` to wasm32 and checks stage ordering (`before_pose` -> `after_pose` -> `after_vertices`), bone pose editing changing output vertices, and `before_pose` cancellation skipping drawing.
- `guest_tests::fox_ai_full_scripts_pounce_through_script_goal_and_f_runtime` (`yoake2-slice,yoake-hook-bench`): compiles `examples/fox_ai_full.rs` to wasm32, opens `script::Session` with `f_world_runtime()`, and verifies `ScriptGoal` (`pounce`), `"hit"` anim attack, and `MOB_HURT` updating `"warmth"`.

## What Is Left (For Claude)

1. Run `crates/yoake-bindgen/regen.sh` to generate `crates/horizon-client/src/yoake_glue/custom_mobs.rs`, update `crates/yoake-bindgen/ids.lock.json`, generate `crates/yoake-guest-sdk/src/stubs/custom_mobs.rs`, and update `docs/yoake2/bindgen/*`.
2. Build and run unit/guest tests for `yoake-bindgen`, `yoake-slice`, `yoake-guest-sdk`, `horizon-client-lib`, and `horizon-client` (both default features and `--features yoake2-slice,yoake-hook-bench`).
3. Inspect any compiler/clippy diagnostics or `regen.sh --check` drift and record results in `.lane/TASK.md`.
