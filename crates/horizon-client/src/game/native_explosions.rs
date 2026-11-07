//! Explosions for native server mods (docs/native-mods.md, "Explosions"): a mod
//! sets one off as `Level.explode` does, in the level in place, which it names
//! by its `dimension` (`minecraft:overworld`) as `native_set_block` does.
use super::{Game, combat::CombatTracker, explosion::BlockInteraction};

/// The largest radius `native_explode` takes. A blast's cost grows with its
/// radius (its 1352 rays walk `radius * 1.3` blocks at most); vanilla's
/// largest is the wither's charged spawn, 7.
pub(super) const NATIVE_EXPLOSION_MAX_RADIUS: f32 = 32.0;

/// What a mod's blast does to blocks (`Level.ExplosionInteraction`). Built by
/// mods only.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NativeBlast {
    /// Blocks stay; entities are hurt and pushed (`ExplosionInteraction.NONE`).
    Keep,
    /// Blocks break and drop by the `block_explosion_drop_decay` rule
    /// (`ExplosionInteraction.BLOCK`).
    #[default]
    Block,
    /// Blocks break and drop by the `tnt_explosion_drop_decay` rule
    /// (`ExplosionInteraction.TNT`).
    Tnt,
}

impl NativeBlast {
    fn interaction(self, drop_decay: impl FnOnce(&str) -> bool) -> BlockInteraction {
        let rule = match self {
            Self::Keep => return BlockInteraction::Keep,
            Self::Block => "block_explosion_drop_decay",
            Self::Tnt => "tnt_explosion_drop_decay",
        };
        if drop_decay(rule) {
            BlockInteraction::DestroyWithDecay
        } else {
            BlockInteraction::Destroy
        }
    }
}

/// Whether `native_explode` takes this centre and radius.
fn blast_ok(center: [f64; 3], radius: f32) -> bool {
    center
        .into_iter()
        .all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
        && radius.is_finite()
        && radius > 0.0
        && radius <= NATIVE_EXPLOSION_MAX_RADIUS
}

impl Game<'_> {
    /// Explodes at `center` in the level in place, `dimension`, as
    /// `Level.explode(null, x, y, z, radius, fire, interaction)` does: the
    /// blocks it reaches break (`native_explosion_detonate` sees them first),
    /// entities are hurt and pushed, `fire` lights a third of the air above
    /// solid blocks, and everyone near hears and sees it. False, with nothing
    /// done, on a client's copy of the world, for another dimension, a chunk
    /// that isn't loaded, or a radius that isn't in `0 < radius <= 32`. Call
    /// it from a server hook; a blast inside `native_explosion_detonate`
    /// starts that hook again. @API:explode_helper@.
    pub(super) fn native_explode(
        &mut self,
        dimension: &str,
        at: [f64; 3],
        power: f32,
        fire: bool,
        interaction: NativeBlast,
    ) -> bool {
        if !blast_ok(at, power) {
            return false;
        }
        let chunk = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
        if !self.world.chunks.contains_key(&chunk) {
            return false;
        }
        let Ok(_delivery) = self.native_damage_reserve(dimension, true) else {
            return false;
        };
        let interaction = interaction.interaction(|rule| self.ui.rules.bool(rule));
        // `Level.explode(null, ...)`: no source. Called from inside another
        // blast (`native_explosion_detonate`), it must not take that blast's
        // source, owner or flags, and that blast gets them back after.
        let outer = BlastState::take(&mut self.combat);
        let done = self.server_explode(at, power, interaction, fire).is_ok();
        outer.restore(&mut self.combat);
        done
    }
}

impl BlastState {
    /// The blast going off now, cleared to a sourceless one.
    fn take(c: &mut CombatTracker) -> Self {
        Self {
            source: c.explosion_source.take(),
            cause: c.explosion_cause.take(),
            source_named: std::mem::take(&mut c.explosion_source_named),
            in_water: std::mem::take(&mut c.explosion_in_water),
            hive_emptying: std::mem::take(&mut c.hive_emptying_blast),
            rail_proof: std::mem::take(&mut c.rail_proof_blast),
            owner: c.explosion_owner.take(),
            player_owner: c.blast_owner.take(),
            player_guest: c.blast_guest.take(),
            resistance_cap: c.blast_resistance_cap.take(),
        }
    }

    /// Puts back what `take` cleared.
    fn restore(self, c: &mut CombatTracker) {
        c.explosion_source = self.source;
        c.explosion_cause = self.cause;
        c.explosion_source_named = self.source_named;
        c.explosion_in_water = self.in_water;
        c.hive_emptying_blast = self.hive_emptying;
        c.rail_proof_blast = self.rail_proof;
        c.explosion_owner = self.owner;
        c.blast_owner = self.player_owner;
        c.blast_guest = self.player_guest;
        c.blast_resistance_cap = self.resistance_cap;
    }
}

/// The fields of `CombatTracker` that describe the blast going off now.
struct BlastState {
    source: Option<u32>,
    cause: Option<horizon_client::advancements::EntityView>,
    source_named: bool,
    in_water: bool,
    hive_emptying: bool,
    rail_proof: bool,
    owner: Option<u32>,
    player_owner: Option<(u32, horizon_client::text::Text)>,
    player_guest: Option<u32>,
    resistance_cap: Option<f32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blast_needs_a_finite_centre_and_a_radius_in_range() {
        assert!(blast_ok([0.5, 64.0, 0.5], 4.0));
        assert!(blast_ok([0.5, 64.0, 0.5], NATIVE_EXPLOSION_MAX_RADIUS));
        assert!(!blast_ok([0.5, 64.0, 0.5], 0.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], -1.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], 33.0));
        assert!(!blast_ok([0.5, 64.0, 0.5], f32::NAN));
        assert!(!blast_ok([f64::INFINITY, 64.0, 0.5], 4.0));
        assert!(!blast_ok([0.5, 64.0, 4.0e7], 4.0));
    }

    #[test]
    fn block_interactions_follow_the_selected_drop_decay_rule() {
        assert_eq!(
            NativeBlast::Keep.interaction(|_| panic!("keep has no drop decay")),
            BlockInteraction::Keep
        );
        for (blast, expected_rule) in [
            (NativeBlast::Block, "block_explosion_drop_decay"),
            (NativeBlast::Tnt, "tnt_explosion_drop_decay"),
        ] {
            for (decay, expected) in [
                (false, BlockInteraction::Destroy),
                (true, BlockInteraction::DestroyWithDecay),
            ] {
                assert_eq!(
                    blast.interaction(|rule| {
                        assert_eq!(rule, expected_rule);
                        decay
                    }),
                    expected
                );
            }
        }
    }

    #[test]
    fn a_nested_sourceless_blast_does_not_inherit_credit_or_flags() {
        let mut combat = CombatTracker::default();
        combat.explosion_source = Some(17);
        combat.explosion_source_named = true;
        combat.explosion_in_water = true;
        combat.hive_emptying_blast = true;
        combat.rail_proof_blast = true;
        combat.explosion_owner = Some(19);
        combat.blast_owner = Some((23, horizon_client::text::Text::literal("actor")));
        combat.blast_guest = Some(29);
        combat.blast_resistance_cap = Some(0.8);
        let outer = BlastState::take(&mut combat);
        assert_eq!(combat.explosion_source, None);
        assert!(!combat.explosion_source_named);
        assert!(!combat.explosion_in_water);
        assert!(!combat.hive_emptying_blast);
        assert!(!combat.rail_proof_blast);
        assert_eq!(combat.explosion_owner, None);
        assert!(combat.blast_owner.is_none());
        assert_eq!(combat.blast_guest, None);
        assert_eq!(combat.blast_resistance_cap, None);
        combat.explosion_source = Some(31);
        combat.blast_guest = Some(37);
        outer.restore(&mut combat);
        assert_eq!(combat.explosion_source, Some(17));
        assert!(combat.explosion_source_named);
        assert!(combat.explosion_in_water);
        assert!(combat.hive_emptying_blast);
        assert!(combat.rail_proof_blast);
        assert_eq!(combat.explosion_owner, Some(19));
        assert_eq!(combat.blast_owner.as_ref().map(|owner| owner.0), Some(23));
        assert_eq!(combat.blast_guest, Some(29));
        assert_eq!(combat.blast_resistance_cap, Some(0.8));
    }
}
