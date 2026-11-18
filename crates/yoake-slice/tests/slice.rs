//! MX3V1 slice tests against a small hand-hooked "engine" (no horizon-client): the same checks run
//! against the real `ArmorStand::tick_settle` in horizon-client-lib's `yoake2_slice` test.

use std::cell::{Cell, RefCell};
use yoake_slice::GUEST_WAT;
use yoake_slice::activation::{FieldTable, live_activations};
use yoake_slice::runtime::{self, ModConfig};

#[derive(Default)]
struct Thing {
    counter: i32,
    x: f64,
    velocity: i64,
}

static TABLE: FieldTable<Thing> = FieldTable {
    get: |t, f| match f {
        0 => Some(i64::from(t.counter)),
        1 => Some(t.x.to_bits() as i64),
        _ => None,
    },
    set: |t, f, v| match f {
        0 => {
            t.counter = v as i32;
            true
        }
        1 => {
            t.x = f64::from_bits(v as u64);
            true
        }
        _ => false,
    },
};

thread_local! {
    static ORIGINAL_RUNS: Cell<u32> = const { Cell::new(0) };
    /// While set, the engine body ticks `OTHER` (a nested hook activation, as the S2 chain does).
    static NEST: Cell<bool> = const { Cell::new(false) };
    static OTHER: RefCell<Thing> = RefCell::new(Thing::default());
    static SLEEP_MS: Cell<u64> = const { Cell::new(0) };
}

/// The hooked engine function: `Thing::tick`, with a slot (here: `dispatch` directly).
fn tick(thing: &mut Thing) -> i64 {
    runtime::dispatch(thing, &TABLE, &mut |thing| real_tick(thing))
}

fn real_tick(thing: &mut Thing) -> i64 {
    ORIGINAL_RUNS.set(ORIGINAL_RUNS.get() + 1);
    thing.velocity += 1;
    let ms = SLEEP_MS.get();
    if ms > 0 {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    if NEST.replace(false) {
        OTHER.with(|other| tick(&mut other.borrow_mut()));
    }
    42
}

fn reports() -> Vec<(i32, i64)> {
    runtime::take_reports()
}
fn find(reports: &[(i32, i64)], slot: i32) -> Vec<i64> {
    reports.iter().filter(|(s, _)| *s == slot).map(|(_, v)| *v).collect()
}

fn install(export: &str) {
    runtime::install(GUEST_WAT.as_bytes(), ModConfig::new(export)).unwrap();
}

#[test]
fn guest_changes_self_state_and_the_game_sees_it() {
    install("hook_basic");
    let mut thing = Thing::default();
    tick(&mut thing);
    assert_eq!(thing.counter, 15, "5 before original, +10 after reading it back");
    assert_eq!(thing.x, 12.5);
    assert_eq!(thing.velocity, 1, "original ran once");
    assert_eq!(ORIGINAL_RUNS.get(), 1);
    let r = reports();
    assert_eq!(find(&r, 10), [0], "original status Ok");
    assert_eq!(find(&r, 11), [0]);
    assert_eq!(find(&r, 12), [5], "the write before original survived it");
    assert_eq!(live_activations(), 0, "arena freed at the end of the activation");
    assert_eq!(runtime::stats().entries, 1);
}

#[test]
fn reentrancy_and_the_lending_lock() {
    install("hook_reenter");
    NEST.set(true);
    let mut outer = Thing::default();
    tick(&mut outer);
    let inner_counter = OTHER.with(|o| o.borrow().counter);
    let r = reports();
    assert_eq!(find(&r, 40), [1, 2], "the guest was re-entered at depth 2 on the same instance");
    // status 2 = Locked: the nested activation may not use the outer handle while `original` runs
    assert_eq!(find(&r, 20), [2], "get on a lent handle");
    assert_eq!(find(&r, 21), [2], "set on a lent handle");
    assert_eq!(find(&r, 22), [2], "a second original on a lent activation");
    assert_eq!(find(&r, 23), [0], "its own handle works");
    assert_eq!(find(&r, 30), [0, 0], "own handle usable again after original");
    assert_eq!(outer.counter, 101, "outer: the rejected write 999 never landed");
    assert_eq!(inner_counter, 102);
    assert_eq!(outer.velocity, 1);
    assert_eq!(ORIGINAL_RUNS.get(), 2);
    assert_eq!(runtime::stats().entries, 2);
    assert!(!runtime::stats().disabled);
    assert_eq!(live_activations(), 0);
}

#[test]
fn stale_and_forged_handles_are_errors() {
    install("hook_stale");
    let mut thing = Thing::default();
    tick(&mut thing);
    tick(&mut thing);
    let r = reports();
    assert_eq!(find(&r, 50), [1], "handle of an ended activation: Stale");
    assert_eq!(find(&r, 51), [4], "forged argument index: NoSuchArg");
    assert_eq!(find(&r, 52), [3], "unknown field: BadField");
    assert_eq!(thing.velocity, 2);
}

#[test]
fn trap_before_original_runs_it_exactly_once_and_disables_the_mod() {
    install("hook_trap_before");
    let mut thing = Thing::default();
    let result = tick(&mut thing);
    assert_eq!(result, 42, "the original's result");
    assert_eq!(ORIGINAL_RUNS.get(), 1);
    assert_eq!(thing.velocity, 1);
    assert_eq!(thing.counter, 31, "what the guest wrote before trapping stays");
    let stats = runtime::stats();
    assert!(stats.disabled);
    assert!(stats.trap.unwrap().contains("unreachable"));
    assert_eq!(live_activations(), 0);
    // the game keeps running, the guest is never called again
    thing.counter = 0;
    tick(&mut thing);
    tick(&mut thing);
    assert_eq!(ORIGINAL_RUNS.get(), 3);
    assert_eq!(thing.counter, 0);
    assert_eq!(runtime::stats().entries, 1);
}

#[test]
fn trap_after_original_does_not_run_it_twice() {
    install("hook_trap_after");
    let mut thing = Thing::default();
    let result = tick(&mut thing);
    assert_eq!(result, 42, "recorded result of the one original run");
    assert_eq!(ORIGINAL_RUNS.get(), 1, "side effects never run twice");
    assert_eq!(thing.velocity, 1);
    assert_eq!(thing.counter, 55);
    assert!(runtime::stats().disabled);
    tick(&mut thing);
    assert_eq!(ORIGINAL_RUNS.get(), 2);
    assert_eq!(runtime::stats().entries, 1);
}

#[test]
fn nested_trap_leaves_the_outer_activation_running() {
    install("hook_trap_nested");
    NEST.set(true);
    let mut outer = Thing::default();
    tick(&mut outer);
    let inner = OTHER.with(|o| (o.borrow().velocity, o.borrow().counter));
    assert_eq!(inner, (1, 0), "the nested activation fell back to the original, once");
    assert_eq!(ORIGINAL_RUNS.get(), 2, "outer original + nested fallback, no double run");
    assert_eq!(outer.counter, 31, "the outer activation finished its hook");
    assert_eq!(outer.velocity, 1);
    let stats = runtime::stats();
    assert!(stats.disabled);
    assert_eq!(stats.entries, 2);
    tick(&mut outer);
    assert_eq!(runtime::stats().entries, 2);
    assert_eq!(live_activations(), 0);
}

#[test]
fn fuel_is_per_activation_and_a_spinning_hook_is_stopped() {
    // measure one hook_burn activation, then budget between one and two of them
    runtime::install(GUEST_WAT.as_bytes(), ModConfig::new("hook_burn")).unwrap();
    tick(&mut Thing::default());
    let one = runtime::stats().max_fuel_used;
    assert!(one > 1000, "{one}");
    let mut cfg = ModConfig::new("hook_burn");
    cfg.fuel = Some(one + one / 4);
    runtime::install(GUEST_WAT.as_bytes(), cfg).unwrap();
    NEST.set(true);
    tick(&mut Thing::default());
    let stats = runtime::stats();
    assert!(!stats.disabled, "two nested activations each fit their own budget: {stats:?}");
    assert_eq!(stats.entries, 2);
    assert!(stats.max_fuel_used > one - one / 8 && stats.max_fuel_used <= one + one / 4);

    let mut cfg = ModConfig::new("hook_spin");
    cfg.fuel = Some(50_000);
    runtime::install(GUEST_WAT.as_bytes(), cfg).unwrap();
    ORIGINAL_RUNS.set(0);
    let mut thing = Thing::default();
    assert_eq!(tick(&mut thing), 42);
    assert_eq!(ORIGINAL_RUNS.get(), 1);
    let stats = runtime::stats();
    assert!(stats.disabled);
    assert!(stats.trap.unwrap().contains("fuel"));
}

#[test]
fn epoch_budget_is_paused_while_original_runs() {
    SLEEP_MS.set(100);
    let mut cfg = ModConfig::new("hook_slow");
    cfg.epoch_ticks = Some(20);
    runtime::install(GUEST_WAT.as_bytes(), cfg.clone()).unwrap();
    let mut thing = Thing::default();
    tick(&mut thing);
    let stats = runtime::stats();
    assert!(!stats.disabled, "paused: the 100 ms in original is not charged: {stats:?}");
    assert_eq!(ORIGINAL_RUNS.get(), 1);

    // negative control: without the pause the same hook is interrupted after the slow original
    cfg.pause_epoch_in_original = false;
    runtime::install(GUEST_WAT.as_bytes(), cfg).unwrap();
    ORIGINAL_RUNS.set(0);
    let result = tick(&mut thing);
    let stats = runtime::stats();
    assert!(stats.disabled, "{stats:?}");
    assert!(stats.trap.unwrap().contains("interrupt"));
    assert_eq!(result, 42);
    assert_eq!(ORIGINAL_RUNS.get(), 1, "the original ran once, before the interrupt");
}

#[test]
fn hooks_do_not_fire_on_other_threads() {
    install("hook_basic");
    let mut here = Thing::default();
    tick(&mut here);
    assert_eq!(here.counter, 15);
    std::thread::spawn(|| {
        let mut there = Thing::default();
        let result = tick(&mut there);
        assert_eq!(result, 42);
        assert_eq!(there.counter, 0, "the guest did not run on a worker thread");
        assert_eq!(there.velocity, 1, "the original did");
        assert_eq!(ORIGINAL_RUNS.get(), 1);
        assert!(!runtime::stats().installed);
    })
    .join()
    .unwrap();
    assert_eq!(runtime::stats().entries, 1);
}

#[test]
fn no_mod_installed_is_a_plain_call() {
    runtime::uninstall();
    let mut thing = Thing::default();
    assert_eq!(tick(&mut thing), 42);
    assert_eq!((thing.counter, thing.velocity), (0, 1));
}
