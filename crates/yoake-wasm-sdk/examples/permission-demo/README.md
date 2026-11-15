# `yoake-wasm-permission-demo`

A minimal server-side Yoake WebAssembly mod (`server-full` world) that registers `/secret`, checks whether the invoking player has the `"secret.access"` permission node via `services::has_permission`, and replies differently depending on the result.

## What It Shows

- Declaring `permissions = ["service.permissions"]` under `[wasm]` and `[permissions."secret.access"]` (`default_op_level = 2`) in `yak.toml`.
- Implementing `yoake_wasm_sdk::full::server::ServerMod` and exporting it with `export_server_full!`.
- Subscribing to `yoake_wasm_sdk::full::hook::REGISTER_COMMANDS` and `yoake_wasm_sdk::full::hook::PERFORM_COMMAND`.
- Registering `/secret` at command level `0` so any player can invoke it and let `services::has_permission("secret.access", &uuid)` evaluate `<world>/permissions.toml` overrides or fall back to `default_op_level`.
- Replying to the player with `services::tell`.

## Building and Checking

1. Compile the WebAssembly component for `wasm32-wasip2`:

   ```sh
   cargo build --release --target wasm32-wasip2 --offline
   ```

2. Copy the compiled `.wasm` artifact to the path declared in `yak.toml` (`bin/wasm/permission-demo.wasm`):

   ```sh
   mkdir -p bin/wasm
   cp target/wasm32-wasip2/release/yoake_wasm_permission_demo.wasm bin/wasm/permission-demo.wasm
   ```

3. Validate the package manifest and WebAssembly component with the Yoake CLI:

   ```sh
   yoake check
   ```
