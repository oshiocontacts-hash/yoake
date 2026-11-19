# FR-F4M-8: procedural constraints and the `mob-pose` interface (library part)

Branch `yoake-free/f4m-8` (from `yoake-free/f4m-fox`). Crate `horizon-mobmodel`, new files only: `src/ik.rs`, `src/procedural.rs`,
`tests/procedural.rs`, `tests/procedural_alloc.rs`, `tests/fixtures/procedural/cinder_fox_type.json`; `lib.rs` got two `pub mod` lines
and a doc bullet. `anim.rs`, `controller.rs`, `pose.rs` and the fox fixtures are untouched. Original code, no GeckoLib or other engine code.

## What is done

* `ik.rs`: stateless, total math helpers (never NaN, never panic): `rotation_arc`, `AimFrame` (yaw/pitch of a direction, rotation
  from yaw/pitch), cone clamp, `rotate_dir_toward`, `move_toward`, `angle_between_deg` (atan2 form, accurate for tiny angles),
  `solve_two_bone` (law of cosines, reach clamp, pole).
* `procedural.rs`:
  * `ProceduralSet` (immutable, shared per type, bound to the skeleton) and `ProceduralState` (per mob: spring particles, follow
    history, smoothed look angles, scratch matrices). `ProceduralSet::evaluate(&mut Pose, &mut ProceduralState, &ProceduralInputs)`
    edits only the rotations of the animated pose, in array order, and returns an `EvalReport`. Proven allocation-free
    (`tests/procedural_alloc.rs`, counting allocator, all six kinds plus bone writes).
  * Constraints: `look_at`, `aim`, `two_bone_ik`, `spring`, `follow`, `counter_rotate` as specified; each has `when` (an `expr`
    expression), `fade` (seconds, default 0.15) and `weight`.
  * `parse_procedural(file, path, &Value, skeleton, locators, syms)` / `parse_procedural_str`: errors are `ModelError`
    (`file: path: message`, e.g. `fox.json: model.procedural[2].yaw: 400 is out of range (0 to 180)`); unknown fields, bones,
    locators, target kinds, bad expressions (with column) and every limit are hard errors.
  * `BoneWrite` / `apply_bone_writes(pose, writes)` / `apply_bone_writes_with_visibility` + `BoneVisibility` bitset + `WriteReport`.
* Tests: 35 in `tests/procedural.rs`, 1 in `tests/procedural_alloc.rs`, 6 unit tests in `ik.rs` and `procedural.rs`. Whole crate:
  162 tests (60 lib, 13 cinder_fox, 1 expr_alloc, 4 expr_props, 6 fuzz, 25 model, 17 pose_rig, 35 procedural, 1 procedural_alloc), all green;
  `clippy --all-targets` has no warning in the crate.

## Conventions (all documented in the `procedural.rs` module header, pinned by tests)

* Model space, pixels, Blockbench axes (+X right, +Y up, -Z forward). All caller points (targets, `model_to_world`) are model space.
  A bone's joint position is `bone_matrices[i] * pivot`. `forward` and `axis` options are in the bone's geometry frame (before its rest rotation).
* Constraints edit `Pose::bones[i].rotation` only. Blend with weight / fade is a short-way slerp of the rotation the constraint
  computes against the one it found. Each constraint sees the matrices after the previous ones (kept up to date incrementally).
* `dt` is sanitised: NaN, infinite, negative become 0; above 0.1 s clamps to 0.1 s.
* Synced enums in `when` are the **index in the type's `values` list**: fox sitting is `q.synced('state') == 1` (idle 0, sit 1,
  pounce 2). `expr` has no enum-name literals (only the synced name literal), so that is the only form.
* Targets: `"view"|"target"|"owner"` and `{"input": 0..4}` are points the caller puts in `ProceduralInputs::targets[slot]`
  (slots View 0, Target 1, Owner 2, Custom(n) 3+n); `None` = unavailable. Also `{"point"}`, `{"locator"}`, `{"bone"}` resolved at load.
  `look_at`/`aim` without a target relax to the animated pose at their speed; IK without a target fades out on its last target.
* `look_at`: yaw/pitch limits symmetric degrees, relative to the bone's animated orientation; speed is degrees/s per angle
  (0 = unlimited, default 360). Pitch is applied before yaw. `aim`: shortest-arc swing of any axis, cone limit, speed.
* `two_bone_ik`: root, mid, tip bone (+ `tip_offset`) or `tip_locator`; mid descends from root, tip from mid. Pole is `dir`, `point` or
  `bone`, else the current bend side is kept. Out of reach: fully extended toward the target; too close: fully folded. Rotations of root and mid are
  swing-only (no twist control). The solve repeats up to 4 passes until the end point is within 1e-4 (needed when `body` has the non-uniform
  scale of the fox's breathing animation; a rigid limb converges in one pass).
* `spring`: tips (next chain bone's pivot; last bone: `tip_length` along `tip_dir`) are damped spring particles simulated in a frame
  given by `ProceduralInputs::model_to_world` (identity by default; pass the mob's transform times 16 to get inertia from movement and turning;
  gravity is world -Y in that frame, pixels/s^2). Fixed step 1/120 s, at most 16 steps per call, semi-implicit Euler with exponential
  damping (stable for stiffness up to 10000 for any dt). Particles are projected onto the sphere of the bone length around the pivot
  every frame (lengths never change) and the swing is limited by `max_angle`. A particle more than 2 bone lengths from its rest tip
  (teleport) is pulled back with zero velocity. Non-finite particles are reset to the animated tip. A deactivated spring restarts from the
  animated pose when it re-activates.
* `follow`: copies the source bone's local rotation (and optionally position and scale, see `channels`) `delay_ticks` ticks (0.05 s) late, with a per-constraint ring buffer of
  `delay_ticks + 2` quaternions, sampled at tick boundaries by interpolating inside the frame, so any frame length gives the same lag. History keeps
  recording while the constraint is inactive.
* `counter_rotate`: minimal swing so the bone's `axis` (default +Y) points along model-space `up` (default +Y); the bone's own twist about it survives.
* Bone writes: additive = translation add, rotation `old * delta` (delta in the bone's local frame), scale multiply; absolute replaces. Rotation as Euler
  degrees (animation order) or quaternion. A write with any NaN/infinite/over-1e6 value or a zero quaternion is rejected whole; unknown bone ids are
  ignored; both are counted in `WriteReport`; at most 1024 writes per call. `visible` is not part of `Pose`, so it goes to a `BoneVisibility` bitset
  (the plain `apply_bone_writes` counts such values as `visibility_ignored`).

## Limits (documented in the module header, enforced at load)

64 constraints per type; per kind: look_at 16, aim 16, two_bone_ik 8, spring 16 (chain 1..16 bones, 64 spring bones in all), follow 16 (delay 0..40
ticks), counter_rotate 16; yaw/pitch/max_angle 0..180; weight 0..1; fade 0..10 s; stiffness 0..10000; damping 0..1000; all numbers finite and within +-1e6.

## Known limits and deviations

* Non-uniform scale on an ancestor makes a "world rotation" of a bone a shear, so exact rigidity of limb lengths is impossible there. The IK still reaches
  its target (iteration) but the posed limb lengths follow the squash (fox test allows 0.15 px against the lengths before the IK).
* IK root and mid are rotated as ball joints: no hinge or twist limits, no tip orientation matching. Not requested; easy to add.
* `look_at` angles are two independent speed-limited numbers (not one great-circle speed); a target that swings across the back goes through the front when the
  limits are wide (no shortest-way wrap).
* No collision for springs (the design says collision off).

## Findings about the fixtures (fix round)

* The Cinder Fox front legs are now two bones (`leg_fl` upper, pivot at the shoulder, and `leg_fl_lower`, the paw; same for `leg_fr`), so
  `two_bone_ik` is a real two-bone chain rooted at the shoulder. The hind legs stay single bones (not needed for the sit pose). The fox now has
  **16 bones and 16 cubes** (was 14); `golden_poses.txt` gained exactly 360 lines (6 animations x 30 frames x the 2 new bones), no earlier line changed
  (the animations still drive the upper bones by their old names, the lower bones inherit). Both formats were regenerated from `gen_cinder_fox.py`
  and stay equal. The front leg cubes shrank to 2 px (upper) and 1 px (paw) with the paw's box UV at `[0, 34]`; the texture changed accordingly
  (`cinder_fox.png` only; the glow mask is the same).
* The fox IK test now roots the chains at the shoulders: the body matrix is unchanged by the IK (asserted to 1e-6) and both paws reach their targets within 1e-3.
* `idle` and `sit` still scale `body` non-uniformly (1.5 to 3 percent, breathing). Accepted: limb lengths follow it by a few hundredths of a pixel
  (the test allows 0.05 px), and the IK iterates to hit the target anyway.
* `tests/fixtures/procedural/cinder_fox_type.json` holds only `synced` and `model.procedural` (look_at head 60/40, tail springs, two IKs on
  `when q.synced('state') == 'sit'`), as section 10 describes.

## What remains for the engine side (not done here)

1. `mobs/custom/procedural.rs`: per mob `ProceduralState` next to the `RigState`; call `evaluate` after `Rig::evaluate` and before composing matrices, then `apply_bone_writes`.
   The caller fills `ProceduralInputs`: view point (camera in model space, only for mobs in range), target/owner points, `model_to_world` (mob position and body yaw times 16),
   `dt` from the frame clock, `ExprInputs` (the same as for the controllers; synced enum index as float). It must feed `BoneVisibility` to the renderer.
   Honour LOD: skip `evaluate` for `static`/`hidden`, run springs at the LOD rate with a bigger dt (up to 0.1 s is stable).
2. Type loader: pass `model.procedural` to `parse_procedural` with the type's `Symbols` (same table as the controllers, so `q.synced` slots agree) and the model's locators.
3. WIT rows (draft, `yoake-mobs.wit`): `record bone-write { bone: u32, mode: write-mode, translation: option<vec3>, rotation: option<rotation>, scale: option<vec3>, visible: option<bool> }`,
   `enum write-mode { additive, absolute }`, `variant rotation { euler-degrees(vec3), quat(quat) }`, client export `mob-pose(frame, bones) -> list<bone-write>`
   (only for types flagged `procedural_script`, only in LOD range), client import `bone-index(type, name) -> option<u32>` (already in section 6.2). The host maps these 1:1 to `BoneWrite`.
4. F4M-6 dependency: needs the type loader, synced values table and the client mob state (position, yaw, view point); nothing here blocks it.
5. Optional: expose `EvalReport` / `WriteReport` counters in the debug overlay.

## Reviewer decisions (fix round)

1. Bone-write rotation: both Euler degrees (Blockbench order) and quaternion stay. The SDK helpers (guest side) should default to Euler.
2. Additive writes are `old * delta` in bone-local space, exactly how additive animation layers combine rotations in `pose.rs` (a delta in the bone's own frame),
   so a script's additive write and an additive layer agree. Translation adds, scale multiplies.
3. The `view` target is per client and cosmetic: the engine resolves it on each client from that client's camera, never stores it in server state or syncs it.
   Do not use it for gameplay; servers evaluate no procedural constraints.
4. `model_to_world` is the mob's full transform (yaw, pitch and roll, translation, times 16). Docs and API say so. `spring_gravity_follows_a_pitched_and_rolled_model_frame`
   settles a tail in a yaw 20 / pitch 35 / roll 70 frame and checks the sag follows model-space down (cos > 0.99), and in a 90 degree roll that it goes sideways.
5. Enum names in `when`: `q.synced('state') == 'sit'` (also `!=`, string first) compiles to the value's index. `Symbols::set_enum_values(synced_name, values)` declares
   the values (the type loader calls it from the type's `synced` enum lists); strings are accepted only as the other operand of `==`/`!=` against a bare `query.synced(..)`.
   Unknown name = `ExprErrorKind::UnknownEnumValue` (message lists the known values, column of the string); through `parse_procedural` it is
   `file: path[i].when: expression: column N: unknown enum value ...`. FR-F4M-2.md's grammar is updated. The engine loader must call `set_enum_values`
   for every enum before compiling controllers and constraints.
6. `follow` has `channels`: a non-empty list of `"rotation"` (default), `"position"`, `"scale"`. Position (local translation) and scale copy with the same delay;
   with weight < 1 each channel blends from the bone's own value (rotation slerp, others linear). Errors: unknown or duplicate channel, empty list.
7. See the fixture findings above.

## Open questions (remaining)

1. Hind legs are single bones; split them the same way if a hind-leg IK demo is wanted.
2. Does the engine want `EvalReport` / `WriteReport` counters in a debug overlay?
