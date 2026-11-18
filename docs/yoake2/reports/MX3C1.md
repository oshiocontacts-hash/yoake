# MX3C1: generate engine bindings from rustdoc JSON

Status: the Round 1 generator, runtime tests, target run, whole-`horizon-client` run, and native overhead measurements are complete. Round 2 fixes the generated host glue and adds a compile-and-call test for the generated fixture output. Code is in `spikes/c1-bindgen/`; no engine source was changed.

## Inputs and reproducibility

The 50 MB input JSON contains 59,986 items and was produced on stable Rust with `RUSTC_BOOTSTRAP=1`, using rustdoc's unstable JSON output and private-item documentation. It uses `format_version` 61. Production must pin one exact toolchain and its rustdoc `format_version`; the generator records the observed version in `summary.json`, but does not claim compatibility with other versions.

The recorded runs used Windows MSVC stable 1.100.0, release mode, and `-j 4` on LENOVO.

Run from the repository root:

```sh
cargo run --manifest-path spikes/c1-bindgen/Cargo.toml --release -- \
  C:/Users/suke0/YoakeBuild/target/doc/horizon_client.json \
  spikes/c1-bindgen/out \
  native_entity_query.rs native_client_items.rs
cargo test --manifest-path spikes/c1-bindgen/Cargo.toml
cargo run --manifest-path spikes/c1-bindgen/Cargo.toml --release --example overhead
```

Span matching is suffix-based and treats slash and backslash equally. The output directory receives `classification.md`, `summary.json`, `ids.lock.json`, `host_glue.rs`, and `guest_stubs.rs`.

## Classification results

The two target files contain 33 functions and produce four category-(a) bindings. The first target run took 7.3 seconds including the release build.

| Category | Count | Percent |
|---|---:|---:|
| (a) mappable now | 4 | 12.1% |
| (b) mappable with rules | 12 | 36.4% |
| (c) not mappable | 17 | 51.5% |
| Total | 33 | 100% |

The generator includes free functions, inherent methods, and functions from derived trait impls. It records each impl's `for` type and substitutes that type for `Self` during classification.

Category (a) is deliberately narrow: fixed-width scalar data, `bool`, owned or borrowed strings, fixed arrays of those types, unit, and ordinary `Game` receivers. The four generated bindings are the three native item/tag methods and `midpoint`.

Category (b) covers copies that need a schema or convention: slices, `Vec`, `Option`, `Result`, tuples, plain-data structs, simple enums, non-`Game` path receivers, and explicit fixed-width treatment of `usize`/`isize`. Examples are `native_entities_in` (`Vec<NativeEntityInfo>` inside `Result`), `player_id` (`Result`), `coordinates` (`Vec3`), and `require_server_context` (an enum).

Category (c) includes generics and `impl Trait`, closures and `dyn Trait`, reference returns, generic lifetimes, lifetime-carrying structs, async/unsafe/const functions, and unsupported receivers. Examples are `NativeEntityInfo::new` (`impl Into<String>`), `Candidate` methods (the struct carries a lifetime), `QuerySources::visit` (a `dyn FnMut` argument), and `QueryBudgets::reserve` (a mutable reference return). Every (b) and (c) row has a reason, and `summary.json` contains the individual-reason histogram.

## Whole-crate coverage and reasons

Running with span suffix `.rs` selected all 12,879 functions in `horizon-client` and took 1.4 seconds after the build.

| Category | Count | Percent |
|---|---:|---:|
| (a) mappable now | 1,863 | 14.5% |
| (b) mappable with rules | 6,394 | 49.6% |
| (c) not mappable | 4,622 | 35.9% |
| Total | 12,879 | 100% |

The most frequent category-(b) reason occurrences are:

- Borrowed non-string arguments needing copy/write-back or handle rules: 5,546.
- `Option` container rules: 4,151.
- `Vec` container rules: 1,855.
- An explicit fixed-width rule for `usize`: 1,850.
- `Result` container rules: 1,496.

The most frequent category-(c) blockers are `dyn Trait` (1,884 occurrences), reference returns (398), direct `impl Trait` type nodes (245), lifetime-carrying `Formatter` arguments (239), and generic function lifetimes (40). The histogram splits generic and named-type reasons by type or parameter name, so those entries are not a complete aggregate of all generic-related failures. Counts are reason occurrences, not unique functions; one function can have several reasons.

## Generator design

`model.rs` scans the rustdoc index, selects function spans, builds an impl-member map, and constructs stable paths. Trait methods include the trait in their path so identical method names from different impls remain distinguishable. Category-(a) paths are sorted before numeric IDs are assigned; the path, ID, rustdoc item ID, and source span are written to `ids.lock.json`.

`classify.rs` recursively inspects rustdoc types. It resolves local record definitions to find hidden lifetime/generic parameters and uses a visiting set to avoid recursive-type loops. A hard failure dominates a rule requirement, which dominates mappable-now. Reasons are sorted and deduplicated for deterministic reports.

`render/host.rs` emits one `call_<id>` function per binding. It decodes from a checked `Decoder`, rejects trailing bytes, re-borrows `Game` through `ResolvePath`, calls the real function, and encodes the return value. Free functions retain the uniform root/path signature but do not resolve a receiver. The text is intended to be included inside the originating private module, as required by the design.

`render/guest.rs` emits wrappers over the single `engine_call(id, ptr, len)` import. The in/out frame starts with a 64-bit generation-checked table handle and a 32-bit argument length, followed by arguments and zeroed output capacity. On success the host overwrites the frame with a 32-bit result length and result bytes; the integer return is a status code. The generated guest text has its own small codec, so it does not import the host runtime.

## Runtime and compile validation

`rt.rs` contains:

- `PathHandle`: a `Game` root plus ordered field, entity-ID, and slot accessors;
- `HandleTable`: slot reuse with generation checks and explicit removal;
- `BorrowTable`: shared/exclusive grants with explicit release tokens;
- `ResolvePath<T>`: the generated/engine-specific re-borrow operation.

Two paths overlap when their roots match and either accessor list is a prefix of the other. An exclusive grant conflicts with any overlapping grant; shared grants may coexist. The Round 1 mock suite compiled and passed four tests with no failures, covering hand-wired calls, a mutating `Game` method, borrow conflicts and sharing, stale handles, and malformed input.

Round 2 adds `build.rs`, which runs the same extraction, classification, ID assignment, and host renderer over `tests/fixtures/sample-items.json` into `OUT_DIR`. `tests/generated_glue.rs` uses `include!` on that generated file, provides a lifetime-carrying mock `Game<'a>` and the real four signatures, then invokes and decodes every generated binding. Thus `cargo test` is now the compile gate for generator output, not only for hand-written glue.

The codec uses little-endian fixed-width numbers, one-byte bool/Option/Result tags, 32-bit byte/string/vector lengths, fixed-array element order, and tuple field order. Host runtime code uses no `unsafe`. The guest text uses `unsafe` only at the extern import call.

### Lesson from Round 1 output

Generated text was not compiled initially, and two errors survived:

1. Zero-argument functions emitted a direct `Decoder::new(args).finish()?` and then referred to a nonexistent `decoder` variable.
2. `impl ResolvePath<Game<'_>> for Game<'_>` gave the trait and self types independent anonymous lifetimes. The resolver now uses `impl<'g> ResolvePath<Game<'g>> for Game<'g>` and returns `&mut Game<'g>`.

Generated glue must be compiled in CI. Text inspection and hand-written equivalents did not prove the actual output valid; the `OUT_DIR` include test now exercises the real renderer.

## Measured overhead

The benchmark measures decode + root-path resolution + real mock call + result encoding. These are medians of 11 samples after one warmup, with 100,000 calls per sample. It measures native glue only, not a WASM transition.

| Case | Glue ns/call | Direct ns/call | Difference |
|---|---:|---:|---:|
| Zero-argument getter | 4.8 | 0.2 | 4.6 |
| Three-scalar plain-data call | 8.4 | 0.6 | 7.8 |
| 64-byte string call | 37.3 | 0.5 | 36.8 |

The roughly 5–37 ns native glue cost is small next to C0's 110–150 ns core-WASM round trip and 320–390 ns round trip with 64-byte arguments and allocation. String decoding/allocation is the dominant measured glue case. A production benchmark should still measure the combined dispatcher, handle validation, borrow grants, and WASM boundary.

## Full multi-crate estimate

`horizon-client` alone has 12,879 functions and its warmed generation run took 1.4 seconds. Linear scaling to the design's roughly 22,000 functions gives approximately 2.4 seconds for generation across the planned crates, excluding rustdoc production, compilation, and any additional cross-crate resolution. This is an estimate from one crate, not a measured multi-crate result; JSON parsing and retained document memory remain likely resource drivers, and peak memory/output size were not recorded.

## Open problems

- Category (b) does not yet emit bindings. It needs record/enum schemas, container renderers, borrowed-plain-data write-back, and non-root path resolvers.
- The host text references `c1_bindgen::rt`; production must move that runtime into an engine-owned module or adjust imports before `include!`ing the glue.
- Only root `Game` resolution is emitted. Field/entity/slot resolution needs type-directed accessor implementations and activation-scoped handle release.
- Borrow acquisition is implemented and tested separately, but the dispatcher still needs to define grant lifetimes around calls and child handles returned from references.
- The guest output buffer has a fixed provisional capacity. Production needs sizing, growth/retry, or caller-provided scratch capacity and a defined trap/error mapping.
- IDs are deterministic for a selected function set. A production lock file must preserve existing assignments when earlier-sorting functions are added, detect signature changes, and provide removed-function shims.
- Raw identifiers, duplicate guest names across modules, generic const array lengths, and richer rustdoc type nodes need explicit handling before broad generation.
- Derived trait methods are included in the denominator; a production coverage report may need a separate hand-written/public-API view to avoid a misleading percentage.
