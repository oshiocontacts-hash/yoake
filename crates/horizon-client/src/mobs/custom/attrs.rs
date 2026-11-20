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
