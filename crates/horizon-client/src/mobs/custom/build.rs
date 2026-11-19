//! Building custom mobs: the [`CustomType`]s from the content root, the engine's [`MobType`] of
//! one, and a [`Mob`] of a type (its state and goals). FR-F4-MOBS.md sections 3.2 and 5.
//!
//! The content root carries the mods' custom mob files in
//! [`CUSTOM_MOBS_FILE`](horizon_mods::custom_mob::CUSTOM_MOBS_FILE) (written by `mod_content.rs`);
//! [`add`] reads them when the mob types load, so the client, the host and a dedicated server
//! build the same types. A file the engine cannot build is skipped with a warning (as a bad row
//! of `horizon-mod-mobs.json` is), never a crash.
use super::{CustomState, CustomType, registry};
use crate::mobs::{
    Mob, MobType, Part, Species,
    goals::{
        Avoid, AvoidEntity, Float, HurtByTarget, LookAtPlayer, MeleeAttack, NearestTarget, Panic,
        RandomLookAround, Stroll, Tempt,
    },
    species,
};
use horizon_mods::custom_mob::{CUSTOM_MOBS_FILE, CustomMobFile, Goal};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
};

/// The custom mob files of `root` as types, models read through the real loaders from the same
/// root. Bad ones are logged and left out.
pub fn load_root(root: &Path) -> Vec<Arc<CustomType>> {
    let Ok(bytes) = std::fs::read(root.join(CUSTOM_MOBS_FILE)) else {
        return Vec::new();
    };
    let rows: Vec<serde_json::Value> = match serde_json::from_slice(&bytes) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("Horizon: {CUSTOM_MOBS_FILE}: {e}");
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for mut row in rows {
        let Some(id) = row
            .as_object_mut()
            .and_then(|o| o.remove("id"))
            .and_then(|id| id.as_str().map(str::to_owned))
        else {
            log::warn!("Horizon: {CUSTOM_MOBS_FILE}: a custom mob without id");
            continue;
        };
        match build_one(root, &id, &row.to_string()) {
            Ok(t) => out.push(Arc::new(t)),
            Err(e) => log::warn!("Horizon: custom mob {id} not loaded: {e}"),
        }
    }
    out
}

fn build_one(root: &Path, id: &str, json: &str) -> Result<CustomType, String> {
    let file = CustomMobFile::parse(id, json.as_bytes())?;
    let model = file.load_model(&mut |path| std::fs::read(root.join(path)).map_err(|e| format!("{path}: {e}")))?;
    let unrun: Vec<&str> = file
        .ai
        .goals
        .iter()
        .chain(&file.ai.targets)
        .filter(|g| !is_basic_goal(g))
        .map(|g| g.kind.as_str())
        .collect();
    if !unrun.is_empty() {
        log::warn!(
            "Horizon: custom mob {id}: the goals {unrun:?} are not run by this version yet; the others are"
        );
    }
    CustomType::new(file, Some(&model))
}

/// Adds the custom types of `root` to `types` (each as the engine's [`MobType`]) and to the
/// registry. A root without custom mobs leaves both alone: vanilla roots load beside a modded one.
pub fn add(root: &Path, types: &mut HashMap<String, MobType>) {
    let custom = load_root(root);
    if custom.is_empty() {
        return;
    }
    for t in &custom {
        types.insert(t.id.clone(), mob_type(t));
    }
    if let Ok(mut table) = registry::registry().write() {
        table.replace(custom);
    }
}

/// `entity.fox.ambient` for `minecraft:entity.fox.ambient`: the engine's mob sounds are vanilla
/// events without their namespace. A sound of another namespace has no path in the mob events
/// yet: it is left out (silent).
fn vanilla_sound(slot: &str, id: &str) -> &'static str {
    match id.strip_prefix("minecraft:") {
        Some(name) => intern(name),
        None => {
            log::warn!("Horizon: sound {slot}: {id} is not a minecraft: sound event; custom mob sounds are vanilla ones for now");
            ""
        }
    }
}

/// Event names are `&'static str` in the mob types: each distinct name is kept once.
fn intern(name: &str) -> &'static str {
    static POOL: Mutex<Option<HashSet<&'static str>>> = Mutex::new(None);
    let mut pool = POOL.lock().unwrap_or_else(|e| e.into_inner());
    let pool = pool.get_or_insert_with(HashSet::new);
    if let Some(known) = pool.get(name) {
        return known;
    }
    let leaked: &'static str = Box::leak(name.to_owned().into_boxed_str());
    pool.insert(leaked);
    leaked
}

/// The engine's type of a custom mob: the neutral row of `Species::Custom` with the file's box,
/// attributes, sounds and category. It has no vanilla model (the client renderer is F4M-6).
pub fn mob_type(t: &Arc<CustomType>) -> MobType {
    let def = species::def(Species::Custom);
    let f = &t.file;
    let value = |name: &str, default: f64| t.builtin_value(name).unwrap_or(default);
    let mut attributes: HashMap<String, f64> = t
        .builtin
        .iter()
        .map(|(name, v)| (format!("minecraft:{name}"), *v))
        .collect();
    for a in &t.attributes {
        attributes.insert(a.id.clone(), a.default);
    }
    let sound = |slot: &str| {
        f.sounds
            .get(slot)
            .map_or("", |id| vanilla_sound(slot, id))
    };
    MobType {
        custom: Some(t.clone()),
        species: Species::Custom,
        width: f.size.width,
        height: f.size.height,
        eye_height: f.size.eye_height.unwrap_or(f.size.height * 0.85),
        max_health: value("max_health", 10.0) as f32,
        speed: value("movement_speed", 0.25),
        flying_speed: value("flying_speed", 0.4),
        attack_damage: value("attack_damage", 0.0) as f32,
        attack_knockback: value("attack_knockback", 0.0),
        follow_range: value("follow_range", 16.0),
        step_height: value("step_height", 0.6),
        armor: value("armor", 0.0) as f32,
        armor_toughness: value("armor_toughness", 0.0) as f32,
        knockback_resistance: value("knockback_resistance", 0.0),
        jump_strength: value("jump_strength", 0.42),
        attributes: Arc::new(attributes),
        gravity: value("gravity", 0.08),
        safe_fall_distance: value("safe_fall_distance", 3.0),
        fall_damage_multiplier: value("fall_damage_multiplier", 1.0),
        water_efficiency: 0.0,
        water_slowdown: t
            .builtin_value("water_slowdown")
            .map_or_else(|| def.water_slowdown(), |v| v as f32),
        air_speed: def.air_speed(),
        baby_scale: def.baby_scale(),
        head_rot_speed: def.head_rot_speed(),
        max_head_x_rot: def.max_head_x_rot(),
        max_head_y_rot: def.max_head_y_rot(),
        ambient_interval: def.ambient_interval(),
        flags: def.flags(false),
        model: Arc::new(Part::parse("root", &serde_json::Value::Null)),
        layers: Default::default(),
        texture: f.model.textures.default.clone().unwrap_or_default(),
        brain: def.brain(),
        anim: def.anim,
        sounds: [sound("ambient"), sound("hurt"), sound("death")],
        step_sound: Some(sound("step")).filter(|s| !s.is_empty()),
        food: Arc::from(Vec::new()),
        tempt: Arc::from(Vec::new()),
        category: f.category.clone(),
        id: t.id.clone(),
    }
}

/// The goals this version runs: the nine the data mobs had (`float`, `wander`, `look_at_player`,
/// `random_look`, `panic`, `tempt`, `flee`, `melee_attack`, `target_player`, `hurt_by_target`)
/// and their new names (`swim`, `stroll`, `avoid`, `hurt_by`, `nearest_target`/
/// `nearest_attackable` at players). The rest of the schema's goals are F4M-4a's.
pub fn is_basic_goal(g: &Goal) -> bool {
    match g.kind.as_str() {
        "float" | "swim" | "float_in_water" | "wander" | "stroll" | "look_at_player"
        | "random_look" | "panic" | "tempt" | "flee" | "avoid" | "melee_attack"
        | "target_player" | "hurt_by_target" | "hurt_by" => true,
        "nearest_target" | "nearest_attackable" => targets_player(g),
        _ => false,
    }
}

fn targets_player(g: &Goal) -> bool {
    g.params
        .get("types")
        .and_then(|t| t.as_array())
        .is_some_and(|t| t.iter().any(|v| v.as_str() == Some("minecraft:player")))
}

fn num(g: &Goal, key: &str, default: f64) -> f64 {
    g.params
        .get(key)
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite())
        .unwrap_or(default)
}
fn strings(g: &Goal, key: &str) -> Vec<String> {
    g.params
        .get(key)
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}
fn species_of(id: &str) -> Option<Species> {
    let name = id.strip_prefix("minecraft:")?;
    species::ALL.iter().find(|d| d.name == name).map(|d| d.species)
}

/// Gives the mob its goals and targets from the file: the ones [`is_basic_goal`] names, by
/// the file's priorities (lower first). Speeds are the file's `speed`.
pub fn attach_goals(m: &mut Mob, t: &CustomType) {
    for g in &t.file.ai.goals {
        let p = g.priority;
        let speed = g.speed;
        match g.kind.as_str() {
            "float" | "swim" | "float_in_water" => m.ai.goals.add(p, Float),
            "wander" | "stroll" => m
                .ai
                .goals
                .add(p, Stroll::avoiding_water(speed.unwrap_or(1.0))),
            "look_at_player" => m
                .ai
                .goals
                .add(p, LookAtPlayer::new(num(g, "range", 8.0) as f32)),
            "random_look" => m.ai.goals.add(p, RandomLookAround::default()),
            "panic" => m.ai.goals.add(p, Panic::new(speed.unwrap_or(1.25))),
            "tempt" => {
                let items: Arc<[String]> = strings(g, "items").into();
                m.ai
                    .goals
                    .add(p, Tempt::new(speed.unwrap_or(1.0), items, false));
            }
            "flee" | "avoid" => {
                let from = strings(g, "from");
                let player = from.is_empty() || from.iter().any(|f| f == "minecraft:player");
                let what = if player {
                    Avoid::Player
                } else {
                    Avoid::Species(from.iter().filter_map(|f| species_of(f)).collect())
                };
                m.ai.goals.add(
                    p,
                    AvoidEntity::new(
                        what,
                        num(g, "distance", 8.0) as f32,
                        num(g, "walk_speed", 1.0),
                        num(g, "sprint_speed", 1.2),
                    ),
                );
            }
            "melee_attack" => m
                .ai
                .goals
                .add(p, MeleeAttack::new(speed.unwrap_or(1.0), false)),
            _ => {}
        }
    }
    for g in &t.file.ai.targets {
        let p = g.priority;
        match g.kind.as_str() {
            "target_player" => m
                .ai
                .targets
                .add(p, NearestTarget::player(g.params.get("must_see").and_then(|v| v.as_bool()).unwrap_or(true))),
            "nearest_target" | "nearest_attackable" if targets_player(g) => m
                .ai
                .targets
                .add(p, NearestTarget::player(g.params.get("must_see").and_then(|v| v.as_bool()).unwrap_or(true))),
            "hurt_by_target" | "hurt_by" => m.ai.targets.add(p, HurtByTarget::new()),
            _ => {}
        }
    }
}

/// `mod_mob::register` for a custom mob: its per-mob state and its goals. False for any other.
pub fn register(m: &mut Mob) -> bool {
    let Some(t) = m.kind.custom.clone() else {
        return false;
    };
    m.extra.custom = Some(Box::new(CustomState::new(&t)));
    attach_goals(m, &t);
    true
}

#[cfg(test)]
mod tests {
    use super::super::tests::fox_type;
    use super::*;
    use horizon_core::geometry::Vec3;

    pub(crate) fn fox() -> Arc<MobType> {
        Arc::new(mob_type(&Arc::new(fox_type())))
    }

    #[test]
    fn a_custom_type_becomes_the_engines_type_with_the_files_numbers() {
        let kind = fox();
        assert_eq!(kind.id, "sample_mobs:cinder_fox");
        assert_eq!(kind.species, Species::Custom);
        assert_eq!((kind.width, kind.height, kind.eye_height), (0.6, 0.7, 0.5));
        assert_eq!((kind.max_health, kind.speed), (14.0, 0.3));
        assert_eq!(kind.category, "creature");
        assert_eq!(kind.attributes["minecraft:max_health"], 14.0);
        assert_eq!(kind.attributes["sample_mobs:warmth"], 0.0);
        // Defaults for what the file leaves out are the engine's own mob defaults.
        assert_eq!((kind.follow_range, kind.step_height, kind.gravity), (16.0, 0.6, 0.08));
        // Vanilla sound events lose their namespace; the engine adds it again.
        assert_eq!(
            kind.sounds,
            ["entity.fox.ambient", "entity.fox.hurt", "entity.fox.death"]
        );
        assert!(kind.step_sound.is_none());
        assert_eq!(kind.texture, "sample_mobs:entity/cinder_fox");
        assert!(kind.custom.is_some());
        // No flags of any vanilla family.
        assert!(!kind.flags.animal && !kind.flags.undead);
    }

    #[test]
    fn a_custom_mob_is_made_ticks_and_has_its_state_and_goals() {
        let kind = fox();
        let mut m = Mob::new(7, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        assert_eq!(m.species(), Species::Custom);
        assert_eq!(m.health, 14.0);
        assert_eq!(m.attrs.max_health, 14.0);
        let state = m.extra.custom.as_ref().expect("its state");
        assert_eq!(state.synced.len(), 2);
        // The goals of the file this version runs (the schema's script goal and mob targets are
        // not), by the file's priorities.
        for g in ["Float", "Stroll", "LookAtPlayer", "RandomLookAround"] {
            assert!(m.ai.goals.has(g), "{g}");
        }
        assert!(!m.ai.goals.has("Breed"), "no vanilla family's goals");
        assert!(!m.hostile());
        // It hurts like a mob: a hit takes health.
        let source = crate::mobs::Damage::of(crate::mobs::DamageKind::Generic);
        assert!(m.damage(4.0, source));
        assert_eq!(m.health, 10.0);
        assert!(m.alive());
    }

    #[test]
    fn a_vanilla_mob_is_not_a_custom_one() {
        let cow = (*crate::mobs::sensor_scan_tests::kind()).clone();
        assert!(cow.custom.is_none());
        let mut m = Mob::new(1, Arc::new(cow), Vec3::ZERO, 0.0);
        assert!(m.extra.custom.is_none());
        assert!(!register(&mut m));
    }
}
