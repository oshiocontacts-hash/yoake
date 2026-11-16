# Mixin demo

This standalone WASI Preview 2 component uses the injection world rather than the
ordinary hooks world. It halves the native fall-damage result, doubles the native
bonemeal crop-growth increment, and logs submitted chat to the bounded host log.
It does not double ordinary random-tick crop growth.

Build from this directory with the installed toolchain:

```sh
cargo build --offline -j 4 --release --target wasm32-wasip2
```

Stage the resulting `yoake_mixin_demo.wasm` as `bin/wasm/mixin-demo.wasm` beside
`yak.toml`, using a separate package staging directory. The same component is used
for both sides; the host filters server-only targets from the client instance.
The chat target requires the declared `service.commands` permission and, on the
client, `service.chat`, even though this example only observes the text.
The target signatures are pinned to the committed engine catalog. A changed
signature requires reviewing the new catalog and rebuilding the mod.

Packaging, signing, and distribution follow the normal `.yak` owner workflow.
Do not distribute an unsigned package as an official Yoake mod or include build
artifacts in a source commit. See [the injection guide](../../../../docs/yoake-mixin.md)
for value shapes, scoped handles, registration, failure behavior, and limits.
