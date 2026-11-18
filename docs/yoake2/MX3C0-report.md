# MX3C0: re-entrancy spike (around hooks, guest -> host -> guest on one Store)

Code: `horizon-integ/spikes/c0-reentry/` (standalone crate with its own `[workspace]`; `wasmtime =48.0.7` with the yoake-wasm features plus `wat`; built offline, nothing missing from the cache). Run: `target/release/c0-reentry.exe t` (tests 1, 2, 3, 5) and `... b` (cost). No git writes, no game launch. The guests are hand-written WAT (`src/core.wat`, `src/comp.wat`) because only `wasm32-wasip2` is installed (no `wasm32-unknown-unknown`); the WAT mimics LLVM's shadow stack (a mutable `sp` global, frames in linear memory).

## Summary

| Question | Answer |
|---|---|
| Core module re-entry guest -> host -> guest on one Store | Works. Correct result, frames intact, `sp` consistent. One small `unsafe` (the ACTIVE pointer). |
| Component re-entry | **Also works in wasmtime 48.0.7** when the re-entry comes from a host import into an export of the same instance (no "cannot enter"). The trap only appears after a trap: the instance is poisoned ("cannot enter component instance"). So Fable's "components can't be re-entered" did not reproduce for host-mediated re-entry. Components are ~3-4x slower than core modules. |
| Fuel | Works with a per-activation budget: save the caller's fuel on hook entry, `set_fuel(budget)`, restore on exit. Fuel does not tick in native code, so original bodies are free automatically; nesting is the only thing to handle. |
| Epoch (wall clock) | Needs an explicit pause: `original` must re-set the deadline when it returns. Shown to trap without it and pass with it. |
| Cost (release) | about 110-120 ns per round trip, about 320-380 ns with a 64 B payload (core). |
| Nested trap | Core instance is not poisoned. Rule proposed below (disable mod, never re-run an original that already ran). |

## 1. Core module re-entry

Setup: engine `F(x) = 10x + G(x+1)`, `G(x) = 2x`; both have hook slots. `hook_f` and `hook_g` are guest exports. Each hook pushes a 16-byte frame on the guest shadow stack (stores `x`), calls `original_*`, then verifies the frame is intact. Trace for `F(3)` (expected 1000 + 30 + (8+1) = 1039):

```
host -> guest hook (x=3)
  guest: hook_f enter v=3
  guest -> host original_f(3)
  host -> guest hook (x=4)            <- re-entry on the same Store, same thread, hook_f still on the wasm stack
    guest: hook_g enter v=4
    guest -> host original_g(4)
    guest: hook_g after original (frame intact) v=8
  guest: hook_f after original (frame intact) v=39
result = 1039, guest sp after = 65536, depth = 0
```

Memory and stack are consistent: hook_f's frame survived the nested call; wasmtime allows `Func::call` on the same instance while it is already executing a host import, as long as the call goes through the live `Caller`/`StoreContextMut`.

Unsafe: exactly one pair of operations, the `ACTIVE` thread-local holding `*mut StoreContextMut<'static, Host>` (about 12 lines, same idea as yoake-wasm). The `Caller` itself is the only borrow of the store: while native engine code runs under an import, the import re-publishes its own `caller.as_context_mut()`, so the pointer is always the innermost live context and the outer `caller` is not touched until the native code returns.

```rust
thread_local! { static ACTIVE: Cell<*mut StoreContextMut<'static, Host>> = const { Cell::new(null_mut()) }; }

fn with_active<T>(ctx: &mut StoreContextMut<'_, Host>, f: impl FnOnce() -> T) -> T {
    let p = ctx as *mut _ as *mut StoreContextMut<'static, Host>;
    let prev = ACTIVE.with(|a| a.replace(p));
    let out = f();
    ACTIVE.with(|a| a.set(prev));          // (use a drop guard in the real runtime so a panic restores it)
    out
}
fn active<T>(f: impl FnOnce(&mut StoreContextMut<'static, Host>) -> T) -> T {
    let p = ACTIVE.with(|a| a.get());
    f(unsafe { &mut *p })                  // only unsafe dereference
}

// host import: guest -> original_f
l.func_wrap("env", "original_f", |mut c: Caller<'_, Host>, x: i32| -> R<i32> {
    let r = { let mut ctx = c.as_context_mut(); with_active(&mut ctx, || real_f(x)) };
    ...
})
// engine slot (native, no store argument): reads ACTIVE, calls the guest hook
fn engine_g(x: i32) -> i32 {
    match active(|c| c.data().hook_g.clone()) { Some(h) => dispatch(|c| h.call(c, x), x, real_g), None => real_g(x) }
}
```

Caveat: `with_active` should use a drop guard (panic safety) and the `ACTIVE` pointer must be per thread (it is; hooks run only on main/server threads per 4b).

## 2. Component re-entry

`src/comp.wat`: a component exporting `hook-f` / `hook-g` from one core instance; `hook-f` calls the lowered import `original-f`; the host implementation of `original-f` calls `instance.get_typed_func("hook-g")` and calls it on the same Store while `hook-f` is still on the stack.

Result: **no trap.** `hook-f(3)` returned 1035 (= 1000 + 30 + 4+1), and afterwards `hook-g(1)` from the host worked (2). So with wasmtime 48.0.7 and a synchronous host import, a component instance can be re-entered from the host. (The component-model "may enter" check applies to guest-to-guest calls and to instances that already trapped.)

What does trap: after a normal trap in the instance (`hook-trap`, `unreachable`), the instance is permanently poisoned:

```
hook-trap -> wasm trap: wasm `unreachable` instruction executed
after a trap the component instance is poisoned: wasm trap: cannot enter component instance
```

Cost of components: trivial round trip about 360-430 ns, `list<u8>` of 64 bytes about 660-715 ns (host import receives a `Vec<u8>`, canonical ABI copies + realloc). Roughly 3-4x the core-module numbers, and it needs WIT/canonical ABI for every hooked signature, which is the point of the `engine_call(id, ptr, len)` design in 4b. So the decision for core modules still stands, but for cost and simplicity, not because re-entry is impossible. (Caveat: not tested with async or with component-to-component calls; the poisoning on trap is the actual hard limitation: a trap anywhere makes the whole mod unusable, which matches the "disable the mod" rule anyway.)

## 3. Fuel and epoch

Fuel (core module, `consume_fuel`): per-activation budget, saved/restored on each hook entry:

```rust
// entry (dispatch)
let saved = c.get_fuel().ok(); c.set_fuel(budget).ok();
let res = h.call(c, x);
c.set_fuel(saved)             // exit: the caller's remainder is back; nested use is not charged to it
```

Output with budget 10 000 per activation (hook_f burns about 6.3k, hook_g about 7.0k, together 13.4k):

```
hook_g activation used 7040 fuel (budget 10000)
hook_f activation used 6338 fuel (budget 10000)
result=1039, disabled=false
```

Without the swap (one pool for the outermost hook, nested hook draws from it): `TRAP in hook (x=4): all fuel consumed by WebAssembly`, mod disabled. Fuel is only consumed by wasm instructions, so time inside `original` is never charged; no need to set fuel "high" during original unless the nested hook doesn't swap budgets. Simplest rule: swap in the hook entry, nothing in `original`.

Epoch (wall clock; a ticker thread bumps `engine.increment_epoch()` every 1 ms and a shared counter): hook budget 20 ticks, engine body (`original_h`) sleeps 60 ms, guest then runs a 1000-iteration loop (epoch checks happen at loop headers and function entries, so a hook that does nothing after `original` is never interrupted):

```
pause/restore = true : Ok(6)                    in 60.5 ms
pause/restore = false: Err("wasm trap: interrupt") in 60.2 ms
```

Pause recipe, all in the `original` import:

```rust
let used_before = ticks.load() - c.data().hook_start_tick;   // wall-clock the guest has already used
/* run the engine body */
c.as_context_mut().set_epoch_deadline(budget.saturating_sub(used_before).max(1));   // relative to "now"
```

`hook_start_tick` must be saved/restored per activation (stack) in the real runtime, like the fuel. Since the doc says full-access mods have no hard per-call limit and only a watchdog logs, the simplest option is epoch deadline = very large and a watchdog that logs by comparing `Instant` minus time spent in originals. Epoch is not needed for correctness.

## 4. Cost (release, `opt-level=3`, 2M iterations, x86_64 Windows, two runs)

Round trip = host `TypedFunc::call` -> guest `hook_h` -> import `original_h` -> trivial host body -> return. The call goes straight to the typed func (the `ACTIVE` publish and fuel save/restore are not in the loop; they are a few ns each).

| Config | trivial round trip | 64 B (alloc + memcpy + hook + original + free) | alloc+free alone |
|---|---|---|---|
| no limits | 113 / 113 ns | 324 / 345 ns | 205 / 226 ns |
| fuel on | 139 / 112 ns | 364 / 322 ns | 231 / 210 ns |
| epoch on | 123 / 110 ns | 476 / 381 ns | 246 / 241 ns |
| fuel + epoch | 119 / 132 ns | 342 / 389 ns | 228 / 263 ns |

Noise is about +/- 25 ns. Each host->guest call costs about 50-60 ns by itself, so the 64 B case (alloc, hook, free are three guest calls plus a memory write) is mostly call count, not copy size. Fuel/epoch add about 0-20 ns. Consequence for the design: pass arguments by value (scalars) where possible and keep a pre-allocated scratch buffer in guest memory instead of alloc/free per call: that would put the 64 B case near 130-150 ns. This is consistent with the 0.3-2 us per hooked call estimate in 4b (this is the lower end because the body is trivial).

## 5. Trap inside a nested hook

`hook_g` traps (`unreachable`) when `x == 100`; reached by `hook_f(99)` (so hook_f is on the stack). The call into `hook_g` returns `Err` from `Func::call`; the nested wasm frames are unwound by wasmtime; the outer wasm frames (hook_f, still waiting in its import `original_f`) are untouched and the host import returns normally. The core instance is **not poisoned**: `hook_g(5)` called afterwards works.

What is left inconsistent: the guest's mutable globals (the shadow-stack pointer `sp` is 16 bytes low after the unwound frame: `sp=65504` instead of 65536) and any guest-side state that was mid-update. LLVM-generated code works from a local copy of `sp` and restores the global in its epilogue, so the outer function heals it (my default test shows `sp=65536` after the outer returns). A guest that re-checks the global (strict variant) fails by itself.

Three variants tested (all with fallback = original body, mod disabled):

| Variant | Result | F body runs | Notes |
|---|---|---|---|
| A: nested trap, fallback to real G, outer hook continues; normal guest | 2190 (outer hook result, +1000 applied) | 1 | clean, sp healed |
| A with a guest that asserts global `sp` (strict) | 1190 | 1 (with the rule below) | the outer hook trapped too, `sp` stays 65504 |
| B: abort the outer hook from `original_f` (returns Err) | 1190 | 1 (with the rule below) | |

Without the "recorded result" rule, the first version of my dispatch re-ran `real_f` after the outer hook trapped: **F body executions = 2** (double side effects). Fix, now in the code: every activation records `(times original ran, last original result)`; on a trap, if the original never ran, run it; if it already ran, return the recorded result and do not run it again.

Proposed rule for the runtime:
1. On any `Err` from a hook call: log the error (with the wasm backtrace), set the mod's `disabled` flag with Release ordering; the dispatcher checks it with the slot load, so every later call goes straight to the original body.
2. For the activation that trapped: if `original` has not been called, call it now; if it has, return the last recorded result (never run twice).
3. Outer activations of the same mod still on the stack keep running (policy A; they may trap themselves, which goes through the same rule); no need to abort them from `original`.
4. Never call into the guest again after disabling, except to drop it (state is unknown). Component-based sandboxed mods: poisoned anyway.
5. Fuel/epoch traps are handled by the same path ("all fuel consumed", "interrupt").

## Recommended minimal runtime design

- Core module per mod, one Store per side, host state in `Store<Host>`; typed funcs (`TypedFunc`, cloneable not Copy) cached in the hook slot.
- Store access: an `ACTIVE` thread-local pointer to the innermost `StoreContextMut` (published at the top-level entry from the main/server loop and re-published by every import before calling native engine code). One `unsafe` deref, drop-guard restore. Hooks fire on the main/server thread only.
- Hook dispatch (entry): `depth += 1`, push an activation `(orig_calls, last_result)`, save and replace fuel, record epoch start; call; pop; restore fuel. On `Err`, rule above.
- `original` import (`engine_call`-style): look up id, run the native body under `with_active`, record the result in the activation, restore the epoch deadline (if used).
- Time limits: fuel per activation if wanted, no epoch (watchdog only logs). If epoch is used, keep an activation stack of `hook_start_tick`.
- Cost budget: about 110-120 ns per hook round trip with scalar args; avoid alloc/free per call (use a scratch buffer exported by the guest).

## Open points not covered

- Component re-entry was only tested with sync host imports on one instance; async host functions and component-to-component calls were not tried.
- Real Rust-compiled guests were not tested (no `wasm32-unknown-unknown` target installed); the hand-written WAT follows LLVM's shadow-stack conventions but a real guest could keep other global state.
- The bench loop does not include the `ACTIVE` publish, activation push/pop or fuel save/restore (a few ns each).
