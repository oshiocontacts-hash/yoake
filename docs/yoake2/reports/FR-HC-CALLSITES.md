# FR-HC-CALLSITES: Call-Site Patch & Hot-Caller Tuning for `horizon_client`

- **Branch**: `yoake-free/hc-callsites`
- **Starting `HEAD`**: `eb67b33883` (`horizon_client`: `18,128` live sites)
- **Commits**:
  - `0109471b7c` — `yoake-hook-codemod: isolate stamped overlay root per crate`
  - `b137a98efa` — `reports: add hit_counts-hc.txt from 40s game flight`
  - `7e36a32d44` — `yoake: exclude 44 hot horizon_client callsite callers (>=5k hits)`
  - This report commit — `docs/yoake2/reports/FR-HC-CALLSITES.md`

## 1. Stamped Overlay `loot.rs` Bug & Fix (`0109471b7c`)

Building with `--features yoake2-slice,yoake-hook-hit-count` failed in `horizon-world`: `lib.rs` called `loot::__yoake_bridge_install` and `loot::__yoake_bridge_mode`, while `target-stamped/yoake-stamp/root/crates/horizon-world/src/loot.rs` contained the unstamped workspace source (size `96,297`, old mtime `08:57`).

- **Root cause**: `crates/horizon-client/src/game/native_hooks.rs` (`#[cfg(test)]`) contains `include_str!` references across crate boundaries, including `../../../horizon-world/src/loot.rs`, `../../../horizon-world/src/item_defaults.rs`, `../../../horizon-client-render/src/gfx/native_render.rs`, `../client_lib.rs`, `../crafting.rs`, `../hunger.rs`, `../lan/hub.rs`, `../landing.rs`, `../menu.rs`, and `../motion.rs`. Because `overlay::source_closure` copies all `included_files` into `overlay_root`, and every crate previously shared `stamp_dir/root`, stamping `horizon_client` after `horizon_world` overwrote `crates/horizon-world/src/loot.rs` with the unstamped workspace copy while `.state/horizon_world.frozen.txt` still matched workspace inputs and skipped re-stamping `horizon_world`.
- **Fix**:
  - Isolated `overlay_root` per crate in `crates/yoake-hook-codemod/src/overlay.rs` via `crate_overlay_root(stamp_dir, crate_ident)` (`stamp_dir/root/<crate_ident>`) and checked `result.overlay_files.iter().all(|f| f.is_file())` in the fast path.
  - Updated `crates/yoake-hook-codemod/src/callsite_patch.rs`, `verify.rs`, `main.rs`, `tests/overlay_e2e.rs`, and `tests/callsite.rs` to use the per-crate overlay root.
  - Added regression test `per_crate_overlay_root_prevents_cross_crate_include_str_clobber` in `crates/yoake-hook-codemod/src/overlay.rs`.
- **Verification**:
  - `.\hz-cargo.cmd test -p yoake-hook-codemod --offline`: 50 unit + 5 `bridge` + 21 `callsite` + 21 `codemod_tests` + 1 `overlay_e2e` + 4 `shallow` passed (`overlay_e2e -- --ignored`: 2 passed).
  - `.\hz-bash.cmd tools/yoake-stamped-cargo.sh build --release -j 6 -p horizon-client --bin horizon-client --features yoake2-slice,yoake-hook-hit-count`: exited `0` (`130m 19s`), zero `UNSTAMPED` / `yoake-stamp` warnings.

## 2. Hit-Count Flight (`b137a98efa`)

Ran a single 40-second game-mode flight (`--warmup 0`) via `.local/hit-count-run.cmd` with `YOAKE_HOOK_HITS` writing `reports/hit_counts-hc.txt` (`6,923` slot entries):

- **Baseline (`horizon-client.exe`)**: `server_tick_ms=9.224`, `client_frame_p50_ms=1.122`
- **Hit-count (`yoake2-slice,yoake-hook-hit-count`)**: `server_tick_ms=14.702` (`+59.40 %`), `client_frame_p50_ms=1.783` (`+58.97 %`)

## 3. Hot Call-Site List & Exclusions (`7e36a32d44`)

### `horizon_client` callers (`>= 5,000` hits)

`126` call-site slots in `horizon_client` reached `>= 5,000` hits during the 40-second flight, spanning `44` callers. All `44` callers were added to `hook-ledger/callsite-hot-callers.txt`:

| Calls | Caller | Site ID | Fn ID |
|---:|---|---:|---:|
| 1,477,079 | `horizon_client::game::sculk::Game::post_game_event` | 40602 | 7974 |
| 997,423 | `horizon_client::game::Game::tick_blocks` | 25256 | 5101 |
| 972,288 | `horizon_client::game::remote_chunks::ChunkFormat::put_biomes` | 40031 | 7856 |
| 914,032 | `horizon_client::game::minecarts::Game::cart_entity_collisions` | 35885 | 6867 |
| 424,598 | `horizon_client::game::creatures::Game::guardian_bubbles` | 30820 | 5933 |
| 243,735 | `horizon_client::game::natural_spawning::Game::spawning_chunks` | 37694 | 7414 |
| 242,085 | `horizon_client::game::natural_spawning::Game::spawn_natural` | 37701 | 7415 |
| 235,592 | `horizon_client::game::raids::Game::sync_raiders` | 38824 | 7673 |
| 235,592 | `horizon_client::game::raids::Game::sync_villager_raids` | 38862 | 7677 |
| 235,389 | `horizon_client::game::bossbars::Game::boss_events_for` | 28069 | 5509 |
| 235,389 | `horizon_client::game::creatures::Game::projectile_targets` | 30572 | 5921 |
| 235,389 | `horizon_client::game::creatures::Game::tick_projectiles` | 30520 | 5914 |
| 235,389 | `horizon_client::game::lan::Game::lan_entities` | 33622 | 6569 |
| 235,389 | `horizon_client::game::lan::Game::lan_flush_effects` | 33737 | 6575 |
| 235,389 | `horizon_client::game::passengers::Game::tick_skeleton_traps` | 38173 | 7524 |
| 235,389 | `horizon_client::game::sculk::Game::sculk_step_on` | 40658 | 7984 |
| 235,389 | `horizon_client::game::sculk::Game::tick_mob_sculk_events` | 40734 | 8002 |
| 235,349 | `horizon_client::game::minecarts::Game::push_carts_from_mobs` | 36043 | 6880 |
| 212,785 | `horizon_client::game::villagers::Game::villager_item_sense` | 42752 | 8412 |
| 189,209 | `horizon_client::game::client_ambient::Game::sprint_dust` | 28354 | 5588 |
| 189,209 | `horizon_client::game::pandas::Game::panda_effects` | 38112 | 7516 |
| 189,209 | `horizon_client::game::remote::Game::remote_pushes` | 39453 | 7768 |
| 189,209 | `horizon_client::game::remote::Game::tick_remote_entities` | 39497 | 7770 |
| 106,694 | `horizon_client::game::block_entity_ticks::Game::tick_block_entities` | 27594 | 5423 |
| 84,896 | `horizon_client::game::sculk::Game::sculk_scan_chunk` | 40590 | 7973 |
| 40,601 | `horizon_client::game::Game::start_ticking_chunks` | 25237 | 5100 |
| 35,723 | `horizon_client::game::chat::block_properties` | 28203 | 5554 |
| 35,723 | `horizon_client::game::lan::Game::lan_host` | 33582 | 6560 |
| 35,723 | `horizon_client::game::native_hooks::Game::native_containers` | 36915 | 7130 |
| 35,723 | `horizon_client::game::native_hooks::Game::native_energy_tables` | 36921 | 7132 |
| 35,723 | `horizon_client::game::native_hooks::Game::native_fluid_tables` | 36918 | 7131 |
| 31,634 | `horizon_client::game::natural_spawning::<GameLevel as NaturalLevel>::unobstructed` | 37680 | 7405 |
| 28,968 | `horizon_client::game::beehives::Game::tick_beehive` | 27355 | 5389 |
| 28,098 | `horizon_client::game::custom_mob_debug::Game::custom_mob_debug_geometry` | 40063 | 12345 |
| 23,874 | `horizon_client::game::block_entity_anims::Game::tick_block_entity_anims` | 27506 | 5413 |
| 15,607 | `horizon_client::game::sculk::Game::tick_entity_events` | 40730 | 7999 |
| 15,192 | `horizon_client::game::remote_chunks::ChunkFormat::column` | 40027 | 7855 |
| 15,034 | `horizon_client::game::block_entity_anims::Game::find_conduits` | 27532 | 5415 |
| 14,949 | `horizon_client::game::Game::tick_dropped_items` | 25177 | 5094 |
| 14,052 | `horizon_client::game::minecarts::Game::push_from_cart` | 36053 | 6881 |
| 9,508 | `horizon_client::game::closest_space_nudge` | 26766 | 5269 |
| 7,841 | `horizon_client::game::mob_sounds::Game::start_mob_sounds` | 36215 | 6914 |
| 7,245 | `horizon_client::game::levels::Game::levels_to_tick` | 35238 | 6751 |
| 6,035 | `horizon_client::game::lan_entities::Game::lan_projectile_states` | 34628 | 6657 |

### Top 20 other-crate call-site slots

None of the top 20 call-site slots in the other crates run in the per-frame client loop (they belong to chunk generation workers, startup table/mipmap generation, the loading screen, relighting workers, per-tick particles, or tiled UI backgrounds), so no other crate's `callsites.patch` was modified:

| Rank | Hits | Slot | Crate | Caller (Callee) |
|---:|---:|---:|---|---|
| 1 | 30,375,936 | 24576 | `horizon_client_lib` | `horizon_client_lib::vanillagen::<VanillaTerrain as ChunkGenerator>::restore_proto` (`Chunk::block`) |
| 2 | 30,375,936 | 24577 | `horizon_client_lib` | `horizon_client_lib::vanillagen::<VanillaTerrain as ChunkGenerator>::restore_proto` (`ProtoChunk::block`) |
| 3 | 1,357,474 | 15505 | `horizon_world` | `horizon_world::block_survival::BlockSurvivalRegistry::new` (`BlockRegistry::state#1`) |
| 4 | 1,117,116 | 16667 | `horizon_world` | `horizon_world::worldgen::climate::index::build` (`Parameter::center`) |
| 5 | 787,780 | 24578 | `horizon_client_lib` | `horizon_client_lib::vanillagen::<VanillaTerrain as ChunkGenerator>::restore_proto` (`TerrainChunk::set_block_raw`) |
| 6 | 672,214 | 19512 | `horizon_client_render` | `horizon_client_render::level_loading::draw` (`HudBatch::fill`) |
| 7 | 611,910 | 19569 | `horizon_client_render` | `horizon_client_render::light::relight_layer` (`Region::cell#1`) |
| 8 | 413,318 | 18096 | `horizon_world` | `horizon_world::worldgen::structure::template::StructureTemplate::from_nbt` (`StateTransforms::full_block`) |
| 9 | 395,590 | 20791 | `horizon_client_render` | `horizon_client_render::ui::tile` (`HudBatch::rect_uv`) |
| 10 | 321,507 | 20911 | `horizon_client_lib` | `horizon_client_lib::animate_tick::AnimateTables::load` (`BlockRegistry::state#1`) |
| 11 | 316,818 | 16401 | `horizon_world` | `horizon_world::level::states::StateTable::new` (`BlockRegistry::state#2`) |
| 12 | 277,954 | 20347 | `horizon_client_render` | `horizon_client_render::particles::ParticleEngine::tick` (`ParticleEngine::tick_particle`) |
| 13 | 214,338 | 19709 | `horizon_client_render` | `horizon_client_render::mesher::MeshTables::new` (`FaceVisibility::is_empty`) |
| 14 | 214,338 | 19710 | `horizon_client_render` | `horizon_client_render::mesher::MeshTables::new` (`FaceVisibility::occlusion_face`) |
| 15 | 191,190 | 15417 | `horizon_world` | `horizon_world::block_collision::BlockCollisionRegistry::new` (`VoxelShape::min`) |
| 16 | 191,168 | 15418 | `horizon_world` | `horizon_world::block_collision::BlockCollisionRegistry::new` (`VoxelShape::max`) |
| 17 | 187,190 | 19729 | `horizon_client_render` | `horizon_client_render::mipmap::generate_native` (`Image::get`) |
| 18 | 187,190 | 19730 | `horizon_client_render` | `horizon_client_render::mipmap::generate_native` (`Image::get#1`) |
| 19 | 187,190 | 19731 | `horizon_client_render` | `horizon_client_render::mipmap::generate_native` (`Image::get#2`) |
| 20 | 187,190 | 19732 | `horizon_client_render` | `horizon_client_render::mipmap::generate_native` (`Image::get#3`) |

## 4. Call-Site Patch Regeneration & History Check

Adding 44 hot callers retired 529 previously-live sites, which initially triggered `callsite lost` because `--tombstone-site` flags for all 529 sites would exceed Windows' `32,767`-char command-line limit. In `crates/yoake-hook-codemod/src/callsite.rs` (`reconcile_and_plan`), unrewritten live call sites whose `meta.caller` resolves via `fn_key_by_id` to an entry in `hot_callers` are now retired and tombstoned automatically when `config.retire_ineligible` is set (matching the in-place rewrite path). Regression test `callsite_patch_retires_live_sites_when_caller_added_to_hot_callers` was added to `crates/yoake-hook-codemod/tests/callsite.rs`.

- `.\hz-cargo.cmd test -p yoake-hook-codemod --offline`: 50 unit + 5 `bridge` + 22 `callsite` + 21 `codemod_tests` + 1 `overlay_e2e` + 4 `shallow` passed (2 ignored slow `overlay_e2e` tests).
- `yoake-hook-thir dump` -> `.local\hc.calls.json` across 463 files, then `yoake-hook-codemod callsite-patch --workspace-root . --crate horizon_client --thir .local\hc.calls.json --verify --stamp-target C:\Users\USER\NestgateHorizon\horizon-free\target-stamped`:
  - `joined=102260 eligible=17555 rewritten=17555`
  - **Live sites**: `17,596` (`-532` vs `18,128` at `eb67b33883`)
  - **Excluded**: `87,786`:
    - `31,057` `caller not in fn ledger`
    - `26,165` `non-engine callee`
    - `23,082` `path call`
    - `2,981` `inside macro invocation`
    - `1,957` `hot-path`
    - `866` `unparseable return type`
    - `637` `unwritable arg type`
    - `292` `dyn receiver`
    - `292` `track-caller`
    - `214` compile errors (`E0425`: 2; `E0502`: 94; `E0503`: 113; `E0597`: 5)
    - `132` `unparseable arg type`
    - `66` `unwritable return type`
    - `45` `named lifetime in callee return`
  - **Verify result**: `clean in 1 round(s), withdrew 0 site(s)`
  - **Patch output**: `375 file(s), 5755 hunk(s)` in `hook-ledger/horizon_client.callsites.patch`
- `check-history --base .local\base-ledger --current hook-ledger` (against `eb67b33883:hook-ledger`): `check-history: ok`

## 5. Stamped Release Build & 21-Run Game-Mode Bench Gate

Normal stamped release build (`.\hz-bash.cmd tools/yoake-stamped-cargo.sh build --release -j 6 -p horizon-client --bin horizon-client --features yoake2-slice`): exited `0` (`132m 45s`), zero `UNSTAMPED` / `yoake-stamp` warnings.

Bench gate command:
`python tools/yoake-hook-bench-gate.py --mode game --runs 21 --flight-seconds 40 --ctl C:\Users\USER\NestgateHorizon\horizon-free\target\debug\horizon-ctl.exe C:\Users\USER\NestgateHorizon\horizon-free\target\release\horizon-client.exe C:\Users\USER\NestgateHorizon\horizon-free\target-stamped\release\horizon-client.exe`

| Metric | Baseline (median, range) | Stamped (median, range) | Paired median (95% CI) | Unpaired median | Limit | Status |
|---|---:|---:|---:|---:|---:|---|
| Server tick time (fly smoothed `tick_ms`) | 19.414 ms (17.307–22.083, stdev 1.222) | 18.971 ms (16.338–21.448, stdev 1.179) | -3.30 % ([-5.00, -1.41] %) | -2.28 % | 1.0 % | `ok` |
| Client frame time (`p50`) | 2.190 ms (1.911–2.508, stdev 0.144) | 2.221 ms (1.930–2.516, stdev 0.135) | -0.74 % ([-2.91, +2.33] %) | +1.43 % | 1.0 % | `ok` |
| Client CPU frame time (`p50`, info) | 2.066 ms (1.800–2.364, stdev 0.136) | 2.087 ms (1.815–2.377, stdev 0.130) | -0.73 % ([-3.10, +2.27] %) | +1.00 % | 1.0 % | `info` |

Both gated metrics passed under the `1.0 %` paired median limit at the `5,000`-hit exclusion threshold; no `1,000`-hit pass was needed.
