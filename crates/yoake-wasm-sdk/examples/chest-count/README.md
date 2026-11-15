# `yoake-wasm-chest-count`

A minimal server-side Yoake WebAssembly mod (`server-full` world) that registers `/chestcount <x> <y> <z>`, reads that container in the sender's dimension with `services::container_slots`, and replies with per-item totals.

## What It Shows

- Registering a command with arguments and validating its coordinates.
- Building `types::BlockPos` and reading `types::ItemStack` values from container slots.
- Grouping stacks by item id and replying with `services::tell`.
- Using container services without an extra `[wasm] permissions` capability.

## Building

1. Compile the WebAssembly component for `wasm32-wasip2`:

   ```sh
   cargo build --release --target wasm32-wasip2 --offline
   ```

   `--offline` fails if `wit-bindgen` is not already in Cargo's cache. In that case, run the same build once without `--offline` while online, then retry the offline build.

2. For a manual Cargo build, copy the compiled `.wasm` artifact to the path declared in `yak.toml` (`bin/wasm/chest-count.wasm`):

   ```sh
   mkdir -p bin/wasm
   cp target/wasm32-wasip2/release/yoake_wasm_chest_count.wasm bin/wasm/chest-count.wasm
   ```

This branch has no `yoake check` command. With the p10-tools CLI, `yoake build` compiles and validates the component and copies it to the manifest path, so the manual copy step is not needed there.

## Installing

Run `yoake pack`, then import the generated `.yak` using the local-package steps in [3. ゲームに入れて遊ぶ・すぐ試す](../../../../docs/guide-ja/03-play.md). Restart the game and run, for example, `/chestcount 10 64 -20`.
