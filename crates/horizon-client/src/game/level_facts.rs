//! F3-4: what a level's dimension type says about its sky, weather, light and
//! spawning, so a level past the vanilla three behaves by its type and not as
//! the Overworld by default.
//!
//! The three vanilla levels keep their fixed rules (the constants below are
//! the tests the code made on `Dim` before), so they behave byte for byte as
//! they did. Any other level reads its facts from the type its world gives
//! it: the level registry on a server, the dimension type entry the server
//! sent on a client.
use super::{Dim, Game, Resources};
use std::collections::HashMap;
use horizon_client::environment::Environment;
use horizon_client::mobs::{CustomLevel, spawn_rules::LightRule};
use horizon_world::level::LevelRules;
use horizon_world::dimension::{DimensionType, IntProvider};
use horizon_world::nbt::Tag;
use serde_json::Value;

/// The sky a level draws (`DimensionType.skybox`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SkyKind {
    /// Sun, moon, stars and clouds.
    Overworld,
    /// The End's sky box.
    End,
    /// No sky at all.
    None,
}

/// `BedRule.canSleep`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Sleep {
    Always,
    WhenDark,
    Never,
}

/// The `bed_rule` and `straw_bed_rule` attributes and the respawn anchor
/// rule of a level.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct BedRules {
    pub sleep: Sleep,
    pub sets_spawn: bool,
    pub destroy_on_use: bool,
    pub straw_sets_spawn: bool,
    pub straw_destroy_on_leave: bool,
    /// `gameplay/respawn_anchor_works`.
    pub anchor_works: bool,
}
impl BedRules {
    const OVERWORLD: Self = Self {
        sleep: Sleep::WhenDark,
        sets_spawn: true,
        destroy_on_use: false,
        straw_sets_spawn: false,
        straw_destroy_on_leave: true,
        anchor_works: false,
    };
    const NETHER: Self = Self {
        sleep: Sleep::Never,
        sets_spawn: false,
        destroy_on_use: true,
        straw_sets_spawn: false,
        straw_destroy_on_leave: false,
        anchor_works: true,
    };
    const END: Self = Self {
        anchor_works: false,
        ..Self::NETHER
    };
    /// The rules of a type. A rule the type leaves out is the attribute's default:
    /// sleep when dark, set the spawn, no destroying (the straw bed the same).
    fn of_type(t: &DimensionType) -> Self {
        let rule = |key: &str| t.attributes.get(&format!("minecraft:gameplay/{key}"));
        let spawn = |r: Option<&Value>| {
            r.and_then(|r| r["can_set_spawn"].as_str())
                .is_none_or(|s| s != "never")
        };
        let flag = |r: Option<&Value>, k: &str| r.and_then(|r| r[k].as_bool()).unwrap_or(false);
        let bed = rule("bed_rule");
        let straw = rule("straw_bed_rule");
        Self {
            sleep: match bed.and_then(|r| r["can_sleep"].as_str()) {
                Some("never") => Sleep::Never,
                Some("always") => Sleep::Always,
                _ => Sleep::WhenDark,
            },
            sets_spawn: spawn(bed),
            destroy_on_use: flag(bed, "destroy_on_use"),
            straw_sets_spawn: spawn(straw),
            straw_destroy_on_leave: flag(straw, "destroy_on_leave"),
            anchor_works: t.respawn_anchor_works(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LevelFacts {
    /// Beds and respawn anchors.
    pub beds: BedRules,
    pub sky: SkyKind,
    /// `Level.canHaveWeather`: rain and thunder rise and fall here.
    pub weather: bool,
    /// `cardinal_light: nether`: the Nether's block and lightmap shading.
    pub nether_light: bool,
    /// The type sets an environmental fog distance (the Nether's fog).
    pub fog_distance: bool,
    /// `DimensionType.ambientLight`.
    pub ambient: f32,
    /// The ambient light the lightmap at the eye uses (the End's is 0 there
    /// as it always was).
    pub light_ambient: f32,
    /// Water evaporates and buckets of it empty (`ultrawarm`).
    pub ultrawarm: bool,
    /// The sun decides when monsters burn (a day cycle, not a fixed time).
    pub day_cycle: bool,
    /// Raids, patrols and village sieges may start (`gameplay/can_start_raid`).
    pub raids: bool,
    /// What mobs read of a level past the vanilla three (the vanilla levels
    /// keep the fixed rules `MobWorld.nether` and `end` stand for).
    pub custom: Option<CustomLevel>,
    /// What block behaviour reads of the type (`horizon-world`).
    pub rules: LevelRules,
    /// The player's lava flows fast.
    pub fast_lava: bool,
    /// The highest block y mobs and heightmaps look at.
    pub top: i32,
    pub sea_level: i32,
}

impl LevelFacts {
    pub(super) const OVERWORLD: Self = Self {
        sky: SkyKind::Overworld,
        weather: true,
        nether_light: false,
        fog_distance: false,
        ambient: 0.0,
        light_ambient: 0.0,
        ultrawarm: false,
        day_cycle: true,
        custom: None,
        beds: BedRules::OVERWORLD,
        rules: LevelRules::OVERWORLD,
        fast_lava: false,
        raids: true,
        top: 319,
        sea_level: 63,
    };
    pub(super) const NETHER: Self = Self {
        sky: SkyKind::None,
        weather: false,
        nether_light: true,
        fog_distance: true,
        ambient: 0.1,
        light_ambient: 0.1,
        ultrawarm: true,
        day_cycle: false,
        custom: None,
        beds: BedRules::NETHER,
        rules: LevelRules::NETHER,
        fast_lava: true,
        raids: false,
        top: 127,
        sea_level: 32,
    };
    pub(super) const END: Self = Self {
        sky: SkyKind::End,
        weather: false,
        nether_light: false,
        fog_distance: false,
        ambient: 0.25,
        light_ambient: 0.0,
        ultrawarm: false,
        day_cycle: false,
        custom: None,
        beds: BedRules::END,
        rules: LevelRules::END,
        fast_lava: false,
        raids: false,
        top: 319,
        sea_level: 63,
    };

    /// The number shader packs and custom entity models know the level by:
    /// -1 the Nether, 1 the End, 0 any other.
    pub(super) fn shader_dimension(&self) -> i32 {
        match (self.sky, self.nether_light) {
            (SkyKind::End, _) => 1,
            (_, true) => -1,
            _ => 0,
        }
    }

    /// A level's facts from its dimension type.
    pub(super) fn of_type(t: &DimensionType) -> Self {
        let sky = match t.skybox.as_str() {
            "overworld" => SkyKind::Overworld,
            "end" => SkyKind::End,
            _ => SkyKind::None,
        };
        // A type's own timelines drive its day; one with none and a fixed time has no day.
        let day_cycle = sky == SkyKind::Overworld && (!t.has_fixed_time || t.timelines.is_some());
        Self {
            sky,
            // Rain needs open sky under a sky that has weather.
            weather: sky == SkyKind::Overworld && t.has_skylight && !t.has_ceiling,
            nether_light: t.cardinal_light == "nether",
            fog_distance: t.attributes.contains_key("minecraft:visual/fog_end_distance"),
            ambient: t.ambient_light,
            light_ambient: t.ambient_light,
            ultrawarm: t.water_evaporates(),
            day_cycle,
            custom: Some(CustomLevel {
                spawn_light: match &t.monster_spawn_light_level {
                    IntProvider::Constant(n) => LightRule {
                        block_limit: t.monster_spawn_block_light_limit,
                        min: *n,
                        max: *n,
                        constant: true,
                    },
                    IntProvider::Uniform { min, max } => LightRule {
                        block_limit: t.monster_spawn_block_light_limit,
                        min: *min,
                        max: *max,
                        constant: false,
                    },
                    // A provider the engine does not model: the Overworld's.
                    IntProvider::Other(_) => LightRule {
                        block_limit: t.monster_spawn_block_light_limit,
                        min: 0,
                        max: 7,
                        constant: false,
                    },
                },
                piglins_zombify: t.piglins_zombify(),
                snow_golem_melts: t.snow_golem_melts(),
                fast_lava: t.fast_lava(),
                has_skylight: t.has_skylight,
                day_cycle,
                ambient: t.ambient_light,
            }),
            beds: BedRules::of_type(t),
            rules: LevelRules::of_type(t),
            fast_lava: t.fast_lava(),
            raids: t.can_start_raid(),
            top: t.min_y + t.logical_height - 1,
            sea_level: if t.has_ceiling { 32 } else { 63 },
        }
    }
}

/// Where a level's sky and fog attributes come from.
pub(super) enum LevelEnv {
    /// One of the three vanilla environments (a level using a vanilla type).
    Vanilla(Dim),
    /// The type's own attributes.
    Own(Environment),
}

/// The type of a level that is not one of the vanilla three.
pub(super) struct LevelCtx {
    pub facts: LevelFacts,
    pub env: LevelEnv,
    pub dimension_type: DimensionType,
}

/// A dimension type the server sent as NBT, as the JSON the type codec reads:
/// `json_to_nbt` made its booleans bytes, so a byte is a boolean here (the
/// general `tag_json` keeps it a number).
pub(super) fn type_json(tag: &Tag) -> Value {
    match tag {
        Tag::Byte(v) => Value::Bool(*v != 0),
        Tag::List(l) => Value::Array(l.iter().map(type_json).collect()),
        Tag::Compound(c) => Value::Object(
            c.iter()
                .map(|(k, v)| (k.to_string_lossy(), type_json(v)))
                .collect(),
        ),
        other => horizon_client::item_render::tag_json(other),
    }
}

/// The sky and fog attributes of level `dim`: the vanilla three have their
/// own, any other level its type's (or the vanilla one its type is).
pub(super) fn environment_for<'a>(
    dim: Dim,
    ctxs: &'a HashMap<Dim, LevelCtx>,
    res: &'a Resources,
) -> &'a Environment {
    let vanilla = |dim: Dim| match dim {
        Dim::Nether => res.nether_environment(),
        Dim::End => res.end_environment(),
        _ => res.environment(),
    };
    match dim {
        Dim::Overworld | Dim::Nether | Dim::End => vanilla(dim),
        dim => match ctxs.get(&dim).map(|c| &c.env) {
            Some(LevelEnv::Own(env)) => env,
            Some(LevelEnv::Vanilla(of)) => vanilla(*of),
            None => res.environment(),
        },
    }
}

impl Game<'_> {
    /// The facts of the level in place.
    pub(super) fn level_facts(&self) -> LevelFacts {
        match self.dim {
            Dim::Overworld => LevelFacts::OVERWORLD,
            Dim::Nether => LevelFacts::NETHER,
            Dim::End => LevelFacts::END,
            dim => self
                .level_ctxs
                .get(&dim)
                .map_or(LevelFacts::OVERWORLD, |c| c.facts),
        }
    }

    /// Reads the dimension type of the level in place (`self.dim`) once: from
    /// the level registry on a server, from the entry a server sent on a
    /// client (its data, or its name for a type the client knows). A client
    /// calls it with `sent` at every entry, so a type the server changed is
    /// read again.
    pub(super) fn ensure_level_ctx(&mut self, sent: Option<(String, Option<Value>)>) {
        let dim = self.dim;
        if matches!(dim, Dim::Overworld | Dim::Nether | Dim::End) {
            return;
        }
        if sent.is_none() && self.level_ctxs.contains_key(&dim) {
            return;
        }
        self.level_ctxs.remove(&dim);
        let (type_name, dimension_type) = match sent {
            Some((name, Some(json))) => match DimensionType::from_value(&json) {
                Ok(t) => (name, t),
                Err(error) => {
                    log::warn!("level {}: its dimension type is unusable: {error}", dim.id());
                    return;
                }
            },
            Some((name, None)) => match DimensionType::vanilla(&name) {
                Some(t) => (name, t),
                None => return,
            },
            None => {
                let Some(entry) = self.level_registry.get(dim.id()) else {
                    return;
                };
                let name = match entry.stem.as_ref().map(|s| &s.dimension_type) {
                    Some(horizon_world::dimension::TypeRef::Id(name)) => name.clone(),
                    _ => dim.id().to_owned(),
                };
                (name, entry.dimension_type.clone())
            }
        };
        let facts = LevelFacts::of_type(&dimension_type);
        // The world's unzipped data packs: where a pack type's own timelines are.
        let packs: Vec<std::path::PathBuf> = if self.remote.is_some() {
            Vec::new()
        } else {
            let mut dirs: Vec<_> = std::fs::read_dir(self.world_root.join("datapacks"))
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect();
            dirs.sort();
            dirs
        };
        // A type the game ships (the Overworld's, the Nether's, the End's)
        // keeps that environment, with the prepared bundle's visuals; any
        // other type is read from its own data.
        let env = match type_name.as_str() {
            "minecraft:overworld" | "minecraft:overworld_caves" => LevelEnv::Vanilla(Dim::Overworld),
            "minecraft:the_nether" => LevelEnv::Vanilla(Dim::Nether),
            "minecraft:the_end" => LevelEnv::Vanilla(Dim::End),
            _ => match Environment::load_type_timelines(
                &self.options.resources,
                &packs,
                &dimension_type.to_value(),
                facts.day_cycle,
            ) {
                Ok(env) => LevelEnv::Own(env),
                Err(error) => {
                    log::warn!("level {}: no sky data for its type: {error}", dim.id());
                    LevelEnv::Vanilla(Dim::Overworld)
                }
            },
        };
        self.level_ctxs.insert(
            dim,
            LevelCtx {
                facts,
                env,
                dimension_type,
            },
        );
    }

    /// The type of the level in place when it is not a vanilla one.
    pub(super) fn level_type(&self) -> Option<&DimensionType> {
        self.level_ctxs.get(&self.dim).map(|c| &c.dimension_type)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(id: &str) -> LevelFacts {
        LevelFacts::of_type(&DimensionType::vanilla(id).expect("a vanilla type"))
    }

    /// A level that uses a vanilla type behaves as the vanilla level does, as far as
    /// the type says (the Nether's fog distance and the End's lightmap are the
    /// bundle's and the old rule's).
    #[test]
    fn the_vanilla_types_read_as_the_vanilla_levels() {
        for (id, want) in [
            ("minecraft:overworld", LevelFacts::OVERWORLD),
            ("minecraft:the_nether", LevelFacts::NETHER),
            ("minecraft:the_end", LevelFacts::END),
        ] {
            let got = facts(id);
            assert_eq!(got.sky, want.sky, "{id}");
            assert_eq!(got.weather, want.weather, "{id}");
            assert_eq!(got.nether_light, want.nether_light, "{id}");
            assert_eq!(got.ambient, want.ambient, "{id}");
            assert_eq!(got.day_cycle, want.day_cycle, "{id}");
            assert_eq!(got.sea_level, want.sea_level, "{id}");
            assert_eq!(got.top, want.top.min(got.top), "{id}");
        }
        assert_eq!(facts("minecraft:overworld").top, 319);
        assert_eq!(facts("minecraft:the_nether").top, 127);
        assert!(facts("minecraft:the_nether").ultrawarm);
        assert!(!facts("minecraft:overworld").ultrawarm);
        // The Overworld's rule is uniform 0..=7 and block light 0; the Nether's 7 and 15.
        let rule = |id| facts(id).custom.unwrap().spawn_light;
        assert_eq!((rule("minecraft:overworld").min, rule("minecraft:overworld").max), (0, 7));
        assert!(!rule("minecraft:overworld").constant);
        assert_eq!(rule("minecraft:the_nether").block_limit, 15);
        assert!(rule("minecraft:the_nether").constant);
        assert_eq!(rule("minecraft:the_end").min, 15);
    }

    /// The type a server sent as NBT reads back as the type it was made from.
    #[test]
    fn a_type_sent_as_nbt_reads_back_the_same() {
        let mut t = DimensionType::new(0, 128);
        t.ambient_light = 0.3;
        t.has_ceiling = false;
        t.attributes
            .insert("minecraft:gameplay/can_start_raid".into(), Value::Bool(false));
        let nbt = horizon_client::lan::data::json_to_nbt(&t.to_value());
        let back = DimensionType::from_value(&type_json(&nbt)).unwrap();
        assert_eq!(back, t);
        assert!(!LevelFacts::of_type(&back).raids);
        assert_eq!(LevelFacts::of_type(&back).top, 127);
    }

    /// A type with no sky, a ceiling and the Nether's shading.
    #[test]
    fn a_type_without_a_sky_has_no_weather_and_no_day() {
        let mut t = DimensionType::new(0, 64);
        t.skybox = "none".into();
        t.has_ceiling = true;
        t.has_skylight = false;
        t.cardinal_light = "nether".into();
        let f = LevelFacts::of_type(&t);
        assert_eq!(f.sky, SkyKind::None);
        assert!(!f.weather && !f.day_cycle && f.nether_light);
        assert_eq!(f.sea_level, 32);
    }

    /// A Nether-like type of a pack: the Nether's rules from its attributes.
    fn hell() -> DimensionType {
        let mut t = DimensionType::new(0, 128);
        t.skybox = "none".into();
        t.has_skylight = false;
        t.has_ceiling = true;
        t.has_fixed_time = true;
        t.ambient_light = 0.1;
        t.cardinal_light = "nether".into();
        t.monster_spawn_block_light_limit = 15;
        t.monster_spawn_light_level = IntProvider::Constant(7);
        for (k, v) in [
            ("fast_lava", serde_json::json!(true)),
            ("water_evaporates", serde_json::json!(true)),
            ("piglins_zombify", serde_json::json!(false)),
            ("snow_golem_melts", serde_json::json!(true)),
            ("respawn_anchor_works", serde_json::json!(true)),
            ("bed_rule", serde_json::json!({"can_set_spawn": "never", "can_sleep": "never", "destroy_on_use": true})),
            ("straw_bed_rule", serde_json::json!({"can_set_spawn": "never", "can_sleep": "never", "destroy_on_use": true})),
        ] {
            t.attributes.insert(format!("minecraft:gameplay/{k}"), v);
        }
        t
    }

    /// F3-4b: a level of a Nether-like type of its own gets what the Nether gets.
    #[test]
    fn a_nether_like_type_behaves_like_the_nether() {
        let f = LevelFacts::of_type(&hell());
        let c = f.custom.unwrap();
        assert!(f.fast_lava && f.rules.fast_lava && f.rules.water_evaporates && f.ultrawarm);
        assert!(!c.piglins_zombify && c.snow_golem_melts && c.fast_lava && !c.has_skylight);
        assert!(!c.day_cycle && !f.weather && f.nether_light);
        assert_eq!(c.spawn_light.block_limit, 15);
        assert_eq!(f.beds, BedRules::NETHER, "bed, straw bed and anchor rules");
        assert!(!f.rules.has_skylight);
    }

    /// The beds of a type with no bed attributes are the attribute defaults, and a
    /// Nether's and an End's rules read back from the real attributes.
    #[test]
    fn bed_rules_read_the_attributes() {
        let plain = LevelFacts::of_type(&DimensionType::new(0, 128)).beds;
        assert_eq!(plain.sleep, Sleep::WhenDark);
        assert!(plain.sets_spawn && !plain.destroy_on_use && !plain.anchor_works);
        let mut end = hell();
        end.attributes.insert("minecraft:gameplay/respawn_anchor_works".into(), serde_json::json!(false));
        assert_eq!(LevelFacts::of_type(&end).beds, BedRules::END);
        let mut sleepy = DimensionType::new(0, 128);
        sleepy.attributes.insert(
            "minecraft:gameplay/bed_rule".into(),
            serde_json::json!({"can_sleep": "always", "can_set_spawn": "always"}),
        );
        assert_eq!(LevelFacts::of_type(&sleepy).beds.sleep, Sleep::Always);
    }

    /// A type that lists timelines has a day even with a fixed time; one that lists none and
    /// has a fixed time has none.
    #[test]
    fn timelines_give_a_type_a_day() {
        let mut t = DimensionType::new(0, 128);
        t.has_fixed_time = true;
        assert!(!LevelFacts::of_type(&t).day_cycle);
        t.timelines = Some(horizon_world::dimension::Timelines::Tag("#mypack:own".into()));
        assert!(LevelFacts::of_type(&t).day_cycle);
        assert!(LevelFacts::of_type(&t).custom.unwrap().day_cycle);
        assert!(LevelFacts::of_type(&t).rules.day_cycle);
    }
}
