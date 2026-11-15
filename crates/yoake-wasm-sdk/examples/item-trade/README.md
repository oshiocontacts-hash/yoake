# `yoake-wasm-item-trade`

A server-side Yoake WebAssembly mod (`server-full` world) that registers `/trade`. It takes exactly 16 `minecraft:cobblestone` all-or-nothing with `ext::take` (the `inventory_apply` helper), then gives one `minecraft:diamond` with `services::give_items`. If the whole reward does not fit, it removes any delivered reward and returns the input to cancel the trade; incomplete undo attempts are reported explicitly.

## What It Shows

- Taking exactly 16 items all-or-nothing with `ext::take`; any components match, and the off hand is searched.
- Creating a default `types::ItemStack` with `ext::item`.
- Reading leftover stacks from `services::give_items` and undoing an incomplete trade without claiming success if the undo fails.
- Using inventory services without an extra `[wasm] permissions` capability.

## Building

1. Compile the WebAssembly component for `wasm32-wasip2`:

   ```sh
   cargo build --release --target wasm32-wasip2 --offline
   ```

   `--offline` fails if `wit-bindgen` is not already in Cargo's cache. In that case, run the same build once without `--offline` while online, then retry the offline build.

2. For a manual Cargo build, copy the compiled `.wasm` artifact to the path declared in `yak.toml` (`bin/wasm/item-trade.wasm`):

   ```sh
   mkdir -p bin/wasm
   cp target/wasm32-wasip2/release/yoake_wasm_item_trade.wasm bin/wasm/item-trade.wasm
   ```

This branch has no `yoake check` command. With the p10-tools CLI, `yoake build` compiles and validates the component and copies it to the manifest path, so the manual copy step is not needed there.

## Installing

Run `yoake pack`, then import the generated `.yak` using the local-package steps in [3. ゲームに入れて遊ぶ・すぐ試す](../../../../docs/guide-ja/03-play.md). Restart the game, hold at least 16 cobblestone, and run `/trade`.
