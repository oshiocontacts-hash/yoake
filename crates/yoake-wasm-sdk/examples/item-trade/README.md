# `yoake-wasm-item-trade`

A server-side Yoake WebAssembly mod (`server-full` world) that registers `/trade`. It first gives one `minecraft:diamond` with `services::inventory_apply` (`Give`: all-or-nothing, main slots only); if it does not fit, the player is told the inventory is full and nothing changes. Then it takes exactly 16 `minecraft:cobblestone` all-or-nothing with `ext::take`; if that fails, the diamond is taken back with `ext::take_exact`, so no input ever has to be returned.

## What It Shows

- Giving first, then taking, so a failure never loses the input.
- Taking exactly 16 items all-or-nothing with `ext::take`; it matches any components (named or tagged items too) and searches the off hand, while `ext::take_exact` matches only the exact item, count and components.
- Creating a default `types::ItemStack` with `ext::item`.
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
