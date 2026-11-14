//! A player's attack hit box for native mods (API @API:player_attack_hitbox@): the server
//! half sets or clears it, the Horizon client half picks with it (`lan::attack_hitbox`).
use horizon_client::{lan::attack_hitbox, mobs::Mob};
use horizon_core::geometry::Aabb;

use super::{Game, native_server_physics::Operation};

/// Boxes a client keeps at most, one per other player that has one.
const MAX_CLIENT_BOXES: usize = 1024;

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Gives the player `uuid` an attack hit box: `width` wide and `height` tall from their feet
    /// (`Entity.makeBoundingBox`: centred on them), separate from the box the world collides
    /// with. It is the box other players' attacks and interactions are reach-checked against on
    /// the server, and the box a Horizon client's crosshair picks them by; a vanilla client
    /// keeps the ordinary box. Sizes are positive and at most 1024. Ok(false) when that player
    /// is not on this server. Server tick hooks only (512 calls per mod and tick). The box lasts
    /// until it is cleared or the player leaves. API @API:player_attack_hitbox@.
    pub(super) fn native_set_player_attack_hitbox(
        &mut self,
        uuid: u128,
        width: f32,
        height: f32,
    ) -> Result<bool, String> {
        self.native_physics_reserve(Operation::PlayerHitbox, true)?;
        if !attack_hitbox::valid(width, height) {
            return Err(format!(
                "an attack hit box is positive, finite and at most {} blocks a side",
                attack_hitbox::MAX_SIDE
            ));
        }
        self.native_attack_hitbox_apply(uuid, Some((width, height)))
    }

    /// Puts the player's ordinary box back. Ok(false) when that player is not on this server.
    /// API @API:player_attack_hitbox@.
    pub(super) fn native_clear_player_attack_hitbox(&mut self, uuid: u128) -> Result<bool, String> {
        self.native_physics_reserve(Operation::PlayerHitbox, true)?;
        self.native_attack_hitbox_apply(uuid, None)
    }
}

impl Game<'_> {
    fn native_attack_hitbox_apply(
        &mut self,
        uuid: u128,
        hitbox: Option<(f32, f32)>,
    ) -> Result<bool, String> {
        let host = self
            .lan
            .as_mut()
            .ok_or("native physics requires the server host")?;
        Ok(host.hub.set_attack_hitbox(uuid, hitbox))
    }

    /// A `horizon:attack_hitbox` payload the server sent the window.
    pub(super) fn remote_attack_hitbox(&mut self, data: &[u8]) {
        let Some((entity, hitbox)) = attack_hitbox::decode(data) else {
            log::debug!("Horizon: a bad attack hit box message");
            return;
        };
        set_client_box(&mut self.attack_hitboxes, entity, hitbox);
    }

    /// The box the crosshair picks `mob` by: a player's attack hit box when the server gave
    /// them one, else the box they stand in.
    pub(super) fn pick_bounds(&self, mob: &Mob) -> Aabb {
        pick_bounds(
            &self.attack_hitboxes,
            mob.id as i32,
            mob.position.components(),
            || mob.bounds(),
        )
    }
}

fn set_client_box(
    boxes: &mut std::collections::HashMap<i32, (f32, f32)>,
    entity: i32,
    hitbox: Option<(f32, f32)>,
) {
    match hitbox {
        Some(size) if boxes.len() < MAX_CLIENT_BOXES || boxes.contains_key(&entity) => {
            boxes.insert(entity, size);
        }
        Some(_) => {}
        None => {
            boxes.remove(&entity);
        }
    }
}

fn pick_bounds(
    boxes: &std::collections::HashMap<i32, (f32, f32)>,
    entity: i32,
    feet: [f64; 3],
    standing: impl FnOnce() -> Aabb,
) -> Aabb {
    match boxes.get(&entity) {
        Some(&(width, height)) => attack_hitbox::bounds(feet, width, height),
        None => standing(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::geometry::Vec3;
    use std::collections::HashMap;

    fn standing() -> Aabb {
        Aabb::new(Vec3::new(-0.3, 0.0, -0.3), Vec3::new(0.3, 1.8, 0.3))
    }

    #[test]
    fn a_player_without_a_box_is_picked_by_the_one_they_stand_in() {
        let boxes = HashMap::new();
        assert_eq!(pick_bounds(&boxes, 7, [0.0; 3], standing), standing());
    }

    #[test]
    fn a_players_box_replaces_it_for_that_player_only() {
        let mut boxes = HashMap::new();
        set_client_box(&mut boxes, 7, Some((6.0, 18.0)));
        let b = pick_bounds(&boxes, 7, [10.0, 64.0, 10.0], standing);
        assert_eq!(b.max_position(), Vec3::new(13.0, 82.0, 13.0));
        assert_eq!(pick_bounds(&boxes, 8, [0.0; 3], standing), standing());
        set_client_box(&mut boxes, 7, None);
        assert_eq!(pick_bounds(&boxes, 7, [0.0; 3], standing), standing());
    }

    #[test]
    fn a_client_keeps_a_bounded_number_of_boxes() {
        let mut boxes = HashMap::new();
        for id in 0..(MAX_CLIENT_BOXES as i32 + 10) {
            set_client_box(&mut boxes, id, Some((1.0, 1.0)));
        }
        assert_eq!(boxes.len(), MAX_CLIENT_BOXES);
        // A known player's box can still change when the table is full.
        set_client_box(&mut boxes, 3, Some((2.0, 2.0)));
        assert_eq!(boxes[&3], (2.0, 2.0));
    }
}
