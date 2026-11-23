//! The levels (dimensions) of a world, as a registry (F3-3).
//!
//! A world's levels are the three vanilla ones plus every level stem the
//! world knows: the stems saved in `world_gen_settings`, the enabled data
//! packs' (`data/<ns>/dimension/<path>.json`, a pack's stem for an id
//! replacing the saved one, as `WorldOpenFlows.loadWorldStem` does), and the
//! ones mods register before the world starts ticking. Each level has its
//! dimension type, its generator spec and its save folder
//! (`dimensions/<namespace>/<path>/`, [`crate::save::dimension_dir`]).
//!
//! This module holds data only (no `Dim` handles, no streaming): the game
//! interns the ids and opens the levels from it. A stem that does not
//! validate is left out and reported in [`LevelRegistry::problems`]; it never
//! takes the vanilla levels with it.
use crate::chunk::{LevelShape, LevelShapes};
use horizon_world::{
    dimension::{
        DimensionError, DimensionType, DimensionTypeRegistry, GeneratorSpec, LevelStem, TypeRef,
    },
    worldgen::{
        biome_source::BiomeSourceCatalog,
        overrides::{self, WorldgenOverrides},
    },
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
};

/// The three levels every world has, in `MinecraftServer.levels` order.
pub const VANILLA_LEVELS: [&str; 3] = [
    "minecraft:overworld",
    "minecraft:the_nether",
    "minecraft:the_end",
];

/// The side file of the levels created while a world runs.
pub const STORED_LEVELS: &str = "yoake_levels.json";
/// Where ephemeral levels save, inside the world folder; wiped when the world opens.
pub const EPHEMERAL_DIR: &str = "yoake_temp";

/// Where a level comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LevelSource {
    /// One of the three built-in levels.
    Vanilla,
    /// A stem saved in the world's `world_gen_settings`.
    Saved,
    /// A stem of an enabled data pack.
    Pack,
    /// Registered by a mod through [`LevelRegistry::register_level`].
    Mod,
    /// Created while the server runs ([`LevelRegistry::create_level`]); a persistent one is
    /// listed in `data/yoake_levels.json` and comes back when the world opens.
    Created,
}

/// One level of a world.
#[derive(Clone, Debug)]
pub struct LevelEntry {
    pub id: String,
    pub source: LevelSource,
    /// Where the stem that defines the level came from: `None` for a
    /// vanilla level with no saved or pack stem (the world preset's).
    pub stem_from: Option<LevelSource>,
    /// The stem as JSON, as the chunk generator reads it.
    pub stem_json: Option<Value>,
    /// The stem decoded, when it was (vanilla levels with no stem have none,
    /// and a world with no non-vanilla stem needs no biome catalog).
    pub stem: Option<LevelStem>,
    pub dimension_type: DimensionType,
    pub shape: LevelShape,
    /// The generator's sea level (the `sea_level` of the noise settings the
    /// stem names; superflat and debug have their own), for a level past
    /// the vanilla three. `None` when the generator has none (a type's
    /// ceiling decides then).
    pub sea_level: Option<i32>,
    /// A created level that saves nothing in the world folder: its chunks live under
    /// `yoake_temp/` and go with the level, or the next time the world opens.
    pub ephemeral: bool,
    /// The level's own seed (`"seed"` in its stem; a Yoake extension), `None` for the world's,
    /// which is what every level has in vanilla.
    pub seed: Option<i64>,
}

/// A level after the vanilla three, as the network side sees it (F3-4).
#[derive(Clone, Debug, PartialEq)]
pub struct NetLevel {
    pub id: String,
    /// The `minecraft:dimension_type` entry the level uses.
    pub type_name: String,
    pub shape: LevelShape,
    /// The generator's sea level (the client's `SpawnInfo.sea_level`).
    pub sea_level: i32,
    /// The type's data, sent to clients that do not know the entry.
    pub type_json: Value,
}

/// Why a level or a registration was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum LevelError {
    /// The world already started ticking: levels are added before that
    /// (F3-6 adds run-time creation).
    Frozen,
    Duplicate(String),
    BadId { id: String, reason: &'static str },
    Stem { id: String, error: DimensionError },
    /// The stem decodes, but this version cannot run it yet.
    Unsupported { id: String, reason: String },
    /// A stem needs the biome catalog and none is available.
    NoCatalog(String),
    /// No such level in this world.
    Unknown(String),
    /// One of the three vanilla levels.
    Vanilla(String),
    /// A level of a data pack, the save or a mod: not the registry's to remove.
    NotCreated(String),
}
impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frozen => write!(f, "levels can only be registered before the world starts ticking"),
            Self::Duplicate(id) => write!(f, "level `{id}` is already registered"),
            Self::BadId { id, reason } => write!(f, "level id `{id}` is not usable: {reason}"),
            Self::Stem { id, error } => write!(f, "level stem `{id}`: {error}"),
            Self::Unsupported { id, reason } => write!(f, "level `{id}`: {reason}"),
            Self::Unknown(id) => write!(f, "level `{id}` is not one of this world's levels"),
            Self::Vanilla(id) => write!(f, "`{id}` is one of the three vanilla levels"),
            Self::NotCreated(id) => write!(
                f,
                "level `{id}` comes from a data pack, the save or a mod, not from level creation"
            ),
            Self::NoCatalog(id) => write!(
                f,
                "level stem `{id}` cannot be checked: the worldgen data is not available"
            ),
        }
    }
}
impl std::error::Error for LevelError {}

/// A level the world's data or save names but the registry left out.
#[derive(Clone, Debug, PartialEq)]
pub struct LevelProblem {
    pub id: String,
    pub message: String,
}

/// A world's levels and dimension types.
#[derive(Clone, Debug)]
pub struct LevelRegistry {
    types: DimensionTypeRegistry,
    /// The three vanilla levels first, then the others in the order found
    /// (saved, packs, then mods).
    entries: Vec<LevelEntry>,
    problems: Vec<LevelProblem>,
    frozen: bool,
    /// The `sea_level` of each noise settings entry, by id (the prepared data's, then the packs').
    noise_sea: BTreeMap<String, i32>,
}

impl Default for LevelRegistry {
    fn default() -> Self {
        Self::vanilla()
    }
}

/// `ns:path` that is safe to use as a folder name under `dimensions/`: both
/// parts non-empty, `[a-z0-9_.-]` namespace, `[a-z0-9_./-]` path, and no
/// `.`/`..`/empty path segment.
pub fn check_level_id(id: &str) -> Result<(), &'static str> {
    let Some((ns, path)) = id.split_once(':') else {
        return Err("a level id is `namespace:path`");
    };
    if ns.is_empty() || path.is_empty() {
        return Err("empty namespace or path");
    }
    if !ns
        .chars()
        .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-'))
    {
        return Err("the namespace may only hold a-z, 0-9, _, . and -");
    }
    if !path
        .chars()
        .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '.' | '-' | '/'))
    {
        return Err("the path may only hold a-z, 0-9, _, ., - and /");
    }
    if path
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err("the path has an empty, `.` or `..` segment");
    }
    if ns == "." || ns == ".." {
        return Err("the namespace is `.` or `..`");
    }
    Ok(())
}

fn shape_of(t: &DimensionType) -> Result<LevelShape, String> {
    LevelShape::new(
        t.min_y,
        t.height,
        t.logical_height,
        t.has_skylight,
        t.has_ceiling,
    )
}

/// The dimension type JSON files of the prepared game data
/// (`<resources>/data/minecraft/dimension_type/*.json`), by id.
pub fn prepared_dimension_types(resources: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let dir = resources.join("data/minecraft/dimension_type");
    let Ok(read) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let (Some(stem), Ok(text)) = (
            path.file_stem().and_then(|s| s.to_str()),
            std::fs::read_to_string(&path),
        ) else {
            continue;
        };
        out.insert(format!("minecraft:{stem}"), text);
    }
    out
}

/// The `sea_level` of every noise settings entry, by id: the prepared data's
/// (`<resources>/data/minecraft/worldgen/noise_settings`), the packs' over it.
pub fn noise_sea_levels(resources: &Path, overrides: &WorldgenOverrides) -> BTreeMap<String, i32> {
    let mut out = BTreeMap::new();
    let sea = |text: &str| {
        serde_json::from_str::<Value>(text)
            .ok()?
            .get("sea_level")?
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
    };
    if let Ok(read) = std::fs::read_dir(resources.join("data/minecraft/worldgen/noise_settings")) {
        for entry in read.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            if let (Some(stem), Ok(text)) = (
                path.file_stem().and_then(|s| s.to_str()),
                std::fs::read_to_string(&path),
            ) && let Some(n) = sea(&text)
            {
                out.insert(format!("minecraft:{stem}"), n);
            }
        }
    }
    for (id, text) in overrides.entries("settings") {
        if let Some(n) = sea(text) {
            out.insert(id.to_owned(), n);
        }
    }
    out
}

/// The sea level a generator gives its level: the noise settings' `sea_level`,
/// superflat's -63 and the debug world's 63 (`ChunkGenerator.getSeaLevel`).
fn generator_sea_level(generator: &GeneratorSpec, noise_sea: &BTreeMap<String, i32>) -> Option<i32> {
    match generator {
        GeneratorSpec::Noise { settings, .. } => match settings {
            horizon_world::dimension::SettingsRef::Id(id) => noise_sea.get(id).copied(),
            horizon_world::dimension::SettingsRef::Inline(v) => v
                .get("sea_level")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok()),
        },
        GeneratorSpec::Flat(_) => Some(-63),
        GeneratorSpec::Debug => Some(63),
        GeneratorSpec::Mod { .. } => None,
    }
}

/// The `seed` a stem gives its level, when it does (a number, or a numeric string as vanilla's
/// seed fields allow).
fn json_seed(stem: &Value) -> Option<i64> {
    match stem.get("seed")? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// What a registry is built from.
pub struct LevelInputs<'a> {
    /// The prepared dimension type files, by id (`prepared_dimension_types`).
    pub prepared_types: BTreeMap<String, String>,
    /// The enabled data packs' worldgen entries (stems and dimension types).
    pub overrides: &'a WorldgenOverrides,
    /// The stems of the save's `world_gen_settings` `dimensions`, by level id.
    pub saved_stems: BTreeMap<String, Value>,
    /// The vanilla levels' shapes as the world's packs set them.
    pub vanilla_shapes: LevelShapes,
    /// Needed only to decode non-vanilla stems.
    pub catalog: Option<&'a BiomeSourceCatalog>,
    /// `noise_sea_levels`.
    pub noise_sea: BTreeMap<String, i32>,
}

impl LevelRegistry {
    /// The three vanilla levels with the built-in types and no stems: what a
    /// world with no packs, no mods and no extra stems is.
    pub fn vanilla() -> Self {
        Self::build(LevelInputs {
            noise_sea: Default::default(),
            prepared_types: BTreeMap::new(),
            overrides: &WorldgenOverrides::default(),
            saved_stems: BTreeMap::new(),
            vanilla_shapes: LevelShapes::VANILLA,
            catalog: None,
        })
    }

    /// Whether building the registry of this world needs the biome catalog:
    /// some stem (saved or from a pack) names a level other than the three.
    pub fn needs_catalog(world: &Path, overrides: &WorldgenOverrides) -> bool {
        Self::saved_stems(world)
            .keys()
            .any(|id| !VANILLA_LEVELS.contains(&id.as_str()))
            || overrides
                .entries(overrides::DIMENSIONS)
                .any(|(id, _)| !VANILLA_LEVELS.contains(&id))
    }

    /// The stems saved in the world's `world_gen_settings` (26.3's
    /// `data/minecraft/world_gen_settings.dat`, or `level.dat`'s
    /// `Data.WorldGenSettings` in an older save), by level id.
    pub fn saved_stems(world: &Path) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        let Some(settings) = crate::save::world_gen_settings(world) else {
            return out;
        };
        if let Some(horizon_world::nbt::Tag::Compound(dimensions)) = settings.get("dimensions") {
            for (id, stem) in dimensions.iter() {
                out.insert(
                    id.to_string_lossy(),
                    crate::scoreboard::tag_to_json(stem),
                );
            }
        }
        out
    }

    /// The registry of the world at `world`.
    pub fn for_world(
        world: &Path,
        resources: &Path,
        overrides: &WorldgenOverrides,
        vanilla_shapes: LevelShapes,
        catalog: Option<&BiomeSourceCatalog>,
    ) -> Self {
        Self::build(LevelInputs {
            noise_sea: noise_sea_levels(resources, overrides),
            prepared_types: prepared_dimension_types(resources),
            overrides,
            saved_stems: Self::saved_stems(world),
            vanilla_shapes,
            catalog,
        })
    }

    pub fn build(inputs: LevelInputs<'_>) -> Self {
        // The prepared data, the packs' types over it, over the built-ins:
        // the built-in vanilla types carry only the gameplay attributes, the
        // prepared files the visual and audio ones too.
        let mut bundle = inputs.prepared_types;
        for (id, json) in inputs.overrides.entries(overrides::DIMENSION_TYPES) {
            bundle.insert(id.to_owned(), json.to_owned());
        }
        let (types, type_errors) = DimensionTypeRegistry::load(&bundle);
        let mut reg = Self {
            types,
            entries: Vec::new(),
            problems: type_errors
                .into_iter()
                .map(|(id, e)| LevelProblem {
                    id: id.clone(),
                    message: format!("dimension type `{id}`: {e}"),
                })
                .collect(),
            frozen: false,
            noise_sea: inputs.noise_sea,
        };
        // Stems by id: saved, then packs over them.
        let mut stems: BTreeMap<String, (Value, LevelSource)> = inputs
            .saved_stems
            .into_iter()
            .map(|(id, v)| (id, (v, LevelSource::Saved)))
            .collect();
        for (id, json) in inputs.overrides.entries(overrides::DIMENSIONS) {
            match serde_json::from_str::<Value>(json) {
                Ok(v) => {
                    stems.insert(id.to_owned(), (v, LevelSource::Pack));
                }
                Err(e) => reg.problems.push(LevelProblem {
                    id: id.to_owned(),
                    message: format!("level stem `{id}`: invalid JSON: {e}"),
                }),
            }
        }
        // The vanilla levels. A stem is only validated here (the generator
        // reads it as before); a stem that does not validate is reported and
        // the level keeps working as it did.
        for id in VANILLA_LEVELS {
            let (stem_json, stem_from) = match stems.remove(id) {
                Some((v, from)) => (Some(v), Some(from)),
                None => (None, None),
            };
            let mut stem = None;
            let mut ty = reg.types.get(id).cloned();
            if let (Some(json), Some(catalog)) = (&stem_json, inputs.catalog) {
                match LevelStem::from_value(json, catalog) {
                    Ok(s) => {
                        match reg.types.resolve(&s.dimension_type) {
                            Ok(t) => ty = Some(t.clone()),
                            Err(e) => reg.problems.push(LevelProblem {
                                id: id.to_owned(),
                                message: LevelError::Stem { id: id.to_owned(), error: e }.to_string(),
                            }),
                        }
                        stem = Some(s);
                    }
                    Err(error) => reg.problems.push(LevelProblem {
                        id: id.to_owned(),
                        message: LevelError::Stem { id: id.to_owned(), error }.to_string(),
                    }),
                }
            }
            let dimension_type = ty
                .or_else(|| DimensionType::vanilla(id))
                .expect("vanilla level type");
            reg.entries.push(LevelEntry {
                id: id.to_owned(),
                source: LevelSource::Vanilla,
                stem_from,
                stem_json,
                stem,
                dimension_type,
                shape: inputs.vanilla_shapes.get(id),
                sea_level: None,
                ephemeral: false,
                seed: None,
            });
        }
        // The others.
        for (id, (json, from)) in stems {
            let result = reg.make_entry(&id, json, from, inputs.catalog);
            match result {
                Ok(entry) => reg.entries.push(entry),
                Err(e) => reg.problems.push(LevelProblem {
                    id,
                    message: e.to_string(),
                }),
            }
        }
        reg
    }

    /// A non-vanilla level from its stem JSON.
    fn make_entry(
        &self,
        id: &str,
        json: Value,
        from: LevelSource,
        catalog: Option<&BiomeSourceCatalog>,
    ) -> Result<LevelEntry, LevelError> {
        check_level_id(id).map_err(|reason| LevelError::BadId {
            id: id.to_owned(),
            reason,
        })?;
        let catalog = catalog.ok_or_else(|| LevelError::NoCatalog(id.to_owned()))?;
        let stem = LevelStem::from_value(&json, catalog).map_err(|error| LevelError::Stem {
            id: id.to_owned(),
            error,
        })?;
        let dimension_type = self
            .types
            .resolve(&stem.dimension_type)
            .map_err(|error| LevelError::Stem {
                id: id.to_owned(),
                error,
            })?
            .clone();
        // Noise, superflat, debug and mod generators all run in any level (F3-5); a mod's is
        // looked up when the level opens, since mods register after the world is read.
        let shape = shape_of(&dimension_type).map_err(|reason| LevelError::Unsupported {
            id: id.to_owned(),
            reason: format!("its dimension type's shape is refused: {reason}"),
        })?;
        // The generator resolves the stem's type by id from the packs' data;
        // a type only the registry knows (a mod's) is written into the stem.
        let mut stem_json = json;
        if matches!(stem.dimension_type, TypeRef::Id(_)) && matches!(from, LevelSource::Mod | LevelSource::Created) {
            stem_json["type"] = dimension_type.to_value();
        }
        let sea_level = generator_sea_level(&stem.generator, &self.noise_sea);
        let seed = json_seed(&stem_json);
        Ok(LevelEntry {
            id: id.to_owned(),
            source: from,
            stem_from: Some(from),
            stem_json: Some(stem_json),
            stem: Some(stem),
            dimension_type,
            shape,
            sea_level,
            ephemeral: false,
            seed,
        })
    }

    /// Adds a dimension type (a mod's) before the world starts ticking.
    pub fn register_type(&mut self, id: &str, t: DimensionType) -> Result<(), LevelError> {
        if self.frozen {
            return Err(LevelError::Frozen);
        }
        check_level_id(id).map_err(|reason| LevelError::BadId {
            id: id.to_owned(),
            reason,
        })?;
        self.types
            .insert(id, t)
            .map_err(|error| LevelError::Stem {
                id: id.to_owned(),
                error,
            })
    }

    /// Registers a level for a mod, from its stem JSON, before the world
    /// starts ticking. No level is created or removed at run time here (F3-6).
    pub fn register_level(
        &mut self,
        id: &str,
        stem: Value,
        catalog: &BiomeSourceCatalog,
    ) -> Result<&LevelEntry, LevelError> {
        if self.frozen {
            return Err(LevelError::Frozen);
        }
        if self.contains(id) {
            return Err(LevelError::Duplicate(id.to_owned()));
        }
        let entry = self.make_entry(id, stem, LevelSource::Mod, Some(catalog))?;
        self.entries.push(entry);
        Ok(self.entries.last().expect("pushed"))
    }

    /// Creates a level while the server runs (F3-6): the same stem rules as a mod's, no freeze.
    /// `ephemeral` levels save under `yoake_temp/` and are not listed for the next time the
    /// world opens.
    pub fn create_level(
        &mut self,
        id: &str,
        stem: Value,
        catalog: &BiomeSourceCatalog,
        ephemeral: bool,
    ) -> Result<&LevelEntry, LevelError> {
        if self.contains(id) {
            return Err(LevelError::Duplicate(id.to_owned()));
        }
        let mut entry = self.make_entry(id, stem, LevelSource::Created, Some(catalog))?;
        entry.ephemeral = ephemeral;
        self.entries.push(entry);
        Ok(self.entries.last().expect("pushed"))
    }

    /// The seed the level generates with: its own, else `world_seed`.
    pub fn seed(&self, id: &str, world_seed: i64) -> i64 {
        self.get(id).and_then(|e| e.seed).unwrap_or(world_seed)
    }

    /// Gives a created level a new seed (or the world's again with `None`), in its stem too, so
    /// the side file keeps it (F3-7). Only created levels change; the others come from data.
    pub fn set_seed(&mut self, id: &str, seed: Option<i64>) -> Result<(), LevelError> {
        let Some(entry) = self.entries.iter_mut().find(|e| e.id == id) else {
            return Err(LevelError::Unknown(id.to_owned()));
        };
        match entry.source {
            LevelSource::Vanilla => return Err(LevelError::Vanilla(id.to_owned())),
            LevelSource::Created => {}
            _ => return Err(LevelError::NotCreated(id.to_owned())),
        }
        entry.seed = seed;
        if let Some(Value::Object(stem)) = entry.stem_json.as_mut() {
            match seed {
                Some(s) => stem.insert("seed".into(), Value::from(s)),
                None => stem.remove("seed"),
            };
        }
        Ok(())
    }

    /// Takes a created level out of the registry (F3-6). The caller has closed it and cleared
    /// its files; vanilla levels and levels of packs, the save or mods are not removable.
    pub fn remove_level(&mut self, id: &str) -> Result<LevelEntry, LevelError> {
        let Some(index) = self.entries.iter().position(|e| e.id == id) else {
            return Err(LevelError::Unknown(id.to_owned()));
        };
        match self.entries[index].source {
            LevelSource::Vanilla => Err(LevelError::Vanilla(id.to_owned())),
            LevelSource::Created => Ok(self.entries.remove(index)),
            _ => Err(LevelError::NotCreated(id.to_owned())),
        }
    }

    /// The levels created in an earlier session: `data/yoake_levels.json` (absent when there are
    /// none, so a vanilla save is unchanged), as (id, stem).
    pub fn stored_levels(world: &Path) -> Vec<(String, Value)> {
        let Ok(text) = std::fs::read_to_string(world.join("data").join(STORED_LEVELS)) else {
            return Vec::new();
        };
        let Ok(json) = serde_json::from_str::<Value>(&text) else {
            log::warn!("Horizon: {STORED_LEVELS} is not valid JSON; its levels are not restored");
            return Vec::new();
        };
        json["levels"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| Some((l["id"].as_str()?.to_owned(), l["stem"].clone())))
            .collect()
    }

    /// Writes the persistent created levels to `data/yoake_levels.json`, or removes the file when
    /// there are none.
    pub fn write_created(&self, world: &Path) -> std::io::Result<()> {
        let levels: Vec<Value> = self
            .entries
            .iter()
            .filter(|e| e.source == LevelSource::Created && !e.ephemeral)
            .filter_map(|e| {
                Some(serde_json::json!({"id": e.id, "stem": e.stem_json.clone()?}))
            })
            .collect();
        let path = world.join("data").join(STORED_LEVELS);
        if levels.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(path.parent().expect("a data folder"))?;
        std::fs::write(
            path,
            serde_json::to_string_pretty(&serde_json::json!({"version": 1, "levels": levels}))
                .expect("json"),
        )
    }

    /// Puts the levels of an earlier session (`stored`, created levels) and the levels mods
    /// staged (`staged`) into a registry being built for a world (F3-6); a stem that does not
    /// validate is reported and left out.
    pub fn restore_levels(
        &mut self,
        stored: Vec<(String, Value)>,
        staged: Vec<(String, Value)>,
        catalog: Option<&BiomeSourceCatalog>,
    ) {
        self.restore_levels_with_types(Vec::new(), stored, staged, catalog);
    }

    /// `restore_levels`, with the dimension types mods staged registered first.
    pub fn restore_levels_with_types(
        &mut self,
        types: Vec<(String, Value)>,
        stored: Vec<(String, Value)>,
        staged: Vec<(String, Value)>,
        catalog: Option<&BiomeSourceCatalog>,
    ) {
        for (id, json) in types {
            let result = DimensionType::from_value(&json)
                .map_err(|error| LevelError::Stem { id: id.clone(), error })
                .and_then(|t| self.register_type(&id, t));
            if let Err(e) = result {
                self.problems.push(LevelProblem { id, message: e.to_string() });
            }
        }
        for (id, stem) in stored {
            let result = match catalog {
                Some(catalog) => self.create_level(&id, stem, catalog, false).map(|_| ()),
                None => Err(LevelError::NoCatalog(id.clone())),
            };
            if let Err(e) = result {
                self.problems.push(LevelProblem { id, message: e.to_string() });
            }
        }
        for (id, stem) in staged {
            let result = match catalog {
                Some(catalog) => self.register_level(&id, stem, catalog).map(|_| ()),
                None => Err(LevelError::NoCatalog(id.clone())),
            };
            if let Err(e) = result {
                self.problems.push(LevelProblem { id, message: e.to_string() });
            }
        }
    }

    /// From now on the registry takes no more levels (the first server tick).
    pub fn freeze(&mut self) {
        self.frozen = true;
    }
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    pub fn get(&self, id: &str) -> Option<&LevelEntry> {
        self.entries.iter().find(|e| e.id == id)
    }
    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }
    /// The level ids: the three vanilla ones first.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.id.as_str())
    }
    pub fn entries(&self) -> &[LevelEntry] {
        &self.entries
    }
    /// The stems and types the registry left out, with why.
    pub fn problems(&self) -> &[LevelProblem] {
        &self.problems
    }
    pub fn types(&self) -> &DimensionTypeRegistry {
        &self.types
    }
    pub fn shape(&self, id: &str) -> Option<LevelShape> {
        self.get(id).map(|e| e.shape)
    }
    /// The level's `coordinate_scale` (1 for a level not registered).
    pub fn coordinate_scale(&self, id: &str) -> f64 {
        self.get(id).map_or(1.0, |e| e.dimension_type.coordinate_scale)
    }
    /// The stem the chunk generator takes below the packs' own.
    pub fn generator_stem(&self, id: &str) -> Option<&Value> {
        self.get(id)?.stem_json.as_ref()
    }
    /// What the network side needs of each level a client could not know
    /// from vanilla: the levels after the three vanilla ones, with the
    /// dimension type they name (a registered id, or the level's own id for
    /// an inline type) and that type's data (F3-4).
    pub fn network_levels(&self) -> Vec<NetLevel> {
        self.entries
            .iter()
            .filter(|e| !VANILLA_LEVELS.contains(&e.id.as_str()))
            .map(|e| NetLevel {
                id: e.id.clone(),
                type_name: match e.stem.as_ref().map(|s| &s.dimension_type) {
                    Some(TypeRef::Id(name)) => name.clone(),
                    _ => e.id.clone(),
                },
                shape: e.shape,
                sea_level: e.sea_level.unwrap_or(if e.dimension_type.has_ceiling { 32 } else { 63 }),
                type_json: e.dimension_type.to_value(),
            })
            .collect()
    }
    /// The level's save folder inside the world.
    pub fn save_dir(&self, world: &Path, id: &str) -> PathBuf {
        if self.get(id).is_some_and(|e| e.ephemeral) {
            let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
            return world.join(EPHEMERAL_DIR).join(ns).join(path);
        }
        crate::save::dimension_dir(world, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn catalog() -> BiomeSourceCatalog {
        let biomes: BTreeSet<String> = ["minecraft:plains", "minecraft:desert", "minecraft:the_void"]
            .into_iter()
            .map(String::from)
            .collect();
        BiomeSourceCatalog::new(&biomes, &BTreeMap::new(), &BTreeSet::new()).unwrap()
    }

    fn grid_stem(ty: &str) -> Value {
        serde_json::json!({
            "type": ty,
            "generator": {
                "type": "minecraft:noise",
                "settings": "minecraft:overworld",
                "biome_source": {
                    "type": "yoake:grid", "cell_size": 16, "width": 2, "depth": 1,
                    "cells": ["minecraft:plains", "minecraft:desert"],
                    "outside": "minecraft:plains"
                }
            }
        })
    }

    fn overrides_with(stems: &[(&str, Value)], types: &[(&str, Value)]) -> WorldgenOverrides {
        let mut o = WorldgenOverrides::default();
        for (id, v) in stems {
            o.set(overrides::DIMENSIONS, id, v.to_string());
        }
        for (id, v) in types {
            o.set(overrides::DIMENSION_TYPES, id, v.to_string());
        }
        o
    }

    fn build(o: &WorldgenOverrides, saved: &[(&str, Value)]) -> LevelRegistry {
        let cat = catalog();
        LevelRegistry::build(LevelInputs {
            noise_sea: Default::default(),
            prepared_types: BTreeMap::new(),
            overrides: o,
            saved_stems: saved.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
            vanilla_shapes: LevelShapes::VANILLA,
            catalog: Some(&cat),
        })
    }

    #[test]
    fn a_world_with_nothing_has_the_three_vanilla_levels() {
        let reg = LevelRegistry::vanilla();
        assert_eq!(reg.ids().collect::<Vec<_>>(), VANILLA_LEVELS);
        assert!(reg.problems().is_empty());
        for id in VANILLA_LEVELS {
            let e = reg.get(id).unwrap();
            assert_eq!(e.source, LevelSource::Vanilla);
            assert_eq!(e.stem_from, None);
            assert!(e.stem_json.is_none());
            assert_eq!(e.shape, LevelShapes::VANILLA.get(id));
        }
        assert_eq!(reg.coordinate_scale("minecraft:the_nether"), 8.0);
        assert_eq!(reg.coordinate_scale("minecraft:overworld"), 1.0);
        assert_eq!(reg.coordinate_scale("nope:nope"), 1.0);
    }

    #[test]
    fn a_pack_stem_adds_a_fourth_level_with_its_own_save_dir() {
        let o = overrides_with(&[("mypack:moon", grid_stem("minecraft:overworld"))], &[]);
        let reg = build(&o, &[]);
        assert_eq!(reg.problems(), &[]);
        let ids: Vec<_> = reg.ids().collect();
        assert_eq!(ids, [VANILLA_LEVELS[0], VANILLA_LEVELS[1], VANILLA_LEVELS[2], "mypack:moon"]);
        let moon = reg.get("mypack:moon").unwrap();
        assert_eq!(moon.source, LevelSource::Pack);
        assert_eq!(moon.shape, LevelShape::OVERWORLD);
        let world = Path::new("w");
        assert_eq!(
            reg.save_dir(world, "mypack:moon"),
            world.join("dimensions").join("mypack").join("moon")
        );
        assert_ne!(
            reg.save_dir(world, "mypack:moon"),
            reg.save_dir(world, "minecraft:overworld")
        );
    }

    /// The sea level is the generator's: the named noise settings', an inline one's, else the type's rule.
    /// F3-6: levels created while the server runs: no freeze, their own save dir, listed in
    /// `data/yoake_levels.json` unless ephemeral, and removable only when created.
    #[test]
    fn created_levels_persist_restore_and_are_removable() {
        let cat = catalog();
        let mut reg = LevelRegistry::vanilla();
        reg.freeze();
        assert_eq!(reg.register_level("a:late", grid_stem("minecraft:overworld"), &cat).unwrap_err(), LevelError::Frozen);
        let world = std::env::temp_dir().join(format!("hz-f36-reg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&world);
        reg.create_level("a:kept", grid_stem("minecraft:overworld"), &cat, false).expect("no freeze");
        reg.create_level("a:temp", grid_stem("minecraft:overworld"), &cat, true).unwrap();
        assert_eq!(
            reg.create_level("a:kept", grid_stem("minecraft:overworld"), &cat, false).unwrap_err(),
            LevelError::Duplicate("a:kept".into())
        );
        assert_eq!(reg.get("a:kept").unwrap().source, LevelSource::Created);
        assert_eq!(reg.save_dir(&world, "a:kept"), world.join("dimensions").join("a").join("kept"));
        assert_eq!(reg.save_dir(&world, "a:temp"), world.join(EPHEMERAL_DIR).join("a").join("temp"));
        // Only the persistent one is written; vanilla saves have no such file.
        assert!(LevelRegistry::stored_levels(&world).is_empty());
        reg.write_created(&world).unwrap();
        let stored = LevelRegistry::stored_levels(&world);
        assert_eq!(stored.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), ["a:kept"]);
        // The next session restores it (and reports a stem it cannot), beside the staged ones.
        let mut next = LevelRegistry::vanilla();
        let mut broken = stored.clone();
        broken.push(("a:broken".into(), serde_json::json!({"type": "nope:missing"})));
        next.restore_levels(broken, vec![("a:staged".into(), grid_stem("minecraft:overworld"))], Some(&cat));
        assert_eq!(next.get("a:kept").unwrap().source, LevelSource::Created);
        assert_eq!(next.get("a:staged").unwrap().source, LevelSource::Mod);
        assert!(!next.contains("a:broken") && next.problems().iter().any(|p| p.id == "a:broken"));
        // Removal: vanilla, mod and unknown levels are refused; a created one goes.
        assert_eq!(next.remove_level("minecraft:the_end").unwrap_err(), LevelError::Vanilla("minecraft:the_end".into()));
        assert_eq!(next.remove_level("a:staged").unwrap_err(), LevelError::NotCreated("a:staged".into()));
        assert_eq!(next.remove_level("a:nope").unwrap_err(), LevelError::Unknown("a:nope".into()));
        next.remove_level("a:kept").unwrap();
        next.write_created(&world).unwrap();
        assert!(!world.join("data").join(STORED_LEVELS).exists(), "the file goes with the last level");
        let _ = std::fs::remove_dir_all(&world);
    }

    /// F3-7: a level's stem may carry its own seed; otherwise it generates with the world's.
    #[test]
    fn a_level_may_have_its_own_seed() {
        let cat = catalog();
        let mut reg = LevelRegistry::vanilla();
        let mut stem = grid_stem("minecraft:overworld");
        stem["seed"] = serde_json::json!(-77);
        reg.create_level("a:seeded", stem, &cat, false).unwrap();
        let mut text = grid_stem("minecraft:overworld");
        text["seed"] = serde_json::json!("123");
        reg.create_level("a:text", text, &cat, false).unwrap();
        reg.create_level("a:plain", grid_stem("minecraft:overworld"), &cat, false).unwrap();
        assert_eq!(reg.seed("a:seeded", 5), -77);
        assert_eq!(reg.seed("a:text", 5), 123);
        assert_eq!(reg.seed("a:plain", 5), 5);
        assert_eq!(reg.seed("minecraft:overworld", 5), 5, "vanilla levels use the world's seed");
        // A created level's seed changes and is kept in its stem (for the side file); the rest do not.
        reg.set_seed("a:plain", Some(9)).unwrap();
        assert_eq!(reg.seed("a:plain", 5), 9);
        assert_eq!(reg.generator_stem("a:plain").unwrap()["seed"], 9);
        reg.set_seed("a:plain", None).unwrap();
        assert_eq!(reg.seed("a:plain", 5), 5);
        assert!(reg.generator_stem("a:plain").unwrap().get("seed").is_none());
        assert_eq!(reg.set_seed("minecraft:the_end", Some(1)).unwrap_err(), LevelError::Vanilla("minecraft:the_end".into()));
        assert_eq!(reg.set_seed("a:none", Some(1)).unwrap_err(), LevelError::Unknown("a:none".into()));
        let mut o = LevelRegistry::vanilla();
        o.register_level("a:mod", grid_stem("minecraft:overworld"), &cat).unwrap();
        assert_eq!(o.set_seed("a:mod", Some(1)).unwrap_err(), LevelError::NotCreated("a:mod".into()));
    }

    /// F3-5: superflat, debug, mod and every biome-source variant are levels of their own.
    #[test]
    fn flat_debug_mod_and_checkerboard_stems_are_levels() {
        let with = |generator: Value| {
            let mut s = grid_stem("minecraft:overworld");
            s["generator"] = generator;
            s
        };
        let o = overrides_with(
            &[
                ("a:flat", with(serde_json::json!({"type":"minecraft:flat","settings":{"biome":"minecraft:plains","layers":[{"block":"minecraft:stone","height":3}]}}))),
                ("a:debug", with(serde_json::json!({"type":"minecraft:debug"}))),
                ("a:mod", with(serde_json::json!({"type":"yoake:mod","id":"mymod:floor","settings":{"depth":3}}))),
                ("a:checker", with(serde_json::json!({"type":"minecraft:noise","settings":"minecraft:overworld",
                    "biome_source":{"type":"minecraft:checkerboard","biomes":["minecraft:plains","minecraft:desert"],"scale":2}}))),
                ("a:fixed", with(serde_json::json!({"type":"minecraft:noise","settings":"minecraft:overworld",
                    "biome_source":{"type":"minecraft:fixed","biome":"minecraft:desert"}}))),
                ("a:no_mod_id", with(serde_json::json!({"type":"yoake:mod"}))),
                ("a:bad_mod_id", with(serde_json::json!({"type":"yoake:mod","id":"nocolon"}))),
            ],
            &[],
        );
        let reg = build(&o, &[]);
        let ids: Vec<_> = reg.ids().skip(3).collect();
        assert_eq!(ids, ["a:checker", "a:debug", "a:fixed", "a:flat", "a:mod"], "{:?}", reg.problems());
        let sea = |id: &str| reg.get(id).unwrap().sea_level;
        assert_eq!((sea("a:flat"), sea("a:debug"), sea("a:mod")), (Some(-63), Some(63), None));
        let problems: Vec<_> = reg.problems().iter().map(|p| p.id.as_str()).collect();
        assert_eq!(problems, ["a:bad_mod_id", "a:no_mod_id"]);
        // The stem a mod level has survives as the generator reads it.
        let stem = reg.generator_stem("a:mod").unwrap();
        assert_eq!(stem["generator"]["id"], "mymod:floor");
    }

    #[test]
    fn the_sea_level_comes_from_the_generators_noise_settings() {
        let stem = |settings: Value| {
            let mut s = grid_stem("minecraft:overworld");
            s["generator"]["settings"] = settings;
            s
        };
        let o = overrides_with(
            &[
                ("mypack:named", stem(serde_json::json!("mypack:shallow"))),
                ("mypack:inline", stem(serde_json::json!({"sea_level": 20}))),
                ("mypack:unknown", stem(serde_json::json!("mypack:nowhere"))),
            ],
            &[],
        );
        let cat = catalog();
        let reg = LevelRegistry::build(LevelInputs {
            noise_sea: BTreeMap::from([("mypack:shallow".to_owned(), 40)]),
            prepared_types: BTreeMap::new(),
            overrides: &o,
            saved_stems: BTreeMap::new(),
            vanilla_shapes: LevelShapes::VANILLA,
            catalog: Some(&cat),
        });
        assert_eq!(reg.problems(), &[]);
        let sea = |id: &str| reg.get(id).unwrap().sea_level;
        assert_eq!(sea("mypack:named"), Some(40));
        assert_eq!(sea("mypack:inline"), Some(20));
        assert_eq!(sea("mypack:unknown"), None);
        assert_eq!(sea("minecraft:overworld"), None, "the vanilla levels keep their own");
        let net: Vec<_> = reg.network_levels().iter().map(|l| (l.id.clone(), l.sea_level)).collect();
        assert!(net.contains(&("mypack:named".to_owned(), 40)));
        assert!(net.contains(&("mypack:unknown".to_owned(), 63)), "the ceiling rule when the generator has none");
    }

    #[test]
    fn a_pack_type_gives_the_level_its_own_shape_and_scale() {
        let ty = serde_json::json!({
            "min_y": 0, "height": 128, "logical_height": 128,
            "has_skylight": false, "has_ceiling": true, "coordinate_scale": 4.0
        });
        let o = overrides_with(
            &[("mypack:deep", grid_stem("mypack:deep_type"))],
            &[("mypack:deep_type", ty)],
        );
        let reg = build(&o, &[]);
        assert_eq!(reg.problems(), &[]);
        let e = reg.get("mypack:deep").unwrap();
        assert_eq!((e.shape.min_y, e.shape.height, e.shape.sky_light), (0, 128, false));
        assert_eq!(reg.coordinate_scale("mypack:deep"), 4.0);
    }

    #[test]
    fn a_pack_stem_replaces_the_saved_one_of_its_id() {
        let saved = grid_stem("minecraft:overworld");
        let mut pack = grid_stem("minecraft:overworld");
        pack["generator"]["biome_source"]["outside"] = "minecraft:desert".into();
        let o = overrides_with(&[("mypack:moon", pack.clone())], &[]);
        let reg = build(&o, &[("mypack:moon", saved.clone())]);
        let moon = reg.get("mypack:moon").unwrap();
        assert_eq!(moon.source, LevelSource::Pack);
        assert_eq!(moon.stem_json.as_ref(), Some(&pack));
        // With no pack it is the saved one.
        let reg = build(&WorldgenOverrides::default(), &[("mypack:moon", saved.clone())]);
        let moon = reg.get("mypack:moon").unwrap();
        assert_eq!(moon.source, LevelSource::Saved);
        assert_eq!(moon.stem_json.as_ref(), Some(&saved));
    }

    #[test]
    fn a_pack_stem_with_a_vanilla_id_replaces_the_saved_vanilla_one() {
        let saved = grid_stem("minecraft:overworld");
        let mut pack = grid_stem("minecraft:overworld");
        pack["generator"]["biome_source"]["outside"] = "minecraft:desert".into();
        let o = overrides_with(&[("minecraft:overworld", pack.clone())], &[]);
        let reg = build(&o, &[("minecraft:overworld", saved)]);
        let e = reg.get("minecraft:overworld").unwrap();
        assert_eq!(e.source, LevelSource::Vanilla);
        assert_eq!(e.stem_from, Some(LevelSource::Pack));
        assert_eq!(e.stem_json.as_ref(), Some(&pack));
        assert_eq!(reg.ids().count(), 3);
    }

    #[test]
    fn bad_stems_are_reported_and_never_touch_the_vanilla_levels() {
        let mut no_type = grid_stem("minecraft:overworld");
        no_type.as_object_mut().unwrap().remove("type");
        let mut bad_grid = grid_stem("minecraft:overworld");
        bad_grid["generator"]["biome_source"]["cells"] = serde_json::json!(["minecraft:plains"]);
        let mut o = overrides_with(
            &[
                ("a:unknown_type", grid_stem("nope:missing")),
                ("a:no_type", no_type),
                ("a:bad_grid", bad_grid),
                ("a:../escape", grid_stem("minecraft:overworld")),
                ("a:good", grid_stem("minecraft:overworld")),
            ],
            &[("a:bad_type", serde_json::json!({"min_y": 1, "height": 16}))],
        );
        o.set(overrides::DIMENSIONS, "a:not_json", "{ nope".into());
        let reg = build(&o, &[]);
        let ids: Vec<_> = reg.ids().collect();
        assert_eq!(ids, [VANILLA_LEVELS[0], VANILLA_LEVELS[1], VANILLA_LEVELS[2], "a:good"]);
        let msg = |id: &str| {
            reg.problems()
                .iter()
                .find(|p| p.id == id)
                .unwrap_or_else(|| panic!("no problem for {id}: {:?}", reg.problems()))
                .message
                .clone()
        };
        assert!(msg("a:unknown_type").contains("unknown dimension type `nope:missing`"));
        assert!(msg("a:no_type").contains("missing field `type`"));
        assert!(msg("a:bad_grid").contains("biome source"));
        assert!(msg("a:../escape").contains("not usable"));
        assert!(msg("a:not_json").contains("invalid JSON"));
        assert!(msg("a:bad_type").contains("dimension type"));
        // The vanilla levels are what they always were.
        for id in VANILLA_LEVELS {
            assert_eq!(reg.shape(id), Some(LevelShapes::VANILLA.get(id)));
        }
    }

    #[test]
    fn a_pack_stem_that_does_not_validate_leaves_a_vanilla_level_working() {
        let o = overrides_with(
            &[("minecraft:the_nether", serde_json::json!({"type":"nope:x","generator":{"type":"minecraft:noise"}}))],
            &[],
        );
        let reg = build(&o, &[]);
        assert_eq!(reg.ids().count(), 3);
        assert!(reg.problems().iter().any(|p| p.id == "minecraft:the_nether"));
        assert_eq!(reg.shape("minecraft:the_nether"), Some(LevelShape::NETHER));
        assert_eq!(reg.coordinate_scale("minecraft:the_nether"), 8.0);
    }

    #[test]
    fn a_non_vanilla_stem_without_a_catalog_is_left_out_with_a_message() {
        let o = overrides_with(&[("mypack:moon", grid_stem("minecraft:overworld"))], &[]);
        let reg = LevelRegistry::build(LevelInputs {
            noise_sea: Default::default(),
            prepared_types: BTreeMap::new(),
            overrides: &o,
            saved_stems: BTreeMap::new(),
            vanilla_shapes: LevelShapes::VANILLA,
            catalog: None,
        });
        assert_eq!(reg.ids().count(), 3);
        assert!(reg.problems()[0].message.contains("worldgen data is not available"));
    }

    #[test]
    fn mods_register_levels_until_the_world_ticks() {
        let cat = catalog();
        let mut reg = LevelRegistry::vanilla();
        let e = reg
            .register_level("mymod:arena", grid_stem("minecraft:overworld"), &cat)
            .unwrap();
        assert_eq!(e.source, LevelSource::Mod);
        assert_eq!(reg.ids().last(), Some("mymod:arena"));
        assert_eq!(
            reg.register_level("mymod:arena", grid_stem("minecraft:overworld"), &cat).unwrap_err(),
            LevelError::Duplicate("mymod:arena".into())
        );
        assert_eq!(
            reg.register_level("minecraft:overworld", grid_stem("minecraft:overworld"), &cat)
                .unwrap_err(),
            LevelError::Duplicate("minecraft:overworld".into())
        );
        assert!(matches!(
            reg.register_level("Bad Id", grid_stem("minecraft:overworld"), &cat),
            Err(LevelError::BadId { .. })
        ));
        assert!(matches!(
            reg.register_level("mymod:x", grid_stem("mymod:unregistered"), &cat),
            Err(LevelError::Stem { .. })
        ));
        // A mod's own type, registered first, is written into the stem for the generator.
        reg.register_type("mymod:small", DimensionType::new(0, 64)).unwrap();
        let e = reg
            .register_level("mymod:small_level", grid_stem("mymod:small"), &cat)
            .unwrap();
        assert_eq!(e.shape.height, 64);
        assert_eq!(e.stem_json.as_ref().unwrap()["type"]["height"], 64);
        reg.freeze();
        assert_eq!(
            reg.register_level("mymod:late", grid_stem("minecraft:overworld"), &cat).unwrap_err(),
            LevelError::Frozen
        );
        assert_eq!(
            reg.register_type("mymod:late", DimensionType::new(0, 64)).unwrap_err(),
            LevelError::Frozen
        );
        assert!(!reg.contains("mymod:late"));
    }

    #[test]
    fn level_ids_are_checked_before_they_name_a_folder() {
        for ok in ["a:b", "mod_x:arena/one", "a.b:c-d"] {
            assert_eq!(check_level_id(ok), Ok(()), "{ok}");
        }
        for bad in [
            "plain", ":x", "x:", "A:b", "a:B", "a:../b", "a:b/..", "a:./b", "a:b//c", "..:x",
            "a:b\\c", "a:b c",
        ] {
            assert!(check_level_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn prepared_types_fill_the_visual_attributes_of_the_vanilla_types() {
        // A prepared overworld file with a visual attribute: the built-in
        // carries only the gameplay ones, the loaded one has both.
        let mut prepared = BTreeMap::new();
        prepared.insert(
            "minecraft:overworld".to_owned(),
            serde_json::json!({
                "min_y": -64, "height": 384, "logical_height": 384,
                "has_skylight": true, "has_ceiling": false,
                "attributes": {"minecraft:visual/sky_color": "#78a7ff"}
            })
            .to_string(),
        );
        let reg = LevelRegistry::build(LevelInputs {
            noise_sea: Default::default(),
            prepared_types: prepared,
            overrides: &WorldgenOverrides::default(),
            saved_stems: BTreeMap::new(),
            vanilla_shapes: LevelShapes::VANILLA,
            catalog: None,
        });
        assert!(reg
            .get("minecraft:overworld")
            .unwrap()
            .dimension_type
            .attributes
            .contains_key("minecraft:visual/sky_color"));
        assert!(reg.problems().is_empty(), "{:?}", reg.problems());
    }

    /// The whole path from disk: a world folder with a fixture data pack (self-made JSON), read
    /// by the same functions the game uses, gives a fourth level with its own save folder; the
    /// vanilla levels are untouched. A second pack with a bad stem does not break the first.
    #[test]
    fn a_fixture_data_pack_in_a_world_folder_loads_as_a_fourth_level() {
        let dir = std::env::temp_dir().join(format!("hz-f33-levels-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let write = |pack: &str, path: &str, text: &str| {
            let p = dir.join("datapacks").join(pack).join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write("moon", "pack.mcmeta", "{}");
        write(
            "moon",
            "data/mypack/dimension/moon.json",
            &grid_stem("minecraft:overworld").to_string(),
        );
        write("broken", "pack.mcmeta", "{}");
        write(
            "broken",
            "data/mypack/dimension/ghost.json",
            &grid_stem("mypack:nowhere").to_string(),
        );
        let cat = catalog();
        let packs = crate::datapack::discover(&dir);
        let enabled = crate::datapack::enabled_ids(&dir, &packs, &[]);
        assert_eq!(enabled, ["file/broken", "file/moon"]);
        let enabled: Vec<_> = enabled
            .iter()
            .filter_map(|id| packs.iter().find(|p| &p.id == id))
            .collect();
        let (overrides, _) = crate::datapack::worldgen_overrides(&enabled);
        assert!(LevelRegistry::needs_catalog(&dir, &overrides));
        let reg = LevelRegistry::for_world(
            &dir,
            Path::new("no-such-resources"),
            &overrides,
            LevelShapes::VANILLA,
            Some(&cat),
        );
        let ids: Vec<_> = reg.ids().collect();
        assert_eq!(
            ids,
            [VANILLA_LEVELS[0], VANILLA_LEVELS[1], VANILLA_LEVELS[2], "mypack:moon"]
        );
        assert_eq!(reg.problems().len(), 1, "{:?}", reg.problems());
        assert!(reg.problems()[0].message.contains("mypack:nowhere"));
        assert_eq!(
            reg.save_dir(&dir, "mypack:moon"),
            dir.join("dimensions").join("mypack").join("moon")
        );
        // With no pack at all: the three levels, no catalog needed.
        let none = WorldgenOverrides::default();
        assert!(!LevelRegistry::needs_catalog(&dir, &none));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Stems saved in `world_gen_settings` become levels too.
    #[test]
    fn a_saved_stem_in_world_gen_settings_is_a_level() {
        let dir = std::env::temp_dir().join(format!("hz-f33-saved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::save::write_world_gen_settings(&dir, 7, crate::save::WorldGen::default()).unwrap();
        let saved = LevelRegistry::saved_stems(&dir);
        assert_eq!(
            saved.keys().map(String::as_str).collect::<Vec<_>>(),
            ["minecraft:overworld", "minecraft:the_end", "minecraft:the_nether"]
        );
        // A fresh world: the registry is the plain three, whatever the saved stems hold.
        let cat = catalog();
        let reg = LevelRegistry::for_world(
            &dir,
            Path::new("none"),
            &WorldgenOverrides::default(),
            LevelShapes::VANILLA,
            Some(&cat),
        );
        assert_eq!(reg.ids().collect::<Vec<_>>(), VANILLA_LEVELS);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Against the real prepared bundle when it is around: every vanilla type
    /// loads without a problem and the shapes match `LevelShapes::VANILLA`.
    #[test]
    fn the_real_prepared_dimension_types_load_and_match_the_vanilla_shapes() {
        let Some(resources) = std::env::var_os("HORIZON_RESOURCES")
            .map(PathBuf::from)
            .or_else(|| {
                let p = PathBuf::from(
                    r"C:\Users\USER\NestgateHorizon\horizon\.local\horizon-client\26.3",
                );
                p.is_dir().then_some(p)
            })
        else {
            return;
        };
        let prepared = prepared_dimension_types(&resources);
        if prepared.is_empty() {
            return;
        }
        let reg = LevelRegistry::build(LevelInputs {
            noise_sea: Default::default(),
            prepared_types: prepared,
            overrides: &WorldgenOverrides::default(),
            saved_stems: BTreeMap::new(),
            vanilla_shapes: LevelShapes::VANILLA,
            catalog: None,
        });
        assert!(reg.problems().is_empty(), "{:?}", reg.problems());
        for id in VANILLA_LEVELS {
            let t = &reg.get(id).unwrap().dimension_type;
            assert_eq!(shape_of(t).unwrap(), LevelShapes::VANILLA.get(id), "{id}");
            assert!(
                t.attributes.keys().any(|k| k.starts_with("minecraft:visual/")),
                "{id} has no visual attributes: {:?}",
                t.attributes.keys().collect::<Vec<_>>()
            );
        }
    }
}
