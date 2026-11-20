# FR-F3-5: generators for levels past the vanilla three

Branch `yoake-free/f3-5` on `yoake-free/f3-4b` (after two small fixes committed on f3-4b, below). No Minecraft files,
source or prepared resources committed. Nothing pushed.

## Fixes on f3-4b first (commit `254c5907b`)
1. **Sea level from the generator.** `LevelEntry.sea_level` is the `sea_level` of the noise settings the stem's generator
   names (the prepared `noise_settings` files and the packs' over them, or an inline object's), superflat's -63, the debug
   world's 63; a generator with none (or a settings id nobody has) falls back to the old ceiling rule (32 with a ceiling, else
   63). The server's `LevelFacts.sea_level` and `rules.sea_level` take it (mob world, weather, feature placement), the hub
   sends it as `SpawnInfo.sea_level` for a level past the vanilla three (the vanilla levels still send 63), and a client takes
   it from there. Test: `level_registry::tests::the_sea_level_comes_from_the_generators_noise_settings`.
2. **Bed `can_set_spawn: when_dark`.** `BedRules.sets_spawn` / `straw_sets_spawn` are `SetSpawn::{Always, WhenDark, Never}`.
   `WhenDark` is checked when the spawn is set and when it is used at respawn, against the level's own sky darkening
   (`sky_darken() >= 4`, which for a level past the vanilla three samples its own environment and day cycle). Vanilla constants
   unchanged. Test: `bed_rules_read_the_attributes` (parsing), the vanilla answers in the ignored game tests.

## F3-5
### Registry: flat, debug and mod stems are levels
`LevelRegistry` no longer refuses non-noise generators. Biome sources were already decoded for every vanilla variant
(`minecraft:fixed`, `checkerboard`, `the_end`, `multi_noise` inline or by preset) plus `yoake:grid`, and the noise path
(`VanillaTerrain::for_world_with_settings`, which reads the stem and its `settings` id or inline object) was already generic over
them; only the registry's own refusal and the game's Overworld-only flat/debug lookup stood in the way.
`horizon-world`: `GeneratorSpec::Mod { id, settings }` for `{"type": "yoake:mod", "id": "<mod>:<generator>", "settings": {...}}`
(id must be namespaced; settings an object).

### Game: a generator per level (`world_loading::make_streaming`)
* Flat and debug: the existing `FlatTerrain` / `DebugTerrain` (and the flat structure locator) read their settings from a
  `dimensions.minecraft:overworld.generator` tag. The Overworld still reads it from the world's `world_gen_settings`; any other
  level makes the same tag from its own stem (`level_generator_settings`), so the Overworld paths are unchanged. A flat or debug
  stem on the Nether or the End now works too, as vanilla allows.
* Noise: the stem's `settings` and `biome_source` as before, any variant, the level's own folder and structure reference file.
* Mod generator: `mod_generator.rs` (new, in the client lib).
  * `ModColumnSource::generate(&self, &mut ModChunk)` fills a chunk (`set_block`, `fill_layer`, `block`, `set_biome`);
    `ModGeneratorFactory::create(&ModGeneratorContext)` makes the source when the level opens (context: level id, world seed,
    shape, the stem's settings, `block(name)` to state ids). `ModTerrain` is the engine's `ChunkGenerator` for it (final
    blocks and biomes, no decoration or structure step; saves go through the usual `Persistent`).
  * `ModGenerators` is the registry; `Game::register_mod_generator(id, factory)` (native side) registers before the world
    ticks, like `register_mod_level`. A level whose stem names a generator nobody registered stays registered but does not open,
    with a message naming the generator and the registered ones. F3-8 plugs WASM callbacks into `ModColumnSource`.
* Seeds: as vanilla, every level of a world uses the world seed (the generators and the per-level structure reference files
  already key on the level id); the grid's `seed` field and a mod's context carry it for generators that want one.
* The player's level on world load builds its terrain before mods register generators, so a player saved in a mod-generator level
  opens in the Overworld until F3-6/F3-8 order that (the registry rejects the unopenable level at open, not at registration).

## Parity (vanilla generation unchanged)
`level_registry_tests::vanilla_chunks_hash_as_they_did_before_the_generator_work`: seed 5, the final chunk (3, -2) of each vanilla
level streamed through the game and hashed (FNV-1a over every block id and a biome per 4x4x4 cell). The hashes were recorded on the
tree before any F3-5 change (commit `3e60ecead`) and the test passes unchanged after it:
Overworld `10844162340323160155`, Nether `3948111228871227996`, End `2685422101413284537`.

## Tests
* `-p horizon-client-lib --lib level_registry`: the new `flat_debug_mod_and_checkerboard_stems_are_levels` (flat, debug, mod,
  checkerboard and fixed accepted; a mod stem with no or a bad id reported) and the sea level test; the old "flat refused" case is gone.
* Ignored, real resources (`level_registry_tests -- --ignored`):
  * `flat_checkerboard_and_debug_pack_levels_generate_and_save`: a pack's superflat level generates bedrock, stone, stone, dirt,
    grass from its floor with air above and the layer biome, an edit saves into `dimensions/mypack/plain/region`; a noise level with
    a `minecraft:checkerboard` biome source (scale 0: one biome per chunk, neighbouring chunks the two biomes in turn); a debug
    level's barrier floor.
  * `a_mod_generator_makes_the_chunks_of_its_level`: a registered factory (`testmod:floor`, `depth` setting) makes a level's
    chunks (3 stone layers from the floor, desert biome); before registration the level does not open and says why; a duplicate or
    non-namespaced id is refused.
  * the vanilla hash test above, and all earlier level tests.

Results: `-p horizon-client --lib` 1213 passed, 0 failed, 46 ignored; `-p horizon-client-lib --lib` 1657 passed, 0 failed, 48 ignored; `-p horizon-world --lib` 385 passed, 0 failed, 2 ignored; ignored real-resource `level_registry_tests` 6 passed (vanilla hashes unchanged). No offscreen game smoke run.

## Left / needs checking
* `multi_noise` and `the_end` biome sources and `minecraft:fixed` run through the same noise path but have no generation test of
  their own here (decode is tested: `fixed`, `checkerboard`); a multi_noise preset level needs the preset catalog (the registry
  already has it).
* Mod generators have no decoration, structure, carver or spawn step, and no far-terrain (LOD) sampler: far terrain of such a level is
  empty beyond loaded chunks.
* Structure locating (`/locate`) and the flat structure locator exist for flat levels (as for the Overworld) but were not exercised
  for a level past the vanilla three.
* A mod's generator context has no way yet to ask for biomes by climate or for noise; F3-8 decides the callback shape.
* `Game::register_mod_generator` is native only and not reachable by WASM mods (F3-8); `yoake:mod` stems from a data pack are
  accepted (a mod may register the generator after the pack loaded).
