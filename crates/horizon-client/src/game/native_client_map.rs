//! Read-only snapshots of the window's client world, including remote worlds.

use std::collections::{BTreeSet, HashMap};

use horizon_client::blocks::{BlockFacts, Fluid, MAP_COLORS};
use horizon_client::chunk::Chunk;
use horizon_client::scoreboard::{Team, invisible_to};
use horizon_client::world::World;
use horizon_world::registry::BlockStateId;

use super::Game;

const TILES_PER_FRAME: usize = 64;
const CHANGES_PER_TICK: usize = 4096;
const MAX_RADIUS: f64 = 64.0;
const AIR: u8 = 4;
const FLUID: u8 = 3;
const STURDY_TOP: u8 = 8;
const WALL: u8 = 0x80;

#[cfg(test)]
mod tests;

/// The same column selection as the built-in minimap. API @API:client_map_data@.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeMapLayer {
    Surface,
    Below(i32),
}

/// Unshaded minimap columns, indexed by `z * 16 + x`. API @API:client_map_data@.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeMapTile {
    pub color: [u8; 256],
    pub height: [i16; 256],
    pub depth: [u8; 256],
}

/// A known profile positioned in the window's client level. API @API:client_map_data@.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeClientPlayer {
    pub uuid: u128,
    pub name: String,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub dimension: String,
}

/// A visible, tracked entity; `kind` is a resource location. API @API:client_map_data@.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeClientEntity {
    pub id: u32,
    pub kind: String,
    pub pos: [f64; 3],
    pub yaw: f32,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Listeners {
    pub load: bool,
    pub unload: bool,
    pub changed: bool,
}

#[derive(Clone, Copy, Default)]
pub(super) struct Pose {
    pub yaw: f32,
    pub invisible: bool,
}

#[derive(Default)]
pub(super) struct ClientMapState {
    pub enabled: bool,
    pub listeners: Listeners,
    pub track_unloads: bool,
    tiles: Vec<(usize, usize)>,
    changed: BTreeSet<[i32; 2]>,
    poses: HashMap<i32, Pose>,
}

impl ClientMapState {
    pub fn new(enabled: bool, listeners: Listeners) -> Self {
        Self {
            enabled,
            listeners: if enabled {
                listeners
            } else {
                Listeners::default()
            },
            track_unloads: enabled && (listeners.unload || listeners.changed),
            ..Self::default()
        }
    }

    pub fn begin_frame(&mut self) {
        if self.enabled {
            self.tiles.clear();
        }
    }

    fn columns<T>(&mut self, caller: Option<usize>, read: impl FnOnce() -> Option<T>) -> Option<T> {
        if !self.enabled {
            return None;
        }
        let caller = caller?;
        let index = if let Some(index) = self.tiles.iter().position(|entry| entry.0 == caller) {
            index
        } else {
            self.tiles.push((caller, 0));
            self.tiles.len() - 1
        };
        let used = &mut self.tiles[index].1;
        if *used >= TILES_PER_FRAME {
            return None;
        }
        *used += 1;
        read()
    }

    pub fn change(&mut self, chunk: [i32; 2]) {
        if self.listeners.changed && self.changed.len() < CHANGES_PER_TICK {
            self.changed.insert(chunk);
        }
    }

    pub fn block_changed(&mut self, pos: [i32; 3], previous: BlockStateId, next: BlockStateId) {
        if self.listeners.changed && previous != next {
            self.change([pos[0] >> 4, pos[2] >> 4]);
        }
    }

    fn received(&mut self, chunk: [i32; 2], newly_loaded: bool) -> bool {
        self.change(chunk);
        newly_loaded && self.listeners.load
    }

    fn unloaded(&mut self, chunk: [i32; 2]) -> bool {
        if self.listeners.changed {
            self.changed.remove(&chunk);
        }
        self.listeners.unload
    }

    fn changes(&mut self) -> Option<Vec<[i32; 2]>> {
        self.listeners
            .changed
            .then(|| std::mem::take(&mut self.changed).into_iter().collect())
    }

    pub fn spawn(&mut self, id: i32, yaw: f32) {
        if self.enabled {
            self.poses.insert(
                id,
                Pose {
                    yaw,
                    invisible: false,
                },
            );
        }
    }

    pub fn rotate(&mut self, id: i32, rotation: Option<(f32, f32)>) {
        if !self.enabled {
            return;
        }
        if let Some((yaw, _)) = rotation {
            self.poses.entry(id).or_default().yaw = yaw;
        }
    }

    pub fn flags(&mut self, id: i32, flags: u8) {
        if self.enabled {
            self.poses.entry(id).or_default().invisible = flags & 0x20 != 0;
        }
    }

    pub fn remove(&mut self, id: i32) {
        if self.enabled {
            self.poses.remove(&id);
        }
    }

    pub fn pose(&self, id: i32) -> Pose {
        self.poses.get(&id).copied().unwrap_or_default()
    }

    fn clear_level(&mut self) {
        self.changed.clear();
        self.poses.clear();
    }

    fn snapshot<T>(&self, read: impl FnOnce() -> Vec<T>) -> Vec<T> {
        if !self.enabled {
            return Vec::new();
        }
        read()
    }
}

fn palette_entry(facts: &BlockFacts, id: u32) -> (u8, u8) {
    let Some(fact) = facts.states.get(id as usize) else {
        return (0, AIR);
    };
    let mut flags = match fact.fluid {
        Fluid::None => 0,
        Fluid::Water => 1,
        Fluid::Lava => 2,
    };
    if facts.is_air(BlockStateId(id)) {
        flags |= AIR;
    }
    if fact.sturdy_full >> 1 & 1 != 0 {
        flags |= STURDY_TOP;
    }
    (fact.map_color, flags)
}

fn scan_columns(
    chunk: &Chunk,
    air: u32,
    layer: NativeMapLayer,
    palette: impl Fn(u32) -> (u8, u8),
) -> NativeMapTile {
    let bottom = chunk.min_y();
    let surface = (0..chunk.sections.len())
        .rev()
        .find(|&section| {
            chunk
                .sections
                .get(section)
                .is_some_and(|section| !section.is_uniform(air))
        })
        .map_or(bottom, |section| bottom + section as i32 * 16 + 15);
    let top = match layer {
        NativeMapLayer::Surface => surface,
        NativeMapLayer::Below(head) => surface.min(head),
    };
    let mut tile = NativeMapTile {
        color: [0; 256],
        height: [bottom as i16; 256],
        depth: [0; 256],
    };
    for column_z in 0..16 {
        for column_x in 0..16 {
            let index = (column_z * 16 + column_x) as usize;
            if let NativeMapLayer::Below(head) = layer {
                let (color, flags) = palette(chunk.get(column_x, head, column_z));
                if flags & AIR == 0 && color != 0 && flags & FLUID == 0 {
                    tile.color[index] = color | WALL;
                    tile.height[index] = head as i16;
                    continue;
                }
            }
            let mut height = top;
            while height > bottom {
                let (mut color, flags) = palette(chunk.get(column_x, height, column_z));
                if color == 0 {
                    height -= 1;
                    continue;
                }
                let mut depth = 0u8;
                let fluid = flags & FLUID;
                if fluid != 0 {
                    let mut below = height - 1;
                    while below > bottom
                        && palette(chunk.get(column_x, below, column_z)).1 & FLUID != 0
                    {
                        depth = depth.saturating_add(1);
                        below -= 1;
                    }
                    depth = depth.saturating_add(1);
                    if flags & STURDY_TOP == 0 {
                        color = if fluid == 2 { 4 } else { 12 };
                    }
                }
                tile.color[index] = color;
                tile.height[index] = height as i16;
                tile.depth[index] = depth;
                break;
            }
        }
    }
    tile
}

fn loaded_columns(
    world: &World,
    facts: &BlockFacts,
    chunk: [i32; 2],
    layer: NativeMapLayer,
) -> Option<NativeMapTile> {
    let chunk = world.chunks.get(&(chunk[0], chunk[1]))?;
    Some(scan_columns(chunk, world.air.0, layer, |id| {
        palette_entry(facts, id)
    }))
}

fn map_colors() -> [u32; 64] {
    let mut colors = [0; 64];
    colors[..MAP_COLORS.len()].copy_from_slice(&MAP_COLORS);
    colors
}

pub(super) struct KnownEntity {
    pub entity: NativeClientEntity,
    pub profile: Option<(u128, String)>,
    pub visible: bool,
}

pub(super) fn visible_to(
    team: Option<&Team>,
    viewer_team: Option<&Team>,
    viewer_spectator: bool,
    spectator: bool,
    invisible: bool,
    glowing: bool,
) -> bool {
    (!spectator || viewer_spectator)
        && (!invisible_to(team, viewer_team, viewer_spectator, invisible) || glowing)
}

fn players(known: Vec<KnownEntity>, dimension: &str) -> Vec<NativeClientPlayer> {
    let mut players: Vec<_> = known
        .into_iter()
        .filter_map(|known| {
            if !known.visible {
                return None;
            }
            let (uuid, name) = known.profile?;
            Some(NativeClientPlayer {
                uuid,
                name,
                pos: known.entity.pos,
                yaw: known.entity.yaw,
                dimension: dimension.to_owned(),
            })
        })
        .collect();
    players.sort_by_key(|player| player.uuid);
    players.dedup_by_key(|player| player.uuid);
    players
}

fn radius_squared(center: [f64; 3], radius: f32) -> Option<f64> {
    if !radius.is_finite() || radius < 0.0 || !center.iter().all(|axis| axis.is_finite()) {
        return None;
    }
    Some(f64::from(radius).min(MAX_RADIUS).powi(2))
}

fn entities(known: Vec<KnownEntity>, center: [f64; 3], radius_squared: f64) -> Vec<NativeClientEntity> {
    let mut entities: Vec<_> = known
        .into_iter()
        .filter(|known| {
            known.visible
                && known
                    .entity
                    .pos
                    .iter()
                    .zip(center)
                    .map(|(axis, center)| (axis - center).powi(2))
                    .sum::<f64>()
                    <= radius_squared
        })
        .map(|known| known.entity)
        .collect();
    entities.sort_by_key(|entity| entity.id);
    entities
}

impl Game<'_> {
    pub(super) fn client_map_columns(
        &mut self,
        chunk: [i32; 2],
        layer: NativeMapLayer,
    ) -> Option<NativeMapTile> {
        if !self.native_client_map.enabled {
            return None;
        }
        self.client_map_dimension()?;
        let caller = horizon_client::crash::native_mod_call().map(|caller| caller.index);
        let world = &self.world;
        let resources = &self.res;
        self.native_client_map.columns(caller, || {
            loaded_columns(world, resources.facts(), chunk, layer)
        })
    }

    pub(super) fn client_map_colors(&self) -> [u32; 64] {
        map_colors()
    }

    pub(super) fn client_map_players(&self) -> Vec<NativeClientPlayer> {
        self.native_client_map.snapshot(|| {
            let Some(dimension) = self.client_map_dimension() else {
                return Vec::new();
            };
            players(self.client_map_known_entities(), dimension)
        })
    }

    pub(super) fn client_map_entities_in(
        &self,
        center: [f64; 3],
        radius: f32,
    ) -> Vec<NativeClientEntity> {
        self.native_client_map.snapshot(|| {
            let Some(radius_squared) = radius_squared(center, radius) else {
                return Vec::new();
            };
            if self.client_map_dimension().is_none() {
                return Vec::new();
            }
            entities(self.client_map_known_entities(), center, radius_squared)
        })
    }

    pub(super) fn client_map_chunk_received(&mut self, chunk: [i32; 2], newly_loaded: bool) {
        if !self.native_client_map.enabled {
            return;
        }
        if self.native_client_map.received(chunk, newly_loaded) {
            if let Some(dimension) = self.client_map_dimension().map(str::to_owned) {
                self.native_client_chunk_load(&dimension, chunk);
            }
        }
    }

    pub(super) fn client_map_chunk_unloaded(&mut self, chunk: [i32; 2]) {
        if self.native_client_map.unloaded(chunk) {
            if let Some(dimension) = self.client_map_dimension().map(str::to_owned) {
                self.native_client_chunk_unload(&dimension, chunk);
            }
        }
    }

    pub(super) fn client_map_tick(&mut self) {
        let Some(chunks) = self.native_client_map.changes() else {
            return;
        };
        if let Some(dimension) = self.client_map_dimension().map(str::to_owned) {
            self.native_client_chunks_changed(&dimension, &chunks);
        }
    }

    pub(super) fn client_map_clear_level(&mut self) {
        if !self.native_client_map.enabled {
            return;
        }
        if self.native_client_map.listeners.unload {
            if let Some(dimension) = self.client_map_dimension().map(str::to_owned) {
                let mut chunks: Vec<_> = self
                    .world
                    .chunks
                    .keys()
                    .map(|&(column_x, column_z)| [column_x, column_z])
                    .collect();
                chunks.sort_unstable();
                for chunk in chunks {
                    self.native_client_chunk_unload(&dimension, chunk);
                }
            }
        }
        self.native_client_map.clear_level();
    }
}
