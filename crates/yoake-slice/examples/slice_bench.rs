//! Round-trip cost of the runtime alone (no engine): `dispatch` -> guest hook -> `original` import.
//! `cargo run --offline --release -p yoake-slice --features wasm --example slice_bench`
use std::hint::black_box;
use std::time::Instant;
use yoake_slice::GUEST_WAT;
use yoake_slice::activation::FieldTable;
use yoake_slice::runtime::{self, ModConfig};

#[derive(Default)]
struct Thing {
    counter: i32,
    velocity: i64,
}

static TABLE: FieldTable<Thing> = FieldTable {
    get: |t, f| (f == 0).then(|| i64::from(t.counter)),
    set: |t, f, v| {
        if f == 0 {
            t.counter = v as i32;
        }
        f == 0
    },
};

fn tick(thing: &mut Thing) -> i64 {
    runtime::dispatch(thing, &TABLE, &mut |t| {
        t.velocity += 1;
        0
    })
}

fn measure(label: &str, n: u32) -> f64 {
    let mut thing = Thing::default();
    let mut samples = Vec::new();
    for sample in 0..8 {
        let start = Instant::now();
        for _ in 0..n {
            black_box(tick(black_box(&mut thing)));
        }
        if sample > 0 {
            samples.push(start.elapsed().as_secs_f64() * 1e9 / f64::from(n));
        }
    }
    samples.sort_by(f64::total_cmp);
    let ns = samples[samples.len() / 2];
    println!("{label:<52} {ns:>8.1} ns/call");
    ns
}

fn main() {
    tls_floor();
    raw_floor();
    runtime::uninstall().unwrap();
    measure("no mod (dispatch miss + original)", 2_000_000);
    for (name, fuel, epoch) in [
        ("hook_nop", Some(1_000_000), None),
        ("hook_nop", None, None),
        ("hook_min", Some(1_000_000), None),
        ("hook_min", None, None),
        ("hook_rw", Some(1_000_000), None),
        ("hook_rw", None, None),
        ("hook_rw", Some(1_000_000), Some(1000)),
    ] {
        let mut cfg = ModConfig::new(name);
        cfg.fuel = fuel;
        cfg.epoch_ticks = epoch;
        runtime::install(GUEST_WAT.as_bytes(), cfg).unwrap();
        measure(&format!("{name} fuel={} epoch={}", fuel.is_some(), epoch.is_some()), 300_000);
    }
}

/// The platform floor: `TypedFunc::call` of `hook_nop` straight from the host, no runtime around it.
#[allow(dead_code)]
fn raw_floor() {
    use wasmtime::{Engine, Linker, Module, Store};
    let engine = Engine::default();
    let module = Module::new(&engine, GUEST_WAT).unwrap();
    let mut linker = Linker::<()>::new(&engine);
    linker.func_wrap("env", "original", |_h: i64, _o: i32| -> i32 { 0 }).unwrap();
    linker.func_wrap("env", "get", |_h: i64, _f: i32, _o: i32| -> i32 { 0 }).unwrap();
    linker.func_wrap("env", "set", |_h: i64, _f: i32, _v: i64| -> i32 { 0 }).unwrap();
    linker.func_wrap("env", "report", |_s: i32, _v: i64| {}).unwrap();
    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate(&mut store, &module).unwrap();
    for name in ["hook_nop", "hook_min"] {
        let f = instance.get_typed_func::<i64, i64>(&mut store, name).unwrap();
        let mut samples = Vec::new();
        for sample in 0..8 {
            let start = Instant::now();
            for i in 0..1_000_000i64 {
                black_box(f.call(&mut store, black_box(i)).unwrap());
            }
            if sample > 0 {
                samples.push(start.elapsed().as_secs_f64() * 1e9 / 1e6);
            }
        }
        samples.sort_by(f64::total_cmp);
        println!("raw TypedFunc::call {name} (trivial host import){:>20.1} ns/call", samples[3]);
    }
}

thread_local! {
    static T1: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static T2: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
    static T3: std::cell::RefCell<Vec<u64>> = std::cell::RefCell::new(Vec::new());
}

fn tls_floor() {
    for (name, f) in [
        ("const Cell", (|| { T1.with(|c| c.set(black_box(c.get()) + 1)); }) as fn()),
        ("const RefCell<Vec>", || { T2.with(|c| { black_box(c.borrow().len()); }); }),
        ("lazy RefCell<Vec>", || { T3.with(|c| { black_box(c.borrow().len()); }); }),
    ] {
        let start = Instant::now();
        for _ in 0..10_000_000 {
            f();
        }
        println!("thread_local access {name:<24}{:>8.2} ns", start.elapsed().as_secs_f64() * 1e9 / 1e7);
    }
}
