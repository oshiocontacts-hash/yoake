# 4. Exploring the Example Mods

The repository includes **30 packaged examples** (passing `yoake build` and `yoake check`) across `crates/yoake-wasm-sdk/examples/` and `crates/yoake-guest-sdk/examples/`, plus standalone test fixtures.
The fastest way to build your own mod is to open the example closest to what you have in mind and adapt its `yak.toml` and `src/lib.rs`.

---

## 4-1. Example Overview (What Each Example Shows, Hooks, and Host Services)

| Example | WIT world (mod kind) | What It Demonstrates | Main Hooks (`hook::*`) | Main Host Services |
|---|---|---|---|---|
| `hello` | `client-mod` | Minimal `#![no_std]` client mod: counts ticks, draws a background rect and text on the HUD, and returns a settings label | `TICK`, `HUD`, `SETTINGS` | `yoake_wasm_sdk::log`, `hud::rect`, `hud::text` |
| `hello-std` | `client-mod` | Minimal `std` client mod using `println!` and `std::time::Instant`; counts ticks, logs key presses, and draws HUD text | `TICK`, `KEY`, `HUD` | `hud::text` (+ `println!`, `std::time`) |
| `hud-item` | `client-full` | Draws a translucent panel, a `minecraft:diamond` item icon, and a text label on the HUD every frame | `HUD` | None (returns `Rect`, `Item`, and `Text` inside `et::HudOutcome`) |
| `tested-counter` | `client-mod` | Beginner example pairing a client HUD tick counter with `#[yoake_test]` engine tests in the same crate | `TICK`, `HUD` | `hud::text` (+ `TestWorld` calls) |
| `config-demo` | `client-full` | Reads banner text and color options declared under `[config]` in `yak.toml` and updates the HUD whenever `config_changed` fires | `HUD`, `CONFIG_CHANGED` (+ `init`) | `services::config_get` |
| `client-services-demo` | `client-full` | Exercises client services inside `hud` (key bindings, world line, post shader, writable `GuiTexture`, IME) and checks both valid and invalid inputs | `HUD` | `register_key_binding`, `draw_world_line`, `register_post_shader`, `gui_texture_write`, `ime_set_allowed`, etc. |
| `gpu-demo` | `client-full` | Custom WGSL render and compute pipelines: a textured mesh, compute-updated particles, and depth-based tint (`verify`/`reload`/`trap` features) | `HUD` | `gpu::{Shader, RenderPipeline, ComputePipeline, Buffer, BindGroup}` etc. |
| `sprite-demo` | `client-full` | L2 runtime sprite override ([Chapter 9](./09-sprites.md)): tints `minecraft:apple` and `block/stone` on first tick and restores on `R` | `TICK`, `KEY` | `sprites::{item_pixels, set_item, reset_item, block_pixels, set_block, reset_block}` |
| `ime` | `client-full` | Enables OS IME on a mod screen, receives preedit/commit, and places the candidate window in GUI coordinates ([Chapter 10](./10-ime.md)) | `SCREEN_INIT_POST`, `FRAME`, `IME_PREEDIT`, `IME_COMMIT` | `ime_set_allowed`, `ime_set_cursor_area` |
| `browser-demo` | `client-full` | MEDIA1 embedded browser overlay ([Chapter 15](./15-browser-media.md)): toggles a `Browser` on `B` and paints its frames onto a HUD `GuiTexture` | `HUD`, `KEY` | `media::Browser`, `hud::GuiTexture`, `client::gui_texture_id` |
| `fox_texture` | `client-full` | F4M client example ([Chapter 14](./14-custom-mobs.md)): repaints Cinder Fox skin dynamically on `warmth` change and draws a cube and apple via `mob_layer` | `SYNCED_CHANGED_CLIENT`, `PLAYER_RENDER_LISTENED`, `MOB_LAYER` | `mob_texture_create`, `mob_texture_write`, `mob_texture_bind` |
| `server-storage` | `server-full` | Adds a `/visits` command, saves each player's visit count (keyed by UUID) in `World`-scope storage, and replies in chat | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::storage_get`, `services::storage_put`, `services::tell` |
| `permission-demo` | `server-full` | Adds a `/secret` command and checks the `"secret.access"` permission node with `has_permission` before replying | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::has_permission`, `services::tell` |
| `chest-count` | `server-full` | `/chestcount <x> <y> <z>` reads the container at the given coordinates and replies with a total per item ID | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::container_slots`, `services::tell` |
| `item-trade` | `server-full` | `/trade` takes exactly 16 cobblestone only if all 16 are present and gives 1 diamond | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::inventory_apply` (`Give`), `ext::take`, `ext::take_exact`, `services::tell` |
| `player-vitals` | `server-full` | `/vitals` replies with the player's health and the `known` / `done` state of `minecraft:story/root` | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::advancement_progress`, `services::attribute_get`, `services::tell` |
| `mod-call-pair/caller` | `server-full` | `/ping <text>` calls `mod-call-responder`'s `handle_call` and replies with the returned UTF-8 text | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::mod_call`, `services::tell` |
| `mod-call-pair/responder` | `server-full` | Handles `"echo"` calls from `caller` in `handle_call` and sends back a `pong` response | `handle_call` | None |
| `content-demo` | `server-full` | Data-defined blocks, items, effect, recipe, loot, mob, sound, and creative tab with a server module reacting to `gem_cow` | `INTERACT_ENTITY`, `MOB_KILLED`, `ENTITY_TICK_POST` | `acting_player`, `play_sound_at`, `tell`, `ext::attachment_*` |
| `ruby-behaviors` | `server-full` | REM-15 `behavior-register` demo: `ruby:lamp` block, `ruby:wand` item, 9-slot `ruby:vault` menu, and `ruby:alloy` recipe type | `handle_call` (`Behaviors` trait) | `ext::register_behavior`, `ext::recipe_data`, `block_property`, `set_block_property` |
| `level-generator` | `server-full` | F3-8 custom dimension type, level (`levelgen:terraces`), and WASM chunk generator via `handle_call` ([Chapter 13](./13-dimensions.md)) | `handle_call` (+ `init`) | `ext::register_type`, `ext::register_generator`, `ext::register_level` |
| `f2-check` | `server-full` / `worker-mod` | FR-F2 split-crate example: `/data` and `/game` file I/O plus non-blocking worker HTTP POST and TCP echo ([Chapter 12](./12-files-network.md)) | `TICK` (+ `init`) / worker `run` | `std::fs`, `worker::{send, recv}`, `wasi:http`, `std::net` |
| `f2-io` | `server-full` / `worker-mod` | FR-F2 single-crate feature-split example (`server` + `worker`, plus `files` / `main` test-guest features) | `TICK` (+ `init`) / worker `run` | `worker::{send, recv}`, `wasi:http`, `std::net` |
| `fox_ai` | `server-full` | F4M server AI ([Chapter 14](./14-custom-mobs.md)): Cinder Fox `pounce` `ScriptGoal`, `"hit"` timeline leap+bite, and `mob_hurt` updating `warmth` | `MOB_GOAL_*`, `MOB_ANIM_EVENT`, `MOB_HURT` | `mobs::{info, memory_*, trigger_anim, impulse, attack, set_synced}` |
| `seat_fox` | `server-full` | F4M vehicle seats ([Chapter 14](./14-custom-mobs.md)): automatically seats nearby `minecraft:chicken` onto seat 1 of `sample_mobs:cinder_fox` | `CUSTOM_MOB_SPAWNED`, `MOB_TICK`, `ENTITY_JOIN_LEVEL`, `ENTITY_LEAVE_LEVEL` | `mobs::{info, seat_set}` |
| `full-demo` | `client-full` / `server-full` | All-in-one showcase covering client (HUD, hiding hotbar, tooltips, FOV), server (`/wasmping`, `/wasmsvc`), and `handle_call` | `HUD`, `GUI_LAYER_PRE`, `TOOLTIP`, `COMPUTE_FOV`, `REGISTER_COMMANDS`, `PERFORM_COMMAND`, `SERVER_TICK`, `handle_call` | `tell`, `give_items`, `break_block_as`, `edit_blocks`, `run_command`, `mod_call`, etc. |
| `full-services` | `client-full` / `server-full` | Exercises `[config]`, inline/blob storage, and permission checks across both client and server | `CONFIG_CHANGED`, `handle_call` (+ `init`) | `config_get/set`, `storage_*`, `storage_blob_*`, `has_permission` |
| `mixin-demo` | `inject-mod` | Halves fall damage, doubles bonemeal crop-growth increment, and logs chat ([Chapter 8](./08-mixin.md)) | None (Mixin HEAD / RETURN handlers) | None (`println!` logging only) |
| `mixin-config` | `inject-mod` | Configurable `inject-mod` reading and reacting to `[config.count]` updates on `tick_sleep_timer` | None (Mixin + `config_changed`) | `mixin::config_get` |
| `yoake-guest-sdk/f1-f2-demo` | `full-access` / `worker-mod` | F1+F2 demo: `#![no_std]` core server module (`SERVER_TICK`, `yoake_guest_sdk::worker`) paired with a WASI p2 worker | `hooks::SERVER_TICK` / worker `run` | `yoake_guest_sdk::worker::{send, recv}` |

Non-package fixtures:

- `crates/yoake-wasm-sdk/examples/test-demo`: A `test-mod` fixture for testing the test runner itself (no `yak.toml`; see [Chapter 6](./06-testing.md)).
- `crates/yoake-guest-sdk/examples/`: Four standalone single-file `.rs` fixtures (`engine_call_game_path.rs`, `f_client_services.rs`, `fox_ai_full.rs`, `tick_and_item_use.rs`).

---

## 4-2. Highlights and Code Snippets

### ① `hello` / `hud-item` (Drawing Text and Items on the HUD)

- If the 19 core client hooks are enough for your mod, use `yoake_wasm_sdk::Mod` and `export_mod!` just like `hello`.
- If you need full-catalog (`client-full` / `server-full`) features such as item icons (`HudCommand::Item`) or tooltip edits, set `world = "client-full"` in `yak.toml` and use `ClientMod` with `export_client_full!` like `hud-item`.

Excerpt from `hud-item`:[^hud-item]

```rust
        // 2. A 16x16 item icon for "minecraft:diamond".
        //    `components` holds optional binary item component data; an empty
        //    Vec means a default stack with no extra components.
        let icon = et::HudCommand::Item(et::HudItem {
            stack: et::ItemStack {
                id: "minecraft:diamond".to_owned(),
                count: 1,
                components: Vec::new(),
            },
            x: panel_x + 4.0,
            y: panel_y + 4.0,
            decorations: true,
        });
```

### ② `server-storage` (Adding Commands and Saving Data)

Server-side mods declare `server = "bin/wasm/..."` under `[wasm]` in `yak.toml` (the WIT world for a server module is `server-full`, except for Mixin's `inject-mod`; see [Chapter 8](./08-mixin.md)).
To call `storage_get` or `storage_put`, you must also declare `permissions = ["service.storage"]` under `[wasm]` in `yak.toml`.

Excerpts from `server-storage`:[^server-storage]

```rust
    fn register_commands(&mut self, mut commands: Vec<et::NativeCommand>) -> Vec<et::NativeCommand> {
        commands.push(et::NativeCommand {
            usage: "visits".to_owned(),
            level: 0,
        });
        commands
    }
```

```rust
        let current = match services::storage_get(&key, types::StorageScope::World) {
            Ok(Some(bytes)) => decode_u64(&bytes),
            Ok(None) => 0,
            Err(err) => {
                let _ = services::tell(&format!("Failed to read visits: {err}"), &uuid);
                return true;
            }
        };

        let next = current.saturating_add(1);
        if let Err(err) =
            services::storage_put(&key, types::StorageScope::World, &next.to_le_bytes())
        {
            let _ = services::tell(&format!("Failed to save visits: {err}"), &uuid);
            return true;
        }
```

**Storage scopes (`StorageScope`)**: Server modules can use per-world `World` (`<world>/yoake-data/<id>/`) or cross-world `Global` (`<game>/yoake-data/<id>/global/`), while client modules use `Client` (`<game>/yoake-data/<id>/client/`). Each mod can reach only its own package's store (the folder for one scope).[^storage]

- **When `World` storage is available**: `World` storage first becomes available in `world_lifecycle(WorldOpenReady)`. It is not available in `server_about_to_start` or in the first level- and chunk-loading hooks that run before `WorldOpenReady`; use `Global` there instead.
- **Per-key atomic files and locking**: Rather than storing everything in a single file, each store folder holds an exclusive lock file (`store.lock`) for its lifetime (another process that opens the same store gets an error) and writes each key into its own host-generated entry file (`entry-*.bin`). Only keys and value lengths stay resident in memory; values and blob slices are read on demand. Every `storage_put`, `storage_delete`, and `storage_blob_commit` persists before returning: the host writes a temporary file (`.store-*.tmp`), calls `fsync` (`sync_all`), and atomically renames it into place (also syncing the parent directory on Unix). Stale `.store-*.tmp` files are cleaned up when a store is opened.
- **Quotas and blobs**: Each store has a **256 MiB** quota (including entry metadata and reserved pending uploads) and holds at most **65,536 keys** (logical names up to 256 bytes, never paths). Inline `storage_get` / `storage_put` calls and individual blob chunks are capped at **1 MiB**. Larger values use `storage_blob_begin`, `storage_blob_write`, `storage_blob_commit`, `storage_blob_abort`, and `storage_blob_read` (up to **64** outstanding uploads per store and a **32 MiB** aggregate pending-blob limit per mod across all scopes; buffers grow lazily on write, chunks may arrive out of order, and commit requires complete coverage, retaining the upload on failure so you can finish or abort it; pending uploads are not persisted, and `World` uploads are discarded when the world changes or when a module instance is created, dropped, disabled, or traps).
- **Write budget and operation limits**: Across all scopes, encoded storage writes—**including config writes**—share a per-mod token bucket of **8 MiB/s sustained** with a **32 MiB burst**. Each storage operation is charged at least 4 KiB, and a mod may perform at most 64 storage operations per 50 ms hook window. Exceeding either limit returns an error, and a mod that keeps exceeding it window after window is disabled.
- **Corrupted entries**: A corrupted `entry-*.bin` file is renamed to `*.bin.corrupt` (quarantined) so that healthy keys remain usable; at most 16 such files are kept per store, and any excess is deleted.[^legacy]

> [!NOTE]
> The on-disk format (`entry-*.bin`, `store.lock`, `*.bin.corrupt`, and so on) is internal and may change in future versions. Never edit the files in the store folder directly or build external tools that depend on their layout; always read and write data through `storage_get`, `storage_put`, and the `storage_blob_*` calls. (Chapter 7 repeats this as a rule for published mods: [7-6](./07-publishing.md).)

### ③ `mod-call-pair` (Calling Another Mod)

`caller` and `responder` are separate `.yak` packages, so you install both. `caller` turns the text of `/ping <text>` into bytes and sends it with `services::mod_call("echo", ..., "mod-call-responder")`, and `responder` answers with a `pong` from its `handle_call`. The call target is the package ID from `yak.toml`. There is no field in `yak.toml` for naming the target, and no additional service permission is needed. If `responder` is not installed and enabled, the player sees the error `no package ... with a server module`. On both sides, bytes that cannot be converted to UTF-8 are reported as an error instead of a panic.

### ④ `chest-count` (Counting Items in a Container)

`/chestcount <x> <y> <z>` reads the given coordinates, in the same dimension as the player who ran the command, with `services::container_slots`. It takes the contents out of each slot's `Option<types::ItemStack>` and adds up `count` per item ID. A large chest returns the slots of both halves. It works on containers other than chests, but a position with no container or an unloaded chunk produces an error, and a composter is not treated as a container.

### ⑤ `item-trade` (Trading Items)

`/trade` first gives 1 `minecraft:diamond` with `services::inventory_apply` (`Give`); this only targets the main slots, and either all of it fits or none does. If it does not fit, the command reports that the inventory is full and stops, with nothing changed. If it fits, `ext::take` takes exactly 16 `minecraft:cobblestone`; this is also all-or-nothing, and it includes the offhand. If that fails (there is not enough cobblestone), `ext::take_exact` takes back the diamond that was just given and the trade is cancelled. Because the order is "give, then take", there is no need to return the input. `ext::take` also takes cobblestone that carries components such as a name or tags, whereas `ext::take_exact` takes only a stack whose item, count, and components all match exactly.

### ⑥ `player-vitals` (Reading Health and Advancement Progress)

`/vitals` reads the current health from `player.health` in the command hook and the maximum health from `services::attribute_get("minecraft:max_health", ...)`, then reads the state of `minecraft:story/root` with `services::advancement_progress`. In the returned `types::AdvancementState`, `known` says whether the advancement is recognized and `done` says whether it is complete. If reading the advancement or the maximum health fails, the command still replies with the health it could read.

### ⑦ `content-demo` (A Server Module Reacting to Data-Defined Content)

Blocks, items, a mob effect, a recipe, a loot table, a mob, a sound, and a creative tab are defined purely as data under `content/`, and the server module only reacts to them. Right-clicking `content_demo:gem_cow` plays its own sound through `interact_entity` to tell the player, and killing it stores a per-world kill count as a data attachment (`ext::attachment_get` / `ext::attachment_set`) through `mob_killed`. The `id` in `yak.toml` is `content_demo`, and it declares `bin/wasm/content-demo-server.wasm` as the server module.

### ⑧ `mixin-demo` (Intercepting Engine Functions)

This uses the `inject-mod` WIT world to halve fall damage, double the bonemeal crop-growth increment, and log chat. Because it is a Mixin injection rather than an ordinary hook, how to write it is explained in [8. Getting Started with Yoake Mixin (`08-mixin.md`)](./08-mixin.md).

### ⑨ `tested-counter` and `test-demo` (Tests Inside the Game Engine)

`tested-counter` is a beginner example that puts a client mod showing a tick counter on the HUD and `#[yoake_test]` tests in the same crate. `test-demo` is a test module for trying out the test runner itself, with 6 tests: passing, failing, trapping, running out of time, every driver call, and two clients. Read both together with [6. Testing Your Mod Inside the Game Engine (`06-testing.md`)](./06-testing.md).

### ⑩ `client-services-demo`, `gpu-demo`, `sprite-demo`, and `browser-demo` (Client Services, GPU, Sprites, and Browser)

All four are `client-full` client mods using `export_client_full!`. Each comes with a `yak.toml` (`interface = "1.7"`, `world = "client-full"`) and builds directly with `yoake build`.

- `client-services-demo`: Calls key bindings, world lines, post shaders, writable `GuiTexture`, and IME inside `hud`, testing both valid calls and validation errors.
- `gpu-demo`: Builds render and compute pipelines from WGSL shaders through the `gpu` module (requires `"service.gpu"` in `[wasm] permissions`; see the GPU section of [`docs/yoake-wasm-dev.md`](../yoake-wasm-dev.md#client-gpu-api)).
- `sprite-demo`: Tints item and block sprites dynamically on startup and restores them on `R` ([Chapter 9](./09-sprites.md)).
- `browser-demo`: Toggles an embedded browser on `B` and paints its frames onto a HUD `GuiTexture` ([Chapter 15](./15-browser-media.md)).

### ⑪ `ime` (IME Input on Mod Screens)

`ime` enables OS IME on a mod screen and receives preedit text, cursor range, and committed text, placing the candidate window in GUI-scaled coordinates. See [Chapter 10](./10-ime.md).

### ⑫ `fox_ai`, `fox_texture`, and `seat_fox` (F4M Custom Mobs and Seats)

These examples pair with the [`tools/fixtures/cinder_fox/`](../../tools/fixtures/cinder_fox/README.md) data fixture. `fox_ai` and `seat_fox` run on `server-full`, `fox_texture` runs on `client-full`, and all three declare `permissions = ["service.mobs"]` ([Chapter 14](./14-custom-mobs.md)).

### ⑬ `f2-check`, `f2-io`, and `f1-f2-demo` (Files and Workers)

Show how to run HTTP, TCP, and heavy file I/O in a background worker without blocking the game thread ([Chapter 12](./12-files-network.md)). `f1-f2-demo` in `crates/yoake-guest-sdk/examples/f1-f2-demo` pairs a `full-access` core module with a `worker-mod` worker.

### ⑭ `ruby-behaviors`, `level-generator`, and `mixin-config`

- `ruby-behaviors`: Attaches custom block, item, menu, and recipe-type behavior with `ext::register_behavior` (see [MOD-GUIDE.md section 19](../yoake2/MOD-GUIDE.md#19-behaviours-of-the-mods-own-blocks-items-menus-and-recipe-types-rem-15)).
- `level-generator`: Registers a custom dimension type, level, and WASM chunk generator ([Chapter 13](./13-dimensions.md)).
- `mixin-config`: An `inject-mod` (`interface = "1.2"`) that reads `[config.count]` and reacts to `config_changed` ([Chapter 8](./08-mixin.md)).

---

## 4-3. Writing `[wasm] permissions`, `[config]`, and `[permissions."<node>"]` in `yak.toml`

Whenever a mod uses privileged features, declare them in `yak.toml`.

### Available `[wasm] permissions`

There are **10** known service permissions (`[wasm] permissions`):[^manifest]

- `"service.commands"`: Calling `run_command` from a server module (`wasm.server` is required, except with `inject-mod`). Also required for Mixin handlers on sensitive command and chat sites ([Chapter 8](./08-mixin.md))
- `"service.chat"`: Lets a client module rewrite outgoing chat and command text through Mixin (`wasm.client` is required, used together with `service.commands`; see [Chapter 8](./08-mixin.md))
- `"service.gpu"`: Calling the `gpu` module from a client module (only in `client-full`; not available in `server-full` or `client-mod`)
- `"service.mobs"`: Custom-mob scripting (`mobs::*` on `server-full`, and `mob_texture_*` / `mob_bone_index` / `mob_locator` on `client-full`; see [Chapter 14](./14-custom-mobs.md))
- `"service.browser"`: Opening embedded browsers with `full::client::media::Browser` (only for `wasm.client` with `world = "client-full"`; see [Chapter 15](./15-browser-media.md))
- `"service.screen-share"`: Reserved for future screen sharing (not wired yet; same manifest rule as `service.browser`)
- `"service.http"`: Paired with `[http] hosts` (declaring only one fails validation; HTTP transport itself is not wired—use `wasi:http` in a worker, [Chapter 12](./12-files-network.md))
- `"service.config"` / `"service.permissions"` / `"service.storage"`: Accepted in manifests for backward compatibility with older mods. The current host grants config, permission-node, and storage services without requiring them in `permissions` ([Chapter 12](./12-files-network.md))

### Writing `[config]` (Excerpt from `config-demo`'s Manifest)

```toml
[wasm]
interface = "1.0"
client = "bin/wasm/config-demo.wasm"
world = "client-full"
permissions = ["service.config"]

[config.text]
type = "string"
side = "client"
default = "Config Demo HUD"
comment = "Text shown on the HUD banner."
label.en_us = "HUD Text"
label.ja_jp = "HUDテキスト"

[config.color]
type = "enum"
side = "client"
default = "gold"
values = ["gold", "aqua", "lime", "white"]
comment = "Colour preset for the HUD banner text."
label.en_us = "HUD Colour"
label.ja_jp = "HUD文字色"
```

You can also use numeric bounds (`min` / `max`), lists (`type = "list"`), and `side = "synced"` settings that automatically sync from the server to connected clients:[^dev-doc]

```toml
[config.enabled]
type = "bool"
side = "server"
default = true
label.en_us = "Enabled"
label.ja_jp = "有効"

[config.radius]
type = "int"
side = "synced"
default = 8
min = 1
max = 16

[config.layers]
type = "list"
element = "enum"
side = "server"
default = ["surface"]
values = ["surface", "deep"]
```

- **Supported types (`type`)**: `"bool"`, `"int"`, `"float"`, `"string"`, `"enum"`, and `"list"` (for `"list"`, specify any non-list type in `element`).
- **Supported sides (`side`)**: `"client"` (client-only), `"server"` (server-only), and `"synced"` (managed by the server and automatically synced to connected clients).
- Any mod that declares `[config]` automatically gets an in-game settings screen accessible via the **Configure** button in the mod list.

### Writing `[permissions."<node>"]` (Excerpt from `permission-demo`'s Manifest)

```toml
[wasm]
interface = "1.0"
server = "bin/wasm/permission-demo.wasm"
permissions = ["service.permissions"]

[permissions."secret.access"]
default_op_level = 2
```

- Permission node names (such as `"secret.access"`) use dot-separated (`.`) segments of ASCII letters, digits, `_`, or `-`, and `default_op_level` specifies an operator level from `0` through `4`.
- Calling `services::has_permission("secret.access", &uuid)` first checks `<world>/permissions.toml` in the world folder for an explicit per-player or per-group grant/denial; if none is set, it falls back to checking whether the player's OP level is at least `default_op_level`.

---

## Next Steps

If you run into an error while modifying or building a mod, check [5. Common Errors and How to Fix Them (`05-errors.md`)](./05-errors.md) to find the cause and solution.

[^hud-item]: Source: `crates/yoake-wasm-sdk/examples/hud-item/src/lib.rs`.
[^server-storage]: Source: `crates/yoake-wasm-sdk/examples/server-storage/src/lib.rs`.
[^storage]: Source: `DiskStorage::open_scoped_with_quota` / `persist` / `atomic_write` / `import_store` / `import_directory` in `crates/yoake-wasm/src/host_services/storage.rs` and "Package config, permissions and storage" in `docs/yoake-wasm-dev.md`.
[^legacy]: Older versions kept everything in one `store.bin` and in per-key files under `<world>/yoake-wasm/<id>/` and `<game>/config/yoake-wasm/<id>/`. When a `World` or `Global` store is first bound, the host automatically imports those legacy entries within the quota, without replacing newer keys (`Client` stores import neither), and keeps the old files as recovery backups; a `migration-v2.done` marker prevents repeated imports. Corrupt or over-quota legacy records are skipped with a warning, and if a retryable I/O failure occurred the marker is not written, so the import is retried the next time the store is opened. Source: "Package config, permissions and storage" in `docs/yoake-wasm-dev.md`.
[^manifest]: Source: `crates/horizon-mods/src/yak/manifest.rs`.
[^dev-doc]: Source: `docs/yoake-wasm-dev.md`.
