# `yoake-wasm-player-vitals`

A minimal server-side Yoake WebAssembly mod (`server-full` world) that registers `/vitals`. It reports health from the command hook's `et::NativeServerPlayer` record and reads whether `minecraft:story/root` is known and done with `services::advancement_progress`.

## What It Shows

- Reading `player.health` from `et::NativeServerPlayer`.
- Querying `types::AdvancementState` with `services::advancement_progress`.
- Handling service errors while still reporting the available health value.
- Using player and advancement services without an extra `[wasm] permissions` capability.

## Building

1. Compile the WebAssembly component for `wasm32-wasip2`:

   ```sh
   cargo build --release --target wasm32-wasip2 --offline
   ```

   `--offline` fails if `wit-bindgen` is not already in Cargo's cache. In that case, run the same build once without `--offline` while online, then retry the offline build.

2. For a manual Cargo build, copy the compiled `.wasm` artifact to the path declared in `yak.toml` (`bin/wasm/player-vitals.wasm`):

   ```sh
   mkdir -p bin/wasm
   cp target/wasm32-wasip2/release/yoake_wasm_player_vitals.wasm bin/wasm/player-vitals.wasm
   ```

This branch has no `yoake check` command. With the p10-tools CLI, `yoake build` compiles and validates the component and copies it to the manifest path, so the manual copy step is not needed there.

## Installing

Run `yoake pack`, then import the generated `.yak` using the local-package steps in [3. ゲームに入れて遊ぶ・すぐ試す](../../../../docs/guide-ja/03-play.md). Restart the game and run `/vitals`.
