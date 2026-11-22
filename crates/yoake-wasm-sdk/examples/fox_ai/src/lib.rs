//! The Cinder Fox `pounce` script goal (F4M-10, `tools/fixtures/cinder_fox`): a server module of
//! the `service.mobs` permission. The goal can start when the fox's target is 3 to 6 blocks away
//! with a line of sight; it starts the `pounce` animation and holds the fox still; on the server
//! timeline event `hit` of that animation it leaps and bites; it ends after the animation.
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, mobs, types::Vec3};

/// The animation is 0.8 s long.
const POUNCE_TICKS: u32 = 16;

#[derive(Default)]
struct Fox {
    /// Mobs that are pouncing, and the ticks each has done.
    pouncing: Vec<(u32, u32)>,
}

fn dist(a: &Vec3, b: &Vec3) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

impl ServerMod for Fox {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::MOB_GOAL_CAN_USE, hook::MOB_GOAL_CAN_CONTINUE, hook::MOB_GOAL_START, hook::MOB_GOAL_TICK, hook::MOB_GOAL_STOP, hook::MOB_ANIM_EVENT]
    }
    fn mob_goal_can_use(&mut self, mob: u32, goal: String) -> bool {
        if cfg!(feature = "trap") {
            panic!("the test guest traps");
        }
        if cfg!(feature = "spin") {
            loop {
                std::hint::black_box(0);
            }
        }
        let Ok(Some(i)) = mobs::info(mob) else { return false };
        let (true, true, Some(t)) = (goal == "pounce", i.on_ground && i.target_visible, i.target_position) else { return false };
        if matches!(mobs::memory_get(mob, "pounced"), Ok(Some(v)) if v.bool_value) {
            return false;
        }
        (3.0..=6.0).contains(&dist(&i.position, &t))
    }
    fn mob_goal_can_continue(&mut self, mob: u32, goal: String) -> bool {
        goal == "pounce" && self.pouncing.iter().any(|(m, ticks)| *m == mob && *ticks < POUNCE_TICKS)
    }
    fn mob_goal_start(&mut self, mob: u32, goal: String) {
        if goal != "pounce" {
            return;
        }
        self.pouncing.retain(|(m, _)| *m != mob);
        self.pouncing.push((mob, 0));
        let _ = mobs::memory_set(mob, "pounced", Some(mobs::memory_bool(true)), Some(40));
        let _ = mobs::navigate_stop(mob);
        if let Ok(Some(Vec3 { x, y, z })) = mobs::info(mob).map(|i| i.and_then(|i| i.target_position)) {
            let _ = mobs::look_at(mob, Vec3 { x, y, z }, 60.0, 60.0);
        }
        let _ = mobs::trigger_anim(mob, "", "pounce", 1.0);
    }
    fn mob_goal_tick(&mut self, mob: u32, goal: String) {
        if let (true, Some((_, ticks))) = (goal == "pounce", self.pouncing.iter_mut().find(|(m, _)| *m == mob)) {
            *ticks += 1;
        }
    }
    fn mob_goal_stop(&mut self, mob: u32, goal: String) {
        if goal == "pounce" {
            self.pouncing.retain(|(m, _)| *m != mob);
        }
    }
    fn mob_anim_event(&mut self, mob: u32, name: String) {
        if name != "hit" || !self.pouncing.iter().any(|(m, _)| *m == mob) {
            return;
        }
        let Ok(Some(i)) = mobs::info(mob) else { return };
        let Some(t) = i.target_position else { return };
        let d = dist(&i.position, &t).max(0.1);
        // A leap toward the target, then the bite (a target out of reach after the leap is missed).
        let (dx, dz) = ((t.x - i.position.x) / d, (t.z - i.position.z) / d);
        let _ = mobs::impulse(mob, Vec3 { x: dx * 0.6, y: 0.35, z: dz * 0.6 });
        if d <= 4.0 {
            let _ = mobs::attack(mob, i.target, i.target_player.as_deref(), 3.0, 0.4);
        }
    }
}

yoake_wasm_sdk::export_server_full!(Fox);
