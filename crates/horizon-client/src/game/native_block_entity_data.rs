//! A block entity's saved data for native and WebAssembly server mods, by position (REM-15): what
//! `/data get block` shows and `/data merge block` changes, for any block entity, vanilla's or a mod's.
//! They answer `None` (or an error) for an unloaded block or one with no block entity.
use super::Game;
use horizon_world::nbt::{Compound, Tag};

impl Game<'_> {
    /// The block entity at `pos`: its type id and its saved data as `/data get block` shows it
    /// (`BlockEntity.saveWithoutMetadata`: without `id`, `x`, `y`, `z` and `keepPacked`). None for an
    /// unloaded block or a block with no block entity. API @API:block_entity_data@.
    pub(super) fn native_block_entity_data(&self, pos: [i32; 3]) -> Option<(String, Compound)> {
        if !self.world.loaded(pos[0], pos[2]) {
            return None;
        }
        let key = (pos[0], pos[1], pos[2]);
        let entity = self.world.block_entities.get(&key)?;
        let name = self.data_block_name(pos);
        let tag = horizon_client::save::encode_block_entity(key, entity, &name);
        let id = match tag.get("id") {
            Some(Tag::String(id)) => id.to_string_lossy(),
            _ => return None,
        };
        let Tag::Compound(mut compound) = tag else {
            return None;
        };
        for key in ["id", "x", "y", "z", "keepPacked"] {
            compound.remove(&key.into());
        }
        Some((id, compound))
    }

    /// Changes the block entity at `pos`: `nbt` is merged into its saved data as `/data merge block`
    /// does, or with `replace` becomes all of it (the block entity's type, position and lock stay).
    /// The chunk is saved next time and clients are sent the block entity's update tag with the level's
    /// other block changes. An error says why nothing changed (no block entity, data nested too deep,
    /// data the block entity does not accept, or data that changes nothing). API @API:block_entity_data@.
    pub(super) fn native_block_entity_patch(&mut self, pos: [i32; 3], nbt: Compound, replace: bool) -> Result<(), String> {
        let Some((_, old)) = self.native_block_entity_data(pos) else {
            return Err("there is no block entity at that position".into());
        };
        let key = (pos[0], pos[1], pos[2]);
        let name = self.data_block_name(pos);
        let before = match self.world.block_entities.get(&key) {
            Some(entity) => horizon_client::save::encode_block_entity(key, entity, &name),
            None => return Err("there is no block entity at that position".into()),
        };
        if replace && !matches!(self.world.block_entities.get(&key), Some(horizon_client::containers::BlockEntity::Mod(_))) {
            return Err("only a mod block entity's data can be replaced; merge into the others".into());
        }
        let patch = Tag::Compound(nbt);
        if horizon_client::nbt_path::too_deep(&patch, 0) {
            return Err("the data is nested too deep".into());
        }
        let mut tag = if replace {
            patch
        } else {
            horizon_client::data_cmd::merged(&Tag::Compound(old), &patch)
        };
        let Tag::Compound(compound) = &mut tag else {
            return Err("the data is not a compound".into());
        };
        for field in ["id", "x", "y", "z"] {
            if let Some(value) = before.get(field) {
                compound.insert(field.into(), value.clone());
            }
        }
        let Some((_, new)) = horizon_client::save::decode_block_entity(&tag) else {
            return Err("the block entity does not accept that data".into());
        };
        let after = horizon_client::save::encode_block_entity(key, &new, &name);
        if after == before {
            return Err("the data changes nothing".into());
        }
        self.world.block_entities.insert(key, new);
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.native_block_entity_sync(pos);
        Ok(())
    }
}
