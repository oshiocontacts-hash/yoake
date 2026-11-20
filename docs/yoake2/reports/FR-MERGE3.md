# FR-MERGE3: F3-4, F3-4b, F4M-4a and F4M-5b into `yoake-free/merge`

Base `e49fe6031` (MERGE2). Order and commits taken, no push:

| Step | Branch / commit | Notes |
|---|---|---|
| 1 | `yoake-free/f3-4` (f5d2386ef) | clean |
| 2 | `yoake-free/f3-4b` (1b85c162c, the latest on vps when merged) | clean |
| fix | "Merge fix: custom mob spawn rules match on the new Dimension::Level" | see below |
| 3 | `vps/yoake-free/f4m-4a`: first 811cfe9d4, then **fb8421f90** on top | clean |
| fix | "Fix F3-4 mod declarations ..." | see below |
| 4 | `vps/yoake-free/f4m-5b` (54d50bb2f) | one trivial conflict in `game/remote.rs` (`Login`: kept `r.level_keys = p.levels.clone();` and F4M-5b's `r.custom_mobs.clear();`) |
| regen | "Regenerate the engine glue (yoake-bindgen)" | bindings 2517-2532 |

## Semantic fixes made in the merge (not text conflicts)

1. F3-4 x F4M-3: F3-4 added `spawn_rules::Dimension::Level(LightRule)`; `mobs/custom/spawn.rs::dimension_id` (F4M-3) matched the three vanilla cases
   only, so `horizon-client-lib` did not compile after F3-4. Fixed with `Dimension::Level(_) => ""` and a comment. Limitation (for F4M/F3 to decide):
   `NaturalLevel` does not carry a level id, so a custom spawn rule's `dimensions` list never matches a level added by a dimension type.
2. F3-4's own `game.rs` module list: `#[cfg(test)] mod level_facts; mod level_registry_tests;` (the `#[cfg(test)]` of F3-3's `level_registry_tests` ended up on
   `level_facts`). Tests built, but a normal or feature build (`cargo check -p horizon-client --features yoake2-slice`) and the rustdoc of `regen.sh` failed with
   "cannot find `level_facts` in `super`". Fixed to `mod level_facts; #[cfg(test)] mod level_registry_tests;`. This is a bug of the F3-4 branch itself.

## Generated files and ids

No fragment, catalog JSON or WIT changed in these branches. `regen.sh --check` found 20 out-of-date generated files (the F3 `CustomLevel` type and the
F4M-5b `CustomState`/`Guest`/`Remote` fields); `regen.sh` rewrote them (6 files with content changes: `yoake_glue/mobs.rs`, `yoake-guest-sdk/src/stubs/mobs.rs`,
`ids.lock.json`, `coverage.md`, `functions-mobs.md`, `summary.json`; the rest were line-ending noise). New bindings: 2515/2516 (`Seen.custom`, from F4M-4a) and
2517-2532 (`CustomLevel` clone/eq/fields). `ids.lock.json`: 1533 ids, no duplicates, max 2532; the reserved range 1515-2514 and hook ids 710-1710 are untouched.
`regen.sh --check` passes after the regeneration. `horizon-mod-api-gen` (46, incl. `abi_1_0_is_additive`) passes; catalog versions unchanged (client 1.5.0, worker 1.6.0).

## Tests (CARGO_INCREMENTAL=0, CARGO_PROFILE_DEV_DEBUG=0, one target dir; df 20-26 GB throughout, never under 16 GB)

f4m_parity goldens (`-p horizon-client-lib --test f4m_parity -- --ignored`, HORIZON_TEST_RESOURCES=`...\horizon\.local\horizon-client\26.3`):

| Stage | A `golden_a_saves` | B `golden_b_network` | C `golden_c_render_types_and_identity` |
|---|---|---|---|
| after f3-4 + f3-4b | not buildable: `horizon-client-lib` failed to compile (fix 1) | - | - |
| after f3-4b + fix 1 + f4m-4a (fb8421f90) | pass | pass | pass |
| final tree (after f4m-5b; again after the regeneration) | pass | pass | pass |

(The f4m-4a stage was run on the tree with fix 1; it did not yet have fix 2, which only matters for non-test builds.)

Other suites on the final tree: `horizon-client-lib --lib` 1789 passed (48 ignored); `horizon-client --lib` 1228 passed (43 ignored);
`level_registry_tests -- --ignored --test-threads=1` 3 passed (incl. `mypack:moon`); `horizon-protocol` 227; `horizon-mobmodel` all pass
(13+18+1+1+4+6+25+17+37+1 and the golden tests); `horizon-command` 36; `horizon-server` 117; `horizon-world` 385 + its integration tests;
`horizon-mods --features yak` 92 + 42; `horizon-mod-api-gen` 46; `yoake-wasm` all binaries; `yoake-guest-sdk` 5 + 8 + 4;
`cargo check -p horizon-client --features yoake2-slice` ok.

Test notes:
* `level_registry_tests -- --ignored` with parallel threads fails 2 of 3 ("hz-f33-game-<pid> is already open in another instance"): the F3 ignored tests share one
  temp folder named by process id, so they need `--test-threads=1` (then they pass). A test isolation bug of the F3 branches, not of the merge.
* Flaky, each failed once in a full run and passed alone or in the next run: `entry::wasm_mods::tests::wasm_hot_reload_swaps_the_instance_at_the_next_tick`,
  `entry::wasm_mods::full::tests::a_server_guest_runs_server_tick_and_perform_command_on_the_server_thread` (both threaded WASM timing tests under load),
  `yoake-guest-sdk` `std_io::std_guest_uses_std_fs_log_clocks_and_random_through_wasip1` (passed alone, 4 of 4). Not caused by the merge.

## Resource checks (26.3 `entity-types.json`, `assets/minecraft`)

(a) Default attribute set of a custom row (`vanilla_mob_attributes`: keys common to all non-`misc` rows that have attributes): 86 rows, **27 keys, not 28**.
Vanilla rows carry 28 keys (54 rows), 29 (20), 27 (10), 30 (2). The missing key is **`minecraft:tempt_range`**: the 10 rows without it are bat, cod, ender_dragon,
glow_squid, pufferfish, salmon, shulker, squid, tropical_fish and wandering_trader, so the intersection drops it though 35 rows have it (33 of them 10.0). Nothing is extra:
the 27 are exactly pig's 28 without `tempt_range`. Other vanilla keys (attack_damage, attack_speed, flying_speed, luck, ...) are per species and correctly left out.
Values are the most common ones; all neutral (armor 0.0, follow_range 16.0, max_health 20.0, gravity 0.08, jump_strength 0.41999998688697815, step_height 0.6, scale 1.0,
below_name_distance 10.0, camera_distance 4.0, entity_interaction_range 3.0, name_tag_distance 64.0, safe_fall_distance 3.0, ...), except that
`movement_speed` comes to 0.7 (not a typical ground mob's 0.25): a custom mob that does not set it gets 0.7. To get 28 keys the rule would be "keys in at least half of the
rows" (adds `tempt_range` 10.0) or a fixed list. Not changed.

(b) Generated spawn egg: `spawn_egg.png` and `spawn_egg_overlay.png` exist in `crates/horizon-client/assets/horizon/textures/item/` (16x16 RGBA PNGs, 181 and 161 bytes),
are embedded with `include_bytes!` and written to `assets/horizon/textures/item/` of the content root, so `horizon:item/spawn_egg` and `horizon:item/spawn_egg_overlay` resolve.
The model is `parent minecraft:item/generated` (exists in 26.3), `layer0 = horizon:item/spawn_egg`, `layer1 = horizon:item/spawn_egg_overlay`, written to
`assets/<ns>/models/item/<path>_spawn_egg.json`; the item definition `assets/<ns>/items/<path>_spawn_egg.json` is a `minecraft:model` with two `minecraft:constant` tints.
Tints are `0xFF000000 | rgb` as a signed int (`argb()`): opaque, the same signed form as vanilla's item definitions (e.g. -6265536). The item's `item_model` component and the file path agree.
A mod that ships its own definition, model or texture keeps it. Checked by reading the code and the PNG headers; not rendered in a game.

## Two-client end to end (F4M-5b): NOT run

Wanted: a LAN host, a Horizon client and a stock client, Cinder Fox fixture, `/summon`. Why it could not be run here:
* `tools/vanilla_client.py` starts the stock 26.3 client on a virtual screen (`vscreen`, Linux or macOS natives) from the owner's imported game files (`tools/lab.py prepare`).
  This Windows host has no `vscreen` and no Linux natives, and `horizon-free` has no `.local/minecraft/26.3` import (only the prepared resources under `NestgateHorizon\horizon\.local`).
* `tools/lan_e2e.py` (12.7k lines) and `tools/anticheat_e2e.py` have no mob-sync or custom-mob scenario (no `mob_sync` or `cinder_fox` mention). A new scenario (summon the fox,
  pounce trigger, a late-joining client, a vanilla join check, reading the Horizon client's store through horizon-ctl) would have to be written, and the `fast`-profile
  `horizon-client` and `horizon-ctl` binaries built first (several GB, on a disk shared with other builds).
* The F4M-5b report says the same ("not run: no game resources here").

Covered instead, by in-process tests on the final tree (all pass): `horizon-protocol` `mob_sync` codec round trips; `lan::mob_sync` (12: first look, deltas, snapshot, "vanilla mob
nothing", who gets what); `lan::hub::only_a_placed_horizon_client_reads_mob_sync` (a connection without the Horizon protocol is not sent `horizon:mob_sync`: the no-payload half of case a);
`mobs::custom::remote` (9: the client store from anchors and deltas, case b; late join from the snapshot at ticks 5/7/8/9/12, case d; the client `hit` in the server's tick, the
pounce hit of case c); `anim_sync` (a keyframe lands the blow with a 3 tick delay).
NOT covered: a real stock 26.3 client connecting and drawing the fox (a); the glue in `game/lan_mob_sync.rs` and `game/remote*.rs` over a real socket (b, d); a real pounce at a real
player with the server hit counted once (c). Suggested: run on the Linux/macOS lab machine with `vscreen` (`--offscreen --no-focus`), adding a `mob-sync` check to `tools/lan_e2e.py`.
