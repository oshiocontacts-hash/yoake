use super::FluidKind;
use std::{collections::HashMap, io, sync::OnceLock};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FluidMotion {
    Water,
    Lava,
    None,
}

#[derive(Clone, Debug)]
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
        if definitions.len() > 125 {
            return Err(io::Error::other("too many custom fluid families"));
        }
        let mut catalog = Self {
            definitions,
            names: HashMap::new(),
            blocks: HashMap::new(),
            buckets: HashMap::new(),
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
            if catalog.names.insert(definition.id.clone(), source).is_some()
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

static CATALOG: OnceLock<Catalog> = OnceLock::new();

pub fn install(definitions: Vec<FluidDefinition>) -> io::Result<()> {
    if definitions.is_empty() {
        return Ok(());
    }
    let catalog = Catalog::new(definitions)?;
    CATALOG
        .set(catalog)
        .map_err(|_| io::Error::other("fluid registry already frozen"))
}

pub fn definitions() -> &'static [FluidDefinition] {
    CATALOG
        .get()
        .map_or(&[], |catalog| catalog.definitions.as_slice())
}

pub fn definition(kind: FluidKind) -> Option<&'static FluidDefinition> {
    let index = match kind {
        FluidKind::Custom(index) | FluidKind::FlowingCustom(index) => index,
        _ => return None,
    };
    CATALOG.get()?.definitions.get(usize::from(index))
}

pub(super) fn named(name: &str) -> Option<FluidKind> {
    CATALOG.get()?.names.get(name).copied()
}

pub fn block(name: &str) -> Option<FluidKind> {
    CATALOG.get()?.blocks.get(name).copied()
}

pub fn bucket(name: &str) -> Option<FluidKind> {
    CATALOG.get()?.buckets.get(name).copied()
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
}
