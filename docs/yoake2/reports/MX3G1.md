# MX3G1: the binding generator inside the real engine

Branch `yoake2/slice` (working tree on top of 4405a0bc6). No git writes, no installs, no game launch. Default builds are unchanged: all glue is behind the cargo feature `yoake2-slice` (`include!` under `#[cfg(feature = ...)]`) or in new crates the default build does not compile.

## What was built

| Path | What |
|---|---|
| `crates/yoake-bindgen/` (workspace member) | The generator, moved out of `spikes/c1-bindgen` and rewritten: `doc.rs` (rustdoc JSON, cross-crate id resolution), `shape.rs` (what is plain data), `analyze.rs` (functions, fields, receivers), `render.rs` (glue), `lock.rs` (ids), `reach.rs`, `report.rs`. Binary `yoake-bindgen` (`--config`, `--check`, `--survey CRATE`). `bindgen.json` = config, `ids.lock.json` = ids, `regen.sh` = rustdoc JSON + generator. |
| `crates/yoake-slice/src/codec.rs` | The wire codec (moved from the spike's `rt`), no dependencies. |
| `crates/yoake-slice/src/activation.rs` | **Change to the approved core:** `with_arg::<T>(handle, f)` (typed, `TypeId`-checked lent receiver) and `Status::WrongType`. One new `unsafe` deref, same discipline as `get`/`set`. For Fable's review. |
| `crates/horizon-items/src/yoake_glue/items.rs`, `crates/horizon-client/src/yoake_glue/{entities,game_items,entity_query}.rs` | Generated glue, `include!`d at the end of `items.rs`, `entities.rs`, `game/native_client_items.rs`, `game/native_entity_query.rs`. |
| Cargo manifests | feature `yoake2-slice` on horizon-items (new optional dep yoake-slice), horizon-client (new), horizon-client-lib (forwards to horizon-items). Cargo.lock gains only those edges and yoake-bindgen. |
| `crates/horizon-client/tests/yoake2_glue.rs` | 5 tests of the glue on real engine objects. |
| `docs/yoake2/bindgen/` | `coverage.md`, `summary.json`, `functions-<module>.md` (every function with its reason), `survey-horizon_client_lib.md`, `survey-horizon_client.md`. |

The old `spikes/c1-bindgen` is left untouched (MX3C1 stays reproducible); it is superseded and can be deleted.

Modules bound: the two asked for, `items` (`horizon_items::items`, `crates/horizon-items/src/items.rs`) and `entities` (`horizon_client_lib::entities`), plus the two `Game`-root modules `game::native_client_items` and `game::native_entity_query` so the `Game` receiver path is compiled in the real engine too.

Run: `crates/yoake-bindgen/regen.sh` (add `--check` for CI mode).

## Classifier (Fable's list)

| Item | Result | Proof |
|---|---|---|
| tuple, unit and data-carrying enums are plain data | Yes: struct, tuple struct, unit struct, enums with unit/tuple/struct variants (tag = u32 variant index) | fixture `Mode`, `Meters`, `Marker`; engine `StandMedium::Lava{shallow}` |
| non-Game receiver whose Self is not data = handle, not (c) | Yes: any `&self`/`&mut self` on a non-generic, lifetime-free struct/enum | fixture `Thing` (holds a `Cell`) |
| inspect external types' type arguments | A hard failure inside a std/unloaded type's arguments (`dyn`, closure) is reported as excluded, not hidden behind a handle reason | fixture `cb_cell` vs `plain_cell` |
| inspect impl-level generics | `impl<T> .. for Vec<T>`, generic trait impls, lifetime-carrying Self are excluded; lifetimes alone are harmless | fixture `Tick for Vec<T>` |
| split (b) | `BoundA` (scalars, strings, arrays), `BoundMech` (usize, char, Option, Vec, Result, Box, Arc/Rc, maps, sets, tuples, plain structs/enums, `&mut` write-back, `Option<&T>`), `HandleRequired`, `Excluded` | coverage tables |

Other fixes: `const fn` is no longer excluded; function-level lifetime parameters are ignored; `&T` returns of data are copied out; map aliases that only change the hasher (`FxHashMap`) are data; derived trait impls are left out of the denominator (reported separately).

## Receivers and fields

- `&self`/`&mut self` on a handle type: `call(handle, id, args, out)` runs it through `activation::with_arg::<T>`; wrong type, stale, forged index and locked handles are `CallError::Handle(status)`, nothing runs, `out` is left untouched.
- `Game` root: `call_game(&mut Game, id, ..)`. Only compiled in the engine (game_items, entity_query) and run in the fixture; no real `Game` instance was built in a test.
- Reachability: hook argument (any bound method of the type) and, in the report only, the shortest `Game` field path (for example `Game.dropped` for `ItemEntity`, `Game.armor_stands`, `Game.world_tables.items` for `Items`). Resolving those paths at run time is not built.
- Fields: getter and setter per plain-data field, private fields included, ids in the lock (`entities::ItemEntity.fire_ticks#get`). Fields that cannot be data (private fields of another module such as `JoinSeq(u64)`, `Aabb.min`) correctly get none. A scalar `FieldTable` per type (`FIELDS_<type>`) works with `activation::get/set`.
- Generated code: no `unsafe`, no raw pointers (a test greps for them). Arguments are decoded with `d.finish()` (trailing bytes are an error); a vector length is bounded by the remaining bytes.

## Coverage (non-derived functions; derived trait methods are bound too but counted apart)

| module | functions | a | b | **bound now** | handle-required | excluded | derived bound/total | fields with accessors |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| items | 69 | 7 | 13 | **20 (29.0%)** | 15 (21.7%) | 34 (49.3%) | 8/21 | 73/90 |
| entities | 101 | 30 | 25 | **55 (54.5%)** | 32 (31.7%) | 14 (13.9%) | 9/23 | 85/97 |
| game_items | 3 | 3 | 0 | 3 (100%) | 0 | 0 | 0/0 | 0/0 |
| entity_query | 20 | 1 | 6 | 7 (35.0%) | 4 | 9 | 6/10 | 14/43 |

Glue size: 452 bindings in the lock. Main blockers, entities: `&World` (an unloaded handle type, 10 functions), `JoinSeq` and `Aabb` private fields (about 15 functions between them), lifetime-carrying receivers (`StandClimbing`). Items: generic functions (15 with a type parameter), `serde_json::Value`, lifetime-carrying loot contexts. Full reason tables and every unbound function: `docs/yoake2/bindgen/`.

**Whole crate (classifier only, nothing bound, each module as if its own glue were included):**

| crate | modules | functions | bound now | handle-required | excluded |
|---|---:|---:|---:|---:|---:|
| horizon_client_lib | 275 | 8530 | 2557 (30.0%) | 2553 (29.9%) | 3420 (40.1%) |
| horizon_client | 408 | 9265 | 4394 (47.4%) | 3306 (35.7%) | 1565 (16.9%) |

These are what the classifier would bind; only the four modules above are generated.

## Compile check and CI

- `cargo test -p yoake-bindgen --test engine -- --ignored`: rustdoc JSON of six engine crates (feature off), regenerate in memory, fail on any difference to the checked-in glue, lock or reports, then `cargo check -p horizon-client --features yoake2-slice`. It passed (150 s warm; 307 s when rustdoc was cold) and it did fail correctly on a stale report during development.
- `cargo check -p horizon-client-lib --features yoake2-slice` and `-p horizon-client --features yoake2-slice`: compile clean.
- Fast gate without the engine: `tests/mini_engine.rs` generates glue for a fixture module, compiles it with `rustc` against the real `yoake_slice` rlib and runs assertions (every binding kind, wrong-type/stale/forged handles, malformed input, write-back, enum tags), plus classifier decisions, determinism, and lock stability (new binding takes the next id, removed one stays `unbound`, changed signature reported, id never reused).

## Tests

| Suite | Result |
|---|---|
| yoake-bindgen `mini_engine` | 5 passed |
| yoake-bindgen `engine` (ignored, CI) | 1 passed |
| yoake-slice (`--features wasm`): unit 17 (13+1 activation, 2 codec), `slice.rs` 21, doctest 1 | all passed |
| horizon-client-lib `--features yoake2-slice`: `yoake2_glue` 5, `yoake2_slice` 7 (+1 ignored bench) | all passed |
| feature-off `cargo test -p horizon-client --lib` | **1193 passed, 0 failed, 40 ignored** (unchanged; 533 s including the build) |

## Build time and size (this PC, windows-gnu, dev profile, `-j 4`, `CARGO_INCREMENTAL=0`; sources touched, deps warm)

| | feature off | feature on | delta |
|---|---:|---:|---:|
| `cargo check -p horizon-client-lib` (items + client-lib recompiled), 2 runs | 43.1 s / 41.8 s | 45.8 s / 44.9 s | about +3 s (+7%) |
| `cargo build -p horizon-client-lib --lib` | 112.5 s | 84.2 s | not comparable (cache effects; the on build did not cost more) |
| `libhorizon_client_lib.rlib` (debug) | 119.0 MB | 120.4 MB | +1.39 MB (+1.2%) |
| one-time: wasmtime + yoake-slice dev build for the feature | | 207 s | not caused by the glue |

The glue is about 6.4k lines for the two required modules (3.1k + 3.3k) and ~2.8k more for the Game modules. Release/final-binary size was not rebuilt (the full game binary was not built here, 30 GB disk): with the feature off nothing is compiled; with it on the glue is unreferenced, so the linker drops it until a runtime registers a dispatcher. Whole-crate extrapolation from 4 modules: about 25 lines of glue per bound function, 7000 bound functions means 175k lines, which is roughly +7% check time per 6.4k lines linearly scaled to a very large number; measure again once more modules are enabled.

## Open

1. **No runtime import yet.** Nothing registers `call` as the guest's `engine_call` import. `engine_import!` has no pre-step to read guest memory before entering `native`, so adding it changes the approved core; needs Fable. Engine panics inside a glued call need the trap rule (`catch_unwind`).
2. **Handles beyond the hook receiver:** non-Data arguments (`&World`, `&BlockRegistry`, `serde_json::Value`: about 30 percent of the crate), `Root::Game` with accessor-path resolution (paths are reported, not resolved), child handles for returned references, borrow table for two-handle calls.
3. **Private-field types** (`Aabb`, `JoinSeq`, `Wire`): a shim table (wire shape plus to/from expressions) would bind about 15 entities functions; not built.
4. **Hard exclusions to attack later:** generics (instantiations list), closures (guest callback handles), lifetime-carrying types (1651 occurrences in client-lib: `Game<'_>` arguments, `Formatter`), `dyn`, `impl Trait`.
5. **Policy:** setters can break engine invariants (a visible field is writable). An exclusion/frozen-field list is needed before this ships to mods.
6. More crate JSON (`horizon_client_render`, `serde_json`, `yoake_*`) to turn "crate not loaded" into data or documented handles.
7. Whole crate: one `include!` line per module file (about 680 modules), id lock partitions, regeneration time (survey of both crates takes seconds), glue compile time to be measured per module added.
8. Miri still unavailable (the new `with_arg` unsafe is covered by unit tests only). Final binary size and MSVC timings not measured. The idle `spikes/c1-bindgen` can go.
