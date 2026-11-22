# FR-F4M-4b: brain option for custom mobs (sensors, typed memories, activities)

Branch `yoake-free/f4m-4b`, from `yoake-free/merge` (`e44e784d8`).

## 1. What was implemented vs the design (`FR-F4-MOBS.md`)

Design sections covered: 3.3 (Brain option), 3.10 (Persistence `yoake.memories`), 5 (Sensors), 6.1 (Data schema), 6.2 (`memory-get` / `memory-set`), and the F4M-4b work item.

### Schema (`crates/horizon-mods/src/custom_mob.rs`)
* Added brain schema types and limits (`limits::MAX_MEMORIES = 64`, `MAX_SENSORS = 32`, `MAX_ACTIVITIES = 32`, `MAX_MEMORY_STRING = 256`, `MAX_MEMORY_TTL = 72_000`, `MAX_SCHEDULE_ENTRIES = 32`):
  * `Ai`: `memories: BTreeMap<String, MemoryDef>`, `sensors: Vec<Sensor>`, `activities: BTreeMap<String, Activity>`, `schedule: Vec<ScheduleEntry>`, `default_activity: Option<String>`.
  * `Goal.activity: Option<String>` so top-level `ai.goals` / `ai.targets` rows can also be gated by an activity.
  * `MemoryDef`: `type` (`MEMORY_TYPES`: `bool`, `i32`, `f32`, `string`, `vec3`, `block_pos`, `entity`), `default`, `persistent`, `ttl`.
  * `Sensor`: `type` (`SENSOR_TYPES`: `nearest_players`, `nearest_living`, `hurt_by`, `blocks`, `light`), `interval` (`1..=2400`, default `20`), `memory`, `ttl`, `range` (`1..=128`), `must_see`, `ignore_creative`, `types`, `max_age`, `tag`, `blocks`, `radius` (`1..=32`), `vertical_radius` (`0..=16`), `kind` (`LIGHT_KINDS`: `combined`, `sky`, `block`).
  * `Activity`: `priority` (`0..=32`, default `5`, lower wins), `core: bool`, `when: Vec<MemoryPredicate>` (`present`, `eq`, `ne`, `gt`, `gte`, `lt`, `lte`), `goals`, `targets`.
  * `ScheduleEntry`: `time` (`0..24000`), `activity`.
* Validation in `CustomMobFile::check_ai` / `check_brain` checks memory kinds and defaults, sensor-to-memory type compatibility, predicate memory declarations and types, schedule times and non-core activity references, and goal/activity counts.

### Engine brain runtime (`crates/horizon-client/src/mobs/custom/ai/`)
* **`memory.rs`**: `MemoryKind`, `EntityMemory` (`Mob(u32)` | `Player([i32; 4])`), `MemoryValue`, `MemoryDef`, `MemoryEntry`, `MemoryBank` (`from_defs`, `get`, `set`, `erase`, `forget_outdated`). Expiry follows Minecraft's `ExpirableValue` timing (`ttl = Some(N)` stays for `N - 1` tick boundaries and expires on tick `N`; `ttl = Some(0)` expires on the next tick; `ttl = None` persists until overwritten or erased).
* **`sensors.rs`**: `CompiledSensor`, `SensorState`, `tick` (fires each sensor when its cooldown hits 0, then resets cooldown to `interval.saturating_sub(1)`). Implements all five sensor types (`nearest_players`, `nearest_living`, `hurt_by`, `blocks`, `light`). Empty scan results without a TTL erase the memory; with a TTL, the entry expires naturally.
* **`brain.rs`**: `CompiledPredicate`, `ActivityDef`, `BrainDef`, `BrainState`, `tick_pre` (expires outdated memories and ticks due sensors before scripts), `update_activity` (evaluates `core` activities and picks the winning non-core activity from `when` predicates, `ai.schedule` over `day_time % 24000`, `default_activity`, `"idle"`, or lowest-priority unpredicated activity), and `ActivityGoal` wrapping `Goal` so `can_use` and `can_continue` require `is_active(m, &self.activity)`.
  * At equal `priority`, an activity with passing `when` predicates beats a schedule-only (`when` empty) activity.
  * When a mob has no brain (`CustomType::brain == None`), `BrainDef::from_file` returns `None`, no goals are wrapped, and `tick_pre`/`update_activity` return immediately with zero behaviour change to plain goals.
* **`mod.rs`, `custom/mod.rs`, `custom/build.rs`, `mobs.rs`, `mobs/goals.rs`**:
  * `CustomType::brain: Option<BrainDef>`, `CustomState::brain: BrainState`.
  * `GoalSelector::wrap_from` wraps goals/targets belonging to an activity in `ActivityGoal`.
  * `ai::attach` wraps both top-level goals with `g.activity` and `brain_def.activities` goals/targets.
  * `ai::unsupported` and `build::mob_type` (`flags.climber`) inspect `activities` goals/targets in addition to top-level `ai.goals`/`ai.targets`.
  * `Mob::tick` runs `brain::tick_pre` before `script::pre_tick`, and `brain::update_activity` after `script::pre_tick` and before `ai::tick`.
* **Persistence (`crates/horizon-client/src/mobs/custom/persist.rs`)**:
  * Saves persistent memories under `yoake.memories.<name> = { value: ..., ttl?: int }`.
  * Preserves unknown keys in `yoake.memories` across save/load cycles; skips expired (`ttl <= 0`) or non-persistent entries; refreshes active activity on load.

### WASM services, WIT, SDK, and example
* **ABI / WIT / SDK**:
  * `crates/horizon-mod-api-gen/abi-subset-server.json`: added services `287` (`mob_memory_get`) and `288` (`mob_memory_set`), type `mob-memory-value`, enum `mob-memory-kind`, interface version `1.6.0`.
  * `crates/yoake-wasm/wit/yoake-full.wit` (plus `coverage.json`, `inject-sites.json`, `yoake-wasm/src/full.rs`, `yoake-wasm/src/full_hook_names.rs`, `yoake-wasm-sdk/src/full.rs`, `full_*_generated.rs`, `full_adapters.rs`, `full_adapters_report.txt`, `yoake-inject-rt/src/catalog.rs`, `wit-inject/`, `wit-inject-config/`, `wit-worker/`): regenerated for `mob-memory-get` / `mob-memory-set`, `mob-memory-kind`, `mob-memory-value` and verified against `horizon-mod-api-gen` tests.
* **Host implementation**:
  * `crates/horizon-client/src/mobs/custom/script.rs`: `MobView.memories`, `Command::SetMemory`.
  * `crates/horizon-client/src/mobs/custom/script_apply.rs`: applies `Command::SetMemory` (coerces value, applies `ttl_ticks` or memory default TTL, updates active activity immediately when `MobWorld` is available).
  * `crates/horizon-client/src/wasm_mods/mob_services.rs` & `server_services.rs`: `memory_get` and `memory_set` gated by `service.mobs`, validating key names, memory existence, type match, finite floats, string length (`<= 256`), entity target, and TTL (`<= 72_000`).
* **Example (`fox_ai` + Cinder Fox fixture)**:
  * `tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json`: declares `memories` (`prey_nearby: entity`, `pounced: bool`), `sensors` (`nearest_living` watching `minecraft:chicken` into `prey_nearby` every 10 ticks), `default_activity: "idle"`, and `activities` (`hunt` when `prey_nearby` is present with `look_at_target`, and `idle`).
  * `crates/yoake-wasm-sdk/examples/fox_ai/src/lib.rs`: `mob_goal_can_use` checks `mobs::memory_get(mob, "pounced")`, and `mob_goal_start` calls `mobs::memory_set(mob, "pounced", Some(mobs::memory_bool(true)), Some(40))` so `pounce` has a 40-tick brain memory cooldown.
  * `tools/custom_mob_render_e2e.py`: adds `BRAIN_SWITCH` log check (`brain`) verifying `Some("idle") -> Some("hunt") -> Some("idle")` when a chicken is summoned and killed.

### Docs
* `docs/yoake2/MOD-GUIDE.md` (Section 17) and `docs/yoake2/MOD-GUIDE.ja.md` (Section 17): documents `ai.memories`, `ai.sensors`, `ai.activities`, `ai.schedule`, `ai.default_activity`, and `mobs::memory_get` / `mobs::memory_set`.

## 2. Fixes made while finishing the branch

* `crates/horizon-client/src/mobs/custom/ai/tests_4b.rs` (`activity_switching_core_goals_and_schedule_work_end_to_end`):
  * Ran 2 initial `run_server` ticks so gravity settles `on_ground = true` before `AvoidEntity` pathfinding runs (tick 1 leaves `on_ground = false` when `velocity.y == 0.0`).
  * Added `"interval": 1` to the test's `nearest_players` sensor so it re-scans on the next tick after the player enters range.
* `crates/horizon-client/src/wasm_mods/mob_runtime.rs` (`the_memory_services_read_write_expire_and_check_types_and_permissions`):
  * Added `use yoake_wasm::ServerHost;` in the test scope and installed a `NoopRuntime` (cleaned up on drop) so `script::pre_tick` applies queued `SetMemory` commands during `Mob::tick`.
* `crates/horizon-client/src/mobs/custom/ai/brain.rs` (`CompiledPredicate::eval`):
  * Replaced negated float comparisons (`!(num > b)` etc.) with `is_none_or` checks to clear `clippy::neg_cmp_op_on_partial_ord` warnings.

## 3. Tests and commands

* Disk space check (`df -h /c`): `13G`–`14G` free on `C:` (> 12 GB).
* `.\hz-cargo.cmd test -j 6 -p horizon-mods`: **64 passed**, 0 failed (includes `brain_memories_sensors_activities_and_schedule_are_validated`).
* `.\hz-cargo.cmd test -j 6 -p horizon-mod-api-gen --lib`: **46 passed**, 0 failed (includes `abi_1_0_is_additive`, `committed_wasm_adapters_match_the_catalog`, `guest_sdk_full_bindings_match_the_catalog`, `full_wit_matches_catalog_and_parses`).
* `.\hz-cargo.cmd test -j 6 -p horizon-client --lib --features yoake-hook-bench,yoake-wasm/test-guests wasm_mods::mob_runtime`: **9 passed**, 0 failed (rebuilt `target/wasm32-wasip2/release/yoake_wasm_fox_ai.wasm`; includes `the_memory_services_read_write_expire_and_check_types_and_permissions` and `the_fox_ai_guest_writes_and_reads_pounced_memory_with_ttl_and_brain_switches_activities`).
* `.\hz-cargo.cmd build -j 6 -p horizon-client -p horizon-client-lib --bin horizon-client --bin horizon-ctl`: **succeeded**.
* `.\hz-py.cmd tools/custom_mob_render_e2e.py --bin-dir target/debug --wasm target/wasm32-wasip2/release/yoake_wasm_fox_ai.wasm --out target/custom-mob-render-e2e-wasm --shots target/e2e-shots-wasm`: **passed** (8/8 checks: `idle`, `glow`, `pounce`, `lod`, `name`, `brain` with `Some("idle") -> Some("hunt") -> Some("idle")`, `strike` dealing 3.0 damage on guest, `death`). *(Run prior to the clippy-only `is_none_or` rewrite in `brain.rs`.)*
* `.\hz-cargo.cmd clippy -j 6 -p horizon-mods -p horizon-client-lib --lib --tests`: **succeeded** (4 `clippy::neg_cmp_op_on_partial_ord` warnings in `brain.rs` were fixed afterwards).
* `.\hz-cargo.cmd test -j 6 -p horizon-client-lib --lib custom`: **262 passed**, 0 failed (1717 filtered out; 255 prior + 7 new tests in `mobs::custom::ai::tests_4b`), run after the final `brain.rs` edit.

## 4. What is left

* Full `horizon-client --lib` test suite, `tools/custom_mob_render_e2e.py` without `--wasm` (data-only mode), `tools/pr_ready.py`, `tools/parity.py`, and `crates/yoake-bindgen/regen.sh --check` were not run in this task.
* Temporary helper scripts `hz-cargo.cmd` and `hz-py.cmd` remain untracked in the worktree root (per instructions, not committed).
