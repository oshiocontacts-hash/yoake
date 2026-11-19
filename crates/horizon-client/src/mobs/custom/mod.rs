//! Fully custom mobs (FR-F4-MOB, docs/yoake2/FR-F4-MOBS.md). F4M-0 only lays the foundation: the
//! shared immutable [`CustomType`] and the empty [`registry`]. Nothing registers a type yet (F4M-3
//! builds them from `horizon_mods::custom_mob`), so with no mod every `MobType.custom` is `None`
//! and none of this code runs.
//!
//! New code for custom mobs lives under this directory (and `game/custom_mob_*.rs`) so that
//! parallel work items do not collide in the shared mob files.
pub mod registry;

pub use registry::{CustomRegistry, registry};

use horizon_mods::custom_mob::CustomMobFile;

/// One custom mob type: loaded identically on client, host and dedicated server, shared by every
/// mob of the type behind an `Arc`.
#[derive(Debug)]
pub struct CustomType {
    /// `<namespace>:<path>`.
    pub id: String,
    /// The validated definition as written in the mod's JSON.
    pub file: CustomMobFile,
}

#[cfg(test)]
mod tests {
    use super::super::{Brain, Species, species};

    /// The neutral row of `Species::Custom`: no vanilla behaviour leaks into a custom mob through
    /// a table row or a wildcard arm, and the vanilla rows are found as before.
    #[test]
    fn the_custom_species_row_is_neutral() {
        let row = species::def(Species::Custom);
        assert_eq!(row.species, Species::Custom);
        assert!(matches!(row.baby, species::Baby::None));
        assert_eq!(row.sounds, ["", "", ""]);
        assert!(row.step_sound.is_none());
        assert_eq!(row.brain(), Brain::Passive);
        assert_eq!(row.water_slowdown(), 0.8);
        assert_eq!(row.air_speed(), 0.02);
        assert_eq!(row.baby_scale(), 0.5);
        assert_eq!(row.head_rot_speed(), 10.0);
        assert_eq!(row.max_head_x_rot(), 40.0);
        assert_eq!(row.max_head_y_rot(), 75.0);
        assert_eq!(row.ambient_interval(), 80);
        let flags = row.flags(false);
        assert!(!(flags.aquatic_swimmer || flags.fall_immune || flags.flying || flags.climber));
        assert!(!(flags.lava_walker || flags.omnidirectional || flags.fire_immune));
        assert!(!(flags.burns_in_day || flags.undead || flags.breathes_water || flags.needs_water));
        assert!(!(flags.water_sensitive || flags.animal));
        assert!(row.food_tag().is_none() && row.tempt_tag().is_none());
        // No table row is for it, and every vanilla species still resolves to its own row.
        assert!(species::ALL.iter().all(|r| r.species != Species::Custom));
        for r in species::ALL {
            let first = species::ALL.iter().find(|d| d.species == r.species).unwrap();
            assert_eq!(species::def(r.species).name, first.name);
        }
    }
}
