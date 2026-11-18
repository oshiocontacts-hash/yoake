//! The core-module WASM runtime of the slice: C0's ACTIVE-store pattern over the activation arena.
//!
//! One mod instance per thread (the thread that called [`install`]). A hook shim calls
//! [`dispatch`]; on any other thread, with no instance, or once the mod is disabled, `dispatch`
//! just runs the original.
//!
//! Guest ABI (module `env`, all statuses are [`Status`] as `i32`):
//! * `original(handle: i64, out: i32) -> i32`: runs the original, writes its `i64` result at `out`;
//! * `get(handle: i64, field: i32, out: i32) -> i32`: writes the field's `i64` bits at `out`;
//! * `set(handle: i64, field: i32, bits: i64) -> i32`;
//! * `engine_take(ticket: i32, out_ptr: i32, out_cap: i32) -> i32`: the kept result of an `engine_call`
//!   whose buffer was too small; the call is never repeated (see [`call_status`]);
//! * `report(slot: i32, value: i64)`: test aid, appended to [`take_reports`];
//! * `engine_call(handle: i64, id: i32, ptr: i32, len: i32, out_ptr: i32, out_cap: i32) -> i32`: runs a
//!   generated binding (MX3E1, see [`call_status`]): the argument bytes are copied out of guest memory
//!   (cap [`MAX_CALL_ARG_BYTES`]) before engine code runs, the engine call is wrapped in `catch_unwind`
//!   (a panic traps the guest, so the trap rule disables the mod), the result is written afterwards.
//!
//! The guest exports `<name>(handle: i64) -> i64` (the hook; its result is the hooked function's
//! result) and a `memory`. `out` points into a scratch buffer the guest owns, so there is no
//! alloc/free per call.
//!
//! # Context discipline (ACTIVE)
//!
//! `ACTIVE` points at the innermost store context that engine code may use. It is published only by
//! [`native`] (every import that runs engine code: `original`, `get`, `set` go through it) and by
//! the top-level entry. It is **null while the guest runs**, so engine code reached from guest code
//! outside such an import (or from an import someone forgot to wrap) finds no context and the hook
//! simply does not fire. [`install`], [`uninstall`], [`take_reports`] and [`stats`] use
//! `try_borrow` and report "busy" instead of panicking when called from inside `original`.
//!
//! # Unsafe surface
//!
//! One function: [`context`] dereferences the thread's `ACTIVE` pointer to the innermost live
//! `StoreContextMut`, which is published by [`with_active`] (top-level entry and every import that
//! calls native engine code). The invariant is C0's: native code below an import never touches the
//! outer `Caller` until it returns, so the innermost published context is the only live borrow.

use crate::activation::{self, FieldTable, Finished, Status};
use std::cell::{Cell, RefCell};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;
use wasmtime::{
    AsContextMut, Caller, Config, Engine, Error, Linker, Memory, Module, Store, StoreContextMut,
    TypedFunc,
};

type R<T> = std::result::Result<T, Error>;

#[derive(Clone, Debug)]
pub struct ModConfig {
    /// Name of the guest export `(i64) -> i64` that is the hook.
    pub hook_export: String,
    /// Fuel per activation (`None`: unmetered).
    pub fuel: Option<u64>,
    /// Wall-clock budget per activation in 1 ms epoch ticks (`None`: no epoch).
    pub epoch_ticks: Option<u64>,
    /// Pause the epoch budget while `original` runs (C0 section 3). Off only as a negative control.
    pub pause_epoch_in_original: bool,
}

impl ModConfig {
    pub fn new(hook_export: &str) -> Self {
        Self {
            hook_export: hook_export.to_owned(),
            fuel: Some(1_000_000),
            epoch_ticks: None,
            pause_epoch_in_original: true,
        }
    }
}

struct EpochAct {
    guest_used: u64,
    resumed_at: u64,
}

struct Host {
    hook: Option<TypedFunc<i64, i64>>,
    memory: Option<Memory>,
    cfg: ModConfig,
    disabled: bool,
    trap: Option<String>,
    ticks: Arc<AtomicU64>,
    epoch: Vec<EpochAct>,
    reports: Vec<(i32, i64)>,
    entries: u64,
    max_fuel_used: u64,
    depth: u32,
    /// The open activations of this instance, innermost last (their entry numbers).
    acts: Vec<u64>,
    /// Results that did not fit the guest's buffer, kept for `engine_take` (never re-run).
    tickets: Vec<Ticket>,
    next_ticket: u32,
}

struct Ticket {
    id: u32,
    act: u64,
    bytes: Vec<u8>,
}

struct Ticker {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Ticker {
    fn drop(&mut self) {
        // Detach: joining from a thread-local destructor (thread exit) aborts the process.
        self.stop.store(true, Ordering::Relaxed);
        drop(self.thread.take());
    }
}

struct ModInst {
    store: Store<Host>,
    _ticker: Option<Ticker>,
}

/// All per-thread runtime state behind one thread-local, so a hook entry reads thread-local storage
/// once (on the windows-gnu toolchain one access costs about 15 ns).
struct Rt {
    /// The innermost live store context (C0's ACTIVE pointer); null outside guest runs.
    active: Cell<*mut StoreContextMut<'static, Host>>,
    inst: RefCell<Option<ModInst>>,
}

thread_local! {
    static RT: Rt = const { Rt { active: Cell::new(std::ptr::null_mut()), inst: RefCell::new(None) } };
}

/// Restores the previous `active` pointer, also when a panic unwinds through.
struct ActiveGuard<'r> {
    cell: &'r Cell<*mut StoreContextMut<'static, Host>>,
    previous: *mut StoreContextMut<'static, Host>,
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.cell.set(self.previous);
    }
}

/// Publishes `ctx` as the innermost context while `f` runs. Returns the pointer `f` is given: all
/// use of the context from here on goes through it (not through `ctx`), as in C0.
fn with_active<T>(
    rt: &Rt,
    ctx: &mut StoreContextMut<'_, Host>,
    f: impl FnOnce(*mut StoreContextMut<'static, Host>) -> T,
) -> T {
    let pointer = (ctx as *mut StoreContextMut<'_, Host>).cast::<StoreContextMut<'static, Host>>();
    let _restore = ActiveGuard { cell: &rt.active, previous: rt.active.replace(pointer) };
    f(pointer)
}

/// # Safety
/// `pointer` is the innermost context published by [`with_active`] (the contract of every caller:
/// it was just read from `Rt::active` or just published), and no other reference to it is live.
unsafe fn context<'x>(pointer: *mut StoreContextMut<'static, Host>) -> &'x mut StoreContextMut<'static, Host> {
    // SAFETY: caller contract. The native code between the publishing frame and this call does not
    // use the `Caller` it came from (module docs).
    unsafe { &mut *pointer }
}

/// The unmetered store's starting fuel; every top-level activation must leave it unchanged.
pub const INITIAL_FUEL: u64 = u64::MAX / 4;

/// Installs a mod (WAT text or a wasm binary) on the calling thread, replacing any earlier one.
/// Fails (instead of panicking) when called while this thread's guest is running.
pub fn install(wasm_or_wat: &[u8], cfg: ModConfig) -> R<()> {
    let mut config = Config::new();
    config.consume_fuel(cfg.fuel.is_some()).epoch_interruption(cfg.epoch_ticks.is_some());
    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, wasm_or_wat)?;
    let ticks = Arc::new(AtomicU64::new(0));
    let ticker = cfg.epoch_ticks.map(|_| {
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, counter, engine) = (stop.clone(), ticks.clone(), engine.clone());
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
                counter.fetch_add(1, Ordering::Relaxed);
                engine.increment_epoch();
            }
        });
        Ticker { stop, thread: Some(thread) }
    });
    let linker = build_linker(&engine)?;
    let export = cfg.hook_export.clone();
    let host = Host {
        hook: None,
        memory: None,
        cfg,
        disabled: false,
        trap: None,
        ticks,
        epoch: Vec::new(),
        reports: Vec::new(),
        entries: 0,
        max_fuel_used: 0,
        depth: 0,
        acts: Vec::new(),
        tickets: Vec::new(),
        next_ticket: 0,
    };
    let mut store = Store::new(&engine, host);
    if store.data().cfg.fuel.is_some() {
        store.set_fuel(INITIAL_FUEL)?;
    }
    if store.data().cfg.epoch_ticks.is_some() {
        store.set_epoch_deadline(u64::MAX / 4);
    }
    let instance = linker.instantiate(&mut store, &module)?;
    let hook = instance.get_typed_func::<i64, i64>(&mut store, &export)?;
    let memory = instance.get_memory(&mut store, "memory");
    store.data_mut().hook = Some(hook);
    store.data_mut().memory = memory;
    RT.with(|rt| match rt.inst.try_borrow_mut() {
        Ok(mut slot) => {
            *slot = Some(ModInst { store, _ticker: ticker });
            Ok(())
        }
        Err(_) => Err(Error::msg("install: this thread's guest is running (called from inside a hook)")),
    })
}

/// Drops the calling thread's mod instance. Fails when called from inside a hook.
#[must_use = "uninstall fails when called from inside a hook"]
pub fn uninstall() -> R<()> {
    RT.with(|rt| match rt.inst.try_borrow_mut() {
        Ok(mut slot) => {
            *slot = None;
            Ok(())
        }
        Err(_) => Err(Error::msg("uninstall: this thread's guest is running (called from inside a hook)")),
    })
}

/// What the calling thread's mod did so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub installed: bool,
    /// The guest is running (the call came from inside a hook); the other fields are then empty.
    pub busy: bool,
    pub disabled: bool,
    pub trap: Option<String>,
    /// Guest hook activations entered.
    pub entries: u64,
    /// The most fuel one activation used (0 when unmetered).
    pub max_fuel_used: u64,
    /// Open guest activations (0 outside a hook, also after a panic).
    pub depth: u32,
    /// Open epoch records (0 outside a hook, also after a panic).
    pub epoch_depth: usize,
    /// The store's fuel right now (`INITIAL_FUEL` outside a hook).
    pub fuel_left: Option<u64>,
}

pub fn stats() -> Stats {
    RT.with(|rt| match rt.inst.try_borrow() {
        Ok(slot) => match slot.as_ref() {
            Some(inst) => {
                let host = inst.store.data();
                Stats {
                    installed: true,
                    busy: false,
                    disabled: host.disabled,
                    trap: host.trap.clone(),
                    entries: host.entries,
                    max_fuel_used: host.max_fuel_used,
                    depth: host.depth,
                    epoch_depth: host.epoch.len(),
                    fuel_left: inst.store.get_fuel().ok(),
                }
            }
            None => Stats::default(),
        },
        Err(_) => Stats { installed: true, busy: true, ..Stats::default() },
    })
}

/// The `report(slot, value)` calls the guest made since the last call. Fails from inside a hook.
pub fn take_reports() -> R<Vec<(i32, i64)>> {
    RT.with(|rt| match rt.inst.try_borrow_mut() {
        Ok(mut slot) => Ok(match slot.as_mut() {
            Some(inst) => std::mem::take(&mut inst.store.data_mut().reports),
            None => Vec::new(),
        }),
        Err(_) => Err(Error::msg("take_reports: this thread's guest is running (called from inside a hook)")),
    })
}

/// Runs engine code on behalf of an import: republishes this call's own context as the innermost
/// one, so a nested hook (from `original`, or from a field accessor) uses it, and pauses the
/// outer activation's epoch budget for the duration, restoring the outer deadline afterwards (a
/// nested hook sets its own deadline and must not leave it behind). Every import that can reach
/// engine code goes through here; use [`engine_import!`] to register one so it cannot be forgotten.
fn native<T>(caller: &mut Caller<'_, Host>, f: impl FnOnce() -> T) -> T {
    let epoch_on = caller.data().cfg.epoch_ticks.is_some() && caller.data().cfg.pause_epoch_in_original;
    if epoch_on {
        // the guest's time so far in this activation is charged, the engine's is not
        let now = caller.data().ticks.load(Ordering::Relaxed);
        if let Some(top) = caller.data_mut().epoch.last_mut() {
            top.guest_used += now.saturating_sub(top.resumed_at);
        }
    }
    let out = RT.with(|rt| {
        let mut ctx = caller.as_context_mut();
        with_active(rt, &mut ctx, |_| f())
    });
    if epoch_on {
        let now = caller.data().ticks.load(Ordering::Relaxed);
        let budget = caller.data().cfg.epoch_ticks.unwrap_or(1);
        let mut remaining = budget;
        if let Some(top) = caller.data_mut().epoch.last_mut() {
            top.resumed_at = now;
            remaining = budget.saturating_sub(top.guest_used).max(1);
        }
        caller.as_context_mut().set_epoch_deadline(remaining);
    }
    out
}

/// Registers an import that reaches engine code. The `engine:` expression is the only part that
/// runs engine code and it always runs inside [`native`]; `finish:` gets the caller and the engine
/// result (for example to write the result into guest memory). A generator emits only this form.
macro_rules! engine_import {
    // With a `prepare:` step: it runs BEFORE `native()` with the caller (to copy argument bytes out
    // of guest memory); its value `$p` is borrowed by `engine:` (which still runs only inside
    // `native`). `?` in `prepare` traps the guest (a pointer outside its memory).
    ($linker:expr, $name:literal, |$($arg:ident : $ty:ty),*| -> $ret:ty,
     prepare: |$pc:ident| $prep:expr => $p:ident,
     engine: $engine:expr, finish: |$c:ident, $e:ident| $finish:expr) => {
        $linker.func_wrap("env", $name, move |mut caller: Caller<'_, Host>, $($arg: $ty),*| -> $ret {
            let $p = {
                let $pc = &mut caller;
                $prep
            };
            let $e = native(&mut caller, || $engine);
            let $c = &mut caller;
            $finish
        })
    };
    ($linker:expr, $name:literal, |$($arg:ident : $ty:ty),*| -> $ret:ty,
     engine: $engine:expr, finish: |$c:ident, $e:ident| $finish:expr) => {
        $linker.func_wrap("env", $name, move |mut caller: Caller<'_, Host>, $($arg: $ty),*| -> $ret {
            let $e = native(&mut caller, || $engine);
            let $c = &mut caller;
            $finish
        })
    };
}

/// What generated glue looks like to the runtime: `(handle, binding id, argument bytes, out)`.
/// The engine registers one function that tries each module's generated `call` in turn.
pub type EngineDispatch = fn(u64, u32, &[u8], &mut Vec<u8>) -> Result<(), crate::codec::CallError>;

static DISPATCH: std::sync::Mutex<Option<EngineDispatch>> = std::sync::Mutex::new(None);

/// Registers (or with `None` removes) the table `engine_call` dispatches through. Process-wide.
pub fn set_engine_dispatch(dispatch: Option<EngineDispatch>) {
    *DISPATCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = dispatch;
}

/// Results one activation may keep for `engine_take`, and their total size.
pub const MAX_TICKETS: usize = 8;
pub const MAX_TICKET_BYTES: usize = 16 << 20;

/// The most argument bytes one `engine_call` copies out of guest memory.
pub const MAX_CALL_ARG_BYTES: usize = 1 << 20;

/// `engine_call` results: `>= 0` is the number of result bytes written at `out_ptr`; negative is one
/// of these (handle statuses are `HANDLE_BASE - status`: Stale = -17, Locked = -18, BadField = -19,
/// NoSuchArg = -20, WrongType = -21). An engine panic is not a status: it traps the guest.
pub mod call_status {
    pub const ARGS_TOO_LARGE: i32 = -1;
    /// The result needs more than `out_cap` bytes; its length is written as a little-endian `u32` at
    /// `out_ptr` when `out_cap >= 4`. Nothing else was written. Call again with a bigger buffer.
    pub const OUT_TOO_SMALL: i32 = -2;
    /// `engine_call` result when the buffer was too small: the call HAS run and its result is kept under
    /// a ticket; the return value is `-(OUT_TOO_SMALL_BASE + ticket)` (so `<= -257`). Fetch it with
    /// `engine_take(ticket, out_ptr, out_cap)`; never call again to retry.
    pub const OUT_TOO_SMALL_BASE: i32 = 256;
    /// `engine_take`: no such ticket (never issued, already taken, or another activation's).
    pub const NO_SUCH_TICKET: i32 = -7;
    /// The result did not fit and this activation already holds [`super::MAX_TICKETS`] results or
    /// [`super::MAX_TICKET_BYTES`] bytes: the call ran, its result is lost. Size the buffer first.
    pub const TICKET_LIMIT: i32 = -8;

    /// The ticket inside an `engine_call` return value, if it is one.
    pub fn ticket_of(status: i32) -> Option<u32> {
        (status <= -(OUT_TOO_SMALL_BASE + 1)).then(|| (-status - OUT_TOO_SMALL_BASE) as u32)
    }
    pub const BAD_ARGS: i32 = -3;
    pub const READ_ONLY: i32 = -4;
    pub const UNKNOWN_BINDING: i32 = -5;
    pub const NO_DISPATCHER: i32 = -6;
    pub const HANDLE_BASE: i32 = -16;
}

enum CallOutcome {
    Done(Vec<u8>),
    Status(i32),
    Panic(String),
}

/// Runs the registered dispatcher on `args`. Always called inside `native()`. An engine panic caused
/// by guest-supplied arguments is caught here and reported to the import, which traps the guest.
fn run_engine_call(handle: u64, id: u32, args: Result<Vec<u8>, i32>) -> CallOutcome {
    use crate::codec::CallError;
    let args = match args {
        Ok(args) => args,
        Err(status) => return CallOutcome::Status(status),
    };
    let Some(dispatch) = *DISPATCH.lock().unwrap_or_else(std::sync::PoisonError::into_inner) else {
        return CallOutcome::Status(call_status::NO_DISPATCHER);
    };
    let mut out = Vec::new();
    match catch_unwind(AssertUnwindSafe(|| dispatch(handle, id, &args, &mut out))) {
        Ok(Ok(())) => CallOutcome::Done(out),
        Ok(Err(error)) => CallOutcome::Status(match error {
            CallError::Handle(status) => call_status::HANDLE_BASE - status as i32,
            CallError::ReadOnly(_) => call_status::READ_ONLY,
            CallError::UnknownBinding(_) => call_status::UNKNOWN_BINDING,
            _ => call_status::BAD_ARGS,
        }),
        Err(payload) => CallOutcome::Panic(panic_message(&payload)),
    }
}

fn write_i64(caller: &mut Caller<'_, Host>, out: i32, value: i64) -> R<()> {
    let memory = caller.data().memory.ok_or_else(|| Error::msg("guest has no memory"))?;
    memory.write(&mut *caller, out as u32 as usize, &value.to_le_bytes())?;
    Ok(())
}

/// Copies `len` bytes at `ptr` out of guest memory, or the status when `len` is over the cap
/// (nothing is read then). A range outside the guest memory is an error (a trap).
fn copy_args(caller: &mut Caller<'_, Host>, ptr: i32, len: i32) -> R<Result<Vec<u8>, i32>> {
    if len < 0 || len as usize > MAX_CALL_ARG_BYTES {
        return Ok(Err(call_status::ARGS_TOO_LARGE));
    }
    let memory = caller.data().memory.ok_or_else(|| Error::msg("guest has no memory"))?;
    let mut bytes = vec![0u8; len as usize];
    memory.read(&*caller, ptr as u32 as usize, &mut bytes)?;
    Ok(Ok(bytes))
}

/// Writes `bytes` at `out_ptr` if they fit `out_cap`. Otherwise the call has already run, so the
/// result is kept under a ticket for [`engine_take`](take_ticket) (and its size is written as a `u32`
/// at `out_ptr` when there is room): the return value carries the ticket, nothing is ever run twice.
fn write_result(caller: &mut Caller<'_, Host>, out_ptr: i32, out_cap: i32, bytes: Vec<u8>) -> R<i32> {
    let memory = caller.data().memory.ok_or_else(|| Error::msg("guest has no memory"))?;
    if out_cap >= 0 && bytes.len() <= out_cap as usize {
        memory.write(&mut *caller, out_ptr as u32 as usize, &bytes)?;
        return Ok(bytes.len() as i32);
    }
    if out_cap >= 4 {
        memory.write(&mut *caller, out_ptr as u32 as usize, &(bytes.len() as u32).to_le_bytes())?;
    }
    let host = caller.data_mut();
    let Some(&act) = host.acts.last() else { return Ok(call_status::TICKET_LIMIT) };
    let (count, held) = host.tickets.iter().filter(|t| t.act == act).fold((0, 0), |(n, b), t| (n + 1, b + t.bytes.len()));
    if count >= MAX_TICKETS || held + bytes.len() > MAX_TICKET_BYTES {
        return Ok(call_status::TICKET_LIMIT);
    }
    host.next_ticket += 1;
    let id = host.next_ticket;
    host.tickets.push(Ticket { id, act, bytes });
    Ok(-(call_status::OUT_TOO_SMALL_BASE + id as i32))
}

/// `engine_take(ticket, out_ptr, out_cap)`: copies the kept result of an earlier `engine_call` of this
/// same activation. Returns the byte count (the ticket is then spent), or `OUT_TOO_SMALL` (size written
/// at `out_ptr` when `out_cap >= 4`, ticket kept) or `NO_SUCH_TICKET`. Runs no engine code.
fn take_ticket(caller: &mut Caller<'_, Host>, ticket: i32, out_ptr: i32, out_cap: i32) -> R<i32> {
    let memory = caller.data().memory.ok_or_else(|| Error::msg("guest has no memory"))?;
    let host = caller.data();
    let Some(act) = host.acts.last().copied() else { return Ok(call_status::NO_SUCH_TICKET) };
    let Some(index) = host.tickets.iter().position(|t| t.act == act && i64::from(t.id) == i64::from(ticket)) else {
        return Ok(call_status::NO_SUCH_TICKET);
    };
    let len = host.tickets[index].bytes.len();
    if out_cap < 0 || len > out_cap as usize {
        if out_cap >= 4 {
            memory.write(&mut *caller, out_ptr as u32 as usize, &(len as u32).to_le_bytes())?;
        }
        return Ok(call_status::OUT_TOO_SMALL);
    }
    let bytes = std::mem::take(&mut caller.data_mut().tickets[index].bytes);
    memory.write(&mut *caller, out_ptr as u32 as usize, &bytes)?;
    caller.data_mut().tickets.remove(index);
    Ok(len as i32)
}

fn status_code(status: Status) -> i32 {
    status as i32
}

fn build_linker(engine: &Engine) -> R<Linker<Host>> {
    let mut linker = Linker::new(engine);
    engine_import!(linker, "get", |handle: i64, field: i32, out: i32| -> R<i32>,
        engine: activation::get(handle as u64, field as u32),
        finish: |c, result| match result {
            Ok(value) => {
                write_i64(c, out, value)?;
                Ok(status_code(Status::Ok))
            }
            Err(status) => Ok(status_code(status)),
        })?;
    engine_import!(linker, "set", |handle: i64, field: i32, bits: i64| -> i32,
        engine: activation::set(handle as u64, field as u32, bits),
        finish: |_c, result| match result {
            Ok(()) => status_code(Status::Ok),
            Err(status) => status_code(status),
        })?;
    engine_import!(linker, "original", |handle: i64, out: i32| -> R<i32>,
        engine: activation::run_original(handle as u64),
        finish: |c, result| match result {
            Ok(value) => {
                write_i64(c, out, value)?;
                Ok(status_code(Status::Ok))
            }
            Err(status) => Ok(status_code(status)),
        })?;
    // engine_call(handle, id, ptr, len, out_ptr, out_cap) -> i32: runs a generated binding.
    engine_import!(linker, "engine_call", |handle: i64, id: i32, ptr: i32, len: i32, out_ptr: i32, out_cap: i32| -> R<i32>,
        prepare: |c| copy_args(c, ptr, len)? => args,
        engine: run_engine_call(handle as u64, id as u32, args),
        finish: |c, outcome| match outcome {
            CallOutcome::Status(status) => Ok(status),
            CallOutcome::Panic(message) => {
                eprintln!("yoake: engine state may be inconsistent: binding {id} panicked on guest-supplied arguments ({message}); the mod is disabled");
                Err(Error::msg(format!("engine panic inside engine_call {id}: {message}; engine state may be inconsistent")))
            }
            CallOutcome::Done(bytes) => write_result(c, out_ptr, out_cap, bytes),
        })?;
    // engine_take(ticket, out_ptr, out_cap) -> i32: fetches a result that did not fit, without re-running.
    linker.func_wrap("env", "engine_take", |mut c: Caller<'_, Host>, ticket: i32, out_ptr: i32, out_cap: i32| -> R<i32> {
        take_ticket(&mut c, ticket, out_ptr, out_cap)
    })?;
    // not engine imports: they only touch the host state
    linker.func_wrap("env", "report", |mut c: Caller<'_, Host>, slot: i32, value: i64| {
        c.data_mut().reports.push((slot, value));
    })?;
    linker.func_wrap("env", "ticks", |c: Caller<'_, Host>| -> i64 {
        c.data().ticks.load(Ordering::Relaxed) as i64
    })?;
    Ok(linker)
}

fn enter(ctx: &mut StoreContextMut<'static, Host>) -> Option<u64> {
    ctx.data_mut().depth += 1;
    ctx.data_mut().entries += 1;
    let entry = ctx.data().entries;
    ctx.data_mut().acts.push(entry);
    let saved = match ctx.data().cfg.fuel {
        Some(budget) => {
            let saved = ctx.get_fuel().ok();
            ctx.set_fuel(budget).ok();
            saved
        }
        None => None,
    };
    if let Some(budget) = ctx.data().cfg.epoch_ticks {
        let now = ctx.data().ticks.load(Ordering::Relaxed);
        ctx.data_mut().epoch.push(EpochAct { guest_used: 0, resumed_at: now });
        ctx.set_epoch_deadline(budget);
    }
    saved
}

fn leave(ctx: &mut StoreContextMut<'static, Host>, saved_fuel: Option<u64>) {
    ctx.data_mut().depth -= 1;
    // tickets die with their activation
    if let Some(act) = ctx.data_mut().acts.pop() {
        ctx.data_mut().tickets.retain(|t| t.act != act);
    }
    if let Some(saved) = saved_fuel {
        if let (Some(budget), Ok(left)) = (ctx.data().cfg.fuel, ctx.get_fuel()) {
            let used = budget.saturating_sub(left);
            let host = ctx.data_mut();
            host.max_fuel_used = host.max_fuel_used.max(used);
        }
        ctx.set_fuel(saved).ok();
    }
    if ctx.data().cfg.epoch_ticks.is_some() {
        ctx.data_mut().epoch.pop();
    }
}

enum Outcome {
    /// The guest's return value.
    Value(i64),
    /// No hook ran (no instance on this thread, disabled, or the guest is already running here).
    Fallthrough,
    /// The guest trapped; the mod is now disabled.
    Trapped(Finished),
}

/// Deepest guest nesting (hook -> import -> engine -> hook ...). Past it a hooked call falls
/// through to the original body and the guest is not entered, so guest-driven recursion (for
/// example through a field accessor) cannot exhaust the native stack. 16, not more: every level is a
/// wasm entry plus host frames under wasmtime's one stack budget, which ran out at depth 58 in a debug
/// build (a clean "call stack exhausted" trap, but the cap should come first).
pub const MAX_DEPTH: u32 = 16;

type ScopeFn<'s> = &'s mut dyn FnMut(u64) -> R<i64>;

/// The hook entry a shim calls. `orig` runs the engine's original body for the hooked function and
/// returns its result as an `i64` (0 for `()`); the return value is what the hooked function
/// should return (for the original's own result when the mod did not override it).
pub fn dispatch<T: 'static>(
    arg: &mut T,
    table: &'static FieldTable<T>,
    orig: &mut dyn FnMut(&mut T) -> i64,
) -> i64 {
    let outcome = dispatch_with(|body| activation::scope(&mut *arg, table, &mut *orig, |h| body(h)));
    match outcome {
        Outcome::Value(value) => value,
        Outcome::Fallthrough => orig(arg),
        Outcome::Trapped(done) => {
            if done.original_calls == 0 { orig(arg) } else { done.last_result }
        }
    }
}

/// [`dispatch`] for a hook whose argument 0 is a path root (`Game`): the guest gets the root handle, and
/// through it path handles to the objects the root can reach (`activation::open_path`, generated
/// "get handle" bindings). Only a function that really receives the root may use this: inside any other
/// hook the caller holds an outer `&mut` and there is no root to re-borrow from (design 4b).
pub fn dispatch_root<G: activation::PathRoot>(root: &mut G, orig: &mut dyn FnMut(&mut G) -> i64) -> i64 {
    let outcome = dispatch_with(|body| activation::scope_root(&mut *root, &mut *orig, |h| body(h)));
    match outcome {
        Outcome::Value(value) => value,
        Outcome::Fallthrough => orig(root),
        Outcome::Trapped(done) => {
            if done.original_calls == 0 { orig(root) } else { done.last_result }
        }
    }
}

/// [`dispatch`] for a function with two `&mut` arguments: `original` re-receives both.
pub fn dispatch2<A: 'static, B: 'static>(
    a: &mut A,
    table_a: &'static FieldTable<A>,
    b: &mut B,
    table_b: &'static FieldTable<B>,
    orig: &mut dyn FnMut(&mut A, &mut B) -> i64,
) -> i64 {
    let outcome = dispatch_with(|body| {
        activation::scope2(&mut *a, table_a, &mut *b, table_b, &mut *orig, |h| body(h))
    });
    match outcome {
        Outcome::Value(value) => value,
        Outcome::Fallthrough => orig(a, b),
        Outcome::Trapped(done) => {
            if done.original_calls == 0 { orig(a, b) } else { done.last_result }
        }
    }
}

fn dispatch_with(scope: impl FnOnce(ScopeFn<'_>) -> (R<i64>, Finished)) -> Outcome {
    RT.with(|rt| {
        let pointer = rt.active.get();
        if !pointer.is_null() {
            // Inside an import that published its context (`native`): a nested hook.
            // SAFETY: just read from `active`.
            return run_hook(rt, unsafe { context(pointer) }, scope);
        }
        // Top level: this thread's instance publishes its store for the whole guest run. If the
        // guest is already running here (ACTIVE is null while it runs, so this is engine code
        // reached from guest code outside a `native` import), the instance is borrowed: no hook.
        let Ok(mut guard) = rt.inst.try_borrow_mut() else { return Outcome::Fallthrough };
        let Some(inst) = guard.as_mut() else { return Outcome::Fallthrough };
        if inst.store.data().disabled {
            return Outcome::Fallthrough;
        }
        let mut ctx = inst.store.as_context_mut();
        with_active(rt, &mut ctx, |pointer| {
            // SAFETY: just published by `with_active`; `ctx` is not used again.
            run_hook(rt, unsafe { context(pointer) }, scope)
        })
    })
}

fn run_hook(
    rt: &Rt,
    ctx: &mut StoreContextMut<'static, Host>,
    scope: impl FnOnce(ScopeFn<'_>) -> (R<i64>, Finished),
) -> Outcome {
    let Some(hook) = (if ctx.data().disabled { None } else { ctx.data().hook.clone() }) else {
        return Outcome::Fallthrough;
    };
    if ctx.data().depth >= MAX_DEPTH {
        return Outcome::Fallthrough;
    }
    let (result, finished) = scope(&mut |handle| {
        let saved = enter(ctx);
        // The guest runs with no published context: only `native` imports may reach engine code.
        let hidden = ActiveGuard { cell: &rt.active, previous: rt.active.replace(std::ptr::null_mut()) };
        // A panic from the engine body inside `original` unwinds through wasmtime to here: restore
        // the per-activation state (fuel, epoch stack, depth) before letting it continue.
        let called = catch_unwind(AssertUnwindSafe(|| hook.call(&mut *ctx, handle as i64)));
        drop(hidden);
        leave(ctx, saved);
        match called {
            Ok(result) => result,
            Err(payload) => {
                // The guest's frames were abandoned mid-run (its shadow stack and allocator may be
                // corrupt): same rule as a trap, never enter this instance again. Then re-throw.
                let host = ctx.data_mut();
                host.disabled = true;
                host.trap.get_or_insert(format!("engine panic inside a hook: {}", panic_message(&payload)));
                resume_unwind(payload)
            }
        }
    });
    match result {
        Ok(value) => Outcome::Value(value),
        Err(error) => {
            // The trap rule (MX3C0 section 5): disable the mod; the caller runs the original only
            // if it has not run in this activation, otherwise returns what it returned.
            let host = ctx.data_mut();
            host.disabled = true;
            host.trap.get_or_insert(format!("{error:#}"));
            Outcome::Trapped(finished)
        }
    }
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.clone()
    } else {
        "non-string payload".to_owned()
    }
}
