//! Typed dimension types and level stems (26.3 data shape), plus the registry
//! later work fills from data packs, mods and saves. No client dependencies.
//!
//! Since 26.x many former dimension-type fields (`ultrawarm`, `natural`,
//! `piglin_safe`, `has_raids`, `bed_works`, `respawn_anchor_works`, `effects`
//! and the sky, fog and cloud values) are *environment attributes* in the
//! `attributes` map. This model keeps that map verbatim and offers typed
//! accessors for the gameplay ones the engine reads. The pre-26 flat fields
//! `fixed_time`, `ultrawarm`, `piglin_safe`, `has_raids` and
//! `respawn_anchor_works` are still accepted when parsing and folded into the
//! new fields.
use crate::worldgen::biome_source::{BiomeSourceCatalog, BiomeSourceDefinition};
use crate::worldgen::overrides::LevelHeight;
use serde_json::{Map, Number, Value, json};
use std::collections::BTreeMap;

pub const OVERWORLD: &str = "minecraft:overworld";
pub const OVERWORLD_CAVES: &str = "minecraft:overworld_caves";
pub const THE_NETHER: &str = "minecraft:the_nether";
pub const THE_END: &str = "minecraft:the_end";

/// `DimensionType.MIN_Y` and `MAX_Y`.
const MIN_Y_LIMIT: i32 = -2032;
const MAX_Y_LIMIT: i32 = 2032;
const MIN_SCALE: f64 = 1.0e-5;
const MAX_SCALE: f64 = 3.0e7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DimensionError {
    /// The value is not a JSON object (`what` names it).
    NotObject(String),
    MissingField(&'static str),
    /// A field has the wrong JSON type or an out-of-range value.
    Invalid {
        field: &'static str,
        reason: String,
    },
    UnknownType(String),
    UnknownGeneratorType(String),
    Biome(String),
    Json(String),
}
impl std::fmt::Display for DimensionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotObject(what) => write!(f, "{what} must be a JSON object"),
            Self::MissingField(field) => write!(f, "missing field `{field}`"),
            Self::Invalid { field, reason } => write!(f, "field `{field}`: {reason}"),
            Self::UnknownType(id) => write!(f, "unknown dimension type `{id}`"),
            Self::UnknownGeneratorType(id) => write!(f, "unsupported generator type `{id}`"),
            Self::Biome(e) => write!(f, "biome source: {e}"),
            Self::Json(e) => write!(f, "invalid JSON: {e}"),
        }
    }
}
impl std::error::Error for DimensionError {}
fn invalid(field: &'static str, reason: impl Into<String>) -> DimensionError {
    DimensionError::Invalid {
        field,
        reason: reason.into(),
    }
}

/// `monster_spawn_light_level`: an int provider. Constant and uniform are
/// modelled; any other provider type is kept verbatim.
#[derive(Clone, Debug, PartialEq)]
pub enum IntProvider {
    Constant(i32),
    Uniform { min: i32, max: i32 },
    Other(Value),
}
impl IntProvider {
    fn parse(field: &'static str, v: &Value) -> Result<Self, DimensionError> {
        let int = |v: &Value| {
            v.as_i64()
                .and_then(|n| i32::try_from(n).ok())
                .ok_or_else(|| invalid(field, "expected an integer"))
        };
        let light = |n: i32| {
            if (0..=15).contains(&n) {
                Ok(n)
            } else {
                Err(invalid(field, format!("{n} is outside 0..=15")))
            }
        };
        if v.is_number() {
            return Ok(Self::Constant(light(int(v)?)?));
        }
        let o = v
            .as_object()
            .ok_or_else(|| invalid(field, "expected an integer or an int provider"))?;
        let kind = o.get("type").and_then(Value::as_str).unwrap_or("");
        match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "constant" => {
                let value = o.get("value").ok_or_else(|| invalid(field, "missing value"))?;
                Ok(Self::Constant(light(int(value)?)?))
            }
            "uniform" => {
                let get = |k: &str| {
                    o.get(k)
                        .ok_or_else(|| invalid(field, format!("missing {k}")))
                };
                let min = light(int(get("min_inclusive")?)?)?;
                let max = light(int(get("max_inclusive")?)?)?;
                if min > max {
                    return Err(invalid(field, "min_inclusive is above max_inclusive"));
                }
                Ok(Self::Uniform { min, max })
            }
            _ => Ok(Self::Other(v.clone())),
        }
    }
    fn to_value(&self) -> Value {
        match self {
            Self::Constant(n) => json!(n),
            Self::Uniform { min, max } => {
                json!({"type": "minecraft:uniform", "min_inclusive": min, "max_inclusive": max})
            }
            Self::Other(v) => v.clone(),
        }
    }
}

/// `timelines`: a tag (`#minecraft:in_overworld`) or a list of timeline ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Timelines {
    Tag(String),
    List(Vec<String>),
}

/// A dimension type. Defaults match what the engine already assumed for
/// fields a pack leaves out (`LevelHeight::of_json`).
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionType {
    pub min_y: i32,
    pub height: i32,
    pub logical_height: i32,
    pub has_skylight: bool,
    pub has_ceiling: bool,
    pub has_ender_dragon_fight: bool,
    /// The day time stays fixed (the End and the Nether).
    pub has_fixed_time: bool,
    pub coordinate_scale: f64,
    /// The light every block gets, whatever its level.
    pub ambient_light: f32,
    /// A block tag id with a leading `#`.
    pub infiniburn: String,
    pub monster_spawn_light_level: IntProvider,
    pub monster_spawn_block_light_limit: i32,
    /// The `minecraft:world_clock` this level's day time follows.
    pub default_clock: Option<String>,
    pub timelines: Option<Timelines>,
    /// `overworld`, `end` or `none`.
    pub skybox: String,
    /// `default` or `nether`.
    pub cardinal_light: String,
    /// Environment attributes by namespaced key (`minecraft:gameplay/...`).
    pub attributes: BTreeMap<String, Value>,
}

impl DimensionType {
    /// An open-sky type of the given range with every other field at its
    /// default.
    pub fn new(min_y: i32, height: i32) -> Self {
        Self {
            min_y,
            height,
            logical_height: height,
            has_skylight: true,
            has_ceiling: false,
            has_ender_dragon_fight: false,
            has_fixed_time: false,
            coordinate_scale: 1.0,
            ambient_light: 0.0,
            infiniburn: "#minecraft:infiniburn_overworld".into(),
            monster_spawn_light_level: IntProvider::Uniform { min: 0, max: 7 },
            monster_spawn_block_light_limit: 0,
            default_clock: None,
            timelines: None,
            skybox: "overworld".into(),
            cardinal_light: "default".into(),
            attributes: BTreeMap::new(),
        }
    }

    /// The four vanilla types as the engine uses them today: build range,
    /// sky and ceiling, ambient light, clock, scale, spawning and the
    /// gameplay attributes. Sky, fog, cloud and audio attributes are not
    /// built in; they come from the prepared bundle
    /// ([`DimensionTypeRegistry::load`]), which layers over these.
    pub fn vanilla(id: &str) -> Option<Self> {
        let bed = |set: &str, sleep: &str| json!({"can_set_spawn": set, "can_sleep": sleep});
        let t = match id {
            OVERWORLD | OVERWORLD_CAVES => {
                let mut t = Self::new(-64, 384);
                t.has_ceiling = id == OVERWORLD_CAVES;
                t.default_clock = Some(OVERWORLD.into());
                t.timelines = Some(Timelines::Tag("#minecraft:in_overworld".into()));
                t.set_attr("gameplay/nether_portal_spawns_piglin", json!(true));
                t.set_attr("gameplay/respawn_anchor_works", json!(false));
                t.set_attr("gameplay/bed_rule", bed("always", "when_dark"));
                t
            }
            THE_NETHER => {
                let mut t = Self::new(0, 256);
                t.logical_height = 128;
                t.has_skylight = false;
                t.has_ceiling = true;
                t.has_fixed_time = true;
                t.coordinate_scale = 8.0;
                t.ambient_light = 0.1;
                t.infiniburn = "#minecraft:infiniburn_nether".into();
                t.monster_spawn_light_level = IntProvider::Constant(7);
                t.monster_spawn_block_light_limit = 15;
                t.timelines = Some(Timelines::Tag("#minecraft:in_nether".into()));
                t.skybox = "none".into();
                t.cardinal_light = "nether".into();
                t.set_attr("gameplay/can_start_raid", json!(false));
                t.set_attr("gameplay/fast_lava", json!(true));
                t.set_attr("gameplay/piglins_zombify", json!(false));
                t.set_attr("gameplay/respawn_anchor_works", json!(true));
                t.set_attr("gameplay/snow_golem_melts", json!(true));
                t.set_attr("gameplay/water_evaporates", json!(true));
                t.set_attr("gameplay/bed_rule", bed("never", "never"));
                t
            }
            THE_END => {
                let mut t = Self::new(0, 256);
                t.has_ender_dragon_fight = true;
                t.has_fixed_time = true;
                t.ambient_light = 0.25;
                t.infiniburn = "#minecraft:infiniburn_end".into();
                t.monster_spawn_light_level = IntProvider::Constant(15);
                t.default_clock = Some(THE_END.into());
                t.timelines = Some(Timelines::Tag("#minecraft:in_end".into()));
                t.skybox = "end".into();
                t.set_attr("gameplay/respawn_anchor_works", json!(false));
                t.set_attr("gameplay/bed_rule", bed("never", "never"));
                t
            }
            _ => return None,
        };
        debug_assert!(t.validate().is_ok(), "built-in types are valid");
        Some(t)
    }

    fn set_attr(&mut self, key: &str, value: Value) {
        self.attributes.insert(format!("minecraft:{key}"), value);
    }
    fn attr_bool(&self, key: &str, default: bool) -> bool {
        self.attributes
            .get(&format!("minecraft:gameplay/{key}"))
            .and_then(Value::as_bool)
            .unwrap_or(default)
    }
    /// Lava flows fast (the old `ultrawarm`).
    pub fn fast_lava(&self) -> bool {
        self.attr_bool("fast_lava", false)
    }
    /// Water cannot be placed (the old `ultrawarm`).
    pub fn water_evaporates(&self) -> bool {
        self.attr_bool("water_evaporates", false)
    }
    /// The old `!piglin_safe`.
    pub fn piglins_zombify(&self) -> bool {
        self.attr_bool("piglins_zombify", true)
    }
    /// The old `has_raids`.
    pub fn can_start_raid(&self) -> bool {
        self.attr_bool("can_start_raid", true)
    }
    pub fn respawn_anchor_works(&self) -> bool {
        self.attr_bool("respawn_anchor_works", false)
    }
    pub fn snow_golem_melts(&self) -> bool {
        self.attr_bool("snow_golem_melts", false)
    }
    pub fn nether_portal_spawns_piglin(&self) -> bool {
        self.attr_bool("nether_portal_spawns_piglin", false)
    }

    /// The cheap build-range view the generator and loader use.
    pub fn level_height(&self) -> LevelHeight {
        LevelHeight {
            min_y: self.min_y,
            height: self.height,
            logical_height: self.logical_height,
            has_skylight: self.has_skylight,
            has_ceiling: self.has_ceiling,
        }
    }

    /// The `DimensionType` codec's ranges: `min_y` and `height` multiples of
    /// 16, height at least 16, `-2032 <= min_y` and `min_y + height <= 2032`,
    /// logical height within the height, scale in `1e-5..=3e7`, light limit
    /// 0..=15, ambient light finite.
    pub fn validate(&self) -> Result<(), DimensionError> {
        if self.min_y % 16 != 0 {
            return Err(invalid("min_y", "must be a multiple of 16"));
        }
        if self.min_y < MIN_Y_LIMIT {
            return Err(invalid("min_y", format!("below {MIN_Y_LIMIT}")));
        }
        if self.height % 16 != 0 || self.height < 16 {
            return Err(invalid("height", "must be a multiple of 16, at least 16"));
        }
        if i64::from(self.min_y) + i64::from(self.height) > i64::from(MAX_Y_LIMIT) {
            return Err(invalid(
                "height",
                format!("min_y + height is above {MAX_Y_LIMIT}"),
            ));
        }
        if !(0..=self.height).contains(&self.logical_height) {
            return Err(invalid("logical_height", "must be within 0..=height"));
        }
        if !self.coordinate_scale.is_finite()
            || !(MIN_SCALE..=MAX_SCALE).contains(&self.coordinate_scale)
        {
            return Err(invalid("coordinate_scale", "must be within 1e-5..=3e7"));
        }
        if !self.ambient_light.is_finite() {
            return Err(invalid("ambient_light", "must be finite"));
        }
        if !(0..=15).contains(&self.monster_spawn_block_light_limit) {
            return Err(invalid("monster_spawn_block_light_limit", "must be 0..=15"));
        }
        if !self.infiniburn.starts_with('#') {
            return Err(invalid("infiniburn", "must be a block tag (leading `#`)"));
        }
        Ok(())
    }

    pub fn from_json_str(json: &str) -> Result<Self, DimensionError> {
        let v: Value =
            serde_json::from_str(json).map_err(|e| DimensionError::Json(e.to_string()))?;
        Self::from_value(&v)
    }

    /// Parse and validate. `min_y` and `height` are required; every other
    /// field defaults as in [`DimensionType::new`]. The pre-26 flat fields
    /// are folded into the new fields and attributes unless the new form is
    /// also present. Unknown fields are ignored.
    pub fn from_value(v: &Value) -> Result<Self, DimensionError> {
        let o = v
            .as_object()
            .ok_or_else(|| DimensionError::NotObject("dimension type".into()))?;
        let present = |field: &str| o.get(field).filter(|v| !v.is_null());
        let int = |field: &'static str| -> Result<Option<i32>, DimensionError> {
            present(field)
                .map(|n| {
                    n.as_i64()
                        .and_then(|n| i32::try_from(n).ok())
                        .ok_or_else(|| invalid(field, "expected an integer"))
                })
                .transpose()
        };
        let boolean = |field: &'static str| -> Result<Option<bool>, DimensionError> {
            present(field)
                .map(|b| {
                    b.as_bool()
                        .ok_or_else(|| invalid(field, "expected a boolean"))
                })
                .transpose()
        };
        let float = |field: &'static str| -> Result<Option<f64>, DimensionError> {
            present(field)
                .map(|n| {
                    n.as_f64()
                        .ok_or_else(|| invalid(field, "expected a number"))
                })
                .transpose()
        };
        let string = |field: &'static str| -> Result<Option<String>, DimensionError> {
            present(field)
                .map(|s| match s {
                    // `default_clock` may also be written as {"id": ...}.
                    Value::Object(c) if field == "default_clock" => c
                        .get("id")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .ok_or_else(|| invalid(field, "expected a string")),
                    s => s
                        .as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| invalid(field, "expected a string")),
                })
                .transpose()
        };
        let min_y = int("min_y")?.ok_or(DimensionError::MissingField("min_y"))?;
        let height = int("height")?.ok_or(DimensionError::MissingField("height"))?;
        let mut t = Self::new(min_y, height);
        t.logical_height = int("logical_height")?.unwrap_or(height);
        t.has_skylight = boolean("has_skylight")?.unwrap_or(true);
        t.has_ceiling = boolean("has_ceiling")?.unwrap_or(false);
        t.has_ender_dragon_fight = boolean("has_ender_dragon_fight")?.unwrap_or(false);
        t.has_fixed_time = match boolean("has_fixed_time")? {
            Some(b) => b,
            // Pre-26 `fixed_time` was a tick count; its presence means fixed.
            None => present("fixed_time").is_some(),
        };
        t.coordinate_scale = float("coordinate_scale")?.unwrap_or(1.0);
        t.ambient_light = float("ambient_light")?.unwrap_or(0.0) as f32;
        if let Some(s) = string("infiniburn")? {
            t.infiniburn = s;
        }
        if let Some(p) = present("monster_spawn_light_level") {
            t.monster_spawn_light_level = IntProvider::parse("monster_spawn_light_level", p)?;
        }
        t.monster_spawn_block_light_limit = int("monster_spawn_block_light_limit")?.unwrap_or(0);
        t.default_clock = string("default_clock")?;
        t.timelines = match present("timelines") {
            None => None,
            Some(Value::String(s)) => Some(Timelines::Tag(s.clone())),
            Some(Value::Array(a)) => Some(Timelines::List(
                a.iter()
                    .map(|e| {
                        e.as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| invalid("timelines", "expected timeline ids"))
                    })
                    .collect::<Result<_, _>>()?,
            )),
            Some(_) => return Err(invalid("timelines", "expected a tag or a list")),
        };
        if let Some(s) = string("skybox")? {
            t.skybox = s;
        }
        if let Some(s) = string("cardinal_light")? {
            t.cardinal_light = s;
        }
        match present("attributes") {
            None => {}
            Some(Value::Object(a)) => {
                t.attributes = a.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            }
            Some(_) => return Err(invalid("attributes", "expected an object")),
        }
        // Pre-26 flat fields, only where the attribute is not already there.
        let legacy = |t: &mut Self, key: &str, value: Value| {
            t.attributes
                .entry(format!("minecraft:gameplay/{key}"))
                .or_insert(value);
        };
        if let Some(b) = boolean("ultrawarm")? {
            legacy(&mut t, "fast_lava", json!(b));
            legacy(&mut t, "water_evaporates", json!(b));
        }
        if let Some(b) = boolean("piglin_safe")? {
            legacy(&mut t, "piglins_zombify", json!(!b));
        }
        if let Some(b) = boolean("has_raids")? {
            legacy(&mut t, "can_start_raid", json!(b));
        }
        if let Some(b) = boolean("respawn_anchor_works")? {
            legacy(&mut t, "respawn_anchor_works", json!(b));
        }
        t.validate()?;
        Ok(t)
    }

    /// The 26.3 JSON shape. `from_value(to_value())` is the identity.
    pub fn to_value(&self) -> Value {
        let number = |f: f64| Number::from_f64(f).map_or(Value::Null, Value::Number);
        let mut o = Map::new();
        o.insert("min_y".into(), json!(self.min_y));
        o.insert("height".into(), json!(self.height));
        o.insert("logical_height".into(), json!(self.logical_height));
        o.insert("has_skylight".into(), json!(self.has_skylight));
        o.insert("has_ceiling".into(), json!(self.has_ceiling));
        o.insert(
            "has_ender_dragon_fight".into(),
            json!(self.has_ender_dragon_fight),
        );
        o.insert("has_fixed_time".into(), json!(self.has_fixed_time));
        o.insert("coordinate_scale".into(), number(self.coordinate_scale));
        o.insert("ambient_light".into(), number(f64::from(self.ambient_light)));
        o.insert("infiniburn".into(), json!(self.infiniburn));
        o.insert(
            "monster_spawn_light_level".into(),
            self.monster_spawn_light_level.to_value(),
        );
        o.insert(
            "monster_spawn_block_light_limit".into(),
            json!(self.monster_spawn_block_light_limit),
        );
        if let Some(c) = &self.default_clock {
            o.insert("default_clock".into(), json!(c));
        }
        match &self.timelines {
            Some(Timelines::Tag(t)) => {
                o.insert("timelines".into(), json!(t));
            }
            Some(Timelines::List(l)) => {
                o.insert("timelines".into(), json!(l));
            }
            None => {}
        }
        o.insert("skybox".into(), json!(self.skybox));
        o.insert("cardinal_light".into(), json!(self.cardinal_light));
        o.insert(
            "attributes".into(),
            Value::Object(
                self.attributes
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect(),
            ),
        );
        Value::Object(o)
    }
}

/// A level stem's `type`: a registered id or an inline dimension type.
#[derive(Clone, Debug, PartialEq)]
pub enum TypeRef {
    Id(String),
    Inline(Box<DimensionType>),
}

/// Noise settings by id or inline JSON (decoded by the generator).
#[derive(Clone, Debug, PartialEq)]
pub enum SettingsRef {
    Id(String),
    Inline(Value),
}
impl SettingsRef {
    fn parse(v: &Value) -> Result<Self, DimensionError> {
        match v {
            Value::String(s) => Ok(Self::Id(s.clone())),
            Value::Object(_) => Ok(Self::Inline(v.clone())),
            _ => Err(invalid("settings", "expected an id or an object")),
        }
    }
    fn to_value(&self) -> Value {
        match self {
            Self::Id(s) => json!(s),
            Self::Inline(v) => v.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GeneratorSpec {
    Noise {
        settings: SettingsRef,
        biome_source: BiomeSourceDefinition,
    },
    /// Superflat; the settings object is kept as written (flatgen reads it).
    Flat(Value),
    Debug,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LevelStem {
    pub dimension_type: TypeRef,
    pub generator: GeneratorSpec,
}
impl LevelStem {
    pub fn from_json_str(
        json: &str,
        catalog: &BiomeSourceCatalog,
    ) -> Result<Self, DimensionError> {
        let v: Value =
            serde_json::from_str(json).map_err(|e| DimensionError::Json(e.to_string()))?;
        Self::from_value(&v, catalog)
    }
    pub fn from_value(v: &Value, catalog: &BiomeSourceCatalog) -> Result<Self, DimensionError> {
        let o = v
            .as_object()
            .ok_or_else(|| DimensionError::NotObject("level stem".into()))?;
        let dimension_type = match o.get("type") {
            Some(Value::String(id)) => TypeRef::Id(id.clone()),
            Some(inline @ Value::Object(_)) => {
                TypeRef::Inline(Box::new(DimensionType::from_value(inline)?))
            }
            Some(_) => return Err(invalid("type", "expected a dimension type id or object")),
            None => return Err(DimensionError::MissingField("type")),
        };
        let g = o
            .get("generator")
            .ok_or(DimensionError::MissingField("generator"))?
            .as_object()
            .ok_or_else(|| DimensionError::NotObject("generator".into()))?;
        let kind = g
            .get("type")
            .and_then(Value::as_str)
            .ok_or(DimensionError::MissingField("generator.type"))?;
        let generator = match kind {
            "minecraft:noise" => {
                let settings = g
                    .get("settings")
                    .ok_or(DimensionError::MissingField("settings"))?;
                let source = g
                    .get("biome_source")
                    .ok_or(DimensionError::MissingField("biome_source"))?;
                GeneratorSpec::Noise {
                    settings: SettingsRef::parse(settings)?,
                    biome_source: catalog
                        .decode(&source.to_string())
                        .map_err(|e| DimensionError::Biome(e.to_string()))?,
                }
            }
            "minecraft:flat" => match g.get("settings") {
                Some(s @ Value::Object(_)) => GeneratorSpec::Flat(s.clone()),
                Some(_) => return Err(invalid("settings", "expected an object")),
                None => return Err(DimensionError::MissingField("settings")),
            },
            "minecraft:debug" => GeneratorSpec::Debug,
            other => return Err(DimensionError::UnknownGeneratorType(other.to_owned())),
        };
        Ok(Self {
            dimension_type,
            generator,
        })
    }
    pub fn to_value(&self, catalog: &BiomeSourceCatalog) -> Result<Value, DimensionError> {
        let ty = match &self.dimension_type {
            TypeRef::Id(id) => json!(id),
            TypeRef::Inline(t) => t.to_value(),
        };
        let generator = match &self.generator {
            GeneratorSpec::Noise {
                settings,
                biome_source,
            } => {
                let source = catalog
                    .encode(biome_source)
                    .map_err(|e| DimensionError::Biome(e.to_string()))?;
                json!({
                    "type": "minecraft:noise",
                    "settings": settings.to_value(),
                    "biome_source": serde_json::from_str::<Value>(&source)
                        .map_err(|e| DimensionError::Json(e.to_string()))?,
                })
            }
            GeneratorSpec::Flat(s) => json!({"type": "minecraft:flat", "settings": s}),
            GeneratorSpec::Debug => json!({"type": "minecraft:debug"}),
        };
        Ok(json!({"type": ty, "generator": generator}))
    }
}

/// Dimension types by id, with the vanilla ones built in.
#[derive(Clone, Debug, PartialEq)]
pub struct DimensionTypeRegistry {
    types: BTreeMap<String, DimensionType>,
}
impl Default for DimensionTypeRegistry {
    fn default() -> Self {
        Self::vanilla()
    }
}
impl DimensionTypeRegistry {
    pub fn vanilla() -> Self {
        let types = [OVERWORLD, OVERWORLD_CAVES, THE_NETHER, THE_END]
            .into_iter()
            .map(|id| (id.to_owned(), DimensionType::vanilla(id).expect("vanilla")))
            .collect();
        Self { types }
    }
    /// The vanilla types with a bundle's entries (`id -> JSON text`: the
    /// prepared data layered with packs, last wins) over them. An entry that
    /// does not validate is left out and reported; a vanilla id keeps its
    /// built-in value then.
    pub fn load(bundle: &BTreeMap<String, String>) -> (Self, Vec<(String, DimensionError)>) {
        let mut reg = Self::vanilla();
        let mut errors = Vec::new();
        for (id, json) in bundle {
            match DimensionType::from_json_str(json) {
                Ok(t) => {
                    reg.types.insert(id.clone(), t);
                }
                Err(e) => errors.push((id.clone(), e)),
            }
        }
        (reg, errors)
    }
    pub fn get(&self, id: &str) -> Option<&DimensionType> {
        self.types.get(id)
    }
    pub fn insert(&mut self, id: &str, t: DimensionType) -> Result<(), DimensionError> {
        t.validate()?;
        self.types.insert(id.to_owned(), t);
        Ok(())
    }
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.types.keys().map(String::as_str)
    }
    pub fn resolve<'a>(&'a self, r: &'a TypeRef) -> Result<&'a DimensionType, DimensionError> {
        match r {
            TypeRef::Id(id) => self
                .types
                .get(id)
                .ok_or_else(|| DimensionError::UnknownType(id.clone())),
            TypeRef::Inline(t) => Ok(t),
        }
    }
}

/// The dimension types and level stems a world knows. F3-3 fills it from the
/// built-ins, data packs, mods and the save; this module only holds and
/// validates it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LevelRegistry {
    pub types: DimensionTypeRegistry,
    stems: BTreeMap<String, LevelStem>,
}
impl LevelRegistry {
    pub fn new(types: DimensionTypeRegistry) -> Self {
        Self {
            types,
            stems: BTreeMap::new(),
        }
    }
    /// Add or replace a level's stem; its type must resolve.
    pub fn insert_stem(&mut self, id: &str, stem: LevelStem) -> Result<(), DimensionError> {
        self.types.resolve(&stem.dimension_type)?;
        self.stems.insert(id.to_owned(), stem);
        Ok(())
    }
    pub fn remove_stem(&mut self, id: &str) -> Option<LevelStem> {
        self.stems.remove(id)
    }
    pub fn stem(&self, id: &str) -> Option<&LevelStem> {
        self.stems.get(id)
    }
    pub fn stem_ids(&self) -> impl Iterator<Item = &str> {
        self.stems.keys().map(String::as_str)
    }
    /// The level's dimension type.
    pub fn level_type(&self, id: &str) -> Option<&DimensionType> {
        self.types.resolve(&self.stems.get(id)?.dimension_type).ok()
    }
    pub fn level_height(&self, id: &str) -> Option<LevelHeight> {
        self.level_type(id).map(DimensionType::level_height)
    }
    /// Read every `dimensions` entry of a world preset JSON (the vanilla
    /// stems) into the registry. A bad stem is skipped and reported.
    pub fn load_preset_stems(
        &mut self,
        preset_json: &str,
        catalog: &BiomeSourceCatalog,
    ) -> Vec<(String, DimensionError)> {
        let v: Value = match serde_json::from_str(preset_json) {
            Ok(v) => v,
            Err(e) => return vec![(String::new(), DimensionError::Json(e.to_string()))],
        };
        let mut errors = Vec::new();
        for (id, stem) in v["dimensions"].as_object().into_iter().flatten() {
            let result =
                LevelStem::from_value(stem, catalog).and_then(|s| self.insert_stem(id, s));
            if let Err(e) = result {
                errors.push((id.clone(), e));
            }
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn catalog() -> BiomeSourceCatalog {
        let biomes: BTreeSet<String> = ["minecraft:plains", "minecraft:desert"]
            .into_iter()
            .map(String::from)
            .collect();
        BiomeSourceCatalog::new(&biomes, &BTreeMap::new(), &BTreeSet::new()).unwrap()
    }

    /// The values `LevelHeight::vanilla` and the client's vanilla dimension
    /// table hold today: (min_y, height, logical, skylight, ceiling,
    /// ambient, clock, scale).
    #[test]
    fn vanilla_matches_todays_engine_values() {
        let rows = [
            (OVERWORLD, (-64, 384, 384, true, false), 0.0, Some(OVERWORLD), 1.0),
            (OVERWORLD_CAVES, (-64, 384, 384, true, true), 0.0, Some(OVERWORLD), 1.0),
            (THE_NETHER, (0, 256, 128, false, true), 0.1, None, 8.0),
            (THE_END, (0, 256, 256, true, false), 0.25, Some(THE_END), 1.0),
        ];
        for (id, (min_y, height, logical, sky, ceil), ambient, clock, scale) in rows {
            let t = DimensionType::vanilla(id).unwrap();
            assert_eq!(
                (t.min_y, t.height, t.logical_height, t.has_skylight, t.has_ceiling),
                (min_y, height, logical, sky, ceil),
                "{id}"
            );
            assert_eq!(t.ambient_light, ambient, "{id}");
            // The client reads the Nether's clock as absent.
            assert_eq!(t.default_clock.as_deref().filter(|_| id != THE_NETHER), clock);
            assert_eq!(t.coordinate_scale, scale, "{id}");
            assert_eq!(LevelHeight::vanilla(id), Some(t.level_height()), "{id}");
        }
        assert!(DimensionType::vanilla("minecraft:nope").is_none());
    }
    #[test]
    fn vanilla_gameplay_flags() {
        let (o, n, e) = (
            DimensionType::vanilla(OVERWORLD).unwrap(),
            DimensionType::vanilla(THE_NETHER).unwrap(),
            DimensionType::vanilla(THE_END).unwrap(),
        );
        assert!(n.fast_lava() && n.water_evaporates() && n.respawn_anchor_works());
        assert!(!n.piglins_zombify() && !n.can_start_raid() && n.snow_golem_melts());
        assert!(!o.fast_lava() && o.piglins_zombify() && o.can_start_raid());
        assert!(o.nether_portal_spawns_piglin() && !e.nether_portal_spawns_piglin());
        assert!(e.has_ender_dragon_fight && e.has_fixed_time && n.has_fixed_time);
        assert!(!o.has_fixed_time);
    }
    #[test]
    fn built_ins_round_trip() {
        for id in [OVERWORLD, OVERWORLD_CAVES, THE_NETHER, THE_END] {
            let t = DimensionType::vanilla(id).unwrap();
            let json = serde_json::to_string(&t.to_value()).unwrap();
            assert_eq!(DimensionType::from_json_str(&json).unwrap(), t, "{id}");
        }
    }
    #[test]
    fn parses_prepared_26_3_shape() {
        let t = DimensionType::from_json_str(
            r##"{"ambient_light":0.1,"attributes":{"minecraft:gameplay/fast_lava":true,
            "minecraft:visual/fog_color":"#c0d8ff"},"cardinal_light":"nether",
            "coordinate_scale":8.0,"has_ceiling":true,"has_ender_dragon_fight":false,
            "has_fixed_time":true,"has_skylight":false,"height":256,
            "infiniburn":"#minecraft:infiniburn_nether","logical_height":128,"min_y":0,
            "monster_spawn_block_light_limit":15,"monster_spawn_light_level":7,
            "skybox":"none","timelines":"#minecraft:in_nether"}"##,
        )
        .unwrap();
        assert_eq!(t.coordinate_scale, 8.0);
        assert_eq!(t.monster_spawn_light_level, IntProvider::Constant(7));
        assert!(t.fast_lava());
        assert_eq!(
            t.attributes["minecraft:visual/fog_color"],
            json!("#c0d8ff")
        );
        let uniform = DimensionType::from_json_str(
            r#"{"min_y":-64,"height":384,"monster_spawn_light_level":
            {"type":"minecraft:uniform","min_inclusive":0,"max_inclusive":7},
            "default_clock":{"id":"minecraft:overworld"}}"#,
        )
        .unwrap();
        assert_eq!(uniform.monster_spawn_light_level, IntProvider::Uniform { min: 0, max: 7 });
        assert_eq!(uniform.default_clock.as_deref(), Some(OVERWORLD));
    }
    #[test]
    fn defaults_for_a_minimal_type() {
        let t = DimensionType::from_json_str(r#"{"min_y":0,"height":64}"#).unwrap();
        assert_eq!(t, DimensionType::new(0, 64));
        assert_eq!(t.logical_height, 64);
        assert!(t.has_skylight && !t.has_ceiling);
        assert!(t.piglins_zombify() && t.can_start_raid() && !t.fast_lava());
    }
    #[test]
    fn legacy_flat_fields_fold_into_attributes() {
        let t = DimensionType::from_json_str(
            r#"{"min_y":0,"height":256,"ultrawarm":true,"piglin_safe":false,
            "has_raids":false,"respawn_anchor_works":true,"fixed_time":18000}"#,
        )
        .unwrap();
        assert!(t.fast_lava() && t.water_evaporates() && t.piglins_zombify());
        assert!(!t.can_start_raid() && t.respawn_anchor_works() && t.has_fixed_time);
        // The new attribute wins over the old field.
        let t = DimensionType::from_json_str(
            r#"{"min_y":0,"height":256,"ultrawarm":true,
            "attributes":{"minecraft:gameplay/fast_lava":false}}"#,
        )
        .unwrap();
        assert!(!t.fast_lava() && t.water_evaporates());
        // And the folded form round-trips.
        assert_eq!(DimensionType::from_value(&t.to_value()).unwrap(), t);
    }
    #[test]
    fn rejects_bad_types_with_clear_errors() {
        let err = |json: &str| DimensionType::from_json_str(json).unwrap_err().to_string();
        assert_eq!(err(r#"{"height":64}"#), "missing field `min_y`");
        assert_eq!(err(r#"{"min_y":0}"#), "missing field `height`");
        assert!(err(r#"{"min_y":8,"height":64}"#).contains("`min_y`"));
        assert!(err(r#"{"min_y":-2048,"height":64}"#).contains("`min_y`"));
        assert!(err(r#"{"min_y":0,"height":8}"#).contains("`height`"));
        assert!(err(r#"{"min_y":0,"height":50}"#).contains("`height`"));
        assert!(err(r#"{"min_y":2000,"height":64}"#).contains("`height`"));
        assert!(err(r#"{"min_y":0,"height":64,"logical_height":80}"#).contains("`logical_height`"));
        assert!(err(r#"{"min_y":0,"height":64,"coordinate_scale":0}"#).contains("`coordinate_scale`"));
        assert!(err(r#"{"min_y":0,"height":64,"coordinate_scale":1e9}"#).contains("`coordinate_scale`"));
        assert!(err(r#"{"min_y":0,"height":64,"monster_spawn_block_light_limit":16}"#)
            .contains("monster_spawn_block_light_limit"));
        assert!(err(r#"{"min_y":0,"height":64,"monster_spawn_light_level":16}"#)
            .contains("monster_spawn_light_level"));
        assert!(err(r#"{"min_y":0,"height":64,"monster_spawn_light_level":
            {"type":"minecraft:uniform","min_inclusive":5,"max_inclusive":1}}"#)
            .contains("above"));
        assert!(err(r#"{"min_y":0,"height":64,"has_skylight":"yes"}"#).contains("`has_skylight`"));
        assert!(err(r#"{"min_y":0,"height":64,"infiniburn":"minecraft:x"}"#).contains("`infiniburn`"));
        assert!(err("[]").contains("JSON object"));
        assert!(err("{").contains("invalid JSON"));
    }
    #[test]
    fn height_limits_accepted() {
        assert!(DimensionType::from_json_str(r#"{"min_y":-2032,"height":4064}"#).is_ok());
        assert!(DimensionType::from_json_str(r#"{"min_y":-2032,"height":4080}"#).is_err());
        assert!(DimensionType::from_json_str(r#"{"min_y":0,"height":16,"logical_height":0}"#).is_ok());
    }

    #[test]
    fn registry_layers_a_bundle_over_vanilla() {
        let bundle: BTreeMap<String, String> = [
            ("mypack:moon".to_owned(), r#"{"min_y":0,"height":128,"has_skylight":false}"#.to_owned()),
            (OVERWORLD.to_owned(), r#"{"min_y":-64,"height":2096}"#.to_owned()),
            (THE_END.to_owned(), r#"{"min_y":1,"height":256}"#.to_owned()),
        ]
        .into();
        let (reg, errors) = DimensionTypeRegistry::load(&bundle);
        assert_eq!(reg.get("mypack:moon").unwrap().height, 128);
        assert_eq!(reg.get(OVERWORLD).unwrap().height, 2096);
        // The bad End override is reported and the built-in kept.
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, THE_END);
        assert_eq!(reg.get(THE_END), DimensionType::vanilla(THE_END).as_ref());
        assert_eq!(reg.ids().count(), 5);
        let (plain, none) = DimensionTypeRegistry::load(&BTreeMap::new());
        assert!(none.is_empty());
        assert_eq!(plain, DimensionTypeRegistry::vanilla());
    }
    #[test]
    fn stems_parse_and_round_trip() {
        let cat = catalog();
        let noise = r#"{"type":"minecraft:overworld","generator":{"type":"minecraft:noise",
            "settings":"minecraft:overworld",
            "biome_source":{"type":"minecraft:fixed","biome":"minecraft:plains"}}}"#;
        let flat = r#"{"type":{"min_y":0,"height":64},"generator":{"type":"minecraft:flat",
            "settings":{"biome":"minecraft:plains","layers":[]}}}"#;
        let debug = r#"{"type":"minecraft:the_end","generator":{"type":"minecraft:debug"}}"#;
        let grid = r#"{"type":"mypack:moon","generator":{"type":"minecraft:noise",
            "settings":{"noise":{"min_y":0,"height":64}},
            "biome_source":{"type":"yoake:grid","cell_size":16,"width":1,"depth":1,
            "cells":["minecraft:desert"],"outside":"minecraft:plains","seed":7}}}"#;
        for json in [noise, flat, debug, grid] {
            let stem = LevelStem::from_json_str(json, &cat).unwrap();
            let again = LevelStem::from_value(&stem.to_value(&cat).unwrap(), &cat).unwrap();
            assert_eq!(again, stem);
        }
        let stem = LevelStem::from_json_str(grid, &cat).unwrap();
        assert!(matches!(
            stem.generator,
            GeneratorSpec::Noise { biome_source: BiomeSourceDefinition::Grid { seed: 7, .. }, .. }
        ));
        let stem = LevelStem::from_json_str(flat, &cat).unwrap();
        assert!(matches!(&stem.dimension_type, TypeRef::Inline(t) if t.height == 64));
    }
    #[test]
    fn stem_errors() {
        let cat = catalog();
        let err = |json: &str| LevelStem::from_json_str(json, &cat).unwrap_err().to_string();
        assert_eq!(err(r#"{"generator":{"type":"minecraft:debug"}}"#), "missing field `type`");
        assert_eq!(err(r#"{"type":"a:b"}"#), "missing field `generator`");
        assert!(err(r#"{"type":"a:b","generator":{"type":"mod:custom"}}"#).contains("mod:custom"));
        assert!(err(r#"{"type":"a:b","generator":{"type":"minecraft:noise","settings":"a:s",
            "biome_source":{"type":"minecraft:fixed","biome":"minecraft:nope"}}}"#)
            .contains("biome source"));
        assert!(err(r#"{"type":{"min_y":3,"height":64},"generator":{"type":"minecraft:debug"}}"#)
            .contains("`min_y`"));
        assert_eq!(err(r#"{"type":"a:b","generator":{"type":"minecraft:flat"}}"#), "missing field `settings`");
    }
    #[test]
    fn level_registry_resolves_types() {
        let cat = catalog();
        let mut reg = LevelRegistry::new(DimensionTypeRegistry::vanilla());
        let debug = |ty: &str| {
            LevelStem::from_json_str(
                &format!(r#"{{"type":"{ty}","generator":{{"type":"minecraft:debug"}}}}"#),
                &cat,
            )
            .unwrap()
        };
        reg.insert_stem("minecraft:the_nether", debug(THE_NETHER)).unwrap();
        assert_eq!(
            reg.level_height("minecraft:the_nether"),
            LevelHeight::vanilla(THE_NETHER)
        );
        assert!(reg.level_type("minecraft:the_nether").unwrap().fast_lava());
        assert_eq!(
            reg.insert_stem("mypack:moon", debug("mypack:moon")).unwrap_err(),
            DimensionError::UnknownType("mypack:moon".into())
        );
        assert!(reg.stem("mypack:moon").is_none());
        reg.types
            .insert("mypack:moon", DimensionType::new(0, 128))
            .unwrap();
        reg.insert_stem("mypack:moon", debug("mypack:moon")).unwrap();
        assert_eq!(reg.stem_ids().collect::<Vec<_>>(), ["minecraft:the_nether", "mypack:moon"]);
        assert!(reg.remove_stem("mypack:moon").is_some());
        assert!(reg.types.insert("x:bad", DimensionType::new(1, 64)).is_err());
    }
    #[test]
    fn preset_stems_load_with_errors_reported() {
        let cat = catalog();
        let mut reg = LevelRegistry::default();
        let errors = reg.load_preset_stems(
            r#"{"dimensions":{
              "minecraft:overworld":{"type":"minecraft:overworld","generator":{"type":"minecraft:debug"}},
              "x:bad":{"type":"x:missing","generator":{"type":"minecraft:debug"}}}}"#,
            &cat,
        );
        assert_eq!(reg.stem_ids().collect::<Vec<_>>(), [OVERWORLD]);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, "x:bad");
        assert_eq!(reg.load_preset_stems("nope", &cat).len(), 1);
    }
}
