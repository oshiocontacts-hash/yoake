# FR-F4M-9: animation events (library part)

Branch `yoake-free/f4m-9` (from `yoake-free/f4m-8`). Crate `horizon-mobmodel`. New: `src/events.rs`, `tests/events.rs`, `tests/events_alloc.rs`.
Touched: `anim.rs` (event parameters, indexed emit), `pose.rs` (`RigEvent` gained `event_index` and `source`, new `RigSource`),
`skeleton.rs` (`bone_matrices_into`), `lib.rs`, the fixture `tests/fixtures/procedural/cinder_fox_type.json` (two new keys). The Cinder Fox
model, animations and golden poses are untouched (no pose changed). Original code only.

## What is done

* `MobAnimEvent` (Copy) with `MobEventKind`:
  `Sound { sound, volume, pitch, locator }`, `Particle { particle, locator, offset }`, `Custom { name, locator }`; plus `side`, `animation`,
  `layer`, `source: RigSource` (`State { state, slot }` or `Trigger { trigger }`), the exact keyframe `time` and the `loop_index`. Strings are interned
  (`StrId`, read with `MobEvents::string`), so emitting never allocates.
* Binding at load (`MobEvents::build` / `from_model_json`): locator names to `LocatorIndex` (unknown locator = `ModelError`
  `file: animations["pounce"].events[2].locator: unknown locator 'x' (the model has: ...)`), particle effect names through the type's `particles` table
  to a vanilla or mod id (a name that is not in the table and not already `ns:id` is an error), id and name validation, limits.
* New optional effect fields in animation.json and bbmodel data points: sound `volume` (0 to 4) and `pitch` (0.5 to 2), particle `offset` `[x,y,z]` pixels.
  Bad values are load errors.
* Type JSON: `model.particles` (`{"name": "ns:id"}` or `{"name": {"id": ...}}`, no namespace means `minecraft:`) and `model.server_events`
  (`true`, `false`, an array of animation names, or an object of flags); errors carry file and path (`type.json: model.server_events[1]: unknown animation 'b'`).
* `EventSide::Client | Server` filter in `MobEvents::resolve`: the client gets sound, particle and custom events of all animations (cosmetic, `is_cosmetic()`);
  the server gets only custom events of animations flagged `server_events`, never sounds or particles. **Gameplay effects come from the server event only;
  the client copy of the same name is for the cosmetic hook `mob-anim-event-client`.**
* Emitting: `MobEvents::update` (wraps `Rig::update`), `trigger_window`, `animation_window` write into an `EventBuf` of fixed capacity (never grows; overflow is
  dropped and counted, `dropped()`). `tests/events_alloc.rs`: counting allocator, 400 ticks of update, trigger window, cursor, locator positions, zero allocations.
* Locator positions: `locator_position` / `event_position` give the model-space position (pixels, `PIXELS_PER_BLOCK` = 16) from matrices made with `bone_matrices_into`
  (the buffer-reusing form of `bone_matrices`). The particle offset is in the locator's bone frame.
* `EventCursor` (client, over an `AnimClock`), rules documented in the type and tested:
  half-open `[last, now)` windows, so once per loop and nothing while paused; **late join** (`joined`) starts at the current time and only events strictly after
  it fire; a **continuous** re-anchor (`AnimClock::reanchor`) changes nothing; a **discontinuous** anchor (backwards, forwards, other state) is a seek: nothing fires
  for the jump, only strictly later events fire afterwards; an anchor at time 0 at or after the last seen tick is a **restart** (fires from 0); a step over
  `MAX_CATCH_UP_SECONDS` (1 s) is a seek; negative speed fires nothing.
* Server determinism: the server uses `Rig::update` / trigger clocks through the marked calls; the same inputs give the same events in the same order (test).

## Tests

`tests/events.rs` 14, `tests/events_alloc.rs` 1; the crate has 177 tests, all green; `clippy --all-targets` clean for the crate. Fox (real fixture):
server pounce emits exactly one `custom:hit` at tick 7 or 8 and nothing else; the client gets `hit`, `cinder_fox_pounce` and the particle `sample_mobs:ember`
(locator `mouth`) in that order at the same tick; the server flag off or missing gives the server nothing while the client is unaffected; locator position follows the head pose.

## Engine-side remainder

* `mobs/custom/events.rs`: own one `MobEvents` per type (built when the type loads, with the model's locators), one `EventBuf` per thread; on the server call
  `MobEvents::update` each tick and `trigger_window` with the trigger's integrated clock; route the `Custom` names to the goal/script hook `mob-anim-event`.
* `game/custom_mob_events.rs`: on the client keep one `EventCursor` per synced clock (`joined` when the mob enters view or on join, `fresh` when the start was seen),
  call `animation_window` / `trigger_window`, play sounds (volume, pitch) and spawn particles at `event_position` transformed by the mob's model-to-world
  matrix (divide pixels by 16), and call the cosmetic hook `mob-anim-event-client`.
* `melee_attack` / `leap_attack` damage frames: read the damage keyframe name from the goal config and apply damage when the server's `Custom` event with that name arrives
  (fallback tick count when the type has no event).
* WIT: `mob-anim-event-client(mob, name)` and `mob-anim-event(mob, name)`, name passed as the string (`MobEvents::string`).

## Decisions (reviewer, fix round)

1. **`server_events` lives in the type JSON only** (`model.server_events`), never in animation files: those stay plain Bedrock or Blockbench exports. Write it as the
   **list of animation names**, `"server_events": ["pounce"]`; `true`, `false` and the object form `{"pounce": true}` are accepted too.
2. **Sound `locator`** (`"sound_effects": {"0.35": {"effect": "...", "locator": "mouth"}}`) is an optional **Yoake extension**, not part of Bedrock. Without it the sound plays at the mob's position.
   `volume`, `pitch` (sounds) and `offset` (particles) are extensions in the same way.
3. **`MAX_CATCH_UP_SECONDS` = 1.0** is a named, documented `pub const` in `events.rs`: a client step larger than this is a seek (nothing fires, the mark moves), so stalls never burst effects.
4. **High-water marks, no event twice.** `EventMarks` (server and any window-driven emission: `update_marked`, `trigger_window_marked`, `animation_window_marked`) keeps, per (layer, state slot or
   trigger), the furthest `(loop_index, keyframe time)` emitted; only events past it fire. `EventCursor` keeps the same mark in clock time (which already contains the loop). Events that share
   a keyframe time fire together (compared with the mark as it was before the call). Resets: seek, restart and state change (cursor); `update_marked` resets a layer that entered a new state;
   `reset_trigger` when a trigger is played again; `reset_all`. A new loop is past the mark and fires again. All allocation is in `EventMarks::new`; the allocation test covers the marked calls.
   Tests: rewind (negative speed) then forward gives no second `hit` (cursor), no repeat of the rewound part and a new loop fires again, repeated and rewound server windows give no second `hit`,
   seek resets (existing re-anchor test), `update_marked` equals `update` on the plain run.
5. **Rule for the engine sync code (F4M-6): `joined` versus `fresh`.**
   * Build the client's `EventCursor` with **`joined(clock, tick, partial)`** whenever the client did not see the animation start from its first tick: a late join, a mob entering view or
     being loaded, a chunk or entity re-sent, a hot reload, anything where the first clock it gets already has `anchor_time` or the tick difference beyond the start. This is the default; it never
     replays the past and only events strictly after the join moment fire.
   * Use **`fresh(clock)`** only for a clock the client received at or before its own anchor tick (`now_tick <= anchor_tick`, and `anchor_time == 0`), that is, the start of the animation is still ahead
     or exactly now. Then events at the anchor time (0) fire.
   * Later anchors for the same cursor go straight to `advance`; the cursor classifies them (continuous, seek, restart) itself. Never rebuild the cursor per anchor packet.
   * The server uses `EventMarks` (and calls `reset_trigger` when it starts a trigger), not a cursor.
   * Hidden mobs: keep advancing the cursor (cheap) or rebuild with `joined` when they become visible again after more than `MAX_CATCH_UP_SECONDS`.

## Also on this branch (reviewer additions, separate commits)

* Cinder Fox hind legs split (`leg_bl_lower`, `leg_br_lower`): 18 bones, golden file +360 lines (no old line changed), `cinder_fox_four_leg_ground_ik` test. See `FR-F4M-FOX.md` and `FR-F4M-8.md`.
* `ProceduralStats` per-mob counters in `procedural.rs`. See `FR-F4M-8.md`.
