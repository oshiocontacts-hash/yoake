//! Fully custom mobs (FR-F4-MOB, docs/yoake2/FR-F4-MOBS.md). F4M-0 laid the foundation (the shared
//! immutable [`CustomType`], the [`registry`], `Species::Custom`); F4M-3 fills it: a type is built
//! from the validated JSON (`registry` holds them, `build` makes the engine's `MobType` and a mob's
//! per-instance state), with its custom attributes (`attrs`), hit and pick parts (`hitbox`), natural
//! spawn and despawn rules (`spawn`) and its save format (`persist`).
//!
//! With no mod there is no custom type: every `MobType.custom` is `None`, the registry is empty and
//! none of this code runs.
//!
//! New code for custom mobs lives under this directory (and `game/custom_mob_*.rs`) so that
//! parallel work items do not collide in the shared mob files.
pub mod ai;
pub mod attrs;
pub mod build;
pub mod hitbox;
pub mod persist;
pub mod registry;
pub mod spawn;

pub use registry::{CustomRegistry, get, ids, registry};

use super::spawn_selection::Category;
use attrs::{AttrDef, AttrSet, Sync};
use horizon_mods::custom_mob::{
    Attribute, BUILTIN_ATTRIBUTES, CustomMobFile, LoadedModel, Synced,
};
use serde_json::Value;
use std::collections::BTreeMap;

/// The value of one tracked (synced) value of a mob.
#[derive(Clone, Debug, PartialEq)]
pub enum SyncedValue {
    Bool(bool),
    I32(i32),
    F32(f32),
    Str(String),
    Vec3([f32; 3]),
    /// The index of the value's name in the type's list.
    Enum(u8),
}

/// One tracked value of a type, in slot order (by name).
#[derive(Clone, Debug, PartialEq)]
pub struct SyncedDef {
    pub name: String,
    pub kind: String,
    pub default: SyncedValue,
    pub min: Option<i32>,
    pub max: Option<i32>,
    pub values: Vec<String>,
    pub persistent: bool,
    pub sync: Sync,
}

impl SyncedDef {
    fn new(name: &str, s: &Synced) -> Result<Self, String> {
        let mut def = Self {
            name: name.to_owned(),
            kind: s.kind.clone(),
            default: SyncedValue::Bool(false),
            min: s.min,
            max: s.max,
            values: s.values.clone(),
            persistent: s.persistent,
            sync: Sync::parse(&s.sync),
        };
        def.default = def.from_json(s.default.as_ref())?;
        Ok(def)
    }

    /// A JSON value as this value's type (a missing one is the type's zero).
    pub fn from_json(&self, v: Option<&Value>) -> Result<SyncedValue, String> {
        let bad = || format!("synced {}: {:?} is not a {}", self.name, v, self.kind);
        Ok(match self.kind.as_str() {
            "bool" => SyncedValue::Bool(match v {
                None => false,
                Some(v) => v.as_bool().ok_or_else(bad)?,
            }),
            "i32" => {
                let n = match v {
                    None => 0,
                    Some(v) => v.as_i64().ok_or_else(bad)?,
                };
                SyncedValue::I32(self.clamp_i32(n))
            }
            "f32" => SyncedValue::F32(match v {
                None => 0.0,
                Some(v) => v.as_f64().ok_or_else(bad)? as f32,
            }),
            "string" => {
                let s = match v {
                    None => "",
                    Some(v) => v.as_str().ok_or_else(bad)?,
                };
                SyncedValue::Str(truncate(
                    s,
                    horizon_mods::custom_mob::limits::MAX_SYNCED_STRING,
                ))
            }
            "vec3" => match v {
                None => SyncedValue::Vec3([0.0; 3]),
                Some(v) => {
                    let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(bad)?;
                    let mut out = [0.0f32; 3];
                    for (o, n) in out.iter_mut().zip(a) {
                        *o = n.as_f64().ok_or_else(bad)? as f32;
                    }
                    SyncedValue::Vec3(out)
                }
            },
            "enum" => match v {
                None => SyncedValue::Enum(0),
                Some(Value::String(name)) => {
                    SyncedValue::Enum(self.values.iter().position(|n| n == name).ok_or_else(bad)? as u8)
                }
                Some(_) => return Err(bad()),
            },
            _ => return Err(bad()),
        })
    }

    fn clamp_i32(&self, n: i64) -> i32 {
        let n = n.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
        n.clamp(self.min.unwrap_or(i32::MIN), self.max.unwrap_or(i32::MAX))
    }

    /// `value` as this type's value (cut to its range): `None` for a value of another type.
    pub fn coerce(&self, value: SyncedValue) -> Option<SyncedValue> {
        Some(match (self.kind.as_str(), value) {
            ("bool", v @ SyncedValue::Bool(_)) => v,
            ("i32", SyncedValue::I32(n)) => SyncedValue::I32(self.clamp_i32(i64::from(n))),
            ("f32", SyncedValue::F32(f)) if f.is_finite() => SyncedValue::F32(f),
            ("string", SyncedValue::Str(s)) => SyncedValue::Str(truncate(
                &s,
                horizon_mods::custom_mob::limits::MAX_SYNCED_STRING,
            )),
            ("vec3", SyncedValue::Vec3(v)) if v.iter().all(|f| f.is_finite()) => {
                SyncedValue::Vec3(v)
            }
            ("enum", SyncedValue::Enum(i)) if usize::from(i) < self.values.len() => {
                SyncedValue::Enum(i)
            }
            _ => return None,
        })
    }
}

/// The longest prefix of `s` of at most `max` bytes that ends on a character.
fn truncate(s: &str, max: usize) -> String {
    let mut end = s.len().min(max);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_owned()
}

/// One custom mob type: loaded identically on client, host and dedicated server, shared by every
/// mob of the type behind an `Arc`.
#[derive(Debug)]
pub struct CustomType {
    /// `<namespace>:<path>`.
    pub id: String,
    /// The validated definition as written in the mod's JSON.
    pub file: CustomMobFile,
    pub category: Category,
    /// The type's own (namespaced) attributes, by id.
    pub attributes: Vec<AttrDef>,
    /// The built-in attributes the file sets, by bare name.
    pub builtin: BTreeMap<String, f64>,
    pub hitboxes: Vec<hitbox::HitPart>,
    pub spawn: Vec<spawn::Rule>,
    pub despawn: spawn::Despawn,
    /// The loot table its death rolls.
    pub loot_table: String,
    /// The tracked values in slot order (by name).
    pub synced: Vec<SyncedDef>,
}

impl CustomType {
    /// Builds the type from its validated file. `model` is the loaded model (hitbox parts that
    /// follow a bone need it).
    pub fn new(file: CustomMobFile, model: Option<&LoadedModel>) -> Result<Self, String> {
        let category = Category::from_name(&file.category);
        let mut attributes = Vec::new();
        let mut builtin = BTreeMap::new();
        for (name, value) in &file.attributes {
            let known = BUILTIN_ATTRIBUTES.contains(&name.as_str());
            match value {
                Attribute::Value(v) if known => {
                    builtin.insert(name.clone(), *v);
                }
                Attribute::Value(v) => attributes.push(AttrDef {
                    id: name.clone(),
                    default: *v,
                    min: None,
                    max: None,
                    sync: Sync::All,
                }),
                Attribute::Custom(c) if !known => attributes.push(AttrDef {
                    id: name.clone(),
                    default: c.default,
                    min: c.min,
                    max: c.max,
                    sync: Sync::parse(&c.sync),
                }),
                Attribute::Custom(_) => {
                    return Err(format!("attribute {name}: a built-in one is a number"));
                }
            }
        }
        // Their definitions must hold (a bad range is the file's error).
        AttrSet::new(&attributes)?;
        let hitboxes = file
            .hitboxes
            .iter()
            .map(|h| hitbox::HitPart::resolve(h, model))
            .collect::<Result<Vec<_>, _>>()?;
        let spawn = file
            .spawn
            .iter()
            .map(|s| spawn::Rule::from_file(s, category))
            .collect::<Result<Vec<_>, _>>()?;
        let despawn = spawn::Despawn::from_file(file.despawn.as_ref(), category);
        let synced = file
            .synced
            .iter()
            .map(|(name, s)| SyncedDef::new(name, s))
            .collect::<Result<Vec<_>, _>>()?;
        let loot_table = file.loot_table.clone().unwrap_or_else(|| {
            let (ns, path) = file.id.split_once(':').unwrap_or(("minecraft", &file.id));
            format!("{ns}:entities/{path}")
        });
        Ok(Self {
            id: file.id.clone(),
            file,
            category,
            attributes,
            builtin,
            hitboxes,
            spawn,
            despawn,
            loot_table,
            synced,
        })
    }

    /// The rule of a type that lists none (or whose entry matches none).
    pub fn default_rule(&self) -> spawn::Rule {
        spawn::Rule::default_for(self.category)
    }

    /// A built-in attribute's default as the file sets it (`max_health`...).
    pub fn builtin_value(&self, name: &str) -> Option<f64> {
        self.builtin.get(name).copied()
    }

    /// The slot of a tracked value.
    pub fn synced_slot(&self, name: &str) -> Option<usize> {
        self.synced.iter().position(|s| s.name == name)
    }
}

/// What one mob of a custom type holds beyond the engine's fields (`Extra.custom`).
#[derive(Clone, Debug)]
pub struct CustomState {
    /// Its custom attributes with their modifiers.
    pub attrs: AttrSet,
    /// Its tracked values, in the type's slot order.
    pub synced: Vec<SyncedValue>,
    /// What its attack goals wait for and ask of the animation side (F4M-4a).
    pub attack: ai::attacks::AttackState,
}

impl CustomState {
    pub fn new(t: &CustomType) -> Self {
        Self {
            // `CustomType::new` already built one from the same definitions.
            attrs: AttrSet::new(&t.attributes).unwrap_or_default(),
            synced: t.synced.iter().map(|s| s.default.clone()).collect(),
            attack: Default::default(),
        }
    }
    pub fn synced_value(&self, t: &CustomType, name: &str) -> Option<&SyncedValue> {
        self.synced.get(t.synced_slot(name)?)
    }
    /// Sets a tracked value, cut to its type's range; false for an unknown name or a value of
    /// another type.
    pub fn set_synced(&mut self, t: &CustomType, name: &str, value: SyncedValue) -> bool {
        let Some(slot) = t.synced_slot(name) else {
            return false;
        };
        match t.synced[slot].coerce(value) {
            Some(v) => {
                self.synced[slot] = v;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::{Brain, Species, species};
    use super::*;
    use std::sync::Arc;

    /// The neutral row of `Species::Custom`: no vanilla behaviour leaks into a custom mob through
    /// a table row or a wildcard arm, and the vanilla rows are found as before.
    #[test]
    fn the_custom_species_row_is_neutral() {
        let row = species::def(Species::Custom);
        assert_eq!(row.species, Species::Custom);
        assert!(matches!(row.baby, species::Baby::None));
        assert_eq!(row.sounds, ["", "", ""]);
        assert!(row.step_sound.is_none());
        assert_eq!(row.brain(), Brain::Passive);
        assert_eq!(row.water_slowdown(), 0.8);
        assert_eq!(row.air_speed(), 0.02);
        assert_eq!(row.baby_scale(), 0.5);
        assert_eq!(row.head_rot_speed(), 10.0);
        assert_eq!(row.max_head_x_rot(), 40.0);
        assert_eq!(row.max_head_y_rot(), 75.0);
        assert_eq!(row.ambient_interval(), 80);
        let flags = row.flags(false);
        assert!(!(flags.aquatic_swimmer || flags.fall_immune || flags.flying || flags.climber));
        assert!(!(flags.lava_walker || flags.omnidirectional || flags.fire_immune));
        assert!(!(flags.burns_in_day || flags.undead || flags.breathes_water || flags.needs_water));
        assert!(!(flags.water_sensitive || flags.animal));
        assert!(row.food_tag().is_none() && row.tempt_tag().is_none());
        // No table row is for it, and every vanilla species still resolves to its own row.
        assert!(species::ALL.iter().all(|r| r.species != Species::Custom));
        for r in species::ALL {
            let first = species::ALL.iter().find(|d| d.species == r.species).unwrap();
            assert_eq!(species::def(r.species).name, first.name);
        }
    }

    /// The fixture fox, built the way the game builds it: the file, then its model through the
    /// real loaders (read from the fixture folder as the content root is read).
    pub(crate) fn fox_type() -> CustomType {
        let json = include_str!(
            "../../../../../tools/fixtures/cinder_fox/data/sample_mobs/horizon/entity_type/cinder_fox.json"
        );
        let file = CustomMobFile::parse("sample_mobs:cinder_fox", json.as_bytes()).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fixtures/cinder_fox");
        let model = file
            .load_model(&mut |p| std::fs::read(root.join(p)).map_err(|e| e.to_string()))
            .unwrap();
        CustomType::new(file, Some(&model)).unwrap()
    }

    #[test]
    fn the_fixture_fox_builds_into_a_type() {
        let t = fox_type();
        assert_eq!(t.category, Category::Creature);
        assert_eq!(t.builtin_value("max_health"), Some(14.0));
        assert_eq!(t.builtin_value("movement_speed"), Some(0.3));
        assert_eq!(t.attributes.len(), 1);
        assert_eq!(t.attributes[0].id, "sample_mobs:warmth");
        assert_eq!((t.attributes[0].min, t.attributes[0].max), (Some(0.0), Some(10.0)));
        assert_eq!(t.loot_table, "sample_mobs:entities/cinder_fox");
        assert_eq!(t.spawn.len(), 1);
        // The tracked values are in name order: state, warmth.
        assert_eq!(t.synced_slot("state"), Some(0));
        assert_eq!(t.synced_slot("warmth"), Some(1));
        assert_eq!(t.synced[0].default, SyncedValue::Enum(0));
        // The head part follows the model's head bone; the body part is a plain box.
        assert_eq!(t.hitboxes.len(), 2);
        assert!(t.hitboxes[0].bone.is_none());
        let head = t.hitboxes[1].bone.expect("head bone");
        assert!(head[1] > 0.0, "the head bone is above the feet: {head:?}");
        assert_eq!(t.hitboxes[1].damage_multiplier, 1.5);
        // A loot table defaults to `<ns>:entities/<path>`.
        let plain = CustomMobFile::parse(
            "demo:thing",
            br#"{"template":"yoake:mob","size":{"width":1,"height":1}}"#,
        )
        .unwrap();
        assert_eq!(
            CustomType::new(plain, None).unwrap().loot_table,
            "demo:entities/thing"
        );
    }

    #[test]
    fn tracked_values_keep_their_type_and_range() {
        let t = fox_type();
        let mut s = CustomState::new(&t);
        assert_eq!(s.synced_value(&t, "warmth"), Some(&SyncedValue::I32(0)));
        assert!(s.set_synced(&t, "warmth", SyncedValue::I32(99)));
        assert_eq!(
            s.synced_value(&t, "warmth"),
            Some(&SyncedValue::I32(10)),
            "cut to max"
        );
        assert!(!s.set_synced(&t, "warmth", SyncedValue::Bool(true)), "wrong type");
        assert!(!s.set_synced(&t, "nothing", SyncedValue::Bool(true)));
        assert!(s.set_synced(&t, "state", SyncedValue::Enum(2)));
        assert!(!s.set_synced(&t, "state", SyncedValue::Enum(3)), "no such value");
    }

    /// A custom mob of `category`, as the game makes one.
    fn mob_of(category: &str) -> crate::mobs::Mob {
        let json = format!(
            r#"{{"template":"yoake:mob","category":"{category}","size":{{"width":1,"height":1}}}}"#
        );
        let file = CustomMobFile::parse("demo:thing", json.as_bytes()).unwrap();
        let t = Arc::new(CustomType::new(file, None).unwrap());
        let kind = Arc::new(build::mob_type(&t));
        crate::mobs::Mob::new(1, kind, horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5), 0.0)
    }

    fn vanilla(species: Species) -> crate::mobs::Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        Arc::make_mut(&mut kind).species = species;
        crate::mobs::Mob::new(1, kind, horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5), 0.0)
    }

    /// `instanceof Monster` has no Java class for a custom mob: its category stands in. A vanilla
    /// mob answers exactly as the species-based test does.
    #[test]
    fn a_custom_mob_is_a_monster_by_its_category() {
        use crate::mobs::{ai, drops};
        let monster = mob_of("monster");
        let creature = mob_of("creature");
        assert!(ai::mob_is_monster(&monster) && drops::mob_is_monster(&monster));
        assert!(!ai::mob_is_monster(&creature) && !drops::mob_is_monster(&creature));
        for s in [Species::Zombie, Species::Cow, Species::Hoglin, Species::Creaking, Species::Giant] {
            let m = vanilla(s);
            assert_eq!(ai::mob_is_monster(&m), ai::is_monster(s), "{s:?}");
            assert_eq!(drops::mob_is_monster(&m), drops::is_monster(s), "{s:?}");
        }
        // The call sites follow: swim sound, experience and the loot rule.
        let mut monster = mob_of("monster");
        let mut creature = mob_of("creature");
        assert_eq!(ai::swim_sound(&mut monster), "entity.hostile.swim");
        assert_eq!(ai::swim_sound(&mut creature), "entity.generic.swim");
        assert!(drops::drops_experience(&monster));
        monster.extra.age = -24000;
        creature.extra.age = -24000;
        assert!(drops::drops_experience(&monster), "a baby monster still drops experience");
        assert!(!drops::drops_experience(&creature), "a baby creature does not");
        assert!(drops::death_loot_table(&monster, true).is_some());
        assert!(drops::death_loot_table(&creature, true).is_none());
    }
}
