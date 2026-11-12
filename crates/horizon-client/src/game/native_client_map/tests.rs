use std::sync::Arc;
use std::time::Duration;

use horizon_client::blocks::{RenderShape, Sounds, StateFacts};
use horizon_client::minimap::{Layer, Minimap, View};
use horizon_client::scoreboard::{CollisionRule, Visibility, literal};

use super::*;

fn facts() -> BlockFacts {
    let state = |name: &str, color, fluid, sturdy| StateFacts {
        name: name.into(),
        emission: 0,
        dampening: 0,
        skylight_down: true,
        shape_occlusion: false,
        can_occlude: false,
        solid_render: false,
        hardness: 0.0,
        requires_tool: false,
        replaceable: false,
        random_ticks: false,
        block_entity: false,
        render_shape: RenderShape::Model,
        full_collision: false,
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
        fluid,
        fluid_level: if fluid == Fluid::None { 0 } else { 8 },
        fluid_falling: false,
        waterlogged: fluid == Fluid::Water && color == 11,
        random_offset: false,
        outline_offset: false,
        legacy_solid: sturdy,
        sturdy_full: if sturdy { 2 } else { 0 },
        classes: Arc::from([]),
        outline: Arc::from([]),
        outline_edges: Arc::from([]),
        map_color: color,
    };
    BlockFacts {
        states: vec![
            state("minecraft:air", 0, Fluid::None, false),
            state("minecraft:grass_block", 1, Fluid::None, true),
            state("minecraft:stone", 11, Fluid::None, true),
            state("minecraft:water", 12, Fluid::Water, false),
            state("minecraft:glass", 0, Fluid::None, false),
            state("test:waterlogged", 11, Fluid::Water, false),
            state("minecraft:lava", 4, Fluid::Lava, false),
            state("test:sturdy_waterlogged", 11, Fluid::Water, true),
        ],
        air: BlockStateId(0),
        woods: HashMap::new(),
    }
}

fn surface_world() -> World {
    let mut chunk = Chunk::empty(0, 0);
    for column_z in 0..16 {
        for column_x in 0..16 {
            chunk.set(column_x, 32 + column_z * 2 + column_x, column_z, 1);
        }
    }
    chunk.set(0, 60, 0, 2);
    chunk.set(0, 61, 0, 1);
    chunk.set(1, 61, 0, 1);
    chunk.set(1, 62, 0, 4);
    chunk.set(2, 60, 0, 2);
    for height in 61..=63 {
        chunk.set(2, height, 0, 3);
    }
    chunk.set(3, 70, 0, 6);
    chunk.set(4, 80, 0, 5);
    chunk.set(5, 80, 0, 7);
    chunk.set(6, 80, 0, 999);
    chunk.set(15, 77, 15, 0);
    let mut world = World::new(BlockStateId(0), 0);
    world.chunks.insert((0, 0), Arc::new(chunk));
    world
}

fn cave_world() -> World {
    let mut chunk = Chunk::empty(0, 0);
    for height in 0..=20 {
        chunk.set(0, height, 0, 2);
        chunk.set(1, height, 0, 2);
    }
    chunk.set(0, 10, 0, 0);
    chunk.set(0, 11, 0, 0);
    chunk.set(2, 11, 0, 3);
    chunk.set(4, 5, 0, 2);
    chunk.set(4, 11, 0, 4);
    chunk.set(5, chunk.min_y(), 0, 2);
    let mut world = World::new(BlockStateId(0), 0);
    world.chunks.insert((0, 0), Arc::new(chunk));
    world
}

fn listeners() -> Listeners {
    Listeners {
        load: true,
        unload: true,
        changed: true,
    }
}

#[test]
fn no_mod_columns_do_not_scan_or_allocate() {
    let mut state = ClientMapState::new(false, listeners());
    assert_eq!(state.columns::<()>(Some(0), || panic!("scan")), None);
    state.begin_frame();
    assert_eq!(state.tiles.capacity(), 0);
}

#[test]
fn columns_preserve_raw_minimap_layout_and_fluids() {
    let world = surface_world();
    let tile = loaded_columns(&world, &facts(), [0, 0], NativeMapLayer::Surface).unwrap();
    let mut expected = NativeMapTile {
        color: [1; 256],
        height: [0; 256],
        depth: [0; 256],
    };
    for column_z in 0..16 {
        for column_x in 0..16 {
            expected.height[column_z * 16 + column_x] = (32 + column_z * 2 + column_x) as i16;
        }
    }
    for (index, color, height, depth) in [
        (0, 1, 61, 0),
        (1, 1, 61, 0),
        (2, 12, 63, 3),
        (3, 4, 70, 1),
        (4, 12, 80, 1),
        (5, 11, 80, 1),
        (255, 0, world.chunks[&(0, 0)].min_y() as i16, 0),
    ] {
        expected.color[index] = color;
        expected.height[index] = height;
        expected.depth[index] = depth;
    }
    assert_eq!(tile, expected);
}

#[test]
fn below_columns_preserve_cut_walls_and_bottom_boundary() {
    let world = cave_world();
    let tile = loaded_columns(&world, &facts(), [0, 0], NativeMapLayer::Below(11)).unwrap();
    let mut expected = NativeMapTile {
        color: [0; 256],
        height: [world.chunks[&(0, 0)].min_y() as i16; 256],
        depth: [0; 256],
    };
    for (index, color, height, depth) in [
        (0, 11, 9, 0),
        (1, 11 | WALL, 11, 0),
        (2, 12, 11, 1),
        (4, 11, 5, 0),
    ] {
        expected.color[index] = color;
        expected.height[index] = height;
        expected.depth[index] = depth;
    }
    assert_eq!(tile, expected);
}

#[test]
fn columns_return_none_for_unloaded_chunks_and_saturate_depth() {
    let mut world = World::new(BlockStateId(0), 0);
    assert!(loaded_columns(&world, &facts(), [0, 0], NativeMapLayer::Surface).is_none());
    let mut chunk = Chunk::empty(0, 0);
    for height in chunk.min_y() + 1..chunk.max_y() {
        chunk.set(0, height, 0, 3);
    }
    let top = chunk.max_y() - 1;
    world.chunks.insert((0, 0), Arc::new(chunk));
    let tile = loaded_columns(&world, &facts(), [0, 0], NativeMapLayer::Surface).unwrap();
    assert_eq!(
        (tile.color[0], tile.height[0], tile.depth[0]),
        (12, top as i16, 255),
    );
    world.chunks.remove(&(0, 0));
    assert!(loaded_columns(&world, &facts(), [0, 0], NativeMapLayer::Below(0)).is_none());
}

fn tile_pixel(tile: &NativeMapTile, column_x: usize, column_z: usize) -> [u8; 4] {
    let index = column_z * 16 + column_x;
    let color = tile.color[index];
    if color & !WALL == 0 {
        return [20, 20, 20, 200];
    }
    let parity = ((column_x + column_z) & 1) as f64;
    let brightness = if color & WALL != 0 {
        3
    } else if color == 12 {
        let difference = f64::from(tile.depth[index]) * 0.1 + parity * 0.2;
        if difference < 0.5 {
            2
        } else if difference > 0.9 {
            0
        } else {
            1
        }
    } else {
        let north = if column_z > 0 && tile.color[index - 16] & !WALL != 0 {
            tile.height[index - 16]
        } else {
            tile.height[index]
        };
        let difference = f64::from(tile.height[index] - north) * 4.0 / 5.0 + (parity - 0.5) * 0.4;
        if difference > 0.6 {
            2
        } else if difference < -0.6 {
            0
        } else {
            1
        }
    };
    let rgb = map_colors()[usize::from(color & !WALL)];
    let modifier: u32 = [180, 220, 255, 135][brightness];
    let channel = |shift: u32| ((rgb >> shift & 255) * modifier / 255) as u8;
    [channel(16), channel(8), channel(0), 255]
}

#[test]
fn columns_reproduce_builtin_scan_tile_through_public_minimap() {
    for (world, native_layer, render_layer) in [
        (surface_world(), NativeMapLayer::Surface, Layer::Surface),
        (cave_world(), NativeMapLayer::Below(11), Layer::Below(11)),
    ] {
        let facts = facts();
        let tile = loaded_columns(&world, &facts, [0, 0], native_layer).unwrap();
        let mut minimap = Minimap::default();
        assert_eq!(
            minimap.update(
                &world,
                &facts,
                (0, 0),
                0,
                render_layer,
                Duration::from_secs(1),
            ),
            1,
        );
        assert!(minimap.compose(View {
            x: 8.0,
            z: 8.0,
            yaw: 0.0,
            rotate: false,
            blocks_per_pixel: 1.0,
            round: false,
        }));
        for column_z in 0..16 {
            for column_x in 0..16 {
                let offset = ((56 + column_z) * 128 + 56 + column_x) * 4;
                assert_eq!(
                    &minimap.image[offset..offset + 4],
                    &tile_pixel(&tile, column_x, column_z),
                    "column {column_x},{column_z} in {native_layer:?}",
                );
            }
        }
    }
}

#[test]
fn palette_is_the_builtin_rgb_table_with_reserved_zero_entries() {
    let colors = map_colors();
    assert_eq!(&colors[..MAP_COLORS.len()], &MAP_COLORS);
    assert!(colors[MAP_COLORS.len()..].iter().all(|color| *color == 0));
}

#[test]
fn tile_budget_is_per_mod_counts_misses_and_resets_only_at_frame_start() {
    let mut state = ClientMapState::new(true, Listeners::default());
    assert!(
        state
            .columns::<()>(None, || panic!("unattributed scan"))
            .is_none()
    );
    assert!(state.tiles.is_empty());
    for _ in 0..TILES_PER_FRAME {
        assert_eq!(state.columns(Some(7), || Some(42)), Some(42));
    }
    assert!(
        state
            .columns::<()>(Some(7), || panic!("over budget"))
            .is_none()
    );
    state.clear_level();
    assert!(
        state
            .columns::<()>(Some(7), || panic!("level reset bypass"))
            .is_none()
    );
    for _ in 0..TILES_PER_FRAME {
        assert!(state.columns::<()>(Some(8), || None).is_none());
    }
    assert!(
        state
            .columns::<()>(Some(8), || panic!("misses bypass"))
            .is_none()
    );
    state.begin_frame();
    assert_eq!(state.columns(Some(7), || Some(42)), Some(42));
}

#[derive(Debug, PartialEq, Eq)]
enum Event {
    Load(String, [i32; 2]),
    Unload(String, [i32; 2]),
    Changed(String, Vec<[i32; 2]>),
}

struct FakeRemoteClient {
    dimension: String,
    world: World,
    map: ClientMapState,
    events: Vec<Event>,
}

impl FakeRemoteClient {
    fn new(enabled: bool, listeners: Listeners) -> Self {
        Self {
            dimension: "minecraft:overworld".into(),
            world: World::new(BlockStateId(0), 0),
            map: ClientMapState::new(enabled, listeners),
            events: Vec::new(),
        }
    }

    fn receive(&mut self, chunk: [i32; 2]) {
        let newly_loaded = self
            .world
            .chunks
            .insert((chunk[0], chunk[1]), Arc::new(Chunk::empty(chunk[0], chunk[1])))
            .is_none();
        if self.map.enabled && self.map.received(chunk, newly_loaded) {
            self.events.push(Event::Load(self.dimension.clone(), chunk));
        }
    }

    fn block_update(&mut self, pos: [i32; 3], next: u32) {
        let Some(chunk) = self.world.chunks.get(&(pos[0] >> 4, pos[2] >> 4)) else {
            return;
        };
        let previous = BlockStateId(chunk.get(pos[0] & 15, pos[1], pos[2] & 15));
        let next = BlockStateId(next);
        if self.world.set(pos[0], pos[1], pos[2], next).is_ok() {
            self.map.block_changed(pos, previous, next);
        }
    }

    fn section_update(&mut self, updates: &[([i32; 3], u32)]) {
        for &(pos, next) in updates {
            self.block_update(pos, next);
        }
    }

    fn unload(&mut self, chunk: [i32; 2]) {
        if self.world.chunks.remove(&(chunk[0], chunk[1])).is_none() {
            return;
        }
        if self.map.track_unloads && self.map.unloaded(chunk) {
            self.events
                .push(Event::Unload(self.dimension.clone(), chunk));
        }
    }

    fn tick(&mut self) {
        if let Some(chunks) = self.map.changes() {
            self.events
                .push(Event::Changed(self.dimension.clone(), chunks));
        }
    }
}

#[test]
fn no_mod_chunk_events_do_not_track_or_dispatch() {
    let mut client = FakeRemoteClient::new(false, listeners());
    client.receive([0, 0]);
    client.block_update([1, 5, 1], 1);
    client.section_update(&[([2, 5, 1], 2)]);
    client.tick();
    client.unload([0, 0]);
    assert!(!client.map.track_unloads);
    assert!(client.map.changed.is_empty());
    assert!(client.events.is_empty());
    assert_eq!(client.events.capacity(), 0);
}

#[test]
fn unlistened_changes_do_not_track_chunks_even_with_a_mod() {
    let mut state = ClientMapState::new(
        true,
        Listeners {
            load: true,
            unload: true,
            changed: false,
        },
    );
    assert!(state.received([1, 2], true));
    state.block_changed([16, 10, 32], BlockStateId(0), BlockStateId(1));
    state.change([2, 3]);
    assert!(state.changed.is_empty());
    assert_eq!(state.changes(), None);
    assert!(state.unloaded([1, 2]));
    let mut state = ClientMapState::new(true, Listeners::default());
    assert!(!state.track_unloads);
    assert!(!state.received([1, 2], true));
    assert!(!state.unloaded([1, 2]));
    assert_eq!(state.changes(), None);
}

#[test]
fn remote_client_chunks_load_unload_and_batch_changed_columns_once_per_tick() {
    let mut client = FakeRemoteClient::new(true, listeners());
    let dimension = client.dimension.clone();
    client.receive([1, -1]);
    client.receive([-1, 2]);
    client.receive([1, -1]);
    assert_eq!(
        client.events,
        vec![
            Event::Load(dimension.clone(), [1, -1]),
            Event::Load(dimension.clone(), [-1, 2]),
        ],
    );
    assert!(
        loaded_columns(&client.world, &facts(), [1, -1], NativeMapLayer::Surface).is_some()
    );
    client.tick();
    assert_eq!(
        client.events.pop(),
        Some(Event::Changed(dimension.clone(), vec![[-1, 2], [1, -1]])),
    );
    client.events.clear();
    client.block_update([-16, 5, 32], 1);
    client.section_update(&[([16, 5, -16], 2), ([-15, 6, 33], 2), ([-14, 7, 34], 1)]);
    client.block_update([-16, 5, 32], 1);
    client.block_update([5000, 5, 5000], 1);
    assert!(client.events.is_empty());
    client.tick();
    client.tick();
    assert_eq!(
        client.events,
        vec![
            Event::Changed(dimension.clone(), vec![[-1, 2], [1, -1]]),
            Event::Changed(dimension.clone(), Vec::new()),
        ],
    );
    client.events.clear();
    client.block_update([-16, 5, 32], 1);
    client.tick();
    assert_eq!(
        client.events.pop(),
        Some(Event::Changed(dimension.clone(), Vec::new())),
    );
    client.receive([1, -1]);
    client.block_update([-16, 5, 32], 2);
    client.unload([1, -1]);
    client.unload([-1, 2]);
    client.unload([-1, 2]);
    client.tick();
    assert_eq!(
        client.events,
        vec![
            Event::Unload(dimension.clone(), [1, -1]),
            Event::Unload(dimension.clone(), [-1, 2]),
            Event::Changed(dimension, Vec::new()),
        ],
    );
    assert!(
        loaded_columns(&client.world, &facts(), [1, -1], NativeMapLayer::Surface).is_none()
    );
}

#[test]
fn changes_are_bounded_and_level_resets_discard_old_pending_chunks() {
    let mut client = FakeRemoteClient::new(true, listeners());
    for column_x in 0..CHANGES_PER_TICK as i32 {
        client.map.change([column_x, 0]);
        client.map.change([column_x, 0]);
    }
    client.map.change([-1, 0]);
    let changed = client.map.changes().unwrap();
    assert_eq!(changed.len(), CHANGES_PER_TICK);
    assert_eq!(changed.first(), Some(&[0, 0]));
    assert_eq!(changed.last(), Some(&[CHANGES_PER_TICK as i32 - 1, 0]));
    client.map.change([-1, 0]);
    assert_eq!(client.map.changes(), Some(vec![[-1, 0]]));
    client.receive([0, 0]);
    client.map.clear_level();
    client.world.chunks.clear();
    client.dimension = "minecraft:the_nether".into();
    client.events.clear();
    client.receive([2, 3]);
    client.tick();
    assert_eq!(
        client.events,
        vec![
            Event::Load(client.dimension.clone(), [2, 3]),
            Event::Changed(client.dimension.clone(), vec![[2, 3]]),
        ],
    );
}

fn team(name: &str, see_friendly_invisibles: bool) -> Team {
    Team {
        name: name.into(),
        display_name: literal(name),
        color: None,
        allow_friendly_fire: true,
        see_friendly_invisibles,
        prefix: literal(""),
        suffix: literal(""),
        nametag_visibility: Visibility::Always,
        death_message_visibility: Visibility::Always,
        collision_rule: CollisionRule::Always,
        players: Default::default(),
    }
}

fn known(
    id: u32,
    kind: &str,
    pos: [f64; 3],
    profile: Option<(u128, &str)>,
    visible: bool,
) -> KnownEntity {
    KnownEntity {
        entity: NativeClientEntity {
            id,
            kind: kind.into(),
            pos,
            yaw: 0.75,
        },
        profile: profile.map(|(uuid, name)| (uuid, name.into())),
        visible,
    }
}

#[test]
fn no_mod_entity_snapshots_do_not_enumerate_or_track_poses() {
    let mut state = ClientMapState::new(false, listeners());
    assert!(state.snapshot::<KnownEntity>(|| panic!("enumerate")).is_empty());
    state.spawn(42, 0.5);
    state.rotate(42, Some((1.0, 0.0)));
    state.flags(42, 0x20);
    state.remove(42);
    assert_eq!(state.poses.capacity(), 0);
    assert_eq!(state.pose(42).yaw, 0.0);
    assert!(!state.pose(42).invisible);
}

#[test]
fn remote_player_and_mob_snapshots_use_known_profiles_positions_and_visibility() {
    let tracked = || {
        vec![
            known(
                41,
                "minecraft:player",
                [8.0, 65.0, 4.0],
                Some((123, "Remote")),
                true,
            ),
            known(42, "minecraft:zombie", [9.0, 65.0, 4.0], None, true),
            known(
                43,
                "minecraft:player",
                [10.0, 65.0, 4.0],
                Some((456, "Hidden")),
                visible_to(None, None, false, false, true, false),
            ),
            known(44, "minecraft:player", [11.0, 65.0, 4.0], None, true),
        ]
    };
    assert_eq!(
        players(tracked(), "minecraft:overworld"),
        vec![NativeClientPlayer {
            uuid: 123,
            name: "Remote".into(),
            pos: [8.0, 65.0, 4.0],
            yaw: 0.75,
            dimension: "minecraft:overworld".into(),
        }],
    );
    let visible = entities(
        tracked(),
        [8.0, 65.0, 4.0],
        radius_squared([8.0, 65.0, 4.0], 2.0).unwrap(),
    );
    assert_eq!(
        visible.iter().map(|entity| entity.id).collect::<Vec<_>>(),
        vec![41, 42],
    );
    assert_eq!(visible[1].kind, "minecraft:zombie");
    assert_eq!(visible[1].pos, [9.0, 65.0, 4.0]);
    assert_eq!(visible[1].yaw, 0.75);
}

#[test]
fn entity_radius_is_capped_in_three_dimensions_and_invalid_queries_are_empty() {
    let center = [0.0; 3];
    let tracked = || {
        vec![
            known(1, "minecraft:zombie", center, None, true),
            known(2, "minecraft:zombie", [64.0, 0.0, 0.0], None, true),
            known(3, "minecraft:zombie", [64.001, 0.0, 0.0], None, true),
            known(4, "minecraft:zombie", [0.0, 65.0, 0.0], None, true),
            known(5, "minecraft:zombie", [45.0, 45.0, 0.0], None, true),
            known(6, "minecraft:zombie", [46.0, 46.0, 0.0], None, true),
            known(
                7,
                "minecraft:player",
                [5.0, 0.0, 0.0],
                Some((7, "Hidden")),
                false,
            ),
        ]
    };
    for radius in [64.0, 1000.0, f32::MAX] {
        let result = entities(tracked(), center, radius_squared(center, radius).unwrap());
        assert_eq!(
            result.iter().map(|entity| entity.id).collect::<Vec<_>>(),
            vec![1, 2, 5],
        );
    }
    let result = entities(tracked(), center, radius_squared(center, 0.0).unwrap());
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, 1);
    for radius in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(radius_squared(center, radius), None);
    }
    for invalid_center in [
        [f64::NAN, 0.0, 0.0],
        [0.0, f64::INFINITY, 0.0],
        [0.0, 0.0, f64::NEG_INFINITY],
    ] {
        assert_eq!(radius_squared(invalid_center, 64.0), None);
    }
}

#[test]
fn visibility_matches_spectators_team_invisibles_and_glowing_outlines() {
    let friendly = team("friendly", true);
    let other = team("other", true);
    let hidden = team("friendly", false);
    assert!(!visible_to(None, None, false, false, true, false));
    assert!(visible_to(None, None, false, false, false, false));
    assert!(visible_to(
        Some(&friendly),
        Some(&friendly),
        false,
        false,
        true,
        false,
    ));
    assert!(!visible_to(
        Some(&friendly),
        Some(&other),
        false,
        false,
        true,
        false,
    ));
    assert!(!visible_to(
        Some(&hidden),
        Some(&friendly),
        false,
        false,
        true,
        false,
    ));
    assert!(visible_to(None, None, true, false, true, false));
    assert!(!visible_to(None, None, false, true, false, false));
    assert!(!visible_to(None, None, false, true, true, true));
    assert!(visible_to(None, None, true, true, true, false));
    assert!(visible_to(None, None, false, false, true, true));
}

#[test]
fn tracked_entity_pose_handles_rotation_flags_reuse_removal_and_level_reset() {
    let mut state = ClientMapState::new(true, Listeners::default());
    state.spawn(42, 0.5);
    state.flags(42, 0x20);
    state.rotate(42, None);
    assert_eq!(state.pose(42).yaw, 0.5);
    assert!(state.pose(42).invisible);
    state.rotate(42, Some((1.25, -0.5)));
    state.flags(42, 0x01);
    assert_eq!(state.pose(42).yaw, 1.25);
    assert!(!state.pose(42).invisible);
    state.flags(42, 0x20);
    state.spawn(42, 2.0);
    assert_eq!(state.pose(42).yaw, 2.0);
    assert!(!state.pose(42).invisible);
    state.remove(42);
    assert!(state.poses.is_empty());
    state.spawn(43, 3.0);
    state.clear_level();
    assert!(state.poses.is_empty());
}
