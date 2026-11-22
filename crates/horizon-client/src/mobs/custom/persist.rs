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
    // F4M-4c: the owner (`UUIDUtil` int array), the order to sit, the breeding age (a baby's
    // negative age or the pause after breeding) and the love ticks left. Only what is not the
    // default is written, so a mob nobody tamed or bred writes nothing here.
    for (key, tag) in [
        ("owner", m.extra.owner.map(|o| Tag::IntArray(o.to_vec()))),
        ("sitting", m.extra.sitting.then_some(Tag::Byte(1))),
        ("age", (m.extra.age != 0).then_some(Tag::Int(m.extra.age))),
        ("love", (m.extra.in_love > 0).then_some(Tag::Int(m.extra.in_love))),
    ] {
        match tag {
            Some(tag) => {
                yoake.insert(key.into(), tag);
            }
            None => {
                yoake.remove(&key.into());
            }
        }
    }
    // F4M-4d: the seats the players on it hold (and the ones a load named for players not back yet),
    // `player_seats: [{ uuid: [int; 4], seat: int }]`, so a player restored into the vehicle (its
    // `RootVehicle`) gets its seat back.
    let mut player_seats: Vec<Tag> = Vec::new();
    for c in state.player_claims.iter().chain(&state.saved_player_seats) {
        if !player_seats.iter().any(|t| crate::save::uuid_from_tag(t.get("uuid")) == Some(c.player)) {
            player_seats.push(Tag::compound([("uuid", Tag::IntArray(c.player.to_vec())), ("seat", Tag::Int(i32::from(c.seat)))]));
        }
    }
    if player_seats.is_empty() {
        yoake.remove(&"player_seats".into());
    } else {
        yoake.insert("player_seats".into(), Tag::List(player_seats));
    }
    // F4M-10: what the scripts keep with the mob (`mob-save` may replace it), opaque bytes.
    let script_data = super::script::data_to_save(m);
    if script_data.is_empty() {
        yoake.remove(&"script".into());
    } else {
        yoake.insert("script".into(), Tag::ByteArray(script_data));
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
/// Vanilla's top-level `Age` is also accepted, so summon NBT can make a baby.
pub fn read(m: &mut Mob, tag: &Tag) {
    let Some(t) = m.kind.custom.clone() else {
        return;
    };
    let state = m
        .extra
        .custom
        .get_or_insert_with(|| Box::new(CustomState::new(&t)));
    m.extra.age = number(tag.get("Age")).unwrap_or(0) as i32;
    if let Some(yoake @ Tag::Compound(_)) = tag.get("yoake") {
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
        if let Some(Tag::List(list)) = yoake.get("player_seats") {
            state.saved_player_seats = list
                .iter()
                .filter_map(|t| {
                    Some(super::PlayerClaim {
                        player: crate::save::uuid_from_tag(t.get("uuid"))?,
                        seat: u8::try_from(number(t.get("seat"))?).ok()?,
                    })
                })
                .take(16)
                .collect();
        }
        if let Some(Tag::ByteArray(bytes)) = yoake.get("script")
            && bytes.len() <= super::script::MAX_DATA_BYTES
        {
            state.script.data = bytes.clone();
        }
        m.extra.owner = crate::save::uuid_from_tag(yoake.get("owner"));
        m.extra.tame = m.extra.owner.is_some();
        m.extra.sitting = m.extra.tame && number(yoake.get("sitting")).is_some_and(|n| n != 0);
        m.extra.age = number(yoake.get("age")).unwrap_or(i64::from(m.extra.age)) as i32;
        m.extra.in_love = number(yoake.get("love")).unwrap_or(0).max(0) as i32;
        if let Some(rest @ Tag::Compound(_)) = yoake.get("rest") {
            state.rest.home = block_pos(rest.get("home"));
            state.rest.place = block_pos(rest.get("place"));
        }
    }
    super::attrs::sync_scale(m, None);
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

/// The key of the seats of the riders, in the `yoake` compound of a custom vehicle (F4M-4d).
const PASSENGER_SEATS: &str = "passenger_seats";

/// The seat of each rider of custom vehicle `m` in `riders` (their entity ids, in the order they are
/// saved under `Passengers`): the seat index, or -1 for one that holds none. The engine's own
/// `Passengers` list carries the riders themselves (it already supports a list); this is only which seat
/// of the type each one sits on, so the riders keep their seats across a save.
pub fn passenger_seats(m: &Mob, riders: &[u32]) -> Vec<i32> {
    riders
        .iter()
        .map(|&r| super::ai::riding::claim_of(m, r).and_then(|s| i32::try_from(s).ok()).unwrap_or(-1))
        .collect()
}

/// Writes `passenger_seats` of the riders `riders` of `m` into the `yoake` compound of `tag` (nothing, and
/// a stale list removed, when no rider holds a seat or `m` is no custom mob).
pub fn write_passenger_seats(tag: &mut Tag, m: &Mob, riders: &[u32]) {
    if m.kind.custom.is_none() {
        return;
    }
    let seats = passenger_seats(m, riders);
    let Some(Tag::Compound(yoake)) = tag.get_mut("yoake") else { return };
    if seats.iter().all(|&s| s < 0) {
        yoake.remove(&PASSENGER_SEATS.into());
    } else {
        yoake.insert(PASSENGER_SEATS.into(), Tag::List(seats.into_iter().map(Tag::Int).collect()));
    }
}

/// The saved seats of a custom vehicle's riders, in `Passengers` order (empty when it has none).
pub fn read_passenger_seats(tag: &Tag) -> Vec<i32> {
    match tag.get("yoake").and_then(|y| y.get(PASSENGER_SEATS)) {
        Some(Tag::List(l)) => {
            l.iter().map(|t| number(Some(t)).and_then(|n| i32::try_from(n).ok()).unwrap_or(-1)).collect()
        }
        _ => Vec::new(),
    }
}

/// The seat claims of the riders that were loaded (`loaded[n]` is the entity id of the n-th saved
/// rider, `None` for one that did not load) from the saved `seats` of the same order.
pub fn claims_from(seats: &[i32], loaded: &[Option<u32>]) -> Vec<super::SeatClaim> {
    seats
        .iter()
        .zip(loaded)
        .filter_map(|(&s, r)| Some(super::SeatClaim { rider: (*r)?, seat: u8::try_from(s).ok()? }))
        .collect()
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

    /// F4M-4c: the owner, the order to sit, the breeding age and the love ticks are saved in the
    /// `yoake` compound, come back as they were, the second save is the first, and an untamed mob
    /// writes none of them (and drops them from an older save).
    #[test]
    fn the_owner_the_sit_order_and_the_breeding_state_survive_a_save_and_load() {
        use crate::mobs::custom::ai::tame;
        let types = types();
        let mut m = fox(&types);
        let fresh = mob_to_nbt(&m, UUID, None, None);
        for key in ["owner", "sitting", "age", "love"] {
            assert!(fresh.get("yoake").unwrap().get(key).is_none(), "a fresh mob has no {key}");
        }
        tame::tame(&mut m, [11, 22, 33, 44]);
        assert!(m.extra.sitting, "the fox sits on command");
        m.extra.age = -12000;
        let tag = mob_to_nbt(&m, UUID, None, None);
        let y = tag.get("yoake").unwrap();
        assert_eq!(y.get("owner"), Some(&Tag::IntArray(vec![11, 22, 33, 44])));
        assert_eq!(number(y.get("sitting")), Some(1));
        assert_eq!(number(y.get("age")), Some(-12000));
        assert!(y.get("love").is_none());
        let back = mob_from_nbt(&tag, &types, 9, None).unwrap();
        assert_eq!(back.extra.owner, Some([11, 22, 33, 44]));
        assert!(back.extra.tame && back.extra.sitting);
        assert_eq!(back.extra.age, -12000);
        assert_eq!(mob_to_nbt(&back, UUID, Some(&tag), None), tag, "the second save is the first");
        // The love ticks of an adult.
        let mut lover = fox(&types);
        lover.extra.in_love = 300;
        let tag = mob_to_nbt(&lover, UUID, None, None);
        assert_eq!(number(tag.get("yoake").unwrap().get("love")), Some(300));
        assert_eq!(mob_from_nbt(&tag, &types, 9, None).unwrap().extra.in_love, 300);
        // Un-taming drops the keys from the compound the mob was loaded with.
        let mut again = mob_from_nbt(&mob_to_nbt(&m, UUID, None, None), &types, 9, None).unwrap();
        let old = mob_to_nbt(&again, UUID, None, None);
        tame::untame(&mut again);
        again.extra.age = 0;
        let now = mob_to_nbt(&again, UUID, Some(&old), None);
        for key in ["owner", "sitting", "age"] {
            assert!(now.get("yoake").unwrap().get(key).is_none(), "{key} is gone");
        }
        // The entity compound of a vanilla mob never gets any of it.
        let cow = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
        assert!(mob_to_nbt(&cow, UUID, None, None).get("yoake").is_none());
    }

    /// Vanilla summon NBT makes a custom mob a baby even before it has a `yoake` compound; a saved
    /// `yoake.age` still wins over the stale input key on a later load.
    #[test]
    fn vanilla_age_nbt_makes_a_custom_mob_a_baby() {
        let types = types();
        let mut tag = mob_to_nbt(&fox(&types), UUID, None, None);
        let Tag::Compound(root) = &mut tag else { panic!("entity compound") };
        root.remove(&"yoake".into());
        root.insert("Age".into(), Tag::Int(-24000));
        let baby = mob_from_nbt(&tag, &types, 9, None).unwrap();
        assert_eq!(baby.extra.age, -24000);
        assert!(baby.is_baby());
        let saved = mob_to_nbt(&baby, UUID, Some(&tag), None);
        assert_eq!(number(saved.get("yoake").unwrap().get("age")), Some(-24000));
        assert_eq!(mob_from_nbt(&saved, &types, 9, None).unwrap().extra.age, -24000);
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

#[cfg(test)]
mod all_kinds_tests {
    use super::*;
    use crate::{
        entity_nbt::{mob_from_nbt, mob_to_nbt},
        mobs::{MobType, custom::build::mob_type},
    };
    use horizon_core::geometry::Vec3;
    use horizon_mods::custom_mob::CustomMobFile;
    use std::{collections::HashMap, sync::Arc};

    /// A type with one value of every kind (and one that is not persistent).
    fn all_kinds() -> HashMap<String, Arc<MobType>> {
        let json = r#"{"template":"yoake:mob","size":{"width":1,"height":1},"synced":{
            "a_bool":{"type":"bool","persistent":true},
            "b_int":{"type":"i32","min":-5,"max":5,"persistent":true},
            "c_float":{"type":"f32","persistent":true},
            "d_text":{"type":"string","persistent":true},
            "e_vec":{"type":"vec3","persistent":true},
            "f_enum":{"type":"enum","values":["x","y","z"],"default":"x","persistent":true},
            "g_temp":{"type":"i32"}}}"#;
        let file = CustomMobFile::parse("demo:thing", json.as_bytes()).unwrap();
        let t = Arc::new(CustomType::new(file, None).unwrap());
        let kind = Arc::new(mob_type(&t));
        HashMap::from([(kind.id.clone(), kind)])
    }

    /// Every kind of persistent value round trips, a value that is not persistent does not, and
    /// loading marks nothing dirty (the first snapshot sends everything anyway).
    #[test]
    fn every_kind_of_persistent_synced_value_round_trips() {
        let types = all_kinds();
        let mut m = Mob::new(1, types["demo:thing"].clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
        let t = m.kind.custom.clone().unwrap();
        let s = m.extra.custom.as_mut().unwrap();
        let set = [
            ("a_bool", SyncedValue::Bool(true)),
            ("b_int", SyncedValue::I32(-4)),
            ("c_float", SyncedValue::F32(2.5)),
            ("d_text", SyncedValue::Str("héllo \u{1F98A}".into())),
            ("e_vec", SyncedValue::Vec3([1.0, -2.0, 3.5])),
            ("f_enum", SyncedValue::Enum(2)),
            ("g_temp", SyncedValue::I32(99)),
        ];
        for (name, v) in &set {
            assert!(s.set_synced(&t, name, v.clone()), "{name}");
        }
        let tag = mob_to_nbt(&m, [1, 2, 3, 4], None, None);
        let saved = tag.get("yoake").unwrap().get("synced").unwrap();
        assert!(saved.get("g_temp").is_none(), "not persistent, not saved");
        let back = mob_from_nbt(&tag, &types, 2, None).unwrap();
        let b = back.extra.custom.as_ref().unwrap();
        for (name, v) in &set[..6] {
            assert_eq!(b.synced_value(&t, name), Some(v), "{name}");
        }
        assert_eq!(b.synced_value(&t, "g_temp"), Some(&SyncedValue::I32(0)), "back to the default");
        assert_eq!(b.synced_dirty, 0);
        assert_eq!(mob_to_nbt(&back, [1, 2, 3, 4], Some(&tag), None), tag);
    }

    /// `/summon ... {Age:-24000}` (no `yoake` compound): `sync_scale` runs immediately on load
    /// so baby bounds, attributes and seat offsets start at `baby_scale` before the first tick.
    #[test]
    fn summoned_baby_custom_mob_syncs_scale_without_tick() {
        let json = r#"{"template":"yoake:mob","category":"creature",
            "size":{"width":0.6,"height":0.7},
            "breedable":{"food":["minecraft:sweet_berries"],"baby_scale":0.5}}"#;
        let file = CustomMobFile::parse("demo:fox", json.as_bytes()).unwrap();
        let t = Arc::new(CustomType::new(file, None).unwrap());
        let types = HashMap::from([(t.id.clone(), Arc::new(mob_type(&t)))]);
        let summon = Tag::compound([
            ("id", Tag::String("demo:fox".into())),
            (
                "Pos",
                Tag::List(vec![
                    Tag::Double(0.5_f64.to_bits()),
                    Tag::Double(64.0_f64.to_bits()),
                    Tag::Double(0.5_f64.to_bits()),
                ]),
            ),
            ("Age", Tag::Int(-24000)),
        ]);
        let m = mob_from_nbt(&summon, &types, 4, None).unwrap();
        assert!(m.is_baby());
        assert!((m.model_scale() - 0.5).abs() < 1e-6, "got {}", m.model_scale());
    }
}
