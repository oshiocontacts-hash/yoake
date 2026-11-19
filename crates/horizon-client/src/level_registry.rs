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
}
impl fmt::Display for LevelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frozen => write!(f, "levels can only be registered before the world starts ticking"),
            Self::Duplicate(id) => write!(f, "level `{id}` is already registered"),
            Self::BadId { id, reason } => write!(f, "level id `{id}` is not usable: {reason}"),
            Self::Stem { id, error } => write!(f, "level stem `{id}`: {error}"),
            Self::Unsupported { id, reason } => write!(f, "level `{id}`: {reason}"),
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
}

impl LevelRegistry {
    /// The three vanilla levels with the built-in types and no stems: what a
    /// world with no packs, no mods and no extra stems is.
    pub fn vanilla() -> Self {
        Self::build(LevelInputs {
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
        // Superflat and debug generators are built for the Overworld only
        // until F3-5 builds a generator for every level.
        if !matches!(stem.generator, GeneratorSpec::Noise { .. }) {
            return Err(LevelError::Unsupported {
                id: id.to_owned(),
                reason: "only noise generators run outside the three vanilla levels for now \
                         (flat and debug generators for any level come with F3-5)"
                    .to_owned(),
            });
        }
        let shape = shape_of(&dimension_type).map_err(|reason| LevelError::Unsupported {
            id: id.to_owned(),
            reason: format!("its dimension type's shape is refused: {reason}"),
        })?;
        // The generator resolves the stem's type by id from the packs' data;
        // a type only the registry knows (a mod's) is written into the stem.
        let mut stem_json = json;
        if matches!(stem.dimension_type, TypeRef::Id(_)) && from == LevelSource::Mod {
            stem_json["type"] = dimension_type.to_value();
        }
        Ok(LevelEntry {
            id: id.to_owned(),
            source: from,
            stem_from: Some(from),
            stem_json: Some(stem_json),
            stem: Some(stem),
            dimension_type,
            shape,
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
    /// The level's save folder inside the world.
    pub fn save_dir(&self, world: &Path, id: &str) -> PathBuf {
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
        let mut flat = grid_stem("minecraft:overworld");
        flat["generator"] = serde_json::json!({"type":"minecraft:flat","settings":{}});
        let mut o = overrides_with(
            &[
                ("a:unknown_type", grid_stem("nope:missing")),
                ("a:no_type", no_type),
                ("a:bad_grid", bad_grid),
                ("a:flat", flat),
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
        assert!(msg("a:flat").contains("F3-5"));
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
        let (overrides, _) = crate::datapack::worldgen_overrides(&enabled, |b| {
            ["minecraft:plains", "minecraft:desert", "minecraft:the_void"].contains(&b)
        });
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
