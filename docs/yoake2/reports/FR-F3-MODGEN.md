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

## Part B: per-quart biomes and the cooking-duel lifecycle

### Fill format and native API

A `Fill` that uses only `biome`, `uniform` and `palette` still emits the exact v1 bytes. `biome_at(qx, qy, qz, id)`
and `biome_column(qx, qz, id)` select v2. Quart x/z are 0..4 inside the chunk; quart y and section indices count from
the level's bottom. V2 is little-endian, with `str` as a u16 length plus UTF-8:

```text
u8 version = 2
u8 flags: bit 0 = whole-chunk biome follows, bit 1 = all-height columns follow the palette
[str whole-chunk biome]
u16 palette_count, then palette_count * str biome
[16 * u16 column palette indices, qz * 4 + qx]
u16 quart_section_count
per section: u16 section_index, then 64 * u16 palette indices, (qy * 4 + qz) * 4 + qx
u16 block_section_count, then the unchanged v1 block sections
```

The whole-chunk biome is applied first, then columns at every height, then explicit quart sections. The host resolves
all v2 biome names and checks flags, palette indices, section ranges/duplicates/counts and trailing bytes before any
writes; failures use the existing failed-chunk/log-once path. V1 decoding and its apply-time unknown-biome warning are
unchanged. `ModChunk::set_biome_at` provides the same checked per-quart operation to native generators, and the
factory context now supplies the biome lookup needed for host-side validation.

### Cooking-duel sample and e2e

The existing `level-generator` example was extended rather than adding a second package: it already had the manifest,
generator registration, build output and LAN harness this scenario needs. Its new `levelgen:cooking_duel` generator
writes a flat bedrock/stone/grass arena and 16-block cells in a 4x4 plains/desert/forest/taiga grid using v2 columns.
`/cookingduel create`, `/cookingduel regenerate [seed]` and `/cookingduel delete` call the existing WASM level
services; there is no WIT or catalog change.

`tools/levelgen_wasm_e2e.py` now creates the ephemeral level before the guest logs in (level keys are login data),
enters both host and guest, reads every grid cell through `horizon-ctl block` (whose response now includes `biome`),
and edits the arena. Both players leave before regeneration, as the level manager requires; the guest returns to check
the edit is gone and the grid remains. Both are safely in the Overworld for deletion, and the harness checks that the
`yoake_temp/levelgen/cooking_duel` folder and registry entry are gone.

### Files and tests

* `crates/yoake-wasm-sdk/src/full_generator.rs`: v2 encoder, SDK APIs and exact-byte/layout tests.
* `crates/horizon-client/src/wasm_mods/level_services.rs`: versioned decoder, validation, application and fuzz/unit
  tests, including v2 data applied to real `ModChunk` quart storage.
* `crates/horizon-client/src/mod_generator.rs`: native per-quart API, biome lookup and a 4x4 grid/bounds test.
* `crates/horizon-client/src/game/agent.rs`: biome names in block observations.
* `crates/yoake-wasm-sdk/examples/level-generator/`: the duel generator and lifecycle command.
* `tools/levelgen_wasm_e2e.py`: the default run's two-player create/grid/regenerate/delete check.

Run by Claude on LENOVO: horizon-client-lib --lib 1967 passed, horizon-client --lib (incl. the level_services v2 tests) and its integration tests, horizon-ctl 18, yoake-wasm-sdk (full) 13, the `level-generator` example builds for wasm32-wasip2. Claude's fixes after Kimi's two rounds: two test expectations (the SDK's checkerboard column test compared against `i % 2` instead of `(qx + qz) % 2`; the host's apply test expected `(qx + qz) % 2` / `(qx + qy + qz) % 2` for fixtures laid out as `i % 2`, i.e. `qx % 2`); the encoder and decoder agree on `qz * 4 + qx` (and `qy * 16 + qz * 4 + qx`). Not run here: `tools/levelgen_wasm_e2e.py` (the cooking-duel run needs the game).
Round 1 verified 1967 horizon-client-lib tests, the wasm32-wasip2 example build and Python syntax. Its review fixes
made the cross-crate biome-ID/test helpers `#[doc(hidden)] pub` (the WASM host and native library are different crates)
and added the omitted `palette_count` field to the SDK and host test offsets. Full horizon-client tests and the
resource-backed e2e remain for Round 2; there is no known implementation gap.
