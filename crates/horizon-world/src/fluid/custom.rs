use super::FluidKind;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    sync::RwLock,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidMotion {
    Water,
    Lava,
    None,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FluidDefinition {
    pub id: String,
    pub flowing_id: String,
    pub still_block: String,
    pub flowing_block: String,
    pub bucket: String,
    pub still_texture: String,
    pub flowing_texture: String,
    pub tint: u32,
    pub viscosity: u8,
    pub tick_rate: u32,
    pub light: u8,
    pub can_drown: bool,
    pub motion: FluidMotion,
}

struct Catalog {
    definitions: Vec<FluidDefinition>,
    names: HashMap<String, FluidKind>,
    blocks: HashMap<String, FluidKind>,
    buckets: HashMap<String, FluidKind>,
    /// Content-managed ids stay in this set even when a reload removes their files.
    content: BTreeSet<String>,
    /// Only the first content load may add families; later ones would not be in `Resources`.
    content_loaded: bool,
}

fn identifier(value: &str) -> bool {
    value.split_once(':').is_some_and(|(namespace, path)| {
        !namespace.is_empty()
            && !path.is_empty()
            && namespace != "minecraft"
            && namespace.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
            })
            && path.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_./-".contains(&byte)
            })
    })
}

impl Catalog {
    fn new(mut definitions: Vec<FluidDefinition>) -> io::Result<Self> {
        definitions.sort_by(|left, right| left.id.cmp(&right.id));
        Self::ordered(definitions)
    }
    fn ordered(definitions: Vec<FluidDefinition>) -> io::Result<Self> {
        if definitions.len() > 125 {
            return Err(io::Error::other("too many custom fluid families"));
        }
        let mut catalog = Self {
            definitions,
            names: HashMap::new(),
            blocks: HashMap::new(),
            buckets: HashMap::new(),
            content: BTreeSet::new(),
            content_loaded: false,
        };
        for (index, definition) in catalog.definitions.iter().enumerate() {
            if ![
                &definition.id,
                &definition.flowing_id,
                &definition.still_block,
                &definition.flowing_block,
                &definition.bucket,
                &definition.still_texture,
                &definition.flowing_texture,
            ]
            .into_iter()
            .all(|value| identifier(value))
                || !(1..=8).contains(&definition.viscosity)
                || !(1..=i32::MAX as u32).contains(&definition.tick_rate)
                || definition.light > 15
                || definition.tint > 0xff_ffff
            {
                return Err(io::Error::other("invalid custom fluid definition"));
            }
            let source = FluidKind::Custom(index as u8);
            let flowing = FluidKind::FlowingCustom(index as u8);
            if catalog
                .names
                .insert(definition.id.clone(), source)
                .is_some()
                || catalog
                    .names
                    .insert(definition.flowing_id.clone(), flowing)
                    .is_some()
                || catalog
                    .blocks
                    .insert(definition.still_block.clone(), source)
                    .is_some()
                || catalog
                    .blocks
                    .insert(definition.flowing_block.clone(), flowing)
                    .is_some()
                || catalog
                    .buckets
                    .insert(definition.bucket.clone(), source)
                    .is_some()
            {
                return Err(io::Error::other("duplicate custom fluid binding"));
            }
        }
        Ok(catalog)
    }
}

/// Native mods [`install`] once, then the initial content load and later [`reload`]s replace the
/// catalog reference. Old references stay alive for readers on other threads.
static CATALOG: RwLock<Option<&'static Catalog>> = RwLock::new(None);

fn current() -> Option<&'static Catalog> {
    *CATALOG.read().unwrap_or_else(|e| e.into_inner())
}

/// `existing` with `added` joined (the families sort by id, so a fluid's index is its place
/// among them all), checked as one registry.
fn extended(existing: Option<&Catalog>, added: Vec<FluidDefinition>) -> io::Result<Catalog> {
    let content = existing.map(|c| c.content.clone()).unwrap_or_default();
    let content_loaded = existing.is_some_and(|c| c.content_loaded);
    let mut all = existing.map(|c| c.definitions.clone()).unwrap_or_default();
    all.extend(added);
    let mut catalog = Catalog::new(all)?;
    catalog.content = content;
    catalog.content_loaded = content_loaded;
    Ok(catalog)
}

/// Adds native fluid families at startup, before a world loads. Content mods use [`reload`].
pub fn install(definitions: Vec<FluidDefinition>) -> io::Result<()> {
    if definitions.is_empty() {
        return Ok(());
    }
    let mut slot = CATALOG.write().unwrap_or_else(|e| e.into_inner());
    let catalog = extended(*slot, definitions)?;
    *slot = Some(Box::leak(Box::new(catalog)));
    Ok(())
}

/// Reconciles content files without changing any existing family's index or identity bindings.
/// Additions after the initial content load, removals and unsupported edits request a restart.
fn reloaded(
    existing: Option<&Catalog>,
    definitions: Vec<FluidDefinition>,
) -> io::Result<(Catalog, bool)> {
    let mut incoming = BTreeMap::new();
    for definition in definitions {
        if incoming.insert(definition.id.clone(), definition).is_some() {
            return Err(io::Error::other("duplicate custom fluid id"));
        }
    }
    let startup = existing.is_none_or(|catalog| !catalog.content_loaded);
    let mut content = existing.map(|c| c.content.clone()).unwrap_or_default();
    if startup {
        content.extend(incoming.keys().cloned());
    }
    let mut all = Vec::new();
    let mut restart = false;
    if let Some(existing) = existing {
        all.reserve(existing.definitions.len() + incoming.len());
        for old in &existing.definitions {
            let mut kept = old.clone();
            if existing.content.contains(&old.id) {
                if let Some(new) = incoming.remove(&old.id) {
                    kept.motion = new.motion;
                    kept.tick_rate = new.tick_rate;
                    kept.viscosity = new.viscosity;
                    restart |= kept != new;
                } else {
                    restart = true;
                }
            }
            all.push(kept);
        }
    }
    if startup {
        all.extend(incoming.into_values());
    } else {
        restart |= !incoming.is_empty();
    }
    let mut catalog = Catalog::ordered(all)?;
    catalog.content = content;
    catalog.content_loaded = true;
    Ok((catalog, restart))
}

/// Applies the current content mods' fluid files. Motion, tick rate and viscosity update in
/// place. Returns whether additions, removals or other changes were left for restart: the game's
/// `Resources` and its block-state-indexed behaviour tables do not change during a pack reload.
pub fn reload(definitions: Vec<FluidDefinition>) -> io::Result<bool> {
    let mut slot = CATALOG.write().unwrap_or_else(|e| e.into_inner());
    let (catalog, restart) = reloaded(*slot, definitions)?;
    if (*slot).is_none_or(|old| {
        old.definitions != catalog.definitions
            || old.content != catalog.content
            || old.content_loaded != catalog.content_loaded
    }) {
        *slot = Some(Box::leak(Box::new(catalog)));
    }
    Ok(restart)
}

pub fn definitions() -> &'static [FluidDefinition] {
    current().map_or(&[], |catalog| catalog.definitions.as_slice())
}

pub fn definition(kind: FluidKind) -> Option<&'static FluidDefinition> {
    let index = match kind {
        FluidKind::Custom(index) | FluidKind::FlowingCustom(index) => index,
        _ => return None,
    };
    current()?.definitions.get(usize::from(index))
}

pub(super) fn named(name: &str) -> Option<FluidKind> {
    current()?.names.get(name).copied()
}

pub fn block(name: &str) -> Option<FluidKind> {
    current()?.blocks.get(name).copied()
}

pub fn bucket(name: &str) -> Option<FluidKind> {
    current()?.buckets.get(name).copied()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn oil() -> FluidDefinition {
        FluidDefinition {
            id: "test:oil".into(),
            flowing_id: "test:flowing_oil".into(),
            still_block: "test:oil".into(),
            flowing_block: "test:flowing_oil".into(),
            bucket: "test:oil_bucket".into(),
            still_texture: "test:block/oil_still".into(),
            flowing_texture: "test:block/oil_flow".into(),
            tint: 0x443322,
            viscosity: 2,
            tick_rate: 10,
            light: 0,
            can_drown: true,
            motion: FluidMotion::Water,
        }
    }

    #[test]
    fn empty_registry_has_no_custom_bindings() {
        let catalog = Catalog::new(Vec::new()).unwrap();
        assert!(catalog.names.is_empty());
        assert!(catalog.blocks.is_empty());
        assert!(catalog.buckets.is_empty());
    }

    #[test]
    fn one_family_binds_source_flow_and_bucket_without_lava_aliasing() {
        let catalog = Catalog::new(vec![oil()]).unwrap();
        let source = catalog.names["test:oil"];
        let flowing = catalog.names["test:flowing_oil"];
        assert!(source.same_family(flowing));
        assert!(!source.same_family(FluidKind::Lava));
        assert_eq!(catalog.blocks["test:oil"], source);
        assert_eq!(catalog.buckets["test:oil_bucket"], source);
        assert_eq!(source.wire_id(), 5);
        assert_eq!(flowing.wire_id(), 6);
        assert_eq!(source.mask(), 0);
    }

    #[test]
    fn families_from_two_sources_join_one_registry() {
        let tar = || FluidDefinition {
            id: "mod:tar".into(),
            flowing_id: "mod:flowing_tar".into(),
            still_block: "mod:tar".into(),
            flowing_block: "mod:flowing_tar".into(),
            bucket: "mod:tar_bucket".into(),
            still_texture: "mod:block/tar_still".into(),
            flowing_texture: "mod:block/tar_flow".into(),
            ..oil()
        };
        let first = extended(None, vec![oil()]).unwrap();
        let both = extended(Some(&first), vec![tar()]).unwrap();
        // sorted by id, as one registry: `mod:tar` before `test:oil`.
        assert_eq!(
            both.definitions
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>(),
            ["mod:tar", "test:oil"]
        );
        assert_eq!(both.names["mod:tar"], FluidKind::Custom(0));
        assert_eq!(both.names["test:oil"], FluidKind::Custom(1));
        // a name both bind is refused, whichever source came first.
        assert!(extended(Some(&both), vec![tar()]).is_err());
        assert!(extended(Some(&first), vec![oil()]).is_err());
    }

    #[test]
    fn bucket_fill_and_place_resolve_the_same_family() {
        let catalog = Catalog::new(vec![oil()]).unwrap();
        let picked_up = catalog.blocks["test:oil"];
        let bucket = &catalog.definitions[usize::from(match picked_up {
            FluidKind::Custom(index) => index,
            other => panic!("not a custom source: {other:?}"),
        })]
        .bucket;
        assert_eq!(bucket, "test:oil_bucket");
        assert_eq!(catalog.buckets[bucket], picked_up);
        assert_eq!(catalog.blocks["test:oil"], picked_up);
    }

    #[test]
    fn content_reload_updates_physics_and_defers_registry_changes() {
        let added = FluidDefinition {
            id: "a:new".into(),
            flowing_id: "a:flowing_new".into(),
            still_block: "a:new".into(),
            flowing_block: "a:flowing_new".into(),
            bucket: "a:new_bucket".into(),
            still_texture: "a:block/new_still".into(),
            flowing_texture: "a:block/new_flow".into(),
            ..oil()
        };
        let native = Catalog::new(vec![added.clone()]).unwrap();
        let (startup, restart) = reloaded(Some(&native), vec![oil()]).unwrap();
        assert!(!restart);
        assert_eq!(startup.names["a:new"], FluidKind::Custom(0));
        assert_eq!(startup.names["test:oil"], FluidKind::Custom(1));

        let (first, restart) = reloaded(None, vec![oil()]).unwrap();
        assert!(!restart);
        let changed = FluidDefinition {
            motion: FluidMotion::Lava,
            tick_rate: 33,
            viscosity: 1,
            ..oil()
        };
        let (updated, restart) = reloaded(Some(&first), vec![changed, added]).unwrap();
        assert!(
            restart,
            "a fluid added after the initial load needs restart"
        );
        assert_eq!(updated.definitions.len(), 1);
        assert_eq!(updated.names["test:oil"], FluidKind::Custom(0));
        assert!(!updated.names.contains_key("a:new"));
        assert_eq!(updated.definitions[0].tick_rate, 33);
        assert_eq!(updated.definitions[0].viscosity, 1);
        assert_eq!(updated.definitions[0].motion, FluidMotion::Lava);
        assert_eq!(FluidKind::Custom(0).wire_id(), 5);

        let rebound = FluidDefinition {
            bucket: "test:different_bucket".into(),
            tint: 0,
            tick_rate: 7,
            ..oil()
        };
        let (after, restart) = reloaded(Some(&updated), vec![rebound]).unwrap();
        assert!(
            restart,
            "rebinding oil and changing its tint need a restart"
        );
        assert_eq!(after.definitions.len(), 1);
        assert_eq!(after.names["test:oil"], FluidKind::Custom(0));
        assert_eq!(after.definitions[0].bucket, "test:oil_bucket");
        assert_eq!(after.definitions[0].tint, 0x443322);
        assert_eq!(after.definitions[0].tick_rate, 7);
        assert!(after.buckets.contains_key("test:oil_bucket"));
        assert!(!after.buckets.contains_key("test:different_bucket"));

        let (removed, restart) = reloaded(Some(&after), Vec::new()).unwrap();
        assert!(restart);
        assert_eq!(removed.names["test:oil"], FluidKind::Custom(0));
        assert_eq!(removed.definitions.len(), 1);
    }
}
