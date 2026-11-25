# FR-REM09-DEF: F4M-10 deferred items (lane DEF)

Branch `yoake-free/rem09-def` in `wt-r09-def`. Engine-side completion of F4M-10's deferred items without Minecraft resources, keeping F-route reusable helpers and leaving protected/generated files untouched.

## What changed

### 1. Built-in attributes (`mob-attribute-get/-set-base/-add-modifier/-remove-modifier`, ids 272-275)
- `crates/horizon-client/src/mobs/finalize.rs`: Exposed `Attr`, `Op`, `base_value`, `value`, `set_base`, `has_modifier`, `add_modifier`, and `remove_modifier` (`pub`). Built-in clamping (`Attr::range`) and lowering-max-health health clamping (`refresh(m, Attr::MaxHealth)`) run on every built-in update.
- `crates/horizon-client/src/mobs/custom/attrs.rs`:
  - Added `has(m, id)` and `get(m, id) -> Option<(f64, f64)>` alongside `set_base`, `add_modifier`, and `remove_modifier` so both S-route (`mob_services`) and `rem09-full` (F-route) share one engine path dispatching between declared built-ins (`m.kind.attributes.contains_key(a.id())` via `Attr::from_id`) and custom `AttrSet` entries.
  - Updated `attribute_scale(m)` to read `finalize::value(m, Attr::Scale)` when `minecraft:scale` is in `m.kind.attributes` (or `m.extra.attributes` is set), keeping default `1.0` for vanilla / no-scale mobs.
- `crates/horizon-client/src/mobs/custom/script.rs`: `MobView::of_in` now includes declared built-in attributes `(a.id(), base_value, value)` without duplicating custom attributes, sorted by id.
- `crates/horizon-client/src/mobs/custom/script_apply.rs`: Calls `attrs::sync_scale(m, w)` when `SetAttributeBase`, `AddModifier`, or `RemoveModifier` modifies `scale` / `minecraft:scale`.
- `crates/horizon-client/src/wasm_mods/mob_services.rs`: Extracted `find_attribute(&v, attribute)` shared across `attribute_get`, `attribute_set_base`, `attribute_add_modifier`, and `attribute_remove_modifier` (accepts bare or `minecraft:` built-in names and custom ids; fails closed if absent). `attribute_add_modifier` also validates modifier identifiers via `horizon_world::attributes::Modifier::new`.

### 2. Engine support for `skeleton` and client-side `play-local-anim`
- `crates/horizon-client/src/mobs/custom/render_state.rs`:
  - Extracted shared bone-quad emission helper `TypeRender::emit_bone_from` used by both `TypeRender::emit_bone` and `TypeRender::draw_skeleton`.
  - Added `TypeRender::draw_skeleton` and `render_state::draw_skeleton_model` to pose a custom model skeleton from a rest pose using `MobBoneWrite` bone writes + visibility and draw a requested layer (`None`/`""`/`"default"`, `"glowmask"`, or a named `model.layers` entry).
  - Added client-only `LocalAnimPlay` playback on `MobRender` (`MobRender::play_local_anim`, `MobRender::local_anim_count`) and `CustomRender::play_local_anim`, advancing by client frame `dt` and sampling in `evaluate_rig` without any server sync. Cleared on `MobRender::rebind`.
- `crates/horizon-client/src/mobs/custom/script.rs`: Added `LocalAnimRequest` client queue (`has_local_anims`, `queue_local_anim`, `take_local_anims`, `clear_local_anims`) with lock-free `AtomicBool` fast path.
- `crates/horizon-client/src/game/custom_mob_draw.rs`: Drains queued `script::take_local_anims(mob.id)` into `MobRender::play_local_anim` before `state.update` (fast-path skipped when empty).
- `crates/horizon-client/src/game/native_block_entity_draw.rs`: Added `Game::native_draw_skeleton` helper over `draw_skeleton_model`.

### 3. Per-`Game` / per-session command bus (`mobs/custom/script.rs`)
- Replaced the single process-wide blocking `BUS` + `RUNTIME` constraint with per-`Session` `SessionInner` (`id`, `runtime: RwLock<Option<Arc<dyn Runtime>>>`, `bus: Mutex<Bus>` holding `views`, `queue`, `current`, and `dropped`).
- `script::open_session(runtime)` now allows multiple concurrent sessions without returning `SessionBusy`, each with its own runtime and command bus. When no fallback slot is occupied, the session also populates the fallback slot for legacy single-session callers.
- Added `SessionHandle`, `Session::bind() -> SessionGuard` (thread-local RAII scope binding), `Session::in_scope`, and direct session methods (`publish`, `forget`, `view`, `push`, `take_for`, `queued`, `dropped`, `current`, `reset`, `runtime`, `installed`).
- `Session::drop` clears only its own runtime/bus (and the fallback slot only if still owned by that session id), leaving concurrent sessions intact; clears locators and local anims when the last open session drops.
- `crates/horizon-client/src/game/native_lifecycle.rs`, `crates/horizon-client/src/game/levels.rs`, and `crates/horizon-client/src/wasm_mods.rs`: Updated `open_world_script_session`, added `Game::bind_script_session`, and bound the server's session around `Game::tick_server` and `native_lifecycle_world_ready`.

## WIT / ABI changes
- **None applied directly**, due to lane constraints and frozen-baseline rules (see "What is left"):
  - `abi-subset-client.json` and `wasm_mods/client_services.rs` are explicitly protected by the lane's `Do not touch` list, so `play-local-anim` could not be added to `client-services` here.
  - `yoake-full.wit` is generated by `horizon-mod-api-gen` (not hand-editable; no shell available), and `variant model-command` is frozen in `crates/yoake-wasm/wit/abi-1.0-baseline.wit` where `baseline.rs::existing_wit_items_are_immutable` currently rejects appending cases to an existing `variant`. Adding `skeleton` to `MODEL_COMMAND` in `wit_full/types.rs` without regenerating `yoake-full.wit` and updating `wasm_mods/full.rs::apply_model` together would break compilation/tests.

## Files modified
- `crates/horizon-client/src/mobs/finalize.rs` (prior agent; retained)
- `crates/horizon-client/src/mobs/custom/attrs.rs`
- `crates/horizon-client/src/mobs/custom/script.rs`
- `crates/horizon-client/src/mobs/custom/script_apply.rs`
- `crates/horizon-client/src/mobs/custom/ai/tests_script.rs`
- `crates/horizon-client/src/mobs/custom/render_state.rs`
- `crates/horizon-client/src/wasm_mods.rs`
- `crates/horizon-client/src/wasm_mods/mob_services.rs`
- `crates/horizon-client/src/game/native_lifecycle.rs`
- `crates/horizon-client/src/game/levels.rs`
- `crates/horizon-client/src/game/custom_mob_draw.rs`
- `crates/horizon-client/src/game/native_block_entity_draw.rs`

## Tests added (resource-free, not yet executed — no shell in this lane)
- `mobs::custom::attrs::builtin_tests::builtin_attributes_get_set_modify_clamp_and_sync_health_and_scale`
- `wasm_mods::mob_services::attr_tests::attribute_services_support_builtin_and_custom_attributes_with_health_clamp`
- `mobs::custom::ai::tests_script::two_concurrent_sessions_keep_separate_buses_and_runtimes`
- `mobs::custom::ai::tests_script::a_test_runtime_and_replacement_stay_untouched_when_an_unrelated_session_drops`
- `mobs::custom::ai::tests_script::builtin_attribute_commands_apply_and_clamp_health_and_sync_scale`
- `mobs::custom::render_state::tests::draw_skeleton_poses_bones_applies_visibility_and_selects_layer`
- `mobs::custom::render_state::tests::play_local_anim_advances_on_client_only_and_clears_after_ending`

## What is left
1. **WASM client service for `play-local-anim`**: Add `mob_play_local_anim` (e.g., id `299`, `entity: u32, name: string, speed: f32 -> result<bool, string>`, permission `service.mobs`, catalog minor version `1.10.0 -> 1.11.0`) to `abi-subset-client.json`, `crates/yoake-wasm/src/client_services.rs`, `crates/horizon-client/src/wasm_mods/client_services.rs` (calling `script::queue_local_anim`), and `yoake-wasm-sdk/src/full.rs`, then regenerate WIT/SDK/adapters.
2. **WIT `skeleton` `model-command` case**: Decide between (a) allowing trailing guest-to-host variant extension in `crates/horizon-mod-api-gen/src/abi/baseline.rs` (or refreshing `abi-1.0-baseline.wit` before release) vs (b) adding a new non-baseline command type/hook, then add `skeleton(model-skeleton)` (`{ model: string, pose: list<mob-bone-write>, texture-layer: option<string> }`) to `wit_full/types.rs` and handle `et::ModelCommand::Skeleton` in `wasm_mods/full.rs::apply_model`. Note: `Game::native_draw_skeleton` currently resolves textures against `self.gui.atlas.sprites`; verify/pass the entity atlas if custom mob textures are not registered in the GUI sprite atlas when drawn via `BlockEntityDraw`.
3. **Remaining per-`Game` gaps**: `MobWorld` still does not carry a `SessionHandle` directly (thread-local `Session::bind()` brackets `Game::tick_server` and lifecycle hooks); `ClientRuntime` (`CLIENT`), `LOCATORS`, and `mob_services::GRANTED` remain process-wide.
4. **Build & test verification**: Run Claude's commands listed in `.lane/TASK.md` under `## Commands for Claude`.
