# `yoake-wasm-hud-item`

A minimal client-side Yoake WebAssembly mod (`client-full` world) that draws a background box, a `"minecraft:diamond"` item icon, and a text label on the HUD.

## What It Shows

- Implementing `yoake_wasm_sdk::full::client::ClientMod` and exporting it with `export_client_full!`.
- Subscribing to `yoake_wasm_sdk::full::hook::HUD` so the host calls `ClientMod::hud` each frame.
- Returning `et::HudOutcome` containing `et::HudCommand::Rect`, `et::HudCommand::Item` (`et::ItemStack`), and `et::HudCommand::Text` commands.
- Declaring a client-side `client-full` package in `yak.toml`.

## Building and Checking

1. Compile the WebAssembly component for `wasm32-wasip2`:

   ```sh
   cargo build --release --target wasm32-wasip2 --offline
   ```

2. Copy the compiled `.wasm` artifact to the path declared in `yak.toml` (`bin/wasm/hud-item.wasm`):

   ```sh
   mkdir -p bin/wasm
   cp target/wasm32-wasip2/release/yoake_wasm_hud_item.wasm bin/wasm/hud-item.wasm
   ```

3. Validate the package manifest and WebAssembly component with the Yoake CLI:

   ```sh
   yoake check
   ```
