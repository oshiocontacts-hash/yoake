# 14. Creating Custom Mobs (F4M)

With F4M, a `.yak` package can add **original custom mobs**—with their own model, animations, hitboxes, AI, and vehicle seats—without copying a vanilla mob template. Blockbench `.bbmodel` files and Bedrock `.geo.json` / `.animation.json` files load directly.

> [!NOTE]
> Place the mob definition, model, animations, and textures under `content/` as JSON and PNG files, and write custom AI or event reactions in a `server-full` (and optionally `client-full`) Wasm module. Declare `"service.mobs"` in `[wasm] permissions`.

---

## 14-1. Quick Start with the `custom-mob` Template

Run `yoake new custom-mob` in your workspace folder to scaffold a working custom mob project:

```sh
yoake new custom-mob my-mob
cd my-mob
yoake build
```

It creates this folder layout:

```text
my-mob/
  yak.toml
  Cargo.toml
  src/lib.rs                                               <- server-full ScriptGoal "pounce" code
  content/
    horizon-mod.toml                                       <- api = "0.2", capabilities = ["registries"]
    data/my_mob/horizon/entity_type/starter_mob.json       <- Mob definition ("template": "yoake:mob")
    assets/my_mob/
      geo/starter_mob.geo.json                             <- Bedrock geometry
      animations/starter_mob.animation.json                <- Animations
      controllers/starter_mob.json                         <- Animation controllers
      textures/entity/starter_mob.png                      <- 64x32 texture
```

- Hyphens (`-`) in `<name>` become underscores (`_`) in the content namespace (e.g., `my-mob` becomes `id = "my_mob"`, and the entity type ID is `my_mob:starter_mob`).
- The generated `yak.toml` sets:

```toml
[side]
client = true
server = true
required_on_join = false

[wasm]
interface = "1.7"
server = "bin/wasm/my-mob.wasm"
permissions = ["service.mobs"]
```

---

## 14-2. Main Fields of `entity_type/*.json`

Define your mob in `content/data/<ns>/horizon/entity_type/<name>.json` with `"template": "yoake:mob"` (excerpt below; see [`tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json`](../../tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json) for a full file):

```json
{
  "template": "yoake:mob",
  "format": 1,
  "category": "creature",
  "size": { "width": 0.6, "height": 0.7, "eye_height": 0.5 },
  "hitboxes": [
    { "name": "body", "size": [0.6, 0.7] },
    { "name": "head", "size": [0.4, 0.4], "bone": "head", "damage_multiplier": 1.5 }
  ],
  "attributes": {
    "max_health": 14,
    "movement_speed": 0.3
  },
  "model": {
    "geo": "sample_mobs:geo/cinder_fox",
    "textures": { "default": "sample_mobs:entity/cinder_fox" },
    "animations": ["sample_mobs:animations/cinder_fox"],
    "controllers": ["sample_mobs:controllers/cinder_fox"],
    "server_events": ["pounce"],
    "shadow": { "radius": 0.4, "strength": 0.8 }
  },
  "synced": {
    "state": { "type": "enum", "values": ["idle", "sit", "pounce"], "default": "idle", "persistent": true }
  },
  "spawn_egg": { "primary": "0xD2691E", "secondary": "0xFFF0A0" },
  "proxy": "minecraft:fox"
}
```

- **`size` and `hitboxes`**: `size` is the single world collision box. `hitboxes` adds hurt/pick parts; setting `bone` makes a part follow that model bone, and `damage_multiplier` scales damage taken on that part.
- **`synced`**: Values automatically synced from server to clients (`bool`, `i32`, `f32`, `string`, `vec3`, `enum`; up to 32 per type and 256 bytes total). Animation controller expressions and client mods can read them.
- **`spawn_egg`**: `{ "primary": "0x...", "secondary": "0x..." }` generates `<ns>:<name>_spawn_egg` automatically.
- **`proxy`**: A vanilla entity ID (such as `"minecraft:fox"`) shown to vanilla clients if they join. If any custom mob omits `proxy`, only Horizon clients can join.

---

## 14-3. Models and Animations

- **Model (`model.geo`)**: Loads `assets/<ns>/geo/<name>.geo.json` (Bedrock 1.8 or 1.12+) or `assets/<ns>/geo/<name>.bbmodel` (Blockbench project directly).
- **Animations (`model.animations`)**: Loads `assets/<ns>/animations/<name>.animation.json`. Listing an animation in `server_events` also fires its timeline events (such as `"hit"`) on the server as `hook::MOB_ANIM_EVENT`.
- **Emissive layers**: Place `<name>_glowmask.png` next to the texture, or add a layer with `"mode": "emissive"` in `model.layers`.
- **Controllers (`model.controllers`)**: `assets/<ns>/controllers/<name>.json` defines states, transition expressions, blending, and one-shot triggers (excerpt from [`cinder_fox.json`](../../tools/fixtures/cinder_fox/content/assets/sample_mobs/controllers/cinder_fox.json)):

```json
{
  "controllers": {
    "locomotion": {
      "initial": "idle",
      "states": {
        "idle": {
          "animations": ["idle"],
          "transitions": [{ "to": "walk", "when": "q.ground_speed > 0.05" }],
          "blend": 0.15
        },
        "walk": {
          "animations": [{ "name": "walk", "speed": "q.ground_speed * 4" }],
          "transitions": [{ "to": "idle", "when": "q.ground_speed < 0.03" }]
        }
      }
    },
    "action": {
      "weight": 1,
      "triggers": { "pounce": "pounce" }
    }
  }
}
```

---

## 14-4. Writing AI with `ScriptGoal` and `service.mobs`

### JSON Side (`ai`)

You can mix built-in data goals (`swim`, `wander`, `look_at_player`, `random_look`, `follow_owner`, `sit`, `nearest_target`, etc.) with `"type": "script"` goals, plus the brain option (`memories`, `sensors`, `activities`, `schedule`).

```json
"ai": {
  "navigator": "ground",
  "memories": { "pounced": { "type": "bool" } },
  "goals": [
    { "type": "swim", "priority": 0 },
    { "type": "script", "id": "pounce", "priority": 2 },
    { "type": "wander", "priority": 6, "speed": 1.0 }
  ],
  "script_goals": [
    { "id": "pounce", "priority": 2, "flags": ["move", "look"], "interval": 10 }
  ]
}
```

### Rust Side (`server-full` + `service.mobs`)

Use `yoake_wasm_sdk::full::server::mobs` to inspect state, steer movement, attack, trigger animations, and read or write memories and synced values. Excerpt from the `custom-mob` template:

```rust
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{mobs, ServerMod};

impl ServerMod for Starter {
    fn subscriptions() -> &'static [&'static str] {
        &[
            hook::MOB_GOAL_CAN_USE,
            hook::MOB_GOAL_CAN_CONTINUE,
            hook::MOB_GOAL_START,
            hook::MOB_GOAL_TICK,
            hook::MOB_GOAL_STOP,
        ]
    }

    fn mob_goal_can_use(&mut self, mob: u32, goal: String) -> bool {
        if goal != "pounce" {
            return false;
        }
        let Ok(Some(info)) = mobs::info(mob) else { return false };
        info.type_id == "my_mob:starter_mob" && info.on_ground && info.target_visible
    }

    fn mob_goal_start(&mut self, mob: u32, goal: String) {
        if goal == "pounce" {
            let _ = mobs::navigate_stop(mob);
            let _ = mobs::trigger_anim(mob, "", "pounce", 1.0);
        }
    }
    // Implement mob_goal_can_continue, mob_goal_tick, and mob_goal_stop as well.
}
```

---

## 14-5. Vehicle Seats (`seats`)

Add a `seats` array (up to 8 seats) in `entity_type/*.json` so players or other mobs can ride your custom mob:

```json
"seats": [
  { "offset": [0.0, 0.8, 0.0], "controls": "player", "yaw_lock": true },
  { "locator": "mouth", "controls": "none", "allowed_types": ["minecraft:chicken"] }
]
```

- `offset`: Position `[x, y, z]` relative to the mob origin. You can instead specify `"locator"` to follow a model locator.
- `controls`: `"player"` (driver seat) or `"none"` (passenger seat).
- `allowed_types`: Optional list of entity IDs allowed to board that seat.
- From server code, call `mobs::seat_set(mob_id, seat_index, Some(rider_id))` to mount a mob and `None` to dismount it.

---

## 14-6. Fox Examples and Debug Commands

The repository includes three Wasm examples paired with the [`tools/fixtures/cinder_fox/`](../../tools/fixtures/cinder_fox/README.md) fixture:

- [`crates/yoake-wasm-sdk/examples/fox_ai/`](../../crates/yoake-wasm-sdk/examples/fox_ai/src/lib.rs) — Server-side: `ScriptGoal` `"pounce"`, leaping and biting on the `"hit"` animation event, and updating `warmth` when hurt.
- [`crates/yoake-wasm-sdk/examples/fox_texture/`](../../crates/yoake-wasm-sdk/examples/fox_texture/src/lib.rs) — Client-side (`client-full` + `service.mobs`): repaints the fox's skin dynamically on `SYNCED_CHANGED_CLIENT` using `mob_texture_create` / `mob_texture_write` / `mob_texture_bind`, and draws a gold cube and apple via `mob_layer`.
- [`crates/yoake-wasm-sdk/examples/seat_fox/`](../../crates/yoake-wasm-sdk/examples/seat_fox/src/lib.rs) — Server-side: automatically mounts nearby chickens onto seat 1 of `sample_mobs:cinder_fox` using `mobs::seat_set`.
- (Reference) Full-access version of `fox_ai`: [`crates/yoake-guest-sdk/examples/fox_ai_full.rs`](../../crates/yoake-guest-sdk/examples/fox_ai_full.rs).

In-game debug and development commands:

- `/yoake mobdebug [on|off] [hitboxes] [bones] [anim] [ai]` (or `F3+M`) — Visualizes hitboxes, skeleton bones, animation states, and AI paths.
- `/yoake mobs` — Prints per-type rendering and pathfinding cost.
- `/yoake mob reload [type]` — Reloads JSON and model files while running in developer mode (`YOAKE_WASM_UNSIGNED=1`).

For the full reference, see [`docs/yoake2/MOD-GUIDE.md` section 17](../yoake2/MOD-GUIDE.md#17-custom-mobs-and-the-brain-option-f4m-4b) and [`docs/mods.md`](../mods.md#custom-mobs-f4m).
