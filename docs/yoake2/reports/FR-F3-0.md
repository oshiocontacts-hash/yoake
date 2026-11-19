# FR-F3-0: `Dim` becomes an interned handle

## Design choice
`enum Dim` (crates/horizon-client/src/game.rs) is now `struct Dim(u16)` (Copy, Eq, Hash, Debug) with a process-wide
interner (`dim_interner`: `RwLock` over a `Vec<&'static str>` plus a `HashMap`, ids leaked on purpose, bounded by the
distinct ids packs and mods declare). `minecraft:overworld`, `minecraft:the_nether`, `minecraft:the_end` are
pre-registered at 0/1/2.

The vanilla names stay as associated consts spelled `Dim::Overworld`, `Dim::Nether`, `Dim::End` (not
`Dim::OVERWORLD`), with `#[allow(non_upper_case_globals)]`. Reason: ~230 uses and every `match` pattern keep compiling
unchanged, because the derive of `PartialEq, Eq` makes the consts usable as patterns. Churn: 0 renamed uses. F3-4 can
rename later if wanted.

API: `Dim::id()`, `Dim::lookup(id) -> Option<Dim>`, `Dim::intern(id) -> Dim`, and `Dim::lookup_or_overworld(id)`
(the old `from_id` behaviour under an honest name). `from_id` no longer exists, so no new caller can use it by accident.

## Call-site groups
1. All 28 former `Dim::from_id` sites now call `Dim::lookup_or_overworld` and carry the comment
   `// F3-0: unknown level id falls back to the Overworld, as before; F3-4/F3-3 may flip it.` The decision at every
   site is "keep Overworld fallback" (no site was changed in behaviour):
   commands/execute.rs (2: 643, 649), commands/players.rs (200), commands/test_runs.rs (216), commands.rs (485),
   command_harness.rs (73, 267), integrated.rs (1400, 1457), lan.rs (1420, 1474, 1492, 2000, 2636),
   lan_actions.rs (2923, 2927, 3714), lan_far.rs (44), native_mannequin.rs (187), native_player_ops.rs (522),
   nether_portal.rs (246), remote.rs (2276, 2303, 3350), respawn_anchor.rs (48), server_travel.rs (868),
   world_loading.rs (225), game.rs (12625). Line numbers are those before the change.
2. Four string-to-Dim matches replaced by `Dim::lookup(..)` with the same error/None result for unknown ids:
   native_hooks.rs (`native_level_random`), native_player_ops.rs (`dimension`), native_server_physics.rs
   (`native_physics_world`), native_world_read.rs. Today identical (only vanilla ids are interned); later they accept
   any registered level, which is the intent.
3. Exhaustive `match` over Dim that needed a `_` arm, each marked `// F3-4: dimension-type query` (unknown level takes
   the Overworld-like branch): `Dim::shape` (game.rs), commands.rs `loaded_dims` sort key (unknown sorts after the
   three, stable), creatures.rs cem frame dimension, discord.rs, end.rs `dim_environment`, item_draw.rs `item_level`,
   shader_packs.rs, game.rs level-dimension for the tick rules and the shadow ambient light. Other uses (`==`,
   `if`, `matches!`) compile unchanged and were not touched.
4. New tests: crates/horizon-client/src/game/dim_tests.rs.

Not touched: `crates/horizon-shaders/src/textures.rs` (`Dim` there is texture dimensionality), `discord_presence::Dimension`,
`spawn_rules::Dimension`, `horizon-world::Dimension`.

## Behaviour
No change with vanilla data: ids, indices, saves (`id()` strings), packets and rendering take the same values.

## Tests
See the end of this file (filled in after the runs).

## Needs checking
* Only `horizon-client` / `horizon-client-lib` were built and tested here; golden tests A/B and the parity scripts from the
  design were not run (they need the prepared game data / launched game).
* `Dim::id()` panics on an index not produced by the interner; impossible via the public constructors, but F3-3 must
  never build `Dim(n)` by hand.
* The interner never shrinks (leaked ids). Fine for packs/mods, but a runtime `LevelManager` creating unbounded
  distinct ids (F3-6) must reuse ids or quota them.
* `Dim::intern` currently has no non-test caller (marked `allow(dead_code)`).
