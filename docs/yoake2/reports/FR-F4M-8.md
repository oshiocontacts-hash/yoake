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
* Tests: 31 in `tests/procedural.rs`, 1 in `tests/procedural_alloc.rs`, 6 unit tests in `ik.rs` and `procedural.rs`. Whole crate:
  153 tests (59 lib, 13 cinder_fox, 1 expr_alloc, 4 expr_props, 6 fuzz, 25 model, 13 pose_rig, 31 procedural, 1 procedural_alloc), all green;
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
* `follow`: copies the source bone's local rotation `delay_ticks` ticks (0.05 s) late, with a per-constraint ring buffer of
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

## Findings about the fixtures

* The Cinder Fox has single-bone legs, so there is no real two-bone chain on it. The fox test and fixture JSON use `root: body, mid: leg_fl|fr, tip_offset (0,-3,0)`
  to exercise the real files; it works, but both front legs share `body` as root (the second solve moves the first leg again, only the last is exact) and it
  tilts the whole body. A proper fox needs `leg_fl_upper`/`leg_fl_lower` style bones (a model change, not done: the fox fixtures are being edited on another branch).
* `idle`/`sit` scale the `body` bone non-uniformly (about 1.5 to 3 percent).
* The type JSON for the fox is not in the repo yet; `tests/fixtures/procedural/cinder_fox_type.json` holds only `synced` and `model.procedural` (look_at head 60/40,
  tail springs, two IKs on `when q.synced('state') == 1`) as the design in section 10 describes.

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

## Open questions

1. Is the rotation in `bone-write` Euler degrees (Blockbench-friendly), quaternion, or both (as `WriteRotation` now)?
2. Additive rotation order: `old * delta` (delta in the bone's own frame) was chosen; scripts used to Euler channel addition may expect the sum. They agree for single-axis deltas.
3. Should `look_at`'s `view` target be per player (each client sees the mob look at its own camera) or a game-wide point? The library does not care; it is the engine's choice.
4. Spring inertia needs the mob's world transform each frame (`model_to_world`); if the client only has an interpolated position, is the 16x pixel scale and yaw enough, or is the body's pitch/roll needed too?
5. Enum names in `when` (`q.synced('state') == 'sit'`) would be nicer than indices; that is an `expr` change (F4M-2 owner), not done.
6. Should a `follow` constraint also support copying translation or scale? Only rotation is implemented.
7. The fox needs upper/lower leg bones (or a different IK demo) to be a convincing two-bone case; who owns the fixture change?
