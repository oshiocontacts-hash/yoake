# FR-F3-4: network and type queries for levels past the vanilla three

Branch `yoake-free/f3-4` on `yoake-free/f3-3`. No Minecraft files, source or prepared resources committed. Minimum scope
from the coordinator (network, sky/fog/light/weather/spawning/raids by type, vanilla parity, `mypack:moon` tests). The
rest of the F3-4 a-d split (portals, travel, commands, maps, stats, discord) is not done; see "Left".

## What landed

### 1. Network (hub and client)
* `SyncData` (`lan/data.rs`) gets `levels: Vec<NetLevel>` and `add_levels`. A level past the vanilla three carries the
  dimension type entry it names (`NetLevel.type_name`; the level id for an inline or mod type), its shape and the type's
  data (`DimensionType::to_value`). `add_levels` appends the type to `minecraft:dimension_type` after every existing entry
  with `from_pack = true`, so a client that knows the vanilla pack still gets its data. Vanilla entries keep their places.
  `level_type_id(id)` and `level_shape(id, vanilla)` are what the hub asks. `/reload` keeps `levels` with the registries.
* `LevelRegistry::network_levels()` builds those from the registry (`level_registry.rs`).
* `Hub::new`, `Hub::set_dimension` use `sync.level_type_id` / `sync.level_shape` (they used the level id as the type name
  and the three-slot `LevelShapes`, so a pack level was told type 0 and an Overworld shape). Login `levels` lists the extra
  level keys after the vanilla three. `game/lan.rs::lan_host` reloads the world's sync (as it does for biome packs) when the
  registry has levels past the vanilla three, and adds them.
* Client (`game/remote.rs`): the Login level keys are kept (`Remote.level_keys`); a respawn into a listed level interns it
  (`Dim::intern`; an unlisted unknown id is still the Overworld, so a server cannot grow the interner). The pure part of
  `enter_remote_dimension` is now `remote_level_type(registries, type_id)`: the type, the chunk shape (min_y/height) and the
  type's own data as JSON (`level_facts::type_json`: NBT bytes are booleans, as `json_to_nbt` made them).

### 2. Sky, fog, light, weather, spawning, raids follow the type (`game/level_facts.rs`, new)
* `LevelFacts`: skybox (Overworld / End / none), weather, Nether-style light, fog distance, ambient light, lightmap ambient,
  ultrawarm, day cycle, raids, highest mob y, sea level, monster spawn light rule. The vanilla three are constants with the
  exact values the old `Dim` tests gave; any other level is `LevelFacts::of_type(&DimensionType)`. `Game::level_facts()` is
  the query (`self.level_ctxs`, one `LevelCtx` per level read once at `enter_level`/world open; a client rereads at every entry).
* Sky/fog attributes: `environment_for(dim, ctxs, res)` returns the vanilla environment for the vanilla three and for a level
  whose type is a vanilla one; otherwise an `Environment` built from the type's own attributes
  (`Environment::load_type`, new in horizon-client-render, which `load_dimension` now calls). Used by `dim_environment()` and the
  per-tick level state (`environment:` in the level tick input).
* Flipped from `Dim::` tests to facts: weather and rain/thunder in sky/fog/clouds/particles/weather geometry/lightning/trident/
  loot place; skybox, clouds, celestial, end sky; environment fog; Nether lighting and diffuse lights (frames, armor stands,
  mob blocks, minecarts, native mob/block-entity draws); lightmap ambient; shadow ambient; firefly/creaking day timeline;
  ultrawarm (particles, dispenser bucket smoke); streaming nether shading; sky darkening (a level past the vanilla three samples
  its own environment); mob world: weather, `monsters_burn`, `daylight`, `sea_level`, `motion_top`/`surface_height` top; natural
  spawning ambient light; raid boss bar; item level dimension name (the level's own id); shader pack and custom entity model
  dimension number.
* Monster spawn light: `spawn_rules::Dimension::Level(LightRule)` (block light limit, constant or uniform light level from the
  type's `monster_spawn_light_level`; a uniform provider draws once even for an empty range, a constant none).

### 3. Parity
* Hub: the encoded `Respawn` packet and the encoder's section range for each vanilla level are identical in a world with and
  without a pack level; Login `levels` is the old three keys when the world has none (hub test).
* Registry packets: with no level past the vanilla three `add_levels(vec![])` leaves them byte for byte (Debug of the packets
  compared, data test); with one, the vanilla entries stay without data at the same indexes.
* Facts: `the_vanilla_types_read_as_the_vanilla_levels` checks the type reading agrees with the constants; the game test checks the
  Nether and Overworld keep the constants after visiting the pack level.
* Saves are untouched (no save code changed).

## Tests
* `-p horizon-client-lib --lib`: `lan::data::tests::a_level_type_is_added_after_the_vanilla_ones`,
  `lan::hub::tests::a_pack_level_is_told_its_type_and_shape_and_the_vanilla_levels_do_not_change` (type index 4, sections 4..12,
  level keys, nether and back, vanilla bytes equal with/without the pack level), plus the 14 F3-3 registry tests.
* `-p horizon-client --lib`: `game::level_facts::tests` (3: vanilla types read as the vanilla levels, a type sent as NBT reads back
  equal, a no-sky type).
* Ignored, real resources (`HORIZON_TEST_RESOURCES`, `-p horizon-client --lib level_registry_tests -- --ignored`): the F3-3 test now
  with a pack type; and `a_pack_level_follows_its_type_and_a_client_reads_the_same_type`: an integrated server loads a world
  whose `datapacks/moon` defines `mypack:moon` with its own type (`mypack:moon_type`: y 0..128, ambient 0.3, sky `#336699`),
  hosts it, enters the level (facts: overworld sky, weather, ambient 0.3, top 127, sky colour the pack's), keeps the
  Nether/Overworld constants; the hub's sync sends the registry, a client `Dynamic` built from those packets reads the same shape
  (min_y 0, height 128), the type's data, and the same `LevelFacts` as the server; the three vanilla types are known (no data) and have
  the vanilla shapes.
* Results: see RESULTS below.

Results: `-p horizon-client --lib` 1210 passed, 0 failed, 42 ignored; `-p horizon-client-lib --lib` 1655 passed, 0 failed, 48 ignored; ignored real-resource `level_registry_tests`: 2 passed. No offscreen game smoke run (the disk and the shared build lock made it impractical).

## Needs checking / left
* The "in-process hub + client" test is two halves joined at the registry packets, not a socket pair: the hub half builds the
  guest's Login/Respawn data (`spawn_info`, `level_keys`) and the client half is `remote_level_type` on the registry the hub sends. A
  two-process e2e that joins and travels (golden B) is F3-7.
* `Remote.heard_level` mods, `LevelLoadTracker` and the client's `Dim` of a level never listed still use the Overworld fallback.
* Not done from the F3-4 a-d split: portals/travel/respawn/beds/advancements (c), commands/maps/stats/discord/native APIs
  (d) still use the F3-0 key tests (`Dim::Nether`/`End`); `/execute in` and mod teleport already reach registered levels (F3-3). The
  cat, wandering trader, patrol and village siege spawners stay Overworld only: vanilla runs those custom spawners in the Overworld
  alone, a level of another id has none.
* `LevelDimension` (horizon-world, three variants) is still the Overworld for a level past the vanilla three in the per-tick
  level state; only its environment follows the type. A fuller query waits for F3-4c/d.
* `MobWorld.nether` and `end` stay key-based (they carry piglin, allay, golem, trader rules, not light), so a custom level of a
  Nether-like type is not a Nether for those mobs; light, ceiling, weather and ambient follow the type.
* `fast_lava`/water evaporation in fluid physics: F3-3 did the world's `fast_lava`; the player's `fluids.fast_lava` (game.rs, the
  player in lava) is still `dim == Nether`.
* The type's `skybox`, `timelines` and `cardinal_light` are read; `timelines` do not drive a custom type's day (a custom
  overworld-skybox type with a day cycle uses the prepared `day.json` timeline).
* Pack type attributes with no visual keys give black sky (`Environment` defaults), as an attribute-less vanilla type would.
* `SpawnInfo.sea_level` stays 63 for every level (vanilla parity for the three).
