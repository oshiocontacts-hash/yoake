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
}
impl ModChunk<'_> {
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
    /// The stem's `generator.settings` object.
    pub settings: Value,
    assets: Arc<Assets>,
}
impl ModGeneratorContext {
    /// The default state id of a block (`minecraft:stone`).
    pub fn block(&self, name: &str) -> Option<u32> {
        self.assets.registry.default_state(name).map(|s| s.0)
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
            source,
            shape,
            plains,
            biomes,
        })
    }
}

/// A [`ModColumnSource`] as the engine's chunk generator.
pub struct ModTerrain {
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
        self.source.generate(&mut ModChunk {
            chunk: &mut chunk,
            chunk_x: cx,
            chunk_z: cz,
            shape: self.shape,
            biome_of: &biome_of,
        });
        for s in &mut chunk.sections {
            Arc::make_mut(s).compact();
        }
        Box::new(chunk)
    }
    fn finish(&self, proto: Proto) -> Chunk {
        *proto.downcast::<Chunk>().expect("mod terrain proto")
    }
}
