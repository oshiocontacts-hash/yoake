//! MX3L1: several mods on one thread. Each mod is its own store; for one hook id the registered mods run
//! in priority order (lowest first, ties by load order) and each wraps the next: its `original` runs the next
//! mod, the last one's runs the engine. A trap removes only that mod.

use yoake_slice::activation::FieldTable;
use yoake_slice::runtime::{self, ModConfig, ModId, register_status};

struct Thing {
    count: i32,
    /// how often the engine body ran
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

/// The hooked engine function (hook id 1): its body adds 300 to the count.
fn engine(t: &mut Thing) {
    runtime::dispatch_id(1, t, &TABLE, &mut |t| {
        t.count += 300;
        t.runs += 1;
        0
    });
}

/// A multiplexed guest for hook 1 at `priority`: it reports the count it sees before (slot `base`) and after
/// (slot `base + 1`) calling `original`, and adds `add` to the count before. `trap` makes it `unreachable`
/// instead of doing anything.
fn guest(priority: i32, base: i32, add: i32, trap: bool) -> String {
    let body = if trap {
        "(unreachable)".to_string()
    } else {
        format!(
            r#"(local.set $v (i32.wrap_i64 (call $get_count (local.get $h))))
    (call $report (i32.const {base}) (i64.extend_i32_s (local.get $v)))
    (drop (call $set (local.get $h) (i32.const 0) (i64.extend_i32_s (i32.add (local.get $v) (i32.const {add})))))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (call $report (i32.const {after}) (i64.extend_i32_s (i32.wrap_i64 (call $get_count (local.get $h)))))"#,
            after = base + 1
        )
    };
    format!(
        r#"(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "get" (func $get (param i64 i32 i32) (result i32)))
  (import "env" "set" (func $set (param i64 i32 i64) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func $get_count (param $h i64) (result i64)
    (drop (call $get (local.get $h) (i32.const 0) (i32.const 32)))
    (i64.load (i32.const 32)))
  (func (export "yoake_init")
    (call $report (i32.const 90) (i64.extend_i32_s (call $register (i32.const 1) (i32.const {priority})))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (local $v i32)
    {body}
    (i64.const 0)))"#
    )
}

fn load(wat: &str, name: &str) -> ModId {
    runtime::load(wat.as_bytes(), ModConfig::new(""), name).unwrap()
}

fn reports(id: ModId) -> Vec<(i32, i64)> {
    runtime::take_reports_of(id).unwrap()
}

fn slot(r: &[(i32, i64)], n: i32) -> i64 {
    r.iter().find(|(s, _)| *s == n).unwrap_or_else(|| panic!("no report {n} in {r:?}")).1
}

#[test]
fn mods_compose_in_priority_order_and_each_wraps_the_next() {
    runtime::uninstall().unwrap();
    // A (priority 10) is outside B (priority 20): A sees the plain count before, B sees A's change
    let a = load(&guest(10, 1, 1, false), "a");
    let b = load(&guest(20, 11, 20, false), "b");
    let mut t = Thing { count: 0, runs: 0 };
    engine(&mut t);
    assert_eq!(t.count, 321, "A added 1, B added 20, the engine added 300");
    assert_eq!(t.runs, 1, "the engine body ran exactly once through two wrapping mods");
    let (ra, rb) = (reports(a), reports(b));
    assert_eq!(slot(&ra, 1), 0, "the outer mod sees the count before anyone");
    assert_eq!(slot(&rb, 11), 1, "the inner mod sees the outer one's change");
    assert_eq!(slot(&ra, 2), 321, "the outer mod sees the inner mod's effect and the engine's after `original`");
    assert_eq!(slot(&rb, 12), 321);
    assert_eq!(runtime::stats_of(a).unwrap().entries, 1);
    assert_eq!(runtime::stats_of(b).unwrap().entries, 1);

    // the same two mods with the priorities swapped: B is outside now
    runtime::uninstall().unwrap();
    let a = load(&guest(30, 1, 1, false), "a");
    let b = load(&guest(5, 11, 20, false), "b");
    let mut t = Thing { count: 0, runs: 0 };
    engine(&mut t);
    assert_eq!(slot(&reports(b), 11), 0, "B is the outer mod");
    assert_eq!(slot(&reports(a), 1), 20, "A sees B's change");
    assert_eq!(t.runs, 1);

    // equal priority: load order decides (first loaded is outermost)
    runtime::uninstall().unwrap();
    let first = load(&guest(7, 1, 1, false), "first");
    let second = load(&guest(7, 11, 20, false), "second");
    let mut t = Thing { count: 0, runs: 0 };
    engine(&mut t);
    assert_eq!(slot(&reports(first), 1), 0);
    assert_eq!(slot(&reports(second), 11), 1);
    let all = runtime::all_stats();
    assert_eq!(all.iter().map(|(_, name, _)| name.as_str()).collect::<Vec<_>>(), ["first", "second"]);
    runtime::uninstall().unwrap();
}

#[test]
fn a_trap_removes_only_that_mod_and_the_others_keep_working() {
    runtime::uninstall().unwrap();
    let a = load(&guest(10, 1, 1, false), "a");
    let bad = load(&guest(15, 50, 0, true), "bad");
    let b = load(&guest(20, 11, 20, false), "b");
    let mut t = Thing { count: 0, runs: 0 };
    engine(&mut t);
    // the middle mod trapped before calling `original`: the rest of the chain and the engine still ran, once
    assert_eq!(t.count, 321);
    assert_eq!(t.runs, 1);
    assert_eq!(slot(&reports(b), 11), 1, "B saw A's change although the mod between them trapped");
    assert!(runtime::stats_of(bad).unwrap().disabled);
    assert!(runtime::stats_of(bad).unwrap().trap.unwrap().contains("unreachable"));
    assert!(!runtime::stats_of(a).unwrap().disabled);
    assert!(!runtime::stats_of(b).unwrap().disabled);
    // the next call: the trapped mod is not entered, the others are
    let mut t = Thing { count: 0, runs: 0 };
    engine(&mut t);
    assert_eq!(t.count, 321);
    assert_eq!(t.runs, 1);
    assert_eq!(runtime::stats_of(bad).unwrap().entries, 1, "never entered again");
    assert_eq!(runtime::stats_of(a).unwrap().entries, 2);
    assert_eq!(runtime::stats_of(b).unwrap().entries, 2);
    runtime::uninstall().unwrap();
}

#[test]
fn a_trap_after_original_does_not_run_the_chain_twice_and_the_outer_mod_continues() {
    runtime::uninstall().unwrap();
    // the inner mod calls `original` (the engine runs) and then traps: its result is the recorded one
    let wat = r#"(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (drop (call $register (i32.const 1) (i32.const 20))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (drop (call $orig (local.get $h) (i32.const 64)))
    (unreachable)))"#;
    let outer = load(&guest(10, 1, 1, false), "outer");
    let inner = load(wat, "inner");
    let mut t = Thing { count: 0, runs: 0 };
    engine(&mut t);
    assert_eq!(t.runs, 1, "the engine ran once, not again after the trap");
    assert!(runtime::stats_of(inner).unwrap().disabled);
    let r = reports(outer);
    assert_eq!(slot(&r, 1), 0);
    assert_eq!(slot(&r, 2), 301, "the outer mod continued after the inner one trapped");
    assert!(!runtime::stats_of(outer).unwrap().disabled);
    runtime::uninstall().unwrap();
}

#[test]
fn hook_register_is_for_init_only_capped_and_never_id_zero() {
    runtime::uninstall().unwrap();
    // registering from a hook is refused (and reported); id 0 is refused in init
    let wat = r#"(module
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init")
    (call $report (i32.const 1) (i64.extend_i32_s (call $register (i32.const 0) (i32.const 0))))
    (call $report (i32.const 2) (i64.extend_i32_s (call $register (i32.const 1) (i32.const 0)))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (call $report (i32.const 3) (i64.extend_i32_s (call $register (i32.const 9) (i32.const 0))))
    (i64.const 0)))"#;
    let id = load(wat, "late");
    let mut t = Thing { count: 0, runs: 0 };
    engine(&mut t);
    let r = reports(id);
    assert_eq!(slot(&r, 1), i64::from(register_status::RESERVED_ID));
    assert_eq!(slot(&r, 2), i64::from(register_status::OK));
    assert_eq!(slot(&r, 3), i64::from(register_status::NOT_IN_INIT), "outside yoake_init");
    assert_eq!(runtime::stats_of(id).unwrap().registered, vec![(1, 0)]);
    runtime::uninstall().unwrap();

    // the cap: 5000 distinct ids asked, 4096 granted
    let wat = r#"(module
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (local $i i32)
    (local.set $i (i32.const 1))
    (loop $again
      (if (i32.lt_s (call $register (local.get $i) (i32.const 0)) (i32.const 0))
        (then (call $report (i32.const 1) (i64.extend_i32_s (local.get $i)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br_if $again (i32.le_s (local.get $i) (i32.const 5000)))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64) (i64.const 0)))"#;
    let id = load(wat, "greedy");
    assert_eq!(runtime::stats_of(id).unwrap().registered.len(), runtime::MAX_REGISTRATIONS);
    assert_eq!(reports(id).len(), 5000 - runtime::MAX_REGISTRATIONS, "every id past the cap was refused");
    runtime::uninstall().unwrap();
}

#[test]
fn an_init_that_never_ends_is_stopped_by_its_budget() {
    runtime::uninstall().unwrap();
    let wat = r#"(module
  (memory (export "memory") 1)
  (func (export "yoake_init") (loop $forever (br $forever)))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64) (i64.const 0)))"#;
    let error = runtime::load(wat.as_bytes(), ModConfig::new(""), "spin").unwrap_err().to_string();
    assert!(error.contains("yoake_init failed"), "{error}");
    assert!(runtime::all_stats().is_empty(), "nothing is left loaded");
}

#[test]
fn a_legacy_single_export_guest_is_hook_zero_and_unload_removes_a_mod() {
    runtime::uninstall().unwrap();
    // the slice's guest is hook id 0, which a multiplexed guest cannot register
    let a = load(&guest(10, 1, 1, false), "a");
    let mut t = Thing { count: 0, runs: 0 };
    runtime::dispatch_id(0, &mut t, &TABLE, &mut |t| {
        t.count += 5;
        0
    });
    assert_eq!(t.count, 5, "hook 0 is not registered by the multiplexed guest");
    assert_eq!(runtime::stats_of(a).unwrap().entries, 0);
    runtime::unload(a).unwrap();
    assert!(runtime::stats_of(a).is_none());
    let mut t = Thing { count: 0, runs: 0 };
    engine(&mut t);
    assert_eq!(t.count, 300, "no mod left: a plain call");
}
