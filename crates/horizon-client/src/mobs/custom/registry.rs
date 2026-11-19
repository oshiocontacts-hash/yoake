//! The custom mob type table. Empty unless a mod ships a `yoake:mob` type (F4M-3 fills it).
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
}

/// The process-wide table (client, integrated server and dedicated server share one process).
pub fn registry() -> &'static RwLock<CustomRegistry> {
    static REGISTRY: OnceLock<RwLock<CustomRegistry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registry_starts_empty_and_refuses_a_second_id() {
        assert!(registry().read().unwrap().is_empty());
        let file = horizon_mods::custom_mob::CustomMobFile::parse(
            "demo:thing",
            br#"{"template":"yoake:mob","size":{"width":1,"height":1}}"#,
        )
        .unwrap();
        let mut table = CustomRegistry::default();
        let kind = Arc::new(CustomType {
            id: "demo:thing".into(),
            file,
        });
        assert!(table.insert(kind.clone()));
        assert!(!table.insert(kind));
        assert_eq!(table.len(), 1);
        assert!(table.get("demo:thing").is_some());
        table.clear();
        assert!(table.is_empty());
    }
}
