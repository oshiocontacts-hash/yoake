# FR-F4-MOB: fully custom mobs (own type, AI, hitboxes, model, animation)

Status: design only, no engine code changed. Base: branch `yoake-free/merge` at 62e17adf1. File:line references were read
on that commit; line numbers drift, grep the symbol.

Goal: a mod adds a new mob type end to end with no vanilla template: registration, behaviour, rendering, network,
saves. It must be at least as capable as Forge + GeckoLib and go beyond it (hot reload, built in, server-authoritative
animation sync, debug view, automatic asset sync on join). With no mods installed, vanilla mobs, network, saves and
rendering stay byte-for-byte unchanged (section 9).

Assumed author tools: Blockbench (`.bbmodel`, and its GeckoLib/Bedrock export: `.geo.json` + `.animation.json`). Both
formats load directly; nothing is converted by hand.

No Minecraft source, game files or GeckoLib code are copied here. Behaviours are described in our own words; vanilla ids,
file formats and JSON field names are used.

---------------------------------------------------------------------------------------------------------------------

## 1. Current state inventory

### 1.1 Data route (route D)

| What | Where | Note |
|---|---|---|
| Content kinds under `data/<id>/horizon/` | `crates/horizon-mods/src/content.rs:1066-1080` | `entity_type` is one of six kinds. |
| `ENTITY_TEMPLATES` (1 item: snowball), `MOB_TEMPLATES` (8) | `content.rs:419`, `:424` | The only templates. `EntityType::is_mob` at `:733`. |
| `struct EntityType` (template, texture, width/height/eye_height, attributes map, behaviours, loot_table, spawn, spawn_egg) | `content.rs:691-730` | Eight attribute names only; `SpawnRule` (biomes, weight, min, max) at `:664`. |
| `enum Behaviour` (9 goals: float, wander, look_at_player, random_look, panic, tempt, flee, melee_attack, target_player, hurt_by_target) | `content.rs:587` | Declarative, maps 1:1 onto goal structs. |
| Mob type construction: copy the template `MobType`, patch texture/box/attrs | `crates/horizon-client/src/mobs/mod_mob.rs:96` (`add`), goals attached in `register` `:158` | Model, animation family, sounds, spawn rules, species behaviour are the template's. |
| Network ids of mod entity types: appended after vanilla's in `registries.json` | `crates/horizon-client/src/mod_content.rs:1496-1520` | Same for items/blocks/effects/menus. |
| Identity hash covers `minecraft:entity_type` mod entries | `crates/horizon-client/src/content_identity.rs:112-118` | A mismatching client cannot join. |
| Documented limits | `docs/mods.md:327-405` | "No Blockbench or JSON model path for mod entities yet"; a vanilla client "may be disconnected". |

### 1.2 Engine mob model (server and client are one code base)

| What | Where |
|---|---|
| `enum Species` (closed, about 100 variants; 2451 `Species::` uses in 149 files) | `crates/horizon-client/src/mobs/species.rs:7` |
| `struct MobType` (box, attributes, flags, `model: Arc<Part>`, `layers: FxHashMap<String, Arc<Part>>` keyed `model#layer`, `texture`, `brain`, `anim`, sounds) | `crates/horizon-client/src/mobs.rs:365-419` |
| `struct Mob` (position/previous, yaw/body/head angles, health, hurt/death time, limb animation, `extra`, `attrs`, `ai`, `needs_sync`) | `mobs.rs:1210-1330` (`attrs` at `:1325`, `extra` at `:1269`) |
| `struct Part` (name, pose `[x,y,z,rx,ry,rz,sx,sy,sz]`, polygons, children): the model tree | `mobs.rs:232` |
| Goal system: `trait Goal` (flags, can_use, can_continue, start, stop, tick), `GoalSelector` with four locked flag slots and priorities | `mobs/goals.rs:24`, `:80` |
| Navigators: `enum NavKind { Ground, WallClimber, Water, Flying, Amphibious, ... }`, path finder, `MoveKind` | `mobs/nav.rs:11`, `mobs/control.rs:20` |
| Brains (animals, monsters): memories, sensors, activities | `mobs/animal_brain.rs:335`, `mobs/monster_brain.rs:110`, `mobs/brain.rs` |
| Spawn rules per species | `mobs/spawn_rules.rs:127-317`, `mobs/spawn_selection.rs`, `game/natural_spawning.rs` |
| Entity NBT save/load | `crates/horizon-client/src/entity_nbt.rs:335` (`mob_to_nbt`), `:1355` (`put_brain`) |
| Multi-part precedent (dragon wings/heads) | `mobs/dragon_contacts.rs:7-20`; mobs otherwise have one box (`Mob::dims`, `mobs.rs`) |
| Server tick: `tick_creatures` then `tick_creature_entities` (an inject point `game::creatures::tick_creature_entities`, side server), 20 Hz, only in entity-ticking chunks | `game/creatures.rs:915-948`, ticking area `:938` |
| Mod spawn and control services (native API): `native_spawn_mob` (kind must already be a known mob type), `native_mob(s)`, teleport, push, health, kill, remove | `game/native_mobs.rs:166-299` |
| External control of ground walkers (claim, move_to, find_path, look, jump) | `mobs/native_control.rs:165-298`, `game/native_mob_navigation.rs` (`docs/native-mods.md:3259`) |

### 1.3 Rendering

| What | Where |
|---|---|
| A mob frame is drawn on the CPU into `Vec<Vertex>` each frame (`mobs::render::draw(part, base, poses, uv, color, light, overlay, out)`); no instancing or skinning | `mobs/render.rs:1374` |
| Per-mob culling by distance and frustum before drawing | `game/creatures.rs:6800-6830` |
| Mod hook point: `native_mob_submit` asked each frame for every mob in view but players; `drawn=true` skips the vanilla model; name tag, shadow, fire, outline still drawn | `game/native_mob_draw.rs:381` (`mod_mob_geometry`), `:473`, `creatures.rs:6880`, `game/native_hooks.rs:3473` |
| `native_mob_layer` adds geometry in the entity frame (no bone access) | `native_hooks.rs:3514` |
| `native_draw_model` draws a baked standalone block model; model commands `quad`, `cube`, `block`, `model`, `item` | `game/native_block_entity_draw.rs:593`, `yoake-full.wit:890-915` |
| `native_standalone_models` bakes extra block-model ids at resource load | `native_hooks.rs:3210` |
| OptiFine `.jem` loader builds `Part` from boxes, with a small animation language | `mobs/cem.rs` (1365 lines), `mobs/cem_anim.rs` (1248 lines) |
| Keyframe animation runtime used for vanilla animations (position/rotation/scale channels, catmull-rom) | `mobs/keyframes.rs:1-60` |
| One entity atlas for all entity textures; shader `native_entity.rs` samples it | `crates/horizon-client-render/src/gfx.rs:1118`, `:4253`, `gfx/native_entity.rs:55-60` |
| Runtime textures (`gui-texture`, 64 per mod) are HUD or world quads only; cannot be an entity skin | `yoake-mod.wit:154-160`, GAP2 row "Dynamic textures" |
| Emissive precedent: `FULL_BRIGHT` light constant used for layers | `mobs/render.rs:43` |

### 1.4 Network

| What | Where |
|---|---|
| Host builds `AddEntity` with the entity type's registry id from the built-in registry | `crates/horizon-client/src/lan/hub.rs:499` (`entity_type()`) |
| Client resolves the id back to a name, then `spawn_entity` | `game/remote.rs:3509-3545` |
| Entity data applied by vanilla's index scheme | `game/remote_mob_data.rs:15-40` |
| Brand check: `minecraft:brand` is `"horizon"` for Horizon clients; non-Horizon clients are disconnected when custom item components exist or `horizon_clients_only` is set | `lan/conn.rs:998-1040` |
| Horizon-only side channels precedent (a stock client is sent nothing) | `lan/attack_hitbox.rs:1-30` (`horizon:attack_hitbox`), `game/lan_particles.rs:40-60` (mod particles only to Horizon clients) |
| Join-time mod fetch (hash pinned, host scoped, full-access included) | `docs/yoake2/JOIN-MODS.md:1-47` |

### 1.5 WASM surfaces

| What | Where |
|---|---|
| Sandboxed (S) hooks: `mob-spawned`, `mob-killed`, `mob-effect-added`, `potential-spawns`, `mob-split`, `mob-submit`, `mob-layer`, `player-render-listened`, `standalone-models` | `yoake-full.wit:2679`, `:2683`, `:3284`, `:3586`, `:3606`, `:6860`, `:6877` |
| Only spawn service: `spawn-mannequin`, `remove-spawned-entity`; `draw-mob-as-player` | `yoake-full.wit:1999-2000`, `:2214` |
| Attachments on entities (persistent, synced, copy-on-death) | `yoake-full.wit:2049` (`attachment-define`) |
| `native-mob-render` record (frame data a renderer reads) | `yoake-full.wit:1216` |
| Full-access (F): hooks of the bridged `mobs` module (68 hooks), bound getters/setters of `Mob` fields | `docs/yoake2/MOD-GUIDE.md:116-117`, `docs/yoake2/bindgen/functions-mobs.md` |
| Dev hot reload of client WASM modules; server modules are not hot-reloaded | `docs/yoake-wasm-dev.md:121-135`, `:455-456` |

### 1.6 Gap summary (matches GAP2 rows 30-31 and item 5)

No registrable entity type beyond 8 templates; no model, texture or animation path for mod mobs; no goal code; no custom
attributes; no generic spawn service; no animation controllers; no multi-hitbox; no synced mod data on entities; a vanilla
client's behaviour on a modded entity is undefined.

---------------------------------------------------------------------------------------------------------------------

## 2. Design principles

1. **Registration is static data; behaviour is code by id.** A custom type is a JSON file (it needs a network id before
   the connection, exactly as items and blocks do). WASM only registers implementations by id (goals, hooks).
2. **One shared immutable definition per type** (`Arc<CustomType>`), loaded identically on client, host and dedicated
   server; per-mob state is small and separate.
3. **Server authoritative.** Gameplay, hitboxes, AI, synced data and the animation clock are server-side. The client
   renders from synced state and the server's game tick.
4. **Pure model/animation crate with no GPU**, so a dedicated server can read the same files (hitbox follow, timeline
   events) without a window.
5. **Everything new is gated by `kind.custom.is_some()`** or by an empty registry. No mod, no new code path executed.
6. **New code lives in new files** (`mobs/custom/`, `game/custom_mob_*.rs`, new crate) so parallel coders do not collide.

---------------------------------------------------------------------------------------------------------------------

## 3. Target architecture

### 3.1 Crates and modules

| New | Purpose |
|---|---|
| crate `horizon-mobmodel` (no GPU, no game deps; name reserved, created in F4M-1) | `.geo.json`, `.bbmodel`, `.animation.json` parsers; internal `Model` and `Animation`; expression compiler and VM; animation controllers; pose evaluation; bake of bone-local vertices. Used by client and server. |
| `horizon-mods/src/custom_mob.rs` | JSON schema types for `"template": "yoake:mob"` (serde, validation, limits). `content.rs` only dispatches. |
| `horizon-client/src/mobs/custom/` | `registry.rs` (type table), `build.rs` (`Mob` from `CustomType`), `attrs.rs`, `hitbox.rs`, `spawn.rs`, `persist.rs`, `ai/` (goals, brain, nav, attacks, riding, tame, breed), `anim_sync.rs`, `procedural.rs`. |
| `horizon-client/src/game/custom_mob_draw.rs` | Client draw path (pose to vertices), layers, items on locators, shadow, LOD, debug overlays. |
| `horizon-client/src/lan/mob_sync.rs`, `game/remote_custom_mob.rs` | Server send and client apply of synced values, animation triggers, hitbox state. |
| `horizon-client/src/wasm_mods/mob_services.rs` | Sandboxed host functions and exports for custom mobs. |
| `crates/yoake-wasm/wit/yoake-mobs.wit` (draft) | New additive WIT (section 6.2). |

Shared enum edits are limited to: one new variant `Species::Custom`, one new field `MobType.custom: Option<Arc<CustomType>>`,
and one dispatch line at each of the draw, spawn, NBT and network sites listed in section 7.

### 3.2 Type registry (server and client)

`CustomType` (immutable): id, category, box and hitbox parts, attribute defaults and custom attribute definitions, spawn
and despawn rules, loot table id, sound events, synced value schema, AI definition (data goals, brain, script goal ids),
model reference (`ModelRuntime`: bones, baked vertices, locators, animations, controllers, layers, LOD table), network
proxy settings, debug name. Built once when content loads (`mod_mob.rs::add` path), stored in the same map as vanilla
`MobType`s so `res.mob_types()` finds it. Its `Species` is `Custom`; every `Species`-keyed table has a `Custom` row of
neutral defaults (no sunburn, no special sounds, no baby form) that the definition overrides.

Network id: appended to `minecraft:entity_type` in `registries.json` as today (`mod_content.rs:1496`). The identity
hash already covers it.

### 3.3 Server behaviour layers (all optional, mixable)

1. **Data goals**: the existing nine become about thirty (section 6.1). Each is a native `Goal` struct, so zero WASM cost.
2. **Script goals**: a goal id declared in JSON with priority and control flags; its body is a WASM export called as
   `can-use`, `can-continue`, `start`, `tick`, `stop`. An engine adaptor `ScriptGoal: Goal` forwards to the guest.
   Controls: a goal claims `move`, `look`, `jump`, `target` flags exactly as vanilla goals do (four slots,
   `goals.rs:80`). Goals declare an `interval` (evaluate `can-use` every N ticks) to bound cost.
3. **Brain option**: declarative sensors fill typed memories on an interval; activities (named groups of goals) switch on
   memory predicates; scripts read and write memories. Built over the same `Goal` objects (an activity enables a group),
   so there is one execution engine, not two.
4. **Full external control**: the existing `native_control` ownership (suspend selectors, script drives move/look/jump/
   path) is exposed per custom mob, extended from "ordinary ground walkers" to every navigator kind.

Navigators: `ground`, `climb`, `swim`, `fly`, `amphibious` map to the existing `NavKind`; move control kind is chosen
by the same field. Look, move and jump controls are the existing ones.

### 3.4 Hitboxes

`hitboxes` is a list of parts: `{ name, size, offset, rotation_yaw_follow, bone?, damage_multiplier, pick, collide,
hurt }`. The first part (or the primary `size`) is the entity box used for world collision, spawning space and mob
pushing (the engine has exactly one collision box per mob; extra solid collision parts are a documented non-goal, see
risks). Extra parts are hurt and pick boxes, in the way the dragon's wings and heads are (`dragon_contacts.rs`), with a
per-part damage multiplier and the part name passed to the hurt hook. A part with `bone` follows that bone: the server
evaluates only the tracks of that bone's chain from the shared model, at the mob's synced animation time, once per tick
and only while the mob is within range of a player. A part without `bone` is a static offset turned with the body.
A Horizon client's crosshair uses the same parts through a horizon-only channel (precedent `horizon:attack_hitbox`).

### 3.5 Synced data and animation state

Per type, JSON declares up to 32 tracked values: `bool`, `i32` (min, max), `f32`, `string` (at most 64 bytes), `vec3`,
`enum` (names, one byte). Each has `default`, `persistent`, `sync` (`all`, `owner`, `none`), and `interp` (client lerps a
float). The server changes a value (script or native code) and marks it dirty; at the end of the tick the sync layer
sends a delta per mob to tracking players. The client keeps the value, raises a `synced-changed` event (script hook and
animation controller input), and restores everything for a late joiner from a full snapshot sent right after `AddEntity`.

Animation state is itself synced data of the engine: per mob a list of controller states (u8 each) and a ring of
one-shot **triggers** `(trigger_id: u16, start_game_tick: i64, speed: u8)`. The client computes
`t = (server_game_time + partial - start_tick) / 20` and evaluates, so a late joiner or a lagging client lands on the
right pose, animation never depends on client frame rate, and a server pause (single-player menu) pauses animation.

### 3.6 Model and animation runtime (`horizon-mobmodel`)

Inputs (all loaded from the mod's `assets/<ns>/`):

* `geo.json`: `format_version` 1.12.0 and newer and the older 1.8 layout. Fields used: `description` (`identifier`,
  `texture_width`, `texture_height`, `visible_bounds_*`), `bones[]` (`name`, `parent`, `pivot`, `rotation`, `mirror`,
  `inflate`, `neverRender`, `locators`, `cubes[]`, `poly_mesh`), `cubes[]` (`origin`, `size`, `pivot`, `rotation`,
  `inflate`, `mirror`, `uv` as box offset or per-face `{north,east,south,west,up,down: {uv, uv_size, uv_rotation}}`).
* `bbmodel`: `meta`, `resolution`, `elements[]` (`cube`: `from`, `to`, `origin`, `rotation`, `inflate`, `box_uv`,
  `uv_offset`, `faces`; `mesh`: `vertices`, `faces` with per-vertex `uv`; `locator` and `null_object`), `outliner` tree
  (groups with origin and rotation, child order), `textures[]` (embedded base64 PNG or path), `animations[]`
  (`length`, `loop` once/hold/loop, `animators` per group with `keyframes` of channel rotation/position/scale,
  `interpolation` linear/catmullrom/bezier/step, bezier handles; an effects animator for sound and particle keyframes).
* `animation.json`: `animations{}` with `loop` (true, false, `hold_on_last_frame`), `animation_length`,
  `override_previous_animation`, `anim_time_update`, `blend_weight`, `bones{}` with `rotation`/`position`/`scale` as a
  constant, a vector of expressions, or time-keyed tables with `pre`/`post`, `lerp_mode` (`linear`, `catmullrom`,
  `bezier`, `step`) and easing names (the common sine/quad/cubic/quart/quint/expo/circ/back/elastic/bounce families,
  in/out/in-out), plus `sound_effects`, `particle_effects`, `timeline` (event names).
* Animation controllers: `animation_controllers/<name>.json` (our own simple format, section 6.1): states, animations
  per state, transitions on expressions, `blend_transition` seconds, per-state speed. Several controllers run as layers
  (for example `locomotion`, `action`, `head`), each with a weight and a bone mask.

Output of one frame: an array of per-bone local transforms (translation, rotation as quaternion, scale), then composed
into matrices by the skeleton in parent order. Rotation order and axis conventions follow Blockbench's Bedrock export;
the loader tests pin them with golden poses.

**Expression language** (safe subset of the Molang family): numbers, `+ - * / %`, comparisons, `&& || !`, ternary,
parentheses; `math.` functions (sin, cos, abs, clamp, lerp, min, max, floor, ceil, round, sqrt, pow, mod, trunc,
sign, random with a per-mob seed); read-only inputs `query.anim_time`, `query.life_time`, `query.ground_speed`,
`query.vertical_speed`, `query.yaw_speed`, `query.head_x_rotation`, `query.head_y_rotation`, `query.body_y_rotation`,
`query.health`, `query.max_health`, `query.is_on_ground`, `query.is_in_water`, `query.is_baby`, `query.hurt_time`,
`query.death_time`, `query.is_riding`, `query.has_target`, `query.time_of_day`, and `query.synced('name')` for tracked
values; `variable.*` per-mob floats assigned by controllers or scripts. Aliases `q.`, `v.`, `m.`. No loops, no
functions, no strings except the synced name literal (resolved to a slot at compile time), no allocation. Compiled to
a flat bytecode at load; hard cap 256 instructions per expression; invalid expressions are rejected at load with the
file and column, never at run time. This covers what GeckoLib-style files use; unsupported Molang constructs are
listed by the loader as warnings with the animation name.

### 3.7 Client renderer registry

`mob_draw` asks: `if let Some(custom) = mob.kind.custom` then `custom_mob_draw`, else today's path (the one test in the
existing hot loop). `native_mob_submit` still runs first; a script that cancels replaces the engine renderer (full
custom renderer escape hatch). Steps per visible mob:

1. Cull by the type's `cull` box (separate from the hitbox) with the existing frustum code.
2. LOD choice from distance (table in JSON).
3. Evaluate controllers, layers and procedural constraints into bone transforms; optional script `mob-pose`.
4. Compose matrices; for each visible bone, transform its pre-baked vertices into the output vertex buffer.
5. Extra passes from the same pose: emissive layer, translucent layer, items on locators, armor on mapped bones.
6. Name tag at `name_tag_height`, shadow (`shadow.radius`, `shadow.strength`), fire, outline as today.

The atlas: mod textures are added to the entity atlas when packs load (as the `texture: "<ns>:entity/..."` field does
now). A bbmodel's embedded textures are extracted at load into the same path. Optional per-mob variants
(`textures.variants`) are chosen by a synced `variant` value.

### 3.8 Runtime entity textures

New reserved sub-region of the entity atlas (for example 1024x1024, 64 slots) updated with partial uploads: services
`mob-texture-create(width, height) -> id`, `mob-texture-write(id, rect, rgba)`, `mob-texture-bind(entity, slot, id)`.
A bound runtime texture replaces the `default` texture or a named layer for that mob. Replacing an existing texture file
(hot reload) is the same upload. Capped per mod like `gui-texture` (64 slots, 4 MiB per write).

### 3.9 Network protocol

Registered types appear in the entity-type registry like today, so a Horizon client decodes `AddEntity` unchanged.
New data rides on a **Horizon-only side channel**, `horizon:mob_sync` (clientbound custom payload, a stock client is
sent nothing, same pattern as `horizon:attack_hitbox`), with three message kinds sent after the entity exists on the
client (the connection preserves order):

* `snapshot(entity, type_version, synced values, controller states, trigger ring, hitbox enabled mask)` right after `AddEntity`.
* `delta(entity, dirty mask, changed values)` at the end of each tick, merged per mob.
* `trigger(entity, trigger_id, start_tick, speed)` for one-shot animations (immediate).

Frame header carries the server game time used for animation. Per-mob cost is bounded: at most 256 bytes of synced
values, at most 10 deltas per second per mob by default (`sync_rate` per value), distance-gated like entity tracking.

**Vanilla client joins a Yoake host:**

* A type with `proxy` (a vanilla entity id such as `minecraft:pig`, optional `proxy_name_tag`) is sent to a stock client
  as that vanilla type, with only valid vanilla entity data indices; the stock client sees a pig where the mod has a
  fox, can hit it, and nothing breaks. Hitboxes beyond the proxy's box are not visible to it.
* A type without `proxy` makes the host require a Horizon client when any such type is loaded. This extends the existing
  `horizon_clients_only` gate (`lan/conn.rs:1020`) so the player is refused with the existing mod-mismatch text instead
  of being disconnected later when a spawn decode fails.
* Spawn packets for non-proxy types are never sent to a stock client in any case (defence in depth).

**Horizon client joins a stock or unmodded server:** its local registry additions are unused (the server's registry
sync decides ids); no custom mob exists; join-time identity checks handle mismatched content as for blocks. Needs
checking (section 11).

**Assets:** a mod's models, animations, controllers and textures are inside its `.yak`, so the join-time mod fetch
(`JOIN-MODS.md`: hash pinned, host scoped, automatic) delivers them with the registration. Nothing extra is needed. The
host advertises the mod's sha256; a client whose copy matches has identical assets, hence identical animation.

**Integrated server** (single player, LAN host): same code, the client reads server state directly, no payload. The
paths are tested the same through the side channel by the two-client e2e.

### 3.10 Persistence

Entity id is the type id; position, health and ordinary fields use the existing NBT. Custom state lives in one
namespaced compound `yoake` inside the entity tag: `{ v: type_version, synced: {...}, anim: {...}, attachments: {...},
memories: {...}, script: <nbt> }`. No other keys are added, and a vanilla entity never gets a `yoake` key.
A world opened without the mod keeps the saved custom entity as raw NBT and writes it back unchanged (needs checking
how `mob_to_nbt`/loading treat an unknown id today: snowball precedent says frozen in place; mobs must be verified).
Attribute values are saved with modifiers by id.

### 3.11 Hot reload and debug

Hot reload levels (development mode, same switch as WASM dev reload):

* **R1 assets** (geo, bbmodel, animation, controllers, textures, glowmasks): a file watcher rebuilds `ModelRuntime` and
  swaps the `Arc` at the next frame, on the client only; running mobs keep their animation clock. Failure keeps the old
  model and logs one warning with file and line. No server involvement needed, since assets are client-side.
* **R2 definitions** (JSON data): `/yoake mob reload [type]` rebuilds the `CustomType` and applies it to live mobs
  (attributes recomputed, goal list rebuilt, hitboxes replaced; synced schema changes are refused with a message unless
  compatible). New spawns use it immediately.
* **R3 WASM**: client modules already hot-reload. Server modules are not hot-reloaded today (`yoake-wasm-dev.md:455`);
  because mob state lives in the engine, not in the guest, reloading a server module for custom mobs is feasible by
  swapping the instance at a tick boundary and re-registering goal ids. Included as a work item with a Needs-checking
  flag.

Debug view (`/yoake mobdebug [on|off] [hitboxes] [bones] [anim] [ai]` and a key chord in `debug_keys.rs`), drawn with the
existing gizmo layer (`gizmos.rs`): all hitbox parts with names and colour per part role; skeleton lines and bone names
(billboard text); locators; per layer the animation name, time and weight, controller state and the transition that
fired; synced values; running goals, memory table, current path and target. `/yoake mobs` prints per type counts,
cost per frame (animation, vertex build) and server tick cost per type from the existing span timers
(`spans::time`).

---------------------------------------------------------------------------------------------------------------------

## 4. Rendering extras in detail

* **Layers**: `layers[]`: `{ name, texture, mode: cutout|translucent|additive|emissive, bones? (mask), tint expression,
  visible expression }`. `emissive` draws the same geometry at full light with alpha test (a `_glowmask.png` next to the
  texture is picked up as an emissive layer automatically when present). Translucent layers sort after opaque, as the
  existing translucent entity layers do.
* **Held items and armor**: geo `locators` (and bbmodel locators) named in `item_slots` map `mainhand`, `offhand`, `head`
  to a transform; the held item is drawn with the existing item draw path. Armor: bones named in an `armor` map to the
  six humanoid armor parts; the vanilla armor models are drawn on those bones. First delivered: held items and head item;
  armor on mapped bones is a later item.
* **Per-bone control** (both native and script): visibility, additive or absolute translation/rotation/scale, set per
  mob; bones are addressed by index resolved from names at load, so the per-frame script call passes integers.
* **Procedural constraints** (native, declarative, no WASM cost): `look_at` (bone, yaw and pitch limits, speed, weight,
  target: view/target/owner), `aim`, `two_bone_ik` (root, mid, tip bones, target locator or point, pole), `spring`
  (tail, ears, hair: stiffness, damping, gravity, collision off), `follow` (copy another bone's motion with delay),
  `counter_rotate` (keep a bone upright against its parent). Evaluated after animations. This is the beyond-GeckoLib part.
* **Animation events**: client side, keyframed sound and particle effects fire at their time at the locator position
  (particle ids map to vanilla particles or mod particles through the type's `particles` table; sound ids are normal
  sound events); `timeline` names fire the client hook `mob-anim-event`. **Server side**, animations flagged
  `server_events: true` also emit their `timeline` names on the server clock, so an attack's damage frame is the
  keyframe of the swing, not a guessed tick count; clients do not duplicate gameplay effects.
* **Shadow, cull, LOD**: `shadow {radius, strength}`, `cull {width, height, y_offset}`, `lod [{distance, mode}]` with
  modes `full`, `half_rate` (animation every second frame, interpolated), `quarter_rate`, `low_bones` (skip bones tagged
  `lod_min`), `static` (rest pose), `hidden`.
* **Death, hurt flash, baby scale, name tag**: the engine's hurt tint, death tilt (or a `death` animation that holds the
  entity until it finishes, then the normal removal), baby scale and name tag height come from the type's fields.

---------------------------------------------------------------------------------------------------------------------

## 5. Behaviour details

* **Attributes**: the eight existing plus `flying_speed`, `step_height`, `gravity`, `jump_strength`, `safe_fall_distance`,
  `water_slowdown`, `attack_knockback`, `scale`; and **custom attributes** declared in the type or globally in
  `data/<id>/horizon/attribute/<name>.json` (`default`, `min`, `max`, `sync`). Values and modifiers (id, operation add/
  multiply base/multiply total) follow the engine's attribute implementation; scripts read, set base and add or remove
  modifiers.
* **Spawning**: `spawn` becomes a list of rules `{ biomes, structures, dimensions, weight, min, max, placement:
  on_ground|in_water|in_lava|on_surface|no_restrictions|custom, light: any|dark|bright|{sky_max, block_max}, time, y_range,
  min_distance_from_player, cluster }` plus `category` (`monster`, `creature`, `ambient`, `water_creature`,
  `water_ambient`, `underground_water_creature`, `misc`). `placement: custom` calls the script `mob-spawn-check`. The
  natural spawner uses the category's cap and spawn rules like vanilla's mobs of that category.
* **Despawn**: `despawn { persistent_by_default, immediate_distance, random_distance, near_player_distance_keep,
  age_limit }`. Naming, picking up, leashing and `persist` NBT behave as for vanilla mobs.
* **Spawn egg**: `spawn_egg: { primary, secondary }` generates `<id>:<name>_spawn_egg` with a template model and tinting
  (no item file needed); `spawn_egg: true` keeps the old meaning (the mod declares the item).
* **Loot, sounds, name**: loot table, sound events (`ambient`, `hurt`, `death`, `step`, `attack`, `swim`, `fly`), lang key
  `entity.<id>.<name>` as today.
* **Attacks**: data goals `melee_attack`, `ranged_attack` (projectile type, speed, spread, interval, arc),
  `leap_attack` (windup animation, impulse, damage frame), `area_attack` (radius, damage, knockback, shape), all with a
  trigger animation and optional damage frame (from a server timeline event). Projectiles use existing projectile code;
  a custom projectile entity kind is a separate stretch item (F4M-17).
* **Riding and passengers**: `seats [{ offset | locator, controls: none|player, allowed_types, yaw_lock }]`, `saddle_item`,
  `rideable_when`. The player seat follows a locator on the client (smooth) and the static offset on the server.
* **Interaction**: right-click opens the hook `mob-interact(mob, player, hand, item) -> consume|pass|success`; data
  shortcuts: `feed`, `tame`, `ride`, `milk`-like `give_item`.
* **Taming and breeding**: `tameable { items, chance, sits_on_command, follow_owner }` and `breedable { food, partner,
  offspring, cooldown, baby_scale }` implemented once natively and used by data goals (`follow_owner`, `sit`,
  `owner_hurt_target`, `breed`), with hooks `mob-tamed`, `mob-bred`.
* **Targeting and sensors**: target selectors (`nearest_attackable(types, must_see, range)`, `hurt_by`, `owner_target`),
  sensors for the brain option (`nearest_players`, `nearest_living(types)`, `hurt_by`, `blocks(tag, radius)`, `light`).

---------------------------------------------------------------------------------------------------------------------

## 6. Mod API per route

### 6.1 Data route (JSON, no code)

`data/<id>/horizon/entity_type/<name>.json`, `"template": "yoake:mob"` (a new accepted value; the old templates are
unchanged). Fields are optional unless stated. Full sample in section 10.

```
{
  "template": "yoake:mob", "format": 1,
  "category": "creature",
  "size": { "width": 0.6, "height": 0.7, "eye_height": 0.5 },
  "hitboxes": [ { "name": "head", "size": [0.4,0.4], "bone": "head", "damage_multiplier": 1.5 } ],
  "attributes": { "max_health": 12, "movement_speed": 0.3, "yoake:fury": { "default": 0, "max": 10, "sync": "all" } },
  "model": { "geo": "<ns>:geo/x", "textures": { "default": "<ns>:entity/x", "variants": [] },
             "animations": ["<ns>:animations/x"], "controllers": ["<ns>:controllers/x"],
             "scale": 1.0, "shadow": {"radius":0.4}, "cull": {"width":2,"height":2},
             "layers": [], "lod": [], "item_slots": {}, "procedural": [], "particles": {} },
  "synced": { "state": {"type":"enum","values":["idle","sit","pounce"]} },
  "ai": { "navigator": "ground", "goals": [ { "type": "wander", "priority": 5, "speed": 1.0 } ],
          "targets": [], "sensors": [], "script_goals": [] },
  "spawn": [ ... ], "despawn": { }, "spawn_egg": { "primary": "0xD2691E", "secondary": "0xFFF0A0" },
  "loot_table": "<ns>:entities/x", "sounds": { }, "tameable": { }, "breedable": { }, "seats": [ ],
  "proxy": "minecraft:fox"
}
```

Existing `Behaviour` values keep working; new goal `type`s: `swim`, `fly_wander`, `climb`, `leap_at_target`, `ranged_attack`,
`area_attack`, `follow_owner`, `sit`, `owner_hurt_target`, `owner_hurt_by`, `breed`, `follow_parent`, `avoid`, `eat_block`,
`go_to_block`, `sleep_in_bed_like(light)`, `patrol_around`, `stay_near_home`, `open_door`, `retreat_when_hurt`,
`call_for_help`, `random_swim`, `nearest_target(types)`, and `script` (a script goal id).

Animation controller file (ours):

```
{ "controllers": { "locomotion": { "initial": "idle",
    "states": { "idle": { "animations": ["idle"], "transitions": [ { "to": "walk", "when": "q.ground_speed > 0.05" } ], "blend": 0.15 },
                "walk": { "animations": [ { "name": "walk", "speed": "q.ground_speed * 4" } ], "transitions": [ { "to": "idle", "when": "q.ground_speed < 0.03" } ] } } },
    "action": { "weight": 1, "bones_mask": [], "triggers": { "pounce": "pounce" } } } }
```

### 6.2 Sandboxed WASM (additive minor; draft)

New file `yoake-mobs.wit` imported by the worlds, package version bumped by one minor. Guest-to-host variant growth
(a new `model-command` case such as `skeleton(...)`, new `host-error` is not needed) is forward-compatible; host-to-guest
records and enums are never extended in place, new records are added instead. New permission `service.mobs`.

Server imports (`mobs-server`): `spawn(type, dimension, pos, opts) -> result<u32>`, `info(entity) -> option<mob-info>`,
`set-synced(entity, key, value)`, `get-synced`, `trigger-anim(entity, controller, name, speed)`, `set-controller-state`,
`navigate-to(entity, pos, speed, opts)`, `navigate-stop`, `look-at`, `jump`, `set-target(entity, option<entity>)`,
`attack(entity, target, damage, knockback)`, `attribute-get/set-base/add-modifier/remove-modifier`, `memory-get/set`,
`set-hitbox-enabled(entity, name, bool)`, `data-get/put(entity, nbt)`, `seat-set(entity, seat, rider)`, `despawn`,
`claim-control(entity, bool)`.

Server exports (guest implements what it registered): `goal-can-use(mob, goal) -> bool`, `goal-can-continue`, `goal-start`,
`goal-tick`, `goal-stop`, `mob-tick(mob)`, `mob-spawn-check(type, dimension, pos) -> bool`, `mob-spawned(mob, reason)`
(existing hook extended by a new record, not by editing the old one), `mob-hurt(mob, part, source, amount) ->
hurt-outcome`, `mob-died`, `mob-interact(mob, player, hand) -> interact-result`, `mob-anim-event(mob, name)`,
`mob-load(mob, nbt)`, `mob-save(mob) -> nbt`, `mob-tamed`, `mob-bred`, `synced-changed-server(mob, key, old, new)`.

Client imports (`mobs-client`): `mob-texture-create/write/bind`, `bone-index(type, name) -> option<u32>`, `locator(entity,
name) -> option<transform>`, `play-local-anim` (cosmetic only). Client exports: `mob-pose(frame, bones) -> list<bone-write>`
(asked only for types flagged `procedural_script`, only inside the LOD range), `mob-anim-event-client(mob, name)`,
`synced-changed-client(mob, key, old, new)`; the existing `mob-submit` (and `draw-mob-as-player`) remain the full renderer
callback, and a new `model-command` case `skeleton(model-id, pose: list<bone-write>, texture-layer)` lets a script draw a
geo model it poses itself.

Budgets for guest calls: per-call timeout, at most 32 script goal evaluations per mob per tick, `interval` honoured,
batched `mob-think` available (`list<mob-view> -> list<decision>`) for mod authors who run big groups.

### 6.3 Full-access WASM

* New engine_call bindings generated by `yoake-bindgen` in a new module `custom_mobs` (ids appended to `ids.lock.json`,
  ABI minor bump): the same operations as 6.2 imports, with typed handles.
* The same exports as hooks: the guest registers goal and hook functions by name through `hook_register`; the engine
  adaptor `ScriptGoal` calls them. Because the guest cannot implement the Rust `Goal` trait, this adaptor is the one
  mechanism for both routes (no second implementation).
* Direct field access: the existing getters/setters of `Mob` fields (`mobs` module, 68 hooks) work on custom mobs, and
  new fields (`custom`, synced values, controller states) are bound. Mixin-style hooks on any engine mob function are
  already part of the F1 plan and apply to custom mobs.
* Full-access mods may also replace the whole renderer (they can hook `custom_mob_draw` stages: `before_pose`,
  `after_pose`, `after_vertices`).

---------------------------------------------------------------------------------------------------------------------

## 7. Integration points (the only edits to shared files)

| Shared file | Edit |
|---|---|
| `mobs/species.rs` | `Species::Custom` variant and its neutral default rows. |
| `mobs.rs` (`MobType`) | field `custom: Option<Arc<CustomType>>`, default `None` in every existing constructor (`:4246`, `:5432` and the builder at `:505-567`). |
| `mobs/mod_mob.rs` | `add` builds custom types from `yoake:mob` rows; `register` attaches goals. |
| `horizon-mods/src/content.rs` | accept `yoake:mob`, hand the object to `custom_mob.rs`. |
| `mod_content.rs` | register the type's network id and the generated spawn egg item. |
| `game/creatures.rs` (draw `:6880`, tick `:948`) | one `if custom` branch each. |
| `entity_nbt.rs` | one call to `custom::persist` for custom types. |
| `lan/hub.rs:499` / `lan/conn.rs:1020` | proxy mapping for stock clients; brand gate extension. |
| `game/remote.rs:3509` | after `spawn_entity`, nothing; custom data comes by payload. |
| `native_mobs.rs:166` | `native_spawn_mob` accepts custom ids (it already looks the kind up in the type table). |

All hook ids of new functions are appended to the existing hook id lock; no existing id changes.

---------------------------------------------------------------------------------------------------------------------

## 8. Performance budget

Target scene: 200 custom mobs in view range, 30 bones and about 400 vertices each, 60 fps on the dev machine's
reference settings, plus 200 mobs ticking on the server with three goals each.

| Stage | Budget (per frame) | How |
|---|---|---|
| Cull and LOD | 0.1 ms | existing frustum and distance tests; cull box per type. |
| Animation evaluation | 1.0 ms | bytecode expressions, sparse tracks, binary search with per-channel cursor, no allocation, skip by LOD (half or quarter rate for far mobs, rest pose beyond 64 blocks). About 6000 bones per frame at 150 ns each is 0.9 ms. |
| Matrix composition | 0.3 ms | flat parent-ordered array, `glam` Mat4. |
| Vertex generation | 1.5 ms | bone-local baked vertices; one matrix multiply per vertex (about 80k vertices when all 200 are visible). Hidden bones skipped. |
| Upload and draw | existing entity pass | a single buffer per frame as today. |
| Total CPU for custom mobs | 3 ms | leaves headroom in 16.6 ms. |
| Server tick | 5 ms of the 50 ms tick | native goals; script goals gated by `interval` and at most 32 evaluations per mob per tick (about 600 guest calls at 2 us is 1.2 ms); sensors on intervals; path search budgets (`max_visited` clamp exists); mobs outside entity-ticking chunks do not tick. |
| Server hitbox follow | 0.3 ms | only bones with a `bone` hitbox, only within player range, once per tick. |
| Network | 5 KB/s per player | deltas only, merged, rate limited, tracked range only. |

Memory: `ModelRuntime` shared (an `Arc` per type, baked vertices once). Per mob: animation state, small controller
array, pose buffer of 30 matrices (1.4 KB). 200 mobs is under 1 MB.

If CPU vertex generation is not enough: an optional **GPU skinning** stage (instanced draw, bone matrices in a storage
buffer, bone index in the vertex) moves vertex cost to the GPU and leaves 30 matrices per mob on the CPU. It needs a new
pipeline in `horizon-client-render`, interacts with shader packs and outline passes (`pack_post.rs:2045`), and is
therefore scheduled last (F4M-16) and kept optional behind the measured need.

Measurement: a bench scenario `custom_mobs_200` in `horizon-bench` (or an e2e in `yoake-test-runner`) that spawns the
sample mod's mob 200 times in a ring, records frame time percentiles and the span timers for `custom_anim`,
`custom_verts`, `custom_goals`; thresholds fail the item that regresses them.

---------------------------------------------------------------------------------------------------------------------

## 9. Vanilla parity: how to prove it

With no custom mob type loaded, `MobType.custom` is `None` for every type, the custom registry is empty, no side channel
is opened (`horizon:mob_sync` is only advertised when a type exists), no NBT key is added, no atlas region is reserved,
no file is written, and no registry entry is appended.

Proof, per work item (all run before and after, fixtures captured by F4M-0 before any engine change):

* **Golden A, saves**: spawn a fixed set of vanilla mobs with a fixed seed, run 2000 ticks, hash the entity NBT of every
  mob and the region file bytes. Compare.
* **Golden B, network**: scripted session; hash the bytes of registry sync, `AddEntity`, `SetEntityData` and movement
  packets sent to a stock-brand client and to a Horizon-brand client. A Horizon client must receive no `horizon:mob_sync`
  payload when no custom type exists.
* **Golden C, render**: for every vanilla mob type, fixed pose and frame, hash `Vec<Vertex>` output of the draw path
  (existing render tests are the base); also `registries.json` and `content_identity` output with zero mods.
* **Test D, tick and frame cost**: `horizon-bench` with the feature compiled in and unused is within 1 % of baseline.
* **Test E**: exhaustive `match` fallout of `Species::Custom` compiles with no behavioural arm change (reviewed by diff:
  only `_` arms and new rows).
* **Test F**: an unmodded world opened after a modded session keeps custom entities as raw NBT and rewrites them
  unchanged; a vanilla world's chunks are byte-identical after an open and save cycle.

---------------------------------------------------------------------------------------------------------------------

## 10. Sample mod and end-to-end acceptance test: "Cinder Fox"

Mod id `sample_mobs`, one mob `sample_mobs:cinder_fox`, built as a `.yak` with data, assets and one sandboxed WASM
module. Small on purpose.

Files:

* `data/sample_mobs/horizon/entity_type/cinder_fox.json`: category `creature`; size 0.6 by 0.7; hitboxes `body` and `head`
  (head bone, 1.5x damage); attributes (health 14, speed 0.3, custom `sample_mobs:warmth` 0 to 10, synced); synced
  `state` enum (`idle`, `sit`, `pounce`) and `warmth`; `ai`: `swim`, `script` goal `pounce`, `follow_owner`, `sit`,
  `wander`, `look_at_player`, `random_look`, targets `nearest_target(minecraft:chicken)`; `tameable` with
  `minecraft:sweet_berries`, `sits_on_command`; spawn in `#minecraft:is_forest`, weight 6, groups 1 to 3, daytime light;
  `spawn_egg` with two colours; `proxy: "minecraft:fox"`; sounds and loot table.
* `assets/sample_mobs/geo/cinder_fox.geo.json` and the source `cinder_fox.bbmodel` (both supported; the test loads each in
  turn): 14 bones (root, body, head, ears x2, snout, legs x4, tail x3), a locator `mouth` for the held item.
* `assets/sample_mobs/animations/cinder_fox.animation.json`: `idle` (loop), `walk` (loop, speed from ground speed),
  `run`, `sit` (hold), `pounce` (once, 0.8 s, `timeline` event `hit` at 0.35 s, sound and particle effects at 0.35 s),
  `death` (hold).
* `assets/sample_mobs/controllers/cinder_fox.json`: layers `locomotion` (idle/walk/run from `q.ground_speed`),
  `action` (triggers `pounce`), `pose` (`sit` from `q.synced('state')`).
* `assets/sample_mobs/textures/entity/cinder_fox.png` and `cinder_fox_glowmask.png` (emissive eyes and tail tip).
* `procedural`: `look_at` head (yaw 60, pitch 40), `spring` on tail bones, `two_bone_ik` on the front legs while sitting.
* `wasm/fox_ai.wasm` (sandboxed): goal `pounce`: `can-use` when the target is 3 to 6 blocks away with line of sight,
  `start` triggers the `pounce` animation and claims move, on the server timeline event `hit` applies a leap impulse and
  damage, `stop` releases. About 60 lines. A full-access variant `fox_ai_full` does the same through bindings.

Acceptance scenarios (each is one e2e in `yoake-test-runner`; all must pass):

1. **Data only**: remove the WASM module; the fox spawns by egg and `/summon`, wanders, looks at the player, walks and
   idles; hitbox parts appear in the debug view.
2. **Model formats**: the same pose is produced (within 1e-4 per bone matrix) from the `.geo.json` and the `.bbmodel`
   export, and a 30-frame golden pose table matches for every animation.
3. **Behaviour**: with the WASM module the fox pounces on a chicken, damage lands at the `hit` frame (server tick equals
   keyframe time plus or minus one), loot drops, the fox can be tamed with berries, follows and sits on command.
4. **Persistence**: save and quit while sitting with warmth 7; reload; state, synced values, owner and animation state are
   restored; open the world without the mod, save, reopen with it, and the fox is unchanged.
5. **Multiplayer**: host plus two clients; client B joins without the mod, fetches it automatically (`JOIN-MODS.md`), and
   sees the same fox in the same animation frame within one tick of the server clock; a late joiner during a pounce sees
   the correct pose; synced `warmth` changes raise one `synced-changed` event on B.
6. **Vanilla client**: a stock-brand client joins; it sees a vanilla fox at the fox's position (proxy), no disconnect;
   with `proxy` removed it is refused up front with the mod-required text.
7. **Hot reload**: edit the geo (move the ear), the animation (slower walk) and the glowmask while the game runs; the
   running fox changes within one second and keeps its animation phase; a malformed edit keeps the old model and logs one
   warning.
8. **Debug view**: shows bones, hitboxes, the controller state and the running goal; screenshot comparison with tolerance.
9. **Performance**: the scene `custom_mobs_200` meets section 8 thresholds.
10. **Parity**: goldens A to F pass with the mod uninstalled.

---------------------------------------------------------------------------------------------------------------------

## 11. Staged plan (parallel work items)

Naming `F4M-n`. Sizes: S under a day, M 1-3 days, L 3-7 days, XL over a week. Files are the ones primarily touched;
items in the same group have disjoint files so Sonnet, Kimi and Gemini coders can work at once. Do not build while the
disk or CPU is busy; item tests list `cargo test -p <crate>` only for crates the item touches.

Groups: G0 = F4M-0 (first, short). G1 = F4M-1, F4M-3, F4M-5a (disjoint). G2 = F4M-2, F4M-4a, F4M-4b, F4M-4c, F4M-5b,
F4M-7 after their dependencies. G3 = F4M-6, F4M-8, F4M-9, F4M-10 after G2. G4 = F4M-11, F4M-12, F4M-13, F4M-14.
G5 = F4M-15 and optional F4M-16, F4M-17, F4M-18.

### F4M-0 (critical): foundations, schema and parity goldens
* Goal: capture goldens A to C before any engine change; add `Species::Custom`, `MobType.custom`, empty registry,
  `custom_mob.rs` schema types with validation and limits (bones 256, cubes 4096, animations 128 per type, 32 synced
  values, 256 expression instructions), reserved hook and binding id ranges, the Cinder Fox fixture mod directory (data only stubs). The crate name `horizon-mobmodel` is
  reserved: it is created by the F4M-1/F4M-2 lane, not by F4M-0 (F4M-0 only puts the small `ModelInfo` trait in
  `horizon-mods/src/custom_mob.rs`, to move there; see `reports/FR-F4M-0.md`).
* Files: `species.rs`, `mobs.rs` (constructors only), `horizon-mods/src/custom_mob.rs`, `content.rs` (dispatch),
  new `crates/horizon-mobmodel/`, `tools/fixtures/mob_parity/`, `ids.lock.json` reservation.
* Depends on: none. Size: M (the variant adds match fallout; land first, keep the branch short).
* Tests: build, `cargo test -p horizon-client -p horizon-mods`, golden A/B/C stored and passing, schema accept/reject tests.
* Done: zero behaviour diff, goldens committed, clippy clean.

### F4M-1 (critical): model loaders and bake
* Goal: `.geo.json` (1.12+ and 1.8), `.bbmodel` (cubes, meshes, outliner, embedded textures, locators) to the internal
  `Model`; bone-local baked vertex arrays; UV box and per-face; mirror, inflate, rotation; mesh triangulation; limits and
  clear errors with file and path; fuzz tests on malformed input.
* Files: `crates/horizon-mobmodel/src/{geo,bbmodel,model,bake}.rs`, tests.
* Depends on: F4M-0. Size: L. Tests: golden vertex sets for a fixture cube, mirrored cube, per-face UV, mesh, nested
  pivots; bbmodel and geo of the same fox compare equal.
* Done: both formats load the Cinder Fox into identical bone matrices at rest.

### F4M-2 (critical): animation runtime and expression language
* Goal: `animation.json` and bbmodel animators; interpolation modes and easing families; loops, hold, override,
  blend; expression compiler and VM (section 3.6); controllers and layers with bone masks; pose evaluation; server-side
  timeline event emission; sparse evaluation for bone subsets (for hitbox follow).
* Files: `crates/horizon-mobmodel/src/{anim,expr,controller,pose}.rs`.
* Depends on: F4M-0 (interface), F4M-1 for real models (can start on synthetic skeletons). Size: L.
* Tests: golden pose tables, expression property tests (no panic, bounded cost), controller transition tests, easing
  tables, step and bezier edge cases.
* Done: Cinder Fox animations produce the golden pose tables; invalid expressions rejected at load.

### F4M-3 (critical): server registry and `Mob` construction
* Goal: build `CustomType` from JSON; `Mob` from type; custom attributes; hitbox parts (hurt and pick); categories;
  spawn rules and despawn; auto spawn egg item; `/summon`; loot table, sounds, name; NBT `yoake` compound with unknown-type
  preservation; `native_spawn_mob` for custom ids.
* Files: new `mobs/custom/{registry,build,attrs,hitbox,spawn,persist}.rs`, small edits in `mod_mob.rs`, `spawn_rules.rs`,
  `spawn_selection.rs`, `entity_nbt.rs`, `mod_content.rs`, `native_mobs.rs`.
* Depends on: F4M-0. Size: L. Tests: spawn, tick, save and load round trip, world without mod, attribute modifier math,
  hitbox part damage multiplier, spawn rule unit tests, parity goldens.
* Done: a data-only fox spawns, persists, drops loot and spawns naturally in the right biomes.

### F4M-4a: goals, navigators and attacks (data)
* Goal: the new goal catalogue (section 6.1), navigator selection, attack goals with damage frame hook, target selectors.
* Files: `mobs/custom/ai/{goals,attacks,targets,nav_select}.rs`, `horizon-mods/src/custom_mob.rs` (goal enum rows only).
* Depends on: F4M-3. Size: L. Tests: one deterministic test per goal on a flat world; flags conflicts; path budget.

### F4M-4b: brain option (sensors, memories, activities)
* Goal: declarative sensors and memories, activities as goal groups, memory persistence, script access.
* Files: `mobs/custom/ai/{brain,sensors,memory}.rs`.
* Depends on: F4M-3. Size: M. Tests: sensor interval, memory expiry, activity switching.

### F4M-4c: riding, interaction, taming, breeding
* Goal: seats, saddle, `mob-interact` plumbing, tame and breed natives and their goals.
* Files: `mobs/custom/ai/{riding,interact,tame,breed}.rs`, edit-free of the vanilla `tame.rs`.
* Depends on: F4M-3. Size: M. Tests: tame chance, owner follow, breeding offspring and cooldown, passenger position.

### F4M-5a (critical): network, proxy and brand gate
* Goal: proxy mapping for stock clients; brand gate extension; `AddEntity` for custom types; no-send rules.
* Files: `lan/hub.rs` (the `entity_type` call site), `lan/conn.rs` (gate), new `lan/mob_proxy.rs`.
* Depends on: F4M-3. Size: M. Tests: stock-brand sees proxy type only, refuses non-proxy types with the text, parity B.

### F4M-5b (critical): synced data, animation sync, side channel
* Goal: `horizon:mob_sync` snapshot, delta, trigger; client apply; change events; late join; rate limits; persistence
  of synced values; server tick clock for animation.
* Files: new `lan/mob_sync.rs`, `game/remote_custom_mob.rs`, `horizon-protocol` channel constants, `mobs/custom/anim_sync.rs`.
* Depends on: F4M-3, F4M-2 (trigger ids). Size: L. Tests: encode and decode, delta merge, late join, loss and reorder
  tolerance of the state, byte budget test, parity B (no channel without custom types).

### F4M-6 (critical): client renderer
* Goal: `custom_mob_draw`: pose to vertices, atlas inclusion of mod textures, layers (cutout, translucent, emissive),
  locators and held items, per-bone state, shadow, cull, LOD, name tag, hurt and death, outline; integration at the
  draw site with `native_mob_submit` override still honoured.
* Files: new `game/custom_mob_draw.rs`, `mobs/custom/render_state.rs`; edits at `creatures.rs:6880` and the atlas loader
  (`packs.rs` texture list only).
* Depends on: F4M-1, F4M-2, F4M-3, F4M-5b for synced input. Size: L. Tests: golden vertex hash for the fox at fixed
  frames, LOD mode tests, culling tests, parity C.
* Done: the fox animates correctly in single player (scenario 1 visuals).

### F4M-7: runtime entity textures
* Goal: reserved atlas region, partial uploads, `mob-texture-*` services, bind per mob and per layer, replace on hot
  reload.
* Files: `horizon-client-render/src/gfx.rs` (atlas region), `wasm_mods/client_services.rs` (new functions only), new
  `game/custom_mob_textures.rs`.
* Depends on: F4M-6 (layer binding) for the bind; the atlas region part can start at once. Size: M.
* Tests: write and read-back, slot limits, atlas rebuild keeps slots, shader pack toggle.

### F4M-8: procedural constraints and `mob-pose`
* Goal: native look-at, aim, two-bone IK, spring, follow, counter-rotate; script pose hook with integer bone ids.
* Files: `mobs/custom/procedural.rs`, `crates/horizon-mobmodel/src/ik.rs`, WIT draft rows.
* Depends on: F4M-2, F4M-6. Size: M. Tests: IK reach and pole tests, spring stability, limit clamps.

### F4M-9: animation events (client effects and server timeline)
* Goal: keyframed sounds and particles at locators; `timeline` to client hook; server `server_events`; attack damage
  frame wiring for `melee_attack` and `leap_attack`.
* Files: `mobs/custom/events.rs`, `game/custom_mob_events.rs`.
* Depends on: F4M-2, F4M-3, F4M-6. Size: M. Tests: event fires once per loop, not on pause, deterministic on server.

### F4M-10 (critical): sandboxed WASM API
* Goal: `yoake-mobs.wit` (additive), host imports and exports, `ScriptGoal` adaptor, budgets, permission
  `service.mobs`, docs rows, fox_ai module.
* Files: new `crates/yoake-wasm/wit/yoake-mobs.wit`, `wasm_mods/mob_services.rs`, `mobs/custom/ai/script_goal.rs`,
  `crates/horizon-mod-api-gen` rows, `docs/yoake-wasm-dev.md` (services table).
* Depends on: F4M-3, F4M-4a, F4M-5b. Size: L. Tests: guest crash and timeout handling, budget limits, wit conformance,
  scenario 3.

### F4M-11: full-access bindings and exports
* Goal: bindgen module `custom_mobs`, guest SDK stubs, hook registration of goals and hooks, renderer stage hooks.
* Files: `crates/yoake-bindgen`, `ids.lock.json`, `crates/yoake-guest-sdk/src/custom_mobs.rs`, `wasm_mods/full/` adapters.
* Depends on: F4M-10 (semantics frozen). Size: M. Tests: binding generation snapshot, full-access fox variant (scenario 3).

### F4M-12: debug view and commands
* Goal: `/yoake mobdebug`, `/yoake mobs`, key chord, gizmo drawing, overlay text.
* Files: new `game/custom_mob_debug.rs`, `debug_keys.rs` (one entry), `gizmos.rs` (use only).
* Depends on: F4M-6. Size: M. Tests: draw lists are produced, no-op when off, no cost when off.

### F4M-13: hot reload
* Goal: R1 asset watcher and swap, R2 `/yoake mob reload`, R3 server module swap at tick boundary (flagged), failure
  handling, per-mob phase preservation.
* Files: new `mobs/custom/reload.rs`, `game/custom_mob_reload.rs`, `wasm_mods` dev-reload glue.
* Depends on: F4M-6 (R1, R2), F4M-10 (R3). Size: M. Tests: scenario 7.

### F4M-14: performance pass
* Goal: bench scenario, tune LOD defaults, sparse evaluation, vertex build loop (SIMD where it measurably helps), write
  the measured numbers into the doc.
* Files: `horizon-bench`, `horizon-mobmodel` (inner loops), `game/custom_mob_draw.rs` (hot loop only).
* Depends on: F4M-6, F4M-2. Size: M. Tests: scenario 9, goldens unchanged.
* **Measured (F4M-14, report `reports/FR-F4M-14.md`)**: `custom_mobs_200` is the ignored test in
  `mobs/custom/render_state.rs` (`cargo test -p horizon-client-lib --lib custom_mobs_200 -- --ignored --nocapture`,
  release, headless, 200 Cinder Foxes, 300 frames after 30 warm-up). Release, spread scene (0..120 blocks), median of 5:
  pose (update) 0.17 ms, vertices (draw) 0.78 ms, total 0.95 ms; p95 total 1.06 ms. Near scene (10..30 blocks, all Full
  LOD): pose 0.42 ms, vertices 0.78 ms, total 1.21 ms. Section 8 budgets: animation 1.0 ms, vertices 1.5 ms, custom
  total 3 ms: all met. 124,800 vertices per frame in both scenes. Server tick not measured (the bench has no server).
  Tuning kept: none; see the report for what was tried and rejected.

### F4M-15 (critical, last): kit, docs and acceptance
* Goal: Cinder Fox complete, Blockbench workflow guide (EN and JA), MOD-GUIDE chapter, `docs/mods.md` rewrite of the mob
  section, SDK kit example, all ten acceptance scenarios as e2e.
* Files: `docs/mods.md`, `docs/yoake2/MOD-GUIDE*.md`, sample directory, `yoake-test-runner` scenarios.
* Depends on: everything above except optional items. Size: L.

### F4M-16 (optional): GPU skinning
* Goal: instanced skinned draw with a bone matrix buffer, shader pack and outline compatibility, fall back to CPU path.
* Files: `horizon-client-render/src/gfx/` new pipeline and shader. Depends on: F4M-14 (only if budget misses). Size: XL.

### F4M-17 (optional): custom projectile and non-mob entity kinds (arrow-like, area effect, display entity)
* Same registration, hit and render path without AI. Depends on: F4M-3, F4M-6. Size: L.

### F4M-18 (optional): armor on mapped bones, bone-attached translucent effects
* Depends on: F4M-6. Size: M.

### Critical path

Two long lanes meet at F4M-15:

* Render lane: F4M-0, F4M-1, F4M-2, F4M-6, F4M-9, F4M-14, F4M-15.
* Server lane: F4M-0, F4M-3, F4M-5b, F4M-10, F4M-13, F4M-15.

The shortest total with five parallel coders is about the length of the render lane (about 16 coder-days) plus the
final acceptance item. F4M-0 must land first (about two days). Everything between F4M-0 and F4M-15 is independent
across lanes except where "Depends on" says otherwise.

---------------------------------------------------------------------------------------------------------------------

## 12. Risks

1. **Species coupling**: 2451 `Species::` uses may include exhaustive matches that carry vanilla assumptions for a
   `Custom` value (sunburn, sounds, brain defaults, drops). Mitigation: F4M-0 takes the compile fallout first and adds
   neutral rows; review only the diff of arms.
2. **One collision box**: multi-part hitboxes are hurt and pick boxes, not solid collision. Authors expecting a big
   dragon with wings that block the player will find the primary box only. Documented; solid parts are a separate later
   change.
3. **Unknown entity preservation in saves** may not exist for mobs (only projectiles were verified frozen). Data loss
   risk for users who uninstall a mod; this is the first thing F4M-3 tests.
4. **Vanilla client decode failures** if any custom-only packet or id leaks. The proxy and gate plus golden B guard it.
5. **Entity atlas limits and rebuild cost** with many models, runtime texture region, and shader packs. Needs measurement.
6. **Molang compatibility**: real mod files may use constructs outside the safe subset; the loader warns, never
   evaluates unknown constructs. Some files will need small edits.
7. **bbmodel variety** (older formats, free-form models with non-box rotations on meshes, bezier handle semantics).
   Mitigation: fixtures from several Blockbench versions, strict warnings.
8. **Server-side bone evaluation** for hitboxes could drift from the client pose if time bases differ. Mitigation:
   one shared crate and the synced game-tick clock, pose parity tests between server and client evaluation.
9. **Script cost**: 200 mobs with script goals can blow the tick. Mitigation: `interval`, per-mob evaluation caps,
   batched `mob-think`, watchdog that disables the script goal of a type that exceeds its budget and logs it.
10. **WIT evolution**: additive minor only; host-to-guest records are never edited. Wrong choices here are hard to fix.
11. **Server WASM hot reload** (R3) may be harder than described if guests hold state outside the engine.
12. **Disk and CPU are busy** on the dev machines; builds of the large client crate are slow. Keep new code in the new
    crate (fast to test alone) wherever possible.

---------------------------------------------------------------------------------------------------------------------

## 13. Needs checking

* How `entity_nbt` and chunk load treat an unknown mob id today (preserved raw or dropped).
* Whether a stock client ignores `SetEntityData` entries with an unknown index (would allow synced data without a side
  channel); the design assumes it does not and uses the side channel.
* Exact wire effects of a Horizon client with a locally installed mod joining a stock server (registry ids, identity
  check) and whether an extra local entity type id can cause a mismatch.
* The count of exhaustive `match` expressions over `Species` and over `MobType` constructors.
* Entity atlas size, free space and what a texture add does to atlas rebuild time; shader pack `entity` passes with a
  second layer pass.
* How `Mob.attrs` represents modifiers and whether arbitrary named attributes are possible without a type change.
* Whether guest exports can be declared optional in the current WIT worlds (a guest lacking `mob-pose` must be fine) and
  how `hook_register` names exports for full access.
* Whether `native_mob_control` can drive flying, swimming and climbing navigators or only ground walkers, as its header
  says ("ordinary ground walkers").
* Armor drawing: whether `mobs/armor.rs` is tied to vanilla part names so bone mapping is cheap.
* Whether entity picking (crosshair) and damage on a server use one box everywhere (dragon is the only multi-part
  precedent) and how many sites need the part-aware variant.
* Dedicated-server loading of a mod's `assets/` for hitbox follow and timeline events (assets are client-side today).
* Whether `horizon-bench` can spawn and render mobs headless or the performance scene must live in the e2e runner.
* Server game time continuity across pause for the animation clock in single player.
* Unity of the `Species::Custom` with `cem.rs` (OptiFine models should not apply to custom types; confirm the layer key
  scheme keeps them apart).
