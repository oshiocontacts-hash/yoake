//! A native mod's tint for the sky and the clouds (API @API:sky_tint@): the client draws the
//! next sky dome and cloud layer a `strength` of the way towards `color`, once. A mod that wants
//! the tint on every frame asks on every frame; a frame it does not ask on is drawn untinted.
//! Terrain, fog, water and everything else keep their colours.
use horizon_client::environment::Sky;

use super::Game;

/// A request: the colour and how far towards it, 0 (none) to 1 (all of it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tint {
    color: [f32; 3],
    strength: f32,
}

impl Tint {
    /// A request, clamped to 0 to 1; `None` for a value that is not a number.
    fn new(color: [f32; 3], strength: f32) -> Option<Self> {
        (strength.is_finite() && color.iter().all(|c| c.is_finite())).then(|| Self {
            color: color.map(|c| c.clamp(0.0, 1.0)),
            strength: strength.clamp(0.0, 1.0),
        })
    }
}

fn mix(from: f32, to: f32, strength: f32) -> f32 {
    from + (to - from) * strength
}

/// The sky dome's and the cloud layer's colours moved towards the tint. Nothing else of `sky`
/// changes: its fog colour, lighting and sunrise stay vanilla's, and the clouds keep their alpha.
pub(super) fn apply(sky: &mut Sky, tint: Tint) {
    for i in 0..3 {
        sky.sky_color[i] = mix(sky.sky_color[i], tint.color[i], tint.strength);
        sky.cloud_color[i] = mix(sky.cloud_color[i], tint.color[i], tint.strength);
    }
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Tints the sky dome and the clouds of the frame being drawn: `strength` (0 to 1, clamped) of
    /// the way from their colour to `color` (RGB, 0 to 1, clamped). Call it again each frame the
    /// tint should show; a frame it is not called on is drawn as vanilla draws it (a call made
    /// after a frame's sky was built shows on the next frame, once). Fog, terrain, water and the
    /// shader packs' other stages are not tinted. Client only; an error for values that are not
    /// numbers and on a server. API @API:sky_tint@.
    pub(super) fn native_set_sky_tint(
        &mut self,
        color: [f32; 3],
        strength: f32,
    ) -> Result<(), String> {
        if !self.options.execution_role.is_client() {
            return Err("a sky tint is drawn by a client".into());
        }
        let tint = Tint::new(color, strength).ok_or("a sky tint is made of numbers")?;
        self.native_sky_tint = Some(tint);
        Ok(())
    }

    /// The sky of the frame being built, tinted when a mod asked since the last one.
    pub(super) fn native_tint_sky(&mut self, sky: &mut Sky) {
        if let Some(tint) = self.native_sky_tint.take() {
            apply(sky, tint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sky() -> Sky {
        Sky {
            sky_color: [0.2, 0.4, 0.8],
            fog_color: [0.7, 0.8, 1.0],
            cloud_color: [1.0, 1.0, 1.0, 0.8],
            ..Sky::default()
        }
    }

    #[test]
    fn the_tint_moves_only_the_sky_dome_and_the_clouds() {
        let mut tinted = sky();
        apply(&mut tinted, Tint::new([1.0, 0.0, 0.0], 0.5).unwrap());
        assert_eq!(tinted.sky_color, [0.6, 0.2, 0.4]);
        assert_eq!(tinted.cloud_color, [1.0, 0.5, 0.5, 0.8]);
        let plain = sky();
        assert_eq!(tinted.fog_color, plain.fog_color);
        assert_eq!(tinted.sky_light_color, plain.sky_light_color);
        assert_eq!(tinted.sunrise_color, plain.sunrise_color);
    }

    #[test]
    fn strength_zero_changes_nothing_and_one_is_the_colour() {
        let mut none = sky();
        apply(&mut none, Tint::new([1.0, 0.0, 0.0], 0.0).unwrap());
        assert_eq!(none.sky_color, sky().sky_color);
        let mut all = sky();
        apply(&mut all, Tint::new([1.0, 0.0, 0.0], 1.0).unwrap());
        assert_eq!(all.sky_color, [1.0, 0.0, 0.0]);
        assert_eq!(all.cloud_color, [1.0, 0.0, 0.0, 0.8]);
    }

    #[test]
    fn values_are_clamped_and_not_a_number_is_refused() {
        let t = Tint::new([2.0, -1.0, 0.5], 7.0).unwrap();
        assert_eq!((t.color, t.strength), ([1.0, 0.0, 0.5], 1.0));
        assert!(Tint::new([f32::NAN, 0.0, 0.0], 0.5).is_none());
        assert!(Tint::new([0.0; 3], f32::INFINITY).is_none());
    }
}
