mod pvp_attacker_tests {
    use super::*;

    struct DeathObserver {
        damage_source: Arc<Mutex<Option<(i32, i32)>>>,
        killer: Arc<Mutex<Option<u32>>>,
    }

    impl Hooks for DeathObserver {
        fn living_incoming_damage(&self, event: &mut LivingIncomingDamageEvent<'_>) -> Flow {
            *self.damage_source.lock().unwrap() = Some((event.causing, event.direct));
            Flow::Continue
        }

        fn living_death(&self, _: &mut dyn Host, event: &mut LivingDeathEvent<'_>) -> Flow {
            assert_eq!(event.kind, "minecraft:player");
            *self.killer.lock().unwrap() = Some(event.attacker);
            Flow::Continue
        }
    }

    #[test]
    fn no_mod_leaves_damage_and_death_uncanceled() {
        let mods: Vec<Box<dyn Hooks>> = Vec::new();
        let mut amount = 5.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            &mods,
            7,
            "minecraft:player",
            "minecraft:arrow",
            3,
            19,
            &mut amount,
            &mut canceled,
        );
        dispatch_living_death(
            &mut TestHost,
            &mods,
            7,
            "minecraft:player",
            "minecraft:arrow",
            3,
            &mut canceled,
        );
        assert_eq!(amount, 5.0);
        assert!(!canceled);
    }

    #[test]
    fn mod_observes_projectile_shooter_as_pvp_killer() {
        let damage_source = Arc::new(Mutex::new(None));
        let killer = Arc::new(Mutex::new(None));
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(DeathObserver {
            damage_source: damage_source.clone(),
            killer: killer.clone(),
        })];
        let mut amount = 5.0;
        let mut canceled = false;
        dispatch_living_incoming_damage(
            &mods,
            7,
            "minecraft:player",
            "minecraft:arrow",
            3,
            19,
            &mut amount,
            &mut canceled,
        );
        dispatch_living_death(
            &mut TestHost,
            &mods,
            7,
            "minecraft:player",
            "minecraft:arrow",
            3,
            &mut canceled,
        );
        assert_eq!(*damage_source.lock().unwrap(), Some((3, 19)));
        assert_eq!(*killer.lock().unwrap(), Some(3));
        assert!(!canceled);
    }
}
