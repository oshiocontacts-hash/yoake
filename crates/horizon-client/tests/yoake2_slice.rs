//! MX3V1: the vertical slice on a real engine function, `ArmorStand::tick_settle` (S2 slot id 71),
//! hooked by a WASM core module through the S2 around-hook chain.
//!
//! Run (the feature pulls in yoake-slice and wasmtime; the default build has neither):
//!   cargo test --offline -j 4 -p horizon-client-lib --features yoake2-slice --test yoake2_slice
//! Numbers:
//!   cargo test --offline --release -j 4 -p horizon-client-lib --features yoake2-slice \
//!     --test yoake2_slice -- --ignored --nocapture bench_round_trip

use horizon_client_lib::entities::{hook_bench, ArmorStand, StandMedium, StandStep};
use horizon_core::geometry::Vec3;
use std::cell::{Cell, RefCell};
use std::sync::Once;
use yoake_slice::GUEST_WAT;
use yoake_slice::activation::{FieldTable, live_activations};
use yoake_slice::runtime::{self, ModConfig};

/// Field ids of the hand-written accessor table (what a generator would emit for the type).
const F_FIRE_TICKS: u32 = 0;
const F_POSITION_X: u32 = 1;

static ARMOR_STAND: FieldTable<ArmorStand> = FieldTable {
    get: |s, f| match f {
        F_FIRE_TICKS => Some(i64::from(s.fire_ticks)),
        F_POSITION_X => Some(s.position.x.to_bits() as i64),
        _ => None,
    },
    set: |s, f, v| match f {
        F_FIRE_TICKS => {
            s.fire_ticks = v as i32;
            true
        }
        F_POSITION_X => {
            s.position.x = f64::from_bits(v as u64);
            true
        }
        _ => false,
    },
};

/// The shim registered in the S2 slot of `ArmorStand::tick_settle`: one activation per call, `self`
/// as `Root::HookArg(activation, 0)`, the guest called by `runtime::dispatch`.
fn wasm_settle(
    original: &mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep),
    stand: &mut ArmorStand,
    medium: StandMedium,
    step: StandStep,
) {
    runtime::dispatch(stand, &ARMOR_STAND, &mut |stand| {
        original(stand, medium, step);
        0
    });
}

thread_local! {
    /// How often the chain reached the engine body (below the WASM entry).
    static ORIGINAL_RUNS: Cell<u32> = const { Cell::new(0) };
    /// While set, the engine body (as the entry below the WASM one) ticks `OTHER` first: a nested
    /// hook activation while the outer guest is waiting in `original`.
    static NEST: Cell<bool> = const { Cell::new(false) };
    static OTHER: RefCell<ArmorStand> = RefCell::new(ArmorStand::new(2, Vec3::ZERO, 0.0));
}

/// A plain Rust entry below the WASM one (lower priority): counts and optionally nests.
fn counting_entry(
    original: &mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep),
    stand: &mut ArmorStand,
    medium: StandMedium,
    step: StandStep,
) {
    ORIGINAL_RUNS.set(ORIGINAL_RUNS.get() + 1);
    if NEST.replace(false) {
        OTHER.with(|other| other.borrow_mut().tick_settle(StandMedium::Air, step));
    }
    original(stand, medium, step);
}

static REGISTER_WASM: Once = Once::new();
static REGISTER_COUNTING: Once = Once::new();

fn register_wasm() {
    REGISTER_WASM.call_once(|| {
        hook_bench::register_tick_settle(wasm_settle, 10, 0);
    });
}

fn register_all() {
    register_wasm();
    REGISTER_COUNTING.call_once(|| {
        hook_bench::register_tick_settle(counting_entry, 0, 0);
    });
}

fn step() -> StandStep {
    StandStep::for_hook_bench(true, 0.6)
}

fn stand() -> ArmorStand {
    ArmorStand::new(1, Vec3::new(3.0, 64.0, 5.0), 30.0)
}

fn install(export: &str) {
    runtime::install(GUEST_WAT.as_bytes(), ModConfig::new(export)).unwrap();
}

fn find(reports: &[(i32, i64)], slot: i32) -> Vec<i64> {
    reports.iter().filter(|(s, _)| *s == slot).map(|(_, v)| *v).collect()
}

/// What the unhooked engine does to a fresh stand in this medium.
fn control_velocity(medium: StandMedium) -> Vec3 {
    runtime::uninstall().unwrap();
    let mut control = stand();
    control.tick_settle(medium, step());
    control.velocity
}

#[test]
fn guest_changes_self_state_and_the_game_sees_it() {
    register_all();
    let expected = control_velocity(StandMedium::Water);
    ORIGINAL_RUNS.set(0);
    install("hook_basic");
    let mut s = stand();
    s.tick_settle(StandMedium::Water, step());
    assert_eq!(s.fire_ticks, 15, "the counter field: 5 before original, +10 after reading it back");
    assert_eq!(s.position.x, 12.5, "position.x written through the handle");
    assert_eq!(s.velocity, expected, "the engine body ran with the same effect as unhooked");
    assert_eq!(ORIGINAL_RUNS.get(), 1);
    let r = runtime::take_reports().unwrap();
    assert_eq!(find(&r, 10), [0]);
    assert_eq!(find(&r, 12), [5]);
    assert_eq!(live_activations(), 0);
    assert_eq!(runtime::stats().entries, 1);
}

#[test]
fn reentrancy_and_the_lending_lock() {
    register_all();
    ORIGINAL_RUNS.set(0);
    install("hook_reenter");
    NEST.set(true);
    let mut outer = stand();
    outer.tick_settle(StandMedium::Air, step());
    let inner_ticks = OTHER.with(|o| o.borrow().fire_ticks);
    let r = runtime::take_reports().unwrap();
    assert_eq!(find(&r, 40), [1, 2], "the guest was re-entered on the same store");
    assert_eq!(find(&r, 20), [2], "get through the lent outer handle: Locked");
    assert_eq!(find(&r, 21), [2], "set through the lent outer handle: Locked");
    assert_eq!(find(&r, 22), [2], "original on the lent activation: Locked");
    assert_eq!(find(&r, 23), [0]);
    assert_eq!(find(&r, 30), [0, 0]);
    assert_eq!(outer.fire_ticks, 101, "the rejected write (999) did not land");
    assert_eq!(inner_ticks, 102);
    assert_eq!(ORIGINAL_RUNS.get(), 2);
    assert!(!runtime::stats().disabled);
    assert_eq!(live_activations(), 0);
}

#[test]
fn trap_before_original_leaves_the_game_running_and_runs_original_once() {
    register_all();
    let expected = control_velocity(StandMedium::Air);
    ORIGINAL_RUNS.set(0);
    install("hook_trap_before");
    let mut s = stand();
    s.tick_settle(StandMedium::Air, step());
    assert_eq!(ORIGINAL_RUNS.get(), 1, "original exactly once");
    assert_eq!(s.velocity, expected);
    assert_eq!(s.fire_ticks, 31, "what the guest wrote before the trap");
    let stats = runtime::stats();
    assert!(stats.disabled && stats.trap.unwrap().contains("unreachable"));
    // never called again, and the game goes on
    s.fire_ticks = 0;
    s.tick_settle(StandMedium::Air, step());
    s.tick_settle(StandMedium::Air, step());
    assert_eq!(ORIGINAL_RUNS.get(), 3);
    assert_eq!(s.fire_ticks, 0);
    assert_eq!(runtime::stats().entries, 1);
    assert_eq!(live_activations(), 0);
}

#[test]
fn trap_after_original_does_not_run_it_twice() {
    register_all();
    let expected = control_velocity(StandMedium::Air);
    ORIGINAL_RUNS.set(0);
    install("hook_trap_after");
    let mut s = stand();
    s.tick_settle(StandMedium::Air, step());
    assert_eq!(ORIGINAL_RUNS.get(), 1, "side effects once");
    assert_eq!(s.velocity, expected, "velocity changed once, not twice");
    assert_eq!(s.fire_ticks, 55);
    assert!(runtime::stats().disabled);
}

#[test]
fn nested_trap_leaves_the_outer_hook_running() {
    register_all();
    ORIGINAL_RUNS.set(0);
    install("hook_trap_nested");
    NEST.set(true);
    let mut outer = stand();
    outer.tick_settle(StandMedium::Air, step());
    assert_eq!(ORIGINAL_RUNS.get(), 2, "outer original + the nested fallback, nothing twice");
    assert_eq!(outer.fire_ticks, 31, "the outer hook finished");
    assert!(runtime::stats().disabled);
    assert_eq!(runtime::stats().entries, 2);
    assert_eq!(live_activations(), 0);
}

#[test]
fn stale_and_forged_handles_are_errors() {
    register_all();
    install("hook_stale");
    let mut s = stand();
    s.tick_settle(StandMedium::Air, step());
    s.tick_settle(StandMedium::Air, step());
    let r = runtime::take_reports().unwrap();
    assert_eq!(find(&r, 50), [1]);
    assert_eq!(find(&r, 51), [4]);
    assert_eq!(find(&r, 52), [3]);
}

#[test]
fn hook_does_not_fire_on_another_thread() {
    register_all();
    install("hook_basic");
    let mut here = stand();
    here.tick_settle(StandMedium::Air, step());
    assert_eq!(here.fire_ticks, 15);
    std::thread::spawn(|| {
        let mut there = stand();
        let before = there.velocity;
        there.tick_settle(StandMedium::Air, step());
        assert_eq!(there.fire_ticks, 0, "no guest on a worker thread");
        assert_eq!(there.position.x, 3.0);
        assert_ne!(there.velocity, before, "the original ran");
        assert!(!runtime::stats().installed);
    })
    .join()
    .unwrap();
    assert_eq!(runtime::stats().entries, 1);
}

fn median(samples: &mut [f64]) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn measure(label: &str, iterations: u32) -> f64 {
    let mut stand = stand();
    let step = step();
    let mut samples = Vec::new();
    for sample in 0..8 {
        let start = std::time::Instant::now();
        for _ in 0..iterations {
            std::hint::black_box(&mut stand).tick_settle(std::hint::black_box(StandMedium::Air), step);
        }
        let ns = start.elapsed().as_secs_f64() * 1e9 / f64::from(iterations);
        if sample > 0 {
            samples.push(ns);
        }
    }
    let ns = median(&mut samples);
    println!("{label:<58} {ns:>9.1} ns/call");
    ns
}

/// Release numbers for the report. `cargo test --release ... -- --ignored --nocapture bench_round_trip`
#[test]
#[ignore = "benchmark: run in release with --ignored --nocapture"]
fn bench_round_trip() {
    // 1. the slot is still empty: the S2 cost with nothing registered
    let empty = measure("slot empty (nothing registered)", 2_000_000);
    register_wasm();
    // 2. the slot has the WASM entry, this thread has no mod: chain walk + shim + dispatch miss
    let miss = measure("slot set, no mod on this thread (fall through)", 1_000_000);
    install("hook_min");
    let min = measure("guest hook_min: hook + original (scalar handle)", 300_000);
    install("hook_rw");
    let rw = measure("guest hook_rw: get + set + original", 300_000);
    let mut cfg = ModConfig::new("hook_rw");
    cfg.fuel = None;
    runtime::install(GUEST_WAT.as_bytes(), cfg).unwrap();
    let rw_nofuel = measure("guest hook_rw, unmetered (no fuel)", 300_000);
    let mut cfg = ModConfig::new("hook_rw");
    cfg.epoch_ticks = Some(1000);
    runtime::install(GUEST_WAT.as_bytes(), cfg).unwrap();
    let rw_epoch = measure("guest hook_rw, fuel + epoch", 300_000);
    println!(
        "overhead over empty slot: miss {:+.1}, min {:+.1}, rw {:+.1}, rw unmetered {:+.1}, rw fuel+epoch {:+.1} (ns)",
        miss - empty,
        min - empty,
        rw - empty,
        rw_nofuel - empty,
        rw_epoch - empty
    );
}
