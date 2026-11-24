//! The registry-entry content kinds of `data/<id>/horizon/` (REM-16): `enchantment`,
//! `damage_type`, `game_rule` and `fluid`. Each file is checked here, in the same way a
//! block or an item file is, and becomes a value of [`crate::content::Content`]; the game
//! registers it when it builds the content root (`horizon-client`'s `mod_content`).
//!
//! `enchantment` and `damage_type` files are vanilla's own data-pack JSON
//! (`data/<ns>/enchantment/*.json`, `damage_type/*.json`), kept as written so the engine reads
//! them the way it reads its own: this module only refuses what vanilla's codecs would.
use serde::Deserialize;
use serde_json::Value;

/// `<namespace>:<path>`, both of vanilla's resource-location characters.
fn id_like(id: &str, allow_slash: bool) -> bool {
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b);
    id.split_once(':').is_some_and(|(namespace, path)| {
        !namespace.is_empty()
            && !path.is_empty()
            && namespace.bytes().all(ok)
            && path.bytes().all(|b| ok(b) || (allow_slash && b == b'/'))
    })
}

/// `EquipmentSlotGroup` names an enchantment may list.
pub const ENCHANTMENT_SLOTS: [&str; 11] = [
    "any", "mainhand", "offhand", "hand", "feet", "legs", "chest", "head", "armor", "body",
    "saddle",
];

/// An enchantment the mod adds (`data/<id>/horizon/enchantment/<path>.json`): vanilla's
/// enchantment JSON, which a data pack writes in `data/<ns>/enchantment/`.
#[derive(Clone, Debug, PartialEq)]
pub struct Enchantment {
    /// `<mod id>:<path>`.
    pub id: String,
    /// The file as written (checked).
    pub json: Value,
}

const ENCHANTMENT_KEYS: [&str; 11] = [
    "description",
    "exclusive_set",
    "supported_items",
    "primary_items",
    "weight",
    "max_level",
    "min_cost",
    "max_cost",
    "anvil_cost",
    "slots",
    "effects",
];

/// A holder set: `"#ns:tag"`, `"ns:id"` or a list of ids.
fn holder_set(key: &str, v: &Value) -> Result<(), String> {
    let one = |s: &str| match s.strip_prefix('#') {
        Some(tag) => id_like(tag, true),
        None => id_like(s, true),
    };
    let ok = match v {
        Value::String(s) => one(s),
        Value::Array(list) => list
            .iter()
            .all(|e| e.as_str().is_some_and(|s| !s.starts_with('#') && one(s))),
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{key} is an id (namespace:path), a tag (#namespace:path) or a list of ids"
        ))
    }
}

fn whole(v: &Value) -> Option<i64> {
    v.as_i64()
}

fn cost(key: &str, v: &Value) -> Result<(), String> {
    let bad = || format!("{key} is {{\"base\": whole number, \"per_level_above_first\": whole number}}");
    let o = v.as_object().ok_or_else(bad)?;
    if o.len() != 2
        || !o.get("base").and_then(whole).is_some()
        || !o.get("per_level_above_first").and_then(whole).is_some()
    {
        return Err(bad());
    }
    Ok(())
}

impl Enchantment {
    /// Parses and checks one enchantment file.
    pub fn parse(id: &str, bytes: &[u8]) -> Result<Self, String> {
        let json: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let o = json.as_object().ok_or("expected a JSON object")?;
        if let Some(k) = o.keys().find(|k| !ENCHANTMENT_KEYS.contains(&k.as_str())) {
            return Err(format!(
                "unknown key {k:?} (an enchantment has {})",
                ENCHANTMENT_KEYS.join(", ")
            ));
        }
        for required in [
            "description",
            "supported_items",
            "weight",
            "max_level",
            "min_cost",
            "max_cost",
            "anvil_cost",
            "slots",
        ] {
            if !o.contains_key(required) {
                return Err(format!("{required} is missing"));
            }
        }
        if !matches!(o["description"], Value::Object(_) | Value::String(_)) {
            return Err("description is a text component ({\"translate\": ...})".into());
        }
        holder_set("supported_items", &o["supported_items"])?;
        for key in ["primary_items", "exclusive_set"] {
            if let Some(v) = o.get(key) {
                holder_set(key, v)?;
            }
        }
        if !whole(&o["weight"]).is_some_and(|w| (1..=1024).contains(&w)) {
            return Err("weight is a whole number 1 to 1024".into());
        }
        if !whole(&o["max_level"]).is_some_and(|w| (1..=255).contains(&w)) {
            return Err("max_level is a whole number 1 to 255".into());
        }
        if !whole(&o["anvil_cost"]).is_some_and(|w| w >= 0) {
            return Err("anvil_cost is a whole number, 0 or more".into());
        }
        cost("min_cost", &o["min_cost"])?;
        cost("max_cost", &o["max_cost"])?;
        let slots = o["slots"].as_array().filter(|s| !s.is_empty());
        if !slots.is_some_and(|s| {
            s.iter()
                .all(|v| v.as_str().is_some_and(|v| ENCHANTMENT_SLOTS.contains(&v)))
        }) {
            return Err(format!(
                "slots is a list of {}",
                ENCHANTMENT_SLOTS.join(", ")
            ));
        }
        if o.get("effects").is_some_and(|e| !e.is_object()) {
            return Err("effects is an object of enchantment effect components".into());
        }
        Ok(Self {
            id: id.to_owned(),
            json,
        })
    }
}

/// `DamageScaling`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DamageScaling {
    Never,
    WhenCausedByLivingNonPlayer,
    Always,
}

impl DamageScaling {
    /// Its name in vanilla's JSON.
    pub fn name(self) -> &'static str {
        match self {
            Self::Never => "never",
            Self::WhenCausedByLivingNonPlayer => "when_caused_by_living_non_player",
            Self::Always => "always",
        }
    }
}

/// `DamageEffects`: the sound and visual a hit of the type has.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DamageEffects {
    #[default]
    Hurt,
    Thorns,
    Drowning,
    Burning,
    Poking,
    Freezing,
}

impl DamageEffects {
    /// Its name in vanilla's JSON.
    pub fn name(self) -> &'static str {
        match self {
            Self::Hurt => "hurt",
            Self::Thorns => "thorns",
            Self::Drowning => "drowning",
            Self::Burning => "burning",
            Self::Poking => "poking",
            Self::Freezing => "freezing",
        }
    }
}

/// `DeathMessageType`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum DeathMessageType {
    #[default]
    Default,
    FallVariants,
    IntentionalGameDesign,
}

impl DeathMessageType {
    /// Its name in vanilla's JSON.
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::FallVariants => "fall_variants",
            Self::IntentionalGameDesign => "intentional_game_design",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DamageTypeFile {
    message_id: String,
    scaling: DamageScaling,
    #[serde(default)]
    exhaustion: f32,
    #[serde(default)]
    effects: DamageEffects,
    #[serde(default)]
    death_message_type: DeathMessageType,
}

/// A damage type the mod adds (`data/<id>/horizon/damage_type/<path>.json`): vanilla's
/// `DamageType` JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageType {
    /// `<mod id>:<path>`.
    pub id: String,
    /// The translation key suffix of its death message (`death.attack.<message_id>`).
    pub message_id: String,
    pub scaling: DamageScaling,
    pub exhaustion: f32,
    pub effects: DamageEffects,
    pub death_message_type: DeathMessageType,
    /// The file as written (checked): what a data pack's `damage_type/<path>.json` holds.
    pub json: Value,
}

impl DamageType {
    pub fn parse(id: &str, bytes: &[u8]) -> Result<Self, String> {
        let json: Value = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let file: DamageTypeFile = serde_json::from_value(json.clone()).map_err(|e| e.to_string())?;
        if file.message_id.is_empty()
            || !file
                .message_id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
        {
            return Err("message_id is a translation key part: lower case letters, digits, _ . -".into());
        }
        if !file.exhaustion.is_finite() || !(0.0..=40.0).contains(&file.exhaustion) {
            return Err("exhaustion is 0 to 40".into());
        }
        Ok(Self {
            id: id.to_owned(),
            message_id: file.message_id,
            scaling: file.scaling,
            exhaustion: file.exhaustion,
            effects: file.effects,
            death_message_type: file.death_message_type,
            json,
        })
    }
}

/// A game rule's category (`GameRuleCategory`).
pub const GAME_RULE_CATEGORIES: [&str; 7] =
    ["player", "mobs", "spawning", "drops", "updates", "chat", "misc"];

/// A game rule's type, default and bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameRuleValue {
    Bool(bool),
    Int(i32),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GameRuleFile {
    #[serde(rename = "type")]
    kind: String,
    default: Value,
    min: Option<i32>,
    max: Option<i32>,
    category: Option<String>,
}

/// A game rule the mod adds (`data/<id>/horizon/game_rule/<path>.json`): its rule name is
/// `<mod id>:<path>`, readable and settable with `/gamerule` and by a mod's `world-control`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameRule {
    /// `<mod id>:<path>`.
    pub id: String,
    pub default: GameRuleValue,
    /// An integer rule's bounds (0 for a boolean).
    pub min: i32,
    pub max: i32,
    pub category: String,
}

impl GameRule {
    pub fn parse(id: &str, bytes: &[u8]) -> Result<Self, String> {
        let file: GameRuleFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        // The rule's path is one word, as a command's literal.
        let path = id.split_once(':').map_or(id, |(_, p)| p);
        if path.is_empty()
            || !path
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err("a game rule's file name is lower case letters, digits and underscores".into());
        }
        let category = file.category.unwrap_or_else(|| "misc".to_owned());
        if !GAME_RULE_CATEGORIES.contains(&category.as_str()) {
            return Err(format!(
                "category is one of {}",
                GAME_RULE_CATEGORIES.join(", ")
            ));
        }
        let (default, min, max) = match file.kind.as_str() {
            "bool" | "boolean" => {
                if file.min.is_some() || file.max.is_some() {
                    return Err("min and max are for an int rule".into());
                }
                let d = file.default.as_bool().ok_or("default of a bool rule is true or false")?;
                (GameRuleValue::Bool(d), 0, 0)
            }
            "int" | "integer" => {
                let d = file
                    .default
                    .as_i64()
                    .and_then(|d| i32::try_from(d).ok())
                    .ok_or("default of an int rule is a whole number")?;
                let (min, max) = (file.min.unwrap_or(0), file.max.unwrap_or(i32::MAX));
                if min > max || !(min..=max).contains(&d) {
                    return Err("an int rule needs min <= default <= max".into());
                }
                (GameRuleValue::Int(d), min, max)
            }
            other => return Err(format!("type {other:?} is bool or int")),
        };
        Ok(Self {
            id: id.to_owned(),
            default,
            min,
            max,
            category,
        })
    }
}

/// How a fluid moves things in it.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum FluidMotion {
    #[default]
    Water,
    Lava,
    None,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FluidFile {
    flowing_id: Option<String>,
    still_block: Option<String>,
    flowing_block: Option<String>,
    bucket: Option<String>,
    still_texture: Option<String>,
    flowing_texture: Option<String>,
    #[serde(default = "white")]
    tint: u32,
    #[serde(default = "two")]
    viscosity: u8,
    #[serde(default = "five")]
    tick_rate: u32,
    #[serde(default)]
    light: u8,
    #[serde(default = "yes")]
    can_drown: bool,
    #[serde(default)]
    motion: FluidMotion,
}
fn white() -> u32 {
    0xFF_FFFF
}
fn two() -> u8 {
    2
}
fn five() -> u32 {
    5
}
fn yes() -> bool {
    true
}

/// A fluid family the mod adds (`data/<id>/horizon/fluid/<path>.json`): what a WebAssembly
/// mod's `fluids` registration (`native-fluid-definition`) takes. The source fluid is
/// `<mod id>:<path>`; the rest default to names built from it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fluid {
    /// The source fluid, `<mod id>:<path>`.
    pub id: String,
    pub flowing_id: String,
    /// The blocks of the source and of the flowing fluid (a mod's own, declared in `block/`).
    pub still_block: String,
    pub flowing_block: String,
    /// The bucket item that places and takes the source (a mod's own).
    pub bucket: String,
    pub still_texture: String,
    pub flowing_texture: String,
    pub tint: u32,
    pub viscosity: u8,
    pub tick_rate: u32,
    pub light: u8,
    pub can_drown: bool,
    pub motion: FluidMotion,
}

impl Fluid {
    pub fn parse(id: &str, bytes: &[u8]) -> Result<Self, String> {
        let file: FluidFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        let (ns, path) = id.split_once(':').ok_or("an id is namespace:path")?;
        let fluid = Self {
            id: id.to_owned(),
            flowing_id: file.flowing_id.unwrap_or_else(|| format!("{ns}:flowing_{path}")),
            still_block: file.still_block.unwrap_or_else(|| id.to_owned()),
            flowing_block: file
                .flowing_block
                .unwrap_or_else(|| format!("{ns}:flowing_{path}")),
            bucket: file.bucket.unwrap_or_else(|| format!("{id}_bucket")),
            still_texture: file
                .still_texture
                .unwrap_or_else(|| format!("{ns}:block/{path}_still")),
            flowing_texture: file
                .flowing_texture
                .unwrap_or_else(|| format!("{ns}:block/{path}_flow")),
            tint: file.tint,
            viscosity: file.viscosity,
            tick_rate: file.tick_rate,
            light: file.light,
            can_drown: file.can_drown,
            motion: file.motion,
        };
        for (what, value) in [
            ("flowing_id", &fluid.flowing_id),
            ("still_block", &fluid.still_block),
            ("flowing_block", &fluid.flowing_block),
            ("bucket", &fluid.bucket),
            ("still_texture", &fluid.still_texture),
            ("flowing_texture", &fluid.flowing_texture),
        ] {
            if !id_like(value, true) || value.starts_with("minecraft:") {
                return Err(format!("{what} {value:?} is a namespace:path id of a mod"));
            }
        }
        if fluid.flowing_id == fluid.id || fluid.flowing_block == fluid.still_block {
            return Err("the flowing fluid and block differ from the source's".into());
        }
        if !(1..=8).contains(&fluid.viscosity) {
            return Err("viscosity is 1 to 8".into());
        }
        if !(1..=i32::MAX as u32).contains(&fluid.tick_rate) {
            return Err("tick_rate is 1 or more (game ticks between flow steps)".into());
        }
        if fluid.light > 15 {
            return Err("light is 0 to 15".into());
        }
        if fluid.tint > 0xFF_FFFF {
            return Err("tint is RGB, 0 to 16777215 (0xFFFFFF)".into());
        }
        Ok(fluid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENCHANTMENT: &str = r##"{
        "description": {"translate": "enchantment.ruby.edge"},
        "supported_items": "#minecraft:enchantable/sharp_weapon",
        "weight": 5, "max_level": 3,
        "min_cost": {"base": 5, "per_level_above_first": 8},
        "max_cost": {"base": 25, "per_level_above_first": 8},
        "anvil_cost": 2, "slots": ["mainhand"],
        "effects": {"minecraft:damage": []}}"##;

    #[test]
    fn enchantments_keep_vanillas_json() {
        let e = Enchantment::parse("ruby:edge", ENCHANTMENT.as_bytes()).unwrap();
        assert_eq!(e.id, "ruby:edge");
        assert_eq!(e.json["max_level"], 3);
        assert_eq!(e.json["effects"], serde_json::json!({"minecraft:damage": []}));
    }

    #[test]
    fn enchantments_refuse_what_vanillas_codec_would() {
        let with = |from: &str, to: &str| ENCHANTMENT.replacen(from, to, 1);
        for (text, why) in [
            (with(r#""weight": 5"#, r#""weight": 0"#), "weight"),
            (with(r#""weight": 5"#, r#""weight": 2000"#), "weight"),
            (with(r#""max_level": 3"#, r#""max_level": 256"#), "max_level"),
            (with(r#""anvil_cost": 2"#, r#""anvil_cost": -1"#), "anvil_cost"),
            (with(r#""mainhand""#, r#""pocket""#), "slots"),
            (with(r#""slots": ["mainhand"]"#, r#""slots": []"#), "slots"),
            (with(r##""#minecraft:enchantable/sharp_weapon""##, "7"), "supported_items"),
            (with(r##""#minecraft:enchantable/sharp_weapon""##, r##"["#minecraft:x"]"##), "supported_items"),
            (with(r#""base": 25"#, r#""base": "x""#), "max_cost"),
            (with(r#""effects": {"minecraft:damage": []}"#, r#""effects": 1"#), "effects"),
            (with(r#""anvil_cost": 2,"#, ""), "anvil_cost is missing"),
            (with(r#""weight": 5,"#, r#""weight": 5, "rarity": 1,"#), "unknown key"),
            (with(r#"{"translate": "enchantment.ruby.edge"}"#, "3"), "description"),
        ] {
            let e = Enchantment::parse("ruby:edge", text.as_bytes()).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
        assert!(Enchantment::parse("ruby:edge", b"[]").is_err());
    }

    #[test]
    fn damage_types_take_vanillas_fields() {
        let d = DamageType::parse(
            "ruby:shatter",
            br#"{"message_id": "shatter", "scaling": "when_caused_by_living_non_player", "exhaustion": 0.3, "effects": "freezing"}"#,
        )
        .unwrap();
        assert_eq!(d.scaling, DamageScaling::WhenCausedByLivingNonPlayer);
        assert_eq!(d.effects, DamageEffects::Freezing);
        assert_eq!(d.death_message_type, DeathMessageType::Default);
        assert!((d.exhaustion - 0.3).abs() < 1e-6);
        // the optional fields default as vanilla's codec does.
        let d = DamageType::parse("ruby:x", br#"{"message_id": "x", "scaling": "never"}"#).unwrap();
        assert_eq!((d.exhaustion, d.effects), (0.0, DamageEffects::Hurt));
        for (text, why) in [
            (r#"{"scaling": "never"}"#, "message_id"),
            (r#"{"message_id": "x", "scaling": "sometimes"}"#, "unknown variant"),
            (r#"{"message_id": "", "scaling": "never"}"#, "message_id"),
            (r#"{"message_id": "x", "scaling": "never", "exhaustion": -1}"#, "exhaustion"),
            (r#"{"message_id": "x", "scaling": "never", "effects": "boom"}"#, "unknown variant"),
            (r#"{"message_id": "x", "scaling": "never", "color": 1}"#, "color"),
        ] {
            let e = DamageType::parse("ruby:x", text.as_bytes()).unwrap_err();
            assert!(e.contains(why), "{why}: {e}");
        }
    }

    #[test]
    fn game_rules_are_bool_or_bounded_int() {
        let b = GameRule::parse("ruby:fast_mining", br#"{"type": "bool", "default": true, "category": "player"}"#)
            .unwrap();
        assert_eq!((b.default, b.category.as_str()), (GameRuleValue::Bool(true), "player"));
        let i = GameRule::parse("ruby:gem_rate", br#"{"type": "int", "default": 3, "min": 0, "max": 10}"#).unwrap();
        assert_eq!((i.default, i.min, i.max, i.category.as_str()), (GameRuleValue::Int(3), 0, 10, "misc"));
        for (id, text, why) in [
            ("ruby:a", r#"{"type": "int", "default": 11, "min": 0, "max": 10}"#, "min <= default"),
            ("ruby:a", r#"{"type": "int", "default": "x"}"#, "whole number"),
            ("ruby:a", r#"{"type": "bool", "default": 1}"#, "true or false"),
            ("ruby:a", r#"{"type": "bool", "default": true, "max": 3}"#, "for an int"),
            ("ruby:a", r#"{"type": "string", "default": "x"}"#, "bool or int"),
            ("ruby:a", r#"{"type": "bool", "default": true, "category": "fun"}"#, "category"),
            ("ruby:a/b", r#"{"type": "bool", "default": true}"#, "underscores"),
            ("ruby:A", r#"{"type": "bool", "default": true}"#, "underscores"),
        ] {
            let e = GameRule::parse(id, text.as_bytes()).unwrap_err();
            assert!(e.contains(why), "{text}: {e}");
        }
    }

    #[test]
    fn fluids_default_their_names_from_the_id() {
        let f = Fluid::parse("ruby:oil", b"{}").unwrap();
        assert_eq!(f.flowing_id, "ruby:flowing_oil");
        assert_eq!((f.still_block.as_str(), f.flowing_block.as_str()), ("ruby:oil", "ruby:flowing_oil"));
        assert_eq!(f.bucket, "ruby:oil_bucket");
        assert_eq!(f.still_texture, "ruby:block/oil_still");
        assert_eq!(f.flowing_texture, "ruby:block/oil_flow");
        assert_eq!((f.viscosity, f.tick_rate, f.light, f.can_drown), (2, 5, 0, true));
        assert_eq!((f.tint, f.motion), (0xFF_FFFF, FluidMotion::Water));
        let f = Fluid::parse(
            "ruby:lava",
            br#"{"flowing_id": "ruby:lava_flow", "tint": 16711680, "viscosity": 8, "tick_rate": 30, "light": 15, "can_drown": false, "motion": "lava", "bucket": "ruby:magma_pail"}"#,
        )
        .unwrap();
        assert_eq!((f.flowing_id.as_str(), f.tint, f.motion, f.bucket.as_str()), ("ruby:lava_flow", 0xFF0000, FluidMotion::Lava, "ruby:magma_pail"));
    }

    #[test]
    fn fluids_refuse_what_the_registry_would() {
        for (text, why) in [
            (r#"{"viscosity": 0}"#, "viscosity"),
            (r#"{"viscosity": 9}"#, "viscosity"),
            (r#"{"tick_rate": 0}"#, "tick_rate"),
            (r#"{"light": 16}"#, "light"),
            (r#"{"tint": 16777216}"#, "tint"),
            (r#"{"motion": "air"}"#, "unknown variant"),
            (r#"{"bucket": "minecraft:water_bucket"}"#, "bucket"),
            (r#"{"still_block": "water"}"#, "still_block"),
            (r#"{"flowing_id": "ruby:oil"}"#, "differ"),
            (r#"{"sticky": true}"#, "sticky"),
        ] {
            let e = Fluid::parse("ruby:oil", text.as_bytes()).unwrap_err();
            assert!(e.contains(why), "{text}: {e}");
        }
    }
}
