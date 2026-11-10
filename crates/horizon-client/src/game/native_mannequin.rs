use std::collections::HashMap;

use horizon_client::mobs::MannequinDescription;
use horizon_client::pose::Pose;
use horizon_protocol::entity_data::Profile;

use super::{Game, native_entity_query::require_server_context, native_mobs::NativeSpawn};
use crate::mod_api::{NativeTickId, NativeWorldEpoch};

const MAX_CALLS: u16 = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMannequinPose {
    Standing,
    Sleeping,
}

impl From<NativeMannequinPose> for Pose {
    fn from(pose: NativeMannequinPose) -> Self {
        match pose {
            NativeMannequinPose::Standing => Self::Standing,
            NativeMannequinPose::Sleeping => Self::Sleeping,
        }
    }
}

#[derive(Clone, Copy)]
enum Operation {
    Spawn,
    Remove,
}

#[derive(Default)]
pub(super) struct NativeMannequins {
    tick: Option<(NativeWorldEpoch, NativeTickId)>,
    usage: HashMap<usize, [u16; 2]>,
    owned: HashMap<u32, (usize, String)>,
}

impl NativeMannequins {
    fn reserve(
        &mut self,
        tick: (NativeWorldEpoch, NativeTickId),
        caller: usize,
        operation: Operation,
    ) -> bool {
        if self.tick != Some(tick) {
            if self.tick.is_some_and(|previous| previous.0 != tick.0) {
                self.owned.clear();
            }
            self.tick = Some(tick);
            self.usage.clear();
        }
        let calls = &mut self.usage.entry(caller).or_default()[operation as usize];
        if *calls >= MAX_CALLS {
            return false;
        }
        *calls += 1;
        true
    }

    pub(super) fn contains(&self, id: u32) -> bool {
        self.owned.contains_key(&id)
    }

    fn clear(&mut self) {
        *self = Self::default();
    }

    fn owned_by(&self, id: u32, caller: usize, dimension: &str) -> bool {
        self.owned
            .get(&id)
            .is_some_and(|(owner, level)| *owner == caller && level == dimension)
    }

    fn release(&mut self, id: u32, caller: usize, dimension: &str) -> bool {
        if !self.owned_by(id, caller, dimension) {
            return false;
        }
        self.owned.remove(&id);
        true
    }
}

impl Game<'_> {
    fn native_mannequin_reserve(&mut self, operation: Operation) -> Option<usize> {
        require_server_context(
            self.options.execution_role,
            self.remote.is_some(),
            self.lan_acting.is_some(),
            self.lan.is_some(),
        )
        .ok()?;
        let caller = horizon_client::crash::native_mod_call()?.index;
        let tick = self.native_lifecycle.entity_query_tick()?;
        self.native_mannequins
            .reserve(tick, caller, operation)
            .then_some(caller)
    }

    pub(super) fn native_spawn_mannequin_impl(
        &mut self,
        x: f64,
        y: f64,
        z: f64,
        yaw: f32,
        profile: u128,
        pose: NativeMannequinPose,
    ) -> Option<u32> {
        let caller = self.native_mannequin_reserve(Operation::Spawn)?;
        if !yaw.is_finite() {
            return None;
        }
        let online_profile = self.lan.as_ref().and_then(|host| {
            host.hub
                .guests
                .iter()
                .find(|guest| guest.uuid == profile)
                .map(|guest| {
                    let mut properties = guest.properties.clone();
                    if host.hub.singleplayer_owner == Some(profile)
                        && !properties.iter().any(|property| property.name == "textures")
                        && let Some(owner) = host.hub.lan.shared.config.owner_profile.as_deref()
                        && let Some(textures) = owner
                            .properties()
                            .iter()
                            .find(|property| property.name == "textures")
                    {
                        properties.push(textures.clone());
                    }
                    (guest.name.clone(), properties)
                })
        });
        let dimension = self.dim.id().to_owned();
        let options = NativeSpawn {
            no_ai: true,
            invulnerable: true,
            no_gravity: true,
            ..Default::default()
        };
        let id = self.native_spawn_mob(&dimension, "minecraft:mannequin", [x, y, z], &options)?;
        let Some(mob) = self.mobs.iter_mut().find(|mob| mob.id == id && !mob.removed) else {
            return None;
        };
        mob.profile = Some(Profile {
            uuid: Some(profile),
            name: online_profile.as_ref().and_then(|(name, properties)| {
                properties
                    .iter()
                    .any(|property| property.name == "textures")
                    .then(|| name.clone())
            }),
            properties: online_profile
                .map(|(_, properties)| properties)
                .unwrap_or_default(),
            ..Default::default()
        });
        mob.extra.avatar_pose = pose.into();
        // CHECK: Confirm forced movement behavior against vanilla.
        mob.extra.immovable = true;
        mob.extra.description = MannequinDescription::Hidden;
        mob.native_damage_immune = true;
        mob.yaw = yaw;
        mob.previous_yaw = yaw;
        mob.body_yaw = yaw;
        mob.previous_body_yaw = yaw;
        mob.head_yaw = yaw;
        mob.previous_head_yaw = yaw;
        mob.needs_sync = true;
        self.native_mannequins.owned.insert(id, (caller, dimension));
        Some(id)
    }

    pub(super) fn native_remove_spawned_entity_impl(&mut self, id: u32) -> bool {
        let Some(caller) = self.native_mannequin_reserve(Operation::Remove) else {
            return false;
        };
        let Some((owner, dimension)) = self.native_mannequins.owned.get(&id).cloned() else {
            return false;
        };
        if owner != caller {
            return false;
        }
        let removed = if dimension == self.dim.id() {
            self.native_remove_mob(&dimension, id)
        } else {
            let dim = super::Dim::from_id(&dimension);
            self.levels
                .parked_mut(dim)
                .and_then(|level| level.mobs.iter_mut().find(|mob| mob.id == id && !mob.removed))
                .is_some_and(|mob| {
                    mob.native_control_drop();
                    mob.removed = true;
                    true
                })
        };
        removed && self.native_mannequins.release(id, caller, &dimension)
    }

    pub(super) fn native_clear_mannequins(&mut self) {
        for mob in &mut self.mobs {
            if self.native_mannequins.contains(mob.id) {
                mob.removed = true;
            }
        }
        for dim in self.levels.parked_dims() {
            if let Some(level) = self.levels.parked_mut(dim) {
                for mob in &mut level.mobs {
                    if self.native_mannequins.contains(mob.id) {
                        mob.removed = true;
                    }
                }
            }
        }
        self.native_mannequins.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_are_per_mod_operation_and_tick() {
        let mut state = NativeMannequins::default();
        let tick = (NativeWorldEpoch::new(1), NativeTickId::new(1));
        for _ in 0..MAX_CALLS {
            assert!(state.reserve(tick, 1, Operation::Spawn));
        }
        assert!(!state.reserve(tick, 1, Operation::Spawn));
        assert!(state.reserve(tick, 2, Operation::Spawn));
        assert!(state.reserve(tick, 1, Operation::Remove));
        assert!(state.reserve(
            (tick.0, NativeTickId::new(2)),
            1,
            Operation::Spawn
        ));
    }

    #[test]
    fn ownership_is_per_mod_and_level_and_expires_with_session() {
        let mut state = NativeMannequins::default();
        let tick = (NativeWorldEpoch::new(1), NativeTickId::new(1));
        assert!(state.reserve(tick, 7, Operation::Spawn));
        state.owned.insert(42, (7, "minecraft:overworld".into()));
        assert!(state.owned_by(42, 7, "minecraft:overworld"));
        assert!(!state.owned_by(42, 8, "minecraft:overworld"));
        assert!(!state.owned_by(42, 7, "minecraft:the_nether"));
        assert!(!state.release(42, 8, "minecraft:overworld"));
        assert!(!state.release(42, 7, "minecraft:the_nether"));
        assert!(state.release(42, 7, "minecraft:overworld"));
        assert!(!state.release(42, 7, "minecraft:overworld"));
        assert!(!state.owned_by(42, 7, "minecraft:overworld"));
        state.owned.insert(43, (7, "minecraft:overworld".into()));
        assert!(state.reserve(
            (NativeWorldEpoch::new(2), tick.1),
            7,
            Operation::Remove
        ));
        assert!(!state.contains(43));
    }

    #[test]
    fn world_end_clears_ownership_and_budget() {
        let mut state = NativeMannequins::default();
        let tick = (NativeWorldEpoch::new(1), NativeTickId::new(1));
        assert!(state.reserve(tick, 7, Operation::Spawn));
        state.owned.insert(42, (7, "minecraft:overworld".into()));
        state.clear();
        assert!(state.tick.is_none());
        assert!(state.usage.is_empty());
        assert!(!state.contains(42));
    }
}
