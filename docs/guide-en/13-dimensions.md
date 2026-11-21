# 13. Dimensions, levels and chunk generators

Yoake manages dimensions (called **levels** inside the engine) the same way Minecraft 26.3 data packs do. You can ship dimension definitions as JSON in data packs or a mod's `.yak` / `.hmod` content, use a `server-full` WebAssembly mod to create and delete levels at runtime and write your own chunk generator in Rust, or subscribe to full-access (DLL) level and `ModChunk` hooks.

> [!NOTE]
> This chapter covers data-pack and mod-content JSON, **sandboxed WebAssembly mods (`server-full`)**, and **full-access (DLL) mods**. When no mod or data pack is installed, the three vanilla dimensions (`minecraft:overworld`, `minecraft:the_nether`, and `minecraft:the_end`) behave identically to vanilla.

---

## 13-1. Adding a dimension with data-pack or `.yak` JSON

To ship dimension data without writing code, place **`dimension_type`**, **`dimension`** (level stem), and optional **`worldgen/**`** JSON files in a world's data pack (`<world>/datapacks/<pack>/`) or inside a mod's `.yak` content (`content/data/<ns>/`).

### Data-pack folder layout and save location

For a dimension named `mypack:moon`, put the files in the data pack like this:

```text
<world>/datapacks/moon/
  pack.mcmeta
  data/
    mypack/
      dimension_type/
        moon_type.json       <- type id: mypack:moon_type
      dimension/
        moon.json            <- level id: mypack:moon
```

Chunks, entities, and POI data for the level are saved under the standard 26.3 path inside the world folder:

```text
<world>/dimensions/<namespace>/<path>/
  region/
  entities/
  poi/
```

For `mypack:moon`, that is `<world>/dimensions/mypack/moon/`.

### Shipping level data in a `.yak` (`content/data/<ns>/`)

A content mod or a `.yak` package's `content/` directory can ship level and worldgen files under its own namespaces (see [docs/mods.md](../mods.md#dimensions-and-world-generation-in-a-yak)):

```text
my_mod/
  yak.toml
  content/
    horizon-mod.toml               <- api = "0.2", capabilities = ["registries"]
    data/
      <ns>/                        <- mod id or a namespace listed in `namespaces`
        dimension_type/
          <name>.json              <- dimension type JSON
        dimension/
          <name>.json              <- level stem JSON
        worldgen/
          **.json                  <- biomes, noise settings, and other worldgen files
```

When content is read ([`crates/horizon-mods/src/content.rs`](../../crates/horizon-mods/src/content.rs)), the loader checks each file and **always prefixes error messages with the file path**:

- **Namespace fence**: Files under `dimension/`, `dimension_type/`, and `worldgen/` must stay within the mod's own namespaces (`id` or `namespaces`). Files in another namespace (such as `minecraft`) are refused:
  - Example: `data/minecraft/dimension/a.json: a mod adds files in its own namespaces (data/<id>/ and those listed in namespaces), to vanilla and convention (c) tags, or to vanilla's atlases`
- **JSON object required**: Every `.json` file under `dimension/`, `dimension_type/`, and `worldgen/**` must parse as valid JSON and be a JSON object (`{...}`):
  - Example: `data/ruby/worldgen/noise/a.json: expected a JSON object`
- **`dimension/<name>.json`**: Must define `"type"` (a dimension type ID string or inline object) and `"generator"` (a JSON object):
  - Example: `data/ruby/dimension/a.json: a dimension needs "type" (a dimension type id)`
  - Example: `data/ruby/dimension/a.json: a dimension needs a "generator" object`
- **`dimension_type/<name>.json`**: When present, `min_y`, `height`, and `logical_height` must be integers; `min_y` and `height` must be **multiples of 16**; `height` must be **16 to 4064**; and `logical_height` must be in **0 to `height`**:
  - Example: `data/ruby/dimension_type/a.json: min_y is a whole number`
  - Example: `data/ruby/dimension_type/a.json: min_y is a multiple of 16`
  - Example: `data/ruby/dimension_type/a.json: height is between 16 and 4064`
  - Example: `data/ruby/dimension_type/a.json: logical_height is within height`

### `dimension_type` example and what its attributes change

A `dimension_type` controls sky, light, weather, beds, lava, build height, and gameplay rules for the level (based on the test fixtures in [`level_registry_tests.rs`](../../crates/horizon-client/src/game/level_registry_tests.rs)):

```json
{
  "min_y": 0,
  "height": 128,
  "logical_height": 128,
  "has_skylight": true,
  "has_ceiling": false,
  "ambient_light": 0.3,
  "coordinate_scale": 1.0,
  "default_clock": "minecraft:overworld",
  "skybox": "overworld",
  "cardinal_light": "default",
  "timelines": "#minecraft:in_overworld",
  "attributes": {
    "minecraft:visual/sky_color": "#336699",
    "minecraft:visual/fog_color": "#223344"
  }
}
```

Here is what the main fields and `attributes` entries control:

| Area | Fields / attributes | What changes |
|---|---|---|
| Height | `min_y`, `height`, `logical_height` | Build range and chunk section count. The highest mob spawn Y is `min_y + logical_height - 1`. |
| Sky & day cycle | `skybox`, `has_skylight`, `has_ceiling`, `has_fixed_time`, `timelines`, `default_clock` | `skybox` is `"overworld"`, `"end"`, or `"none"`. A level has a day cycle (`day_cycle`) when `skybox` is `"overworld"` and either `has_fixed_time` is `false` or `timelines` is set. |
| Fog & sky visuals | `attributes`: `minecraft:visual/sky_color`, `fog_color`, `fog_start_distance`, `fog_end_distance`, `cloud_fog_end_distance`, `cloud_height`, etc. | Sets sky and fog colours, fog distances, and cloud height. Setting `fog_end_distance` enables environmental fog distance. |
| Light | `ambient_light`, `cardinal_light` | `ambient_light` is the base light floor. Setting `cardinal_light` to `"nether"` turns on Nether-style block shading and lightmap lighting. |
| Weather | `skybox` + `has_skylight` + `has_ceiling` | Rain and thunder occur only when `skybox == "overworld"`, `has_skylight == true`, and `has_ceiling == false`. |
| Beds & anchors | `attributes`: `minecraft:gameplay/bed_rule`, `straw_bed_rule`, `respawn_anchor_works` | `bed_rule` sets `can_sleep` (`"always"` / `"when_dark"` / `"never"`), `can_set_spawn` (`"always"` / `"when_dark"` / `"never"`), and `destroy_on_use`. `respawn_anchor_works: true` lets respawn anchors charge and work (the legacy top-level `respawn_anchor_works` field is also accepted). |
| Lava & water | `attributes`: `minecraft:gameplay/fast_lava`, `water_evaporates` | `fast_lava` speeds up lava flow; `water_evaporates` evaporates placed water and empties water buckets (legacy `"ultrawarm": true` sets both). |
| Mobs & raids | `attributes`: `minecraft:gameplay/piglins_zombify`, `snow_golem_melts`, `can_start_raid`, `monster_spawn_light_level`, `monster_spawn_block_light_limit`, `infiniburn` | Controls piglin/hoglin zombification (legacy `piglin_safe`), snow golem melting, raids (legacy `has_raids`), monster spawn light checks (`0..=15` constant or `minecraft:uniform`), and the `#`-prefixed infiniburn block tag. |
| Coordinate scale | `coordinate_scale` | Coordinate multiplier when moving between levels via portals or `/execute in` (Overworld is `1.0`, Nether is `8.0`). |

> [!NOTE]
> **Sea level (`sea_level`) comes from the generator, not from `dimension_type`.**
> `minecraft:noise` uses the `sea_level` from its `noise_settings`, `minecraft:flat` uses `-63`, and `minecraft:debug` uses `63`. For generators without one (such as `yoake:mod`), it falls back to `32` if `has_ceiling` is `true`, otherwise `63`.

### `dimension` (level stem) example

A `dimension/*.json` file binds a `dimension_type` to a `generator`:

```json
{
  "type": "mypack:moon_type",
  "generator": {
    "type": "minecraft:noise",
    "settings": "minecraft:overworld",
    "biome_source": {
      "type": "minecraft:fixed",
      "biome": "minecraft:plains"
    }
  }
}
```

`"type"` can be either a registered type ID such as `"mypack:moon_type"` or an inline dimension-type JSON object.

### Validation errors you will see

When a data pack or mod supplies an invalid dimension type or stem, the engine logs an error (`LevelProblem` / `LevelError`) and leaves that level out without breaking the three vanilla levels (even an invalid stem overriding a vanilla ID is reported while keeping the vanilla level working).

Common validation errors:

- **Invalid ID (`LevelError::BadId`)**: Because a level ID names a folder, it must be `namespace:path`, with namespace characters `[a-z0-9_.-]` (not `.` or `..`) and path characters `[a-z0-9_./-]` (no empty segments, `.`, `..`, uppercase letters, or spaces).
- **`min_y` and `height` rules**: Both `min_y` and `height` are required and must be **multiples of 16**, with `min_y >= -2032`, `height >= 16`, `min_y + height <= 2032`, and `0 <= logical_height <= height`.
- **Value bounds and types**: `coordinate_scale` must be finite and in `1e-5..=3e7`; `ambient_light` must be finite; `monster_spawn_block_light_limit` and `monster_spawn_light_level` must be in `0..=15`; `infiniburn` must start with `#`.
- **Unknown type or unsupported generator**: Naming a missing type gives `unknown dimension type`; an unknown `generator.type` gives `unsupported generator type`. A `yoake:mod` generator without a namespaced `"id"` (containing `:` and no whitespace) is also rejected.

---

## 13-2. Generators, biome sources, and per-level seeds

### The four generator types

`generator.type` in a level stem accepts four values:

1. **`minecraft:noise`**: Standard noise terrain. Give `"settings"` as a noise-settings ID (such as `"minecraft:overworld"`) or inline object, plus `"biome_source"`.
2. **`minecraft:flat`**: Superflat terrain. Put `"biome"`, `"layers"`, etc. inside `"settings"` (same format as vanilla). Works in any dimension, not only the Overworld.

```json
{
  "type": "minecraft:overworld",
  "generator": {
    "type": "minecraft:flat",
    "settings": {
      "biome": "minecraft:plains",
      "features": false,
      "lakes": false,
      "layers": [
        { "block": "minecraft:bedrock", "height": 1 },
        { "block": "minecraft:stone", "height": 2 },
        { "block": "minecraft:dirt", "height": 1 },
        { "block": "minecraft:grass_block", "height": 1 }
      ]
    }
  }
}
```

3. **`minecraft:debug`**: Debug world (barrier floor at `y = 60` and all block states). Requires no extra settings.

```json
{
  "type": "minecraft:overworld",
  "generator": { "type": "minecraft:debug" }
}
```

4. **`yoake:mod`**: A Yoake extension that calls a mod-registered chunk generator. Provide `"id"` (the registered generator ID) and an optional `"settings"` JSON object (defaults to `{}`). See Section 13-4.

### Biome sources (`biome_source`)

For `minecraft:noise`, `"biome_source"` supports:

- **`minecraft:fixed`**: One biome everywhere.
  ```json
  { "type": "minecraft:fixed", "biome": "minecraft:desert" }
  ```
- **`minecraft:checkerboard`**: Alternates biomes from a list or `#tag`. `scale` is `0..=62` (default `2`), where each cell is `1 << (scale + 2)` quarts wide (`scale: 0` is 16 blocks, or one chunk).
  ```json
  {
    "type": "minecraft:checkerboard",
    "biomes": ["minecraft:plains", "minecraft:desert"],
    "scale": 0
  }
  ```
- **`minecraft:multi_noise`**: Either an inline `"biomes"` list of parameter points (`temperature`, `humidity`, `continentalness`, `erosion`, `depth`, `weirdness`, `offset`) or a `"preset"` (`"minecraft:overworld"`, `"minecraft:nether"`, etc.).
- **`minecraft:the_end`**: The five-biome End layout.
- **`yoake:grid`** (Yoake extension): A fast rectangular biome grid with `cell_size` (multiple of 4), `origin` (`[x, z]`, multiples of 4), `width` and `depth` (`1..=256`), `cells`, `outside`, and optional `seed`.

### Per-level seed (`"seed"`)

In vanilla every level shares the world seed. In Yoake you can add `"seed"` (an integer or numeric string, e.g. `"seed": -77` or `"seed": "123"`) to a level stem to give that level its own seed; when omitted, the world seed is used.

For levels created at runtime with `create_level`, `regenerate_level(id, Some(new_seed))` can also assign a new seed and persist it.

---

## 13-3. Travel and the reconnect rule

### `/execute in` and `/tp`

Commands can target any level registered in the world's `LevelRegistry`:

- `/execute in mypack:moon run tp @s 0 80 0`
- `/execute in mypack:moon run tp @s ~ ~ ~`

`/execute in <level-id>` scales the command source position by **`coordinate_scale(from) / coordinate_scale(to)`**. Relative coordinates (`~ ~ ~`) in `/tp` reflect that scaled position, whereas explicit absolute coordinates (`0 80 0`) teleport directly to those coordinates. `/tp <targets> <entity>` also moves into the destination entity's level. Unknown level IDs fail with `argument.dimension.invalid`.

### Portal coordinate scale

`Game::travel_scale(from, to)` computes `coordinate_scale(from) / coordinate_scale(to)`, giving `1/8` from the Overworld (`1.0`) to the Nether (`8.0`) and `8` on the way back. However, **Nether portal pairing and End portals/gateways stay keyed to the three vanilla dimensions**, so custom levels do not form new portal destinations on their own; use mod teleports or commands to move players into custom levels.

### The reconnect rule for runtime-created levels

When a server creates a level with `create_level` while players are already connected, Minecraft's network protocol imposes a rule:

The `minecraft:dimension_type` registry is only sent during the login/configuration phase, so a brand-new dimension type cannot be pushed to an already-connected client.

- **Players who join afterwards**: Receive the created level and its dimension type at login and can enter immediately.
- **Already-connected players (existing type)**: If the new level uses a dimension type their client already received (such as `"minecraft:overworld"`, or a pack/staged type loaded before they joined), they **can enter without reconnecting** (a Horizon client interns up to 64 runtime-created levels on respawn).
- **Already-connected players (new type)**: If the new level introduces a new dimension type (such as an inline type object), teleporting into it is refused with a chat message telling the player to reconnect (`reconnect to enter it`). Once they disconnect and rejoin, they can enter.

---

## 13-4. WebAssembly mods: levels and chunk generators

A `server-full` WebAssembly mod can register dimension types, levels, and chunk generators at startup, and manage levels at runtime via `yoake_wasm_sdk::full::server::ext` and `yoake_wasm_sdk::full::generator`.

### `yak.toml` setup

Configure the package as a `server-full` module (from `crates/yoake-wasm-sdk/examples/level-generator/yak.toml`):

```toml
format = 1
id = "level-generator"
version = "0.1.0"
name = "Level Generator"
authors = ["Yoake"]
description = "Registers the level levelgen:terraces: a dimension type of its own (64 high, a blue sky) and a chunk generator written in WebAssembly."

[yoake]
version = "*"

[side]
client = false
server = true
required_on_join = false

[wasm]
interface = "1.0"
server = "bin/wasm/level-generator-server.wasm"
```

> [!NOTE]
> A module declared with `server = "..."` under `[wasm]` is always loaded in the `server-full` world. Do not write `world = "server-full"`: the manifest loader only accepts `client-mod`, `client-full`, or `inject-mod` for `world` and refuses `world = "server-full"`.

### Registering in `init`

Types, generators, and levels that should exist before the world opens are registered in `ServerMod::init`:

- `ext::register_type(id, type_json)`: stages a dimension type.
- `ext::register_generator(id)`: declares that this package generates chunks for stems with `"type": "yoake:mod", "id": id`.
- `ext::register_level(id, stem_json)`: stages a level (saved under `<world>/dimensions/<ns>/<path>/`).

Because `init` runs before the world loads and stages registrations for every world open, a player who logged out inside a mod level will reopen there on the next session. Calling `register_level` or `register_generator` in a hook after the world has ticked (`freeze()`) is refused.

### Runtime level lifecycle (persistent vs temporary)

Inside server hooks you can manage levels while the server is running:

| Function | Behavior and rules |
|---|---|
| `ext::create_level(id, stem_json, persistent)` | Creates a level at runtime (at most **64** created levels at once). `persistent == true` saves under `dimensions/<ns>/<path>/` and writes `<world>/data/yoake_levels.json` so it comes back next session. `persistent == false` (ephemeral) saves under `<world>/yoake_temp/<ns>/<path>/` and is wiped on delete or next world open. |
| `ext::delete_level(id)` | Deletes a `Created` level and its save folder. Refused if anyone is in or travelling to it (`Occupied`), if it is vanilla (`Vanilla`), or if it came from a pack, save, or `register_level` (`NotCreated`). |
| `ext::unload_level(id)` | Saves and closes an open non-vanilla level (freeing chunks and region files) while keeping it registered; it reopens from disk when someone enters it. Queued to run at the **start of the next server tick**. Refused if occupied or vanilla. |
| `ext::regenerate_level(id, seed)` | Closes a non-vanilla level, removes its `region`, `entities`, and `poi` folders, and keeps it registered under the same ID. Passing `seed: Some(s)` updates the seed on `Created` levels (persisted in `yoake_levels.json`); passing `Some(_)` on pack/mod/saved levels is refused (`NotCreated`). Refused if occupied or vanilla. |
| `ext::levels()` | Returns `Result<Vec<String>, String>` listing all registered level IDs (the three vanilla levels first). Also works in read-only server hooks. |
| `ext::current_level(player)` | Takes a player's UUID string (hyphenated or compact 32-hex) and returns `Result<String, String>` with the real ID of the level that player is currently in (e.g. `"minecraft:overworld"` or `"levelgen:terraces"`), or `Err("the player is not here")` if offline (service 235, `server-read`). Works in read-only server hooks. |

```rust
// Read each online player's real level ID (string ID, not the frozen 3-value enum).
for player in ext::server_players()? {
    let level_id = ext::current_level(&player.uuid)?;
    println!("{} is in {}", player.name, level_id);
}
```

### Walkthrough: the `level-generator` example

[`crates/yoake-wasm-sdk/examples/level-generator/src/lib.rs`](../../crates/yoake-wasm-sdk/examples/level-generator/src/lib.rs) registers a custom type, generator, and level in `init`, then generates terraced hills in `handle_call` (verified end-to-end across two processes by [`tools/levelgen_wasm_e2e.py`](../../tools/levelgen_wasm_e2e.py)):

```rust
use yoake_wasm_sdk::full::generator::{self, Fill, Request};
use yoake_wasm_sdk::full::server::{ServerMod, ext};

const TYPE: &str = r##"{
  "min_y": 0, "height": 64, "logical_height": 64, "has_skylight": true, "has_ceiling": false,
  "ambient_light": 0.2, "coordinate_scale": 1.0, "skybox": "overworld", "cardinal_light": "default",
  "attributes": {"minecraft:visual/sky_color": "#4466aa", "minecraft:visual/fog_color": "#8899cc"}
}"##;

const STEM: &str = r#"{
  "type": "levelgen:terrace_type",
  "generator": {"type": "yoake:mod", "id": "levelgen:terraces", "settings": {"step": 8}}
}"#;
```

In `init`, it registers all three:

```rust
fn init(&mut self) {
    for (what, result) in [
        ("type", ext::register_type("levelgen:terrace_type", TYPE)),
        ("generator", ext::register_generator("levelgen:terraces")),
        ("level", ext::register_level("levelgen:terraces", STEM)),
    ] {
        if let Err(e) = result {
            println!("level-generator: registering the {what}: {e}");
        }
    }
}
```

When a chunk needs to be generated, a chunk worker thread invokes **`handle_call`** with caller `"yoake:engine"` and method `generator::METHOD` (`"yoake:generate-chunk"`):

```rust
fn handle_call(&mut self, _caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
    if method != generator::METHOD {
        return Err("unknown method".into());
    }
    let request = Request::parse(&payload).ok_or("a request this mod cannot read")?;
    let step = request.setting_int("step").unwrap_or(8) as i32;
    let sections = (request.height / 16) as u16;
    let mut fill = Fill::new();
    fill.biome("minecraft:plains");
    for index in 0..sections {
        let bottom = request.min_y + i32::from(index) * 16;
        let mut indices = vec![0u16; 4096];
        let mut any = false;
        for z in 0..16 {
            for x in 0..16 {
                let top = surface(request.x * 16 + x, request.z * 16 + z, step, request.seed);
                for y in 0..16 {
                    let wy = bottom + y;
                    let block = if wy == request.min_y {
                        3 // bedrock
                    } else if wy < top - 1 {
                        1 // stone
                    } else if wy == top - 1 {
                        2 // grass
                    } else {
                        0 // air
                    };
                    if block != 0 {
                        any = true;
                    }
                    indices[((y * 16 + z) * 16 + x) as usize] = block;
                }
            }
        }
        if any {
            fill.palette(
                index,
                &["minecraft:air", "minecraft:stone", "minecraft:grass_block", "minecraft:bedrock"],
                indices,
            );
        }
    }
    Ok(fill.to_bytes())
}
```

### Chunk generator contract (`Request`, `Fill`, limits, and failures)

- **`Request` fields**: JSON carrying `level`, `generator`, `x`, `z`, `min_y`, `height`, `seed`, and `settings` (JSON string; `request.setting_int("key")` reads top-level integer settings).
- **`Fill` builder**:
  - `fill.biome("minecraft:plains")`: sets one biome for the whole chunk (defaults to `minecraft:plains` if omitted).
  - `fill.uniform(index, "minecraft:stone")`: fills section `index` (`0 .. height / 16`, measured up from `min_y`) with one block's default state.
  - `fill.palette(index, palette, indices)`: supplies `1..=4096` block names and `4096` `u16` indices ordered as **`(y * 16 + z) * 16 + x`** inside the section.
  - Omitted sections remain air. `fill.to_bytes()` produces the version-1 little-endian binary payload (max 1 MiB).
- **Pure function**: Compute blocks solely from `(x, z)`, `seed`, and `settings`. Server host services (world read/write, etc.) are unavailable during a generator `handle_call`.
- **Time limits and failure behavior**:
  - Per-call limits are the standard **25 ms calling-thread CPU deadline** (with the 20×, minimum-one-second wall-clock safety cap) and 10 million fuel units, **exempt from the 50 ms aggregate budget** so burst chunk generation at startup does not disable the mod (calls slower than 100 ms log a warning). A worker waits up to **2 seconds** if another hook holds the module mutex.
  - The host validates the entire `Fill` before writing any block (version `1`, flags, section bounds/duplicates, unknown block IDs, palette size `1..=4096` and index bounds, leftover bytes).
  - **Retry and `not_generated` safety**: If the generator call fails (busy module, trap, per-call limit exceeded, etc. — except when no server module is running), the engine **waits 50 ms (about one server tick) and retries once**. If the second attempt also fails, or the `Fill` fails validation, the chunk is left empty (all air, or whatever hooks wrote), **marked `not_generated`** (`ModChunk::mark_failed` → `Chunk::not_generated`), and logged once per level at `error` (further chunks at `debug`). Both `save` and `save_later` **skip `not_generated` chunks**, so the chunk regenerates on the next load. As soon as any block in that chunk is changed to a different state (`Chunk::set`), the `not_generated` mark is cleared so edits are saved. Traps and budget overruns disable the module just like any hook.

---

## 13-5. Full-access (DLL) hooks and `ModChunk` bindings

For full-access (DLL) mods, four injected server-side hook points ([`crates/yoake-wasm/wit/inject-sites.json`](../../crates/yoake-wasm/wit/inject-sites.json)) and `yoake-bindgen`-generated `ModChunk` bindings ([`crates/yoake-guest-sdk/src/stubs/mod_generator.rs`](../../crates/yoake-guest-sdk/src/stubs/mod_generator.rs)) are available. Every hook is a no-op unless a mod subscribes to it.

| Hook ID | Arguments | When it runs |
|---|---|---|
| `level::before_open` | `&mut Game`, level ID (`&str`) | Before a level that is not open is read from its generator and folders |
| `level::after_open` | `&mut Game`, level ID (`&str`) | After the level has opened, entered the open-levels registry, and mods heard `LevelEvent.Load` |
| `level::before_close` | `&mut Game`, level ID (`&str`) | Before `unload`, `delete`, or `regenerate` drops the level, while its chunks and files are still present |
| `worldgen::generate_chunk` | level ID (`&str`), `cx: i32`, `cz: i32`, `&mut ModChunk` | **Mod-generator (`yoake:mod`) levels only**, once per chunk before the mod generator fills it |

- **`worldgen::generate_chunk` leaves vanilla terrain untouched**: It never runs for the three vanilla dimensions or for `minecraft:flat`, `minecraft:debug`, or `minecraft:noise` levels.
- **Runs on a chunk-generating worker thread**: Keep `worldgen::generate_chunk` short; exceeding the module's per-call limit loses the chunk to the generator safety rule above (`not_generated`).
- **`ModChunk` bindings**: The `&mut ModChunk` argument is a scoped handle valid only for that call, exposed via generated stubs in [`crates/yoake-guest-sdk/src/stubs/mod_generator.rs`](../../crates/yoake-guest-sdk/src/stubs/mod_generator.rs):
  - Field getters: `mod_generator_modchunk_chunk_x_get`, `mod_generator_modchunk_chunk_z_get`, `mod_generator_modchunk_failed_get`
  - Chunk operations: `mod_generator_modchunk_block(h, x, y, z)`, `mod_generator_modchunk_set_block(h, x, y, z, state)`, `mod_generator_modchunk_fill_layer(h, y, state)`, `mod_generator_modchunk_set_biome(h, name)`, `mod_generator_modchunk_mark_failed(h)`
  - See [`docs/yoake2/MOD-GUIDE.md` section 16](../yoake2/MOD-GUIDE.md#16-levels-and-world-generation-f3-8b) and [`docs/yoake2/ENGINE_CALL_ABI.md`](../yoake2/ENGINE_CALL_ABI.md) for the full contract and ABI notes.

---

## 13-6. Limits and known gaps

1. **No decoration, structures, carvers, or LOD on `yoake:mod` generators**: `ModTerrain` writes final blocks and biomes directly with no feature/structure/carver/spawn pass on top and no far-terrain (LOD) sampler, so distant unloaded chunks look empty. Generator callbacks also cannot query climate biomes or density functions yet.
2. **No custom biomes**: Data packs cannot add new biome IDs that the game does not already know (they are dropped with a warning).
3. **Frozen `level-dimension` WIT enum**: To preserve the 1.0 ABI baseline, hooks carrying `et::LevelDimension` still report `overworld` for non-vanilla levels. Use **`ext::current_level(player)`** to look up which level a player is actually in, along with `ext::levels()` and string payloads (sounds, particles, loot).
4. **Shared border, clock, and some mob day checks**: Created levels have no separate world border and share parts of world time/weather state; hard-coded mob day-tick windows (fireflies, creaking, bees) do not yet read custom `timelines` tracks.
5. **Deleted created types persist until restart**: A deleted created level's `dimension_type` stays in the registry sent to new joiners until the server restarts so existing type indexes keep their meaning.
6. **Late-loaded server mods (`init` only at world start)**: `server-full` `init` runs at startup/world open; loading a server mod while a world is already open will not stage its `init` levels until the next world.
7. **Client biome blending on custom seeds**: The client's biome zoom uses the world's hashed `SpawnInfo.seed`, so visual biome-edge blending in a level with its own `"seed"` can differ slightly from the server's exact biome edges (placed blocks and server biome checks use the level's seed).
