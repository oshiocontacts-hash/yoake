# S2 typed hook-slot experiment

This is a throwaway, process-local Rust experiment, not a supported mod API or a foreign ABI.
It does not load mods, grant permissions, or replace the existing inject runtime, signature,
fingerprint, identity, or ABI checks. Do not connect arbitrary DLLs to these safe Rust callbacks.

## Mechanics

`#[yoake_hook(id = N)]` instruments a function against a single global slot array
`yoake_hook_rt::SLOTS: [Slot; MAX_SLOTS]` (`MAX_SLOTS = 4096`) and emits a compile-time
assertion that `id < MAX_SLOTS` plus `pub const __YOAKE_HOOK_ID_<function>: usize = N;`
(inside the `impl` for `#[yoake_hook(method, id = N)]`, including associated functions).
No per-function static or registration getter is emitted.

Callbacks are **around** hooks. On entry, the wrapper performs one `Acquire` load of
`SLOTS[N]`. If null, it runs the original body via an `#[inline(always)]` helper with no
exit check. If non-null, it dispatches through the cold out-of-line chain: the callback
receives `original: &mut dyn FnMut(...) -> Result` followed by the arguments, and returns
the result. It may call `original` zero times (early return), once (with original or
replaced arguments), or multiple times; calling `original` from entry `i` invokes entry
`i + 1`, and the last entry runs the original body.

```rust
use yoake_hook_macro::yoake_hook;
use yoake_hook_rt::{Order, register};

#[yoake_hook(id = 1)]
fn score(value: u32) -> u32 {
    value + 1
}

fn double_after_replace(original: &mut dyn FnMut(u32) -> u32, _value: u32) -> u32 {
    original(40) * 2
}

fn main() {
    register::<fn(&mut dyn FnMut(u32) -> u32, u32) -> u32>(
        __YOAKE_HOOK_ID_score,
        double_after_replace,
        Order::default(),
    )
    .unwrap();
    assert_eq!(score(1), 82);
}
```

Callbacks are `fn(...)` pointers, not capturing closures, registered via
`yoake_hook_rt::register::<Callback>(id, callback, order)`. Borrowed inputs use
higher-ranked function pointers, never a `'static` data requirement. For example, a
method `fn f(&mut self, value: &str) -> usize` uses
`fn(&mut dyn FnMut(&mut Self, &str) -> usize, &mut Self, &str) -> usize`
(desugaring to `for<'o, 's, 'b>`). Shared named lifetimes between arguments remain
shared in both the `dyn for<...> FnMut(...)` trait object and the outer `fn(...)`
binder. Capturing closures can still be *arguments* through supported borrowed
`dyn FnMut(usize)` types.

One slot is shared across monomorphizations of `'static`-bounded generic functions and
const generics. Each entry stores its exact callback pointer using `Any`; `Snapshot::next`
skips entries whose `TypeId` does not match the active instantiation. Generic type
parameters must already explicitly have `'static` bounds. The macro never adds bounds to
restrict existing vanilla callers.

Registered calls visit priority-descending, mod-load-order-ascending, then insertion-order
callbacks. Explicit `return` and `?` stay inside the `#[inline(always)]` original helper.
Panics unwind normally without post-hooks. Hooks may re-enter the hooked function from
inside `original` or the callback, running the chain independently per activation.

Registration is serialized by a global mutex using copy-on-write and `Release`
publication. Old immutable `Chain` snapshots are intentionally leaked until process exit
so concurrent lock-free readers never observe freed memory. Each slot allows at most 64
registrations. There is no unregister, DLL hot-unload, dynamic reclamation, WASM bridge,
or stable cross-build callback identity. Dispatch is out of line and `#[cold]`; an empty
slot does not allocate, lock, or perform a type lookup.

## Explicit exclusions

Use `#[yoake_hook(skip = "specific reason")]`; this emits the original function unchanged.
The experiment rejects instead of guessing for:

- async, const, unsafe, extern-ABI, and variadic functions;
- trait impl methods (Rust forbids adding `__YOAKE_HOOK_ID_*` or a sibling
  `#[inline(always)]` original-body helper method inside `impl Trait for Type`, a local
  `fn` inside the method body cannot name `Self` or take `self`, and a closure capturing
  `self` ignores a replacement receiver passed to `original`; `StandClimbing<'_>` also
  has a non-`'static` `Self` lifetime), plus by-value/typed receivers;
- `impl Trait` inputs/outputs, never results, non-static borrowed results;
- unconstrained generic types, bounded lifetime parameters, lifetime-dependent where clauses;
- destructuring/ref/subpattern arguments;
- higher-order callable inputs with their own borrowed parameters or lifetime binders;
- `#[track_caller]`, whose call location a helper function would change.

Closure expressions are not attribute targets; capturing callback closures are not supported.
Aliases hiding unsupported lifetimes, complex generic impl bounds, macro-generated declarations,
and attribute-order/cfg interactions are not a general-purpose normalization facility here;
the Rust compiler remains the final check. Supporting borrowed returns or suspended async
futures needs a different lifetime and phase contract, not casts or relaxed checks.

## Instrumentation and reproduction

Every production function in `horizon-client/src/entities.rs` has a numbered hook
(`id = 1..=71`; slot `0` stays unused) matching `crates/yoake-hook-rt/hook-ids.lock.json`,
or an explicit exclusion. Test-only helper functions are left alone. `yoake-hook-baseline`
disables those attributes; `yoake-hook-bench` exposes the small `StandStep` fixture
constructor and enables the example. Neither feature is a player-facing option. Normal
builds are instrumented for this spike.

```text
cargo test --offline -j 4 -p yoake-hook-macro -p yoake-hook-rt
cargo build --offline -j 4 -p yoake-hook-rt --release --example inline_probe
cargo build --offline -j 4 -p horizon-client-lib --release --example yoake_hook_bench --features yoake-hook-bench
cargo build --offline -j 4 -p horizon-client-lib --release --example yoake_hook_bench --features yoake-hook-bench,yoake-hook-baseline
cargo test --offline -j 4 -p horizon-client --lib
```

Run separate saved copies of the two release example executables: baseline `null`, instrumented
`null`, and instrumented `noop`. The latter registers exactly one around hook on
`ArmorStand::tick_settle` (id `71`) that calls `original` once.
Each run discards one warmup, reports eleven sample medians and raw samples, and black-boxes
inputs/results. The slice updates 1024 armor stands using settling, interpolation, and position
rendering; it is **not a complete game tick** or a mob/collision/streaming benchmark. Binary size
means the example PE's linked size, not the game distribution. Warm-dependency forced crate
rebuild timings are not clean dependency-build timings.
