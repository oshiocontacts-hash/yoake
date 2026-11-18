//! MX3E1: a WASM guest reaches the generated glue through the `engine_call` import, on a real
//! `ArmorStand` inside the real S2 hook of `tick_settle`.
//!
//!   cargo test --offline -j 4 -p horizon-client-lib --features yoake2-slice --test yoake2_engine_call
//!
//! The guests are WAT generated per scenario (binding id and argument bytes are data segments), so the
//! ids always come from the generated tables, never from a copy.

use horizon_client_lib::entities::{self, ArmorStand, ItemEntity, StandMedium, StandStep};
use horizon_client_lib::items::ItemStack;
use std::sync::Arc;
use horizon_client_lib::items;
use horizon_core::geometry::Vec3;
use std::cell::Cell;
use std::sync::Once;
use yoake_hook_rt::Order;
use yoake_slice::activation::{self, Accessor, FieldTable, PathRoot, Status, live_activations};
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

thread_local! {
    /// How many times the engine side really ran (a non-idempotent counter: a retry would show up).
    static ENGINE_RUNS: Cell<u32> = const { Cell::new(0) };
}

/// The id of the hand-written "get a path handle for a dropped item" binding. It mirrors what the
/// generator emits for `Game.<name>#handle` (that glue lives in `horizon-client`, which needs a real
/// `Game`; the generated one is exercised by the fixture in yoake-bindgen).
const GET_ITEM: u32 = 4_000_000;

/// What the runtime dispatches through: each module's generated table in turn (ids are disjoint).
fn dispatch(handle: u64, id: u32, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {
    ENGINE_RUNS.set(ENGINE_RUNS.get() + 1);
    if id == GET_ITEM {
        let key = u32::from_le_bytes(args.try_into().map_err(|_| CallError::UnexpectedEnd)?);
        let path = activation::open_path(handle, vec![Accessor::Field(0), Accessor::Id(u64::from(key))])?;
        path.encode(out);
        return Ok(());
    }
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

/// A guest that calls once, then asks for the kept result: slot 1 = the `engine_call` status, 4 = the ticket
/// (from the status), 5 = the `engine_take` status, 6 = a second `engine_take` of the same ticket,
/// 2 and 3 = what was written at 512 and 520 after the first take.
fn take_guest(id: u32, args: &[u8], cap: i32, take_cap: i32) -> String {
    let data: String = args.iter().map(|b| format!("\\{b:02x}")).collect();
    format!(
        r#"(module
  (import "env" "engine_call" (func $ec (param i64 i32 i32 i32 i32 i32) (result i32)))
  (import "env" "engine_take" (func $et (param i32 i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (data (i32.const 256) "{data}")
  (func (export "hook") (param $h i64) (result i64)
    (local $s i32) (local $t i32)
    (local.set $s (call $ec (local.get $h) (i32.const {id}) (i32.const 256) (i32.const {len}) (i32.const 512) (i32.const {cap})))
    (call $report (i32.const 1) (i64.extend_i32_s (local.get $s)))
    (local.set $t (i32.sub (i32.sub (i32.const 0) (local.get $s)) (i32.const 256)))
    (call $report (i32.const 4) (i64.extend_i32_s (local.get $t)))
    (call $report (i32.const 5) (i64.extend_i32_s (call $et (local.get $t) (i32.const 1024) (i32.const {take_cap}))))
    (call $report (i32.const 2) (i64.load (i32.const 1024)))
    (call $report (i32.const 3) (i64.load (i32.const 1032)))
    (call $report (i32.const 6) (i64.extend_i32_s (call $et (local.get $t) (i32.const 1024) (i32.const 64))))
    (i64.const 0)))"#,
        len = args.len()
    )
}

#[test]
fn a_result_that_does_not_fit_is_kept_under_a_ticket_and_never_run_twice() {
    let pos = id_of("entities::ArmorStand.position#get");
    setup();
    // cap 8 < 24: the call HAS run (once); the status carries the ticket, the size is written at out_ptr
    ENGINE_RUNS.set(0);
    install(&take_guest(pos, &[], 8, 64));
    let mut s = stand();
    s.position = Vec3::new(1.5, 2.5, 3.5);
    s.tick_settle(StandMedium::Air, step());
    let r = runtime::take_reports().unwrap();
    let status = slot(&r, 1);
    assert!(status <= -257, "OUT_TOO_SMALL carries a ticket: {status}");
    assert_eq!(call_status::ticket_of(status as i32), Some(slot(&r, 4) as u32));
    assert_eq!(slot(&r, 5), 24, "engine_take copied the 24 kept bytes");
    assert_eq!(f64::from_bits(slot(&r, 2) as u64), 1.5, "the first f64 of the result");
    assert_eq!(f64::from_bits(slot(&r, 3) as u64), 2.5);
    assert_eq!(slot(&r, 6), i64::from(call_status::NO_SUCH_TICKET), "a ticket is spent by its take");
    assert_eq!(ENGINE_RUNS.get(), 1, "the engine ran once; nothing was re-run to fetch the result");
    assert!(!runtime::stats().disabled);

    // the take buffer may be too small as well: the size comes back and the ticket stays
    ENGINE_RUNS.set(0);
    install(&take_guest(pos, &[], 8, 8));
    stand().tick_settle(StandMedium::Air, step());
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 5), i64::from(call_status::OUT_TOO_SMALL));
    assert_eq!(slot(&r, 6), 24, "the second take, with room, still gets the result");
    assert_eq!(ENGINE_RUNS.get(), 1);

    // no room even for the size (cap 2): the ticket is still issued, nothing written
    ENGINE_RUNS.set(0);
    install(&take_guest(pos, &[], 2, 64));
    stand().tick_settle(StandMedium::Air, step());
    let r = runtime::take_reports().unwrap();
    assert!(slot(&r, 1) <= -257);
    assert_eq!(slot(&r, 5), 24);
    assert_eq!(ENGINE_RUNS.get(), 1);
}

/// A mutating, non-idempotent call whose result does not fit: its effect must happen exactly once.
#[test]
fn a_mutating_call_with_a_too_small_buffer_changes_the_object_once() {
    setup();
    // `Experience`-free: `ArmorStand::interpolate` mutates, so use a setter + getter pair in one guest:
    // fire_ticks = 77, then read the (24-byte) position with a tiny buffer and take it.
    let set = id_of("entities::ArmorStand.fire_ticks#set");
    let pos = id_of("entities::ArmorStand.position#get");
    let args = bytes(|b| 77i32.encode(b));
    let wat = format!(
        r#"(module
  (import "env" "engine_call" (func $ec (param i64 i32 i32 i32 i32 i32) (result i32)))
  (import "env" "engine_take" (func $et (param i32 i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (data (i32.const 256) "\4d\00\00\00")
  (func (export "hook") (param $h i64) (result i64)
    (local $s i32)
    (call $report (i32.const 1) (i64.extend_i32_s (call $ec (local.get $h) (i32.const {set}) (i32.const 256) (i32.const 4) (i32.const 512) (i32.const 0))))
    (local.set $s (call $ec (local.get $h) (i32.const {pos}) (i32.const 0) (i32.const 0) (i32.const 512) (i32.const 1)))
    (call $report (i32.const 2) (i64.extend_i32_s (local.get $s)))
    (call $report (i32.const 3) (i64.extend_i32_s (call $et (i32.sub (i32.sub (i32.const 0) (local.get $s)) (i32.const 256)) (i32.const 1024) (i32.const 64))))
    (i64.const 0)))"#
    );
    assert_eq!(args, [77, 0, 0, 0]);
    ENGINE_RUNS.set(0);
    install(&wat);
    let mut s = stand();
    s.tick_settle(StandMedium::Air, step());
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 1), 0, "the 4-byte setter has no result and fits any buffer");
    assert!(slot(&r, 2) <= -257);
    assert_eq!(slot(&r, 3), 24);
    assert_eq!(s.fire_ticks, 77);
    assert_eq!(ENGINE_RUNS.get(), 2, "one setter + one getter, no retry");
}

#[test]
fn tickets_are_bounded_and_die_with_the_activation() {
    setup();
    let pos = id_of("entities::ArmorStand.position#get");
    // nine too-small calls in one activation: eight tickets, the ninth result is lost with TICKET_LIMIT
    let mut calls = String::new();
    for _ in 0..9 {
        calls.push_str(&format!(
            "(call $report (i32.const 1) (i64.extend_i32_s (call $ec (local.get $h) (i32.const {pos}) (i32.const 0) (i32.const 0) (i32.const 512) (i32.const 0))))\n"
        ));
    }
    let wat = format!(
        r#"(module
  (import "env" "engine_call" (func $ec (param i64 i32 i32 i32 i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "hook") (param $h i64) (result i64)
    {calls}
    (i64.const 0)))"#
    );
    install(&wat);
    stand().tick_settle(StandMedium::Air, step());
    let statuses = reports(1);
    assert_eq!(statuses.len(), 9);
    assert!(statuses[..8].iter().all(|s| *s <= -257), "{statuses:?}");
    assert_eq!(statuses[8], i64::from(call_status::TICKET_LIMIT));
    // a later activation cannot take an earlier one's ticket
    let take = r#"(module
  (import "env" "engine_take" (func $et (param i32 i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "hook") (param $h i64) (result i64)
    (call $report (i32.const 1) (i64.extend_i32_s (call $et (i32.const 1) (i32.const 512) (i32.const 64))))
    (i64.const 0)))"#;
    install(take);
    stand().tick_settle(StandMedium::Air, step());
    assert_eq!(reports(1), [i64::from(call_status::NO_SUCH_TICKET)]);
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

// ---- Root::Game accessor paths (MX3R1) ----

/// A stand-in for `Game` with the same shape of resolver the generator emits: `dropped` by id.
struct World {
    dropped: Vec<ItemEntity>,
}

impl PathRoot for World {
    fn resolve(&mut self, path: &[Accessor], f: &mut dyn FnMut(&mut dyn std::any::Any)) -> Result<(), Status> {
        match path {
            [Accessor::Field(0), Accessor::Id(id)] => match self.dropped.iter_mut().find(|e| u64::from(e.id) == *id) {
                Some(e) => {
                    f(e);
                    Ok(())
                }
                None => Err(Status::Stale),
            },
            _ => Err(Status::BadField),
        }
    }
}

fn item(id: u32, x: f64) -> ItemEntity {
    let at = Vec3::new(x, 64.0, 0.5);
    ItemEntity {
        join: Default::default(),
        no_gravity: false,
        id,
        thrower: None,
        target: None,
        stack: ItemStack { item: Arc::from("minecraft:stone"), count: 3, damage: 0, extra: None },
        health: 5,
        fire_ticks: 0,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        on_ground: true,
        vertical_collision: false,
        horizontal_collision: false,
        age: 0,
        pickup_delay: 0,
        bob_offset: 0.0,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
    }
}

/// The guest of a Game-root hook: get a handle for item `key`, read its position (slot 3 = x bits), write
/// `fire_ticks`, read it back (slot 4), and report each status (10, 11, 12, 13).
fn root_guest(get: u32, pos: u32, set: u32, getter: u32, key: u32) -> String {
    let key_bytes: String = key.to_le_bytes().iter().map(|b| format!("\\{b:02x}")).collect();
    format!(
        r#"(module
  (import "env" "engine_call" (func $ec (param i64 i32 i32 i32 i32 i32) (result i32)))
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (data (i32.const 256) "{key_bytes}")
  (data (i32.const 272) "\37\00\00\00")
  (func (export "hook") (param $h i64) (result i64)
    (local $hh i64)
    (call $report (i32.const 10) (i64.extend_i32_s (call $ec (local.get $h) (i32.const {get}) (i32.const 256) (i32.const 4) (i32.const 512) (i32.const 16))))
    (local.set $hh (i64.load (i32.const 512)))
    (call $report (i32.const 11) (i64.extend_i32_s (call $ec (local.get $hh) (i32.const {pos}) (i32.const 0) (i32.const 0) (i32.const 600) (i32.const 64))))
    (call $report (i32.const 3) (i64.load (i32.const 600)))
    (call $report (i32.const 12) (i64.extend_i32_s (call $ec (local.get $hh) (i32.const {set}) (i32.const 272) (i32.const 4) (i32.const 700) (i32.const 8))))
    (call $report (i32.const 13) (i64.extend_i32_s (call $ec (local.get $hh) (i32.const {getter}) (i32.const 0) (i32.const 0) (i32.const 720) (i32.const 8))))
    (call $report (i32.const 4) (i64.load32_u (i32.const 720)))
    ;; the engine removes the item while `original` runs; the handle is Stale afterwards
    (drop (call $orig (local.get $h) (i32.const 800)))
    (call $report (i32.const 14) (i64.extend_i32_s (call $ec (local.get $hh) (i32.const {pos}) (i32.const 0) (i32.const 0) (i32.const 600) (i32.const 64))))
    (i64.const 0)))"#
    )
}

#[test]
fn a_guest_in_a_game_root_hook_reaches_an_entity_by_id_through_a_path_handle() {
    setup();
    let (pos, set, getter) = (
        id_of("entities::ItemEntity.position#get"),
        id_of("entities::ItemEntity.fire_ticks#set"),
        id_of("entities::ItemEntity.fire_ticks#get"),
    );
    install(&root_guest(GET_ITEM, pos, set, getter, 7));
    let mut world = World { dropped: vec![item(6, 0.5), item(7, 12.25)] };
    let runs = Cell::new(0);
    runtime::dispatch_root(&mut world, &mut |w: &mut World| {
        runs.set(runs.get() + 1);
        w.dropped.retain(|e| e.id != 7);
        0
    });
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 10), 8, "the path handle is eight bytes");
    assert_eq!(slot(&r, 11), 24);
    assert_eq!(f64::from_bits(slot(&r, 3) as u64), 12.25, "the position of item 7");
    assert_eq!(slot(&r, 12), 0, "fire_ticks is writable");
    assert_eq!(slot(&r, 13), 4);
    assert_eq!(slot(&r, 4), 0x37, "the write landed on the item (0x37 = 55)");
    assert_eq!(slot(&r, 14), i64::from(call_status::HANDLE_BASE) - Status::Stale as i64, "the item was removed: Stale");
    assert_eq!(runs.get(), 1);
    assert_eq!(world.dropped.len(), 1, "the other item is untouched");
    assert_eq!(world.dropped[0].id, 6);
    assert!(!runtime::stats().disabled);
    assert_eq!(activation::live_path_handles(), 0, "freed with the activation");
    assert_eq!(live_activations(), 0);
}

#[test]
fn a_missing_id_is_stale_and_a_non_game_hook_has_no_root() {
    setup();
    let (pos, set, getter) = (
        id_of("entities::ItemEntity.position#get"),
        id_of("entities::ItemEntity.fire_ticks#set"),
        id_of("entities::ItemEntity.fire_ticks#get"),
    );
    install(&root_guest(GET_ITEM, pos, set, getter, 99));
    let mut world = World { dropped: vec![item(6, 0.5)] };
    runtime::dispatch_root(&mut world, &mut |_: &mut World| 0);
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 10), i64::from(call_status::HANDLE_BASE) - Status::Stale as i64, "no item 99");
    // inside `ArmorStand::tick_settle` the handle is a hook argument, not a root: asking it for a path is refused
    install(&root_guest(GET_ITEM, pos, set, getter, 6));
    stand().tick_settle(StandMedium::Air, step());
    let r = runtime::take_reports().unwrap();
    assert_eq!(slot(&r, 10), i64::from(call_status::HANDLE_BASE) - Status::WrongType as i64, "a non-Game hook cannot reach Game");
    assert!(!runtime::stats().disabled);
}
