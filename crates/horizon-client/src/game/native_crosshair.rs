use std::cell::RefCell;

use super::{
    Game, Hit,
    dragon::{CRYSTAL_IDS, SPECIAL},
};

const MAX_QUERIES_PER_MOD: usize = 8;
const MAX_VIEW_DISTANCE: f64 = 128.0;

/// A snapshot of the entity currently under the client crosshair.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeCrosshairEntity {
    pub entity_id: u32,
    pub uuid: Option<u128>,
    pub kind: String,
    pub name: String,
    pub distance: f32,
}

/// The block or entity intersected by a native view ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NativeViewTarget {
    Block { position: [i32; 3], face: [i32; 3] },
    Entity { entity_id: u32, uuid: u128 },
}

/// A view-ray target and its intersection in world coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeViewHit {
    pub target: NativeViewTarget,
    pub position: [f64; 3],
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

fn valid_view_distance(distance: f64) -> bool {
    distance.is_finite() && distance > 0.0 && distance <= MAX_VIEW_DISTANCE
}

fn select_view_hit(
    eye: [f64; 3],
    direction: [f64; 3],
    block: Option<Hit>,
    entity: Option<(f64, u32, Option<u128>)>,
) -> Option<NativeViewHit> {
    if let Some((distance, entity_id, uuid)) = entity
        && block.is_none_or(|hit| {
            (0..3)
                .map(|axis| (hit.point[axis] - eye[axis]).powi(2))
                .sum::<f64>()
                .sqrt()
                > distance
        })
    {
        return Some(NativeViewHit {
            target: NativeViewTarget::Entity {
                entity_id,
                uuid: uuid?,
            },
            position: std::array::from_fn(|axis| eye[axis] + direction[axis] * distance),
        });
    }
    block.map(|hit| NativeViewHit {
        target: NativeViewTarget::Block {
            position: hit.pos,
            face: hit.face,
        },
        position: hit.point,
    })
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
    /// API @API:view_raycast@. Raycast the client view up to the requested distance.
    #[allow(dead_code)]
    pub(super) fn native_client_view_hit(&self, max_distance: f64) -> Option<NativeViewHit> {
        if !valid_view_distance(max_distance) {
            return None;
        }
        let caller = horizon_client::crash::native_mod_call()?;
        if !FRAME.with(|state| state.borrow_mut().admit(caller.index)) {
            return None;
        }
        let (eye, direction) = self.aim()?;
        let block = self.clip_ray(eye, direction, max_distance);
        let entity = self
            .native_pick_entity_at(max_distance)
            .map(|(distance, id)| (distance, id, self.native_picked_uuid(id)));
        select_view_hit(eye, direction, block, entity)
    }

    fn native_pick_entity_at(&self, distance_limit: f64) -> Option<(f64, u32)> {
        let mut best = None;
        if let Some(hit) = self.remote_object_hit_at(true, distance_limit) {
            offer(&mut best, hit.distance, hit.id as u32, true);
        }
        if let Some((index, distance)) = self.mob_hit_at(distance_limit) {
            offer(&mut best, distance, self.mobs[index].id, true);
        }
        if let Some((index, distance)) = self.redirectable_hit_at(distance_limit) {
            offer(&mut best, distance, self.creatures.projectiles[index].id, true);
        }
        if let Some((entity_id, distance)) = self.dragon_hit_at(distance_limit) {
            let crystal_id = entity_id
                .checked_sub(CRYSTAL_IDS)
                .and_then(|index| self.crystals.get(index as usize))
                .map(|crystal| crystal.id);
            if let Some(entity_id) = tracked_special_id(
                entity_id,
                self.dragon.as_ref().map(|dragon| dragon.id),
                crystal_id,
            ) {
                offer(&mut best, distance, entity_id, true);
            }
        }
        best
    }

    fn native_picked_uuid(&self, entity_id: u32) -> Option<u128> {
        let identity_id = self
            .dragon
            .as_ref()
            .map(|dragon| dragon.id)
            .filter(|id| is_dragon_part(entity_id, Some(*id)))
            .unwrap_or(entity_id);
        self.remote_player_uuid(identity_id).or_else(|| {
            self.persist
                .uuids
                .get(&identity_id)
                .copied()
                .map(horizon_client::lan::playerdata::ints_uuid)
        })
    }

    pub(super) fn native_crosshair_entity_impl(&self) -> Option<NativeCrosshairEntity> {
        let caller = horizon_client::crash::native_mod_call()?;
        if !FRAME.with(|state| state.borrow_mut().admit(caller.index)) {
            return None;
        }
        let (distance, entity_id) = self.native_pick_entity_at(self.entity_reach)?;
        if !self.before_block(distance) {
            return None;
        }
        let dragon_part = is_dragon_part(entity_id, self.dragon.as_ref().map(|dragon| dragon.id));
        let uuid = self.native_picked_uuid(entity_id);
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
    fn view_ray_limits_distance_and_prefers_the_first_intersection() {
        assert!(valid_view_distance(128.0));
        assert!(!valid_view_distance(0.0));
        assert!(!valid_view_distance(128.1));
        assert!(!valid_view_distance(f64::NAN));
        assert!(!valid_view_distance(f64::INFINITY));
        let eye = [1.0, 2.0, 3.0];
        let direction = [1.0, 0.0, 0.0];
        let block = Hit {
            pos: [5, 2, 3],
            face: [-1, 0, 0],
            point: [5.0, 2.0, 3.0],
        };
        assert_eq!(
            select_view_hit(eye, direction, Some(block), Some((2.0, 7, Some(123)))),
            Some(NativeViewHit {
                target: NativeViewTarget::Entity {
                    entity_id: 7,
                    uuid: 123
                },
                position: [3.0, 2.0, 3.0],
            })
        );
        let block_hit = Some(NativeViewHit {
            target: NativeViewTarget::Block {
                position: [5, 2, 3],
                face: [-1, 0, 0],
            },
            position: [5.0, 2.0, 3.0],
        });
        assert_eq!(
            select_view_hit(eye, direction, Some(block), Some((4.0, 7, Some(123)))),
            block_hit
        );
        assert_eq!(
            select_view_hit(eye, direction, Some(block), Some((8.0, 7, None))),
            block_hit
        );
        assert_eq!(
            select_view_hit(eye, direction, Some(block), Some((2.0, 7, None))),
            None
        );
        assert_eq!(select_view_hit(eye, direction, None, None), None);
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
