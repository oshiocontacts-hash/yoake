# FR-MERGE2: F3-3, F4M-0, F4M-9 and F4M-3 into `yoake-free/merge`

Base `d84aea8e3` (MERGE1). Merged in this order, no conflicts in any step, no push:

1. `yoake-free/f3-3` (6ab055573): dimension registry. "Merge FR-f3-3".
2. `yoake-free/f4m-0` (538be16ce): custom-mob engine foundations. "Merge FR-f4m-0".
3. `vps/yoake-free/f4m-9` (7719be46a): pure `horizon-mobmodel` crate (contains f4m-1, f4m-2, f4m-fox, f4m-8). "Merge FR-F4M-9".
4. `vps/yoake-free/f4m-3` (174c502f8): custom-mob registry/build/attrs/hitbox/spawn/persist, NBT, natural spawn, /summon,
   mod_content, spawn eggs, peaceful rule. It contains f4m-0 and f4m-9 at the merged base, so it was merged on top of them
   (f4m-0 + f4m-9 were merged first, as briefed; f4m-3 added afterwards, no conflicts).

## Generated files, catalog

None of the four branches touches `tools/native-hook-fragments`, `abi-subset-*.json`, the WIT or the adapters; the only generated
document they change is `docs/yoake2/bindgen/coverage.md` (by f4m-3, as committed). `horizon-mod-api-gen` (46 tests, WIT/adapter
match, `abi_1_0_is_additive`) passes unchanged, so no regeneration was needed. Catalog versions stay: client 1.5.0, worker WIT 1.6.0.
F4M reserved hook ids 710..1710 and binding ids (`reserved_ids`) do not collide with the catalog ids 201-226.

## Tests (single `target/`, CARGO_INCREMENTAL=0, CARGO_PROFILE_DEV_DEBUG=0)

| Suite | Result |
|---|---|
| `horizon-mobmodel` | 60+13+18+1+1+4+6+25+17+37+1 passed |
| `clippy -p horizon-mobmodel --all-targets` | 1 warning only: `thread_local` initializer can be `const` in `tests/procedural_alloc.rs` (a test file; not touched by me) |
| `horizon-mods --features yak` | 89 + 42 passed |
| `horizon-mod-api-gen` | 46 passed |
| `yoake-wasm` (all binaries) | all pass (67, 6, 2, 11, 5, 1, 9, 7, 16, 2, 3, 5) |
| `horizon-client-lib --lib` | 1677 passed, 48 ignored |
| `horizon-client --lib` (full) | 1222 passed, 41 ignored |
| `cargo check -p horizon-client --features yoake2-slice` | ok |
| `f4m_parity -- --ignored` (HORIZON_TEST_RESOURCES=`...\horizon\.local\horizon-client\26.3`) | 3 passed: `golden_a_saves`, `golden_b_network`, `golden_c_render_types_and_identity` |
| F3-3 `level_registry_tests -- --ignored` (mypack:moon Game test) | 1 passed |

A first `yoake-wasm` run in the batch failed with "only metadata stub found for gimli" / ICEs: damaged build artifacts caused by
my cancelling the first batch mid-build (to merge f4m-3 first). Rerun cleanly: pass. No flaky tests.

## F4M-3 checks

All three golden tests pass on the merged tree, so there was no need to run them on a detached 174c502f8.

### Spawn egg item definitions (`mod_content.rs` `generated_egg_definition`, `content.rs` generated item)

What is written: item `<id>_spawn_egg` (template `minecraft:pig_spawn_egg`, component `minecraft:item_model` = `"<ns>:<path>_spawn_egg"`),
and `assets/<ns>/items/<path>_spawn_egg.json`:
`{"model": {"type":"minecraft:model","model":"minecraft:item/template_spawn_egg","tints":[{"type":"minecraft:constant","value":PRIMARY},{"type":"minecraft:constant","value":SECONDARY}]}}`.

Real 26.3 resources (`assets/minecraft/items/pig_spawn_egg.json`): `{"model":{"type":"minecraft:model","model":"minecraft:item/pig_spawn_egg"}}`, no tints.
`models/item/pig_spawn_egg.json` is `parent item/generated`, `layer0 = item/pig_spawn_egg`; the textures are per species
(88 `textures/item/*_spawn_egg.png`, already coloured).

Differences:
1. `minecraft:item/template_spawn_egg` does NOT exist in the 26.3 resources (no `models/item/template_spawn_egg.json`, no `textures/item/spawn_egg*.png`,
   no reference to it anywhere in `items/` or `models/`). The comment says "items/<name>.json of 1.21.4 and later"; that template belongs to older versions
   (<= 1.21.x). In 26.3 the generated egg would point at a missing model. A tinted egg needs the mod to ship its own model and texture, or the engine a bundled
   `template_spawn_egg` model (parent `item/generated`, layer0/layer1 spawn_egg + overlay with tintindex 0/1) in the content root.
2. Tint values are written as `primary.0` (u32, 0xRRGGBB, no alpha). Vanilla tint ints are signed ARGB (`-6265536`); whether a constant tint without the alpha byte
   renders opaque depends on the item tint code (not checked); `0xFF000000 | rgb` is the safe form.
3. Vanilla eggs have no tints (the colour is in the texture). The behaviour (tinted template) is a deliberate design, but only valid once item 1 is fixed.
4. Item id and file path are consistent (`items/<path>_spawn_egg.json` in the mod's namespace matches `item_model`). The generated item template (pig egg) is fine
   because `item_model` overrides the model.

### `entity-types.json` rows (`mod_content.rs` `custom_row`)

Vanilla row keys (all 1xxx rows): `width`, `height`, `eye_height`, `category`, `fire_immune`, `attributes`. `custom_row` writes exactly these keys with the
same JSON types (floats, string, bool, object of `namespace:name` -> float). Readers in the engine only use the keys of the object (spawner, `chat.rs`
command suggestions) and `eye_height` as a number (`execute.rs` `type_eye_height`); both are satisfied.
Differences / notes:
* `category`: `CATEGORIES` allows `monster, creature, ambient, water_creature, water_ambient, underground_water_creature, misc`; vanilla also has `axolotls`
  (not offered to custom mobs; harmless).
* `attributes` holds only what the mob file declares (builtin names are prefixed `minecraft:`), while vanilla rows carry the full default set
  (28 keys, e.g. `air_drag_modifier`, `bounciness`, `burning_time`, `camera_distance`, ...). Nothing in the files read by this tree needs the other keys, but a
  consumer that expects the full set (a future attribute-defaults reader) would see them missing.
* `BUILTIN_ATTRIBUTES` contains `water_slowdown`, which is not a 26.3 attribute key (absent from every vanilla row); a mob declaring it writes
  `minecraft:water_slowdown`, which is not a registered vanilla attribute. The other 15 names exist in the file (`flying_speed`, `attack_damage` only on some mobs).
* `eye_height` default `height * 0.85` (vanilla uses 0.85 for most mobs, e.g. pig 0.765 for 0.9). Matches.
