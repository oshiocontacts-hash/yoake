use std::collections::BTreeMap;
use std::sync::Arc;

use crate::writes::WriteTarget;

pub const MAX_HANDLES: usize = 4096;
const MAX_DIMENSION_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateHandle(u64);

impl StateHandle {
    pub fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateObject {
    Player { uuid: [u8; 16] },
    Entity { dimension: String, entity_id: i32 },
    BlockEntity { dimension: String, position: [i32; 3] },
    Chunk { dimension: String, position: [i32; 2] },
    Level { dimension: String },
    Server,
}

impl StateObject {
    fn validate(&self) -> Result<(), String> {
        let dimension = match self {
            Self::Entity { dimension, .. }
            | Self::BlockEntity { dimension, .. }
            | Self::Chunk { dimension, .. }
            | Self::Level { dimension } => dimension,
            Self::Player { .. } | Self::Server => return Ok(()),
        };
        if dimension.is_empty()
            || dimension.len() > MAX_DIMENSION_BYTES
            || dimension.chars().any(char::is_control)
        {
            return Err("invalid dimension identifier".into());
        }
        Ok(())
    }
}

struct Entry {
    instance: u64,
    world_generation: u64,
    object_generation: u64,
    object: StateObject,
}

#[derive(Default)]
pub struct HandleTable {
    next: u64,
    entries: BTreeMap<StateHandle, Entry>,
    identity: Arc<()>,
}

pub struct ResolvedHandle<'a> {
    object: &'a StateObject,
    target: WriteTarget,
}

impl ResolvedHandle<'_> {
    pub fn object(&self) -> &StateObject {
        self.object
    }

    pub fn object_generation(&self) -> u64 {
        self.target.object_generation
    }

    pub(crate) fn write_target(&self) -> WriteTarget {
        self.target.clone()
    }
}

impl HandleTable {
    pub fn issue(
        &mut self,
        instance: u64,
        world_generation: u64,
        object_generation: u64,
        object: StateObject,
    ) -> Result<StateHandle, String> {
        object.validate()?;
        if self.entries.len() >= MAX_HANDLES {
            return Err("too many state handles".into());
        }
        let next = self.next.checked_add(1).ok_or("state handles exhausted")?;
        let handle = StateHandle(next);
        self.entries.insert(
            handle,
            Entry {
                instance,
                world_generation,
                object_generation,
                object,
            },
        );
        self.next = next;
        Ok(handle)
    }

    pub fn resolve(
        &self,
        handle: StateHandle,
        instance: u64,
        world_generation: u64,
    ) -> Result<ResolvedHandle<'_>, String> {
        let entry = self
            .entries
            .get(&handle)
            .filter(|entry| {
                entry.instance == instance && entry.world_generation == world_generation
            })
            .ok_or("invalid or stale state handle")?;
        Ok(ResolvedHandle {
            object: &entry.object,
            target: WriteTarget {
                table: Arc::clone(&self.identity),
                instance,
                world_generation,
                object_generation: entry.object_generation,
                handle: handle.as_u64(),
            },
        })
    }

    pub fn invalidate(&mut self, handle: StateHandle, instance: u64) -> Result<(), String> {
        match self.entries.get(&handle) {
            Some(entry) if entry.instance == instance => {
                self.entries.remove(&handle);
                Ok(())
            }
            _ => Err("invalid or stale state handle".into()),
        }
    }

    pub fn invalidate_instance(&mut self, instance: u64) {
        self.entries.retain(|_, entry| entry.instance != instance);
    }

    pub fn invalidate_world(&mut self, world_generation: u64) {
        self.entries
            .retain(|_, entry| entry.world_generation != world_generation);
    }

    pub fn invalidate_object(
        &mut self,
        world_generation: u64,
        object_generation: u64,
        object: &StateObject,
    ) {
        self.entries.retain(|_, entry| {
            entry.world_generation != world_generation
                || entry.object_generation != object_generation
                || &entry.object != object
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_each_object_only_for_its_instance_and_world() {
        let objects = [
            StateObject::Player { uuid: [7; 16] },
            StateObject::Entity {
                dimension: "minecraft:overworld".into(),
                entity_id: 42,
            },
            StateObject::BlockEntity {
                dimension: "minecraft:overworld".into(),
                position: [-1, 64, 3],
            },
            StateObject::Chunk {
                dimension: "minecraft:the_nether".into(),
                position: [-1, 3],
            },
            StateObject::Level {
                dimension: "minecraft:the_end".into(),
            },
            StateObject::Server,
        ];
        let mut table = HandleTable::default();
        for (index, object) in objects.into_iter().enumerate() {
            let generation = index as u64 + 1;
            let handle = table.issue(11, 22, generation, object.clone()).unwrap();
            let resolved = table.resolve(handle, 11, 22).unwrap();
            assert_eq!(resolved.object(), &object);
            assert_eq!(resolved.object_generation(), generation);
            assert_eq!(
                resolved.write_target(),
                WriteTarget {
                    table: Arc::clone(&table.identity),
                    instance: 11,
                    world_generation: 22,
                    object_generation: generation,
                    handle: handle.as_u64(),
                }
            );
            assert!(table.resolve(handle, 12, 22).is_err());
            assert!(table.resolve(handle, 11, 23).is_err());
            assert!(table.invalidate(handle, 12).is_err());
        }
        assert!(table.resolve(StateHandle::from_u64(0), 11, 22).is_err());
    }

    #[test]
    fn invalidation_covers_objects_worlds_and_instances() {
        let mut table = HandleTable::default();
        let object = StateObject::Player { uuid: [1; 16] };
        let first = table.issue(1, 10, 4, object.clone()).unwrap();
        let other_instance = table.issue(2, 10, 4, object.clone()).unwrap();
        let newer_object = table.issue(1, 10, 5, object.clone()).unwrap();
        let other_world = table.issue(1, 11, 4, object.clone()).unwrap();

        table.invalidate_object(10, 4, &object);
        assert!(table.resolve(first, 1, 10).is_err());
        assert!(table.resolve(other_instance, 2, 10).is_err());
        assert!(table.resolve(newer_object, 1, 10).is_ok());
        assert!(table.resolve(other_world, 1, 11).is_ok());

        table.invalidate_world(10);
        assert!(table.resolve(newer_object, 1, 10).is_err());
        assert!(table.resolve(other_world, 1, 11).is_ok());
        table.invalidate_instance(1);
        assert!(table.resolve(other_world, 1, 11).is_err());

        let replacement = table.issue(1, 10, 4, object).unwrap();
        assert_ne!(replacement, first);
        table.invalidate(replacement, 1).unwrap();
        assert!(table.resolve(replacement, 1, 10).is_err());
    }

    #[test]
    fn invalid_dimensions_do_not_consume_handles() {
        let mut table = HandleTable::default();
        for dimension in [String::new(), "a".repeat(257), "bad\nname".into()] {
            assert!(table.issue(1, 1, 1, StateObject::Level { dimension }).is_err());
        }
        let handle = table.issue(1, 1, 1, StateObject::Server).unwrap();
        assert_eq!(handle.as_u64(), 1);
    }

    #[test]
    fn handle_count_and_counter_are_bounded() {
        let mut table = HandleTable::default();
        for _ in 0..MAX_HANDLES {
            table.issue(1, 1, 1, StateObject::Server).unwrap();
        }
        assert!(table.issue(1, 1, 1, StateObject::Server).is_err());
        table.invalidate_instance(1);
        assert!(table.issue(1, 1, 1, StateObject::Server).is_ok());
        table.next = u64::MAX;
        assert!(table.issue(1, 1, 1, StateObject::Server).is_err());
    }
}
