mod mannequin_corpse_tests {
    use super::*;

    #[derive(Default)]
    struct RecordingHost {
        spawned: Option<u32>,
    }

    impl Host for RecordingHost {
        fn spawn_mannequin(
            &mut self,
            x: f64,
            y: f64,
            z: f64,
            yaw: f32,
            uuid: u128,
            pose: NativeMannequinPose,
        ) -> Option<u32> {
            assert_eq!(([x, y, z], yaw, uuid), ([1.0, 64.0, 3.0], 90.0, 123));
            assert_eq!(pose, NativeMannequinPose::Sleeping);
            self.spawned = Some(42);
            self.spawned
        }

        fn remove_spawned_entity(&mut self, id: u32) -> bool {
            if self.spawned == Some(id) {
                self.spawned = None;
                true
            } else {
                false
            }
        }
    }

    struct CorpseHook;

    impl Hooks for CorpseHook {
        fn living_death(&self, host: &mut dyn Host, event: &mut LivingDeathEvent<'_>) -> Flow {
            if event.kind == "minecraft:player" {
                assert_eq!(
                    host.spawn_mannequin(
                        1.0,
                        64.0,
                        3.0,
                        90.0,
                        123,
                        NativeMannequinPose::Sleeping,
                    ),
                    Some(42)
                );
            }
            Flow::Continue
        }
    }

    #[test]
    fn death_hook_can_spawn_and_later_remove_a_mannequin() {
        let mods: Vec<Box<dyn Hooks>> = vec![Box::new(CorpseHook)];
        let mut host = RecordingHost::default();
        let mut canceled = false;
        dispatch_living_death(
            &mut host,
            &mods,
            7,
            "minecraft:player",
            "minecraft:arrow",
            3,
            &mut canceled,
        );
        assert!(!canceled);
        assert_eq!(host.spawned, Some(42));
        assert!(host.remove_spawned_entity(42));
        assert!(!host.remove_spawned_entity(42));
    }
}
