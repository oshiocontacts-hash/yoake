# tested-counter (`yoake-wasm-tested-counter`)

A small beginner example showing a client-side Yoake WebAssembly mod (`TestedCounter`) paired with
`#[yoake_test]` engine tests (`YOAKE_TESTS`) in the same crate.

- **Engine tests (`yoake test`)**: The `#[yoake_test]` tests are compiled into a separate `test-mod`
  component (`--features tests`) that exercises the engine's server-side world through `TestWorld`;
  they do not load or drive the `TestedCounter` client mod's hooks (`tick` / `hud`).
- **Host unit tests (`cargo test`)**: Running `cargo test` on the host runs `YOAKE_TESTS` against a
  local `MockWorld` (`TestDriver` mock), not the game engine.

## Building

Install the WebAssembly target once:

```sh
rustup target add wasm32-wasip2
```

Build the client mod (`client-mod` world):

```sh
cargo build --release --target wasm32-wasip2
```

Build the test module (`test-mod` world, enabled by the `tests` Cargo feature):

```sh
cargo build --release --target wasm32-wasip2 --features tests
```

Run the local mock-driver unit test on the host (against `MockWorld`, not the game):

```sh
cargo test
```

## Running with the `yoake` CLI

`yoake build` compiles the client module into `bin/wasm/tested-counter.wasm` and validates it with
`yoake check`. `yoake test` additionally compiles the test module (`--features tests`) into
`bin/wasm/tested-counter-tests.wasm` and runs every test in `YOAKE_TESTS` inside an offscreen
`horizon-client` process:

```sh
yoake build
yoake check
export YOAKE_GAME=/path/to/horizon-client
yoake test -- --resources /path/to/prepared/resources
```
