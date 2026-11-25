//! Typed F-route custom-mob helpers over `stubs::custom_mobs` (F4M-11).
//!
//! Server and client F modules register synthetic hook IDs (`hooks::MOB_GOAL_*`, `hooks::MOB_TICK`,
//! `hooks::CUSTOM_MOB_*`, etc.) through `Registrar::hook` in `yoake_init`, inspect active events via
//! [`hook_event`] / [`draw_event`], operate on mobs via [`CustomMob`], and answer via [`reply_bool`],
//! [`reply_hurt`], [`reply_interact`], [`reply_save`], [`reply_pose`], or [`draw_cancel`].

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

pub use crate::stubs::custom_mobs::types::*;
use crate::stubs::custom_mobs as raw;
use crate::{Error, RegisterError, Registrar, hooks};

/// Error from a custom-mob engine service call: either transport/wire failure or service validation refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallError {
    Transport(Error),
    Service(String),
}

impl fmt::Display for CallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CallError::Transport(e) => write!(f, "transport error: {e:?}"),
            CallError::Service(msg) => f.write_str(msg),
        }
    }
}

fn flatten<T>(res: Result<Result<T, String>, Error>) -> Result<T, CallError> {
    match res {
        Ok(Ok(val)) => Ok(val),
        Ok(Err(msg)) => Err(CallError::Service(msg)),
        Err(err) => Err(CallError::Transport(err)),
    }
}

/// Typed handle to a custom mob entity (`u32` engine entity id).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CustomMob(pub u32);

impl CustomMob {
    pub const fn new(entity_id: u32) -> Self {
        Self(entity_id)
    }

    pub const fn id(self) -> u32 {
        self.0
    }

    pub fn info(self) -> Result<Option<FMobInfo>, CallError> {
        flatten(raw::custom_mobs_mob_info(self.0))
    }

    pub fn synced(self, key: String) -> Result<Option<FMobValue>, CallError> {
        flatten(raw::custom_mobs_mob_get_synced(self.0, key))
    }

    pub fn set_synced(self, key: String, value: FMobValue) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_set_synced(self.0, key, value))
    }

    pub fn trigger_anim(self, controller: String, trigger: String, speed: f32) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_trigger_anim(self.0, controller, trigger, speed))
    }

    pub fn set_controller_state(self, controller: String, state: String) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_set_controller_state(self.0, controller, state))
    }

    pub fn hold_controller_state(
        self,
        controller: String,
        state: String,
        hold_ticks: Option<u32>,
    ) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_set_controller_state_hold(self.0, controller, state, hold_ticks))
    }

    pub fn release_controller_state(self, controller: String) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_release_controller_state(self.0, controller))
    }

    pub fn navigate_to(self, position: FVec3, speed: f64) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_navigate_to(self.0, position, speed))
    }

    pub fn navigate_stop(self) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_navigate_stop(self.0))
    }

    pub fn look_at(self, position: FVec3, yaw_speed: f32, pitch_speed: f32) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_look_at(self.0, position, yaw_speed, pitch_speed))
    }

    pub fn jump(self) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_jump(self.0))
    }

    pub fn set_target(self, target: Option<u32>, player: Option<String>) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_set_target(self.0, target, player))
    }

    pub fn attack(
        self,
        target: Option<u32>,
        player: Option<String>,
        damage: f32,
        knockback: f32,
    ) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_attack(self.0, target, player, damage, knockback))
    }

    pub fn attribute(self, attribute: String) -> Result<FAttributeValue, CallError> {
        flatten(raw::custom_mobs_mob_attribute_get(self.0, attribute))
    }

    pub fn set_attribute_base(self, attribute: String, value: f64) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_attribute_set_base(self.0, attribute, value))
    }

    pub fn add_attribute_modifier(
        self,
        attribute: String,
        modifier: String,
        amount: f64,
        operation: FAttributeOperation,
    ) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_attribute_add_modifier(
            self.0, attribute, modifier, amount, operation,
        ))
    }

    pub fn remove_attribute_modifier(self, attribute: String, modifier: String) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_attribute_remove_modifier(self.0, attribute, modifier))
    }

    pub fn set_hitbox_enabled(self, name: String, enabled: bool) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_set_hitbox_enabled(self.0, name, enabled))
    }

    pub fn data(self) -> Result<Vec<u8>, CallError> {
        flatten(raw::custom_mobs_mob_data_get(self.0))
    }

    pub fn put_data(self, data: Vec<u8>) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_data_put(self.0, data))
    }

    pub fn despawn(self) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_despawn(self.0))
    }

    pub fn claim_control(self, claim: bool) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_claim_control(self.0, claim))
    }

    pub fn impulse(self, velocity: FVec3) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_impulse(self.0, velocity))
    }

    pub fn seat_set(self, seat: u32, rider: Option<u32>) -> Result<bool, CallError> {
        flatten(raw::custom_mobs_mob_seat_set(self.0, seat, rider))
    }

    pub fn memory(self, key: String) -> Result<Option<FMobMemoryValue>, CallError> {
        flatten(raw::custom_mobs_mob_memory_get(self.0, key))
    }

    pub fn set_memory(
        self,
        key: String,
        value: Option<FMobMemoryValue>,
        ttl_ticks: Option<u32>,
    ) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_memory_set(self.0, key, value, ttl_ticks))
    }

    pub fn locator(self, name: String) -> Result<Option<FMobTransform>, Error> {
        raw::custom_mobs_mob_locator(self.0, name)
    }

    pub fn bind_texture(self, slot: String, id: Option<u32>) -> Result<(), CallError> {
        flatten(raw::custom_mobs_mob_texture_bind(self.0, slot, id))
    }
}

/// Registers all five `ScriptGoal` synthetic hook IDs (`can-use`, `can-continue`, `start`, `tick`, `stop`)
/// inside `yoake_init`.
pub fn register_goal_hooks(r: &mut Registrar, priority: i32) -> Result<(), RegisterError> {
    r.hook(hooks::MOB_GOAL_CAN_USE, priority)?;
    r.hook(hooks::MOB_GOAL_CAN_CONTINUE, priority)?;
    r.hook(hooks::MOB_GOAL_START, priority)?;
    r.hook(hooks::MOB_GOAL_TICK, priority)?;
    r.hook(hooks::MOB_GOAL_STOP, priority)?;
    Ok(())
}

/// Registers the three custom-mob renderer stage hooks (`before_pose`, `after_pose`, `after_vertices`)
/// inside `yoake_init`.
pub fn register_draw_stage_hooks(r: &mut Registrar, priority: i32) -> Result<(), RegisterError> {
    r.hook(hooks::CUSTOM_MOB_BEFORE_POSE, priority)?;
    r.hook(hooks::CUSTOM_MOB_AFTER_POSE, priority)?;
    r.hook(hooks::CUSTOM_MOB_AFTER_VERTICES, priority)?;
    Ok(())
}

pub fn register_goal(goal: String) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_goal_register(goal))
}

pub fn spawn_queued(
    dimension: String,
    type_id: String,
    position: FVec3,
    options: FMobSpawnOptions,
) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_spawn_queued(dimension, type_id, position, options))
}

pub fn bone_index(type_id: String, name: String) -> Result<Option<u32>, CallError> {
    flatten(raw::custom_mobs_mob_bone_index(type_id, name))
}

pub fn texture_create(width: u32, height: u32) -> Result<u32, CallError> {
    flatten(raw::custom_mobs_mob_texture_create(width, height))
}

pub fn texture_write(
    id: u32,
    origin: (u32, u32),
    size: (u32, u32),
    rgba: Vec<u8>,
) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_texture_write(id, origin, size, rgba))
}

pub fn hook_event() -> Result<Option<FMobHookEvent>, Error> {
    raw::custom_mobs_mob_hook_event()
}

pub fn reply_bool(claim: bool) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_reply_bool(claim))
}

pub fn reply_hurt(amount: f32, cancel: bool) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_reply_hurt(amount, cancel))
}

pub fn reply_interact(reply: FInteractReply) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_reply_interact(reply))
}

pub fn reply_save(data: Vec<u8>) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_reply_save(data))
}

pub fn reply_pose(writes: Vec<FMobBoneWrite>) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_reply_pose(writes))
}

pub fn draw_event() -> Result<Option<FMobDrawEvent>, Error> {
    raw::custom_mobs_mob_draw_event()
}

pub fn draw_cancel() -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_draw_cancel())
}

pub fn pose_bone_count() -> Result<u32, CallError> {
    flatten(raw::custom_mobs_mob_pose_bone_count())
}

pub fn pose_bone_name(bone: u32) -> Result<Option<String>, CallError> {
    flatten(raw::custom_mobs_mob_pose_bone_name(bone))
}

pub fn pose_bone_index(name: String) -> Result<Option<u32>, CallError> {
    flatten(raw::custom_mobs_mob_pose_bone_index(name))
}

pub fn pose_get(bone: u32) -> Result<FBoneTransform, CallError> {
    flatten(raw::custom_mobs_mob_pose_get(bone))
}

pub fn pose_get_all() -> Result<Vec<FBoneTransform>, CallError> {
    flatten(raw::custom_mobs_mob_pose_get_all())
}

pub fn pose_set(bone: u32, transform: FBoneTransform) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_pose_set(bone, transform))
}

pub fn pose_write(write: FMobBoneWrite) -> Result<(), CallError> {
    flatten(raw::custom_mobs_mob_pose_write(write))
}
