# FR-REM16b: the gaps REM-16 left (fluids, tags, dedicated command tree, reload)

Branch `yoake-free/rem16b` (from `yoake-free/merge` f7516323c). Three Kimi lanes (code, no shell), merged here; builds,
tests and fixes by Claude on LENOVO. Each lane's full report: `FR-REM16b-FLUID.md`, `FR-REM16b-TAGS.md`,
`FR-REM16b-RULES.md` (their "not run" notes predate the runs below).

## Against the request

| Item | State |
| --- | --- |
| 1. Mod fluids flow, buckets | Done on vanilla's `FlowingFluid` path (`horizon-world/src/level/fluids.rs`): the fluid's `viscosity` drives drop-off and slope search, its `tick_rate` the scheduling; `motion: none` is inert. New sources only with water motion and `water_source_conversion`; lava motion never makes sources. A mod fluid never counts as vanilla water/lava (`FluidKind::same_family`); water/lava reactions (stone, obsidian, cobble, basalt) stay vanilla-only. All 16 `level` states of a data fluid's blocks already exist (content-root state cloning) and bind (`FluidBlockBindings::with_custom`). Buckets: an empty bucket on a mod source gives the fluid's `bucket` and removes the source; the bucket places it through the shared `bucket_empty` (survival gets the empty bucket back); native bucket-empty hooks get the mod fluid's id; dispensers dispatch mod buckets. |
| 2. Tags | Done. Mod `data/<ns>/tags/damage_type/*.json` go through the content root's `TagMerge` (load order, `replace`, `#tag`, `required: false`) into `damage-tags.json`, which `damage_source::Catalog` reads (so `#minecraft:bypasses_armor`, `is_fire`, ... and mod tags apply). `Enchantments::load_with` uses the shared multi-namespace tag loader: `exclusive_set: "#ruby:gems"`, `in_enchanting_table`, `treasure`, etc. see mod tags and mod entries. The configuration-phase tag sync sends merged tags of every content namespace. No mods: bundle and packets unchanged. |
| 3. Dedicated command tree | Done. `lan/commands.rs` adds `/gamerule <mod>:<rule> <value>` under vanilla's `gamerule` node of the `commands.json` report: `brigadier:bool`, or `brigadier:integer` with the rule's min/max, with the vanilla rule nodes' permission. No mod rules: identical tree. |
| 4. Reload | Rules: done. `register_mod_rules` replaces the registry on every content-root build and on `/reload`; added rules appear, defaults/bounds/category change, ints clamp to new bounds, removed rules are hidden but keep their saved value (restored if re-added); command trees are resent (also through the integrated-server handoff). Fluids: partly, on purpose: a reload updates `motion`, `tick_rate` and `viscosity` of existing fluids in place; **adding/removing** a fluid or changing its blocks/bucket/textures still warns to restart, because `Resources` (block facts, collision, behaviours, indexed by the startup block registry) is loaded once and a new fluid's block states would be unknown to gameplay. |

## Fixes by Claude after the lanes

* Tags: the armor test compared `f32` exactly (3.9999998 vs 4.0); now within 1e-4.
* Fluids: the all-levels binding test built falling states with amount 8 for block levels 9..15; the engine (like
  vanilla `LiquidBlock`) maps falling amount `a` to level `16 - a`, so the test now does.
* Rules: a compile error (`clamp` on `&i32`). The mod-rule registry became process-wide mutable (it was a
  `OnceLock`), so the new command-tree test raced `save::tests::level_save_keeps_changed_rules` in the same test
  binary: the command-tree test now uses a thread-local rule list (`cfg(test)`), and the core `gamerules` tests take
  a shared lock and leave the registry empty.
* `cargo fmt` was not applied repo-wide (the repo is not rustfmt-clean and it reorders `game.rs`); lane changes keep
  the surrounding style.

## Tests (LENOVO, merged branch)

horizon-world --lib 402 passed (incl. the existing `water_spreads_on_a_floor`, `water_falls_and_makes_sources`,
`bucket_source_removal_drains_spread_water`, `dispenser_buckets_pick_up_and_empty` and the new flow/bucket/binding
tests), horizon-items 66, horizon-mods 74, horizon-command 38, horizon-client-lib 1969, horizon-client --lib
--bins --tests (1296 and the integration tests), horizon-server builds. horizon-client-core: 61 passed, 1 failed,
`lang::tests::every_horizon_key_the_code_names_is_in_en_us` (`horizon.custommobs.stats` named in
`game/commands/horizon_cmd.rs` is missing from `en_us.json`): already so on the base, no file of it changed here.
Not run here (need the game): `tools/content_kinds_e2e.py` and a dedicated-server client.

## Not done / left

* Fluids: adding or removing fluid families at reload (above). Custom fluids do not waterlog blocks (vanilla water
  only). Not played in a game here: flow, bucket fill/place and a dedicated client's `/gamerule ruby:` completion
  need the e2e on GALLERIA (`tools/content_kinds_e2e.py` can grow these checks).
* Rules: changing a rule's type uses the new default (no value migration); an open rules screen refreshes when
  reopened; replaced rule metadata stays leaked (small).
