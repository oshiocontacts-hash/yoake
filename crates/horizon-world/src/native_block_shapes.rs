use std::sync::OnceLock;

use horizon_core::{geometry::VoxelShape, position::BlockPos};

use crate::{
    collision_context::CollisionContext,
    registry::{BlockRegistry, BlockStateId},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockShapeKind {
    Outline,
    Collision,
}

pub struct NativeBlockShapes {
    pub claims: fn(&str) -> bool,
    pub random_ticks: fn(&str) -> bool,
    pub shape: fn(
        &BlockRegistry,
        BlockStateId,
        BlockPos,
        &CollisionContext<'_>,
        BlockShapeKind,
    ) -> Option<VoxelShape>,
}

static SHAPES: OnceLock<NativeBlockShapes> = OnceLock::new();

pub fn install(shapes: NativeBlockShapes) -> Result<(), NativeBlockShapes> {
    SHAPES.set(shapes)
}

pub fn claims(name: &str) -> bool {
    SHAPES.get().is_some_and(|shapes| (shapes.claims)(name))
}

pub fn randomly_ticking<'name>(name: impl FnOnce() -> &'name str) -> bool {
    SHAPES.get().is_some_and(|shapes| (shapes.random_ticks)(name()))
}

pub fn shape(
    registry: &BlockRegistry,
    state: BlockStateId,
    position: BlockPos,
    context: &CollisionContext<'_>,
    kind: BlockShapeKind,
) -> Option<VoxelShape> {
    let shapes = SHAPES.get()?;
    (shapes.shape)(registry, state, position, context, kind)
}

pub fn shape_without_context(
    registry: &BlockRegistry,
    state: BlockStateId,
    position: [i32; 3],
) -> Option<VoxelShape> {
    let shapes = SHAPES.get()?;
    (shapes.shape)(
        registry,
        state,
        BlockPos::new(position[0], position[1], position[2]),
        &CollisionContext::empty(false),
        BlockShapeKind::Collision,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_provider_does_not_resolve_random_tick_names() {
        assert!(!randomly_ticking(|| panic!("no mod resolved a block name")));
        assert!(!claims("test:machine"));
    }
}
