use std::collections::HashMap;
use std::sync::Arc;

use horizon_world::collision::CachedCollisionRegistry;
use horizon_world::nbt::Tag;
use horizon_world::registry::{BlockRegistry, BlockStateId};

use super::*;
use crate::blocks::{RenderShape, Sounds, StateFacts};
use crate::chunk::Chunk;

fn fixture() -> (World, BlockFacts, BlockRegistry, Tag) {
    let states = vec![
        Tag::String("minecraft:air".into()),
        Tag::String("minecraft:stone".into()),
    ];
    let registry = BlockRegistry::from_nbt(&Tag::compound([
        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ("states", Tag::List(states.clone())),
        (
            "defaults",
            Tag::compound([
                ("minecraft:air", Tag::Int(0)),
                ("minecraft:stone", Tag::Int(1)),
            ]),
        ),
    ]))
    .unwrap();
    let empty = Tag::compound([
        ("x", Tag::LongArray(vec![0])),
        ("y", Tag::LongArray(vec![0])),
        ("z", Tag::LongArray(vec![0])),
        ("occupied", Tag::ByteArray(vec![])),
        ("cube", Tag::Byte(0)),
        ("cube_axes", Tag::Byte(0)),
        ("discrete_min", Tag::IntArray(vec![0; 3])),
        ("discrete_max", Tag::IntArray(vec![0; 3])),
    ]);
    let coords = Tag::LongArray(vec![0, 1.0f64.to_bits() as i64]);
    let full = Tag::compound([
        ("x", coords.clone()),
        ("y", coords.clone()),
        ("z", coords),
        ("occupied", Tag::ByteArray(vec![1])),
        ("cube", Tag::Byte(1)),
        ("cube_axes", Tag::Byte(7)),
        ("discrete_min", Tag::IntArray(vec![0; 3])),
        ("discrete_max", Tag::IntArray(vec![1; 3])),
    ]);
    let collision = Tag::compound([
        ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ("states", Tag::List(states)),
        ("shapes", Tag::List(vec![empty, full])),
        ("cached_shapes", Tag::IntArray(vec![0, 1])),
    ]);
    let state = |name: &str, solid: bool| StateFacts {
        name: name.into(),
        emission: 0,
        dampening: 0,
        skylight_down: true,
        shape_occlusion: false,
        can_occlude: false,
        solid_render: solid,
        hardness: 0.0,
        requires_tool: false,
        replaceable: !solid,
        random_ticks: false,
        block_entity: false,
        render_shape: RenderShape::Model,
        full_collision: solid,
        friction: 0.6,
        explosion_resistance: 0.0,
        item: name.into(),
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
        legacy_solid: solid,
        sturdy_full: if solid { 2 } else { 0 },
        classes: Arc::from([]),
        outline: Arc::from([]),
        outline_edges: Arc::from([]),
        map_color: 0,
    };
    let facts = BlockFacts {
        states: vec![
            state("minecraft:air", false),
            state("minecraft:stone", true),
        ],
        air: BlockStateId(0),
        woods: HashMap::new(),
    };
    let mut world = World::new(BlockStateId(0), 0);
    world.chunks.insert((0, -3), Arc::new(Chunk::empty(0, -3)));
    for column_x in 7..=9 {
        for column_z in -35..=-33 {
            world.set(column_x, 99, column_z, BlockStateId(1)).unwrap();
        }
    }
    (world, facts, registry, collision)
}

#[test]
fn moved_item_on_a_stone_floor_survives_and_stays_on_the_surface() {
    let (world, facts, registry, collision_tag) = fixture();
    let collision = BlockCollisionRegistry::new(Arc::new(
        CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap(),
    ))
    .unwrap();
    let mut item = honey_slide_tests::falling_item(8.202, 100.0, -33.102, 0.0, 0.0, 0.0);
    for _ in 0..100 {
        let no_physics = item.inside_block(&world, &collision);
        assert!(!no_physics);
        let step = item.tick_move(
            &world,
            &facts,
            &collision,
            no_physics,
            (0.0, 0.0),
            &[],
            None,
        );
        assert!(item.tick_settle(&world, &facts, no_physics, step));
        assert_eq!(item.position, Vec3::new(8.202, 100.0, -33.102));
        assert!(item.on_ground);
    }
}

#[test]
fn moved_item_inside_a_stone_floor_is_pushed_out_and_survives() {
    let (world, facts, registry, collision_tag) = fixture();
    let collision = BlockCollisionRegistry::new(Arc::new(
        CachedCollisionRegistry::from_nbt(&collision_tag, &registry).unwrap(),
    ))
    .unwrap();
    let mut item = honey_slide_tests::falling_item(8.202, 99.8, -33.102, 0.0, 0.0, 0.0);
    assert!(item.inside_block(&world, &collision));
    for _ in 0..100 {
        let no_physics = item.inside_block(&world, &collision);
        if no_physics {
            item.move_towards_closest_space(&world, &facts, 0.2);
        }
        let step = item.tick_move(
            &world,
            &facts,
            &collision,
            no_physics,
            (0.0, 0.0),
            &[],
            None,
        );
        assert!(item.tick_settle(&world, &facts, no_physics, step));
    }
    assert_eq!(item.position.y, 100.0);
    assert!(!item.inside_block(&world, &collision));
}
