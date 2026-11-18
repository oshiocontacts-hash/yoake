//! Hook activations: the per-activation arena of handles.
//!
//! An *activation* is one run of a hook shim: the engine called a hooked function, the shim
//! registered the function's `&mut` arguments and its `original` callable here, and a guest may
//! now reach them through 64-bit handles until the activation ends.
//!
//! * `Root::HookArg { activation, arg }` is the handle root for a hook argument. The caller of the
//!   hooked function holds an outer `&mut` to it, so it can never be re-borrowed from the `Game`
//!   root; the activation stores the pointer the shim was handed and re-derives the reference on
//!   every access.
//! * Handles are never reused: `activation` is a process-wide 56-bit serial. A handle of an ended
//!   activation is `Status::Stale`, whatever the guest kept.
//! * The lending lock: while `original` runs, every handle of that activation is *lent* to it.
//!   A guest access through a lent handle returns `Status::Locked`; no second `&mut` is ever made.
//! * The arena lives in a thread-local stack and is freed when the [`Activation`] guard drops
//!   (normal end, trap, or panic).
//!
//! # Unsafe surface
//!
//! Exactly three functions contain `unsafe` (each block marked `SAFETY`):
//! 1. [`get_impl`] / [`set_impl`]: deref of the registered argument pointer and the table pointer;
//! 2. [`run_cell`]: re-derives `&mut T` and runs `original` through the cell pointer;
//! 3. the three call sites that invoke them through the stored function pointers
//!    ([`get`], [`set`], [`run_original`]).
//!
//! The invariants they rely on: the pointers come from `&'a mut` borrows that the [`Activation`]
//! guard keeps borrowed until it drops, a record is removed from the thread-local stack before
//! that borrow ends (drop, finish, unwind), and an argument is dereferenced only while `original`
//! is not running (the lending lock), on the thread that opened the activation. There is no
//! lifetime transmute and no heap allocation: the original lives in a caller-stack [`OrigCell`].

use std::cell::RefCell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

/// Arguments one activation can register.
pub const MAX_ARGS: usize = 4;

/// What a guest-facing call returns instead of trapping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum Status {
    Ok = 0,
    /// The activation has ended (or never existed on this thread).
    Stale = 1,
    /// `original` is running and the handle is lent to it.
    Locked = 2,
    /// The type has no such field, or the value does not fit.
    BadField = 3,
    /// The activation has no such argument index.
    NoSuchArg = 4,
}

/// A handle root (MX3C1 "paths from a root"). Only the hook-argument root exists in the slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Root {
    HookArg { activation: u64, arg: u8 },
}

impl Root {
    /// `activation << 8 | arg`.
    pub fn to_handle(self) -> u64 {
        match self {
            Root::HookArg { activation, arg } => activation << 8 | u64::from(arg),
        }
    }
    pub fn from_handle(handle: u64) -> Root {
        Root::HookArg { activation: handle >> 8, arg: (handle & 0xff) as u8 }
    }
}

/// Typed field access for one argument type; the engine-specific part a generator would emit.
pub struct FieldTable<T> {
    pub get: fn(&T, u32) -> Option<i64>,
    pub set: fn(&mut T, u32, i64) -> bool,
}

/// One registered argument, type-erased: the pointer, its table and two monomorphized accessors.
struct ArgSlot {
    ptr: *mut (),
    table: *const (),
    get: unsafe fn(*mut (), *const (), u32) -> Option<i64>,
    set: unsafe fn(*mut (), *const (), u32, i64) -> bool,
}

/// # Safety
/// `ptr` is a live `*mut T` with no `&mut T` in use, `table` is a `&'static FieldTable<T>`.
unsafe fn get_impl<T>(ptr: *mut (), table: *const (), field: u32) -> Option<i64> {
    // SAFETY: caller contract; the lending lock guarantees no `&mut T` is live.
    let (table, value) = unsafe { (&*(table as *const FieldTable<T>), &*(ptr as *const T)) };
    (table.get)(value, field)
}

/// # Safety
/// As [`get_impl`], and no other reference to `*ptr` is live.
unsafe fn set_impl<T>(ptr: *mut (), table: *const (), field: u32, v: i64) -> bool {
    // SAFETY: caller contract; the lending lock guarantees exclusivity.
    let (table, value) = unsafe { (&*(table as *const FieldTable<T>), &mut *(ptr as *mut T)) };
    (table.set)(value, field, v)
}

struct Record {
    serial: u64,
    args: [Option<ArgSlot>; MAX_ARGS],
    n_args: u8,
    /// The caller's [`OrigCell`] and the monomorphized function that runs it.
    cell: *mut (),
    run: unsafe fn(*mut ()) -> i64,
    /// The lending lock: true from the start of `original` to its end.
    lent: bool,
    original_calls: u32,
    last_result: i64,
}

#[derive(Default)]
struct Acts {
    stack: Vec<Record>,
}

impl Acts {
    fn find(&mut self, serial: u64) -> Option<&mut Record> {
        self.stack.iter_mut().rev().find(|record| record.serial == serial)
    }
}

static NEXT_SERIAL: AtomicU64 = AtomicU64::new(1);

thread_local! {
    static ACTS: RefCell<Acts> = RefCell::new(Acts::default());
}

/// How many activations are open on this thread (tests: "freed at the end").
pub fn live_activations() -> usize {
    ACTS.with(|acts| acts.borrow().stack.len())
}

/// What an activation did to `original`, for the trap rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Finished {
    pub original_calls: u32,
    pub last_result: i64,
}

/// The guard of one open activation. Dropping it frees the arena.
pub struct Activation<'a> {
    serial: u64,
    open: bool,
    /// Keeps the registered `&'a mut` borrows alive, and keeps the guard on its thread.
    _borrow: PhantomData<(&'a mut (), *mut ())>,
}

/// The caller-stack home of one activation's `self` and `original`. The guard borrows it, so it
/// cannot move or be used while the activation is open.
pub struct OrigCell<'b, T> {
    arg: *mut T,
    original: &'b mut dyn FnMut(&mut T) -> i64,
}

impl<'b, T> OrigCell<'b, T> {
    /// `arg` is argument 0 (`self` in the slice); `original` returns the function's result as an
    /// `i64` (0 for `()`).
    pub fn new(arg: &'b mut T, original: &'b mut dyn FnMut(&mut T) -> i64) -> Self {
        Self { arg, original }
    }
}

/// # Safety
/// `cell` is a live `*mut OrigCell<T>` and no reference to `*arg` is in use (lending lock held).
unsafe fn run_cell<T>(cell: *mut ()) -> i64 {
    // SAFETY: caller contract.
    let cell = unsafe { &mut *(cell as *mut OrigCell<'_, T>) };
    // SAFETY: `arg` came from a `&mut T` the cell owns the borrow of; the lock excludes other users.
    (cell.original)(unsafe { &mut *cell.arg })
}

impl<'a> Activation<'a> {
    /// Opens an activation with the cell's argument as argument 0 and its `original` as the
    /// callable the guest may run (any number of times).
    pub fn open<'b: 'a, T: 'static>(
        cell: &'a mut OrigCell<'b, T>,
        table: &'static FieldTable<T>,
    ) -> Self {
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        let mut args: [Option<ArgSlot>; MAX_ARGS] = [const { None }; MAX_ARGS];
        args[0] = Some(ArgSlot {
            ptr: cell.arg.cast(),
            table: (table as *const FieldTable<T>).cast(),
            get: get_impl::<T>,
            set: set_impl::<T>,
        });
        let cell_ptr: *mut OrigCell<'b, T> = cell;
        ACTS.with(|acts| {
            acts.borrow_mut().stack.push(Record {
                serial,
                args,
                n_args: 1,
                cell: cell_ptr.cast(),
                run: run_cell::<T>,
                lent: false,
                original_calls: 0,
                last_result: 0,
            });
        });
        Self { serial, open: true, _borrow: PhantomData }
    }

    /// Registers one more argument; returns its index. Panics past [`MAX_ARGS`].
    pub fn add_arg<T: 'static>(&mut self, arg: &'a mut T, table: &'static FieldTable<T>) -> u8 {
        let ptr: *mut T = arg;
        ACTS.with(|acts| {
            let mut acts = acts.borrow_mut();
            let record = acts.find(self.serial).expect("open activation");
            let index = record.n_args as usize;
            assert!(index < MAX_ARGS, "too many hook arguments");
            record.args[index] = Some(ArgSlot {
                ptr: ptr.cast(),
                table: (table as *const FieldTable<T>).cast(),
                get: get_impl::<T>,
                set: set_impl::<T>,
            });
            record.n_args += 1;
            index as u8
        })
    }

    /// The handle of argument `arg` (0 is `self` in the slice).
    pub fn handle(&self, arg: u8) -> u64 {
        Root::HookArg { activation: self.serial, arg }.to_handle()
    }

    /// Ends the activation (frees the arena) and reports what it did to `original`.
    pub fn finish(mut self) -> Finished {
        self.close()
    }

    fn close(&mut self) -> Finished {
        if !self.open {
            return Finished { original_calls: 0, last_result: 0 };
        }
        self.open = false;
        ACTS.with(|acts| {
            let mut acts = acts.borrow_mut();
            let index = acts.stack.iter().rposition(|record| record.serial == self.serial);
            match index {
                Some(index) => {
                    let record = acts.stack.remove(index);
                    Finished { original_calls: record.original_calls, last_result: record.last_result }
                }
                None => Finished { original_calls: 0, last_result: 0 },
            }
        })
    }
}

impl Drop for Activation<'_> {
    fn drop(&mut self) {
        self.close();
    }
}

fn locate<R>(handle: u64, f: impl FnOnce(&mut Record, usize) -> Result<R, Status>) -> Result<R, Status> {
    let Root::HookArg { activation, arg } = Root::from_handle(handle);
    ACTS.with(|acts| {
        let mut acts = acts.borrow_mut();
        let record = acts.find(activation).ok_or(Status::Stale)?;
        if usize::from(arg) >= usize::from(record.n_args) {
            return Err(Status::NoSuchArg);
        }
        f(record, usize::from(arg))
    })
}

/// Reads `field` of the argument behind `handle`.
pub fn get(handle: u64, field: u32) -> Result<i64, Status> {
    locate(handle, |record, arg| {
        if record.lent {
            return Err(Status::Locked);
        }
        let slot = record.args[arg].as_ref().ok_or(Status::NoSuchArg)?;
        // SAFETY: `slot` was registered by an open activation (its `&'a mut` is still borrowed),
        // and `original` is not running (`lent` is false), so no `&mut` to the value is live.
        unsafe { (slot.get)(slot.ptr, slot.table, field) }.ok_or(Status::BadField)
    })
}

/// Writes `value` to `field` of the argument behind `handle`.
pub fn set(handle: u64, field: u32, value: i64) -> Result<(), Status> {
    locate(handle, |record, arg| {
        if record.lent {
            return Err(Status::Locked);
        }
        let slot = record.args[arg].as_ref().ok_or(Status::NoSuchArg)?;
        // SAFETY: as in `get`; the arena borrow (`RefCell`) also excludes a concurrent accessor.
        if unsafe { (slot.set)(slot.ptr, slot.table, field, value) } {
            Ok(())
        } else {
            Err(Status::BadField)
        }
    })
}

/// Runs the activation's `original`, with its handles lent for the duration. Returns the result
/// the engine body returned. The arena is not borrowed while `original` runs, so it may open and
/// close nested activations (re-entrancy).
pub fn run_original(handle: u64) -> Result<i64, Status> {
    let Root::HookArg { activation, .. } = Root::from_handle(handle);
    let (cell, run) = locate(handle, |record, _| {
        if record.lent {
            return Err(Status::Locked);
        }
        record.lent = true;
        Ok((record.cell, record.run))
    })?;
    // SAFETY: `cell`/`run` were registered together by `open` and the guard keeps the cell borrowed
    // and alive until the record is removed; the lock is held (`lent`), so nothing else uses the
    // argument, and the arena is not borrowed here so nested activations may open and close.
    // If `original` unwinds, `lent` stays set until the guard drops and removes the record.
    let result = unsafe { run(cell) };
    ACTS.with(|acts| {
        if let Some(record) = acts.borrow_mut().find(activation) {
            record.lent = false;
            record.original_calls += 1;
            record.last_result = result;
        }
    });
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn handle_roundtrip() {
        let root = Root::HookArg { activation: 123_456_789, arg: 3 };
        assert_eq!(Root::from_handle(root.to_handle()), root);
    }

    #[test]
    fn get_set_through_the_handle_and_the_owner_sees_it() {
        let mut thing = Thing::default();
        let mut orig = |_: &mut Thing| 0;
        let mut cell = OrigCell::new(&mut thing, &mut orig);
        let act = Activation::open(&mut cell, &THING);
        let h = act.handle(0);
        assert_eq!(set(h, 0, 41), Ok(()));
        assert_eq!(set(h, 1, 2.5f64.to_bits() as i64), Ok(()));
        assert_eq!(get(h, 0), Ok(41));
        assert_eq!(get(h, 7), Err(Status::BadField));
        assert_eq!(set(h, 7, 0), Err(Status::BadField));
        drop(act);
        assert_eq!(thing, Thing { count: 41, x: 2.5 });
    }

    #[test]
    fn handles_go_stale_and_are_never_reused() {
        let mut thing = Thing::default();
        let mut orig = |_: &mut Thing| 0;
        let first = {
            let mut cell = OrigCell::new(&mut thing, &mut orig);
            let act = Activation::open(&mut cell, &THING);
            let h = act.handle(0);
            assert_eq!(live_activations(), 1);
            act.finish();
            h
        };
        assert_eq!(live_activations(), 0);
        assert_eq!(get(first, 0), Err(Status::Stale));
        assert_eq!(set(first, 0, 1), Err(Status::Stale));
        let mut cell = OrigCell::new(&mut thing, &mut orig);
        let act = Activation::open(&mut cell, &THING);
        assert_ne!(act.handle(0), first);
        assert_eq!(get(first, 0), Err(Status::Stale));
        assert_eq!(get(act.handle(0) | 1, 0), Err(Status::NoSuchArg));
    }

    #[test]
    fn lending_lock_rejects_while_original_runs_and_unlocks_after() {
        let mut thing = Thing::default();
        let seen = std::cell::RefCell::new(Vec::new());
        let handle = std::cell::Cell::new(0u64);
        let mut orig = |t: &mut Thing| {
            // The guest-side attempt: the handle is lent to `original`.
            seen.borrow_mut().push(get(handle.get(), 0));
            seen.borrow_mut().push(set(handle.get(), 0, 99).map(|()| 0));
            // A second run of the same original from inside it is refused too.
            seen.borrow_mut().push(run_original(handle.get()));
            t.count += 1;
            7
        };
        let mut cell = OrigCell::new(&mut thing, &mut orig);
        let act = Activation::open(&mut cell, &THING);
        handle.set(act.handle(0));
        assert_eq!(run_original(handle.get()), Ok(7));
        assert_eq!(*seen.borrow(), vec![Err(Status::Locked); 3]);
        // After original the handle works again, and sees the engine's change, not the rejected write.
        assert_eq!(get(handle.get(), 0), Ok(1));
        assert_eq!(set(handle.get(), 0, 10), Ok(()));
        // original may run again; the record counts both runs and the last result.
        assert_eq!(run_original(handle.get()), Ok(7));
        let fin = act.finish();
        assert_eq!(fin, Finished { original_calls: 2, last_result: 7 });
        assert_eq!(thing.count, 11);
    }

    #[test]
    fn nested_activation_cannot_use_the_lent_outer_handle() {
        let mut outer = Thing::default();
        let mut inner = Thing::default();
        let outer_handle = std::cell::Cell::new(0u64);
        let seen = std::cell::RefCell::new(Vec::new());
        let mut orig_outer = |_: &mut Thing| {
            let mut inner_orig = |_: &mut Thing| 0;
            let mut cell = OrigCell::new(&mut inner, &mut inner_orig);
            let act = Activation::open(&mut cell, &THING);
            let h = act.handle(0);
            seen.borrow_mut().push(get(outer_handle.get(), 0)); // outer is lent: Locked
            seen.borrow_mut().push(get(h, 0)); // own handle: fine
            assert_eq!(live_activations(), 2);
            0
        };
        let mut cell = OrigCell::new(&mut outer, &mut orig_outer);
        let act = Activation::open(&mut cell, &THING);
        outer_handle.set(act.handle(0));
        run_original(outer_handle.get()).unwrap();
        assert_eq!(*seen.borrow(), vec![Err(Status::Locked), Ok(0)]);
        drop(act);
        assert_eq!(live_activations(), 0);
    }

    #[test]
    fn arena_is_freed_on_panic() {
        let mut thing = Thing::default();
        let mut orig = |_: &mut Thing| 0;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut cell = OrigCell::new(&mut thing, &mut orig);
            let _act = Activation::open(&mut cell, &THING);
            assert_eq!(live_activations(), 1);
            panic!("shim bug");
        }));
        assert!(result.is_err());
        assert_eq!(live_activations(), 0);
    }

    #[test]
    fn panic_inside_original_still_frees_the_arena() {
        let mut thing = Thing::default();
        let mut orig = |_: &mut Thing| -> i64 { panic!("engine body panics") };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut cell = OrigCell::new(&mut thing, &mut orig);
            let act = Activation::open(&mut cell, &THING);
            let _ = run_original(act.handle(0));
        }));
        assert!(result.is_err());
        assert_eq!(live_activations(), 0);
    }

    #[test]
    fn two_arguments_have_two_handles() {
        #[derive(Default)]
        struct Other(i32);
        static OTHER: FieldTable<Other> = FieldTable {
            get: |o, f| (f == 0).then(|| i64::from(o.0)),
            set: |o, f, v| {
                if f == 0 {
                    o.0 = v as i32;
                }
                f == 0
            },
        };
        let mut a = Thing::default();
        let mut b = Other::default();
        let mut orig = |_: &mut Thing| 0;
        let mut cell = OrigCell::new(&mut a, &mut orig);
        let mut act = Activation::open(&mut cell, &THING);
        let index = act.add_arg(&mut b, &OTHER);
        assert_eq!(index, 1);
        assert_eq!(set(act.handle(0), 0, 3), Ok(()));
        assert_eq!(set(act.handle(1), 0, 4), Ok(()));
        assert_eq!(get(act.handle(2), 0), Err(Status::NoSuchArg));
        drop(act);
        assert_eq!((a.count, b.0), (3, 4));
    }

    #[test]
    fn other_thread_sees_nothing() {
        let mut thing = Thing::default();
        let mut orig = |_: &mut Thing| 0;
        let mut cell = OrigCell::new(&mut thing, &mut orig);
        let act = Activation::open(&mut cell, &THING);
        let h = act.handle(0);
        std::thread::spawn(move || {
            assert_eq!(get(h, 0), Err(Status::Stale));
            assert_eq!(live_activations(), 0);
        })
        .join()
        .unwrap();
    }
}
