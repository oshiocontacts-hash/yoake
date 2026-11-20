//! W8: a hook on a fn with five arguments (two `&mut Thing`, a by-value `i32`, two `&Thing`). The guest reads every
//! argument, replaces the value, swaps the two `&mut` objects, replaces one shared argument by the other, and
//! `original` receives all of it. The refusals (aliasing a `&mut`, a value call on an object) are statuses, not traps.

use std::cell::Cell;

use yoake_slice::activation::{ArgSet, ArgView, FieldTable, RefFieldTable};
use yoake_slice::runtime::{self, ModConfig, ModId};

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
static REF_TABLE: RefFieldTable<Thing> = RefFieldTable { get: |t, f| (f == 0).then(|| i64::from(t.count)) };

/// The hooked engine function (hook id 1).
fn engine(a: &mut Thing, n: i32, b: &mut Thing, c: &Thing, d: &Thing) -> i64 {
    let args = ArgSet::new().mut_ref(a, &TABLE).value(i64::from(n)).mut_ref(b, &TABLE).shared(c, &REF_TABLE).shared(d, &REF_TABLE);
    runtime::dispatch_n_id(1, args, &mut |view: &ArgView<'_>| {
        let a = view.mut_ref::<Thing>(0).unwrap();
        let n = view.value(1).unwrap() as i32;
        let b = view.mut_ref::<Thing>(2).unwrap();
        let c = view.shared::<Thing>(3).unwrap();
        a.runs += 1;
        i64::from(a.count + n + b.count * 10 + c.count * 100)
    })
}

const GUEST: &str = r#"(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "get" (func $get (param i64 i32 i32) (result i32)))
  (import "env" "set" (func $set (param i64 i32 i64) (result i32)))
  (import "env" "arg_get" (func $aget (param i64 i32) (result i32)))
  (import "env" "arg_set" (func $aset (param i64 i64) (result i32)))
  (import "env" "arg_swap" (func $aswap (param i64 i64) (result i32)))
  (import "env" "arg_replace" (func $arep (param i64 i64) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func $field (param $h i64) (result i64)
    (drop (call $get (local.get $h) (i32.const 0) (i32.const 32)))
    (i64.load (i32.const 32)))
  (func $value (param $h i64) (result i64)
    (drop (call $aget (local.get $h) (i32.const 32)))
    (i64.load (i32.const 32)))
  (func (export "yoake_init")
    (drop (call $register (i32.const 1) (i32.const 0))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (call $report (i32.const 1) (call $field (local.get $h)))
    (call $report (i32.const 2) (call $value (i64.add (local.get $h) (i64.const 1))))
    (call $report (i32.const 3) (call $field (i64.add (local.get $h) (i64.const 2))))
    (call $report (i32.const 4) (call $field (i64.add (local.get $h) (i64.const 3))))
    (call $report (i32.const 5) (call $field (i64.add (local.get $h) (i64.const 4))))
    ;; refusals are statuses: a &mut cannot be replaced by another object (Overlap = 8), a value call on an object (WrongKind = 9)
    (call $report (i32.const 10) (i64.extend_i32_s (call $arep (local.get $h) (i64.add (local.get $h) (i64.const 2)))))
    (call $report (i32.const 11) (i64.extend_i32_s (call $aset (local.get $h) (i64.const 1))))
    (call $report (i32.const 12) (i64.extend_i32_s (call $arep (i64.add (local.get $h) (i64.const 3)) (local.get $h))))
    ;; replace every argument
    (drop (call $set (local.get $h) (i32.const 0) (i64.const 2)))
    (drop (call $aset (i64.add (local.get $h) (i64.const 1)) (i64.const 7)))
    (drop (call $aswap (local.get $h) (i64.add (local.get $h) (i64.const 2))))
    (drop (call $arep (i64.add (local.get $h) (i64.const 3)) (i64.add (local.get $h) (i64.const 4))))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (i64.load (i32.const 64))))"#;

fn load(wat: &str) -> ModId {
    runtime::load(wat.as_bytes(), ModConfig::new(""), "nary").unwrap()
}

fn slot(r: &[(i32, i64)], n: i32) -> i64 {
    r.iter().find(|(s, _)| *s == n).unwrap_or_else(|| panic!("no report {n} in {r:?}")).1
}

#[test]
fn a_guest_reads_and_replaces_every_argument_of_a_five_argument_hook() {
    runtime::uninstall().unwrap();
    let id = load(GUEST);
    let (mut a, mut b) = (Thing { count: 1, runs: 0 }, Thing { count: 3, runs: 0 });
    let (c, d) = (Thing { count: 4, runs: 0 }, Thing { count: 5, runs: 0 });
    let result = engine(&mut a, 2, &mut b, &c, &d);
    let r = runtime::take_reports_of(id).unwrap();
    assert_eq!([slot(&r, 1), slot(&r, 2), slot(&r, 3), slot(&r, 4), slot(&r, 5)], [1, 2, 3, 4, 5], "every argument was readable");
    assert_eq!(slot(&r, 10), 8, "Overlap");
    assert_eq!(slot(&r, 11), 9, "WrongKind");
    assert_eq!(slot(&r, 12), 8, "a shared argument cannot take a &mut's object either");
    // original saw: a' = b (3), n = 7, b' = a (count set to 2), c' = d (5)
    assert_eq!(result, i64::from(3 + 7 + 2 * 10 + 5 * 100));
    assert_eq!(b.runs, 1, "the swapped-in object is the one the body ran on");
    assert_eq!(a.runs, 0);
    assert_eq!((a.count, b.count), (2, 3), "the field write reached its owner; nothing else moved");
    runtime::uninstall().unwrap();
}

#[test]
fn without_a_mod_the_original_runs_on_the_arguments_as_given() {
    runtime::uninstall().unwrap();
    let (mut a, mut b) = (Thing { count: 1, runs: 0 }, Thing { count: 3, runs: 0 });
    let (c, d) = (Thing { count: 4, runs: 0 }, Thing { count: 5, runs: 0 });
    assert_eq!(engine(&mut a, 2, &mut b, &c, &d), i64::from(1 + 2 + 30 + 400));
}

#[test]
fn two_mods_compose_and_the_second_sees_the_first_ones_replacements() {
    runtime::uninstall().unwrap();
    // A (priority 0) sets the value to 100 and calls original; B (priority 1) reports the value it sees.
    let a = r#"(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "arg_set" (func $aset (param i64 i64) (result i32)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (drop (call $register (i32.const 1) (i32.const 0))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (drop (call $aset (i64.add (local.get $h) (i64.const 1)) (i64.const 100)))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (i64.load (i32.const 64))))"#;
    let b = r#"(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "arg_get" (func $aget (param i64 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (drop (call $register (i32.const 1) (i32.const 1))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (drop (call $aget (i64.add (local.get $h) (i64.const 1)) (i32.const 32)))
    (call $report (i32.const 1) (i64.load (i32.const 32)))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (i64.load (i32.const 64))))"#;
    let _a = load(a);
    let b = runtime::load(b.as_bytes(), ModConfig::new(""), "b").unwrap();
    let (mut x, mut y) = (Thing { count: 1, runs: 0 }, Thing { count: 3, runs: 0 });
    let (c, d) = (Thing { count: 4, runs: 0 }, Thing { count: 5, runs: 0 });
    let result = engine(&mut x, 2, &mut y, &c, &d);
    assert_eq!(slot(&runtime::take_reports_of(b).unwrap(), 1), 100, "B sees the value A put in");
    assert_eq!(result, i64::from(1 + 100 + 30 + 400));
    let seen = Cell::new(0);
    seen.set(1);
    runtime::uninstall().unwrap();
}
