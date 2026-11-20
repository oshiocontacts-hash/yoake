//! Target selectors (`ai.targets`, FR-F4-MOBS.md section 5): which entity a custom mob
//! turns on. All three are the engine's own `TARGET`-flag goals; a custom mob's attack goals
//! (`melee_attack`, `ranged_attack`, ...) act on whatever target a selector left in
//! `Mob.ai.target`.
//!
//! * `nearest_attackable` / `nearest_target` `{ types, must_see, range, interval }`:
//!   `NearestAttackableTargetGoal`. `types` are entity type ids: `minecraft:player` and vanilla
//!   mobs. `must_see` (default true) is the engine's: it decides how long a target that is out of
//!   sight is kept (60 ticks of memory) and a combat check of the line of sight always runs to
//!   pick it. `range` caps the search at that many blocks (default: the mob's `follow_range`).
//!   `interval` is the random 1-in-N pick delay (default 10). Without `types` it is the player.
//! * `target_player`: the same for the player only.
//! * `hurt_by` / `hurt_by_target` `{ alert, ignore }`: `HurtByTargetGoal`; `alert` calls mobs of
//!   the same species to help; `ignore` lists attackers to ignore.
//! * `call_for_help`: `hurt_by` with `alert` on.
//! * `owner_target` needs the tameable owner (F4M-4c) and is deferred.
//!
//! A custom mob type in `types` is not distinguishable by the engine's `Seen` snapshot (it has a
//! species, not a type id), so it is left out with a warning; mob-versus-custom-mob targeting
//! waits for a type id in the snapshot.
use super::{Params, species_list};
use crate::mobs::{
    Mob,
    goals::{HurtByTarget, NearestTarget},
};
use horizon_mods::custom_mob::Goal as GoalDef;

/// Adds the selector `g` names if it is one of this module's; false for any other type.
pub fn add(m: &mut Mob, g: &GoalDef) -> bool {
    let p = Params(g);
    let prio = p.priority();
    match g.kind.as_str() {
        "target_player" => {
            let goal = finish(m, NearestTarget::player(p.flag("must_see", true)), p);
            m.ai.targets.add(prio, goal);
        }
        "nearest_target" | "nearest_attackable" => {
            let must_see = p.flag("must_see", true);
            let (list, player, unknown) = species_list(&p.strings("types"));
            for id in &unknown {
                log::warn!(
                    "Horizon: target selector {}: {id} is not a vanilla mob type or the player; left out",
                    g.kind
                );
            }
            // No usable type at all: the player (the schema's default).
            let player = player || (list.is_empty() && unknown.is_empty());
            if player {
                let goal = finish(m, NearestTarget::player(must_see), p);
                m.ai.targets.add(prio, goal);
            }
            if !list.is_empty() {
                let goal = finish(m, NearestTarget::species(&list, must_see), p);
                m.ai.targets.add(prio, goal);
            }
        }
        "hurt_by_target" | "hurt_by" | "call_for_help" => {
            let (ignore, _, _) = species_list(&p.strings("ignore"));
            let mut goal = HurtByTarget::new().ignoring(&ignore);
            if g.kind == "call_for_help" || p.flag("alert", false) {
                goal = goal.alert();
            }
            m.ai.targets.add(prio, goal);
        }
        _ => return false,
    }
    true
}

/// The pick delay and the search range of a selector row.
fn finish(m: &Mob, goal: NearestTarget, p: Params) -> NearestTarget {
    let mut goal = goal.interval(p.int("interval", 10).max(1));
    let range = p.num("range", 0.0);
    let follow = m.attrs.follow_range;
    if range > 0.0 && follow > 0.0 {
        goal = goal.follow_scale((range / follow).clamp(0.01, 16.0));
    }
    goal
}
