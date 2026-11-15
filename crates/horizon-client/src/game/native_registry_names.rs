use std::borrow::Cow;

use super::Game;

fn registry_translation_key(registry: &str, id: &str) -> Option<String> {
    let registry = registry.strip_prefix("minecraft:").unwrap_or(registry);
    let prefix = match registry {
        "item" => "item",
        "block" => "block",
        "entity_type" => "entity",
        "mob_effect" => "effect",
        _ => return None,
    };
    let (namespace, path) = id.split_once(':').unwrap_or(("minecraft", id));
    if namespace.is_empty() || path.is_empty() {
        return None;
    }
    let path = path.replace('/', ".");
    Some(format!("{prefix}.{namespace}.{path}"))
}

fn translated_item_name(id: &str, mut lookup: impl FnMut(&str) -> Option<String>) -> Option<String> {
    ["item", "block"].into_iter().find_map(|registry| {
        registry_translation_key(registry, id).and_then(|key| lookup(&key))
    })
}

fn translated_registry_name(
    registry: &str,
    id: &str,
    lookup: impl FnOnce(&str) -> Option<String>,
) -> String {
    registry_translation_key(registry, id)
        .and_then(|key| lookup(&key))
        .unwrap_or_else(|| id.to_owned())
}

fn owner_mod(owners: &super::mods::ModOwners, namespace: &str) -> Option<(String, String)> {
    if namespace == "minecraft" {
        return Some(("minecraft".to_owned(), "Minecraft".to_owned()));
    }
    owners.get(namespace).cloned()
}

#[allow(dead_code)]
impl Game<'_> {
    /// Returns the current-language display name for supported registries.
    /// Unknown registries and missing non-item translations return the ID.
    /// API @API:registry_names@.
    pub(super) fn native_display_name(&self, registry: &str, id: &str) -> String {
        let registry = registry.strip_prefix("minecraft:").unwrap_or(registry);
        if registry == "item" {
            if let Some(name) = horizon_client::lang::current().and_then(|language| {
                translated_item_name(id, |key| language.get(key).map(str::to_owned))
            }) {
                return name;
            }
            if let Some(name) =
                translated_item_name(id, |key| self.pack.names.get(key).map(str::to_owned))
            {
                return name;
            }
            return self.pack.names.display_item(id);
        }
        if registry == "enchantment" {
            let canonical_id = if id.contains(':') {
                Cow::Borrowed(id)
            } else {
                Cow::Owned(format!("minecraft:{id}"))
            };
            return self
                .items()
                .enchantments
                .get(canonical_id.as_ref())
                .map(|enchantment| {
                    self.pack
                        .names
                        .display_get(&enchantment.description_key)
                        .unwrap_or_else(|| enchantment.description.clone())
                })
                .unwrap_or_else(|| id.to_owned());
        }
        translated_registry_name(registry, id, |key| self.pack.names.display_get(key))
    }

    /// Returns the ID and manifest name of the active owner of a namespace.
    /// The vanilla namespace is owned by Minecraft; unknown namespaces return None.
    /// API @API:registry_names@.
    pub(super) fn native_owner_mod(&self, namespace: &str) -> Option<(String, String)> {
        owner_mod(&self.pack.mod_owners, namespace)
    }
}

#[cfg(test)]
mod tests {
    use super::{owner_mod, registry_translation_key, translated_item_name, translated_registry_name};
    use crate::game::mods::ModOwners;

    #[test]
    fn builds_keys_for_supported_registries_and_namespaces() {
        assert_eq!(
            registry_translation_key("minecraft:entity_type", "example:forest/deer"),
            Some("entity.example.forest.deer".to_owned())
        );
        assert_eq!(
            registry_translation_key("block", "stone"),
            Some("block.minecraft.stone".to_owned())
        );
        assert_eq!(
            registry_translation_key("block", "example:forest/log"),
            Some("block.example.forest.log".to_owned())
        );
        assert_eq!(
            registry_translation_key("mob_effect", "minecraft:speed"),
            Some("effect.minecraft.speed".to_owned())
        );
        assert_eq!(registry_translation_key("block", ":stone"), None);
    }

    #[test]
    fn item_names_prefer_item_then_block_translations() {
        let lookup = |key: &str| match key {
            "item.example.forest.log" => Some("Item".to_owned()),
            "block.example.forest.log" => Some("Block".to_owned()),
            _ => None,
        };
        assert_eq!(translated_item_name("example:forest/log", lookup), Some("Item".to_owned()));
        assert_eq!(
            translated_item_name("example:forest/log", |key| {
                (key == "block.example.forest.log").then(|| "Block".to_owned())
            }),
            Some("Block".to_owned())
        );
        assert_eq!(translated_item_name(":missing", |_| unreachable!()), None);
    }

    #[test]
    fn returns_translation_or_original_id_when_unavailable() {
        assert_eq!(
            translated_registry_name("block", "minecraft:stone", |key| {
                (key == "block.minecraft.stone").then(|| "Pierre".to_owned())
            }),
            "Pierre"
        );
        assert_eq!(
            translated_registry_name("block", "example:missing", |_| None),
            "example:missing"
        );
        assert_eq!(
            translated_registry_name("unknown", "example:thing", |_| unreachable!()),
            "example:thing"
        );
    }

    #[test]
    fn resolves_vanilla_content_and_unknown_owners() {
        let mut owners = ModOwners::new();
        owners.insert(
            "alpine_blocks".to_owned(),
            ("alpine".to_owned(), "Alpine".to_owned()),
        );
        assert_eq!(
            owner_mod(&owners, "minecraft"),
            Some(("minecraft".to_owned(), "Minecraft".to_owned()))
        );
        assert_eq!(
            owner_mod(&owners, "alpine_blocks"),
            Some(("alpine".to_owned(), "Alpine".to_owned()))
        );
        assert_eq!(owner_mod(&owners, "unknown"), None);
    }
}
