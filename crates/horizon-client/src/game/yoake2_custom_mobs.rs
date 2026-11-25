//! F4M-11: full-access (route F) custom-mob host services, script goal/hook bridge, and renderer
//! stage pose bindings (`yoake-bindgen` target `custom_mobs`).
//!
//! This file is the bound surface only (plain wire types and guest-callable free functions):
//! `yoake-bindgen` reads every function here and generates `yoake_glue::call` and
//! `crates/yoake-guest-sdk/src/stubs/custom_mobs.rs`. Internal state, validation helpers, runtime
//! adapters, and stage-hook plumbing live in `yoake2_custom_mobs_state.rs`, which is not a target.

/// A 3D double-precision point or vector.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FVec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A 3D single-precision vector (rotation in degrees, translation in model pixels, scale, or synced vec3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FVec3f {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// A unit quaternion `(x, y, z, w)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FQuat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

/// A tracked (synced) value of a custom mob, matching S-route `mob-value`.
#[derive(Clone, Debug, PartialEq)]
pub enum FMobValue {
    Bool(bool),
    Int(i32),
    Float(f32),
    Text(String),
    Vec3(FVec3f),
    Choice(String),
}

/// Snapshot returned by `mob_info`.
#[derive(Clone, Debug, PartialEq)]
pub struct FMobInfo {
    pub handle: u32,
    pub type_id: String,
    pub dimension: String,
    pub position: FVec3,
    pub velocity: FVec3,
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub max_health: f32,
    pub alive: bool,
    pub on_ground: bool,
    pub target: Option<u32>,
    pub target_player: Option<String>,
    pub target_position: Option<FVec3>,
    pub target_visible: bool,
    pub owner: Option<String>,
    pub sitting: bool,
    pub baby: bool,
    pub age: i32,
    pub controlled: bool,
}

/// Result of `mob_attribute_get`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FAttributeValue {
    pub base: f64,
    pub value: f64,
}

/// Attribute modifier operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FAttributeOperation {
    AddValue,
    AddMultipliedBase,
    AddMultipliedTotal,
}

/// Typed brain memory value matching S-route `mob-memory-value`.
#[derive(Clone, Debug, PartialEq)]
pub enum FMobMemoryValue {
    Bool(bool),
    I32(i32),
    F32(f32),
    String(String),
    Vec3(FVec3f),
    BlockPos(i32, i32, i32),
    EntityMob(u32),
    EntityPlayer(String),
}

/// Spawn options for `mob_spawn_queued`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FMobSpawnOptions {
    pub yaw: f32,
    pub baby: bool,
    pub persistent: bool,
}

/// World-space locator transform returned by `mob_locator`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FMobTransform {
    pub position: FVec3,
    pub yaw: f32,
    pub pitch: f32,
}

/// One bone's change returned by `mob_pose` or applied via `mob_pose_write`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FMobBoneWrite {
    pub bone: u32,
    pub additive: bool,
    pub translation: Option<FVec3f>,
    pub rotation_degrees: Option<FVec3f>,
    pub scale: Option<FVec3f>,
    pub visible: Option<bool>,
}

/// Frame info delivered to `MOB_POSE`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FMobPoseFrame {
    pub time: f32,
    pub partial: f32,
    pub ground_speed: f32,
    pub head_yaw: f32,
    pub head_pitch: f32,
    pub on_ground: bool,
    pub target: Option<FVec3f>,
}

/// One bone's local transform in the evaluated pose (`after_pose` stage).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FBoneTransform {
    pub translation: FVec3f,
    pub rotation: FQuat,
    pub scale: FVec3f,
    pub visible: bool,
}

/// Reply returned from `MOB_INTERACT`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FInteractReply {
    Pass,
    Consume,
    Success,
}

/// Payload of the active custom-mob hook, read with [`mob_hook_event`].
#[derive(Clone, Debug, PartialEq)]
pub enum FMobHookEvent {
    GoalCanUse { mob: u32, goal: String },
    GoalCanContinue { mob: u32, goal: String },
    GoalStart { mob: u32, goal: String },
    GoalTick { mob: u32, goal: String },
    GoalStop { mob: u32, goal: String },
    MobTick { mob: u32 },
    MobSpawnCheck { type_id: String, dimension: String, pos: (i32, i32, i32) },
    MobSpawned { mob: u32, type_id: String, reason: String },
    MobHurt { mob: u32, part: String, source: String, amount: f32 },
    MobDied { mob: u32, type_id: String, attacker: Option<u32> },
    MobInteract { mob: u32, player: String, hand: String, item: String },
    MobAnimEvent { mob: u32, name: String },
    MobLoad { mob: u32, data: Vec<u8> },
    MobSave { mob: u32, data: Vec<u8> },
    MobTamed { mob: u32, owner: String },
    MobBred { mob: u32, partner: u32, child: u32 },
    SyncedChangedServer { mob: u32, key: String, old: FMobValue, new: FMobValue },
    MobPose { mob: u32, type_id: String, frame: FMobPoseFrame },
    MobAnimEventClient { mob: u32, name: String },
    SyncedChangedClient { mob: u32, key: String, old: FMobValue, new: FMobValue },
}

/// Custom mob renderer hook stage (`before_pose`, `after_pose`, `after_vertices`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FMobDrawStage {
    BeforePose,
    AfterPose,
    AfterVertices,
}

/// LOD mode of the custom mob being rendered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FMobLodMode {
    Full,
    HalfRate,
    QuarterRate,
    LowBones,
    Static,
    Hidden,
}

/// Context of the active custom-mob draw stage hook, read with [`mob_draw_event`].
#[derive(Clone, Debug, PartialEq)]
pub struct FMobDrawEvent {
    pub entity: u32,
    pub type_id: String,
    pub stage: FMobDrawStage,
    pub lod: FMobLodMode,
    pub partial: f32,
    pub scale: f32,
    pub yaw: f32,
    pub roll: f32,
    pub bone_count: u32,
    pub vertex_count: u32,
}

// ---------------------------------------------------------------------------------------------------------------
// Server custom-mob services (S-route parity via engine view & command bus)
// ---------------------------------------------------------------------------------------------------------------

pub fn mob_info(entity: u32) -> Result<Option<FMobInfo>, String> {
    super::yoake2_custom_mobs_state::info(entity)
}

pub fn mob_get_synced(entity: u32, key: String) -> Result<Option<FMobValue>, String> {
    super::yoake2_custom_mobs_state::get_synced(entity, &key)
}

pub fn mob_set_synced(entity: u32, key: String, value: FMobValue) -> Result<(), String> {
    super::yoake2_custom_mobs_state::set_synced(entity, &key, &value)
}

pub fn mob_trigger_anim(entity: u32, controller: String, trigger: String, speed: f32) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::trigger_anim(entity, &controller, &trigger, speed)
}

pub fn mob_set_controller_state(entity: u32, controller: String, state: String) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::set_controller_state(entity, &controller, &state)
}

pub fn mob_set_controller_state_hold(entity: u32, controller: String, state: String, hold_ticks: Option<u32>) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::set_controller_state_hold(entity, &controller, &state, hold_ticks)
}

pub fn mob_release_controller_state(entity: u32, controller: String) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::release_controller_state(entity, &controller)
}

pub fn mob_navigate_to(entity: u32, position: FVec3, speed: f64) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::navigate_to(entity, position, speed)
}

pub fn mob_navigate_stop(entity: u32) -> Result<(), String> {
    super::yoake2_custom_mobs_state::navigate_stop(entity)
}

pub fn mob_look_at(entity: u32, position: FVec3, yaw_speed: f32, pitch_speed: f32) -> Result<(), String> {
    super::yoake2_custom_mobs_state::look_at(entity, position, yaw_speed, pitch_speed)
}

pub fn mob_jump(entity: u32) -> Result<(), String> {
    super::yoake2_custom_mobs_state::jump(entity)
}

pub fn mob_set_target(entity: u32, target: Option<u32>, player: Option<String>) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::set_target(entity, target, player.as_deref())
}

pub fn mob_attack(entity: u32, target: Option<u32>, player: Option<String>, damage: f32, knockback: f32) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::attack(entity, target, player.as_deref(), damage, knockback)
}

pub fn mob_attribute_get(entity: u32, attribute: String) -> Result<FAttributeValue, String> {
    super::yoake2_custom_mobs_state::attribute_get(entity, &attribute)
}

pub fn mob_attribute_set_base(entity: u32, attribute: String, value: f64) -> Result<(), String> {
    super::yoake2_custom_mobs_state::attribute_set_base(entity, &attribute, value)
}

pub fn mob_attribute_add_modifier(
    entity: u32,
    attribute: String,
    modifier: String,
    amount: f64,
    operation: FAttributeOperation,
) -> Result<(), String> {
    super::yoake2_custom_mobs_state::attribute_add_modifier(entity, &attribute, &modifier, amount, operation)
}

pub fn mob_attribute_remove_modifier(entity: u32, attribute: String, modifier: String) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::attribute_remove_modifier(entity, &attribute, &modifier)
}

pub fn mob_set_hitbox_enabled(entity: u32, name: String, enabled: bool) -> Result<(), String> {
    super::yoake2_custom_mobs_state::set_hitbox_enabled(entity, &name, enabled)
}

pub fn mob_data_get(entity: u32) -> Result<Vec<u8>, String> {
    super::yoake2_custom_mobs_state::data_get(entity)
}

pub fn mob_data_put(entity: u32, data: Vec<u8>) -> Result<(), String> {
    super::yoake2_custom_mobs_state::data_put(entity, data)
}

pub fn mob_despawn(entity: u32) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::despawn(entity)
}

pub fn mob_claim_control(entity: u32, claim: bool) -> Result<(), String> {
    super::yoake2_custom_mobs_state::claim_control(entity, claim)
}

pub fn mob_impulse(entity: u32, velocity: FVec3) -> Result<(), String> {
    super::yoake2_custom_mobs_state::impulse(entity, velocity)
}

pub fn mob_seat_set(entity: u32, seat: u32, rider: Option<u32>) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::seat_set(entity, seat, rider)
}

pub fn mob_memory_get(entity: u32, key: String) -> Result<Option<FMobMemoryValue>, String> {
    super::yoake2_custom_mobs_state::memory_get(entity, &key)
}

pub fn mob_memory_set(
    entity: u32,
    key: String,
    value: Option<FMobMemoryValue>,
    ttl_ticks: Option<u32>,
) -> Result<(), String> {
    super::yoake2_custom_mobs_state::memory_set(entity, &key, value.as_ref(), ttl_ticks)
}

/// Validates and queues a custom mob spawn to run at the start of the next server tick when `Game` is in scope.
pub fn mob_spawn_queued(
    dimension: String,
    type_id: String,
    position: FVec3,
    options: FMobSpawnOptions,
) -> Result<(), String> {
    super::yoake2_custom_mobs_state::queue_spawn(dimension, type_id, position, options)
}

// ---------------------------------------------------------------------------------------------------------------
// Client custom-mob helpers
// ---------------------------------------------------------------------------------------------------------------

pub fn mob_bone_index(type_id: String, name: String) -> Result<Option<u32>, String> {
    super::yoake2_custom_mobs_state::bone_index(&type_id, &name)
}

pub fn mob_locator(entity: u32, name: String) -> Option<FMobTransform> {
    super::yoake2_custom_mobs_state::locator(entity, &name)
}

pub fn mob_texture_create(width: u32, height: u32) -> Result<u32, String> {
    super::yoake2_custom_mobs_state::texture_create(width, height)
}

pub fn mob_texture_write(id: u32, origin: (u32, u32), size: (u32, u32), rgba: Vec<u8>) -> Result<(), String> {
    super::yoake2_custom_mobs_state::texture_write(id, origin, size, &rgba)
}

pub fn mob_texture_bind(entity: u32, slot: String, id: Option<u32>) -> Result<(), String> {
    super::yoake2_custom_mobs_state::texture_bind(entity, &slot, id)
}

pub fn mob_play_local_anim(entity: u32, name: String, speed: f32) -> Result<bool, String> {
    super::yoake2_custom_mobs_state::play_local_anim(entity, &name, speed)
}

// ---------------------------------------------------------------------------------------------------------------
// Script goal & lifecycle hook registration, event query, and typed replies
// ---------------------------------------------------------------------------------------------------------------

pub fn mob_goal_register(goal: String) -> Result<(), String> {
    super::yoake2_custom_mobs_state::register_goal(goal)
}

pub fn mob_hook_event() -> Option<FMobHookEvent> {
    super::yoake2_custom_mobs_state::hook_event()
}

pub fn mob_reply_bool(claim: bool) -> Result<(), String> {
    super::yoake2_custom_mobs_state::reply_bool(claim)
}

pub fn mob_reply_hurt(amount: f32, cancel: bool) -> Result<(), String> {
    super::yoake2_custom_mobs_state::reply_hurt(amount, cancel)
}

pub fn mob_reply_interact(reply: FInteractReply) -> Result<(), String> {
    super::yoake2_custom_mobs_state::reply_interact(reply)
}

pub fn mob_reply_save(data: Vec<u8>) -> Result<(), String> {
    super::yoake2_custom_mobs_state::reply_save(data)
}

pub fn mob_reply_pose(writes: Vec<FMobBoneWrite>) -> Result<(), String> {
    super::yoake2_custom_mobs_state::reply_pose(writes)
}

// ---------------------------------------------------------------------------------------------------------------
// Renderer stage hooks (before_pose, after_pose, after_vertices)
// ---------------------------------------------------------------------------------------------------------------

pub fn mob_draw_event() -> Option<FMobDrawEvent> {
    super::yoake2_custom_mobs_state::draw_event()
}

pub fn mob_draw_cancel() -> Result<(), String> {
    super::yoake2_custom_mobs_state::draw_cancel()
}

pub fn mob_pose_bone_count() -> Result<u32, String> {
    super::yoake2_custom_mobs_state::pose_bone_count()
}

pub fn mob_pose_bone_name(bone: u32) -> Result<Option<String>, String> {
    super::yoake2_custom_mobs_state::pose_bone_name(bone)
}

pub fn mob_pose_bone_index(name: String) -> Result<Option<u32>, String> {
    super::yoake2_custom_mobs_state::pose_bone_index(&name)
}

pub fn mob_pose_get(bone: u32) -> Result<FBoneTransform, String> {
    super::yoake2_custom_mobs_state::pose_get(bone)
}

pub fn mob_pose_get_all() -> Result<Vec<FBoneTransform>, String> {
    super::yoake2_custom_mobs_state::pose_get_all()
}

pub fn mob_pose_set(bone: u32, transform: FBoneTransform) -> Result<(), String> {
    super::yoake2_custom_mobs_state::pose_set(bone, transform)
}

pub fn mob_pose_write(write: FMobBoneWrite) -> Result<(), String> {
    super::yoake2_custom_mobs_state::pose_write(write)
}

#[cfg(feature = "yoake2-slice")]
include!("../yoake_glue/custom_mobs.rs");
