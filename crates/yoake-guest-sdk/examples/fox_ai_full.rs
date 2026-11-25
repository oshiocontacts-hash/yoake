//! Example full-access SERVER mod (F4M-11): mirrors `crates/yoake-wasm-sdk/examples/fox_ai` via Route F.
//!
//! Scripts the Cinder Fox `pounce` `ScriptGoal`, `mob-anim-event` ("hit" leap + bite), and `mob-hurt`
//! ("warmth" synced update) using `yoake_guest_sdk::custom_mobs` over `engine_call` bindings.
//!
//! Compiled as a `#![no_std]` wasm32 core module (see `tests/support/build.rs`).
#![no_std]
extern crate alloc;

use alloc::string::String;
use core::sync::atomic::{AtomicI64, AtomicU32, Ordering};

use yoake_guest_sdk::custom_mobs::{
    self, CustomMob, FMobHookEvent, FMobValue, FVec3, register_goal_hooks,
};
use yoake_guest_sdk::{Handle, Registrar, export_guest, hooks, report};

const POUNCE_TICKS: u32 = 16;
const MAX_POUNCING: usize = 32;

// Report slots verified by `game/yoake2_custom_mobs_state/guest_tests.rs`.
const SLOT_CAN_USE_TRUE: i32 = 1;
const SLOT_START: i32 = 2;
const SLOT_HIT_ATTACK: i32 = 3;
const SLOT_HURT_WARMTH: i32 = 4;

// Lock-free `(entity_id, ticks)` table (`0` = empty slot) so re-entrant activations never trap.
static POUNCING_IDS: [AtomicU32; MAX_POUNCING] = [const { AtomicU32::new(0) }; MAX_POUNCING];
static POUNCING_TICKS: [AtomicU32; MAX_POUNCING] = [const { AtomicU32::new(0) }; MAX_POUNCING];
static REGISTERED: AtomicI64 = AtomicI64::new(0);

fn is_pouncing(mob: u32) -> Option<usize> {
    for (i, slot) in POUNCING_IDS.iter().enumerate() {
        if slot.load(Ordering::Relaxed) == mob {
            return Some(i);
        }
    }
    None
}

fn start_pouncing(mob: u32) {
    if let Some(i) = is_pouncing(mob) {
        POUNCING_TICKS[i].store(0, Ordering::Relaxed);
        return;
    }
    for (i, slot) in POUNCING_IDS.iter().enumerate() {
        if slot.compare_exchange(0, mob, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            POUNCING_TICKS[i].store(0, Ordering::Relaxed);
            return;
        }
    }
}

fn stop_pouncing(mob: u32) {
    if let Some(i) = is_pouncing(mob) {
        POUNCING_TICKS[i].store(0, Ordering::Relaxed);
        POUNCING_IDS[i].store(0, Ordering::Relaxed);
    }
}

fn dist_sq(a: FVec3, b: FVec3) -> f64 {
    let dx = a.x - b.x;
    let dy = a.y - b.y;
    let dz = a.z - b.z;
    dx * dx + dy * dy + dz * dz
}

fn sqrt_f64(x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    let mut g = x.max(1.0);
    for _ in 0..16 {
        g = 0.5 * (g + x / g);
    }
    g
}

fn warmth_from_hp(health: f32, max_health: f32, damage: f32) -> i32 {
    let next_hp = (health - damage).max(0.0);
    if next_hp <= 0.0 {
        return 0;
    }
    let max_hp = max_health.max(1.0);
    let raw = ((max_hp - next_hp) * 10.0) / max_hp;
    let i = raw as i32;
    let ceiled = if raw > i as f32 { i + 1 } else { i };
    ceiled.clamp(1, 10)
}

fn init(r: &mut Registrar) {
    register_goal_hooks(r, 0).expect("register goal hooks");
    r.hook(hooks::MOB_ANIM_EVENT, 0).expect("register MOB_ANIM_EVENT");
    r.hook(hooks::MOB_HURT, 0).expect("register MOB_HURT");
}

fn hook(id: u32, _arg0: Handle) -> i64 {
    if REGISTERED.swap(1, Ordering::Relaxed) == 0 {
        let _ = custom_mobs::register_goal(String::from("pounce"));
    }
    let Ok(Some(event)) = custom_mobs::hook_event() else {
        return 0;
    };
    match (id, event) {
        (hooks::MOB_GOAL_CAN_USE, FMobHookEvent::GoalCanUse { mob, goal }) => {
            if goal == "pounce" {
                let cm = CustomMob::new(mob);
                if let Ok(Some(info)) = cm.info()
                    && info.on_ground
                    && info.target_visible
                    && let Some(target) = info.target_position
                {
                    let cooled = matches!(
                        cm.memory(String::from("pounced")),
                        Ok(Some(custom_mobs::FMobMemoryValue::Bool(true)))
                    );
                    let d2 = dist_sq(info.position, target);
                    if !cooled && (9.0..=36.0).contains(&d2) {
                        let _ = custom_mobs::reply_bool(true);
                        report(SLOT_CAN_USE_TRUE, i64::from(mob));
                        return 1;
                    }
                }
            }
            0
        }
        (hooks::MOB_GOAL_CAN_CONTINUE, FMobHookEvent::GoalCanContinue { mob, goal }) => {
            let keep = goal == "pounce"
                && is_pouncing(mob)
                    .is_some_and(|i| POUNCING_TICKS[i].load(Ordering::Relaxed) < POUNCE_TICKS);
            if keep {
                let _ = custom_mobs::reply_bool(true);
                1
            } else {
                0
            }
        }
        (hooks::MOB_GOAL_START, FMobHookEvent::GoalStart { mob, goal }) => {
            if goal == "pounce" {
                let cm = CustomMob::new(mob);
                start_pouncing(mob);
                let _ = cm.set_memory(
                    String::from("pounced"),
                    Some(custom_mobs::FMobMemoryValue::Bool(true)),
                    Some(40),
                );
                let _ = cm.navigate_stop();
                if let Ok(Some(info)) = cm.info()
                    && let Some(pos) = info.target_position
                {
                    let _ = cm.look_at(pos, 60.0, 60.0);
                }
                let _ = cm.trigger_anim(String::new(), String::from("pounce"), 1.0);
                report(SLOT_START, i64::from(mob));
            }
            0
        }
        (hooks::MOB_GOAL_TICK, FMobHookEvent::GoalTick { mob, goal }) => {
            if goal == "pounce"
                && let Some(i) = is_pouncing(mob)
            {
                POUNCING_TICKS[i].fetch_add(1, Ordering::Relaxed);
            }
            0
        }
        (hooks::MOB_GOAL_STOP, FMobHookEvent::GoalStop { mob, goal }) => {
            if goal == "pounce" {
                stop_pouncing(mob);
            }
            0
        }
        (hooks::MOB_ANIM_EVENT, FMobHookEvent::MobAnimEvent { mob, name }) => {
            if name == "hit" && is_pouncing(mob).is_some() {
                let cm = CustomMob::new(mob);
                if let Ok(Some(info)) = cm.info()
                    && let Some(target) = info.target_position
                {
                    let d = sqrt_f64(dist_sq(info.position, target)).max(0.1);
                    let dx = (target.x - info.position.x) / d;
                    let dz = (target.z - info.position.z) / d;
                    let _ = cm.impulse(FVec3 { x: dx * 0.6, y: 0.35, z: dz * 0.6 });
                    if d <= 4.0 && matches!(cm.attack(info.target, info.target_player, 3.0, 0.4), Ok(true)) {
                        report(SLOT_HIT_ATTACK, i64::from(mob));
                    }
                }
            }
            0
        }
        (hooks::MOB_HURT, FMobHookEvent::MobHurt { mob, amount, .. }) => {
            let cm = CustomMob::new(mob);
            if let Ok(Some(info)) = cm.info() {
                let warmth = warmth_from_hp(info.health, info.max_health, amount);
                if cm.set_synced(String::from("warmth"), FMobValue::Int(warmth)).is_ok() {
                    report(SLOT_HURT_WARMTH, i64::from(warmth));
                }
            }
            0
        }
        _ => 0,
    }
}

export_guest!(init, hook);
