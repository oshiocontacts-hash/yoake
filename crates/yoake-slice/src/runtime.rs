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
//! * `report(slot: i32, value: i64)`: test aid, appended to [`take_reports`].
//!
//! The guest exports `<name>(handle: i64) -> i64` (the hook; its result is the hooked function's
//! result) and a `memory`. `out` points into a scratch buffer the guest owns, so there is no
//! alloc/free per call.
//!
//! # Unsafe surface
//!
//! One function: [`context`] dereferences the thread's `ACTIVE` pointer to the innermost live
//! `StoreContextMut`, which is published by [`with_active`] (top-level entry and every import that
//! calls native engine code). The invariant is C0's: native code below an import never touches the
//! outer `Caller` until it returns, so the innermost published context is the only live borrow.

use crate::activation::{self, Activation, FieldTable, OrigCell, Status};
use std::cell::{Cell, RefCell};
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

/// Installs a mod (WAT text or a wasm binary) on the calling thread, replacing any earlier one.
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
    };
    let mut store = Store::new(&engine, host);
    if store.data().cfg.fuel.is_some() {
        store.set_fuel(u64::MAX / 4)?;
    }
    if store.data().cfg.epoch_ticks.is_some() {
        store.set_epoch_deadline(u64::MAX / 4);
    }
    let instance = linker.instantiate(&mut store, &module)?;
    let hook = instance.get_typed_func::<i64, i64>(&mut store, &export)?;
    let memory = instance.get_memory(&mut store, "memory");
    store.data_mut().hook = Some(hook);
    store.data_mut().memory = memory;
    RT.with(|rt| *rt.inst.borrow_mut() = Some(ModInst { store, _ticker: ticker }));
    Ok(())
}

/// Drops the calling thread's mod instance.
pub fn uninstall() {
    RT.with(|rt| *rt.inst.borrow_mut() = None);
}

/// What the calling thread's mod did so far.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub installed: bool,
    pub disabled: bool,
    pub trap: Option<String>,
    /// Guest hook activations entered.
    pub entries: u64,
    /// The most fuel one activation used (0 when unmetered).
    pub max_fuel_used: u64,
}

pub fn stats() -> Stats {
    RT.with(|rt| match rt.inst.borrow().as_ref() {
        Some(inst) => {
            let host = inst.store.data();
            Stats { installed: true, disabled: host.disabled, trap: host.trap.clone(), entries: host.entries, max_fuel_used: host.max_fuel_used }
        }
        None => Stats::default(),
    })
}

/// The `report(slot, value)` calls the guest made since the last call.
pub fn take_reports() -> Vec<(i32, i64)> {
    RT.with(|rt| match rt.inst.borrow_mut().as_mut() {
        Some(inst) => std::mem::take(&mut inst.store.data_mut().reports),
        None => Vec::new(),
    })
}

fn write_i64(caller: &mut Caller<'_, Host>, out: i32, value: i64) -> R<()> {
    let memory = caller.data().memory.ok_or_else(|| Error::msg("guest has no memory"))?;
    memory.write(&mut *caller, out as u32 as usize, &value.to_le_bytes())?;
    Ok(())
}

fn status_code(status: Status) -> i32 {
    status as i32
}

fn build_linker(engine: &Engine) -> R<Linker<Host>> {
    let mut linker = Linker::new(engine);
    linker.func_wrap("env", "get", |mut c: Caller<'_, Host>, handle: i64, field: i32, out: i32| -> R<i32> {
        match activation::get(handle as u64, field as u32) {
            Ok(value) => {
                write_i64(&mut c, out, value)?;
                Ok(status_code(Status::Ok))
            }
            Err(status) => Ok(status_code(status)),
        }
    })?;
    linker.func_wrap("env", "set", |_c: Caller<'_, Host>, handle: i64, field: i32, bits: i64| -> i32 {
        match activation::set(handle as u64, field as u32, bits) {
            Ok(()) => status_code(Status::Ok),
            Err(status) => status_code(status),
        }
    })?;
    linker.func_wrap("env", "report", |mut c: Caller<'_, Host>, slot: i32, value: i64| {
        c.data_mut().reports.push((slot, value));
    })?;
    linker.func_wrap("env", "original", |mut c: Caller<'_, Host>, handle: i64, out: i32| -> R<i32> {
        // Epoch: the guest's time so far in this activation is charged, `original`'s is not.
        let epoch_on = c.data().cfg.epoch_ticks.is_some() && c.data().cfg.pause_epoch_in_original;
        if epoch_on {
            let now = c.data().ticks.load(Ordering::Relaxed);
            if let Some(top) = c.data_mut().epoch.last_mut() {
                top.guest_used += now.saturating_sub(top.resumed_at);
            }
        }
        // Re-publish this call's own context: the engine body may hit another hook slot.
        let result = RT.with(|rt| {
            let mut ctx = c.as_context_mut();
            with_active(rt, &mut ctx, |_| activation::run_original(handle as u64))
        });
        if epoch_on {
            let now = c.data().ticks.load(Ordering::Relaxed);
            let budget = c.data().cfg.epoch_ticks.unwrap_or(1);
            let mut remaining = budget;
            if let Some(top) = c.data_mut().epoch.last_mut() {
                top.resumed_at = now;
                remaining = budget.saturating_sub(top.guest_used).max(1);
            }
            c.as_context_mut().set_epoch_deadline(remaining);
        }
        match result {
            Ok(value) => {
                write_i64(&mut c, out, value)?;
                Ok(status_code(Status::Ok))
            }
            Err(status) => Ok(status_code(status)),
        }
    })?;
    Ok(linker)
}

fn enter(ctx: &mut StoreContextMut<'static, Host>) -> Option<u64> {
    ctx.data_mut().depth += 1;
    ctx.data_mut().entries += 1;
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

/// The hook entry a shim calls. `orig` runs the engine's original body for the hooked function and
/// returns its result as an `i64` (0 for `()`); the return value is what the hooked function
/// should return (for the original's own result when the mod did not override it).
pub fn dispatch<T: 'static>(
    arg: &mut T,
    table: &'static FieldTable<T>,
    orig: &mut dyn FnMut(&mut T) -> i64,
) -> i64 {
    let ran = RT.with(|rt| {
        let pointer = rt.active.get();
        if !pointer.is_null() {
            // Nested: inside an `original` import (or the guest), the innermost context is published.
            // SAFETY: just read from `active`.
            return Some(run_hook(unsafe { context(pointer) }, &mut *arg, table, &mut *orig));
        }
        // Top level: this thread's instance publishes its store for the whole guest run.
        let Ok(mut guard) = rt.inst.try_borrow_mut() else { return None };
        let inst = guard.as_mut()?;
        if inst.store.data().disabled {
            return None;
        }
        let mut ctx = inst.store.as_context_mut();
        Some(with_active(rt, &mut ctx, |pointer| {
            // SAFETY: just published by `with_active`; `ctx` is not used again.
            run_hook(unsafe { context(pointer) }, &mut *arg, table, &mut *orig)
        }))
    });
    match ran {
        Some(value) => value,
        None => orig(arg),
    }
}

fn run_hook<T: 'static>(
    ctx: &mut StoreContextMut<'static, Host>,
    arg: &mut T,
    table: &'static FieldTable<T>,
    orig: &mut dyn FnMut(&mut T) -> i64,
) -> i64 {
    let Some(hook) = (if ctx.data().disabled { None } else { ctx.data().hook.clone() }) else {
        return orig(arg);
    };
    let mut cell = OrigCell::new(&mut *arg, &mut *orig);
    let act = Activation::open(&mut cell, table);
    let handle = act.handle(0);
    let saved = enter(ctx);
    let result = hook.call(&mut *ctx, handle as i64);
    leave(ctx, saved);
    match result {
        Ok(value) => {
            act.finish();
            value
        }
        Err(error) => {
            // The trap rule (MX3C0 section 5): disable the mod, run the original only if it has not
            // run in this activation, otherwise return what it returned; never call the guest again.
            let message = format!("{error:#}");
            let finished = act.finish();
            let host = ctx.data_mut();
            host.disabled = true;
            host.trap.get_or_insert(message);
            if finished.original_calls == 0 { orig(arg) } else { finished.last_result }
        }
    }
}
