//! Custom attributes of a custom mob (`attributes` entries that are objects, FR-F4-MOBS.md
//! section 5) with the engine's own attribute implementation (`horizon_world::attributes`): the
//! same operation order (`add_value`, then `add_multiplied_base`, then `add_multiplied_total`),
//! the same ranged clamp, the same permanent/transient modifier rules. Built-in attributes
//! (`max_health`, `movement_speed`...) keep the engine's existing path (`Mob.attrs`,
//! `mobs::finalize`) and only get their defaults from the file (`build::mob_type`).
use crate::save::{float, text};
use horizon_world::{
    attributes::{Definition, Instance, Modifier, Observer, Operation},
    nbt::Tag,
};
use std::convert::Infallible;

/// Who is told the value (`sync` of the file). Sent by the sync layer (F4M-5b); stored here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync {
    All,
    Owner,
    None,
}
impl Sync {
    pub fn parse(s: &str) -> Self {
        match s {
            "owner" => Self::Owner,
            "none" => Self::None,
            _ => Self::All,
        }
    }
}

/// One custom attribute of a type.
#[derive(Clone, Debug, PartialEq)]
pub struct AttrDef {
    /// `<namespace>:<name>`.
    pub id: String,
    pub default: f64,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub sync: Sync,
}
impl AttrDef {
    fn definition(&self) -> Result<Definition, String> {
        let lo = self.min.unwrap_or(f64::MIN);
        let hi = self.max.unwrap_or(f64::MAX);
        // The default of a ranged attribute lies inside the range (the engine's rule): a file that
        // writes one outside is cut to it rather than refused.
        let default = self.default.clamp(lo, hi);
        Definition::ranged(default, lo, hi).map_err(|e| format!("{}: {e}", self.id))
    }
}

struct Quiet;
impl Observer for Quiet {
    type Error = Infallible;
    fn dirty(&mut self, _: &mut Instance) -> Result<(), Infallible> {
        Ok(())
    }
}

/// The attribute instances of one mob, in the type's declaration order (by id).
#[derive(Clone, Debug, Default)]
pub struct AttrSet {
    list: Vec<(String, Instance)>,
}

fn operation_name(op: Operation) -> &'static str {
    match op {
        Operation::AddValue => "add_value",
        Operation::AddMultipliedBase => "add_multiplied_base",
        Operation::AddMultipliedTotal => "add_multiplied_total",
    }
}
/// `add_value`, `add_multiplied_base` and `add_multiplied_total` (the names of vanilla's save).
pub fn operation(name: &str) -> Option<Operation> {
    Operation::ALL
        .into_iter()
        .find(|op| operation_name(*op) == name)
}

/// After a service changed a built-in attribute: lowering max health clamps the current health
/// (`LivingEntity.setHealth` clamps to the new maximum).
fn after_builtin(m: &mut crate::mobs::Mob, a: crate::mobs::finalize::Attr) {
    if a == crate::mobs::finalize::Attr::MaxHealth {
        m.health = m.health.min(m.attrs.max_health);
    }
}

fn builtin(m: &crate::mobs::Mob, id: &str) -> Option<crate::mobs::finalize::Attr> {
    let a = crate::mobs::finalize::Attr::from_id(id)?;
    m.kind.attributes.contains_key(a.id()).then_some(a)
}

/// Whether `m` holds `id` (a built-in attribute in its type's attribute map or a custom attribute).
pub fn has(m: &crate::mobs::Mob, id: &str) -> bool {
    builtin(m, id).is_some()
        || m.kind
            .custom
            .as_ref()
            .is_some_and(|t| t.attributes.iter().any(|a| a.id == id))
}

/// Reads `(base, value)` for a built-in attribute in `m`'s attribute map or a custom attribute.
pub fn get(m: &mut crate::mobs::Mob, id: &str) -> Option<(f64, f64)> {
    if let Some(a) = builtin(m, id) {
        Some((
            crate::mobs::finalize::base_value(m, a),
            crate::mobs::finalize::value(m, a),
        ))
    } else {
        let s = m.extra.custom.as_mut()?;
        Some((s.attrs.base(id)?, s.attrs.value(id)?))
    }
}

/// Sets either a built-in attribute of the type or one of its custom attributes.
pub fn set_base(m: &mut crate::mobs::Mob, id: &str, value: f64) -> bool {
    if let Some(a) = builtin(m, id) {
        crate::mobs::finalize::set_base(m, a, value);
        after_builtin(m, a);
        true
    } else {
        m.extra.custom.as_mut().is_some_and(|s| s.attrs.set_base(id, value))
    }
}

/// Adds a permanent modifier through the same path for built-in and custom attributes.
pub fn add_modifier(m: &mut crate::mobs::Mob, id: &str, modifier: &str, amount: f64, op: Operation) -> bool {
    if let Some(a) = builtin(m, id) {
        if Modifier::new(modifier, amount, op).is_err() {
            return false;
        }
        let op = match op {
            Operation::AddValue => crate::mobs::finalize::Op::AddValue,
            Operation::AddMultipliedBase => crate::mobs::finalize::Op::AddMultipliedBase,
            Operation::AddMultipliedTotal => crate::mobs::finalize::Op::AddMultipliedTotal,
        };
        crate::mobs::finalize::add_modifier(m, a, modifier, amount, op);
        after_builtin(m, a);
        true
    } else {
        m.extra.custom.as_mut().is_some_and(|s| s.attrs.add_modifier(id, modifier, amount, op))
    }
}

pub fn remove_modifier(m: &mut crate::mobs::Mob, id: &str, modifier: &str) -> bool {
    if let Some(a) = builtin(m, id) {
        let had = crate::mobs::finalize::has_modifier(m, a, modifier);
        crate::mobs::finalize::remove_modifier(m, a, modifier);
        after_builtin(m, a);
        had
    } else {
        m.extra.custom.as_mut().is_some_and(|s| s.attrs.remove_modifier(id, modifier))
    }
}

impl AttrSet {
    pub fn new(defs: &[AttrDef]) -> Result<Self, String> {
        let mut list = Vec::with_capacity(defs.len());
        for d in defs {
            list.push((d.id.clone(), Instance::new(d.definition()?)));
        }
        Ok(Self { list })
    }
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.list.iter().map(|(id, _)| id.as_str())
    }
    fn get(&self, id: &str) -> Option<&Instance> {
        self.list.iter().find(|(i, _)| i == id).map(|(_, a)| a)
    }
    fn get_mut(&mut self, id: &str) -> Option<&mut Instance> {
        self.list.iter_mut().find(|(i, _)| i == id).map(|(_, a)| a)
    }
    /// The value with every modifier applied (`getAttributeValue`).
    pub fn value(&mut self, id: &str) -> Option<f64> {
        Some(self.get_mut(id)?.value())
    }
    pub fn base(&self, id: &str) -> Option<f64> {
        Some(self.get(id)?.base_value())
    }
    /// `setBaseValue`; false for an attribute the type does not have.
    pub fn set_base(&mut self, id: &str, value: f64) -> bool {
        match self.get_mut(id) {
            Some(a) => {
                let _ = a.set_base_value(value, &mut Quiet);
                true
            }
            None => false,
        }
    }
    /// Adds a modifier that is saved (`addPermanentModifier`); an existing one of the same id is
    /// replaced. False for an unknown attribute or a bad modifier id.
    pub fn add_modifier(&mut self, id: &str, modifier: &str, amount: f64, op: Operation) -> bool {
        let Some(a) = self.get_mut(id) else {
            return false;
        };
        let Ok(m) = Modifier::new(modifier, amount, op) else {
            return false;
        };
        a.add_or_replace_permanent(m, &mut Quiet).is_ok()
    }
    /// `removeModifier`.
    pub fn remove_modifier(&mut self, id: &str, modifier: &str) -> bool {
        self.get_mut(id)
            .is_some_and(|a| a.remove(modifier, &mut Quiet).unwrap_or(false))
    }

    /// `AttributeMap.pack`: `[{id, base, modifiers: [{id, amount, operation}]}]`, only for the
    /// attributes that differ from their default or carry a permanent modifier.
    pub fn to_nbt(&self, defs: &[AttrDef]) -> Vec<Tag> {
        let mut out = Vec::new();
        for (id, a) in &self.list {
            let default = defs.iter().find(|d| &d.id == id).map(|d| d.default);
            let packed = a.pack();
            if packed.modifiers.is_empty() && default == Some(packed.base) {
                continue;
            }
            let mut entry = vec![
                ("id", Tag::String(id.as_str().into())),
                ("base", Tag::Double(packed.base.to_bits())),
            ];
            if !packed.modifiers.is_empty() {
                let list = packed
                    .modifiers
                    .iter()
                    .map(|m| {
                        Tag::compound([
                            ("id", Tag::String(m.id().into())),
                            ("amount", Tag::Double(m.amount().to_bits())),
                            (
                                "operation",
                                Tag::String(operation_name(m.operation()).into()),
                            ),
                        ])
                    })
                    .collect();
                entry.push(("modifiers", Tag::List(list)));
            }
            out.push(Tag::compound(entry));
        }
        out
    }
    /// The saved list over the defaults; an entry of an attribute the type no longer has is
    /// skipped here (the save keeps it: see [`unknown_entries`]).
    pub fn read_nbt(&mut self, list: &[Tag]) {
        for entry in list {
            let Some(id) = text(entry.get("id")) else {
                continue;
            };
            let Some(a) = self.get_mut(&id) else {
                continue;
            };
            if let Some(base) = float(entry.get("base")) {
                let _ = a.set_base_value(base, &mut Quiet);
            }
            if let Some(Tag::List(mods)) = entry.get("modifiers") {
                for m in mods {
                    let (Some(mid), Some(amount), Some(op)) = (
                        text(m.get("id")),
                        float(m.get("amount")),
                        text(m.get("operation")).and_then(|o| operation(&o)),
                    ) else {
                        continue;
                    };
                    if let Ok(modifier) = Modifier::new(&mid, amount, op) {
                        let _ = a.add_or_replace_permanent(modifier, &mut Quiet);
                    }
                }
            }
        }
    }
}

/// The entries of a saved attribute list that name an attribute `known` does not have (the type
/// lost it in a mod update): the save keeps them so an update that brings it back finds its value.
pub fn unknown_entries(saved: &[Tag], known: &AttrSet) -> Vec<Tag> {
    saved
        .iter()
        .filter(|e| text(e.get("id")).is_none_or(|id| known.get(&id).is_none()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defs() -> Vec<AttrDef> {
        vec![
            AttrDef {
                id: "demo:fury".into(),
                default: 2.0,
                min: Some(0.0),
                max: Some(10.0),
                sync: Sync::All,
            },
            AttrDef {
                id: "demo:free".into(),
                default: 1.0,
                min: None,
                max: None,
                sync: Sync::None,
            },
        ]
    }

    /// The engine's order: all `add_value` first, then the base multipliers over that sum, then
    /// each total multiplier, and the result is cut to the range.
    #[test]
    fn modifiers_follow_the_engines_operation_order_and_range() {
        let mut a = AttrSet::new(&defs()).unwrap();
        assert_eq!(a.value("demo:fury"), Some(2.0));
        assert!(a.add_modifier("demo:fury", "demo:a", 1.0, Operation::AddValue));
        assert!(a.add_modifier("demo:fury", "demo:b", 0.5, Operation::AddMultipliedBase));
        assert!(a.add_modifier("demo:fury", "demo:c", 1.0, Operation::AddMultipliedTotal));
        // base 2 + 1 = 3; 3 + 3 * 0.5 = 4.5; 4.5 * (1 + 1) = 9.
        assert_eq!(a.value("demo:fury"), Some(9.0));
        // the range cuts a bigger result.
        assert!(a.add_modifier("demo:fury", "demo:d", 1.0, Operation::AddMultipliedTotal));
        assert_eq!(a.value("demo:fury"), Some(10.0));
        // a modifier is replaced by its id, removed by its id.
        assert!(a.add_modifier("demo:fury", "demo:d", -1.0, Operation::AddMultipliedTotal));
        assert_eq!(a.value("demo:fury"), Some(0.0));
        assert!(a.remove_modifier("demo:fury", "demo:d"));
        assert!(!a.remove_modifier("demo:fury", "demo:d"));
        assert_eq!(a.value("demo:fury"), Some(9.0));
        // the base moves under the modifiers.
        assert!(a.set_base("demo:fury", 4.0));
        assert_eq!(a.base("demo:fury"), Some(4.0));
        assert_eq!(a.value("demo:fury"), Some(10.0));
        // an unranged one is not cut, an unknown one is refused.
        assert!(a.add_modifier("demo:free", "demo:x", 1000.0, Operation::AddValue));
        assert_eq!(a.value("demo:free"), Some(1001.0));
        assert!(!a.set_base("demo:nothing", 1.0));
        assert!(!a.add_modifier("demo:nothing", "demo:x", 1.0, Operation::AddValue));
        assert!(!a.add_modifier("demo:free", "Bad Id", 1.0, Operation::AddValue));
    }

    #[test]
    fn the_save_round_trips_bases_and_modifiers_by_id() {
        let defs = defs();
        let mut a = AttrSet::new(&defs).unwrap();
        // Nothing changed: nothing is saved.
        assert!(a.to_nbt(&defs).is_empty());
        a.set_base("demo:fury", 5.0);
        a.add_modifier("demo:free", "demo:boost", 0.25, Operation::AddMultipliedBase);
        let saved = a.to_nbt(&defs);
        assert_eq!(saved.len(), 2);
        let mut b = AttrSet::new(&defs).unwrap();
        b.read_nbt(&saved);
        assert_eq!(b.value("demo:fury"), Some(5.0));
        assert_eq!(b.value("demo:free"), Some(1.25));
        assert_eq!(b.to_nbt(&defs), saved);
        // An entry for an attribute the type lost is skipped by the read and found by `unknown_entries`.
        let mut extra = saved.clone();
        extra.push(Tag::compound([
            ("id", Tag::String("demo:gone".into())),
            ("base", Tag::Double(3.0f64.to_bits())),
        ]));
        let mut c = AttrSet::new(&defs).unwrap();
        c.read_nbt(&extra);
        assert_eq!(unknown_entries(&extra, &c).len(), 1);
    }

    #[test]
    fn a_default_outside_its_range_is_cut_to_it() {
        let d = AttrDef {
            id: "demo:x".into(),
            default: 50.0,
            min: Some(0.0),
            max: Some(10.0),
            sync: Sync::parse("owner"),
        };
        let mut a = AttrSet::new(&[d]).unwrap();
        assert_eq!(a.value("demo:x"), Some(10.0));
        assert_eq!(Sync::parse("none"), Sync::None);
    }
}

// ---- the scale of a mob (F4M-4c) -----------------------------------------------------------------

/// The range of vanilla's `minecraft:scale`.
const SCALE_RANGE: (f64, f64) = (0.0625, 16.0);

/// The mob's `scale` attribute: the value its type's `attributes` give (default 1).
pub fn attribute_scale(m: &crate::mobs::Mob) -> f32 {
    m.kind
        .custom
        .as_ref()
        .and_then(|t| t.builtin_value("scale"))
        .map_or(1.0, |s| s.clamp(SCALE_RANGE.0, SCALE_RANGE.1) as f32)
}

/// The one scale of a custom mob: its `scale` attribute times the `baby_scale` of its `breedable` type
/// while it is a baby (as vanilla stacks a baby's scale on the attribute, never over it).
pub fn effective_scale(m: &crate::mobs::Mob) -> f32 {
    let baby = if m.extra.age < 0 {
        super::ai::breed::baby_scale(m).unwrap_or(1.0)
    } else {
        1.0
    };
    attribute_scale(m) * baby
}

/// Puts [`effective_scale`] on the mob (`extra.model_scale`, which every box, seat and model scale
/// reads; 0, the engine's "none", when it is 1). When the mob got bigger (a baby grew up, the type
/// is larger than the box it spawned with) and `w` is given, it is moved to where the bigger box
/// fits (`Mob::fudge_position_after_size_change`, vanilla's `refreshDimensions`). Custom mobs only.
pub fn sync_scale(m: &mut crate::mobs::Mob, w: Option<&crate::mobs::MobWorld>) {
    if m.kind.custom.is_none() {
        return;
    }
    let want = effective_scale(m);
    let field = if (want - 1.0).abs() < f32::EPSILON { 0.0 } else { want };
    if m.extra.model_scale == field {
        return;
    }
    let (old_w, old_h, _) = m.dims();
    m.extra.model_scale = field;
    if let Some(w) = w {
        m.fudge_position_after_size_change(w, old_w, old_h);
    }
}

#[cfg(test)]
mod builtin_tests {
    use super::*;
    use crate::mobs::{Mob, custom::{CustomType, build}};
    use horizon_core::geometry::Vec3;
    use horizon_mods::custom_mob::CustomMobFile;
    use std::sync::Arc;

    fn mob_with_attrs(json: &str) -> Mob {
        let file = CustomMobFile::parse("demo:mob", json.as_bytes()).unwrap();
        let t = Arc::new(CustomType::new(file, None).unwrap());
        let kind = Arc::new(build::mob_type(&t));
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        build::register(&mut m);
        m
    }

    #[test]
    fn builtin_attributes_get_set_modify_and_clamp_health() {
        let mut m = mob_with_attrs(
            r#"{"template":"yoake:mob","category":"creature",
                "size":{"width":0.6,"height":1.0,"eye_height":0.8},
                "attributes":{"max_health":20,"movement_speed":0.25,"attack_damage":4},
                "ai":{"goals":[]}}"#,
        );
        assert!(has(&m, "max_health"));
        assert!(has(&m, "minecraft:max_health"));
        assert!(has(&m, "follow_range"));
        assert!(!has(&m, "armor"), "absent built-in is not in the map");

        assert_eq!(get(&mut m, "max_health"), Some((20.0, 20.0)));
        assert_eq!(m.health, 20.0);

        // Lowering max health clamps current health (`LivingEntity` health rule).
        assert!(set_base(&mut m, "minecraft:max_health", 12.0));
        assert_eq!(get(&mut m, "minecraft:max_health"), Some((12.0, 12.0)));
        assert_eq!(m.attrs.max_health, 12.0);
        assert_eq!(m.health, 12.0);

        // Vanilla range clamping keeps value >= 1.0 and clamps health to 1.0.
        assert!(set_base(&mut m, "max_health", 0.0));
        assert_eq!(get(&mut m, "max_health"), Some((0.0, 1.0)));
        assert_eq!(m.health, 1.0);
        assert!(set_base(&mut m, "max_health", 20.0));

        // Modifiers combine with base and clamp; removing lowers max health and clamps health.
        m.health = 18.0;
        assert!(add_modifier(&mut m, "max_health", "demo:boost", 0.5, Operation::AddMultipliedBase));
        assert_eq!(get(&mut m, "max_health"), Some((20.0, 30.0)));
        m.health = 28.0;
        assert!(remove_modifier(&mut m, "max_health", "demo:boost"));
        assert_eq!(m.attrs.max_health, 20.0);
        assert_eq!(m.health, 20.0);

        // Movement speed & attack damage update Mob.attrs.
        assert!(set_base(&mut m, "attack_damage", 6.0));
        assert!(add_modifier(&mut m, "minecraft:movement_speed", "demo:dash", 0.2, Operation::AddMultipliedTotal));
        assert!((m.attrs.speed - 0.3).abs() < 1e-9);
        assert_eq!(m.attrs.attack_damage, 6.0);

        // Built-ins not declared in the type map are refused.
        assert_eq!(get(&mut m, "armor"), None);
        assert!(!set_base(&mut m, "armor", 5.0));
        assert!(!add_modifier(&mut m, "armor", "demo:plate", 2.0, Operation::AddValue));
        assert!(!remove_modifier(&mut m, "armor", "demo:plate"));
    }
}
