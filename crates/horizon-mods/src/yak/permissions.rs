use super::config::{PermissionNode, has_permission, valid_name};
use super::manifest::identifier;
use serde::Deserialize;
use std::collections::BTreeMap;

const MAX_DOCUMENT_BYTES: usize = 1024 * 1024;
const MAX_MODS: usize = 256;
const MAX_PLAYERS: usize = 4096;
const MAX_RULES: usize = 65_536;

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
}

#[derive(Default)]
pub struct PermissionOverrides {
    mods: BTreeMap<String, BTreeMap<String, BTreeMap<String, bool>>>,
}

fn normalize_uuid(uuid: &str) -> Option<String> {
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
        for (mod_id, permissions) in parsed.mods {
            identifier(&mod_id)?;
            player_count = player_count.checked_add(permissions.players.len()).ok_or("too many players")?;
            if player_count > MAX_PLAYERS {
                return Err("too many players in permissions document".into());
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
            mods.insert(mod_id, players);
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
            .and_then(|players| players.get(&player_uuid))
            .and_then(|rules| rules.get(node))
            .copied();
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
}
