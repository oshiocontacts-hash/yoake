//! Tick-bounded authoritative block and player physics helpers for native mods.

use std::sync::atomic::Ordering;

use horizon_client::{
    arrows::clip_block_segment,
    collisions::SceneCollisions,
    mod_api::{NativeTickId, NativeWorldEpoch},
    world::World,
};
use horizon_core::geometry::{Aabb, Direction, Vec3};
use horizon_world::{
    block_collision_scan::{BlockCollisionScan, BlockCollisionWorld, scan_cells},
    collision_context::CollisionContext,
};

use super::{Dim, Game, native_entity_query::require_server_context};

const MAX_CALLS: u16 = 512;
const MAX_BOX_CELLS: usize = 65_536;
const MAX_COORDINATE: f64 = i32::MAX as f64 - 1024.0;

/// A block collider hit by a native clip query.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlockHit {
    pub pos: [i32; 3],
    pub location: [f64; 3],
    pub face: Direction,
}

/// The authoritative server's current movement state for a player.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NativeServerPlayerMotion {
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub fall_distance: f64,
}

#[derive(Clone, Copy)]
enum Operation {
    Clip,
    Box,
    PlayerMotion,
    SetPlayerVelocity,
    PlayerLatency,
}

impl Operation {
    fn index(self) -> usize {
        match self {
            Self::Clip => 0,
            Self::Box => 1,
            Self::PlayerMotion => 2,
            Self::SetPlayerVelocity => 3,
            Self::PlayerLatency => 4,
        }
    }
}

struct PhysicsUsage {
    mod_index: usize,
    calls: [u16; 5],
}

#[derive(Default)]
pub(super) struct PhysicsBudgets {
    tick: Option<(NativeWorldEpoch, NativeTickId)>,
    mods: Vec<PhysicsUsage>,
}

impl PhysicsBudgets {
    fn reserve(
        &mut self,
        tick: (NativeWorldEpoch, NativeTickId),
        mod_index: usize,
        operation: Operation,
    ) -> Result<(), String> {
        if self.tick != Some(tick) {
            self.mods.clear();
            self.tick = Some(tick);
        }
        let index = match self
            .mods
            .iter()
            .position(|usage| usage.mod_index == mod_index)
        {
            Some(index) => index,
            None => {
                self.mods.push(PhysicsUsage {
                    mod_index,
                    calls: [0; 5],
                });
                self.mods.len() - 1
            }
        };
        let calls = &mut self.mods[index].calls[operation.index()];
        if *calls >= MAX_CALLS {
            return Err(
                "native physics limit exceeded (512 calls per helper, mod and server tick)".into(),
            );
        }
        *calls += 1;
        Ok(())
    }
}

fn coordinate_vector(value: [f64; 3]) -> Result<Vec3, String> {
    if !value
        .into_iter()
        .all(|coordinate| coordinate.is_finite() && coordinate.abs() <= MAX_COORDINATE)
    {
        return Err(
            "native collision coordinates must be finite and within the supported range".into(),
        );
    }
    Ok(Vec3::new(value[0], value[1], value[2]))
}

fn velocity_vector(value: [f64; 3]) -> Result<Vec3, String> {
    if !value.into_iter().all(f64::is_finite) {
        return Err("native player velocity must be finite".into());
    }
    Ok(Vec3::new(value[0], value[1], value[2]))
}

fn replace_velocity(current: &mut Vec3, sync_velocity: bool, replacement: Vec3) -> Option<Vec3> {
    *current = replacement;
    (!sync_velocity).then_some(replacement)
}

fn direction(normal: [i32; 3]) -> Result<Direction, String> {
    Direction::ALL
        .into_iter()
        .find(|face| face.normal() == normal)
        .ok_or_else(|| "native block clip returned an invalid face".into())
}

fn box_collides<W: BlockCollisionWorld>(world: &mut W, bounds: Aabb) -> Result<bool, String>
where
    W::Error: std::fmt::Display,
{
    let cells = scan_cells(bounds).ok_or("native collision box has invalid bounds")?;
    if cells > MAX_BOX_CELLS {
        return Err("native collision box exceeds 65536 candidate cells".into());
    }
    let mut scan =
        BlockCollisionScan::new(world, CollisionContext::empty(false), bounds, false, cells)
            .map_err(|error| error.to_string())?;
    match scan.next() {
        Some(Ok(_)) => Ok(true),
        Some(Err(error)) => Err(error.to_string()),
        None => Ok(false),
    }
}

impl Game<'_> {
    fn native_physics_reserve(&mut self, operation: Operation, write: bool) -> Result<(), String> {
        require_server_context(
            self.options.execution_role,
            self.remote.is_some(),
            self.lan_acting.is_some(),
            self.lan.is_some(),
        )?;
        let caller = horizon_client::crash::native_mod_call()
            .ok_or("native physics requires an active native mod call")?;
        let tick = if write {
            self.native_lifecycle.mob_control_tick()
        } else {
            self.native_lifecycle.entity_query_tick()
        }
        .ok_or("native physics requires an active authoritative server tick")?;
        self.native_server_physics_budgets
            .reserve(tick, caller.index, operation)
    }

    fn native_physics_world(&self, dimension: &str) -> Result<&World, String> {
        let dim = match dimension {
            "minecraft:overworld" => Dim::Overworld,
            "minecraft:the_nether" => Dim::Nether,
            "minecraft:the_end" => Dim::End,
            _ => return Err("unknown native collision dimension".into()),
        };
        if dim == self.dim {
            Ok(&self.world)
        } else {
            self.levels
                .parked_world(dim)
                .ok_or_else(|| "native collision dimension is not loaded".into())
        }
    }

    pub(super) fn native_clip_impl(
        &mut self,
        dimension: &str,
        from: [f64; 3],
        to: [f64; 3],
    ) -> Result<Option<BlockHit>, String> {
        self.native_physics_reserve(Operation::Clip, false)?;
        let from = coordinate_vector(from)?;
        let to = coordinate_vector(to)?;
        let world = self.native_physics_world(dimension)?;
        clip_block_segment(world, self.res.facts(), self.collision, from, to)
            .map_err(|error| error.to_string())?
            .map(|(location, pos, normal)| {
                Ok(BlockHit {
                    pos: [pos.x, pos.y, pos.z],
                    location: location.components(),
                    face: direction(normal)?,
                })
            })
            .transpose()
    }

    pub(super) fn native_box_collides_impl(
        &mut self,
        dimension: &str,
        min: [f64; 3],
        max: [f64; 3],
    ) -> Result<bool, String> {
        self.native_physics_reserve(Operation::Box, false)?;
        let bounds = Aabb::new(coordinate_vector(min)?, coordinate_vector(max)?);
        let world = self.native_physics_world(dimension)?;
        let mut scene = SceneCollisions {
            world,
            binding: self.collision,
            subject: None,
        };
        box_collides(&mut scene, bounds)
    }

    pub(super) fn native_server_player_motion_impl(
        &mut self,
        uuid: u128,
    ) -> Result<NativeServerPlayerMotion, String> {
        self.native_physics_reserve(Operation::PlayerMotion, false)?;
        let host = self
            .lan
            .as_ref()
            .ok_or("native physics requires the server host")?;
        let guest = host
            .hub
            .guests
            .iter()
            .find(|guest| guest.uuid == uuid && guest.placed && host.play.contains_key(&guest.conn))
            .ok_or("native player is disconnected or not ready")?;
        let motion = guest
            .motion
            .as_deref()
            .ok_or("native player motion is unavailable")?;
        // CHECK: Verify the collision flag lifetime across teleports, passengers, and rejected moves.
        Ok(NativeServerPlayerMotion {
            position: guest.position,
            velocity: motion.velocity.components(),
            on_ground: guest.on_ground,
            horizontal_collision: guest.native_horizontal_collision,
            fall_distance: motion.fall_distance,
        })
    }

    pub(super) fn native_set_server_player_velocity_impl(
        &mut self,
        dimension: &str,
        uuid: u128,
        velocity: [f64; 3],
    ) -> Result<bool, String> {
        self.native_physics_reserve(Operation::SetPlayerVelocity, true)?;
        let velocity = velocity_vector(velocity)?;
        if !matches!(
            dimension,
            "minecraft:overworld" | "minecraft:the_nether" | "minecraft:the_end"
        ) {
            return Err("unknown native player dimension".into());
        }
        let mut host = self
            .lan
            .take()
            .ok_or("native physics requires the server host")?;
        let conn = host
            .hub
            .guests
            .iter()
            .find(|guest| {
                guest.uuid == uuid
                    && guest.dimension == dimension
                    && guest.placed
                    && host.play.contains_key(&guest.conn)
            })
            .map(|guest| guest.conn);
        let changed = conn.and_then(|conn| {
            let motion = host.hub.guest_mut(conn)?.motion.as_deref_mut()?;
            let sent =
                replace_velocity(&mut motion.velocity, motion.health.sync_velocity, velocity);
            Some((conn, sent))
        });
        if let Some((conn, sent)) = changed {
            self.send_guest_motion(&host, conn, sent);
        }
        self.lan = Some(host);
        Ok(changed.is_some())
    }

    pub(super) fn native_player_latency_ms_impl(&mut self, uuid: u128) -> Option<u32> {
        self.native_physics_reserve(Operation::PlayerLatency, false)
            .ok()?;
        let host = self.lan.as_ref()?;
        let guest = host.hub.guests.iter().find(|guest| {
            guest.uuid == uuid && guest.placed && host.play.contains_key(&guest.conn)
        })?;
        u32::try_from(guest.latency.load(Ordering::Relaxed)).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::{
        geometry::VoxelShape,
        position::{BlockPos, ChunkPos},
    };
    use horizon_world::{block_collision_scan::CollisionBlock, registry::BlockStateId};

    #[derive(Default)]
    struct TestWorld {
        loaded: bool,
        solid: bool,
        shape_queries: usize,
    }

    impl BlockCollisionWorld for TestWorld {
        type Error = std::io::Error;
        type Chunk = ();

        fn collision_chunk(&mut self, _position: ChunkPos) -> Result<Option<()>, Self::Error> {
            Ok(self.loaded.then_some(()))
        }

        fn collision_block(
            &mut self,
            _chunk: (),
            position: BlockPos,
        ) -> Result<CollisionBlock, Self::Error> {
            Ok(CollisionBlock {
                state: BlockStateId(0),
                large_collision_shape: false,
                moving_piston: false,
                empty_shape: !(self.solid && position == BlockPos::new(0, 0, 0)),
            })
        }

        fn is_suffocating(
            &mut self,
            _chunk: (),
            _block: CollisionBlock,
            _position: BlockPos,
        ) -> Result<bool, Self::Error> {
            Ok(false)
        }

        fn context_shape(
            &mut self,
            _block: CollisionBlock,
            _position: BlockPos,
            _context: &CollisionContext<'_>,
        ) -> Result<VoxelShape, Self::Error> {
            self.shape_queries += 1;
            Ok(VoxelShape::block())
        }
    }

    #[test]
    fn no_mod_does_not_allocate_or_query() {
        let budgets = PhysicsBudgets::default();
        let world = TestWorld::default();
        assert_eq!(budgets.mods.capacity(), 0);
        assert_eq!(world.shape_queries, 0);
    }

    #[test]
    fn mod_collision_query_uses_loaded_block_shapes() {
        let bounds = Aabb::new(Vec3::new(0.1, 0.1, 0.1), Vec3::new(0.9, 0.9, 0.9));
        let mut world = TestWorld {
            loaded: true,
            solid: true,
            ..Default::default()
        };
        assert_eq!(box_collides(&mut world, bounds), Ok(true));
        assert!(world.shape_queries > 0);
        world.loaded = false;
        assert_eq!(box_collides(&mut world, bounds), Ok(false));
    }

    #[test]
    fn budgets_are_per_mod_operation_and_tick() {
        let mut budgets = PhysicsBudgets::default();
        let tick = (NativeWorldEpoch::new(1), NativeTickId::new(1));
        for _ in 0..MAX_CALLS {
            assert!(budgets.reserve(tick, 1, Operation::Clip).is_ok());
        }
        assert!(budgets.reserve(tick, 1, Operation::Clip).is_err());
        for _ in 0..MAX_CALLS {
            assert!(budgets.reserve(tick, 1, Operation::Box).is_ok());
        }
        assert!(budgets.reserve(tick, 1, Operation::Box).is_err());
        assert!(budgets.reserve(tick, 2, Operation::Clip).is_ok());
        let next = (NativeWorldEpoch::new(1), NativeTickId::new(2));
        assert!(budgets.reserve(next, 1, Operation::Clip).is_ok());
        let next_world = (NativeWorldEpoch::new(2), NativeTickId::new(2));
        assert!(budgets.reserve(next_world, 1, Operation::Box).is_ok());
    }

    #[test]
    fn rejects_invalid_vectors_and_maps_faces() {
        assert!(coordinate_vector([f64::NAN, 0.0, 0.0]).is_err());
        assert!(coordinate_vector([MAX_COORDINATE + 1.0, 0.0, 0.0]).is_err());
        assert!(velocity_vector([0.0, f64::INFINITY, 0.0]).is_err());
        assert_eq!(direction([1, 0, 0]), Ok(Direction::East));
        assert!(direction([0, 0, 0]).is_err());
    }

    #[test]
    fn velocity_write_sends_changes_and_defers_pending_hurt() {
        let original = Vec3::new(0.0, 0.0, 0.0);
        let changed = Vec3::new(1.0, 2.0, 3.0);
        let mut velocity = original;
        assert_eq!(velocity, original);
        assert_eq!(
            replace_velocity(&mut velocity, false, changed),
            Some(changed)
        );
        assert_eq!(velocity, changed);
        assert_eq!(
            replace_velocity(&mut velocity, false, changed),
            Some(changed)
        );
        assert_eq!(replace_velocity(&mut velocity, true, original), None);
        assert_eq!(velocity, original);
    }
}
