use horizon_client::{entities::ItemEntity, world::World};
use horizon_core::geometry::Vec3;

use super::Game;

impl Game<'_> {
    pub(super) fn native_chunk_loaded(&self, position: [f64; 3]) -> bool {
        destination_loaded(&self.world, position)
    }
}

fn destination_loaded(world: &World, position: [f64; 3]) -> bool {
    let chunk_x = (position[0].floor() as i32) >> 4;
    let chunk_z = (position[2].floor() as i32) >> 4;
    world.chunks.contains_key(&(chunk_x, chunk_z))
}

/// Only loaded destinations are safe: item saving and unloading use its current chunk.
pub(super) fn move_item(
    world: &World,
    items: &mut [ItemEntity],
    id: u32,
    position: [f64; 3],
    velocity: [f64; 3],
) -> Result<bool, String> {
    if !position
        .iter()
        .chain(&velocity)
        .all(|value| value.is_finite() && value.abs() < 3.0e7)
    {
        return Err("the position and the velocity must be finite and within the world".into());
    }
    let Some(item) = items.iter_mut().find(|item| item.id == id) else {
        return Ok(false);
    };
    if !destination_loaded(world, position) {
        return Err("the destination chunk is not loaded".into());
    }
    let destination = Vec3::new(position[0], position[1], position[2]);
    item.position = destination;
    item.previous = destination;
    item.velocity = Vec3::new(velocity[0], velocity[1], velocity[2]);
    item.on_ground = false;
    item.needs_sync = true;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use horizon_client::{chunk::Chunk, items::ItemStack};
    use horizon_world::registry::BlockStateId;

    use super::*;

    fn item() -> ItemEntity {
        ItemEntity {
            join: Default::default(),
            health: 5,
            fire_ticks: -1,
            id: 71,
            stack: ItemStack::new("minecraft:stone", 1),
            thrower: None,
            target: None,
            position: Vec3::new(8.0, 100.0, 8.0),
            previous: Vec3::new(8.0, 100.0, 8.0),
            velocity: Vec3::ZERO,
            on_ground: true,
            age: 37,
            pickup_delay: 0,
            bob_offset: 0.0,
            ticks: 9,
            needs_sync: false,
            stuck: Vec3::ZERO,
            no_gravity: false,
            vertical_collision: false,
            horizontal_collision: false,
        }
    }

    #[test]
    fn item_moves_across_loaded_chunks_with_teleport_history() {
        let mut world = World::new(BlockStateId(0), 0);
        for chunk in [(0, 0), (0, -3), (0, -2), (-1, 0)] {
            world
                .chunks
                .insert(chunk, Arc::new(Chunk::empty(chunk.0, chunk.1)));
        }
        let mut items = [item()];
        for position in [
            [8.202, 100.0, -33.102],
            [8.202, 100.0, -32.308],
            [8.202, 100.0, -31.9],
            [-0.01, 100.0, 0.0],
            [8.0, 100.0, 8.0],
        ] {
            assert_eq!(
                move_item(&world, &mut items, 71, position, [0.25, 0.0, 0.5]),
                Ok(true)
            );
            let moved = &items[0];
            let destination = Vec3::new(position[0], position[1], position[2]);
            assert_eq!(moved.position, destination);
            assert_eq!(moved.previous, destination);
            assert_eq!(moved.velocity, Vec3::new(0.25, 0.0, 0.5));
            assert!(!moved.on_ground);
            assert!(moved.needs_sync);
            assert_eq!(moved.health, 5);
            assert_eq!(moved.age, 37);
            assert_eq!(moved.ticks, 9);
            assert_eq!(moved.stack.count, 1);
        }
    }

    #[test]
    fn unknown_item_precedes_the_unloaded_destination_check() {
        let world = World::new(BlockStateId(0), 0);
        let mut items = [item()];
        assert_eq!(
            move_item(&world, &mut items, 999, [505.0, 100.0, 5.0], [0.0; 3]),
            Ok(false)
        );
        assert_eq!(items[0].position, Vec3::new(8.0, 100.0, 8.0));
    }

    #[test]
    fn item_move_to_unloaded_chunk_leaves_item_unchanged() {
        let mut world = World::new(BlockStateId(0), 0);
        world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let mut items = [item()];
        assert_eq!(
            move_item(
                &world,
                &mut items,
                71,
                [8.0, 100.0, -0.01],
                [0.25, 0.0, 0.5]
            ),
            Err("the destination chunk is not loaded".into())
        );
        let unchanged = &items[0];
        assert_eq!(unchanged.position, Vec3::new(8.0, 100.0, 8.0));
        assert_eq!(unchanged.previous, unchanged.position);
        assert_eq!(unchanged.velocity, Vec3::ZERO);
        assert!(unchanged.on_ground);
        assert!(!unchanged.needs_sync);
        assert_eq!(unchanged.stack.count, 1);
    }
}
