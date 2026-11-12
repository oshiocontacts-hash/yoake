use crate::{crafting::CustomRecipe, items::ItemStack, recipe_book::RecipeBookCategory};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug, PartialEq)]
pub struct NativeRecipeDisplay {
    pub inputs: Vec<Vec<ItemStack>>,
    pub outputs: Vec<ItemStack>,
}

pub trait NativeRecipeCodec: Send + Sync {
    fn decode(&self, json: &Value) -> Result<Value, String>;
    fn matches(&self, data: &Value, input: &[ItemStack]) -> bool;
    fn assemble(&self, data: &Value, input: &[ItemStack]) -> Vec<ItemStack>;
    fn display(&self, data: &Value) -> NativeRecipeDisplay;
}

pub struct NativeRecipeType {
    pub id: String,
    pub version: u32,
    pub category: RecipeBookCategory,
    pub codec: Arc<dyn NativeRecipeCodec>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeRecipeEntry {
    pub id: String,
    pub recipe_type: String,
    pub version: u32,
    pub category: RecipeBookCategory,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NativeRecipeMatch {
    pub recipe: NativeRecipeEntry,
    pub outputs: Vec<ItemStack>,
}

struct Registry(BTreeMap<String, NativeRecipeType>);
static TYPES: OnceLock<Registry> = OnceLock::new();

fn namespaced(id: &str) -> bool {
    id.split_once(':').is_some_and(|(namespace, path)| {
        !namespace.is_empty()
            && !path.is_empty()
            && namespace.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"_.-".contains(&byte)
            })
            && path.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"/_.-".contains(&byte)
            })
    })
}

impl Registry {
    fn new(types: Vec<NativeRecipeType>) -> Result<Self, String> {
        let mut registry = BTreeMap::new();
        let mut categories = std::collections::BTreeSet::new();
        for recipe_type in types {
            if !namespaced(&recipe_type.id)
                || recipe_type.id.starts_with("minecraft:")
                || recipe_type.version == 0
                || !namespaced(&recipe_type.category.id)
                || recipe_type.category.id.starts_with("minecraft:")
                || recipe_type.category.recipe_type != recipe_type.id
                || !namespaced(&recipe_type.category.icon)
                || !categories.insert(recipe_type.category.id.clone())
            {
                return Err(format!("invalid recipe type: {}", recipe_type.id));
            }
            let id = recipe_type.id.clone();
            if registry.insert(id.clone(), recipe_type).is_some() {
                return Err(format!("duplicate recipe type: {id}"));
            }
        }
        Ok(Self(registry))
    }

    fn decode(&self, kind: &str, json: &Value) -> Result<Option<Arc<Value>>, String> {
        let Some(recipe_type) = self.0.get(kind) else {
            return Ok(None);
        };
        if json["version"].as_u64() != Some(u64::from(recipe_type.version)) {
            return Err(format!(
                "{kind} requires recipe version {}",
                recipe_type.version
            ));
        }
        recipe_type.codec.decode(json).map(|data| Some(Arc::new(data)))
    }

    fn entry(recipe_type: &NativeRecipeType, recipe: &CustomRecipe) -> NativeRecipeEntry {
        NativeRecipeEntry {
            id: recipe.id.clone(),
            recipe_type: recipe_type.id.clone(),
            version: recipe_type.version,
            category: recipe_type.category.clone(),
        }
    }

    fn list(&self, kind: &str, recipes: &[CustomRecipe]) -> Vec<NativeRecipeEntry> {
        let Some(recipe_type) = self.0.get(kind) else {
            return Vec::new();
        };
        recipes
            .iter()
            .filter(|recipe| recipe.decoded.is_some())
            .map(|recipe| Self::entry(recipe_type, recipe))
            .collect()
    }

    fn matching(
        &self,
        kind: &str,
        input: &[ItemStack],
        recipes: &[CustomRecipe],
    ) -> Vec<NativeRecipeMatch> {
        let Some(recipe_type) = self.0.get(kind) else {
            return Vec::new();
        };
        recipes
            .iter()
            .filter_map(|recipe| {
                let data = recipe.decoded.as_deref()?;
                recipe_type
                    .codec
                    .matches(data, input)
                    .then(|| NativeRecipeMatch {
                        recipe: Self::entry(recipe_type, recipe),
                        outputs: recipe_type.codec.assemble(data, input),
                    })
            })
            .collect()
    }

    fn display(&self, kind: &str, recipe: &CustomRecipe) -> Option<NativeRecipeDisplay> {
        let recipe_type = self.0.get(kind)?;
        Some(recipe_type.codec.display(recipe.decoded.as_deref()?))
    }
}

pub fn install(types: Vec<NativeRecipeType>) -> io::Result<()> {
    if types.is_empty() {
        return Ok(());
    }
    let registry = Registry::new(types).map_err(io::Error::other)?;
    TYPES
        .set(registry)
        .map_err(|_| io::Error::other("recipe types already installed"))
}

pub fn registered() -> bool {
    TYPES.get().is_some()
}

pub fn decode(kind: &str, json: &Value) -> Result<Option<Arc<Value>>, String> {
    let Some(registry) = TYPES.get() else {
        return Ok(None);
    };
    registry.decode(kind, json)
}

pub fn append_sync_types(types: &mut Vec<String>) {
    if let Some(registry) = TYPES.get() {
        types.extend(registry.0.keys().cloned());
    }
}

pub fn append_categories(categories: &mut Vec<RecipeBookCategory>) {
    if let Some(registry) = TYPES.get() {
        categories.extend(
            registry
                .0
                .values()
                .map(|recipe_type| recipe_type.category.clone()),
        );
    }
}

pub fn recipe_list(kind: &str, recipes: &[CustomRecipe]) -> Vec<NativeRecipeEntry> {
    TYPES
        .get()
        .map_or_else(Vec::new, |registry| registry.list(kind, recipes))
}

pub fn recipes_matching(
    kind: &str,
    input: &[ItemStack],
    recipes: &[CustomRecipe],
) -> Vec<NativeRecipeMatch> {
    TYPES
        .get()
        .map_or_else(Vec::new, |registry| registry.matching(kind, input, recipes))
}

pub fn recipe_display(kind: &str, recipe: &CustomRecipe) -> Option<NativeRecipeDisplay> {
    TYPES.get()?.display(kind, recipe)
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_world::nbt::Tag;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Press(Arc<AtomicUsize>);

    fn stack(item: &str) -> ItemStack {
        ItemStack {
            item: Arc::from(item),
            count: 1,
            damage: 0,
            extra: None,
        }
    }

    impl NativeRecipeCodec for Press {
        fn decode(&self, json: &Value) -> Result<Value, String> {
            self.0.fetch_add(1, Ordering::Relaxed);
            json.get("ingredient")
                .and_then(Value::as_str)
                .ok_or_else(|| "ingredient missing".to_owned())?;
            Ok(json.clone())
        }

        fn matches(&self, data: &Value, input: &[ItemStack]) -> bool {
            self.0.fetch_add(1, Ordering::Relaxed);
            input.first().is_some_and(|stack| {
                stack.item.as_ref() == data["ingredient"].as_str().unwrap()
                    && stack.damage == 7
                    && matches!(
                        stack.data().and_then(|data| data.other("test:mode")),
                        Some(Tag::String(mode)) if mode.to_string_lossy() == "pressed"
                    )
            })
        }

        fn assemble(&self, _data: &Value, input: &[ItemStack]) -> Vec<ItemStack> {
            let mut result = input[0].clone();
            result.item = Arc::from("test:plate");
            vec![result]
        }

        fn display(&self, _data: &Value) -> NativeRecipeDisplay {
            NativeRecipeDisplay {
                inputs: vec![vec![stack("test:ingot")]],
                outputs: vec![stack("test:plate")],
            }
        }
    }

    fn registration(calls: &Arc<AtomicUsize>) -> NativeRecipeType {
        NativeRecipeType {
            id: "test:press".into(),
            version: 2,
            category: RecipeBookCategory {
                id: "test:pressing".into(),
                recipe_type: "test:press".into(),
                icon: "test:plate".into(),
            },
            codec: Arc::new(Press(calls.clone())),
        }
    }

    #[test]
    fn no_registration_leaves_json_untouched_and_does_no_codec_work() {
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = Registry::new(Vec::new()).unwrap();
        let json = serde_json::json!({"type":"test:press","version":2});
        assert_eq!(registry.decode("test:press", &json).unwrap(), None);
        assert!(registry.matching("test:press", &[], &[]).is_empty());
        assert!(registry.list("test:press", &[]).is_empty());
        assert_eq!(calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn versioned_recipe_matches_components_and_lists_after_existing_sync() {
        let calls = Arc::new(AtomicUsize::new(0));
        let registry = Registry::new(vec![registration(&calls)]).unwrap();
        let json = serde_json::json!({
            "type":"test:press", "version":2, "ingredient":"test:ingot"
        });
        assert!(
            registry
                .decode("test:press", &serde_json::json!({"version":1}))
                .is_err()
        );
        let payloads = crate::lan::recipes::encode_content(
            &["test:press".into()],
            &[("test:plate".into(), json)],
            false,
        );
        let mut recipes = Vec::new();
        for payload in payloads {
            let part: crate::lan::recipes::ContentPart = serde_json::from_slice(&payload).unwrap();
            for (id, json) in part.recipes {
                let decoded = registry.decode("test:press", &json).unwrap();
                recipes.push(CustomRecipe {
                    id,
                    json: Arc::new(json),
                    decoded,
                });
            }
        }
        let mut input = stack("test:ingot");
        assert!(
            registry
                .matching("test:press", &[input.clone()], &recipes)
                .is_empty()
        );
        input.damage = 7;
        input
            .data_mut()
            .set_loot_component("test:mode", &serde_json::json!("unpressed"));
        assert!(
            registry
                .matching("test:press", &[input.clone()], &recipes)
                .is_empty()
        );
        input
            .data_mut()
            .set_loot_component("test:mode", &serde_json::json!("pressed"));
        let matches = registry.matching("test:press", &[input.clone()], &recipes);
        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].outputs[0].extra, input.extra);
        assert_eq!(matches[0].outputs[0].damage, 7);
        assert_eq!(registry.list("test:press", &recipes)[0].version, 2);
        assert_eq!(
            registry.display("test:press", &recipes[0]).unwrap().outputs[0]
                .item
                .as_ref(),
            "test:plate"
        );
    }
}
