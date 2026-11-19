# FR-F3: registry-based dimensions and full data-pack worldgen

Status: design only, no code changed. Base: branch `yoake-free/merge` at 862d10c89. Every file:line below was read on
that commit; line numbers drift, grep the symbol.

Goal: Yoake matches vanilla 26.3 (dimensions are data-pack level stems over a dimension-type registry), and mods can
add, generate, regenerate, unload and delete dimensions at run time. Motivating case: a cooking-duel minigame builds a
dimension per match from a GUI-chosen biome grid and seed, replays it, and deletes it afterwards.

No Minecraft source or game files are copied here. Vanilla ids and JSON field names are used; behaviour is described in
our own words.

---------------------------------------------------------------------------------------------------------------------

## 1. Inventory of the fixed-dimension assumption

### 1.1 The enums

| What | Where | Note |
|---|---|---|
| `enum Dim { Overworld, Nether, End }` + `id()`, `from_id()`, `shape()` | `crates/horizon-client/src/game.rs:3421-3449` | The one that matters. `Copy + Hash + Eq`. `from_id` maps every unknown id to `Overworld` (silent). |
| `enum Dim { D1, D2, D3 }` | `crates/horizon-shaders/src/textures.rs:29` | Texture dimensionality (1D/2D/3D) of shader-pack textures. Unrelated. Leave alone. |
| `TextureDim` alias | `crates/horizon-client-render/src/gfx/pack_textures.rs` (10 uses) | Same unrelated enum. |
| `enum Dimension` (`Overworld/Nether/End`) used as `LevelDimension` | `horizon_client::level` (imported `mod_api.rs:118`, field `mod_api.rs:204`), mapped at `game.rs:4744` and `game/discord.rs:40` | Mod API facing, see 1.7. |
| WIT `enum level-dimension { overworld, nether, end }` | `crates/yoake-wasm/wit/yoake-full.wit:663`, used at `:1241` (`native-note-block-change`) | Closed enum in a published ABI. Adding a case is a breaking change, see section 5. |

### 1.2 `Dim::` uses in `horizon-client` (232 in total)

Count by file group (`grep -c "Dim::"`):

| Group | Count | Files (count) |
|---|---|---|
| Core | 31 | `game.rs` (31) |
| Level management and loading | 18 | `game/levels.rs` (5), `game/world_loading.rs` (11), `game/integrated.rs` (2) |
| Portals, travel, respawn, beds | 59 | `game/nether_portal.rs` (19, 6 of them tests), `game/server_travel.rs` (12), `game/end.rs` (15), `game/respawn_anchor.rs` (4), `game/beds.rs` (1), `game/advancements.rs` (3), `game/lan_actions.rs` (6) |
| LAN/server guests (dimension is a `String` on the guest, converted back to `Dim`) | 15 | `game/lan.rs` (7), `game/lan_far.rs` (1), `game/remote.rs` (6), `game/command_harness.rs` (2) |
| Spawning, mobs, raids | 27 | `game/creatures.rs` (10), `game/natural_spawning.rs` (4), `game/raids.rs` (5), `game/cat_spawner.rs`, `wandering_trader.rs`, `trident.rs`, `lightning.rs`, `mob_blocks.rs`, `minecarts.rs`, `dragon.rs` (5) |
| Rendering, sky, light, audio | 31 | `game/render_tick.rs` (10), `game/item_draw.rs` (7), `game/native_block_entity_draw.rs` (3), `game/armor_stands.rs` (2), `game/frames.rs`, `native_mob_draw.rs`, `level_particles.rs`, `music.rs`, `extras.rs`, `shader_packs.rs` (3, Iris-style ids 0/-1/1), plus `game.rs` sky block around 16393-16544 |
| Commands | 17 | `game/commands.rs` (7), `commands/execute.rs` (3), `commands/players.rs`, `commands/test_runs.rs`, `game/maps.rs` (3), `game/stats.rs`, `game/discord.rs` (3) |
| Mod-facing native APIs | 16 | `game/native_hooks.rs` (4), `native_player_ops.rs` (4), `native_server_physics.rs` (3), `native_world_read.rs` (3), `native_server.rs`, `native_mannequin.rs`, `native_hooks.rs:9231` |
| Tests/fixtures | about 15 | `game/game_tests.rs` (8), `dispense_fixture.rs` (3), parts of `nether_portal.rs` |

The remaining uses are inside `Dim`-keyed maps: `Levels.parked: HashMap<Dim, Parked>` (`levels.rs:123`), `heard_open:
Vec<Dim>`, `levels_to_tick` iterating the literal array `[Overworld, Nether, End]` (`levels.rs:624`, `:743`).

Four hand-copied string-to-`Dim` matches that return `None`/error for any unknown id, so a mod API can only ever reach
the three vanilla levels: `native_hooks.rs:9231`, `native_player_ops.rs:476`, `native_server_physics.rs:216`,
`native_world_read.rs:106`. About 28 other sites call `Dim::from_id` (see above) and would silently turn a custom id
into the Overworld.

Classification of what the code actually tests (this decides what the replacement looks like). Vanilla branches on
either the level key or on a dimension-type property:

* Type property (replace with a query on the level's `DimensionType`): `ultrawarm` (fast lava `world_loading.rs:245`,
  `levels.rs:185`, `game.rs:3930`; water refusal `game.rs:13777`, `dispenser.rs:1303`), `has_skylight`/`has_ceiling`
  (`streaming.sky_light`, `world_loading.rs:869`, `game.rs:12788`, `game.rs:4286`), `coordinate_scale` (the hard-coded
  8.0 at `nether_portal.rs:321`, `server_travel.rs:458`, `commands/execute.rs:644`), `natural`/`bed_works`/
  `respawn_anchor_works` (`beds.rs:76`, `respawn_anchor.rs:110/285/602`), `has_raids` (`raids.rs`), `piglin_safe`,
  `monster_spawn_*`, `ambient_light` (`render_tick.rs:381` uses 0.1 for the Nether), `cloud_height`, `fixed_time`,
  `effects` (sky/fog/cloud style: `game.rs:16393-16544`, `end.rs:303`), `min_y/height/logical_height` (already data
  driven through `LevelShape`).
* Level key (stay keyed by id, because vanilla keys them by the level, not the type): the dragon fight (`dragon.rs`,
  `end.rs`), end portal targets, nether portal pairing (Overworld to Nether), advancement triggers
  (`advancements.rs:1784`), `/execute in` shortcuts, first-person level names, `minecraft:overworld` as respawn
  fallback and as the always-ticking level (`levels.rs:652`, `:692`).
* Weather/rain/thunder, wandering traders, cats, trident channeling, raids, the sky box: `dim == Overworld` in many
  places (`render_tick.rs:112/137/406/429/457/460/471`, `game.rs:16150/16433/16521`, `trident.rs:256`,
  `wandering_trader.rs:86`, `cat_spawner.rs:15`). In vanilla these follow the type (`has_skylight`, `natural`, whether
  the level can rain) or the level's own weather state. They must become property queries or a per-level capability
  flag.

### 1.3 Save paths

* Already vanilla for 26.3: `dimension_dir()` at `crates/horizon-client/src/save.rs:127-132` returns
  `world/dimensions/<namespace>/<path>`. It already accepts any id. `SaveDir::new(dimension_dir(root, dim.id()), shape)`
  at `world_loading.rs:661-664`.
* Legacy: `migrate_legacy` (`save.rs:~399`) moves `region/`, `DIM-1/`, `DIM1/` into the 26.3 layout. Pre-26.3 literals
  `DIM-1`/`DIM1` also appear in `horizon-world/src/datafix/*.rs` (data fixers, must stay) and in tests. Keep as is.
* Per-dimension side files: `horizon_structure_references_<id with ':' replaced>.json` (`world_loading.rs:830`),
  `migrate_end_gateway_exits` for the End (`:671`), forced chunks (`save.load_forced_chunks`).
* `world_gen_settings` / `WorldGen::read` / `preset` (`save.rs:~3377-3540`) read and write the level stems.
  `world_gen_unstable` (`save.rs:369`) and `datapack_dimensions` (`save.rs:351`) only decide the "experimental
  settings" warning (`dimension_is_vanilla_like`, `save.rs:297`). Test `world_stem_compatibility_reads_data_pack_
  dimensions` (`save.rs:~7092`) adds `mypack:moon`.
* Custom-dimension chunks today: a world with `mypack:moon` in its stems opens `Dim::from_id("mypack:moon")` as the
  Overworld, so the player is placed in the Overworld save folder. There is no loader for such a level.

### 1.4 Sky, fog, light, clouds

* Three `Environment` objects, one per vanilla dimension (`game.rs:2186-2216`, selected `end.rs:303-305`), no lookup by
  dimension type `effects` id.
* Sky boxes and fog per `Dim` (`game.rs:16393-16544`: overworld sky, end sky, nether fog as `environment_fog`).
* Lightmap: `nether_lighting`, `diffuse_lights(self.dim == Nether)` at eleven sites (`item_draw.rs`,
  `armor_stands.rs`, `frames.rs`, `native_*_draw.rs`), ambient light 0.1 only for the Nether (`render_tick.rs:381`,
  `natural_spawning.rs:364`).
* Streaming flags `sky_light` and `nether_shading` (`world_loading.rs:869-870`, `game.rs:12788`, `remote.rs:2377`).

### 1.5 Network and client

* Server to client already sends `dimension_type` (a registry index) and the level key string: `lan/hub.rs:360`
  (`dimension_type: i32`), `:610` (index looked up from `config.dimension` in the `minecraft:dimension_type` registry
  sync), and `lan/data.rs:37` / `:401` (`changed_dimension_type` only rewrites a type when a level's shape differs from
  `LevelShapes::VANILLA`).
* Client (`remote.rs:2255-2305`) reads `registries.dimension(spawn.dimension_type)`, builds a `LevelShape` from its
  `min_y/height/logical_height/has_skylight/has_ceiling`, falls back to `Dim::from_id(&dimension.name).shape()` if the
  shape is invalid, and then sets `self.dim = Dim::from_id(&spawn.dimension)`. So the shape is data driven but every
  visual and gameplay decision still goes through the three-value `Dim`, and the level key string is only kept as
  `r.level` / `heard_level`.
* `Hub::new` takes one `config.dimension` (the integrated host's active level). Guests carry `guest.dimension:
  String`.
* Chunk caches: the client `World` is one per active level and is replaced on respawn (`keep_world` logic in
  `remote.rs:2262`). No per-dimension chunk cache exists on the client; it is rebuilt on a level change.
* Level load tracker and "loading terrain" screen are per respawn (`level_load.start_client_load`).
* F3 text: `crates/horizon-client/src/debug_overlay.rs` (4 literal-id uses).

### 1.6 Other per-level state

* Time/weather: `p.world.game_time/day_time/weather` copied from the active level on every switch
  (`levels.rs:323-325`), so all levels share one clock and one weather. Registry `minecraft:world_clock` is synced
  (`lan/data.rs:46`) but there is no per-level clock.
* World border: per-dimension API strings (`native_world_border.rs`, 2 literal ids) but the border state itself is
  read per active level. Needs checking whether non-active levels have independent borders.
* Maps: `maps.rs:840` writes the level name of a map from `Dim`; `maps.rs:1323` uses `Nether` for the map colour
  quirk.
* Advancements: `advancements.rs:1080`, `:1784` (changed dimension trigger, hard-coded pairs).
* Mob spawning: `creatures.rs:800-848` builds the spawn context (`top` 127 or 319, `sea_level` 32 or 63, `nether`,
  `end`, `monsters_burn`) from `Dim`. Per-biome spawns and structure mobs read the biome source.
* Dragon fight and end gateways: `dragon.rs`, `end.rs`, `end_gateway.rs` keyed to `Dim::End`.
* Respawn: `respawn_anchor.rs`, `beds.rs`, `lan.rs:2615-2636` (a guest's dimension reset to Overworld when it cannot be
  found), `lan_actions.rs:3705`.
* Commands: `/execute in <dimension>` (`commands/execute.rs:643-649`), `/tp` and `/execute` argument `dimension`
  (`commands.rs:485`, `:1859`, `:3240`), `/pregen`, `/locate`, `test_locate.rs`. `commands.rs:925-927` maps `Dim` to
  0/1/2 (probably for the command argument list or `/time`/forced chunk ids, check).
* Lights: `lightning.rs:73` excludes End.
* LOD: `vanillagen/lod.rs` has 16 literal level ids, `lod_net/server.rs`.

### 1.7 Mod surfaces

* `docs/mods.md` worldgen section (data mods): tags, `horizon:add_to` for structure sets/pools/processor lists,
  NeoForge-style biome modifiers. Content validation `crates/horizon-mods/src/content.rs` (`worldgen/template_pool/...`
  at line ~2814 shows what is accepted). No dimension, dimension_type or biome file is accepted from a mod; the
  `minecraft` namespace is closed to mods except tags.
* Sandboxed WASM services take a `dimension: string` already (`yoake-full.wit:1156...2129`, `wasm_mods/
  server_services.rs`), but pass it through `dimension_id()` (`:48`, syntax check only) and then the four string
  matches of 1.2 reject anything but the vanilla three.
* Full-access (`docs/yoake2/*.md`): around hooks on nearly every engine function, handles, `engine_call`
  (`ENGINE_CALL_ABI.md`).

---------------------------------------------------------------------------------------------------------------------

## 2. Worldgen coverage today

Pack reading: `crates/horizon-world/src/worldgen/overrides.rs` (`WorldgenOverrides`). It is the one place a world's
enabled data packs meet the generator. The client side feeds it from `horizon_client::datapack::worldgen_overrides`
(called from `world_loading.rs:432`, `:539`, `:566`, `:603`, `:628`) and `horizon-client-render/src/datapack.rs`.

### 2.1 Kinds read and applied (`overrides.rs` `KINDS`, `TAG_KINDS`)

| Data-pack folder | Bundle | Consumed by the generator? |
|---|---|---|
| `dimension/*.json` (level stems) | `dimensions` | Yes: `Generator::for_world_with_settings` (`generator.rs:3441-3500`) takes `data.dimensions[id]` first, then a saved stem, then the world preset's. Only `generator.type == minecraft:noise` is built here; other types fail with an error (`generator.rs:3466`), and flat/debug are handled outside, for the Overworld only (`world_loading.rs:687-715`, `flatgen.rs`, `debugworld.rs`). |
| `dimension_type/*.json` | `dimension_types` | Partly: only `min_y`, `height`, `logical_height`, `has_skylight`, `has_ceiling` (`LevelHeight::of_json`, `overrides.rs:105`), with vanilla types hard-coded in `LevelHeight::vanilla` (`:85`). All other fields (ultrawarm, natural, effects, ambient_light, coordinate_scale, bed_works, ...) are read by nothing. |
| `worldgen/biome` | `biomes` | Replaces known biomes. A biome id the game does not know is dropped with a warning (`finish`, `overrides.rs:210`: "Horizon cannot add biomes yet, so it is left out"), and removed from tags and multi-noise lists. |
| `worldgen/noise_settings` | `settings` | Yes (inline or by id, `generator.rs:3470-3480`). `legacy_random_source`, noise `min_y/height` clamped to the level (`:3481-3495`). |
| `worldgen/density_function`, `worldgen/noise` | `densities`, `noises` | Yes. |
| `worldgen/material_rule`, `material_condition` | `rules`, `conditions` | Yes (surface rules). |
| `worldgen/block_state_provider`, `processor_list` | | Yes. |
| `worldgen/carver` | `carvers` | Yes (26.3 name). |
| `worldgen/feature`, `placed_feature` | `features`, `placed_features` | Yes (feature graph). |
| `worldgen/structure`, `structure_set`, `template_pool` | | Yes. |
| `tags/block`, `tags/fluid`, `tags/worldgen/biome` | tag bundles | Yes, tag merge semantics. |
| any other `worldgen/*` or `tags/worldgen/*` file | | Ignored, reported as `PackFile::NotRead(folder)` (`overrides.rs:61-72`), e.g. `world_preset`, `flat_level_generator_preset`, `multi_noise_biome_source_parameter_list`, `tags/worldgen/structure`, `tags/worldgen/world_preset`, `tags/worldgen/flat_level_generator_preset`. Needs checking which of these `GeneratorData` also loads from the prepared bundle without overrides. |

Inputs to the structure builder that exist only as the built-in bundle (prepared by `horizon-prepare`, loaded by
`GeneratorData::load_with`, `generator.rs:359-365`): world presets (`data.world_presets`), the presets for multi-noise
biome lists.

### 2.2 Biome sources (`worldgen/biome_source/codec.rs:24`)

`BiomeSourceDefinition { Fixed(String), Checkerboard { biomes, scale }, End, MultiNoise(Vec<(ParameterPoint,String)>),
Preset { name, inline } }`. The codec (`codec.rs:120-175`) decodes `minecraft:fixed`, `checkerboard`, `the_end`,
`multi_noise` (inline `biomes` list, or `preset` limited to the built-in `minecraft:overworld` and `minecraft:nether`).
The runtime is `biome_source/runtime.rs`. The generator takes the definition from the stem's `generator.biome_source`.
Biome catalog limits: 8192 biomes/tags (`codec.rs:43`). The End source requires the five vanilla End biomes.

So: the generator does consume data-pack overrides, for the kinds in 2.1, on top of the built-in bundle. It does not
consume data-pack biomes that the game does not already know, any other `generator.type`, dimension-type properties
besides height/skylight/ceiling, or a mod-supplied chunk generator.

### 2.3 Where it is wired, and what is Overworld-only

* `world_loading.rs:660-870` builds the terrain for `dim`: flat, debug and test terrain only when `dim == Overworld`;
  `VanillaTerrain::for_world_with_settings(.., dim.id(), ..)` for the rest. Old-world blending and structure search
  rings only for the Overworld.
* `vanillagen.rs:178-235` (`VanillaTerrain::for_world_with_settings`, `FlatTerrain`), `shared_data_with` caches
  `GeneratorData` by (dir, overrides) at `:678-725`.
* The structure reference file is keyed by dimension id (fine).

### 2.4 Parity infrastructure to build on

`crates/horizon-world/src/bin/worldgen_parity.rs` (JSON decoding transport; a protocol harness against the original
game), `tools/test_worldgen_parity.py`, `tools/test_datapack_worldgen_parity.py` with
`tools/fixtures/datapacks` and `tools/fixtures/tall_datapacks` (`docs/world-generation.md:216`, `docs/world-height.md`).
These already cover data-pack stems, custom heights and the Nether/End presets.

---------------------------------------------------------------------------------------------------------------------

## 3. Target model

### 3.1 Dimension registry (shared, in `horizon-world`)

New module `crates/horizon-world/src/dimension.rs` (no client dependencies):

```
struct DimensionType { /* every vanilla field, parsed from JSON, with defaults */ }
   // fixed_time, has_skylight, has_ceiling, ultrawarm, natural, coordinate_scale, bed_works,
   // respawn_anchor_works, min_y, height, logical_height, infiniburn, effects (id), ambient_light,
   // piglin_safe, has_raids, monster_spawn_light_level, monster_spawn_block_light_limit,
   // cloud_height, skybox/other 26.x environment attributes (see Needs checking)
struct LevelStem  { dimension_type: TypeRef (id | inline), generator: GeneratorSpec }
enum  GeneratorSpec {
    Noise { settings: Ref|Inline, biome_source: BiomeSourceDefinition },
    Flat(FlatSettings), Debug,
    Mod { handler: ModGeneratorId, biome_source: BiomeSourceDefinition, base: Option<Box<GeneratorSpec>> },
}
struct LevelRegistry { types: Map<Id,DimensionType>, stems: Map<Id,LevelStem>, ... }
```

Rules:

* Ids are the vanilla ones: `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end` (types of the same
  names, plus `minecraft:overworld_caves`). The built-in registry is a pure function of the prepared bundle; no
  vanilla behaviour changes when no pack or mod adds anything.
* Layering is vanilla's: built-in pack, then each enabled data pack in order, the last file for an id wins (already
  `WorldgenOverrides` semantics). Dimension types and stems become first-class (today `DIMENSIONS`/`DIMENSION_TYPES`
  exist but only the height is read).
* A stem for an id with a vanilla default replaces that level (already works for the three vanilla ones). A stem for a
  new id is a new level. A world has: the stems listed in `world_gen_settings.dimensions` (saved at creation) plus the
  enabled packs' stems (a pack's replacing a saved one of its id, as `world_gen_unstable` already encodes).
* `LevelHeight` stays as a cheap view derived from `DimensionType`.

### 3.2 Runtime level handle (client crate)

Replace `enum Dim` by a small copy handle, keeping source compatibility:

```
#[derive(Clone, Copy, PartialEq, Eq, Hash)] struct Dim(u16);
impl Dim { const Overworld: Dim = Dim(0); const Nether: Dim = Dim(1); const End: Dim = Dim(2); ... }
```

Because `Dim` derives structural `PartialEq/Eq`, the associated consts keep working in `==` and in `match` patterns,
so most of the 232 uses compile unchanged. Matches that list all three variants need a `_` arm (about 12, listed in
section 1.2: `commands.rs:925`, `creatures.rs:6766`, `discord.rs:40`, `end.rs:303`, `item_draw.rs:281`, `maps.rs:840`,
`natural_spawning.rs:364`, `shader_packs.rs:206`, `game.rs:4744/11429`, `native_*` string matches). Handles are slots
in a process-wide interner `LevelNames` (`id() -> &'static str`, the string leaked once per distinct id; names recycle
when an id is created again, so memory is bounded by distinct ids ever used). `Dim::from_id` returns
`Option<Dim>`-like `Dim::lookup(id)` for unknown ids; a thin `from_id` stays for vanilla callers during the migration
but must not map unknown ids to the Overworld.

Behaviour queries go through `Game::level_type(dim) -> &DimensionType` (or a cached `LevelProps` copied into `Game` on
level switch, beside `self.world.shape`). Each `self.dim == Dim::Nether` that tests a type property becomes
`self.props.ultrawarm` and so on (classification in 1.2). Key-based tests stay `dim == Dim::Overworld`.

`Levels.parked: HashMap<Dim, Parked>` already supports any key; `levels_to_tick` iterates the live registry instead of
the array of three. `open_level(dim)` (`levels.rs:170`) builds a level from its `LevelStem`.

### 3.3 Level lifecycle API (server-side, one `LevelManager` in `game/levels.rs`)

```
create_level(spec) -> Result<Dim>
   spec: id (ns:path, must not exist unless replace=true), dimension_type (id | inline JSON),
         generator: GeneratorSpec (datapack-style stem | Noise | Flat | Void | Mod),
         biome source (Fixed | Checkerboard | MultiNoise | Grid { cell_size, origin, cells[], outside }),
         seed: i64 (default world seed), persistent: bool, border, game rules inherited,
         spawn: pos option, keep_loaded: bool
regenerate_level(dim, new_seed: Option<i64>, spec_patch: Option<..>)   // players must be moved out first
unload_level(dim)       // save and close (what close_idle_levels already does after 300 idle ticks)
delete_level(dim)       // unload, then remove dimensions/<ns>/<path>/ and its side files
teleport_into(dim, player/entity, pos, yaw, pitch)   // existing player_teleport, generalised
list_levels() / level_info(dim)
```

Semantics:

* Persistent levels (created by a data pack stem or `persistent: true`) save under
  `dimensions/<ns>/<path>/` exactly as vanilla (`region/`, `entities/`, `poi/`, `data/`). The registry of
  mod-created persistent levels is written to a Yoake side file `<world>/data/yoake_levels.json` (absent when there are
  none, so vanilla saves are unchanged). Data-pack stems are not listed there; they are re-read from the packs, which
  is how vanilla works.
* Ephemeral levels (the minigame case) save nothing to the world folder: their `SaveDir` points under
  `<world>/yoake_temp/<ns>/<path>/` and is wiped when the world opens, and on `delete_level`. They are never written to
  `level.dat`, so closing the game mid-match leaves no residue.
* `regenerate_level`: require no player in the level (the API refuses, or moves players to a fallback level given by
  the caller and fires the normal respawn/changed-dimension path), drop the level, delete its `region`/`entities`/`poi`
  folders, reopen with the new seed. A regenerate keeps the same id and `Dim` slot, so mod references stay valid.
* `delete_level` of a level that holds players is refused unless `evacuate_to` is given. The three vanilla levels
  cannot be deleted. The respawn fallback, the tick-always rule and `closing` logic exclude only
  `minecraft:overworld`, as now.
* All operations run on the server tick thread, take effect at the next tick boundary, and report success or a typed
  error (`Exists`, `Unknown`, `Occupied`, `BadType`, `BadGenerator`, `Vanilla`).

### 3.4 Biome grid source

New `BiomeSourceDefinition::Grid { cell: i32 (blocks, multiple of 4), origin_x, origin_z, width, height (cells),
cells: Vec<BiomeIdx>, outside: BiomeIdx }`. A pure lookup at quart coordinates (biomes are stored per 4x4x4 cell in
chunk sections): cell index = floor((x - origin_x)/cell). Climate-free, so no noise sampling cost. It is added to the
codec (JSON `minecraft:grid`? no, a namespaced `yoake:grid` so vanilla ids stay unextended) and to the runtime. It
feeds the same decoration/structure biome queries as other sources. A multi-noise approximation of a grid is not
exact; this keeps the GUI's grid authoritative.

Terrain under the grid is any noise settings (for example a flat-ish vanilla `amplified` replacement, or a mod's own
`noise_settings` with a constant `final_density`) or the mod chunk generator below. Biome-dependent surface rules and
features then follow the grid because they read the biome source.

### 3.5 Mod chunk generator

`GeneratorSpec::Mod { handler, biome_source, base }`:

* Called once per chunk column (16 x 16, all sections), on a generation worker, never per block.
* Input: level id, chunk x/z, seed, `LevelHeight`. Output: for each section either a uniform block state, or a palette
  plus 4096 indices (`u16`), plus optional biome palette per section (64 entries), optional block entities, optional
  heightmap hint.
* Stages: `base = None`: only the mod's blocks, biomes from `biome_source`, then the engine does heightmaps, light and
  (if `decorate: true`) features and structures. `base = Some(spec)`: the engine generates the noise terrain first and
  the mod's callback post-processes the column (edit sections).
* Failure or timeout (budget about 2 ms warm) falls back to an all-air column and logs once, never panics the
  generator thread.

### 3.6 Data pack worldgen completion (vanilla parity)

* Read every dimension-type field into `DimensionType` and use it (3.2).
* Accept non-noise generators in any dimension: `flat`, `debug`, and `noise` with `legacy_random_source`. `flat`/`debug`
  must stop depending on `dim == Overworld` (`world_loading.rs:687-715`).
* Accept `world_preset` and `flat_level_generator_preset` files from packs, so a world-creation screen can list a
  pack's presets (read-only; `PackFile::NotRead` becomes `Read` for these).
* Biomes: new biome ids from data packs. Today they are dropped (`overrides.rs:210`). Needs a biome registry that is
  growable at the client too (colours, particle, music, spawn tables, `horizon-client-render/src/datapack.rs`). This is
  a separate item (F3-9), because the minigame works with existing biomes.
* `tags/worldgen/structure` and other tag folders in the ignored list: read them where the generator already has the
  consumer; report the rest.

---------------------------------------------------------------------------------------------------------------------

## 4. Client side

* Respawn/login: already carry the level key and dimension-type index (`remote.rs:2255`, `hub.rs:610`). Change the
  client to keep the level key (`r.level`) as a `Dim` handle from a client-side interner and look up the dimension type
  by index (`registries.dimension(..)`), not by name through `from_id`. The shape fallback `Dim::from_id(&name).shape()`
  becomes the vanilla type with that name or the Overworld type.
* Sky, fog, clouds, light: choose by the type's `effects` id (`minecraft:overworld`, `minecraft:the_nether`,
  `minecraft:the_end` stay the three built-in styles; an unknown id uses the Overworld style) plus per-type values
  (`fixed_time`, `ambient_light`, `cloud_height`, `has_skylight`). `game.rs:16393-16544`, `end.rs:303`, `render_tick.rs`,
  and the eleven `diffuse_lights(self.dim == Nether)` calls move onto a `SkyStyle`/`LevelProps` value stored once per
  level change. Shader packs: `shader_packs.rs:206` keeps `0/-1/1` for the three vanilla levels and maps other
  levels by their `effects` style (Iris maps by type name; Needs checking against the shader pack spec).
* Chunk cache per dimension: stays one `World` per respawn (`keep_world` rule) as now. A mod level has no special
  client cache. The client does not hold other levels.
* Dynamic dimension types: the `minecraft:dimension_type` registry is sent in the configuration phase. A level whose
  type was declared in a data pack or mod (so known when the session starts) works with no protocol change. A type
  created at runtime needs either re-entering configuration for connected clients (26.x has a reconfigure path, Needs
  checking) or is restricted to the types already in the sync data. First version: mod levels use declared types
  (`dimension_type/*.json` in mod data), inline types are accepted only when equal to an existing one.
* F3 screen (`debug_overlay.rs`): print the level key (already a string) and the dimension type id; stop assuming three
  names. The `Local Difficulty`/`Biome` lines already read the world.
* Loading screen and level load tracker: unchanged, per respawn.

---------------------------------------------------------------------------------------------------------------------

## 5. Mod API surface

### 5.1 Data mods (data pack JSON route)

* Allow in a mod's namespaces: `data/<ns>/dimension/*.json`, `data/<ns>/dimension_type/*.json`,
  `data/<ns>/worldgen/noise_settings`, `density_function`, `noise`, `biome` (own ids only, until F3-9),
  `world_preset`. Validated in `horizon-mods/src/content.rs` with the rule that a mod may only create ids in its own
  namespace (the `minecraft` namespace stays closed except tags and `horizon:add_to`).
* Mod stems are enabled like pack stems: always, in order after packs, with the same last-wins rule. A mod that wants a
  dimension per world makes it persistent through the data file; per-match levels are made at run time (below).
* `docs/mods.md`: new subsection "Dimensions".

### 5.2 Sandboxed WASM (WIT, additive minor)

All additions are new functions/types in an existing interface version, never changes to existing ones. The closed
`enum level-dimension` (`yoake-full.wit:663`) cannot gain a case without a major bump, so keep it for old mods
(vanilla levels only; a new level reports as `overworld`-like with a `dimension-id` companion) and add string-id
variants of the affected records. Concretely:

```
interface levels {                       // new, in the server-services world
  record level-spec { id: string, dimension-type: string, generator: level-generator, seed: option<s64>,
                      persistent: bool, spawn: option<block-pos> }
  variant level-generator { vanilla-stem(json), noise(noise-spec), flat(json), void,
                            mod-callback(callback-spec) }
  record biome-grid { cell: u32, origin-x: s32, origin-z: s32, width: u32, height: u32,
                      biomes: list<string>, cells: list<u16>, outside: string }
  create-level: func(spec: level-spec, grid: option<biome-grid>) -> result<_, level-error>
  regenerate-level: func(id: string, seed: option<s64>, evacuate-to: option<string>) -> result<_, level-error>
  unload-level / delete-level: func(id: string, evacuate-to: option<string>) -> result<_, level-error>
  list-levels: func() -> list<level-info>
  level-info: func(id: string) -> option<level-info>
}
// existing, now unrestricted by the four-match gate: player-teleport, edit-blocks, entities-in, world-border, ...
```

Exports a mod implements for a `mod-callback` generator:

```
interface chunk-generator {
  record column { sections: list<section>, block-entities: list<block-entity-blob> }
  variant section { uniform(string), paletted(palette-data) }       // block state strings, indices u16 RLE
  generate-column: func(level: string, cx: s32, cz: s32, seed: s64, min-y: s32, height: u32) -> column
}
```

Capability gate: `levels` calls need a new permission `levels` in the mod manifest (a mod may only delete levels it
created unless it also holds `levels-admin`). Per-mod quota: at most N live levels (default 4) and a disk budget for
ephemeral ones. Per-call time and size limits follow the existing service rules (`server_service`).

### 5.3 Full-access WASM (engine_call bindings + hook points)

* New bindings (ids appended to `crates/yoake-bindgen/ids.lock.json`, ABI minor bump 1.0 to 1.1): `levels::create`,
  `regenerate`, `unload`, `delete`, `info`, `list`, with the same request/response records as 5.2.
* New hook points (around hooks, `YOAKE_MIXIN2.md` semantics), all at per-chunk or per-lookup granularity:
  `worldgen::generate_chunk(level, cx, cz, &mut ColumnBuffer)` before/after the engine's noise fill (per chunk, so about
  100-200 ns hook overhead against milliseconds of generation); `worldgen::biome_at(level, qx, qy, qz) -> biome`
  (documented as hot: a per-column table API is provided so mods do not hook it per quart); `worldgen::sample_density(
  function_id, x, y, z)` as a read-only binding, not a hook; `level::before_open`, `level::after_open`,
  `level::before_close`; `level::spawn_params` (type property overrides).
* A mod's `ColumnBuffer` handle exposes `set_section_uniform`, `set_section_paletted`, `set_biome_section`, and
  `fill_box`, so a callback copies a prebuilt column in one call.

### 5.4 Chunk-generator replacement cost

The callback unit is a column (24 sections for the Overworld type), sections are uniform or palette+indices, so one
call and one memcpy per section, not one call per block. A full 256-chunk arena is a few tens of milliseconds of
callbacks.

---------------------------------------------------------------------------------------------------------------------

## 6. Vanilla parity rule

With no mods and no data packs the output must match today, byte for byte, in three areas:

1. Terrain and decoration output: same chunks for the same seed in all three dimensions.
2. Saves: same file names and bytes in `dimensions/minecraft/{overworld,the_nether,the_end}/...`, same `level.dat`,
   same side files; no new file appears unless a mod created a level (`data/yoake_levels.json` and `yoake_temp/` are
   only written when used).
3. Network: same registry sync packets and same login/respawn fields for all three levels (the `LevelShapes::VANILLA`
   path in `lan/data.rs:598-606` must keep returning "unchanged").

Tests proving it:

* Existing: `tools/test_worldgen_parity.py` and `tools/test_datapack_worldgen_parity.py`
  (`crates/horizon-world/src/bin/worldgen_parity.rs`), `cargo test -p horizon-world worldgen`, `lan/data.rs` registry
  tests (`:742-758`), `save.rs` tests including `world_stem_compatibility_reads_data_pack_dimensions`,
  `migrate_legacy` tests, `levels.rs` and `game_tests.rs`.
* New golden test A (save): generate 1,000 chunks per vanilla dimension with a fixed seed before the refactor (a tool
  `tools/gen_golden_chunks.rs` or an existing `probe` example) and store region-file SHA-256 hashes in
  `tools/fixtures/dim_golden.json`; the test regenerates and compares after each work item.
* New golden test B (network): dump the configuration-phase registry packets and login/respawn packets for the three
  levels to a hash fixture.
* New test C (refactor safety): `Dim::Overworld.id()`, `Dim::lookup("minecraft:the_nether")`, and an unknown id returns
  `None` (not Overworld).
* New test D (gameplay smoke): the portal tests in `nether_portal.rs` (they already assert pairs and scale), plus a new
  `tools/` e2e that enters Nether and End and returns, comparing player data.
* Per work item, a "no behaviour change" check: run the existing e2e suites (`yoake-test-runner`) before and after.

---------------------------------------------------------------------------------------------------------------------

## 7. Staged implementation plan

Naming: `F3-n`. Sizes S (under a day), M (1-3 days), L (3+ days). Files listed are the ones primarily touched; "never
touch" means conflict avoidance between parallel coders. Critical path: F3-0, F3-3, F3-6, F3-8 (bold).

Parallel groups: G1 = F3-0, F3-1, F3-2 (disjoint crates/files); G2 = F3-4a to d and F3-5 after G1; G3 = F3-6, F3-7
after F3-3 and F3-4; G4 = F3-8 after F3-6; F3-9 and F3-10 optional and independent.

### F3-0 (critical): `Dim` becomes an interned handle, API unchanged
* Goal: replace the enum by `struct Dim(u16)` with the three consts, interner, `lookup`, and no change of behaviour;
  add `_` arms; delete the four string matches in favour of `Dim::lookup`. Remove the silent Overworld fallback in
  new code paths but keep old callers behaving the same until F3-4.
* Files: `game.rs` (the enum), the 12 exhaustive matches listed in 1.2, the four `native_*` string matches. No other
  files should need edits.
* Depends on: none. Size: M (mostly mechanical; high conflict risk, land first, keep the branch short-lived).
* Tests: build, `cargo test -p horizon-client`, golden test C, parity golden A/B.
* Done: zero diff in behaviour; clippy clean.

### F3-1: `DimensionType` and `LevelStem` model in `horizon-world`
* Goal: section 3.1 module with full dimension-type parsing, vanilla built-ins, stem parsing, `LevelHeight` as a view,
  unit tests for the vanilla types, inline types, bad ranges (reuse `LevelHeight::of_json` rules).
* Files: new `crates/horizon-world/src/dimension.rs`, `worldgen/overrides.rs` (use it), `lib.rs` export.
* Depends on: none. Size: M. Tests: parse vanilla JSON for the four types; existing `overrides` tests unchanged.
* Done: `LevelHeight::vanilla` values equal the new built-ins; no consumer change.

### F3-2: Biome `Grid` source
* Goal: `BiomeSourceDefinition::Grid` plus codec (`yoake:grid`) and runtime lookup, with decode/encode tests; the
  multi-noise and fixed paths untouched.
* Files: `worldgen/biome_source/codec.rs`, `worldgen/biome_source/runtime.rs`, `worldgen/biome_source.rs`.
* Depends on: none. Size: S-M. Tests: unit tests (cells, outside, negative coords, non-multiple-of-4 rejected), a
  decoration smoke test on a 3-biome grid. Done: a stem with a grid builds a generator.

### F3-3 (critical): Level registry in the client, level stems from packs and mods, per-level `SaveDir`
* Goal: a `LevelRegistry` built from the built-ins, `world_gen_settings` stems and enabled packs; `Dim::lookup` returns
  handles for every stem; `open_level(dim)` takes the stem; `levels_to_tick` and `close_idle_levels` iterate the
  registry; `datapack_dimensions`/`world_gen_unstable` reuse it; persistence file `data/yoake_levels.json`; unknown
  level in a save (`level.dat` player dimension) falls back to the Overworld with a warning.
* Files: `game/levels.rs`, `game/world_loading.rs` (loading part), `save.rs` (the stem reading functions only),
  `datapack.rs`.
* Depends on: F3-0, F3-1. Size: L. Tests: save round trip with `mypack:moon` (open, move a block, close, reopen),
  existing save tests, golden A.
* Done: a world with a pack stem for a new id opens and saves under `dimensions/<ns>/<path>/`.

### F3-4: Replace key-based tests by type queries (split in four, disjoint files, parallel)
* a. Rendering and sky: `game/render_tick.rs`, `game/end.rs` (the environment selection only), `game.rs` sky block
  (16393-16544), `item_draw.rs`, `armor_stands.rs`, `frames.rs`, `native_*_draw.rs`, `level_particles.rs`,
  `shader_packs.rs`, `horizon-client-render` if needed. `SkyStyle` from the type `effects`. Size M.
* b. Spawning and mobs: `creatures.rs`, `natural_spawning.rs`, `raids.rs`, `cat_spawner.rs`, `wandering_trader.rs`,
  `trident.rs`, `lightning.rs`, `mob_blocks.rs`, `minecarts.rs`, `music.rs`. Size M.
* c. Portals, travel, respawn: `nether_portal.rs`, `server_travel.rs`, `respawn_anchor.rs`, `beds.rs`,
  `advancements.rs`, `lan_actions.rs`, `lan.rs`, `lan_far.rs`, `integrated.rs`, `remote.rs`. Coordinate scale from the
  type; portal pairing stays keyed. Size M-L.
* d. Commands and APIs: `commands.rs`, `commands/*.rs`, `maps.rs`, `stats.rs`, `discord.rs`, `native_*.rs`,
  `mod_api.rs`, `command_harness.rs`. `/execute in <any known level>`, `/tp ... <dimension>`, level argument completion
  from the registry. Size M.
* Depends on: F3-0, F3-1 (a, b), plus F3-3 for c and d. Tests: existing gameplay tests (`game_tests.rs`,
  `nether_portal.rs` tests), golden A/B, one new test per group that runs the same code on a custom level with the
  Nether's type and checks the Nether result.
* Done: `grep "Dim::Nether\|Dim::End"` shows only key-based uses (dragon fight, portal pairing, advancement triggers).

### F3-5: Generator for every level, any generator type
* Goal: flat/debug/noise per stem in any dimension, inline noise settings, structure search rings and old-world blending
  by key not by `dim == Overworld`; `GeneratorSpec::Mod` skeleton with a Rust-side trait `ModColumnSource` so F3-8 only
  plugs a callback in.
* Files: `game/world_loading.rs` (terrain building part), `vanillagen.rs`, `flatgen.rs`, `debugworld.rs`,
  `worldgen/generator.rs` (`for_world_with_settings` and the stem path).
* Depends on: F3-1, F3-2, F3-3 (shares `world_loading.rs`, so coordinate: F3-3 changes `open` paths, F3-5 changes the
  `Streaming` construction below line 660). Size L.
* Tests: flat in the Nether slot, a custom noise stem, golden A, the datapack parity script.
* Done: a stem with `minecraft:flat` for a new id generates flat terrain.

### F3-6 (critical): `LevelManager` run-time API (create / regenerate / unload / delete / teleport)
* Goal: section 3.3. A single `Game` method set called from commands, the native API and the WASM services; error type;
  ephemeral levels; evacuation; quotas.
* Files: `game/levels.rs` (new `level_manager.rs` submodule to keep conflicts low), `game/commands/` (a `/level`
  operator command: `create`, `regenerate`, `unload`, `delete`, `list`), `game/server_travel.rs` (reuse).
* Depends on: F3-3, F3-5. Size L. Tests: create/enter/leave/regenerate/delete cycle with files on disk checked;
  create 20 and delete 20 with no leak (memory and open file handles); delete refuses an occupied level; vanilla
  levels refuse delete.
* Done: the cooking-duel flow works from a command: create with a grid and seed, teleport in, regenerate, delete.

### F3-7: Network and client dynamic levels
* Goal: section 4 client parts: level key and type by index, `SkyStyle` on the client, F3 text, dynamic types from
  declared data, tests for a remote client entering a mod level.
* Files: `game/remote.rs`, `lan/hub.rs`, `lan/data.rs`, `net.rs`, `debug_overlay.rs`, `lan/guest.rs`, `lan/playerdata.rs`.
* Depends on: F3-0, F3-3 (levels), F3-4a for the sky. Size M. Tests: `lan/data.rs` registry tests, two-process e2e that
  joins a world while a mod level exists and goes there and back; golden B.
* Done: a remote client sees the right sky/ceiling/height in a level with a custom type.

### F3-8 (critical for the minigame): Mod API surface
* a. Data route: `horizon-mods/src/content.rs` validation, `docs/mods.md`. Size S-M. Depends on F3-1, F3-3.
* b. Sandboxed WIT: new `levels` and `chunk-generator` interfaces in `crates/yoake-wasm/wit/yoake-full.wit` (and
  `yoake-mod.wit` if the sandbox uses it), `wasm_mods/server_services.rs`, `wasm_mods/full.rs`, permission, quotas,
  coverage and ABI baseline files (`abi-1.0-baseline.wit`, `ids.json`). Size L. Depends on F3-6.
* c. Full-access: new bindings (`yoake-bindgen`, `ids.lock.json`), hook points `worldgen::generate_chunk`,
  `level::before_open/after_open/before_close`, ABI 1.1 note in `ENGINE_CALL_ABI.md`, guide in `MOD-GUIDE.md`. Size L.
  Depends on F3-6, F3-5 (the `ModColumnSource`).
* d. Sample mod and e2e: a "cooking duel" test mod (WASM) that draws a 4x4 biome grid, creates a level, teleports
  two bots, regenerates it, deletes it. Size M. Depends on b or c.
* Tests: WIT compatibility check (the repo's existing ABI baseline test must show additive-only changes), the e2e
  sample mod, a fuzz test for `chunk-generator` output validation (bad palette index, wrong section count).

### F3-9 (optional, independent): New biomes from packs and mods
* Goal: allow biome ids the game does not know (removes the drop in `overrides.rs:210`), client biome registry growth
  (colours, particles, music, mob spawns).
* Files: `worldgen/overrides.rs`, `horizon-client-render/src/datapack.rs`, biome tables in `horizon-client`.
* Depends on: F3-1. Size L. Not required for the minigame; it is the Forge/Fabric-level freedom item.

### F3-10 (optional): Per-level clock, weather and border
* Goal: `world_clock` per level, level weather, level border independent of the active level (removes
  `levels.rs:323-325` copying).
* Files: `game/levels.rs`, `native_world_border.rs`, weather code. Depends on: F3-3. Size M. Needs checking against
  vanilla 26.3 behaviour first (one shared clock for the Overworld and the Nether coordinated by type).

Critical path: F3-0 -> F3-3 -> F3-5 -> F3-6 -> F3-8b/c -> F3-8d. F3-1, F3-2, F3-4, F3-7 run in parallel with it
and join before F3-8d.

Suggested assignment: F3-0 and F3-3 one coder (they are the high-conflict ones, serialise), F3-1 + F3-2 (horizon-world
only) a second coder, F3-4a..d one coder each once F3-0 lands, F3-5 and F3-6 the strongest model, F3-8a/b/c separate
coders after F3-6 (b and c touch different files: `wasm_mods/server_services.rs` vs the bindgen crates).

---------------------------------------------------------------------------------------------------------------------

## 8. Risks and Needs checking

Risks
1. Merge conflicts: F3-0 and F3-4 touch about 50 files. Mitigation: F3-0 first and fast; const-based variants keep
   call sites unchanged; the F3-4 groups are file-disjoint.
2. Silent Overworld fallback in `from_id` (28 call sites): a missed site sends a mod level's entities into the
   Overworld save folder (data corruption). Mitigation: a clippy-style grep test in CI that forbids `Dim::from_id`
   after F3-4, and a debug assertion in the interner.
3. Registry sync for dynamic types (4): the dimension-type registry is a configuration-phase packet. A type unknown
   at join cannot be sent without re-configuration. First version declares types in data.
4. Per-level memory and file handles: every open level has a `Streaming`, `SaveDir`, tickets. Quotas and the
   300-tick idle close (`IDLE_TICKS`) must apply to mod levels. Leaked interned strings are bounded by distinct ids.
5. The closed WIT enum `level-dimension` and `Dimension` in the mod API: adding enum cases is a major ABI change.
   Use string ids in new records only and leave the enums.
6. Single active level model: `Game` holds one active `World` and swaps (`enter_level`, `swap_level`,
   `levels.rs:309-349`). Many players in many levels is already supported (parked levels tick), but each switch swaps
   large state, so a mod that teleports many players across levels every tick is slow. Document and measure.
7. Generation cost of mod callbacks: bound per-call time; a hanging callback must not stall the worker pool
   (use the existing WASM fuel/time limits and a fallback column).
8. Ephemeral level cleanup after a crash: `yoake_temp/` wipe at world open; also on delete failure retry at next open.
9. `chunk-generator` correctness: invalid palettes crash the mesher. Validate before accepting a column.
10. Structure generation in a level created at run time needs the structure reference file and the structure search
   rings (`search_rings_behind`, saved references) to be per level; today both are keyed by the dimension id string,
   which is fine, but the rings are Overworld only.

Needs checking
* Exact 26.3 dimension-type field set: this repo reads only five fields; 26.x moved some sky/fog/cloud settings into
  environment attributes and timelines. Confirm from the prepared bundle (`horizon-prepare` output) and update 3.1
  before F3-1. Same for `world_clock` per type (`lan/data.rs:46`).
* Whether 26.3 lets a client enter configuration again mid-session (for runtime types).
* How `world_gen_settings` stores dimensions in 26.3 saves (`save.rs:3377-3540`, `WorldGen`): needed by F3-3.
* Whether non-active levels have their own world border and weather in vanilla (F3-10).
* `commands.rs:925-927` meaning of the 0/1/2 mapping.
* Iris/OptiFine shader pack dimension ids for non-vanilla levels (`shader_packs.rs:206`).
* Where `GeneratorData::load` takes `world_presets` and the multi-noise presets from, and whether a pack can replace
  them (`PackFile::NotRead`).
* The prepared-bundle names for tag folders beyond block/fluid/biome.
* LOD code (`vanillagen/lod.rs`, 16 literal ids, `lod_net/server.rs`): per-level LOD for mod levels is out of scope
  but must not misattribute a mod level's chunks to the Overworld.
* Fabric/Forge mods that add dimensions usually also need portals and teleporters; a portal API is not designed
  here (use `teleport_into`). A follow-up F3-11 can add a data-driven portal type.
