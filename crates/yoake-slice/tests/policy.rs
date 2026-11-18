//! MX3L2: the limits of a full-access mod (`ModConfig::full_access`: no fuel limit, a logging watchdog that never
//! traps, a finite `yoake_init`) and the per-hook chain cache.

use std::time::Duration;
use yoake_slice::activation::FieldTable;
use yoake_slice::runtime::{self, ModConfig, ModId, WATCHDOG_MS};

struct Thing {
    count: i32,
    runs: u32,
}

static TABLE: FieldTable<Thing> = FieldTable {
    get: |t, f| (f == 0).then(|| i64::from(t.count)),
    set: |t, f, v| {
        if f == 0 {
            t.count = v as i32;
            true
        } else {
            false
        }
    },
};

/// Hook `id`; the engine body adds 300 and takes `engine_ms` milliseconds first.
fn engine(id: u32, t: &mut Thing, engine_ms: u64) {
    runtime::dispatch_id(id, t, &TABLE, &mut |t| {
        if engine_ms > 0 {
            std::thread::sleep(Duration::from_millis(engine_ms));
        }
        t.count += 300;
        t.runs += 1;
        0
    });
}

/// A multiplexed guest for hooks 1, 2 and 3:
/// * hook 1 burns `loop_iters` loop iterations (each a few instructions) and then calls `original`;
/// * hook 2 spins until `spin_ms` of wall time passed (the `ticks` import) and then calls `original`;
/// * hook 3 calls `original` at once (the engine side is slow, not the guest).
fn guest(loop_iters: u64, spin_ms: u64) -> String {
    format!(
        r#"(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "ticks" (func $ticks (result i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init")
    (drop (call $register (i32.const 1) (i32.const 0)))
    (drop (call $register (i32.const 2) (i32.const 0)))
    (drop (call $register (i32.const 3) (i32.const 0))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (local $n i64) (local $start i64)
    (if (i32.eq (local.get $id) (i32.const 1))
      (then
        (loop $burn
          (local.set $n (i64.add (local.get $n) (i64.const 1)))
          (br_if $burn (i64.lt_u (local.get $n) (i64.const {loop_iters}))))))
    (if (i32.eq (local.get $id) (i32.const 2))
      (then
        (local.set $start (call $ticks))
        (loop $spin
          (br_if $spin (i64.lt_s (call $ticks) (i64.add (local.get $start) (i64.const {spin_ms})))))))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (i64.const 0)))"#
    )
}

fn load(wat: &str, cfg: ModConfig) -> ModId {
    runtime::load(wat.as_bytes(), cfg, "mod").unwrap()
}

#[test]
fn the_full_access_policy_is_no_fuel_limit_and_a_logging_watchdog() {
    let cfg = ModConfig::full_access();
    assert_eq!(cfg.fuel, None, "no fuel limit per activation");
    assert_eq!(cfg.epoch_ticks, None, "no trapping epoch");
    assert_eq!(cfg.watchdog_ms, Some(WATCHDOG_MS));
    assert_eq!(WATCHDOG_MS, 50);
    let tests = ModConfig::new("");
    assert_eq!((tests.fuel, tests.epoch_ticks, tests.watchdog_ms), (Some(1_000_000), None, None), "the limits of the tests are unchanged");
}

#[test]
fn a_long_computation_is_not_cut_off_by_fuel_for_a_full_access_mod() {
    runtime::uninstall().unwrap();
    // tens of millions of instructions: far over the tests' 1 million fuel
    let wat = guest(20_000_000, 0);
    let limited = load(&wat, ModConfig::new(""));
    let mut t = Thing { count: 0, runs: 0 };
    engine(1, &mut t, 0);
    assert!(runtime::stats_of(limited).unwrap().disabled, "the tests' fuel limit stops it (the old default)");
    assert_eq!(t.runs, 1, "and the engine still ran once");
    runtime::uninstall().unwrap();

    let open = load(&wat, ModConfig::full_access());
    let mut t = Thing { count: 0, runs: 0 };
    engine(1, &mut t, 0);
    let stats = runtime::stats_of(open).unwrap();
    assert!(!stats.disabled, "{:?}", stats.trap);
    assert_eq!(t.runs, 1);
    assert_eq!(t.count, 300);
    // a second activation also gets a full tank
    engine(1, &mut t, 0);
    let stats = runtime::stats_of(open).unwrap();
    assert!(!stats.disabled);
    assert_eq!(stats.entries, 2);
    assert_eq!(stats.fuel_left, Some(runtime::INITIAL_FUEL), "every top-level activation leaves the tank full");
    runtime::uninstall().unwrap();
}

#[test]
fn a_long_activation_is_logged_by_the_watchdog_and_never_trapped() {
    runtime::uninstall().unwrap();
    let id = load(&guest(0, 200), ModConfig::full_access());
    let mut t = Thing { count: 0, runs: 0 };

    // a quick activation: nothing logged
    engine(3, &mut t, 0);
    assert_eq!(runtime::stats_of(id).unwrap().slow_activations, 0);

    // 200 ms of the guest's own time: logged once, not stopped, the engine ran, the mod lives on
    engine(2, &mut t, 0);
    let stats = runtime::stats_of(id).unwrap();
    assert_eq!(stats.slow_activations, 1, "logged once per activation, not once per tick");
    assert!(stats.slowest_ms >= WATCHDOG_MS, "{}", stats.slowest_ms);
    assert!(!stats.disabled, "the watchdog never traps: {:?}", stats.trap);
    assert_eq!(t.runs, 2);
    assert_eq!(stats.epoch_depth, 0, "the activation's epoch record is closed");

    // time spent inside `original` (the engine's own work) is not the guest's: 150 ms of engine, no log
    engine(3, &mut t, 150);
    let stats = runtime::stats_of(id).unwrap();
    assert_eq!(stats.slow_activations, 1, "a slow engine call under a quick guest is not the mod's fault");
    assert!(!stats.disabled);

    // and the mod keeps working afterwards
    engine(2, &mut t, 0);
    let stats = runtime::stats_of(id).unwrap();
    assert_eq!(stats.slow_activations, 2);
    assert_eq!(stats.entries, 4);
    assert!(!stats.disabled);
    runtime::uninstall().unwrap();
}

#[test]
fn the_trapping_epoch_budget_of_the_tests_still_traps() {
    runtime::uninstall().unwrap();
    // `epoch_ticks` is the configurable hard limit (kept for tests): it traps, the watchdog is not involved
    let cfg = ModConfig { fuel: None, epoch_ticks: Some(30), watchdog_ms: Some(10), ..ModConfig::new("") };
    let id = load(&guest(0, 500), cfg);
    let mut t = Thing { count: 0, runs: 0 };
    engine(2, &mut t, 0);
    let stats = runtime::stats_of(id).unwrap();
    assert!(stats.disabled, "the hard epoch budget traps the spin");
    assert_eq!(stats.slow_activations, 0);
    assert_eq!(t.runs, 1, "the engine still ran once");
    runtime::uninstall().unwrap();
}

#[test]
fn yoake_init_keeps_a_finite_budget_under_the_full_access_policy() {
    runtime::uninstall().unwrap();
    let wat = r#"(module
  (memory (export "memory") 1)
  (func (export "yoake_init") (loop $forever (br $forever)))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64) (i64.const 0)))"#;
    let error = runtime::load(wat.as_bytes(), ModConfig::full_access(), "spin").unwrap_err().to_string();
    assert!(error.contains("yoake_init failed"), "{error}");
    assert!(runtime::all_stats().is_empty(), "nothing is left loaded");
}

#[test]
fn cached_chains_follow_loads_unloads_and_deaths() {
    runtime::uninstall().unwrap();
    let mut t = Thing { count: 0, runs: 0 };
    engine(3, &mut t, 0);
    assert_eq!(t.runs, 1, "no mod: a plain call (and an empty chain is now cached for hook 3)");

    // a load invalidates the cached empty chain
    let a = load(&guest(0, 0), ModConfig::new(""));
    engine(3, &mut t, 0);
    assert_eq!(runtime::stats_of(a).unwrap().entries, 1, "the new mod is in the chain");
    engine(3, &mut t, 0);
    assert_eq!(runtime::stats_of(a).unwrap().entries, 2, "served from the cache, still correct");

    // a second load invalidates again
    let b = load(&guest(0, 0), ModConfig::new(""));
    engine(3, &mut t, 0);
    assert_eq!((runtime::stats_of(a).unwrap().entries, runtime::stats_of(b).unwrap().entries), (3, 1));

    // an unload invalidates
    runtime::unload(a).unwrap();
    engine(3, &mut t, 0);
    assert_eq!(runtime::stats_of(b).unwrap().entries, 2);
    assert!(runtime::stats_of(a).is_none());

    // a death invalidates: hook 1 burns 20M iterations, the tests' fuel limit kills the mod, the next call has no chain
    runtime::uninstall().unwrap();
    let victim = load(&guest(20_000_000, 0), ModConfig::new(""));
    let mut t = Thing { count: 0, runs: 0 };
    engine(1, &mut t, 0);
    assert!(runtime::stats_of(victim).unwrap().disabled);
    assert_eq!(runtime::stats_of(victim).unwrap().entries, 1);
    engine(1, &mut t, 0);
    assert_eq!(runtime::stats_of(victim).unwrap().entries, 1, "a dead mod is never entered again");
    assert_eq!(t.runs, 2, "the engine ran each time");

    // uninstall invalidates
    runtime::uninstall().unwrap();
    let mut t = Thing { count: 0, runs: 0 };
    engine(1, &mut t, 0);
    assert_eq!(t.runs, 1);
}
