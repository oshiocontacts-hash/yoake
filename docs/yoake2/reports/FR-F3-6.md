# FR-F3-6: travel scale, run-time levels, registration order, real level ids

Branch `yoake-free/f3-6` on `yoake-free/f3-5`. No Minecraft files, source or prepared resources committed. Nothing pushed.

## 1. Travel
* `Game::travel_scale(from, to)`: the ratio of the two levels' types' `coordinate_scale` (`level_manager.rs`). Portal travel
  (`server_travel.rs`) keeps its old table for the vanilla pairs (Overworld to Nether 1/8, End to Nether 1/8, Nether to Overworld 8,
  everything else 1) and uses `travel_scale` for any pair with a level past the vanilla three, so vanilla portals give the same
  numbers. A custom level has no portal frames: portal pairing stays keyed on the three, as the plan says.
* `/execute in <level>` (F3-3) already scales by the types' ratio and accepts every registered level; `/tp` across levels is
  `/execute in ... run tp` and so the same path; a mod's teleport (`native_player_teleport_impl`) reaches any registered level
  with the destination coordinates it is given (no implicit scaling; `travel_scale` is what a mod uses to scale).
* End gateways and the end portal stay End-only.
* A teleport into a level whose type the player's client does not have is refused with a message that says to reconnect (see 2).

## 2. Levels created while the server runs
`game/level_manager.rs` (new): `create_level(id, stem, persistent)`, `delete_level(id)`, `list_levels()`, errors
`LevelManagerError::{Registry(..), Occupied, Io}`.
* **Create.** The stem is a data pack's or a mod's (`{"type": id | inline, "generator": ...}`, any generator of F3-5, including
  `yoake:mod`). `LevelRegistry::create_level` has no freeze check; source `LevelSource::Created`. The level gets its own folder
  `dimensions/<ns>/<path>/` like a pack's. `persistent: false` makes it ephemeral: it saves under `<world>/yoake_temp/<ns>/<path>/`,
  is never listed for the next session, and `yoake_temp` is wiped when a world opens.
* **Persistence.** Persistent created levels are written to `<world>/data/yoake_levels.json` (`{"version":1,"levels":[{"id","stem"}]}`;
  the file is absent when there are none, so vanilla saves are unchanged, and is removed with the last level). The stem keeps its
  inline dimension type, so a level whose type only a mod knew still comes back. Loading (`build_level_registry`) reads the file and
  restores the levels (`LevelRegistry::restore_levels`); a stem that no longer validates is reported and left out.
* **Delete.** Refused while the local player, a guest or a pending teleport/trip is in or headed for the level
  (`Occupied`), for the vanilla three (`Vanilla`) and for levels of packs, the save or mods (`NotCreated`). Otherwise the parked
  level is dropped (its region files close), its folder is removed, the registry entry and the file entry go, and the hub is told.
* **Announcing to clients.** The protocol cannot add a level to a connected client: the dimension type registry and the login level
  list are sent when it joins. The rule, implemented and documented in the module: (a) `SyncData::announce_level` (interior-mutable,
  shared with the connection threads) puts the level and its dimension type, when no entry has it, into what every new joiner is
  sent (type appended after every earlier entry, so indexes already sent keep their meaning) and into the login level list;
  (b) a client connected earlier can be sent into a created level whose type is one it was sent (a vanilla or pack type); (c) one
  whose type is new gets a refusal naming the level and the reconnect (`Hub::guest_knows_level`, checked by the mod teleport; each
  guest remembers how many types it was sent, `Guest.known_types`); (d) the Horizon client interns a respawn into a level it did not
  see listed at login when its id is valid and it is not vanilla, up to 64 such levels, so (b) works.

## 3. Registration order
Mods that register before a world is read now count when the player's saved level is looked up. `mod_generator::{stage_level,
stage_generator, staged}` (process wide, never drained: the integrated server reads worlds on its own thread and a world may reopen).
`build_level_registry` restores the staged levels with the others (`LevelSource::Mod`) and `prepare_world` builds the player's level
with the staged generators; `finish_open_world` gives the game the staged generators beside the ones registered at run time. A player
saved in a level of a mod's generator reopens there (test below). `Game::register_mod_level` / `register_mod_generator` still work
after the world opened, before it ticks.

## 4. Real level ids for mods
* `LevelAccess::level_id()` (horizon-world, additive; default from `dimension()`), implemented by `GameLevel` with the level's real id;
  `dimension()` still answers one of the three, the Overworld for a level past them, so old guests see only vanilla values.
* The native sounds and particles a level hook queues (`native_play_sound`, particles) carry that id (they carried the enum's).
  Loot drops, `native_dimension()`, native teleport and the level events already used `Dim::id()`.
* `native_set_server_player_velocity` accepts every registered level (it allowed the three names).
* Not changed: the world border API (one border for the level in place, F3-10) still allows the three names.

## Parity (vanilla unchanged)
* The F3-5 chunk hashes (seed 5, chunk (3, -2) of each vanilla level) pass unchanged: Overworld `10844162340323160155`, Nether
  `3948111228871227996`, End `2685422101413284537`.
* Portal scale: `travel_scales_by_the_coordinate_scale_of_the_types` asserts the vanilla pairs (1/8, 8, 1/8, 1) and the pack levels.
* Packets: the hub test for vanilla levels (Respawn bytes, sections, keys) of F3-4 still passes; the registry packets with no created
  level are byte for byte the old ones (`a_level_type_is_added_after_the_vanilla_ones` and the new announce test);
  `data/yoake_levels.json` does not exist for a world with none (registry test).
* Saves: a world with no created level writes nothing new; `yoake_temp` is only removed.
* One behaviour change found by the tests and fixed: the flat structure generator (`VanillaTerrain::for_flat`) builds for the
  Overworld's build height, so a flat level of another height now keeps the plain flat ground instead of generating nothing.

## Tests
* client-lib: `created_levels_persist_restore_and_are_removable` (no freeze, save dirs, side file written/read/removed, restore with
  staged levels and a broken stem reported, removal rules), `a_created_level_reaches_new_joiners_and_keeps_old_indexes`,
  `a_level_created_after_a_guest_joined_needs_a_reconnect_when_its_type_is_new` (hub), `staged_levels_and_generators_are_kept_for_every_world`.
* Ignored, real resources: `a_level_created_at_run_time_has_its_own_folder_comes_back_and_can_be_deleted` (hosted server: create
  persistent flat level with an inline type, announced to the hub, side file, generate, edit, save into its folder, delete refused
  while in it, vanilla/pack/unknown refused, the registry the next session builds has it, an ephemeral level under `yoake_temp`,
  delete removes folders, registry entry, file and hub entry), `a_player_saved_in_a_mods_level_reopens_there`,
  `travel_scales_by_the_coordinate_scale_of_the_types`, and all earlier level tests incl. the vanilla hashes.

Results: `-p horizon-client --lib` 1213 passed, 0 failed, 49 ignored; `-p horizon-client-lib --lib` 1661 passed, 0 failed, 48 ignored; `-p horizon-world --lib` 385 passed, 0 failed, 2 ignored; ignored real-resource `level_registry_tests` 9 passed (incl. the vanilla chunk hashes). No offscreen game smoke run.

## Left / needs checking
* No two-process e2e: a client connected during creation, entering the level and the reconnect rule are covered by hub and sync unit
  tests and the client's intern rule is not exercised end to end (F3-7 golden B).
* `regenerate_level`, `unload_level` and per-level seeds of the plan's 3.3 are not done (create/delete/list are).
* Created levels have no world border of their own and share the clock and weather, as every level does (F3-10).
* Mod hooks that carry a `LevelDimension` still say Overworld for a level past the vanilla three; a mod reads the real id with
  `native_dimension()` or `LevelAccess::level_id()`.
* A deleted level's dimension type stays in what new joiners are sent until the server restarts (indexes already sent keep meaning).
* Teleporting the local player of an integrated server through the same rule is by its memory connection; not separately tested.
