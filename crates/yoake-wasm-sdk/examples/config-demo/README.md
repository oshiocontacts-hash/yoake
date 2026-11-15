# `yoake-wasm-config-demo`

A minimal client-side Yoake WebAssembly mod (`client-full` world) that declares `text` and `color` settings in `[config]` of `yak.toml`, reads them at startup via `services::config_get`, and refreshes its HUD banner whenever `ClientMod::config_changed` fires.

## What It Shows

- Declaring typed client settings (`type = "string"` and `type = "enum"` with `values = [...]`) under `[config]` in `yak.toml`, along with `permissions = ["service.config"]` under `[wasm]`.
- Subscribing to `yoake_wasm_sdk::full::hook::HUD` and `yoake_wasm_sdk::full::hook::CONFIG_CHANGED`.
- Reading initial config values in `ClientMod::init` with `yoake_wasm_sdk::full::client::services::config_get`.
- Updating cached HUD state in `ClientMod::config_changed` from `types::ConfigValue` (`types::ConfigValueKind::String` and `types::ConfigValueKind::Enum`).

## Building

1. Compile the WebAssembly component for `wasm32-wasip2`:

   ```sh
   cargo build --release --target wasm32-wasip2 --offline
   ```

   `--offline` fails if `wit-bindgen` is not already in Cargo's cache. In that case, run the same build once without `--offline` while online, then retry the offline build.

2. For a manual Cargo build, copy the compiled `.wasm` artifact to the path declared in `yak.toml` (`bin/wasm/config-demo.wasm`):

   ```sh
   mkdir -p bin/wasm
   cp target/wasm32-wasip2/release/yoake_wasm_config_demo.wasm bin/wasm/config-demo.wasm
   ```

This branch has no `yoake check` command. With the p10-tools CLI, `yoake build` compiles and validates the component and copies it to the manifest path, so the manual copy step is not needed there.
