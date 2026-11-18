//! Hook activations: the per-activation arena of handles.
//!
//! An *activation* is one run of a hook shim: the engine called a hooked function, the shim
//! registered the function's `&mut` arguments and its `original` callable here, and a guest may
//! now reach them through 64-bit handles until the activation ends.
//!
//! * The only public way to open one is [`scope`] / [`scope2`]: the activation lives exactly as long
//!   as the closure you pass, the guard is private, and the arguments stay borrowed by the call.
//!   There is no guard value to `mem::forget`, so a handle can never outlive the borrows behind it
//!   (a handle used after the scope is `Status::Stale`).
//! * `Root::HookArg { activation, arg }` is the handle root for a hook argument. The caller of the
//!   hooked function holds an outer `&mut` to it, so it can never be re-borrowed from the `Game`
//!   root; the activation stores the pointer the shim was handed and re-derives the reference on
//!   every access. The handle of argument `i` is the handle of argument 0 plus `i`.
//! * Handles are never reused: `activation` is a process-wide 56-bit serial.
//! * The lending lock: while `original` runs, or while a field accessor runs, every handle of that
//!   activation is *lent* and a guest access through it returns `Status::Locked`; no second `&mut`
//!   is ever made. The arena (`RefCell`) is never borrowed while `original` or an accessor runs, so
//!   both may open nested activations (re-entrancy, accessors that call hooked engine functions).
//! * `original` re-receives all the lent arguments (`FnMut(&mut A)` / `FnMut(&mut A, &mut B)`), so
//!   shims and generated code hold no raw pointers.
//!
//! # Unsafe surface
//!
//! Exactly these functions contain `unsafe` (each block marked `SAFETY`): `get_impl`, `set_impl`,
//! `run_cell`, `run_cell2`, `with_arg` (MX3G1: typed receiver for generated glue, `TypeId`-checked), `resolve_impl`, `open_path` and `with_path` (MX3R1: path handles; each calls a root's resolver while the activation is locked)
//! and the three call sites that invoke them through the stored function pointers ([`get`], [`set`],
//! [`run_original`]).
//!
//! Invariants: the pointers come from `&mut` borrows that [`scope`] holds for its whole call; the
//! record is removed from the thread-local stack by the private guard's `Drop` before `scope`
//! returns or unwinds; an argument is dereferenced only while its activation is locked, on the
//! thread that opened it. No lifetime transmute, no heap allocation in the activation itself.
//!
//! The guard is not public, so the old hazard (`mem::forget` of a guard, then a handle read of a
//! dead stack) cannot be written:
//!
//! ```compile_fail
//! // `Activation` and `Activation::open` no longer exist; the only entry points are `scope`/`scope2`.
//! use yoake_slice::activation::Activation;
//! ```

use std::any::{Any, TypeId};
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
    /// `original` or an accessor is running and the handle is lent.
    Locked = 2,
    /// The type has no such field, or the value does not fit.
    BadField = 3,
    /// The activation has no such argument index.
    NoSuchArg = 4,
    /// [`with_arg`]: the argument behind the handle is not a `T` (MX3G1).
    WrongType = 5,
    /// A path handle could not be made: this activation already holds [`MAX_PATHS_PER_ACTIVATION`].
    LimitReached = 6,
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

/// One step of an accessor path from a root (MX3R1, design 4b: "handles are paths from a root").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Accessor {
    /// A field of the current object, by the generator's index (`Game.dropped` is `Field(n)`).
    Field(u32),
    /// The element of a list whose id field equals this (an entity by id). The only way into a list:
    /// a position (`Index`) would silently name another element after a removal, so there is none.
    Id(u64),
}

/// The engine side of a root (`Game`): finds the object at a path and lends it to `f` as `&mut dyn Any`.
/// Generated code implements it with a `match` over the paths it knows; an object that is gone is
/// `Err(Status::Stale)`, a path it does not know `Err(Status::BadField)`. `f` runs at most once.
pub trait PathRoot {
    fn resolve(&mut self, path: &[Accessor], f: &mut dyn FnMut(&mut dyn Any)) -> Result<(), Status>;
}

type ResolveFn = unsafe fn(*mut (), &[Accessor], &mut dyn FnMut(&mut dyn Any)) -> Result<(), Status>;

/// Marks the argument slot of a path root; no user type can be this.
struct RootMarker;

/// Typed field access for one argument type; the engine-specific part a generator would emit.
/// Both take `&mut T`: an accessor may call hooked engine functions on its receiver.
pub struct FieldTable<T> {
    pub get: fn(&mut T, u32) -> Option<i64>,
    pub set: fn(&mut T, u32, i64) -> bool,
}

/// One registered argument, type-erased: the pointer, its table and two monomorphized accessors.
#[derive(Clone, Copy)]
struct ArgSlot {
    /// `TypeId` of `T`, so [`with_arg`] can refuse a handle that points at another type.
    type_id: TypeId,
    ptr: *mut (),
    table: *const (),
    get: unsafe fn(*mut (), *const (), u32) -> Option<i64>,
    set: unsafe fn(*mut (), *const (), u32, i64) -> bool,
    /// Present for a path root (`scope_root`).
    resolve: Option<ResolveFn>,
}

impl ArgSlot {
    fn new<T: 'static>(ptr: *mut T, table: &'static FieldTable<T>) -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            ptr: ptr.cast(),
            table: (table as *const FieldTable<T>).cast(),
            get: get_impl::<T>,
            set: set_impl::<T>,
            resolve: None,
        }
    }

    /// A path root: no scalar fields, only path resolution. `G` may carry a lifetime (`Game<'a>`): the
    /// type is erased to a pointer and a function pointer whose signature has no lifetime in it.
    fn new_root<G: PathRoot>(ptr: *mut G) -> Self {
        Self {
            type_id: TypeId::of::<RootMarker>(),
            ptr: ptr.cast(),
            table: std::ptr::null(),
            get: |_, _, _| None,
            set: |_, _, _, _| false,
            resolve: Some(resolve_impl::<G>),
        }
    }
}

/// # Safety
/// `ptr` is a live `*mut G` with no other reference in use (the activation is locked for the call).
unsafe fn resolve_impl<G: PathRoot>(ptr: *mut (), path: &[Accessor], f: &mut dyn FnMut(&mut dyn Any)) -> Result<(), Status> {
    // SAFETY: caller contract.
    let root = unsafe { &mut *(ptr as *mut G) };
    root.resolve(path, f)
}

/// # Safety
/// `ptr` is a live `*mut T` with no other reference in use, `table` is a `&'static FieldTable<T>`.
unsafe fn get_impl<T>(ptr: *mut (), table: *const (), field: u32) -> Option<i64> {
    // SAFETY: caller contract; the activation is locked for the call, so nothing else uses `*ptr`.
    let (table, value) = unsafe { (&*(table as *const FieldTable<T>), &mut *(ptr as *mut T)) };
    (table.get)(value, field)
}

/// # Safety
/// As [`get_impl`].
unsafe fn set_impl<T>(ptr: *mut (), table: *const (), field: u32, v: i64) -> bool {
    // SAFETY: caller contract.
    let (table, value) = unsafe { (&*(table as *const FieldTable<T>), &mut *(ptr as *mut T)) };
    (table.set)(value, field, v)
}

struct Record {
    serial: u64,
    args: [Option<ArgSlot>; MAX_ARGS],
    n_args: u8,
    /// The caller's cell (arguments + `original`) and the monomorphized function that runs it.
    cell: *mut (),
    run: unsafe fn(*mut ()) -> i64,
    /// The lending lock: true while `original` or an accessor runs.
    lent: bool,
    original_calls: u32,
    last_result: i64,
    /// Path handles this activation opened: the bound, and the reason `close` can skip the table scan.
    paths_opened: u32,
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

/// Most path handles one activation may hold.
pub const MAX_PATHS_PER_ACTIVATION: usize = 256;

/// Path handles have this bit set (hook-argument handles never do: their activation serial is 56 bits).
const PATH_BIT: u64 = 1 << 63;

struct LivePath {
    activation: u64,
    path: Vec<Accessor>,
}

#[derive(Default)]
struct PathEntry {
    generation: u32,
    live: Option<LivePath>,
}

#[derive(Default)]
struct PathTable {
    entries: Vec<PathEntry>,
    free: Vec<usize>,
}

impl PathTable {
    fn bits(slot: usize, generation: u32) -> u64 {
        PATH_BIT | (u64::from(generation & 0x7fff_ffff) << 32) | slot as u64
    }

    fn lookup(&self, handle: u64) -> Option<(u64, Vec<Accessor>)> {
        let slot = (handle & 0xffff_ffff) as usize;
        let generation = ((handle >> 32) & 0x7fff_ffff) as u32;
        let entry = self.entries.get(slot)?;
        if entry.generation & 0x7fff_ffff != generation {
            return None;
        }
        entry.live.as_ref().map(|l| (l.activation, l.path.clone()))
    }

    /// Frees every handle of an activation; the slots' generations move on, so old handles are `Stale`.
    fn release_activation(&mut self, activation: u64) {
        for (slot, entry) in self.entries.iter_mut().enumerate() {
            if entry.live.as_ref().is_some_and(|l| l.activation == activation) {
                entry.live = None;
                entry.generation = entry.generation.wrapping_add(1);
                self.free.push(slot);
            }
        }
    }
}

thread_local! {
    static PATHS: RefCell<PathTable> = RefCell::new(PathTable::default());
}

/// How many path handles are alive on this thread (tests: "freed at the end").
pub fn live_path_handles() -> usize {
    PATHS.with(|p| p.borrow().entries.iter().filter(|e| e.live.is_some()).count())
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

/// The private guard of one open activation. Dropping it frees the arena. Never handed out.
struct Guard<'a> {
    serial: u64,
    open: bool,
    /// Keeps the borrows of `scope` alive, and keeps the guard on its thread.
    _borrow: PhantomData<(&'a mut (), *mut ())>,
}

impl Guard<'_> {
    fn open(
        args: [Option<ArgSlot>; MAX_ARGS],
        n_args: u8,
        cell: *mut (),
        run: unsafe fn(*mut ()) -> i64,
    ) -> Self {
        let serial = NEXT_SERIAL.fetch_add(1, Ordering::Relaxed);
        ACTS.with(|acts| {
            acts.borrow_mut().stack.push(Record {
                serial,
                args,
                n_args,
                cell,
                run,
                lent: false,
                original_calls: 0,
                last_result: 0,
                paths_opened: 0,
            });
        });
        Guard { serial, open: true, _borrow: PhantomData }
    }

    fn handle(&self) -> u64 {
        Root::HookArg { activation: self.serial, arg: 0 }.to_handle()
    }

    fn close(&mut self) -> Finished {
        if !self.open {
            return Finished { original_calls: 0, last_result: 0 };
        }
        self.open = false;
        ACTS.with(|acts| {
            let mut acts = acts.borrow_mut();
            match acts.stack.iter().rposition(|record| record.serial == self.serial) {
                Some(index) => {
                    let record = acts.stack.remove(index);
                    // the common activation opened no path handle: no table scan on the hot path
                    if record.paths_opened > 0 {
                        PATHS.with(|p| {
                            if let Ok(mut paths) = p.try_borrow_mut() {
                                paths.release_activation(self.serial);
                            }
                        });
                    }
                    Finished { original_calls: record.original_calls, last_result: record.last_result }
                }
                None => Finished { original_calls: 0, last_result: 0 },
            }
        })
    }
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.close();
    }
}

struct Cell1<'b, A> {
    a: *mut A,
    original: &'b mut dyn FnMut(&mut A) -> i64,
}

struct Cell2<'b, A, B> {
    a: *mut A,
    b: *mut B,
    original: &'b mut dyn FnMut(&mut A, &mut B) -> i64,
}

/// # Safety
/// `cell` is a live `*mut Cell1<A>` and no reference to `*a` is in use (lending lock held).
unsafe fn run_cell<A>(cell: *mut ()) -> i64 {
    // SAFETY: caller contract.
    let cell = unsafe { &mut *(cell as *mut Cell1<'_, A>) };
    // SAFETY: `a` came from a `&mut A` that `scope` holds borrowed; the lock excludes other users.
    (cell.original)(unsafe { &mut *cell.a })
}

/// # Safety
/// As [`run_cell`], for both arguments.
unsafe fn run_cell2<A, B>(cell: *mut ()) -> i64 {
    // SAFETY: caller contract.
    let cell = unsafe { &mut *(cell as *mut Cell2<'_, A, B>) };
    // SAFETY: as in `run_cell`; `a` and `b` are distinct borrows held by `scope2`.
    (cell.original)(unsafe { &mut *cell.a }, unsafe { &mut *cell.b })
}

/// Runs `body` inside one activation whose argument 0 is `arg` and whose `original` is the
/// callable the guest may run (any number of times). `body` gets the handle of argument 0. When
/// `body` returns (or unwinds) the arena is freed and every handle is `Stale`.
pub fn scope<A: 'static, R>(
    arg: &mut A,
    table: &'static FieldTable<A>,
    original: &mut dyn FnMut(&mut A) -> i64,
    body: impl FnOnce(u64) -> R,
) -> (R, Finished) {
    let a: *mut A = arg;
    let mut cell = Cell1 { a, original };
    let mut args: [Option<ArgSlot>; MAX_ARGS] = [None; MAX_ARGS];
    args[0] = Some(ArgSlot::new(a, table));
    // Declared after `cell`, so dropped (record removed) before the cell goes away.
    let mut guard = Guard::open(args, 1, (&mut cell as *mut Cell1<'_, A>).cast(), run_cell::<A>);
    let result = body(guard.handle());
    (result, guard.close())
}

/// [`scope`] with two `&mut` arguments. `body` gets the handle of argument 0; argument 1 is that
/// handle + 1. `original` re-receives both.
pub fn scope2<A: 'static, B: 'static, R>(
    a: &mut A,
    table_a: &'static FieldTable<A>,
    b: &mut B,
    table_b: &'static FieldTable<B>,
    original: &mut dyn FnMut(&mut A, &mut B) -> i64,
    body: impl FnOnce(u64) -> R,
) -> (R, Finished) {
    let (pa, pb): (*mut A, *mut B) = (a, b);
    let mut cell = Cell2 { a: pa, b: pb, original };
    let mut args: [Option<ArgSlot>; MAX_ARGS] = [None; MAX_ARGS];
    args[0] = Some(ArgSlot::new(pa, table_a));
    args[1] = Some(ArgSlot::new(pb, table_b));
    let mut guard = Guard::open(args, 2, (&mut cell as *mut Cell2<'_, A, B>).cast(), run_cell2::<A, B>);
    let result = body(guard.handle());
    (result, guard.close())
}

/// [`scope`] for a root that guests reach objects through (`Game`): argument 0 is the root, its handle is
/// the root handle, and [`open_path`] makes path handles that live exactly as long as this call. The
/// root is borrowed for the whole call; a path handle re-borrows from it on every use.
pub fn scope_root<G: PathRoot, R>(root: &mut G, original: &mut dyn FnMut(&mut G) -> i64, body: impl FnOnce(u64) -> R) -> (R, Finished) {
    let a: *mut G = root;
    let mut cell = Cell1 { a, original };
    let mut args: [Option<ArgSlot>; MAX_ARGS] = [None; MAX_ARGS];
    args[0] = Some(ArgSlot::new_root(a));
    let mut guard = Guard::open(args, 1, (&mut cell as *mut Cell1<'_, G>).cast(), run_cell::<G>);
    let result = body(guard.handle());
    (result, guard.close())
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

/// Clears the lock when an accessor or `original` ends, also when it unwinds.
struct Unlend {
    serial: u64,
    /// Set after `original` returns; the call itself is counted before the body runs.
    result: Option<i64>,
}

impl Drop for Unlend {
    fn drop(&mut self) {
        // `try_borrow_mut`: never panic in a destructor.
        ACTS.with(|acts| {
            if let Ok(mut acts) = acts.try_borrow_mut() {
                if let Some(record) = acts.find(self.serial) {
                    record.lent = false;
                    if let Some(result) = self.result {
                        record.last_result = result;
                    }
                }
            }
        });
    }
}

/// Locks the activation and returns the argument's slot, so it can be used with the arena unborrowed.
fn lend_arg(handle: u64) -> Result<(ArgSlot, Unlend), Status> {
    if handle & PATH_BIT != 0 {
        return Err(Status::WrongType);
    }
    let Root::HookArg { activation, .. } = Root::from_handle(handle);
    let slot = locate(handle, |record, arg| {
        if record.lent {
            return Err(Status::Locked);
        }
        let slot = record.args[arg].ok_or(Status::NoSuchArg)?;
        record.lent = true;
        Ok(slot)
    })?;
    Ok((slot, Unlend { serial: activation, result: None }))
}

/// Reads `field` of the argument behind `handle`. The accessor runs with the arena unborrowed and
/// the activation locked, so it may call hooked engine functions.
pub fn get(handle: u64, field: u32) -> Result<i64, Status> {
    let (slot, _unlend) = lend_arg(handle)?;
    // SAFETY: `slot` was registered by an open `scope` (its borrows are held for the whole call and
    // `locate` just found the record) and the activation stays locked until `_unlend` drops.
    unsafe { (slot.get)(slot.ptr, slot.table, field) }.ok_or(Status::BadField)
}

/// Writes `value` to `field` of the argument behind `handle` (same rules as [`get`]).
pub fn set(handle: u64, field: u32, value: i64) -> Result<(), Status> {
    let (slot, _unlend) = lend_arg(handle)?;
    // SAFETY: as in `get`.
    if unsafe { (slot.set)(slot.ptr, slot.table, field, value) } {
        Ok(())
    } else {
        Err(Status::BadField)
    }
}

/// Runs `f` on the argument behind `handle`, which must be a `T` (MX3G1: the receiver of generated
/// glue for `&self` / `&mut self` methods). Same rules as [`get`]: the activation is locked while `f`
/// runs (so `f` may open nested activations but a second access through any handle of this
/// activation is `Locked`), and the lock is released when `f` returns or unwinds. A handle to
/// another type is `Status::WrongType` and `f` does not run.
pub fn with_arg<T: 'static, R>(handle: u64, f: impl FnOnce(&mut T) -> R) -> Result<R, Status> {
    if handle & PATH_BIT != 0 {
        return with_path(handle, f);
    }
    let (slot, _unlend) = lend_arg(handle)?;
    if slot.type_id != TypeId::of::<T>() {
        return Err(Status::WrongType);
    }
    // SAFETY: `slot.ptr` was registered by an open `scope` as a `*mut T` (the `TypeId` just
    // checked says `T` is that type), its borrow is held for the whole `scope` call and `lend_arg`
    // just found the record; the activation stays locked until `_unlend` drops, so no other
    // reference to `*ptr` is in use.
    Ok(f(unsafe { &mut *slot.ptr.cast::<T>() }))
}

/// Makes a path handle: `path` from the root behind `root_handle` (the handle [`scope_root`] gave the
/// body). The object must exist now (else `Stale`/`BadField`, from the root's resolver). The handle is
/// valid only inside this activation and is `Stale` after it, and it re-borrows from the root on every
/// use, so it never keeps a reference alive. `Locked` while `original` or another call runs.
pub fn open_path(root_handle: u64, path: Vec<Accessor>) -> Result<u64, Status> {
    if root_handle & PATH_BIT != 0 {
        return Err(Status::WrongType);
    }
    let Root::HookArg { activation, .. } = Root::from_handle(root_handle);
    {
        let (slot, _unlend) = lend_arg(root_handle)?;
        let resolve = slot.resolve.ok_or(Status::WrongType)?;
        let mut found = false;
        // SAFETY: `slot` was registered by an open `scope_root` whose borrow is held for the whole
        // call, `lend_arg` just found its record and locked the activation until `_unlend` drops.
        unsafe { resolve(slot.ptr, &path, &mut |_| found = true) }?;
        if !found {
            return Err(Status::Stale);
        }
    }
    // counted on the activation's record (no scan of the table), and before the entry exists
    locate(root_handle, |record, _| {
        if record.paths_opened as usize >= MAX_PATHS_PER_ACTIVATION {
            return Err(Status::LimitReached);
        }
        record.paths_opened += 1;
        Ok(())
    })?;
    PATHS.with(|paths| {
        let mut paths = paths.borrow_mut();
        let live = LivePath { activation, path };
        let slot = match paths.free.pop() {
            Some(slot) => slot,
            None => {
                paths.entries.push(PathEntry::default());
                paths.entries.len() - 1
            }
        };
        paths.entries[slot].live = Some(live);
        Ok(PathTable::bits(slot, paths.entries[slot].generation))
    })
}

/// Runs `f` on the object a path handle names, re-resolved from the root now. `Stale`: the handle's
/// activation ended, or the object is gone (removed since the handle was made). `WrongType`: it is not
/// a `T`. `Locked`: `original` or another call on this activation is running.
fn with_path<T: 'static, R>(handle: u64, f: impl FnOnce(&mut T) -> R) -> Result<R, Status> {
    let (activation, path) = PATHS.with(|p| p.borrow().lookup(handle)).ok_or(Status::Stale)?;
    let root_handle = Root::HookArg { activation, arg: 0 }.to_handle();
    let (slot, _unlend) = lend_arg(root_handle)?;
    let resolve = slot.resolve.ok_or(Status::WrongType)?;
    let mut f = Some(f);
    let mut result: Option<R> = None;
    // SAFETY: as in `open_path`: the root is borrowed by the open `scope_root`, the activation is locked
    // until `_unlend` drops, so the root is not otherwise in use. The resolver lends its object to the
    // closure only for the closure's duration.
    unsafe {
        resolve(slot.ptr, &path, &mut |any| {
            if let (Some(t), Some(f)) = (any.downcast_mut::<T>(), f.take()) {
                result = Some(f(t));
            }
        })
    }?;
    result.ok_or(Status::WrongType)
}

/// Runs the activation's `original`, with its handles lent for the duration. Returns the result
/// the engine body returned. The arena is not borrowed while `original` runs, so it may open and
/// close nested activations (re-entrancy).
pub fn run_original(handle: u64) -> Result<i64, Status> {
    if handle & PATH_BIT != 0 {
        return Err(Status::WrongType);
    }
    let Root::HookArg { activation, .. } = Root::from_handle(handle);
    let (cell, run) = locate(handle, |record, _| {
        if record.lent {
            return Err(Status::Locked);
        }
        record.lent = true;
        // Counted before the body runs: an `original` that panics still counts as having run.
        record.original_calls += 1;
        Ok((record.cell, record.run))
    })?;
    let mut unlend = Unlend { serial: activation, result: None };
    // SAFETY: `cell`/`run` were registered together by `scope`, whose stack frame (owning the cell
    // and the argument borrows) is below us; the lock is held, so nothing else uses the arguments,
    // and the arena is not borrowed so nested activations may open and close. If `original`
    // unwinds, `unlend` clears the lock and the guard of `scope` removes the record.
    let result = unsafe { run(cell) };
    unlend.result = Some(result);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

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

    fn nop(_: &mut Thing) -> i64 {
        0
    }

    #[test]
    fn with_arg_gives_the_typed_receiver_and_refuses_another_type() {
        let mut thing = Thing { count: 3, x: 1.5 };
        scope(&mut thing, &THING, &mut nop, |h| {
            assert_eq!(with_arg::<Thing, _>(h, |t| { t.count += 4; t.count }), Ok(7));
            assert_eq!(with_arg::<String, _>(h, |_| ()), Err(Status::WrongType));
            // the lock is released after both calls
            assert_eq!(get(h, 0), Ok(7));
            // a nested call from inside `f` through the same activation is Locked
            let inner = with_arg::<Thing, _>(h, |_| with_arg::<Thing, _>(h, |_| ()));
            assert_eq!(inner, Ok(Err(Status::Locked)));
            assert_eq!(with_arg::<Thing, _>(h | 5, |_| ()), Err(Status::NoSuchArg));
        });
        assert_eq!(thing.count, 7);
    }

    #[test]
    fn handle_roundtrip() {
        let root = Root::HookArg { activation: 123_456_789, arg: 3 };
        assert_eq!(Root::from_handle(root.to_handle()), root);
    }

    #[test]
    fn get_set_through_the_handle_and_the_owner_sees_it() {
        let mut thing = Thing::default();
        scope(&mut thing, &THING, &mut nop, |h| {
            assert_eq!(set(h, 0, 41), Ok(()));
            assert_eq!(set(h, 1, 2.5f64.to_bits() as i64), Ok(()));
            assert_eq!(get(h, 0), Ok(41));
            assert_eq!(get(h, 7), Err(Status::BadField));
            assert_eq!(set(h, 7, 0), Err(Status::BadField));
        });
        assert_eq!(thing, Thing { count: 41, x: 2.5 });
    }

    #[test]
    fn handles_go_stale_and_are_never_reused() {
        let mut thing = Thing::default();
        let (first, _) = scope(&mut thing, &THING, &mut nop, |h| {
            assert_eq!(live_activations(), 1);
            h
        });
        assert_eq!(live_activations(), 0);
        assert_eq!(get(first, 0), Err(Status::Stale));
        assert_eq!(set(first, 0, 1), Err(Status::Stale));
        scope(&mut thing, &THING, &mut nop, |h| {
            assert_ne!(h, first);
            assert_eq!(get(first, 0), Err(Status::Stale));
            assert_eq!(get(h | 1, 0), Err(Status::NoSuchArg));
        });
    }

    /// The old hazard: forgetting the guard and then using the handle. There is no guard to forget;
    /// forgetting what the body does have (the handle) changes nothing, and the arena is freed.
    #[test]
    fn forgetting_cannot_keep_an_activation_alive() {
        let mut thing = Thing::default();
        let (h, _) = scope(&mut thing, &THING, &mut nop, |h| {
            std::mem::forget(Box::new(h));
            h
        });
        assert_eq!(live_activations(), 0);
        assert_eq!(get(h, 0), Err(Status::Stale));
    }

    #[test]
    fn lending_lock_rejects_while_original_runs_and_unlocks_after() {
        let mut thing = Thing::default();
        let seen = RefCell::new(Vec::new());
        let handle = Cell::new(0u64);
        let mut orig = |t: &mut Thing| {
            seen.borrow_mut().push(get(handle.get(), 0));
            seen.borrow_mut().push(set(handle.get(), 0, 99).map(|()| 0));
            seen.borrow_mut().push(run_original(handle.get()));
            t.count += 1;
            7
        };
        let (_, fin) = scope(&mut thing, &THING, &mut orig, |h| {
            handle.set(h);
            assert_eq!(run_original(h), Ok(7));
            assert_eq!(*seen.borrow(), vec![Err(Status::Locked); 3]);
            assert_eq!(get(h, 0), Ok(1));
            assert_eq!(set(h, 0, 10), Ok(()));
            assert_eq!(run_original(h), Ok(7));
        });
        assert_eq!(fin, Finished { original_calls: 2, last_result: 7 });
        assert_eq!(thing.count, 11);
    }

    #[test]
    fn nested_activation_cannot_use_the_lent_outer_handle() {
        let mut outer = Thing::default();
        let mut inner = Thing::default();
        let outer_handle = Cell::new(0u64);
        let seen = RefCell::new(Vec::new());
        let mut orig_outer = |_: &mut Thing| {
            scope(&mut inner, &THING, &mut nop, |h| {
                seen.borrow_mut().push(get(outer_handle.get(), 0));
                seen.borrow_mut().push(get(h, 0));
                assert_eq!(live_activations(), 2);
            });
            0
        };
        scope(&mut outer, &THING, &mut orig_outer, |h| {
            outer_handle.set(h);
            run_original(h).unwrap();
        });
        assert_eq!(*seen.borrow(), vec![Err(Status::Locked), Ok(0)]);
        assert_eq!(live_activations(), 0);
    }

    #[test]
    fn arena_is_freed_on_panic() {
        let mut thing = Thing::default();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scope(&mut thing, &THING, &mut nop, |_| {
                assert_eq!(live_activations(), 1);
                panic!("shim bug");
            });
        }));
        assert!(result.is_err());
        assert_eq!(live_activations(), 0);
    }

    #[test]
    fn panic_inside_original_frees_the_arena_and_the_lock() {
        let mut thing = Thing::default();
        let mut orig = |_: &mut Thing| -> i64 { panic!("engine body panics") };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scope(&mut thing, &THING, &mut orig, |h| {
                let _ = run_original(h);
            });
        }));
        assert!(result.is_err());
        assert_eq!(live_activations(), 0);
    }

    #[derive(Default, Debug, PartialEq)]
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

    #[test]
    fn a_panicking_original_still_counts_as_run() {
        let mut thing = Thing::default();
        let mut orig = |_: &mut Thing| -> i64 { panic!("engine body panics") };
        let (_, fin) = scope(&mut thing, &THING, &mut orig, |h| {
            let caught = std::panic::catch_unwind(|| run_original(h));
            assert!(caught.is_err());
            // the lock was released by the unwind, so the handle works again
            assert_eq!(get(h, 0), Ok(0));
        });
        assert_eq!(fin.original_calls, 1);
    }

    #[test]
    fn two_arguments_have_two_handles_and_original_receives_both() {
        let mut a = Thing::default();
        let mut b = Other::default();
        let mut orig = |a: &mut Thing, b: &mut Other| {
            a.count += 1;
            b.0 += 10;
            5
        };
        let (_, fin) = scope2(&mut a, &THING, &mut b, &OTHER, &mut orig, |h| {
            assert_eq!(set(h, 0, 3), Ok(()));
            assert_eq!(set(h + 1, 0, 4), Ok(()));
            assert_eq!(get(h + 2, 0), Err(Status::NoSuchArg));
            assert_eq!(run_original(h), Ok(5));
            // original lent both: either handle is locked while it ran, free again after
            assert_eq!(get(h, 0), Ok(4));
            assert_eq!(get(h + 1, 0), Ok(14));
        });
        assert_eq!(fin.original_calls, 1);
        assert_eq!((a.count, b.0), (4, 14));
    }

    #[test]
    fn both_arguments_are_locked_while_original_runs() {
        let mut a = Thing::default();
        let mut b = Other::default();
        let h0 = Cell::new(0u64);
        let seen = RefCell::new(Vec::new());
        let mut orig = |_: &mut Thing, _: &mut Other| {
            seen.borrow_mut().push(get(h0.get(), 0));
            seen.borrow_mut().push(get(h0.get() + 1, 0));
            0
        };
        scope2(&mut a, &THING, &mut b, &OTHER, &mut orig, |h| {
            h0.set(h);
            run_original(h).unwrap();
        });
        assert_eq!(*seen.borrow(), vec![Err(Status::Locked); 2]);
    }

    /// A field accessor that calls a function with its own hook (a nested scope on the same object)
    /// must neither panic on the arena `RefCell` nor see the activation unlocked.
    #[test]
    fn an_accessor_may_open_a_nested_activation() {
        static HOOKING: FieldTable<Thing> = FieldTable {
            get: |t, f| {
                if f == 9 {
                    // "a hooked engine fn called by the accessor": opens its own activation on `t`
                    let (r, _) = scope(t, &THING, &mut nop, |h2| {
                        assert_eq!(get(h2, 0), Ok(0));
                        live_activations() as i64
                    });
                    return Some(r);
                }
                None
            },
            set: |_, _, _| false,
        };
        let mut thing = Thing::default();
        let outer_h = Cell::new(0u64);
        scope(&mut thing, &HOOKING, &mut nop, |h| {
            outer_h.set(h);
            assert_eq!(get(h, 9), Ok(2), "outer and nested are both open");
            // and the outer one is unlocked again afterwards
            assert_eq!(get(h, 9), Ok(2));
        });
        assert_eq!(live_activations(), 0);
    }

    #[test]
    fn accessor_locks_the_activation_while_it_runs() {
        static PROBE: FieldTable<Thing> = FieldTable {
            get: |_, f| {
                if f == 5 {
                    let h = PROBE_HANDLE.with(Cell::get);
                    return Some(match get(h, 0) {
                        Err(Status::Locked) => 1,
                        _ => 0,
                    });
                }
                None
            },
            set: |_, _, _| false,
        };
        thread_local! { static PROBE_HANDLE: Cell<u64> = const { Cell::new(0) }; }
        let mut thing = Thing::default();
        scope(&mut thing, &PROBE, &mut nop, |h| {
            PROBE_HANDLE.set(h);
            assert_eq!(get(h, 5), Ok(1), "re-entering the same handle from the accessor is Locked");
        });
    }

    #[test]
    fn other_thread_sees_nothing() {
        let mut thing = Thing::default();
        scope(&mut thing, &THING, &mut nop, |h| {
            std::thread::spawn(move || {
                assert_eq!(get(h, 0), Err(Status::Stale));
                assert_eq!(live_activations(), 0);
            })
            .join()
            .unwrap();
        });
    }

    // ---- MX3R1: path roots and path handles ----

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

    fn thing_path(id: u64) -> Vec<Accessor> {
        vec![Accessor::Field(0), Accessor::Id(id)]
    }

    #[test]
    fn a_path_handle_reaches_an_object_from_the_root_and_dies_with_the_activation() {
        let mut world = World { things: vec![Thing { count: 7, x: 1.0 }, Thing { count: 8, x: 2.0 }] };
        let mut kept = 0;
        scope_root(&mut world, &mut |w: &mut World| i64::from(w.things.len() as i32), |root| {
            assert_eq!(open_path(root, thing_path(99)), Err(Status::Stale), "no such object");
            assert_eq!(open_path(root, vec![Accessor::Field(5)]), Err(Status::BadField));
            let a = open_path(root, thing_path(7)).unwrap();
            let b = open_path(root, thing_path(8)).unwrap();
            kept = a;
            assert_eq!(live_path_handles(), 2);
            assert_eq!(with_arg::<Thing, _>(a, |t| { t.x += 10.0; t.x }), Ok(11.0));
            assert_eq!(with_arg::<Thing, _>(b, |t| t.x), Ok(2.0));
            assert_eq!(with_arg::<String, _>(a, |_| ()), Err(Status::WrongType));
            // re-borrowed on every use: two handles to the same object are fine one after the other
            let a2 = open_path(root, thing_path(7)).unwrap();
            assert_eq!(with_arg::<Thing, _>(a2, |t| t.x), Ok(11.0));
            // FieldTable access and `original` are not for path handles
            assert_eq!(get(a, 0), Err(Status::WrongType));
            assert_eq!(run_original(a), Err(Status::WrongType));
            // a path handle cannot be a root
            assert_eq!(open_path(a, thing_path(7)), Err(Status::WrongType));
        });
        assert_eq!(world.things[0].x, 11.0);
        assert_eq!(live_path_handles(), 0, "freed with the activation");
        assert_eq!(with_arg::<Thing, _>(kept, |_| ()), Err(Status::Stale));
    }

    #[test]
    fn a_removed_object_makes_its_handle_stale_and_the_lock_rejects_overlap() {
        let mut world = World { things: vec![Thing { count: 7, x: 1.0 }, Thing { count: 8, x: 2.0 }] };
        let mut seen = Vec::new();
        scope_root(
            &mut world,
            &mut |w: &mut World| {
                // runs inside `original`: the activation is lent, so every handle of it is Locked
                seen.push(format!("{:?}", with_arg::<Thing, _>(KEPT.with(|k| k.get()), |_| ())));
                w.things.retain(|t| t.count != 7);
                0
            },
            |root| {
                let a = open_path(root, thing_path(7)).unwrap();
                let b = open_path(root, thing_path(8)).unwrap();
                KEPT.with(|k| k.set(a));
                assert_eq!(with_arg::<Thing, _>(a, |t| t.count), Ok(7));
                run_original(root).unwrap();
                assert_eq!(with_arg::<Thing, _>(a, |_| ()), Err(Status::Stale), "the entity was removed");
                assert_eq!(with_arg::<Thing, _>(b, |t| t.count), Ok(8), "the other one is unaffected");
                // an access from inside an access is an overlap: Locked, nothing runs twice
                let nested = with_arg::<Thing, _>(b, |_| with_arg::<Thing, _>(b, |_| ()));
                assert_eq!(nested, Ok(Err(Status::Locked)));
            },
        );
        assert_eq!(seen, ["Err(Locked)"]);
        assert_eq!(world.things.len(), 1);
    }

    thread_local! {
        static KEPT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }

    #[test]
    fn an_activation_holds_a_bounded_number_of_path_handles() {
        let mut world = World { things: vec![Thing { count: 7, x: 1.0 }] };
        scope_root(&mut world, &mut |_: &mut World| 0, |root| {
            for _ in 0..MAX_PATHS_PER_ACTIVATION {
                open_path(root, thing_path(7)).unwrap();
            }
            assert_eq!(open_path(root, thing_path(7)), Err(Status::LimitReached));
        });
        assert_eq!(live_path_handles(), 0);
    }

    #[test]
    fn a_panic_in_the_resolver_frees_the_path_handles() {
        let mut world = World { things: vec![Thing { count: 7, x: 1.0 }] };
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scope_root(&mut world, &mut |_: &mut World| 0, |root| {
                let a = open_path(root, thing_path(7)).unwrap();
                let _ = with_arg::<Thing, _>(a, |_| panic!("engine bug"));
            });
        }));
        assert!(caught.is_err());
        assert_eq!(live_path_handles(), 0);
        assert_eq!(live_activations(), 0);
    }

    #[test]
    fn an_activation_without_paths_does_not_free_the_handles_of_another() {
        let mut world = World { things: vec![Thing { count: 7, x: 1.0 }] };
        scope_root(&mut world, &mut |_: &mut World| 0, |root| {
            let a = open_path(root, thing_path(7)).unwrap();
            // a nested activation that opens no path ends without touching the outer one's handles
            let mut inner = Thing::default();
            scope(&mut inner, &THING, &mut nop, |_| ());
            assert_eq!(with_arg::<Thing, _>(a, |t| t.count), Ok(7));
            assert_eq!(live_path_handles(), 1);
        });
        assert_eq!(live_path_handles(), 0);
    }
}
