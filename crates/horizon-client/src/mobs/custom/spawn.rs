//! Natural spawning and despawning of custom mobs (`spawn`, `despawn`, FR-F4-MOBS.md section 5).
//!
//! Where a custom mob may appear is the biome list: the content root writes one `add_spawns` biome
//! modifier per rule (`mod_content.rs`), so the engine's own `NaturalSpawner` port picks the entry,
//! counts the category's cap and calls into this module for what is the type's own: the
//! placement, the light, the time, the height, the dimension, the structure, the distance to the
//! player, the cluster size and the despawn rules. Everything here is only reached for a mob of
//! [`Species::Custom`](crate::mobs::Species::Custom).
//!
//! A biome entry carries `(weight, min, max)` only, so the rule of a pick is found by those three
//! numbers; rules that share them are or-ed (a position that suits any of them spawns). A
//! type without a rule that a modifier from elsewhere still lists gets the category's defaults.
use super::CustomType;
use crate::{
    biome::SpawnEntry,
    blocks::Fluid,
    mobs::{
        Mob, MobWorld,
        spawn_rules::{self, Dimension, Reason, SpawnLevel},
        spawn_selection::{self, Category, NaturalLevel},
    },
};
use horizon_core::{geometry::Aabb, random::RandomSource};
use horizon_mods::custom_mob::Spawn;
use serde_json::Value;

/// How bright it must be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Light {
    Any,
    /// `Monster.isDarkEnoughToSpawn`.
    Dark,
    /// More than 8 (`Animal.isBrightEnoughToSpawn`).
    Bright,
    /// At most these raw levels (sky and block).
    Levels { sky_max: i32, block_max: i32 },
}
/// The part of the day.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Time {
    Any,
    /// `Level.isBrightOutside`: the sky is darkened by less than 4.
    Day,
    Night,
}
/// Where a spawn position is valid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// Solid ground to stand on, two free blocks.
    OnGround,
    InWater,
    InLava,
    /// `OnGround` with the sky above.
    OnSurface,
    NoRestrictions,
    /// Decided by the `mob-spawn-check` hook of a script (the WASM item): until it exists, never.
    Custom,
}

/// One compiled `spawn` rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub weight: u32,
    pub min: u32,
    pub max: u32,
    pub placement: Placement,
    pub light: Light,
    pub time: Time,
    pub y_range: Option<(i32, i32)>,
    /// Blocks; the squared distance to the nearest player must be at least this squared.
    pub min_distance: Option<f64>,
    pub cluster: Option<u32>,
    /// Dimension ids; none is any.
    pub dimensions: Vec<String>,
    /// Structure ids; none is anywhere.
    pub structures: Vec<String>,
}

fn id_list(what: &str, v: &Option<Value>) -> Result<Vec<String>, String> {
    match v {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) => Ok(vec![s.clone()]),
        Some(Value::Array(a)) => a
            .iter()
            .map(|e| {
                e.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| format!("spawn {what}: a list of ids"))
            })
            .collect(),
        Some(_) => Err(format!("spawn {what}: an id or a list of ids")),
    }
}

/// The placement a category gets without one: water creatures in water, the rest on the ground.
pub fn default_placement(category: Category) -> Placement {
    match category {
        Category::Axolotls
        | Category::UndergroundWaterCreature
        | Category::WaterCreature
        | Category::WaterAmbient => Placement::InWater,
        _ => Placement::OnGround,
    }
}
/// The light a category gets without one: monsters need the dark, animals the light.
pub fn default_light(category: Category) -> Light {
    match category {
        Category::Monster => Light::Dark,
        Category::Creature => Light::Bright,
        _ => Light::Any,
    }
}

impl Rule {
    /// The rule of a category without any (`weight 10, 1 to 4`).
    pub fn default_for(category: Category) -> Self {
        Self {
            weight: 10,
            min: 1,
            max: 4,
            placement: default_placement(category),
            light: default_light(category),
            time: Time::Any,
            y_range: None,
            min_distance: None,
            cluster: None,
            dimensions: Vec::new(),
            structures: Vec::new(),
        }
    }

    pub fn from_file(s: &Spawn, category: Category) -> Result<Self, String> {
        let placement = match s.placement.as_deref() {
            None => default_placement(category),
            Some("on_ground") => Placement::OnGround,
            Some("in_water") => Placement::InWater,
            Some("in_lava") => Placement::InLava,
            Some("on_surface") => Placement::OnSurface,
            Some("no_restrictions") => Placement::NoRestrictions,
            Some("custom") => Placement::Custom,
            Some(p) => return Err(format!("spawn placement {p:?}")),
        };
        let level = |v: &Value, key: &str| -> Result<i32, String> {
            match v.get(key) {
                None => Ok(15),
                Some(n) => n
                    .as_i64()
                    .filter(|n| (0..=15).contains(n))
                    .map(|n| n as i32)
                    .ok_or_else(|| format!("spawn light.{key} is 0 to 15")),
            }
        };
        let light = match &s.light {
            None => default_light(category),
            Some(Value::String(l)) => match l.as_str() {
                "any" => Light::Any,
                "dark" => Light::Dark,
                "bright" => Light::Bright,
                other => return Err(format!("spawn light {other:?} is any, dark, bright or levels")),
            },
            Some(v @ Value::Object(_)) => Light::Levels {
                sky_max: level(v, "sky_max")?,
                block_max: level(v, "block_max")?,
            },
            Some(_) => return Err("spawn light is any, dark, bright or {sky_max, block_max}".into()),
        };
        let time = match s.time.as_deref() {
            None | Some("any") => Time::Any,
            Some("day") => Time::Day,
            Some("night") => Time::Night,
            Some(t) => return Err(format!("spawn time {t:?} is any, day or night")),
        };
        Ok(Self {
            weight: s.weight,
            min: s.min,
            max: s.max,
            placement,
            light,
            time,
            y_range: s.y_range.map(|[lo, hi]| (lo, hi)),
            min_distance: s.min_distance_from_player.filter(|d| d.is_finite() && *d > 0.0),
            cluster: s.cluster.filter(|c| *c > 0),
            dimensions: id_list("dimensions", &s.dimensions)?,
            structures: id_list("structures", &s.structures)?,
        })
    }
}

/// What a position offers, read once: the pure rule checks work on it.
#[derive(Clone, Debug)]
pub struct Site {
    pub y: i32,
    pub sky: i32,
    pub block: i32,
    pub sky_darken: i32,
    pub dimension: &'static str,
    /// To the nearest player, squared (infinite when unknown).
    pub distance_sqr: f64,
}

/// The dimension's id as the rules write it.
pub fn dimension_id(d: Dimension) -> &'static str {
    match d {
        Dimension::Overworld => "minecraft:overworld",
        Dimension::Nether => "minecraft:the_nether",
        Dimension::End => "minecraft:the_end",
    }
}

impl Rule {
    /// Whether the rule's conditions (not the placement) hold. `dark` answers the vanilla dark
    /// test (it draws random numbers), `structures` whether the position is in any of the listed
    /// structures; both are asked only when the rule needs them.
    pub fn allows(
        &self,
        site: &Site,
        dark: &mut dyn FnMut() -> bool,
        structures: &dyn Fn(&[String]) -> bool,
    ) -> bool {
        if let Some((lo, hi)) = self.y_range
            && !(lo..=hi).contains(&site.y)
        {
            return false;
        }
        if !self.dimensions.is_empty() && !self.dimensions.iter().any(|d| d == site.dimension) {
            return false;
        }
        if let Some(d) = self.min_distance
            && site.distance_sqr < d * d
        {
            return false;
        }
        let day = site.sky_darken < 4;
        match self.time {
            Time::Any => {}
            Time::Day if !day => return false,
            Time::Night if day => return false,
            _ => {}
        }
        let light_ok = match &self.light {
            Light::Any => true,
            Light::Bright => site.block.max(site.sky) > 8,
            Light::Levels { sky_max, block_max } => {
                site.sky <= *sky_max && site.block <= *block_max
            }
            Light::Dark => dark(),
        };
        light_ok && (self.structures.is_empty() || structures(&self.structures))
    }
}

/// The rules a picked biome entry stands for: those with its weight and count, else every rule,
/// else the category's default.
pub fn applicable(rules: &[Rule], default: &Rule, entry: &SpawnEntry) -> Vec<Rule> {
    let same: Vec<Rule> = rules
        .iter()
        .filter(|r| (r.weight, r.min, r.max) == (entry.weight, entry.min, entry.max))
        .cloned()
        .collect();
    if !same.is_empty() {
        same
    } else if !rules.is_empty() {
        rules.to_vec()
    } else {
        vec![default.clone()]
    }
}

fn rules_for(c: &CustomType, entry: &SpawnEntry) -> Vec<Rule> {
    applicable(&c.spawn, &c.default_rule(), entry)
}

fn fluid_at<L: SpawnLevel>(l: &L, x: i32, y: i32, z: i32) -> Fluid {
    l.facts().get(l.state(x, y, z)).fluid
}

/// `SpawnPlacements.isSpawnPositionOk` of one placement.
pub fn placement_ok<L: SpawnLevel>(p: Placement, l: &L, x: i32, y: i32, z: i32) -> bool {
    use crate::mobs::Species::Custom;
    let ground = || {
        spawn_rules::is_valid_spawn(Custom, l, x, y - 1, z)
            && spawn_rules::is_valid_empty_spawn(Custom, l, x, y, z)
            && spawn_rules::is_valid_empty_spawn(Custom, l, x, y + 1, z)
    };
    match p {
        Placement::OnGround => ground(),
        Placement::OnSurface => ground() && l.sky_light(x, y, z) >= 15,
        Placement::InWater => {
            fluid_at(l, x, y, z) == Fluid::Water
                && l.mob_data().state(l.state(x, y + 1, z)).flags & crate::mobs::data::REDSTONE_CONDUCTOR
                    == 0
        }
        Placement::InLava => fluid_at(l, x, y, z) == Fluid::Lava,
        Placement::NoRestrictions => true,
        Placement::Custom => false,
    }
}

/// `isSpawnPositionOk` for a custom type's picked entry: any applicable rule's placement.
pub fn position_ok<L: SpawnLevel>(c: &CustomType, entry: &SpawnEntry, l: &L, x: i32, y: i32, z: i32) -> bool {
    rules_for(c, entry)
        .iter()
        .any(|r| placement_ok(r.placement, l, x, y, z))
}

/// `checkSpawnRules` for a custom type's picked entry: some applicable rule whose placement and
/// conditions hold. `distance_sqr` is the squared distance to the nearest player.
#[allow(clippy::too_many_arguments)]
pub fn rules_ok<L: NaturalLevel, R: RandomSource>(
    c: &CustomType,
    entry: &SpawnEntry,
    l: &L,
    random: &mut R,
    reason: Reason,
    distance_sqr: f64,
    x: i32,
    y: i32,
    z: i32,
) -> bool {
    if c.category == Category::Monster && l.peaceful() {
        return false;
    }
    let site = Site {
        y,
        sky: l.sky_light(x, y, z),
        block: l.block_light(x, y, z),
        sky_darken: if l.thundering() { 10 } else { l.sky_darken() },
        dimension: dimension_id(l.dimension()),
        distance_sqr,
    };
    for rule in rules_for(c, entry) {
        if !placement_ok(rule.placement, l, x, y, z) {
            continue;
        }
        let mut dark = || reason.ignores_light() || spawn_rules::is_dark_enough(l, random, x, y, z);
        let structures = |wanted: &[String]| {
            l.structures_at(x, y, z, &|id| wanted.iter().any(|w| w == id))
                .iter()
                .any(|s| s.in_bounds)
        };
        if rule.allows(&site, &mut dark, &structures) {
            return true;
        }
    }
    false
}

/// `Mob.removeWhenFarAway` of a fresh mob: only a type that is not kept by default.
pub fn removed_when_far(c: &CustomType) -> bool {
    !c.despawn.persistent_by_default
}

/// `Mob.getMaxSpawnClusterSize`: the rule's `cluster`, else 4.
pub fn max_cluster_size(c: &CustomType, entry: &SpawnEntry) -> u32 {
    rules_for(c, entry)
        .iter()
        .filter_map(|r| r.cluster)
        .max()
        .unwrap_or(4)
}

/// `checkSpawnObstruction`: water and lava mobs only need the room, the others no liquid in it.
pub fn obstruction_ok<L: NaturalLevel>(
    c: &CustomType,
    entry: &SpawnEntry,
    l: &L,
    bounds: Aabb,
    spawned: Option<&[Aabb]>,
) -> bool {
    let unobstructed =
        spawned.is_none_or(|s| l.unobstructed(bounds) && !s.iter().any(|b| b.intersects(bounds)));
    let wet = rules_for(c, entry)
        .iter()
        .all(|r| matches!(r.placement, Placement::InWater | Placement::InLava));
    if wet {
        unobstructed
    } else {
        !spawn_selection::contains_any_liquid(l, bounds) && unobstructed
    }
}

/// `despawn`, compiled.
#[derive(Clone, Debug, PartialEq)]
pub struct Despawn {
    /// Like a villager: never removed for being far away (and not counted as removable).
    pub persistent_by_default: bool,
    /// Beyond this many blocks from every player it is removed at once (the category's 128, 64
    /// for water ambient creatures, when absent).
    pub immediate_distance: f64,
    /// Beyond this it may be removed at random after 30 seconds idle (32 when absent).
    pub random_distance: f64,
    /// Within this a player keeps it from idling (32 when absent).
    pub near_player_distance_keep: f64,
    /// Removed after this many ticks of life unless persistent.
    pub age_limit: Option<u32>,
}
impl Despawn {
    pub fn from_file(d: Option<&horizon_mods::custom_mob::Despawn>, category: Category) -> Self {
        let immediate = f64::from(category.despawn_distance());
        let d = d.cloned().unwrap_or_default();
        let blocks = |v: Option<f64>, default: f64| {
            v.filter(|v| v.is_finite() && *v > 0.0).unwrap_or(default)
        };
        Self {
            persistent_by_default: d.persistent_by_default,
            immediate_distance: blocks(d.immediate_distance, immediate),
            random_distance: blocks(d.random_distance, 32.0),
            near_player_distance_keep: blocks(d.near_player_distance_keep, 32.0),
            age_limit: d.age_limit.filter(|a| *a > 0),
        }
    }
}

/// What a despawn check decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    Keep,
    /// Keep, and it counts as having had a player close (`noActionTime = 0`).
    KeepActive,
    Remove,
}

/// `Mob.checkDespawn` for a custom type, as a decision on plain numbers. `roll` is the 1 in 800
/// chance drawn only when it is asked.
pub fn decide(
    d: &Despawn,
    persistent: bool,
    age: u32,
    distance_sqr: f64,
    no_action_time: i32,
    roll: &mut dyn FnMut() -> bool,
) -> Decision {
    let removable = !d.persistent_by_default;
    if !persistent && let Some(limit) = d.age_limit && age > limit {
        return Decision::Remove;
    }
    if !persistent && removable && distance_sqr > d.immediate_distance * d.immediate_distance {
        return Decision::Remove;
    }
    if !persistent
        && removable
        && no_action_time > 600
        && distance_sqr > d.random_distance * d.random_distance
        && roll()
    {
        return Decision::Remove;
    }
    if distance_sqr < d.near_player_distance_keep * d.near_player_distance_keep {
        Decision::KeepActive
    } else {
        Decision::Keep
    }
}

/// `Entity.checkDespawn`: false when the mob is gone. Mirrors the vanilla check in `mobs::ai`:
/// monsters go on peaceful, then the age limit, the distance to the nearest player and the idle
/// roll, with the type's own distances.
pub fn check_despawn(m: &mut Mob, w: &MobWorld, c: &CustomType) -> bool {
    if w.difficulty.effective == 0.0 && c.category == Category::Monster {
        return false;
    }
    let persistent = m.persistent || crate::mobs::ai::requires_custom_persistence(m);
    // Without a player nothing measures the distance; the age limit still runs.
    let distance = w.nearest_player.map_or(f64::INFINITY, |p| m.distance_sqr(p));
    let near = w.nearest_player.is_some();
    let decision = if near {
        let mut roll = || m.random.next_int_bound(800).unwrap_or(1) == 0;
        // A closure borrowing the mob's random cannot also read the mob: copy what is read.
        let (age, idle) = (m.age, m.ai.no_action_time);
        decide(&c.despawn, persistent, age, distance, idle, &mut roll)
    } else {
        decide(&c.despawn, persistent, m.age, f64::NEG_INFINITY, 0, &mut || false)
    };
    match decision {
        Decision::Remove => false,
        Decision::KeepActive => {
            if near {
                m.ai.no_action_time = 0;
            }
            true
        }
        Decision::Keep => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_mods::custom_mob::CustomMobFile;

    fn spawn(json: &str) -> Spawn {
        let v: Value = serde_json::from_str(json).unwrap();
        serde_json::from_value(v).unwrap()
    }
    fn site(y: i32, sky: i32, block: i32, darken: i32, distance: f64) -> Site {
        Site {
            y,
            sky,
            block,
            sky_darken: darken,
            dimension: "minecraft:overworld",
            distance_sqr: distance * distance,
        }
    }
    fn allows(rule: &Rule, s: &Site, dark: bool) -> bool {
        rule.allows(s, &mut || dark, &|_| true)
    }

    #[test]
    fn a_rule_is_read_from_the_file_with_category_defaults() {
        let r = Rule::from_file(&spawn(r##"{"biomes": "#minecraft:is_forest"}"##), Category::Creature).unwrap();
        assert_eq!((r.weight, r.min, r.max), (10, 1, 4));
        assert_eq!(r.placement, Placement::OnGround);
        assert_eq!(r.light, Light::Bright);
        let m = Rule::from_file(&spawn(r#"{"biomes": "a:b"}"#), Category::Monster).unwrap();
        assert_eq!(m.light, Light::Dark);
        let w = Rule::from_file(&spawn(r#"{"biomes": "a:b"}"#), Category::WaterCreature).unwrap();
        assert_eq!((w.placement, w.light), (Placement::InWater, Light::Any));
        let full = Rule::from_file(
            &spawn(
                r#"{"biomes": ["a:b"], "weight": 3, "min": 2, "max": 5, "placement": "on_surface",
                    "light": {"sky_max": 7}, "time": "night", "y_range": [60, 90],
                    "min_distance_from_player": 30, "cluster": 6,
                    "dimensions": "minecraft:overworld", "structures": ["minecraft:village_plains"]}"#,
            ),
            Category::Creature,
        )
        .unwrap();
        assert_eq!(full.placement, Placement::OnSurface);
        assert_eq!(full.light, Light::Levels { sky_max: 7, block_max: 15 });
        assert_eq!((full.time, full.y_range, full.cluster), (Time::Night, Some((60, 90)), Some(6)));
        assert_eq!(full.dimensions, ["minecraft:overworld"]);
        assert_eq!(full.structures, ["minecraft:village_plains"]);
        for bad in [
            r#"{"biomes": "a:b", "light": "dim"}"#,
            r#"{"biomes": "a:b", "light": {"sky_max": 40}}"#,
            r#"{"biomes": "a:b", "time": "dusk"}"#,
            r#"{"biomes": "a:b", "dimensions": 3}"#,
        ] {
            assert!(Rule::from_file(&spawn(bad), Category::Creature).is_err(), "{bad}");
        }
    }

    #[test]
    fn conditions_check_light_time_height_dimension_and_distance() {
        let base = Rule::default_for(Category::Creature);
        // Bright: more than 8 from either light.
        assert!(allows(&base, &site(64, 15, 0, 0, 40.0), false));
        assert!(allows(&base, &site(64, 0, 9, 0, 40.0), false));
        assert!(!allows(&base, &site(64, 8, 8, 0, 40.0), false));
        // Dark asks the vanilla test.
        let mut monster = Rule::default_for(Category::Monster);
        assert!(allows(&monster, &site(64, 0, 0, 0, 40.0), true));
        assert!(!allows(&monster, &site(64, 0, 0, 0, 40.0), false));
        monster.light = Light::Levels { sky_max: 4, block_max: 2 };
        assert!(allows(&monster, &site(64, 4, 2, 0, 40.0), false));
        assert!(!allows(&monster, &site(64, 5, 2, 0, 40.0), false));
        assert!(!allows(&monster, &site(64, 4, 3, 0, 40.0), false));
        // Time of day by the sky's darkening.
        let mut night = Rule::default_for(Category::Ambient);
        night.time = Time::Night;
        assert!(allows(&night, &site(64, 0, 0, 11, 40.0), false));
        assert!(!allows(&night, &site(64, 15, 0, 0, 40.0), false));
        night.time = Time::Day;
        assert!(allows(&night, &site(64, 15, 0, 3, 40.0), false));
        assert!(!allows(&night, &site(64, 15, 0, 4, 40.0), false));
        // Height, dimension, distance.
        let mut r = Rule::default_for(Category::Ambient);
        r.y_range = Some((10, 20));
        r.dimensions = vec!["minecraft:the_nether".into()];
        r.min_distance = Some(30.0);
        assert!(!allows(&r, &site(15, 0, 0, 0, 40.0), false), "overworld");
        r.dimensions.clear();
        assert!(allows(&r, &site(15, 0, 0, 0, 40.0), false));
        assert!(!allows(&r, &site(21, 0, 0, 0, 40.0), false), "too high");
        assert!(!allows(&r, &site(15, 0, 0, 0, 29.0), false), "too near a player");
        // Structures are asked only for a rule that lists some.
        r.structures = vec!["minecraft:village_plains".into()];
        assert!(!r.allows(&site(15, 0, 0, 0, 40.0), &mut || true, &|_| false));
        assert!(r.allows(&site(15, 0, 0, 0, 40.0), &mut || true, &|s| s == ["minecraft:village_plains"]));
    }

    #[test]
    fn a_picked_entry_finds_its_rules_by_weight_and_count() {
        let entry = |weight, min, max| SpawnEntry { entity: "a:b".into(), weight, min, max, constant: false };
        let mut a = Rule::default_for(Category::Creature);
        (a.weight, a.min, a.max) = (6, 1, 3);
        let mut b = Rule::default_for(Category::Creature);
        (b.weight, b.min, b.max) = (2, 1, 1);
        let rules = [a.clone(), b.clone()];
        let default = Rule::default_for(Category::Creature);
        assert_eq!(applicable(&rules, &default, &entry(6, 1, 3)), [a.clone()]);
        assert_eq!(applicable(&rules, &default, &entry(2, 1, 1)), [b.clone()]);
        // Weights a mod changed: every rule stands for it.
        assert_eq!(applicable(&rules, &default, &entry(99, 1, 3)).len(), 2);
        // No rules: the category's.
        assert_eq!(applicable(&[], &default, &entry(1, 1, 1)), [default]);
    }

    #[test]
    fn a_type_builds_its_rules_and_refuses_one_the_engine_cannot_read() {
        let file = CustomMobFile::parse(
            "demo:thing",
            br##"{"template":"yoake:mob","size":{"width":1,"height":1},
                "spawn":[{"biomes":"#minecraft:is_forest","weight":6,"min":1,"max":3,"light":"bright"}]}"##,
        )
        .unwrap();
        let t = CustomType::new(file, None).unwrap();
        assert_eq!(t.spawn.len(), 1);
        assert_eq!(t.spawn[0].light, Light::Bright);
        let bad = CustomMobFile::parse(
            "demo:thing",
            br##"{"template":"yoake:mob","size":{"width":1,"height":1},
                "spawn":[{"biomes":"a:b","time":"dusk"}]}"##,
        )
        .unwrap();
        assert!(CustomType::new(bad, None).unwrap_err().contains("dusk"));
    }

    #[test]
    fn despawn_follows_the_vanilla_shape_with_the_types_distances() {
        let d = Despawn::from_file(None, Category::Creature);
        assert_eq!(
            (d.immediate_distance, d.random_distance, d.near_player_distance_keep),
            (128.0, 32.0, 32.0)
        );
        assert_eq!(Despawn::from_file(None, Category::WaterAmbient).immediate_distance, 64.0);
        let mut yes = || true;
        let mut no = || false;
        // Far: removed unless persistent.
        assert_eq!(decide(&d, false, 100, 129.0 * 129.0, 0, &mut no), Decision::Remove);
        assert_eq!(decide(&d, true, 100, 129.0 * 129.0, 0, &mut no), Decision::Keep);
        // Idle for a while and farther than 32: a roll decides.
        assert_eq!(decide(&d, false, 100, 40.0 * 40.0, 601, &mut yes), Decision::Remove);
        assert_eq!(decide(&d, false, 100, 40.0 * 40.0, 601, &mut no), Decision::Keep);
        assert_eq!(decide(&d, false, 100, 40.0 * 40.0, 600, &mut yes), Decision::Keep);
        // Near a player the idle clock restarts.
        assert_eq!(decide(&d, false, 100, 10.0 * 10.0, 900, &mut yes), Decision::KeepActive);
        // A type kept by default is never removed for distance or idling.
        let kept = Despawn { persistent_by_default: true, ..d.clone() };
        assert_eq!(decide(&kept, false, 100, 500.0 * 500.0, 900, &mut yes), Decision::Keep);
        // Its own distances and age limit.
        let own = Despawn {
            immediate_distance: 50.0,
            random_distance: 20.0,
            near_player_distance_keep: 10.0,
            age_limit: Some(200),
            ..d
        };
        assert_eq!(decide(&own, false, 100, 51.0 * 51.0, 0, &mut no), Decision::Remove);
        assert_eq!(decide(&own, false, 201, 5.0, 0, &mut no), Decision::Remove);
        assert_eq!(decide(&own, true, 201, 5.0, 0, &mut no), Decision::KeepActive);
        assert_eq!(decide(&own, false, 100, 15.0 * 15.0, 0, &mut no), Decision::Keep);
    }
}
