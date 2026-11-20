# FR-F3-7: unload, regenerate, per-level seeds, and the levels e2e

Branch `yoake-free/f3-7` on `yoake-free/f3-6`. No Minecraft files, source or prepared resources committed. Nothing pushed.

## Important: a build break in f3-4 to f3-6, fixed here
`game.rs` declared `mod level_facts;` and `mod level_manager;` just above `#[cfg(test)] mod level_registry_tests;`, so the attribute
moved onto the wrong module: the non-test build had no `level_facts` (and compiled the test module). Every `cargo test` run passed
because the test build has all three; `cargo build -p horizon-client --bin horizon-client` failed. The e2e build found it. Fixed in
this branch (`mod level_facts; mod level_manager; #[cfg(test)] mod level_registry_tests;`). f3-4, f3-4b, f3-5 and f3-6 as pushed do
not build the game binary until this is merged; cherry-pick that three-line change if they are built first.

## 1. Leftovers
* **Per-level seeds.** A level's stem may carry `"seed": <number or numeric string>` (a Yoake extension; vanilla stems ignore it).
  `LevelEntry.seed` / `LevelRegistry::seed(id, world_seed)`: the level's own, else the world's (as every vanilla level has).
  `open_level` and the player's level at load generate with it (terrain, mod-generator context, the level's `World` seed). It is part
  of the stored stem, so `data/yoake_levels.json` keeps it. `set_seed` changes it for created levels only.
* **`unload_level(id)`**: saves the level and closes it (chunks, region files and streaming freed) while it stays registered;
  entering it opens it again from its folder. Refused for the level in place, for levels anyone is in or headed for, and for the
  vanilla three; a level that is not open is already unloaded.
* **`regenerate_level(id, new_seed)`**: closes the level, deletes its `region`, `entities` and `poi` folders and leaves it registered
  under the same id (so references stay valid); with `new_seed` a created level takes it (persisted). Refused while occupied and for
  the vanilla three; a level of a pack, the save or a mod cannot take a new seed.
* Control socket (dev tooling, no player command): `horizon-ctl level list | create ID STEM_JSON [temp] | delete ID | unload ID |
  regenerate ID [SEED]` (an integrated server's levels are its server thread's, so the call runs there). `status` has a new
  `level` object: `id`, `min_y`, `height`, `sky`, `weather`, `ambient`, `nether_light`, `sea_level` of the level in place.
* **Travel guard.** A level teleport (`/execute in`, `tp`, mod teleport) into a level created after the player joined whose type the
  player's client was not sent is refused for any player, with a chat line that says to reconnect (it was only the mod teleport in F3-6).

## 2. Isolation of the ignored tests
The `level_registry_tests` shared one world folder per process and process-wide state (staged mods, caches), so they failed in
parallel. `with_world_options` now takes a serial guard (they run one at a time) and a world folder of its own per test
(`hz-levels-<pid>-<n>`), and the staging test stages inside the guard. `cargo test ... level_registry_tests -- --ignored` passes
without `--test-threads=1`.

## 3. The levels e2e (`tools/levels_e2e.py`, `crates/horizon-client/examples/level_probe.rs`)
Horizon games only: a host, a guest and a late-joining client, each `--offscreen --no-pause --no-focus` in its own scratch folder,
on a port the OS picks, through the lan_e2e.py harness (`Game`, `horizon-ctl`). The script refuses to start while another
horizon-client of the machine is running (`--force` overrides), so it never overlaps the MERGE agent's runs. A stock vanilla client
cannot run on this Windows host (`vanilla_client.py` wants vscreen, Linux/macOS natives and imported jars), so the stock side is a
protocol-level client, `examples/level_probe.rs`: no game or window, the handshake, configuration and play a multiplayer client does,
answering only keep-alives and `PlayerLoaded`, sending its command as chat. Run (after `cargo build -p horizon-client --bin
horizon-client --example level_probe -p horizon-client-lib --bin horizon-ctl`):
`PYTHONUTF8=1 python tools/levels_e2e.py --bin-dir target/debug --resources <prepared resources>`.
Results of the last run, all five checks pass:
* **setup**: the world's pack (two types, flat `mypack:moon` and `mypack:hell`) is in `level list` after the vanilla three; the
  Overworld's facts are the vanilla ones (`-64`, 384, overworld sky, weather).
* **travel**: the guest runs `execute in mypack:moon run tp` and is there: `status.level` says min_y 0, height 128, overworld sky
  with weather, ambient 0.3 (the pack's), sea level -63 (the flat generator's); grass at y 4 and bedrock at y 0 where the server's
  flat floor is; `mypack:hell` gives no sky, no weather, Nether light, ambient 0.1; back in the Overworld the facts and the block
  under the player are the vanilla ones; the host stayed in the Overworld all along.
* **stock** (protocol client): it is sent `dimension_type` entries `overworld, overworld_caves, the_end, the_nether, hell_type,
  moon_type` (the pack's after the vanilla ones, as `add_levels` orders them), a login with the five level keys, and runs
  `execute in mypack:moon` through chat: the respawn names `mypack:moon` with type `mypack:moon_type` and sea level -63, 9 chunks
  arrive of 8 sections (128 blocks) each, and it is not disconnected.
* **created**: `level create made:arena` (a flat level with an inline 64 high type) while the guest is connected: the guest asking to
  enter is told "reconnect to enter it" and stays in the Overworld; after `disconnect` and `connect` it enters, stands on the level's
  floor with `status.level` 64 high; the folder and `data/yoake_levels.json` exist; a client that joins afterwards enters at once; a
  second protocol client sees `made:arena` in its login list and registry, enters it and gets 4-section chunks.
* **lifecycle**: delete refused while two players are in the level; unload keeps the folder and the level comes back with its edit
  (a gold block); regenerate with seed 77 loses it; delete removes the folder, the registry entry and the file.

## Parity and tests
* The F3-5 vanilla chunk hashes still pass; the hub/registry byte tests of F3-4/F3-6 pass.
* New unit test `a_level_may_have_its_own_seed`; new ignored `levels_take_their_own_seed_unload_and_regenerate` (a mod generator
  records the seed it was made with: the world's by default, the stem's, the regenerated one; an edit survives unload and goes with
  regeneration).

Results: `-p horizon-client --lib` 1213 passed, 0 failed, 50 ignored; `-p horizon-client-lib --lib` 1662 passed, 0 failed, 48 ignored; `-p horizon-world --lib` 385 passed, 0 failed, 2 ignored; ignored real-resource `level_registry_tests` 10 passed in parallel (default threads); `tools/levels_e2e.py` 5 of 5 checks pass.

## Needs checking / left
* **The real stock client** (`tools/vanilla_client.py --join`, needs vscreen and the imported vanilla jars on Linux/macOS) was not run:
  what a vanilla client does with the extra `dimension_type` entries, the login level list and the pack level's respawn is covered
  only by the protocol-level probe (which is Horizon's own `Session`, so it also speaks Horizon's extras) and by the packet tests.
  Run `levels_e2e.py` on Linux with a stock client against the same host to close it.
* The client's biome zoom for a level with its own seed uses the world's hashed seed (`SpawnInfo.seed`), so biome blending at the
  edge of a biome in such a level can differ slightly on a client; terrain and biomes themselves are the server's.
* Created levels' types stay in the registry new joiners are sent after deletion until the server restarts.
* The e2e uses flat levels (the debug build generates noise terrain too slowly for a harness); noise levels are covered by the
  ignored game tests.
