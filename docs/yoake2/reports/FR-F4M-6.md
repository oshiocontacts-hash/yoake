# FR-F4M-6: client renderer for fully custom mobs

Branch `yoake-free/f4m-6` (from `yoake-free/f4m-4c` @6b106b7e5). Not pushed. No bindgen-annotated struct changed (the
generated glue is untouched); `Game` got one field and `CustomType` one field, neither is bindgen annotated.

## Pipeline (one custom mob, one frame)

```
creatures.rs mob_geometry loop
  cull: the type's `cull` box (else the hitbox) x model scale, +0.5 margin      [TypeRender::cull_box]
  native_mob_submit (mod_mob_geometry) still first: a mod that draws it wins
  `custom` = mob.kind.custom + .render                                           (the one test in the hot loop)
  Game::custom_mob_geometry                                       game/custom_mob_draw.rs
    LOD from the camera distance                                  [TypeRender::lod_mode]
    MobRender::update  (mobs/custom/render_state.rs)
       1 clocks: RemoteCustomMob layers  -> Controller::apply_anchor (fade by this client's frame time)
       2 triggers -> TriggerPlay { elapsed = AnimClock at tick+partial }
       3 Rig::evaluate -> Pose
       4 ProceduralSet::evaluate (look_at view/target, springs in the mob's full transform x16, IK...)
       5 script BoneWrite hook point (MobRender::writes, F4M-10 fills it) + BoneVisibility
       6 bone_matrices_into
    TypeRender::draw: body (cutout), opaque layers (cutout, emissive), automatic `<texture>_glowmask`,
                      then translucent / additive layers; quads of every drawn bone x its matrix
    held items (item_slots mainhand / offhand / head at their locators) through resolve_item_in_hand + submit_item_world
  mod_mob_layer_geometry (native_mob_layer) still runs after it
  outline / equipment outlines / entity culler: the same epilogue as a mod-drawn mob
shadow:  shadows.rs reads model.shadow {radius x model_scale, strength}
name tag: unchanged (the hitbox height + 0.5 the engine uses for every mob)
```

Per game tick (`tick_custom_mob_events`, after `tick_particles`, single player/host and remote client): the local mobs'
mirrors are synced, every custom mob within 64 blocks of the player polls its `RemoteCustomMob::poll_events`, and the
client events go to the engine's sound and particle paths at the locator position (`event_position` x 1/16 through the
mob's transform). Timeline names are logged at trace level (the `mob-anim-event` script hook is F4M-10).

## Files

| File | What |
|---|---|
| `crates/horizon-client/src/mobs/custom/render_state.rs` (new) | `TypeRender` (per type: baked quads, locators, layers, LOD, cull, shadow, procedural set, embedded textures), `MobRender` (per mob), `LocalMirror` (the integrated server's own mobs through the same store), `CustomRender` (container), vertex output, transform helpers, `route_events`, tests |
| `crates/horizon-client/src/game/custom_mob_draw.rs` (new) | game glue: pose context, draw, items, per-tick events, the audio/particle sink |
| `mobs/custom/mod.rs` | `CustomType.render: Option<TypeRender>` built in `CustomType::new` next to `anim` |
| `mobs/custom/build.rs` | `sound_event()`; `vanilla_sound` keeps a namespaced id (see "Sound ids") |
| `game/creatures.rs` | the cull box, the `custom` branch after the `modded` branch, `begin_frame`, `beam_target` made `pub(super)`, `sound_event` at two playing sites |
| `game.rs`, `game/remote.rs` | `mod custom_mob_draw`, the `custom_mob_render` field, `tick_custom_mob_events` after both `tick_particles` calls, `sound_event` at the hurt/death site, embedded textures to the atlas |
| `game/packs.rs` | `embedded_textures()` into the atlas build (the texture list) |
| `crates/horizon-client-render/src/hud.rs` | `HudAtlas::build_over_with_images` (extra images; the existing entry delegates with none), `pack` takes them |
| `shadows.rs` | `model.shadow` radius and strength |

## Conventions and decisions

* **Model to world** (decisions given): model space is Blockbench space (pixels, -Z north). `world = T(feet) * S(scale) * Ry(-yaw) * Rz(death roll) * Ry(180) * S(1/16)`
  (`world_from_model`). A mob at yaw 0 faces +Z, so the model's front (-Z) lands there; yaw 90 faces -X. The hitbox offset is the box bottom centre
  (the feet). `scale` = the type's `model.scale` x the engine's `model_scale()` (so the baby scale applies).
* **Death**: the engine's tilt (`sqrt((t - 1 + partial) / 20 * 1.6)` of 90 degrees about the mob's own Z) and the red overlay; the `death` animation that holds the
  entity (design 4) is not implemented: the engine removes the mob when its `death_time` ends. See open questions.
* **Hurt tint**: `overlay::living(0, hurt_time > 0 || death_time > 0)` on the body and cutout/translucent layers; emissive layers never take it.
* **Layers**: `cutout` (as the body), `emissive` (full bright, shaded by its normal, overlay 0: the OptiFine emissive overlay path), `translucent`
  (`OIT_ENTITY`, drawn after everything opaque), `additive` (full bright, unshaded, `OIT_ENTITY`, the HDR eyes tier). `bones` = the named bones and their
  descendants. `visible` is true when the expression is non-zero. `tint` is the expression's value as `0xRRGGBB` (exact in f32): **open question**.
  A `<default texture>_glowmask` sprite in the atlas is an emissive layer of the whole model unless a layer already names it.
* **Quads**: the baked triangle lists are paired back into quads at type load (a cube face is the pair `(a,d,c) (a,c,b)`), because the entity pass draws quads;
  an unpaired triangle becomes a quad with a repeated corner. Vertices are two-sided shaded (`normal.w = -127`) with the flat face normal through the
  inverse transpose of the bone's matrix.
* **LOD** as in the module docs: `half_rate` / `quarter_rate` re-evaluate every 2nd / 4th frame (staggered by entity id) and **hold** the last pose in between
  (design says "interpolated"; holding is what is implemented: the mob itself moves smoothly, only its limbs step); `low_bones` skips bones whose name ends in
  `_lod_min` (the design does not say how a bone is tagged: **open question**); `static` = rest pose, never runs the rig; `hidden` draws nothing and does not
  evaluate. No row before the first = full.
* **Local mobs** (single player, LAN host) go through `LocalMirror`: the same `RemoteCustomMobs` store a network client fills, fed from the mob's `MobAnim` (a
  snapshot the first time, then the anchors the sync layer would send and the triggers that started). So the pose, the join/fresh event rule and the events are
  one code path for both. Every tracked value (also `sync: none`) is copied into the mirror: the animations of this process read them all.
* **First application snaps**: a `MobRender`'s first update enters the anchored state without a fade (the state the server shows), so a mob that comes into view later
  shows the pose of one watched all along (tested).
* **View target** is per client: the camera in model space (`model_point_of`); `Target` is the mob's attack target; `Owner` is not filled (open question).
* **Sound ids**: `MobType.sounds` / `MobEvent::Sound` stay `&'static str`. A custom mob's sound of another namespace is now kept interned with its namespace
  (`sample_mobs:entity.fox.hurt`); the three places that play a mob event call `build::sound_event(name)`, which only prefixes a name without `:` with `minecraft:`.
  Vanilla names never contain `:`, so the vanilla path produces the same strings. Animation-event sounds come with full ids from the event table and are played directly.
* **Atlas**: textures under `assets/<ns>/textures/entity/` were already walked into the entity atlas for content namespaces (a file route needs nothing). A bbmodel's
  embedded texture (first texture only, F4M-1 Q4) is added at pack load under the default texture's name (`HudAtlas::build_over_with_images`); a file of that name wins.

## Performance notes

* A world without a custom type: per mob one `Option` test (`mob.kind.custom`), one `borrow_mut` + add per frame (`begin_frame`), one `RwLock` read per game tick
  (`tick_custom_mob_events` returns when the registry is empty), `embedded_textures()` returns an empty `Vec` (no allocation). Nothing else changes: the vanilla
  cull box expression is the same arithmetic, `sound_event` returns the same string.
* Per drawn custom mob: bone matrices are composed once per evaluation; one 3x3 inverse per drawn bone and pass; one matrix-point multiply per vertex. The `Vec<Vertex>`
  is reserved once per mob. `MobRender` owns every buffer (pose, scratch, matrices, trigger list): `update` allocates only when the skeleton or the trigger list grows.
* The measured scenario `custom_mobs_200` is F4M-14's. Not measured here (no GPU, and a laptop under the shared build lock).

## Tests

**STATUS: built and run** (laptop, no game resources, through `cargo-locked.sh`). The WIP commit `48c2b50f0` itself does not compile (one missing
`use crate::mesher::Vertex`); the commits after it do. Fixes made on the first real build: that import; test fixtures (the fox's bones are `tail_1`..`tail_3`, not `tail1`;
the fixture mod's `.bbmodel` is a one-bone stub without a texture, so the bbmodel test serves horizon-mobmodel's full fox bbmodel with its embedded texture; the timeline `hit`
has no locator, so it is placed at the model origin, the feet; the prune test needed >720 frames; the look-at test uses an inline `look_at head` instead of the mobmodel
procedural fixture, whose bones belong to another rig).

**Counts**: `mobs::custom::render_state`: 25 tests (24 + `the_hosts_mirror_hides_owner_values_of_mobs_the_host_does_not_own`), `mobs::custom` + `lan::mob_sync`: 188 passed;
`mobs::custom::build` gained `vanilla_sound_event_ids_are_the_same_strings_as_before` (the `sound_event` / `vanilla_sound` strings equal the old `format!("minecraft:{name}")`).
Full runs after the merge (all ok): `horizon-client-lib --lib` 1855 passed / 48 ignored; `horizon-client --lib` 1228 / 50 ignored; `horizon-client-render --lib` 1455 / 40 ignored;
`horizon-mobmodel` all targets ok; `horizon-mods` 59 ok; `build -p horizon-client --features yoake2-slice` ok.

**Golden vertex hashes**: `GOLDEN` was filled from this code (`F4M6_PRINT=1`), identical over two separate runs, 624 vertices per animation, all six different. They were
**not** checked against a real render or Blockbench: they only pin the pipeline as it is today.

**Merge of `yoake-free/f4m-4c` (040ce9eb0)**: merged cleanly. The integrated host's `LocalMirror::sync` takes the host player's uuid (`level.player_uuid`) and copies each synced value
only when `lan::mob_sync::value_visible_to(mob, slot, viewer)` holds, else the slot's default (what a remote client sees; same rule as `host_value`). The scale needs no change: the render
already reads `mob.model_scale()`, which `custom::attrs` now fills from `effective_scale`.

24 resource-free tests in `render_state.rs` (all on the Cinder Fox fixture, no Minecraft file):

* quads: every baked triangle pair becomes a quad of its bone (ranges contiguous, unit normals).
* golden vertex hashes at frame 10 of 30 of each of the 6 animations (idle, walk, run, sit, pounce, death), all different, same vertex count.
* layers: cutout/emissive split of the fox (counts `cutout = emissive = quads x 4`, glow not drawn twice); the automatic `_glowmask` without a layer row and not without the sprite;
  translucent/additive sort after opaque with the OIT tag, additive unshaded with the eyes tier, a hidden layer draws nothing, a tint expression; the hurt overlay on body not glow.
* LOD: the table (24/48/64 rows), half/quarter rate evaluate every 2nd/4th frame staggered by id (counts 9 and 5 over 16 frames, full 16, hidden/static 0), static = rest pose,
  hidden = no vertices, `low_bones` and script-hidden bones.
* culling: the `cull` box, the hitbox fallback, `y_offset`, model scale; the shadow row.
* locators and transform: the mouth is ahead of the feet at yaw 0, west at yaw 90, scale, death roll, the item frame, the camera in model pixels.
* embedded bbmodel texture goes to the atlas under the default texture's name.
* late-join pose equality (a mob first seen at tick 200 equals one watched since tick 90), the blend fading on client frame time, the look-at constraint following the view, script writes.
* client events: the pounce's sound + particle + custom `hit` at the mouth in tick `start + 8` and not again; a pounce first heard of 0.5 s late replays nothing; server-side events are not routed.
* integrated mobs: a server-ticked fox mirrored by `LocalMirror` fires the sound once in tick 18 (asked in 10); values come through whole; a gone mob is forgotten.
* no work without a custom mob: `CustomRender` stays empty over 1000 frames, no atlas image/allocation, a vanilla mob is never mirrored; stale mob states are dropped after 600 frames.

Not written: a counting-allocator test of the draw path (the lib test binary has no allocator hook; `horizon-mobmodel` already proves its parts allocation free), the GPU/offscreen
render (Yoake), parity golden C (Yoake). No test of `custom_mob_draw.rs` itself (it needs a `Game`).

## In-game checks for Yoake

1. **Offscreen fox render**: spawn `sample_mobs:cinder_fox` (egg or `/summon`), look at it with the camera two blocks away. It must stand facing where it walks (front = -Z of the
   model = the mob's front), not mirrored, the glow mask lit in the dark (a cave), the texture upright. If it is turned half around or mirrored, the half-turn decision is wrong.
2. **Parity golden C**: the vanilla render hashes must be unchanged (nothing in the vanilla path moved; see the performance notes).
3. **Pounce**: make it attack a chicken (`pounce` trigger). In single player the `hit` keyframe lands the damage and the client event (the fixture has only a `timeline` `hit`; add a
   `sound_effects` / `particle_effects` key at 0.35 to see the sound and particle at the mouth) fires in the same tick, once. On a LAN client the same through `horizon:mob_sync`.
4. **Glow mask**: the explicit `glow` layer and (remove the layer row) the automatic `_glowmask` pick-up give the same picture.
5. **LOD**: walk away: 24 blocks half rate (limbs step at 10 Hz of frames), 48 quarter, 64 the rest pose, never invisible; check the cull box (a 2x2 box) by turning the camera at the fox's feet.
6. **Name tag**: `/summon` with `CustomName`: above the hitbox as for vanilla mobs. There is no `name_tag_height` field in the schema (open question).
7. **Death**: kill it: red flash, falls on its side, removed after the engine's 20 ticks.
8. **Held item**: give it an item in the main hand (`item_slots.mainhand` = `mouth`): it sits at the mouth locator in the ground display; check its orientation.
9. **Hide a bone / script**: nothing calls `MobRender::writes` yet.

## Open questions

1. **Goldens** are unverified against a real render (Yoake: in-game check 1 and parity golden C).
2. `tint`: the layer's expression is read as a packed `0xRRGGBB` number. The design only says "tint expression". Alternatives: a 0..1 multiplier or an alpha. Needs a decision.
3. `lod_min` bones: the design says "bones tagged `lod_min`" and the schema has no tag; I used the name suffix `_lod_min`.
4. `half_rate` / `quarter_rate` hold the last pose instead of interpolating (see above). Interpolating poses needs one frame of latency.
5. `death` animation that holds the entity until it ends (design section 4): only the engine's tilt and removal. Needs the engine to keep a dying custom mob alive.
6. `look_at: owner` is not filled (`PoseCtx::owner` exists); `target` and `view` are.
7. There is no `name_tag_height` in the schema; the tag hangs at the hitbox height + 0.5 like any mob.
8. A mod particle id (not a vanilla `ParticleType`) is logged and skipped; sounds of any namespace play.
9. Events are polled once per game tick (partial 0) and placed with the matrices of the last drawn frame (the rest pose for a mob never drawn). The event time inside the tick is not used.
10. `variable.*` are not synced: the client's copy changes only through the controllers' `on_enter` it replays; an expression that depends on a script-written variable differs from the server's.
11. Held item: the ground display at the locator, oriented by the bone; a type may want per-slot offsets/rotation (a `item_slots` object form).
12. Emissive layers use the OptiFine emissive-overlay path (full bright, shaded normal). Mobs with HDR output do not get a tier for them (only `additive` does).
13. The startup atlas (`UiResources::load`) also receives `embedded_textures()`, but custom types are normally registered only after it; a pack reload picks them up. Embedded textures are not checked against the atlas size limit.
14. `FRESH_TRIGGER_SECONDS` / local mirror: the mirror sends a trigger as a delta message in the tick it first sees it, so a trigger asked for in tick N plays its 0 s keyframes in N+1.
