# 4. Exploring the Example Mods

The `crates/yoake-wasm-sdk/examples/` folder contains purpose-built example mods.
The fastest way to build your own mod is to open the example closest to what you have in mind and adapt its `yak.toml` and `src/lib.rs`.

---

## 4-1. Example Overview (What Each Example Shows, Hooks, and Services)

| Example | WIT world (mod kind) | What It Demonstrates | Main Hooks (`hook::*`) | Main Services / Host Calls |
|---|---|---|---|---|
| `hello` (and `hello-std`) | `client-mod` | Minimal client mod. Counts ticks, draws a background rectangle and text on the HUD, and returns a settings button label (`hello` is `#![no_std]`; `hello-std` uses `std`) | `TICK`, `HUD`, `SETTINGS` (`hello-std` uses `TICK`, `KEY`, `HUD`) | `yoake_wasm_sdk::log`, `hud::rect`, `hud::text` |
| `hud-item` | `client-full` | Draws a translucent panel, a `minecraft:diamond` item icon, and a text label on the HUD every frame | `HUD` | None (returns `Rect`, `Item`, and `Text` inside `et::HudOutcome`) |
| `server-storage` | `server-full` | Adds a `/visits` command, saves each player's visit count (keyed by UUID) in world storage, and replies in chat | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::storage_get`, `services::storage_put`, `services::tell` |
| `config-demo` | `client-full` | Reads banner text and color options declared under `[config]` in `yak.toml` and updates the HUD whenever `config_changed` fires | `HUD`, `CONFIG_CHANGED` (+ `init`) | `services::config_get` |
| `permission-demo` | `server-full` | Adds a `/secret` command and checks the `"secret.access"` permission node with `has_permission` before replying | `REGISTER_COMMANDS`, `PERFORM_COMMAND` | `services::has_permission`, `services::tell` |
| `full-demo` | `client-full` / `server-full` | All-in-one showcase covering the client (HUD, hiding the hotbar, custom tooltips, FOV adjustment), server (`/wasmping` and `/wasmsvc` exercising host services), and cross-mod calls (`peer`) | client: `HUD`, `GUI_LAYER_PRE`, `TOOLTIP`, `COMPUTE_FOV` / server: `REGISTER_COMMANDS`, `PERFORM_COMMAND`, `SERVER_TICK` / peer: `handle_call` | `tell`, `give_items`, `break_block_as`, `edit_blocks`, `run_command`, `mod_call`, and many more |
| `full-services` | `client-full` / `server-full` | End-to-end test of config, storage (inline and chunked blobs), and permission checks, plus verifying `"permission denied: ..."` rejections when permissions are omitted | `CONFIG_CHANGED` (+ client `init` / server `handle_call`) | `config_get`, `config_set`, `storage_get`, `storage_put`, `storage_delete`, `storage_blob_begin/write/commit/abort/read`, `has_permission` |

---

## 4-2. Highlights and Code Snippets

### ① `hello` / `hud-item` (Drawing Text and Items on the HUD)

- If the 19 basic hooks are enough for your mod, use `yoake_wasm_sdk::Mod` and `export_mod!` just like `hello`.
- If you need full-catalog features such as item icons (`HudCommand::Item`) or tooltip edits, set `world = "client-full"` in `yak.toml` and use `ClientMod` with `export_client_full!` like `hud-item`.

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

Server-side mods declare `server = "bin/wasm/..."` under `[wasm]` in `yak.toml` (the WIT world for a server module is always `server-full`).
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

- **Storage scopes and folders (`StorageScope`)**: Server modules can use per-world `World` (`<world>/yoake-data/<mod-id>/`) or cross-world `Global` (`<game>/yoake-data/<mod-id>/global/`), while client modules use `Client` (`<game>/yoake-data/<mod-id>/client/`).[^storage]
- **Per-key atomic files and locking**: Rather than storing everything in a single `store.bin`, each store folder holds an exclusive lock file (`store.lock`) for its lifetime (concurrent processes receive an error) and writes each key into its own host-generated entry file (`entry-*.bin`). Only keys and value lengths stay resident in memory; values and blob slices are read on demand. Every `storage_put`, `storage_delete`, and `storage_blob_commit` persists before returning by writing to a temporary file (`.store-*.tmp`), calling `fsync` (`sync_all`), and atomically renaming it into place (also syncing the parent directory on Unix); stale `.store-*.tmp` files are cleaned up on open.[^storage]
- **Quotas, blobs, and write budget**: Each store has a **256 MiB** quota (including entry metadata and reserved pending uploads) and holds at most **65,536 keys** (logical names up to 256 bytes, never paths). Inline `storage_get` / `storage_put` calls and individual blob chunks are capped at **1 MiB**. Larger values use `storage_blob_begin`, `storage_blob_write`, `storage_blob_commit`, `storage_blob_abort`, and `storage_blob_read` (up to **64** concurrent uploads per store and a **32 MiB** aggregate pending-blob limit per mod across all scopes; buffers grow lazily on write, chunks may arrive out of order, and commit requires complete coverage, retaining the upload on failure for completion or abort; pending uploads are not persisted and world uploads are discarded on world change or when an instance is created, dropped, disabled, or traps). Across all scopes, encoded storage writes—**including config writes**—share a per-mod token bucket of **8 MiB/s sustained** with a **32 MiB burst**, returning an error if exceeded.[^storage]
- **Automatic legacy migration and corruption handling**: On first bind, server stores (`World` and `Global`) automatically import existing `store.bin` data and legacy `<world>/yoake-wasm/<mod-id>/<key>.bin` / `<game>/config/yoake-wasm/<mod-id>/<key>.bin` entries (stripping `.bin`) within quota without overwriting newer keys (`Client` stores do not import legacy sources). The old `store.bin` and legacy `.bin` files are kept as recovery backups, and a `migration-v2.done` marker prevents repeated imports. Corrupted or unreadable entry files (or legacy entries during import) are skipped with a warning so healthy keys remain usable.[^storage]

> **Note:** This per-key storage design requires an engine version from `yoake/w2d-services` (`@4ec9e65b2`) or later.
> The on-disk format (`entry-*.bin`, `store.lock`, `migration-v2.done`, etc.) is internal and may change in future versions—never edit the files in the store folder directly or build external tools that depend on their layout; always read and write data through `storage_get`, `storage_put`, and the `storage_blob_*` calls.

---

## 4-3. Writing `[wasm] permissions`, `[config]`, and `[permissions."node"]` in `yak.toml`

Whenever a mod uses privileged features—reading or writing config, persisting storage, checking permissions, or running server commands—it must declare them in `yak.toml`.

### Available `[wasm] permissions`

There are 5 permissions (capabilities) you can declare:[^manifest]

- `"service.config"`: Calling `config_get`, `config_set`, or `config_dir`, and receiving synced config
- `"service.storage"`: Calling `storage_*` as well as `world_storage_*`, `config_read`, and `config_write`
- `"service.permissions"`: Calling `has_permission` from a server module (`wasm.server` is required)
- `"service.commands"`: Calling `run_command` from a server module (`wasm.server` is required)
- `"service.http"`: HTTP permission declaration paired with `[http] hosts` (note that HTTP transport itself is not connected yet)[^dev-doc]

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

### Writing `[permissions."node"]` (Excerpt from `permission-demo`'s Manifest)

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
[^storage]: Source: `DiskStorage::open_scoped_with_quota` / `persist` / `atomic_write` / `import_store` / `import_directory` in `crates/yoake-wasm/src/host_services/storage.rs` and `docs/yoake-wasm-dev.md`.
[^manifest]: Source: `crates/horizon-mods/src/yak/manifest.rs`.
[^dev-doc]: Source: `docs/yoake-wasm-dev.md`.
