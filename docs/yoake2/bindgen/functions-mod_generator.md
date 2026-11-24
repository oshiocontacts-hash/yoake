# `mod_generator`: every function (horizon-client/src/mod_generator.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `mod_generator::<ModGenerators as Clone>::clone` | 158 | &self | derived (not counted) | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::<ModGenerators as Default>::default` | 158 | - | derived (not counted) | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::<ModTerrain as ChunkGenerator>::all_idle` | 310 | &self | excluded | impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::cache_census` | 444 | &self | excluded | a reference nested inside another type needs a handle [return value]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::decorate` | 393 | &self | excluded | `dyn Trait` values cannot cross the data ABI; associated-type projection requires a listed instantiation; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::decorates` | 357 | &self | excluded | impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::decorates_beside_final` | 360 | &self | excluded | impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::final_block_entities` | 384 | &self | excluded | `dyn Trait` values cannot cross the data ABI; field `0` of `Wire` is private to another module [return value]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::final_writes` | 379 | &self | excluded | `dyn Trait` values cannot cross the data ABI; field `min_y` of `Chunk` is private to another module [&argument]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::finish` | 466 | &self | excluded | `dyn Trait` values cannot cross the data ABI; field `min_y` of `Chunk` is private to another module [return value]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::idle` | 305 | &self | excluded | associated-type projection requires a listed instantiation; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::is_decorated` | 429 | &self | excluded | `dyn Trait` values cannot cross the data ABI; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::light_view` | 434 | &self | excluded | `dyn Trait` values cannot cross the data ABI; field `min_y` of `Chunk` is private to another module [return value]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::non_air_sections` | 439 | &self | excluded | `dyn Trait` values cannot cross the data ABI; field `0` of `SectionMask` is private to another module [return value]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::prepare_loaded` | 315 | &self | excluded | associated-type projection requires a listed instantiation; field `min_y` of `Chunk` is private to another module [&argument]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::proto` | 320 | &self | excluded | `dyn Trait` values cannot cross the data ABI; associated-type projection requires a listed instantiation; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::proto_beside_final` | 363 | &self | excluded | `dyn Trait` values cannot cross the data ABI; associated-type projection requires a listed instantiation; field `min_y` of `Chunk` is private to another module [&argument]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::proto_heap_bytes` | 447 | &self | excluded | `dyn Trait` values cannot cross the data ABI; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::restore_proto` | 458 | &self | excluded | `dyn Trait` values cannot cross the data ABI; field `min_y` of `Chunk` is private to another module; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::saved_proto` | 452 | &self | excluded | `dyn Trait` values cannot cross the data ABI; field `min_y` of `Chunk` is private to another module [return value]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<ModTerrain as ChunkGenerator>::worker` | 302 | &self | excluded | associated-type projection requires a listed instantiation; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::<Staged as Default>::default` | 487 | - | derived (not counted) | external type `serde_json::value::Value` (crate not loaded) [return value] |
| `mod_generator::ModChunk::block` | 55 | &self | bound (a) | - |
| `mod_generator::ModChunk::fill_layer` | 47 | &mut self | bound (a) | - |
| `mod_generator::ModChunk::mark_failed` | 32 | &mut self | bound (a) | - |
| `mod_generator::ModChunk::set_biome` | 59 | &mut self | bound (a) | - |
| `mod_generator::ModChunk::set_biome_at` | 72 | &mut self | bound (a) | - |
| `mod_generator::ModChunk::set_block` | 38 | &mut self | bound (a) | - |
| `mod_generator::ModGeneratorContext::biome_lookup` | 147 | &self | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModGeneratorContext::block` | 128 | &self | bound (b, mechanical) | - |
| `mod_generator::ModGeneratorContext::block_lookup` | 136 | &self | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModGenerators::contains` | 181 | &self | bound (a) | - |
| `mod_generator::ModGenerators::factory` | 178 | &self | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModGenerators::ids` | 184 | &self | excluded | `impl Trait` requires a listed instantiation |
| `mod_generator::ModGenerators::register` | 164 | &mut self | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModGenerators::terrain` | 188 | &self | excluded | a reference nested inside another type needs a handle; external type `serde_json::value::Value` (crate not loaded); field `generator` of `VanillaTerrain` is private to another module; field `info_noise` of `Biomes` is private to another module; generic type `ModTerrain` requires a listed instantiation |
| `mod_generator::ModTerrain::empty` | 285 | &self | excluded | field `min_y` of `Chunk` is private to another module [return value]; impl-level type parameter `G` requires a listed instantiation; receiver type is generic: bind a concrete instantiation first |
| `mod_generator::ModTerrain::wrapped` | 295 | - | excluded | `dyn Trait` values cannot cross the data ABI; impl-level type parameter `G` requires a listed instantiation |
| `mod_generator::air_with_biomes` | 276 | - | handle-required | field `min_y` of `Chunk` is private to another module [&argument]; field `min_y` of `Chunk` is private to another module [return value] |
| `mod_generator::clear_staged` | 803 | - | bound (a) | - |
| `mod_generator::generate_chunk` | 240 | - | handle-required | field `min_y` of `Chunk` is private to another module [&mut argument] |
| `mod_generator::stage_generator` | 543 | - | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::stage_generator_replacing` | 536 | - | excluded | `dyn Trait` values cannot cross the data ABI |
| `mod_generator::stage_level` | 500 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::stage_level_replacing` | 512 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::stage_type` | 522 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::staged` | 550 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [return value] |
| `mod_generator::staged_lock` | 495 | - | excluded | lifetime-carrying type `std::sync::MutexGuard` |
| `mod_generator::staged_types` | 532 | - | handle-required | external type `serde_json::value::Value` (crate not loaded) [return value] |

## Fields

| field | status |
|---|---|
| `mod_generator::ModChunk.chunk` | no accessor: field `min_y` of `Chunk` is private to another module |
| `mod_generator::ModChunk.chunk_x` | get + set |
| `mod_generator::ModChunk.chunk_z` | get + set |
| `mod_generator::ModChunk.shape` | get + set |
| `mod_generator::ModChunk.biome_of` | no accessor: `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModChunk.failed` | get + set |
| `mod_generator::ModGeneratorContext.level` | get + set |
| `mod_generator::ModGeneratorContext.seed` | get + set |
| `mod_generator::ModGeneratorContext.shape` | get + set |
| `mod_generator::ModGeneratorContext.generator` | get + set |
| `mod_generator::ModGeneratorContext.settings` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::ModGeneratorContext.assets` | no accessor: a reference nested inside another type needs a handle |
| `mod_generator::ModGeneratorContext.biome_of` | no accessor: `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModGenerators.factories` | no accessor: `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModProto.inner` | no accessor: `dyn Trait` values cannot cross the data ABI |
| `mod_generator::ModProto.failed` | get + set |
| `mod_generator::ModTerrain.level` | no accessor: `ModTerrain` is generic |
| `mod_generator::ModTerrain.source` | no accessor: `ModTerrain` is generic |
| `mod_generator::ModTerrain.shape` | no accessor: `ModTerrain` is generic |
| `mod_generator::ModTerrain.plains` | no accessor: `ModTerrain` is generic |
| `mod_generator::ModTerrain.biome_of` | no accessor: `ModTerrain` is generic |
| `mod_generator::ModTerrain.based` | no accessor: `ModTerrain` is generic |
| `mod_generator::ModTerrain.decorate` | no accessor: `ModTerrain` is generic |
| `mod_generator::ModTerrain.structures` | no accessor: `ModTerrain` is generic |
| `mod_generator::ModTerrain.pipeline` | no accessor: `ModTerrain` is generic |
| `mod_generator::Staged.levels` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::Staged.types` | no accessor: external type `serde_json::value::Value` (crate not loaded) |
| `mod_generator::Staged.generators` | no accessor: `dyn Trait` values cannot cross the data ABI |
