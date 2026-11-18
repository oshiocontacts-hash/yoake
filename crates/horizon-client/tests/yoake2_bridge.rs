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
    let unbridged = hook_bridge::install(&runtime::stats().registered);
    assert!(unbridged.is_empty(), "unbridged hook ids: {unbridged:?}");

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
