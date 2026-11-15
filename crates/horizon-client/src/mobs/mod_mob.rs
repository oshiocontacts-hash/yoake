//! Content mods' mob types (`data/<id>/horizon/entity_type/<name>.json` with a
//! mob template, docs/mods.md "Mob types"): a copy of a vanilla ground mob's
//! type with the mod's id, texture, size, attributes and goals, loot table and
//! (through a biome modifier the content root builds) natural spawns.
//!
//! The content root lists them in [`MOBS_FILE`]; [`MobTypes::load`] builds the
//! types from it, so the client, the host and a dedicated server all agree.
//! A mob that lists `behaviours` has exactly those goals; one that doesn't
//! keeps its template's. What the template's species does apart from its goals
//! (a zombie burns in sunlight, a cow can be milked) stays.
use super::{
    Mob, MobType, Species,
    goals::{
        Avoid, AvoidEntity, Float, HurtByTarget, LookAtPlayer, MeleeAttack, NearestTarget, Panic,
        RandomLookAround, Stroll, Tempt,
    },
    species,
};
use horizon_mods::content::{Behaviour, EntityType, MOBS_FILE};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, RwLock},
};

/// The mob types the loaded content root defines, by id.
static MOBS: RwLock<Option<HashMap<String, Arc<EntityType>>>> = RwLock::new(None);

/// A mod mob type's definition, if `id` is one.
pub fn spec(id: &str) -> Option<Arc<EntityType>> {
    MOBS.read().ok()?.as_ref()?.get(id).cloned()
}

/// The ids of the mod mob types, sorted.
pub fn ids() -> Vec<String> {
    let mut ids: Vec<String> = MOBS
        .read()
        .ok()
        .and_then(|m| m.as_ref().map(|m| m.keys().cloned().collect()))
        .unwrap_or_default();
    ids.sort();
    ids
}

/// The loot table a mod mob type's death rolls: its own, else
/// `<namespace>:entities/<path>` as vanilla names an entity type's.
pub fn loot_table(id: &str) -> Option<String> {
    let spec = spec(id)?;
    Some(spec.loot_table.clone().unwrap_or_else(|| {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        format!("{ns}:entities/{path}")
    }))
}

/// The mod mob definitions in `root`'s [`MOBS_FILE`]: `(id, definition)`.
pub fn read(root: &Path) -> Vec<(String, EntityType)> {
    let Ok(bytes) = std::fs::read(root.join(MOBS_FILE)) else {
        return Vec::new();
    };
    let rows: Vec<serde_json::Value> = match serde_json::from_slice(&bytes) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("Horizon: {MOBS_FILE}: {e}");
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
            log::warn!("Horizon: {MOBS_FILE}: a mob type without id");
            continue;
        };
        match serde_json::from_value::<EntityType>(row) {
            Ok(def) => out.push((id, def)),
            Err(e) => log::warn!("Horizon: {MOBS_FILE}: {id}: {e}"),
        }
    }
    out
}

/// Adds the mod mob types of `root` to `types`, each a copy of its template's
/// (built from the vanilla types already in `types`), and remembers their
/// definitions for [`register`] and [`loot_table`].
pub fn add(root: &Path, types: &mut HashMap<String, MobType>) {
    let defs = read(root);
    let mut specs = HashMap::new();
    for (id, def) in defs {
        let Some(template) = types.get(&def.template) else {
            log::warn!("Horizon: mob type {id}: template {} is not loaded", def.template);
            continue;
        };
        let mut t = template.clone();
        t.id = id.clone();
        if let Some(texture) = &def.texture {
            t.texture = texture.clone();
        }
        if let Some(w) = def.width {
            t.width = w;
        }
        if let Some(h) = def.height {
            t.height = h;
        }
        if let Some(e) = def.eye_height {
            t.eye_height = e;
        } else if def.height.is_some() {
            // the eyes keep their place in the box when it is resized.
            let was = template.eye_height / template.height.max(0.01);
            t.eye_height = t.height * was;
        }
        let mut attributes = (*t.attributes).clone();
        for (name, &value) in &def.attributes {
            attributes.insert(format!("minecraft:{name}"), value);
            match name.as_str() {
                "max_health" => t.max_health = value as f32,
                "movement_speed" => t.speed = value,
                "attack_damage" => t.attack_damage = value as f32,
                "attack_knockback" => t.attack_knockback = value,
                "armor" => t.armor = value as f32,
                "armor_toughness" => t.armor_toughness = value as f32,
                "knockback_resistance" => t.knockback_resistance = value,
                "follow_range" => t.follow_range = value,
                _ => {}
            }
        }
        t.attributes = Arc::new(attributes);
        specs.insert(id.clone(), Arc::new(def));
        types.insert(id, t);
    }
    // A root without mod mobs leaves them alone: vanilla roots load beside a modded one.
    if specs.is_empty() {
        return;
    }
    if let Ok(mut m) = MOBS.write() {
        *m = Some(specs);
    }
}

/// The species of a vanilla mob id (`minecraft:wolf`).
fn species_of(id: &str) -> Option<Species> {
    let name = id.strip_prefix("minecraft:")?;
    species::ALL.iter().find(|d| d.name == name).map(|d| d.species)
}

/// Gives a mod mob that lists behaviours exactly those goals and targets;
/// false (nothing done) for any other mob.
pub fn register(m: &mut Mob) -> bool {
    let Some(spec) = spec(&m.kind.id) else {
        return false;
    };
    let Some(list) = &spec.behaviours else {
        return false;
    };
    let mut targets = 0;
    for (priority, behaviour) in list.iter().enumerate() {
        let priority = priority as i32;
        match behaviour {
            Behaviour::Float => m.ai.goals.add(priority, Float),
            Behaviour::Wander { speed } => m.ai.goals.add(priority, Stroll::avoiding_water(*speed)),
            Behaviour::LookAtPlayer { range } => {
                m.ai.goals.add(priority, LookAtPlayer::new(*range))
            }
            Behaviour::RandomLook => m.ai.goals.add(priority, RandomLookAround::default()),
            Behaviour::Panic { speed } => m.ai.goals.add(priority, Panic::new(*speed)),
            Behaviour::Tempt { items, speed } => {
                let items: Arc<[String]> = items.clone().into();
                m.ai.goals.add(priority, Tempt::new(*speed, items, false));
            }
            Behaviour::Flee {
                from,
                distance,
                walk_speed,
                sprint_speed,
            } => {
                let player = from.iter().any(|f| f == "minecraft:player");
                let mobs: Vec<Species> = from.iter().filter_map(|f| species_of(f)).collect();
                let what = if player { Avoid::Player } else { Avoid::Species(mobs) };
                m.ai.goals.add(
                    priority,
                    AvoidEntity::new(what, *distance, *walk_speed, *sprint_speed),
                );
            }
            Behaviour::MeleeAttack { speed } => {
                m.ai.goals.add(priority, MeleeAttack::new(*speed, false))
            }
            Behaviour::TargetPlayer { must_see } => {
                targets += 1;
                m.ai.targets.add(targets, NearestTarget::player(*must_see));
            }
            Behaviour::HurtByTarget => {
                targets += 1;
                m.ai.targets.add(targets, HurtByTarget::new());
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str, text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("horizon-mod-mob-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MOBS_FILE), text).unwrap();
        dir
    }

    #[test]
    fn the_mobs_file_is_read_with_ids_and_bad_rows_are_skipped() {
        let dir = root(
            "read",
            r#"[
                {"id": "ruby:cow", "template": "minecraft:cow", "item": "", "damage": 0.0,
                 "attributes": {"max_health": 30.0}, "spawn_egg": false},
                {"template": "minecraft:cow"},
                {"id": "ruby:bad", "template": "minecraft:cow", "wings": 2}
            ]"#,
        );
        let rows = read(&dir);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].0, "ruby:cow");
        assert_eq!(rows[0].1.attributes["max_health"], 30.0);
        assert!(read(&dir.join("none")).is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_mod_mob_is_its_templates_copy_with_its_own_stats_and_goals() {
        use horizon_core::geometry::Vec3;
        let dir = root(
            "add",
            r#"[
                {"id": "ruby:gem_cow", "template": "minecraft:cow",
                 "texture": "ruby:entity/gem_cow", "width": 2.0, "height": 2.8,
                 "attributes": {"max_health": 40.0, "movement_speed": 0.3},
                 "behaviours": [{"type": "float"},
                                {"type": "tempt", "items": ["minecraft:wheat"]}]},
                {"id": "ruby:plain_cow", "template": "minecraft:cow"},
                {"id": "ruby:lost", "template": "minecraft:unicorn"}
            ]"#,
        );
        let cow = (*crate::mobs::sensor_scan_tests::kind()).clone();
        let mut types = HashMap::from([("minecraft:cow".to_owned(), cow.clone())]);
        add(&dir, &mut types);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!types.contains_key("ruby:lost"));
        let t = &types["ruby:gem_cow"];
        assert_eq!((t.id.as_str(), t.species), ("ruby:gem_cow", Species::Cow));
        assert_eq!(t.texture, "ruby:entity/gem_cow");
        assert_eq!((t.width, t.height), (2.0, 2.8));
        // the eyes keep their place in a resized box.
        assert!((t.eye_height - 1.3 / 1.4 * 2.8).abs() < 1e-9);
        assert_eq!((t.max_health, t.speed), (40.0, 0.3));
        assert_eq!(t.attributes["minecraft:max_health"], 40.0);
        assert_eq!(types["minecraft:cow"].max_health, cow.max_health);
        assert_eq!(ids(), ["ruby:gem_cow", "ruby:plain_cow"]);
        assert_eq!(
            loot_table("ruby:gem_cow").as_deref(),
            Some("ruby:entities/gem_cow")
        );
        assert_eq!(loot_table("minecraft:cow"), None);

        let mob = |id: &str| Mob::new(1, Arc::new(types[id].clone()), Vec3::ZERO, 0.0);
        // its behaviours replace the template's goals.
        let own = mob("ruby:gem_cow");
        assert!(own.ai.goals.has("Float") && own.ai.goals.has("Tempt"));
        assert!(!own.ai.goals.has("Breed"));
        // without them it keeps its template's.
        assert!(mob("ruby:plain_cow").ai.goals.has("Breed"));
    }

    #[test]
    fn a_vanilla_mob_id_names_its_species() {
        assert_eq!(species_of("minecraft:pig"), Some(Species::Pig));
        assert_eq!(species_of("ruby:pig"), None);
        assert_eq!(species_of("minecraft:nothing"), None);
    }
}
