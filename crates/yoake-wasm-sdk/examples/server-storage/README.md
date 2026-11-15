# `yoake-wasm-server-storage`

A minimal server-side Yoake WebAssembly mod (`server-full` world) that registers `/visits`, increments a per-player counter persisted in world storage, and replies to the player with the updated count.

## What It Shows

- Implementing `yoake_wasm_sdk::full::server::ServerMod` and exporting it with `export_server_full!`.
- Subscribing to `yoake_wasm_sdk::full::hook::REGISTER_COMMANDS` and `yoake_wasm_sdk::full::hook::PERFORM_COMMAND`.
- Registering a custom `/visits` command via `et::NativeCommand`.
- Reading and writing persistent per-world key-value data with `services::storage_get`, `services::storage_put`, and `types::StorageScope::World`.
- Replying to the command sender with `services::tell`.
- Declaring `permissions = ["service.storage"]` in `yak.toml`.

## Building and Checking

1. Compile the WebAssembly component for `wasm32-wasip2`:

   ```sh
   cargo build --release --target wasm32-wasip2 --offline
   ```

2. Copy the compiled `.wasm` artifact to the path declared in `yak.toml` (`bin/wasm/server-storage.wasm`):

   ```sh
   mkdir -p bin/wasm
   cp target/wasm32-wasip2/release/yoake_wasm_server_storage.wasm bin/wasm/server-storage.wasm
   ```

3. Validate the package manifest and WebAssembly component with the Yoake CLI:

   ```sh
   yoake check
   ```
