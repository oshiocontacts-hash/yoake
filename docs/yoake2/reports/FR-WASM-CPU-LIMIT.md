# FR-WASM-CPU-LIMIT

## Result

The per-call 20 ms client, 25 ms server, and 250 ms initialization limits now measure CPU time on the thread executing the guest. Scheduler time while that thread is not running no longer consumes the limit. This changes only WebAssembly mod accounting; vanilla and no-mod execution do not enter this path.

The aggregate 50 ms window uses the same guest CPU measurement. Its limits and strike policy are unchanged.

## Clock and safety policy

- Unix reads `clock_gettime(CLOCK_THREAD_CPUTIME_ID)` through `libc`.
- Windows reads `GetThreadTimes(GetCurrentThread(), ...)` through the already-locked `windows-sys` 0.61 crate.
- The epoch callback compares guest CPU since call entry with `TimeLimits::call`.
- Every call also has a wall-clock safety cap of `max(20 * call_limit, 1 s)`. The cap still interrupts a guest whose CPU clock does not advance.
- If either CPU-clock read is unavailable, the per-call check uses the previous wall-clock limit. The wall cap remains active in all modes.

No allocation was added to successful calls. Limit strings are formatted only while handling a trap.

## Host services and aggregate accounting

Host services now record both wall time and calling-thread CPU time. Subtracting their old wall duration from a CPU total would over-subtract sleeping services and under-subtract CPU-bound services, so separate CPU totals are necessary.

Ordinary host-service CPU extends the per-call CPU deadline and is excluded from the aggregate, matching the old wall-time policy. Storage, configuration, package-service, and counted GPU host CPU also extend the per-call deadline, but remain charged to the aggregate. Wall host time continues to extend the wall fallback and safety cap. The shared extension remains capped at `MAX_HOST_TIME` (400 ms).

## Diagnostics

A deadline interrupt retains the existing prefix, `WebAssembly mod X disabled in HOOK:`, but identifies the limit and supplies measurements:

```text
WebAssembly mod X disabled in HOOK: over its per-call CPU limit (cpu 31.2 ms, wall 212.0 ms, limit 25 ms)
WebAssembly mod X disabled in HOOK: over its wall-clock safety cap (cpu 2.0 ms, wall 1100.0 ms, cap 1000 ms)
```

CPU-clock fallback is identified as a per-call wall-clock limit. Traps not caused by the deadline callback retain their original error text.

## Tests added

Tests in `crates/yoake-wasm/src/lib.rs` require no game resources:

- a real spinning Wasm guest is interrupted, disabled, and records the CPU limit;
- an injected CPU clock with no progress allows a call across 30 ms of wall time without disabling it;
- an injected wall clock reaches the safety cap even with no CPU progress;
- exact CPU-limit and wall-cap diagnostic wording;
- existing wall-accounting tests explicitly select CPU-clock fallback, preserving their original coverage.

Run by Claude on LENOVO: yoake-wasm --lib 72 passed; horizon-client --lib 1252; horizon-client-lib --lib 1921; horizon-client --lib --features yoake-hook-bench,yoake-wasm/test-guests `wasm_mods` 84 (the spinning and trapping fox_ai builds are still disabled). Claude's review fixes: the spinning test component's code body length (6 -> 7 bytes), `mut context` in the deadline callback, and on Windows the CPU limit allows one scheduler tick more (`CPU_CLOCK_GRANULARITY` 15.625 ms: `GetThreadTimes` advances in ticks, so a healthy 10 ms call could read as two). Not run in the real game.

## Wasmtime trap/re-entry finding

No retry strike system was added. Wasmtime 48.0.7 cannot re-enter the same component instance after a trap:

- `src/runtime/func.rs:1477-1481` calls `set_trapped()` whenever a Wasm entry returns an error;
- `src/runtime/component/store.rs:324-329` stores that trapped bit;
- `src/runtime/component/concurrent_disabled.rs:177-179` permits entry only when the store is not trapped.

A future retry policy therefore needs re-instantiation, including explicit state-reset and aggregate-accounting semantics, rather than another call into the trapped instance.

## Documentation

Updated `docs/yoake-wasm-dev.md`, `docs/yoake-mixin.md`, the English and Japanese mixin guides, and the English and Japanese dimension guides. Historical reports were left unchanged.
