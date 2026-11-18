use super::config::{PermissionNode, has_permission, valid_name};
use super::manifest::identifier;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::Path;

const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_MODS: usize = 256;
const MAX_PLAYERS: usize = 4096;
const MAX_RULES: usize = 65_536;
const MAX_GROUPS: usize = 4096;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PermissionDocument {
    #[serde(default)]
    mods: BTreeMap<String, ModPermissions>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ModPermissions {
    #[serde(default)]
    players: BTreeMap<String, BTreeMap<String, bool>>,
    #[serde(default)]
    groups: BTreeMap<String, BTreeMap<String, bool>>,
    #[serde(default)]
    members: BTreeMap<String, Vec<String>>,
}

#[derive(Default)]
struct ModOverrides {
    players: BTreeMap<String, BTreeMap<String, bool>>,
    groups: BTreeMap<String, BTreeMap<String, bool>>,
    members: BTreeMap<String, Vec<String>>,
}

#[derive(Default)]
pub struct PermissionOverrides {
    mods: BTreeMap<String, ModOverrides>,
}

pub fn normalize_uuid(uuid: &str) -> Option<String> {
    let bytes = uuid.as_bytes();
    if bytes.len() != 32 && bytes.len() != 36 {
        return None;
    }
    if bytes.len() == 36 && [8, 13, 18, 23].iter().any(|index| bytes[*index] != b'-') {
        return None;
    }
    let mut normalized = String::with_capacity(32);
    for (index, byte) in bytes.iter().copied().enumerate() {
        if bytes.len() == 36 && [8, 13, 18, 23].contains(&index) {
            continue;
        }
        if !byte.is_ascii_hexdigit() {
            return None;
        }
        normalized.push(char::from(byte.to_ascii_lowercase()));
    }
    Some(normalized)
}

impl PermissionOverrides {
    pub fn load_world(world_dir: &Path) -> Result<Self, String> {
        let path = world_dir.join("permissions.toml");
        let root = world_dir
            .canonicalize()
            .map_err(|error| error.to_string())?;
        let directory = super::fs::Dir::open(&root, false).map_err(|error| error.to_string())?;
        let Some(file) = directory
            .read(std::ffi::OsStr::new("permissions.toml"))
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?
        else {
            return Ok(Self::default());
        };
        let mut bytes = Vec::new();
        file.take((MAX_DOCUMENT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(format!("{} exceeds the permissions size limit", path.display()));
        }
        let document = std::str::from_utf8(&bytes)
            .map_err(|error| format!("invalid UTF-8 in {}: {error}", path.display()))?;
        Self::parse(document).map_err(|error| format!("invalid {}: {error}", path.display()))
    }

    pub fn parse(document: &str) -> Result<Self, String> {
        if document.len() > MAX_DOCUMENT_BYTES {
            return Err("permissions document is too large".into());
        }
        let parsed: PermissionDocument = toml::from_str(document).map_err(|error| error.to_string())?;
        if parsed.mods.len() > MAX_MODS {
            return Err("too many mods in permissions document".into());
        }
        let mut mods = BTreeMap::new();
        let mut player_count = 0usize;
        let mut rule_count = 0usize;
        let mut group_count = 0usize;
        for (mod_id, permissions) in parsed.mods {
            identifier(&mod_id)?;
            player_count = player_count.checked_add(permissions.players.len())
                .and_then(|count| count.checked_add(permissions.members.len()))
                .ok_or("too many players")?;
            if player_count > MAX_PLAYERS {
                return Err("too many players in permissions document".into());
            }
            group_count = group_count.checked_add(permissions.groups.len()).ok_or("too many groups")?;
            if group_count > MAX_GROUPS {
                return Err("too many groups in permissions document".into());
            }
            let mut players = BTreeMap::new();
            for (player, rules) in permissions.players {
                let player = normalize_uuid(&player).ok_or("invalid player UUID")?;
                rule_count = rule_count.checked_add(rules.len()).ok_or("too many rules")?;
                if rule_count > MAX_RULES || rules.keys().any(|node| !valid_name(node)) {
                    return Err("invalid or excessive permission rules".into());
                }
                if players.insert(player, rules).is_some() {
                    return Err("duplicate player UUID".into());
                }
            }
            for (group, rules) in &permissions.groups {
                rule_count = rule_count.checked_add(rules.len()).ok_or("too many rules")?;
                if !valid_name(group) || rule_count > MAX_RULES
                    || rules.keys().any(|node| !valid_name(node)) {
                    return Err("invalid or excessive permission group rules".into());
                }
            }
            let mut members = BTreeMap::new();
            for (player, groups) in permissions.members {
                let player = normalize_uuid(&player).ok_or("invalid player UUID")?;
                rule_count = rule_count.checked_add(groups.len()).ok_or("too many rules")?;
                let mut seen = BTreeSet::new();
                if rule_count > MAX_RULES || groups.iter().any(|group|
                    !permissions.groups.contains_key(group) || !seen.insert(group)) {
                    return Err("invalid or excessive permission group memberships".into());
                }
                if members.insert(player, groups).is_some() {
                    return Err("duplicate player UUID".into());
                }
            }
            mods.insert(mod_id, ModOverrides { players, groups: permissions.groups, members });
        }
        Ok(Self { mods })
    }

    pub fn check(
        &self,
        mod_id: &str,
        player_uuid: &str,
        declarations: &BTreeMap<String, PermissionNode>,
        node: &str,
        op_level: u8,
    ) -> bool {
        if identifier(mod_id).is_err() {
            return false;
        }
        let Some(player_uuid) = normalize_uuid(player_uuid) else {
            return false;
        };
        let explicit = self.mods.get(mod_id)
            .and_then(|permissions| permissions.players.get(&player_uuid))
            .and_then(|rules| rules.get(node))
            .copied()
            .or_else(|| {
                let permissions = self.mods.get(mod_id)?;
                permissions.members.get(&player_uuid)?.iter()
                    .filter_map(|group| permissions.groups.get(group)?.get(node).copied())
                    .reduce(|left, right| left && right)
            });
        has_permission(declarations, node, op_level, explicit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overrides_are_per_mod_and_uuid_and_never_grant_undeclared_nodes() {
        let document = r#"
[mods.example.players.'01234567-89ab-cdef-0123-456789abcdef']
'example.edit' = true
'example.admin' = false
'example.undeclared' = true
"#;
        let permissions = PermissionOverrides::parse(document).unwrap();
        let declarations = BTreeMap::from([
            ("example.edit".to_string(), PermissionNode { default_op_level: 2 }),
            ("example.admin".to_string(), PermissionNode { default_op_level: 2 }),
        ]);
        let player = "0123456789ABCDEF0123456789ABCDEF";
        assert!(permissions.check("example", player, &declarations, "example.edit", 0));
        assert!(!permissions.check("example", player, &declarations, "example.admin", 4));
        assert!(!permissions.check("example", player, &declarations, "example.undeclared", 4));
        assert!(!permissions.check("another", player, &declarations, "example.edit", 0));
        assert!(permissions.check("another", player, &declarations, "example.edit", 2));
        assert!(!permissions.check("example", player, &declarations, "example.edit", 5));
        assert!(!permissions.check("example", "bad-uuid", &declarations, "example.edit", 4));
    }

    #[test]
    fn rejects_invalid_or_ambiguous_permission_documents() {
        assert!(PermissionOverrides::parse("").is_ok());
        assert!(PermissionOverrides::parse(&"x".repeat(MAX_DOCUMENT_BYTES + 1)).is_err());
        assert!(PermissionOverrides::parse("[mods.'../bad']").is_err());
        assert!(PermissionOverrides::parse("[mods.example.players.invalid]").is_err());
        assert!(PermissionOverrides::parse("[mods.example.players.'0123456789abcdef0123456789abcdef']\n'bad..node' = true").is_err());
        assert!(PermissionOverrides::parse(r#"
[mods.example.players.'0123456789abcdef0123456789abcdef']
'example.edit' = true
[mods.example.players.'01234567-89AB-CDEF-0123-456789ABCDEF']
'example.edit' = false
"#).is_err());
    }

    #[test]
    fn groups_deny_before_allow_and_player_rules_take_precedence() {
        let permissions = PermissionOverrides::parse(r#"
[mods.example.groups.builders]
'example.edit' = true
'example.admin' = true
[mods.example.groups.restricted]
'example.edit' = false
'example.admin' = false
[mods.example.members]
'01234567-89ab-cdef-0123-456789abcdef' = ['builders', 'restricted']
'11111111-1111-1111-1111-111111111111' = ['builders']
[mods.example.players.'0123456789abcdef0123456789abcdef']
'example.edit' = true
"#).unwrap();
        let declarations = BTreeMap::from([
            ("example.edit".to_string(), PermissionNode { default_op_level: 2 }),
            ("example.admin".to_string(), PermissionNode { default_op_level: 2 }),
        ]);
        let player = "0123456789abcdef0123456789abcdef";
        assert!(permissions.check("example", player, &declarations, "example.edit", 0));
        assert!(!permissions.check("example", player, &declarations, "example.admin", 4));
        assert!(permissions.check("example", "11111111111111111111111111111111", &declarations, "example.admin", 0));
        assert!(PermissionOverrides::parse("[mods.example.members]\n'0123456789abcdef0123456789abcdef' = ['missing']").is_err());
        assert!(PermissionOverrides::parse("[mods.example.groups.builders]\n[mods.example.members]\n'0123456789abcdef0123456789abcdef' = ['builders', 'builders']").is_err());
    }

    #[test]
    fn world_permissions_load_without_creating_or_replacing_files() {
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap().join(format!(
            "yoake-permissions-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("permissions.toml");
        let declarations = BTreeMap::from([(
            "example.edit".to_string(),
            PermissionNode { default_op_level: 2 },
        )]);
        let player = "0123456789abcdef0123456789abcdef";
        let empty = PermissionOverrides::load_world(&root).unwrap();
        assert!(!empty.check("example", player, &declarations, "example.edit", 0));
        assert!(!path.exists());

        let valid = "[mods.example.players.'0123456789abcdef0123456789abcdef']\n'example.edit' = true\n";
        std::fs::write(&path, valid).unwrap();
        let loaded = PermissionOverrides::load_world(&root).unwrap();
        assert!(loaded.check("example", player, &declarations, "example.edit", 0));

        let malformed = "[mods.example";
        std::fs::write(&path, malformed).unwrap();
        assert!(PermissionOverrides::load_world(&root).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
        crate::test_support::remove_dir_all(root);
    }
}
