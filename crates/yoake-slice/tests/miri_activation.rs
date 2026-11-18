//! Miri-only tests of the activation arena and handle table (MX3MIRI). No wasm, no wasmtime, no FFI: a fake host
//! drives the public API of `yoake_slice::activation`. Compiled only under Miri, so normal `cargo test` is unaffected.
#![cfg(miri)]

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::panic::{catch_unwind, AssertUnwindSafe};

use yoake_slice::activation::*;

#[derive(Debug, Default, PartialEq)]
struct Thing {
    count: i32,
    x: f64,
}

static THING: FieldTable<Thing> = FieldTable {
    get: |t, f| match f {
        0 => Some(i64::from(t.count)),
        1 => Some(t.x.to_bits() as i64),
        _ => None,
    },
    set: |t, f, v| match f {
        0 => {
            t.count = v as i32;
            true
        }
        1 => {
            t.x = f64::from_bits(v as u64);
            true
        }
        _ => false,
    },
};

static THING_REF: RefFieldTable<Thing> = RefFieldTable {
    get: |t, f| match f {
        0 => Some(i64::from(t.count)),
        _ => None,
    },
};

fn nop(_: &mut Thing) -> i64 {
    0
}

/// A `&mut` accessor that itself calls back into the arena (accessors may call hooked engine functions).
static REENTRANT: FieldTable<Thing> = FieldTable {
    get: |t, f| {
        t.count += 1;
        // opens a nested activation on a local object while this one is lent
        let mut local = Thing { count: 100, x: 0.0 };
        let ((), fin) = scope(
            &mut local,
            &THING,
            &mut |l: &mut Thing| {
                l.count += 1;
                i64::from(l.count)
            },
            |h| {
                assert_eq!(run_original(h), Ok(101));
                assert_eq!(get(h, 0), Ok(101));
            },
        );
        assert_eq!(fin.original_calls, 1);
        Some(i64::from(t.count) + i64::from(f))
    },
    set: |_, _, _| false,
};

#[test]
fn miri_mut_handle_round_trip_and_owner_sees_writes() {
    let mut thing = Thing::default();
    let (h, _) = scope(
        &mut thing,
        &THING,
        &mut |t: &mut Thing| {
            t.count *= 2;
            i64::from(t.count)
        },
        |h| {
            assert_eq!(set(h, 0, 21), Ok(()));
            assert_eq!(run_original(h), Ok(42));
            // the `&mut` that `original` used is gone; a fresh access through the handle sees its write
            assert_eq!(get(h, 0), Ok(42));
            assert_eq!(
                with_arg::<Thing, _>(h, |t| {
                    t.x = 1.5;
                    t.count
                }),
                Ok(42)
            );
            assert_eq!(get(h, 1), Ok(1.5f64.to_bits() as i64));
            h
        },
    );
    assert_eq!(thing, Thing { count: 42, x: 1.5 });
    assert_eq!(get(h, 0), Err(Status::Stale));
}

#[test]
fn miri_shared_handle_reads_only() {
    let thing = Thing { count: 5, x: 0.0 };
    let (_, fin) = scope_ref(&thing, &THING_REF, &mut |t: &Thing| i64::from(t.count) + 1, |h| {
        assert_eq!(get(h, 0), Ok(5));
        assert_eq!(set(h, 0, 1), Err(Status::ReadOnly));
        assert_eq!(with_arg::<Thing, _>(h, |_| ()), Err(Status::ReadOnly));
        assert_eq!(run_original(h), Ok(6));
        assert_eq!(get(h, 0), Ok(5));
    });
    assert_eq!(fin.original_calls, 1);
    assert_eq!(thing.count, 5);
}

#[test]
fn miri_two_args_are_distinct_borrows_and_both_locked_during_original() {
    let (mut a, mut b) = (Thing { count: 1, x: 0.0 }, Thing { count: 2, x: 0.0 });
    let hs = Cell::new(0u64);
    let during = RefCell::new(Vec::new());
    let mut orig = |a: &mut Thing, b: &mut Thing| {
        during.borrow_mut().push(get(hs.get(), 0));
        during.borrow_mut().push(get(hs.get() + 1, 0));
        std::mem::swap(&mut a.count, &mut b.count);
        0
    };
    scope2(&mut a, &THING, &mut b, &THING, &mut orig, |h| {
        hs.set(h);
        assert_eq!(set(h + 1, 0, 20), Ok(()));
        run_original(h).unwrap();
        assert_eq!(get(h, 0), Ok(20));
        assert_eq!(get(h + 1, 0), Ok(1));
        assert_eq!(get(h + 2, 0), Err(Status::NoSuchArg));
    });
    assert_eq!(*during.borrow(), vec![Err(Status::Locked), Err(Status::Locked)]);
    assert_eq!((a.count, b.count), (20, 1));
}

#[test]
fn miri_reentry_nested_activation_while_outer_holds_a_lent_handle() {
    let (mut outer, mut inner) = (Thing { count: 1, x: 0.0 }, Thing { count: 10, x: 0.0 });
    let outer_h = Cell::new(0u64);
    let log = RefCell::new(Vec::new());
    let mut orig_outer = |o: &mut Thing| {
        o.count += 1; // `original` holds the live `&mut` to the outer argument here
        scope(
            &mut inner,
            &THING,
            &mut |i: &mut Thing| {
                i.count += 5;
                0
            },
            |ih| {
                log.borrow_mut().push(get(outer_h.get(), 0)); // outer is lent
                log.borrow_mut().push(set(outer_h.get(), 0, 99).map(|()| 0));
                log.borrow_mut().push(run_original(outer_h.get()));
                log.borrow_mut().push(get(ih, 0));
                run_original(ih).unwrap();
                assert_eq!(live_activations(), 2);
                let mut third = Thing::default();
                scope(&mut third, &THING, &mut nop, |ih2| {
                    assert_eq!(live_activations(), 3);
                    assert_eq!(get(ih2, 0), Ok(0));
                    assert_eq!(get(ih, 0), Ok(15), "the middle activation is not lent: its original returned");
                });
            },
        );
        o.count += 1;
        7
    };
    let ((), fin) = scope(&mut outer, &THING, &mut orig_outer, |h| {
        outer_h.set(h);
        assert_eq!(run_original(h), Ok(7));
        assert_eq!(get(h, 0), Ok(3), "writes of `original` are visible; nothing from the nested scope leaked in");
    });
    assert_eq!(fin, Finished { original_calls: 1, last_result: 7 });
    assert_eq!(*log.borrow(), vec![Err(Status::Locked), Err(Status::Locked), Err(Status::Locked), Ok(10)]);
    assert_eq!(inner.count, 15);
    assert_eq!(live_activations(), 0);
}

#[test]
fn miri_accessor_runs_lent_and_may_open_a_nested_activation() {
    let mut thing = Thing::default();
    scope(&mut thing, &REENTRANT, &mut nop_reentrant, |h| {
        assert_eq!(get(h, 7), Ok(1 + 7));
        assert_eq!(get(h, 7), Ok(2 + 7));
    });
    assert_eq!(thing.count, 2);
}

fn nop_reentrant(_: &mut Thing) -> i64 {
    0
}

#[test]
fn miri_handles_are_stale_after_scope_and_never_reused() {
    let mut thing = Thing::default();
    let mut handles = Vec::new();
    for _ in 0..4 {
        let (h, _) = scope(&mut thing, &THING, &mut nop, |h| h);
        assert!(!handles.contains(&h));
        assert_eq!(get(h, 0), Err(Status::Stale));
        assert_eq!(set(h, 0, 1), Err(Status::Stale));
        assert_eq!(run_original(h), Err(Status::Stale));
        assert_eq!(with_arg::<Thing, _>(h, |_| ()), Err(Status::Stale));
        handles.push(h);
    }
    assert_eq!(thing.count, 0);
    // a handle that never existed
    assert_eq!(get(0x77_7700_0000, 0), Err(Status::Stale));
}

#[test]
fn miri_unwind_through_scope_frees_the_arena_and_keeps_the_owner_usable() {
    let mut thing = Thing::default();
    let kept = Cell::new(0u64);
    let r = catch_unwind(AssertUnwindSafe(|| {
        scope(&mut thing, &THING, &mut nop, |h| {
            kept.set(h);
            set(h, 0, 5).unwrap();
            panic!("body panics");
        })
    }));
    assert!(r.is_err());
    assert_eq!(live_activations(), 0);
    assert_eq!(get(kept.get(), 0), Err(Status::Stale));
    thing.count += 1; // the owner's `&mut` is valid again
    assert_eq!(thing.count, 6);

    // a panic inside `original` unwinds through `run`, `Unlend` and the guard
    let r = catch_unwind(AssertUnwindSafe(|| {
        scope(
            &mut thing,
            &THING,
            &mut |t: &mut Thing| {
                t.count = -1;
                panic!("original panics")
            },
            |h| {
                let _ = run_original(h);
            },
        )
    }));
    assert!(r.is_err());
    assert_eq!(live_activations(), 0);
    assert_eq!(thing.count, -1);
}

#[test]
fn miri_unwind_inside_an_accessor_clears_the_lock_of_the_outer_activation() {
    let mut thing = Thing::default();
    scope(&mut thing, &THING, &mut nop, |h| {
        let r = catch_unwind(AssertUnwindSafe(|| with_arg::<Thing, _>(h, |_| panic!("accessor panics"))));
        assert!(r.is_err());
        assert_eq!(get(h, 0), Ok(0), "Unlend ran during the unwind");
    });
}

#[test]
fn miri_other_thread_sees_nothing() {
    let mut thing = Thing::default();
    scope(&mut thing, &THING, &mut nop, |h| {
        let seen = std::thread::spawn(move || (get(h, 0), live_activations())).join().unwrap();
        assert_eq!(seen, (Err(Status::Stale), 0));
    });
}

// ---- path roots ----

struct World {
    things: Vec<Thing>,
}

impl PathRoot for World {
    fn resolve(&mut self, path: &[Accessor], f: &mut dyn FnMut(&mut dyn Any)) -> Result<(), Status> {
        match path {
            [Accessor::Field(0), Accessor::Id(id)] => match self.things.iter_mut().find(|t| t.count as u64 == *id) {
                Some(thing) => {
                    f(thing);
                    Ok(())
                }
                None => Err(Status::Stale),
            },
            _ => Err(Status::BadField),
        }
    }
}

fn path(id: u64) -> Vec<Accessor> {
    vec![Accessor::Field(0), Accessor::Id(id)]
}

#[test]
fn miri_path_handles_reborrow_from_the_root_and_go_stale() {
    let mut world = World { things: vec![Thing { count: 7, x: 1.0 }, Thing { count: 8, x: 2.0 }] };
    let mut kept = Vec::new();
    scope_root(
        &mut world,
        &mut |w: &mut World| {
            w.things.retain(|t| t.count != 7);
            w.things.len() as i64
        },
        |root| {
            let a = open_path(root, path(7)).unwrap();
            let b = open_path(root, path(8)).unwrap();
            kept.push(a);
            kept.push(b);
            assert_eq!(
                with_arg::<Thing, _>(a, |t| {
                    t.x += 1.0;
                    t.x
                }),
                Ok(2.0)
            );
            // a second path access from inside an access is an overlap
            let nested = with_arg::<Thing, _>(a, |_| with_arg::<Thing, _>(b, |_| ()));
            assert_eq!(nested, Ok(Err(Status::Locked)));
            // `original` moves the elements of the Vec: a retained reference would dangle, handles re-resolve
            assert_eq!(run_original(root), Ok(1));
            assert_eq!(with_arg::<Thing, _>(a, |_| ()), Err(Status::Stale));
            assert_eq!(with_arg::<Thing, _>(b, |t| t.count), Ok(8));
            // a new handle of the same object is another handle
            let c = open_path(root, path(8)).unwrap();
            assert_ne!(c, b);
            assert_eq!(with_arg::<Thing, _>(c, |t| t.count), Ok(8));
            kept.push(c);
        },
    );
    assert_eq!(live_path_handles(), 0);
    for h in kept {
        assert_eq!(with_arg::<Thing, _>(h, |_| ()), Err(Status::Stale));
    }
    // handles of a finished activation do not alias a later activation's path slots
    let mut world2 = World { things: vec![Thing { count: 7, x: 0.0 }] };
    let mut old = 0;
    scope_root(&mut world2, &mut |_| 0, |root| old = open_path(root, path(7)).unwrap());
    scope_root(&mut world2, &mut |_| 0, |root| {
        let fresh = open_path(root, path(7)).unwrap();
        assert_ne!(fresh, old);
        assert_eq!(with_arg::<Thing, _>(old, |_| ()), Err(Status::Stale));
        assert_eq!(with_arg::<Thing, _>(fresh, |t| t.count), Ok(7));
    });
}

#[test]
fn miri_path_resolver_panic_frees_everything() {
    struct Bomb;
    impl PathRoot for Bomb {
        fn resolve(&mut self, _: &[Accessor], f: &mut dyn FnMut(&mut dyn Any)) -> Result<(), Status> {
            let mut v = 3u32;
            f(&mut v);
            Ok(())
        }
    }
    let mut bomb = Bomb;
    let r = catch_unwind(AssertUnwindSafe(|| {
        scope_root(&mut bomb, &mut |_| 0, |root| {
            let h = open_path(root, vec![]).unwrap();
            let _ = with_arg::<u32, _>(h, |_| panic!("f panics inside the resolver"));
        })
    }));
    assert!(r.is_err());
    assert_eq!(live_activations(), 0);
    assert_eq!(live_path_handles(), 0);
}

/// The `ACTIVE` pointer pattern of `runtime.rs` (`with_active` / `context` / `dispatch_with`), on a fake context:
/// a `Cell<*mut Ctx>` that is null while "guest code" runs, republished by an "import" (`native`) for the duration
/// of that import, restored by a guard (also on unwind). `runtime.rs` itself needs wasmtime and cannot run under
/// Miri; this models only the pointer discipline, not the real code.
mod active_model {
    use super::*;

    pub struct Ctx {
        pub depth: u32,
        pub log: Vec<u32>,
    }

    pub struct Guard<'r> {
        cell: &'r Cell<*mut Ctx>,
        previous: *mut Ctx,
    }
    impl Drop for Guard<'_> {
        fn drop(&mut self) {
            self.cell.set(self.previous);
        }
    }

    pub fn with_active<T>(cell: &Cell<*mut Ctx>, ctx: &mut Ctx, f: impl FnOnce(*mut Ctx) -> T) -> T {
        let pointer = ctx as *mut Ctx;
        let _restore = Guard { cell, previous: cell.replace(pointer) };
        f(pointer)
    }

    /// # Safety
    /// `pointer` is the innermost published context and no other reference to it is live.
    pub unsafe fn context<'x>(pointer: *mut Ctx) -> &'x mut Ctx {
        unsafe { &mut *pointer }
    }

    /// "Guest code": runs with `active` cleared to null, calls an import that republishes the context.
    pub fn run_guest(cell: &Cell<*mut Ctx>, ctx: *mut Ctx, import_depth: u32) {
        let saved = cell.replace(std::ptr::null_mut());
        assert!(cell.get().is_null());
        // SAFETY: model of the store lent to the guest: no reference to `*ctx` is live here.
        unsafe { (*ctx).log.push(import_depth) };
        // the import (`native`): re-derives the context from the pointer it was handed and republishes it
        let ctx_ref = unsafe { context(ctx) };
        with_active(cell, ctx_ref, |p| {
            let inner = cell.get();
            assert_eq!(inner, p);
            let c = unsafe { context(inner) };
            c.depth += 1;
            if import_depth > 0 {
                run_guest(cell, p, import_depth - 1);
            }
            let c = unsafe { context(p) };
            c.depth -= 1;
        });
        assert!(cell.get().is_null(), "restored to null (the guest is running again)");
        cell.set(saved);
    }
}

#[test]
fn miri_active_pointer_is_null_while_a_guest_runs_and_republished_by_native() {
    use active_model::*;
    let mut ctx = Ctx { depth: 0, log: Vec::new() };
    let cell: Cell<*mut Ctx> = Cell::new(std::ptr::null_mut());
    with_active(&cell, &mut ctx, |p| {
        assert_eq!(cell.get(), p);
        run_guest(&cell, p, 2);
        assert_eq!(cell.get(), p, "restored after the guest returned");
    });
    assert!(cell.get().is_null());
    assert_eq!(ctx.log, vec![2, 1, 0]);
    assert_eq!(ctx.depth, 0);
    // an unwind restores the previous pointer
    let r = catch_unwind(AssertUnwindSafe(|| {
        with_active(&cell, &mut ctx, |_| panic!("guest trapped"));
    }));
    assert!(r.is_err());
    assert!(cell.get().is_null());
}
