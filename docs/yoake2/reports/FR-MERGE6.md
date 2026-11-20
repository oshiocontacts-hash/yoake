# FR-MERGE6: permanent id reservations, F4M-6 (custom mob rendering), F4M-10 (WASM mob API) and the in-game checks

Base `05c4d6bae` (MERGE5). No push.

## 1. Permanent fix for the id collisions (commit "Reserved id ranges ...")

* **Hook ids.** `hook-ledger/meta.json` has a `reserved` list; `LedgerSet::assign` (the allocator used by `sync-ledger` and the stamp tool) jumps over reserved ranges and `LedgerSet::validate`
  (run on every load and save) fails when a ledger entry lies in one, when a range is empty or outside `1..TEST_ID_BASE`, or when two ranges overlap. The custom-mob hook range moved to
  **1_040_000..=1_040_999** (`reserved_ids::HOOK_FIRST/LAST`); `next_id` stays 11755. Tests: `the_allocator_skips_a_reserved_range`, `a_live_id_inside_a_reserved_range_fails_validation`,
  `reserved_ranges_must_not_overlap_or_be_empty` (`yoake-hook-ledger`, 36 tests), plus the pinned `the_reserved_id_ranges_are_in_the_locks` (`horizon-mods`).
* **Binding ids.** `ids.lock.json` has a `reserved` list (custom_mobs 1515..=2514, the `unbound` sentinel stays); `Lock::assign` skips reserved ranges, `reserved_violations` (also part of
  `append_only_violations`) reports an entry inside a range (an `unbound` sentinel excepted). Tests: `new_ids_skip_a_reserved_range`, `an_entry_inside_a_reserved_range_is_a_violation_but_the_unbound_sentinel_is_not`
  (`yoake-bindgen`).
* Documented in the new `hook-ledger/README.md` and `crates/yoake-bindgen/README.md` (incl. how to merge a branch that was cut before a range was reserved). F4M-10 assigns no ledger hook ids, so the range is unaffected.

## 2. Merges

* `f4m-6` @88509ce78 (client rendering of custom mobs; on top of f4m-4c 040ce9eb0): no conflicts.
* `f4m-10` @4e67e7d49 (WASM mob API: server catalog 1.3.0 / client 1.7.0, services 260-283, 20 script hooks in `tools/native-hook-fragments/f4m_mobs.json`, permission `service.mobs`, `fox_ai` guest; it already
  contained MERGE5): one conflict in `game/custom_mob_draw.rs` (`custom()` in the event sink: kept my `info` log line and F4M-10's `rt.mob_anim_event` call).
* After the merge: `YOAKE_UPDATE_WIT=1 cargo test -p horizon-mod-api-gen --lib` and `regen.sh` produced no change beyond line endings (discarded); `regen.sh --check` passes. Catalog ids unique, `abi_1_0_is_additive` passes.

## 3. Test results

Parity goldens `f4m_parity -- --ignored` (A `golden_a_saves`, B `golden_b_network`, C `golden_c_render_types_and_identity`):

| Stage | A | B | C |
|---|---|---|---|
| after f4m-6 (93556fba4) | pass | pass | pass |
| after f4m-10 (final tree) | pass | pass | pass |

Final tree: `horizon-client-lib --lib` 1877 passed (48 ignored); `horizon-client --lib` 1243 passed (52 ignored), which includes `golden_vertex_hashes_of_every_animation`; `level_registry_tests --ignored` 11 passed;
`horizon-command` 37; `horizon-world` 385 + integration; `horizon-mods --features yak` 96 + 43; `horizon-mod-api-gen` 46; `yoake-wasm` all binaries; `yoake-slice` 32 + 1; `horizon-mobmodel` all (incl. its goldens);
hook crates and `yoake-bindgen` all pass; `cargo check -p horizon-client --features yoake2-slice` ok; `regen.sh --check` ok.

**Golden vertex hashes vs the laptop.** The hashes of `golden_vertex_hashes_of_every_animation` were produced on the laptop; the same test passes here on a different machine (Windows host), so the vertices of the six animations at frame 10 of 30 are
bit-for-bit (to the test's 1e-4 quantisation) the same on both. They are computed by the render code's own vertex builder from the fixture, not read back from the running game; no hook exists to hash what the real game draws, so
"the real game produces these vertices" is only established indirectly by the screenshots below (the fox is drawn in the pose the animation names).

## 4. In-game checks (FR-F4M-6 "In-game checks for Yoake")

New: `/horizon custommobs trigger <targets> <name>` (op 2, `horizon_commands`, next to `custommobs stats`; needs `--pregen`, which turns Horizon's own commands on): fires an animation trigger as the server's
runtime would (`anim_sync::request_trigger`); parse test `horizon_custommobs_trigger_takes_targets_and_a_name_for_game_masters`. New log lines (info): `Horizon custom-mob server frame` (server `hit` frame), `events` (one per tick
with client events), `sound`, `particle`, `event` (the `hit` custom event). New repeatable harness `tools/custom_mob_render_e2e.py` (real games, `--offscreen --no-focus --no-pause`, host + guest, scratch copy of the
fixture patched with a bite sound and a flame particle at the pounce's `hit` keyframe and a `melee_attack` goal; PYTHONUTF8=1; refuses to start while another `horizon-client` runs). Last run on the final tree, dev binaries:

| Check | Result | Evidence |
|---|---|---|
| 1 offscreen fox render | pass | `docs/yoake2/reports/img/f4m6-idle-day.png`: upright, textured, standing on its feet, head to the right (front of the model = the mob's front); not mirrored |
| 4 glow mask | pass (judged by eye) | `f4m6-idle-night.png`: at midnight the fox is dark and one small spot (eye) stays lit; 1.26 M pixels differ from the daylight shot (whole scene). Not compared with the automatic `_glowmask` pick-up |
| 3 pounce, client events | pass | `f4m6-pounce.png` (6 ticks after the trigger: body shifted, flame particle at the mouth). The server logs the `hit` frame once (tick 1952); the host's client logs the sound, the particle and the `hit` event in tick 1952, once each; the guest's client (through `horizon:mob_sync`) logs the same, once each, in tick 1952 |
| 5 LOD | pass | `f4m6-lod-30.png`, `f4m6-lod-70.png`: the fox is still drawn at 30 and 70 blocks (pixels differ from the empty scene). The 24/48/64 step behaviour itself was not checked, and the 2x2 cull box was not turned at |
| 6 name tag | **not covered** | `data merge entity ... CustomName` is refused for a custom mob ("selector yields multiple"), so no tag was shown (`f4m6-nametag.png` shows the fox without one). A custom mob cannot be named by `summon` NBT either (the entity type is not found with an NBT argument) |
| 7 death | pass, timing varies | `/damage` 100: the body is gone 4 to 20 ticks after the damage in four runs (`f4m6-death.png`: the removal smoke), not the fixture clip's 12 ticks consistently; the sampling is 2 ticks plus command latency, so only "removed within 20 ticks" is established |
| 3 server hit (damage) | **fails / open** | with the fox in reach (distance 1.0 to 0.6, difficulty normal, survival host): the server delivers the `hit` frame (once per strike) but neither the host (20 -> 20) nor a chicken (4 -> 4 or removed) loses health. The same goal's unit tests pass; the integration looks broken between `deliver_frame`/`land` and the game (`MobEvent::AttackPlayer` -> `mob_hits_player`), or the patched fixture's `melee_attack` is incomplete. Not resolved here; for F4M-4a/5b to look at |
| 8 held item, 9 hide a bone | not run | |

Where the harness cannot do what was asked:
* **Real script goal / `fox_ai` guest:** not loaded. The guest has no `yak.toml` and no packaging slot for a `[wasm]` section in the fixture (F4M-10 report, open question 1), so a dev run cannot install it; the pounce was fired
  with the dev trigger command and the strike was tried with a data-driven `melee_attack` goal. The guest itself is covered by `wasm_mods::mob_runtime` (4 passed, per the F4M-10 report).
* **Stock client:** none on this host.

mob-sync e2e on the final tree (`tools/mob_sync_e2e.py`): quiet, fox, store (1 snapshot, 13 deltas, 0 dropped, 0 incompatible), late (first line a snapshot): all pass.

## Screenshots

`docs/yoake2/reports/img/f4m6-idle-day.png`, `f4m6-idle-night.png`, `f4m6-pounce.png`, `f4m6-lod-30.png`, `f4m6-lod-70.png`, `f4m6-nametag.png`, `f4m6-death.png`.

## Disk

13 GB free at the start and the end; the run waited (the script gates every step on 12 GB) three times while other builds ran. Freed: `target/doc`, `target/release`, `target/tmp` (~0.2 GB).
