//! The custom mob type table. Empty unless a mod ships a `yoake:mob` type: `build::add` fills it
//! from the content root when the mob types load.
use super::CustomType;
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock, RwLock},
};

/// Every registered custom type by id.
#[derive(Debug, Default)]
pub struct CustomRegistry {
    types: BTreeMap<String, Arc<CustomType>>,
}

impl CustomRegistry {
    pub fn is_empty(&self) -> bool {
        self.types.is_empty()
    }
    pub fn len(&self) -> usize {
        self.types.len()
    }
    pub fn get(&self, id: &str) -> Option<&Arc<CustomType>> {
        self.types.get(id)
    }
    pub fn iter(&self) -> impl Iterator<Item = &Arc<CustomType>> {
        self.types.values()
    }
    /// Adds a type; false when its id is taken (the first stays).
    pub fn insert(&mut self, kind: Arc<CustomType>) -> bool {
        if self.types.contains_key(&kind.id) {
            return false;
        }
        self.types.insert(kind.id.clone(), kind);
        true
    }
    pub fn clear(&mut self) {
        self.types.clear();
    }
    /// Replaces every type (a reload).
    pub fn replace(&mut self, kinds: impl IntoIterator<Item = Arc<CustomType>>) {
        self.clear();
        for kind in kinds {
            self.insert(kind);
        }
    }
}

/// The process-wide table (client, integrated server and dedicated server share one process).
pub fn registry() -> &'static RwLock<CustomRegistry> {
    static REGISTRY: OnceLock<RwLock<CustomRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// The custom type `id`, if there is one. Cheap for every vanilla id: the table is empty.
pub fn get(id: &str) -> Option<Arc<CustomType>> {
    registry().read().ok()?.get(id).cloned()
}

/// The ids of the custom types, sorted.
pub fn ids() -> Vec<String> {
    registry()
        .read()
        .map(|r| r.iter().map(|t| t.id.clone()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(id: &str) -> Arc<CustomType> {
        let file = horizon_mods::custom_mob::CustomMobFile::parse(
            id,
            br#"{"template":"yoake:mob","size":{"width":1,"height":1}}"#,
        )
        .unwrap();
        Arc::new(CustomType::new(file, None).unwrap())
    }

    /// A local table: the process-wide one is shared by every test of the process.
    #[test]
    fn the_registry_refuses_a_second_id_and_can_be_replaced() {
        let mut table = CustomRegistry::default();
        assert!(table.is_empty());
        let a = kind("demo:thing");
        assert!(table.insert(a.clone()));
        assert!(!table.insert(a));
        assert_eq!(table.len(), 1);
        assert!(table.get("demo:thing").is_some());
        table.replace([kind("demo:other"), kind("demo:third")]);
        assert!(table.get("demo:thing").is_none());
        assert_eq!(table.len(), 2);
        table.clear();
        assert!(table.is_empty());
    }

    fn kind_in(id: &str, category: &str) -> Arc<CustomType> {
        let json = format!(
            r#"{{"template":"yoake:mob","category":"{category}","size":{{"width":1,"height":1}}}}"#
        );
        let file = horizon_mods::custom_mob::CustomMobFile::parse(id, json.as_bytes()).unwrap();
        Arc::new(CustomType::new(file, None).unwrap())
    }

    /// A monster-category custom mob is refused on peaceful like a vanilla `Monster`; a creature
    /// is not; a vanilla id is unchanged.
    #[test]
    fn peaceful_follows_the_category() {
        use crate::mobs::{finalize, spawn_rules};
        let monster = kind_in("peacetest:monster", "monster");
        let creature = kind_in("peacetest:creature", "creature");
        assert!(registry().write().unwrap().insert(monster.clone()));
        assert!(registry().write().unwrap().insert(creature.clone()));
        assert!(!finalize::allowed_in_peaceful("peacetest:monster"));
        assert!(finalize::allowed_in_peaceful("peacetest:creature"));
        assert!(!spawn_rules::allowed_in_peaceful_type(&super::super::build::mob_type(&monster)));
        assert!(spawn_rules::allowed_in_peaceful_type(&super::super::build::mob_type(&creature)));
        assert!(!finalize::allowed_in_peaceful("minecraft:creeper"));
        assert!(finalize::allowed_in_peaceful("minecraft:cow"));
    }
}
