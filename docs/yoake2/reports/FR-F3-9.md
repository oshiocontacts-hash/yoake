# FR-F3-9: new biomes from data packs and content mods

## Scope and design match

Implemented the F3-9 path without changing the network protocol or any WIT/catalog surface.

- **Registry growth and worldgen.** `WorldgenOverrides` no longer removes unknown biome ids, biome-tag entries, or
  multi-noise climate points. The existing client `Biomes::with_packs` table keeps built-in ids in place and appends new
  ids in `Identifier` order (path, then namespace). The generator's existing catalog now receives those entries, so
  fixed, checkerboard, multi-noise, features, carvers, climate, and spawn lookups can resolve them.
- **Content mods.** `data/<ns>/worldgen/biome/*.json` now joins `worldgen/biomes.json`. Mod biome ids are known to tag
  filtering and biome modifiers, including modifiers addressed through a mod's own biome tag. The content root is layered
  beneath world data packs; a pack file for the same id wins. Mod biomes also join command-registry suggestions.
- **Client data.** Each `BiomeInfo` now carries parsed ambient particles, ambient loop/mood/additions sounds, background
  music, and music volume in addition to its colours and spawn tables. Missing audio attributes inherit the dimension as
  before. Existing colour defaults and grass modifiers remain in use. Animation, ambient-sound, and music consumers read
  the active world's table, including a table rebuilt from LAN data.
- **Spawns.** Natural and chunk-generation spawning already read `BiomeInfo.spawns`; the expanded table now supplies new
  biome spawn entries. Biome modifiers can rewrite new mod biomes before that table is built.
- **LAN.** The existing configuration-phase `minecraft:worldgen/biome` registry packet sends new entries with their JSON
  as NBT. Existing chunk encoding translates internal ids to that registry's ids, and `client_biomes` reconstructs the
  client's table from entries it lacks. No additive packet was necessary.
- **Saves.** Existing chunk palettes already serialize biome names rather than numeric ids. All world chunk readers and
  writers use the expanded table; flat, debug, mod, and old-world paths that previously received only the base table were
  corrected as well.
- **Vanilla unchanged.** An empty override set takes the same generator path as before. With no added biome, the client
  keeps the original `Arc<Biomes>` and no dynamic per-chunk registry work was introduced. Existing network packets and
  name-based save encoding are unchanged.

## Files changed

- `crates/horizon-world/src/worldgen/overrides.rs`: removed new-biome filtering and its tag/stem rewriting.
- `crates/horizon-client-render/src/datapack.rs`: removed the obsolete known-biome callback.
- `crates/horizon-client/src/game/world_loading.rs`: uses one expanded table for packs, content mods, generation,
  persistence, and old chunks.
- `crates/horizon-client/src/game.rs`, `game/levels.rs`, `game/level_manager.rs`, `game/functions.rs`,
  `level_registry.rs`: adjusted override-loader call sites.
- `crates/horizon-client-render/src/biome.rs`: structured effect/audio/particle data, parsing, registry and default tests.
- `crates/horizon-client/src/ambience.rs`, `music.rs`, `game/music.rs`, `animate_tick.rs`: consume the parsed biome data.
- `crates/horizon-client/src/mod_content.rs`, `mod_content_tests.rs`: mod biome bundle, tag, modifier, and command support.
- `crates/horizon-client/tests/datapack_biome_sync.rs`: registry-data and chunk save/load integration tests.
- `crates/horizon-world/src/worldgen/biome_source/codec.rs`: new-biome source/tag resolution and sampling test.
- `crates/horizon-client/examples/level_probe.rs`, `tools/levels_e2e.py`: real protocol/chunk biome check.
- `docs/mods.md`: documented mod biome JSON.

## Tests (run by Claude on LENOVO)

horizon-world --lib 386 passed; horizon-client-render --lib 1457; horizon-client-lib --lib 1932; horizon-client --lib 1255 and `--test datapack_biome_sync` 3; `level_probe` example builds. Claude's fixes after Kimi's draft: a raw string in `codec.rs` (`r##`), the `Music`/`BackgroundMusic` methods moved next to their types in `horizon-client-render` (Kimi round 2), a stale call of `worldgen_overrides_of` with the removed `biomes` argument, two now-unused parameters prefixed with `_`. Not run here: `tools/parity.py` and `tools/levels_e2e.py` (they need the game's resources; Yoake's PC).

Tests written:

Resource-independent tests use generated JSON and tiny synthetic resources:

- Registry replacement, id stability, append order, network order, and return to the built-in table.
- New biome retention in tags and multi-noise points; fixed/checkerboard/tag decoding and an actual multi-noise sample.
- All colour groups, grass modifier, particles, ambient/mood/additions sounds, music, music volume, natural spawns, and
  missing-field defaults.
- LAN registry serialization and reconstruction, including effects and audio data.
- Chunk NBT save/load: the palette contains `mypack:azure_caves` by name and reloads to the same client id.
- A content mod's new biome, its biome tag, and an `add_spawns` modifier targeting that tag.
- Existing no-pack/no-modifier assertions remain as the unchanged-vanilla checks.

`tools/levels_e2e.py` now gives `mypack:moon` the new `mypack:luminous_moon` biome. Its protocol-level client checks that
the biome registry contains the id and that received moon chunks carry it. The Horizon guest also takes the existing moon
screenshot with the biome's colours available.

## Left for real-game/resource validation

- Build and run all tests listed in `.lane/TASK.md`; none were executed here because shell execution was forbidden.
- Run the extended LAN e2e with prepared resources. It checks registry sync and chunk ids, but not pixel colours or audible
  output; inspect the moon screenshot and, if needed, drive a client with distinctive loop/music/particle settings.
- The fast e2e uses flat terrain. Exercise a real noise level whose inline multi-noise list selects a new biome, then save,
  close, reopen, and confirm its region chunk and client colours/effects.
- Natural-spawn parsing and modifier output are unit-tested; an actual randomized spawn in a new biome still needs a game
  soak or a deterministic spawn fixture.

## Open questions / limits

- The parser follows the existing 26.3 environment-attribute format. Unsupported/custom particle types are skipped by the
  existing particle-id resolver; no new particle registry or WIT surface was added.
- Malformed biome JSON retains the existing loaders' leniency rather than adding a full vanilla codec validation pass.
- No real vanilla-client interoperability run was possible here; the e2e uses the existing protocol-level client and no
  Horizon-specific packet.
