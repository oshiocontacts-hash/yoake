//! Chunk generators a mod registers by id (F3-5), for levels whose stem says
//! `{"type": "yoake:mod", "id": "<mod>:<generator>", "settings": {...}}`.
//!
//! This is the native side: a Rust mod (or the engine on behalf of a WASM mod
//! later, F3-8) registers a [`ModGeneratorFactory`]; when the level opens the
//! factory makes a [`ModColumnSource`] for it (given the level's seed, shape
//! and the stem's settings) and the engine asks the source to edit each chunk
//! after its optional noise base. The stem's `decorate` and `structures` flags
//! then select the same post-processing pipeline as a noise level. Chunks are
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
pub struct ModChunk {
    chunk: Chunk,
    pub chunk_x: i32,
    pub chunk_z: i32,
    pub shape: LevelShape,
    biome_of: Arc<dyn Fn(&str) -> Option<u16> + Send + Sync>,
    failed: bool,
}
impl ModChunk {
    /// Says the generator could not fill this chunk (it ran over its limits, say). The fallback is
    /// all air and is not saved while nobody edits it, so it generates again when it next loads.
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
    /// One 4x4x4 biome cell. `qx` and `qz` are 0..4 within the chunk and `qy` counts from the
    /// level's bottom, not from the bottom of the widened storage column. Out-of-range cells and
    /// unknown biome names are refused without changing the chunk.
    pub fn set_biome_at(&mut self, qx: i32, qy: i32, qz: i32, name: &str) -> bool {
        let Some(id) = (self.biome_of)(name) else {
            return false;
        };
        self.set_biome_id_at(qx, qy, qz, id)
    }
    #[doc(hidden)]
    pub fn set_biome_id_at(&mut self, qx: i32, qy: i32, qz: i32, id: u16) -> bool {
        if !(0..4).contains(&qx)
            || !(0..4).contains(&qz)
            || !(0..self.shape.height / 4).contains(&qy)
        {
            return false;
        }
        let column_qy = qy + (self.shape.min_y - self.chunk.min_y()) / 4;
        self.chunk.set_biome_cell(qx, column_qy, qz, id);
        true
    }
    #[doc(hidden)]
    pub fn for_test(shape: LevelShape, biome_of: Arc<dyn Fn(&str) -> Option<u16> + Send + Sync>) -> Self {
        Self {
            chunk: Chunk::with_layout(0, 0, shape.column()),
            chunk_x: 0,
            chunk_z: 0,
            shape,
            biome_of,
            failed: false,
        }
    }
    #[doc(hidden)]
    pub fn biome_at_block(&self, x: i32, y: i32, z: i32) -> u16 {
        self.chunk.biome(x, y, z)
    }
}

/// A generator of chunks: given the position of a chunk, fills it.
pub trait ModColumnSource: Send + Sync + 'static {
    fn generate(&self, chunk: &mut ModChunk);
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
    assets: Option<Arc<Assets>>,
    biome_of: Arc<dyn Fn(&str) -> Option<u16> + Send + Sync>,
}
impl ModGeneratorContext {
    /// The default state id of a block (`minecraft:stone`).
    pub fn block(&self, name: &str) -> Option<u32> {
        self.assets
            .as_ref()?
            .registry
            .default_state(name)
            .map(|s| s.0)
    }
    /// The same lookup, to keep and call from the generating threads.
    pub fn block_lookup(&self) -> Arc<dyn Fn(&str) -> Option<u32> + Send + Sync> {
        let assets = self.assets.clone();
        Arc::new(move |name| {
            assets
                .as_ref()?
                .registry
                .default_state(name)
                .map(|s| s.0)
        })
    }
    /// A biome's table id, to keep and call from the generating threads.
    pub fn biome_lookup(&self) -> Arc<dyn Fn(&str) -> Option<u16> + Send + Sync> {
        self.biome_of.clone()
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
        based: bool,
        decorate: bool,
        structures: bool,
        pipeline: Option<Arc<crate::vanillagen::VanillaTerrain>>,
    ) -> Result<ModTerrain<crate::vanillagen::VanillaTerrain>, String> {
        let factory = self.factories.get(id).ok_or_else(|| {
            format!(
                "level {level} asks for generator `{id}`, which no mod registered (registered: {})",
                self.factories.keys().cloned().collect::<Vec<_>>().join(", ")
            )
        })?;
        let plains = biomes.id("minecraft:plains");
        let biome_of = Arc::new(move |name: &str| biomes.contains(name).then(|| biomes.id(name)));
        let context = ModGeneratorContext {
            generator: id.to_owned(),
            level: level.to_owned(),
            seed,
            shape,
            settings,
            assets: Some(assets),
            biome_of: biome_of.clone(),
        };
        let source = factory
            .create(&context)
            .map_err(|e| format!("generator `{id}` for level {level}: {e}"))?;
        Ok(ModTerrain {
            level: level.to_owned(),
            source,
            shape,
            plains,
            biome_of,
            based,
            decorate,
            structures,
            pipeline,
        })
    }
}

/// A chunk of a level made by a mod's generator is about to be filled (F3-8b). A mod's hook here sees
/// the base column first (air without a base) and may write into it through the `ModChunk` bindings
/// (`set_block`, `fill_layer`, `block`, `set_biome`, `set_biome_at`); the level's generator then edits it. Only levels
/// of mod generators, so vanilla terrain is untouched. About one call per chunk.
#[yoake_inject_macro::yoake_inject(id = "worldgen::generate_chunk", side = "server")]
fn generate_chunk(level: &str, cx: i32, cz: i32, chunk: &mut ModChunk) {
    let _ = (level, cx, cz, chunk);
}

/// The proto-column conversion a mod generator needs from its engine pipeline.
#[doc(hidden)]
pub trait ModBase: ChunkGenerator {
    fn mod_column(&self, proto: &Proto) -> Option<Chunk>;
    fn mod_restore(&self, proto: &mut Proto, saved: crate::save::SavedProto) {
        self.mod_replace(proto, saved.column);
    }
    fn mod_replace(&self, proto: &mut Proto, column: Chunk);
    fn mod_decorate(&self, worker: &mut Self::Worker, region: &mut [Proto; 9], _features: bool) {
        self.decorate(worker, region);
    }
}

struct ModProto {
    inner: Proto,
    failed: bool,
}

/// A [`ModColumnSource`] over an optional engine terrain/decoration pipeline.
pub struct ModTerrain<G = crate::vanillagen::VanillaTerrain> {
    level: String,
    source: Arc<dyn ModColumnSource>,
    shape: LevelShape,
    plains: u16,
    biome_of: Arc<dyn Fn(&str) -> Option<u16> + Send + Sync>,
    based: bool,
    decorate: bool,
    structures: bool,
    pipeline: Option<Arc<G>>,
}

fn air_with_biomes(shape: LevelShape, source: &Chunk) -> Chunk {
    let mut air = Chunk::with_layout(source.x, source.z, shape.column());
    for (to, from) in air.sections.iter_mut().zip(&source.sections) {
        Arc::make_mut(to).biomes = from.biomes.clone();
    }
    air
}

impl<G: ModBase> ModTerrain<G> {
    fn empty(&self, x: i32, z: i32) -> Chunk {
        let mut chunk = Chunk::with_layout(x, z, self.shape.column());
        for section in &mut chunk.sections {
            Arc::make_mut(section).biomes = horizon_core::paletted::PalettedContainer::new(
                BIOME_STRATEGY,
                u32::from(self.plains),
            );
        }
        chunk
    }
    fn wrapped(proto: &Proto) -> &ModProto {
        proto.downcast_ref::<ModProto>().expect("mod terrain proto")
    }
}

impl<G: ModBase> ChunkGenerator for ModTerrain<G> {
    type Worker = Option<G::Worker>;
    fn worker(&self) -> Self::Worker {
        self.pipeline.as_ref().map(|base| base.worker())
    }
    fn idle(&self, worker: &mut Self::Worker) {
        if let (Some(base), Some(worker)) = (&self.pipeline, worker) {
            base.idle(worker);
        }
    }
    fn all_idle(&self) {
        if let Some(base) = &self.pipeline {
            base.all_idle();
        }
    }
    fn prepare_loaded(&self, worker: &mut Self::Worker, chunk: &Chunk) {
        if let (Some(base), Some(worker)) = (&self.pipeline, worker) {
            base.prepare_loaded(worker, chunk);
        }
    }
    fn proto(&self, worker: &mut Self::Worker, cx: i32, cz: i32) -> Proto {
        let (mut inner, mut chunk) = if let Some(base) = &self.pipeline {
            let worker = worker.as_mut().expect("mod base worker");
            let inner = base.proto(worker, cx, cz);
            let mut column = base.mod_column(&inner).expect("mod base column");
            if !self.based {
                column = air_with_biomes(self.shape, &column);
            }
            (inner, column)
        } else {
            (Box::new(()) as Proto, self.empty(cx, cz))
        };
        let mut mod_chunk = ModChunk {
            chunk,
            chunk_x: cx,
            chunk_z: cz,
            shape: self.shape,
            biome_of: self.biome_of.clone(),
            failed: false,
        };
        generate_chunk(&self.level, cx, cz, &mut mod_chunk);
        self.source.generate(&mut mod_chunk);
        let failed = mod_chunk.failed;
        chunk = mod_chunk.chunk;
        if failed {
            chunk = air_with_biomes(self.shape, &chunk);
        }
        if let Some(base) = &self.pipeline {
            base.mod_replace(&mut inner, chunk);
        } else {
            for section in &mut chunk.sections {
                Arc::make_mut(section).compact();
            }
            inner = Box::new(chunk);
        }
        Box::new(ModProto { inner, failed })
    }
    fn decorates(&self) -> bool {
        self.pipeline.is_some() && (self.decorate || self.structures)
    }
    fn decorates_beside_final(&self) -> bool {
        self.decorates()
    }
    fn proto_beside_final(&self, worker: &mut Self::Worker, snapshot: &Chunk) -> Option<Proto> {
        let base = self.pipeline.as_ref()?;
        let proto = self.proto(worker, snapshot.x, snapshot.z);
        let mut proto = proto.downcast::<ModProto>().expect("mod terrain proto");
        base.mod_restore(
            &mut proto.inner,
            crate::save::SavedProto {
                column: snapshot.clone(),
                decorated: true,
                block_entities: Vec::new(),
                entities: Vec::new(),
            },
        );
        proto.failed = false;
        Some(proto)
    }
    fn final_writes(&self, snapshot: &Chunk, proto: &Proto) -> Vec<([i32; 3], u32)> {
        self.pipeline.as_ref().map_or_else(Vec::new, |base| {
            base.final_writes(snapshot, &Self::wrapped(proto).inner)
        })
    }
    fn final_block_entities(
        &self,
        proto: &Proto,
        writes: &[([i32; 3], u32)],
    ) -> Vec<((i32, i32, i32), crate::chunk::block_entity::BlockEntity)> {
        self.pipeline.as_ref().map_or_else(Vec::new, |base| {
            base.final_block_entities(&Self::wrapped(proto).inner, writes)
        })
    }
    fn decorate(&self, worker: &mut Self::Worker, region: &mut [Proto; 9]) {
        let Some(base) = &self.pipeline else {
            return;
        };
        let taken: Vec<ModProto> = region
            .iter_mut()
            .map(|proto| {
                *std::mem::replace(proto, Box::new(()))
                    .downcast::<ModProto>()
                    .expect("mod terrain proto")
            })
            .collect();
        if taken.iter().any(|proto| proto.failed) {
            for (slot, proto) in region.iter_mut().zip(taken) {
                *slot = Box::new(proto);
            }
            return;
        }
        let mut inner: [Proto; 9] = taken
            .into_iter()
            .map(|proto| proto.inner)
            .collect::<Vec<_>>()
            .try_into()
            .expect("nine mod protos");
        base.mod_decorate(
            worker.as_mut().expect("mod base worker"),
            &mut inner,
            self.decorate,
        );
        for (slot, inner) in region.iter_mut().zip(inner) {
            *slot = Box::new(ModProto {
                inner,
                failed: false,
            });
        }
    }
    fn is_decorated(&self, proto: &Proto) -> bool {
        self.pipeline.as_ref().is_some_and(|base| {
            base.is_decorated(&Self::wrapped(proto).inner)
        })
    }
    fn light_view(&self, proto: &Proto) -> Option<Chunk> {
        self.pipeline
            .as_ref()
            .and_then(|base| base.light_view(&Self::wrapped(proto).inner))
    }
    fn non_air_sections(&self, proto: &Proto) -> Option<crate::section_mask::SectionMask> {
        self.pipeline
            .as_ref()
            .and_then(|base| base.non_air_sections(&Self::wrapped(proto).inner))
    }
    fn cache_census(&self) -> Vec<(&'static str, usize, Option<usize>)> {
        self.pipeline.as_ref().map_or_else(Vec::new, |base| base.cache_census())
    }
    fn proto_heap_bytes(&self, proto: &Proto) -> ([usize; 6], [usize; 16]) {
        self.pipeline.as_ref().map_or(([0; 6], [0; 16]), |base| {
            base.proto_heap_bytes(&Self::wrapped(proto).inner)
        })
    }
    fn saved_proto(&self, proto: &Proto) -> Option<crate::save::SavedProto> {
        let proto = Self::wrapped(proto);
        (!proto.failed)
            .then(|| self.pipeline.as_ref()?.saved_proto(&proto.inner))
            .flatten()
    }
    fn restore_proto(&self, proto: &mut Proto, saved: crate::save::SavedProto) {
        let Some(base) = &self.pipeline else {
            return;
        };
        let proto = proto.downcast_mut::<ModProto>().expect("mod terrain proto");
        base.mod_restore(&mut proto.inner, saved);
        proto.failed = false;
    }
    fn finish(&self, proto: Proto) -> Chunk {
        let proto = *proto.downcast::<ModProto>().expect("mod terrain proto");
        let mut chunk = if let Some(base) = &self.pipeline {
            base.finish(proto.inner)
        } else {
            *proto.inner.downcast::<Chunk>().expect("direct mod column")
        };
        if proto.failed {
            let mut air = air_with_biomes(self.shape, &chunk);
            air.not_generated = true;
            return air;
        }
        chunk.not_generated = false;
        chunk
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

    const Y: i32 = -64;
    struct TestFactory {
        seen: Arc<std::sync::Mutex<Vec<u32>>>,
        fail: bool,
    }
    struct TestSource {
        seen: Arc<std::sync::Mutex<Vec<u32>>>,
        fail: bool,
    }
    impl ModGeneratorFactory for TestFactory {
        fn create(&self, _: &ModGeneratorContext) -> Result<Arc<dyn ModColumnSource>, String> {
            Ok(Arc::new(TestSource {
                seen: self.seen.clone(),
                fail: self.fail,
            }))
        }
    }
    impl ModColumnSource for TestSource {
        fn generate(&self, chunk: &mut ModChunk) {
            self.seen.lock().unwrap().push(chunk.block(0, Y, 0));
            chunk.set_block(0, Y, 0, 20);
            assert!(chunk.set_biome("minecraft:desert"));
            if self.fail {
                chunk.mark_failed();
            }
        }
    }

    struct Synthetic;
    struct SyntheticProto {
        chunk: Chunk,
        decorated: bool,
    }
    impl ChunkGenerator for Synthetic {
        type Worker = ();
        fn worker(&self) {}
        fn proto(&self, _: &mut (), x: i32, z: i32) -> Proto {
            let mut chunk = Chunk::with_layout(x, z, LevelShape::OVERWORLD.column());
            for z in 0..16 {
                for x in 0..16 {
                    chunk.set(x, Y, z, 11);
                }
            }
            Box::new(SyntheticProto {
                chunk,
                decorated: false,
            })
        }
        fn decorate(&self, _: &mut (), region: &mut [Proto; 9]) {
            let center = region[4]
                .downcast_mut::<SyntheticProto>()
                .expect("synthetic proto");
            center.chunk.set(0, Y, 0, 30);
            center.decorated = true;
        }
        fn is_decorated(&self, proto: &Proto) -> bool {
            proto
                .downcast_ref::<SyntheticProto>()
                .is_some_and(|proto| proto.decorated)
        }
        fn light_view(&self, proto: &Proto) -> Option<Chunk> {
            Some(
                proto
                    .downcast_ref::<SyntheticProto>()?
                    .chunk
                    .clone(),
            )
        }
        fn finish(&self, proto: Proto) -> Chunk {
            proto
                .downcast::<SyntheticProto>()
                .expect("synthetic proto")
                .chunk
        }
    }
    impl ModBase for Synthetic {
        fn mod_column(&self, proto: &Proto) -> Option<Chunk> {
            Some(
                proto
                    .downcast_ref::<SyntheticProto>()?
                    .chunk
                    .clone(),
            )
        }
        fn mod_replace(&self, proto: &mut Proto, column: Chunk) {
            proto
                .downcast_mut::<SyntheticProto>()
                .expect("synthetic proto")
                .chunk = column;
        }
    }

    fn source(fail: bool) -> (Arc<dyn ModColumnSource>, Arc<std::sync::Mutex<Vec<u32>>>) {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let factory = TestFactory {
            seen: seen.clone(),
            fail,
        };
        let context = ModGeneratorContext {
            level: "test:level".into(),
            seed: 5,
            shape: LevelShape::OVERWORLD,
            generator: "test:generator".into(),
            settings: serde_json::json!({}),
            assets: None,
            biome_of: Arc::new(|name| match name {
                "minecraft:plains" => Some(0),
                "minecraft:desert" => Some(1),
                _ => None,
            }),
        };
        (factory.create(&context).unwrap(), seen)
    }
    fn terrain(
        source: Arc<dyn ModColumnSource>,
        based: bool,
        decorate: bool,
    ) -> ModTerrain<Synthetic> {
        ModTerrain {
            level: "test:level".into(),
            source,
            shape: LevelShape::OVERWORLD,
            plains: 0,
            biome_of: Arc::new(|name| match name {
                "minecraft:plains" => Some(0),
                "minecraft:desert" => Some(1),
                _ => None,
            }),
            based,
            decorate,
            structures: false,
            pipeline: Some(Arc::new(Synthetic)),
        }
    }

    #[test]
    fn a_source_sees_and_edits_the_base_column() {
        let (source, seen) = source(false);
        let terrain = terrain(source, true, false);
        let mut worker = terrain.worker();
        let chunk = terrain.finish(terrain.proto(&mut worker, 2, -1));
        assert_eq!(*seen.lock().unwrap(), [11], "base before the source");
        assert_eq!(chunk.get(0, Y, 0), 20, "the source's edit");
        assert_eq!(chunk.get(1, Y, 0), 11, "unmodified base blocks remain");
        assert_eq!(chunk.biome(0, Y, 0), 1);
        assert!(!chunk.not_generated);
    }

    #[test]
    fn native_biome_cells_form_a_four_by_four_grid() {
        struct Grid;
        impl ModColumnSource for Grid {
            fn generate(&self, chunk: &mut ModChunk) {
                assert!(chunk.set_biome("minecraft:desert"));
                for qz in 0..4 {
                    for qx in 0..4 {
                        let name = if (qx + qz) % 2 == 0 { "minecraft:plains" } else { "minecraft:desert" };
                        assert!(chunk.set_biome_at(qx, 0, qz, name));
                    }
                }
                assert!(!chunk.set_biome_at(4, 0, 0, "minecraft:plains"));
                assert!(!chunk.set_biome_at(0, chunk.shape.height / 4, 0, "minecraft:plains"));
                assert!(!chunk.set_biome_at(0, 0, 0, "minecraft:forest"));
            }
        }
        let terrain = terrain(Arc::new(Grid), false, false);
        let mut worker = terrain.worker();
        let chunk = terrain.finish(terrain.proto(&mut worker, 0, 0));
        for qz in 0..4i32 {
            for qx in 0..4i32 {
                let block = (qx * 4 + 1, qz * 4 + 1);
                assert_eq!(chunk.biome(block.0, Y, block.1), ((qx + qz) % 2) as u16, "quart {qx} {qz}");
            }
        }
        assert_eq!(chunk.biome(0, Y + 4, 0), 1, "the next quart keeps the whole-chunk biome");
    }

    #[test]
    fn decoration_is_a_switchable_postprocessing_stage() {
        for (decorate, expected) in [(false, 20), (true, 30)] {
            let (source, seen) = source(false);
            let terrain = terrain(source, false, decorate);
            assert_eq!(terrain.decorates(), decorate);
            let mut worker = terrain.worker();
            let proto = if decorate {
                let mut region: [Proto; 9] = std::array::from_fn(|index| {
                    terrain.proto(&mut worker, index as i32 % 3 - 1, index as i32 / 3 - 1)
                });
                terrain.decorate(&mut worker, &mut region);
                std::mem::replace(&mut region[4], Box::new(()))
            } else {
                terrain.proto(&mut worker, 0, 0)
            };
            let chunk = terrain.finish(proto);
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), if decorate { 9 } else { 1 });
            assert!(seen.iter().all(|&state| state == 0), "an unbased source starts with air");
            drop(seen);
            assert_eq!(chunk.get(0, Y, 0), expected, "decorate={decorate}");
            assert_eq!(chunk.get(1, Y, 0), 0, "the synthetic base was cleared");
        }
    }

    #[test]
    fn a_failed_source_falls_back_to_air_without_panicking() {
        let (source, _) = source(true);
        let terrain = terrain(source, true, true);
        let mut worker = terrain.worker();
        let chunk = terrain.finish(terrain.proto(&mut worker, 0, 0));
        assert!(chunk.not_generated);
        for x in 0..16 {
            for z in 0..16 {
                assert_eq!(chunk.get(x, Y, z), 0);
            }
        }
    }
}

/// Forgets everything staged (tests).
pub fn clear_staged() {
    *staged_lock() = None;
}

// F3-8b: binding glue for `ModChunk`, generated by yoake-bindgen; compiled only with the feature.
#[cfg(feature = "yoake2-slice")]
include!("yoake_glue/mod_generator.rs");
