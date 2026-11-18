use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use wasmtime::*;

type R<T> = std::result::Result<T, Error>;

/// Per-store host state ("engine state" in the real design).
struct Host {
    hook_f: Option<TypedFunc<i32, i32>>,
    hook_g: Option<TypedFunc<i32, i32>>,
    hook_h: Option<TypedFunc<i32, i32>>,
    hook_p: Option<TypedFunc<(i32, i32), i32>>,
    memory: Option<Memory>,
    trace: Vec<String>,
    depth: u32,
    side_effects: u32,
    disabled: bool,
    abort_policy: bool, // false = A (finish outer with original G), true = B (abort outer)
    pending: Option<String>,
    // fuel
    fuel_budget: Option<u64>,
    nested_fuel_swap: bool,
    // epoch
    epoch_budget_ticks: Option<u64>,
    epoch_pause: bool,
    ticks: Arc<AtomicU64>,
    hook_start_tick: u64,
    acts: Vec<(u32, i32)>, // per hook activation: (times original ran, last original result)
}

impl Host {
    fn new(ticks: Arc<AtomicU64>) -> Self {
        Host { hook_f: None, hook_g: None, hook_h: None, hook_p: None, memory: None, trace: vec![], depth: 0,
            side_effects: 0, disabled: false, abort_policy: false, pending: None, fuel_budget: None,
            nested_fuel_swap: true, epoch_budget_ticks: None, epoch_pause: true, ticks, hook_start_tick: 0, acts: vec![] }
    }
}

// ---------- the one piece of unsafe: the ACTIVE store pointer (like crates/yoake-wasm ACTIVE) ----------
thread_local! {
    static ACTIVE: Cell<*mut StoreContextMut<'static, Host>> = const { Cell::new(std::ptr::null_mut()) };
}

/// Run `f` with ACTIVE pointing at `ctx`. Nested calls (host import -> native engine code) re-publish
/// the Caller's own context, so the pointer is always the innermost live one.
fn with_active<T>(ctx: &mut StoreContextMut<'_, Host>, f: impl FnOnce() -> T) -> T {
    let p = ctx as *mut StoreContextMut<'_, Host> as *mut StoreContextMut<'static, Host>;
    let prev = ACTIVE.with(|a| a.replace(p));
    let out = f();
    ACTIVE.with(|a| a.set(prev));
    out
}
fn active<T>(f: impl FnOnce(&mut StoreContextMut<'static, Host>) -> T) -> T {
    let p = ACTIVE.with(|a| a.get());
    assert!(!p.is_null(), "no active store");
    // SAFETY: p was published by with_active() higher up this same thread's stack, and the context
    // it points to is not otherwise used while the native code below it runs.
    f(unsafe { &mut *p })
}

// ---------- "engine" functions with hook slots ----------
fn engine_f(x: i32) -> i32 {
    if let Some(h) = active(|c| if c.data().disabled { None } else { c.data().hook_f.clone() }) {
        dispatch(|c| h.call(c, x), x, real_f)
    } else { real_f(x) }
}
fn real_f(x: i32) -> i32 {
    active(|c| c.data_mut().side_effects += 1);
    x * 10 + engine_g(x + 1)
}
fn engine_g(x: i32) -> i32 {
    if let Some(h) = active(|c| if c.data().disabled { None } else { c.data().hook_g.clone() }) {
        dispatch(|c| h.call(c, x), x, real_g)
    } else { real_g(x) }
}
fn real_g(x: i32) -> i32 { x * 2 }

/// Calls a guest hook with trace/fuel/trap handling; on trap, applies the "disable mod" rule.
fn dispatch(call: impl FnOnce(&mut StoreContextMut<'static, Host>) -> R<i32>, x: i32, real: fn(i32) -> i32) -> i32 {
    // fuel: give each hook activation its own budget; restore the caller's remainder afterwards
    let saved_fuel = active(|c| {
        c.data_mut().depth += 1;
        let depth = c.data().depth;
        c.data_mut().trace.push(format!("{}host -> guest hook (x={})", "  ".repeat(depth as usize - 1), x));
        let mut saved = None;
        if let Some(b) = c.data().fuel_budget {
            if c.data().nested_fuel_swap || c.data().depth == 1 {
                saved = c.get_fuel().ok();
                c.set_fuel(b).ok();
            }
        }
        saved
    });
    active(|c| c.data_mut().acts.push((0, 0)));
    let res = active(|c| call(c));
    let act = active(|c| c.data_mut().acts.pop().unwrap());
    active(|c| {
        c.data_mut().depth -= 1;
        if let (Some(s), Some(b)) = (saved_fuel, c.data().fuel_budget) {
            let used = b.saturating_sub(c.get_fuel().unwrap_or(0));
            let d = c.data().depth;
            c.data_mut().trace.push(format!("{}hook activation used {} fuel (budget {})", "  ".repeat(d as usize), used, b));
            c.set_fuel(s).ok();
        }
    });
    match res {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("{}", e.root_cause());
            active(|c| {
                let d = c.data_mut();
                let what = if act.0 == 0 { "original not yet run -> running original body" } else { "original already ran -> returning its recorded result, NOT re-running it" };
                d.trace.push(format!("TRAP in hook (x={}): {} -> mod disabled; {}", x, msg, what));
                d.disabled = true;
                d.pending.get_or_insert(msg);
            });
            if act.0 == 0 { real(x) } else { act.1 }
        }
    }
}

// ---------- host imports (called by the guest) ----------
fn enter_native<T>(caller: &mut Caller<'_, Host>, f: impl FnOnce() -> T) -> T {
    let mut ctx = caller.as_context_mut();
    with_active(&mut ctx, f)
}

fn build_linker(engine: &Engine) -> R<Linker<Host>> {
    let mut l = Linker::new(engine);
    l.func_wrap("env", "log", |mut c: Caller<'_, Host>, tag: i32, v: i32| {
        let d = c.data_mut();
        let ind = "  ".repeat(d.depth as usize);
        let name = ["", "hook_f enter", "hook_f after original (frame intact)", "hook_g enter", "hook_g after original (frame intact)"][tag as usize];
        d.trace.push(format!("{}guest: {} v={}", ind, name, v));
    })?;
    l.func_wrap("env", "original_f", |mut c: Caller<'_, Host>, x: i32| -> R<i32> {
        let ind = "  ".repeat(c.data().depth as usize);
        c.data_mut().trace.push(format!("{}guest -> host original_f({})", ind, x));
        let r = enter_native(&mut c, || real_f(x));
        if let Some(a) = c.data_mut().acts.last_mut() { a.0 += 1; a.1 = r; }
        if c.data().abort_policy && c.data().pending.is_some() {
            return Err(Error::msg("original_f: nested hook trapped, aborting outer activation"));
        }
        Ok(r)
    })?;
    l.func_wrap("env", "original_g", |mut c: Caller<'_, Host>, x: i32| -> i32 {
        let ind = "  ".repeat(c.data().depth as usize);
        c.data_mut().trace.push(format!("{}guest -> host original_g({})", ind, x));
        enter_native(&mut c, || real_g(x))
    })?;
    // original with epoch pause: charge only guest time to the hook
    l.func_wrap("env", "original_h", |mut c: Caller<'_, Host>, x: i32| -> i32 {
        if let Some(t) = c.data().epoch_budget_ticks {
            let used_before = c.data().ticks.load(Ordering::Relaxed) - c.data().hook_start_tick;
            // slow engine body (wall clock): 60 ms
            std::thread::sleep(Duration::from_millis(60));
            if c.data().epoch_pause {
                // restore: remaining = budget - guest time used so far, counted from now
                c.as_context_mut().set_epoch_deadline(t.saturating_sub(used_before).max(1));
            }
        }
        x + 1
    })?;
    l.func_wrap("env", "original_p", |c: Caller<'_, Host>, ptr: i32, len: i32| -> i32 {
        let m = c.data().memory.unwrap();
        let data = m.data(&c);
        let s = &data[ptr as usize..(ptr + len) as usize];
        let mut x = 0u8;
        for b in s { x ^= *b; }
        x as i32 + len
    })?;
    Ok(l)
}

struct Env { store: Store<Host>, inst: Instance }

fn make_core(fuel: bool, epoch: bool) -> R<(Engine, Env, Arc<AtomicU64>)> {
    let mut cfg = Config::new();
    cfg.consume_fuel(fuel).epoch_interruption(epoch);
    let engine = Engine::new(&cfg)?;
    let module = Module::new(&engine, include_str!("core.wat"))?;
    let ticks = Arc::new(AtomicU64::new(0));
    let linker = build_linker(&engine)?;
    let mut store = Store::new(&engine, Host::new(ticks.clone()));
    if fuel { store.set_fuel(u64::MAX / 4)?; }
    if epoch { store.set_epoch_deadline(u64::MAX / 4); }
    let inst = linker.instantiate(&mut store, &module)?;
    store.data_mut().hook_f = Some(inst.get_typed_func(&mut store, "hook_f")?);
    store.data_mut().hook_g = Some(inst.get_typed_func(&mut store, "hook_g")?);
    store.data_mut().hook_h = Some(inst.get_typed_func(&mut store, "hook_h")?);
    store.data_mut().hook_p = Some(inst.get_typed_func(&mut store, "hook_p")?);
    store.data_mut().memory = inst.get_memory(&mut store, "memory");
    Ok((engine, Env { store, inst }, ticks))
}

fn top_call_f(env: &mut Env, x: i32) -> i32 {
    let mut ctx = env.store.as_context_mut();
    with_active(&mut ctx, || engine_f(x))
}
fn print_trace(env: &mut Env) {
    for l in env.store.data_mut().trace.drain(..) { println!("    {l}"); }
}
fn sp(env: &mut Env) -> i32 {
    match env.inst.get_global(&mut env.store, "sp").unwrap().get(&mut env.store) { Val::I32(v) => v, _ => 0 }
}

fn test1() -> R<()> {
    println!("== 1. core module re-entry ==");
    let (_e, mut env, _) = make_core(false, false)?;
    println!("  real F(3) = 3*10 + G(4)=8 -> 38; with hooks: hook_f(3) = 1000 + (30 + hook_g(4) = 8+1) -> expect 1039");
    let r = top_call_f(&mut env, 3);
    print_trace(&mut env);
    println!("  result = {r} (expected 1039); guest sp after = {} (expected 65536); depth={}", sp(&mut env), env.store.data().depth);
    assert_eq!(r, 1039);
    assert_eq!(sp(&mut env), 65536);
    Ok(())
}

fn test2() -> R<()> {
    println!("== 2. component re-entry ==");
    let mut cfg = Config::new();
    cfg.wasm_component_model(true);
    let engine = Engine::new(&cfg)?;
    let comp = component::Component::new(&engine, include_str!("comp.wat"))?;
    struct CH { inst: Option<component::Instance> }
    let mut linker = component::Linker::<CH>::new(&engine);
    linker.root().func_wrap("original-f", |mut c: StoreContextMut<'_, CH>, (x,): (i32,)| -> R<(i32,)> {
        // "real F" calls G whose hook lives in the same instance -> re-enter via export hook-g
        let inst = c.data().inst.unwrap();
        let g = inst.get_typed_func::<(i32,), (i32,)>(&mut c, "hook-g")?;
        let (r,) = g.call(&mut c, (x + 1,))?;
        Ok((x * 10 + r,))
    })?;
    linker.root().func_wrap("original-h", |_c: StoreContextMut<'_, CH>, (x,): (i32,)| -> R<(i32,)> { Ok((x + 1,)) })?;
    linker.root().func_wrap("original-p", |_c: StoreContextMut<'_, CH>, (d,): (Vec<u8>,)| -> R<(i32,)> { Ok((d.len() as i32,)) })?;
    let mut store = Store::new(&engine, CH { inst: None });
    let inst = linker.instantiate(&mut store, &comp)?;
    store.data_mut().inst = Some(inst);
    let f = inst.get_typed_func::<(i32,), (i32,)>(&mut store, "hook-f")?;
    match f.call(&mut store, (3,)) {
        Ok((v,)) => println!("  UNEXPECTED success: {v}"),
        Err(e) => {
            println!("  root cause: {}", e.root_cause());
            println!("  full chain: {e:#}");
        }
    }
    let g = inst.get_typed_func::<(i32,), (i32,)>(&mut store, "hook-g")?;
    match g.call(&mut store, (1,)) {
        Ok((v,)) => println!("  afterwards hook-g(1) from the host works: {v}"),
        Err(e) => println!("  afterwards the instance is unusable: {}", e.root_cause()),
    }
    // trap poisoning
    let t = inst.get_typed_func::<(i32,), (i32,)>(&mut store, "hook-trap")?;
    match t.call(&mut store, (1,)) { Ok(_) => println!("  hook-trap unexpectedly ok"), Err(e) => println!("  hook-trap -> {}", e.root_cause()) }
    match g.call(&mut store, (1,)) {
        Ok((v,)) => println!("  after a trap hook-g(1) still works: {v}"),
        Err(e) => println!("  after a trap the component instance is poisoned: {}", e.root_cause()),
    }
    // component cost
    bench_component(&engine, &comp)?;
    Ok(())
}

fn bench_component(engine: &Engine, comp: &component::Component) -> R<()> {
    struct CH { inst: Option<component::Instance> }
    let mut linker = component::Linker::<CH>::new(engine);
    linker.root().func_wrap("original-f", |_c: StoreContextMut<'_, CH>, (x,): (i32,)| -> R<(i32,)> { Ok((x + 1,)) })?;
    linker.root().func_wrap("original-h", |_c: StoreContextMut<'_, CH>, (x,): (i32,)| -> R<(i32,)> { Ok((x + 1,)) })?;
    linker.root().func_wrap("original-p", |_c: StoreContextMut<'_, CH>, (d,): (Vec<u8>,)| -> R<(i32,)> {
        let mut x = 0u8; for b in &d { x ^= *b; } Ok((x as i32 + d.len() as i32,))
    })?;
    let mut store = Store::new(engine, CH { inst: None });
    let inst = linker.instantiate(&mut store, comp)?;
    store.data_mut().inst = Some(inst);
    let h = inst.get_typed_func::<(i32,), (i32,)>(&mut store, "hook-h")?;
    let p = inst.get_typed_func::<(Vec<u8>,), (i32,)>(&mut store, "hook-p")?;
    let n = 1_000_000u32;
    let mut acc = 0i64;
    for i in 0..10_000 { acc += h.call(&mut store, (i,))?.0 as i64; }
    let t = Instant::now();
    for i in 0..n { acc += h.call(&mut store, (i as i32,))?.0 as i64; }
    let a = t.elapsed().as_nanos() as f64 / n as f64;
    let payload = vec![7u8; 64];
    let t = Instant::now();
    for _ in 0..n { acc += p.call(&mut store, (payload.clone(),))?.0 as i64; }
    let b = t.elapsed().as_nanos() as f64 / n as f64;
    println!("  component cost (no limits): trivial round trip {a:.1} ns | list<u8> of 64 bytes {b:.1} ns (acc={acc})");
    Ok(())
}

fn burn_setup(env: &mut Env, f: i32, g: i32, h: i32) -> R<()> {
    let set = env.inst.get_typed_func::<(i32, i32, i32), ()>(&mut env.store, "set_burn")?;
    set.call(&mut env.store, (f, g, h))
}

fn test3() -> R<()> {
    println!("== 3. fuel and epoch ==");
    let (_e, mut env, _) = make_core(true, false)?;
    burn_setup(&mut env, 900, 1000, 0)?;
    env.store.data_mut().fuel_budget = Some(10_000);
    env.store.set_fuel(u64::MAX / 4)?;
    let before = env.store.get_fuel()?;
    let r = top_call_f(&mut env, 3);
    let after = env.store.get_fuel()?;
    print_trace(&mut env);
    println!("  per-activation budget 10_000, burn_f=900 iters (~7k fuel), burn_g=1000 iters (~8k fuel): result={r}, disabled={}, total fuel actually burned by guest (outer store counter): {}", env.store.data().disabled, before - after);
    let (_e, mut env, _) = make_core(true, false)?;
    burn_setup(&mut env, 900, 1000, 0)?;
    env.store.data_mut().fuel_budget = Some(10_000);
    env.store.data_mut().nested_fuel_swap = false;
    let r = top_call_f(&mut env, 3);
    print_trace(&mut env);
    println!("  naive single pool (nested hook draws from outer budget): result={r}, disabled={}, pending={:?}", env.store.data().disabled, env.store.data().pending);
    for pause in [true, false] {
        let (engine, mut env, ticks) = make_core(false, true)?;
        let stop = Arc::new(AtomicU64::new(0));
        let (e2, t2, s2) = (engine.clone(), ticks.clone(), stop.clone());
        let th = std::thread::spawn(move || {
            while s2.load(Ordering::Relaxed) == 0 { std::thread::sleep(Duration::from_millis(1)); t2.fetch_add(1, Ordering::Relaxed); e2.increment_epoch(); }
        });
        burn_setup(&mut env, 0, 0, 1000)?;
        env.store.data_mut().epoch_budget_ticks = Some(20);
        env.store.data_mut().epoch_pause = pause;
        let h = env.store.data().hook_h.clone().unwrap();
        let t0 = Instant::now();
        let res = {
            let mut ctx = env.store.as_context_mut();
            with_active(&mut ctx, || {
                active(|c| { let n = c.data().ticks.load(Ordering::Relaxed); c.data_mut().hook_start_tick = n; c.set_epoch_deadline(20); });
                active(|c| h.call(c, 5))
            })
        };
        println!("  epoch budget 20 ms, engine body sleeps 60 ms, pause/restore={pause}: {:?} in {:?}", res.as_ref().map_err(|e| e.root_cause().to_string()), t0.elapsed());
        stop.store(1, Ordering::Relaxed); th.join().unwrap();
    }
    Ok(())
}

fn bench() -> R<()> {
    println!("== 4. cost (release) ==");
    let n = 2_000_000u32;
    for (label, fuel, epoch) in [("no limits", false, false), ("fuel on", true, false), ("epoch on", false, true), ("fuel+epoch", true, true)] {
        let (_e, mut env, _) = make_core(fuel, epoch)?;
        let h = env.store.data().hook_h.clone().unwrap();
        let p = env.store.data().hook_p.clone().unwrap();
        let alloc = env.inst.get_typed_func::<i32, i32>(&mut env.store, "alloc")?;
        let free = env.inst.get_typed_func::<i32, ()>(&mut env.store, "free")?;
        let mem = env.store.data().memory.unwrap();
        let mut ctx = env.store.as_context_mut();
        let mut acc = 0i64;
        for i in 0..10_000 { acc += h.call(&mut ctx, i)? as i64; }
        let t = Instant::now();
        for i in 0..n { acc += h.call(&mut ctx, i as i32)? as i64; }
        let trivial = t.elapsed().as_nanos() as f64 / n as f64;
        let payload = [7u8; 64];
        let t = Instant::now();
        for _ in 0..n {
            let ptr = alloc.call(&mut ctx, 64)?;
            mem.write(&mut ctx, ptr as usize, &payload)?;
            acc += p.call(&mut ctx, (ptr, 64))? as i64;
            free.call(&mut ctx, ptr)?;
        }
        let with64 = t.elapsed().as_nanos() as f64 / n as f64;
        let t = Instant::now();
        for _ in 0..n {
            let ptr = alloc.call(&mut ctx, 64)?;
            free.call(&mut ctx, ptr)?;
        }
        let allocfree = t.elapsed().as_nanos() as f64 / n as f64;
        println!("  {label:11}: trivial round trip {trivial:6.1} ns | 64 B (alloc+write+hook+orig+free) {with64:6.1} ns | alloc+free alone {allocfree:6.1} ns (acc={acc})");
    }
    Ok(())
}

fn test5() -> R<()> {
    println!("== 5. trap inside nested hook (hook_g traps when x==100, reached via hook_f(99)) ==");
    for (abort, strict) in [(false, false), (false, true), (true, false)] {
        let (_e, mut env, _) = make_core(false, false)?;
        env.store.data_mut().abort_policy = abort;
        let ss = env.inst.get_typed_func::<i32, ()>(&mut env.store, "set_strict")?;
        ss.call(&mut env.store, strict as i32)?;
        let r = top_call_f(&mut env, 99);
        println!("  policy {}, guest outer frame check on global sp = {strict}:", if abort { "B (abort outer, then run original F)" } else { "A (finish outer with original G)" });
        print_trace(&mut env);
        let spv = sp(&mut env); println!("    result={r}, F body executions={}, depth={}, disabled={}, guest sp={} (clean = 65536)", env.store.data().side_effects, env.store.data().depth, env.store.data().disabled, spv);
        let g = env.inst.get_typed_func::<i32, i32>(&mut env.store, "hook_g")?;
        let ok = { let mut c = env.store.as_context_mut(); with_active(&mut c, || active(|c| g.call(c, 5))) };
        println!("    raw call to hook_g(5) after the trap: {:?} (core instance is not poisoned)", ok.map_err(|e| e.to_string()));
        let r2 = top_call_f(&mut env, 3);
        print_trace(&mut env);
        println!("    next f(3) with mod disabled = {r2} (original bodies: 38)");
    }
    Ok(())
}

fn main() -> R<()> {
    let which = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    if which == "all" || which == "t" { test1()?; test2()?; test3()?; test5()?; }
    if which == "all" || which == "b" { bench()?; }
    Ok(())
}
