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
* Server determinism: the server keeps using `Rig::update` / trigger clocks; the same inputs give the same events in the same order (test).

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

## Open questions

1. Where does the `server_events` flag live? Done as `model.server_events` in the type JSON (list, bool or object) because animation.json is Bedrock format; a per-animation
   key in the animation file is a small change if preferred.
2. The fox sound has no locator (plays at the mob); a locator on a sound is supported (`"locator"` in the effect object) but not Bedrock-standard.
3. `MAX_CATCH_UP_SECONDS` = 1 s is a guess; large frame stalls skip cosmetics instead of bursting them.
4. A client that rewinds by negative speed and plays forward again fires events again (a new pass). Say if a high-water mark is wanted instead.
5. A client-seen state start is only "fresh" if the engine says so; the engine must call `joined` for a late join (default for safety) - needs a note in the sync code.
