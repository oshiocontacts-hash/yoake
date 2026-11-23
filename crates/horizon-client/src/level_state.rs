//! Clock and environment state shared by the server's level manager.
//!
//! The three vanilla levels deliberately keep the port's historical shared
//! Overworld clock, weather and border. Other levels use their type's clock;
//! clocks with the same id are one entry in [`ClockBank`].
use crate::{world::Weather, worldborder::WorldBorder};
use std::collections::BTreeMap;

pub const OVERWORLD_CLOCK: &str = "minecraft:overworld";

/// The clock a level follows. Vanilla's three levels retain their old shared
/// day time even though the 26.3 End type names its own clock.
pub fn level_clock(vanilla: bool, default_clock: Option<&str>) -> Option<&str> {
    if vanilla { Some(OVERWORLD_CLOCK) } else { default_clock }
}

/// The value of every named clock the server has advanced or read from its save.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClockBank {
    ticks: BTreeMap<String, i64>,
}

impl ClockBank {
    pub fn get(&self, id: &str) -> Option<i64> {
        self.ticks.get(id).copied()
    }

    pub fn set(&mut self, id: &str, ticks: i64) {
        self.ticks.insert(id.to_owned(), ticks);
    }

    pub fn get_or_insert(&mut self, id: &str, initial: i64) -> i64 {
        *self.ticks.entry(id.to_owned()).or_insert(initial)
    }

    /// Advance every clock once. The level in place supplies its own current
    /// value so a hook or command that changed it earlier in the tick is kept.
    /// A fixed-time level has no active clock and changes no day time itself.
    pub fn advance(&mut self, active: Option<&str>, active_day: i64, amount: i64) -> Option<i64> {
        for (id, ticks) in &mut self.ticks {
            if Some(id.as_str()) != active {
                *ticks += amount;
            }
        }
        active.map(|id| {
            let ticks = active_day + amount;
            self.set(id, ticks);
            ticks
        })
    }

    /// The entries `Level` writes in addition to its Overworld clock.
    pub fn saved(&self) -> BTreeMap<String, i64> {
        self.ticks
            .iter()
            .filter(|(id, _)| id.as_str() != OVERWORLD_CLOCK)
            .map(|(id, ticks)| (id.clone(), *ticks))
            .collect()
    }
}

/// State shared by the vanilla levels, separate from a custom level in place.
#[derive(Clone, Debug, Default)]
pub struct SharedEnvironment {
    pub weather: Weather,
    pub border: WorldBorder,
}

/// The environment fields one level carries into and out of the active slot.
#[derive(Clone, Debug, Default)]
pub struct LevelEnvironment {
    pub day_time: i64,
    pub weather: Weather,
    pub border: WorldBorder,
}

pub fn remember_level(
    clocks: &mut ClockBank,
    shared: &mut SharedEnvironment,
    vanilla: bool,
    clock: Option<&str>,
    active: &LevelEnvironment,
) {
    if vanilla {
        shared.weather = active.weather;
        shared.border = active.border.clone();
    }
    if let Some(clock) = clock {
        clocks.set(clock, active.day_time);
    }
}

/// Restore a target after its level's own fields were loaded or unparked.
/// Custom weather and borders are deliberately left untouched.
pub fn restore_level(
    clocks: &mut ClockBank,
    shared: &SharedEnvironment,
    vanilla: bool,
    clock: Option<&str>,
    initial_clock: i64,
    target: &mut LevelEnvironment,
) {
    if let Some(clock) = clock {
        target.day_time = clocks.get_or_insert(clock, initial_clock);
    }
    if vanilla {
        target.weather = shared.weather;
        target.border = shared.border.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn different_clocks_advance_once_and_keep_their_own_values() {
        let mut clocks = ClockBank::default();
        clocks.set("example:sun", 100);
        clocks.set("example:moon", 9_000);
        assert_eq!(clocks.advance(Some("example:sun"), 100, 2), Some(102));
        assert_eq!(clocks.get("example:sun"), Some(102));
        assert_eq!(clocks.get("example:moon"), Some(9_002));
        clocks.advance(None, 18_000, 1);
        assert_eq!(clocks.get("example:sun"), Some(103));
        assert_eq!(clocks.get("example:moon"), Some(9_003));
    }

    #[test]
    fn levels_naming_one_clock_share_and_vanilla_keeps_the_old_clock() {
        let mut clocks = ClockBank::default();
        clocks.set("example:shared", 40);
        assert_eq!(clocks.get_or_insert("example:shared", 999), 40);
        assert_eq!(level_clock(false, Some("example:shared")), Some("example:shared"));
        for default in [None, Some("minecraft:the_end"), Some("example:other")] {
            assert_eq!(level_clock(true, default), Some(OVERWORLD_CLOCK));
        }
        assert_eq!(level_clock(false, None), None);
    }

    #[test]
    fn swaps_keep_custom_weather_and_borders_but_not_vanilla_ones() {
        let mut clocks = ClockBank::default();
        let mut shared = SharedEnvironment::default();
        let overworld = LevelEnvironment {
            day_time: 700,
            weather: Weather::default(),
            border: WorldBorder::default(),
        };
        remember_level(&mut clocks, &mut shared, true, Some(OVERWORLD_CLOCK), &overworld);

        let mut custom = LevelEnvironment {
            day_time: 0,
            weather: Weather {
                raining: true,
                thundering: true,
                rain_time: 50,
                thunder_time: 60,
                rain_level: (1.0, 1.0),
                thunder_level: (1.0, 1.0),
                ..Weather::default()
            },
            border: WorldBorder::default(),
        };
        custom.border.set_size(80.0);
        restore_level(&mut clocks, &shared, false, Some("example:moon"), 500, &mut custom);
        assert_eq!(custom.day_time, 500);
        assert_eq!(custom.border.size(), 80.0);
        custom.day_time = 540;
        remember_level(&mut clocks, &mut shared, false, Some("example:moon"), &custom);

        for default in [None, Some("minecraft:the_end")] {
            let mut vanilla = LevelEnvironment::default();
            restore_level(&mut clocks, &shared, true, level_clock(true, default), 0, &mut vanilla);
            assert_eq!(vanilla.day_time, 700);
            assert!(!vanilla.weather.raining);
            assert_eq!(vanilla.border, shared.border);
            remember_level(&mut clocks, &mut shared, true, Some(OVERWORLD_CLOCK), &vanilla);
        }
        restore_level(&mut clocks, &shared, false, Some("example:moon"), 0, &mut custom);
        assert_eq!(custom.day_time, 540);
        assert!(custom.weather.raining && custom.weather.thundering);
        assert_eq!((custom.weather.rain_time, custom.weather.thunder_time), (50, 60));
        assert_eq!(custom.border.size(), 80.0);
    }
}
