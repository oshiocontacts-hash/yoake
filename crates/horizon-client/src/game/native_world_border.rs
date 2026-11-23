//! Reading and writing the world border for native mods (API @API:world_border_state@),
//! with vanilla's semantics: `WorldBorder.setCenter`, `setSize` and `lerpSizeBetween`, and the
//! packets the border's own change listener sends (`lan::border`: a new centre, a new size,
//! a size change in progress), which reach the players in that border's level at the end of
//! the server tick as the `/worldborder` command's do.
use horizon_client::{
    mod_api::NativeWorldBorderState, worldborder::MAX_SIZE, worldborder::WorldBorder,
};

use super::{Game, native_server_physics::Operation};

/// `WorldBorderCommand.CENTER`'s limit: a centre is at most this far from the origin.
const MAX_CENTER: f64 = 2.999_998_4e7;

fn dimension_ok(dimension: &str) -> Result<(), String> {
    horizon_client::level_registry::check_level_id(dimension)
        .map_err(|_| "unknown native world border dimension".to_owned())
}

/// The border as a mod reads it.
pub(crate) fn state(border: &WorldBorder) -> NativeWorldBorderState {
    NativeWorldBorderState {
        center: border.center(),
        size: border.size(),
        target_size: border.target_size(),
        remaining_ticks: border.remaining_ticks().max(0) as u64,
    }
}

/// Checks and applies a mod's border change: the centre, then either the new size at once or
/// a lerp from `size` to the target over the ticks. Nothing changes on an error.
pub(crate) fn apply(
    border: &mut WorldBorder,
    center: [f64; 2],
    size: f64,
    lerp: Option<(f64, u64)>,
) -> Result<(), String> {
    if center
        .iter()
        .any(|c| !c.is_finite() || c.abs() > MAX_CENTER)
    {
        return Err(format!(
            "a world border centre is finite and within {MAX_CENTER} of the origin"
        ));
    }
    let in_range = |value: f64| value.is_finite() && (1.0..=MAX_SIZE).contains(&value);
    if !in_range(size) {
        return Err(format!("a world border size is 1 to {MAX_SIZE}"));
    }
    if let Some((target, ticks)) = lerp {
        if !in_range(target) {
            return Err(format!("a world border target size is 1 to {MAX_SIZE}"));
        }
        if ticks == 0 || ticks > i64::MAX as u64 {
            return Err("a world border lerp lasts 1 or more ticks".into());
        }
    }
    border.set_center(center);
    match lerp {
        None => border.set_size(size),
        Some((target, ticks)) => border.lerp_between(size, target, ticks as i64),
    }
    Ok(())
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    fn prepare_border_level(&mut self, dim: super::Dim) -> Result<(), String> {
        if super::levels::custom_level(dim)
            && dim != self.dim
            && self.levels.parked_border(dim).is_none()
        {
            let back = self.dim;
            self.enter_level(None, dim).map_err(|e| e.to_string())?;
            self.enter_level(None, back).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn border_level(&self, dim: super::Dim) -> &WorldBorder {
        if super::levels::custom_level(dim) {
            if dim == self.dim {
                &self.border
            } else {
                self.levels.parked_border(dim).expect("border level opened")
            }
        } else if super::levels::custom_level(self.dim) {
            &self.levels.shared.border
        } else {
            &self.border
        }
    }

    fn border_level_mut(&mut self, dim: super::Dim) -> &mut WorldBorder {
        if super::levels::custom_level(dim) {
            if dim == self.dim {
                &mut self.border
            } else {
                self.levels
                    .parked_border_mut(dim)
                    .expect("border level opened")
            }
        } else if super::levels::custom_level(self.dim) {
            &mut self.levels.shared.border
        } else {
            &mut self.border
        }
    }

    /// The world border of `dimension`: its centre, current size, the size it is moving to and
    /// the ticks that are left of that move (0 when still). Vanilla levels share the Overworld's
    /// border; every custom level has its own. Server tick hooks only (512 calls per mod and
    /// tick). API @API:world_border_state@.
    pub(super) fn native_world_border(
        &mut self,
        dimension: &str,
    ) -> Result<NativeWorldBorderState, String> {
        self.native_physics_reserve(Operation::WorldBorder, false)?;
        dimension_ok(dimension)?;
        let dim = self
            .level_dim(dimension)
            .ok_or_else(|| "unknown native world border dimension".to_owned())?;
        self.prepare_border_level(dim)?;
        Ok(state(self.border_level(dim)))
    }

    /// Sets the world border of `dimension`: the centre, and the size, at once, or with `lerp`
    /// `(target, ticks)` moving from `size` to `target` over `ticks` server ticks
    /// (`WorldBorder.lerpSizeBetween`, as `/worldborder set <size> <time>` does). Sizes are 1 to
    /// 59999968, centres within 29999984 of the origin; nothing changes on an error. Players in
    /// that level hear it through vanilla's border packets. Mods listening to border changes
    /// (`native_world_border_bounds_changed`) are not told of this one. Server tick hooks only
    /// (512 calls per mod and tick). API @API:world_border_state@.
    pub(super) fn native_set_world_border(
        &mut self,
        dimension: &str,
        center: [f64; 2],
        size: f64,
        lerp: Option<(f64, u64)>,
    ) -> Result<(), String> {
        self.native_physics_reserve(Operation::SetWorldBorder, true)?;
        dimension_ok(dimension)?;
        let dim = self
            .level_dim(dimension)
            .ok_or_else(|| "unknown native world border dimension".to_owned())?;
        self.prepare_border_level(dim)?;
        apply(self.border_level_mut(dim), center, size, lerp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::lan::border::Sent;
    use horizon_protocol::packets::play::Clientbound as C;

    fn border() -> WorldBorder {
        let mut border = WorldBorder::default();
        border.set_size(100.0);
        border.set_center([0.0, 0.0]);
        border
    }

    #[test]
    fn the_state_is_the_borders_own() {
        let mut b = border();
        assert_eq!(
            state(&b),
            NativeWorldBorderState {
                center: [0.0, 0.0],
                size: 100.0,
                target_size: 100.0,
                remaining_ticks: 0,
            }
        );
        b.lerp_between(100.0, 40.0, 200);
        let s = state(&b);
        assert_eq!(
            (s.size, s.target_size, s.remaining_ticks),
            (100.0, 40.0, 200)
        );
        b.tick();
        assert_eq!(state(&b).remaining_ticks, 199);
    }

    #[test]
    fn an_immediate_change_sets_centre_and_size_and_sends_vanillas_packets() {
        let mut b = border();
        let sent = Sent::of(&b);
        apply(&mut b, [10.0, -20.0], 50.0, None).unwrap();
        assert_eq!(state(&b).center, [10.0, -20.0]);
        assert_eq!((state(&b).size, state(&b).remaining_ticks), (50.0, 0));
        let packets = sent.changes(&b);
        assert!(matches!(&packets[0], C::SetBorderCenter(p) if (p.x, p.z) == (10.0, -20.0)));
        assert!(matches!(&packets[1], C::SetBorderSize(p) if p.size == 50.0));
        assert_eq!(packets.len(), 2);
    }

    #[test]
    fn a_lerp_runs_from_the_given_size_to_the_target() {
        let mut b = border();
        let sent = Sent::of(&b);
        apply(&mut b, [0.0, 0.0], 80.0, Some((20.0, 600))).unwrap();
        let s = state(&b);
        assert_eq!(
            (s.size, s.target_size, s.remaining_ticks),
            (80.0, 20.0, 600)
        );
        let packets = sent.changes(&b);
        assert!(matches!(
            packets.as_slice(),
            [C::SetBorderLerpSize(p)] if (p.old_size, p.new_size, p.lerp_time) == (80.0, 20.0, 600)
        ));
        // It counts down with the server's ticks and lands on the target.
        for _ in 0..600 {
            b.tick();
        }
        let s = state(&b);
        assert_eq!((s.size, s.target_size, s.remaining_ticks), (20.0, 20.0, 0));
    }

    #[test]
    fn a_refused_change_leaves_the_border_alone() {
        let mut b = border();
        let before = state(&b);
        for (center, size, lerp) in [
            ([f64::NAN, 0.0], 50.0, None),
            ([MAX_CENTER + 1.0, 0.0], 50.0, None),
            ([0.0, 0.0], 0.5, None),
            ([0.0, 0.0], f64::INFINITY, None),
            ([0.0, 0.0], MAX_SIZE + 1.0, None),
            ([5.0, 5.0], 50.0, Some((0.5, 10))),
            ([5.0, 5.0], 50.0, Some((20.0, 0))),
            ([5.0, 5.0], 50.0, Some((f64::NAN, 10))),
        ] {
            assert!(
                apply(&mut b, center, size, lerp).is_err(),
                "{center:?} {size}"
            );
            assert_eq!(state(&b), before);
        }
    }

    #[test]
    fn custom_dimension_ids_are_valid_before_the_registry_lookup() {
        assert!(dimension_ok("minecraft:the_nether").is_ok());
        assert!(dimension_ok("example:arena").is_ok());
        assert!(dimension_ok("not a dimension").is_err());
    }
}
