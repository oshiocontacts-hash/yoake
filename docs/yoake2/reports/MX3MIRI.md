# MX3MIRI: Miri over the unsafe code of Yoake 2.0

Worktree: `C:\Users\USER\NestgateHorizon\horizon-miri` (detached at 3828f998b). Nothing committed or pushed. Host x86_64-pc-windows-gnu, nightly Miri, `-j 4`, mingw64 on PATH. Raw logs: `C:\Users\USER\NestgateHorizon\reports\mx3miri-logs\`.

## Checklist

- [x] Read the unsafe surface (activation.rs, bridge.rs, codec.rs, hook-rt lib.rs, runtime.rs ACTIVE pointer)
- [x] Existing tests of `yoake-hook-rt` under Miri: Stacked Borrows (default) and Tree Borrows, with `-Zmiri-strict-provenance`
- [x] Existing lib tests of `yoake-slice` (no `wasm` feature) under Miri, both models
- [x] New Miri-only tests written (2 new files), run under both models
- [x] Leak reports analysed (one experiment, below)
- [x] Report written

## Commands

```
export PATH="/c/Users/USER/NestgateHorizon/toolchain/mingw64/bin:$PATH"
# hook-rt (all targets): leak check on, then Stacked Borrows and Tree Borrows with leaks ignored
MIRIFLAGS="-Zmiri-strict-provenance"                                          cargo +nightly miri test -j 4 -p yoake-hook-rt --no-fail-fast
MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-ignore-leaks"                      cargo +nightly miri test -j 4 -p yoake-hook-rt --no-fail-fast
MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-ignore-leaks -Zmiri-tree-borrows"  cargo +nightly miri test -j 4 -p yoake-hook-rt --no-fail-fast
# slice lib (activation, bridge, codec); no wasm feature
MIRIFLAGS="-Zmiri-strict-provenance"                      cargo +nightly miri test -j 4 -p yoake-slice --lib --no-fail-fast
MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-tree-borrows"  cargo +nightly miri test -j 4 -p yoake-slice --lib --no-fail-fast
# new tests
MIRIFLAGS="-Zmiri-strict-provenance"                      cargo +nightly miri test -j 4 -p yoake-slice --test miri_activation
MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-tree-borrows"  cargo +nightly miri test -j 4 -p yoake-slice --test miri_activation
MIRIFLAGS="-Zmiri-strict-provenance -Zmiri-ignore-leaks [-Zmiri-tree-borrows]" cargo +nightly miri test -j 4 -p yoake-hook-rt --test miri_slots
```

`-Zmiri-strict-provenance` worked everywhere: no int-to-pointer cast is reached by any test that can run. Handles are plain `u64` serials and generations, never addresses, so there is nothing to flag.

## Counts (same result under Stacked Borrows and Tree Borrows)

| target | pass | fail | skip |
|---|---|---|---|
| yoake-hook-rt tests/callsite.rs | 1 | 0 | 0 |
| yoake-hook-rt tests/ledger_key.rs | 5 | 0 | 0 |
| yoake-hook-rt tests/mechanics.rs | 12 | 0 | 0 |
| yoake-hook-rt tests/miri_slots.rs (new) | 3 | 0 | 0 |
| yoake-slice `--lib` (activation, bridge, codec) | 27 | 0 | 0 |
| yoake-slice tests/miri_activation.rs (new) | 12 | 0 | 0 |
| yoake-slice tests/slice.rs 21, multi.rs 6, policy.rs 6, readonly.rs 6, modes.rs 1 | 0 | 0 | 40 |
| yoake-slice runtime.rs unit test `a_ticket_status_always_fits_an_i32` | 0 | 0 | 1 |

Totals: 60 run, 60 passed, no UB, no data race, under both models. (The `finished in` times of the Tree Borrows runs equal the Stacked Borrows ones because Miri prints virtual time.)

Two crate-level failures exist only from the end-of-run leak check, see F1.

### Skipped, with the reason

- The five integration tests of yoake-slice (40 tests): `required-features = ["wasm"]`; they instantiate wasmtime (Cranelift JIT, mmap, signal handlers), which Miri cannot execute.
- `runtime::tests::a_ticket_status_always_fits_an_i32`: pure, but `runtime.rs` is compiled only with the `wasm` feature (wasmtime); not worth building wasmtime under Miri for one test.
- Examples (`slice_bench`, hook-rt examples): benchmarks, not tests.

## Findings

### Undefined behaviour: none

No Stacked Borrows violation, no Tree Borrows violation, no data race, no invalid value, in any test that can run. That includes 15 new tests written for re-entry, lending, unwinding and Stale handles.

### F1. Leak reports (not a bug, a design cost; needs a CI flag)

1. hook-rt `__register_typed`. Message: `error: memory leaked: alloc... (Rust heap, size: 128, align: 8)` and `(size: 24, align: 8)`, allocated at `crates/yoake-hook-rt/src/lib.rs:311-315` (`Vec<Entry>` growth) and the `Box::new(Chain { entries })` at the end of the function. Tests: `tests/callsite.rs::one_callsite_wraps_replaces_and_skips_without_affecting_the_other_call` (4 reports) and `tests/mechanics.rs` (many, from `concurrent_publication_keeps_old_snapshots_alive_and_bounds_registrations`). Reading: every republish leaks the previous `Box<Chain>` (24 bytes) and its entry buffer (128 bytes at the first growth). Experiment (temporary test, removed): one registration on a slot gives 0 reports (the live chain is reachable from the `SLOTS` static); two registrations give exactly 2 (the superseded chain). This is the documented design (old chains leaked so lock-free readers never see freed memory). The cost is bounded by the 64-entries-per-slot limit: at most about 64 superseded chains per slot, roughly 65 KB worst case per slot. Suggested handling: keep; run Miri on this crate with `-Zmiri-ignore-leaks` (without it `cargo miri test -p yoake-hook-rt` fails on callsite and mechanics). If registrations ever happen in volume after startup, add epoch or hazard-pointer reclamation.
2. yoake-slice lib test `activation::tests::forgetting_cannot_keep_an_activation_alive` (`activation.rs:814`, `std::mem::forget(Box::new(h))`) leaks on purpose. All 27 tests pass; only the end-of-run leak check fails (`error: memory leaked: alloc... (Rust heap, size: 8, align: 8)`). Suggested handling: `-Zmiri-ignore-leaks` for this crate too, or `drop` the box after the assertion if the leak check is wanted in CI.

### F2. Reading notes on the unsafe surface (no error found)

1. `activation.rs` `scope*`: the raw pointer is derived from the `&mut A` argument, which is never used again inside the call; the `Cell` holds the original closure. Both models accept it, including a nested `scope` while the outer `run_cell` holds `&mut *cell.a`, an accessor opening a nested activation on a local, and a three-deep nest.
2. `run_original`, `get`, `with_arg`, `open_path`, `with_path` copy the `ArgSlot` (it is `Copy`) out of the `RefCell<Acts>` before user code runs, so no reference into the arena is live while a nested activation pushes to `Acts::stack` (which may reallocate). The nesting tests confirm there is no dangling.
3. The `ACTIVE` pointer in `runtime.rs` (`with_active`, `context`, `dispatch_with`; null while a guest runs, republished by `native()`) cannot run under Miri: `StoreContextMut` needs a wasmtime `Store`, hence an Engine. It is covered only by a model (`active_model` in `miri_activation.rs`): the same pointer discipline on a fake `Ctx`, null while the guest runs, republished for the import, nested re-entry through the pointer, restore by drop guard on normal exit and on unwind. The model is clean under both models. It proves the pattern, not `runtime.rs`. The real `unsafe { &mut *pointer }` in `context` depends on the SAFETY claim that native code between publish and use does not use the `Caller` it came from; Miri cannot check that against wasmtime. The `'_` to `'static` pointer cast in `with_active` is a lifetime-only cast, harmless as long as the pointer does not outlive the frame (the drop guard restores it).
4. `codec.rs` and `bridge.rs` contain no `unsafe`; their tests pass (the deep-nesting codec test dominates the run: about 127 s of Miri time for the whole lib suite).

## What I added (worktree only, two untracked files; nothing else touched)

- `crates/yoake-slice/tests/miri_activation.rs` (`#![cfg(miri)]`, no `wasm` feature), 12 tests: Mut handle round trip and the owner sees writes; Shared handle (ReadOnly paths, `original` through `&A`); two-argument scope with both handles Locked during `original`; re-entry with a lent outer handle (get, set and run_original from the nested scope are Locked; three-deep nest; the middle activation is not lent after its `original` returned); an accessor that opens a nested activation; Stale after the scope and never-reused serials; unwind from the body and from `original`; unwind inside an accessor clears the lock; another thread sees Stale; path handles re-resolving after `Vec::retain` moved the elements, nested path access Locked, generation change on slot reuse, old handle Stale across activations; resolver panic frees everything; the `ACTIVE` pointer model.
- `crates/yoake-hook-rt/tests/miri_slots.rs` (`#![cfg(miri)]`), 3 tests: an old `Snapshot` stays readable after two republishes and the new chain gives the right order and result; typed registration rejects a different callback type, an out-of-range id and the 65th entry; two writer threads republishing while a reader dispatches (no race).

`#![cfg(miri)]` makes both files empty in normal `cargo test`. The diff is these two new files in full (below).

## Not covered

- `runtime.rs` and everything that needs wasmtime: the real `ACTIVE` pointer, `native()` imports, fuel and epoch, the trap rule, the 40 integration tests. Only the pointer pattern is modelled.
- The generated hook bridges and `callsite!` expansions in the engine crates (outside the listed files); `yoake-hook-macro`.
- Real multi-threaded engine paths: the only multi-thread tests are `Slot` publish and read, and "other thread sees nothing". Default scheduler only, no `-Zmiri-many-seeds` (suggest `-Zmiri-many-seeds=0..16` on `miri_slots` in CI).
- Release-mode and `inline(always)` code (Miri ran unoptimized).
- `-Zmiri-symbolic-alignment-check`, `-Zmiri-recursive-validation`, preemption-rate variations: not tried.

## Appendix: diff of the new files

```diff
+++ b/crates/yoake-hook-rt/tests/miri_slots.rs
@@ -0,0 +1,75 @@
+//! Miri-only tests of the hook slots (MX3MIRI): publication of chains, snapshots that outlive a republish,
+//! typed registration, concurrent registration. Compiled only under Miri.
+#![cfg(miri)]
+
+use yoake_hook_rt::{CallsiteHook, Order, Slot, SLOTS, TEST_ID_BASE, __register_typed};
+
+type Hook = CallsiteHook<u32, u32>;
+
+fn register(slot: usize, hook: Hook, order: Order) -> Result<(), &'static str> {
+    __register_typed(slot, std::any::TypeId::of::<Hook>(), hook, order)
+}
+
+fn run(slot: &'static Slot, value: u32) -> u32 {
+    match slot.load() {
+        Some(chain) => yoake_hook_rt::callsite_dispatch(chain, value, &mut |v| v + 1),
+        None => value + 1,
+    }
+}
+
+#[test]
+fn miri_an_old_snapshot_stays_readable_after_a_republish() {
+    let id = TEST_ID_BASE + 40;
+    assert!(SLOTS[id].load().is_none());
+    register(id, |original, v| original(v) + 100, Order::default()).unwrap();
+    let old = SLOTS[id].load().unwrap();
+    // republish twice: the chain `old` points into is superseded (and intentionally leaked)
+    register(id, |original, v| original(v) * 2, Order { priority: 5, mod_load_order: 0 }).unwrap();
+    register(id, |_, _| 7, Order { priority: -5, mod_load_order: 1 }).unwrap();
+    let (first, rest) = old.next::<Hook>().unwrap();
+    assert!(rest.next::<Hook>().is_none());
+    let mut orig = |v: u32| v + 1;
+    assert_eq!(first(&mut orig, 1), 102);
+    // the new chain: x2 (priority 5) around +100 (priority 0); the priority -5 hook ignores `original` and returns 7
+    assert_eq!(run(&SLOTS[id], 1), 214);
+}
+
+#[test]
+fn miri_registration_is_typed_and_bounded() {
+    let id = TEST_ID_BASE + 41;
+    register(id, |o, v| o(v), Order::default()).unwrap();
+    type Other = fn(u8) -> u8;
+    let other: Other = |v| v;
+    assert!(__register_typed(id, std::any::TypeId::of::<Other>(), other, Order::default()).is_err());
+    assert!(__register_typed(1 << 20, std::any::TypeId::of::<Other>(), other, Order::default()).is_err());
+    for _ in 1..64 {
+        register(id, |o, v| o(v), Order::default()).unwrap();
+    }
+    assert!(register(id, |o, v| o(v), Order::default()).is_err());
+}
+
+#[test]
+fn miri_concurrent_registration_and_reads_do_not_race() {
+    let id = TEST_ID_BASE + 42;
+    let writers: Vec<_> = (0..2)
+        .map(|n| {
+            std::thread::spawn(move || {
+                for k in 0..4 {
+                    register(id, |o, v| o(v) + 1, Order { priority: n, mod_load_order: k }).unwrap();
+                }
+            })
+        })
+        .collect();
+    let reader = std::thread::spawn(move || {
+        let mut last = 0;
+        for _ in 0..8 {
+            last = run(&SLOTS[id], 0);
+        }
+        last
+    });
+    for w in writers {
+        w.join().unwrap();
+    }
+    reader.join().unwrap();
+    assert_eq!(run(&SLOTS[id], 0), 8 + 1);
+}
```

```diff
+++ b/crates/yoake-slice/tests/miri_activation.rs
@@ -0,0 +1,444 @@
+//! Miri-only tests of the activation arena and handle table (MX3MIRI). No wasm, no wasmtime, no FFI: a fake host
+//! drives the public API of `yoake_slice::activation`. Compiled only under Miri, so normal `cargo test` is unaffected.
+#![cfg(miri)]
+
+use std::any::Any;
+use std::cell::{Cell, RefCell};
+use std::panic::{catch_unwind, AssertUnwindSafe};
+
+use yoake_slice::activation::*;
+
+#[derive(Debug, Default, PartialEq)]
+struct Thing {
+    count: i32,
+    x: f64,
+}
+
+static THING: FieldTable<Thing> = FieldTable {
+    get: |t, f| match f {
+        0 => Some(i64::from(t.count)),
+        1 => Some(t.x.to_bits() as i64),
+        _ => None,
+    },
+    set: |t, f, v| match f {
+        0 => {
+            t.count = v as i32;
+            true
+        }
+        1 => {
+            t.x = f64::from_bits(v as u64);
+            true
+        }
+        _ => false,
+    },
+};
+
+static THING_REF: RefFieldTable<Thing> = RefFieldTable {
+    get: |t, f| match f {
+        0 => Some(i64::from(t.count)),
+        _ => None,
+    },
+};
+
+fn nop(_: &mut Thing) -> i64 {
+    0
+}
+
+/// A `&mut` accessor that itself calls back into the arena (accessors may call hooked engine functions).
+static REENTRANT: FieldTable<Thing> = FieldTable {
+    get: |t, f| {
+        t.count += 1;
+        // opens a nested activation on a local object while this one is lent
+        let mut local = Thing { count: 100, x: 0.0 };
+        let ((), fin) = scope(
+            &mut local,
+            &THING,
+            &mut |l: &mut Thing| {
+                l.count += 1;
+                i64::from(l.count)
+            },
+            |h| {
+                assert_eq!(run_original(h), Ok(101));
+                assert_eq!(get(h, 0), Ok(101));
+            },
+        );
+        assert_eq!(fin.original_calls, 1);
+        Some(i64::from(t.count) + i64::from(f))
+    },
+    set: |_, _, _| false,
+};
+
+#[test]
+fn miri_mut_handle_round_trip_and_owner_sees_writes() {
+    let mut thing = Thing::default();
+    let (h, _) = scope(
+        &mut thing,
+        &THING,
+        &mut |t: &mut Thing| {
+            t.count *= 2;
+            i64::from(t.count)
+        },
+        |h| {
+            assert_eq!(set(h, 0, 21), Ok(()));
+            assert_eq!(run_original(h), Ok(42));
+            // the `&mut` that `original` used is gone; a fresh access through the handle sees its write
+            assert_eq!(get(h, 0), Ok(42));
+            assert_eq!(
+                with_arg::<Thing, _>(h, |t| {
+                    t.x = 1.5;
+                    t.count
+                }),
+                Ok(42)
+            );
+            assert_eq!(get(h, 1), Ok(1.5f64.to_bits() as i64));
+            h
+        },
+    );
+    assert_eq!(thing, Thing { count: 42, x: 1.5 });
+    assert_eq!(get(h, 0), Err(Status::Stale));
+}
+
+#[test]
+fn miri_shared_handle_reads_only() {
+    let thing = Thing { count: 5, x: 0.0 };
+    let (_, fin) = scope_ref(&thing, &THING_REF, &mut |t: &Thing| i64::from(t.count) + 1, |h| {
+        assert_eq!(get(h, 0), Ok(5));
+        assert_eq!(set(h, 0, 1), Err(Status::ReadOnly));
+        assert_eq!(with_arg::<Thing, _>(h, |_| ()), Err(Status::ReadOnly));
+        assert_eq!(run_original(h), Ok(6));
+        assert_eq!(get(h, 0), Ok(5));
+    });
+    assert_eq!(fin.original_calls, 1);
+    assert_eq!(thing.count, 5);
+}
+
+#[test]
+fn miri_two_args_are_distinct_borrows_and_both_locked_during_original() {
+    let (mut a, mut b) = (Thing { count: 1, x: 0.0 }, Thing { count: 2, x: 0.0 });
+    let hs = Cell::new(0u64);
+    let during = RefCell::new(Vec::new());
+    let mut orig = |a: &mut Thing, b: &mut Thing| {
+        during.borrow_mut().push(get(hs.get(), 0));
+        during.borrow_mut().push(get(hs.get() + 1, 0));
+        std::mem::swap(&mut a.count, &mut b.count);
+        0
+    };
+    scope2(&mut a, &THING, &mut b, &THING, &mut orig, |h| {
+        hs.set(h);
+        assert_eq!(set(h + 1, 0, 20), Ok(()));
+        run_original(h).unwrap();
+        assert_eq!(get(h, 0), Ok(20));
+        assert_eq!(get(h + 1, 0), Ok(1));
+        assert_eq!(get(h + 2, 0), Err(Status::NoSuchArg));
+    });
+    assert_eq!(*during.borrow(), vec![Err(Status::Locked), Err(Status::Locked)]);
+    assert_eq!((a.count, b.count), (20, 1));
+}
+
+#[test]
+fn miri_reentry_nested_activation_while_outer_holds_a_lent_handle() {
+    let (mut outer, mut inner) = (Thing { count: 1, x: 0.0 }, Thing { count: 10, x: 0.0 });
+    let outer_h = Cell::new(0u64);
+    let log = RefCell::new(Vec::new());
+    let mut orig_outer = |o: &mut Thing| {
+        o.count += 1; // `original` holds the live `&mut` to the outer argument here
+        scope(
+            &mut inner,
+            &THING,
+            &mut |i: &mut Thing| {
+                i.count += 5;
+                0
+            },
+            |ih| {
+                log.borrow_mut().push(get(outer_h.get(), 0)); // outer is lent
+                log.borrow_mut().push(set(outer_h.get(), 0, 99).map(|()| 0));
+                log.borrow_mut().push(run_original(outer_h.get()));
+                log.borrow_mut().push(get(ih, 0));
+                run_original(ih).unwrap();
+                assert_eq!(live_activations(), 2);
+                let mut third = Thing::default();
+                scope(&mut third, &THING, &mut nop, |ih2| {
+                    assert_eq!(live_activations(), 3);
+                    assert_eq!(get(ih2, 0), Ok(0));
+                    assert_eq!(get(ih, 0), Ok(15), "the middle activation is not lent: its original returned");
+                });
+            },
+        );
+        o.count += 1;
+        7
+    };
+    let ((), fin) = scope(&mut outer, &THING, &mut orig_outer, |h| {
+        outer_h.set(h);
+        assert_eq!(run_original(h), Ok(7));
+        assert_eq!(get(h, 0), Ok(3), "writes of `original` are visible; nothing from the nested scope leaked in");
+    });
+    assert_eq!(fin, Finished { original_calls: 1, last_result: 7 });
+    assert_eq!(*log.borrow(), vec![Err(Status::Locked), Err(Status::Locked), Err(Status::Locked), Ok(10)]);
+    assert_eq!(inner.count, 15);
+    assert_eq!(live_activations(), 0);
+}
+
+#[test]
+fn miri_accessor_runs_lent_and_may_open_a_nested_activation() {
+    let mut thing = Thing::default();
+    scope(&mut thing, &REENTRANT, &mut nop_reentrant, |h| {
+        assert_eq!(get(h, 7), Ok(1 + 7));
+        assert_eq!(get(h, 7), Ok(2 + 7));
+    });
+    assert_eq!(thing.count, 2);
+}
+
+fn nop_reentrant(_: &mut Thing) -> i64 {
+    0
+}
+
+#[test]
+fn miri_handles_are_stale_after_scope_and_never_reused() {
+    let mut thing = Thing::default();
+    let mut handles = Vec::new();
+    for _ in 0..4 {
+        let (h, _) = scope(&mut thing, &THING, &mut nop, |h| h);
+        assert!(!handles.contains(&h));
+        assert_eq!(get(h, 0), Err(Status::Stale));
+        assert_eq!(set(h, 0, 1), Err(Status::Stale));
+        assert_eq!(run_original(h), Err(Status::Stale));
+        assert_eq!(with_arg::<Thing, _>(h, |_| ()), Err(Status::Stale));
+        handles.push(h);
+    }
+    assert_eq!(thing.count, 0);
+    // a handle that never existed
+    assert_eq!(get(0x77_7700_0000, 0), Err(Status::Stale));
+}
+
+#[test]
+fn miri_unwind_through_scope_frees_the_arena_and_keeps_the_owner_usable() {
+    let mut thing = Thing::default();
+    let kept = Cell::new(0u64);
+    let r = catch_unwind(AssertUnwindSafe(|| {
+        scope(&mut thing, &THING, &mut nop, |h| {
+            kept.set(h);
+            set(h, 0, 5).unwrap();
+            panic!("body panics");
+        })
+    }));
+    assert!(r.is_err());
+    assert_eq!(live_activations(), 0);
+    assert_eq!(get(kept.get(), 0), Err(Status::Stale));
+    thing.count += 1; // the owner's `&mut` is valid again
+    assert_eq!(thing.count, 6);
+
+    // a panic inside `original` unwinds through `run`, `Unlend` and the guard
+    let r = catch_unwind(AssertUnwindSafe(|| {
+        scope(
+            &mut thing,
+            &THING,
+            &mut |t: &mut Thing| {
+                t.count = -1;
+                panic!("original panics")
+            },
+            |h| {
+                let _ = run_original(h);
+            },
+        )
+    }));
+    assert!(r.is_err());
+    assert_eq!(live_activations(), 0);
+    assert_eq!(thing.count, -1);
+}
+
+#[test]
+fn miri_unwind_inside_an_accessor_clears_the_lock_of_the_outer_activation() {
+    let mut thing = Thing::default();
+    scope(&mut thing, &THING, &mut nop, |h| {
+        let r = catch_unwind(AssertUnwindSafe(|| with_arg::<Thing, _>(h, |_| panic!("accessor panics"))));
+        assert!(r.is_err());
+        assert_eq!(get(h, 0), Ok(0), "Unlend ran during the unwind");
+    });
+}
+
+#[test]
+fn miri_other_thread_sees_nothing() {
+    let mut thing = Thing::default();
+    scope(&mut thing, &THING, &mut nop, |h| {
+        let seen = std::thread::spawn(move || (get(h, 0), live_activations())).join().unwrap();
+        assert_eq!(seen, (Err(Status::Stale), 0));
+    });
+}
+
+// ---- path roots ----
+
+struct World {
+    things: Vec<Thing>,
+}
+
+impl PathRoot for World {
+    fn resolve(&mut self, path: &[Accessor], f: &mut dyn FnMut(&mut dyn Any)) -> Result<(), Status> {
+        match path {
+            [Accessor::Field(0), Accessor::Id(id)] => match self.things.iter_mut().find(|t| t.count as u64 == *id) {
+                Some(thing) => {
+                    f(thing);
+                    Ok(())
+                }
+                None => Err(Status::Stale),
+            },
+            _ => Err(Status::BadField),
+        }
+    }
+}
+
+fn path(id: u64) -> Vec<Accessor> {
+    vec![Accessor::Field(0), Accessor::Id(id)]
+}
+
+#[test]
+fn miri_path_handles_reborrow_from_the_root_and_go_stale() {
+    let mut world = World { things: vec![Thing { count: 7, x: 1.0 }, Thing { count: 8, x: 2.0 }] };
+    let mut kept = Vec::new();
+    scope_root(
+        &mut world,
+        &mut |w: &mut World| {
+            w.things.retain(|t| t.count != 7);
+            w.things.len() as i64
+        },
+        |root| {
+            let a = open_path(root, path(7)).unwrap();
+            let b = open_path(root, path(8)).unwrap();
+            kept.push(a);
+            kept.push(b);
+            assert_eq!(
+                with_arg::<Thing, _>(a, |t| {
+                    t.x += 1.0;
+                    t.x
+                }),
+                Ok(2.0)
+            );
+            // a second path access from inside an access is an overlap
+            let nested = with_arg::<Thing, _>(a, |_| with_arg::<Thing, _>(b, |_| ()));
+            assert_eq!(nested, Ok(Err(Status::Locked)));
+            // `original` moves the elements of the Vec: a retained reference would dangle, handles re-resolve
+            assert_eq!(run_original(root), Ok(1));
+            assert_eq!(with_arg::<Thing, _>(a, |_| ()), Err(Status::Stale));
+            assert_eq!(with_arg::<Thing, _>(b, |t| t.count), Ok(8));
+            // a new handle of the same object is another handle
+            let c = open_path(root, path(8)).unwrap();
+            assert_ne!(c, b);
+            assert_eq!(with_arg::<Thing, _>(c, |t| t.count), Ok(8));
+            kept.push(c);
+        },
+    );
+    assert_eq!(live_path_handles(), 0);
+    for h in kept {
+        assert_eq!(with_arg::<Thing, _>(h, |_| ()), Err(Status::Stale));
+    }
+    // handles of a finished activation do not alias a later activation's path slots
+    let mut world2 = World { things: vec![Thing { count: 7, x: 0.0 }] };
+    let mut old = 0;
+    scope_root(&mut world2, &mut |_| 0, |root| old = open_path(root, path(7)).unwrap());
+    scope_root(&mut world2, &mut |_| 0, |root| {
+        let fresh = open_path(root, path(7)).unwrap();
+        assert_ne!(fresh, old);
+        assert_eq!(with_arg::<Thing, _>(old, |_| ()), Err(Status::Stale));
+        assert_eq!(with_arg::<Thing, _>(fresh, |t| t.count), Ok(7));
+    });
+}
+
+#[test]
+fn miri_path_resolver_panic_frees_everything() {
+    struct Bomb;
+    impl PathRoot for Bomb {
+        fn resolve(&mut self, _: &[Accessor], f: &mut dyn FnMut(&mut dyn Any)) -> Result<(), Status> {
+            let mut v = 3u32;
+            f(&mut v);
+            Ok(())
+        }
+    }
+    let mut bomb = Bomb;
+    let r = catch_unwind(AssertUnwindSafe(|| {
+        scope_root(&mut bomb, &mut |_| 0, |root| {
+            let h = open_path(root, vec![]).unwrap();
+            let _ = with_arg::<u32, _>(h, |_| panic!("f panics inside the resolver"));
+        })
+    }));
+    assert!(r.is_err());
+    assert_eq!(live_activations(), 0);
+    assert_eq!(live_path_handles(), 0);
+}
+
+/// The `ACTIVE` pointer pattern of `runtime.rs` (`with_active` / `context` / `dispatch_with`), on a fake context:
+/// a `Cell<*mut Ctx>` that is null while "guest code" runs, republished by an "import" (`native`) for the duration
+/// of that import, restored by a guard (also on unwind). `runtime.rs` itself needs wasmtime and cannot run under
+/// Miri; this models only the pointer discipline, not the real code.
+mod active_model {
+    use super::*;
+
+    pub struct Ctx {
+        pub depth: u32,
+        pub log: Vec<u32>,
+    }
+
+    pub struct Guard<'r> {
+        cell: &'r Cell<*mut Ctx>,
+        previous: *mut Ctx,
+    }
+    impl Drop for Guard<'_> {
+        fn drop(&mut self) {
+            self.cell.set(self.previous);
+        }
+    }
+
+    pub fn with_active<T>(cell: &Cell<*mut Ctx>, ctx: &mut Ctx, f: impl FnOnce(*mut Ctx) -> T) -> T {
+        let pointer = ctx as *mut Ctx;
+        let _restore = Guard { cell, previous: cell.replace(pointer) };
+        f(pointer)
+    }
+
+    /// # Safety
+    /// `pointer` is the innermost published context and no other reference to it is live.
+    pub unsafe fn context<'x>(pointer: *mut Ctx) -> &'x mut Ctx {
+        unsafe { &mut *pointer }
+    }
+
+    /// "Guest code": runs with `active` cleared to null, calls an import that republishes the context.
+    pub fn run_guest(cell: &Cell<*mut Ctx>, ctx: *mut Ctx, import_depth: u32) {
+        let saved = cell.replace(std::ptr::null_mut());
+        assert!(cell.get().is_null());
+        // SAFETY: model of the store lent to the guest: no reference to `*ctx` is live here.
+        unsafe { (*ctx).log.push(import_depth) };
+        // the import (`native`): re-derives the context from the pointer it was handed and republishes it
+        let ctx_ref = unsafe { context(ctx) };
+        with_active(cell, ctx_ref, |p| {
+            let inner = cell.get();
+            assert_eq!(inner, p);
+            let c = unsafe { context(inner) };
+            c.depth += 1;
+            if import_depth > 0 {
+                run_guest(cell, p, import_depth - 1);
+            }
+            let c = unsafe { context(p) };
+            c.depth -= 1;
+        });
+        assert!(cell.get().is_null(), "restored to null (the guest is running again)");
+        cell.set(saved);
+    }
+}
+
+#[test]
+fn miri_active_pointer_is_null_while_a_guest_runs_and_republished_by_native() {
+    use active_model::*;
+    let mut ctx = Ctx { depth: 0, log: Vec::new() };
+    let cell: Cell<*mut Ctx> = Cell::new(std::ptr::null_mut());
+    with_active(&cell, &mut ctx, |p| {
+        assert_eq!(cell.get(), p);
+        run_guest(&cell, p, 2);
+        assert_eq!(cell.get(), p, "restored after the guest returned");
+    });
+    assert!(cell.get().is_null());
+    assert_eq!(ctx.log, vec![2, 1, 0]);
+    assert_eq!(ctx.depth, 0);
+    // an unwind restores the previous pointer
+    let r = catch_unwind(AssertUnwindSafe(|| {
+        with_active(&cell, &mut ctx, |_| panic!("guest trapped"));
+    }));
+    assert!(r.is_err());
+    assert!(cell.get().is_null());
+}
```
