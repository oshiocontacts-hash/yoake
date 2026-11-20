//! End-to-end SDK guest -> generated engine bridge -> function and call-site hooks.
//!
//! The guest is compiled to a wasm32 core module at test time. It registers
//! `ArmorStand::tick_settle` (id 71) and the `ItemEntity::hurt` call site in
//! `ItemEntity::environment_tick` (id 215), then delegates each to `original`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use horizon_client_lib::blocks::{BlockFacts, Fluid, RenderShape, Sounds, StateFacts};
use horizon_client_lib::entities::{ArmorStand, ItemEntity, StandMedium, StandStep, hook_bridge};
use horizon_client_lib::items::{ItemStack, Items};
use horizon_client_render::world::World;
use horizon_core::geometry::Vec3;
use horizon_world::nbt::Tag;
use horizon_world::registry::{BlockRegistry, BlockStateId};
use yoake_slice::runtime::{self, ModConfig};

#[path = "../../yoake-guest-sdk/tests/support/build.rs"]
mod build;

fn build_guest() -> Vec<u8> {
    let scratch = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("yoake2_bridge");
    let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
    let rlib = build::build_sdk(&sdk, &scratch);
    build::build_guest(
        &rlib,
        &sdk.join("tests/guests/bridge.rs"),
        &scratch,
        "yoake2_bridge",
    )
}

fn fixture() -> (World, BlockFacts, BlockRegistry, Items) {
    let air = BlockStateId(0);
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        (
            "states",
            Tag::List(vec![Tag::String("minecraft:air".into())]),
        ),
        (
            "defaults",
            Tag::compound([("minecraft:air", Tag::Int(0))]),
        ),
    ]))
    .unwrap();
    let facts = BlockFacts {
        states: vec![StateFacts {
            name: "minecraft:air".into(),
            emission: 0,
            dampening: 0,
            skylight_down: true,
            shape_occlusion: false,
            can_occlude: false,
            solid_render: false,
            hardness: 0.0,
            requires_tool: false,
            replaceable: true,
            random_ticks: false,
            block_entity: false,
            render_shape: RenderShape::Model,
            full_collision: false,
            friction: 0.6,
            explosion_resistance: 0.0,
            item: "minecraft:air".into(),
            sounds: Sounds {
                volume: 1.0,
                pitch: 1.0,
                break_sound: "".into(),
                step: "".into(),
                place: "".into(),
                hit: "".into(),
                fall: "".into(),
            },
            fluid: Fluid::None,
            fluid_level: 0,
            fluid_falling: false,
            waterlogged: false,
            random_offset: false,
            outline_offset: false,
            legacy_solid: false,
            sturdy_full: 0,
            classes: Arc::from([]),
            outline: Arc::from([]),
            outline_edges: Arc::from([]),
            map_color: 0,
        }],
        air,
        woods: HashMap::new(),
    };
    (
        World::new(air, 0),
        facts,
        registry,
        Items::for_tests(HashMap::new()),
    )
}

fn item() -> ItemEntity {
    let at = Vec3::new(0.5, 64.0, 0.5);
    ItemEntity {
        join: Default::default(),
        health: 5,
        fire_ticks: 20,
        id: 1,
        stack: ItemStack {
            item: Arc::from("minecraft:stone"),
            count: 1,
            damage: 0,
            extra: None,
        },
        thrower: None,
        target: None,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        on_ground: false,
        age: 0,
        pickup_delay: 0,
        bob_offset: 0.0,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
        no_gravity: false,
        vertical_collision: false,
        horizontal_collision: false,
    }
}

fn stand() -> ArmorStand {
    ArmorStand::new(1, Vec3::new(3.0, 64.0, 5.0), 30.0)
}

fn reports(all: &[(i32, i64)], slot: i32) -> Vec<i64> {
    all.iter()
        .filter(|(s, _)| *s == slot)
        .map(|(_, value)| *value)
        .collect()
}

#[test]
fn sdk_guest_reaches_function_and_callsite_hooks_and_originals_run() {
    let step = StandStep::for_hook_bench(true, 0.6);
    let mut control = stand();
    control.tick_settle(StandMedium::Air, step);

    runtime::install(&build_guest(), ModConfig::new("")).unwrap();
    assert_eq!(runtime::stats().registered, vec![(71, 0), (215, 5)]);
    let report = hook_bridge::install(&runtime::stats().registered);
    assert_eq!(report.installed, [71, 215], "{report:?}");
    assert!(report.not_mine.is_empty() && report.not_bridgeable.is_empty() && report.failed.is_empty(), "{report:?}");

    let mut hooked = stand();
    hooked.tick_settle(StandMedium::Air, step);
    assert_eq!(hooked.velocity, control.velocity, "function original ran once");

    let (world, facts, registry, items) = fixture();
    let mut item = item();
    let environment = item.environment_tick(&world, &facts, &registry, &items);
    assert!(environment.alive);
    assert!(!environment.lava_hurt);
    assert!(environment.spilled.is_empty());
    assert_eq!(item.health, 4, "call-site original hurt the item once");
    assert_eq!(item.fire_ticks, 19);

    let entries = runtime::stats().entries;
    assert!(item.hurt(0).is_none());
    assert_eq!(item.health, 4);
    assert_eq!(
        runtime::stats().entries,
        entries,
        "unregistered function hook id 10 did not enter the guest"
    );

    let all = runtime::take_reports().unwrap();
    assert_eq!(reports(&all, 10), [1], "function handler ran exactly once");
    assert_eq!(reports(&all, 11), [1], "function original succeeded");
    assert_eq!(reports(&all, 20), [1], "call-site handler ran exactly once");
    assert_eq!(reports(&all, 21), [1], "call-site original succeeded");
    assert!(reports(&all, 99).is_empty(), "no unregistered hook ran");
    let stats = runtime::stats();
    assert_eq!(stats.entries, 2);
    assert!(!stats.disabled, "{:?}", stats.trap);
}

// ---- Hook semantics through the generated bridge (review cases). Each test installs a WAT guest on its own thread
// (the runtime keeps mods per thread; the bridge dispatchers are process-wide and installed once).

/// A guest on ledger hook `id` whose `yoake_hook` reports `(1, 1)` on entry and then runs `body` (which must leave
/// the i64 result on the stack). `$orig` calls the original through the handle.
fn wat_guest(id: u32, body: &str) -> String {
    wat_guest_with(id, 0, 1, body)
}

/// [`wat_guest`] with the registration priority and the report slot chosen (for several mods on one id).
fn wat_guest_with(id: u32, priority: i32, slot: i32, body: &str) -> String {
    format!(
        r#"(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (drop (call $register (i32.const {id}) (i32.const {priority}))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    (call $report (i32.const {slot}) (i64.const 1))
    {body}))"#
    )
}

const CALL_ORIGINAL: &str = "(drop (call $orig (local.get $h) (i32.const 64)))";

fn with_guest<R>(id: u32, body: &str, f: impl FnOnce() -> R) -> R {
    runtime::uninstall().unwrap();
    runtime::install(wat_guest(id, body).as_bytes(), ModConfig::new("")).unwrap();
    let report = hook_bridge::install(&runtime::stats().registered);
    assert_eq!(report.installed, [id], "{report:?}");
    let out = f();
    out
}

fn entries() -> usize {
    reports(&runtime::take_reports().unwrap(), 1).len()
}

/// A fresh stand after `times` settle steps. Call it before installing a guest: once a guest is on hook 71 these
/// calls go through it too.
fn settled(times: usize) -> ArmorStand {
    let mut s = stand();
    for _ in 0..times {
        s.tick_settle(StandMedium::Air, StandStep::for_hook_bench(true, 0.6));
    }
    s
}

fn hooked_settle() -> ArmorStand {
    let mut s = stand();
    s.tick_settle(StandMedium::Air, StandStep::for_hook_bench(true, 0.6));
    s
}

#[test]
fn a_full_hook_whose_guest_does_not_call_original_is_cancelled() {
    let (zero, one) = (settled(0).velocity, settled(1).velocity);
    assert_ne!(one, zero, "the settle step changes the velocity (else this test proves nothing)");
    with_guest(71, "(i64.const 0)", || {
        assert_eq!(hooked_settle().velocity, zero, "the original did not run");
        assert_eq!(entries(), 1);
        assert!(!runtime::stats().disabled);
    });
    runtime::uninstall().unwrap();
}

#[test]
fn a_guest_that_calls_original_twice_runs_it_twice() {
    let (one, two) = (settled(1).velocity, settled(2).velocity);
    assert_ne!(two, one);
    with_guest(71, &format!("{CALL_ORIGINAL} {CALL_ORIGINAL} (i64.const 0)"), || {
        assert_eq!(hooked_settle().velocity, two);
        assert_eq!(entries(), 1);
    });
    runtime::uninstall().unwrap();
}

#[test]
fn a_trap_before_original_disables_the_mod_and_the_original_still_runs() {
    let one = settled(1).velocity;
    with_guest(71, "(unreachable)", || {
        assert_eq!(hooked_settle().velocity, one, "the original ran once");
        let stats = runtime::stats();
        assert!(stats.disabled && stats.trap.is_some(), "{stats:?}");
        // disabled: the next call does not enter the guest and runs the original
        assert_eq!(hooked_settle().velocity, one);
        assert_eq!(runtime::stats().entries, 1);
    });
    runtime::uninstall().unwrap();
}

#[test]
fn a_trap_after_original_keeps_the_original_result_and_does_not_run_it_again() {
    let one = settled(1).velocity;
    with_guest(71, &format!("{CALL_ORIGINAL} (unreachable)"), || {
        assert_eq!(hooked_settle().velocity, one, "the original ran exactly once");
        assert!(runtime::stats().disabled);
    });
    runtime::uninstall().unwrap();
}

#[test]
fn installing_twice_adds_no_second_dispatcher() {
    let one = settled(1).velocity;
    with_guest(71, &format!("{CALL_ORIGINAL} (i64.const 0)"), || {
        let again = hook_bridge::install(&runtime::stats().registered);
        assert_eq!(again.installed, [71], "already installed counts as installed: {again:?}");
        assert!(yoake_slice::bridge::is_installed(71));
        assert_eq!(hooked_settle().velocity, one);
        assert_eq!(entries(), 1, "the guest was entered once per call");
    });
    runtime::uninstall().unwrap();
}

#[test]
fn install_separates_installed_not_bridgeable_and_not_mine() {
    // 211: an entities hook that cannot be bridged (borrowed return type); 103: a game::item_use hook (another module); 0x8000_0000: synthetic
    let report = hook_bridge::install(&[(71, 0), (211, 0), (103, 0), (0x8000_0000, 0), (999_999, 0)]);
    assert_eq!(report.installed, [71]);
    assert_eq!(report.not_bridgeable.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [211]);
    assert_eq!(report.not_mine, [103, 0x8000_0000, 999_999]);
    assert!(report.failed.is_empty());
    use yoake_slice::bridge::Mode;
    assert_eq!(hook_bridge::mode(71), Some(Mode::Full));
    assert_eq!(hook_bridge::mode(215), Some(Mode::ObserveOnly));
    assert_eq!(hook_bridge::mode(211), None, "unbridged: borrowed return type");
    assert_eq!(hook_bridge::mode(1), Some(Mode::Full), "five scalar arguments: bridged n-ary");
    assert_eq!(hook_bridge::mode(103), None, "another module's id");
    assert!(hook_bridge::BRIDGED.windows(2).all(|w| w[0] < w[1]) && hook_bridge::OBSERVE_ONLY.windows(2).all(|w| w[0] < w[1]), "sorted for binary_search");
}

fn stone() -> ItemStack {
    ItemStack { item: Arc::from("minecraft:stone"), count: 1, damage: 0, extra: None }
}

#[test]
fn a_bool_result_is_replaced_by_the_guest_and_out_of_range_values_keep_the_original_result() {
    // hook 58: ArmorStand::name_with_tag(&mut self, &ItemStack) -> bool; with a stone the original returns false
    assert!(!stand().name_with_tag(&stone()));
    with_guest(58, "(i64.const 1)", || {
        assert!(stand().name_with_tag(&stone()), "the guest's 1 is true");
    });
    let before = yoake_slice::bridge::out_of_range_count();
    with_guest(58, "(i64.const 5)", || {
        assert!(!stand().name_with_tag(&stone()), "5 is not a bool: the original ran and its false is used");
    });
    with_guest(58, &format!("{CALL_ORIGINAL} (i64.const -1)"), || {
        assert!(!stand().name_with_tag(&stone()), "-1 is not a bool: the original's last result is used");
        assert_eq!(entries(), 1);
    });
    assert!(yoake_slice::bridge::out_of_range_count() >= before + 2, "both counted (other tests may add more)");
    with_guest(58, &format!("{CALL_ORIGINAL} (i64.const 0)"), || {
        assert!(!stand().name_with_tag(&stone()));
    });
    runtime::uninstall().unwrap();
}

// ---- Observe-only (call site 215: `ItemEntity::hurt(1)` inside `ItemEntity::environment_tick`; a burning item at
// health 5 loses one point per tick). The real original runs exactly once per call whatever the guest does.

/// One `environment_tick` of a burning item (health 5, fire 20): the item's health afterwards.
fn burn_once() -> i32 {
    let (world, facts, registry, items) = fixture();
    let mut item = item();
    let environment = item.environment_tick(&world, &facts, &registry, &items);
    assert!(environment.alive);
    i32::from(item.health)
}

#[test]
fn observe_only_runs_the_original_exactly_once_whatever_the_guest_does() {
    assert_eq!(burn_once(), 4, "without a guest: hurt once");
    let cases = [
        ("original not called: it runs after the guest", "(i64.const 0)".to_string(), false),
        ("original called twice: the second call does not hurt again", format!("{CALL_ORIGINAL} {CALL_ORIGINAL} (i64.const 0)"), false),
        ("a trap before original: the original still runs", "(unreachable)".to_string(), true),
        ("a trap after original: not run again", format!("{CALL_ORIGINAL} (unreachable)"), true),
        ("the guest's value is ignored", format!("{CALL_ORIGINAL} (i64.const 77)"), false),
    ];
    for (what, body, traps) in cases {
        with_guest(215, &body, || {
            assert_eq!(burn_once(), 4, "{what}");
            assert_eq!(entries(), 1, "{what}");
            assert_eq!(runtime::stats().disabled, traps, "{what}");
        });
    }
    runtime::uninstall().unwrap();
}

#[test]
fn two_mods_on_one_bridged_id_share_one_dispatcher_and_both_run_once() {
    let one = settled(1).velocity;
    runtime::uninstall().unwrap();
    // two mods on hook 71: the outer one (priority 10) calls original, which runs the inner one (20), which calls the
    // real original; each reports its own slot
    runtime::load(wat_guest_with(71, 10, 10, &format!("{CALL_ORIGINAL} (i64.const 0)")).as_bytes(), ModConfig::new(""), "outer").unwrap();
    runtime::load(wat_guest_with(71, 20, 20, &format!("{CALL_ORIGINAL} (i64.const 0)")).as_bytes(), ModConfig::new(""), "inner").unwrap();
    // the loader installs per mod: the second install of 71 reuses the first dispatcher
    for (_, _, stats) in runtime::all_stats() {
        assert_eq!(hook_bridge::install(&stats.registered).installed, [71]);
    }
    assert_eq!(hooked_settle().velocity, one, "the real original ran once");
    let all = runtime::take_reports().unwrap();
    assert_eq!((reports(&all, 10).len(), reports(&all, 20).len()), (1, 1), "each mod entered once: {all:?}");

    // the outer mod does not call original: the inner one and the engine do not run (a Full hook is cancelled)
    runtime::uninstall().unwrap();
    runtime::load(wat_guest_with(71, 10, 10, "(i64.const 0)").as_bytes(), ModConfig::new(""), "outer").unwrap();
    runtime::load(wat_guest_with(71, 20, 20, &format!("{CALL_ORIGINAL} (i64.const 0)")).as_bytes(), ModConfig::new(""), "inner").unwrap();
    assert_eq!(hooked_settle().velocity, settled_without_guest_zero(), "cancelled by the outer mod");
    let all = runtime::take_reports().unwrap();
    assert_eq!((reports(&all, 10).len(), reports(&all, 20).len()), (1, 0), "{all:?}");
    runtime::uninstall().unwrap();
}

/// The velocity of a stand that did not settle (computed without running the hooked function).
fn settled_without_guest_zero() -> Vec3 {
    stand().velocity
}

// ---- Read-only arguments (`&self` hooks through `dispatch_id_ref` and the generated `REF_FIELDS_` tables).

#[test]
fn a_read_only_hook_reads_fields_cannot_write_them_and_can_replace_the_result() {
    use horizon_client_lib::entities::Experience;
    // hook 40: Experience::death_reward(&self, keep: bool) -> u32 (Full: a u32 result)
    let xp = Experience { level: 3, progress: 0.25, total: 50 };
    let plain = xp.death_reward(false);
    assert_eq!(hook_bridge::mode(40), Some(yoake_slice::bridge::Mode::Full));
    // the guest reads field 2 (`total`) through the shared handle, tries to write it, and returns total * 2
    let body = "(call $report (i32.const 2) (i64.extend_i32_s (call $get (local.get $h) (i32.const 2) (i32.const 32))))
    (call $report (i32.const 3) (i64.extend_i32_s (call $set (local.get $h) (i32.const 2) (i64.const 0))))
    (i64.mul (i64.load (i32.const 32)) (i64.const 2))";
    let wat = wat_guest(40, body).replace(
        r#"(import "env" "report""#,
        r#"(import "env" "get" (func $get (param i64 i32 i32) (result i32)))
  (import "env" "set" (func $set (param i64 i32 i64) (result i32)))
  (import "env" "report""#,
    );
    runtime::uninstall().unwrap();
    runtime::install(wat.as_bytes(), ModConfig::new("")).unwrap();
    assert_eq!(hook_bridge::install(&runtime::stats().registered).installed, [40]);
    assert_eq!(xp.death_reward(false), 100, "the guest's value replaced the result (the original gives {plain})");
    let all = runtime::take_reports().unwrap();
    assert_eq!(reports(&all, 2), [0], "get worked");
    assert_eq!(reports(&all, 3), [7], "set was refused ReadOnly (7)");
    assert_eq!((xp.total, xp.level), (50, 3), "the argument is unchanged");
    assert!(!runtime::stats().disabled);
    runtime::uninstall().unwrap();
}

// ---- W9: a hook with five scalar arguments goes through the n-ary bridge; the guest reads and replaces them.
#[test]
fn a_five_argument_fn_is_bridged_n_ary_and_the_guest_reads_and_replaces_scalars() {
    use horizon_client_lib::entities::vertical_collision_velocity as velocity;
    let call = || velocity(-1.0, -0.5, 0.04, true, true);
    runtime::uninstall().unwrap();
    let plain = call();
    assert_ne!(plain, 0.0, "slime bounce (else this proves nothing)");
    let wat = r#"(module
  (import "env" "original" (func $orig (param i64 i32) (result i32)))
  (import "env" "arg_get" (func $aget (param i64 i32) (result i32)))
  (import "env" "arg_set" (func $aset (param i64 i64) (result i32)))
  (import "env" "hook_register" (func $register (param i32 i32) (result i32)))
  (import "env" "report" (func $report (param i32 i64)))
  (memory (export "memory") 1)
  (func (export "yoake_init") (drop (call $register (i32.const 1) (i32.const 0))))
  (func (export "yoake_hook") (param $id i32) (param $h i64) (result i64)
    ;; read argument 0 (current_y bits) and argument 4 (slime)
    (drop (call $aget (local.get $h) (i32.const 32)))
    (call $report (i32.const 2) (i64.load (i32.const 32)))
    (drop (call $aget (i64.add (local.get $h) (i64.const 4)) (i32.const 32)))
    (call $report (i32.const 5) (i64.load (i32.const 32)))
    ;; replace `slime` by false: no bounce
    (drop (call $aset (i64.add (local.get $h) (i64.const 4)) (i64.const 0)))
    (drop (call $orig (local.get $h) (i32.const 64)))
    (i64.load (i32.const 64))))"#;
    runtime::install(wat.as_bytes(), ModConfig::new("")).unwrap();
    let report = hook_bridge::install(&runtime::stats().registered);
    assert_eq!(report.installed, [1], "{report:?}");
    assert_eq!(call(), 0.0, "the guest replaced `slime`: the original saw false");
    let r = runtime::take_reports().unwrap();
    assert_eq!(reports(&r, 2), vec![(-1.0f64).to_bits() as i64], "argument 0 read as f64 bits");
    assert_eq!(reports(&r, 5), vec![1], "argument 4 read as bool");
    runtime::uninstall().unwrap();
}
