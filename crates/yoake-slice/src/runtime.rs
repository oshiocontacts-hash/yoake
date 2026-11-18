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
use std::rc::Rc;
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
    /// The watchdog: an activation whose guest code runs longer than this many milliseconds (time inside
    /// `original` is not counted) is **logged**, once per activation, and keeps running; it is never trapped.
    /// Ignored when `epoch_ticks` is set (that budget traps). `None`: no watchdog.
    pub watchdog_ms: Option<u64>,
}

impl ModConfig {
    /// The limits of the slice's tests: 1 million fuel per activation, no epoch, no watchdog.
    pub fn new(hook_export: &str) -> Self {
        Self {
            hook_export: hook_export.to_owned(),
            fuel: Some(1_000_000),
            epoch_ticks: None,
            pause_epoch_in_original: true,
            watchdog_ms: None,
        }
    }

    /// The policy for a full-access mod (design 4b, `ENGINE_CALL_ABI.md`): a multiplexed guest, **no fuel
    /// limit and no trapping epoch** per activation, and a watchdog that only logs an activation whose guest
    /// time exceeds [`WATCHDOG_MS`]. `yoake_init` keeps its finite budget ([`INIT_FUEL`]).
    pub fn full_access() -> Self {
        Self { fuel: None, epoch_ticks: None, watchdog_ms: Some(WATCHDOG_MS), ..Self::new("") }
    }

    /// The deadline the epoch machinery works with (a trapping budget or the watchdog), in milliseconds.
    fn epoch_ms(&self) -> Option<u64> {
        self.epoch_ticks.or(self.watchdog_ms)
    }

    /// Milliseconds per epoch tick: 1 for a trapping budget, coarser for the logging watchdog (it only has to
    /// notice a slow activation, and a ticker thread that wakes every millisecond per mod would be a cost).
    fn tick_ms(&self) -> u64 {
        if self.epoch_ticks.is_some() { 1 } else { WATCHDOG_TICK_MS }
    }

    /// `ms` as a deadline in epoch ticks (at least one).
    fn ticks_for(&self, ms: u64) -> u64 {
        ms.div_ceil(self.tick_ms()).max(1)
    }
}

/// The full-access watchdog threshold: a slower activation is logged.
pub const WATCHDOG_MS: u64 = 50;
/// Period of the watchdog's ticker thread in milliseconds.
const WATCHDOG_TICK_MS: u64 = 5;

struct EpochAct {
    guest_used: u64,
    resumed_at: u64,
    /// The watchdog already logged this activation.
    logged: bool,
}

struct Host {
    hook: Option<TypedFunc<i64, i64>>,
    /// Multiplexed guests: one export `yoake_hook(hook_id: i32, handle: i64) -> i64`, called only for
    /// the ids the guest registered with `hook_register` in `yoake_init`.
    multi: Option<TypedFunc<(i32, i64), i64>>,
    /// hook id -> priority, as registered (shared with the slot, so chains can be built while the guest runs).
    registered: Rc<RefCell<std::collections::HashMap<u32, i32>>>,
    /// `hook_register` is accepted only while `yoake_init` runs.
    in_init: bool,
    /// This store's own innermost published context (null while its guest runs and outside imports).
    active: Rc<Cell<*mut StoreContextMut<'static, Host>>>,
    /// Mirror of `disabled` the slot can read while the instance is borrowed.
    dead: Rc<Cell<bool>>,
    memory: Option<Memory>,
    cfg: ModConfig,
    disabled: bool,
    trap: Option<String>,
    ticks: Arc<AtomicU64>,
    epoch: Vec<EpochAct>,
    reports: Vec<(i32, i64)>,
    entries: u64,
    max_fuel_used: u64,
    /// Activations the watchdog logged as slow, and the longest guest time seen when it fired (ms).
    slow_activations: u64,
    slowest_ms: u64,
    /// This mod's name (for log lines).
    name: String,
    depth: u32,
    /// The open activations of this instance, innermost last (their entry numbers).
    acts: Vec<u64>,
    /// Results that did not fit the guest's buffer, kept for `engine_take` (never re-run).
    tickets: Vec<Ticket>,
    next_ticket: u32,
}

impl Host {
    /// The trap rule's last step: never enter this mod again, and drop it from every chain.
    fn disable(&mut self) {
        self.disabled = true;
        self.dead.set(true);
        // chains are cached per hook id: a dead mod must leave them
        invalidate_chains();
    }
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

/// Identifies a loaded mod on its thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModId(pub u64);

/// One loaded full-access mod: its own store, its own published-context cell, and the few facts a chain
/// needs while the instance is borrowed (registered hooks, whether it is dead).
struct ModSlot {
    id: ModId,
    name: String,
    /// Load order: the tie-break between mods of equal priority.
    load_order: u64,
    inst: RefCell<ModInst>,
    active: Rc<Cell<*mut StoreContextMut<'static, Host>>>,
    dead: Rc<Cell<bool>>,
    registered: Rc<RefCell<std::collections::HashMap<u32, i32>>>,
}

/// All per-thread runtime state behind one thread-local, so a hook entry reads thread-local storage
/// once (on the windows-gnu toolchain one access costs about 15 ns).
struct Rt {
    /// Every mod loaded on this thread, in load order.
    mods: RefCell<Vec<Rc<ModSlot>>>,
    next_load: Cell<u64>,
    /// The chain of each hook id, built on first use (see [`chain_for`]). Valid while `generation` equals
    /// `chains_generation`; a load, an unload or a mod's death bumps `generation`.
    chains: RefCell<Option<std::collections::HashMap<u32, Chain>>>,
    generation: Cell<u64>,
    chains_generation: Cell<u64>,
}

/// The mods of one hook id in chain order (shared, so a hook entry clones one pointer).
type Chain = Rc<Vec<Rc<ModSlot>>>;

thread_local! {
    static RT: Rt = const {
        Rt {
            mods: RefCell::new(Vec::new()),
            next_load: Cell::new(1),
            chains: RefCell::new(None),
            generation: Cell::new(0),
            chains_generation: Cell::new(0),
        }
    };
}

/// Forgets the cached chains of this thread (a mod was loaded, unloaded or died).
fn invalidate_chains() {
    let _ = RT.try_with(|rt| rt.generation.set(rt.generation.get() + 1));
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

/// Publishes `ctx` as this store's innermost context while `f` runs. Returns the pointer `f` is given:
/// all use of the context from here on goes through it (not through `ctx`), as in C0.
fn with_active<T>(
    cell: &Cell<*mut StoreContextMut<'static, Host>>,
    ctx: &mut StoreContextMut<'_, Host>,
    f: impl FnOnce(*mut StoreContextMut<'static, Host>) -> T,
) -> T {
    let pointer = (ctx as *mut StoreContextMut<'_, Host>).cast::<StoreContextMut<'static, Host>>();
    let _restore = ActiveGuard { cell, previous: cell.replace(pointer) };
    f(pointer)
}

/// # Safety
/// `pointer` is the innermost context published by [`with_active`] (the contract of every caller:
/// it was just read from the slot's `active` or just published), and no other reference to it is live.
unsafe fn context<'x>(pointer: *mut StoreContextMut<'static, Host>) -> &'x mut StoreContextMut<'static, Host> {
    // SAFETY: caller contract. The native code between the publishing frame and this call does not
    // use the `Caller` it came from (module docs).
    unsafe { &mut *pointer }
}

/// The export of a multiplexed guest: `(hook_id: i32, handle: i64) -> i64`.
pub const HOOK_EXPORT: &str = "yoake_hook";
/// The optional init export of a multiplexed guest: `() -> ()`, calls `hook_register`.
pub const INIT_EXPORT: &str = "yoake_init";

/// Hook ids at or above this are engine-synthetic hooks (not S2 slots): see `docs/yoake2/ENGINE_CALL_ABI.md`.
pub const SYNTHETIC_HOOK_BASE: u32 = 0x8000_0000;
/// `Game::yoake2_server_tick`, the one hook whose argument 0 is the `Game` root.
pub const HOOK_SERVER_TICK: u32 = SYNTHETIC_HOOK_BASE + 1;

/// The unmetered store's starting fuel; every top-level activation must leave it unchanged.
pub const INITIAL_FUEL: u64 = u64::MAX / 4;

/// Most hook ids one mod may register.
pub const MAX_REGISTRATIONS: usize = 4096;
/// Fuel `yoake_init` may burn, and its epoch budget in 1 ms ticks (a finite budget for the one call that
/// is not inside an activation).
pub const INIT_FUEL: u64 = 50_000_000;
pub const INIT_EPOCH_TICKS: u64 = 2000;

/// `hook_register` results (negative = refused).
pub mod register_status {
    pub const OK: i32 = 0;
    /// Called outside `yoake_init`.
    pub const NOT_IN_INIT: i32 = -1;
    /// This mod already registered [`super::MAX_REGISTRATIONS`] ids.
    pub const TOO_MANY: i32 = -2;
    /// Hook id 0 belongs to the legacy single-export API.
    pub const RESERVED_ID: i32 = -3;
}

/// Loads a mod (WAT text or a wasm binary) next to the ones already on this thread. It runs after them in
/// every chain of equal priority. `name` is for `stats`. Fails (instead of panicking) when `yoake_init`
/// traps or runs out of its budget; nothing is left loaded then.
pub fn load(wasm_or_wat: &[u8], cfg: ModConfig, name: &str) -> R<ModId> {
    let mut config = Config::new();
    // fuel is always on: `yoake_init` needs a finite budget even for an unmetered mod
    config.consume_fuel(true).epoch_interruption(cfg.epoch_ms().is_some());
    let engine = Engine::new(&config)?;
    let module = Module::new(&engine, wasm_or_wat)?;
    let ticks = Arc::new(AtomicU64::new(0));
    let ticker = cfg.epoch_ms().map(|_| {
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, counter, engine) = (stop.clone(), ticks.clone(), engine.clone());
        // `ticks` counts milliseconds; one epoch tick is `period` of them
        let period = cfg.tick_ms();
        let thread = std::thread::spawn(move || {
            while !flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(period));
                counter.fetch_add(period, Ordering::Relaxed);
                engine.increment_epoch();
            }
        });
        Ticker { stop, thread: Some(thread) }
    });
    let linker = build_linker(&engine)?;
    let export = cfg.hook_export.clone();
    let registered = Rc::new(RefCell::new(std::collections::HashMap::new()));
    let active = Rc::new(Cell::new(std::ptr::null_mut()));
    let dead = Rc::new(Cell::new(false));
    let host = Host {
        hook: None,
        multi: None,
        registered: registered.clone(),
        in_init: false,
        active: active.clone(),
        dead: dead.clone(),
        memory: None,
        cfg,
        disabled: false,
        trap: None,
        ticks,
        epoch: Vec::new(),
        reports: Vec::new(),
        entries: 0,
        max_fuel_used: 0,
        slow_activations: 0,
        slowest_ms: 0,
        name: name.to_owned(),
        depth: 0,
        acts: Vec::new(),
        tickets: Vec::new(),
        next_ticket: 0,
    };
    let mut store = Store::new(&engine, host);
    store.set_fuel(INITIAL_FUEL)?;
    if store.data().cfg.epoch_ms().is_some() {
        store.set_epoch_deadline(u64::MAX / 4);
    }
    if store.data().cfg.epoch_ticks.is_none() && store.data().cfg.watchdog_ms.is_some() {
        // the watchdog: an expired deadline logs and goes on, it never traps
        store.epoch_deadline_callback(watchdog_fired);
    }
    let instance = linker.instantiate(&mut store, &module)?;
    let memory = instance.get_memory(&mut store, "memory");
    store.data_mut().memory = memory;
    // A multiplexed guest exports `yoake_hook`; its optional `yoake_init` registers the hook ids it wants.
    // Anything else is the one-export guest of the slice (`ModConfig::hook_export`): hook id 0, which is
    // reserved for it (a multiplexed guest cannot register 0).
    match instance.get_typed_func::<(i32, i64), i64>(&mut store, HOOK_EXPORT) {
        Ok(multi) => {
            store.data_mut().multi = Some(multi);
            if let Ok(init) = instance.get_typed_func::<(), ()>(&mut store, INIT_EXPORT) {
                // a finite budget for the one guest call that is not inside an activation
                store.set_fuel(INIT_FUEL)?;
                if store.data().cfg.epoch_ticks.is_some() {
                    store.set_epoch_deadline(INIT_EPOCH_TICKS);
                }
                store.data_mut().in_init = true;
                let result = init.call(&mut store, ());
                store.data_mut().in_init = false;
                result.map_err(|e| Error::msg(format!("yoake_init failed: {e:#}")))?;
                store.set_fuel(INITIAL_FUEL)?;
                if store.data().cfg.epoch_ms().is_some() {
                    store.set_epoch_deadline(u64::MAX / 4);
                }
            }
        }
        Err(_) => {
            let hook = instance.get_typed_func::<i64, i64>(&mut store, &export)?;
            store.data_mut().hook = Some(hook);
            registered.borrow_mut().insert(0, 0);
        }
    }
    RT.with(|rt| {
        let load_order = rt.next_load.get();
        rt.next_load.set(load_order + 1);
        let id = ModId(load_order);
        let slot = Rc::new(ModSlot {
            id,
            name: name.to_owned(),
            load_order,
            inst: RefCell::new(ModInst { store, _ticker: ticker }),
            active,
            dead,
            registered,
        });
        match rt.mods.try_borrow_mut() {
            Ok(mut mods) => {
                mods.push(slot);
                drop(mods);
                invalidate_chains();
                Ok(id)
            }
            Err(_) => Err(Error::msg("load: the mod list is in use")),
        }
    })
}

/// Unloads one mod. Fails when its guest is running (called from inside one of its hooks).
pub fn unload(id: ModId) -> R<()> {
    RT.with(|rt| {
        let mut mods = rt.mods.try_borrow_mut().map_err(|_| Error::msg("unload: the mod list is in use"))?;
        let Some(index) = mods.iter().position(|m| m.id == id) else { return Ok(()) };
        if mods[index].inst.try_borrow_mut().is_err() {
            return Err(Error::msg("unload: this mod's guest is running (called from inside its hook)"));
        }
        mods.remove(index);
        drop(mods);
        invalidate_chains();
        Ok(())
    })
}

/// Installs a single mod on the calling thread, replacing every earlier one (the convenience of the
/// slice's tests). Fails when any guest is running.
pub fn install(wasm_or_wat: &[u8], cfg: ModConfig) -> R<()> {
    uninstall()?;
    load(wasm_or_wat, cfg, "mod").map(|_| ())
}

/// Drops every mod of the calling thread. Fails when one of their guests is running.
#[must_use = "uninstall fails when called from inside a hook"]
pub fn uninstall() -> R<()> {
    RT.with(|rt| {
        let mut mods = rt.mods.try_borrow_mut().map_err(|_| Error::msg("uninstall: the mod list is in use"))?;
        if mods.iter().any(|m| m.inst.try_borrow_mut().is_err()) {
            return Err(Error::msg("uninstall: this thread's guest is running (called from inside a hook)"));
        }
        mods.clear();
        drop(mods);
        invalidate_chains();
        Ok(())
    })
}

/// What a mod did so far.
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
    /// `(hook id, priority)` the guest registered, sorted by id.
    pub registered: Vec<(u32, i32)>,
    /// Activations the watchdog logged as slow (guest time over `ModConfig::watchdog_ms`).
    pub slow_activations: u64,
    /// The longest guest time (ms) the watchdog saw when it fired; 0 when it never did.
    pub slowest_ms: u64,
}

fn slot_stats(slot: &ModSlot) -> Stats {
    let mut registered: Vec<(u32, i32)> = slot.registered.borrow().iter().map(|(k, v)| (*k, *v)).collect();
    registered.sort_unstable();
    match slot.inst.try_borrow() {
        Ok(inst) => {
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
                registered,
                slow_activations: host.slow_activations,
                slowest_ms: host.slowest_ms,
            }
        }
        Err(_) => Stats { installed: true, busy: true, disabled: slot.dead.get(), registered, ..Stats::default() },
    }
}

/// The first loaded mod's stats (the slice's single-mod view); `Stats::default()` when none is loaded.
pub fn stats() -> Stats {
    RT.with(|rt| rt.mods.borrow().first().map(|slot| slot_stats(slot)).unwrap_or_default())
}

/// The stats of one mod.
pub fn stats_of(id: ModId) -> Option<Stats> {
    RT.with(|rt| rt.mods.borrow().iter().find(|m| m.id == id).map(|slot| slot_stats(slot)))
}

/// `(id, name, load order, stats)` of every mod on this thread, in load order.
pub fn all_stats() -> Vec<(ModId, String, Stats)> {
    RT.with(|rt| rt.mods.borrow().iter().map(|slot| (slot.id, slot.name.clone(), slot_stats(slot))).collect())
}

fn take_slot_reports(slot: &ModSlot) -> R<Vec<(i32, i64)>> {
    match slot.inst.try_borrow_mut() {
        Ok(mut inst) => Ok(std::mem::take(&mut inst.store.data_mut().reports)),
        Err(_) => Err(Error::msg("take_reports: this mod's guest is running (called from inside a hook)")),
    }
}

/// The `report(slot, value)` calls of every mod since the last call, in load order. Fails from inside a hook.
pub fn take_reports() -> R<Vec<(i32, i64)>> {
    RT.with(|rt| {
        let mods = rt.mods.borrow();
        let mut all = Vec::new();
        for slot in mods.iter() {
            all.extend(take_slot_reports(slot)?);
        }
        Ok(all)
    })
}

/// The reports of one mod.
pub fn take_reports_of(id: ModId) -> R<Vec<(i32, i64)>> {
    RT.with(|rt| match rt.mods.borrow().iter().find(|m| m.id == id) {
        Some(slot) => take_slot_reports(slot),
        None => Ok(Vec::new()),
    })
}

/// Runs engine code on behalf of an import: republishes this call's own context as the innermost
/// one, so a nested hook (from `original`, or from a field accessor) uses it, and pauses the
/// outer activation's epoch budget for the duration, restoring the outer deadline afterwards (a
/// nested hook sets its own deadline and must not leave it behind). Every import that can reach
/// engine code goes through here; use [`engine_import!`] to register one so it cannot be forgotten.
fn native<T>(caller: &mut Caller<'_, Host>, f: impl FnOnce() -> T) -> T {
    let epoch_on = caller.data().cfg.epoch_ms().is_some() && caller.data().cfg.pause_epoch_in_original;
    if epoch_on {
        // the guest's time so far in this activation is charged, the engine's is not
        let now = caller.data().ticks.load(Ordering::Relaxed);
        if let Some(top) = caller.data_mut().epoch.last_mut() {
            top.guest_used += now.saturating_sub(top.resumed_at);
        }
    }
    let out = {
        let active = caller.data().active.clone();
        let mut ctx = caller.as_context_mut();
        with_active(&active, &mut ctx, |_| f())
    };
    if epoch_on {
        let now = caller.data().ticks.load(Ordering::Relaxed);
        let budget = caller.data().cfg.epoch_ms().unwrap_or(1);
        let mut remaining = budget;
        if let Some(top) = caller.data_mut().epoch.last_mut() {
            top.resumed_at = now;
            remaining = budget.saturating_sub(top.guest_used).max(1);
        }
        let ticks = caller.data().cfg.ticks_for(remaining);
        caller.as_context_mut().set_epoch_deadline(ticks);
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
/// The largest ticket id: `-(OUT_TOO_SMALL_BASE + id)` must fit an `i32`.
pub const MAX_TICKET_ID: u32 = (i32::MAX - call_status::OUT_TOO_SMALL_BASE) as u32;
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
    // the status is `-(256 + id)` in an i32: stop issuing ids before it would overflow (never wrap)
    if host.next_ticket >= MAX_TICKET_ID {
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
    // hook_register(id, priority) -> i32: a multiplexed guest says which hooks it wants (host state only).
    linker.func_wrap("env", "hook_register", |c: Caller<'_, Host>, id: i32, priority: i32| -> i32 {
        let host = c.data();
        if !host.in_init {
            return register_status::NOT_IN_INIT;
        }
        if id == 0 {
            return register_status::RESERVED_ID;
        }
        let mut registered = host.registered.borrow_mut();
        if registered.len() >= MAX_REGISTRATIONS && !registered.contains_key(&(id as u32)) {
            return register_status::TOO_MANY;
        }
        registered.insert(id as u32, priority);
        register_status::OK
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
    if let Some(budget) = ctx.data().cfg.epoch_ms() {
        let now = ctx.data().ticks.load(Ordering::Relaxed);
        ctx.data_mut().epoch.push(EpochAct { guest_used: 0, resumed_at: now, logged: false });
        let ticks = ctx.data().cfg.ticks_for(budget);
        ctx.set_epoch_deadline(ticks);
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
    } else if ctx.data().cfg.fuel.is_none() && ctx.data().depth == 0 {
        // unmetered: a top-level activation starts from a full tank, so there is no cumulative limit either
        ctx.set_fuel(INITIAL_FUEL).ok();
    }
    if ctx.data().cfg.epoch_ms().is_some() {
        ctx.data_mut().epoch.pop();
    }
}

/// The watchdog's epoch callback: an activation's guest time went over `watchdog_ms`. Logs it (once per
/// activation), counts it, and lets the guest go on with a fresh deadline: it never traps.
fn watchdog_fired(mut ctx: StoreContextMut<'_, Host>) -> R<wasmtime::UpdateDeadline> {
    let threshold = ctx.data().cfg.watchdog_ms.unwrap_or(WATCHDOG_MS);
    let again = ctx.data().cfg.ticks_for(threshold);
    let now = ctx.data().ticks.load(Ordering::Relaxed);
    let host = ctx.data_mut();
    let Some(top) = host.epoch.last_mut() else {
        // not inside an activation (`yoake_init`, which has its own fuel budget): nothing to report
        return Ok(wasmtime::UpdateDeadline::Continue(again));
    };
    let used = top.guest_used + now.saturating_sub(top.resumed_at);
    let first = !top.logged;
    top.logged = true;
    host.slowest_ms = host.slowest_ms.max(used);
    if first {
        host.slow_activations += 1;
        eprintln!(
            "yoake: mod {}: a hook activation has run {used} ms of guest time (watchdog: {threshold} ms); it is not stopped",
            host.name
        );
    }
    Ok(wasmtime::UpdateDeadline::Continue(again))
}

enum Entry {
    Single(TypedFunc<i64, i64>),
    Multi(TypedFunc<(i32, i64), i64>),
}

enum Outcome {
    /// The guest's return value.
    Value(i64),
    /// No hook ran (no instance on this thread, disabled, or the guest is already running here).
    Fallthrough,
    /// The guest trapped; the mod is now disabled.
    Trapped(Finished),
}

/// Deepest guest nesting (**per mod**: each mod has its own depth counter, so a chain of several mods can
/// nest deeper in total) (hook -> import -> engine -> hook ...). Past it a hooked call falls
/// through to the original body and the guest is not entered, so guest-driven recursion (for
/// example through a field accessor) cannot exhaust the native stack. 16, not more: every level is a
/// wasm entry plus host frames under wasmtime's one stack budget, which ran out at depth 58 in a debug
/// build (a clean "call stack exhausted" trap, but the cap should come first).
pub const MAX_DEPTH: u32 = 16;

type ScopeFn<'s> = &'s mut dyn FnMut(u64) -> R<i64>;

/// The mods registered for `hook_id`, in chain order: `(priority, load order)`, lowest first. The first is
/// the outermost wrapper (its `original` runs the next mod); a dead mod is not in the chain.
///
/// Chains are cached per hook id (hot path: every hooked engine call asks): the cache is dropped when a mod is
/// loaded or unloaded or dies (`invalidate_chains`). A mod's registrations only change inside its `yoake_init`,
/// before it is in the list, so they cannot make a cached chain stale.
fn chain_for(hook_id: u32) -> Option<Chain> {
    RT.with(|rt| {
        // no mod loaded (the common case): nothing to look up, nothing to allocate
        if rt.mods.borrow().is_empty() {
            return None;
        }
        if rt.chains_generation.get() != rt.generation.get() {
            *rt.chains.borrow_mut() = None;
            rt.chains_generation.set(rt.generation.get());
        }
        if let Some(chain) = rt.chains.borrow().as_ref().and_then(|cache| cache.get(&hook_id)) {
            return Some(chain.clone());
        }
        let mods = rt.mods.borrow();
        let mut found: Vec<(i32, u64, Rc<ModSlot>)> = mods
            .iter()
            .filter(|m| !m.dead.get())
            .filter_map(|m| m.registered.borrow().get(&hook_id).map(|priority| (*priority, m.load_order, m.clone())))
            .collect();
        found.sort_by_key(|(priority, order, _)| (*priority, *order));
        let chain: Chain = Rc::new(found.into_iter().map(|(_, _, slot)| slot).collect());
        rt.chains.borrow_mut().get_or_insert_with(Default::default).insert(hook_id, chain.clone());
        Some(chain)
    })
}

/// The first live mod of `chain` at or after `from`.
fn next_live(chain: &[Rc<ModSlot>], from: usize) -> Option<usize> {
    (from..chain.len()).find(|i| !chain[*i].dead.get())
}

/// The hook entry a shim calls. `orig` runs the engine's original body for the hooked function and
/// returns its result as an `i64` (0 for `()`); the return value is what the hooked function
/// should return (for the original's own result when the mod did not override it). With several mods the
/// chain is composed: each mod's `original` runs the next one, the last runs `orig`.
pub fn dispatch<T: 'static>(arg: &mut T, table: &'static FieldTable<T>, orig: &mut dyn FnMut(&mut T) -> i64) -> i64 {
    dispatch_id(0, arg, table, orig)
}

/// [`dispatch`] for hook `hook_id`: a multiplexed guest is entered only if it registered that id.
pub fn dispatch_id<T: 'static>(hook_id: u32, arg: &mut T, table: &'static FieldTable<T>, orig: &mut dyn FnMut(&mut T) -> i64) -> i64 {
    let Some(chain) = chain_for(hook_id).filter(|c| !c.is_empty()) else { return orig(arg) };
    chain_step(&chain, 0, hook_id, arg, table, orig)
}

fn chain_step<T: 'static>(chain: &[Rc<ModSlot>], from: usize, hook_id: u32, arg: &mut T, table: &'static FieldTable<T>, orig: &mut dyn FnMut(&mut T) -> i64) -> i64 {
    let Some(i) = next_live(chain, from) else { return orig(arg) };
    let mut next = |a: &mut T| chain_step(chain, i + 1, hook_id, a, table, &mut *orig);
    let outcome = dispatch_with(&chain[i], hook_id, |body| activation::scope(&mut *arg, table, &mut next, |h| body(h)));
    match outcome {
        Outcome::Value(value) => value,
        Outcome::Fallthrough => next(arg),
        Outcome::Trapped(done) => {
            // the trapped mod is out of every chain now; the rest of the chain (and the engine) still run
            if done.original_calls == 0 { next(arg) } else { done.last_result }
        }
    }
}

/// [`dispatch`] for a hook whose argument 0 is a path root (`Game`): the guest gets the root handle, and
/// through it path handles to the objects the root can reach (`activation::open_path`, generated
/// "get handle" bindings). Only a function that really receives the root may use this: inside any other
/// hook the caller holds an outer `&mut` and there is no root to re-borrow from (design 4b).
pub fn dispatch_root<G: activation::PathRoot>(root: &mut G, orig: &mut dyn FnMut(&mut G) -> i64) -> i64 {
    dispatch_root_id(0, root, orig)
}

/// [`dispatch_root`] for hook `hook_id` (the server tick is [`HOOK_SERVER_TICK`]).
pub fn dispatch_root_id<G: activation::PathRoot>(hook_id: u32, root: &mut G, orig: &mut dyn FnMut(&mut G) -> i64) -> i64 {
    let Some(chain) = chain_for(hook_id).filter(|c| !c.is_empty()) else { return orig(root) };
    chain_step_root(&chain, 0, hook_id, root, orig)
}

fn chain_step_root<G: activation::PathRoot>(chain: &[Rc<ModSlot>], from: usize, hook_id: u32, root: &mut G, orig: &mut dyn FnMut(&mut G) -> i64) -> i64 {
    let Some(i) = next_live(chain, from) else { return orig(root) };
    let mut next = |g: &mut G| chain_step_root(chain, i + 1, hook_id, g, &mut *orig);
    let outcome = dispatch_with(&chain[i], hook_id, |body| activation::scope_root(&mut *root, &mut next, |h| body(h)));
    match outcome {
        Outcome::Value(value) => value,
        Outcome::Fallthrough => next(root),
        Outcome::Trapped(done) => {
            if done.original_calls == 0 { next(root) } else { done.last_result }
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
    let Some(chain) = chain_for(0).filter(|c| !c.is_empty()) else { return orig(a, b) };
    chain_step2(&chain, 0, a, table_a, b, table_b, orig)
}

fn chain_step2<A: 'static, B: 'static>(
    chain: &[Rc<ModSlot>],
    from: usize,
    a: &mut A,
    table_a: &'static FieldTable<A>,
    b: &mut B,
    table_b: &'static FieldTable<B>,
    orig: &mut dyn FnMut(&mut A, &mut B) -> i64,
) -> i64 {
    let Some(i) = next_live(chain, from) else { return orig(a, b) };
    let mut next = |x: &mut A, y: &mut B| chain_step2(chain, i + 1, x, table_a, y, table_b, &mut *orig);
    let outcome = dispatch_with(&chain[i], 0, |body| activation::scope2(&mut *a, table_a, &mut *b, table_b, &mut next, |h| body(h)));
    match outcome {
        Outcome::Value(value) => value,
        Outcome::Fallthrough => next(a, b),
        Outcome::Trapped(done) => {
            if done.original_calls == 0 { next(a, b) } else { done.last_result }
        }
    }
}

/// Enters one mod's guest for one hook, on this mod's own store.
fn dispatch_with(slot: &ModSlot, hook_id: u32, scope: impl FnOnce(ScopeFn<'_>) -> (R<i64>, Finished)) -> Outcome {
    let pointer = slot.active.get();
    if !pointer.is_null() {
        // Inside an import of this very store that published its context (`native`): a nested hook.
        // SAFETY: just read from this store's `active`.
        return run_hook(unsafe { context(pointer) }, hook_id, scope);
    }
    // Top level for this store: it publishes its context for the whole guest run. If its guest is already
    // running (its `active` is null while it runs, so this is engine code reached from guest code outside
    // a `native` import), the instance is borrowed: no hook.
    let Ok(mut inst) = slot.inst.try_borrow_mut() else { return Outcome::Fallthrough };
    if slot.dead.get() {
        return Outcome::Fallthrough;
    }
    let mut ctx = inst.store.as_context_mut();
    with_active(&slot.active, &mut ctx, |pointer| {
        // SAFETY: just published by `with_active`; `ctx` is not used again.
        run_hook(unsafe { context(pointer) }, hook_id, scope)
    })
}

fn run_hook(ctx: &mut StoreContextMut<'static, Host>, hook_id: u32, scope: impl FnOnce(ScopeFn<'_>) -> (R<i64>, Finished)) -> Outcome {
    if ctx.data().disabled {
        return Outcome::Fallthrough;
    }
    let entry = match (ctx.data().multi.clone(), ctx.data().hook.clone()) {
        (Some(multi), _) if ctx.data().registered.borrow().contains_key(&hook_id) => Entry::Multi(multi),
        (Some(_), _) => return Outcome::Fallthrough,
        (None, Some(hook)) => Entry::Single(hook),
        (None, None) => return Outcome::Fallthrough,
    };
    if ctx.data().depth >= MAX_DEPTH {
        return Outcome::Fallthrough;
    }
    let active = ctx.data().active.clone();
    let (result, finished) = scope(&mut |handle| {
        let saved = enter(ctx);
        // The guest runs with no published context: only `native` imports may reach engine code.
        let hidden = ActiveGuard { cell: &active, previous: active.replace(std::ptr::null_mut()) };
        // A panic from the engine body inside `original` unwinds through wasmtime to here: restore
        // the per-activation state (fuel, epoch stack, depth) before letting it continue.
        let called = catch_unwind(AssertUnwindSafe(|| match &entry {
            Entry::Single(hook) => hook.call(&mut *ctx, handle as i64),
            Entry::Multi(multi) => multi.call(&mut *ctx, (hook_id as i32, handle as i64)),
        }));
        drop(hidden);
        leave(ctx, saved);
        match called {
            Ok(result) => result,
            Err(payload) => {
                // The guest's frames were abandoned mid-run (its shadow stack and allocator may be
                // corrupt): same rule as a trap, never enter this instance again. Then re-throw.
                let host = ctx.data_mut();
                host.disable();
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
            host.disable();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_status_always_fits_an_i32() {
        assert!(i64::from(MAX_TICKET_ID) + i64::from(call_status::OUT_TOO_SMALL_BASE) <= i64::from(i32::MAX));
        let status = -(call_status::OUT_TOO_SMALL_BASE + MAX_TICKET_ID as i32);
        assert_eq!(call_status::ticket_of(status), Some(MAX_TICKET_ID));
        assert_eq!(call_status::ticket_of(-2), None);
        assert_eq!(call_status::ticket_of(0), None);
    }
}
