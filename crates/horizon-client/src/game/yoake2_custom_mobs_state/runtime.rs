//! F4M-11 server and client `script::Runtime` / `script::ClientRuntime` adapters over synthetic hooks.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::sync::Arc;

use horizon_client::mobs::custom::script::{
    self, ClientRuntime, HurtReply, InteractReply, MAX_DATA_BYTES, MAX_POSE_WRITES, MobBoneWrite, MobPoseFrame,
    MobScriptValue, Runtime, ScriptVec3,
};
use yoake_slice::runtime as yr;

use super::{FInteractReply, FMobBoneWrite, FMobHookEvent, FMobPoseFrame, FVec3f, to_f_value};

const SERVER_HOOKS: &[u32] = &[
    yr::HOOK_MOB_GOAL_CAN_USE,
    yr::HOOK_MOB_GOAL_CAN_CONTINUE,
    yr::HOOK_MOB_GOAL_START,
    yr::HOOK_MOB_GOAL_TICK,
    yr::HOOK_MOB_GOAL_STOP,
    yr::HOOK_MOB_TICK,
    yr::HOOK_MOB_SPAWN_CHECK,
    yr::HOOK_MOB_SPAWNED,
    yr::HOOK_MOB_HURT,
    yr::HOOK_MOB_DIED,
    yr::HOOK_MOB_INTERACT,
    yr::HOOK_MOB_ANIM_EVENT,
    yr::HOOK_MOB_LOAD,
    yr::HOOK_MOB_SAVE,
    yr::HOOK_MOB_TAMED,
    yr::HOOK_MOB_BRED,
    yr::HOOK_MOB_SYNCED_CHANGED_SERVER,
];

const CLIENT_HOOKS: &[u32] = &[
    yr::HOOK_MOB_POSE,
    yr::HOOK_MOB_ANIM_EVENT_CLIENT,
    yr::HOOK_MOB_SYNCED_CHANGED_CLIENT,
];

#[derive(Default)]
struct HookContext {
    event: Option<FMobHookEvent>,
    bool_reply: Option<bool>,
    hurt_reply: Option<HurtReply>,
    interact_reply: Option<InteractReply>,
    save_reply: Option<Vec<u8>>,
    pose_writes: Vec<MobBoneWrite>,
    registered_goals: BTreeSet<String>,
}

thread_local! {
    static HOOK: RefCell<HookContext> = RefCell::new(HookContext::default());
}

struct ClearGuard;

impl Drop for ClearGuard {
    fn drop(&mut self) {
        HOOK.with(|h| {
            let mut h = h.borrow_mut();
            h.event = None;
            h.bool_reply = None;
            h.hurt_reply = None;
            h.interact_reply = None;
            h.save_reply = None;
            h.pose_writes.clear();
        });
    }
}

fn dispatch_with_event<R>(hook: u32, event: FMobHookEvent, collect: impl FnOnce(i64, &HookContext) -> R) -> R {
    HOOK.with(|h| {
        let mut h = h.borrow_mut();
        h.event = Some(event);
        h.bool_reply = None;
        h.hurt_reply = None;
        h.interact_reply = None;
        h.save_reply = None;
        h.pose_writes.clear();
    });
    let _clear = ClearGuard;
    let raw = crate::yoake2_mods::dispatch_custom_mob_hook(hook);
    HOOK.with(|h| collect(raw, &h.borrow()))
}

pub(crate) fn register_goal(goal: String) -> Result<(), String> {
    super::validate_name(&goal, "goal")?;
    HOOK.with(|h| {
        h.borrow_mut().registered_goals.insert(goal);
    });
    Ok(())
}

pub(crate) fn reset_goals() {
    HOOK.with(|h| h.borrow_mut().registered_goals.clear());
}

pub(crate) fn goal_registered(goal: &str) -> bool {
    HOOK.with(|h| h.borrow().registered_goals.contains(goal))
}

pub(crate) fn hook_event() -> Option<FMobHookEvent> {
    HOOK.with(|h| h.borrow().event.clone())
}

pub(crate) fn reply_bool(claim: bool) -> Result<(), String> {
    HOOK.with(|h| {
        let mut h = h.borrow_mut();
        let accepts = matches!(
            h.event,
            Some(
                FMobHookEvent::GoalCanUse { .. }
                    | FMobHookEvent::GoalCanContinue { .. }
                    | FMobHookEvent::MobSpawnCheck { .. }
            )
        );
        if !accepts {
            return Err("mob_reply_bool is only valid in goal can-use/can-continue or mob_spawn_check".into());
        }
        if claim {
            h.bool_reply = Some(true);
        } else if h.bool_reply.is_none() {
            h.bool_reply = Some(false);
        }
        Ok(())
    })
}

pub(crate) fn reply_hurt(amount: f32, cancel: bool) -> Result<(), String> {
    if !amount.is_finite() || amount < 0.0 {
        return Err("hurt amount must be finite and non-negative".into());
    }
    HOOK.with(|h| {
        let mut h = h.borrow_mut();
        if !matches!(h.event, Some(FMobHookEvent::MobHurt { .. })) {
            return Err("mob_reply_hurt is only valid in mob_hurt".into());
        }
        let prev = h.hurt_reply;
        h.hurt_reply = Some(HurtReply {
            amount,
            cancel: cancel || prev.is_some_and(|p| p.cancel),
        });
        Ok(())
    })
}

pub(crate) fn reply_interact(reply: FInteractReply) -> Result<(), String> {
    let reply = match reply {
        FInteractReply::Pass => InteractReply::Pass,
        FInteractReply::Consume => InteractReply::Consume,
        FInteractReply::Success => InteractReply::Success,
    };
    HOOK.with(|h| {
        let mut h = h.borrow_mut();
        if !matches!(h.event, Some(FMobHookEvent::MobInteract { .. })) {
            return Err("mob_reply_interact is only valid in mob_interact".into());
        }
        if reply != InteractReply::Pass && matches!(h.interact_reply, None | Some(InteractReply::Pass)) {
            h.interact_reply = Some(reply);
        } else if h.interact_reply.is_none() {
            h.interact_reply = Some(reply);
        }
        Ok(())
    })
}

pub(crate) fn reply_save(data: Vec<u8>) -> Result<(), String> {
    if data.len() > MAX_DATA_BYTES {
        return Err("script save data is at most 64 KiB".into());
    }
    HOOK.with(|h| {
        let mut h = h.borrow_mut();
        if !matches!(h.event, Some(FMobHookEvent::MobSave { .. })) {
            return Err("mob_reply_save is only valid in mob_save".into());
        }
        h.save_reply = Some(data);
        Ok(())
    })
}

pub(crate) fn reply_pose(writes: Vec<FMobBoneWrite>) -> Result<(), String> {
    HOOK.with(|h| {
        let mut h = h.borrow_mut();
        if !matches!(h.event, Some(FMobHookEvent::MobPose { .. })) {
            return Err("mob_reply_pose is only valid in mob_pose".into());
        }
        for w in writes {
            if h.pose_writes.len() >= MAX_POSE_WRITES {
                break;
            }
            let v = |p: FVec3f| ScriptVec3 { x: p.x, y: p.y, z: p.z };
            h.pose_writes.push(MobBoneWrite {
                bone: w.bone,
                additive: w.additive,
                translation: w.translation.map(v),
                rotation_degrees: w.rotation_degrees.map(v),
                scale: w.scale.map(v),
                visible: w.visible,
            });
        }
        Ok(())
    })
}

/// F-route server script runtime forwarding custom-mob hooks to loaded F modules on the active thread.
pub(crate) struct FScriptRuntime;

pub(crate) fn f_world_runtime() -> Arc<dyn Runtime> {
    Arc::new(FScriptRuntime)
}

pub(crate) fn f_client_runtime() -> Arc<dyn ClientRuntime> {
    Arc::new(FScriptRuntime)
}

/// Opens a server `script::Session` when F modules registered custom-mob hooks and no S-route session holds it.
pub(crate) fn ensure_world_script_session(slot: &mut Option<script::Session>) {
    if slot.is_some() || !crate::yoake2_mods::thread_registered_any(SERVER_HOOKS) {
        return;
    }
    match script::open_session(f_world_runtime()) {
        Ok(session) => *slot = Some(session),
        Err(_) => log::warn!("another server already holds the custom-mob script runtime"),
    }
}

fn claim_answer(hook: u32, event: FMobHookEvent) -> Option<bool> {
    if !crate::yoake2_mods::mob_hook_registered(hook) {
        return None;
    }
    dispatch_with_event(hook, event, |raw, ctx| {
        ctx.bool_reply.or_else(|| (raw != 0).then_some(true))
    })
}

impl Runtime for FScriptRuntime {
    fn live(&self) -> bool {
        crate::yoake2_mods::thread_registered_any(SERVER_HOOKS)
    }

    fn goal_can_use(&self, mob: u32, goal: &str) -> Option<bool> {
        claim_answer(yr::HOOK_MOB_GOAL_CAN_USE, FMobHookEvent::GoalCanUse { mob, goal: goal.to_owned() })
    }

    fn goal_can_continue(&self, mob: u32, goal: &str) -> Option<bool> {
        claim_answer(
            yr::HOOK_MOB_GOAL_CAN_CONTINUE,
            FMobHookEvent::GoalCanContinue { mob, goal: goal.to_owned() },
        )
    }

    fn goal_start(&self, mob: u32, goal: &str) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_GOAL_START) {
            dispatch_with_event(
                yr::HOOK_MOB_GOAL_START,
                FMobHookEvent::GoalStart { mob, goal: goal.to_owned() },
                |_, _| {},
            );
        }
    }

    fn goal_tick(&self, mob: u32, goal: &str) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_GOAL_TICK) {
            dispatch_with_event(
                yr::HOOK_MOB_GOAL_TICK,
                FMobHookEvent::GoalTick { mob, goal: goal.to_owned() },
                |_, _| {},
            );
        }
    }

    fn goal_stop(&self, mob: u32, goal: &str) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_GOAL_STOP) {
            dispatch_with_event(
                yr::HOOK_MOB_GOAL_STOP,
                FMobHookEvent::GoalStop { mob, goal: goal.to_owned() },
                |_, _| {},
            );
        }
    }

    fn mob_tick(&self, mob: u32) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_TICK) {
            dispatch_with_event(yr::HOOK_MOB_TICK, FMobHookEvent::MobTick { mob }, |_, _| {});
        }
    }

    fn mob_spawn_check(&self, type_id: &str, dimension: &str, pos: [i32; 3]) -> Option<bool> {
        claim_answer(
            yr::HOOK_MOB_SPAWN_CHECK,
            FMobHookEvent::MobSpawnCheck {
                type_id: type_id.to_owned(),
                dimension: dimension.to_owned(),
                pos: (pos[0], pos[1], pos[2]),
            },
        )
    }

    fn mob_spawned(&self, mob: u32, type_id: &str, reason: &str) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_SPAWNED) {
            dispatch_with_event(
                yr::HOOK_MOB_SPAWNED,
                FMobHookEvent::MobSpawned { mob, type_id: type_id.to_owned(), reason: reason.to_owned() },
                |_, _| {},
            );
        }
    }

    fn mob_hurt(&self, mob: u32, part: &str, source: &str, amount: f32) -> Option<HurtReply> {
        if !crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_HURT) {
            return None;
        }
        dispatch_with_event(
            yr::HOOK_MOB_HURT,
            FMobHookEvent::MobHurt {
                mob,
                part: part.to_owned(),
                source: source.to_owned(),
                amount,
            },
            |_, ctx| ctx.hurt_reply,
        )
    }

    fn mob_died(&self, mob: u32, type_id: &str, attacker: Option<u32>) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_DIED) {
            dispatch_with_event(
                yr::HOOK_MOB_DIED,
                FMobHookEvent::MobDied { mob, type_id: type_id.to_owned(), attacker },
                |_, _| {},
            );
        }
    }

    fn mob_interact(&self, mob: u32, player: &str, hand: &str, item: &str) -> InteractReply {
        if !crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_INTERACT) {
            return InteractReply::Pass;
        }
        dispatch_with_event(
            yr::HOOK_MOB_INTERACT,
            FMobHookEvent::MobInteract {
                mob,
                player: player.to_owned(),
                hand: hand.to_owned(),
                item: item.to_owned(),
            },
            |_, ctx| ctx.interact_reply.unwrap_or(InteractReply::Pass),
        )
    }

    fn mob_anim_event(&self, mob: u32, name: &str) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_ANIM_EVENT) {
            dispatch_with_event(
                yr::HOOK_MOB_ANIM_EVENT,
                FMobHookEvent::MobAnimEvent { mob, name: name.to_owned() },
                |_, _| {},
            );
        }
    }

    fn mob_load(&self, mob: u32, data: &[u8]) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_LOAD) {
            dispatch_with_event(
                yr::HOOK_MOB_LOAD,
                FMobHookEvent::MobLoad { mob, data: data.to_vec() },
                |_, _| {},
            );
        }
    }

    fn mob_save(&self, mob: u32, data: &[u8]) -> Option<Vec<u8>> {
        if !crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_SAVE) {
            return None;
        }
        dispatch_with_event(
            yr::HOOK_MOB_SAVE,
            FMobHookEvent::MobSave { mob, data: data.to_vec() },
            |_, ctx| ctx.save_reply.clone(),
        )
    }

    fn mob_tamed(&self, mob: u32, owner: &str) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_TAMED) {
            dispatch_with_event(
                yr::HOOK_MOB_TAMED,
                FMobHookEvent::MobTamed { mob, owner: owner.to_owned() },
                |_, _| {},
            );
        }
    }

    fn mob_bred(&self, mob: u32, partner: u32, child: u32) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_BRED) {
            dispatch_with_event(
                yr::HOOK_MOB_BRED,
                FMobHookEvent::MobBred { mob, partner, child },
                |_, _| {},
            );
        }
    }

    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_SYNCED_CHANGED_SERVER) {
            dispatch_with_event(
                yr::HOOK_MOB_SYNCED_CHANGED_SERVER,
                FMobHookEvent::SyncedChangedServer {
                    mob,
                    key: key.to_owned(),
                    old: to_f_value(old),
                    new: to_f_value(new),
                },
                |_, _| {},
            );
        }
    }
}

impl ClientRuntime for FScriptRuntime {
    fn live(&self) -> bool {
        crate::yoake2_mods::thread_registered_any(CLIENT_HOOKS)
    }

    fn mob_pose(&self, mob: u32, kind: &str, frame: &MobPoseFrame) -> Vec<MobBoneWrite> {
        if !crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_POSE) {
            return Vec::new();
        }
        let wire_frame = FMobPoseFrame {
            time: frame.time,
            partial: frame.partial,
            ground_speed: frame.ground_speed,
            head_yaw: frame.head_yaw,
            head_pitch: frame.head_pitch,
            on_ground: frame.on_ground,
            target: frame.target.map(|t| FVec3f { x: t.x, y: t.y, z: t.z }),
        };
        dispatch_with_event(
            yr::HOOK_MOB_POSE,
            FMobHookEvent::MobPose { mob, type_id: kind.to_owned(), frame: wire_frame },
            |_, ctx| ctx.pose_writes.clone(),
        )
    }

    fn mob_anim_event(&self, mob: u32, name: &str) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_ANIM_EVENT_CLIENT) {
            dispatch_with_event(
                yr::HOOK_MOB_ANIM_EVENT_CLIENT,
                FMobHookEvent::MobAnimEventClient { mob, name: name.to_owned() },
                |_, _| {},
            );
        }
    }

    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue) {
        if crate::yoake2_mods::mob_hook_registered(yr::HOOK_MOB_SYNCED_CHANGED_CLIENT) {
            dispatch_with_event(
                yr::HOOK_MOB_SYNCED_CHANGED_CLIENT,
                FMobHookEvent::SyncedChangedClient {
                    mob,
                    key: key.to_owned(),
                    old: to_f_value(old),
                    new: to_f_value(new),
                },
                |_, _| {},
            );
        }
    }
}
