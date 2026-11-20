//! Chunk generators a mod registers by id (F3-5), for levels whose stem says
//! `{"type": "yoake:mod", "id": "<mod>:<generator>", "settings": {...}}`.
//!
//! This is the native side: a Rust mod (or the engine on behalf of a WASM mod
//! later, F3-8) registers a [`ModGeneratorFactory`]; when the level opens the
//! factory makes a [`ModColumnSource`] for it (given the level's seed, shape
//! and the stem's settings) and the engine asks the source to fill each chunk
//! as a proto chunk. There is no decoration or structure step: the source
//! writes the final blocks and biomes of the chunk it is given. Chunks are
//! generated on worker threads, so a source is `Send + Sync` and takes `&self`.
use crate::{
    assets::Assets,
    biome::Biomes,
    chunk::{BIOME_STRATEGY, Chunk, LevelShape},
    streaming::{ChunkGenerator, Proto},
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};

/// What a source writes into: one chunk of the level, `min_y..min_y + height` tall.
pub struct ModChunk<'a> {
    chunk: &'a mut Chunk,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub shape: LevelShape,
    biome_of: &'a dyn Fn(&str) -> Option<u16>,
    failed: bool,
}
impl ModChunk<'_> {
    /// Says the generator could not fill this chunk (it ran over its limits, say). What it wrote stays,
    /// but the chunk is not saved while nobody edits it, so it generates again when it next loads.
    pub fn mark_failed(&mut self) {
        self.failed = true;
    }
    /// Block `state` (a state id from [`ModGeneratorContext::block`]) at the
    /// chunk-local column `x`, `z` (0..16) and world height `y`. Positions
    /// outside the level's height are ignored.
    pub fn set_block(&mut self, x: i32, y: i32, z: i32, state: u32) {
        if (0..16).contains(&x)
            && (0..16).contains(&z)
            && (self.shape.min_y..self.shape.min_y + self.shape.height).contains(&y)
        {
            self.chunk.set(x, y, z, state);
        }
    }
    /// Every block of `y` in the chunk.
    pub fn fill_layer(&mut self, y: i32, state: u32) {
        for z in 0..16 {
            for x in 0..16 {
                self.set_block(x, y, z, state);
            }
        }
    }
    /// The state at a chunk-local position (air where nothing was written).
    pub fn block(&self, x: i32, y: i32, z: i32) -> u32 {
        self.chunk.get(x, y, z)
    }
    /// The whole chunk's biome (the default is the plains).
    pub fn set_biome(&mut self, name: &str) -> bool {
        let Some(id) = (self.biome_of)(name) else {
            return false;
        };
        for s in &mut self.chunk.sections {
            Arc::make_mut(s).biomes =
                horizon_core::paletted::PalettedContainer::new(BIOME_STRATEGY, u32::from(id));
        }
        true
    }
}

/// A generator of chunks: given the position of a chunk, fills it.
pub trait ModColumnSource: Send + Sync + 'static {
    fn generate(&self, chunk: &mut ModChunk<'_>);
}

/// What a factory is given when its level opens.
pub struct ModGeneratorContext {
    /// The level this generator makes chunks for.
    pub level: String,
    /// The world's seed (every level has it, as in vanilla).
    pub seed: i64,
    pub shape: LevelShape,
    /// The generator id the stem names (`yoake:mod`'s `id`).
    pub generator: String,
    /// The stem's `generator.settings` object.
    pub settings: Value,
    assets: Arc<Assets>,
}
impl ModGeneratorContext {
    /// The default state id of a block (`minecraft:stone`).
    pub fn block(&self, name: &str) -> Option<u32> {
        self.assets.registry.default_state(name).map(|s| s.0)
    }
    /// The same lookup, to keep and call from the generating threads.
    pub fn block_lookup(&self) -> Arc<dyn Fn(&str) -> Option<u32> + Send + Sync> {
        let assets = self.assets.clone();
        Arc::new(move |name| assets.registry.default_state(name).map(|s| s.0))
    }
}

/// A generator a mod registers: makes the source of a level.
pub trait ModGeneratorFactory: Send + Sync + 'static {
    fn create(&self, context: &ModGeneratorContext) -> Result<Arc<dyn ModColumnSource>, String>;
}

/// The registered generators, by id.
#[derive(Clone, Default)]
pub struct ModGenerators {
    factories: BTreeMap<String, Arc<dyn ModGeneratorFactory>>,
}
impl ModGenerators {
    /// Registers `factory` under `id` (`mod:name`).
    pub fn register(
        &mut self,
        id: &str,
        factory: Arc<dyn ModGeneratorFactory>,
    ) -> Result<(), String> {
        if id.is_empty() || !id.contains(':') || id.chars().any(char::is_whitespace) {
            return Err(format!("`{id}` is not a namespaced generator id"));
        }
        if self.factories.contains_key(id) {
            return Err(format!("generator `{id}` is already registered"));
        }
        self.factories.insert(id.to_owned(), factory);
        Ok(())
    }
    pub fn factory(&self, id: &str) -> Option<Arc<dyn ModGeneratorFactory>> {
        self.factories.get(id).cloned()
    }
    pub fn contains(&self, id: &str) -> bool {
        self.factories.contains_key(id)
    }
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.factories.keys().map(String::as_str)
    }
    /// The terrain of a level whose stem asks for generator `id`.
    pub fn terrain(
        &self,
        id: &str,
        level: &str,
        seed: i64,
        shape: LevelShape,
        settings: Value,
        assets: Arc<Assets>,
        biomes: Arc<Biomes>,
    ) -> Result<ModTerrain, String> {
        let factory = self.factories.get(id).ok_or_else(|| {
            format!(
                "level {level} asks for generator `{id}`, which no mod registered (registered: {})",
                self.factories.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        let context = ModGeneratorContext {
            generator: id.to_owned(),
            level: level.to_owned(),
            seed,
            shape,
            settings,
            assets,
        };
        let source = factory
            .create(&context)
            .map_err(|e| format!("generator `{id}` for level {level}: {e}"))?;
        let plains = biomes.id("minecraft:plains");
        Ok(ModTerrain {
            level: level.to_owned(),
            source,
            shape,
            plains,
            biomes,
        })
    }
}

/// A chunk of a level made by a mod's generator is about to be filled (F3-8b). A mod's hook here sees
/// the empty chunk first and may write into it through the `ModChunk` bindings (`set_block`,
/// `fill_layer`, `block`, `set_biome`); the level's generator then fills it. Only levels of mod
/// generators, so the vanilla terrain is untouched. About one call per chunk.
#[yoake_inject_macro::yoake_inject(id = "worldgen::generate_chunk", side = "server")]
fn generate_chunk(level: &str, cx: i32, cz: i32, chunk: &mut ModChunk<'_>) {
    let _ = (level, cx, cz, chunk);
}

/// A [`ModColumnSource`] as the engine's chunk generator.
pub struct ModTerrain {
    level: String,
    source: Arc<dyn ModColumnSource>,
    shape: LevelShape,
    plains: u16,
    biomes: Arc<Biomes>,
}
impl ChunkGenerator for ModTerrain {
    type Worker = ();
    fn worker(&self) {}
    fn proto(&self, _: &mut (), cx: i32, cz: i32) -> Proto {
        let mut chunk = Chunk::with_layout(cx, cz, self.shape.column());
        for s in &mut chunk.sections {
            Arc::make_mut(s).biomes = horizon_core::paletted::PalettedContainer::new(
                BIOME_STRATEGY,
                u32::from(self.plains),
            );
        }
        let biomes = self.biomes.clone();
        let biome_of = move |name: &str| {
            biomes.contains(name).then(|| biomes.id(name))
        };
        let mut mod_chunk = ModChunk {
            chunk: &mut chunk,
            chunk_x: cx,
            chunk_z: cz,
            shape: self.shape,
            biome_of: &biome_of,
            failed: false,
        };
        generate_chunk(&self.level, cx, cz, &mut mod_chunk);
        self.source.generate(&mut mod_chunk);
        let failed = mod_chunk.failed;
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        chunk.not_generated = failed;
        Box::new(chunk)
    }
    fn finish(&self, proto: Proto) -> Chunk {
        *proto.downcast::<Chunk>().expect("mod terrain proto")
    }
}

/// What mods registered before a world was read (F3-6): levels and generators the world's own
/// loading takes in, so a player saved in a level of a mod reopens there. Process wide, because
/// native mods start before any world opens and the integrated server reads worlds on its own
/// thread; a world reads it when it opens and never drains it.
#[derive(Default)]
struct Staged {
    levels: Vec<(String, Value)>,
    /// Dimension types mods register (`id`, the type's JSON), before any level that names them.
    types: Vec<(String, Value)>,
    generators: ModGenerators,
}
static STAGED: std::sync::Mutex<Option<Staged>> = std::sync::Mutex::new(None);
fn staged_lock() -> std::sync::MutexGuard<'static, Option<Staged>> {
    STAGED.lock().unwrap_or_else(|e| e.into_inner())
}
/// Registers a level stem (a level of `id`) for every world opened from now on. Refused when `id`
/// is staged already or is not a usable level id.
pub fn stage_level(id: &str, stem: Value) -> Result<(), String> {
    crate::level_registry::check_level_id(id).map_err(|r| format!("level id `{id}`: {r}"))?;
    let mut staged = staged_lock();
    let staged = staged.get_or_insert_with(Staged::default);
    if staged.levels.iter().any(|(l, _)| l == id) {
        return Err(format!("level `{id}` is already staged"));
    }
    staged.levels.push((id.to_owned(), stem));
    Ok(())
}
/// Registers a level stem like `stage_level`, replacing the one a package registered before under the same id (a
/// WebAssembly package's `init` runs again at each world start and registers again).
pub fn stage_level_replacing(id: &str, stem: Value) -> Result<(), String> {
    crate::level_registry::check_level_id(id).map_err(|r| format!("level id `{id}`: {r}"))?;
    let mut staged = staged_lock();
    let staged = staged.get_or_insert_with(Staged::default);
    staged.levels.retain(|(l, _)| l != id);
    staged.levels.push((id.to_owned(), stem));
    Ok(())
}
/// Registers a dimension type (its JSON, as a data pack's file) for every world opened from now on; a later
/// registration of the same id replaces it.
pub fn stage_type(id: &str, json: Value) -> Result<(), String> {
    crate::level_registry::check_level_id(id).map_err(|r| format!("type id `{id}`: {r}"))?;
    horizon_world::dimension::DimensionType::from_value(&json).map_err(|e| format!("type `{id}`: {e}"))?;
    let mut staged = staged_lock();
    let staged = staged.get_or_insert_with(Staged::default);
    staged.types.retain(|(t, _)| t != id);
    staged.types.push((id.to_owned(), json));
    Ok(())
}
/// The types staged so far.
pub fn staged_types() -> Vec<(String, Value)> {
    staged_lock().as_ref().map_or_else(Vec::new, |s| s.types.clone())
}
/// Registers a chunk generator like `stage_generator`, replacing the one registered under the same id.
pub fn stage_generator_replacing(id: &str, factory: Arc<dyn ModGeneratorFactory>) -> Result<(), String> {
    let mut staged = staged_lock();
    let staged = staged.get_or_insert_with(Staged::default);
    staged.generators.factories.remove(id);
    staged.generators.register(id, factory)
}
/// Registers a chunk generator for every world opened from now on.
pub fn stage_generator(id: &str, factory: Arc<dyn ModGeneratorFactory>) -> Result<(), String> {
    staged_lock()
        .get_or_insert_with(Staged::default)
        .generators
        .register(id, factory)
}
/// The levels and generators staged so far.
pub fn staged() -> (Vec<(String, Value)>, ModGenerators) {
    staged_lock()
        .as_ref()
        .map_or_else(Default::default, |s| (s.levels.clone(), s.generators.clone()))
}
#[cfg(test)]
mod tests {
    use super::*;

    struct Nothing;
    impl ModGeneratorFactory for Nothing {
        fn create(&self, _: &ModGeneratorContext) -> Result<Arc<dyn ModColumnSource>, String> {
            Err("not made in this test".into())
        }
    }

    #[test]
    fn staged_levels_and_generators_are_kept_for_every_world() {
        clear_staged();
        stage_level("a:one", serde_json::json!({})).unwrap();
        assert!(stage_level("a:one", serde_json::json!({})).is_err(), "twice");
        assert!(stage_level("../x:y", serde_json::json!({})).is_err(), "an unusable id");
        stage_generator("a:gen", Arc::new(Nothing)).unwrap();
        assert!(stage_generator("a:gen", Arc::new(Nothing)).is_err());
        // A world reading them never drains them.
        for _ in 0..2 {
            let (levels, generators) = staged();
            assert_eq!(levels.len(), 1);
            assert!(generators.contains("a:gen"));
        }
        clear_staged();
        assert!(staged().0.is_empty());
    }
}

/// Forgets everything staged (tests).
pub fn clear_staged() {
    *staged_lock() = None;
}
