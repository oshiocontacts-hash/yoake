//! MX3E1: a WASM guest reaches the generated glue through the `engine_call` import, on a real
//! `ArmorStand` inside the real S2 hook of `tick_settle`.
//!
//!   cargo test --offline -j 4 -p horizon-client-lib --features yoake2-slice --test yoake2_engine_call
//!
//! The guests are WAT generated per scenario (binding id and argument bytes are data segments), so the
//! ids always come from the generated tables, never from a copy.

use horizon_client_lib::entities::{self, ArmorStand, StandMedium, StandStep};
use horizon_client_lib::items;
use horizon_core::geometry::Vec3;
use std::cell::Cell;
use std::sync::Once;
use yoake_hook_rt::Order;
use yoake_slice::activation::{FieldTable, live_activations};
use yoake_slice::codec::{CallError, Encode};
use yoake_slice::runtime::{self, ModConfig, call_status, set_engine_dispatch};

type SettleCallback = fn(&mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep), &mut ArmorStand, StandMedium, StandStep);

fn table() -> &'static FieldTable<ArmorStand> {
    &entities::yoake_glue::FIELDS_crate__entities__ArmorStand
}

fn wasm_settle(original: &mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep), stand: &mut ArmorStand, medium: StandMedium, step: StandStep) {
    runtime::dispatch(stand, table(), &mut |stand| {
        original(stand, medium, step);
        0
    });
}

thread_local! {
    static ORIGINAL_RUNS: Cell<u32> = const { Cell::new(0) };
}

fn counting_entry(original: &mut dyn FnMut(&mut ArmorStand, StandMedium, StandStep), stand: &mut ArmorStand, medium: StandMedium, step: StandStep) {
    ORIGINAL_RUNS.set(ORIGINAL_RUNS.get() + 1);
    original(stand, medium, step);
}

/// What the runtime dispatches through: each module's generated table in turn (ids are disjoint).
fn dispatch(handle: u64, id: u32, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {
    match entities::yoake_glue::call(handle, id, args, out) {
        Err(CallError::UnknownBinding(_)) => items::yoake_glue::call(handle, id, args, out),
        other => other,
    }
}

fn setup() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        yoake_hook_rt::register::<SettleCallback>(ArmorStand::__YOAKE_HOOK_ID_tick_settle, wasm_settle, Order { priority: 10, mod_load_order: 0 }).unwrap();
        yoake_hook_rt::register::<SettleCallback>(ArmorStand::__YOAKE_HOOK_ID_tick_settle, counting_entry, Order { priority: 0, mod_load_order: 0 }).unwrap();
        set_engine_dispatch(Some(dispatch));
    });
    ORIGINAL_RUNS.set(0);
}

fn id_of(path: &str) -> u32 {
    entities::yoake_glue::BINDINGS.iter().chain(items::yoake_glue::BINDINGS).find(|(_, p)| *p == path).unwrap_or_else(|| panic!("no binding {path}")).0
}

fn bytes(f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut v = Vec::new();
    f(&mut v);
    v
}

/// A guest whose hook makes one `engine_call` and reports: slot 1 = status, slot 2 = the first 8 bytes
/// of the out buffer, slot 3 = the next 8. It never calls `original`.
fn guest(id: u32, args: &[u8], len: i32, cap: i32) -> String {
    let data: String = args.iter().map(|b| format!("\\{b:02x}")).collect();
    format!(
        r#"(module
  (import "env" "engine_call" (func $ec (param i64 i32 i32 i32 i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (data (i32.const 256) "{data}")
  (func (export "hook") (param $h i64) (result i64)
    (call $report (i32.const 1) (i64.extend_i32_s (call $ec (local.get $h) (i32.const {id}) (i32.const 256) (i32.const {len}) (i32.const 512) (i32.const {cap}))))
    (call $report (i32.const 2) (i64.load (i32.const 512)))
    (call $report (i32.const 3) (i64.load (i32.const 520)))
    (i64.const 0)))"#
    )
}

fn install(wat: &str) {
    runtime::install(wat.as_bytes(), ModConfig::new("hook")).unwrap();
}

fn stand() -> ArmorStand {
    ArmorStand::new(1, Vec3::new(3.0, 64.0, 5.0), 30.0)
}

fn step() -> StandStep {
    StandStep::for_hook_bench(true, 0.6)
}

fn reports(slot: i32) -> Vec<i64> {
    runtime::take_reports().unwrap().iter().filter(|(s, _)| *s == slot).map(|(_, v)| *v).collect()
}

fn run_guest(wat: &str) -> (ArmorStand, Vec<(i32, i64)>) {
    setup();
    install(wat);
    let mut s = stand();
    s.tick_settle(StandMedium::Air, step());
    let r = runtime::take_reports().unwrap();
    (s, r)
}

fn slot(r: &[(i32, i64)], n: i32) -> i64 {
    r.iter().find(|(s, _)| *s == n).unwrap_or_else(|| panic!("no report {n}")).1
}

#[test]
fn a_guest_calls_a_generated_binding_on_a_real_armor_stand_inside_a_hook() {
    let control = stand();
    let expected = control.render_yaw(0.5);
    let args = bytes(|b| 0.5f32.encode(b));
    let (_, r) = run_guest(&guest(id_of("entities::ArmorStand::render_yaw"), &args, 4, 64));
    assert_eq!(slot(&r, 1), 4, "four result bytes");
    assert_eq!(slot(&r, 2) as u32, expected.to_bits(), "the f32 the engine computed");
    assert_eq!(live_activations(), 0);

    // a writable field is written through the glue, on the very object the hook received
    let args = bytes(|b| 77i32.encode(b));
    let (s, r) = run_guest(&guest(id_of("entities::ArmorStand.fire_ticks#set"), &args, 4, 64));
    assert_eq!(slot(&r, 1), 0);
    assert_eq!(s.fire_ticks, 77);

    // a frozen field (the id) is read-only: the setter id is reserved and refused
    let args = bytes(|b| 9u32.encode(b));
    let (s, r) = run_guest(&guest(id_of("entities::ArmorStand.id#set"), &args, 4, 64));
    assert_eq!(slot(&r, 1), i64::from(call_status::READ_ONLY));
    assert_eq!(s.id, 1);
    // while the getter works
    let (_, r) = run_guest(&guest(id_of("entities::ArmorStand.id#get"), &[], 0, 64));
    assert_eq!(slot(&r, 1), 4);
    assert_eq!(slot(&r, 2) as u32, 1);
    // an unknown id and a malformed call are statuses, not traps
    let (_, r) = run_guest(&guest(999_999, &[], 0, 64));
    assert_eq!(slot(&r, 1), i64::from(call_status::UNKNOWN_BINDING));
    let (_, r) = run_guest(&guest(id_of("entities::ArmorStand::render_yaw"), &[1, 2], 2, 64));
    assert_eq!(slot(&r, 1), i64::from(call_status::BAD_ARGS));
    assert!(!runtime::stats().disabled, "none of these disables the mod");
}

#[test]
fn oversized_arguments_are_refused_before_anything_is_read() {
    // 1 MiB + 1 claimed from a one-page guest: refused by length, the memory is never touched
    let (_, r) = run_guest(&guest(id_of("entities::ArmorStand::render_yaw"), &[], (runtime::MAX_CALL_ARG_BYTES + 1) as i32, 64));
    assert_eq!(slot(&r, 1), i64::from(call_status::ARGS_TOO_LARGE));
    let (_, r) = run_guest(&guest(id_of("entities::ArmorStand::render_yaw"), &[], -5, 64));
    assert_eq!(slot(&r, 1), i64::from(call_status::ARGS_TOO_LARGE), "a negative length");
    assert!(!runtime::stats().disabled);
}

#[test]
fn a_result_that_does_not_fit_returns_the_needed_size_not_a_trap() {
    let pos = id_of("entities::ArmorStand.position#get");
    let (_, r) = run_guest(&guest(pos, &[], 0, 8));
    assert_eq!(slot(&r, 1), i64::from(call_status::OUT_TOO_SMALL));
    assert_eq!(slot(&r, 2) as u32, 24, "three f64: the size to retry with is written at out_ptr");
    assert_eq!(slot(&r, 3), 0, "nothing else was written");
    let (_, r) = run_guest(&guest(pos, &[], 0, 2));
    assert_eq!(slot(&r, 1), i64::from(call_status::OUT_TOO_SMALL));
    assert_eq!(slot(&r, 2), 0, "no room even for the size: nothing written");
    let (_, r) = run_guest(&guest(pos, &[], 0, 24));
    assert_eq!(slot(&r, 1), 24);
    assert!(!runtime::stats().disabled);
}

#[test]
fn a_guest_provoked_engine_panic_disables_the_mod_and_the_game_goes_on() {
    // `is_disabled(99)` indexes `[0, 5, 1, 2, 3, 4][99]`: the engine panics on the guest's argument
    let args = bytes(|b| 99usize.encode(b));
    setup();
    install(&guest(id_of("entities::ArmorStand::is_disabled"), &args, 8, 64));
    let mut s = stand();
    s.tick_settle(StandMedium::Air, step());
    let stats = runtime::stats();
    assert!(stats.disabled, "the panic is the trap rule");
    assert!(stats.trap.as_deref().unwrap_or("").contains("engine panic inside engine_call"), "{:?}", stats.trap);
    assert_eq!(ORIGINAL_RUNS.get(), 1, "the guest never ran `original`, so it runs once");
    assert_eq!(live_activations(), 0);
    assert_eq!((stats.depth, stats.epoch_depth), (0, 0));
    // never entered again, and the game goes on
    s.tick_settle(StandMedium::Air, step());
    s.tick_settle(StandMedium::Air, step());
    assert_eq!(ORIGINAL_RUNS.get(), 3);
    assert_eq!(runtime::stats().entries, 1);
    // the activation lock was released by the unwind: a fresh mod works on the same thread
    let args = bytes(|b| 0.5f32.encode(b));
    install(&guest(id_of("entities::ArmorStand::render_yaw"), &args, 4, 64));
    s.tick_settle(StandMedium::Air, step());
    assert!(!runtime::stats().disabled);
    assert_eq!(slot(&runtime::take_reports().unwrap(), 1), 4);
    let _ = reports;
}
