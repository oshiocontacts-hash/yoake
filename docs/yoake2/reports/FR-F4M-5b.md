# FR-F4M-5b: synced data, animation sync and the `horizon:mob_sync` side channel

Branch `yoake-free/f4m-5b`, from `yoake-free/f4m-4a` (`fb8421f90`). Not pushed. Server and protocol side; the client renderer is F4M-6.

## What landed

* `mobs/custom/anim_sync.rs` (+ `anim_sync_tests.rs`): the server animation runtime of a custom mob (below).
* `mobs/custom/mod.rs`: `CustomState` got `synced_dirty` (dirty mask, set by a real change only), `anim`, `hitbox_mask` / `hitbox_dirty`, `sync_out`
  (rate-limit memory); `SyncedDef.rate` (`sync_rate`, 1 to 20, default 10); `CustomType.anim` (the type's rig) and `CustomType.sync_version`.
* `horizon-mods/src/custom_mob.rs`: `LoadedModel` now carries the bound `Rig` and the typed `MobEvents` (`rig`, `events`); `load_model` builds them
  (an unknown animation named by a controller, a bad `server_events` flag or a bad locator is now a load error with context); the `model` object accepts
  `server_events` (`true`, `false`, a list of animation names, or an object of flags: F4M-9's field, until now only read by the mobmodel tests).
  `tools/fixtures/cinder_fox/.../cinder_fox.json` got `"server_events": ["pounce"]`.
* `horizon-protocol/src/mob_sync.rs`: channel constant, limits, codec (`encode`, `decode`).
* `lan/mob_sync.rs`: the server half: `collect`, `snapshot`, `for_viewer`, `frames`, rate limits.
* `game/lan_mob_sync.rs`: `flush_mob_sync`, called after `flush_attachment_sync` in the LAN tick; `lan/hub.rs` (`take_tracking`) and `lan/guest.rs`
  (`mob_sync_starts`) queue the snapshots.
* `mobs/custom/remote.rs` (+ `remote/tests.rs`): the client store and apply rules. `game/remote_custom_mob.rs` + `game/remote.rs`: the payload branch and
  `Remote.custom_mobs` (removed on `RemoveEntities`, cleared on `Login`).
  **Deviation from the work item:** the store lives in `horizon-client-lib` (`mobs/custom/remote.rs`) and `game/remote_custom_mob.rs` is only the glue, so the same
  apply code is tested end to end against a real server mob (the `horizon-client` crate cannot see the test world of the lib).
* `mobs.rs` `Mob::tick`: one `if self.kind.custom.is_some() { anim_sync::tick }` before `ai::tick`. `ai/expr.rs`: `fill` (what `inputs` did, now reusable).
  `ai/attacks.rs` `Timing::begin`: the deadline rule below.

## Wire format (`horizon:mob_sync`, clientbound, version 1)

Big endian. One payload is one frame; every message is absolute (a state, not a change since the last message).

```
frame    := version u8 (=1)  game_time i64  count varint  message*count
message  := kind u8  body_len varint  body            (an unknown kind or a bad body is skipped by its length; the frame survives)
snapshot (1) := entity varint  type_version u32  values  anchors  triggers  hitbox_mask u32
delta    (2) := entity varint  flags u8 (1 values, 2 anchors, 4 hitbox)  [values] [anchors] [hitbox_mask u32]
trigger  (3) := entity varint  trigger_id u16  start_tick i64  speed u8          (speed = speed * 16, 16 is real time)
values   := mask u32  value*popcount(mask)           (ascending slot order)
value    := tag u8 + payload   0 bool u8 | 1 i32 | 2 f32 | 3 string (len u8 <= 64, UTF-8) | 4 vec3 3*f32 | 5 enum u8
anchors  := count u8  (layer u8  state u8  tick i64  n u8  (time f32  speed f32)*n)*count
triggers := count u8  (trigger_id u16  start_tick i64  speed u8)*count          (at most 8 accepted, 4 sent)
```

* `trigger_id` is the global index over the controller layers' trigger lists in file order (`TypeAnim.triggers`). Anchor `time` is f32: the server's clocks are f32,
  so f32 to the wire and back to the f64 `AnimClock` is lossless. An anchor carries every animation slot of the layer's current state and replaces the layer's
  older one whole.
* `type_version` is an FNV-1a hash of the type id, the tracked values (name, kind, enum values, range) and the controller shape (layers, states, slots per state,
  trigger names). A client whose type differs ignores the mob.
* Examples (tests): a trigger message is 15 bytes; a delta with one layer anchor is 25 bytes; an empty frame is 10 bytes.

## Budgets and limits

* **256 bytes of values per message and mob**: counted as the schema counts them (bool and enum 1, i32 and f32 4, vec3 12, string 65; the tag byte is framing). The
  schema (`CustomMobFile::parse`) already refuses a type whose values exceed 256 (it counts every value, also `sync: none`), the codec refuses to encode or decode more.
* **Rate**: a value is sent at most `sync_rate` times a second (default 10: one in two ticks; 20 is every tick, 1 is every 20). A mob's deltas are no closer together
  than the fastest rate of what they carry (anchors and a hitbox change count as 10), so a mob sends at most 10 deltas a second unless a value asks for more. A value
  that changes again before its turn is sent once with its latest value (merged). Triggers go at once, never merged, never limited.
* **Anchors** are re-sent when the controller state changes, a slot starts or stops, or the speed has changed so much that a client extrapolating from the last
  anchor is more than `ANCHOR_TOLERANCE` = 20 ms from the server's clock (`needs_anchor`). With a tolerance of 0 this is exactly "whenever the speed or state
  changes" (tested); the 20 ms keeps a walking mob whose `ground_speed` jitters from sending a message every tick (see open questions). A steady mob sends nothing:
  the test runs an idle mob 300 ticks and counts 0 bytes after its first delta.
* **Distance gate**: only players who track the entity get its deltas and triggers (entity tracking is the range check); a player that starts tracking a custom mob
  gets a snapshot (and nothing else about that mob that tick).
* **Who**: `Guest.placed && brand == "horizon"` only (`reads_mob_sync`). **Nothing without custom types**: `mob_sync::active()` (the registry is non-empty) gates the
  tracking queue in `take_tracking` and `flush_mob_sync` returns before it scans the mobs; a stock client is never sent the channel. `owner` and `none` values are not sent
  (the owner state is F4M-4c).
* Frames are cut at 16 KB (`MAX_FRAME`), the cap of a custom payload is 1 MiB.

## Server animation runtime (`anim_sync.rs`)

`TypeAnim` (per type, built when the type loads, from `LoadedModel`): the `Rig`, the `MobEvents`, the trigger table and the server frames (the `Custom`/timeline events of
the animations flagged `server_events`). `MobAnim` (per mob, in `CustomState.anim`, created by the first tick): `RigState`, `EventMarks`, an `ExprInputs` (its `variables`
persist between ticks), the layers' anchors, the running triggers and the ring of the last four.

`anim_sync::tick(mob, world)` once per server tick, **before** the goals (a mob in a paused game does not tick, so its animation pauses with it):

1. the trigger names the attack goals asked for (`attacks::take_triggers`) and `request_trigger(mob, name, speed)` (for scripts) start playing;
2. inputs: the engine's (`ai::expr::fill`: health, on ground, in water, baby, hurt time, has target, riding, life time, time of day, the tracked values as floats) plus the motion
   queries from the position of the previous tick: `ground_speed` (horizontal blocks per tick, as the fixture's thresholds expect), `vertical_speed`, `yaw_speed`, head and body rotations, `death_time`;
3. `MobEvents::update_marked` (controllers advance by 0.05 s, transitions run) and `trigger_window_marked` per running trigger with the window `[k*step, (k+1)*step)` computed from the
   tick count (no drift) with `step = 0.05 * speed`; a trigger ends when its clock passes its animation's length;
4. every server `Custom` event goes to `attacks::deliver_frame(mob, name)`;
5. the anchors of every layer are refreshed from the integrated clocks.

**Tick convention**: the anchor of tick `T` holds the clocks after tick `T`'s update, and a trigger started in tick `T` has `start_tick = T - 1`, so the client's
`(T - start_tick) * 0.05 * speed` is the server's clock after the update of `T`, and an event of server tick `T` is in the client window that ends at `T`.

**How frames reach `deliver_frame`, and the fallback.** The attack goal asks for its `trigger` in tick `N`; the animation starts it in `N + 1`; the Cinder Fox's `hit` at 0.35 s
is the 8th window, emitted in tick `N + 8`, and the goal (which runs after the animation in that tick) strikes in that same tick (test `the_keyframe_not_the_delay_lands_the_blow`:
`damage_delay` 3, blow exactly 8 ticks after the animation's start). `Timing::begin` now looks the frame up (`frame_deadline`): if a server-flagged animation of the type has a timeline
event of that name, the keyframe decides and `damage_delay` is no longer the clock (the deadline becomes the animation's length plus 4 ticks as a safety net against a frame that is
never emitted, or `damage_delay` if that is longer); if no animation has it (no animation, `server_events` off, another name) the 4a rule is unchanged (test
`a_frame_nobody_emits_falls_back_to_the_delay`).

## Client apply (`mobs/custom/remote.rs`)

Per entity: values by slot, per layer the clocks (`AnimClock`) with an `EventCursor` each, the trigger ring (with cursors), the hitbox mask, a `synced-changed` queue (`take_changes`:
entity, slot, name, old, new; raised for every slot whose value actually changes, a snapshot included; the script hook of F4M-10 will drain it). `poll_events(tick, partial, &mut EventBuf)` gives
the client-side events (sound, particle, custom) of the state animations and triggers through `animation_window` / `trigger_window` for F4M-6.

* **Late join**: the snapshot creates the store and applies everything; its clocks and triggers are `joined`: nothing before the join is replayed, only strictly later events fire (a keyframe exactly at the join moment counts as past).
* **EventCursor rule**: a layer's first clocks from a delta are `fresh` only when received at or before their anchor tick with time 0, else `joined`; later anchors go straight to `advance`.
  A **trigger message** is `fresh` when heard of within 0.25 s of its start (`FRESH_TRIGGER_SECONDS`), else `joined` (deviation, see open questions); a trigger in a snapshot is always `joined`.
* **Loss and reorder** (last writer by frame time, per piece): a value slot takes a value from a frame at least as new as the one that set it; a layer's anchors are replaced by a frame at least as new; a trigger is
  added once per `(id, start_tick)` (a stale copy of a replayed trigger is ignored); the hitbox mask follows the newest frame. So a repeated or late delta changes nothing; a lost delta leaves that slot stale
  until the slot changes again or a snapshot arrives (the snapshot converges everything, and an old snapshot arriving late rolls nothing back). A delta or trigger for an entity with no store (no snapshot, unknown type) is dropped and
  counted (`Applied.dropped`); a snapshot with another `type_version` marks the mob incompatible and everything for it is ignored (`Applied.incompatible`). The ordered connection means the server never sends a delta before the snapshot.
  The server has no uplink to learn of a lost payload (the connection is reliable; a payload is only lost with the connection), so no periodic resync is sent; `mob_sync::snapshot` can be queued again by anything that wants a resync.
* **Integrated server / LAN host**: reads the mob's own `CustomState` (values, `anim`); nothing is sent to its own window and `Remote.custom_mobs` stays empty. The same apply code is what the tests run on a server mob's messages.

## Persistence

Persistent values were already in the `yoake` compound (`synced`, by name; an enum by its value's name; unknown keys and dropped values kept). This item adds the dirty mask (a loaded mob is not dirty; the first snapshot sends
everything), and the test that every kind (bool, i32, f32, string with a multi-byte character, vec3, enum) round trips, a non-persistent value does not and the second save equals the first. The animation state itself is
not saved (`yoake.anim` is not written): a loaded mob starts in its controllers' initial states.

## Tests

(all through `cargo-locked.sh`, offline, `-j 4`)

| run | result |
|---|---|
| `-p horizon-client-lib --lib` | **1773 passed**, 0 failed, 48 ignored (was 1740 after F4M-4a: +33) |
| `mobs::custom` subset | 117 (was 97), plus 12 `lan::mob_sync` and 1 `lan::hub` test |
| `-p horizon-client --lib` | **1203 passed**, 0 failed, 40 ignored (unchanged count) |
| `-p horizon-protocol` | **227 passed** (220 before: +7 `mob_sync`) |
| `-p horizon-mods` | **58 passed** (the F4M-4a run had one failure in the id lock test; it does not occur on this tree) |
| `-p horizon-mobmodel` | **183 passed** over its test binaries (unchanged) |

New tests: `horizon-protocol` `mob_sync` 7 (every message and value type round trip, documented byte layout of a trigger message, 256 byte budget and mask check, a bad message skipped, garbage truncated and byte-flipped never panics, speed bytes,
typical message sizes); `anim_sync` 9 (rig and server frame of the fixture fox, keyframe lands the blow with a 3 tick delay, fallback when the frame is not emitted, no animations means nothing, anchors on state and speed change only with a client extrapolation check,
`needs_anchor` at tolerance 0 and 20 ms, triggers play once / ring / end, wire version, schema budget); `lan::mob_sync` 12 (first look and idle costs 0 bytes, merge into one delta with the latest value, `sync_rate` 20/10/2/1, at most 10 deltas a second with mixed changes,
triggers at once, values nobody is told, snapshot, hitbox mask, frames split at the limit, who gets what, vanilla mob nothing, wire value conversion) + `lan::hub` 1 (`only_a_placed_horizon_client_reads_mob_sync`); `mobs::custom::remote` 9 (late join from the snapshot with the notifications,
deltas keep the client within the anchor tolerance of the server for 240 ticks, reorder and repetition, loss repaired by the next delta or a snapshot, stale anchor, dropped / incompatible / garbage, the client `hit` in the server's tick, late joiners at ticks 5/7/8/9/12, a trigger message heard two ticks late);
`mobs::custom` 2 (dirty mask, every persistent kind round trips).

## Needs checking

* **Yoake: parity B** (and A, C): `f4m_parity` needs game resources and was not run. Nothing changes without a custom type: `take_tracking` and `flush_mob_sync` check `mob_sync::active()` first, `Mob::tick` adds one `is_some()` test, no payload is registered or sent.
* **Two-client end to end** (a LAN host, a Horizon client and a stock client joining, a custom mob in range, walking, pouncing, a late join) is not run: no game resources here. The pieces are tested apart (server mob to codec to store); the glue in `game/lan_mob_sync.rs` and `game/remote*.rs` only compiles here.
* `crates/yoake-bindgen/regen.sh --check`: `CustomState`, `CustomType`, `SyncedDef`, `Guest` (new `mob_sync_starts`) and `Remote` (new `custom_mobs`) changed; none is annotated (`CustomState` is behind `Extra.custom`'s box, as before), `Mob::tick` has the same signature. Not run.
* `load_model` is stricter: a controller state or trigger naming an animation that does not exist, an unknown locator in an event and a bad `server_events` entry are errors (they were not checked before).
* `ground_speed` is horizontal blocks per tick from the position change of the previous tick; the fixture's thresholds (walk 0.12, run 0.3) agree. A mob moved by something that does not change `position` has 0.
* The animation of a mob outside entity-ticking chunks does not run (the mob does not tick).

## Open questions

1. **Anchor tolerance**: the reviewer's rule is "re-send whenever speed or state changes". The implementation re-sends at once on a state change, on a slot starting or stopping, and on a speed change as soon as the client's extrapolation would be more than 20 ms
   wrong (with tolerance 0 it is the literal rule). The reason is a walking mob whose `q.ground_speed * 4` changes by a tiny amount every tick, which would be 10 anchors a second per mob (about 250 B/s per walking mob, over the 5 KB/s player budget with 20 mobs). Is 20 ms right?
2. **Fresh triggers**: F4M-9's rule says `fresh` only for a clock received at or before its anchor tick. A trigger message always arrives at or after the tick it started in, so the literal rule would drop its time-0 events (a roar sound at 0.0). I made a trigger message
   `fresh` when it is heard of within 0.25 s of its start. Confirm, or give the client's tick offset so the literal rule can be used.
3. **Anchors per layer or per slot**: sent per layer (all slots of the current state in one entry), so a speed change of one slot re-sends the layer's other slots too. Cheap for the usual one or two slots.
4. **`sync: owner`** is not sent to anyone until F4M-4c has an owner; should it fall back to everyone?
5. `speed` of a trigger is a byte of sixteenths (0 to 15.9); a script that wants a faster one is clamped.
6. A frame is not re-sent for a lost payload (the connection is reliable). If a future channel can drop payloads (a lossy transport), a periodic snapshot per mob will be needed; the client rules already accept it.
7. The attack goal's `damage_delay` is now only a safety net when the animation has the frame: should the schema get a separate key for it?
8. The client does not copy values into its own `Mob.extra.custom.synced` (F4M-6 reads the store); say if script code on the client needs that mirror.
