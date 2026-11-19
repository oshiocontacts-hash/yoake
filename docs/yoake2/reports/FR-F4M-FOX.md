# FR-F4M-FOX: the Cinder Fox fixtures and golden pose tables (`horizon-mobmodel`)

Branch `yoake-free/f4m-fox` (F4M-2 merged with F4M-1). Test and fixture work only: **no library code was changed.**

## Files

All under `crates/horizon-mobmodel/tests/fixtures/cinder_fox/` unless noted.

* `gen_cinder_fox.py`: the generator (deterministic, run twice gives identical bytes; one description, all outputs derived).
* `cinder_fox.bbmodel` and `cinder_fox.geo.json`: the same model. 14 bones (the 13 of the design plus `jaw`; the design list
  adds up to 13 although it says 14), 14 cubes, locator `mouth` on `head`, rest rotations on ears and tail bones, one rotated cube
  (`tuft`), box UV, mirrored box UV, per-face UV with a UV rotation (`snout`, tail tip). Texture 64x64 embedded in the bbmodel.
* `cinder_fox.png`, `cinder_fox_glowmask.png`: generated pixels; the mask has 2 eye texels and the six 3x3 faces of the tail tip.
* `cinder_fox.animation.json` (Bedrock): `idle` (loop 3 s), `walk` (loop 0.8 s, clock speed from ground speed through
  `anim_time_update`), `run` (loop 0.5 s), `sit` (hold, 0.6 s), `pounce` (once, 0.8 s; timeline `hit`, sound
  `cinder_fox_pounce`, particle `cinder_fox_embers` at locator `mouth`, all at 0.35 s), `death` (hold, 1.2 s). Uses constant
  vectors, keyed channels, `pre`/`post` (a jump in `pounce.head`), all four `lerp_mode`s (linear, catmullrom, step, bezier with
  handles), eight easing names (cubic, sine, quad, back with `easingArgs`, elastic with `easingArgs`, bounce) and expressions
  with `query.anim_time`, `query.ground_speed`, `math.sin/cos/abs/clamp/lerp`.
* `cinder_fox.json`: controller. `locomotion` idle -> walk (`q.ground_speed > 0.05`) -> run (`> 0.2`) and back with hysteresis,
  `run` plays at speed `clamp(q.ground_speed * 3, 0.8, 2)`; `action` (mode `add`, trigger `pounce`); `pose` (`stand` -> `sit` while
  `q.synced('state') == 1`).
* `golden_poses.txt`: 6 animations x 30 frames x 14 bones, 2520 lines (285 KB), 12 numbers per line (the three axis columns and the
  translation of the model-space bone matrix, 5 decimals; the last row is asserted to be 0 0 0 1).
* `README.md`: what the fox is and how to regenerate.
* `crates/horizon-mobmodel/tests/cinder_fox.rs`: 13 tests.

## Tests (`cargo test -p horizon-mobmodel`: 115 pass; new: 13; clippy `--all-targets` clean)

* (a) `both_formats_load_with_the_same_skeleton_rest_matrices_and_vertices`: identical skeleton (bit equal), rest matrices within 1e-4,
  baked vertices, normals, UVs and indices equal within 1e-4, locator equal. `textures_are_64x64_and_the_embedded_one_is_the_file`
  (decoded embedded texture equals the PNG file, glowmask texel count). `everything_loads_without_warnings` (models, animations,
  controller, rig binding, both routes).
* (b) `golden_pose_tables_30_frames_per_animation`: bone matrices at `t = length * i / 30` (i = 0..29), geo route, compared within 1e-4 with
  the checked-in file. `MOBMODEL_UPDATE_GOLDENS=1` rewrites it. Ground speed is fixed per animation (walk 0.12, run 0.3, others 0) so
  the expressions have an input. `golden_covers_real_motion` guards against a golden of rest poses; `animation_properties_match_the_spec`
  (loop modes, 0.8 s, events, four lerp modes, easings with arguments present); `hold_animations_hold_and_the_one_shot_ends`
  (hold after the length, pounce gives the rest pose at 0.8 s, ground speed changes the walk).
* **The first goldens were produced by this code.** They pin the behaviour of F4M-1/F4M-2 as it is; they were NOT checked against
  Blockbench, GeckoLib or Minecraft. The two direction rules and the axis conversion remain the unverified decisions of the F4M-1 and
  F4M-2 reports; if one of them changes, the golden diff shows exactly which poses move.
* (c) `geo_route_and_bbmodel_route_give_the_same_pose_tables`: all 30 frames of all animations, all bones except the eased leaf bones
  (2000+ matrix comparisons), plus events, length and loop mode. `the_documented_difference_is_exactly_the_eased_leaf_bones` proves the
  excluded bones really differ (so the limit below cannot go stale unnoticed).
* (d) `locomotion_goes_idle_walk_run_with_ground_speed_and_back` (idle, walk, run, walk, idle, hysteresis bands, run clock speed
  0.9 / 2.0 / 0.8, a real cross-fade); `pounce_trigger_fires_hit_exactly_once_at_0_35_s` (60 ticks of 0.05 s: one `hit`, on tick 7 or 8,
  sound and particle on the same tick, the added pose at 0.35 s is body +1.6 up / 3.5 forward, ended after 0.8 s);
  `sit_follows_the_synced_state_and_holds` (sit when synced `state` = 1, pose identical 2 s and 12 s later and equal to the animation's last
  frame, standing up is a cut back to locomotion).
* (e) `anim_clock_anchors_reproduce_the_server_pose_on_a_client`: server runs 60 ticks, an anchor is taken, a late-joining client (state set
  without fade, anchor applied) gives the same bone matrices within 1e-4 at ticks 60, 75 and 75.5; `anchor_stale` reports a speed change;
  the pounce trigger clock `AnimClock(start, speed 1)` gives the same pose as the integrated server clock.

## What could not be expressed in both formats

1. **easing and `easingArgs`.** The bbmodel keyframe form that F4M-2 reads (Blockbench's own) has `interpolation` but no easing, so the
   bbmodel copy drops it. The pose differs on exactly the bones that use easing, all of them leaves (`leg_*`, `ear_*`, `jaw`, `tail3`),
   in `sit`, `pounce` and `death`. Everything else in all six animations is equal. (A GeckoLib-flavoured Blockbench plugin may write easing
   fields into bbmodel keyframes; F4M-2's `animations_from_bbmodel` does not read them.)
2. **Order of events at the same time.** Bedrock: the order of serde_json's sorted object (particle, sound, timeline); bbmodel: keyframe
   order. Same set; the tests compare sorted. Matters only if a consumer relies on order at one instant (the server mod should not).
3. Nothing else: expressions, pre/post, step, catmullrom, bezier (with value handles), constant vectors, `anim_time_update`, loop modes
   and effect events all convert to equal poses.

## Notes from building it (no library change needed)

* `walk` gets its speed from `anim_time_update`, which is stateless (the clock is multiplied, not integrated), so a ground-speed change
  while walking jumps the phase. `run` therefore uses the controller `speed` expression, which integrates. A real fox probably wants the
  controller form for walk too; the design text says "speed from ground speed through an expression", so both forms are covered.
* The `pose` layer cannot fade `sit` out: leaving sit with a blend towards an empty `stand` state lerps to the rest value on the channels
  sit animates and then pops to locomotion. The controller therefore uses a cut (`blend: 0`) on the way out. (F4M-2 open question 7.)
* The `action` layer is `add`: with `override` the pounce would replace the locomotion values on its channels and pop at its end.

## Open questions

1. Is a hand-tuned golden enough for scenario 2, or should a Blockbench-exported pair of this fox (open it in Blockbench, export
   `.geo.json` and `.animation.json`) replace the generated files before the goldens are trusted? The generator's Bedrock output follows the
   documented conversion, but its animation JSON is not a Blockbench export.
2. Easing in bbmodel: read the GeckoLib plugin's keyframe fields (name and place of `easing` / `easingArgs`) in `animations_from_bbmodel`,
   so both routes can match? Needs a real file.
3. Same-time event order: make it deterministic (sort by kind) in the Bedrock loader?
4. Ability for a layer to fade out a state into "nothing" (see notes), needed for a clean `sit` exit.

## Fix round (reviewer decisions)

1. The generated fixture and goldens stay. A real Blockbench export of the fox will be added later as a second fixture.
2. **bbmodel easing is read** and the generator writes it, so the geo route and the bbmodel route now give equal pose tables on ALL
   bones (`geo_route_and_bbmodel_route_give_the_same_pose_tables`: 2520 matrix comparisons, no skips; the old "documented difference"
   test is replaced by `the_bbmodel_route_really_carries_the_easing`). The goldens did not change.
3. **Same-time events** come out in one order in both loaders: time, then kind (timeline, sound, particle), then declaration order.
   The tests compare without sorting.
4. **Layer fade to "no state"**: a controller state with `"inactive": true`. Blending into it fades the layer weight 1 -> 0 over the
   blend time (the old state keeps playing while it fades), then the layer is off; blending out of it fades the weight 0 -> 1. Blend 0
   stays instant. The fox's `stand` is inactive with blend 0.2, so `sit` fades in and out; the cut on the way out is gone
   (`sit_follows_the_synced_state_and_holds` checks no jump at the start, a mid value, and locomotion afterwards).
5. `anim_time_update` is stateless by design: changing the ground speed while walking jumps the phase. Accepted behaviour; use the
   controller `speed` expression for an integrated clock (the fox's `run` does).

Library changes (minimal): `anim.rs` (`bb_easing`, `sort_events` used by both loaders), `controller.rs` (`State::inactive` and its
parsing), `pose.rs` (`Rig::evaluate` layer fade). New tests in `tests/pose_rig.rs` (4: fade weights over time and inactive after,
instant switch, bbmodel easing incl. nested form and start-key rule, event order in both formats). Totals: 119 pass, clippy clean.
