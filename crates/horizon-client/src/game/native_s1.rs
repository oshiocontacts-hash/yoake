//! Small client and server helpers for the services of sandboxed WebAssembly mods that the
//! native API had no name for (fragment `fr_s1_services`): client-only UI sounds, the local
//! player's items, and a server-side food setter. Nothing here does anything unless a mod calls
//! it, so a game without such a mod behaves as before.
use std::cell::RefCell;

use horizon_client::{hud::HudBatch, items::ItemStack};

use super::Game;

/// Most UI sounds that wait for the next frame, from all mods together.
const MAX_PENDING_SOUNDS: usize = 64;
/// Longest sound event id.
const MAX_SOUND_ID: usize = 128;

/// One client-only sound that waits for the frame to start.
struct PendingSound {
    sound: String,
    at: Option<[f64; 3]>,
    volume: f32,
    pitch: f32,
}

/// Sounds queued by hooks that hold only a shared view of the game.
#[derive(Default)]
pub(super) struct NativeS1 {
    sounds: RefCell<Vec<PendingSound>>,
}

/// A sound event id as `namespace:path` of lower case letters, digits and `_.-` (and `/` in the
/// path), the same shape the other local sound helpers take.
fn sound_event_ok(id: &str) -> bool {
    if id.len() > MAX_SOUND_ID {
        return false;
    }
    let Some((namespace, path)) = id.split_once(':') else {
        return false;
    };
    let allowed = |c: char, slash: bool| {
        c.is_ascii_lowercase() || c.is_ascii_digit() || "_.-".contains(c) || (slash && c == '/')
    };
    !namespace.is_empty()
        && !path.is_empty()
        && namespace.chars().all(|c| allowed(c, false))
        && path.chars().all(|c| allowed(c, true))
}

fn sound_values_ok(volume: f32, pitch: f32, at: Option<[f64; 3]>) -> bool {
    volume.is_finite()
        && (0.0..=16.0).contains(&volume)
        && pitch.is_finite()
        && (0.0..=8.0).contains(&pitch)
        && at.is_none_or(|at| at.iter().all(|v| v.is_finite() && v.abs() <= 30_000_000.0))
}

fn counts(stack: &ItemStack) -> bool {
    stack.count != 0 && &*stack.item != "minecraft:air"
}

impl Game<'_> {
    /// Queues a sound for this client only: `at` of `None` is heard as a UI sound (relative to the
    /// listener, with no attenuation), a position is a place in the world. It plays when the next
    /// frame starts, so a hook with a shared view of the game (a screen key or mouse hook) may
    /// call it. False, with nothing queued, off the window's own client, for an id that is not
    /// `namespace:path`, a value out of range (volume 0 to 16, pitch 0 to 8) or when 64 sounds
    /// already wait. A sound event the resources do not have plays nothing, as vanilla's.
    pub(super) fn native_queue_local_sound(
        &self,
        sound: &str,
        at: Option<[f64; 3]>,
        volume: f32,
        pitch: f32,
    ) -> bool {
        if !self.native_window() || !sound_event_ok(sound) || !sound_values_ok(volume, pitch, at) {
            return false;
        }
        let mut sounds = self.native_s1.sounds.borrow_mut();
        if sounds.len() >= MAX_PENDING_SOUNDS {
            return false;
        }
        sounds.push(PendingSound {
            sound: sound.to_owned(),
            at,
            volume,
            pitch,
        });
        true
    }

    /// A quad of the GUI sprite `name` (a vanilla `gui/...` sprite or a content mod's own,
    /// `mymod:gui/...`): `corners` are four GUI-pixel points in order round the shape and `uv` the
    /// sprite's own corners for them, 0 to 1 over the whole sprite. `argb` tints it with straight
    /// alpha. A quad that is turned, sheared or cut out of the sprite is what `native_sprite`'s
    /// rectangles cannot draw. False, with nothing drawn, for an unknown sprite or a value that
    /// is not finite.
    pub(super) fn native_sprite_quad(
        &self,
        batch: &mut HudBatch,
        name: &str,
        corners: [[f32; 2]; 4],
        uv: [[f32; 2]; 4],
        argb: u32,
    ) -> bool {
        if !corners
            .iter()
            .chain(uv.iter())
            .flatten()
            .all(|v| v.is_finite())
        {
            return false;
        }
        let Some(rect) = self.gui.atlas.sprites.get(name).copied() else {
            return false;
        };
        let at = |p: [f32; 2]| {
            [
                rect.u0 + p[0].clamp(0.0, 1.0) * (rect.u1 - rect.u0),
                rect.v0 + p[1].clamp(0.0, 1.0) * (rect.v1 - rect.v0),
            ]
        };
        let color = (argb & 0xff00_ff00) | ((argb & 0xff) << 16) | ((argb >> 16) & 0xff);
        batch.quad(corners, [at(uv[0]), at(uv[1]), at(uv[2]), at(uv[3])], color);
        true
    }

    /// Plays what `native_queue_local_sound` queued, at the start of a frame.
    pub(super) fn flush_native_local_sounds(&mut self) {
        let pending = std::mem::take(&mut *self.native_s1.sounds.borrow_mut());
        for sound in pending {
            self.audio
                .play_unshared(&sound.sound, sound.at, sound.volume, sound.pitch);
        }
    }

    /// The stacks the local player carries as this client sees them: the main inventory
    /// (hotbar first), then the armor and the off hand, without the empty slots and without the
    /// stack on the cursor. Empty off the window's own client.
    pub(super) fn native_local_player_stacks(&self) -> Vec<ItemStack> {
        if !self.native_window() {
            return Vec::new();
        }
        self.inventory
            .iter()
            .flatten()
            .chain(self.armor.iter().flatten())
            .chain(self.offhand.iter())
            .filter(|stack| counts(stack))
            .cloned()
            .collect()
    }

    /// Sets a player's food level (0 to 20) and saturation (0 up to the new food level), on the
    /// authoritative server tick. The food-level hook hears the change as it hears any other.
    /// Ok(()) when it is set; an error for a value out of range or a player who is not here.
    pub(super) fn native_player_set_food(
        &mut self,
        uuid: u128,
        food: i32,
        saturation: f32,
    ) -> Result<(), String> {
        self.wasm_service("player_set_food")?;
        if !(0..=20).contains(&food) || !saturation.is_finite() || saturation < 0.0 {
            return Err("food is 0 to 20 and saturation is not negative".into());
        }
        let (target, _, dimension) = self.native_player_target(uuid)?;
        self.for_native_player(target, dimension, |game| {
            let player = game.player.as_mut().ok_or("the player is not here")?;
            player.health.food.food = food;
            player.health.food.saturation = saturation.min(food as f32);
            Ok(())
        })?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sound_ids_are_namespaced_lowercase_paths() {
        assert!(sound_event_ok("minecraft:ui.button.click"));
        assert!(sound_event_ok("cooking:ui/sizzle_2"));
        assert!(!sound_event_ok("click"));
        assert!(!sound_event_ok("Minecraft:click"));
        assert!(!sound_event_ok("a:"));
        assert!(!sound_event_ok(":b"));
        assert!(!sound_event_ok(&format!("a:{}", "b".repeat(MAX_SOUND_ID))));
    }

    #[test]
    fn sound_values_must_be_finite_and_in_range() {
        assert!(sound_values_ok(1.0, 1.0, None));
        assert!(sound_values_ok(0.0, 0.5, Some([1.0, 2.0, 3.0])));
        assert!(!sound_values_ok(f32::NAN, 1.0, None));
        assert!(!sound_values_ok(1.0, f32::INFINITY, None));
        assert!(!sound_values_ok(-0.1, 1.0, None));
        assert!(!sound_values_ok(17.0, 1.0, None));
        assert!(!sound_values_ok(1.0, 9.0, None));
        assert!(!sound_values_ok(1.0, 1.0, Some([f64::NAN, 0.0, 0.0])));
        assert!(!sound_values_ok(1.0, 1.0, Some([4.0e7, 0.0, 0.0])));
    }
}
