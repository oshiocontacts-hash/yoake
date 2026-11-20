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
use horizon_client::mobs::spawn_rules::LightRule;
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct LevelFacts {
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
    /// Monster spawn light rule, for a level past the vanilla three (the
    /// vanilla levels keep the fixed rules of `mobs::spawn_rules`).
    pub spawn_light: Option<LightRule>,
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
        spawn_light: None,
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
        spawn_light: None,
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
        spawn_light: None,
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
        Self {
            sky,
            // Rain needs open sky under a sky that has weather.
            weather: sky == SkyKind::Overworld && t.has_skylight && !t.has_ceiling,
            nether_light: t.cardinal_light == "nether",
            fog_distance: t.attributes.contains_key("minecraft:visual/fog_end_distance"),
            ambient: t.ambient_light,
            light_ambient: t.ambient_light,
            ultrawarm: t.water_evaporates(),
            day_cycle: sky == SkyKind::Overworld && !t.has_fixed_time,
            spawn_light: Some(match &t.monster_spawn_light_level {
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
            }),
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
        // A type the game ships (the Overworld's, the Nether's, the End's)
        // keeps that environment, with the prepared bundle's visuals; any
        // other type is read from its own data.
        let env = match type_name.as_str() {
            "minecraft:overworld" | "minecraft:overworld_caves" => LevelEnv::Vanilla(Dim::Overworld),
            "minecraft:the_nether" => LevelEnv::Vanilla(Dim::Nether),
            "minecraft:the_end" => LevelEnv::Vanilla(Dim::End),
            _ => match Environment::load_type(
                &self.options.resources,
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
        let rule = |id| facts(id).spawn_light.unwrap();
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
}
