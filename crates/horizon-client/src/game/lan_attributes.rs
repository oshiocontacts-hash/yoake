//! The attribute instances a server syncs for the living entities it tracks
//! (`ClientboundUpdateAttributesPacket`, lan/attributes.rs): the ones
//! vanilla's `AttributeMap` would have created by then, since only those are
//! sent, never the type's other defaults.
use super::Game;
use super::commands::Target;
use horizon_client::{
    effects,
    lan::attributes::{add_modifier, generic_syncable, instance, operation_named},
    mobs::{
        Mob, Species,
        finalize::{self, Attr, Op},
    },
};
use horizon_protocol::packets::play::{AttributeModifier, AttributeSnapshot};
use std::sync::Arc;

/// `EquipmentSlotGroup.test`: whether a modifier's slot group covers a slot.
fn slot_in_group(group: &str, slot: &str) -> bool {
    group == slot
        || group == "any"
        || (group == "hand" && matches!(slot, "mainhand" | "offhand"))
        || (group == "armor" && matches!(slot, "feet" | "legs" | "chest" | "head" | "body"))
}

fn generic_attributes_into(
    map: &horizon_world::attributes::entity::EntityAttributes,
    out: &mut Vec<AttributeSnapshot>,
) {
    for s in generic_syncable(map) {
        let (attribute, base) = (s.attribute, s.base);
        *instance(out, attribute, base) = s;
    }
}

/// Syncable registry id for `minecraft:scale` on a custom mob that carries a
/// non-unit scale (`breedable.baby_scale`, `scale` attribute, or `model_scale`).
/// Custom types are absent from `attributes.json`'s `entities` table, so the
/// attribute is resolved directly from the catalog rather than `default_base`.
fn custom_scale_attr(
    catalog: &horizon_world::attributes::entity::Catalog,
    m: &Mob,
) -> Option<i32> {
    let custom = m.kind.custom.as_deref()?;
    if m.extra.model_scale <= 0.0
        && custom.breed.is_none()
        && custom.builtin_value("scale").is_none()
    {
        return None;
    }
    let id = catalog.id("minecraft:scale")?;
    catalog.attribute(id).syncable.then(|| id.registry_id())
}

impl Game<'_> {
    /// `getAttributeValue(KNOCKBACK_RESISTANCE)` of a mob: its own value plus
    /// the modifiers of the items it wears or holds in matching slots.
    pub(super) fn mob_knockback_resistance(&self, m: &Mob) -> f64 {
        let e = &m.extra;
        let slots = [
            ("mainhand", &e.main_hand),
            ("offhand", &e.off_hand),
            ("feet", &e.feet),
            ("legs", &e.legs),
            ("chest", &e.chest),
            ("head", &e.head),
        ];
        let mut extra = Vec::new();
        for (slot, item) in slots {
            let Some(info) = item.as_ref().and_then(|i| self.items().get(i)) else {
                continue;
            };
            for (attribute, _, amount, op, group) in &info.modifiers {
                if attribute == "minecraft:knockback_resistance" && slot_in_group(group, slot) {
                    let op = match operation_named(op) {
                        1 => Op::AddMultipliedBase,
                        2 => Op::AddMultipliedTotal,
                        _ => Op::AddValue,
                    };
                    extra.push((*amount, op));
                }
            }
        }
        finalize::value_with(m, Attr::KnockbackResistance, &extra)
    }

    /// Each mob's `gear_knockback_resistance`, for the pushes that read the
    /// mob (`Mob::knockback`, Punch, the mace's smash): worked out for the
    /// mobs that wear or hold something, since the mob does not know items.
    pub(super) fn refresh_gear_knockback_resistance(&mut self) {
        let values: Vec<Option<f64>> = self
            .mobs
            .iter()
            .map(|m| {
                let e = &m.extra;
                [
                    &e.main_hand,
                    &e.off_hand,
                    &e.feet,
                    &e.legs,
                    &e.chest,
                    &e.head,
                ]
                .iter()
                .any(|s| s.is_some())
                .then(|| self.mob_knockback_resistance(m))
            })
            .collect();
        for (m, value) in self.mobs.iter_mut().zip(values) {
            m.gear_knockback_resistance = value;
        }
    }

    /// The registry id of a client-syncable attribute the entity type has
    /// (`AttributeMap.getInstance` is null for the others), by name.
    fn synced_attribute(&self, entity: &str, name: &str) -> Option<i32> {
        let catalog = &self.motion.attributes;
        let id = catalog.id(name)?;
        (catalog.attribute(id).syncable && catalog.default_base(entity, id).is_some())
            .then(|| id.registry_id())
    }

    /// The type's base value of an attribute, as the instance starts.
    fn attribute_default(&self, entity: &str, name: &str) -> f64 {
        let catalog = &self.motion.attributes;
        catalog
            .id(name)
            .and_then(|id| catalog.default_base(entity, id))
            .unwrap_or(0.0)
    }

    /// `LivingEntity.collectEquipmentChanges`: each worn or held item's
    /// attribute modifiers, slot by slot in `EquipmentSlot` order, create
    /// their instances (an instance without a modifier keeps its base).
    fn equipment_attributes<'a>(
        &self,
        entity: &str,
        base: impl Fn(&str) -> Option<f64>,
        slots: impl IntoIterator<Item = (&'static str, Option<&'a Arc<str>>)>,
        out: &mut Vec<AttributeSnapshot>,
    ) {
        for (slot, item) in slots {
            let Some(info) = item.and_then(|i| self.items().get(i)) else {
                continue;
            };
            for (attribute, id, amount, op, group) in &info.modifiers {
                if !slot_in_group(group, slot) {
                    continue;
                }
                let Some(a) = self.synced_attribute(entity, attribute) else {
                    continue;
                };
                let start =
                    base(attribute).unwrap_or_else(|| self.attribute_default(entity, attribute));
                add_modifier(
                    out,
                    a,
                    start,
                    AttributeModifier {
                        id: id.clone(),
                        amount: *amount,
                        operation: operation_named(op),
                    },
                );
            }
        }
    }

    /// A mob's synced instances: movement speed (its first `aiStep` reads it,
    /// `LivingEntity.removeFrost`), what finalizeSpawn set, the baby speed
    /// modifier, its effects' modifiers, its equipment's, and what
    /// `/attribute` changed.
    pub(super) fn mob_attributes(&self, m: &Mob, out: &mut Vec<AttributeSnapshot>) {
        let kind = m.kind.id.as_str();
        // Movement speed is synced for nearly every mob, and read up to three
        // times below: looked up once per mob and tick.
        let speed = self
            .synced_attribute(kind, Attr::MovementSpeed.id())
            .map(|a| (a, finalize::base_value(m, Attr::MovementSpeed)));
        if let Some((a, base)) = speed {
            instance(out, a, base);
        }
        if let Some(s) = m.extra.attributes.as_deref() {
            for (attr, v) in &s.bases {
                if let Some(a) = self.synced_attribute(kind, attr.id()) {
                    instance(out, a, *v).base = *v;
                }
            }
            for x in &s.modifiers {
                if let Some(a) = self.synced_attribute(kind, x.attr.id()) {
                    add_modifier(
                        out,
                        a,
                        finalize::base_value(m, x.attr),
                        AttributeModifier {
                            id: x.id.to_owned(),
                            amount: x.amount,
                            operation: match x.op {
                                Op::AddValue => 0,
                                Op::AddMultipliedBase => 1,
                                Op::AddMultipliedTotal => 2,
                            },
                        },
                    );
                }
            }
        }
        // Custom mobs fold their scale attribute and `breedable.baby_scale` into
        // `extra.model_scale` (`custom::attrs::sync_scale`); sync `minecraft:scale`
        // so clients size and seat babies accurately and return to 1.0 on growth.
        // Custom types live in entity-types.json rather than vanilla attributes.json's
        // `entities` table, so look up the attribute directly in the catalog.
        if let Some(a) = custom_scale_attr(&self.motion.attributes, m) {
            instance(out, a, f64::from(m.model_scale()));
        }
        // AbstractHorse.randomizeAttributes rolls a horse's jump strength as
        // its base (kept in `vector[0]`), so the instance is synced: its
        // rider's client jumps it (`getCustomJump`).
        let rolled = matches!(
            m.species(),
            Species::Horse | Species::SkeletonHorse | Species::ZombieHorse
        );
        let jump_base = m
            .extra
            .attributes
            .as_deref()
            .is_some_and(|s| s.bases.iter().any(|(a, _)| *a == Attr::JumpStrength));
        if rolled
            && !jump_base
            && m.extra.vector[0] > 0.0
            && let Some(a) = self.synced_attribute(kind, Attr::JumpStrength.id())
        {
            instance(out, a, m.extra.vector[0]).base = m.extra.vector[0];
        }
        if let (Some((a, base)), Some((amount, Op::AddMultipliedBase))) =
            (speed, finalize::transient(m, Attr::MovementSpeed))
        {
            add_modifier(
                out,
                a,
                base,
                AttributeModifier {
                    id: "minecraft:baby".into(),
                    amount,
                    operation: 1,
                },
            );
        }
        // LivingEntity.setSprinting: SPEED_MODIFIER_SPRINTING (a dashing cat or ocelot).
        if let (true, Some((a, base))) = (m.extra.sprinting, speed) {
            add_modifier(
                out,
                a,
                base,
                AttributeModifier {
                    id: "minecraft:sprinting".into(),
                    amount: f64::from(0.3f32),
                    operation: 2,
                },
            );
        }
        let base = |name: &str| {
            Attr::ALL
                .into_iter()
                .find(|a| name.strip_prefix("minecraft:") == Some(a.name()))
                .map(|a| finalize::base_value(m, a))
        };
        // LivingEntity.onEffectAdded: MobEffect.addAttributeModifiers.
        for &(effect, _, amplifier) in &m.extra.effects {
            let Some((attribute, id, amount, op)) =
                effects::effect_modifier(effect.trim_start_matches("minecraft:"))
            else {
                continue;
            };
            let Some(a) = self.synced_attribute(kind, attribute) else {
                continue;
            };
            let start = base(attribute).unwrap_or_else(|| self.attribute_default(kind, attribute));
            add_modifier(
                out,
                a,
                start,
                AttributeModifier {
                    id: id.to_owned(),
                    amount: amount * f64::from(amplifier + 1),
                    operation: operation_named(op),
                },
            );
        }
        let e = &m.extra;
        self.equipment_attributes(
            kind,
            base,
            [
                ("mainhand", e.main_hand.as_ref()),
                ("offhand", e.off_hand.as_ref()),
                ("feet", e.feet.as_ref()),
                ("legs", e.legs.as_ref()),
                ("chest", e.chest.as_ref()),
                ("head", e.head.as_ref()),
            ],
            out,
        );
        self.generic_attributes_of(Target::Mob(m.id), out);
        if let Some(map) = m.extra.armor_attributes.as_deref() {
            generic_attributes_into(map, out);
        }
    }

    /// An armor stand's synced instances: movement speed (its first
    /// `aiStep`), its equipment's modifiers and what `/attribute` changed.
    pub(super) fn stand_attributes(
        &self,
        id: u32,
        equipment: &[Option<horizon_client::items::ItemStack>; 6],
    ) -> Vec<AttributeSnapshot> {
        const KIND: &str = "minecraft:armor_stand";
        let mut out = Vec::new();
        if let Some(a) = self.synced_attribute(KIND, "minecraft:movement_speed") {
            instance(
                &mut out,
                a,
                self.attribute_default(KIND, "minecraft:movement_speed"),
            );
        }
        let slots = horizon_client::entities::ArmorStand::SLOTS
            .into_iter()
            .zip(equipment.iter().map(|s| s.as_ref().map(|s| &s.item)));
        self.equipment_attributes(KIND, |_| None, slots, &mut out);
        self.generic_attributes_of(Target::Stand(id), &mut out);
        out
    }

    /// The instances a server player's tick creates, and
    /// `ServerPlayer.updatePlayerAttributes`: a creative player reaches 0.5
    /// further to blocks and 2 to entities. Its client reads the reach from
    /// these synced modifiers. (The crouching player's waypoint range is
    /// server-side only: lan_waypoints.rs hides it.)
    pub(super) fn update_player_attributes(&mut self) {
        let creative = self.mode_id() == 1;
        let Some(p) = self.player.as_mut() else {
            return;
        };
        // LivingEntity.aiStep's removeFrost reads the movement speed instance
        // every tick, which creates it (and its first sync).
        if let Some(key) = p.attributes.key("minecraft:movement_speed") {
            let _ = p.attributes.get_instance(Some(key));
        }
        for (attribute, id, amount) in [
            (
                "minecraft:block_interaction_range",
                "minecraft:creative_mode_block_range",
                0.5,
            ),
            (
                "minecraft:entity_interaction_range",
                "minecraft:creative_mode_entity_range",
                2.0,
            ),
        ] {
            let Some(key) = p.attributes.key(attribute) else {
                continue;
            };
            // getAttribute creates the instance, a creative player's or not.
            if !matches!(p.attributes.get_instance(Some(key.clone())), Ok(Some(_))) {
                continue;
            }
            // addOrUpdateTransientModifier with the same modifier and
            // removeModifier of a missing one leave the instance clean.
            let has = p.attributes.has_modifier(Some(key), id).unwrap_or(false);
            if has != creative {
                let _ = p.set_modifier(attribute, id, amount, "add_value", creative);
            }
        }
    }

    /// The instances `/attribute` created in an entity's generic map
    /// replace the ones derived from its state.
    fn generic_attributes_of(&self, t: Target, out: &mut Vec<AttributeSnapshot>) {
        if let Some(map) = self.cmds.attributes.get(&t) {
            generic_attributes_into(map, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::slot_in_group;

    #[test]
    fn slot_groups_cover_their_slots() {
        assert!(slot_in_group("head", "head"));
        assert!(slot_in_group("armor", "chest"));
        assert!(!slot_in_group("armor", "mainhand"));
        assert!(slot_in_group("hand", "offhand"));
        assert!(slot_in_group("any", "feet"));
        assert!(!slot_in_group("mainhand", "offhand"));
    }

    #[test]
    fn custom_mob_scale_attr_resolves_without_entity_entry_in_attributes_json() {
        use horizon_client::mobs::{
            Mob,
            custom::{CustomType, attrs, build},
        };
        use horizon_core::geometry::Vec3;
        use horizon_mods::custom_mob::CustomMobFile;
        use horizon_world::{DATA_VERSION, attributes::entity::Catalog};
        use std::sync::Arc;

        let catalog = Catalog::from_json(&serde_json::json!({
            "data_version": DATA_VERSION,
            "attributes": [{
                "id": "minecraft:scale",
                "default": format!("{:016x}", 1.0_f64.to_bits()),
                "syncable": true
            }],
            "entities": {}
        }))
        .unwrap();
        let json = br#"{"template":"yoake:mob","category":"creature","size":{"width":0.6,"height":0.7},
            "breedable":{"food":["minecraft:sweet_berries"],"baby_scale":0.5}}"#;
        let file = CustomMobFile::parse("demo:fox", json).unwrap();
        let kind = Arc::new(build::mob_type(&Arc::new(CustomType::new(file, None).unwrap())));
        let mut m = Mob::new(1, kind, Vec3::ZERO, 0.0);
        m.extra.age = -24000;
        attrs::sync_scale(&mut m, None);
        assert_eq!(super::custom_scale_attr(&catalog, &m), Some(0));
        assert!((m.model_scale() - 0.5).abs() < 1e-6);
    }
}
