# Writing a Yoake WebAssembly mod

> **Build the engine with `YOAKE_VERSION=0.2.0`.** A source build without it reports version 0.1.0, and the game then refuses `.yak` packages whose `[yoake] version` range is `">=0.2, <1.0"` ("incompatible build"). The `.yak` id and version must also match the id and version in its `content/horizon-mod.toml`.

A Yoake wasm mod is a WebAssembly **component** (`wasm32-wasip2`) for the WIT world
`yoake:mods/client-mod@1.0.0` (`crates/yoake-wasm/wit/yoake-mod.wit`). The guest SDK is
`crates/yoake-wasm-sdk`; a complete example is `crates/yoake-wasm-sdk/examples/hello`.

## Toolchain

```sh
rustup target add wasm32-wasip2
```

## Build

```sh
cd crates/yoake-wasm-sdk/examples/hello
cargo build --release --target wasm32-wasip2
# -> target/wasm32-wasip2/release/yoake_wasm_hello.wasm  (about 18 KB)
```

Use `crate-type = ["cdylib"]` and, for small output, `opt-level = "s"`, `lto = true`,
`strip = true`, `panic = "abort"` in `[profile.release]`. The SDK crates are excluded from the
host workspace; each has its own `[workspace]` table.

## Mod code

```rust
#![no_std]
use yoake_wasm_sdk::prelude::*;
use yoake_wasm_sdk::{Mod, export_mod, hook, hud};

#[derive(Default)]
struct Hello;

impl Mod for Hello {
    fn subscriptions() -> &'static [&'static str] { &[hook::HUD] }
    fn hud(&mut self) { let _ = hud::text("hi", (4.0, 4.0), 0xffffff, true); }
}
export_mod!(Hello);
```

- Implement `Mod` (all hooks default to no-ops) and list every hook you override in
  `subscriptions()` using the `hook::*` constants. The host only calls subscribed hooks.
- **Normal `std` works** (SDK feature `std`, on by default): `println!`/`eprintln!` go to the mod's
  log (one line per newline, long lines cut, bounded per call), `std::time::{Instant, SystemTime}`,
  `HashMap`, threads-free code, random (`HashMap`'s hasher) all work. The host links exactly this
  WASI preview 2 subset: `wasi:cli/{stdin,stdout,stderr,environment,exit,terminal-*}`,
  `wasi:clocks/{monotonic-clock,wall-clock}`, `wasi:random/*`, `wasi:io/*` and
  `wasi:filesystem/{types,preopens}`. stdin is empty, the environment and the arguments are empty,
  `std::process::exit` traps the mod (only that mod is disabled), the filesystem has **no
  preopened directory** (every `std::fs` call fails; the interfaces are linked only because `std`
  imports them) and there are **no sockets** (a component that imports `wasi:sockets/*`, or any
  other interface outside `yoake:mods/*` and the list above, is rejected at load). The example is
  `crates/yoake-wasm-sdk/examples/hello-std`.
- For the smallest modules turn `std` off (`default-features = false, features = ["runtime"]`) and
  write `#![no_std]`: the SDK ships its own allocator and panic handler (feature `runtime`) and
  `yoake_wasm_sdk::prelude` re-exports `String`, `Vec`, `Box`, `format!`, `vec!`. A panic traps
  the instance. See `examples/hello`.
- Host calls: `log`, `feedback`, `hud::{text, rect, text_width}`, `settings::{get, set}`,
  `net_send`, `local_player`, `playing`, `world_key`, `snapshot_world`, `instance_dir`,
  `account`, `screen::*`. All return `Result<_, HostError>` (`net_send`: `SendError`).
- The generated bindings come from `build.rs` (wit-bindgen 0.57.1's proc macro does not escape the
  package name `mod`).

Check the imports of a build (`yoake:mods/*` and, for `std` mods, the WASI subset):
`wasm-tools component wit target/wasm32-wasip2/release/yoake_wasm_hello.wasm`.

## Packaging in a `.yak`

Put the component at `bin/wasm/<name>.wasm` and declare it in `yak.toml`:

```toml
[wasm]
client = "bin/wasm/hello.wasm"
interface = "1.0"
```

`interface = "1.0"` is the `yoake:mods` major.minor the component was built against; the host
rejects components that import anything outside `yoake:mods/*` and the WASI subset above.

## Developer mode: hot reload

With `YOAKE_DLL_MODS_UNSIGNED=1` the game polls every unsigned wasm module of an installed `.yak`
once a second and, when the file changed (and stayed unchanged for a whole poll), builds a fresh
instance and swaps it in at the next frame or tick. A new build that fails to load keeps the old
instance running (one warning in the log). `YOAKE_WASM_DEV_DIR=<folder>` additionally loads every
`<id>.wasm` in the folder as a mod (no `.yak` needed) and reloads it the same way. Both are off
without the flag. The startup log has one line saying `wasm: no mods (engine not started)` or
`wasm: N mod(s), engine started`.
