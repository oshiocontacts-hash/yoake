# FR-F4M-14: performance pass

Branch `yoake-free/f4m14-bench` (from `yoake-free/merge` @f7516323c). Not pushed.

## Scenario

`custom_mobs_200`: the ignored test at the end of the test module in
`crates/horizon-client/src/mobs/custom/render_state.rs`. Headless (no window, no GPU): 200 Cinder Foxes from the
fixture, each posed (`MobRender::update`: rig, procedural, bone matrices) and vertex-built (`TypeRender::draw`) every
frame. Pose and vertex stages are timed apart. 30 warm-up frames, 300 measured.

- `spread` (default): mobs 0..120 blocks away, so every LOD mode is used.
- `near`: mobs 10..30 blocks away, all Full LOD (the worst case of the vertex load).

Run: `hz-cargo test --release -p horizon-client-lib --lib custom_mobs_200 -- --ignored --nocapture`.
The game scenario in `horizon-bench` was not built: that crate holds only the report format and cannot depend on the
client. The doc allowed this place.

## Numbers (release, median of the frames, 5 runs each)

Machine: Windows 11, shared with other work (a build of another job may run during tests). Runs were interleaved
(base, new, base, new...) for the change below.

| Scene | Pose (update) | Vertices (draw) | Total | p95 total | Evaluated / frame | Vertices / frame |
|---|---|---|---|---|---|---|
| spread | 0.17 ms | 0.78 ms | 0.95 ms | 1.06 ms | 67 | 124,800 |
| near | 0.42 ms | 0.78 ms | 1.21 ms | 1.33 ms | 170 | 124,800 |

The first release run series (before interleaving) gave a higher spread: pose 0.23 ms, draw 1.03 ms, total 1.26 ms,
and near: pose 0.55 ms, draw 1.02 ms, total 1.55 ms. Absolute numbers move by about 25% between runs and machines; the
comparisons below use interleaved runs only.

Debug build for comparison (not a target): spread update 1.27 ms, draw 14.27 ms.

Against FR-F4-MOBS.md section 8: animation 1.0 ms, vertices 1.5 ms, total 3 ms. All met with margin. The bench
evaluates 67 mobs (spread) or 170 (near) per frame at 30 bones each; the doc's 6000-bone figure is not reached in
either scene, so the per-bone cost was not measured separately.

## Tuning

| Change | Result | Kept |
|---|---|---|
| Vertex loop: one reserve plus `extend` over a `flat_map` instead of per-vertex `push` (`emit_bone`) | draw median 0.78 to 0.85 ms (slower in every interleaved pair); total 0.95 to 1.02 ms spread | no, reverted |
| LOD defaults | The engine has no default table: a type without `model.lod` is Full at every distance. The fixture's table (24/48/64 blocks) only sets the evaluation rate. Vertices per frame are unchanged by LOD (only Hidden skips drawing), so a table change moves only the pose cost, which is already 0.17 ms. Not changed. | no |
| Sparse evaluation | Already in place (`MobRender::due`: half rate beyond 24, quarter beyond 48, static beyond 64). Measured: 67 of 200 evaluated per frame in spread, 170 in near. | n/a (existing) |
| SIMD | Not tried: the vertex loop is 8 ns per vertex and the budget has room; the doc asks for SIMD only where it measurably helps. | no |

Nothing kept beyond the bench test. The code under `render_state.rs` and `custom_mob_draw.rs` is unchanged apart
from the test.

## Left

- Server tick cost (budget 5 ms of 50 ms, 200 mobs with three goals) is not measured. The bench has no server; a
  separate scenario would drive `tick_custom_mob_events` or the server goal loop.
- Vertices are drawn for every mob not Hidden, even when Static (distance over 64). A cache of the vertices of a
  Static mob (positions only change with the transform) could cut draw cost for far mobs. Not done: it changes the
  draw path and is not needed for the budget.
- GPU skinning (F4M-16) is not needed by these numbers.
