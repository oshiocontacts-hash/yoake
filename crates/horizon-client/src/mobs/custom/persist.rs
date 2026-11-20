//! The save format of a custom mob (FR-F4-MOBS.md section 3.10).
//!
//! The entity id is the type id; position, health and the ordinary fields use the engine's entity
//! NBT. Everything of the custom mob lives in one namespaced compound, `yoake`, inside the entity
//! tag: `{ v: <type format>, synced: { <name>: <value> }, attributes: [ {id, base, modifiers} ] }`.
//! A vanilla entity never gets a `yoake` key. Keys this version does not know (later items add
//! `anim`, `attachments`, `memories`, `script`) and entries of values or attributes the type
//! no longer declares stay in the compound as they were read, so a mod update that drops a value
//! and a later one that brings it back lose nothing.
//!
//! A world opened without the mod: `mob_from_nbt` finds no type for the id and the game keeps
//! the whole compound (`Game::load_other`: "an entity of a type Horizon does not simulate"), which
//! writes it back unchanged but for its position, rotation and UUID. Tracked values are saved by
//! name (an enum by the name of its value), not by slot, so reordering a type's values or adding
//! one keeps old saves valid.
use super::{CustomState, CustomType, SyncedDef, SyncedValue, attrs};
use crate::{
    mobs::Mob,
    save::{float, number, text},
};
use horizon_world::nbt::{Compound, Tag};

fn string(s: &str) -> Tag {
    Tag::String(s.into())
}

fn encode(def: &SyncedDef, v: &SyncedValue) -> Tag {
    match v {
        SyncedValue::Bool(b) => Tag::Byte(i8::from(*b)),
        SyncedValue::I32(n) => Tag::Int(*n),
        SyncedValue::F32(f) => Tag::Float(f.to_bits()),
        SyncedValue::Str(s) => string(s),
        SyncedValue::Vec3(v) => Tag::List(v.iter().map(|f| Tag::Float(f.to_bits())).collect()),
        SyncedValue::Enum(i) => string(def.values.get(usize::from(*i)).map_or("", String::as_str)),
    }
}

fn decode(def: &SyncedDef, tag: &Tag) -> Option<SyncedValue> {
    let v = match def.kind.as_str() {
        "bool" => SyncedValue::Bool(number(Some(tag))? != 0),
        "i32" => SyncedValue::I32(i32::try_from(number(Some(tag))?).ok()?),
        "f32" => SyncedValue::F32(float(Some(tag))? as f32),
        "string" => SyncedValue::Str(text(Some(tag))?),
        "vec3" => {
            let Tag::List(l) = tag else { return None };
            if l.len() != 3 {
                return None;
            }
            let mut out = [0.0f32; 3];
            for (o, t) in out.iter_mut().zip(l) {
                *o = float(Some(t))? as f32;
            }
            SyncedValue::Vec3(out)
        }
        "enum" => {
            let name = text(Some(tag))?;
            SyncedValue::Enum(def.values.iter().position(|v| *v == name)? as u8)
        }
        _ => return None,
    };
    def.coerce(v)
}

/// Writes the `yoake` compound of custom mob `m` over `tag` (the mob's entity compound, which
/// starts as the one it was loaded with). Does nothing for any other mob.
pub fn write(tag: &mut Tag, m: &Mob) {
    let (Some(t), Some(state)) = (&m.kind.custom, &m.extra.custom) else {
        return;
    };
    let Tag::Compound(map) = tag else { return };
    let mut yoake = match map.get(&"yoake".into()) {
        Some(Tag::Compound(c)) => c.clone(),
        _ => Compound::new(),
    };
    yoake.insert("v".into(), Tag::Int(t.file.format as i32));
    // Tracked values: the persistent ones by name, over what the save had (so a value the type
    // dropped stays).
    let mut synced = match yoake.get(&"synced".into()) {
        Some(Tag::Compound(c)) => c.clone(),
        _ => Compound::new(),
    };
    for (def, value) in t.synced.iter().zip(&state.synced) {
        if def.persistent {
            synced.insert(def.name.as_str().into(), encode(def, value));
        } else {
            synced.remove(&def.name.as_str().into());
        }
    }
    if synced.is_empty() {
        yoake.remove(&"synced".into());
    } else {
        yoake.insert("synced".into(), Tag::Compound(synced));
    }
    // Where `sleep_in_bed_like` rests: `rest: { home: [x, y, z], place: [x, y, z] }`, over what
    // the save had (a key of a later version stays).
    let mut rest = match yoake.get(&"rest".into()) {
        Some(Tag::Compound(c)) => c.clone(),
        _ => Compound::new(),
    };
    for (key, at) in [("home", state.rest.home), ("place", state.rest.place)] {
        if let Some(p) = at {
            rest.insert(key.into(), Tag::List(p.iter().map(|n| Tag::Int(*n)).collect()));
        }
    }
    if !rest.is_empty() {
        yoake.insert("rest".into(), Tag::Compound(rest));
    }
    // Attributes: the changed ones, then the saved entries of attributes the type lost.
    let saved: Vec<Tag> = match yoake.get(&"attributes".into()) {
        Some(Tag::List(l)) => l.clone(),
        _ => Vec::new(),
    };
    let mut list = state.attrs.to_nbt(&t.attributes);
    list.extend(attrs::unknown_entries(&saved, &state.attrs));
    if list.is_empty() {
        yoake.remove(&"attributes".into());
    } else {
        yoake.insert("attributes".into(), Tag::List(list));
    }
    map.insert("yoake".into(), Tag::Compound(yoake));
}

/// Reads the `yoake` compound of a saved custom mob into `m` (which `Mob::new` gave the defaults).
pub fn read(m: &mut Mob, tag: &Tag) {
    let Some(t) = m.kind.custom.clone() else {
        return;
    };
    let state = m
        .extra
        .custom
        .get_or_insert_with(|| Box::new(CustomState::new(&t)));
    let Some(yoake @ Tag::Compound(_)) = tag.get("yoake") else {
        return;
    };
    if let Some(synced @ Tag::Compound(_)) = yoake.get("synced") {
        for (slot, def) in t.synced.iter().enumerate() {
            if def.persistent
                && let Some(v) = synced.get(&def.name).and_then(|tag| decode(def, tag))
            {
                state.synced[slot] = v;
            }
        }
    }
    if let Some(Tag::List(list)) = yoake.get("attributes") {
        state.attrs.read_nbt(list);
    }
    if let Some(rest @ Tag::Compound(_)) = yoake.get("rest") {
        state.rest.home = block_pos(rest.get("home"));
        state.rest.place = block_pos(rest.get("place"));
    }
}

/// A saved block position: a list of three ints.
fn block_pos(tag: Option<&Tag>) -> Option<[i32; 3]> {
    let Some(Tag::List(l)) = tag else { return None };
    if l.len() != 3 {
        return None;
    }
    let mut out = [0; 3];
    for (o, t) in out.iter_mut().zip(l) {
        *o = i32::try_from(number(Some(t))?).ok()?;
    }
    Some(out)
}

/// Whether the type is the one a save was written for (a newer format is read as far as it
/// is understood).
pub fn saved_format(tag: &Tag) -> Option<i64> {
    number(tag.get("yoake")?.get("v"))
}

#[allow(dead_code)]
fn _t(_: &CustomType) {}

#[cfg(test)]
mod tests {
    use super::super::{SyncedValue, build::mob_type, tests::fox_type};
    use super::*;
    use crate::{
        entity_nbt::{mob_from_nbt, mob_to_nbt},
        mobs::{MobType, Species},
    };
    use horizon_core::geometry::Vec3;
    use horizon_world::attributes::Operation;
    use std::{collections::HashMap, sync::Arc};

    fn types() -> HashMap<String, Arc<MobType>> {
        let t = Arc::new(fox_type());
        let kind = Arc::new(mob_type(&t));
        HashMap::from([(kind.id.clone(), kind)])
    }
    fn fox(types: &HashMap<String, Arc<MobType>>) -> Mob {
        Mob::new(
            3,
            types["sample_mobs:cinder_fox"].clone(),
            Vec3::new(10.5, 70.0, -4.5),
            90.0,
        )
    }
    const UUID: [i32; 4] = [1, 2, 3, 4];

    /// A custom mob saved and loaded again is the same mob: position, health, tracked values and
    /// attributes with their modifiers.
    #[test]
    fn a_custom_mob_survives_a_save_and_load() {
        let types = types();
        let mut m = fox(&types);
        m.health = 9.0;
        let t = m.kind.custom.clone().unwrap();
        let state = m.extra.custom.as_mut().unwrap();
        assert!(state.set_synced(&t, "state", SyncedValue::Enum(2)));
        assert!(state.set_synced(&t, "warmth", SyncedValue::I32(7)));
        state.attrs.set_base("sample_mobs:warmth", 3.0);
        state.attrs.add_modifier(
            "sample_mobs:warmth",
            "sample_mobs:campfire",
            2.0,
            Operation::AddValue,
        );
        let tag = mob_to_nbt(&m, UUID, None, None);
        assert_eq!(tag.get("id"), Some(&string("sample_mobs:cinder_fox")));
        let yoake = tag.get("yoake").expect("a yoake compound");
        assert_eq!(number(yoake.get("v")), Some(1));
        assert_eq!(text(yoake.get("synced").unwrap().get("state")).as_deref(), Some("pounce"));
        assert_eq!(number(yoake.get("synced").unwrap().get("warmth")), Some(7));

        let mut back = mob_from_nbt(&tag, &types, 9, None).expect("the type is known");
        assert_eq!(back.kind.id, "sample_mobs:cinder_fox");
        assert_eq!(back.health, 9.0);
        assert_eq!((back.position.x, back.position.y, back.position.z), (10.5, 70.0, -4.5));
        let state = back.extra.custom.as_mut().unwrap();
        assert_eq!(state.synced_value(&t, "state"), Some(&SyncedValue::Enum(2)));
        assert_eq!(state.synced_value(&t, "warmth"), Some(&SyncedValue::I32(7)));
        assert_eq!(state.attrs.base("sample_mobs:warmth"), Some(3.0));
        assert_eq!(state.attrs.value("sample_mobs:warmth"), Some(5.0));
        // And the second save is the first one.
        assert_eq!(mob_to_nbt(&back, UUID, Some(&tag), None), tag);
    }

    /// Defaults write only what must be: a fresh fox keeps no attribute entry, and a vanilla mob
    /// never gets a `yoake` key.
    #[test]
    fn a_fresh_mob_writes_the_least_and_a_vanilla_mob_nothing() {
        let types = types();
        let tag = mob_to_nbt(&fox(&types), UUID, None, None);
        let yoake = tag.get("yoake").unwrap();
        assert!(yoake.get("attributes").is_none());
        // `state` and `warmth` are persistent, so both are written.
        assert_eq!(text(yoake.get("synced").unwrap().get("state")).as_deref(), Some("idle"));
        let cow = crate::mobs::sensor_scan_tests::kind();
        let m = Mob::new(1, cow, Vec3::ZERO, 0.0);
        assert!(mob_to_nbt(&m, UUID, None, None).get("yoake").is_none());
    }

    /// Entries this version cannot place stay: a later key, a value or attribute the type dropped.
    #[test]
    fn what_the_type_does_not_know_is_kept_in_the_compound() {
        let types = types();
        let m = fox(&types);
        let mut tag = mob_to_nbt(&m, UUID, None, None);
        let Tag::Compound(map) = &mut tag else { panic!() };
        let Some(Tag::Compound(yoake)) = map.get_mut(&"yoake".into()) else { panic!() };
        yoake.insert("anim".into(), Tag::compound([("t", Tag::Int(5))]));
        let Some(Tag::Compound(synced)) = yoake.get_mut(&"synced".into()) else { panic!() };
        synced.insert("old_value".into(), Tag::Int(42));
        yoake.insert(
            "attributes".into(),
            Tag::List(vec![Tag::compound([
                ("id", string("sample_mobs:gone")),
                ("base", Tag::Double(2.5f64.to_bits())),
            ])]),
        );
        let back = mob_from_nbt(&tag, &types, 9, None).unwrap();
        let again = mob_to_nbt(&back, UUID, Some(&tag), None);
        let y = again.get("yoake").unwrap();
        assert!(y.get("anim").is_some());
        assert_eq!(number(y.get("synced").unwrap().get("old_value")), Some(42));
        let attrs = y.get("attributes").unwrap();
        let Tag::List(list) = attrs else { panic!() };
        assert_eq!(list.len(), 1);
        assert_eq!(text(list[0].get("id")).as_deref(), Some("sample_mobs:gone"));
    }

    /// A world opened without the mod: no type, no mob; the compound goes to the game's
    /// unknown-entity path, which keeps it whole.
    #[test]
    fn a_world_without_the_mod_does_not_build_the_mob() {
        let types = types();
        let tag = mob_to_nbt(&fox(&types), UUID, None, None);
        let without: HashMap<String, Arc<MobType>> = HashMap::new();
        assert!(mob_from_nbt(&tag, &without, 1, None).is_none());
        assert_eq!(crate::entity_nbt::kind(&tag).as_deref(), Some("sample_mobs:cinder_fox"));
        assert!(crate::entity_nbt::position(&tag).is_some());
        // The mob of a type with the right id but not a custom one (a mod that now ships a data
        // template of that id) reads and ignores the compound.
        let cow = (*crate::mobs::sensor_scan_tests::kind()).clone();
        let impostor = MobType { id: "sample_mobs:cinder_fox".into(), ..cow };
        let others = HashMap::from([(impostor.id.clone(), Arc::new(impostor))]);
        let m = mob_from_nbt(&tag, &others, 1, None).unwrap();
        assert_eq!(m.species(), Species::Cow);
        assert!(m.extra.custom.is_none());
    }

    /// The rest place and home of `sleep_in_bed_like` are saved in `yoake.rest`, read back, and
    /// the keys a later version adds to it stay.
    #[test]
    fn the_rest_place_and_home_survive_a_save_and_load() {
        let types = types();
        let mut m = fox(&types);
        m.extra.custom.as_mut().unwrap().rest = crate::mobs::custom::RestMemory {
            home: Some([-3, 64, 8]),
            place: Some([12, 70, -5]),
        };
        let mut tag = mob_to_nbt(&m, UUID, None, None);
        let rest = tag.get("yoake").unwrap().get("rest").expect("a rest compound");
        assert_eq!(block_pos(rest.get("home")), Some([-3, 64, 8]));
        assert_eq!(block_pos(rest.get("place")), Some([12, 70, -5]));
        // A key of a later version inside it, and a bad position, which reads as none.
        let Tag::Compound(map) = &mut tag else { panic!() };
        let Some(Tag::Compound(yoake)) = map.get_mut(&"yoake".into()) else { panic!() };
        let Some(Tag::Compound(rest)) = yoake.get_mut(&"rest".into()) else { panic!() };
        rest.insert("since".into(), Tag::Int(77));
        let back = mob_from_nbt(&tag, &types, 9, None).unwrap();
        assert_eq!(back.extra.custom.as_ref().unwrap().rest, m.extra.custom.as_ref().unwrap().rest);
        let again = mob_to_nbt(&back, UUID, Some(&tag), None);
        assert_eq!(again, tag, "the second save is the first, unknown key and all");
        let Tag::Compound(map) = &mut tag else { panic!() };
        let Some(Tag::Compound(yoake)) = map.get_mut(&"yoake".into()) else { panic!() };
        yoake.insert("rest".into(), Tag::compound([("home", Tag::List(vec![Tag::Int(1)]))]));
        let back = mob_from_nbt(&tag, &types, 9, None).unwrap();
        assert_eq!(back.extra.custom.as_ref().unwrap().rest, crate::mobs::custom::RestMemory::default());
        // A mob that never rested writes no rest key.
        assert!(mob_to_nbt(&fox(&types), UUID, None, None).get("yoake").unwrap().get("rest").is_none());
    }

    /// A newer type format and a bad value read as far as they can.
    #[test]
    fn a_bad_saved_value_falls_back_to_the_default() {
        let types = types();
        let mut tag = mob_to_nbt(&fox(&types), UUID, None, None);
        let Tag::Compound(map) = &mut tag else { panic!() };
        let Some(Tag::Compound(yoake)) = map.get_mut(&"yoake".into()) else { panic!() };
        yoake.insert("v".into(), Tag::Int(5));
        let Some(Tag::Compound(synced)) = yoake.get_mut(&"synced".into()) else { panic!() };
        synced.insert("state".into(), string("flying"));
        synced.insert("warmth".into(), Tag::Int(500));
        assert_eq!(saved_format(&tag), Some(5));
        let back = mob_from_nbt(&tag, &types, 9, None).unwrap();
        let t = back.kind.custom.clone().unwrap();
        let state = back.extra.custom.as_ref().unwrap();
        assert_eq!(state.synced_value(&t, "state"), Some(&SyncedValue::Enum(0)));
        assert_eq!(state.synced_value(&t, "warmth"), Some(&SyncedValue::I32(10)));
    }
}
