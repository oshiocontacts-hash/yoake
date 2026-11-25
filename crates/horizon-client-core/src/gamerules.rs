//! Game rules (26.3 `GameRules`): the registered rules with their category,
//! type, default and bounds, a value map, and persistence in the world's
//! `data/minecraft/game_rules.dat` saved-data file.
use horizon_world::{
    DATA_VERSION,
    compression::Compression,
    nbt::{self, Limits, NamedTag, Tag},
};
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    sync::RwLock,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    Bool(bool),
    Int(i32),
}
impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
        }
    }
}
pub struct Rule {
    pub name: &'static str,
    pub category: &'static str,
    pub default: Value,
    pub min: i32,
    pub max: i32,
}
const fn b(name: &'static str, category: &'static str, default: bool) -> Rule {
    Rule {
        name,
        category,
        default: Value::Bool(default),
        min: 0,
        max: 0,
    }
}
const fn i(name: &'static str, category: &'static str, default: i32, min: i32, max: i32) -> Rule {
    Rule {
        name,
        category,
        default: Value::Int(default),
        min,
        max,
    }
}
/// Every rule available without experimental features, in registration order.
pub const RULES: &[Rule] = &[
    b("advance_time", "updates", true),
    b("advance_weather", "updates", true),
    b("allow_entering_nether_using_portals", "misc", true),
    b("block_drops", "drops", true),
    b("block_explosion_drop_decay", "drops", true),
    b("command_blocks_work", "misc", true),
    b("command_block_output", "chat", true),
    b("drowning_damage", "player", true),
    b("elytra_movement_check", "player", true),
    b("ender_pearls_vanish_on_death", "player", true),
    b("entity_drops", "drops", true),
    b("fall_damage", "player", true),
    b("fire_damage", "player", true),
    i(
        "fire_spread_radius_around_player",
        "updates",
        128,
        -1,
        i32::MAX,
    ),
    b("forgive_dead_players", "mobs", true),
    b("freeze_damage", "player", true),
    b("global_sound_events", "misc", true),
    b("immediate_respawn", "player", false),
    b("keep_inventory", "player", false),
    b("lava_source_conversion", "updates", false),
    b("limited_crafting", "player", false),
    b("locator_bar", "player", true),
    b("log_admin_commands", "chat", true),
    i("max_block_modifications", "misc", 32768, 1, i32::MAX),
    i("max_command_forks", "misc", 65536, 0, i32::MAX),
    i("max_command_sequence_length", "misc", 65536, 0, i32::MAX),
    i("max_entity_cramming", "mobs", 24, 0, i32::MAX),
    i("max_snow_accumulation_height", "updates", 1, 0, 8),
    b("mob_drops", "drops", true),
    b("mob_explosion_drop_decay", "drops", true),
    b("mob_griefing", "mobs", true),
    b("natural_health_regeneration", "player", true),
    b("player_movement_check", "player", true),
    i(
        "players_nether_portal_creative_delay",
        "player",
        0,
        0,
        i32::MAX,
    ),
    i(
        "players_nether_portal_default_delay",
        "player",
        80,
        0,
        i32::MAX,
    ),
    i("players_sleeping_percentage", "player", 100, 0, i32::MAX),
    b("projectiles_can_break_blocks", "drops", true),
    b("pvp", "player", true),
    b("raids", "mobs", true),
    i("random_tick_speed", "updates", 3, 0, i32::MAX),
    b("reduced_debug_info", "misc", false),
    i("respawn_radius", "player", 10, 0, i32::MAX),
    b("send_command_feedback", "chat", true),
    b("show_advancement_messages", "chat", true),
    b("show_death_messages", "chat", true),
    b("spawner_blocks_work", "misc", true),
    b("spawn_mobs", "spawning", true),
    b("spawn_monsters", "spawning", true),
    b("spawn_patrols", "spawning", true),
    b("spawn_phantoms", "spawning", true),
    b("spawn_wandering_traders", "spawning", true),
    b("spawn_wardens", "spawning", true),
    b("spectators_generate_chunks", "player", true),
    b("spread_vines", "updates", true),
    b("tnt_explodes", "misc", true),
    b("tnt_explosion_drop_decay", "drops", false),
    b("universal_anger", "mobs", false),
    b("water_source_conversion", "updates", true),
];
/// GameRules.MAX_MINECART_SPEED (26.3), available with minecart improvements.
pub const MAX_MINECART_SPEED: Rule = i("max_minecart_speed", "misc", 8, 1, 1000);
pub const MINECART_IMPROVEMENTS: &str = "minecraft:minecart_improvements";

pub fn available_rules(minecart_improvements: bool) -> impl Iterator<Item = &'static Rule> {
    RULES
        .iter()
        .chain(minecart_improvements.then_some(&MAX_MINECART_SPEED))
        .chain(mod_rules())
}

pub fn rule(name: &str) -> Option<&'static Rule> {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    available_rules(true).find(|r| r.name == name)
}

/// The game rules content mods declare (`data/<mod>/horizon/game_rule/`), under the name
/// `<mod id>:<path>`. Rules are leaked so values and open screens can safely keep their names
/// across a registry replacement.
static MOD_RULES: RwLock<Vec<&'static Rule>> = RwLock::new(Vec::new());

pub fn mod_rules() -> Vec<&'static Rule> {
    MOD_RULES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

/// One mod rule as a file declares it: its `<mod id>:<path>` name, default, bounds (for an
/// integer rule) and category.
pub struct ModRule {
    pub name: String,
    pub default: Value,
    pub min: i32,
    pub max: i32,
    pub category: String,
}

/// Replaces the content mods' game rules after the content root is built or reloaded.
pub fn register_mod_rules(rules: Vec<ModRule>) {
    let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    let mut list: Vec<&'static Rule> = rules
        .into_iter()
        .filter(|r| r.name.contains(':') && !r.name.starts_with("minecraft:"))
        .map(|r| {
            let category = ["player", "mobs", "spawning", "drops", "updates", "chat", "misc"]
                .into_iter()
                .find(|c| *c == r.category)
                .unwrap_or("misc");
            &*Box::leak(Box::new(Rule {
                name: leak(r.name),
                category,
                default: r.default,
                min: r.min,
                max: r.max,
            }))
        })
        .collect();
    list.sort_by_key(|r| r.name);
    *MOD_RULES.write().unwrap_or_else(|e| e.into_inner()) = list;
}

/// The names of the registered mod rules, sorted.
pub fn mod_rule_names() -> Vec<&'static str> {
    mod_rules().iter().map(|r| r.name).collect()
}

impl Rule {
    /// The rule's id in saved data and on the wire: `minecraft:<name>`, or a mod rule's own
    /// `<mod id>:<path>`.
    pub fn id(&self) -> String {
        if self.name.contains(':') {
            self.name.to_owned()
        } else {
            format!("minecraft:{}", self.name)
        }
    }
    /// The translation key of the rule's name (`gamerule.<namespace>.<path>`).
    pub fn name_key(&self) -> String {
        format!("gamerule.{}", self.id().replace(':', "."))
    }
}

/// Current values (defaults unless set).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameRules {
    values: BTreeMap<&'static str, Value>,
    minecart_improvements: bool,
}
impl GameRules {
    pub fn with_features(features: &[String]) -> Self {
        Self {
            minecart_improvements: features.iter().any(|f| f == MINECART_IMPROVEMENTS),
            ..Self::default()
        }
    }
    /// GameRules.copy(enabledFeatures): keep available values, add defaults for
    /// newly enabled rules and discard rules absent from the target feature set.
    pub fn copy_with_features(&self, features: &[String]) -> Self {
        let mut rules = Self::with_features(features);
        for (&name, &value) in &self.values {
            rules.set(name, value);
        }
        rules
    }
    pub fn available_rules(&self) -> impl Iterator<Item = &'static Rule> + use<> {
        available_rules(self.minecart_improvements)
    }
    pub fn get(&self, name: &str) -> Option<Value> {
        let r = rule(name)?;
        if r.name == MAX_MINECART_SPEED.name && !self.minecart_improvements {
            return None;
        }
        let value = self.values.get(r.name).copied().unwrap_or(r.default);
        Some(match (r.default, value) {
            (Value::Int(_), Value::Int(v)) => Value::Int(v.clamp(r.min, r.max)),
            (Value::Bool(_), Value::Bool(_)) => value,
            _ => r.default,
        })
    }
    pub fn bool(&self, name: &str) -> bool {
        matches!(self.get(name), Some(Value::Bool(true)))
    }
    /// Parse a network rule value, retaining type and range validation.
    pub fn set_string(&mut self, name: &str, text: &str) -> bool {
        let value = match self.get(name) {
            Some(Value::Bool(_)) => text.parse::<bool>().ok().map(Value::Bool),
            Some(Value::Int(_)) => text.parse::<i32>().ok().map(Value::Int),
            None => None,
        };
        value.is_some_and(|v| self.set(name, v))
    }
    pub fn int(&self, name: &str) -> i32 {
        match self.get(name) {
            Some(Value::Int(v)) => v,
            _ => 0,
        }
    }
    /// Set a rule; false if the name or value type is wrong.
    pub fn set(&mut self, name: &str, value: Value) -> bool {
        let Some(r) = rule(name).filter(|r| self.get(r.name).is_some()) else {
            return false;
        };
        let ok = match (r.default, value) {
            (Value::Bool(_), Value::Bool(_)) => true,
            (Value::Int(_), Value::Int(v)) => (r.min..=r.max).contains(&v),
            _ => false,
        };
        if ok && value == r.default {
            self.values.remove(r.name);
        } else if ok {
            self.values.insert(r.name, value);
        }
        ok
    }
    /// Every rule's value by name without namespace, as `save::Level` holds
    /// them (booleans as 0 or 1).
    pub fn by_name(&self) -> BTreeMap<String, i32> {
        self.available_rules()
            .map(|r| {
                let v = match self.get(r.name).unwrap_or(r.default) {
                    Value::Bool(b) => i32::from(b),
                    Value::Int(i) => i,
                };
                (r.name.to_owned(), v)
            })
            .collect()
    }
    /// GameRuleMap codec: `{data: {"minecraft:<rule>": value...}, DataVersion}`.
    /// Removed mod rules' explicit values stay in the map, but not in `available_rules`.
    pub fn to_nbt(&self) -> NamedTag {
        let tag = |v| match v {
            Value::Bool(b) => Tag::Byte(i8::from(b)),
            Value::Int(i) => Tag::Int(i),
        };
        let mut data = BTreeMap::new();
        for (&name, &value) in &self.values {
            let id = if name.contains(':') {
                name.to_owned()
            } else {
                format!("minecraft:{name}")
            };
            data.insert(horizon_world::nbt::JavaString::from(id.as_str()), tag(value));
        }
        for r in self.available_rules() {
            data.insert(
                horizon_world::nbt::JavaString::from(r.id().as_str()),
                tag(self.get(r.name).unwrap_or(r.default)),
            );
        }
        NamedTag {
            name: "".into(),
            tag: Tag::compound([
                ("data", Tag::Compound(data)),
                ("DataVersion", Tag::Int(DATA_VERSION)),
            ]),
        }
    }
    pub fn from_nbt(root: &Tag) -> Self {
        Self::from_nbt_with_features(root, &[])
    }
    pub fn from_nbt_with_features(root: &Tag, features: &[String]) -> Self {
        let mut rules = Self::with_features(features);
        let Some(data) = root.get("data") else {
            return rules;
        };
        for r in rules.available_rules() {
            let key = r.id();
            let value = data.get(&key).or_else(|| data.get(r.name));
            match (r.default, value) {
                (Value::Bool(_), Some(Tag::Byte(v))) => {
                    rules.set(r.name, Value::Bool(*v != 0));
                }
                (Value::Int(_), Some(Tag::Int(v))) => {
                    let value = if r.name.contains(':') {
                        (*v).clamp(r.min, r.max)
                    } else {
                        *v
                    };
                    rules.set(r.name, Value::Int(value));
                }
                _ => {}
            }
        }
        if let Tag::Compound(entries) = data {
            for (key, value) in entries {
                let name = key.to_string_lossy();
                if !name.contains(':') || name.starts_with("minecraft:") || rule(&name).is_some() {
                    continue;
                }
                let value = match value {
                    Tag::Byte(v) => Value::Bool(*v != 0),
                    Tag::Int(v) => Value::Int(*v),
                    _ => continue,
                };
                rules.values.insert(Box::leak(name.into_boxed_str()), value);
            }
        }
        rules
    }
    pub fn load(world: &Path) -> Self {
        Self::load_with_features(world, &[])
    }
    pub fn load_with_features(world: &Path, features: &[String]) -> Self {
        let Ok(bytes) = fs::read(world.join("data/minecraft/game_rules.dat")) else {
            return Self::with_features(features);
        };
        let Ok(raw) = Compression::Gzip.decode(&bytes, 16 << 20) else {
            return Self::with_features(features);
        };
        match nbt::read_named(&raw, Limits::default()) {
            Ok((root, _)) => Self::from_nbt_with_features(&root.tag, features),
            Err(_) => Self::with_features(features),
        }
    }
    pub fn save(&self, world: &Path) -> crate::Result<()> {
        let dir = world.join("data/minecraft");
        fs::create_dir_all(&dir)?;
        let bytes =
            nbt::write_named(&self.to_nbt(), Limits::default()).map_err(|e| e.to_string())?;
        fs::write(
            dir.join("game_rules.dat"),
            Compression::Gzip.encode(&bytes)?,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// The mod-rule registry is process-wide: the tests that read or replace it take turns.
    static REGISTRY: std::sync::Mutex<()> = std::sync::Mutex::new(());
    use super::*;

    #[test]
    fn network_values_validate_rule_names_types_and_bounds() {
        let _registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        let mut rules = GameRules::default();
        assert!(rules.set_string("minecraft:keep_inventory", "true"));
        assert!(rules.bool("keep_inventory"));
        assert!(!rules.set_string("minecraft:keep_inventory", "1"));
        assert!(rules.bool("keep_inventory"));
        assert!(rules.set_string("minecraft:random_tick_speed", "7"));
        assert_eq!(rules.int("random_tick_speed"), 7);
        assert!(!rules.set_string("minecraft:random_tick_speed", "-1"));
        assert!(!rules.set_string("minecraft:random_tick_speed", "999999999999"));
        assert!(!rules.set_string("minecraft:unknown", "true"));
        assert!(!rules.set_string("other:keep_inventory", "false"));
        assert_eq!(rules.int("random_tick_speed"), 7);
    }
    #[test]
    fn defaults_bounds_and_round_trip() {
        let _registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        let mut rules = GameRules::default();
        assert!(rules.bool("advance_time"));
        assert_eq!(rules.int("random_tick_speed"), 3);
        assert!(!rules.set("random_tick_speed", Value::Bool(true)));
        assert!(!rules.set("max_snow_accumulation_height", Value::Int(9)));
        assert!(rules.set("keep_inventory", Value::Bool(true)));
        assert!(rules.set("minecraft:random_tick_speed", Value::Int(10)));
        let back = GameRules::from_nbt(&rules.to_nbt().tag);
        assert_eq!(back, rules);
        assert!(back.bool("keep_inventory"));
        let dir = std::env::temp_dir().join(format!("horizon-gamerules-{}", std::process::id()));
        rules.save(&dir).unwrap();
        assert_eq!(GameRules::load(&dir), rules);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn mod_rules_reload_without_changing_vanilla_rules() {
        let _registry = REGISTRY.lock().unwrap_or_else(|e| e.into_inner());
        // No mod: only vanilla's, unchanged.
        let before: Vec<&str> = available_rules(false).map(|r| r.name).collect();
        assert!(before.iter().all(|n| !n.contains(':')));
        assert!(rule("ruby:fast_mining").is_none());
        assert!(!GameRules::default().set("ruby:fast_mining", Value::Bool(true)));
        // Mods' rules are `<mod id>:<path>`; a name without a namespace is refused.
        register_mod_rules(vec![
            ModRule {
                name: "ruby:fast_mining".into(),
                default: Value::Bool(false),
                min: 0,
                max: 0,
                category: "player".into(),
            },
            ModRule {
                name: "ruby:gem_rate".into(),
                default: Value::Int(3),
                min: 1,
                max: 9,
                category: "nonsense".into(),
            },
            ModRule {
                name: "bare".into(),
                default: Value::Bool(true),
                min: 0,
                max: 0,
                category: "misc".into(),
            },
        ]);
        assert_eq!(mod_rule_names(), ["ruby:fast_mining", "ruby:gem_rate"]);
        // Vanilla's rules come first and are the same.
        let after: Vec<&str> = available_rules(false).map(|r| r.name).collect();
        assert_eq!(&after[..before.len()], before.as_slice());
        assert_eq!(rule("ruby:gem_rate").unwrap().category, "misc");
        assert_eq!(rule("ruby:fast_mining").unwrap().category, "player");
        assert_eq!(rule("ruby:fast_mining").unwrap().id(), "ruby:fast_mining");
        assert_eq!(rule("keep_inventory").unwrap().id(), "minecraft:keep_inventory");
        assert_eq!(rule("ruby:gem_rate").unwrap().name_key(), "gamerule.ruby.gem_rate");
        let mut rules = GameRules::default();
        assert!(!rules.bool("ruby:fast_mining"));
        assert_eq!(rules.int("ruby:gem_rate"), 3);
        assert!(rules.set_string("ruby:fast_mining", "true"));
        assert!(rules.bool("ruby:fast_mining"));
        assert!(rules.set("ruby:gem_rate", Value::Int(9)));
        assert!(!rules.set("ruby:gem_rate", Value::Int(10)), "bounds");
        assert!(!rules.set("ruby:gem_rate", Value::Bool(true)), "type");
        assert_eq!(rules.by_name()["ruby:gem_rate"], 9);
        // Saved under the rule's own id and read back.
        let tag = rules.to_nbt().tag;
        assert!(tag.get("data").unwrap().get("ruby:gem_rate").is_some());
        assert!(tag.get("data").unwrap().get("minecraft:ruby:gem_rate").is_none());
        let back = GameRules::from_nbt(&tag);
        assert_eq!(back, rules);
        assert_eq!(back.int("ruby:gem_rate"), 9);

        register_mod_rules(vec![
            ModRule {
                name: "ruby:gem_rate".into(),
                default: Value::Int(4),
                min: 0,
                max: 5,
                category: "chat".into(),
            },
            ModRule {
                name: "ruby:new_rule".into(),
                default: Value::Bool(true),
                min: 0,
                max: 0,
                category: "misc".into(),
            },
        ]);
        assert_eq!(mod_rule_names(), ["ruby:gem_rate", "ruby:new_rule"]);
        assert_eq!(rules.int("ruby:gem_rate"), 5, "existing value is clamped");
        assert_eq!(rule("ruby:gem_rate").unwrap().category, "chat");
        assert!(rules.bool("ruby:new_rule"), "added rule is available");
        assert_eq!(rules.get("ruby:fast_mining"), None, "removed rule is hidden");
        assert!(!rules.by_name().contains_key("ruby:fast_mining"));
        let hidden = GameRules::from_nbt(&rules.to_nbt().tag);
        assert_eq!(hidden.int("ruby:gem_rate"), 5);
        assert_eq!(hidden.get("ruby:fast_mining"), None);

        register_mod_rules(vec![ModRule {
            name: "ruby:fast_mining".into(),
            default: Value::Bool(false),
            min: 0,
            max: 0,
            category: "player".into(),
        }]);
        assert!(hidden.bool("ruby:fast_mining"), "re-adding restores its value");
        let after_reload: Vec<&str> = available_rules(false).map(|r| r.name).collect();
        assert_eq!(&after_reload[..before.len()], before.as_slice());
        assert_eq!(rules.int("random_tick_speed"), 3);
        register_mod_rules(Vec::new());
    }
}
