//! The data schema of a fully custom mob (`"template": "yoake:mob"`, FR-F4-MOB,
//! docs/yoake2/FR-F4-MOBS.md sections 3 to 6): serde types, validation and limits.
//!
//! A custom mob is one JSON file, `data/<id>/horizon/entity_type/<name>.json`. It needs a network
//! id before a connection starts, exactly as an item or a block does, so it is data; WASM only
//! supplies behaviour by id. [`content`](crate::content) dispatches a file whose `template` is
//! [`TEMPLATE`] here; the other templates are untouched.
//!
//! This module only reads and checks the file. Nothing is registered with the engine yet (F4M-3
//! builds the type from a [`CustomMobFile`]); a custom mob a mod ships loads, validates, and is
//! kept in [`Content::custom_mobs`](crate::content::Content::custom_mobs) without effect.
//!
//! Model, animation and expression files are read by the `horizon-mobmodel` crate (F4M-1, F4M-2);
//! the numeric limits for them are defined here so that one place states them. [`ModelInfo`] is
//! the small interface the schema check needs; [`LoadedModel`] implements it over a real
//! `horizon_mobmodel::Model` (the crate itself has no game or mod dependencies, so the trait stays
//! on this side) and [`CustomMobFile::load_model`] reads the files through the real loaders.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The accepted value of `template`.
pub const TEMPLATE: &str = "yoake:mob";
/// The accepted value of `format`.
pub const FORMAT: u32 = 1;
/// The file the content root keeps the mods' custom mob definitions in (a JSON array of
/// [`CustomMobFile`] objects, each with its `id`): the game's mob loader reads it, so the client,
/// the host and a dedicated server agree (as `horizon-mod-mobs.json` does for the template mobs).
pub const CUSTOM_MOBS_FILE: &str = "horizon-mod-custom-mobs.json";

/// The id of the generated spawn egg item of the custom mob `id` (`<ns>:<path>_spawn_egg`): the
/// game spawns the mob when an item named `<type>_spawn_egg` is used.
pub fn spawn_egg_item(id: &str) -> String {
    format!("{id}_spawn_egg")
}

/// Limits a custom mob's files must keep (checked at load, never at run time).
pub mod limits {
    /// Bones in one model.
    pub const MAX_BONES: usize = 256;
    /// Cubes in one model.
    pub const MAX_CUBES: usize = 4096;
    /// Animations of one type, over all its animation files.
    pub const MAX_ANIMATIONS: usize = 128;
    /// Tracked (synced) values of one type.
    pub const MAX_SYNCED: usize = 32;
    /// Instructions of one compiled animation expression.
    pub const MAX_EXPRESSION_INSTRUCTIONS: usize = 256;
    /// The bytes of all synced values of one mob.
    pub const MAX_SYNCED_BYTES: usize = 256;
    /// The bytes of a synced string.
    pub const MAX_SYNCED_STRING: usize = 64;
    /// Hitbox parts, goals (data and script), attributes, layers, LOD steps, seats, spawn rules.
    pub const MAX_HITBOXES: usize = 16;
    pub const MAX_GOALS: usize = 64;
    pub const MAX_ATTRIBUTES: usize = 32;
    pub const MAX_LAYERS: usize = 16;
    pub const MAX_LOD: usize = 8;
    pub const MAX_SEATS: usize = 8;
    pub const MAX_SPAWN_RULES: usize = 16;
    pub const MAX_ENUM_VALUES: usize = 64;
    pub const MAX_ANIMATION_FILES: usize = 32;
    /// A box side, in blocks (the data mob templates allow the same).
    pub const MAX_SIZE: f64 = 16.0;
}

/// Hook ids and binding ids set aside for custom mobs, so that other lanes' additions do not take
/// the numbers the F4M items will need. `hook-ledger/meta.json`'s `next_id` was moved from 8475 to
/// 9475 (first 710 to 1710; FR-F1 W2 then used ids 710..=8474 for the stamped hooks, so the range moved
/// to follow it, MERGE4): the ledger tool hands out ids from `next_id`, so ledger ids `HOOK_FIRST..=HOOK_LAST` stay
/// free for the custom mob hooks (they are assigned in that range by hand, one per new hook).
/// `crates/yoake-bindgen/ids.lock.json` got an `unbound` sentinel at `BINDING_LAST`: the generator
/// assigns `max id + 1`, so every later generated binding (the `custom_mobs` module included) is
/// numbered from 2515 on and ids `BINDING_FIRST..BINDING_LAST` are never handed out by it. A test
/// below keeps both in place; nothing existing was renumbered.
pub mod reserved_ids {
    /// Ledger hook ids `FIRST..=LAST` belong to the custom mob hooks (FR-F4-MOB section 7).
    pub const HOOK_FIRST: u32 = 8475;
    pub const HOOK_LAST: u32 = 9474;
    /// Engine-call binding ids of the `custom_mobs` module (section 6.3).
    pub const BINDING_FIRST: u32 = 1515;
    pub const BINDING_LAST: u32 = 2514;
    /// The path of the sentinel entry in the binding lock.
    pub const BINDING_SENTINEL: &str = "custom_mobs::__reserved_range_end";
}

/// What a model loader tells the schema check about a model (`horizon-mobmodel` implements it).
pub trait ModelInfo {
    fn bone_count(&self) -> usize;
    fn cube_count(&self) -> usize;
    fn animation_count(&self) -> usize;
    /// The bone's index (`BoneIndex` in the model crate), or `None`.
    fn bone_index(&self, name: &str) -> Option<u16>;
    fn has_locator(&self, name: &str) -> bool;
}

/// A model read through the real loaders (`horizon-mobmodel`), with the number of animations its
/// animation files hold. `model` is `None` when the type declares no `model.geo`.
#[derive(Debug)]
pub struct LoadedModel {
    pub model: Option<horizon_mobmodel::Model>,
    pub animations: usize,
    /// The model's skeleton with the type's animations and controllers bound (F4M-5b: the server
    /// runs it). `None` without a model, or when the type lists no animation and no controller.
    pub rig: Option<horizon_mobmodel::pose::Rig>,
    /// The typed animation events of the rig (`model.server_events`, `model.particles`).
    pub events: Option<horizon_mobmodel::events::MobEvents>,
}

impl LoadedModel {
    /// The rest position of bone `name`'s pivot in the model's space (pixels, +X east, +Y up, -Z
    /// north, the origin at the mob's feet), with the rest rotations of its parents applied.
    pub fn bone_rest_position(&self, name: &str) -> Option<[f64; 3]> {
        use horizon_mobmodel::{Pose, skeleton::bone_matrices};
        let model = self.model.as_ref()?;
        let index = model.skeleton.find(name)?;
        let matrices = bone_matrices(&model.skeleton, &Pose::rest(&model.skeleton));
        let p = matrices[index.index()].transform_point3(model.skeleton.bones[index.index()].pivot);
        Some([f64::from(p.x), f64::from(p.y), f64::from(p.z)])
    }
}

impl LoadedModel {
    /// The rest position of locator `name` in the model's space (pixels; same frame as
    /// [`bone_rest_position`](Self::bone_rest_position)), with the rest rotations of its bone's
    /// parents applied. A server seat that follows a locator sits here (F4M-4c).
    pub fn locator_rest_position(&self, name: &str) -> Option<[f64; 3]> {
        use horizon_mobmodel::{Pose, skeleton::bone_matrices};
        let model = self.model.as_ref()?;
        let l = model.locators.iter().find(|l| l.name == name)?;
        let matrices = bone_matrices(&model.skeleton, &Pose::rest(&model.skeleton));
        let p = matrices.get(l.bone.index())?.transform_point3(l.offset);
        Some([f64::from(p.x), f64::from(p.y), f64::from(p.z)])
    }
}

impl ModelInfo for LoadedModel {
    fn bone_count(&self) -> usize {
        self.model.as_ref().map_or(0, |m| m.skeleton.len())
    }
    fn cube_count(&self) -> usize {
        self.model.as_ref().map_or(0, |m| m.element_count())
    }
    fn animation_count(&self) -> usize {
        self.animations
    }
    fn bone_index(&self, name: &str) -> Option<u16> {
        self.model.as_ref()?.skeleton.find(name).map(|b| b.0)
    }
    fn has_locator(&self, name: &str) -> bool {
        self.model
            .as_ref()
            .is_some_and(|m| m.locators.iter().any(|l| l.name == name))
    }
}

/// The mob's categories (`MobCategory`).
pub const CATEGORIES: [&str; 8] = [
    "monster",
    "creature",
    "ambient",
    "water_creature",
    "axolotls",
    "water_ambient",
    "underground_water_creature",
    "misc",
];
/// Navigators ([`Ai::navigator`]).
pub const NAVIGATORS: [&str; 5] = ["ground", "climb", "swim", "fly", "amphibious"];
/// The built-in attribute names (bare, all 26.3 attributes); a custom attribute is a namespaced id.
pub const BUILTIN_ATTRIBUTES: [&str; 15] = [
    "max_health",
    "movement_speed",
    "attack_damage",
    "attack_knockback",
    "follow_range",
    "armor",
    "armor_toughness",
    "knockback_resistance",
    "flying_speed",
    "step_height",
    "gravity",
    "jump_strength",
    "safe_fall_distance",
    "scale",
    "fall_damage_multiplier",
];
/// Yoake-namespaced attributes a file sets as a plain number like a built-in one (they are not 26.3
/// attributes): `yoake:water_slowdown` is how much of its speed a mob keeps each tick in water
/// (the engine's `water_slowdown`, 0.8 by default).
pub const YOAKE_ATTRIBUTES: [&str; 1] = ["yoake:water_slowdown"];
/// Data goal types: the nine that exist (`Behaviour`) and the new ones (section 6.1), and
/// `script` (a WASM goal id).
pub const GOAL_TYPES: [&str; 41] = [
    "float",
    "wander",
    "look_at_player",
    "random_look",
    "panic",
    "tempt",
    "flee",
    "melee_attack",
    "target_player",
    "hurt_by_target",
    "swim",
    "fly_wander",
    "climb",
    "leap_at_target",
    "leap_attack",
    "ranged_attack",
    "area_attack",
    "follow_owner",
    "sit",
    "owner_hurt_target",
    "owner_hurt_by",
    "breed",
    "follow_parent",
    "avoid",
    "eat_block",
    "go_to_block",
    "sleep_in_bed_like",
    "patrol_around",
    "stay_near_home",
    "open_door",
    "retreat_when_hurt",
    "call_for_help",
    "random_swim",
    "nearest_target",
    "nearest_attackable",
    "hurt_by",
    "owner_target",
    "script",
    "look_at_target",
    "stroll",
    "float_in_water",
];
/// The control flags a script goal may claim.
pub const GOAL_FLAGS: [&str; 4] = ["move", "look", "jump", "target"];
/// Synced value types.
pub const SYNCED_TYPES: [&str; 6] = ["bool", "i32", "f32", "string", "vec3", "enum"];
/// Spawn placements.
pub const PLACEMENTS: [&str; 6] = [
    "on_ground",
    "in_water",
    "in_lava",
    "on_surface",
    "no_restrictions",
    "custom",
];
/// Sound slots of [`CustomMobFile::sounds`].
pub const SOUND_SLOTS: [&str; 7] = ["ambient", "hurt", "death", "step", "attack", "swim", "fly"];
/// Layer modes.
pub const LAYER_MODES: [&str; 4] = ["cutout", "translucent", "additive", "emissive"];
/// LOD modes.
pub const LOD_MODES: [&str; 6] = [
    "full",
    "half_rate",
    "quarter_rate",
    "low_bones",
    "static",
    "hidden",
];

fn one() -> f64 {
    1.0
}
fn yes() -> bool {
    true
}
fn format_one() -> u32 {
    FORMAT
}
fn creature() -> String {
    "creature".to_owned()
}
fn ground() -> String {
    "ground".to_owned()
}
fn all() -> String {
    "all".to_owned()
}
fn cutout() -> String {
    "cutout".to_owned()
}
fn five() -> i32 {
    5
}
fn ten_u32() -> u32 {
    10
}
fn one_u32() -> u32 {
    1
}
fn four_u32() -> u32 {
    4
}

/// A colour: an integer or a `0xRRGGBB` / `#RRGGBB` string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rgb(pub u32);
impl<'de> Deserialize<'de> for Rgb {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(u32),
            Text(String),
        }
        let value = match Raw::deserialize(d)? {
            Raw::Int(v) => v,
            Raw::Text(text) => {
                let digits = text
                    .strip_prefix("0x")
                    .or_else(|| text.strip_prefix("0X"))
                    .or_else(|| text.strip_prefix('#'))
                    .ok_or_else(|| D::Error::custom("a colour is 0xRRGGBB or #RRGGBB"))?;
                u32::from_str_radix(digits, 16).map_err(D::Error::custom)?
            }
        };
        if value > 0xFF_FFFF {
            return Err(D::Error::custom("a colour is at most 0xFFFFFF"));
        }
        Ok(Rgb(value))
    }
}

/// `size`: the entity box (the only collision box), in blocks.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Size {
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub eye_height: Option<f64>,
}

/// One hurt and pick part (`hitboxes`); the extra parts of a mob are not solid collision.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Hitbox {
    pub name: String,
    /// Width and height in blocks.
    pub size: [f64; 2],
    #[serde(default)]
    pub offset: [f64; 3],
    /// Follows this bone of the model (needs the model); otherwise a static offset turned with the body.
    #[serde(default)]
    pub bone: Option<String>,
    #[serde(default = "one")]
    pub damage_multiplier: f64,
    #[serde(default = "yes")]
    pub pick: bool,
    #[serde(default = "yes")]
    pub hurt: bool,
    #[serde(default = "yes")]
    pub rotation_yaw_follow: bool,
}

/// `attributes` values: a number sets a built-in attribute's base, an object declares a custom one.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum Attribute {
    Value(f64),
    Custom(CustomAttribute),
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CustomAttribute {
    #[serde(default)]
    pub default: f64,
    #[serde(default)]
    pub min: Option<f64>,
    #[serde(default)]
    pub max: Option<f64>,
    /// `all`, `owner` or `none`.
    #[serde(default = "all")]
    pub sync: String,
}

/// One tracked value (`synced`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Synced {
    /// One of [`SYNCED_TYPES`].
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub default: Option<serde_json::Value>,
    /// `i32` range.
    #[serde(default)]
    pub min: Option<i32>,
    #[serde(default)]
    pub max: Option<i32>,
    /// `enum` names (one byte on the wire).
    #[serde(default)]
    pub values: Vec<String>,
    #[serde(default)]
    pub persistent: bool,
    /// `all`, `owner` or `none`.
    #[serde(default = "all")]
    pub sync: String,
    /// The client interpolates a float.
    #[serde(default)]
    pub interp: bool,
    /// Most updates per second (1 to 20; 10 when absent).
    #[serde(default)]
    pub sync_rate: Option<u32>,
}
impl Synced {
    /// The wire size of the value in bytes, for the per-mob budget ([`limits::MAX_SYNCED_BYTES`]).
    pub fn wire_bytes(&self) -> usize {
        match self.kind.as_str() {
            "bool" | "enum" => 1,
            "i32" | "f32" => 4,
            "vec3" => 12,
            "string" => limits::MAX_SYNCED_STRING + 1,
            _ => 0,
        }
    }
}

/// A goal or target selector (`ai.goals`, `ai.targets`); the keys other than these are the goal
/// type's own parameters.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Goal {
    /// One of [`GOAL_TYPES`].
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default = "five")]
    pub priority: i32,
    #[serde(default)]
    pub speed: Option<f64>,
    /// The `script` goal's id (a [`ScriptGoal`] of this type).
    #[serde(default)]
    pub id: Option<String>,
    #[serde(flatten)]
    pub params: BTreeMap<String, serde_json::Value>,
}

/// A goal whose body is a WASM export (`ai.script_goals`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ScriptGoal {
    pub id: String,
    #[serde(default = "five")]
    pub priority: i32,
    /// Of [`GOAL_FLAGS`].
    #[serde(default)]
    pub flags: Vec<String>,
    /// Evaluate `can-use` every this many ticks (1 to 200).
    #[serde(default = "ten_u32")]
    pub interval: u32,
}

/// `ai`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ai {
    #[serde(default = "ground")]
    pub navigator: String,
    #[serde(default)]
    pub goals: Vec<Goal>,
    #[serde(default)]
    pub targets: Vec<Goal>,
    /// Brain sensors; checked by the AI item (F4M-4).
    #[serde(default)]
    pub sensors: Vec<serde_json::Value>,
    #[serde(default)]
    pub script_goals: Vec<ScriptGoal>,
}

impl Default for Ai {
    fn default() -> Self {
        Ai {
            navigator: ground(),
            goals: Vec::new(),
            targets: Vec::new(),
            sensors: Vec::new(),
            script_goals: Vec::new(),
        }
    }
}

/// `model.shadow`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Shadow {
    pub radius: f64,
    #[serde(default = "one")]
    pub strength: f64,
}
/// `model.cull`: the box a frustum test uses, separate from the hitbox.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Cull {
    pub width: f64,
    pub height: f64,
    #[serde(default)]
    pub y_offset: f64,
}
/// `model.textures`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Textures {
    pub default: Option<String>,
    #[serde(default)]
    pub variants: Vec<String>,
}
/// `model.layers`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Layer {
    pub name: String,
    #[serde(default)]
    pub texture: Option<String>,
    #[serde(default = "cutout")]
    pub mode: String,
    #[serde(default)]
    pub bones: Vec<String>,
    /// Expressions, compiled by the model crate.
    #[serde(default)]
    pub tint: Option<String>,
    #[serde(default)]
    pub visible: Option<String>,
}
/// `model.lod`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Lod {
    pub distance: f64,
    pub mode: String,
}

/// `model`: the files and the drawing options. Ids are `<namespace>:<path>`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Model {
    /// `<ns>:geo/<name>`: `assets/<ns>/geo/<name>.geo.json`, or `<name>.bbmodel` when only that exists.
    #[serde(default)]
    pub geo: Option<String>,
    #[serde(default)]
    pub textures: Textures,
    /// `<ns>:animations/<name>`: `assets/<ns>/animations/<name>.animation.json`.
    #[serde(default)]
    pub animations: Vec<String>,
    /// `<ns>:controllers/<name>`: `assets/<ns>/controllers/<name>.json`.
    #[serde(default)]
    pub controllers: Vec<String>,
    #[serde(default = "one")]
    pub scale: f64,
    #[serde(default)]
    pub shadow: Option<Shadow>,
    #[serde(default)]
    pub cull: Option<Cull>,
    #[serde(default)]
    pub layers: Vec<Layer>,
    #[serde(default)]
    pub lod: Vec<Lod>,
    /// `mainhand`, `offhand`, `head` to a locator name.
    #[serde(default)]
    pub item_slots: BTreeMap<String, String>,
    /// Procedural constraints; checked by the animation item (F4M-2).
    #[serde(default)]
    pub procedural: Vec<serde_json::Value>,
    #[serde(default)]
    pub particles: BTreeMap<String, String>,
    /// The animations whose `Custom` timeline events the server runs (F4M-9, F4M-5b): `true`,
    /// `false`, a list of animation names or an object of flags; checked when the model loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_events: Option<serde_json::Value>,
}

impl Default for Model {
    fn default() -> Self {
        Model {
            geo: None,
            textures: Textures::default(),
            animations: Vec::new(),
            controllers: Vec::new(),
            scale: 1.0,
            shadow: None,
            cull: None,
            layers: Vec::new(),
            lod: Vec::new(),
            item_slots: BTreeMap::new(),
            procedural: Vec::new(),
            particles: BTreeMap::new(),
            server_events: None,
        }
    }
}

/// One natural spawn rule (`spawn`).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Spawn {
    /// An id, a `#tag`, or a list of them.
    pub biomes: serde_json::Value,
    #[serde(default)]
    pub structures: Option<serde_json::Value>,
    #[serde(default)]
    pub dimensions: Option<serde_json::Value>,
    #[serde(default = "ten_u32")]
    pub weight: u32,
    #[serde(default = "one_u32")]
    pub min: u32,
    #[serde(default = "four_u32")]
    pub max: u32,
    #[serde(default)]
    pub placement: Option<String>,
    /// `any`, `dark`, `bright` or `{sky_max, block_max}`.
    #[serde(default)]
    pub light: Option<serde_json::Value>,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub y_range: Option<[i32; 2]>,
    #[serde(default)]
    pub min_distance_from_player: Option<f64>,
    #[serde(default)]
    pub cluster: Option<u32>,
}

/// `despawn`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Despawn {
    #[serde(default)]
    pub persistent_by_default: bool,
    #[serde(default)]
    pub immediate_distance: Option<f64>,
    #[serde(default)]
    pub random_distance: Option<f64>,
    #[serde(default)]
    pub near_player_distance_keep: Option<f64>,
    #[serde(default)]
    pub age_limit: Option<u32>,
}

/// `spawn_egg`: `true` (the mod declares the item itself) or two colours (the egg is generated).
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(untagged)]
pub enum SpawnEgg {
    Declared(bool),
    Generated { primary: Rgb, secondary: Rgb },
}

/// `tameable`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Tameable {
    /// Item ids or `#tags`.
    pub items: Vec<String>,
    #[serde(default = "tame_chance")]
    pub chance: f64,
    #[serde(default)]
    pub sits_on_command: bool,
    #[serde(default)]
    pub follow_owner: bool,
}
fn tame_chance() -> f64 {
    0.33
}
/// `breedable`.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Breedable {
    pub food: Vec<String>,
    #[serde(default)]
    pub partner: Option<String>,
    #[serde(default)]
    pub offspring: Option<String>,
    #[serde(default)]
    pub cooldown: Option<u32>,
    #[serde(default)]
    pub baby_scale: Option<f64>,
}
/// `seats`.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Seat {
    #[serde(default)]
    pub offset: Option<[f64; 3]>,
    #[serde(default)]
    pub locator: Option<String>,
    /// `none` or `player`.
    #[serde(default)]
    pub controls: Option<String>,
    #[serde(default)]
    pub allowed_types: Vec<String>,
    #[serde(default)]
    pub yaw_lock: bool,
}

/// What an `interact` row does (F4M-4c data shortcuts of the `mob-interact` hook).
pub const INTERACT_ACTIONS: [&str; 4] = ["feed", "tame", "ride", "give_item"];

/// One `interact` row: what a right click with `item` does. `feed` (the item is the type's
/// `breedable.food` unless the row names one), `tame` (`tameable.items`), `ride` (the seats) and
/// `give_item` (the player gets `give`). The rows are tried in order, before the implicit ones that
/// `tameable`, `breedable` and `seats` give.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Interact {
    /// One of [`INTERACT_ACTIONS`].
    pub action: String,
    /// An item id or `#tag`; none means any item (an empty hand too).
    #[serde(default)]
    pub item: Option<String>,
    /// `give_item`: the item given.
    #[serde(default)]
    pub give: Option<String>,
    /// `give_item`: how many (1 to 64, default 1).
    #[serde(default)]
    pub count: Option<u32>,
    /// Whether one of the held item is used up (default: `give_item` and `feed` and `tame` do,
    /// `ride` does not).
    #[serde(default)]
    pub consume: Option<bool>,
    /// `true` only when sneaking, `false` only when not; unset, either.
    #[serde(default)]
    pub sneaking: Option<bool>,
}

/// A custom mob's file, as written.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CustomMobFile {
    /// [`TEMPLATE`].
    pub template: String,
    /// [`FORMAT`].
    #[serde(default = "format_one")]
    pub format: u32,
    /// `<mod namespace>:<path>`, set by the loader from the file's path.
    #[serde(skip_deserializing, default)]
    pub id: String,
    /// One of [`CATEGORIES`].
    #[serde(default = "creature")]
    pub category: String,
    pub size: Size,
    #[serde(default)]
    pub hitboxes: Vec<Hitbox>,
    #[serde(default)]
    pub attributes: BTreeMap<String, Attribute>,
    #[serde(default)]
    pub model: Model,
    #[serde(default)]
    pub synced: BTreeMap<String, Synced>,
    #[serde(default)]
    pub ai: Ai,
    #[serde(default)]
    pub spawn: Vec<Spawn>,
    #[serde(default)]
    pub despawn: Option<Despawn>,
    #[serde(default)]
    pub spawn_egg: Option<SpawnEgg>,
    #[serde(default)]
    pub loot_table: Option<String>,
    /// Slots of [`SOUND_SLOTS`] to sound event ids.
    #[serde(default)]
    pub sounds: BTreeMap<String, String>,
    #[serde(default)]
    pub tameable: Option<Tameable>,
    #[serde(default)]
    pub breedable: Option<Breedable>,
    #[serde(default)]
    pub seats: Vec<Seat>,
    /// An item that must be put on the mob (a right click, one is used up) before it can be ridden.
    #[serde(default)]
    pub saddle_item: Option<String>,
    /// A safe expression (the goal rows' `when`) that must be true for a rider to board.
    #[serde(default)]
    pub rideable_when: Option<String>,
    /// Data shortcuts of `mob-interact`.
    #[serde(default)]
    pub interact: Vec<Interact>,
    /// The vanilla entity a stock client is shown instead (`minecraft:fox`).
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub proxy_name_tag: Option<String>,
}

/// `<namespace>:<path>` of lowercase letters, digits, `_.-` and `/` in the path.
pub fn is_id(id: &str) -> bool {
    let ok = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b);
    id.split_once(':').is_some_and(|(ns, path)| {
        !ns.is_empty()
            && !path.is_empty()
            && ns.bytes().all(ok)
            && path.bytes().all(|b| ok(b) || b == b'/')
    })
}

fn finite_in(what: &str, v: f64, lo: f64, hi: f64) -> Result<(), String> {
    if v.is_finite() && v >= lo && v <= hi {
        Ok(())
    } else {
        Err(format!("{what} is {lo} to {hi}"))
    }
}
fn one_of(what: &str, v: &str, set: &[&str]) -> Result<(), String> {
    if set.contains(&v) {
        Ok(())
    } else {
        Err(format!("{what} {v:?} is not one of {set:?}"))
    }
}
fn too_many(what: &str, n: usize, max: usize) -> Result<(), String> {
    if n > max {
        Err(format!("{what}: {n} entries, at most {max}"))
    } else {
        Ok(())
    }
}
/// A part name: 1 to 48 of `[A-Za-z0-9_.-]`.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
}

/// Whether a mob type file asks for the custom mob schema (its `template` is [`TEMPLATE`]).
pub fn declares_custom(bytes: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .is_some_and(|v| v.get("template").and_then(|t| t.as_str()) == Some(TEMPLATE))
}

impl CustomMobFile {
    /// Reads and checks a file's bytes; `id` is the type's `<namespace>:<path>`.
    pub fn parse(id: &str, bytes: &[u8]) -> Result<CustomMobFile, String> {
        let mut file: CustomMobFile = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        file.id = id.to_owned();
        file.validate()?;
        Ok(file)
    }

    /// The checks that need only the file itself.
    pub fn validate(&self) -> Result<(), String> {
        if self.template != TEMPLATE {
            return Err(format!("template is {TEMPLATE:?}"));
        }
        if self.format != FORMAT {
            return Err(format!(
                "format {} is not supported (this is {FORMAT})",
                self.format
            ));
        }
        one_of("category", &self.category, &CATEGORIES)?;
        finite_in("size.width", self.size.width, 0.0001, limits::MAX_SIZE)?;
        finite_in("size.height", self.size.height, 0.0001, limits::MAX_SIZE)?;
        if let Some(eye) = self.size.eye_height {
            finite_in("size.eye_height", eye, 0.0, limits::MAX_SIZE)?;
        }
        self.check_hitboxes()?;
        self.check_attributes()?;
        self.check_synced()?;
        self.check_ai()?;
        self.check_model()?;
        self.check_rest()?;
        Ok(())
    }

    fn check_hitboxes(&self) -> Result<(), String> {
        too_many("hitboxes", self.hitboxes.len(), limits::MAX_HITBOXES)?;
        let mut names = BTreeSet::new();
        for h in &self.hitboxes {
            if !is_name(&h.name) || !names.insert(h.name.as_str()) {
                return Err(format!(
                    "hitbox {:?}: a name of up to 48 [A-Za-z0-9_.-], used once",
                    h.name
                ));
            }
            finite_in(
                &format!("hitbox {} width", h.name),
                h.size[0],
                0.0001,
                limits::MAX_SIZE,
            )?;
            finite_in(
                &format!("hitbox {} height", h.name),
                h.size[1],
                0.0001,
                limits::MAX_SIZE,
            )?;
            for v in h.offset {
                finite_in(&format!("hitbox {} offset", h.name), v, -64.0, 64.0)?;
            }
            finite_in(
                &format!("hitbox {} damage_multiplier", h.name),
                h.damage_multiplier,
                0.0,
                100.0,
            )?;
            if h.bone.as_deref().is_some_and(|b| !is_name(b)) {
                return Err(format!("hitbox {}: bone is a bone name", h.name));
            }
        }
        Ok(())
    }

    fn check_attributes(&self) -> Result<(), String> {
        too_many("attributes", self.attributes.len(), limits::MAX_ATTRIBUTES)?;
        for (name, value) in &self.attributes {
            let bare = name.strip_prefix("minecraft:").unwrap_or(name);
            let builtin = BUILTIN_ATTRIBUTES.contains(&bare);
            match value {
                Attribute::Value(v) => {
                    if !builtin && !YOAKE_ATTRIBUTES.contains(&name.as_str()) {
                        return Err(format!(
                            "attribute {name:?} is not built in: declare a custom one as {{\"default\": ..}} under a namespaced id"
                        ));
                    }
                    finite_in(&format!("attribute {name}"), *v, -1.0e6, 1.0e6)?;
                }
                Attribute::Custom(c) => {
                    if builtin || !is_id(name) {
                        return Err(format!(
                            "custom attribute {name:?} is a namespaced id (<mod>:<name>)"
                        ));
                    }
                    finite_in(
                        &format!("attribute {name} default"),
                        c.default,
                        -1.0e9,
                        1.0e9,
                    )?;
                    for bound in [c.min, c.max].into_iter().flatten() {
                        finite_in(&format!("attribute {name} bound"), bound, -1.0e9, 1.0e9)?;
                    }
                    if c.min.zip(c.max).is_some_and(|(lo, hi)| lo > hi)
                        || c.min.is_some_and(|lo| c.default < lo)
                        || c.max.is_some_and(|hi| c.default > hi)
                    {
                        return Err(format!("attribute {name}: min <= default <= max"));
                    }
                    one_of(
                        &format!("attribute {name} sync"),
                        &c.sync,
                        &["all", "owner", "none"],
                    )?;
                }
            }
        }
        Ok(())
    }

    fn check_synced(&self) -> Result<(), String> {
        too_many("synced values", self.synced.len(), limits::MAX_SYNCED)?;
        let mut bytes = 0;
        for (name, s) in &self.synced {
            if !is_name(name) {
                return Err(format!(
                    "synced {name:?}: a name of up to 48 [A-Za-z0-9_.-]"
                ));
            }
            one_of(&format!("synced {name} type"), &s.kind, &SYNCED_TYPES)?;
            one_of(
                &format!("synced {name} sync"),
                &s.sync,
                &["all", "owner", "none"],
            )?;
            if let Some(rate) = s.sync_rate
                && !(1..=20).contains(&rate)
            {
                return Err(format!("synced {name}: sync_rate is 1 to 20"));
            }
            let default = s.default.as_ref();
            let bad_default = || format!("synced {name}: default does not fit type {}", s.kind);
            match s.kind.as_str() {
                "bool" => {
                    if default.is_some_and(|d| !d.is_boolean()) {
                        return Err(bad_default());
                    }
                }
                "i32" => {
                    let (lo, hi) = (s.min.unwrap_or(i32::MIN), s.max.unwrap_or(i32::MAX));
                    if lo > hi {
                        return Err(format!("synced {name}: min <= max"));
                    }
                    if let Some(d) = default {
                        let v = d.as_i64().ok_or_else(bad_default)?;
                        if v < i64::from(lo) || v > i64::from(hi) {
                            return Err(bad_default());
                        }
                    }
                }
                "f32" => {
                    if default.is_some_and(|d| !d.as_f64().is_some_and(f64::is_finite)) {
                        return Err(bad_default());
                    }
                }
                "string" => {
                    if default.is_some_and(|d| {
                        d.as_str()
                            .is_none_or(|t| t.len() > limits::MAX_SYNCED_STRING)
                    }) {
                        return Err(bad_default());
                    }
                }
                "vec3" => {
                    if default.is_some_and(|d| {
                        d.as_array().is_none_or(|a| {
                            a.len() != 3
                                || !a.iter().all(|v| v.as_f64().is_some_and(f64::is_finite))
                        })
                    }) {
                        return Err(bad_default());
                    }
                }
                "enum" => {
                    if s.values.is_empty() || s.values.len() > limits::MAX_ENUM_VALUES {
                        return Err(format!(
                            "synced {name}: an enum has 1 to {} values",
                            limits::MAX_ENUM_VALUES
                        ));
                    }
                    let distinct: BTreeSet<_> = s.values.iter().collect();
                    if distinct.len() != s.values.len() || !s.values.iter().all(|v| is_name(v)) {
                        return Err(format!("synced {name}: enum values are distinct names"));
                    }
                    if let Some(d) = default
                        && !d.as_str().is_some_and(|t| s.values.iter().any(|v| v == t))
                    {
                        return Err(bad_default());
                    }
                }
                _ => {}
            }
            if s.kind != "enum" && !s.values.is_empty() {
                return Err(format!("synced {name}: values are for enums"));
            }
            if s.kind != "i32" && (s.min.is_some() || s.max.is_some()) {
                return Err(format!("synced {name}: min and max are for i32"));
            }
            bytes += s.wire_bytes();
        }
        if bytes > limits::MAX_SYNCED_BYTES {
            return Err(format!(
                "synced values take {bytes} bytes, at most {}",
                limits::MAX_SYNCED_BYTES
            ));
        }
        Ok(())
    }

    fn check_ai(&self) -> Result<(), String> {
        one_of("ai.navigator", &self.ai.navigator, &NAVIGATORS)?;
        too_many(
            "ai goals, targets and script goals",
            self.ai.goals.len() + self.ai.targets.len() + self.ai.script_goals.len(),
            limits::MAX_GOALS,
        )?;
        let mut script_ids = BTreeSet::new();
        for g in &self.ai.script_goals {
            if !is_name(&g.id) || !script_ids.insert(g.id.as_str()) {
                return Err(format!("script goal {:?}: a name, used once", g.id));
            }
            if !(0..=32).contains(&g.priority) {
                return Err(format!("script goal {}: priority is 0 to 32", g.id));
            }
            if !(1..=200).contains(&g.interval) {
                return Err(format!("script goal {}: interval is 1 to 200 ticks", g.id));
            }
            let mut flags = BTreeSet::new();
            for f in &g.flags {
                one_of(&format!("script goal {} flag", g.id), f, &GOAL_FLAGS)?;
                flags.insert(f);
            }
            if flags.len() != g.flags.len() {
                return Err(format!("script goal {}: a flag is listed twice", g.id));
            }
        }
        for g in self.ai.goals.iter().chain(&self.ai.targets) {
            one_of("goal type", &g.kind, &GOAL_TYPES)?;
            if !(0..=32).contains(&g.priority) {
                return Err(format!("goal {}: priority is 0 to 32", g.kind));
            }
            if let Some(speed) = g.speed {
                finite_in(&format!("goal {} speed", g.kind), speed, 0.0, 16.0)?;
            }
            if g.kind == "script" {
                if !g.id.as_deref().is_some_and(|id| script_ids.contains(id)) {
                    return Err("a script goal names one of ai.script_goals by id".into());
                }
            } else if g.id.is_some() {
                return Err(format!("goal {}: id is for script goals", g.kind));
            }
        }
        Ok(())
    }

    fn check_model(&self) -> Result<(), String> {
        let m = &self.model;
        finite_in("model.scale", m.scale, 0.01, 64.0)?;
        too_many(
            "model.animations",
            m.animations.len(),
            limits::MAX_ANIMATION_FILES,
        )?;
        too_many(
            "model.controllers",
            m.controllers.len(),
            limits::MAX_ANIMATION_FILES,
        )?;
        too_many("model.layers", m.layers.len(), limits::MAX_LAYERS)?;
        too_many("model.lod", m.lod.len(), limits::MAX_LOD)?;
        let ids = m
            .geo
            .iter()
            .chain(m.textures.default.iter())
            .chain(&m.textures.variants)
            .chain(&m.animations)
            .chain(&m.controllers)
            .chain(m.layers.iter().filter_map(|l| l.texture.as_ref()));
        for id in ids {
            if !is_id(id) {
                return Err(format!("model: {id:?} is not an id (<namespace>:<path>)"));
            }
        }
        if let Some(s) = &m.shadow {
            finite_in("model.shadow.radius", s.radius, 0.0, 16.0)?;
            finite_in("model.shadow.strength", s.strength, 0.0, 1.0)?;
        }
        if let Some(c) = &m.cull {
            finite_in("model.cull.width", c.width, 0.0, 64.0)?;
            finite_in("model.cull.height", c.height, 0.0, 64.0)?;
            finite_in("model.cull.y_offset", c.y_offset, -64.0, 64.0)?;
        }
        let mut names = BTreeSet::new();
        for l in &m.layers {
            if !is_name(&l.name) || !names.insert(l.name.as_str()) {
                return Err(format!("layer {:?}: a name, used once", l.name));
            }
            one_of(&format!("layer {} mode", l.name), &l.mode, &LAYER_MODES)?;
        }
        let mut last = -1.0;
        for l in &m.lod {
            one_of("lod mode", &l.mode, &LOD_MODES)?;
            finite_in("lod distance", l.distance, 0.0, 1024.0)?;
            if l.distance <= last {
                return Err("lod distances rise".into());
            }
            last = l.distance;
        }
        for slot in m.item_slots.keys() {
            one_of("item_slots key", slot, &["mainhand", "offhand", "head"])?;
        }
        if !m.layers.is_empty() && m.geo.is_none() {
            return Err("model.layers need model.geo".into());
        }
        Ok(())
    }

    fn check_rest(&self) -> Result<(), String> {
        too_many("spawn rules", self.spawn.len(), limits::MAX_SPAWN_RULES)?;
        for s in &self.spawn {
            if !(1..=1000).contains(&s.weight) || s.min < 1 || s.min > s.max || s.max > 32 {
                return Err("spawn: weight is 1 to 1000 and min <= max within 1 to 32".into());
            }
            if let Some(p) = &s.placement {
                one_of("spawn placement", p, &PLACEMENTS)?;
            }
            if s.y_range.is_some_and(|[lo, hi]| lo > hi) {
                return Err("spawn y_range: low <= high".into());
            }
        }
        for (slot, id) in &self.sounds {
            one_of("sounds key", slot, &SOUND_SLOTS)?;
            if !is_id(id) {
                return Err(format!("sound {slot}: {id:?} is not a sound event id"));
            }
        }
        if let Some(t) = &self.loot_table
            && !is_id(t)
        {
            return Err(format!("loot_table {t:?} is not an id"));
        }
        if let Some(p) = &self.proxy
            && !p.starts_with("minecraft:")
        {
            return Err(format!("proxy {p:?} is a vanilla entity (minecraft:...)"));
        }
        if self.proxy.is_none() && self.proxy_name_tag.is_some() {
            return Err("proxy_name_tag needs proxy".into());
        }
        if let Some(t) = &self.tameable {
            finite_in("tameable.chance", t.chance, 0.0, 1.0)?;
            if t.items.is_empty() {
                return Err("tameable.items lists at least one item".into());
            }
        }
        if let Some(b) = &self.breedable {
            if b.food.is_empty() {
                return Err("breedable.food lists at least one item".into());
            }
            if let Some(s) = b.baby_scale {
                finite_in("breedable.baby_scale", s, 0.05, 1.0)?;
            }
        }
        if let Some(i) = &self.saddle_item
            && !is_id(i)
        {
            return Err(format!("saddle_item {i:?} is not an item id"));
        }
        too_many("interact", self.interact.len(), 16)?;
        for row in &self.interact {
            one_of("interact action", &row.action, &INTERACT_ACTIONS)?;
            if row.action == "give_item" && row.give.as_deref().is_none_or(|g| !is_id(g)) {
                return Err("an interact give_item row names the item it gives".into());
            }
            if let Some(n) = row.count
                && !(1..=64).contains(&n)
            {
                return Err("interact count is 1 to 64".into());
            }
            if row.action == "ride" && self.seats.is_empty() {
                return Err("an interact ride row needs seats".into());
            }
        }
        too_many("seats", self.seats.len(), limits::MAX_SEATS)?;
        for seat in &self.seats {
            if seat.offset.is_none() == seat.locator.is_none() {
                return Err("a seat has an offset or a locator".into());
            }
            if let Some(c) = &seat.controls {
                one_of("seat controls", c, &["none", "player"])?;
            }
        }
        Ok(())
    }

    /// The files this type refers to inside its mod, as the candidates of each reference (any one
    /// of the candidates must exist). `own` tells whether a namespace belongs to the mod.
    pub fn referenced_files(&self, own: impl Fn(&str) -> bool) -> Result<Vec<Vec<String>>, String> {
        let mut out = Vec::new();
        let mut path = |id: &str, dir: &str, ext: &[&str]| -> Result<(), String> {
            let (ns, rest) = id
                .split_once(':')
                .ok_or_else(|| format!("{id:?} is not an id"))?;
            if !own(ns) {
                return Err(format!("{id:?} is not in a namespace of this mod"));
            }
            out.push(
                ext.iter()
                    .map(|e| format!("assets/{ns}/{dir}{rest}{e}"))
                    .collect(),
            );
            Ok(())
        };
        let m = &self.model;
        if let Some(geo) = &m.geo {
            // `<ns>:geo/<name>`: the folder is part of the id.
            path(geo, "", &[".geo.json", ".bbmodel"])?;
        }
        for t in m.textures.default.iter().chain(&m.textures.variants) {
            path(t, "textures/", &[".png"])?;
        }
        for l in &m.layers {
            if let Some(t) = &l.texture {
                path(t, "textures/", &[".png"])?;
            }
        }
        for a in &m.animations {
            path(a, "", &[".animation.json"])?;
        }
        for c in &m.controllers {
            path(c, "", &[".json"])?;
        }
        Ok(out)
    }

    /// The tracked values' names in slot order (the order of `synced`, which is by name).
    pub fn synced_names(&self) -> Vec<&str> {
        self.synced.keys().map(String::as_str).collect()
    }

    /// Reads the model, animation and controller files through `horizon-mobmodel`'s loaders (`read`
    /// returns a content file's bytes by its path in the mod, `assets/<ns>/...`), counts the
    /// animations, then runs [`validate_model`](Self::validate_model) on the result. A type
    /// without `model.geo` is checked against an empty model, so a hitbox `bone` fails there.
    pub fn load_model(
        &self,
        read: &mut dyn FnMut(&str) -> Result<Vec<u8>, String>,
    ) -> Result<LoadedModel, String> {
        use horizon_mobmodel::{anim, controller, expr::Symbols, load_bbmodel, load_geo_json};
        let file = |id: &str, dir_ext: &str| -> String {
            let (ns, rest) = id.split_once(':').unwrap_or_default();
            format!("assets/{ns}/{rest}{dir_ext}")
        };
        let model = match &self.model.geo {
            None => None,
            Some(geo) => {
                let json = file(geo, ".geo.json");
                Some(match read(&json) {
                    Ok(bytes) => load_geo_json(&json, &bytes).map_err(|e| e.to_string())?,
                    Err(_) => {
                        let bb = file(geo, ".bbmodel");
                        let bytes = read(&bb)?;
                        load_bbmodel(&bb, &bytes).map_err(|e| e.to_string())?
                    }
                })
            }
        };
        let mut syms = Symbols::new(&self.synced_names());
        for (name, synced) in &self.synced {
            if synced.kind == "enum" {
                let values: Vec<&str> = synced.values.iter().map(String::as_str).collect();
                syms.set_enum_values(name, &values);
            }
        }
        let mut animations = 0;
        let mut library = anim::AnimationLibrary::new();
        let mut layers = Vec::new();
        for id in &self.model.animations {
            let path = file(id, ".animation.json");
            let text = String::from_utf8(read(&path)?).map_err(|e| format!("{path}: {e}"))?;
            let parsed =
                anim::parse_animation_file(&path, &text, &mut syms).map_err(|e| e.to_string())?;
            animations += parsed.animations.len();
            for a in parsed.animations {
                library.add(a).map_err(|e| format!("{path}: {e}"))?;
            }
        }
        for id in &self.model.controllers {
            let path = file(id, ".json");
            let text = String::from_utf8(read(&path)?).map_err(|e| format!("{path}: {e}"))?;
            let parsed = controller::parse_controllers(&path, &text, &mut syms).map_err(|e| e.to_string())?;
            layers.extend(parsed.controllers);
        }
        let mut loaded = LoadedModel {
            model,
            animations,
            rig: None,
            events: None,
        };
        self.validate_model(&loaded)?;
        // The rig the server runs: skeleton, animations and controllers bound by name, and the
        // type's typed events (`server_events` picks the animations whose timeline the server
        // runs). A type that lists neither animations nor controllers has none.
        if let Some(model) = loaded.model.as_ref()
            && (!library.is_empty() || !layers.is_empty())
        {
            let (rig, _warnings) =
                horizon_mobmodel::pose::Rig::new(model.skeleton.clone(), library, layers)
                    .map_err(|e| format!("{}: {e}", self.id))?;
            let mut options = serde_json::Map::new();
            options.insert(
                "particles".into(),
                serde_json::to_value(&self.model.particles).map_err(|e| e.to_string())?,
            );
            if let Some(flags) = &self.model.server_events {
                options.insert("server_events".into(), flags.clone());
            }
            let animation_files = self.model.animations.join(", ");
            let events = horizon_mobmodel::events::MobEvents::from_model_json(
                &self.id,
                &animation_files,
                &serde_json::Value::Object(options),
                rig.animations(),
                &model.locators,
            )
            .map_err(|e| e.to_string())?;
            loaded.rig = Some(rig);
            loaded.events = Some(events);
        }
        Ok(loaded)
    }

    /// The checks that need the loaded model: counts, hitbox bones, locators, layer bones.
    pub fn validate_model(&self, model: &dyn ModelInfo) -> Result<(), String> {
        too_many("bones", model.bone_count(), limits::MAX_BONES)?;
        too_many("cubes", model.cube_count(), limits::MAX_CUBES)?;
        too_many(
            "animations",
            model.animation_count(),
            limits::MAX_ANIMATIONS,
        )?;
        for h in &self.hitboxes {
            if let Some(bone) = &h.bone
                && model.bone_index(bone).is_none()
            {
                return Err(format!("hitbox {}: the model has no bone {bone:?}", h.name));
            }
        }
        for l in &self.model.layers {
            for bone in &l.bones {
                if model.bone_index(bone).is_none() {
                    return Err(format!("layer {}: the model has no bone {bone:?}", l.name));
                }
            }
        }
        for (slot, locator) in &self.model.item_slots {
            if !model.has_locator(locator) {
                return Err(format!(
                    "item slot {slot}: the model has no locator {locator:?}"
                ));
            }
        }
        for seat in &self.seats {
            if let Some(l) = &seat.locator
                && !model.has_locator(l)
            {
                return Err(format!("seat: the model has no locator {l:?}"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FOX: &str = include_str!(
        "../../../tools/fixtures/cinder_fox/data/sample_mobs/horizon/entity_type/cinder_fox.json"
    );

    fn parse(json: &str) -> Result<CustomMobFile, String> {
        CustomMobFile::parse("sample_mobs:cinder_fox", json.as_bytes())
    }
    fn fox() -> serde_json::Value {
        serde_json::from_str(FOX).unwrap()
    }
    fn with(edit: impl FnOnce(&mut serde_json::Value)) -> Result<CustomMobFile, String> {
        let mut v = fox();
        edit(&mut v);
        parse(&v.to_string())
    }

    #[test]
    fn the_cinder_fox_fixture_is_accepted() {
        let file = parse(FOX).unwrap();
        assert_eq!(file.hitboxes.len(), 2);
        assert_eq!(file.synced.len(), 2);
        assert!(file.ai.script_goals.iter().any(|g| g.id == "pounce"));
        assert_eq!(file.proxy.as_deref(), Some("minecraft:fox"));
        assert_eq!(
            file.spawn_egg,
            Some(SpawnEgg::Generated {
                primary: Rgb(0xD2691E),
                secondary: Rgb(0xFFF0A0)
            })
        );
        // The files it refers to are all in the fixture.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fixtures/cinder_fox");
        for candidates in file.referenced_files(|ns| ns == "sample_mobs").unwrap() {
            assert!(
                candidates.iter().any(|c| root.join(c).is_file()),
                "{candidates:?}"
            );
        }
    }

    /// F4M-4c: `saddle_item`, `rideable_when` and the `interact` rows are accepted and checked, and the
    /// locator of a seat is found at its rest position in the loaded model.
    #[test]
    fn saddle_rideable_when_and_interact_rows_are_validated() {
        use serde_json::json;
        let file = with(|v| {
            v["seats"] = json!([{"offset": [0, 1, 0], "controls": "player"}]);
            v["saddle_item"] = json!("minecraft:saddle");
            v["rideable_when"] = json!("query.health > 3");
            v["interact"] = json!([
                {"action": "give_item", "item": "minecraft:bowl", "give": "minecraft:mushroom_stew", "count": 2},
                {"action": "ride", "sneaking": false},
                {"action": "feed"}
            ]);
        })
        .unwrap();
        assert_eq!(file.saddle_item.as_deref(), Some("minecraft:saddle"));
        assert_eq!(file.rideable_when.as_deref(), Some("query.health > 3"));
        assert_eq!(file.interact.len(), 3);
        assert_eq!(file.interact[0].count, Some(2));
        // Plain files have none of it.
        let plain = parse(FOX).unwrap();
        assert!(plain.saddle_item.is_none() && plain.interact.is_empty());
        for (what, edit) in [
            ("a bad action", json!([{"action": "dance"}])),
            ("give_item without an item to give", json!([{"action": "give_item", "item": "minecraft:bowl"}])),
            ("a give id that is no id", json!([{"action": "give_item", "give": "stew"}])),
            ("a count over 64", json!([{"action": "give_item", "give": "a:b", "count": 65}])),
            ("ride without seats", json!([{"action": "ride"}])),
            ("an unknown key", json!([{"action": "feed", "zzz": 1}])),
        ] {
            assert!(with(|v| v["interact"] = edit).is_err(), "{what}");
        }
        assert!(with(|v| v["saddle_item"] = json!("saddle")).is_err(), "a saddle that is no id");
    }

    #[test]
    fn the_minimal_file_and_defaults() {
        let file = parse(r#"{"template":"yoake:mob","size":{"width":1,"height":1}}"#).unwrap();
        assert_eq!(file.category, "creature");
        assert_eq!(file.ai.navigator, "ground");
        assert_eq!(file.model.scale, 1.0);
        assert!(file.hitboxes.is_empty() && file.synced.is_empty());
    }

    /// Every goal type of the vocabulary (but the script one, which names a script goal) is
    /// accepted with free parameters, `leap_attack` among them (F4M-4a).
    #[test]
    fn every_goal_type_is_accepted_with_its_own_parameters() {
        for kind in GOAL_TYPES.iter().filter(|k| **k != "script") {
            let row = serde_json::json!({
                "type": kind, "priority": 3, "speed": 1.0,
                "trigger": "a", "damage_frame": "hit", "damage_delay": 8, "range": 4
            });
            with(|v| v["ai"]["goals"].as_array_mut().unwrap().push(row)).expect(kind);
        }
        assert!(GOAL_TYPES.contains(&"leap_attack"));
    }


    /// `water_slowdown` is not a 26.3 attribute: it is `yoake:water_slowdown` now (a plain number).
    #[test]
    fn water_slowdown_is_a_yoake_attribute_and_not_a_builtin_one() {
        assert!(!BUILTIN_ATTRIBUTES.contains(&"water_slowdown"));
        assert_eq!(BUILTIN_ATTRIBUTES.len(), 15);
        let bare = r#"{"template":"yoake:mob","size":{"width":1,"height":1},"attributes":{"water_slowdown":0.9}}"#;
        assert!(parse(bare).is_err(), "the old bare name is refused");
        let yoake = r#"{"template":"yoake:mob","size":{"width":1,"height":1},"attributes":{"yoake:water_slowdown":0.9}}"#;
        let file = parse(yoake).unwrap();
        assert_eq!(file.attributes["yoake:water_slowdown"], Attribute::Value(0.9));
        // Other namespaced plain numbers are still refused.
        let other = r#"{"template":"yoake:mob","size":{"width":1,"height":1},"attributes":{"yoake:thing":0.9}}"#;
        assert!(parse(other).is_err());
    }

    /// The vanilla `axolotls` category is offered too.
    #[test]
    fn the_axolotls_category_is_accepted() {
        assert!(CATEGORIES.contains(&"axolotls"));
        let file = with(|v| v["category"] = "axolotls".into()).unwrap();
        assert_eq!(file.category, "axolotls");
    }
    #[test]
    fn malformed_files_are_rejected_with_a_reason() {
        let cases: Vec<(&str, Box<dyn FnOnce(&mut serde_json::Value)>)> = vec![
            (
                "template",
                Box::new(|v| v["template"] = "minecraft:pig".into()),
            ),
            ("format", Box::new(|v| v["format"] = 2.into())),
            ("category", Box::new(|v| v["category"] = "pet".into())),
            ("width", Box::new(|v| v["size"]["width"] = (-1).into())),
            ("unknown", Box::new(|v| v["colour"] = "red".into())),
            (
                "navigator",
                Box::new(|v| v["ai"]["navigator"] = "teleport".into()),
            ),
            (
                "goal type",
                Box::new(|v| v["ai"]["goals"][0]["type"] = "explode".into()),
            ),
            (
                "attribute",
                Box::new(|v| v["attributes"]["luck"] = 1.into()),
            ),
            (
                "proxy",
                Box::new(|v| v["proxy"] = "sample_mobs:other".into()),
            ),
            (
                "script",
                Box::new(|v| {
                    v["ai"]["goals"]
                        .as_array_mut()
                        .unwrap()
                        .push(serde_json::json!({"type":"script","id":"missing"}))
                }),
            ),
            (
                "bone",
                Box::new(|v| v["hitboxes"][0]["bone"] = "bad bone".into()),
            ),
        ];
        for (what, edit) in cases {
            let error = with(edit).expect_err(what);
            assert!(!error.is_empty(), "{what}");
        }
    }

    #[test]
    fn the_limits_hold() {
        // 33 synced values.
        let error = with(|v| {
            for i in 0..33 {
                v["synced"][format!("v{i}")] = serde_json::json!({"type":"bool"});
            }
        })
        .unwrap_err();
        assert!(error.contains("32"), "{error}");
        // Too many bytes: 22 vec3 = 264 bytes.
        let error = with(|v| {
            v["synced"] = serde_json::json!({});
            for i in 0..22 {
                v["synced"][format!("v{i}")] = serde_json::json!({"type":"vec3"});
            }
        })
        .unwrap_err();
        assert!(error.contains("256"), "{error}");
        // A synced string default over 64 bytes.
        assert!(
            with(|v| {
                v["synced"]["name"] = serde_json::json!({"type":"string","default":"x".repeat(65)})
            })
            .is_err()
        );
        // Too many hitboxes.
        assert!(
            with(|v| {
                v["hitboxes"] = (0..17)
                    .map(|i| serde_json::json!({"name": format!("p{i}"), "size": [0.2, 0.2]}))
                    .collect::<Vec<_>>()
                    .into()
            })
            .is_err()
        );
        // The numbers the model loaders enforce.
        assert_eq!(limits::MAX_BONES, 256);
        assert_eq!(limits::MAX_CUBES, 4096);
        assert_eq!(limits::MAX_ANIMATIONS, 128);
        assert_eq!(limits::MAX_SYNCED, 32);
        assert_eq!(limits::MAX_EXPRESSION_INSTRUCTIONS, 256);
    }

    struct Fake;
    impl ModelInfo for Fake {
        fn bone_count(&self) -> usize {
            14
        }
        fn cube_count(&self) -> usize {
            20
        }
        fn animation_count(&self) -> usize {
            6
        }
        fn bone_index(&self, name: &str) -> Option<u16> {
            ["root", "body", "head"]
                .iter()
                .position(|b| *b == name)
                .map(|i| i as u16)
        }
        fn has_locator(&self, name: &str) -> bool {
            name == "mouth"
        }
    }
    struct Huge(usize);
    impl ModelInfo for Huge {
        fn bone_count(&self) -> usize {
            self.0
        }
        fn cube_count(&self) -> usize {
            1
        }
        fn animation_count(&self) -> usize {
            1
        }
        fn bone_index(&self, _: &str) -> Option<u16> {
            Some(0)
        }
        fn has_locator(&self, _: &str) -> bool {
            true
        }
    }

    #[test]
    fn the_model_checks_use_the_loaded_model() {
        let file = parse(FOX).unwrap();
        file.validate_model(&Fake).unwrap();
        assert!(file.validate_model(&Huge(257)).is_err());
        let moved = with(|v| v["hitboxes"][1]["bone"] = "tail".into()).unwrap();
        assert!(moved.validate_model(&Fake).unwrap_err().contains("tail"));
    }

    /// The fixture's real model, animations and controllers go through the real loaders.
    #[test]
    fn the_fixture_model_loads_through_the_real_loader() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fixtures/cinder_fox");
        let mut read = |name: &str| std::fs::read(root.join(name)).map_err(|e| e.to_string());
        let file = parse(FOX).unwrap();
        let loaded = file.load_model(&mut read).unwrap();
        assert!(loaded.bone_count() >= 14, "{}", loaded.bone_count());
        assert!(loaded.cube_count() > 0);
        assert!(loaded.animation_count() > 0);
        assert!(loaded.bone_index("head").is_some());
        assert!(loaded.has_locator("mouth"));
        // A hitbox on a bone the model lacks is refused with the real bone table.
        let moved = with(|v| v["hitboxes"][1]["bone"] = "tail_tip_missing".into()).unwrap();
        assert!(
            moved
                .load_model(&mut read)
                .unwrap_err()
                .contains("tail_tip_missing")
        );
        // No geo: an empty model (the schema refuses a bone reference without one).
        let none = with(|v| {
            v["model"] = serde_json::json!({});
            v["hitboxes"][1].as_object_mut().unwrap().remove("bone");
        })
        .unwrap();
        let empty = none.load_model(&mut read).unwrap();
        assert_eq!(empty.bone_count(), 0);
        // A broken model file is the loader's error.
        let mut broken = |name: &str| {
            if name.ends_with(".geo.json") {
                Ok(b"{ not json".to_vec())
            } else {
                read(name)
            }
        };
        assert!(file.load_model(&mut broken).is_err());
    }

    /// The content root keeps the file as JSON (`CUSTOM_MOBS_FILE`): it reads back the same.
    #[test]
    fn the_file_round_trips_through_its_serialization() {
        let file = parse(FOX).unwrap();
        let mut v = serde_json::to_value(&file).unwrap();
        let id = v.as_object_mut().unwrap().remove("id").unwrap();
        assert_eq!(id, "sample_mobs:cinder_fox");
        let back = CustomMobFile::parse("sample_mobs:cinder_fox", v.to_string().as_bytes()).unwrap();
        assert_eq!(back, file);
    }

    #[test]
    fn a_reference_outside_the_mod_is_refused() {
        let file = with(|v| v["model"]["textures"]["default"] = "other:entity/x".into()).unwrap();
        assert!(file.referenced_files(|ns| ns == "sample_mobs").is_err());
    }

    #[test]
    fn colours_parse_both_ways() {
        let c = |s: &str| serde_json::from_str::<Rgb>(s);
        assert_eq!(c("\"0xD2691E\"").unwrap(), Rgb(0xD2691E));
        assert_eq!(c("\"#fff0a0\"").unwrap(), Rgb(0xFFF0A0));
        assert_eq!(c("255").unwrap(), Rgb(255));
        assert!(c("\"red\"").is_err());
        assert!(c("\"0x1000000\"").is_err());
    }

    #[test]
    fn the_fixture_mod_loads_as_content_and_a_missing_file_is_refused() {
        use crate::{content::Content, manifest::Manifest};
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tools/fixtures/cinder_fox");
        let manifest = Manifest::parse(
            &std::fs::read_to_string(root.join(crate::manifest::MANIFEST)).unwrap(),
        )
        .unwrap();
        let content = Content::read(&root, &manifest).unwrap();
        assert_eq!(content.custom_mobs.len(), 1);
        assert_eq!(content.custom_mobs[0].id, "sample_mobs:cinder_fox");
        assert!(content.entity_types.is_empty());
        // The generated spawn egg is an item of the mod, in the colours' model.
        let egg = content
            .items
            .iter()
            .find(|i| i.id == spawn_egg_item("sample_mobs:cinder_fox"))
            .expect("the generated spawn egg");
        assert_eq!(egg.template, crate::content::GENERATED_EGG_TEMPLATE);
        assert_eq!(
            egg.components["minecraft:item_model"],
            "sample_mobs:cinder_fox_spawn_egg"
        );
        // The same mod without its model file.
        let scratch = std::env::temp_dir().join(format!("f4m0-fixture-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        copy_dir(&root, &scratch);
        std::fs::remove_file(scratch.join("assets/sample_mobs/geo/cinder_fox.geo.json")).unwrap();
        std::fs::remove_file(scratch.join("assets/sample_mobs/geo/cinder_fox.bbmodel")).unwrap();
        let error = Content::read(&scratch, &manifest).unwrap_err();
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(error.contains("geo/cinder_fox"), "{error}");
    }

    fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let target = to.join(entry.file_name());
            if entry.file_type().unwrap().is_dir() {
                copy_dir(&entry.path(), &target);
            } else {
                std::fs::copy(entry.path(), target).unwrap();
            }
        }
    }

    #[test]
    fn the_reserved_id_ranges_are_in_the_locks() {
        use reserved_ids::*;
        let meta: serde_json::Value =
            serde_json::from_str(include_str!("../../../hook-ledger/meta.json")).unwrap();
        // The hook ledger hands out ids from `next_id`: past the reserved range.
        assert!(meta["next_id"].as_u64().unwrap() > u64::from(HOOK_LAST));
        for ledger in [
            include_str!("../../../hook-ledger/horizon_client.json"),
            include_str!("../../../hook-ledger/horizon_client_lib.json"),
        ] {
            let ledger: serde_json::Value = serde_json::from_str(ledger).unwrap();
            for entry in ledger["entries"].as_array().unwrap() {
                let id = entry["id"].as_u64().unwrap() as u32;
                assert!(
                    !(HOOK_FIRST..=HOOK_LAST).contains(&id),
                    "ledger id {id} is reserved"
                );
            }
        }
        let lock: serde_json::Value =
            serde_json::from_str(include_str!("../../yoake-bindgen/ids.lock.json")).unwrap();
        let bindings = lock["bindings"].as_array().unwrap();
        let sentinel = bindings
            .iter()
            .find(|b| b["path"] == BINDING_SENTINEL)
            .expect("the sentinel binding");
        assert_eq!(sentinel["id"].as_u64().unwrap(), u64::from(BINDING_LAST));
        assert_eq!(sentinel["status"], "unbound");
        // Nothing else lives in the reserved range, and the generator's next id is past it.
        for b in bindings {
            let id = b["id"].as_u64().unwrap() as u32;
            // ids past the sentinel are new bindings the generator handed out after the range (max + 1)
            assert!(!(BINDING_FIRST..BINDING_LAST).contains(&id), "binding id {id} is reserved");
        }
    }
}
