//! Stock-client proxy mapping and brand-gate helpers for custom mobs (F4M-5a,
//! docs/yoake2/FR-F4-MOBS.md section 3.9).
//!
//! A custom mob type with `"proxy": "minecraft:<entity>"` (and optional `"proxy_name_tag"`) is
//! sent to a stock (`brand != "horizon"`) client as that vanilla entity type with only metadata
//! known to be valid for that vanilla type. A custom mob type without `proxy` causes stock clients
//! to be refused up front in `lan/conn.rs` and is never spawned for a stock viewer in any case.

use crate::mobs::{
    Species,
    custom::{CustomRegistry, registry},
    finalize, species,
};
use horizon_protocol::{
    entity_data::{Value, serializer},
    text::literal,
};
use std::borrow::Cow;

/// The vanilla entity representation used when sending a proxied custom mob to a stock client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProxyTarget {
    pub kind: String,
    pub name_tag: Option<String>,
}

/// Vanilla entity class hierarchy buckets used to determine which `SynchedEntityData` indices are
/// valid on a stock client. Any index outside the class's guaranteed range is dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProxyClass {
    /// Base `Entity` (`0..=7`).
    Entity,
    /// `LivingEntity` (`0..=14`): adds living flags, health, effect particles/ambience, arrows, bed.
    Living,
    /// `Mob` (`0..=15`): adds `Mob.DATA_MOB_FLAGS_ID` (15).
    Mob,
    /// `AgeableMob` (`0..=15`, `16` baby bool, `17` age_locked bool).
    AgeableMob,
    /// `Zombie` / `Husk` / `Drowned` / `ZombieVillager` / `Zoglin` (`0..=15`, `16` baby bool).
    BabyAt16Mob,
}

/// Looks up the stock-client proxy target for `kind` in the process-wide custom mob registry.
pub fn proxy_for(kind: &str) -> Option<ProxyTarget> {
    registry().read().ok().and_then(|r| proxy_in(&r, kind))
}

/// Looks up the stock-client proxy target for `kind` in `reg`.
pub fn proxy_in(reg: &CustomRegistry, kind: &str) -> Option<ProxyTarget> {
    let custom = reg.get(kind)?;
    let proxy_kind = custom.file.proxy.as_ref()?;
    Some(ProxyTarget {
        kind: proxy_kind.clone(),
        name_tag: custom.file.proxy_name_tag.clone(),
    })
}

/// Whether any loaded custom mob type lacks a `proxy` for stock clients.
pub fn any_without_proxy() -> bool {
    registry().read().is_ok_and(|r| any_without_proxy_in(&r))
}

/// Whether any custom mob type in `reg` lacks a `proxy` for stock clients.
pub fn any_without_proxy_in(reg: &CustomRegistry) -> bool {
    reg.iter().any(|t| t.file.proxy.is_none())
}

/// Resolves the entity type id string sent on the wire for `kind`.
///
/// - Horizon clients (`stock == false`) always receive `kind`.
/// - Stock clients (`stock == true`) receive the vanilla `proxy` id when `kind` is a custom mob
///   with `proxy`, `None` when `kind` is a custom mob without `proxy` (never spawn), and `kind`
///   unchanged for non-custom entities.
pub fn wire_kind<'a>(kind: &'a str, stock: bool) -> Option<Cow<'a, str>> {
    if !stock {
        return Some(Cow::Borrowed(kind));
    }
    let guard = registry().read().ok()?;
    if guard.is_empty() {
        return Some(Cow::Borrowed(kind));
    }
    wire_kind_in(&guard, kind, stock).map(|c| Cow::Owned(c.into_owned()))
}

/// Resolves the entity type id string sent on the wire for `kind` against `reg`.
pub fn wire_kind_in<'a>(reg: &CustomRegistry, kind: &'a str, stock: bool) -> Option<Cow<'a, str>> {
    if !stock {
        return Some(Cow::Borrowed(kind));
    }
    match reg.get(kind) {
        Some(custom) => custom
            .file
            .proxy
            .as_deref()
            .map(|p| Cow::Owned(p.to_owned())),
        None => Some(Cow::Borrowed(kind)),
    }
}

/// Whether the vanilla proxy entity `proxy_kind` is a `LivingEntity` (accepts `UpdateAttributes`
/// and `SetEquipment`).
pub fn is_living(proxy_kind: &str) -> bool {
    !matches!(classify(proxy_kind), ProxyClass::Entity)
}

fn classify(proxy_kind: &str) -> ProxyClass {
    let Some(name) = proxy_kind.strip_prefix("minecraft:") else {
        return ProxyClass::Entity;
    };
    if let Some(def) = species::ALL.iter().find(|d| d.name == name) {
        return match def.species {
            Species::Mannequin => ProxyClass::Living,
            Species::Zombie
            | Species::Husk
            | Species::Drowned
            | Species::ZombieVillager
            | Species::Zoglin => ProxyClass::BabyAt16Mob,
            s if is_ageable_mob(s) => ProxyClass::AgeableMob,
            _ => ProxyClass::Mob,
        };
    }
    if matches!(name, "armor_stand" | "player") {
        ProxyClass::Living
    } else {
        ProxyClass::Entity
    }
}

fn is_ageable_mob(s: Species) -> bool {
    finalize::ageable(s)
        && !matches!(
            s,
            Species::Slime | Species::MagmaCube | Species::SulfurCube | Species::ZombieNautilus
        )
}

fn valid_meta_entry(class: ProxyClass, index: u8, ser: i32) -> bool {
    match (index, ser) {
        (0, serializer::BYTE)
        | (1, serializer::INT)
        | (2, serializer::OPTIONAL_COMPONENT)
        | (3, serializer::BOOLEAN)
        | (4, serializer::BOOLEAN)
        | (5, serializer::BOOLEAN)
        | (6, serializer::POSE)
        | (7, serializer::INT) => true,
        (8, serializer::BYTE)
        | (9, serializer::FLOAT)
        | (10, serializer::PARTICLES)
        | (11, serializer::BOOLEAN)
        | (12, serializer::INT)
        | (13, serializer::INT)
        | (14, serializer::OPTIONAL_BLOCK_POS) => !matches!(class, ProxyClass::Entity),
        (15, serializer::BYTE) => matches!(
            class,
            ProxyClass::Mob | ProxyClass::AgeableMob | ProxyClass::BabyAt16Mob
        ),
        (16, serializer::BOOLEAN) => {
            matches!(class, ProxyClass::AgeableMob | ProxyClass::BabyAt16Mob)
        }
        (17, serializer::BOOLEAN) => matches!(class, ProxyClass::AgeableMob),
        _ => false,
    }
}

/// Filters `meta` down to entries valid for `proxy.kind` and applies `proxy.name_tag` when the
/// mob has no explicit custom name.
pub fn stock_meta(proxy: &ProxyTarget, meta: &[(u8, i32, Value)]) -> Vec<(u8, i32, Value)> {
    let class = classify(&proxy.kind);
    let mut out: Vec<(u8, i32, Value)> = meta
        .iter()
        .filter(|(idx, ser, _)| valid_meta_entry(class, *idx, *ser))
        .cloned()
        .collect();
    if let Some(tag) = proxy.name_tag.as_deref().filter(|t| !t.is_empty()) {
        if !out.iter().any(|(idx, ..)| *idx == 2) {
            out.push((
                2,
                serializer::OPTIONAL_COMPONENT,
                Value::OptText(Some(literal(tag))),
            ));
        }
        if !out.iter().any(|(idx, ..)| *idx == 3) {
            out.push((3, serializer::BOOLEAN, Value::Bool(true)));
        }
    }
    out
}

/// Filters `defaults` down to entries valid for `proxy.kind`, keeping `proxy.name_tag` as the
/// fallback name when an explicit custom name is cleared.
pub fn stock_defaults(proxy: &ProxyTarget, defaults: &[(u8, i32, Value)]) -> Vec<(u8, i32, Value)> {
    let class = classify(&proxy.kind);
    let mut out: Vec<(u8, i32, Value)> = defaults
        .iter()
        .filter(|(idx, ser, _)| valid_meta_entry(class, *idx, *ser))
        .cloned()
        .collect();
    if let Some(tag) = proxy.name_tag.as_deref().filter(|t| !t.is_empty()) {
        out.retain(|(idx, ..)| *idx != 2 && *idx != 3);
        out.push((
            2,
            serializer::OPTIONAL_COMPONENT,
            Value::OptText(Some(literal(tag))),
        ));
        out.push((3, serializer::BOOLEAN, Value::Bool(true)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        fast_hash::FxHashSet,
        lan::{
            data::SyncData,
            entity::{Entities, EntityState, Riding, Tracker, Viewer},
            hub::entity_type_with,
        },
        mobs::custom::CustomType,
    };
    use horizon_mods::custom_mob::CustomMobFile;
    use horizon_protocol::{packets::play::Clientbound as C, registry::Ids};
    use std::{collections::HashMap, sync::Arc};

    fn custom_type(id: &str, proxy: Option<&str>, name_tag: Option<&str>) -> Arc<CustomType> {
        let mut obj = serde_json::json!({
            "template": "yoake:mob",
            "size": { "width": 0.6, "height": 0.7 }
        });
        if let Some(p) = proxy {
            obj["proxy"] = serde_json::json!(p);
        }
        if let Some(tag) = name_tag {
            obj["proxy_name_tag"] = serde_json::json!(tag);
        }
        let file = CustomMobFile::parse(id, obj.to_string().as_bytes()).unwrap();
        Arc::new(CustomType::new(file, None).unwrap())
    }

    #[test]
    fn proxy_lookup_returns_vanilla_target_and_name_tag() {
        let mut reg = CustomRegistry::default();
        assert!(reg.insert(custom_type(
            "demo:cinder_fox",
            Some("minecraft:fox"),
            Some("Cinder Fox")
        )));
        assert!(reg.insert(custom_type("demo:shadow", None, None)));

        assert_eq!(
            proxy_in(&reg, "demo:cinder_fox"),
            Some(ProxyTarget {
                kind: "minecraft:fox".into(),
                name_tag: Some("Cinder Fox".into()),
            })
        );
        assert_eq!(proxy_in(&reg, "demo:shadow"), None);
        assert_eq!(proxy_in(&reg, "minecraft:pig"), None);

        assert_eq!(
            wire_kind_in(&reg, "demo:cinder_fox", true).as_deref(),
            Some("minecraft:fox")
        );
        assert_eq!(
            wire_kind_in(&reg, "demo:cinder_fox", false).as_deref(),
            Some("demo:cinder_fox")
        );
        assert_eq!(
            wire_kind_in(&reg, "minecraft:pig", true).as_deref(),
            Some("minecraft:pig")
        );
    }

    #[test]
    fn no_proxy_detection_finds_any_unproxied_custom_type() {
        let mut reg = CustomRegistry::default();
        assert!(!any_without_proxy_in(&reg));

        assert!(reg.insert(custom_type("demo:fox", Some("minecraft:fox"), None)));
        assert!(!any_without_proxy_in(&reg));

        assert!(reg.insert(custom_type("demo:no_proxy", None, None)));
        assert!(any_without_proxy_in(&reg));

        assert_eq!(wire_kind_in(&reg, "demo:no_proxy", true), None);
        assert_eq!(
            wire_kind_in(&reg, "demo:no_proxy", false).as_deref(),
            Some("demo:no_proxy")
        );
    }

    #[test]
    fn stock_metadata_filters_to_valid_vanilla_indices_and_applies_name_tag() {
        let raw = vec![
            (0, serializer::BYTE, Value::Byte(1)),
            (6, serializer::POSE, Value::Id(2)),
            (9, serializer::FLOAT, Value::Float(12.0)),
            (15, serializer::BYTE, Value::Byte(4)),
            (16, serializer::BOOLEAN, Value::Bool(true)),
            (17, serializer::BOOLEAN, Value::Bool(true)),
            (18, serializer::INT, Value::Int(99)),
        ];

        // AgeableMob (`minecraft:fox`) keeps 0..=17 and adds `proxy_name_tag`.
        let fox = ProxyTarget {
            kind: "minecraft:fox".into(),
            name_tag: Some("Cinder Fox".into()),
        };
        let fox_meta = stock_meta(&fox, &raw);
        let fox_indices: Vec<u8> = fox_meta.iter().map(|(i, ..)| *i).collect();
        assert_eq!(fox_indices, vec![0, 6, 9, 15, 16, 17, 2, 3]);
        assert!(fox_meta.contains(&(
            2,
            serializer::OPTIONAL_COMPONENT,
            Value::OptText(Some(literal("Cinder Fox")))
        )));

        // Non-ageable Mob (`minecraft:creeper`) drops 16, 17, 18.
        let creeper = ProxyTarget {
            kind: "minecraft:creeper".into(),
            name_tag: None,
        };
        let creeper_indices: Vec<u8> = stock_meta(&creeper, &raw)
            .iter()
            .map(|(i, ..)| *i)
            .collect();
        assert_eq!(creeper_indices, vec![0, 6, 9, 15]);

        // LivingEntity (`minecraft:armor_stand`) drops 15+.
        let stand = ProxyTarget {
            kind: "minecraft:armor_stand".into(),
            name_tag: None,
        };
        let stand_indices: Vec<u8> = stock_meta(&stand, &raw).iter().map(|(i, ..)| *i).collect();
        assert_eq!(stand_indices, vec![0, 6, 9]);

        // Non-living Entity (`minecraft:oak_boat`) keeps only 0..=7.
        let boat = ProxyTarget {
            kind: "minecraft:oak_boat".into(),
            name_tag: None,
        };
        let boat_indices: Vec<u8> = stock_meta(&boat, &raw).iter().map(|(i, ..)| *i).collect();
        assert_eq!(boat_indices, vec![0, 6]);
    }

    fn decode_packets(bodies: &[Vec<u8>]) -> Vec<C> {
        bodies
            .iter()
            .map(|body| {
                let mut r = horizon_protocol::buf::Reader::new(body);
                let id = r.varint().unwrap();
                C::decode(id, r.rest()).unwrap()
            })
            .collect()
    }

    #[test]
    fn stock_guest_never_gets_a_custom_protocol_id() {
        const FOX_ID: i32 = 42;
        const CUSTOM_FOX_ID: i32 = 200;
        const CUSTOM_NOPROXY_ID: i32 = 201;

        let mut names = vec![String::new(); 202];
        names[FOX_ID as usize] = "minecraft:fox".into();
        names[CUSTOM_FOX_ID as usize] = "mobproxytest:fox".into();
        names[CUSTOM_NOPROXY_ID as usize] = "mobproxytest:noproxy".into();

        let mut sync = SyncData::empty();
        sync.builtin
            .registries
            .insert("minecraft:entity_type".into(), Ids::from_names(names));

        let mut reg = CustomRegistry::default();
        assert!(reg.insert(custom_type(
            "mobproxytest:fox",
            Some("minecraft:fox"),
            Some("Cinder Fox")
        )));
        assert!(reg.insert(custom_type("mobproxytest:noproxy", None, None)));
        // Register in the global registry too so `Entities::tick`'s proxy lookup sees the same
        // definitions under unique `mobproxytest:*` ids.
        if let Ok(mut global) = registry().write() {
            global.insert(custom_type(
                "mobproxytest:fox",
                Some("minecraft:fox"),
                Some("Cinder Fox"),
            ));
            global.insert(custom_type("mobproxytest:noproxy", None, None));
        }

        let mut types = HashMap::new();
        assert_eq!(
            entity_type_with(&reg, &mut types, &sync, "mobproxytest:fox", true),
            Some(FOX_ID)
        );
        assert_eq!(
            entity_type_with(&reg, &mut types, &sync, "mobproxytest:fox", false),
            Some(CUSTOM_FOX_ID)
        );
        assert_eq!(
            entity_type_with(&reg, &mut types, &sync, "mobproxytest:noproxy", true),
            None
        );
        assert_eq!(
            entity_type_with(&reg, &mut types, &sync, "mobproxytest:noproxy", false),
            Some(CUSTOM_NOPROXY_ID)
        );

        let chunks: FxHashSet<(i32, i32)> = (-2..=2)
            .flat_map(|x| (-2..=2).map(move |z| (x, z)))
            .collect();
        let mut level = Entities::default();
        let mut horizon_tracker = Tracker::default();
        let mut stock_tracker = Tracker::default();

        let mut proxied = EntityState {
            id: 10,
            uuid: 10,
            kind: "mobproxytest:fox".into(),
            position: [1.0, 64.0, 1.0],
            meta: vec![
                (9, serializer::FLOAT, Value::Float(14.0)),
                (20, serializer::INT, Value::Int(77)),
            ],
            ..EntityState::default()
        };
        let unproxied = EntityState {
            id: 11,
            uuid: 11,
            kind: "mobproxytest:noproxy".into(),
            position: [2.0, 64.0, 1.0],
            ..EntityState::default()
        };

        let run_tick = |level: &mut Entities,
                        horizon_tracker: &mut Tracker,
                        stock_tracker: &mut Tracker,
                        types: &mut HashMap<String, i32>,
                        entities: &[&EntityState]| {
            let mut horizon_out = Vec::new();
            let mut stock_out = Vec::new();
            let mut viewers = [
                Viewer {
                    entity_id: 1,
                    spectator: false,
                    permission: 0,
                    stock: false,
                    moved: true,
                    position: [0.0, 64.0, 0.0],
                    view_distance: 8,
                    chunks: &chunks,
                    tracker: horizon_tracker,
                    out: &mut horizon_out,
                    gone: None,
                },
                Viewer {
                    entity_id: 2,
                    spectator: false,
                    permission: 0,
                    stock: true,
                    moved: true,
                    position: [0.0, 64.0, 0.0],
                    view_distance: 8,
                    chunks: &chunks,
                    tracker: stock_tracker,
                    out: &mut stock_out,
                    gone: None,
                },
            ];
            level.tick(entities, &Riding::default(), &mut viewers, |kind, stock| {
                entity_type_with(&reg, types, &sync, kind, stock)
            });
            (decode_packets(&horizon_out), decode_packets(&stock_out))
        };

        // Tick 1: spawn packets.
        let (h_pkts, s_pkts) = run_tick(
            &mut level,
            &mut horizon_tracker,
            &mut stock_tracker,
            &mut types,
            &[&proxied, &unproxied],
        );

        let h_adds: Vec<(i32, i32)> = h_pkts
            .iter()
            .filter_map(|p| match p {
                C::AddEntity(a) => Some((a.id, a.kind)),
                _ => None,
            })
            .collect();
        let s_adds: Vec<(i32, i32)> = s_pkts
            .iter()
            .filter_map(|p| match p {
                C::AddEntity(a) => Some((a.id, a.kind)),
                _ => None,
            })
            .collect();
        assert_eq!(h_adds, vec![(10, CUSTOM_FOX_ID), (11, CUSTOM_NOPROXY_ID)]);
        assert_eq!(s_adds, vec![(10, FOX_ID)]);
        assert!(!stock_tracker.is_tracking(11));

        // Stock viewer only gets valid metadata for `minecraft:fox` (9, plus proxy_name_tag 2 & 3),
        // never index 20.
        let s_meta = s_pkts.iter().find_map(|p| match p {
            C::SetEntityData(d) if d.id == 10 => Some(&d.values),
            _ => None,
        });
        let s_meta = s_meta.expect("proxied mob metadata sent to stock viewer");
        assert!(!s_meta.iter().any(|(idx, ..)| *idx == 20));
        assert!(s_meta.iter().any(|(idx, ..)| *idx == 9));
        assert!(s_meta.iter().any(|(idx, ..)| *idx == 2));

        // Tick 2: movement + hurt health update reaches the stock viewer for the proxied mob.
        proxied.position = [2.5, 64.0, 1.0];
        proxied.meta[0] = (9, serializer::FLOAT, Value::Float(9.0));
        let (_, s_pkts2) = run_tick(
            &mut level,
            &mut horizon_tracker,
            &mut stock_tracker,
            &mut types,
            &[&proxied, &unproxied],
        );
        assert!(s_pkts2.iter().any(|p| matches!(
            p,
            C::MoveEntityPos(m) if m.0.id == 10
        ) || matches!(
            p,
            C::EntityPositionSync(m) if m.id == 10
        )));
        assert!(s_pkts2.iter().any(|p| matches!(
            p,
            C::SetEntityData(d) if d.id == 10 && d.values.iter().any(|(i, ..)| *i == 9)
        )));
        assert!(s_pkts2.iter().all(|p| !matches!(
            p,
            C::AddEntity(a) if a.kind >= CUSTOM_FOX_ID
        )));

        // Tick 3: removal removes the proxied mob on the stock viewer cleanly.
        let (_, s_pkts3) = run_tick(
            &mut level,
            &mut horizon_tracker,
            &mut stock_tracker,
            &mut types,
            &[],
        );
        assert!(s_pkts3.iter().any(|p| matches!(
            p,
            C::RemoveEntities(r) if r.ids == [10]
        )));
    }
}
