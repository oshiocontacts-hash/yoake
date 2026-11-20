# FR-F3-3: level registry (packs, saves, mods), per-level open and save dir

Branch `yoake-free/f3-3` on `yoake-free/f3-12` (F3-0/1/2). No Minecraft files, source or prepared resources committed.

## What landed
* `crates/horizon-client/src/level_registry.rs` (pure data, in the client lib): `LevelRegistry` holds the three vanilla
  levels first, then every other level: stems saved in `world_gen_settings` (26.3: `data/minecraft/world_gen_settings.dat`
  `dimensions`, falling back to `level.dat` `Data.WorldGenSettings`), enabled data packs' `data/<ns>/dimension/*.json`
  (a pack stem replaces the saved one of its id, vanilla ids included), and mod registrations. Each `LevelEntry` has id,
  source, stem JSON, decoded `LevelStem`, resolved `DimensionType` and `LevelShape`; the save dir is
  `save::dimension_dir` (`dimensions/<ns>/<path>/`).
* Types go through `DimensionTypeRegistry::load` with the prepared bundle (`<resources>/data/minecraft/dimension_type`) and
  the packs' `dimension_type` files over it, so vanilla types now carry the visual/audio attributes (real-bundle test).
* Validation: level id must be a safe `ns:path` (no `..`, no uppercase, no empty segments: it names a folder);
  unknown type, bad grid, missing field, flat/debug generator on a non-vanilla level (F3-5), no biome catalog: each is a
  `LevelProblem` (logged with `log::error!`, left out). A bad stem never affects the three vanilla levels (a bad
  stem for a vanilla id is reported and the level keeps working as before).
* Mod API: `LevelRegistry::register_level / register_type` (+ `Game::register_mod_level`) before the world ticks;
  `freeze()` is called at the first `tick_server`, later calls return `LevelError::Frozen`. Duplicate ids refused. A mod
  level's registered type is written inline into the stem so the generator (which only knows pack types) can see it.
  Not exposed to WASM yet (F3-6).
* Game: `Game.level_registry`, `PreparedWorld.registry` (built in `prepare_world` via `world_loading::build_level_registry`;
  loads the worldgen data/catalog only when a non-vanilla stem exists), ids interned with `Dim::intern`.
  `open_level(dim)` takes shape, stem and save dir from the registry and refuses an unregistered `Dim` with a clear error
  (it can never fall into another level's folder). `levels_to_tick` and `native_levels_closing` iterate the registry;
  `close_idle_levels` was already generic over parked levels. `make_streaming` takes the stem from the registry instead
  of re-reading `world_gen_settings`; `sky_light` and `fast_lava` for non-vanilla levels come from the level's type
  (the vanilla three keep their exact old rules).

## Flipped sites (F3-0 list)
Registered ids now resolve through the interner (every registered level is interned at world load), so every
`lookup_or_overworld` site is flipped for registered ids; comment updated at all 26 sites. Decisions:
* world_loading.rs (player's level): registry lookup; unknown id logs a warning and uses the Overworld at spawn
  (`level.position = None`), per design. game.rs `finish_open_world`: same registry check.
* lan.rs (guest placement): new check, a saved guest level the world lacks gets a warning and the Overworld.
  `lan/playerdata.rs::apply` used to coerce every non-Nether/End level to the Overworld; it now keeps any well-formed id
  (registry check happens in lan.rs). Without this a player in a pack level was moved to the Overworld on rejoin.
* commands/execute.rs `/execute in`: any registered level (was only the three); unknown id still errors
  `argument.dimension.invalid`; coordinate scale from the type (`8.0` for the Nether, as before). test_runs.rs: registry.
* native_hooks (`level_random`), native_player_ops (teleport destination), native_server_physics (`physics_world`),
  native_world_read: `Dim::lookup` replaced by `Game::level_dim` (registry membership), so mods can reach registered levels.
* Kept as Overworld fallback (id can only be unknown there): commands.rs:486, commands/players.rs, command_harness.rs,
  integrated.rs, lan.rs, lan_actions.rs, lan_far.rs, native_mannequin.rs, nether_portal.rs, remote.rs, respawn_anchor.rs,
  server_travel.rs, game.rs:12694: they read a guest/respawn/packet id that came from a registered level or from a remote server
  (a client holds no levels). They resolve registered ids; a never-interned id is the Overworld as before.
* Not changed: native_server_physics `set_player_velocity` and native_world_border dimension allow-lists (three levels
  only; border is per active level, F3-10), horizon-world `Dim`-less sites.

## Tests
* `cargo test -p horizon-client-lib --lib level_registry`: 14 passed (vanilla-only registry; pack stem adds a 4th level
  with its own save dir; pack type shape/scale; pack replaces saved; vanilla-id replacement; bad stems (unknown type, no
  type, bad grid, flat, `..` id, bad JSON, bad type) reported while vanilla stays; invalid vanilla stem keeps the level;
  no catalog; mod registration, duplicate, bad id, mod type, freeze; id validation; prepared types fill visual attributes;
  real prepared bundle loads and matches `LevelShapes::VANILLA`; fixture data pack folder on disk through
  `datapack::discover/enabled_ids/worldgen_overrides` (4th level, broken pack reported); saved stems from `world_gen_settings`).
* `cargo test -p horizon-client --lib level_registry_tests -- --ignored` with `HORIZON_TEST_RESOURCES` (real game
  resources): 1 passed. Integrated-server `Game` loads a world whose `datapacks/moon` defines `mypack:moon` (yoake:grid),
  registry has 4 levels, no problems, `enter_level` opens it with its shape, `save_level` works, `dimensions/mypack/moon`
  path is right, an interned-but-unregistered id is refused and the active level stays the Overworld.
* Parity runs: see "Parity method".

## Parity method (no packs, no mods)
Vanilla level entries take their shape from the existing `level_shapes()` unchanged, their generator stem is the same
saved stem as before (registry stem == `world_gen_settings` entry when no pack; a pack stem wins in the generator already),
`fast_lava`/`sky_light` rules for the three are the old literals, `/execute in` scale is 8/1 as before, save dirs unchanged.
Verified by the existing suites: `cargo test -p horizon-client --lib` (see results below) and `-p horizon-client-lib --lib`
(save, datapack, playerdata tests). No offscreen game smoke run was done (disk was exhausted twice and the target dir was
wiped once); the ignored `Game` test above is the real-resources check.

Results: `-p horizon-client --lib`: 1206 passed, 1 failed (`entry::wasm_mods::full::tests::mod_call_reaches_the_peers_handle_call_and_never_re_enters_a_waiting_caller`, a threaded WASM timing test under load; passed alone on rerun, 1 passed), 41 ignored. `-p horizon-client-lib --lib`: 1653 passed, 0 failed, 48 ignored. horizon-world only gained a read accessor (`GeneratorData::source_catalog`), not re-run.

## Needs checking
* Network: `Hub` still picks the dimension-type registry index by the level id and `LevelShapes::get` falls back to the
  Overworld shape for unknown ids. A client joining/entering a pack or mod level is told type 0 (Overworld) and gets
  Overworld-shaped chunks. F3-4/F3-6/F3-7 must send the type and shapes per level (`lan/hub.rs:610/4124`, `lan/data.rs` `changed_dimension_type`).
* Travel/portals into a non-vanilla level are not implemented; `enter_level`, `/execute in` and mod teleport reach it.
* Generator for non-vanilla levels is the noise path only (`for_world_with_settings` with the stem). Flat/debug stems
  are rejected (F3-5); structure rings/old-world blending are still Overworld-only.
* A pack stem that the registry rejects for a vanilla id is still read by the generator itself (`GeneratorData.dimensions`).
* Sky/fog/light/spawning/raids etc. in a non-vanilla level use the Overworld-like branch until F3-4.
* `data/yoake_levels.json` (persistence of runtime-created levels) belongs to F3-6 and is not written. Levels with
  a saved folder but no stem anywhere (pack removed) are not registered; their folder is left untouched.
* The interner is process-wide: the guards are `Game::level_dim` and `open_level`'s registry check, not the interner.
* `try`: `entries()`/`ids()` allocate a Vec per `levels_to_tick` (3-4 interner reads per tick), fine.
* Working tree CRLF: git shows all `game/*.rs` as modified (autocrlf noise); only listed files were committed.
