use std::cell::RefCell;

use super::{
    Game,
    dragon::{CRYSTAL_IDS, SPECIAL},
};

const MAX_QUERIES_PER_MOD: usize = 8;

/// A snapshot of the entity currently under the client crosshair.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeCrosshairEntity {
    pub entity_id: u32,
    pub uuid: Option<u128>,
    pub kind: String,
    pub name: String,
    pub distance: f32,
}

#[derive(Default)]
struct FrameState {
    active: bool,
    queries: Vec<(usize, usize)>,
}

impl FrameState {
    fn admit(&mut self, mod_index: usize) -> bool {
        if !self.active {
            return false;
        }
        if let Some((_, count)) = self.queries.iter_mut().find(|(index, _)| *index == mod_index) {
            if *count >= MAX_QUERIES_PER_MOD {
                return false;
            }
            *count += 1;
        } else {
            self.queries.push((mod_index, 1));
        }
        true
    }
}

thread_local! {
    static FRAME: RefCell<FrameState> = RefCell::new(FrameState::default());
}

pub(super) struct FrameScope;

impl Drop for FrameScope {
    fn drop(&mut self) {
        FRAME.with(|state| {
            let mut state = state.borrow_mut();
            state.active = false;
            state.queries.clear();
        });
    }
}

pub(super) fn enter_frame() -> FrameScope {
    FRAME.with(|state| {
        let mut state = state.borrow_mut();
        state.active = true;
        state.queries.clear();
    });
    FrameScope
}

fn offer(best: &mut Option<(f64, u32)>, distance: f64, id: u32, before_block: bool) {
    if before_block && best.is_none_or(|current| distance < current.0) {
        *best = Some((distance, id));
    }
}

fn tracked_special_id(
    entity_id: u32,
    dragon_id: Option<u32>,
    crystal_id: Option<u32>,
) -> Option<u32> {
    if entity_id >= CRYSTAL_IDS {
        crystal_id.filter(|id| *id != 0)
    } else if (SPECIAL..SPECIAL + 8).contains(&entity_id) {
        dragon_id
            .filter(|id| *id != 0)
            .and_then(|id| id.checked_add(entity_id - SPECIAL + 1))
    } else {
        Some(entity_id)
    }
}

fn is_dragon_part(entity_id: u32, dragon_id: Option<u32>) -> bool {
    dragon_id.is_some_and(|id| {
        id != 0 && entity_id.checked_sub(id).is_some_and(|offset| (1..=8).contains(&offset))
    })
}

impl Game<'_> {
    pub(super) fn native_crosshair_entity_impl(&self) -> Option<NativeCrosshairEntity> {
        let caller = horizon_client::crash::native_mod_call()?;
        if !FRAME.with(|state| state.borrow_mut().admit(caller.index)) {
            return None;
        }
        let mut best = None;
        if let Some(hit) = self.remote_object_hit(true) {
            offer(
                &mut best,
                hit.distance,
                hit.id as u32,
                self.before_block(hit.distance),
            );
        }
        if let Some((index, distance)) = self.mob_hit() {
            offer(
                &mut best,
                distance,
                self.mobs[index].id,
                self.before_block(distance),
            );
        }
        if let Some((index, distance)) = self.redirectable_hit() {
            offer(
                &mut best,
                distance,
                self.creatures.projectiles[index].id,
                self.before_block(distance),
            );
        }
        if let Some((id, distance)) = self.dragon_hit() {
            offer(&mut best, distance, id, self.before_block(distance));
        }
        let (distance, entity_id) = best?;
        let crystal_id = entity_id
            .checked_sub(CRYSTAL_IDS)
            .and_then(|index| self.crystals.get(index as usize))
            .map(|crystal| crystal.id);
        let entity_id = tracked_special_id(
            entity_id,
            self.dragon.as_ref().map(|dragon| dragon.id),
            crystal_id,
        )?;
        let dragon_part = is_dragon_part(entity_id, self.dragon.as_ref().map(|dragon| dragon.id));
        let uuid = self.remote_player_uuid(entity_id).or_else(|| {
            self.persist
                .uuids
                .get(&entity_id)
                .copied()
                .map(horizon_client::lan::playerdata::ints_uuid)
        });
        let (kind, name) = if let Some(name) = self.remote_player_name(entity_id) {
            ("minecraft:player".to_owned(), name.to_owned())
        } else if let Some(mob) = self.mobs.iter().find(|mob| mob.id == entity_id) {
            let kind = super::native_hooks::mob_kind(mob);
            let name = mob.command_name().plain();
            (kind, name)
        } else {
            let kind = if let Some(boat) = self.boats.iter().find(|boat| boat.id == entity_id) {
                boat.entity_type()
            } else if let Some(cart) = self.carts.iter().find(|cart| cart.id == entity_id) {
                cart.kind.id().to_owned()
            } else if let Some(frame) = self
                .item_frames
                .iter()
                .find(|frame| frame.id == entity_id)
            {
                frame.kind().to_owned()
            } else if let Some(projectile) = self
                .creatures
                .projectiles
                .iter()
                .find(|projectile| projectile.id == entity_id)
            {
                projectile.kind.id().to_owned()
            } else if self.armor_stands.iter().any(|stand| stand.id == entity_id) {
                "minecraft:armor_stand".to_owned()
            } else if self.cushions.iter().any(|cushion| cushion.id == entity_id) {
                "minecraft:cushion".to_owned()
            } else if self.paintings.iter().any(|painting| painting.id == entity_id) {
                "minecraft:painting".to_owned()
            } else if self.crystals.iter().any(|crystal| crystal.id == entity_id) {
                "minecraft:end_crystal".to_owned()
            } else if self.interactions.contains_key(&(entity_id as i32)) {
                "minecraft:interaction".to_owned()
            } else if dragon_part {
                "minecraft:ender_dragon".to_owned()
            } else {
                return None;
            };
            let name = self
                .armor_stands
                .iter()
                .find(|stand| stand.id == entity_id)
                .and_then(|stand| stand.custom_name.as_ref())
                .map(|name| name.plain())
                .unwrap_or_else(|| {
                    self.translate(&format!("entity.{}", kind.replace(':', ".")), &kind)
                });
            (kind, name)
        };
        Some(NativeCrosshairEntity {
            entity_id,
            uuid,
            kind,
            name,
            distance: distance as f32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_require_a_frame_and_are_bounded_per_mod() {
        let mut state = FrameState::default();
        assert!(!state.admit(1));
        assert!(state.queries.is_empty());
        state.active = true;
        for _ in 0..MAX_QUERIES_PER_MOD {
            assert!(state.admit(1));
        }
        assert!(!state.admit(1));
        assert!(state.admit(2));
    }

    #[test]
    fn crosshair_prefers_nearest_entity_before_the_block() {
        let mut best = None;
        offer(&mut best, 1.0, 7, false);
        assert_eq!(best, None);
        offer(&mut best, 3.0, 8, true);
        offer(&mut best, 1.5, 9, true);
        offer(&mut best, 1.5, 10, true);
        assert_eq!(best, Some((1.5, 9)));
    }

    #[test]
    fn synthetic_targets_require_tracked_entity_ids() {
        assert_eq!(tracked_special_id(42, None, None), Some(42));
        assert_eq!(tracked_special_id(SPECIAL + 2, Some(100), None), Some(103));
        assert_eq!(
            tracked_special_id(CRYSTAL_IDS + 1, None, Some(200)),
            Some(200)
        );
        assert_eq!(tracked_special_id(SPECIAL, None, None), None);
        assert_eq!(tracked_special_id(CRYSTAL_IDS, None, Some(0)), None);
    }

    #[test]
    fn dragon_parts_accept_local_and_remote_ids() {
        let local = tracked_special_id(SPECIAL + 2, Some(100), None).unwrap();
        assert!(is_dragon_part(local, Some(100)));
        assert!(is_dragon_part(108, Some(100)));
        assert!(!is_dragon_part(100, Some(100)));
        assert!(!is_dragon_part(109, Some(100)));
        assert!(!is_dragon_part(3, None));
    }
}
