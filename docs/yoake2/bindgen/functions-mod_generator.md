# `mod_generator`: every function (horizon-client/src/mod_generator.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `mod_generator::<ModGenerators as Clone>::clone` | 107 | &self | derived (not counted) | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::<ModGenerators as Default>::default` | 107 | - | derived (not counted) | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::<ModTerrain as ChunkGenerator>::finish` | 225 | &self | handle-required | external type `horizon_client_render::chunk::Chunk` (crate not loaded) [return value]; external type `horizon_client_render::streaming::Proto` (crate not loaded) |
| `mod_generator::<ModTerrain as ChunkGenerator>::proto` | 195 | &self | handle-required | external type `horizon_client_render::streaming::Proto` (crate not loaded) [return value] |
| `mod_generator::<ModTerrain as ChunkGenerator>::worker` | 194 | &self | bound (a) | - |
| `mod_generator::<Staged as Default>::default` | 234 | - | derived (not counted) | external type `serde_json::value::Value` (crate not loaded) [return value] |
| `mod_generator::ModChunk::block` | 55 | &self | bound (a) | - |
| `mod_generator::ModChunk::fill_layer` | 47 | &mut self | bound (a) | - |
| `mod_generator::ModChunk::mark_failed` | 32 | &mut self | bound (a) | - |
| `mod_generator::ModChunk::set_biome` | 59 | &mut self | bound (a) | - |
| `mod_generator::ModChunk::set_block` | 38 | &mut self | bound (a) | - |
| `mod_generator::ModGeneratorContext::block` | 91 | &self | bound (b, mechanical) | - |
| `mod_generator::ModGeneratorContext::block_lookup` | 95 | &self | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModGenerators::contains` | 130 | &self | bound (a) | - |
| `mod_generator::ModGenerators::factory` | 127 | &self | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModGenerators::ids` | 133 | &self | excluded | `impl Trait` requires a listed instantiation |
| `mod_generator::ModGenerators::register` | 113 | &mut self | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModGenerators::terrain` | 137 | &self | excluded | `dyn Trait` values cannot cross the data ABI; external type `horizon_client_render::assets::Assets` (crate not loaded); external type `horizon_client_render::biome::Biomes` (crate not loaded); external type `horizon_client_render::chunk::LevelShape` (crate not loaded); external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::clear_staged` | 333 | - | bound (a) | - |
| `mod_generator::generate_chunk` | 179 | - | handle-required | external type `horizon_client_render::chunk::Chunk` (crate not loaded) [&mut argument] |
| `mod_generator::stage_generator` | 290 | - | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::stage_generator_replacing` | 283 | - | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::stage_level` | 247 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::stage_level_replacing` | 259 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::stage_type` | 269 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::staged` | 297 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [return value] |
| `mod_generator::staged_lock` | 242 | - | excluded | lifetime-carrying type `std::sync::MutexGuard` |
| `mod_generator::staged_types` | 279 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [return value] |

## Fields

| field | status |
|---|---|
| `mod_generator::ModChunk.chunk` | no accessor: external type `horizon_client_render::chunk::Chunk` (crate not loaded) |
| `mod_generator::ModChunk.chunk_x` | get + set |
| `mod_generator::ModChunk.chunk_z` | get + set |
| `mod_generator::ModChunk.shape` | no accessor: external type `horizon_client_render::chunk::LevelShape` (crate not loaded) |
| `mod_generator::ModChunk.biome_of` | no accessor: `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModChunk.failed` | get + set |
| `mod_generator::ModGeneratorContext.level` | get + set |
| `mod_generator::ModGeneratorContext.seed` | get + set |
| `mod_generator::ModGeneratorContext.shape` | no accessor: external type `horizon_client_render::chunk::LevelShape` (crate not loaded) |
| `mod_generator::ModGeneratorContext.generator` | get + set |
| `mod_generator::ModGeneratorContext.settings` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::ModGeneratorContext.assets` | no accessor: external type `horizon_client_render::assets::Assets` (crate not loaded) |
| `mod_generator::ModGenerators.factories` | no accessor: `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModTerrain.level` | get + set |
| `mod_generator::ModTerrain.source` | no accessor: `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModTerrain.shape` | no accessor: external type `horizon_client_render::chunk::LevelShape` (crate not loaded) |
| `mod_generator::ModTerrain.plains` | get + set |
| `mod_generator::ModTerrain.biomes` | no accessor: external type `horizon_client_render::biome::Biomes` (crate not loaded) |
| `mod_generator::Staged.levels` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::Staged.types` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::Staged.generators` | no accessor: `dyn Trait` values cannot cross the data ABI |
