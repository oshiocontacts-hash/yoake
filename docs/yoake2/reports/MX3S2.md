# MX3S2: Around-Hook Slots Spike Report

Date: 2026-11-18  
Branch: `yoake2/s2`  
Crates: `yoake-hook-rt`, `yoake-hook-macro`, `horizon-client` (`src/entities.rs`, `examples/yoake_hook_bench.rs`)

## Summary

We replaced the draft per-function static `HEAD`/`RETURN` hook slots with a single global slot array (`SLOTS: [Slot; 4096]`) and **around** hooks. Every production function in `crates/horizon-client/src/entities.rs` was instrumented (`71` hooked with explicit ids `1..=71`, `17` explicitly skipped including `13` trait impl methods).

All unit tests, the inlined-leaf release probe (`100` observed hook fires), and `horizon-client --lib` (`1193 passed, 0 failed, 40 ignored`) pass with unchanged behavior. In release microbenchmarks on Windows MSVC (`11` sample medians), an empty slot adds `+0.094 ns` (`+1.8%`) on a single `ArmorStand::tick_settle` call, `+1.20 ns` per entity (`+10.7%`) on a tight 1024-entity `tick_settle + interpolate + render_position` loop, `0` measurable change on `move_applies`, `+15,872 B` (`+6.3%`) on the benchmark executable size, and no measurable change in warm-dependency rebuild time (`152.5 s` vs `152.1 s`). One Rust `fn`-pointer no-op around hook adds `+9.25 ns` on `tick_settle` (before WASM C0 dispatch/marshalling cost).

## Design as Built

- **Global `SLOTS` array (`crates/yoake-hook-rt/src/lib.rs`):**  
  `pub const MAX_SLOTS: usize = 4096;` and `pub static SLOTS: [Slot; MAX_SLOTS] = [EMPTY; MAX_SLOTS];`. Each `Slot` is `#[repr(transparent)] pub struct Slot(AtomicPtr<Chain>)`. There are no per-function `static` items and no registration getter functions.
- **Explicit IDs and lock file:**  
  Functions carry `#[yoake_hook(id = N)]` (or `#[yoake_hook(method, id = N)]` inside inherent `impl` blocks). `yoake-hook-macro` rejects a missing `id` (it cannot see other attributes, so duplicates are caught by the lock-file test below), emits a compile-time `assert!(N < ::yoake_hook_rt::MAX_SLOTS)`, and emits `pub const __YOAKE_HOOK_ID_<fn>: usize = N;`. In `entities.rs`, ids `1..=71` are assigned in source order (`0` stays unused) and recorded in `crates/yoake-hook-rt/hook-ids.lock.json` (`{ id, path, fn }`). A test (`entities_hook_ids_match_lock_file`) parses both via `include_str!` and fails on duplicates or mismatches.
- **Around callback type and dispatch (`crates/yoake-hook-macro/src/lib.rs`):**  
  Entry performs one `Acquire` load via `SLOTS[id].load()`. If null, it calls an `#[inline(always)]` sibling helper `__yoake_orig_<fn>` with no exit check. If non-null, it calls a `#[cold] #[inline(never)]` out-of-line `__yoake_call_<fn>` chain walker and an `#[inline(always)]` generic funnel `__yoake_dispatch_<fn>`. Callbacks receive `original: &mut dyn FnMut(...) -> R` plus the arguments and return `R`, calling `original` zero, one, or multiple times; calling `original` in entry `i` runs entry `i + 1`, and the last entry calls `__yoake_orig_<fn>`. Borrowed arguments use higher-ranked `fn(...)` pointers (for example `for<'o, 's, 'b> fn(&'o mut dyn FnMut(&mut Self, &'b str) -> R, &'s mut Self, &'b str) -> R`), eliding unshared lifetimes and preserving shared named lifetimes without requiring `'static` data.
- **Registration and leaking:**  
  `yoake_hook_rt::register::<Callback>(id, callback, order)` validates `id < MAX_SLOTS` and rejects zero-sized `Callback` types (catching forgotten `fn(...)` pointer coercions). Under a global `REGISTER_LOCK: Mutex<()>`, it copies the current snapshot, enforces a `64`-entry cap per slot, sorts by `(Reverse(priority), mod_load_order, sequence)`, boxes a new `Chain`, and publishes it with `Ordering::Release`. Old `Chain` allocations are intentionally leaked until process exit so lock-free readers holding `&'static [Entry]` never race with a free.
- **Monomorphization choice:**  
  Generic functions share **one slot** across all instantiations, provided every generic type parameter already has an explicit `'static` bound (const generics are also supported). Each `Entry` erases its callback pointer in `Arc<dyn Any + Send + Sync>`, and `Snapshot::next::<Callback>` skips entries whose `TypeId` does not match the active instantiation. Unconstrained generics are rejected rather than silently adding `'static` bounds that would break vanilla callers; this matches `YOAKE_MIXIN2` §2.2, which excludes generics from full-access hooking anyway.

## Benchmark and Test Numbers

Measured on LENOVO, Windows MSVC stable `1.100.0`, `-j 4`, `CARGO_INCREMENTAL=0`, release profile (`yoake_hook_bench` over `entities.rs`: `71` hooked functions, `13` trait methods skipped, `4` other functions skipped; `11` sample medians after `1` warmup):

| metric | baseline (no attrs) | hooked, empty slots | hooked, one no-op around on tick_settle |
| --- | --- | --- | --- |
| tick_settle per call | 5.186 ns | 5.280 ns (+0.094, +1.8%) | 14.434 ns (+9.25 ns) |
| settle+interpolate+render_position, per entity of 1024 | 11.279 ns | 12.481 ns (+1.20 ns, +10.7%) | 22.242 ns |
| move_applies leaf | 6.193 ns | 6.187 ns (no change) | 6.182 ns |
| example exe size | 252,416 B | 268,288 B (+15,872 B, +6.3%) | same exe |
| forced horizon-client-lib release rebuild (warm deps, entities.rs touched) | 152.5 s | 152.1 s (no measurable change) | |

Notes on scope:
- The no-op hook above is a Rust `fn` pointer through the `Any` chain, **not** a WASM call. A WASM around hook adds the C0 round-trip cost on top (~`110–150 ns`, or `~320–390 ns` with `64 B` args and per-call alloc; `~130–150 ns` expected with a scratch buffer).
- This slice benchmarks tiny per-entity methods in a tight loop, not a full game tick; it is a worst case for empty-slot overhead.
- Verification runs all passed: `yoake-hook-macro` + `yoake-hook-rt` tests (`9 + 12` passed, `0` failed, including the lock-file check), `inline_probe` release (`100 inlined calls observed; checksum=6050`), and `cargo test -p horizon-client --lib` (`1193` passed, `0` failed, `40` ignored requiring prepared resources).

## Why the Slice Shows +10.7% While a Single Call Shows +1.8%

**Hypothesis:** In `yoake_hook_bench`, the slice loop runs three hooked methods per entity: `ArmorStand::tick_settle` (`id = 71`), `ArmorStand::interpolate` (`id = 61`, 7 lines), and `ArmorStand::render_position` (`id = 59`, 1 line: `self.previous.lerp(self.position, alpha)`). Each hooked outer wrapper is not marked `#[inline]` and contains a global `SLOTS[id]` `Acquire` load plus a branch to a `#[cold] #[inline(never)]` chain dispatcher. On a tight 1024-entity loop, wrapping trivial 1–7 line functions likely prevents LLVM from inlining `render_position` and `interpolate` directly into the loop body, blocking cross-call register reuse, common subexpression elimination, and vectorization—turning `3` tiny wrappers into `+1.20 ns` (`+10.7%`) per entity. By contrast, the standalone `tick_settle` microbenchmark already places `black_box` barriers around `stand`, `medium`, and `step` every iteration, preventing cross-call optimization in the baseline too, so instrumentation only adds the single slot load and predicted branch (`+0.094 ns`, `+1.8%`). On `move_applies`, LLVM likely inlined the wrapper itself into the microbenchmark loop and folded/hid the branch inside `black_box` noise.

## Macro Limits (Unhookable Function Shapes)

The proc macro rejects (requiring `#[yoake_hook(skip = "reason")]`) or does not support:

1. **Non-standard calling conventions/modifiers:** `async`, `const`, `unsafe`, `extern` ABI, and variadic (`...`) functions.
2. **Trait impl methods and non-borrowed receivers:** By-value `self` / typed `self: Box<Self>` receivers are rejected. Trait impl methods (`13` in `entities.rs`: `EntityRules` on `ItemEntity`/`ExperienceOrb`/`ArmorStand`, `ClimbingWorld` on `StandClimbing<'_>`) are skipped because Rust forbids adding `pub const __YOAKE_HOOK_ID_*` or sibling helper `fn`s (`__yoake_orig_*`, `__yoake_call_*`, `__yoake_dispatch_*`) inside `impl Trait for Type`; meanwhile a local `fn` inside the method body cannot name `Self` (`E0411`) or take `self`, a capturing closure cannot accept a replacement receiver passed to `original` (and double-borrows `&mut self`), and `StandClimbing<'_>` also has a non-`'static` `impl` lifetime on `Self`.
3. **Unsupported return types:** `impl Trait` opaque returns, `!` (never), and non-`'static` borrowed return types (references or types carrying non-`'static` lifetimes).
4. **Unsupported parameter types and patterns:** `impl Trait` inputs (`ExperienceOrb::unstuck`, `ArmorStand::on_pos_legacy`), destructuring patterns, `ref` bindings, and `@` subpatterns (`ItemEntity::tick_move`), and higher-order callable inputs (`&dyn Fn(...)`, `fn(...)`) that carry borrowed parameters or explicit `for<...>` lifetime binders (`merge_with_neighbours`).
5. **Generics and lifetime constraints:** Type parameters without an explicit `: 'static` bound, bounded lifetime parameters (`'a: 'b`), and lifetime-dependent `where` clauses (`where T: 'a`).
6. **Attributes and call-site semantics:** `#[track_caller]` (a helper `fn` changes the reported caller location), closure expressions, capturing callback closures (`register` requires `fn(...)` pointers), type aliases hiding unsupported lifetimes, and non-trivial `cfg`/attribute ordering.

## Recommended Exclusion Policy

1. **Syntactic/type exclusions:** Keep all macro exclusions above enforced via `#[yoake_hook(skip = "reason")]`.
2. **Hot-leaf crates (`YOAKE_MIXIN2` §2.2):** Exclude whole hot-leaf crates and inner loops by default: `mesher`, `light`, `worldgen` inner loops, `fast_hash`, plus allocator and per-vertex rendering loops.
3. **Concrete codemod thresholds:**
   - **Body-size rule:** Skip pure/leaf helpers whose body has `<= 3` statements (or a single non-branching expression, such as `bounds`, `render_position`, `step`, `filter_flag`) unless explicitly allow-listed for a known mod need. The `+10.7%` slice jump shows that hooking trivial accessors inside entity loops dominates empty-slot cost.
   - **Call-frequency rule (`YOAKE_MIXIN2` §4b):** Forbid hooking functions called on the order of `>= 100,000` times per tick or frame (even a `150–390 ns` WASM hook at `100k` calls burns `15–39 ms` of a `50 ms` tick).
4. **CI benchmark gate (`#[yoake::no_hook]`):** Run a baseline-vs-instrumented benchmark suite in CI over representative tick and frame slices. Enforce the `< 1%` empty-slot tick/frame budget (`YOAKE_MIXIN2` §2.3); any function or module whose instrumentation regresses a slice past the gate is added to the `#[yoake::no_hook]` exclusion list shipped with the SDK.

## Open Problems for C2

1. **WASM dispatcher in the chain:** `SLOTS` currently stores typed Rust `fn(...)` pointers via `Arc<dyn Any>`. C2 must dispatch entries targeting core WASM modules through the C0 `ACTIVE` thread-local `&mut Store` without generating bloated per-function dispatchers, and must ensure hooks only fire on the main thread and integrated-server thread (never worker threads, per `YOAKE_MIXIN2` §4b) without slowing down empty slots.
2. **Guest argument marshalling and trap handling:** Converting borrowed/owned Rust arguments into path-based engine handles + dynamic borrow-table entries and a reusable scratch buffer (to stay near C0's `~130–150 ns` bound instead of `~320–390 ns` per-call allocation), writing back `&mut` plain data, and recording per-activation `(calls, last_result)` so a trapped mod runs `original` at most once.
3. **Hot-reload generation:** Current `register` leaks old `Chain`s and caps slots at `64` entries with no unregister. Dev-mode hot reload (`YOAKE_MIXIN2` §4b) needs a generation tag (or slot rebuild under `REGISTER_LOCK`) to replace a reloaded mod's hooks without exhausting the 64-entry cap or keeping disabled/stale instances in the chain.
4. **Trait-method hooking:** Because `SLOTS` is global, trait methods no longer need a trait change, yet `13` methods in `entities.rs` remain skipped due to Rust's `impl Trait` item restrictions. C2's codemod/macro should investigate emitting `__YOAKE_HOOK_ID_*` and free-standing `__yoake_orig_*` / `__yoake_call_*` helpers outside the `impl Trait` block (in an inherent `impl` or sibling module) for trait impls whose `Self` is `'static`.
