# 4. Exploring the Example Mods

The `crates/yoake-wasm-sdk/examples/` folder contains 19 purpose-built example mods (`hello` and `hello-std` share one row in the table in 4-1).
The fastest way to build your own mod is to open the example closest to what you have in mind and adapt its `yak.toml` and `src/lib.rs`.

---

## 4-1. Example Overview (What Each Example Shows, Hooks, and Host Services)

| Example | WIT world (mod kind) | What It Demonstrates | Main Hooks (`hook::*`) | Main Host Services |
|---|---|---|---|---|
| `hello` (and `hello-std`) | `client-mod` | Minimal client mod. Counts ticks, draws a background rectangle and text on the HUD, and returns a settings button label (`hello` is `#![no_std]`; `hello-std` uses `std`) | `TICK`, `HUD`, `SETTINGS` (`hello-std` uses `TICK`, `KEY`, `HUD`) | `yoake_wasm_sdk::log`, `hud::rect`, `hud::text` |
| `hud-item` | `client-full` | Draws a translucent panel, a `minecraft:diamond` item icon, and a text label on the HUD every frame | `HUD` | None (returns `Rect`, `Item`, and `Text` inside `et::HudOutcome`) |
| `server-storage` | `server-full` | Adds a `/visits` command, saves each player's visit count (keyed by UUID) in `World`-scope storage, and replies in chat | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::storage_get`, `services::storage_put`, `services::tell` |
| `config-demo` | `client-full` | Reads banner text and color options declared under `[config]` in `yak.toml` and updates the HUD whenever `config_changed` fires | `HUD`, `CONFIG_CHANGED` (+ `init`) | `services::config_get` |
| `permission-demo` | `server-full` | Adds a `/secret` command and checks the `"secret.access"` permission node with `has_permission` before replying | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::has_permission`, `services::tell` |
| `full-demo` | `client-full` / `server-full` | All-in-one showcase covering the client (HUD, hiding the hotbar, custom tooltips, FOV adjustment), server (`/wasmping` and `/wasmsvc` exercising host services), and cross-mod calls (`peer`) | client: `HUD`, `GUI_LAYER_PRE`, `TOOLTIP`, `COMPUTE_FOV` / server: `REGISTER_COMMANDS`, `PERFORM_COMMAND`, `SERVER_TICK` / peer: `handle_call` | `tell`, `give_items`, `break_block_as`, `edit_blocks`, `run_command`, `mod_call`, and many more |
| `full-services` | `client-full` / `server-full` | End-to-end test of config, storage (inline and chunked blobs), and permission-node checks, plus verifying `"permission denied: ..."` rejections when a service permission is omitted | `CONFIG_CHANGED` (+ client `init` / server `handle_call`) | `config_get`, `config_set`, `storage_get`, `storage_put`, `storage_delete`, `storage_blob_begin/write/commit/abort/read`, `has_permission` |
| `mod-call-pair` | `server-full` | From `caller`'s `/ping <text>`, calls `responder`'s `handle_call` so that two mods exchange UTF-8 text | `REGISTER_COMMANDS`, `PERFORM_COMMAND` / `handle_call` | `services::mod_call`, `services::tell` |
| `chest-count` | `server-full` | `/chestcount <x> <y> <z>` reads the container at the given coordinates and replies with a total per item ID | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::container_slots`, `services::tell` |
| `item-trade` | `server-full` | `/trade` takes exactly 16 cobblestone, but only if it can take all of them (nothing is taken if there are too few), and gives 1 diamond | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::inventory_apply` (`Give`), `ext::take`, `ext::take_exact`, `services::tell` |
| `player-vitals` | `server-full` | `/vitals` replies with the player's health and the `known` / `done` state of the `minecraft:story/root` advancement | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::advancement_progress`, `services::attribute_get`, `services::tell` |
| `content-demo` | `server-full` | A `.yak` with data-defined content (blocks, items, a mob effect, a recipe, a loot table, a mob, a sound, and a creative tab) plus a small server module that reacts to the mob: right-clicking it plays its sound, and kills and ticks are counted | `INTERACT_ENTITY`, `MOB_KILLED`, `ENTITY_TICK_POST` | `services::play_sound_at`, `services::tell`, `ext::attachment_*` |
| `mixin-demo` | `inject-mod` | Halves fall damage, doubles the bonemeal crop-growth increment, and logs chat (see [8. Getting Started with Yoake Mixin](./08-mixin.md)) | None (Mixin handlers instead of hooks) | None |
| `test-demo` | `test-mod` | A test module that exercises every call of the test driver, including passing, failing, and trapping tests (see [6. Testing Your Mod Inside the Game Engine](./06-testing.md)) | None | `TestWorld` calls |
| `tested-counter` | `client-mod` | A small beginner example that pairs a client mod (tick counter on the HUD) with engine tests in the same crate (see [6. Testing Your Mod Inside the Game Engine](./06-testing.md)) | `TICK`, `HUD` | `hud::text`, `TestWorld` calls |
| `client-services-demo` | `client-full` | Calls client services inside the `hud` hook (key bindings, a world line, a post shader, writable GUI textures, and IME enablement/candidate placement), and checks both successful calls and validation errors. It is the guest used by the runtime integration tests | `HUD` | `services::register_key_binding`, `services::key_binding_state`, `services::draw_world_line`, `services::register_post_shader`, `hud::GuiTexture::create`, `services::gui_texture_write`, `services::ime_set_allowed`, `services::ime_set_cursor_area` |
| `gpu-demo` | `client-full` | An example of the GPU surface (the `gpu` module): a textured mesh, a compute-updated particle buffer, and a depth-based tint after the world is drawn. Extra Cargo features `verify` / `reload` / `trap` build variants for testing | `HUD` | `gpu::{Shader, RenderPipeline, ComputePipeline, Buffer, BindGroup}` and more |
| `ime` | `client-full` | Enables IME on a mod screen, receives preedit text/cursor ranges and commits, and places the OS candidate window in GUI-scaled coordinates (see the Japanese `10-ime.md` chapter) | `SCREEN_INIT_POST`, `FRAME`, `IME_PREEDIT`, `IME_COMMIT` | `services::ime_set_allowed`, `services::ime_set_cursor_area` |

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

### ⑩ `client-services-demo` and `gpu-demo` (New Client Services and the GPU)

Both are client mods for the `client-full` world and use `export_client_full!`. Their folders contain no `yak.toml`; the runtime tests load the built `.wasm` directly. If you reuse them in a mod you ship, write `"client-full"` in `[wasm] world` in `yak.toml`.

- `client-services-demo`: Inside the `hud` hook it calls the key binding, world line, post shader, and GUI texture services. It tries both calls that should succeed and calls that should return an error (an invalid shader, a write outside the texture), so it shows how to use these services and how errors come back. The example declares no `[wasm] permissions`.
- `gpu-demo`: Builds render and compute pipelines from WGSL shaders through the `gpu` module. Calling `gpu` requires `"service.gpu"` in `[wasm] permissions` (see 4-3). For details, see the GPU section of `docs/yoake-wasm-dev.md`.

---

## 4-3. Writing `[wasm] permissions`, `[config]`, and `[permissions."<node>"]` in `yak.toml`

Whenever a mod uses privileged features—reading or writing config, persisting storage, checking permission nodes, or running server commands—it must declare them in `yak.toml`.

### Available `[wasm] permissions`

There are 7 service permissions (capabilities, `[wasm] permissions`) you can declare:[^manifest]

- `"service.chat"`: Lets a client module rewrite outgoing chat and command text through Mixin (`wasm.client` is required, and it is used together with `service.commands`; see [Chapter 8](./08-mixin.md))
- `"service.commands"`: Calling `run_command` from a server module (`wasm.server` is required, except with the `inject-mod` WIT world, which does not need a server module). It is also required to register Mixin handlers on the sensitive command and chat sites ([Chapter 8](./08-mixin.md))
- `"service.config"`: Calling `config_get`, `config_set`, or `config_dir`, and receiving synced config
- `"service.gpu"`: Calling the `gpu` module from a client module (only in the `client-full` world; not available in `server-full` or `client-mod`)
- `"service.http"`: A `service.http` service permission declaration that is paired with `[http] hosts` (declaring only one of the two fails validation; note that HTTP transport itself is not connected yet)[^dev-doc]
- `"service.permissions"`: Calling `has_permission` from a server module (`wasm.server` is required)
- `"service.storage"`: Calling `storage_*` as well as `world_storage_*`, `config_read`, and `config_write`

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
