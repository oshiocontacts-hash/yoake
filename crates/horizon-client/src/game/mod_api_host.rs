use super::{BlockStateId, Game};
use crate::entry::wasm_mods;
use horizon_client::mod_api::BlockPos;
use horizon_world::{
    attachments::{ATTACHMENTS_KEY, attachment_compound},
    level::Ticks,
};

mod budget;

fn world_attachments_read(
    world: &horizon_client::world::World,
    holder: horizon_client::mod_api::Holder,
) -> Result<Option<&horizon_client::mod_api::Attachments>, horizon_client::mod_api::AttachmentError>
{
    use horizon_client::mod_api::{AttachmentError, Holder};
    match holder {
        Holder::Level => Ok(Some(&world.attachments)),
        Holder::Chunk([x, z]) => {
            let chunk = world
                .chunks
                .get(&(x, z))
                .ok_or(AttachmentError::MissingHolder)?;
            if let Some(saved) = &chunk.saved {
                attachment_compound(saved.root.get(&ATTACHMENTS_KEY.into()))?;
                Ok(Some(&saved.attachments))
            } else {
                Ok(None)
            }
        }
        Holder::BlockEntity([x, y, z]) => {
            let pos = (x, y, z);
            let chunk = world
                .chunks
                .get(&(x >> 4, z >> 4))
                .ok_or(AttachmentError::MissingHolder)?;
            attachment_compound(
                chunk
                    .saved
                    .as_ref()
                    .and_then(|saved| {
                        saved
                            .block_entity_sources
                            .iter()
                            .find(|(candidate, _)| *candidate == pos)
                    })
                    .and_then(|(_, tag)| tag.get(ATTACHMENTS_KEY)),
            )?;
            world
                .block_entities
                .attachments_read(pos)
                .ok_or(AttachmentError::MissingHolder)
        }
        _ => Err(AttachmentError::MissingHolder),
    }
}

fn world_attachments<'a>(
    world: &'a mut horizon_client::world::World,
    dimension: &str,
    holder: horizon_client::mod_api::Holder,
) -> Result<&'a mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
{
    use horizon_client::mod_api::{AttachmentError, Holder};
    if holder == Holder::Level {
        wasm_mods::prepare_attachments(
            holder,
            &mut world.attachments,
            format!("level:{dimension}"),
        )?;
        return Ok(&mut world.attachments);
    }
    if let Holder::BlockEntity([x, y, z]) = holder {
        let pos = (x, y, z);
        let chunk = world
            .chunks
            .get(&(x >> 4, z >> 4))
            .ok_or(AttachmentError::MissingHolder)?;
        attachment_compound(
            chunk
                .saved
                .as_ref()
                .and_then(|e| e.block_entity_sources.iter().find(|(p, _)| *p == pos))
                .and_then(|(_, tag)| tag.get(ATTACHMENTS_KEY)),
        )?;
        let map = world
            .block_entities
            .attachments_mut(pos)
            .ok_or(AttachmentError::MissingHolder)?;
        world.modified.insert((x >> 4, z >> 4));
        wasm_mods::prepare_attachments(
            holder,
            map,
            format!("block-entity:{dimension}:{x}:{y}:{z}"),
        )?;
        return Ok(map);
    }
    let Holder::Chunk([x, z]) = holder else {
        return Err(AttachmentError::MissingHolder);
    };
    let chunk = world
        .chunks
        .get_mut(&(x, z))
        .ok_or(AttachmentError::MissingHolder)?;
    let chunk = std::sync::Arc::make_mut(chunk);
    let extras = std::sync::Arc::make_mut(chunk.saved.get_or_insert_with(Default::default));
    attachment_compound(extras.root.get(&ATTACHMENTS_KEY.into()))?;
    world.modified.insert((x, z));
    wasm_mods::prepare_attachments(
        holder,
        &mut extras.attachments,
        format!("chunk:{dimension}:{x}:{z}"),
    )?;
    Ok(&mut extras.attachments)
}

impl Game<'_> {
    /// [`Self::reconcile_attachment_holder`] for the hook points, which hold no macros: a failure is logged.
    pub(super) fn reconcile_attachment_holder_or_warn(&mut self, holder: horizon_client::mod_api::Holder) {
        if let Err(error) = self.reconcile_attachment_holder(holder) {
            log::warn!("attachment reconciliation failed for {holder:?}: {error:?}");
        }
    }

    pub(super) fn reconcile_attachment_holder(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<(), horizon_client::mod_api::AttachmentError> {
        use horizon_client::mod_api::{AttachmentError, Holder};
        match holder {
            Holder::Player(uuid) => {
                let map = if self.lan_acting_uuid == Some(uuid) {
                    Some(&mut self.player_attachments)
                } else {
                    self.guest_attachments.get_mut(&uuid)
                };
                if let Some(map) = map {
                    wasm_mods::prepare_attachments(holder, map, format!("player:{uuid}"))?;
                }
            }
            Holder::Entity(id) => {
                if let Some(uuid) = self.persist.uuids.get(&id)
                    && let Some(map) = self.entity_attachments.get_mut(uuid)
                {
                    wasm_mods::prepare_attachments(holder, map, format!("entity:{uuid:?}"))?;
                }
            }
            Holder::Level => {
                wasm_mods::prepare_attachments(
                    holder,
                    &mut self.world.attachments,
                    format!("level:{}", self.dim.id()),
                )?;
            }
            Holder::Chunk([x, z]) => {
                if let Some(chunk) = self.world.chunks.get_mut(&(x, z))
                    && chunk.saved.is_some()
                {
                    let chunk = std::sync::Arc::make_mut(chunk);
                    let extras = std::sync::Arc::make_mut(chunk.saved.as_mut().unwrap());
                    wasm_mods::prepare_attachments(
                        holder,
                        &mut extras.attachments,
                        format!("chunk:{}:{x}:{z}", self.dim.id()),
                    )?;
                }
                for ((block_x, block_y, block_z), map) in
                    self.world.block_entities.chunk_attachments_mut(x, z)
                {
                    wasm_mods::prepare_attachments(
                        Holder::BlockEntity([block_x, block_y, block_z]),
                        map,
                        format!(
                            "block-entity:{}:{block_x}:{block_y}:{block_z}",
                            self.dim.id()
                        ),
                    )?;
                }
            }
            Holder::BlockEntity([x, y, z]) => {
                if self
                    .world
                    .block_entities
                    .attachments_read((x, y, z))
                    .flatten()
                    .is_some()
                    && let Some(map) = self.world.block_entities.attachments_mut((x, y, z))
                {
                    wasm_mods::prepare_attachments(
                        holder,
                        map,
                        format!("block-entity:{}:{x}:{y}:{z}", self.dim.id()),
                    )?;
                }
            }
            _ => return Err(AttachmentError::MissingHolder),
        }
        Ok(())
    }

    pub(super) fn attachment_map_read(
        &self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<
        Option<&horizon_client::mod_api::Attachments>,
        horizon_client::mod_api::AttachmentError,
    > {
        use horizon_client::mod_api::{AttachmentError, Holder};
        if !self.native_authoritative() {
            return self
                .remote
                .as_ref()
                .map(|remote| remote.attachments.get(&holder))
                .ok_or(AttachmentError::MissingHolder);
        }
        match holder {
            Holder::Player(uuid) => {
                if self.lan_acting_uuid == Some(uuid) {
                    Ok(Some(&self.player_attachments))
                } else {
                    self.guest_attachments
                        .get(&uuid)
                        .map(Some)
                        .ok_or(AttachmentError::MissingHolder)
                }
            }
            Holder::Entity(id) => {
                if !self.attachment_entity_exists(id) {
                    return Err(AttachmentError::MissingHolder);
                }
                attachment_compound(
                    self.persist
                        .raw
                        .get(&id)
                        .and_then(|tag| tag.get(ATTACHMENTS_KEY)),
                )?;
                Ok(self
                    .persist
                    .uuids
                    .get(&id)
                    .and_then(|uuid| self.entity_attachments.get(uuid)))
            }
            Holder::Level | Holder::Chunk(_) | Holder::BlockEntity(_) => {
                world_attachments_read(&self.world, holder)
            }
            _ => Err(AttachmentError::MissingHolder),
        }
    }

    /// A guest's attachments back in their record, for `playerdata` to save.
    pub(super) fn guest_attachments_into(&self, guest: &mut horizon_client::lan::guest::Guest) {
        guest.attachments = self
            .guest_attachments
            .get(&guest.uuid)
            .cloned()
            .unwrap_or_default();
    }

    /// Whether entity `id` is in the level, as a holder of attachments: one walk
    /// over the entity lists (a kind added there holds attachments too), then the
    /// kinds that are not on it.
    fn attachment_entity_exists(&self, id: u32) -> bool {
        use super::entity::{Entity as _, entity_lists_of, for_each_entity};
        let mut found = false;
        for_each_entity!(entity_lists_of!(self), entity => {
            found |= entity.id() == id && !entity.is_removed();
        });
        found || self.entity_ids_off_walk().any(|e| e == id)
    }

    /// The attachments of entities that left the level, dropped: each tick
    /// while any entity has some. Only those entities are looked at, and
    /// nothing else of the entities' memory changes.
    pub(super) fn prune_entity_attachments(&mut self) {
        let gone: Vec<[i32; 4]> = self
            .persist
            .uuids
            .iter()
            .filter(|(id, uuid)| {
                self.entity_attachments.contains_key(*uuid) && !self.attachment_entity_exists(**id)
            })
            .map(|(_, uuid)| *uuid)
            .collect();
        for uuid in gone {
            self.entity_attachments.remove(&uuid);
        }
    }

    pub(super) fn native_authoritative(&self) -> bool {
        self.serving.is_some() || self.options.dedicated.is_some()
    }

    /// Access a holder's attachment map, to lend to a mod: a server's, whose
    /// synced changes are sent at the pass's end, or a client's copy of what
    /// its server synced.
    pub(super) fn attachment_map(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
    {
        if self.native_authoritative() {
            self.world.attachment_syncs.insert(holder);
            return self.attachment_map_quiet(holder);
        }
        match self.remote.as_mut() {
            Some(remote) => Ok(remote.attachments.entry(holder).or_default()),
            None => Err(horizon_client::mod_api::AttachmentError::MissingHolder),
        }
    }

    /// `attachment_map` without recording the holder for the sync.
    pub(super) fn attachment_map_quiet(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
    {
        use horizon_client::mod_api::AttachmentError;
        if !self.native_authoritative() {
            return Err(AttachmentError::MissingHolder);
        }
        if let horizon_client::mod_api::Holder::Entity(id) = holder {
            if !self.attachment_entity_exists(id) {
                return Err(AttachmentError::MissingHolder);
            }
            attachment_compound(
                self.persist
                    .raw
                    .get(&id)
                    .and_then(|tag| tag.get(ATTACHMENTS_KEY)),
            )?;
            let uuid = self.uuid_for(id);
            let map = self.entity_attachments.entry(uuid).or_default();
            wasm_mods::prepare_attachments(holder, map, format!("entity:{uuid:?}"))?;
            return Ok(map);
        }
        if let horizon_client::mod_api::Holder::Player(uuid) = holder {
            if self.lan_acting_uuid == Some(uuid) {
                wasm_mods::prepare_attachments(
                    holder,
                    &mut self.player_attachments,
                    format!("player:{uuid}"),
                )?;
                return Ok(&mut self.player_attachments);
            }
            let map = self
                .guest_attachments
                .get_mut(&uuid)
                .ok_or(AttachmentError::MissingHolder)?;
            wasm_mods::prepare_attachments(holder, map, format!("player:{uuid}"))?;
            return Ok(map);
        }
        world_attachments(&mut self.world, self.dim.id(), holder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{
        chunk::{Chunk, ChunkExtras, block_entity::BlockEntity},
        mod_api::{AttachmentError, Attachments, Holder},
        world::World,
    };
    use horizon_world::nbt::{Compound, Tag};
    use std::sync::Arc;

    #[test]
    fn world_reads_preserve_shared_chunks_and_do_not_queue_writes() {
        let mut world = World::new(BlockStateId(0), 0);
        let raw: Compound = [("removed".into(), Tag::Compound(Compound::new()))].into();
        let extras = Arc::new(ChunkExtras {
            attachments: Attachments::from_nbt(raw.clone()),
            ..Default::default()
        });
        let mut chunk = Chunk::empty(0, 0);
        chunk.saved = Some(extras.clone());
        let chunk = Arc::new(chunk);
        world.chunks.insert((0, 0), chunk.clone());
        world.chunks.insert((1, 0), Arc::new(Chunk::empty(1, 0)));
        world
            .block_entities
            .insert((1, 70, 1), BlockEntity::Chest(Vec::new()));

        for _ in 0..3 {
            assert!(
                world_attachments_read(&world, Holder::Level)
                    .unwrap()
                    .is_some()
            );
            let map = world_attachments_read(&world, Holder::Chunk([0, 0]))
                .unwrap()
                .unwrap();
            assert_eq!(map.to_nbt(), raw);
            assert!(map.sync_changes().is_empty());
            assert!(
                world_attachments_read(&world, Holder::Chunk([1, 0]))
                    .unwrap()
                    .is_none()
            );
            assert!(
                world_attachments_read(&world, Holder::BlockEntity([1, 70, 1]))
                    .unwrap()
                    .is_none()
            );
        }

        assert!(Arc::ptr_eq(world.chunks.get(&(0, 0)).unwrap(), &chunk));
        assert!(Arc::ptr_eq(chunk.saved.as_ref().unwrap(), &extras));
        assert!(world.chunks.get(&(1, 0)).unwrap().saved.is_none());
        assert!(world.modified.is_empty());
        assert!(world.attachment_syncs.is_empty());
        assert!(matches!(
            world_attachments_read(&world, Holder::Chunk([2, 0])),
            Err(AttachmentError::MissingHolder)
        ));
        assert!(matches!(
            world_attachments_read(&world, Holder::BlockEntity([2, 70, 2])),
            Err(AttachmentError::MissingHolder)
        ));
    }

    #[test]
    fn malformed_saved_maps_fail_without_copying_or_dirtying_chunks() {
        let mut world = World::new(BlockStateId(0), 0);
        let extras = Arc::new(ChunkExtras {
            root: [(ATTACHMENTS_KEY.into(), Tag::Int(1))].into(),
            ..Default::default()
        });
        let mut chunk = Chunk::empty(0, 0);
        chunk.saved = Some(extras.clone());
        let chunk = Arc::new(chunk);
        world.chunks.insert((0, 0), chunk.clone());
        assert!(matches!(
            world_attachments_read(&world, Holder::Chunk([0, 0])),
            Err(AttachmentError::InvalidNamespace)
        ));
        assert!(Arc::ptr_eq(world.chunks.get(&(0, 0)).unwrap(), &chunk));
        assert!(Arc::ptr_eq(chunk.saved.as_ref().unwrap(), &extras));
        assert!(world.modified.is_empty());
        assert!(world.attachment_syncs.is_empty());
    }
}

mod generated {
    include!(concat!(env!("OUT_DIR"), "/mod_api_host_generated.rs"));
}

impl horizon_client::mod_api::LevelHost for super::GameLevel<'_> {
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
        horizon_world::level::LevelAccess::block_state(self, BlockPos::new(pos[0], pos[1], pos[2]))
    }
    fn is_client_side(&self) -> bool {
        horizon_world::level::LevelAccess::is_client_side(self)
    }
    fn state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        self.native_state_with_property(state, key, value)
    }
}

impl horizon_client::mod_api::LevelRandomHost for super::GameLevel<'_> {
    fn attachments(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
    {
        if !self.native_authoritative {
            return Err(horizon_client::mod_api::AttachmentError::MissingHolder);
        }
        self.world.attachment_syncs.insert(holder);
        let dimension = match self.dimension {
            horizon_world::level::Dimension::Overworld => "minecraft:overworld",
            horizon_world::level::Dimension::Nether => "minecraft:the_nether",
            horizon_world::level::Dimension::End => "minecraft:the_end",
        };
        world_attachments(self.world, dimension, holder)
    }
    fn random(&mut self) -> Option<&mut horizon_client::mod_api::LegacyRandom> {
        self.native_level_random()
    }
}

pub(super) struct LevelTicks<'a, L> {
    pub level: L,
    pub ticks: &'a mut Ticks,
}

impl<'a, L: std::ops::Deref<Target = super::GameLevel<'a>>> horizon_client::mod_api::LevelHost
    for LevelTicks<'_, L>
{
    fn block_state(&self, pos: [i32; 3]) -> BlockStateId {
        horizon_client::mod_api::LevelHost::block_state(&*self.level, pos)
    }
    fn is_client_side(&self) -> bool {
        horizon_client::mod_api::LevelHost::is_client_side(&*self.level)
    }
    fn state_with_property(
        &self,
        state: BlockStateId,
        key: &str,
        value: &str,
    ) -> Option<BlockStateId> {
        horizon_client::mod_api::LevelHost::state_with_property(&*self.level, state, key, value)
    }
}

impl horizon_client::mod_api::LevelRandomHost for LevelTicks<'_, &mut super::GameLevel<'_>> {
    fn attachments(
        &mut self,
        holder: horizon_client::mod_api::Holder,
    ) -> Result<&mut horizon_client::mod_api::Attachments, horizon_client::mod_api::AttachmentError>
    {
        horizon_client::mod_api::LevelRandomHost::attachments(&mut *self.level, holder)
    }
    fn random(&mut self) -> Option<&mut horizon_client::mod_api::LegacyRandom> {
        horizon_client::mod_api::LevelRandomHost::random(&mut *self.level)
    }
}

impl<'a, L: std::ops::Deref<Target = super::GameLevel<'a>>> horizon_client::mod_api::LevelTicksHost
    for LevelTicks<'_, L>
{
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool {
        self.level
            .native_schedule_block_tick(self.ticks, pos, block, delay)
    }
    fn schedule_fluid_tick(&mut self, pos: [i32; 3], fluid: &str, delay: i32) -> bool {
        self.level
            .native_schedule_fluid_tick(self.ticks, pos, fluid, delay)
    }
}

impl horizon_client::mod_api::LevelWriteHost for LevelTicks<'_, &mut super::GameLevel<'_>> {
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, flags: i32) -> bool {
        self.level
            .native_set_block_property(self.ticks, pos, key, value, flags)
    }

    fn play_sound(
        &mut self,
        sound: &str,
        at: [f64; 3],
        source: usize,
        volume: f32,
        pitch: f32,
    ) -> bool {
        self.level
            .native_play_sound(sound, at, source, volume, pitch)
    }

    fn send_particles(
        &mut self,
        options: &horizon_client::mod_api::ParticleOptions,
        state: Option<BlockStateId>,
        at: [f64; 3],
        spread: [f64; 3],
        speed: f64,
        count: i32,
    ) -> bool {
        self.level
            .native_send_particles(options, state, at, spread, speed, count)
    }
}
