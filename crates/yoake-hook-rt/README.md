# S2 typed hook-slot experiment

This is a throwaway, process-local Rust experiment, not a supported mod API or a foreign ABI.
It does not load mods, grant permissions, or replace the existing inject runtime, signature,
fingerprint, identity, or ABI checks. Do not connect arbitrary DLLs to these safe Rust callbacks.

## Mechanics

`#[yoake_hook]` generates a `__yoake_hook_<function>()` registration getter and a function-local
static slot. Use `#[yoake_hook(method)]` inside inherent impls, including associated functions.
HEAD takes `&mut` to each ordinary argument, but takes a borrowed receiver directly as `&Self`
or `&mut Self`. It returns `ControlFlow<Result>`: `Continue(())` runs the body with edited
arguments; `Break(result)` bypasses it. RETURN takes only `&mut Result`, including an early
HEAD result. It does not retain arguments or the receiver until RETURN.

```rust
use std::ops::ControlFlow;
use yoake_hook_macro::yoake_hook;
use yoake_hook_rt::Order;

#[yoake_hook]
fn score(value: u32) -> u32 { value + 1 }

fn replace(value: &mut u32) -> ControlFlow<u32> {
    *value = 40;
    ControlFlow::Continue(())
}

fn boost(result: &mut u32) { *result += 1; }

fn main() {
    __yoake_hook_score().register_head(replace, Order::default()).unwrap();
    __yoake_hook_score().register_return(boost, Order::default()).unwrap();
    assert_eq!(score(1), 42);
}
```

Callbacks are function pointers, not capturing closures. A borrowed input uses a higher-ranked
function pointer, not a `'static` data requirement. For example, a method receiving `&'input str`
can use `for<'receiver, 'input> fn(&'receiver mut Game, &mut &'input str) -> ControlFlow<()>`.
Named lifetimes shared between arguments remain shared; elided lifetimes get distinct binders.
Capturing closures can still be *arguments* through supported borrowed `dyn FnMut(usize)` types.

The slot is shared across **all monomorphizations**, even const-generic values and generic impl
types. Each entry stores its exact callback type using `Any`; dispatch ignores other signatures.
Thus a `u32` registration does not affect `String`, but const instantiations with the same callback
signature do share hooks. This avoids a per-type slot registry/lookup on the empty path, at the
cost of type-erased work on the registered path. Generic type parameters must already explicitly
have `'static` bounds. The macro never adds bounds to restrict existing vanilla callers.

Each boundary performs an acquire load/null check. Registered calls visit priority-descending,
mod-load-order-ascending, then insertion-order callbacks. First HEAD `Break` stops later HEADs;
all matching RETURNs run. Explicit `return` and `?` are captured by a body closure. Panics unwind
without RETURN callbacks. Registration during a body can affect its RETURN because the boundaries
load separately. Hooks may reenter and need their own recursion guard.

Registration is mutex-serialized copy-on-write publication. Old immutable snapshots intentionally
live until process exit so concurrent readers cannot use freed callback chains. Each site allows
at most 64 registrations. There is no unregister, DLL hot-unload, dynamic reclamation, WASM bridge,
or stable cross-build callback identity. A production design needs those lifetime/identity decisions
before lifting the registration cap. HEAD/RETURN dispatch is out of line and cold; an empty slot
does not allocate, lock, or do a type lookup.

## Explicit exclusions

Use `#[yoake_hook(skip = "specific reason")]`; this emits the original function unchanged.
The experiment rejects instead of guessing for:

- async, const, unsafe, extern-ABI, and variadic functions;
- trait methods (a getter cannot be added to a trait impl), by-value/typed receivers;
- `impl Trait` inputs/outputs, never results, non-static borrowed results;
- unconstrained generic types, bounded lifetime parameters, lifetime-dependent where clauses;
- destructuring/ref/subpattern arguments;
- higher-order callable inputs with their own borrowed parameters or lifetime binders;
- `#[track_caller]`, whose call location a body closure would change.

Closure expressions are not attribute targets; capturing callback closures are not supported.
Aliases hiding unsupported lifetimes, complex generic impl bounds, macro-generated declarations,
and attribute-order/cfg interactions are not a general-purpose normalization facility here;
the Rust compiler remains the final check. RETURN can only observe/edit the result, not the input
receiver. Supporting borrowed returns or suspended async futures needs a different lifetime and
phase contract, not casts or relaxed checks.

## Instrumentation and reproduction

Every production function in `horizon-client/src/entities.rs` has a hook or an explicit exclusion.
Test-only helper functions are left alone. `yoake-hook-baseline` disables those attributes;
`yoake-hook-bench` exposes the small `StandStep` fixture constructor and enables the example.
Neither feature is a player-facing option. Normal builds are instrumented for this spike.

```text
cargo test --offline -j 4 -p yoake-hook-macro -p yoake-hook-rt
cargo build --offline -j 4 -p yoake-hook-rt --release --example inline_probe
cargo build --offline -j 4 -p horizon-client-lib --release --example yoake_hook_bench --features yoake-hook-bench
cargo build --offline -j 4 -p horizon-client-lib --release --example yoake_hook_bench --features yoake-hook-bench,yoake-hook-baseline
cargo test --offline -j 4 -p horizon-client --lib
```

Run separate saved copies of the two release example executables: baseline `null`, instrumented
`null`, and instrumented `noop`. The latter registers exactly one HEAD on `ArmorStand::tick_settle`.
Each run discards one warmup, reports eleven sample medians and raw samples, and black-boxes
inputs/results. The slice updates 1024 armor stands using settling, interpolation, and position
rendering; it is **not a complete game tick** or a mob/collision/streaming benchmark. Binary size
means the example PE's linked size, not the game distribution. Warm-dependency forced crate
rebuild timings are not clean dependency-build timings.
