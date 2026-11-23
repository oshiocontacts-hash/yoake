//! `PortalShape`: the frame search shared by vanilla and data-defined portals.
use super::{
    LevelAccess, Sim,
    portal_type::{PortalSize, PortalType},
    update::rel,
};
use crate::registry::BlockStateId;
use horizon_core::{geometry::Direction, position::BlockPos, random::RandomSource};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PortalShape {
    pub x_axis: bool,
    pub right: Direction,
    pub bottom_left: BlockPos,
    pub width: i32,
    pub height: i32,
    pub portal_blocks: i32,
    pub size: PortalSize,
}

impl PortalShape {
    pub const fn is_valid(&self) -> bool {
        self.size.valid(self.width, self.height)
    }
    pub const fn is_complete(&self) -> bool {
        self.is_valid() && self.portal_blocks == self.width * self.height
    }
}

fn step(pos: BlockPos, dir: Direction, n: i32) -> BlockPos {
    let [dx, dy, dz] = dir.normal();
    pos.offset(dx * n, dy * n, dz * n)
}

fn scan_shape(
    min_y: i32,
    pos: BlockPos,
    x_axis: bool,
    size: PortalSize,
    frame: impl Fn(BlockPos) -> bool,
    empty: impl Fn(BlockPos) -> bool,
    portal: impl Fn(BlockPos) -> bool,
) -> PortalShape {
    let right = if x_axis { Direction::West } else { Direction::South };
    let mut shape = PortalShape {
        x_axis,
        right,
        bottom_left: pos,
        width: 0,
        height: 0,
        portal_blocks: 0,
        size,
    };
    let edge_distance = |start: BlockPos, direction: Direction| {
        for width in 0..=size.max_width {
            let at = step(start, direction, width);
            if !empty(at) {
                return if frame(at) { width } else { 0 };
            }
            if !frame(rel(at, Direction::Down)) {
                break;
            }
        }
        0
    };
    let min_y = min_y.max(pos.y - size.max_height);
    let mut start = pos;
    while start.y > min_y && empty(rel(start, Direction::Down)) {
        start = rel(start, Direction::Down);
    }
    let left = right.opposite();
    let edge = edge_distance(start, left) - 1;
    if edge < 0 {
        return shape;
    }
    shape.bottom_left = step(start, left, edge);
    let width = edge_distance(shape.bottom_left, right);
    if !(size.min_width..=size.max_width).contains(&width) {
        return shape;
    }
    shape.width = width;
    let mut height = size.max_height;
    'rows: for h in 0..size.max_height {
        let row = step(shape.bottom_left, Direction::Up, h);
        if !frame(step(row, right, -1)) || !frame(step(row, right, width)) {
            height = h;
            break;
        }
        for i in 0..width {
            let at = step(row, right, i);
            if !empty(at) {
                height = h;
                break 'rows;
            }
            if portal(at) {
                shape.portal_blocks += 1;
            }
        }
    }
    let top = step(shape.bottom_left, Direction::Up, height);
    let has_top = (0..width).all(|i| frame(step(top, right, i)));
    shape.height = if (size.min_height..=size.max_height).contains(&height) && has_top {
        height
    } else {
        0
    };
    shape
}

impl<L: LevelAccess + ?Sized> Sim<'_, L> {
    fn portal_frame(&self, state: BlockStateId) -> bool {
        let b = self.b;
        b.tag("minecraft:nether_portal_frame")
            .is_some_and(|tag| b.in_set(state, tag))
    }

    fn portal_empty(&self, state: BlockStateId) -> bool {
        let b = self.b;
        b.is_air(state)
            || b.tag("minecraft:fire").is_some_and(|tag| b.in_set(state, tag))
            || b.is_block(state, b.named.nether_portal)
    }

    pub fn find_portal_shape(&self, pos: BlockPos, x_axis: bool) -> PortalShape {
        scan_shape(
            self.level.min_y(),
            pos,
            x_axis,
            PortalSize::default(),
            |at| self.portal_frame(self.state(at)),
            |at| self.portal_empty(self.state(at)),
            |at| self.b.is_block(self.state(at), self.b.named.nether_portal),
        )
    }

    pub fn find_custom_portal_shape(
        &self,
        pos: BlockPos,
        x_axis: bool,
        portal: &PortalType,
    ) -> PortalShape {
        scan_shape(
            self.level.min_y(),
            pos,
            x_axis,
            portal.size,
            |at| portal.is_frame(self.state(at), self.b),
            |at| {
                let state = self.state(at);
                self.b.is_air(state)
                    || self.b.tag("minecraft:fire").is_some_and(|tag| self.b.in_set(state, tag))
                    || portal.is_fill(state, self.b)
            },
            |at| portal.is_fill(self.state(at), self.b),
        )
    }

    fn find_custom_ignition_shape(
        &self,
        pos: BlockPos,
        x_axis: bool,
        portal: &PortalType,
        fluid: bool,
    ) -> PortalShape {
        scan_shape(
            self.level.min_y(),
            pos,
            x_axis,
            portal.size,
            |at| portal.is_frame(self.state(at), self.b),
            |at| {
                let state = self.state(at);
                fluid && at == pos
                    || self.b.is_air(state)
                    || self.b.tag("minecraft:fire").is_some_and(|tag| self.b.in_set(state, tag))
                    || portal.is_fill(state, self.b)
            },
            |at| portal.is_fill(self.state(at), self.b),
        )
    }

    pub fn find_empty_portal_shape(&self, pos: BlockPos, x_axis: bool) -> Option<PortalShape> {
        [x_axis, !x_axis]
            .into_iter()
            .map(|axis| self.find_portal_shape(pos, axis))
            .find(|shape| shape.is_valid() && shape.portal_blocks == 0)
    }

    fn fill_shape(&mut self, shape: &PortalShape, state: BlockStateId) {
        for h in 0..shape.height {
            for w in 0..shape.width {
                let at = step(step(shape.bottom_left, Direction::Up, h), shape.right, w);
                self.set_block(at, state, 18);
            }
        }
    }

    pub fn create_portal_blocks(&mut self, shape: &PortalShape) {
        let b = self.b;
        let base = b.default_of(b.named.nether_portal);
        let state = b
            .table
            .with_text(base, b.props.axis, if shape.x_axis { "x" } else { "z" })
            .unwrap_or(base);
        self.fill_shape(shape, state);
    }

    fn custom_shape(
        &self,
        pos: BlockPos,
        item: Option<&str>,
        fluid: Option<&str>,
        fire: bool,
        preferred_axis: bool,
    ) -> Option<(PortalType, PortalShape)> {
        self.level.custom_portals()?.iter().find_map(|portal| {
            if !ignition_matches(portal, item, fluid, fire) {
                return None;
            }
            [preferred_axis, !preferred_axis].into_iter().find_map(|axis| {
                let shape = self.find_custom_ignition_shape(pos, axis, portal, fluid.is_some());
                (shape.is_valid() && shape.portal_blocks == 0).then(|| (portal.clone(), shape))
            })
        })
    }

    pub fn ignite_custom_portal(
        &mut self,
        pos: BlockPos,
        item: Option<&str>,
        fluid: Option<&str>,
        fire: bool,
        preferred_axis: bool,
    ) -> Option<String> {
        let (portal, shape) = self.custom_shape(pos, item, fluid, fire, preferred_axis)?;
        let state = portal.fill_state(shape.x_axis, self.b)?;
        self.fill_shape(&shape, state);
        if let Some(sound) = portal.look.ambient_sound {
            self.emit(super::LevelEvent::Sound {
                pos,
                sound: sound.into(),
                volume: 1.0,
                pitch: 1.0,
            });
        }
        Some(portal.id)
    }

    pub fn fire_can_be_placed_at(&mut self, pos: BlockPos, forward: Direction) -> bool {
        if !self.b.is_air(self.state(pos)) {
            return false;
        }
        if self.can_survive(self.fire_state_for(pos), pos) {
            return true;
        }
        if matches!(self.level.dimension(), super::Dimension::Overworld | super::Dimension::Nether) {
            for face in Direction::ALL {
                if self.portal_frame(self.state(rel(pos, face))) {
                    let x_axis = match forward {
                        Direction::North | Direction::South => true,
                        Direction::West | Direction::East => false,
                        Direction::Up | Direction::Down => {
                            self.random().next_int_bound(2).unwrap_or(0) == 0
                        }
                    };
                    return self.find_empty_portal_shape(pos, x_axis).is_some();
                }
            }
        }
        if self
            .level
            .custom_portals()
            .is_none_or(|portals| portals.types.is_empty())
        {
            return false;
        }
        let x_axis = match forward {
            Direction::North | Direction::South => true,
            Direction::West | Direction::East => false,
            Direction::Up | Direction::Down => self.random().next_int_bound(2).unwrap_or(0) == 0,
        };
        self.custom_shape(pos, None, None, true, x_axis).is_some()
    }
}

fn ignition_matches(portal: &PortalType, item: Option<&str>, fluid: Option<&str>, fire: bool) -> bool {
    item.is_some_and(|item| portal.ignite.item.as_deref() == Some(item))
        || fluid.is_some_and(|fluid| portal.ignite.fluid.as_deref() == Some(fluid))
        || fire && portal.ignite.fire
}

#[cfg(test)]
mod tests;
