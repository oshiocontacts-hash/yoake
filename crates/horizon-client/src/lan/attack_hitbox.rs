//! A player's attack hit box: what native mods set with `native_set_player_attack_hitbox`
//! (API @API:player_attack_hitbox@). A box that starts at the player's feet and is
//! `width` wide and `height` tall, separate from the box the world collides with. The
//! server's attack and interaction reach check uses it for that player as a target, and
//! so does a Horizon client's crosshair for the same player, which is why the server tells
//! every Horizon client on this channel (a vanilla client is sent nothing and keeps the
//! player's ordinary box). Nothing sets one unless a mod does.
use horizon_core::geometry::{Aabb, Vec3};
use horizon_protocol::packets::{configuration::CustomPayload, play};

use super::{guest::Guest, hub::Hub};

/// The custom payload channel, clientbound only: a client sends nothing back.
pub const CHANNEL: &str = "horizon:attack_hitbox";
/// The largest side of a box, in blocks.
pub const MAX_SIDE: f32 = 1024.0;

/// A box's width and height, in blocks: positive, finite and at most [`MAX_SIDE`].
pub fn valid(width: f32, height: f32) -> bool {
    [width, height]
        .into_iter()
        .all(|side| side.is_finite() && side > 0.0 && side <= MAX_SIDE)
}

/// The box a hit box of `width` by `height` makes around feet at `feet`
/// (`Entity.makeBoundingBox`).
pub fn bounds(feet: [f64; 3], width: f32, height: f32) -> Aabb {
    let half = f64::from(width) / 2.0;
    Aabb::new(
        Vec3::new(feet[0] - half, feet[1], feet[2] - half),
        Vec3::new(feet[0] + half, feet[1] + f64::from(height), feet[2] + half),
    )
}

/// A payload: the player's entity id, then the box's width and height as `f32`s. A width
/// or height of 0 or less clears the entry.
pub fn encode(entity: i32, hitbox: Option<(f32, f32)>) -> Vec<u8> {
    let (width, height) = hitbox.unwrap_or((0.0, 0.0));
    let mut out = Vec::with_capacity(12);
    out.extend(entity.to_be_bytes());
    out.extend(width.to_be_bytes());
    out.extend(height.to_be_bytes());
    out
}

/// A payload's entity id and box, `None` for a box it clears; an error-free `None` for bytes
/// that are not a message.
pub fn decode(data: &[u8]) -> Option<(i32, Option<(f32, f32)>)> {
    let data: [u8; 12] = data.try_into().ok()?;
    let entity = i32::from_be_bytes(data[0..4].try_into().ok()?);
    let width = f32::from_be_bytes(data[4..8].try_into().ok()?);
    let height = f32::from_be_bytes(data[8..12].try_into().ok()?);
    Some((entity, valid(width, height).then_some((width, height))))
}

fn packet(entity: i32, hitbox: Option<(f32, f32)>) -> play::Clientbound {
    play::Clientbound::CustomPayload(CustomPayload {
        channel: CHANNEL.into(),
        data: encode(entity, hitbox),
    })
}

impl Guest {
    /// The box this guest is hit in: the mod's attack hit box when one is set, else the box
    /// they stand in (`Entity.getBoundingBox`).
    pub fn attack_bounds(&self) -> Aabb {
        match self.attack_hitbox {
            Some((width, height)) => bounds(self.position, width, height),
            None => self.bounds(),
        }
    }
    /// Whether the guest is placed and on a Horizon client, the only one that reads attack hit
    /// boxes.
    pub fn reads_attack_hitbox(&self) -> bool {
        self.placed && self.brand == "horizon"
    }
}

impl Hub {
    /// Sets or clears (`None`) the attack hit box of the player `uuid` and tells the Horizon
    /// clients; false when no such player is here.
    pub fn set_attack_hitbox(&mut self, uuid: u128, hitbox: Option<(f32, f32)>) -> bool {
        let Some(guest) = self.guests.iter_mut().find(|g| g.uuid == uuid) else {
            return false;
        };
        let entity = guest.entity_id;
        let changed = guest.attack_hitbox != hitbox;
        guest.attack_hitbox = hitbox;
        if changed && guest.placed {
            let packet = packet(entity, hitbox);
            for reader in self.guests.iter().filter(|g| g.reads_attack_hitbox()) {
                reader.send(&packet);
            }
        }
        true
    }
    /// What a Horizon client that was just placed needs to know: every box set so far.
    pub fn send_attack_hitboxes_to(&self, conn: u32) {
        let Some(reader) = self.guests.iter().find(|g| g.conn == conn) else {
            return;
        };
        if !reader.reads_attack_hitbox() {
            return;
        }
        for guest in self.guests.iter().filter(|g| g.attack_hitbox.is_some()) {
            reader.send(&packet(guest.entity_id, guest.attack_hitbox));
        }
    }
    /// A player is gone: the clients drop their box for it.
    pub fn forget_attack_hitbox(&self, guest: &Guest) {
        if guest.attack_hitbox.is_none() {
            return;
        }
        let packet = packet(guest.entity_id, None);
        for reader in self
            .guests
            .iter()
            .filter(|g| g.conn != guest.conn && g.reads_attack_hitbox())
        {
            reader.send(&packet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_starts_at_the_feet_and_is_centred_on_them() {
        let b = bounds([10.0, 64.0, -3.0], 6.0, 18.0);
        assert_eq!(b.min_position(), Vec3::new(7.0, 64.0, -6.0));
        assert_eq!(b.max_position(), Vec3::new(13.0, 82.0, 0.0));
    }

    #[test]
    fn the_payload_round_trips_and_a_bad_one_is_nothing() {
        assert_eq!(
            decode(&encode(41, Some((6.0, 18.0)))),
            Some((41, Some((6.0, 18.0))))
        );
        assert_eq!(decode(&encode(-7, None)), Some((-7, None)));
        assert_eq!(decode(&[0; 11]), None);
        assert_eq!(decode(&[0; 13]), None);
        // A box that is not valid clears, whatever the sender meant.
        assert_eq!(decode(&encode(1, Some((f32::NAN, 2.0)))), Some((1, None)));
        assert_eq!(decode(&encode(1, Some((2.0, -1.0)))), Some((1, None)));
    }

    #[test]
    fn sizes_are_positive_finite_and_bounded() {
        assert!(valid(0.6, 1.8));
        assert!(valid(MAX_SIDE, MAX_SIDE));
        for (w, h) in [
            (0.0, 1.0),
            (1.0, 0.0),
            (-1.0, 1.0),
            (f32::INFINITY, 1.0),
            (1.0, f32::NAN),
            (MAX_SIDE + 1.0, 1.0),
        ] {
            assert!(!valid(w, h), "{w} x {h}");
        }
    }
}
