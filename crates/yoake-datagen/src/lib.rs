//! Data generation for a Yoake content mod (REM-18): typed builders for the JSON a content
//! mod's folder holds, written by a small program in the mod's `datagen/` crate and run by
//! `cargo horizon-mod datagen` (docs/mods.md, "Data generation").
//!
//! The paths are the ones `horizon-mods` reads (`crates/horizon-mods/src/content.rs`):
//! `assets/<id>/{blockstates,models,items,lang}/...` and `data/<id>/{recipe,loot_table,tags}/...`
//! for vanilla's files, `data/<id>/horizon/{block,item,enchantment,damage_type,game_rule,fluid}/`
//! for the mod's own declarations. Output is deterministic (sorted keys, two-space indent), so
//! `--check` can compare it with the files on disk.
//!
//! A datagen program:
//!
//! ```ignore
//! fn main() -> std::process::ExitCode {
//!     yoake_datagen::main(|m| {
//!         m.block("ember_ore", &BlockDef::new("minecraft:iron_ore").hardness(3.0));
//!         m.blockstate("ember_ore", &BlockState::single("ember:block/ember_ore"));
//!         m.model("block/ember_ore", &Model::cube_all("minecraft:block/iron_ore"));
//!     })
//! }
//! ```
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The files a generator produced, by their path relative to the mod folder.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Output {
    files: BTreeMap<String, Vec<u8>>,
}

impl Output {
    /// Adds a file. Panics when the path is already there: two builders must not write one file.
    pub fn insert(&mut self, path: String, value: &Value) {
        let mut text = serde_json::to_string_pretty(value).expect("JSON values serialize");
        text.push('\n');
        if self.files.insert(path.clone(), text.into_bytes()).is_some() {
            panic!("datagen writes {path} twice");
        }
    }

    /// The generated files, in path order.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files.iter().map(|(p, b)| (p.as_str(), b.as_slice()))
    }

    /// The text of one generated file.
    pub fn text(&self, path: &str) -> Option<&str> {
        self.files
            .get(path)
            .and_then(|b| std::str::from_utf8(b).ok())
    }

    /// Writes every file under `root`, creating folders. Returns the paths written.
    pub fn write(&self, root: &Path) -> Result<Vec<String>, String> {
        for (path, bytes) in &self.files {
            let to = root.join(path);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&to, bytes).map_err(|e| format!("{}: {e}", to.display()))?;
        }
        Ok(self.files.keys().cloned().collect())
    }

    /// Compares every generated file with the one under `root`. The error lists the paths that
    /// are missing or differ.
    pub fn check(&self, root: &Path) -> Result<(), Vec<String>> {
        let stale: Vec<String> = self
            .files
            .iter()
            .filter(|(path, bytes)| {
                std::fs::read(root.join(path)).map_or(true, |on_disk| on_disk != **bytes)
            })
            .map(|(path, _)| path.clone())
            .collect();
        if stale.is_empty() { Ok(()) } else { Err(stale) }
    }
}

/// A content mod's generator: the namespace `id` and the files it writes.
pub struct Mod {
    id: String,
    out: Output,
}

/// A vanilla or convention-tag reference or a mod's own id: `namespace:path`.
fn check_namespace(id: &str) {
    assert!(
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b)),
        "the mod id {id:?} is lower case letters, digits, _ . -"
    );
}

impl Mod {
    /// A generator for the mod `id` (its namespace).
    pub fn new(id: &str) -> Self {
        check_namespace(id);
        Self {
            id: id.to_owned(),
            out: Output::default(),
        }
    }

    /// The mod's namespace.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// What it generated so far.
    pub fn output(&self) -> &Output {
        &self.out
    }

    /// Ends generation and returns its files.
    pub fn into_output(self) -> Output {
        self.out
    }

    fn asset(&mut self, kind: &str, path: &str, value: &Value) {
        let file = format!("assets/{}/{kind}/{path}.json", self.id);
        self.out.insert(file, value);
    }

    fn data(&mut self, kind: &str, path: &str, value: &Value) {
        let file = format!("data/{}/{kind}/{path}.json", self.id);
        self.out.insert(file, value);
    }

    fn declaration(&mut self, kind: &str, path: &str, value: &Value) {
        let file = format!("data/{}/horizon/{kind}/{path}.json", self.id);
        self.out.insert(file, value);
    }

    /// `assets/<id>/blockstates/<name>.json`.
    pub fn blockstate(&mut self, name: &str, state: &BlockState) {
        self.asset("blockstates", name, &state.to_json());
    }

    /// `assets/<id>/models/<name>.json`, `name` like `block/ember_ore`.
    pub fn model(&mut self, name: &str, model: &Model) {
        self.asset("models", name, &model.to_json());
    }

    /// `assets/<id>/items/<name>.json`: the item model definition of the item `name`.
    pub fn item_model(&mut self, name: &str, model: &ItemModel) {
        self.asset("items", name, &model.to_json());
    }

    /// `assets/<id>/lang/<locale>.json`, the given translation keys and values.
    pub fn lang(&mut self, locale: Locale, entries: &[(&str, &str)]) {
        let map: Map<String, Value> = entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), Value::String((*v).to_owned())))
            .collect();
        let file = format!("assets/{}/lang/{}.json", self.id, locale.as_str());
        self.out.insert(file, &Value::Object(map));
    }

    /// `data/<id>/recipe/<name>.json`.
    pub fn recipe(&mut self, name: &str, recipe: &Recipe) {
        self.data("recipe", name, &recipe.to_json());
    }

    /// `data/<id>/loot_table/<name>.json`, `name` like `blocks/ember_ore`.
    pub fn loot_table(&mut self, name: &str, table: &LootTable) {
        self.data("loot_table", name, &table.to_json());
    }

    /// `data/<ns>/tags/<registry>/<path>.json` for the tag `id` (`minecraft:mineable/pickaxe`).
    /// The namespace is vanilla's, the convention tags' (`c`) or the mod's own.
    pub fn tag(&mut self, registry: TagRegistry, id: &str, tag: &Tag) {
        let (namespace, path) = id.split_once(':').expect("a tag id is namespace:path");
        let file = format!("data/{namespace}/tags/{}/{path}.json", registry.dir());
        self.out.insert(file, &tag.to_json());
    }

    /// `data/<id>/horizon/block/<name>.json`: a block of the mod, copying a vanilla template.
    pub fn block(&mut self, name: &str, block: &BlockDef) {
        self.declaration("block", name, &block.to_json());
    }

    /// `data/<id>/horizon/item/<name>.json`: an item of the mod, copying a vanilla template.
    pub fn item(&mut self, name: &str, item: &ItemDef) {
        self.declaration("item", name, &item.to_json());
    }

    /// `data/<id>/horizon/enchantment/<name>.json`.
    pub fn enchantment(&mut self, name: &str, enchantment: &Enchantment) {
        self.declaration("enchantment", name, &enchantment.to_json());
    }

    /// `data/<id>/horizon/damage_type/<name>.json`.
    pub fn damage_type(&mut self, name: &str, damage_type: &DamageType) {
        self.declaration("damage_type", name, &damage_type.to_json());
    }

    /// `data/<id>/horizon/game_rule/<name>.json`.
    pub fn game_rule(&mut self, name: &str, rule: &GameRule) {
        self.declaration("game_rule", name, &rule.to_json());
    }

    /// `data/<id>/horizon/fluid/<name>.json`.
    pub fn fluid(&mut self, name: &str, fluid: &Fluid) {
        self.declaration("fluid", name, &fluid.to_json());
    }
}

/// A language file's locale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locale {
    EnUs,
    JaJp,
}

impl Locale {
    /// Its file name's part, `en_us`.
    pub fn as_str(self) -> &'static str {
        match self {
            Locale::EnUs => "en_us",
            Locale::JaJp => "ja_jp",
        }
    }
}

/// A block state file: `variants`, a state's properties (`""` for none) and its model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockState {
    variants: Map<String, Value>,
}

impl BlockState {
    /// One variant with no properties, which uses `model` (`ember:block/ember_ore`).
    pub fn single(model: &str) -> Self {
        Self::default().variant("", model)
    }

    /// A variant for the state properties `properties` (`axis=y`, or `""` for none).
    pub fn variant(mut self, properties: &str, model: &str) -> Self {
        self.variants
            .insert(properties.to_owned(), json!({ "model": model }));
        self
    }

    pub fn to_json(&self) -> Value {
        json!({ "variants": Value::Object(self.variants.clone()) })
    }
}

/// A block or item model: its parent and its texture slots.
#[derive(Clone, Debug, PartialEq)]
pub struct Model {
    parent: String,
    textures: Map<String, Value>,
}

impl Model {
    /// A model with the given parent and no textures yet.
    pub fn parent(parent: &str) -> Self {
        Self {
            parent: parent.to_owned(),
            textures: Map::new(),
        }
    }

    /// Sets texture slot `slot` to `texture` (`minecraft:block/iron_ore`).
    pub fn texture(mut self, slot: &str, texture: &str) -> Self {
        self.textures
            .insert(slot.to_owned(), Value::String(texture.to_owned()));
        self
    }

    /// A full cube with one texture on every face (`minecraft:block/cube_all`).
    pub fn cube_all(texture: &str) -> Self {
        Self::parent("minecraft:block/cube_all").texture("all", texture)
    }

    /// A plant-like cross, the texture on two crossed planes (`minecraft:block/cross`).
    pub fn cross(texture: &str) -> Self {
        Self::parent("minecraft:block/cross").texture("cross", texture)
    }

    /// A flat item sprite (`minecraft:item/generated`).
    pub fn item_generated(texture: &str) -> Self {
        Self::parent("minecraft:item/generated").texture("layer0", texture)
    }

    /// A tool-like item sprite held in the hand (`minecraft:item/handheld`).
    pub fn item_handheld(texture: &str) -> Self {
        Self::parent("minecraft:item/handheld").texture("layer0", texture)
    }

    pub fn to_json(&self) -> Value {
        json!({ "parent": self.parent, "textures": Value::Object(self.textures.clone()) })
    }
}

/// An item's model definition (`assets/<id>/items/<name>.json`): the model it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemModel {
    model: String,
}

impl ItemModel {
    /// Shows the model `model` (`ember:block/ember_ore`, or `ember:item/ember_gem`).
    pub fn model(model: &str) -> Self {
        Self {
            model: model.to_owned(),
        }
    }

    pub fn to_json(&self) -> Value {
        json!({ "model": { "type": "minecraft:model", "model": self.model } })
    }
}

/// A result stack: an id and a count (1 unless set).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    id: String,
    count: u32,
}

impl Stack {
    pub fn new(id: &str, count: u32) -> Self {
        Self {
            id: id.to_owned(),
            count,
        }
    }

    fn to_json(&self) -> Value {
        if self.count == 1 {
            json!({ "id": self.id })
        } else {
            json!({ "id": self.id, "count": self.count })
        }
    }
}

/// A cooking recipe's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cooking {
    Smelting,
    Blasting,
    Smoking,
    CampfireCooking,
}

impl Cooking {
    fn type_name(self) -> &'static str {
        match self {
            Cooking::Smelting => "minecraft:smelting",
            Cooking::Blasting => "minecraft:blasting",
            Cooking::Smoking => "minecraft:smoking",
            Cooking::CampfireCooking => "minecraft:campfire_cooking",
        }
    }
}

/// A recipe file. Ingredients are item ids or `#namespace:tag`s.
#[derive(Clone, Debug, PartialEq)]
pub struct Recipe {
    value: Map<String, Value>,
}

impl Recipe {
    /// A shaped crafting recipe. `pattern` rows use one character per slot, a space for empty;
    /// `keys` gives each character its ingredient.
    pub fn shaped(pattern: &[&str], keys: &[(char, &str)], result: Stack) -> Self {
        let key: Map<String, Value> = keys
            .iter()
            .map(|(c, ingredient)| (c.to_string(), Value::String((*ingredient).to_owned())))
            .collect();
        let mut value = base("minecraft:crafting_shaped");
        value.insert("pattern".into(), json!(pattern));
        value.insert("key".into(), Value::Object(key));
        value.insert("result".into(), result.to_json());
        Self { value }
    }

    /// A shapeless crafting recipe: the ingredients in any place.
    pub fn shapeless(ingredients: &[&str], result: Stack) -> Self {
        let mut value = base("minecraft:crafting_shapeless");
        value.insert("ingredients".into(), json!(ingredients));
        value.insert("result".into(), result.to_json());
        Self { value }
    }

    /// A cooking recipe (furnace, blast furnace, smoker or campfire): the time in ticks and the
    /// experience it gives.
    pub fn cooking(
        kind: Cooking,
        ingredient: &str,
        result: Stack,
        experience: f32,
        cooking_time: u32,
    ) -> Self {
        let mut value = base(kind.type_name());
        value.insert("ingredient".into(), json!(ingredient));
        value.insert("result".into(), result.to_json());
        value.insert("experience".into(), float(experience));
        value.insert("cookingtime".into(), json!(cooking_time));
        Self { value }
    }

    /// Sets the recipe book category (`building`, `misc`, `food`...).
    pub fn category(mut self, category: &str) -> Self {
        self.value.insert("category".into(), json!(category));
        self
    }

    pub fn to_json(&self) -> Value {
        Value::Object(self.value.clone())
    }
}

/// An f32 as the number its shortest text names (0.7, not 0.699999988).
fn float(v: f32) -> Value {
    json!(
        format!("{v}")
            .parse::<f64>()
            .expect("a float prints as a number")
    )
}

fn base(type_name: &str) -> Map<String, Value> {
    let mut value = Map::new();
    value.insert("type".into(), json!(type_name));
    value.insert("category".into(), json!("misc"));
    value
}

/// A loot table: its pools and what it drops, as vanilla's JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct LootTable {
    value: Value,
}

impl LootTable {
    /// A block that drops itself, unless an explosion destroys it (vanilla's `minecraft:block`
    /// table with the `survives_explosion` condition).
    pub fn drop_self(item: &str) -> Self {
        Self::block_pool(item, None)
    }

    /// A block that drops `count` of `item`, unless an explosion destroys it.
    pub fn drop_item(item: &str, count: u32) -> Self {
        Self::block_pool(item, Some(count))
    }

    fn block_pool(item: &str, count: Option<u32>) -> Self {
        let mut entry = json!({ "type": "minecraft:item", "name": item });
        if let Some(count) = count {
            entry["functions"] =
                json!([{ "function": "minecraft:set_count", "count": count, "add": false }]);
        }
        Self {
            value: json!({
                "type": "minecraft:block",
                "pools": [{
                    "rolls": 1,
                    "conditions": [{ "condition": "minecraft:survives_explosion" }],
                    "entries": [entry]
                }]
            }),
        }
    }

    pub fn to_json(&self) -> Value {
        self.value.clone()
    }
}

/// A registry whose tags a mod adds to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TagRegistry {
    Block,
    Item,
}

impl TagRegistry {
    fn dir(self) -> &'static str {
        match self {
            TagRegistry::Block => "block",
            TagRegistry::Item => "item",
        }
    }
}

/// A tag file: its values (ids or `#tags`), and whether it replaces the tag's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    values: Vec<String>,
    replace: bool,
}

impl Tag {
    pub fn new(values: &[&str]) -> Self {
        Self {
            values: values.iter().map(|v| (*v).to_owned()).collect(),
            replace: false,
        }
    }

    /// Replaces the tag's list instead of adding to it.
    pub fn replace(mut self) -> Self {
        self.replace = true;
        self
    }

    pub fn to_json(&self) -> Value {
        let mut value = Map::new();
        if self.replace {
            value.insert("replace".into(), json!(true));
        }
        value.insert("values".into(), json!(self.values));
        Value::Object(value)
    }
}

/// The properties of a block the mod declares (`data/<id>/horizon/block/`): a template and
/// what changes from it. Unset properties keep the template's values.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockDef {
    template: String,
    properties: Map<String, Value>,
    item: bool,
}

impl BlockDef {
    /// A block copying `template` (`minecraft:iron_ore`).
    pub fn new(template: &str) -> Self {
        Self {
            template: template.to_owned(),
            properties: Map::new(),
            item: true,
        }
    }

    /// Sets a property by its name in the file (`hardness`, `light`, `requires_tool`...).
    pub fn property(mut self, key: &str, value: Value) -> Self {
        self.properties.insert(key.to_owned(), value);
        self
    }

    /// Hand-mining time in seconds (-1: unbreakable).
    pub fn hardness(self, hardness: f32) -> Self {
        self.property("hardness", float(hardness))
    }

    /// Light emitted, 0 to 15.
    pub fn light(self, light: u8) -> Self {
        self.property("light", json!(light))
    }

    /// Whether mining it needs the right tool.
    pub fn requires_tool(self, requires: bool) -> Self {
        self.property("requires_tool", json!(requires))
    }

    /// Leaves out the block item (`item: false`).
    pub fn without_item(mut self) -> Self {
        self.item = false;
        self
    }

    pub fn to_json(&self) -> Value {
        let mut value = Map::new();
        value.insert("template".into(), json!(self.template));
        if !self.properties.is_empty() {
            value.insert("properties".into(), Value::Object(self.properties.clone()));
        }
        if !self.item {
            value.insert("item".into(), json!(false));
        }
        Value::Object(value)
    }
}

/// An item the mod declares (`data/<id>/horizon/item/`): a template item and its changes.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemDef {
    template: String,
    max_stack_size: Option<u8>,
    rarity: Option<String>,
    crafting_remainder: Option<String>,
    components: Map<String, Value>,
}

impl ItemDef {
    /// An item copying `template` (`minecraft:emerald`).
    pub fn new(template: &str) -> Self {
        Self {
            template: template.to_owned(),
            max_stack_size: None,
            rarity: None,
            crafting_remainder: None,
            components: Map::new(),
        }
    }

    pub fn max_stack_size(mut self, size: u8) -> Self {
        self.max_stack_size = Some(size);
        self
    }

    /// `common`, `uncommon`, `rare` or `epic`.
    pub fn rarity(mut self, rarity: &str) -> Self {
        self.rarity = Some(rarity.to_owned());
        self
    }

    /// What it leaves in the crafting grid (a bucket, a bottle).
    pub fn crafting_remainder(mut self, item: &str) -> Self {
        self.crafting_remainder = Some(item.to_owned());
        self
    }

    /// A data component patch entry (`minecraft:max_damage`, or `!minecraft:<type>` to remove).
    pub fn component(mut self, key: &str, value: Value) -> Self {
        self.components.insert(key.to_owned(), value);
        self
    }

    pub fn to_json(&self) -> Value {
        let mut value = Map::new();
        value.insert("template".into(), json!(self.template));
        if let Some(size) = self.max_stack_size {
            value.insert("max_stack_size".into(), json!(size));
        }
        if let Some(rarity) = &self.rarity {
            value.insert("rarity".into(), json!(rarity));
        }
        if let Some(item) = &self.crafting_remainder {
            value.insert("crafting_remainder".into(), json!(item));
        }
        if !self.components.is_empty() {
            value.insert("components".into(), Value::Object(self.components.clone()));
        }
        Value::Object(value)
    }
}

/// An enchantment (`data/<id>/horizon/enchantment/`): vanilla's enchantment JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct Enchantment {
    map: Map<String, Value>,
}

impl Enchantment {
    /// An enchantment whose description is the translation key `description_key`, for the items
    /// in `supported_items` (`#minecraft:enchantable/weapon`, or an id), in `slots`
    /// (`mainhand`, `armor`...). Weight 10, max level 1, cost 1 + 0 per level, anvil cost 1 unless set.
    pub fn new(description_key: &str, supported_items: &str, slots: &[&str]) -> Self {
        let mut map = Map::new();
        map.insert(
            "description".into(),
            json!({ "translate": description_key }),
        );
        map.insert("supported_items".into(), json!(supported_items));
        map.insert("weight".into(), json!(10));
        map.insert("max_level".into(), json!(1));
        map.insert(
            "min_cost".into(),
            json!({ "base": 1, "per_level_above_first": 0 }),
        );
        map.insert(
            "max_cost".into(),
            json!({ "base": 1, "per_level_above_first": 0 }),
        );
        map.insert("anvil_cost".into(), json!(1));
        map.insert("slots".into(), json!(slots));
        Self { map }
    }

    /// Items the enchantment is found on, when they are not the supported ones.
    pub fn primary_items(mut self, items: &str) -> Self {
        self.map.insert("primary_items".into(), json!(items));
        self
    }

    /// The enchantments it cannot be on the same item with (`#minecraft:exclusive_set/damage`).
    pub fn exclusive_set(mut self, set: &str) -> Self {
        self.map.insert("exclusive_set".into(), json!(set));
        self
    }

    pub fn weight(mut self, weight: u32) -> Self {
        self.map.insert("weight".into(), json!(weight));
        self
    }

    pub fn max_level(mut self, level: u32) -> Self {
        self.map.insert("max_level".into(), json!(level));
        self
    }

    /// `min_cost` as vanilla writes it: base plus the increase per level above the first.
    pub fn min_cost(mut self, base: i64, per_level_above_first: i64) -> Self {
        self.map.insert(
            "min_cost".into(),
            json!({ "base": base, "per_level_above_first": per_level_above_first }),
        );
        self
    }

    pub fn max_cost(mut self, base: i64, per_level_above_first: i64) -> Self {
        self.map.insert(
            "max_cost".into(),
            json!({ "base": base, "per_level_above_first": per_level_above_first }),
        );
        self
    }

    pub fn anvil_cost(mut self, cost: u32) -> Self {
        self.map.insert("anvil_cost".into(), json!(cost));
        self
    }

    /// Its effect components, vanilla's `effects` object as written.
    pub fn effects(mut self, effects: Value) -> Self {
        self.map.insert("effects".into(), effects);
        self
    }

    pub fn to_json(&self) -> Value {
        Value::Object(self.map.clone())
    }
}

/// `DamageScaling`: when the damage scales with difficulty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageScaling {
    Never,
    WhenCausedByLivingNonPlayer,
    Always,
}

impl DamageScaling {
    fn name(self) -> &'static str {
        match self {
            DamageScaling::Never => "never",
            DamageScaling::WhenCausedByLivingNonPlayer => "when_caused_by_living_non_player",
            DamageScaling::Always => "always",
        }
    }
}

/// `DamageEffects`: the sound and visual of a hit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageEffects {
    Hurt,
    Thorns,
    Drowning,
    Burning,
    Poking,
    Freezing,
}

impl DamageEffects {
    fn name(self) -> &'static str {
        match self {
            DamageEffects::Hurt => "hurt",
            DamageEffects::Thorns => "thorns",
            DamageEffects::Drowning => "drowning",
            DamageEffects::Burning => "burning",
            DamageEffects::Poking => "poking",
            DamageEffects::Freezing => "freezing",
        }
    }
}

/// `DeathMessageType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeathMessageType {
    Default,
    FallVariants,
    IntentionalGameDesign,
}

impl DeathMessageType {
    fn name(self) -> &'static str {
        match self {
            DeathMessageType::Default => "default",
            DeathMessageType::FallVariants => "fall_variants",
            DeathMessageType::IntentionalGameDesign => "intentional_game_design",
        }
    }
}

/// A damage type (`data/<id>/horizon/damage_type/`): vanilla's `DamageType` JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageType {
    message_id: String,
    scaling: DamageScaling,
    exhaustion: f32,
    effects: DamageEffects,
    death_message_type: DeathMessageType,
}

impl DamageType {
    /// A damage type whose death message is `death.attack.<message_id>`. Its effects are hurt
    /// and its death message the default, unless set.
    pub fn new(message_id: &str, scaling: DamageScaling, exhaustion: f32) -> Self {
        Self {
            message_id: message_id.to_owned(),
            scaling,
            exhaustion,
            effects: DamageEffects::Hurt,
            death_message_type: DeathMessageType::Default,
        }
    }

    pub fn effects(mut self, effects: DamageEffects) -> Self {
        self.effects = effects;
        self
    }

    pub fn death_message_type(mut self, kind: DeathMessageType) -> Self {
        self.death_message_type = kind;
        self
    }

    pub fn to_json(&self) -> Value {
        let mut value = json!({
            "message_id": self.message_id,
            "scaling": self.scaling.name(),
            "exhaustion": float(self.exhaustion),
            "effects": self.effects.name(),
        });
        if self.death_message_type != DeathMessageType::Default {
            value["death_message_type"] = json!(self.death_message_type.name());
        }
        value
    }
}

/// A game rule (`data/<id>/horizon/game_rule/`): a boolean or an integer with bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameRule {
    kind: RuleKind,
    category: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RuleKind {
    Bool { default: bool },
    Int { default: i32, min: i32, max: i32 },
}

impl GameRule {
    /// A boolean rule.
    pub fn bool(default: bool) -> Self {
        Self {
            kind: RuleKind::Bool { default },
            category: "misc".into(),
        }
    }

    /// An integer rule; `min <= default <= max`.
    pub fn int(default: i32, min: i32, max: i32) -> Self {
        Self {
            kind: RuleKind::Int { default, min, max },
            category: "misc".into(),
        }
    }

    /// Its category: `player`, `mobs`, `spawning`, `drops`, `updates`, `chat` or `misc`.
    pub fn category(mut self, category: &str) -> Self {
        self.category = category.to_owned();
        self
    }

    pub fn to_json(&self) -> Value {
        match self.kind {
            RuleKind::Bool { default } => {
                json!({ "type": "bool", "default": default, "category": self.category })
            }
            RuleKind::Int { default, min, max } => {
                json!({ "type": "int", "default": default, "min": min, "max": max, "category": self.category })
            }
        }
    }
}

/// A fluid family (`data/<id>/horizon/fluid/`). Unset names are built from the fluid's path:
/// `flowing_<path>`, `<path>_bucket`, `block/<path>_still` and `block/<path>_flow`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Fluid {
    map: Map<String, Value>,
}

impl Fluid {
    pub fn new() -> Self {
        Self::default()
    }

    fn set(mut self, key: &str, value: Value) -> Self {
        self.map.insert(key.to_owned(), value);
        self
    }

    /// The block of the still fluid, a mod's own (`ember:ember_fluid`, default the fluid id).
    pub fn still_block(self, block: &str) -> Self {
        self.set("still_block", json!(block))
    }

    pub fn flowing_block(self, block: &str) -> Self {
        self.set("flowing_block", json!(block))
    }

    /// The bucket that places and takes the fluid.
    pub fn bucket(self, item: &str) -> Self {
        self.set("bucket", json!(item))
    }

    pub fn still_texture(self, texture: &str) -> Self {
        self.set("still_texture", json!(texture))
    }

    pub fn flowing_texture(self, texture: &str) -> Self {
        self.set("flowing_texture", json!(texture))
    }

    /// The flowing fluid's id.
    pub fn flowing_id(self, id: &str) -> Self {
        self.set("flowing_id", json!(id))
    }

    /// RGB tint, 0 to 0xFFFFFF.
    pub fn tint(self, rgb: u32) -> Self {
        self.set("tint", json!(rgb))
    }

    /// 1 to 8.
    pub fn viscosity(self, viscosity: u8) -> Self {
        self.set("viscosity", json!(viscosity))
    }

    /// Game ticks between flow steps.
    pub fn tick_rate(self, ticks: u32) -> Self {
        self.set("tick_rate", json!(ticks))
    }

    /// Light emitted, 0 to 15.
    pub fn light(self, light: u8) -> Self {
        self.set("light", json!(light))
    }

    pub fn can_drown(self, can: bool) -> Self {
        self.set("can_drown", json!(can))
    }

    /// `water`, `lava` or `none`.
    pub fn motion(self, motion: &str) -> Self {
        self.set("motion", json!(motion))
    }

    pub fn to_json(&self) -> Value {
        Value::Object(self.map.clone())
    }
}

/// The arguments of a datagen program: the mod folder, the mod id and whether to check.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub root: PathBuf,
    pub id: String,
    pub check: bool,
}

/// Reads `<root> --id <id> [--check]`, the arguments `cargo horizon-mod datagen` passes.
pub fn parse_args(args: &[String]) -> Result<Args, String> {
    const USAGE: &str = "usage: <datagen> <mod folder> --id <mod id> [--check]";
    let mut root = None;
    let mut id = None;
    let mut check = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--id" => id = Some(rest.next().ok_or(USAGE)?.clone()),
            "--check" => check = true,
            other if root.is_none() && !other.starts_with("--") => {
                root = Some(PathBuf::from(other))
            }
            _ => return Err(USAGE.into()),
        }
    }
    Ok(Args {
        root: root.ok_or(USAGE)?,
        id: id.ok_or(USAGE)?,
        check,
    })
}

/// Runs a datagen program: generates with `generate`, then writes the files under the mod folder,
/// or with `--check` compares them with the files on disk and fails when they differ.
pub fn run(args: &[String], generate: impl FnOnce(&mut Mod)) -> Result<String, String> {
    let args = parse_args(args)?;
    let mut generator = Mod::new(&args.id);
    generate(&mut generator);
    let out = generator.into_output();
    if args.check {
        out.check(&args.root).map_err(|stale| {
            format!(
                "{} generated file(s) differ from the mod folder or are missing: {}; run cargo horizon-mod datagen",
                stale.len(),
                stale.join(", ")
            )
        })?;
        Ok(format!("datagen: {} files up to date", out.files().count()))
    } else {
        let written = out.write(&args.root)?;
        Ok(format!("datagen: wrote {} files", written.len()))
    }
}

/// The `main` of a datagen program: [`run`] with the process arguments, the message printed and
/// a failing exit code on an error.
pub fn main(generate: impl FnOnce(&mut Mod)) -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args, generate) {
        Ok(message) => {
            println!("{message}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests;
