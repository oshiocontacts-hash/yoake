//! F4M-11: internal state, validation, script runtime adapter, and stage hook state for
//! `yoake2_custom_mobs.rs` (kept out of the bindgen target so only the guest API is bound).
//! Reuses `horizon_client::mobs::custom::{script, textures}` directly rather than duplicating
//! command queuing, view publishing, or atlas texture management.

use std::cell::RefCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use horizon_client::mobs::custom::{
    ai::memory::{EntityMemory, MemoryKind, MemoryValue},
    script::{self, Command, MAX_DATA_BYTES, MobScriptValue, MobView, ScriptVec3},
    textures,
};
use horizon_mods::custom_mob::limits;

pub(super) use super::yoake2_custom_mobs::{
    FAttributeOperation, FAttributeValue, FBoneTransform, FInteractReply, FMobBoneWrite, FMobDrawEvent,
    FMobDrawStage, FMobHookEvent, FMobInfo, FMobLodMode, FMobMemoryValue, FMobPoseFrame, FMobSpawnOptions,
    FMobTransform, FMobValue, FQuat, FVec3, FVec3f,
};

#[cfg(feature = "yoake2-slice")]
mod runtime;
#[cfg(feature = "yoake2-slice")]
mod stage;

#[cfg(feature = "yoake2-slice")]
pub(crate) use runtime::{
    ensure_world_script_session, f_client_runtime, f_world_runtime, goal_registered, hook_event,
    register_goal, reply_bool, reply_hurt, reply_interact, reply_pose, reply_save, reset_goals,
};
#[cfg(feature = "yoake2-slice")]
pub(crate) use stage::{
    after_pose_stage, after_vertices_stage, before_pose_stage, draw_cancel, draw_event, pose_bone_count,
    pose_bone_index, pose_bone_name, pose_get, pose_get_all, pose_set, pose_write,
};

const MAX_NAME: usize = 128;
const MAX_QUEUED_SPAWNS: usize = 256;
const F_TEXTURE_OWNER: &str = "yoake-f-client";
static F_TEXTURE_GENERATION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, PartialEq)]
pub(super) struct QueuedSpawn {
    pub dimension: String,
    pub type_id: String,
    pub position: [f64; 3],
    pub options: FMobSpawnOptions,
}

thread_local! {
    static PENDING_SPAWNS: RefCell<Vec<QueuedSpawn>> = const { RefCell::new(Vec::new()) };
}

fn validate_name(n: &str, what: &str) -> Result<(), String> {
    if (n.is_empty() && what != "controller") || n.len() > MAX_NAME || n.chars().any(char::is_control) {
        Err(format!("not a valid {what}"))
    } else {
        Ok(())
    }
}

fn finite3(v: FVec3) -> Result<[f64; 3], String> {
    if [v.x, v.y, v.z].iter().all(|n| n.is_finite()) {
        Ok([v.x, v.y, v.z])
    } else {
        Err("numbers must be finite".into())
    }
}

fn parse_uuid(text: &str) -> Result<[i32; 4], String> {
    let hex: String = text.chars().filter(|&c| c != '-').collect();
    if hex.len() != 32 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(format!("not a valid UUID: {text:.40}"));
    }
    let raw = u128::from_str_radix(&hex, 16).map_err(|_| format!("not a valid UUID: {text:.40}"))?;
    Ok([
        (raw >> 96) as i32,
        (raw >> 64) as i32,
        (raw >> 32) as i32,
        raw as i32,
    ])
}

fn view(entity: u32) -> Result<Arc<MobView>, String> {
    script::view(entity).ok_or_else(|| format!("no custom mob with handle {entity}"))
}

pub(super) fn to_f_value(v: &MobScriptValue) -> FMobValue {
    match v {
        MobScriptValue::Bool(b) => FMobValue::Bool(*b),
        MobScriptValue::Int(n) => FMobValue::Int(*n),
        MobScriptValue::Float(n) => FMobValue::Float(*n),
        MobScriptValue::Text(s) => FMobValue::Text(s.clone()),
        MobScriptValue::Choice(s) => FMobValue::Choice(s.clone()),
        MobScriptValue::Vec3(p) => FMobValue::Vec3(FVec3f { x: p.x, y: p.y, z: p.z }),
    }
}

fn from_f_value(v: &FMobValue) -> Result<MobScriptValue, String> {
    Ok(match v {
        FMobValue::Bool(b) => MobScriptValue::Bool(*b),
        FMobValue::Int(n) => MobScriptValue::Int(*n),
        FMobValue::Float(n) if n.is_finite() => MobScriptValue::Float(*n),
        FMobValue::Float(_) => return Err("numbers must be finite".into()),
        FMobValue::Text(s) => {
            if s.len() > 1024 {
                return Err("a string value is at most 1024 bytes".into());
            }
            MobScriptValue::Text(s.clone())
        }
        FMobValue::Choice(s) => MobScriptValue::Choice(s.clone()),
        FMobValue::Vec3(p) => {
            if ![p.x, p.y, p.z].iter().all(|n| n.is_finite()) {
                return Err("numbers must be finite".into());
            }
            MobScriptValue::Vec3(ScriptVec3 { x: p.x, y: p.y, z: p.z })
        }
    })
}

fn to_f_memory_value(v: &MemoryValue) -> FMobMemoryValue {
    match v {
        MemoryValue::Bool(b) => FMobMemoryValue::Bool(*b),
        MemoryValue::I32(n) => FMobMemoryValue::I32(*n),
        MemoryValue::F32(f) => FMobMemoryValue::F32(*f),
        MemoryValue::Str(s) => FMobMemoryValue::String(s.clone()),
        MemoryValue::Vec3(p) => FMobMemoryValue::Vec3(FVec3f { x: p[0], y: p[1], z: p[2] }),
        MemoryValue::BlockPos(p) => FMobMemoryValue::BlockPos(p[0], p[1], p[2]),
        MemoryValue::Entity(EntityMemory::Mob(id)) => FMobMemoryValue::EntityMob(*id),
        MemoryValue::Entity(EntityMemory::Player(uuid)) => FMobMemoryValue::EntityPlayer(script::uuid_text(*uuid)),
    }
}

fn from_f_memory_value(expected: MemoryKind, v: &FMobMemoryValue) -> Result<MemoryValue, String> {
    let matches_kind = matches!(
        (expected, v),
        (MemoryKind::Bool, FMobMemoryValue::Bool(_))
            | (MemoryKind::I32, FMobMemoryValue::I32(_))
            | (MemoryKind::F32, FMobMemoryValue::F32(_))
            | (MemoryKind::Str, FMobMemoryValue::String(_))
            | (MemoryKind::Vec3, FMobMemoryValue::Vec3(_))
            | (MemoryKind::BlockPos, FMobMemoryValue::BlockPos(_, _, _))
            | (MemoryKind::Entity, FMobMemoryValue::EntityMob(_) | FMobMemoryValue::EntityPlayer(_))
    );
    if !matches_kind {
        return Err(format!("memory expects {}", expected.as_str()));
    }
    Ok(match v {
        FMobMemoryValue::Bool(b) => MemoryValue::Bool(*b),
        FMobMemoryValue::I32(n) => MemoryValue::I32(*n),
        FMobMemoryValue::F32(f) if f.is_finite() => MemoryValue::F32(*f),
        FMobMemoryValue::F32(_) => return Err("numbers must be finite".into()),
        FMobMemoryValue::String(s) => {
            if s.len() > limits::MAX_MEMORY_STRING {
                return Err(format!("a memory string is at most {} bytes", limits::MAX_MEMORY_STRING));
            }
            MemoryValue::Str(s.clone())
        }
        FMobMemoryValue::Vec3(p) => {
            if ![p.x, p.y, p.z].iter().all(|n| n.is_finite()) {
                return Err("numbers must be finite".into());
            }
            MemoryValue::Vec3([p.x, p.y, p.z])
        }
        FMobMemoryValue::BlockPos(x, y, z) => MemoryValue::BlockPos([*x, *y, *z]),
        FMobMemoryValue::EntityMob(id) => MemoryValue::Entity(EntityMemory::Mob(*id)),
        FMobMemoryValue::EntityPlayer(uuid) => MemoryValue::Entity(EntityMemory::Player(parse_uuid(uuid)?)),
    })
}

pub(super) fn info(entity: u32) -> Result<Option<FMobInfo>, String> {
    Ok(script::view(entity).filter(|v| v.alive).map(|v| FMobInfo {
        handle: v.id,
        type_id: v.type_id.clone(),
        dimension: v.dimension.clone(),
        position: FVec3 { x: v.position[0], y: v.position[1], z: v.position[2] },
        velocity: FVec3 { x: v.velocity[0], y: v.velocity[1], z: v.velocity[2] },
        yaw: v.yaw,
        pitch: v.pitch,
        health: v.health,
        max_health: v.max_health,
        alive: v.alive,
        on_ground: v.on_ground,
        target: v.target_mob,
        target_player: v.target_player_uuid.map(script::uuid_text),
        target_position: v.target_position.map(|p| FVec3 { x: p[0], y: p[1], z: p[2] }),
        target_visible: v.target_visible,
        owner: v.owner.map(script::uuid_text),
        sitting: v.sitting,
        baby: v.baby,
        age: v.age,
        controlled: v.controlled,
    }))
}

pub(super) fn get_synced(entity: u32, key: &str) -> Result<Option<FMobValue>, String> {
    validate_name(key, "key")?;
    Ok(view(entity)?.synced.iter().find(|(k, _)| k == key).map(|(_, v)| to_f_value(v)))
}

pub(super) fn set_synced(entity: u32, key: &str, value: &FMobValue) -> Result<(), String> {
    validate_name(key, "key")?;
    let v = view(entity)?;
    if !v.synced.iter().any(|(k, _)| k == key) {
        return Err(format!("{} has no tracked value {key:.64}", v.type_id));
    }
    script::push(entity, Command::SetSynced { key: key.to_owned(), value: from_f_value(value)? });
    Ok(())
}

pub(super) fn trigger_anim(entity: u32, controller: &str, trigger: &str, speed: f32) -> Result<bool, String> {
    validate_name(controller, "controller")?;
    validate_name(trigger, "trigger name")?;
    if !speed.is_finite() {
        return Err("numbers must be finite".into());
    }
    view(entity)?;
    script::push(entity, Command::TriggerAnim { name: trigger.to_owned(), speed });
    Ok(true)
}

pub(super) fn set_controller_state(entity: u32, controller: &str, state: &str) -> Result<bool, String> {
    validate_name(controller, "controller")?;
    validate_name(state, "state")?;
    view(entity)?;
    script::push(entity, Command::SetControllerState { controller: controller.to_owned(), state: state.to_owned() });
    Ok(true)
}

pub(super) fn set_controller_state_hold(
    entity: u32,
    controller: &str,
    state: &str,
    hold_ticks: Option<u32>,
) -> Result<bool, String> {
    validate_name(controller, "controller")?;
    validate_name(state, "state")?;
    view(entity)?;
    script::push(
        entity,
        Command::HoldControllerState { controller: controller.to_owned(), state: state.to_owned(), hold_ticks },
    );
    Ok(true)
}

pub(super) fn release_controller_state(entity: u32, controller: &str) -> Result<bool, String> {
    validate_name(controller, "controller")?;
    view(entity)?;
    script::push(entity, Command::ReleaseControllerState { controller: controller.to_owned() });
    Ok(true)
}

pub(super) fn navigate_to(entity: u32, position: FVec3, speed: f64) -> Result<bool, String> {
    let position = finite3(position)?;
    if !speed.is_finite() || speed <= 0.0 {
        return Err("speed is a positive number".into());
    }
    view(entity)?;
    script::push(entity, Command::NavigateTo { position, speed });
    Ok(true)
}

pub(super) fn navigate_stop(entity: u32) -> Result<(), String> {
    view(entity)?;
    script::push(entity, Command::NavigateStop);
    Ok(())
}

pub(super) fn look_at(entity: u32, position: FVec3, yaw_speed: f32, pitch_speed: f32) -> Result<(), String> {
    let position = finite3(position)?;
    if !pitch_speed.is_finite() || !yaw_speed.is_finite() {
        return Err("numbers must be finite".into());
    }
    view(entity)?;
    script::push(entity, Command::LookAt { position, yaw_speed, pitch_speed });
    Ok(())
}

pub(super) fn jump(entity: u32) -> Result<(), String> {
    view(entity)?;
    script::push(entity, Command::Jump);
    Ok(())
}

pub(super) fn set_target(entity: u32, target: Option<u32>, player: Option<&str>) -> Result<bool, String> {
    view(entity)?;
    if player.is_some_and(|p| !p.is_empty()) {
        script::push(entity, Command::SetTarget { mob: None, player: true });
    } else {
        script::push(entity, Command::SetTarget { mob: target, player: false });
    }
    Ok(true)
}

pub(super) fn attack(
    entity: u32,
    target: Option<u32>,
    player: Option<&str>,
    damage: f32,
    knockback: f32,
) -> Result<bool, String> {
    if !damage.is_finite()
        || !knockback.is_finite()
        || !(0.0..=1000.0).contains(&damage)
        || !(0.0..=10.0).contains(&knockback)
    {
        return Err("damage is 0 to 1000 and knockback 0 to 10".into());
    }
    view(entity)?;
    let is_player = player.is_some_and(|p| !p.is_empty());
    if !is_player && target.is_none() {
        return Err("name a target mob or a player".into());
    }
    script::push(entity, Command::Attack { mob: target, player: is_player, damage, knockback });
    Ok(true)
}

pub(super) fn attribute_get(entity: u32, attribute: &str) -> Result<FAttributeValue, String> {
    validate_name(attribute, "attribute")?;
    let v = view(entity)?;
    v.attributes
        .iter()
        .find(|(id, _, _)| id == attribute)
        .map(|(_, base, value)| FAttributeValue { base: *base, value: *value })
        .ok_or_else(|| format!("{} has no attribute {attribute:.64}", v.type_id))
}

pub(super) fn attribute_set_base(entity: u32, attribute: &str, value: f64) -> Result<(), String> {
    validate_name(attribute, "attribute")?;
    if !value.is_finite() {
        return Err("numbers must be finite".into());
    }
    view(entity)?;
    script::push(entity, Command::SetAttributeBase { attribute: attribute.to_owned(), value });
    Ok(())
}

pub(super) fn attribute_add_modifier(
    entity: u32,
    attribute: &str,
    modifier: &str,
    amount: f64,
    operation: FAttributeOperation,
) -> Result<(), String> {
    validate_name(attribute, "attribute")?;
    validate_name(modifier, "modifier id")?;
    if !amount.is_finite() {
        return Err("numbers must be finite".into());
    }
    view(entity)?;
    let operation = match operation {
        FAttributeOperation::AddValue => "add_value",
        FAttributeOperation::AddMultipliedBase => "add_multiplied_base",
        FAttributeOperation::AddMultipliedTotal => "add_multiplied_total",
    };
    script::push(
        entity,
        Command::AddModifier {
            attribute: attribute.to_owned(),
            modifier: modifier.to_owned(),
            amount,
            operation: operation.to_owned(),
        },
    );
    Ok(())
}

pub(super) fn attribute_remove_modifier(entity: u32, attribute: &str, modifier: &str) -> Result<bool, String> {
    validate_name(attribute, "attribute")?;
    validate_name(modifier, "modifier id")?;
    view(entity)?;
    script::push(
        entity,
        Command::RemoveModifier { attribute: attribute.to_owned(), modifier: modifier.to_owned() },
    );
    Ok(true)
}

pub(super) fn set_hitbox_enabled(entity: u32, name: &str, enabled: bool) -> Result<(), String> {
    validate_name(name, "hitbox name")?;
    view(entity)?;
    script::push(entity, Command::SetHitbox { name: name.to_owned(), enabled });
    Ok(())
}

pub(super) fn data_get(entity: u32) -> Result<Vec<u8>, String> {
    Ok(view(entity)?.data.clone())
}

pub(super) fn data_put(entity: u32, data: Vec<u8>) -> Result<(), String> {
    if data.len() > MAX_DATA_BYTES {
        return Err("script data is at most 64 KiB".into());
    }
    view(entity)?;
    script::push(entity, Command::PutData(data));
    Ok(())
}

pub(super) fn despawn(entity: u32) -> Result<bool, String> {
    view(entity)?;
    script::push(entity, Command::Despawn);
    Ok(true)
}

pub(super) fn claim_control(entity: u32, claim: bool) -> Result<(), String> {
    view(entity)?;
    script::push(entity, Command::Claim(claim));
    Ok(())
}

pub(super) fn impulse(entity: u32, velocity: FVec3) -> Result<(), String> {
    let v = finite3(velocity)?;
    view(entity)?;
    script::push(entity, Command::Impulse(v));
    Ok(())
}

pub(super) fn seat_set(entity: u32, seat: u32, rider: Option<u32>) -> Result<bool, String> {
    let v = view(entity)?;
    if seat >= v.seats {
        return Err(format!("{} has {} seats, not a seat {seat}", v.type_id, v.seats));
    }
    if rider == Some(entity) {
        return Err("a mob does not ride itself".into());
    }
    if !v.alive {
        return Ok(false);
    }
    script::push(entity, Command::SeatSet { seat, rider });
    Ok(true)
}

pub(super) fn memory_get(entity: u32, key: &str) -> Result<Option<FMobMemoryValue>, String> {
    validate_name(key, "key")?;
    let v = view(entity)?;
    Ok(v.memories
        .iter()
        .find(|(k, _, _)| k == key)
        .and_then(|(_, _, val)| val.as_ref().map(to_f_memory_value)))
}

pub(super) fn memory_set(
    entity: u32,
    key: &str,
    value: Option<&FMobMemoryValue>,
    ttl_ticks: Option<u32>,
) -> Result<(), String> {
    validate_name(key, "key")?;
    if let Some(ttl) = ttl_ticks
        && ttl > limits::MAX_MEMORY_TTL
    {
        return Err(format!("ttl is at most {} ticks", limits::MAX_MEMORY_TTL));
    }
    let v = view(entity)?;
    let Some((_, kind, _)) = v.memories.iter().find(|(k, _, _)| k == key) else {
        return Err(format!("{} has no memory {key:.64}", v.type_id));
    };
    let converted = match value {
        Some(val) => Some(from_f_memory_value(*kind, val)?),
        None => None,
    };
    script::push(entity, Command::SetMemory { key: key.to_owned(), value: converted, ttl_ticks });
    Ok(())
}

pub(super) fn queue_spawn(
    dimension: String,
    type_id: String,
    position: FVec3,
    options: FMobSpawnOptions,
) -> Result<(), String> {
    let position = finite3(position)?;
    if !dimension.contains(':') || dimension.len() > MAX_NAME || dimension.chars().any(char::is_control) {
        return Err("a dimension is a namespace:path id".into());
    }
    if !type_id.contains(':') || type_id.len() > MAX_NAME || type_id.chars().any(char::is_control) {
        return Err("a mob type is a namespace:path id".into());
    }
    if !options.yaw.is_finite() {
        return Err("numbers must be finite".into());
    }
    PENDING_SPAWNS.with(|q| {
        let mut q = q.borrow_mut();
        if q.len() >= MAX_QUEUED_SPAWNS {
            return Err("too many queued custom-mob spawns for this tick".into());
        }
        q.push(QueuedSpawn { dimension, type_id, position, options });
        Ok(())
    })
}

pub(crate) fn take_queued_spawns() -> Vec<QueuedSpawn> {
    PENDING_SPAWNS.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

pub(super) fn bone_index(type_id: &str, name: &str) -> Result<Option<u32>, String> {
    if name.len() > MAX_NAME || type_id.len() > MAX_NAME {
        return Err("names are at most 128 characters".into());
    }
    Ok(script::bone_index(type_id, name))
}

pub(super) fn locator(entity: u32, name: &str) -> Option<FMobTransform> {
    script::locator(entity, name).map(|p| FMobTransform {
        position: FVec3 { x: p.position[0], y: p.position[1], z: p.position[2] },
        yaw: p.yaw,
        pitch: p.pitch,
    })
}

pub(super) fn texture_create(width: u32, height: u32) -> Result<u32, String> {
    textures::create(F_TEXTURE_OWNER, F_TEXTURE_GENERATION.load(Ordering::Relaxed), width, height)
}

pub(super) fn texture_write(id: u32, origin: (u32, u32), size: (u32, u32), rgba: &[u8]) -> Result<(), String> {
    textures::write(F_TEXTURE_OWNER, id, [origin.0, origin.1], [size.0, size.1], rgba)
}

pub(super) fn texture_bind(entity: u32, slot: &str, id: Option<u32>) -> Result<(), String> {
    textures::bind(F_TEXTURE_OWNER, entity, slot, id)
}

pub(super) fn play_local_anim(entity: u32, name: &str, speed: f32) -> Result<bool, String> {
    validate_name(name, "animation name")?;
    if !speed.is_finite() || speed <= 0.0 {
        return Err("speed must be a finite positive number".into());
    }
    script::queue_local_anim(entity, name, speed)?;
    Ok(true)
}

/// Client session end: frees all runtime custom-mob textures owned by F modules and advances the generation.
pub(crate) fn release_f_textures() {
    let generation = F_TEXTURE_GENERATION.fetch_add(1, Ordering::Relaxed);
    textures::release_instance(F_TEXTURE_OWNER, generation);
}

#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn register_goal(_goal: String) -> Result<(), String> {
    Err("custom mob goal registration requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn hook_event() -> Option<FMobHookEvent> {
    None
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn reply_bool(_claim: bool) -> Result<(), String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn reply_hurt(_amount: f32, _cancel: bool) -> Result<(), String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn reply_interact(_reply: FInteractReply) -> Result<(), String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn reply_save(_data: Vec<u8>) -> Result<(), String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn reply_pose(_writes: Vec<FMobBoneWrite>) -> Result<(), String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn draw_event() -> Option<FMobDrawEvent> {
    None
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn draw_cancel() -> Result<(), String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn pose_bone_count() -> Result<u32, String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn pose_bone_name(_bone: u32) -> Result<Option<String>, String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn pose_bone_index(_name: &str) -> Result<Option<u32>, String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn pose_get(_bone: u32) -> Result<FBoneTransform, String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn pose_get_all() -> Result<Vec<FBoneTransform>, String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn pose_set(_bone: u32, _transform: FBoneTransform) -> Result<(), String> {
    Err("requires feature yoake2-slice".into())
}
#[cfg(not(feature = "yoake2-slice"))]
pub(super) fn pose_write(_write: FMobBoneWrite) -> Result<(), String> {
    Err("requires feature yoake2-slice".into())
}

#[cfg(test)]
mod tests;
