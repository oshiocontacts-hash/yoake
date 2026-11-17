# `yoake-wasm-permission-demo`

A minimal server-side Yoake WebAssembly mod (`server-full` world) that registers `/secret`, checks whether the invoking player has the `"secret.access"` permission node via `services::has_permission`, and replies differently depending on the result.

## What It Shows

- Declaring `permissions = ["service.permissions"]` under `[wasm]` and `[permissions."secret.access"]` (`default_op_level = 2`) in `yak.toml`.
- Implementing `yoake_wasm_sdk::full::server::ServerMod` and exporting it with `export_server_full!`.
- Subscribing to `yoake_wasm_sdk::full::hook::REGISTER_COMMANDS` and `yoake_wasm_sdk::full::hook::PERFORM_COMMAND`.
- Registering `/secret` at command level `0` so any player can invoke it and let `services::has_permission("secret.access", &uuid)` evaluate `<world>/permissions.toml` overrides or fall back to `default_op_level`.
- Replying to the player with `services::tell`.

## Per-player Override

To allow one player explicitly, add this to `<world>/permissions.toml`, replacing `<uuid>` with that player's UUID:

```toml
[mods.permission-demo.players.'<uuid>']
'secret.access' = true
```

The player must be online and the world must be open when `/secret` checks the permission.

## Building

1. Compile the WebAssembly component for `wasm32-wasip2`:

   ```sh
   cargo build --release --target wasm32-wasip2 --offline
   ```

   `--offline` fails if `wit-bindgen` is not already in Cargo's cache. In that case, run the same build once without `--offline` while online, then retry the offline build.

2. For a manual Cargo build, copy the compiled `.wasm` artifact to the path declared in `yak.toml` (`bin/wasm/permission-demo.wasm`):

   ```sh
   mkdir -p bin/wasm
   cp target/wasm32-wasip2/release/yoake_wasm_permission_demo.wasm bin/wasm/permission-demo.wasm
   ```

This branch has no `yoake check` command. With the p10-tools CLI, `yoake build` compiles and validates the component and copies it to the manifest path, so the manual copy step is not needed there.
