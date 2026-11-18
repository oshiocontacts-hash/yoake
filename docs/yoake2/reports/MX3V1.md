# MX3V1: vertical slice, a real engine function hooked by a WASM core module

Branch `yoake2/slice` in `horizon-integ`. No git writes, no game launch, no installs. Default build behaviour is unchanged: everything is behind the cargo feature `yoake2-slice` (horizon-client-lib) or in the new crate `yoake-slice`, which the default build does not compile. No engine source file was edited.

## What was built

One end-to-end path: `ArmorStand::tick_settle(&mut self, medium, step)` (S2 slot id 71, a non-Game receiver) -> S2 around-hook chain -> hand-written shim -> hook activation -> core-module WASM guest (hand-written WAT) -> `original` / get / set through handles -> back.

Files (all new unless noted):

| File | What |
|---|---|
| `crates/yoake-slice/src/activation.rs` | The activation arena: `Root::HookArg`, handles, lending lock, original record. No dependencies. Where the unsafe is. 9 unit tests. |
| `crates/yoake-slice/src/runtime.rs` | wasmtime core-module runtime (feature `wasm`): C0 ACTIVE-store pattern, fuel and epoch per activation, trap rule, per-thread instance. |
| `crates/yoake-slice/guest/slice_guest.wat` | The guest: hooks for each scenario plus benchmark hooks. |
| `crates/yoake-slice/tests/slice.rs` | 10 tests against a small hand-hooked "engine" (fast, no horizon-client). |
| `crates/yoake-slice/examples/slice_bench.rs` | Runtime-only cost, wasmtime floor and thread-local floor. |
| `crates/horizon-client/tests/yoake2_slice.rs` | 7 tests (+1 ignored bench) on the real `ArmorStand::tick_settle` through the real S2 slot. Registered as a `[[test]]` of horizon-client-lib with `required-features = ["yoake2-slice"]`. |
| `Cargo.toml` (+1 line), `crates/horizon-client-lib/Cargo.toml` (optional dep, feature, `[[test]]`), `Cargo.lock` | Wiring. The lock gains `yoake-slice`, and wasmtime's `wat` and wasmparser's `semver` edges (optional-feature edges, only compiled with the feature). |

Run:

```sh
export PATH="$HOME/.cargo/bin:/c/Users/USER/NestgateHorizon/toolchain/mingw64/bin:$PATH" CARGO_INCREMENTAL=0
cargo test --offline -j 4 -p yoake-slice --features wasm
cargo test --offline -j 4 -p horizon-client-lib --features yoake2-slice --test yoake2_slice
cargo test --offline --release -j 4 -p horizon-client-lib --features yoake2-slice --test yoake2_slice -- --ignored --nocapture bench_round_trip
cargo run  --offline --release -j 4 -p yoake-slice --features wasm --example slice_bench
```

## Design

### The activation (Fable's gap: connect C1 and S2)

S2 hooks a function with a typed `fn(original, args...)` callback. The shim (hand-written, `wasm_settle` in the test; a generator would emit it) does:

```rust
fn wasm_settle(original: &mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep),
               stand: &mut ArmorStand, medium: StandMedium, step: StandStep) {
    runtime::dispatch(stand, &ARMOR_STAND /* FieldTable */, &mut |stand| { original(stand, medium, step); 0 });
}
```

`dispatch` is the whole runtime entry. Per hooked call it does, in order:
1. If there is no instance on this thread, or the mod is disabled: run `orig` and return (the S2 chain continues with the next entry or the engine body).
2. Open an **activation**: an entry on a thread-local stack with a never-reused 56-bit serial. Argument 0 (`self`) is registered as `Root::HookArg { activation, arg: 0 }`; the handle given to the guest is `activation << 8 | arg`. A caller-stack `OrigCell` holds the `&mut` to `self` and the `original` closure; the guard borrows the cell, so neither can move or be used while the activation is open.
3. Save the caller's fuel, give this activation its own budget; push an epoch record and set the epoch deadline.
4. Call the guest export `(handle: i64) -> i64` through the typed func (C0 pattern).
5. Restore fuel; on `Ok` close the activation (frees the arena) and return the guest's value; on `Err` apply the trap rule (below).

Guest imports (module `env`, statuses are `i32`): `original(handle, out)`, `get(handle, field, out)`, `set(handle, field, bits)`, `report(slot, value)` (test aid). `out` is an offset into a **scratch buffer in guest memory** (bytes 64..80 / 72), so there is no alloc/free per call. Status codes: 0 Ok, 1 Stale, 2 Locked, 3 BadField, 4 NoSuchArg. Every error is a status, not a trap.

### Why `Root::HookArg` and where the pointer lives

The caller of the hooked function holds an outer `&mut ArmorStand`, so it can never be re-borrowed from the Game root. The activation stores the raw pointer derived from the `&mut` the shim was handed and re-derives a reference on every access, only while the lock allows it. The accessors are a hand-written `FieldTable<ArmorStand>` (field 0 = `fire_ticks`, field 1 = `position.x`); a generator would emit them from the C1 ids.

### Handle table and borrow table, now wired

- **Per-activation arena, freed at the end**: the record is removed when the guard drops (normal end, `finish`, trap, panic). After that every handle of the activation is `Stale`. Serials are never reused (64-bit), so a handle the guest kept cannot alias a later activation. Tested: stale handle, forged argument index, arena empty after normal, trap and panic ends.
- **Lending lock**: `original` sets `lent` on the activation for the whole run of the engine body. While it is set, `get`, `set` and a second `original` through any handle of that activation return `Locked`. There is no second `&mut`: the arena is not even borrowed while the body runs, so nested activations can open and close (re-entrancy).
- **Release on trap**: the trap path closes the activation before running the fallback, so the lock and handles are gone before the original is re-run.

### Trap rule (MX3C0 section 5), as implemented

On any `Err` from the guest call (unreachable, fuel exhausted, epoch interrupt): record the message, set `disabled` on the mod, close the activation, then run the original **only if it ran zero times** in that activation; otherwise return the recorded `last_result`. `dispatch` never enters the guest of a disabled mod (checked at the top-level and in `run_hook`). Outer activations still on the stack keep running (policy A) and go through the same rule if they trap.

### Threads

The instance, the ACTIVE pointer and the activation stack are thread-local. The S2 slot is process-global, so the callback is called on any thread, and on every thread except the one that called `install` it finds no instance and falls through to the original. (The real engine would install on the main thread and, separately, the integrated-server thread.)

### Fuel and epoch

- Fuel: per-activation budget, caller's remainder saved and restored. Test: a hook that uses about N fuel nests inside itself with a budget of 1.25 N and does not trap, which a shared pool would (`fuel_is_per_activation...`). A spinning hook with 50,000 fuel traps, the original runs once.
- Epoch (1 ms ticker thread, budget in ticks): the guest's time is charged, `original`'s is not. The `original` import adds the guest time used so far, runs the body, and resets the deadline to the remainder. Test: budget 20 ticks, the engine body sleeps 100 ms, the hook finishes (paused); with the pause off (negative control) the same hook traps with "interrupt" and the original has still run exactly once.

## Test results

| Suite | Result |
|---|---|
| `yoake-slice` unit tests (activation, no wasmtime) | 9 passed |
| `yoake-slice` `tests/slice.rs` (10, hand-hooked engine) | 10 passed |
| `horizon-client-lib --features yoake2-slice --test yoake2_slice` (real `ArmorStand::tick_settle`) | 7 passed, 1 ignored (the bench) |
| `cargo test --offline -j 4 -p horizon-client --lib`, feature OFF | 1193 passed, 0 failed, 40 ignored (unchanged) |

The 7 engine tests (8 properties below; 2 and 3 share a test), and what they prove (each on the real S2 slot 71, with a plain Rust entry below the WASM one that counts how often the engine body is reached):

1. **Guest changes `self` through the handle and the game sees it**: `fire_ticks` is 15 (5 written before `original`, read back as 5 after it, +10), `position.x` is 12.5, and the stand's `velocity` equals an unhooked control run in the same medium; the body ran once; the arena is empty afterwards.
2. **Re-entrancy**: with the lower entry ticking a second `ArmorStand` before the engine body, the chain is hook(A) -> guest -> `original` -> chain -> hook(B) -> guest again on the same store (depths 1 then 2), both finish, counters 101 and 102.
3. **Lending lock rejects**: the nested activation uses the outer's lent handle: `get` = Locked, `set` = Locked (the write 999 never lands), `original` = Locked; its own handle works; after `original` returns, the outer handle works again.
4. **Trap before `original`**: `unreachable` after one write; mod disabled with the message; the engine body ran exactly once; the game keeps ticking (2 more calls, the guest never entered again).
5. **Trap after `original`**: the body ran once, not twice, velocity changed once.
6. **Trap in a nested activation**: the nested one falls back to the original (once), the outer hook finishes (policy A), mod disabled, the body ran 2 times in total (outer + nested, nothing doubled).
7. **Stale and forged handles**: a kept handle is Stale, a forged arg index is NoSuchArg, an unknown field is BadField.
8. **Other thread**: the same call on a worker thread leaves `fire_ticks` untouched (guest not entered) while the original ran.

Also in `slice.rs`: fuel per activation, spinning hook stopped, epoch pause with negative control, no-mod call, and the same lock/re-entrancy scenarios without the engine.

### Miri

Not available: the toolchain is `1.99.1-x86_64-pc-windows-gnu` with only cargo, clippy, rustfmt, rustc and the wasm32-wasip2 std; `cargo miri` reports the `miri` component is not installed for it, and there is no nightly. Installing is out of scope. The `activation` module has no dependency and is written so `cargo +nightly miri test -p yoake-slice` (default features) should run all 9 of its unit tests, including the panic and nested-lock tests, as soon as a Miri-capable toolchain is installed. This is **not run**. The pointer discipline was written for stacked borrows (every access goes through the pointer registered at open; the caller's `&mut` is not used while the guard lives).

## Numbers (release, x86_64 Windows, `windows-gnu` toolchain on this PC, median of 7 samples)

Real engine, `ArmorStand::tick_settle` through S2 slot 71 (`bench_round_trip`):

| Case | ns per call | over empty slot |
|---|---:|---:|
| slot empty (nothing registered) | 3.7 | - |
| slot set, no mod on this thread (shim + dispatch miss) | 36.1 | +32.4 |
| guest `hook_min`: hook + `original` | 282.0 | +278 |
| guest `hook_rw`: get + set + `original` | 310.0 | +306 |
| same, unmetered (no fuel) | 310.7 | +307 |
| same, fuel + epoch | 319.6 | +316 |

Runtime alone (`slice_bench`, no engine, no S2 chain): raw wasmtime `TypedFunc::call` floor 77 ns (trivial host import), no-mod miss 16 ns, `hook_nop` (activation + fuel + call, no imports) 185-192 ns, `hook_min` 250 ns, `hook_rw` 291-295 ns, with fuel + epoch 298 ns. Fuel and epoch cost 0-10 ns (within noise).

Reading these:
- A round trip with `original` plus a read-modify-write of a field is about 0.3 us on this PC, in line with the 0.3-2 us estimate in section 4b, and 4-5x the C0 floor (113 ns). The scratch buffer works: `get`/`set` add only about 45-60 ns together (no alloc/free per call).
- **A large part is the toolchain, not the design**: on this `windows-gnu` target one thread-local access costs 15.6 ns (measured in `slice_bench`: `const Cell`, `RefCell<Vec>` and lazy all 15.6 ns), against about 1-2 ns on MSVC (the S2 and C0 numbers were measured on LENOVO MSVC). A `hook_min` call makes 6 thread-local accesses (1 in `dispatch`, 2 in activation open/close, 1 for the `original` import, 2 in `run_original`), about 90 ns of the 250. I cut the accesses from about 10 to 6 by putting `ACTIVE` and the instance in one thread-local and passing the context down; the remaining ones are the arena. On MSVC I expect about 160-190 ns for `hook_min`, which is not measured. **Re-measure on the LENOVO MSVC build.**
- Remaining cost per call that is real: activation open/close (record push/remove, about 180 bytes), the epoch/fuel bookkeeping, the guest-call entry (77 ns), the `original` import with `Caller` (about 60 ns).

## Unsafe surface

All in two files; both documented at the top of the file with the invariants.

`activation.rs` (no dependencies; Miri-testable):
1. `get_impl::<T>` / `set_impl::<T>`: deref of the registered `*mut T` and of the `&'static FieldTable<T>` through erased pointers (one `unsafe fn` each, instantiated per argument type at `open`).
2. `run_cell::<T>`: re-derives `&mut T` and runs `original` through the `OrigCell` pointer.
3. The three call sites of those functions through the stored function pointers: `get`, `set`, `run_original`.
There is no lifetime transmute and no heap allocation (an earlier draft boxed the closure and transmuted its lifetime; replaced by the caller-stack `OrigCell`).

`runtime.rs`:
4. `context()`: the one deref of the thread's ACTIVE pointer to the innermost live `StoreContextMut` (C0's pattern), plus its two call sites in `dispatch`.

Invariants the unsafe relies on, and what enforces them:
- The pointers come from `&'a mut` borrows that the `Activation<'a>` guard keeps borrowed (`PhantomData`, and the `OrigCell` is borrowed `&'a mut`); the record is removed from the thread-local stack by `Drop` before that borrow ends, including on panic (tested). A guard that is `mem::forget`-ten would leave a dangling record; a handle could then be dereferenced. The runtime never leaks it; the guard is `!Send` so it cannot be closed from another thread.
- An argument is dereferenced only while the activation is not lent. The lock is a plain flag set before the body runs and cleared after (if the body unwinds it stays set until the guard removes the record).
- Single thread: the arena and the ACTIVE pointer are thread-local; another thread sees `Stale` (tested).
- ACTIVE: native code under an import never uses the `Caller` it came from; every import that runs engine code re-publishes its own context (RAII guard restores it, also on panic).

Not covered by Rust's checks: a hook shim that registers an argument pointer for the wrong type table (the table is passed with the same `&mut T` at one call site, so mismatch is not expressible in `open`, but `add_arg`'s table is the caller's responsibility).

## What is not done / open problems

1. **Miri not run** (not installed). Do it on a machine with nightly.
2. **MSVC numbers not measured**; the figures above carry about 90 ns of emulated-TLS cost. Re-run both benches on the LENOVO.
3. **Hand-written shim and accessor table.** C1 does not yet generate either: category (b) (non-`Game` receivers, `&mut` plain data) has no renderer, and `ids.lock.json` is not used by the shim (the hook id comes from the S2 `__YOAKE_HOOK_ID_*` const). A generator must emit, per hooked fn: the shim, the `FieldTable`, and the per-arg registration for multi-argument functions. `add_arg` exists and is unit-tested but is not wired into a hooked function with two `&mut` args.
4. **Only a unit-returning function.** The result path (`dispatch` returns an `i64`) works in the runtime tests (original result 42 returned on fallback), but there is no marshalling of other return types or of plain-data arguments (`medium`, `step` are not passed to the guest; the guest can only call `original` with the same ones). Plain-data args need the C1 codec and a guest scratch frame.
5. **No `Root::Game` re-borrow inside a hook.** `HookArg` is the only root; getting a `Game` handle inside a hook (the `&mut Game` receiver case) is not built.
6. **Field accessors are scalar `i64` bits only.** Strings, vectors and child handles that lock their parent (C1's "functions that return references") are not implemented.
7. **Trait methods of `entities.rs`** still cannot be hooked (S2 open problem 4).
8. **Chain composition is only lightly tested**: the WASM entry plus one Rust entry. Several WASM mods in one chain, with one trapping, is not tested (the runtime is one mod per thread).
9. **Disabled outer frames**: after a nested trap the outer guest keeps running with the disabled instance; its guest globals (e.g. the `$outer` handle in the WAT) are in whatever state the unwound inner frame left. Policy A from C0, accepted there, not strengthened here.
10. **Epoch ticker** is a thread per instance, detached on drop (joining from a thread-local destructor aborted the process at exit, found by `slice_bench`). A shared ticker per engine would be better; the doc says epoch is only a log-only watchdog for full-access mods, and the pause is implemented only so that a budget can be used at all.
11. **Real Rust guests not tried** (no `wasm32-unknown-unknown`); the WAT guest keeps no shadow-stack state, so the C0 `sp` question is not exercised.
12. **Hot reload**: the install replaces the instance, but S2's chain still leaks and has no unregister; unchanged from S2's open problem 3.


## Round 2 (Fable review of e734913eb)

All changes are in `crates/yoake-slice` (plus the test updates in `crates/horizon-client/tests/yoake2_slice.rs`). Feature-off builds are untouched: no Cargo manifest changed in this round.

### B1: use-after-free from safe code (fixed)

`Activation::open` / `add_arg` / `OrigCell` are gone from the public API. The only entry points are closure-scoped:

```rust
activation::scope (arg, &TABLE, &mut original, |handle| ...) -> (R, Finished)
activation::scope2(a, &TA, b, &TB, &mut original, |handle| ...) -> (R, Finished)
```

The guard is a private struct created inside `scope`, declared after the stack cell that holds the `&mut` pointers and `original`, so its `Drop` removes the record before the cell or the borrows go away (also on unwind). The caller never holds a guard, so there is nothing to `mem::forget`; what the body can forget (the `u64` handle) is just a number, and after `scope` returns it is `Stale`. Evidence: `forgetting_cannot_keep_an_activation_alive` (forgets a boxed handle inside the body; arena is empty, handle Stale), plus a `compile_fail` doctest showing `Activation` no longer exists.

### S1: ACTIVE discipline (fixed)

- `ACTIVE` is set to **null while the guest runs** (an `ActiveGuard` around `hook.call`). Only the `native()` wrapper publishes `caller.as_context_mut()`, and `original`, `get` and `set` all go through it. An import that forgets the wrapper finds no context: a nested `dispatch` then sees the instance borrowed and falls through (the hook just does not fire), so no `&mut StoreContextMut` can overlap a live `Caller`.
- `get`/`set` no longer hold the `ACTS` borrow while the accessor runs. They look the argument up, lock the activation (`lent = true`, the same lock `original` uses), release the arena, run the accessor, and unlock in a `Drop` (`Unlend`, also on unwind). Accessors take `&mut T` now (`FieldTable::get` changed from `&T`), so an accessor can call a hooked engine function on its receiver; that opens a nested activation on the same object, derived from the accessor's own `&mut`.
- Tests: `an_accessor_that_calls_a_hooked_engine_function_does_not_overlap_contexts` (WASM, field 2's accessor ticks the object; no `BorrowMutError`, guest entered twice, depth 0); in `activation`: `an_accessor_may_open_a_nested_activation` and `accessor_locks_the_activation_while_it_runs` (the same handle used from inside its own accessor is `Locked`).

### S2: RAII for the per-activation state (fixed)

`hook.call` runs under `catch_unwind`; `leave` (fuel restore, epoch pop, depth) runs on both paths and the payload is re-thrown with `resume_unwind`, after which the `scope` guard and the `ActiveGuard` drop during the unwind. The mod is **not** disabled by an engine panic (wasmtime unwinds the guest frames cleanly and the instance stays usable). Test `engine_panic_inside_original_restores_fuel_epoch_and_depth` (fuel 200,000 and epoch on, `hook_burn`; the engine body panics inside `original`; `catch_unwind` at the top): afterwards `depth == 0`, `epoch_depth == 0`, `fuel_left == INITIAL_FUEL`, arena empty, mod enabled; a following call completes with `entries == 2`, fuel back at `INITIAL_FUEL`, `max_fuel_used` within budget.

### S3: no panics from runtime calls inside `original` (fixed)

`install`, `uninstall`, `take_reports` use `try_borrow_mut` and return `Err` ("guest is running") instead of panicking; `stats()` returns `Stats { busy: true, .. }`. Signatures changed: `uninstall() -> Result<()>`, `take_reports() -> Result<Vec<..>>`. Test `runtime_calls_from_inside_original_report_busy_instead_of_panicking` (all four called from the engine body inside `original`; the instance is intact afterwards). `Stats` also gained `depth`, `epoch_depth`, `fuel_left`.

### S4: more than one `&mut` argument (decided and built)

Shape: **`original` re-receives all the lent arguments**, `FnMut(&mut A, &mut B) -> i64`, with no raw pointers in shim or generated code (the cell with the raw pointers is private to the activation module). `scope2`/`dispatch2` do two arguments; the handle of argument `i` is the handle of argument 0 plus `i`; both are locked while `original` runs. More arities are the same pattern (a macro or a generated `scope3`); `MAX_ARGS` is 4. Tests (test-only hooked fn `pair_tick(&mut Thing, &mut Other)`, hand-hooked through `dispatch2`): `two_mut_arguments_original_receives_both`, `two_mut_arguments_trap_runs_the_original_once_with_both`; in `activation`: `two_arguments_have_two_handles_and_original_receives_both`, `both_arguments_are_locked_while_original_runs`. It is not hooked through the S2 macro (no real engine function with two `&mut` arguments is used).

### Tests added from the review

| Test | Result |
|---|---|
| `original` called twice from WASM (`original_called_twice_from_wasm_runs_the_body_twice`) | body runs twice, status Ok |
| `out` pointer out of bounds, for `get` and for `original` | trap "out of bounds", mod disabled; `get`: original runs once as the fallback; `original`: body had run, result returned, not run again |
| fuel exhaustion in a nested activation | nested traps ("fuel"), falls back to the original once, outer hook finishes, caller fuel restored |
| nested hook on the same object (`nested_hook_on_the_same_object`) | outer handle Locked while the nested one runs; both bodies ran on the one object |

### Test counts (after Round 2)

| Suite | Round 1 | Round 2 |
|---|---:|---:|
| `yoake-slice` activation unit tests | 9 | 13 |
| `yoake-slice` doctests (compile_fail) | 0 | 1 |
| `yoake-slice` `tests/slice.rs` (runtime) | 10 | 19 |
| `horizon-client-lib --features yoake2-slice --test yoake2_slice` (real ArmorStand) | 7 (+1 ignored bench) | 7 (+1 ignored bench), re-run green on the new API |
| `cargo test -p horizon-client --lib`, feature OFF | 1193 | 1193 passed, 40 ignored (unchanged) |

Total with the feature: 40 passing tests (13 + 1 + 19 + 7).

### Still open (unchanged or new)

- Miri still not available; `activation` stays dependency-free (its 13 unit tests and the doctest need no wasmtime) so `cargo +nightly miri test -p yoake-slice` can run later. Not run.
- The accessors and shims are still hand-written; generating them for category (b) types is not done. Only 1 and 2 `&mut` arguments are built.
- A forgotten `native()` wrap is made harmless (hook does not fire), but nothing checks at compile time that every import that can reach engine code uses it.
- MSVC timings are still not measured (Round 1 numbers stand; `scope` has the same number of thread-local accesses as before, `catch_unwind` adds none on the happy path).
