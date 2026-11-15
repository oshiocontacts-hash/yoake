# `mod-call-pair`

Two server-side Yoake WebAssembly mods (`server-full` world) that demonstrate package-to-package calls. `caller` registers `/ping <text>` and calls `mod-call-responder` with `services::mod_call`; `responder` answers through `ServerMod::handle_call` with a UTF-8 `pong` message.

Both packages must be built, installed, and enabled. Neither needs a `[wasm] permissions` capability, and the manifest has no separate mod-call target field: the caller uses the responder's package id, `mod-call-responder`.

## Building

Build each crate separately from its own folder:

```sh
cargo build --release --target wasm32-wasip2 --offline
```

`--offline` fails if `wit-bindgen` is not already in Cargo's cache. In that case, run the same build once without `--offline` while online, then retry the offline build.

For manual Cargo builds, copy the artifacts to the paths declared in each `yak.toml`:

```sh
mkdir -p caller/bin/wasm responder/bin/wasm
cp caller/target/wasm32-wasip2/release/yoake_wasm_mod_call_caller.wasm caller/bin/wasm/mod-call-caller.wasm
cp responder/target/wasm32-wasip2/release/yoake_wasm_mod_call_responder.wasm responder/bin/wasm/mod-call-responder.wasm
```

This branch has no `yoake check` command. With the p10-tools CLI, `yoake build` compiles and validates the component and copies it to the manifest path, so the manual copy step is not needed there. Run it once in each crate folder.

## Installing

Run `yoake pack` in both `caller/` and `responder/`, then import both generated `.yak` files (`mod-call-caller` and `mod-call-responder`) using the local-package steps in [3. ゲームに入れて遊ぶ・すぐ試す](../../../../docs/guide-ja/03-play.md). Restart the game after installing them, then run `/ping hello`.
