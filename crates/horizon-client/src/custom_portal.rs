use horizon_world::{
    level::portal_type::{PortalType, PortalTypes},
    loot::PackOverlay,
    registry::BlockRegistry,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::Path,
};

const KIND: &str = "horizon/portal";

pub fn load(
    root: &Path,
    overlay: &PackOverlay<'_>,
    registry: &BlockRegistry,
    block_tags: &HashMap<String, HashSet<String>>,
    level_known: impl Fn(&str) -> bool,
) -> PortalTypes {
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    if let Ok(namespaces) = std::fs::read_dir(root.join("data")) {
        for namespace in namespaces.flatten() {
            let name = namespace.file_name().to_string_lossy().into_owned();
            let Ok(folder) = crate::json_folder::read(&namespace.path().join("horizon/portal")) else {
                continue;
            };
            for file in &folder.files {
                if let Ok(bytes) = file.bytes() {
                    files.insert(format!("{name}:{}", file.name), bytes.to_vec());
                }
            }
        }
    }
    for (id, versions) in overlay.kind(KIND) {
        files.insert(id.to_owned(), PackOverlay::top(versions).to_vec());
    }
    let mut out = PortalTypes::default();
    for (id, bytes) in files {
        match parse(&id, &bytes, &level_known, &|block| {
            registry.default_state(block).is_some()
        }) {
            Ok(mut portal) => {
                if let Some(tag) = portal.frame.strip_prefix('#').map(str::to_owned) {
                    let Some(members) = block_tags.get(&tag).filter(|members| !members.is_empty()) else {
                        log::warn!("Horizon: portal type {id} skipped: frame tag #{tag} is missing or empty");
                        continue;
                    };
                    portal.frame_members = Some(members.iter().cloned().collect());
                }
                if out.types.len() >= u16::MAX as usize {
                    log::warn!("Horizon: portal type {id} skipped: too many portal types");
                    continue;
                }
                portal.slot = out.types.len() as u16;
                if registry.property_values(&portal.portal_block, "axis").len() < 2 {
                    log::warn!(
                        "Horizon: portal type {id} skipped: portal_block {} has no x/z axis property",
                        portal.portal_block
                    );
                    continue;
                }
                out.types.insert(id, portal);
            }
            Err(error) => log::warn!("Horizon: portal type {id} skipped: {error}"),
        }
    }
    out
}

fn parse(
    id: &str,
    bytes: &[u8],
    level_known: &impl Fn(&str) -> bool,
    block_known: &impl Fn(&str) -> bool,
) -> Result<PortalType, String> {
    let json: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|error| format!("invalid JSON: {error}"))?;
    let portal = PortalType::parse(id, &json)?;
    if !level_known(&portal.target) {
        return Err(format!("target level {} is not registered", portal.target));
    }
    if !portal.frame.starts_with('#') && !block_known(&portal.frame) {
        return Err(format!("frame block {} is not registered", portal.frame));
    }
    if !block_known(&portal.portal_block) {
        return Err(format!(
            "portal_block {} is not registered",
            portal.portal_block
        ));
    }
    Ok(portal)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &[u8] = br#"{"frame":"example:frame","ignite":{"fluid":"minecraft:lava"},
        "target":"example:moon","portal_block":"minecraft:nether_portal"}"#;

    #[test]
    fn an_unknown_target_warns_by_skipping_the_type() {
        let error = parse("example:portal", VALID, &|_| false, &|_| true).unwrap_err();
        assert_eq!(error, "target level example:moon is not registered");
    }

    #[test]
    fn unknown_blocks_are_skipped_but_tags_stay_data_defined() {
        assert!(parse("example:portal", VALID, &|_| true, &|block| block != "example:frame").is_err());
        let tagged = br##"{"frame":"#example:frames","ignite":{"fire":true},
            "target":"example:moon"}"##;
        assert!(parse("example:portal", tagged, &|_| true, &|_| true).is_ok());
    }
}
