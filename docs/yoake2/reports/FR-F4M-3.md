# FR-F4M-3: server registry and `Mob` construction for fully custom mobs

Branch `yoake-free/f4m-3`, from `yoake-free/f4m-0`/`-9`. Not pushed.

## What landed

* `mobs/custom/registry.rs`: process-wide table of `CustomType`s (`get`, `ids`, `replace`). Empty unless a mod ships a `yoake:mob`.
* `mobs/custom/build.rs`: `add` reads the content root's custom mobs file when the mob types load (client, host and dedicated server build the same types; a file that cannot be built is skipped with a warning), `mob_type` makes the engine `MobType`, `register`/`attach_goals` give a `Mob` its state and goals.
* `mobs/custom/attrs.rs`: custom attributes on the engine's own attribute implementation (same modifier order, clamp, permanent/transient rules).
* `mobs/custom/hitbox.rs`: hit and pick parts with damage multipliers, `pick` (ray) and `scale_damage` (`Mob::damage_aimed` hook).
* `mobs/custom/spawn.rs`: spawn rules (placement, light, time, height, dimension, structure, distance, cluster) and despawn rules (`check_despawn`).
* `mobs/custom/persist.rs`: the `yoake` compound (`v`, `synced`, `attributes`); unknown keys and values/attributes the type no longer declares are kept.
* `mod_content.rs` (content plan): network ids among entity types, `entity-types.json` rows, one `add_spawns` biome modifier per spawn rule, the custom mobs file, a spawn egg item definition when the file has `spawn_egg`.
* `horizon-mods`: `CUSTOM_MOBS_FILE`, `spawn_egg_item`, `bone_rest_position`, egg synthesis, 2 new tests.
* `Extra.custom: Option<Box<CustomState>>` (new field), `Game::mob_hit_at` picks parts, `/summon` completions list custom ids.
* Peaceful: a monster-category custom mob is refused on peaceful by spawn eggs, `/summon`, the egg tooltip, spawner blocks and natural spawning (new `spawn_rules::allowed_in_peaceful_type`, `finalize::allowed_in_peaceful` looks the registry up first).

Commits: step 0 `bf198a019`, regen `72b5a5320`, (a) `01f7acc46`, (b) `31053ffee`, (c) `34fafc960`, (d) see the git log. Because `custom/mod.rs` declares all submodules, (a) holds the whole `custom/` directory; (b) and (c) only add callers, so each commit compiles, but only the final tree was tested in full.

## Tests

Final tree, all green:

* `-p horizon-client-lib --lib`: 1663 passed, 0 failed, 48 ignored (the `mobs::custom` subset is 23 on the pre-(d) tree, 24 with the new peaceful test).
* `-p horizon-client --lib`: 1203 passed, 0 failed, 40 ignored.
* `-p horizon-mods`: lib 55 passed (yak 0).
* `-p horizon-mobmodel`: lib 60, plus integration binaries 13+18+1+1+4+6+25+17+37+1, all passed.
* Not run: `f4m_parity` (needs game resources), clippy.

## Wildcard audit (`Species::Custom` against `_` arms)

* `spawn_rules::check_spawn_rules`: `_ => true` for Custom. Natural spawning goes through `custom::spawn::rules_ok` (checks peaceful itself). Spawner blocks (`mobs/spawner.rs`) reached the wildcard: **fixed**, a monster-category custom type is skipped on peaceful there (the `custom_spawn_rules` branch already did). `zombie_reinforcements` only passes zombies; `raids`, `patrol`, `trial_spawner` pass fixed vanilla species: no change.
* `allowed_in_peaceful(Species::Custom)` stays true (the species has no category). **Fixed** at every place that has the type: spawn egg (`creatures.rs`), egg tooltip (`stations.rs`) use `allowed_in_peaceful_type(&MobType)`; `/summon` and `item_tooltip` use `finalize::allowed_in_peaceful(id)`, which consults the registry; despawn already handled by `custom::spawn::check_despawn`. Test `registry::tests::peaceful_follows_the_category`.
* `ai::is_monster` / `drops::is_monster` (`instanceof Monster`) are false for Custom. They take a `Species`; ~10 callers (xp 5, loot flag, swim sound, rabbit/panda fear, landing, living) pass `m.species()`, so per-category behaviour needs a `Mob`-level helper and touches vanilla call sites. **Not changed**; custom mobs are not monsters for xp, sounds and fear lists. F4M follow-up if wanted.
* `family(Custom)` is Generic (no undead/arthropod); `voice_sounds` falls to the kind's sounds. Mob sound events are `&'static str` with `minecraft:` prepended, so a non-`minecraft` sound id from a mod is silent. Limitation for F4M-6/F4M-9, not redesigned.
* Unknown-type preservation: an id with no type goes through `Game::load_other` and the whole compound incl. `yoake` is written back. `persist` tests cover unknown keys inside the compound; a `Game`-level test was not feasible without a game harness.

## Decisions

* Hitbox part `offset` = bottom centre of the box.
* Model-to-mob transform: half turn about Y (Blockbench front is north, the mob faces south at yaw 0). F4M-6 must agree.
* A biome entry is matched to its spawn rule by `(weight, min, max)`; rules sharing them are or-ed.
* `placement: custom` never spawns until the script hook exists.
* Structure filters use `structures_at(...).in_bounds`.
* Light/time defaults by category (monsters dark, creatures bright); "day" = `sky_darken < 4`.

## Needs checking

* Yoake: run the parity goldens (`f4m_parity`, A saves, B network, C render); the network part is affected by entity type ids/rows only when a custom mob exists, nothing changes without one. This laptop has no game resources.
* `Extra` got a field (`custom`): `crates/yoake-bindgen/regen.sh --check` should be run by the coordinator (not run here).
* The spawn egg item definition (tinted `item_model` of 1.21.4+) is unverified for 26.3.
* `entity-types.json` fields for a custom row are inferred from the vanilla rows.
* No in-game run.

## Open questions

* Should `is_monster` follow the category (needs a Mob-level helper)?
* Mod sound ids: `&'static str` events cannot carry a namespace.
