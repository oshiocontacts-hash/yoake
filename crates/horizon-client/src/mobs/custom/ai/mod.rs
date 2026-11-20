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
//! * [`tame`], [`breed`], [`riding`], [`interact`] (F4M-4c): taming and the owner goals (`follow_owner`,
//!   `sit`, `owner_hurt_target`, `owner_hurt_by`, `owner_target`), breeding (`breed`, `follow_parent`),
//!   seats and saddle, and the `mob-interact` dispatch.
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
pub mod blocks;
pub mod breed;
pub mod budget;
pub mod expr;
pub mod goals;
pub mod interact;
pub mod nav_select;
pub mod rest;
pub mod riding;
pub mod tame;
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
        | "ranged_attack" | "area_attack" | "climb" | "sleep_in_bed_like" | "open_door"
        | "eat_block" => Goal,
        "target_player" | "hurt_by_target" | "hurt_by" | "call_for_help" | "nearest_target"
        | "nearest_attackable" => Target,
        "follow_owner" | "sit" | "breed" | "follow_parent" => Goal,
        "owner_hurt_target" | "owner_hurt_by" | "owner_target" => Target,
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
        let _ = goals::add(m, g)
            || attacks::add(m, g)
            || rest::add(m, t, g)
            || blocks::add(m, g)
            || tame::add(m, g)
            || breed::add(m, t, g)
            || targets::add(m, g);
    }
    tame::add_defaults(m, t);
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


/// A target or avoid list as the engine matches it: vanilla species, the player, and custom mob
/// types by id (the `Seen` snapshot of a custom mob carries its type id).
#[derive(Clone, Debug, Default)]
pub struct TypeList {
    pub species: Vec<Species>,
    pub player: bool,
    pub custom: Vec<std::sync::Arc<str>>,
}
impl TypeList {
    pub fn is_empty(&self) -> bool {
        self.species.is_empty() && self.custom.is_empty() && !self.player
    }
}

/// `ids` as a [`TypeList`]. An id outside `minecraft:` is taken for a custom type (one that is not
/// loaded now is kept with a warning, since content may load in any order); an unknown
/// `minecraft:` id is left out with a warning. `what` names the goal for the warning.
pub fn type_list(ids: &[String], what: &str) -> TypeList {
    let (species, player, others) = species_list(ids);
    let mut custom = Vec::new();
    for id in others {
        if id.starts_with("minecraft:") || !id.contains(':') {
            log::warn!("Horizon: {what}: {id} is not a vanilla mob type or the player; left out");
        } else {
            if super::get(&id).is_none() {
                log::warn!("Horizon: {what}: {id} is not a loaded custom mob type (kept: it may load later)");
            }
            custom.push(std::sync::Arc::from(id.as_str()));
        }
    }
    TypeList {
        species,
        player,
        custom,
    }
}
#[cfg(test)]
pub(crate) mod test_support;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_4c;
