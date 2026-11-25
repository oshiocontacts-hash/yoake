# FR-REM16b lane FLUID: custom fluid flow, buckets and reload

## Implemented

- Custom fluids stay on the existing vanilla `FlowingFluid` simulation path. Water/lava motion uses the declared
  viscosity for drop-off and slope search and the declared tick rate for scheduling. `motion: none` returns before
  level updates or spreading. Custom lava follows lava's spread-delay path; custom water follows water's.
- Source formation requires two neighboring sources and a solid/source base as before. A custom fluid can form a
  source only with water motion and `water_source_conversion`; custom lava never forms sources, including when
  `lava_source_conversion` is on. Vanilla water/lava rules are unchanged.
- `FluidKind::same_family` remains the identity check. Custom source and flowing forms share one family but never
  match vanilla water/lava or another custom family. Custom fluid tag masks remain zero, so water/lava stone,
  cobblestone, obsidian and basalt reactions stay vanilla-only.
- Both water-template custom blocks have all sixteen `level` states through the existing content-root state cloning.
  `FluidBlockBindings` resolves those states, `apply_custom_fluids` replaces their copied vanilla fluid facts, and the
  existing mesher selects the custom still/flowing sprites. No renderer or generated-file change was needed.
- Empty-bucket pickup returns the declared custom bucket and removes only a source. Custom bucket placement goes
  through the shared `bucket_empty`, creates the source, and the existing survival path returns `minecraft:bucket`.
  Remote prediction uses the same world calls. Native bucket-empty hooks now receive the custom fluid id, and custom
  filled buckets join the dispenser's shared filled-bucket dispatch.
- Reload updates `motion`, `tick_rate` and `viscosity` of existing content fluids in place without moving any
  `FluidKind` index. Unsupported changes keep the old bindings and produce a restart warning.

## Files

- `crates/horizon-world/src/level/fluids.rs`: shared flow policy and resource-free tests.
- `crates/horizon-world/src/fluid.rs`: testable custom block bindings and all-level synthetic-registry test.
- `crates/horizon-world/src/fluid/custom.rs`: stable catalog reconciliation, reload result and registry/bucket tests.
- `crates/horizon-items/src/items.rs`: custom bucket-content helper and resource-free test.
- `crates/horizon-client/src/game.rs`: correct custom fluid id in native bucket-empty hooks.
- `crates/horizon-client/src/game/dispenser.rs`: custom filled-bucket dispatch.
- `crates/horizon-client/src/game/mods.rs`: safe reload call and restart warning, including failed content rebuilds.
- `docs/mods.md`, `docs/yoake2/MOD-GUIDE.md` and `docs/yoake2/MOD-GUIDE.ja.md`: behavior and reload contract.

## Tests

No shell was run in this lane, so none of these has been executed here. Added resource-free unit coverage for:

- water-motion spread amounts 7 through 1 followed by empty, and falling level 8;
- one versus two source neighbors, water/lava/none source policy and both source-conversion rules;
- custom tick rate independent of fast-lava settings, inert none motion, vanilla tick-rate constants and family
  non-mixing;
- every custom source/flowing block level from a synthetic registry, with no Minecraft resources;
- bucket item/source-family round-trip and custom `BucketContent`;
- initial content registration beside native fluids, safe physics reloads, stable IDs, and deferred additions,
  removals, identity and presentation edits.

Claude should also run the existing resource-backed `water_spreads_on_a_floor`, `water_falls_and_makes_sources`,
`bucket_source_removal_drains_spread_water` and `dispenser_buckets_pick_up_and_empty` tests to verify vanilla flow and
buckets are unchanged. A real content-mod world check is still needed for end-to-end flow and inventory handoff.

## What is left

- Pack reload intentionally does **not** add or remove fluid families. `Resources` is documented as loaded once and
  never changed (`game.rs`); its `BlockFacts`, collision/motion data and `Behaviours` are indexed by the startup block
  registry. A pack reload builds new `PackResources` but retains that `Resources`. Registering an added family would
  therefore make its new block-state IDs visible to rendering while gameplay still used startup tables. Additions,
  removals and changes outside `motion`/`tick_rate`/`viscosity` retain the old registry and warn to restart.
- Custom fluids cannot waterlog vanilla liquid containers; that remains vanilla-water-only. Custom/vanilla fluid
  reactions are deliberately not generalized.
