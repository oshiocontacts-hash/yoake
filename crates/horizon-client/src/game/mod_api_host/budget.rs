use super::{Game, wasm_mods};
use horizon_world::{
    attachments::ATTACHMENTS_KEY,
    compression::Compression,
    nbt::{self, Compound, Limits, Tag},
    region::RegionFile,
};
use std::{
    collections::BTreeSet,
    fs::{self, File},
    io::{self, Read},
    path::Path,
};

impl Game<'_> {
    pub(in crate::game) fn index_saved_attachment_usage(
        &self,
        level: &horizon_client::save::Level,
    ) {
        if horizon_client::mod_api::mods().is_empty() {
            return;
        }
        let registry = wasm_mods::attachment_registry();
        let legacy = (!level.singleplayer_file)
            .then_some(level.player_extra.as_ref())
            .flatten()
            .map(|player| (level.player_uuid, player));
        if let Err(error) = scan_saved_attachments(&self.world_root, legacy, &mut |holder, data| {
            registry.account_saved(holder, data);
            Ok(())
        }) {
            registry.deny_growth();
            eprintln!(
                "[attachments] saved-world quota index incomplete; refusing attachment growth: {error}"
            );
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn checked_metadata(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(invalid("quota index does not follow symbolic links"))
        }
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn read_dat(path: &Path) -> io::Result<Tag> {
    let metadata =
        checked_metadata(path)?.ok_or_else(|| invalid("missing saved attachment file"))?;
    let limits = Limits::default();
    if !metadata.is_file() || metadata.len() > limits.max_bytes as u64 {
        return Err(invalid("saved attachment file exceeds quota index limits"));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limits.max_bytes as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > limits.max_bytes {
        return Err(invalid("saved attachment file exceeds quota index limits"));
    }
    let decoded;
    let bytes = if bytes.starts_with(&[0x1f, 0x8b]) {
        decoded = Compression::Gzip.decode(&bytes, limits.max_bytes)?;
        decoded.as_slice()
    } else {
        bytes.as_slice()
    };
    let (root, _) = nbt::read_named(bytes, limits).map_err(|error| invalid(&error.to_string()))?;
    if !matches!(root.tag, Tag::Compound(_)) {
        return Err(invalid("saved attachment root is not a compound"));
    }
    Ok(root.tag)
}

fn account(
    tag: &Tag,
    holder: Option<String>,
    visit: &mut impl FnMut(String, Compound) -> io::Result<()>,
) -> io::Result<()> {
    match tag.get(ATTACHMENTS_KEY) {
        Some(Tag::Compound(data)) if !data.is_empty() => visit(
            holder.ok_or_else(|| invalid("saved attachments have no holder identity"))?,
            data.clone(),
        ),
        Some(Tag::Compound(_)) | None => Ok(()),
        Some(_) => Err(invalid("saved attachment map is not a compound")),
    }
}

fn uuid(tag: &Tag) -> Option<[i32; 4]> {
    match tag.get("UUID")? {
        Tag::IntArray(parts) => parts.as_slice().try_into().ok(),
        _ => None,
    }
}

fn account_entity(
    tag: &Tag,
    visit: &mut impl FnMut(String, Compound) -> io::Result<()>,
) -> io::Result<()> {
    account(tag, uuid(tag).map(|uuid| format!("entity:{uuid:?}")), visit)?;
    if let Some(Tag::List(passengers)) = tag.get("Passengers") {
        for passenger in passengers {
            account_entity(passenger, visit)?;
        }
    }
    Ok(())
}

fn account_player(
    tag: &Tag,
    player_uuid: Option<u128>,
    visit: &mut impl FnMut(String, Compound) -> io::Result<()>,
) -> io::Result<()> {
    account(tag, player_uuid.map(|uuid| format!("player:{uuid}")), visit)?;
    if let Some(vehicle) = tag.get("RootVehicle").and_then(|root| root.get("Entity")) {
        account_entity(vehicle, visit)?;
    }
    Ok(())
}

fn scan_saved_attachments(
    root: &Path,
    legacy: Option<(Option<[i32; 4]>, &Tag)>,
    visit: &mut impl FnMut(String, Compound) -> io::Result<()>,
) -> io::Result<()> {
    checked_metadata(root)?;
    let mut remaining = 100_000usize;
    let mut bounded_visit = |holder, data| {
        if remaining == 0 {
            return Err(invalid("too many saved attachment holders to index"));
        }
        remaining -= 1;
        visit(holder, data)
    };
    if let Some((player_uuid, tag)) = legacy {
        account_player(
            tag,
            player_uuid.map(horizon_client::lan::playerdata::ints_uuid),
            &mut bounded_visit,
        )?;
    }
    let players = root.join("players");
    checked_metadata(&players)?;
    let players = players.join("data");
    if checked_metadata(&players)?.is_some() {
        let mut names = BTreeSet::new();
        for entry in fs::read_dir(&players)? {
            let entry = entry?;
            checked_metadata(&entry.path())?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(invalid("non-text player filename"));
            };
            if let Some(stem) = name
                .strip_suffix(".dat")
                .or_else(|| name.strip_suffix(".dat_old"))
            {
                names.insert(stem.to_owned());
                if names.len() > 100_000 {
                    return Err(invalid("too many saved player files to index"));
                }
            }
        }
        for name in names {
            let player_uuid = u128::from_str_radix(&name.replace('-', ""), 16)
                .map_err(|_| invalid("invalid saved player UUID"))?;
            let current = players.join(format!("{name}.dat"));
            let backup = players.join(format!("{name}.dat_old"));
            let tag = read_dat(&current).or_else(|_| read_dat(&backup))?;
            account_player(&tag, Some(player_uuid), &mut bounded_visit)?;
        }
    }
    let dimensions = root.join("dimensions");
    if checked_metadata(&dimensions)?.is_some() {
        scan_dimensions(&dimensions, &dimensions, 0, &mut bounded_visit)?;
    }
    Ok(())
}

fn dimension_id(root: &Path, dimension: &Path) -> io::Result<String> {
    let relative = dimension
        .strip_prefix(root)
        .map_err(|_| invalid("invalid dimension path"))?;
    let mut parts = relative.iter();
    let namespace = parts
        .next()
        .and_then(|part| part.to_str())
        .ok_or_else(|| invalid("missing dimension namespace"))?;
    let path = parts
        .map(|part| {
            part.to_str()
                .ok_or_else(|| invalid("non-text dimension path"))
        })
        .collect::<io::Result<Vec<_>>>()?
        .join("/");
    if path.is_empty() {
        return Err(invalid("missing dimension path"));
    }
    Ok(format!("{namespace}:{path}"))
}

fn scan_dimensions(
    root: &Path,
    directory: &Path,
    depth: usize,
    visit: &mut impl FnMut(String, Compound) -> io::Result<()>,
) -> io::Result<()> {
    if depth > 64 {
        return Err(invalid("dimension path exceeds quota index limits"));
    }
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = checked_metadata(&path)?
            .ok_or_else(|| invalid("saved file disappeared during quota indexing"))?;
        if metadata.is_dir() {
            scan_dimensions(root, &path, depth + 1, visit)?;
        } else if path
            .file_name()
            .is_some_and(|name| name == "attachments.dat")
            && directory.file_name().is_some_and(|name| name == "horizon")
            && directory
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == "data")
        {
            let dimension = directory
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| invalid("invalid level attachment path"))?;
            let dimension = dimension_id(root, dimension)?;
            let tag = read_dat(&path)?;
            let data = tag
                .get("data")
                .ok_or_else(|| invalid("missing level attachment data"))?;
            let Tag::Compound(data) = data else {
                return Err(invalid("level attachment data is not a compound"));
            };
            if !data.is_empty() {
                visit(format!("level:{dimension}"), data.clone())?;
            }
        } else if path.extension().is_some_and(|extension| extension == "mca")
            && directory
                .file_name()
                .is_some_and(|name| name == "region" || name == "entities")
        {
            if metadata.len() > 1 << 30 {
                return Err(invalid("region exceeds quota index limits"));
            }
            let dimension = directory
                .parent()
                .ok_or_else(|| invalid("invalid region path"))?;
            let dimension = dimension_id(root, dimension)?;
            let entities = directory.file_name().is_some_and(|name| name == "entities");
            let mut region = RegionFile::open(&path)?;
            let positions: Vec<_> = region.chunks().map(|(position, _)| position).collect();
            for position in positions {
                checked_metadata(&directory.join(format!("c.{}.{}.mcc", position.x, position.z)))?;
                let record = region
                    .read(position)?
                    .ok_or_else(|| invalid("missing indexed chunk"))?;
                let tag = record.nbt(Limits::default())?.tag;
                if entities {
                    if let Some(Tag::List(entries)) = tag.get("Entities") {
                        for entity in entries {
                            account_entity(entity, visit)?;
                        }
                    }
                } else {
                    account(
                        &tag,
                        Some(format!("chunk:{dimension}:{}:{}", position.x, position.z)),
                        visit,
                    )?;
                    if let Some(Tag::List(entries)) = tag.get("block_entities") {
                        for block_entity in entries {
                            let coordinates =
                                ["x", "y", "z"].map(|key| match block_entity.get(key) {
                                    Some(Tag::Int(value)) => Some(*value),
                                    _ => None,
                                });
                            let holder = match coordinates {
                                [Some(x), Some(y), Some(z)] => {
                                    Some(format!("block-entity:{dimension}:{x}:{y}:{z}"))
                                }
                                _ => None,
                            };
                            account(block_entity, holder, visit)?;
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::position::ChunkPos;
    use horizon_world::{nbt::NamedTag, region::ChunkRecord};
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    struct SavedWorld(PathBuf);

    impl SavedWorld {
        fn new() -> Self {
            let suffix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let root = std::env::temp_dir()
                .join(format!("yoake-w2bf-budget-{}-{suffix}", std::process::id()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn dat(&self, relative: &str, tag: Tag) {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let bytes = nbt::write_named(
                &NamedTag {
                    name: "".into(),
                    tag,
                },
                Limits::default(),
            )
            .unwrap();
            fs::write(path, Compression::Gzip.encode(&bytes).unwrap()).unwrap();
        }

        fn region(&self, relative: &str, position: ChunkPos, tag: Tag) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let record = ChunkRecord::from_nbt(
                &NamedTag {
                    name: "".into(),
                    tag,
                },
                Compression::Zlib,
                17,
                Limits::default(),
            )
            .unwrap();
            RegionFile::open_writable(&path, true)
                .unwrap()
                .write(position, &record)
                .unwrap();
            path
        }

        fn scan(
            &self,
            legacy: Option<(Option<[i32; 4]>, &Tag)>,
        ) -> io::Result<BTreeMap<String, Compound>> {
            let mut found = BTreeMap::new();
            scan_saved_attachments(&self.0, legacy, &mut |holder, data| {
                assert!(found.insert(holder, data).is_none());
                Ok(())
            })?;
            Ok(found)
        }
    }

    impl Drop for SavedWorld {
        fn drop(&mut self) {
            assert_eq!(self.0.parent(), Some(std::env::temp_dir().as_path()));
            assert!(
                self.0
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .starts_with("yoake-w2bf-budget-")
            );
            crate::test_support::remove_dir_all(&self.0);
        }
    }

    fn compound<const COUNT: usize>(entries: [(&str, Tag); COUNT]) -> Tag {
        Tag::Compound(
            entries
                .into_iter()
                .map(|(key, value)| (key.into(), value))
                .collect(),
        )
    }

    fn data(marker: i32) -> Compound {
        [("ruby:counter".into(), Tag::Int(marker))].into()
    }

    fn saved(marker: i32) -> Tag {
        compound([(ATTACHMENTS_KEY, Tag::Compound(data(marker)))])
    }

    #[test]
    fn indexes_unloaded_holders_without_changing_saved_files() {
        let world = SavedWorld::new();
        let player = compound([
            (ATTACHMENTS_KEY, Tag::Compound(data(1))),
            (
                "RootVehicle",
                compound([(
                    "Entity",
                    compound([
                        ("UUID", Tag::IntArray(vec![5, 6, 7, 8])),
                        (ATTACHMENTS_KEY, Tag::Compound(data(2))),
                    ]),
                )]),
            ),
        ]);
        world.dat(
            "players/data/00000000-0000-0000-0000-000000000001.dat",
            player,
        );
        let legacy = saved(3);
        world.dat(
            "dimensions/ruby/sky/islands/data/horizon/attachments.dat",
            compound([("data", Tag::Compound(data(4)))]),
        );
        let position = ChunkPos { x: 33, z: -4 };
        let chunks = world.region(
            "dimensions/ruby/sky/islands/region/r.1.-1.mca",
            position,
            compound([
                (ATTACHMENTS_KEY, Tag::Compound(data(5))),
                (
                    "block_entities",
                    Tag::List(vec![compound([
                        ("x", Tag::Int(528)),
                        ("y", Tag::Int(70)),
                        ("z", Tag::Int(-64)),
                        (ATTACHMENTS_KEY, Tag::Compound(data(6))),
                    ])]),
                ),
            ]),
        );
        let entities = world.region(
            "dimensions/ruby/sky/islands/entities/r.1.-1.mca",
            position,
            compound([(
                "Entities",
                Tag::List(vec![compound([
                    ("UUID", Tag::IntArray(vec![1, 2, 3, 4])),
                    (ATTACHMENTS_KEY, Tag::Compound(data(7))),
                    (
                        "Passengers",
                        Tag::List(vec![compound([
                            ("UUID", Tag::IntArray(vec![4, 3, 2, 1])),
                            (ATTACHMENTS_KEY, Tag::Compound(data(8))),
                        ])]),
                    ),
                ])]),
            )]),
        );
        let before = [fs::read(&chunks).unwrap(), fs::read(&entities).unwrap()];
        let found = world.scan(Some((Some([0, 0, 0, 2]), &legacy))).unwrap();
        assert_eq!(
            found,
            [
                ("player:1", 1),
                ("entity:[5, 6, 7, 8]", 2),
                ("player:2", 3),
                ("level:ruby:sky/islands", 4),
                ("chunk:ruby:sky/islands:33:-4", 5),
                ("block-entity:ruby:sky/islands:528:70:-64", 6),
                ("entity:[1, 2, 3, 4]", 7),
                ("entity:[4, 3, 2, 1]", 8),
            ]
            .into_iter()
            .map(|(holder, marker)| (holder.to_owned(), data(marker)))
            .collect()
        );
        assert_eq!(
            before,
            [fs::read(chunks).unwrap(), fs::read(entities).unwrap()]
        );
    }

    #[test]
    fn player_backups_are_fallbacks_not_additional_holders() {
        let world = SavedWorld::new();
        let first = "players/data/00000000-0000-0000-0000-000000000001";
        let second = "players/data/00000000-0000-0000-0000-000000000002";
        world.dat(&format!("{first}.dat"), saved(1));
        world.dat(&format!("{first}.dat_old"), saved(9));
        world.dat(&format!("{second}.dat_old"), saved(2));
        fs::write(world.0.join(format!("{second}.dat")), b"invalid NBT").unwrap();
        assert_eq!(
            world.scan(None).unwrap(),
            [
                ("player:1".to_owned(), data(1)),
                ("player:2".to_owned(), data(2)),
            ]
            .into()
        );
    }

    #[test]
    fn malformed_saved_attachment_maps_fail_closed() {
        let world = SavedWorld::new();
        world.dat(
            "players/data/00000000-0000-0000-0000-000000000001.dat",
            compound([(ATTACHMENTS_KEY, Tag::Int(7))]),
        );
        assert_eq!(
            world.scan(None).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn saved_entity_data_requires_a_holder_identity() {
        let world = SavedWorld::new();
        world.region(
            "dimensions/minecraft/overworld/entities/r.0.0.mca",
            ChunkPos { x: 0, z: 0 },
            compound([("Entities", Tag::List(vec![saved(1)]))]),
        );
        assert_eq!(
            world.scan(None).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
