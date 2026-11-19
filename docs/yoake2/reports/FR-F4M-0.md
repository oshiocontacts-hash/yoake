# FR-F4M-0: foundations, schema and parity goldens

Branch `yoake-free/f4m-0` (worktree `NestgateHorizon/horizon-f4m0`), from `yoake-free/f4-mob-design`. Not merged, not pushed.

## What landed

* `Species::Custom` (last variant), neutral row `species::CUSTOM`, returned by `species::def` (without it an unknown species fell back to `ALL[0]`, the tadpole).
* `MobType.custom: Option<Arc<CustomType>>`, `None` in all 15 literals (13 added `custom: None`, the template copy keeps `None`).
* `mobs/custom/` (`mod.rs`, `registry.rs`): `CustomType` and an empty process-wide `registry()`. Nothing fills it (F4M-3).
* `horizon-mods/src/custom_mob.rs`: serde schema for `"template": "yoake:mob"` (size, hitboxes, attributes incl. custom, synced values, ai with data and script goals, model, spawn, despawn, spawn egg, tameable, breedable, seats, proxy), `validate`, `referenced_files`, `limits` (bones 256, cubes 4096, animations 128, synced 32 and 256 bytes, expression 256 instructions, plus the per-list caps), and the small `ModelInfo` trait with `validate_model`. `BoneIndex` is a plain `u16` there; `Pose` was not needed. `horizon-mobmodel` was NOT created (name reserved in the design doc).
* `content.rs`: a `yoake:mob` entity type is parsed by `custom_mob`, its model/animation/controller/texture files must exist in the mod, result in `Content.custom_mobs` (no engine effect yet). Old templates untouched.
* Reserved ids: `hook-ledger/meta.json` `next_id` 710 -> 1710 (ledger ids 710..=1709 free for custom mob hooks); `ids.lock.json` got an `unbound` sentinel `custom_mobs::__reserved_range_end` at id 2514 (generator assigns max+1, so 1515..=2514 are never handed out). A test pins both.
* Fixture `tools/fixtures/cinder_fox/`: manifest, entity_type JSON, a 14-bone geo, `.bbmodel` stub, animations, controllers, two generated PNGs, NOTICE. All written from scratch; no WASM (data-only).
* Generated guest glue patched by hand for the new variant (tag 90): `yoake_glue/mobs.rs` and `yoake-guest-sdk/src/stubs/mobs.rs`.

## Goldens (captured before any engine change, commit 53691d4d4)

Test `crates/horizon-client/tests/f4m_parity.rs` (registered in `horizon-client-lib/Cargo.toml`), `#[ignore]`, needs `HORIZON_TEST_RESOURCES=...\horizon\.local\horizon-client\26.3`; `F4M_PARITY_WRITE=1` rewrites. Hash files in `tools/fixtures/mob_parity/` (no game data). Run twice before the change: identical.

* A (saves): `mob_to_nbt` of a fresh mob of every one of the 90 vanilla types, and after `mob_from_nbt` + save again. sha256 of file `3ef5061c...1b02b`.
* B (network): real hub + protocol bot, once with brand `vanilla`, once `horizon`: hash of all configuration packets (registry sync), per-entity hash of `AddEntity` for 90 entities, and the custom-payload channels (vanilla: `minecraft:brand`; horizon adds `horizon:mod_net`, `horizon:quiet_commands`; no play-state channels). File `54323...cd2e8`.
* C (render): per type the vertices of `mobs::render::render` (with layers) in two poses (90 distinct rest hashes), type facts hash, content identity `244d1bae...dd15` and `registries.json` sha256 `51f62d56...ecdac6` (zero mods). File `97da1089...94829`.

After all changes the three tests pass unchanged (3 passed).

## Species::Custom fallout

Compiling after adding the variant gave 0 errors in `horizon-client(-lib)`: every `match` on `Species` already has a wildcard arm (about 2450 uses). Patterns used:
1. `species::def`: early return of the neutral row for `Custom` (1 place).
2. Negative lists: `Def::flags` `animal` gets `S::Custom` in its exclusion list so a custom mob is not an animal (1 place, marked `// F4M: custom species`).
3. Generated glue/guest stubs: exhaustive enum encode/decode got tag 90 (3 enum edits in 2 files; regenerate with `yoake-bindgen/regen.sh` when convenient).
4. `custom: None` in 13 `MobType` literals (explosion, lightning, names, perspective, native_damage tests, native_entity_query storage test, native_mob_draw, camel, rabbit, render test, villager, mobs.rs x2 tests/builder).
All other `_` arms give `Custom` the generic "other mob" behaviour; this is not audited site by site (see Needs checking). Test `the_custom_species_row_is_neutral` pins the table rows and flags.

## Tests

* `cargo test -p horizon-mods`: 53 passed (incl. 9 new custom_mob tests: fixture accepted, limits, malformed rejection, model checks, missing file, reserved ids).
* `cargo test -p horizon-client-lib --lib`: 1640 passed, 1 failed, 48 ignored on the full run; the one failure was my neutrality test (a bad `ptr::eq` on a const slice), fixed and rerun: it passes. Other lib tests were not rerun after that test-only fix.
* `f4m_parity -- --ignored`: 3 passed.
* clippy on `horizon-mods --all-targets`: no warnings in touched files.

## Needs checking

* The bin tests of `horizon-client` and `--features yoake2-slice` were not built: the shared disk filled up twice (linker "No space left on device"). The glue patch is mechanical but unverified by a compile; `horizon-client --all-targets` check (default features) was clean.
* No offscreen game run: the goldens use the same engine paths in-process (hub, entity NBT, mob render). The fast-profile client was not built because of disk. A world-save golden (region bytes after 2000 ticks) is not captured; A covers entity NBT only, and `SetEntityData` hashes are empty because the test entities carry no meta.
* Wildcard arms: custom mobs get "generic mob" behaviour from `_` arms; the real audit belongs to F4M-3.
* Reserved binding ids are nominal (generator never reuses the gap); hook ids are reserved by `next_id`, assigned by hand.
* `Content.custom_mobs` is validated but ignored by the engine; a mod shipping a custom mob loads without the mob existing until F4M-3.
