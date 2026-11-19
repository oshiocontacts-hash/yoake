# FR-F4M-2: animation runtime and expression language

Crate `crates/horizon-mobmodel`, files `src/{anim,expr,controller,pose}.rs` plus `tests/`. No GPU, no game
dependencies. `skeleton.rs` and the shared types (`Skeleton`, `BoneIndex`, `BoneTransform`, `Pose`, `bone_matrices`)
are unchanged; `lib.rs` only got four `pub mod` lines.

## Supported features

* `anim`: `animation.json` (`parse_animation_file`): `loop` true/false/`"hold_on_last_frame"`, `animation_length`,
  `override_previous_animation`, `anim_time_update`, `blend_weight`, bones with `rotation`/`position`/`scale` as a
  constant, a vector of numbers or expressions (a single number means all three axes), or a time-keyed table with
  `pre`/`post`, `lerp_mode` (linear, catmullrom, bezier, step), `easing` (linear and sine, quad, cubic, quart, quint,
  expo, circ, back, elastic, bounce; in, out, in-out; written `easeInSine`, case and underscores ignored), optional
  bezier handle fields; `sound_effects`, `particle_effects` (with `locator`), `timeline` (string or list of strings).
  `animations_from_bbmodel(&Value)` converts a bbmodel `animations[]` (length, loop once/hold/loop, `override`,
  animators per group name, rotation/position/scale keyframes with `data_points` (one point, or pre and post),
  interpolation linear/catmullrom/bezier/step with the four bezier handle arrays, effect animators for
  sound/particle/timeline) to the same `Animation` type keyed by bone name. `AnimationLibrary` (max 128 animations,
  lookup by exact name or unique `.suffix`).
* `expr`: the language of section 3.6, compiler to flat bytecode, VM (below).
* `controller`: section 6.1 format: layers (several controllers) with `weight` (number or expression), `bones_mask`,
  `include_children`, `mode` override/add, `triggers`; states with animations (name, per-animation `speed` and `weight`),
  state `speed`, `blend`/`blend_transition`, `transitions` (`to`, `when`, `blend`), `on_enter` variable assignments.
* `pose`: `Rig` (skeleton + library + layers, names bound at construction), `RigState`, `Rig::update` (advance clocks,
  transitions, event emission), `Rig::evaluate` (full or sparse pose), `sample_animation` (one animation, for tools
  and golden poses), `bone_closure` + `bone_matrix` (sparse evaluation and a single bone's matrix for hitbox follow),
  `trigger_events`. `Scratch` holds reusable buffers.

## Coordinate conventions (as requested by the coordinator after F4M-1)

Internal space is Blockbench space. `animation.json` is Bedrock and is converted on load with the same mapping as the
geo.json loader: position `(x,y,z)` becomes `(-x,y,z)`, rotation `(rx,ry,rz)` becomes `(-rx,-ry,rz)` (applied to constants,
both pre and post of keyframes, expression results (one `Neg` instruction appended) and bezier value handles; scale and
times are untouched). bbmodel animations are used as is. Rotation order is `Rz*Ry*Rx` (`Quat::from_euler(ZYX, z, y, x)`),
as in `bone_matrices`. Golden test: a Bedrock arm rotation x=+90 swings the hand to +Z; the same keys from a bbmodel
swing it to -Z (`tests/pose_rig.rs`).

## Other conventions chosen (spec was ambiguous)

* Time in seconds. Rotation interpolates per Euler component (as Bedrock does), then becomes a quaternion once per bone.
* A keyframe's `lerp_mode`/`easing` control the segment that starts at it. Easing shapes linear and catmullrom progress,
  is ignored for step and bezier. At exactly a key's time the value is its `post`; before the first key the first `pre`
  holds; after the last the last `post` holds. Catmull-Rom is the uniform spline over key values; at the ends the missing
  neighbour duplicates the endpoint. Bezier is per axis, time solved by 32 bisections, handle times clamped into the
  segment (keeps time monotonic); default handles are Blockbench's (0.1 s, flat). Step holds the start key's `post`.
* Missing `animation_length` = largest key/event time. Zero length = constant pose, acts like hold. `Once` animations
  stop affecting the pose after their length. Events later than the length warn and never fire.
* `anim_time_update` is stateless: sees `query.anim_time` = clock, returns the time to sample (before looping), not an
  accumulator. Keyframe expressions see `query.anim_time` = the wrapped sample time. `blend_weight` multiplies the weight.
* Stacking inside a state: normal animations add (rotation/position `+ w*v`, scale `1 + w*(s-1)` multiplied);
  `override_previous_animation` lerps the channels it animates (weight capped at 1). Layers: `override` (lerp by weight,
  only channels the layer animates) or `add`.
* Controllers: layers run in file order (order of JSON objects is preserved with a serde visitor, not sorted); state
  ids are file order (stable, one byte); trigger ids are file order. One transition per update, the first matching
  in order; `blend` of the transition overrides the state's. The state being left keeps playing while it fades; a
  transition during a fade drops the older state. Each animation has its own clock integrating
  `dt * state_speed * anim_speed` (speeds clamped to [0,100]), so changing speeds never jump. `dt` is sanitised and
  clamped to [0,10]. In `when`, `query.anim_time` is the seconds in the state. `set_state` lets a client apply a
  synced state id.
* Events: `[c0, c1)` over the animation's own clock (speed applied), so consecutive windows never repeat an event, a
  loop animation fires each event once per loop, nothing fires at `dt = 0` (paused) or when time goes backwards. One call
  walks at most 64 loops (a long stall emits only the last 64). Events at time 0 fire on the first update with `dt > 0`.
  The fading-out state's events also fire.
* Empty bbmodel data-point strings mean the channel default (0, scale 1).

## Expression language

```
expr    := or ( '?' expr ( ':' expr )? )?       a ? b means a ? b : 0; right-associative
or      := and ( '||' and )*
and     := equal ( '&&' equal )*
equal   := rel ( ('==' | '!=') rel )*
rel     := add ( ('<' | '<=' | '>' | '>=') add )*
add     := mul ( ('+' | '-') mul )*
mul     := unary ( ('*' | '/' | '%') unary )*
unary   := ('-' | '+' | '!') unary | primary
primary := number | true | false | '(' expr ')' | math.f(args) | math.pi | query.NAME | query.synced('name') | variable.NAME
```
Aliases `q.`, `v.`, `m.`; names are case-insensitive; one trailing `;` is accepted.
Functions: sin cos tan asin acos atan (degrees), abs floor ceil round sqrt trunc sign exp ln, min max pow mod atan2 random,
clamp lerp. Queries: anim_time life_time ground_speed vertical_speed yaw_speed head_x_rotation head_y_rotation
body_y_rotation health max_health is_on_ground is_in_water is_baby hurt_time death_time is_riding has_target time_of_day.

* All values are `f32`; comparisons give 1/0. Division and `%` by 0 give 0; any NaN/inf result becomes 0 (inputs too), so
  the VM only sees finite numbers. `%` and `math.mod` are the truncated remainder.
* `math.random(lo,hi)` is a pure hash of (per-mob seed, caller epoch, expression text, call position): identical on
  server and client; the caller advances `ExprInputs::random_epoch` for new numbers.
* Stack machine, no jumps: both ternary branches are always evaluated, cost = instruction count exactly. Constants are
  folded at compile time (`1+2*3` is one instruction).
* Errors carry a 1-based column. Kinds: `Syntax`, `Unsupported` (unknown math/query name, assignment `=`, `->`, `??`,
  strings other than the synced name, `loop`, `return`, `temp.`/`context.`, nested names, several statements),
  `UnknownSynced`, `Limit`. Loaders turn `Unsupported` into a `Warning` (with the animation or controller name) and use
  the constant 0; everything else is a load error with context and column.
* Limits: 256 instructions (`limits::MAX_EXPR_INSTRUCTIONS`), 2048 source characters, nesting 48, 64 `variable.*` slots,
  32 synced slots, 8 animations per state, 255 states, 16 layers, 128 animations. No panic is possible from any input:
  the VM stack is a fixed array sized from the compile-time depth (24 or 256) with saturating access. Evaluation does
  not allocate (proved with a counting allocator).

## Tests (62 total, all green; clippy clean with `--all-targets`)

* 44 unit tests in the crate: `anim` 22 (easing table of 30+ hand-computed values, names, linear/pre-post/step/catmullrom/
  bezier goldens incl. value handles, bezier clamping and exact end points, loop/hold/once, anim_time_update, events in
  windows, Bedrock axis conversion, bbmodel conversion and errors, library), `expr` 13, `controller` 8, `skeleton` 1 (F4M-0).
* `tests/expr_props.rs` 4: 20,000 seeded token-soup expressions and 20,000 raw-character strings (never panic, finite,
  cost <= 256, error columns in range, both outcomes exercised), instruction cap (255 ok, 256 ok, 257 rejected), and a
  differential test of 4000 random expression trees against a reference evaluator.
* `tests/expr_alloc.rs` 1: zero allocations over 2000 evaluations.
* `tests/pose_rig.rs` 13: golden swing pose through `bone_matrices` (Bedrock and bbmodel), cross-fade golden (5.0, 7.5,
  10.0), stacking/override/weights, layers with masks and triggers, layer modes, weight expression, sparse evaluation equal to
  full, timeline events once per loop and not while paused, trigger events, binding errors/warnings, determinism,
  degenerate inputs (NaN/inf/negative dt).

## Open questions

1. Interpolation owner: Bedrock/Blockbench may apply `lerp_mode`/`easing` to the segment ending at a key rather than
   starting at it. Needs real Blockbench exports (Cinder Fox) to confirm; one-line change in `sample_keys`.
2. Bedrock `bezier` keyframes in animation.json: handle fields use the bbmodel names; no Bedrock sample seen.
3. `catmullrom` neighbour choice at pre/post jumps (uses previous `post`, next `pre`) and the end-point rule are my choice.
4. Because speeds are integrated, a client rebuilding a clock from a synced start tick is exact only for constant speed;
   F4M-5b should sync the speed with the trigger (it already does for one-shots) or accept drift on speed-driven loops.
5. `query.delta_time`, `math.hermite_blend`, `math.lerprotate`, `math.die_roll` etc. are not in the spec list and warn;
   GeckoLib-style files that use them need a decision (real files will show how often).
6. Variable assignment is only via controller `on_enter` (the language has no `=`); scripts write `ExprInputs::variables`.
7. Layer fades (`weight` expression) are the only way to fade a one-shot trigger animation out; a trigger blend-out field
   could be added if the Cinder Fox needs it.
8. The Cinder Fox golden pose tables ("Done" in the work item) need F4M-1's real model and animation files; everything here
   is verified on synthetic skeletons only.
