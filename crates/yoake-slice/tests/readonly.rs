use yoake_slice::activation::{RefFieldTable, Status};
use yoake_slice::runtime::{self, ModConfig, ModId};

struct Thing {
    count: i32,
}

static TABLE: RefFieldTable<Thing> = RefFieldTable {
    get: |t, f| (f == 0).then(|| i64::from(t.count)),
};

fn engine(id: u32, thing: &Thing, runs: &mut u32) -> i64 {
    runtime::dispatch_id_ref(id, thing, &TABLE, &mut |_| {
        *runs += 1;
        42
    })
}

const GUEST: &str = r#"(module
  (import "env" "original" (func $original (param i64 i32) (result i32)))
  (import "env" "get" (func $get (param i64 i32 i32) (result i32)))
  (import "env" "set" (func $set (param i64 i32 i64) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func $register_id (param $id i32)
    (drop (call $register (local.get $id) (i32.const 0))))
  (func (export "yoake_init")
    (call $register_id (i32.const 1))
    (call $register_id (i32.const 2))
    (call $register_id (i32.const 4))
    (call $register_id (i32.const 5))
    (call $register_id (i32.const 6))
    (call $register_id (i32.const 7)))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (if (i32.eq (local.get $id) (i32.const 1))
      (then
        (call $report (i32.const 10) (i64.extend_i32_s (call $get (local.get $h) (i32.const 0) (i32.const 32))))
        (call $report (i32.const 11) (i64.load (i32.const 32)))
        (call $report (i32.const 12) (i64.extend_i32_s (call $set (local.get $h) (i32.const 0) (i64.const 99))))
        (drop (call $get (local.get $h) (i32.const 0) (i32.const 32)))
        (call $report (i32.const 13) (i64.load (i32.const 32)))
        (return (i64.const 77))))
    (if (i32.eq (local.get $id) (i32.const 2))
      (then
        (call $report (i32.const 20) (i64.extend_i32_s (call $set (local.get $h) (i32.const 0) (i64.const 99))))
        (call $report (i32.const 21) (i64.extend_i32_s (call $original (local.get $h) (i32.const 64))))
        (call $report (i32.const 22) (i64.load (i32.const 64)))
        (return (i64.const 88))))
    (if (i32.eq (local.get $id) (i32.const 4))
      (then (unreachable)))
    (if (i32.eq (local.get $id) (i32.const 5))
      (then
        (drop (call $original (local.get $h) (i32.const 64)))
        (unreachable)))
    (if (i32.eq (local.get $id) (i32.const 6))
      (then
        (call $report (i32.const 60) (i64.extend_i32_s (call $original (local.get $h) (i32.const 64))))
        (call $report (i32.const 61) (i64.load (i32.const 64)))
        (return (i64.const 89))))
    (if (i32.eq (local.get $id) (i32.const 7))
      (then (return (i64.const 23))))
    (i64.const 0)))"#;

fn install() {
    runtime::install(GUEST.as_bytes(), ModConfig::new("")).unwrap();
}

fn reports() -> Vec<(i32, i64)> {
    runtime::take_reports().unwrap()
}

fn slot(reports: &[(i32, i64)], slot: i32) -> i64 {
    reports.iter().find(|(s, _)| *s == slot).unwrap_or_else(|| panic!("no report {slot} in {reports:?}")).1
}

#[test]
fn shared_get_works_and_set_is_read_only_without_calling_original() {
    install();
    let thing = Thing { count: 5 };
    let mut runs = 0;
    assert_eq!(engine(1, &thing, &mut runs), 77);
    let reports = reports();
    assert_eq!(slot(&reports, 10), 0);
    assert_eq!(slot(&reports, 11), 5);
    assert_eq!(slot(&reports, 12), i64::from(Status::ReadOnly as i32));
    assert_eq!(slot(&reports, 13), 5);
    assert_eq!((thing.count, runs), (5, 0));
    runtime::uninstall().unwrap();
}

#[test]
fn shared_original_runs_once_and_returns_its_result() {
    install();
    let thing = Thing { count: 5 };
    let mut runs = 0;
    assert_eq!(engine(2, &thing, &mut runs), 88);
    let reports = reports();
    assert_eq!(slot(&reports, 20), i64::from(Status::ReadOnly as i32));
    assert_eq!(slot(&reports, 21), 0);
    assert_eq!(slot(&reports, 22), 42);
    assert_eq!((thing.count, runs), (5, 1));
    runtime::uninstall().unwrap();
}

fn chain_guest(priority: i32, base: i32) -> String {
    format!(
        r#"(module
  (import "env" "original" (func $original (param i64 i32) (result i32)))
  (import "env" "get" (func $get (param i64 i32 i32) (result i32)))
  (import "env" "set" (func $set (param i64 i32 i64) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "yoake_init")
    (drop (call $register (i32.const 3) (i32.const {priority}))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (drop (call $get (local.get $h) (i32.const 0) (i32.const 32)))
    (call $report (i32.const {base}) (i64.load (i32.const 32)))
    (call $report (i32.const {set_slot}) (i64.extend_i32_s (call $set (local.get $h) (i32.const 0) (i64.const 99))))
    (call $report (i32.const {original_slot}) (i64.extend_i32_s (call $original (local.get $h) (i32.const 64))))
    (call $report (i32.const {result_slot}) (i64.load (i32.const 64)))
    (i64.const {base})))"#,
        set_slot = base + 1,
        original_slot = base + 2,
        result_slot = base + 3,
    )
}

fn load(wat: &str, name: &str) -> ModId {
    runtime::load(wat.as_bytes(), ModConfig::new(""), name).unwrap()
}

#[test]
fn two_shared_mods_form_the_same_chain() {
    runtime::uninstall().unwrap();
    let outer = load(&chain_guest(10, 10), "outer");
    let inner = load(&chain_guest(20, 20), "inner");
    let thing = Thing { count: 5 };
    let mut runs = 0;
    assert_eq!(engine(3, &thing, &mut runs), 10);
    let outer_reports = runtime::take_reports_of(outer).unwrap();
    let inner_reports = runtime::take_reports_of(inner).unwrap();
    assert_eq!(slot(&outer_reports, 10), 5);
    assert_eq!(slot(&outer_reports, 11), 7);
    assert_eq!(slot(&outer_reports, 12), 0);
    assert_eq!(slot(&outer_reports, 13), 20, "the outer original ran the inner guest");
    assert_eq!(slot(&inner_reports, 20), 5);
    assert_eq!(slot(&inner_reports, 21), 7);
    assert_eq!(slot(&inner_reports, 22), 0);
    assert_eq!(slot(&inner_reports, 23), 42, "the inner original ran the engine");
    assert_eq!((thing.count, runs), (5, 1));
    runtime::uninstall().unwrap();
}

#[test]
fn shared_reentry_uses_a_nested_activation() {
    install();
    let thing = Thing { count: 5 };
    let mut outer_runs = 0;
    let mut inner_runs = 0;
    let result = runtime::dispatch_id_ref(6, &thing, &TABLE, &mut |_| {
        outer_runs += 1;
        runtime::dispatch_id_ref(7, &thing, &TABLE, &mut |_| {
            inner_runs += 1;
            100
        })
    });
    assert_eq!(result, 89);
    assert_eq!((outer_runs, inner_runs), (1, 0));
    let reports = reports();
    assert_eq!(slot(&reports, 60), 0);
    assert_eq!(slot(&reports, 61), 23);
    runtime::uninstall().unwrap();
}

#[test]
fn shared_trap_before_original_runs_the_fallback_once() {
    install();
    let thing = Thing { count: 5 };
    let mut runs = 0;
    assert_eq!(engine(4, &thing, &mut runs), 42);
    assert_eq!(runs, 1);
    assert!(runtime::stats().disabled);
    runtime::uninstall().unwrap();
}

#[test]
fn shared_trap_after_original_does_not_run_it_twice() {
    install();
    let thing = Thing { count: 5 };
    let mut runs = 0;
    assert_eq!(engine(5, &thing, &mut runs), 42);
    assert_eq!(runs, 1);
    assert!(runtime::stats().disabled);
    runtime::uninstall().unwrap();
}
