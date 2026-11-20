//! The AI of a fully custom mob that comes from its JSON (FR-F4-MOBS.md sections 3.3, 5, 6.1;
//! F4M-4a): the data goal catalogue as native `Goal` structs (no WASM call), the navigator
//! the file picks, the attack goals with their trigger animation and damage frame, and the
//! target selectors.
//!
//! * [`nav_select`]: `ai.navigator` (`ground`, `climb`, `swim`, `fly`, `amphibious`) to the
//!   engine's `NavKind`, move control and look control, and the path budget.
//! * [`goals`]: movement, idle and defensive goals (wrapping the engine's own goals).
//! * [`attacks`]: `melee_attack`, `ranged_attack`, `leap_at_target`, `leap_attack`,
//!   `area_attack`.
//! * [`targets`]: `target_player`, `nearest_target`/`nearest_attackable`, `hurt_by`,
//!   `call_for_help`.
//!
//! Control flags and their conflicts are the engine's: each goal reports its own `MOVE`, `LOOK`,
//! `JUMP` and `TARGET` flags and the `GoalSelector` resolves them as vanilla's does (a goal with
//! a lower priority number replaces a running one that holds a flag it needs, if that one is
//! interruptible; goals with disjoint flags run together).
//!
//! A goal type the schema accepts but this version does not run is listed by [`support`] as
//! deferred with its reason; the mob simply does not get that goal (the type loader logs one
//! warning per type, see [`unsupported`]). The schema keeps the type in its vocabulary so a
//! content pack written for the later items loads.
use super::CustomType;
use crate::mobs::{Mob, Species, species};
use horizon_mods::custom_mob::{CustomMobFile, Goal as GoalDef};

pub mod attacks;
pub mod goals;
pub mod nav_select;
pub mod targets;

/// What this version does with a goal type of the schema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    /// A goal of `ai.goals`.
    Goal,
    /// A target selector (`ai.targets`).
    Target,
    /// Accepted by the schema and not run yet; the text says which item brings it.
    Deferred(&'static str),
}

/// What `kind` (a [`horizon_mods::custom_mob::GOAL_TYPES`] entry) is in this version.
pub fn support(kind: &str) -> Support {
    use Support::*;
    match kind {
        "float" | "swim" | "float_in_water" | "wander" | "stroll" | "random_swim"
        | "fly_wander" | "look_at_player" | "random_look" | "look_at_target" | "panic"
        | "tempt" | "flee" | "avoid" | "retreat_when_hurt" | "go_to_block" | "stay_near_home"
        | "patrol_around" | "leap_at_target" | "leap_attack" | "melee_attack"
        | "ranged_attack" | "area_attack" => Goal,
        "target_player" | "hurt_by_target" | "hurt_by" | "call_for_help" | "nearest_target"
        | "nearest_attackable" => Target,
        "follow_owner" | "sit" | "owner_hurt_target" | "owner_hurt_by" | "owner_target" => {
            Deferred("F4M-4c: needs the tameable owner state")
        }
        "breed" | "follow_parent" => Deferred("F4M-4c: needs the breedable data and offspring"),
        "eat_block" => Deferred("the engine's EatBlock goal is the sheep's; a data version needs a generic rewrite"),
        "sleep_in_bed_like" | "climb" => {
            Deferred("the schema row does not say what it does; needs a definition")
        }
        "open_door" => Deferred("the engine's door goals are private to raiders; needs a generic OpenDoorGoal"),
        "script" => Deferred("F4M-10: script goals"),
        _ => Deferred("unknown goal type"),
    }
}

/// The goals and targets of `file` this version does not run, with the reason.
pub fn unsupported(file: &CustomMobFile) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for g in file.ai.goals.iter().chain(&file.ai.targets) {
        if let Support::Deferred(why) = support(&g.kind)
            && !out.iter().any(|(k, _): &(String, _)| *k == g.kind)
        {
            out.push((g.kind.clone(), why));
        }
    }
    out
}

/// Gives the mob its navigator, goals and targets from the type's file. Goals go to the goal
/// selector and target selectors to the target selector by their *type*, whichever array
/// of the file names them (`call_for_help` is a target selector wherever it is written).
pub fn attach(m: &mut Mob, t: &CustomType) {
    nav_select::apply(m, &t.file.ai.navigator);
    for g in t.file.ai.goals.iter().chain(&t.file.ai.targets) {
        let _ = goals::add(m, g) || attacks::add(m, g) || targets::add(m, g);
    }
}

/// Reads the free parameters of a goal row (`{"type": "...", "range": 8}`), with the engine's
/// defaults for what is missing or not a finite number.
#[derive(Clone, Copy)]
pub struct Params<'a>(pub &'a GoalDef);

impl Params<'_> {
    pub fn num(&self, key: &str, default: f64) -> f64 {
        self.0
            .params
            .get(key)
            .and_then(|v| v.as_f64())
            .filter(|v| v.is_finite())
            .unwrap_or(default)
    }
    pub fn int(&self, key: &str, default: i32) -> i32 {
        self.num(key, f64::from(default))
            .clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    }
    pub fn flag(&self, key: &str, default: bool) -> bool {
        self.0
            .params
            .get(key)
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    }
    pub fn text(&self, key: &str) -> Option<&str> {
        self.0.params.get(key).and_then(|v| v.as_str())
    }
    pub fn strings(&self, key: &str) -> Vec<String> {
        self.0
            .params
            .get(key)
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }
    /// `speed` of the row, or `default`.
    pub fn speed(&self, default: f64) -> f64 {
        self.0.speed.unwrap_or(default)
    }
    pub fn priority(&self) -> i32 {
        self.0.priority
    }
}

/// The vanilla species of `minecraft:<name>`.
pub fn species_of(id: &str) -> Option<Species> {
    let name = id.strip_prefix("minecraft:")?;
    species::ALL.iter().find(|d| d.name == name).map(|d| d.species)
}

/// A list of entity type ids as vanilla species, and whether it names the player. Ids that are
/// neither (another mod's, or a custom type: the engine's `Seen` snapshot has no type id for a
/// custom mob) are returned in the third part so the caller can say so.
pub fn species_list(ids: &[String]) -> (Vec<Species>, bool, Vec<String>) {
    let mut out = Vec::new();
    let mut player = false;
    let mut unknown = Vec::new();
    for id in ids {
        if id == "minecraft:player" {
            player = true;
        } else if let Some(s) = species_of(id) {
            out.push(s);
        } else {
            unknown.push(id.clone());
        }
    }
    (out, player, unknown)
}

#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
