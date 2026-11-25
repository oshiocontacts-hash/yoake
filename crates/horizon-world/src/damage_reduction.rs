//! Living damage-source bypass, armor and resistance/protection dispatch.
use crate::{
    combat_rules::{self, ArmorWorld},
    living_damage::{STAT_LIMIT, positive, stat_amount},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bypass {
    Armor,
    Effects,
    Resistance,
    Enchantments,
}
pub trait DamageReductionWorld: ArmorWorld {
    fn bypasses(&mut self, kind: Bypass) -> Result<bool, Self::Error>;
    fn hurt_armor(&mut self, damage: f32) -> Result<(), Self::Error>;
    fn armor_value(&mut self) -> Result<i32, Self::Error>;
    fn toughness_attribute(&mut self) -> Result<f64, Self::Error>;
    fn has_resistance(&mut self) -> Result<bool, Self::Error>;
    fn resistance_amplifier(&mut self) -> Result<i32, Self::Error>;
    fn victim_is_server_player(&self) -> bool;
    fn award_resisted(&mut self, amount: i32) -> Result<(), Self::Error>;
    fn attacker_is_server_player(&mut self) -> Result<bool, Self::Error>;
    /// Performs the second native attacker lookup before awarding its stat.
    fn award_dealt_resisted(&mut self, amount: i32) -> Result<(), Self::Error>;
    fn damage_protection(&mut self, level: Self::ServerLevel) -> Result<f32, Self::Error>;
}
pub fn armor_from_attribute(value: f64) -> i32 {
    value.floor() as i32
}
pub fn after_armor<W: DamageReductionWorld>(world: &mut W, damage: f32) -> Result<f32, W::Error> {
    if world.bypasses(Bypass::Armor)? {
        return Ok(damage);
    }
    world.hurt_armor(damage)?;
    let armor = world.armor_value()? as f32;
    let toughness = world.toughness_attribute()? as f32;
    combat_rules::after_armor(world, damage, armor, toughness)
}
pub fn after_magic<W: DamageReductionWorld>(
    world: &mut W,
    mut damage: f32,
) -> Result<f32, W::Error> {
    if world.bypasses(Bypass::Effects)? {
        return Ok(damage);
    }
    if world.has_resistance()? && !world.bypasses(Bypass::Resistance)? {
        let reduction = world
            .resistance_amplifier()?
            .wrapping_add(1)
            .wrapping_mul(5);
        let remainder = 25i32.wrapping_sub(reduction);
        let original = damage;
        damage = positive((damage * remainder as f32) / 25.0);
        let resisted = original - damage;
        if resisted > 0.0 && resisted < STAT_LIMIT {
            if world.victim_is_server_player() {
                world.award_resisted(stat_amount(resisted))?;
            } else if world.attacker_is_server_player()? {
                world.award_dealt_resisted(stat_amount(resisted))?;
            }
        }
    }
    if damage <= 0.0 {
        return Ok(0.0);
    }
    if world.bypasses(Bypass::Enchantments)? {
        return Ok(damage);
    }
    let protection = if let Some(level) = world.server_level()? {
        world.damage_protection(level)?
    } else {
        0.0
    };
    if protection > 0.0 {
        damage = combat_rules::after_magic(damage, protection);
    }
    Ok(damage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage_source::Catalog;
    use serde_json::json;
    use std::convert::Infallible;

    struct World {
        bypass: bool,
        armor_hurt: bool,
    }
    impl ArmorWorld for World {
        type Error = Infallible;
        type Weapon = ();
        type ServerLevel = ();
        fn weapon(&mut self) -> Result<Option<()>, Infallible> {
            Ok(None)
        }
        fn server_level(&mut self) -> Result<Option<()>, Infallible> {
            Ok(None)
        }
        fn modify_armor_effectiveness(
            &mut self,
            _: (),
            _: (),
            value: f32,
        ) -> Result<f32, Infallible> {
            Ok(value)
        }
    }
    impl DamageReductionWorld for World {
        fn bypasses(&mut self, _: Bypass) -> Result<bool, Infallible> {
            Ok(self.bypass)
        }
        fn hurt_armor(&mut self, _: f32) -> Result<(), Infallible> {
            self.armor_hurt = true;
            Ok(())
        }
        fn armor_value(&mut self) -> Result<i32, Infallible> {
            Ok(20)
        }
        fn toughness_attribute(&mut self) -> Result<f64, Infallible> {
            Ok(0.0)
        }
        fn has_resistance(&mut self) -> Result<bool, Infallible> {
            Ok(false)
        }
        fn resistance_amplifier(&mut self) -> Result<i32, Infallible> {
            Ok(0)
        }
        fn victim_is_server_player(&self) -> bool {
            false
        }
        fn award_resisted(&mut self, _: i32) -> Result<(), Infallible> {
            Ok(())
        }
        fn attacker_is_server_player(&mut self) -> Result<bool, Infallible> {
            Ok(false)
        }
        fn award_dealt_resisted(&mut self, _: i32) -> Result<(), Infallible> {
            Ok(())
        }
        fn damage_protection(&mut self, _: ()) -> Result<f32, Infallible> {
            Ok(0.0)
        }
    }

    #[test]
    fn a_mod_damage_type_in_bypasses_armor_ignores_armor() {
        let tags = serde_json::to_string(&json!({"values": ["ruby:shatter"]})).unwrap();
        let catalog = Catalog::from_json(
            &json!({
                "data_version": crate::DATA_VERSION,
                "types": [{
                    "id": "ruby:shatter", "message_id": "shatter", "exhaustion": "00000000",
                    "scaling": "never", "effects": "hurt", "death_message_type": "default"
                }]
            }),
            &json!({"minecraft:bypasses_armor": tags}),
        )
        .unwrap();
        let id = catalog.id("ruby:shatter").unwrap();
        let bypass = catalog
            .source::<()>(id, None, None, None)
            .unwrap()
            .bypasses(Bypass::Armor);
        let mut modded = World {
            bypass,
            armor_hurt: false,
        };
        assert_eq!(after_armor(&mut modded, 10.0).unwrap(), 10.0);
        assert!(!modded.armor_hurt);
        let mut ordinary = World {
            bypass: false,
            armor_hurt: false,
        };
        assert!((after_armor(&mut ordinary, 10.0).unwrap() - 4.0).abs() < 1e-4);
        assert!(ordinary.armor_hurt);
    }
}
