//! A block entity's saved data for native and WebAssembly server mods, by position (REM-15): what
//! `/data get block` shows and `/data merge block` changes, for any block entity, vanilla's or a mod's.
//! They answer `None` (or an error) for an unloaded block or one with no block entity.
use super::Game;
use horizon_world::nbt::{Compound, Tag};

/// The type id of a saved block entity tag and its data without `id`, `x`, `y`, `z` and `keepPacked`
/// (`BlockEntity.saveWithoutMetadata`, as `/data get block` shows it).
fn split(saved: &Tag) -> Option<(String, Compound)> {
    let id = match saved.get("id") {
        Some(Tag::String(id)) => id.to_string_lossy(),
        _ => return None,
    };
    let Tag::Compound(mut data) = saved.clone() else {
        return None;
    };
    for key in ["id", "x", "y", "z", "keepPacked"] {
        data.remove(&key.into());
    }
    Some((id, data))
}

/// Whether a type id names a mod's block entity: only those take their whole data replaced.
fn is_mod(id: &str) -> bool {
    id.contains(':') && !id.starts_with("minecraft:")
}

/// The saved tag a patch makes of `saved`: the patch merged into the data as `/data merge block` does, or
/// with `replace` (a mod block entity only) the patch as all of it. The type, the position and the lock stay.
fn patched(saved: &Tag, patch: Compound, replace: bool) -> Result<Tag, String> {
    let (id, old) = split(saved).ok_or("the block entity has no type")?;
    if replace && !is_mod(&id) {
        return Err("only a mod block entity's data can be replaced; merge into the others".into());
    }
    let patch = Tag::Compound(patch);
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
        if let Some(value) = saved.get(field) {
            compound.insert(field.into(), value.clone());
        }
    }
    Ok(tag)
}

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
        split(&horizon_client::save::encode_block_entity(key, entity, &name))
    }

    /// Changes the block entity at `pos`: `nbt` is merged into its saved data as `/data merge block`
    /// does, or with `replace` becomes all of it (the block entity's type, position and lock stay).
    /// The chunk is saved next time and clients are sent the block entity's update tag with the level's
    /// other block changes. An error says why nothing changed (no block entity, data nested too deep,
    /// data the block entity does not accept, or data that changes nothing). API @API:block_entity_data@.
    pub(super) fn native_block_entity_patch(&mut self, pos: [i32; 3], nbt: Compound, replace: bool) -> Result<(), String> {
        if !self.world.loaded(pos[0], pos[2]) {
            return Err("that position is not loaded".into());
        }
        let key = (pos[0], pos[1], pos[2]);
        let name = self.data_block_name(pos);
        let Some(entity) = self.world.block_entities.get(&key) else {
            return Err("there is no block entity at that position".into());
        };
        let before = horizon_client::save::encode_block_entity(key, entity, &name);
        let tag = patched(&before, nbt, replace)?;
        let Some((_, new)) = horizon_client::save::decode_block_entity(&tag) else {
            return Err("the block entity does not accept that data".into());
        };
        if horizon_client::save::encode_block_entity(key, &new, &name) == before {
            return Err("the data changes nothing".into());
        }
        self.world.block_entities.insert(key, new);
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.native_block_entity_sync(pos);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::containers::BlockEntity;
    use horizon_client::save::{decode_block_entity, encode_block_entity};
    use horizon_world::nbt::parse_snbt;

    fn saved(snbt: &str) -> Tag {
        parse_snbt(snbt).unwrap().0
    }

    fn compound(snbt: &str) -> Compound {
        match saved(snbt) {
            Tag::Compound(compound) => compound,
            _ => panic!("not a compound"),
        }
    }

    const MACHINE: &str =
        r#"{id:"gtmod:macerator",x:5,y:-60,z:12,energy:1200L,progress:7,Items:[{Slot:0b,id:"minecraft:iron_ore",count:3}]}"#;

    #[test]
    fn a_mod_block_entity_reads_as_its_type_and_its_own_data() {
        let (id, data) = split(&saved(MACHINE)).expect("a type");
        assert_eq!(id, "gtmod:macerator");
        for key in ["id", "x", "y", "z"] {
            assert!(!data.contains_key(&key.into()), "{key}");
        }
        assert_eq!(data.get(&"energy".into()), Some(&Tag::Long(1200)));
        assert!(data.contains_key(&"Items".into()), "the slots are part of the saved data");
        assert!(split(&saved("{x:1,y:2,z:3}")).is_none(), "no type, no data");
    }

    #[test]
    fn a_merge_keeps_the_rest_and_persists_through_save_and_load() {
        let before = saved(MACHINE);
        let tag = patched(&before, compound("{progress:9,fuel:4}"), false).expect("merges");
        let (pos, entity) = decode_block_entity(&tag).expect("the block entity loads again");
        assert_eq!(pos, (5, -60, 12));
        let BlockEntity::Mod(machine) = &entity else { panic!("not a mod's") };
        assert_eq!(machine.id, "gtmod:macerator");
        assert_eq!(machine.nbt.get(&"energy".into()), Some(&Tag::Long(1200)), "what the patch did not name stays");
        assert_eq!(machine.nbt.get(&"progress".into()), Some(&Tag::Int(9)));
        assert_eq!(machine.nbt.get(&"fuel".into()), Some(&Tag::Int(4)));
        assert_eq!(machine.items.len(), 1, "the slots stay");
        // What the world saves is what the guest reads next.
        let saved_again = encode_block_entity(pos, &entity, "gtmod:macerator");
        let (_, read) = split(&saved_again).unwrap();
        assert_eq!(read.get(&"progress".into()), Some(&Tag::Int(9)));
    }

    #[test]
    fn a_replace_is_a_mod_block_entitys_only_and_keeps_its_type_and_place() {
        let before = saved(MACHINE);
        let tag = patched(&before, compound("{energy:5L}"), true).expect("replaces");
        let (pos, entity) = decode_block_entity(&tag).expect("loads");
        assert_eq!(pos, (5, -60, 12));
        let BlockEntity::Mod(machine) = &entity else { panic!("not a mod's") };
        assert_eq!(machine.id, "gtmod:macerator");
        assert_eq!(machine.nbt.get(&"energy".into()), Some(&Tag::Long(5)));
        assert!(!machine.nbt.contains_key(&"progress".into()), "the old data is gone");
        assert!(machine.items.is_empty(), "so are the slots: the patch was all of it");
        // A vanilla block entity can only be merged into.
        let ender = saved(r#"{id:"minecraft:ender_chest",x:0,y:0,z:0}"#);
        assert!(patched(&ender, compound("{a:1}"), true).unwrap_err().contains("merge"));
        assert!(patched(&ender, compound("{a:1}"), false).is_ok());
    }

    #[test]
    fn data_nested_too_deep_is_refused() {
        let mut deep = Tag::Compound(Compound::new());
        for _ in 0..600 {
            let mut outer = Compound::new();
            outer.insert("n".into(), deep);
            deep = Tag::Compound(outer);
        }
        let Tag::Compound(deep) = deep else { unreachable!() };
        assert!(patched(&saved(MACHINE), deep, false).unwrap_err().contains("too deep"));
    }
}
