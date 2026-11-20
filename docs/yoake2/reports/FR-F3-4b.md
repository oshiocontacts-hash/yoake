# FR-F3-4b: the remaining per-level behaviour follows the dimension type

Branch `yoake-free/f3-4b` on `yoake-free/f3-4`. No Minecraft files, source or prepared resources committed. Portals and travel
are left to F3-6/F3-7 as asked.

## What landed

### 1. horizon-world: type-derived rules instead of `Dimension` (no ABI change)
`LevelDimension` (`horizon_world::level::Dimension`) is the three-variant enum the mod hooks carry, so it is not extended.
Block behaviour asks a new `LevelAccess::level_rules() -> LevelRules` instead, whose default is `LevelRules::of(dimension())`
(the three vanilla constants), so every existing implementer behaves as before. `LevelRules`: `fast_lava`,
`water_evaporates`, `day_cycle`, `has_skylight`, `sky_end`, `sea_level`, `infiniburn` (+ `LevelRules::of_type(&DimensionType)`).
Flipped from `dimension() == X`: lava spread speed, bucket water evaporation, wet sponge drying, ice melting (both sites),
mud/dripstone water, daylight detectors, creaking and eyeblossom day timeline, the random skeleton trap's day check, sea level
(weather, feature placement), infiniburn tag choice, the End's block-light fire spread. The game's `GameLevel` implements
`level_rules()` from the level's facts. Portal sites (`portal.rs`, `hooks.rs` portal) are untouched.

### 2. Mobs (`MobWorld.nether` / `end`)
`MobWorld.custom: Option<CustomLevel>` (F3-4's `spawn_light` folded into it) carries, for a level past the vanilla three,
spawn light rule, `piglins_zombify`, `snow_golem_melts`, `fast_lava`, `has_skylight`, `day_cycle` and the brightness ambient light.
`nether` and `end` stay as the vanilla keys; helpers `piglins_zombify()`, `snow_golem_melts()`, `fast_lava()`,
`has_skylight()`, `has_day_cycle()` answer `custom` when present and otherwise exactly the old `nether`/`end` tests (the
Nether brightness formula is kept literally for the vanilla path). Flipped: piglin/hoglin zombification, snow golem melt, lava
push scale, `can_see_sky`, brightness, bee/creaking-heart/cat-gift day checks, the drinking trader goal. `allay::dimension`
(which names the dimension for the allay) stays key-based.

### 3. Player lava
`player.fluids.fast_lava` is `level_facts().rules.fast_lava` (client and server), lava spread is `level_rules().fast_lava`;
both follow the type (a pack type with `gameplay/fast_lava`), vanilla Nether still true.

### 4. Day cycle from the type's timelines
`Environment::load_type_timelines(root, packs, type_json, day_cycle)` resolves a type's `timelines` (a `#tag` or a list; tags
nest; the world's unzipped `datapacks/*` folders win over the resources) into one merged timeline; a type that lists none, or
none that can be read, keeps the old `day.json` behaviour. `LevelFacts.day_cycle` is now "overworld sky and (not fixed time, or
has timelines)". Mob checks that are hard-coded day tick ranges (firefly, creaking, monsters burn) still use those ranges and not
the track data.

### 5. Beds, anchors, respawn
`LevelFacts.beds` (`BedRules`): `bed_rule` (can_sleep, can_set_spawn, destroy_on_use), `straw_bed_rule` (can_set_spawn,
destroy_on_leave) and `respawn_anchor_works`, read from the type's attributes (defaults when absent: sleep when dark, set the
spawn, no destroying). The vanilla three are constants (Overworld: sleep when dark, bed sets spawn, straw never sets it and
breaks on leaving; Nether: never sleep, destroy on use, anchors work; End: as the Nether without anchors). Flipped:
`anchor_works`, `beds_destroy_on_use`, `bed_sets_spawn` (bed and straw), straw destroy on leave, and the sleep check. Respawn into
a custom level: the saved respawn id already resolves through the interner for any registered level and `find_respawn_and_use`
now asks the loaded level's rules, so a bed or anchor in a pack level whose rules allow it works, otherwise the usual
"missing respawn block" falls to the Overworld world spawn as in vanilla.

## Parity
* `level_rules_tests::the_vanilla_types_give_the_vanilla_rules` and `the_dimension_constants_are_the_vanilla_rules`
  (`DimensionType::vanilla` read through `of_type` equals the constants; `of(dimension)` values).
* `level_facts::tests::the_vanilla_types_read_as_the_vanilla_levels` (F3-4) still passes; the ignored game tests assert the
  vanilla three keep `LevelFacts::OVERWORLD/NETHER/END` and the anchor/bed answers (`anchor_works`, `beds_destroy_on_use`)
  after visiting a custom level: Overworld false/false, Nether true/true, End false/true.
* Vanilla paths in `MobWorld` helpers return the old expressions (verified by reading; the full mob suites pass).

## Tests
* horizon-world `level_rules_tests` (3).
* horizon-client-render `environment::tests::a_types_own_timelines_drive_its_day` (tag from a pack, list, no timelines, unreadable).
* horizon-client `level_facts::tests`: nether-like type, bed rules from attributes, timelines give a type a day (3 new, 6 total).
* Ignored, real resources (`level_registry_tests -- --ignored`, 3 pass): the F3-3/F3-4 tests, now with `mypack:moon_type` listing
  `#minecraft:in_overworld` (its night sky is dark) and a second pack level `mypack:hell` with a Nether-like type of its own
  (`mypack:hell_type`): the Nether's fast lava, evaporating water, no piglin zombification, snow golem melt, anchors work,
  beds destroy and never sleep, spawn light block limit 15, coordinate scale 8; the vanilla three keep their answers.

Results: `-p horizon-world --lib` 385 passed, 0 failed, 2 ignored; `-p horizon-client --lib` 1213 passed, 0 failed, 43 ignored; `-p horizon-client-lib --lib` 1655 passed, 0 failed, 48 ignored; ignored real-resource `level_registry_tests` 3 passed. No offscreen game smoke run.

## Left / needs checking
* The mod-API `LevelDimension` enum stays three variants: hooks and mod events for a pack level still carry `Overworld`
  (and `dimension_id` for native sounds). A mod-API change is an ABI change; F3-6/F3-8.
* Portals, travel, nether portal linking, `/execute in` scale: F3-6/F3-7.
* `MobWorld.nether`/`end` stay for the checks that name the dimension itself (allay dimension id, spawn rules for the End,
  wandering trader, structure rings); only the behaviour flags moved to the type.
* `LevelRules.sea_level` for a custom level is 32 with a ceiling, else 63 (no type attribute exists for it).
* Bed rule `can_set_spawn: when_dark` is treated as "set".
* Hard-coded day tick windows (firefly sounds, creaking, monsters burn, bees) are not read from the timeline tracks.
