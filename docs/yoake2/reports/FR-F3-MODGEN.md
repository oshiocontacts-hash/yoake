# FR-F3-MODGEN: completing the `yoake:mod` generator

Branch `yoake-free/f3-modgen` (on top of `yoake-free/f3-9-11`). Code by Kimi Code; review, fixes, tests and this
report by Claude on LENOVO. Design: `docs/yoake2/FR-F3-DIMENSIONS.md` sections 3.4 and 3.5
(`GeneratorSpec::Mod { handler, biome_source, base }`).

## Part A: a vanilla base, decoration and structures

### Stem fields (all optional; a stem with none of them generates exactly as before)

```json
{"type": "yoake:mod", "id": "mymod:terrain", "settings": {},
 "biome_source": {"type": "minecraft:fixed", "biome": "minecraft:plains"},
 "base": "minecraft:overworld",
 "decorate": true,
 "structures": true}
```

* `biome_source`: any source `minecraft:noise` accepts; default fixed plains. It gives the initial biomes and picks
  the biome features and structures. The callback can still override with `ModChunk::set_biome`.
* `base`: a noise settings id, or a full `minecraft:noise` generator object. The engine generates that terrain (level
  seed, the mod's biome source: noise, surface, carvers) before the callback, which reads it through `ModChunk::block`
  and edits it. Without `base` the callback starts from an all-air column, as before.
* `decorate`: the biome's placed features run after the callback. `structures`: structure starts, terrain adaptation
  and pieces (also needs the world's `generate_structures`); usable without `decorate` (then only structures run).
  Both default to `false`.

### How (files)

* `horizon-world/src/dimension.rs`: `GeneratorSpec::Mod` gains `biome_source`, `base: Option<Box<GeneratorSpec>>`
  (validated as noise), `decorate`, `structures`; parse/validate/encode. An old stem round-trips as before (it was and
  is written with `"settings": {}`).
* `horizon-client/src/mod_generator.rs`: `ModTerrain` is a `ChunkGenerator` that wraps an optional vanilla pipeline
  (`ModBase`, implemented by `VanillaTerrain` in `vanillagen.rs`): the base proto is generated, its column is handed to
  the mod's source, the edited column replaces the proto's, and decoration/structures run through the same
  `decorate` stage noise levels use (`GeneratorWorker::set_structures_only` in `worldgen/generator.rs` when only
  structures are wanted). Heightmaps and light follow the normal stages after the edit. A failed source still gives an
  all-air column with the biomes and logs once.
* `game/world_loading.rs`, `level_registry.rs`, `wasm_mods/level_services.rs`: build the pipeline when the stem asks
  for it; WASM-created levels accept the same fields.
* SDK `full_generator.rs` and the `level-generator` example; docs `guide-en/13-dimensions.md`,
  `guide-ja/13-dimensions.md`; `tools/levelgen_wasm_e2e.py` extended (a based + decorated mod level).

### Tests (run on LENOVO)

horizon-world --lib 392 passed (incl. stem parse/round-trip of the new fields and their errors), horizon-client-lib
--lib 1942 (incl. `a_source_sees_and_edits_the_base_column`, `decoration_is_a_switchable_postprocessing_stage`,
`a_failed_source_falls_back_to_air_without_panicking`), horizon-client --lib 1256 and its integration tests,
yoake-wasm-sdk (full) tests, the `level-generator` example builds for wasm32-wasip2. Claude's fix: the round-trip test
expected an old stem to be written without `"settings": {}`; the code writes it as it always did, the test now says so.

Not run here (need the game's resources): `tools/levelgen_wasm_e2e.py`, `tools/levels_e2e.py`, `parity.py worldgen`
(vanilla noise levels must still match; nothing in the noise path changes unless a mod level asks for a pipeline).

### Left / notes

* Decoration or structures **without** a base still build the shared pipeline from the Overworld noise settings and
  clear its terrain before the callback: correct, but it pays for a noise column per chunk.
* No mod-aware far-terrain (LOD) sampler: distant not-yet-generated chunks can show the base without the callback's
  edits. Callbacks cannot query climate or density functions yet.
* `hook_bridge_mod_generator.rs` stays overlay-only (as agreed).

## Part B (next)

Per-quart biome fill from a WASM `Fill` (today one biome per WASM chunk) and the F3-8d all-in-one cooking-duel WASM
e2e in `tools/levelgen_wasm_e2e.py`.
