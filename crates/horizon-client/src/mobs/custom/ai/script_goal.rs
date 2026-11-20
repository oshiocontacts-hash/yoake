//! The `script` goal (FR-F4-MOBS.md section 3.3, F4M-10): an engine `Goal` whose body is a WASM
//! export. `ScriptGoal` forwards `can-use`, `can-continue`, `start`, `tick` and `stop` to the guest
//! through [`script::call`], claims the control flags of its `ai.script_goals` row, and evaluates
//! `can-use` only every `interval` ticks.
//!
//! * **Budget**: a mob may make [`script::MAX_EVALUATIONS_PER_MOB_TICK`] (32) goal calls in one tick,
//!   all its script goals together. Past it `can-use` is false, `can-continue` true (a running goal
//!   is not cut off), `start` and `tick` are skipped; `stop` is always made (it is not counted).
//! * **No guest, a trapped or timed-out guest**: `can-use` is false and `can-continue` false, so the
//!   goal never starts and a running one stops (the engine's selector then calls `stop`, which
//!   reaches nobody). The sandbox disables a module that traps or overruns; see
//!   `docs/yoake-wasm-dev.md` for its per-call limits.
use super::Params;
use crate::mobs::{
    Mob,
    goals::{Cx, Goal, JUMP, LOOK, MOVE, TARGET, goal_clone},
};
use crate::mobs::custom::script;
use horizon_mods::custom_mob::ScriptGoal as ScriptGoalDef;

/// The control flags a script goal row names.
pub fn flags_of(names: &[String]) -> u8 {
    names.iter().fold(0, |f, n| {
        f | match n.as_str() {
            "move" => MOVE,
            "look" => LOOK,
            "jump" => JUMP,
            "target" => TARGET,
            _ => 0,
        }
    })
}

#[derive(Clone, Debug)]
pub struct ScriptGoal {
    id: String,
    flags: u8,
    interval: i64,
    /// The game time `can-use` is next asked at.
    next_use: i64,
}

impl ScriptGoal {
    pub fn new(def: &ScriptGoalDef) -> Self {
        Self {
            id: def.id.clone(),
            flags: flags_of(&def.flags),
            interval: i64::from(def.interval.clamp(1, 200)),
            next_use: 0,
        }
    }
    /// Whether the mob may make another guest call this tick.
    fn charge(m: &mut Mob, now: i64) -> bool {
        script::state(m).is_some_and(|s| s.charge(now))
    }
}

/// Adds the `script` goal row `g` of the type's file; false for any other type.
pub fn add(m: &mut Mob, t: &super::CustomType, g: &horizon_mods::custom_mob::Goal) -> bool {
    if g.kind != "script" {
        return false;
    }
    let Some(def) = g.id.as_deref().and_then(|id| t.file.ai.script_goals.iter().find(|s| s.id == id)) else {
        return true;
    };
    m.ai.goals.add(Params(g).priority(), ScriptGoal::new(def));
    true
}

impl Goal for ScriptGoal {
    goal_clone!();
    fn name(&self) -> &'static str {
        "ScriptGoal"
    }
    fn flags(&self) -> u8 {
        self.flags
    }
    fn can_use(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        let now = cx.w.game_time;
        if now < self.next_use || !script::installed() {
            return false;
        }
        self.next_use = now + self.interval;
        if !Self::charge(m, now) {
            return false;
        }
        let (id, goal) = (m.id, self.id.as_str());
        script::call(m, Some(cx.w), Some(cx.events), |rt| rt.goal_can_use(id, goal))
            .flatten()
            .unwrap_or(false)
    }
    fn can_continue(&mut self, m: &mut Mob, cx: &mut Cx) -> bool {
        if !Self::charge(m, cx.w.game_time) {
            return true;
        }
        let (id, goal) = (m.id, self.id.as_str());
        script::call(m, Some(cx.w), Some(cx.events), |rt| rt.goal_can_continue(id, goal))
            .flatten()
            .unwrap_or(false)
    }
    fn start(&mut self, m: &mut Mob, cx: &mut Cx) {
        if Self::charge(m, cx.w.game_time) {
            let (id, goal) = (m.id, self.id.as_str());
            script::call(m, Some(cx.w), Some(cx.events), |rt| rt.goal_start(id, goal));
        }
    }
    fn every_tick(&self) -> bool {
        true
    }
    fn tick(&mut self, m: &mut Mob, cx: &mut Cx) {
        if Self::charge(m, cx.w.game_time) {
            let (id, goal) = (m.id, self.id.as_str());
            script::call(m, Some(cx.w), Some(cx.events), |rt| rt.goal_tick(id, goal));
        }
    }
    fn stop(&mut self, m: &mut Mob, cx: &mut Cx) {
        let (id, goal) = (m.id, self.id.as_str());
        script::call(m, Some(cx.w), Some(cx.events), |rt| rt.goal_stop(id, goal));
    }
}
