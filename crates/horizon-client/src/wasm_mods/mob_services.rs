//! The `mobs-server` services of `server-full` (ids 260 to 281, permission `service.mobs`),
//! F4M-10. They read a published view of the mob (`script::view`) and write commands to the bus
//! (`script::push`); `script::call` applies the commands of the mob whose hook is running as soon
//! as the guest returns, and any other mob's at its next tick. Every argument is checked here.
use super::full::server_service;
use crate::mobs::custom::script::{self, Command, MAX_DATA_BYTES, MobScriptValue, MobView, ScriptVec3};
use horizon_world::nbt::{Compound, Tag};
use std::sync::Arc;
use yoake_wasm::server_types as st;

static GRANTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Records that package `id` declared `service.mobs`.
pub(super) fn grant(id: &str) {
    let mut g = GRANTED.lock().unwrap_or_else(|e| e.into_inner());
    if !g.iter().any(|g| g == id) {
        g.push(id.to_owned());
    }
}

/// Whether package `id` declared `service.mobs`.
pub(super) fn granted(id: &str) -> bool {
    GRANTED.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|g| g == id)
}

/// Most characters of a name (key, controller, hitbox, modifier id...).
const MAX_NAME: usize = 128;

pub(super) fn gate(granted: bool) -> Result<(), String> {
    if granted { Ok(()) } else { Err("the mob services need the permission service.mobs (declare it in [wasm] permissions)".into()) }
}

fn name(n: &str, what: &str) -> Result<(), String> {
    if n.is_empty() && what != "controller" || n.len() > MAX_NAME || n.chars().any(char::is_control) {
        Err(format!("not a valid {what}"))
    } else {
        Ok(())
    }
}

fn finite3(v: &st::Vec3) -> Result<[f64; 3], String> {
    if [v.x, v.y, v.z].iter().all(|n| n.is_finite()) { Ok([v.x, v.y, v.z]) } else { Err("numbers must be finite".into()) }
}

fn view(entity: u32) -> Result<Arc<MobView>, String> {
    script::view(entity).ok_or_else(|| format!("no custom mob with handle {entity}"))
}

fn to_value(v: &MobScriptValue) -> st::MobValue {
    let zero = st::Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    let base = st::MobValue {
        kind: st::MobValueKind::Bool,
        bool_value: false,
        int_value: 0,
        float_value: 0.0,
        string_value: String::new(),
        vec3_value: zero,
    };
    match v {
        MobScriptValue::Bool(b) => st::MobValue { bool_value: *b, ..base },
        MobScriptValue::Int(n) => st::MobValue { kind: st::MobValueKind::Int, int_value: *n, ..base },
        MobScriptValue::Float(n) => st::MobValue { kind: st::MobValueKind::Float, float_value: *n, ..base },
        MobScriptValue::Text(s) => st::MobValue { kind: st::MobValueKind::String, string_value: s.clone(), ..base },
        MobScriptValue::Choice(s) => st::MobValue { kind: st::MobValueKind::Choice, string_value: s.clone(), ..base },
        MobScriptValue::Vec3(p) => st::MobValue {
            kind: st::MobValueKind::Vec3,
            vec3_value: st::Vec3 { x: f64::from(p.x), y: f64::from(p.y), z: f64::from(p.z) },
            ..base
        },
    }
}

fn from_value(v: &st::MobValue) -> Result<MobScriptValue, String> {
    Ok(match v.kind {
        st::MobValueKind::Bool => MobScriptValue::Bool(v.bool_value),
        st::MobValueKind::Int => MobScriptValue::Int(v.int_value),
        st::MobValueKind::Float if v.float_value.is_finite() => MobScriptValue::Float(v.float_value),
        st::MobValueKind::Float => return Err("numbers must be finite".into()),
        st::MobValueKind::String => {
            if v.string_value.len() > 1024 {
                return Err("a string value is at most 1024 bytes".into());
            }
            MobScriptValue::Text(v.string_value.clone())
        }
        st::MobValueKind::Choice => MobScriptValue::Choice(v.string_value.clone()),
        st::MobValueKind::Vec3 => {
            let p = finite3(&v.vec3_value)?;
            MobScriptValue::Vec3(ScriptVec3 { x: p[0] as f32, y: p[1] as f32, z: p[2] as f32 })
        }
    })
}

pub(super) fn info(entity: u32) -> Result<Option<st::MobInfo>, String> {
    Ok(script::view(entity).filter(|v| v.alive).map(|v| st::MobInfo {
        handle: v.id,
        type_id: v.type_id.clone(),
        dimension: v.dimension.clone(),
        position: st::Vec3 { x: v.position[0], y: v.position[1], z: v.position[2] },
        velocity: st::Vec3 { x: v.velocity[0], y: v.velocity[1], z: v.velocity[2] },
        yaw: v.yaw,
        pitch: v.pitch,
        health: v.health,
        max_health: v.max_health,
        alive: v.alive,
        on_ground: v.on_ground,
        target: v.target_mob,
        target_player: None,
        target_position: v.target_position.map(|p| st::Vec3 { x: p[0], y: p[1], z: p[2] }),
        target_visible: v.target_visible,
        owner: v.owner.map(script::uuid_text),
        sitting: v.sitting,
        baby: v.baby,
        age: v.age,
        controlled: v.controlled,
    }))
}

pub(super) fn set_synced(entity: u32, key: &str, value: &st::MobValue) -> Result<(), String> {
    name(key, "key")?;
    let v = view(entity)?;
    if !v.synced.iter().any(|(k, _)| k == key) {
        return Err(format!("{} has no tracked value {key:.64}", v.type_id));
    }
    script::push(entity, Command::SetSynced { key: key.to_owned(), value: from_value(value)? });
    Ok(())
}

pub(super) fn get_synced(entity: u32, key: &str) -> Result<Option<st::MobValue>, String> {
    name(key, "key")?;
    Ok(view(entity)?.synced.iter().find(|(k, _)| k == key).map(|(_, v)| to_value(v)))
}

pub(super) fn trigger_anim(controller: &str, entity: u32, trigger: &str, speed: f32) -> Result<bool, String> {
    name(controller, "controller")?;
    name(trigger, "trigger name")?;
    if !speed.is_finite() {
        return Err("numbers must be finite".into());
    }
    view(entity)?;
    script::push(entity, Command::TriggerAnim { name: trigger.to_owned(), speed });
    Ok(true)
}

pub(super) fn set_controller_state(controller: &str, entity: u32, state: &str) -> Result<bool, String> {
    name(controller, "controller")?;
    name(state, "state")?;
    view(entity)?;
    script::push(entity, Command::SetControllerState { controller: controller.to_owned(), state: state.to_owned() });
    Ok(true)
}

pub(super) fn set_controller_state_hold(controller: &str, entity: u32, state: &str, hold_ticks: Option<u32>) -> Result<bool, String> {
    name(controller, "controller")?;
    name(state, "state")?;
    view(entity)?;
    script::push(
        entity,
        Command::HoldControllerState { controller: controller.to_owned(), state: state.to_owned(), hold_ticks },
    );
    Ok(true)
}

pub(super) fn release_controller_state(controller: &str, entity: u32) -> Result<bool, String> {
    name(controller, "controller")?;
    view(entity)?;
    script::push(entity, Command::ReleaseControllerState { controller: controller.to_owned() });
    Ok(true)
}

pub(super) fn navigate_to(entity: u32, position: &st::Vec3, speed: f64) -> Result<bool, String> {
    let position = finite3(position)?;
    if !speed.is_finite() || speed <= 0.0 {
        return Err("speed is a positive number".into());
    }
    view(entity)?;
    script::push(entity, Command::NavigateTo { position, speed });
    Ok(true)
}

pub(super) fn simple(entity: u32, command: Command) -> Result<(), String> {
    view(entity)?;
    script::push(entity, command);
    Ok(())
}

pub(super) fn look_at(entity: u32, pitch_speed: f32, position: &st::Vec3, yaw_speed: f32) -> Result<(), String> {
    let position = finite3(position)?;
    if !pitch_speed.is_finite() || !yaw_speed.is_finite() {
        return Err("numbers must be finite".into());
    }
    simple(entity, Command::LookAt { position, yaw_speed, pitch_speed })
}

pub(super) fn set_target(entity: u32, player: Option<&str>, target: Option<u32>) -> Result<bool, String> {
    view(entity)?;
    if player.is_some_and(|p| !p.is_empty()) {
        // The only player a mob can be aimed at is the one the world has; the engine resolves it.
        script::push(entity, Command::SetTarget { mob: None, player: true });
    } else {
        script::push(entity, Command::SetTarget { mob: target, player: false });
    }
    Ok(true)
}

pub(super) fn attack(damage: f32, entity: u32, knockback: f32, player: Option<&str>, target: Option<u32>) -> Result<bool, String> {
    if !damage.is_finite() || !knockback.is_finite() || !(0.0..=1000.0).contains(&damage) || !(0.0..=10.0).contains(&knockback) {
        return Err("damage is 0 to 1000 and knockback 0 to 10".into());
    }
    view(entity)?;
    let player = player.is_some_and(|p| !p.is_empty());
    if !player && target.is_none() {
        return Err("name a target mob or a player".into());
    }
    script::push(entity, Command::Attack { mob: target, player, damage, knockback });
    Ok(true)
}

pub(super) fn attribute_get(attribute: &str, entity: u32) -> Result<st::AttributeValue, String> {
    name(attribute, "attribute")?;
    let v = view(entity)?;
    v.attributes
        .iter()
        .find(|(id, _, _)| id == attribute)
        .map(|(_, base, value)| st::AttributeValue { base: *base, value: *value })
        .ok_or_else(|| format!("{} has no attribute {attribute:.64}", v.type_id))
}

pub(super) fn attribute_set_base(attribute: &str, entity: u32, value: f64) -> Result<(), String> {
    name(attribute, "attribute")?;
    if !value.is_finite() {
        return Err("numbers must be finite".into());
    }
    simple(entity, Command::SetAttributeBase { attribute: attribute.to_owned(), value })
}

pub(super) fn attribute_add_modifier(amount: f64, attribute: &str, entity: u32, modifier: &str, operation: st::AttributeOperation) -> Result<(), String> {
    name(attribute, "attribute")?;
    name(modifier, "modifier id")?;
    if !amount.is_finite() {
        return Err("numbers must be finite".into());
    }
    let operation = match operation {
        st::AttributeOperation::AddValue => "add_value",
        st::AttributeOperation::AddMultipliedBase => "add_multiplied_base",
        st::AttributeOperation::AddMultipliedTotal => "add_multiplied_total",
    };
    simple(entity, Command::AddModifier { attribute: attribute.to_owned(), modifier: modifier.to_owned(), amount, operation: operation.to_owned() })
}

pub(super) fn attribute_remove_modifier(attribute: &str, entity: u32, modifier: &str) -> Result<bool, String> {
    name(attribute, "attribute")?;
    name(modifier, "modifier id")?;
    simple(entity, Command::RemoveModifier { attribute: attribute.to_owned(), modifier: modifier.to_owned() })?;
    Ok(true)
}

pub(super) fn set_hitbox_enabled(enabled: bool, entity: u32, part: &str) -> Result<(), String> {
    name(part, "hitbox name")?;
    simple(entity, Command::SetHitbox { name: part.to_owned(), enabled })
}

pub(super) fn data_get(entity: u32) -> Result<Vec<u8>, String> {
    Ok(view(entity)?.data.clone())
}

pub(super) fn data_put(data: Vec<u8>, entity: u32) -> Result<(), String> {
    if data.len() > MAX_DATA_BYTES {
        return Err("script data is at most 64 KiB".into());
    }
    simple(entity, Command::PutData(data))
}

pub(super) fn impulse(entity: u32, velocity: &st::Vec3) -> Result<(), String> {
    let v = finite3(velocity)?;
    simple(entity, Command::Impulse(v))
}

pub(super) fn spawn(dimension: &str, options: &st::MobSpawnOptions, position: &st::Vec3, type_id: &str) -> Result<u32, String> {
    let at = finite3(position)?;
    if !type_id.contains(':') || type_id.len() > MAX_NAME {
        return Err("a mob type is a namespace:path id".into());
    }
    if !options.yaw.is_finite() {
        return Err("numbers must be finite".into());
    }
    let mut nbt = Compound::new();
    nbt.insert("Rotation".into(), Tag::List(vec![Tag::Float(options.yaw.to_bits()), Tag::Float(0.0f32.to_bits())]));
    if options.baby {
        nbt.insert("Age".into(), Tag::Int(-24000));
    }
    if options.persistent {
        nbt.insert("PersistenceRequired".into(), Tag::Byte(1));
    }
    server_service(|host| {
        host.summon(dimension, type_id, at, nbt)
            .ok_or_else(|| "the mob could not be spawned here (unloaded chunk, another dimension, an unknown type, or the world is busy with a mob tick)".to_owned())
    })
}
