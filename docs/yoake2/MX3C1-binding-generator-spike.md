# MX3C1 spike: generate engine bindings for one module from rustdoc JSON (throwaway experiment; do not run git commands that write)

Root: C:\Users\USER\NestgateHorizon\horizon-integ (branch spike/s2-hooks). Read first:
- C:\Users\USER\NestgateHorizon\YOAKE_MIXIN2.md, especially sections 2.1, 4b and 4c;
- C:\Users\USER\NestgateHorizon\reports\MX3C0.md, the re-entry spike: core-module runtime, ACTIVE thread-local, fuel and epoch handling. Its code is in spikes\c0-reentry\.

Goal: show that we can auto-generate "call any engine function from a WASM core module" bindings for one real engine module, and measure what fails to map.

Shell (Git Bash): `export PATH="$HOME/.cargo/bin:/c/Users/USER/NestgateHorizon/toolchain/mingw64/bin:$PATH" CARGO_INCREMENTAL=0`, then use `--offline`. Run long builds in the background and wait for them.

Steps:
1. Produce rustdoc JSON for the `horizon-client` crate, the one that holds `game`:
   `cargo +nightly rustdoc -p horizon-client --lib -- -Z unstable-options --output-format json --document-private-items`
   If nightly isn't installed, check `rustup toolchain list`; if none, report that and stop. Do not install.
2. Write a small generator, `spikes/c1-bindgen/`, a standalone crate with its own `[workspace]` that reads the JSON. Target ONE module: the items/item-entity code (`crates/horizon-client/src/entities.rs` item parts) or the module holding `Game::native_item_*` / `native_entities_in`. Classify every function in it:
   - (a) mappable now: plain data args and returns, `&self`/`&mut self` on `Game` or a type reachable from `Game` by a path;
   - (b) mappable with rules: slices, Vec, Option, simple enums, structs of plain data;
   - (c) not mappable: generics, `impl Trait`, closures, `dyn`, functions returning references, lifetime-carrying structs.
   Report the counts and percentages, and list the reasons for (c).
3. For category (a), generate:
   - host glue: a Rust fn per binding with a stable numeric id, reading args from guest memory (a simple length-prefixed binary encoding is fine) and calling the real fn through a **path handle** (root `Game` + accessor list, re-borrowed per call), as described in 4b;
   - a guest-side Rust stub per binding, as text output only (the wasm32 targets may be missing; check `rustup target list --installed`).
4. Hand-wire 2–3 generated bindings into a test that runs inside the engine's own test harness, if you can do it without changing engine behaviour. If you can't, run it against a mock `Game` in the spike crate. Show one `&mut Game` method working through a path handle. Also show that the borrow table rejects two overlapping exclusive handles.
5. Measure the per-call overhead of the glue: decode args, resolve the path, call, encode the result.

Write the report to C:\Users\USER\NestgateHorizon\reports\MX3C1.md:
- counts of a/b/c with examples;
- the generator design;
- what the full 22k-function run would take;
- the measured overhead;
- open problems.

Keep all code under spikes/. Don't modify engine source except, if strictly needed, to expose one test hook (say so).
