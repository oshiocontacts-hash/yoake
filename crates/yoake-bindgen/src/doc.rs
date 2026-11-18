//! Loading rustdoc JSON and resolving item ids across crates.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;

use serde_json::Value;

pub type R<T> = Result<T, Box<dyn Error>>;

/// One crate's rustdoc JSON (format 61, `--document-private-items`).
pub struct Doc {
    /// The crate's name (the root module's name).
    pub name: String,
    pub v: Value,
    pub root: u64,
    /// Canonical path (`a::b::C`) of every local item that has a `paths` entry; types win a clash.
    canon: HashMap<String, u64>,
    /// Item -> the module whose `items` lists it (impls included).
    parent: HashMap<u64, u64>,
    ext_names: HashMap<u64, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ItemRef {
    pub doc: usize,
    pub id: u64,
}

/// What an id in a type resolves to.
#[derive(Clone, Debug)]
pub enum Res {
    /// An item of a loaded crate.
    Local(ItemRef),
    /// `std`, `core` or `alloc`: the canonical path.
    Std(Vec<String>),
    /// An item of a crate whose rustdoc JSON was not loaded (or not found in it).
    Missing(Vec<String>),
}

const TYPE_KINDS: [&str; 5] = ["struct", "enum", "type_alias", "union", "trait"];

impl Doc {
    pub fn parse(text: &str) -> R<Doc> {
        let v: Value = serde_json::from_str(text)?;
        Self::from_value(v)
    }

    pub fn from_value(v: Value) -> R<Doc> {
        let version = v.get("format_version").and_then(Value::as_u64).ok_or("rustdoc JSON has no format_version")?;
        if version != crate::FORMAT_VERSION {
            return Err(format!("rustdoc JSON format_version {version}, this generator reads {}", crate::FORMAT_VERSION).into());
        }
        let root = v.get("root").and_then(Value::as_u64).ok_or("rustdoc JSON has no root")?;
        let index = v.get("index").and_then(Value::as_object).ok_or("rustdoc JSON has no index")?;
        let name = index
            .get(&root.to_string())
            .and_then(|item| item.get("name"))
            .and_then(Value::as_str)
            .ok_or("root module has no name")?
            .to_string();
        let mut parent = HashMap::new();
        for (id, item) in index {
            if let Some(items) = item.pointer("/inner/module/items").and_then(Value::as_array) {
                let id: u64 = id.parse()?;
                for child in items.iter().filter_map(Value::as_u64) {
                    parent.insert(child, id);
                }
            }
        }
        let mut canon: HashMap<String, u64> = HashMap::new();
        if let Some(paths) = v.get("paths").and_then(Value::as_object) {
            for (id, entry) in paths {
                if entry.get("crate_id").and_then(Value::as_u64) != Some(0) {
                    continue;
                }
                let Some(path) = path_of(entry) else { continue };
                let id: u64 = id.parse()?;
                let key = path.join("::");
                let is_type = entry.get("kind").and_then(Value::as_str).is_some_and(|kind| TYPE_KINDS.contains(&kind));
                if is_type || !canon.contains_key(&key) {
                    canon.insert(key, id);
                }
            }
        }
        let mut ext_names = HashMap::new();
        if let Some(crates) = v.get("external_crates").and_then(Value::as_object) {
            for (id, krate) in crates {
                if let (Ok(id), Some(name)) = (id.parse::<u64>(), krate.get("name").and_then(Value::as_str)) {
                    ext_names.insert(id, name.to_string());
                }
            }
        }
        Ok(Doc { name, v, root, canon, parent, ext_names })
    }

    pub fn item(&self, id: u64) -> Option<&Value> {
        self.v.pointer(&format!("/index/{id}"))
    }

    pub fn paths_entry(&self, id: u64) -> Option<&Value> {
        self.v.pointer(&format!("/paths/{id}"))
    }

    pub fn parent_module(&self, id: u64) -> Option<u64> {
        self.parent.get(&id).copied()
    }

    pub fn item_name(&self, id: u64) -> Option<&str> {
        self.item(id)?.get("name")?.as_str()
    }

    /// `crate_name::a::b` of a module item.
    pub fn module_path(&self, module: u64) -> Vec<String> {
        let mut segments = Vec::new();
        let mut current = module;
        loop {
            if current == self.root {
                segments.push(self.name.clone());
                break;
            }
            segments.push(self.item_name(current).unwrap_or("?").to_string());
            match self.parent.get(&current) {
                Some(next) => current = *next,
                None => {
                    segments.push(self.name.clone());
                    break;
                }
            }
        }
        segments.reverse();
        segments
    }

    /// The canonical path of an item: its `paths` entry, else its module path plus its name.
    pub fn item_path(&self, id: u64) -> Vec<String> {
        if let Some(path) = self.paths_entry(id).and_then(path_of) {
            if self.paths_entry(id).and_then(|entry| entry.get("crate_id")).and_then(Value::as_u64) == Some(0) {
                return path;
            }
        }
        let mut path = match self.parent.get(&id) {
            Some(parent) => self.module_path(*parent),
            None => vec![self.name.clone()],
        };
        path.push(self.item_name(id).unwrap_or("?").to_string());
        path
    }

    /// The module that contains an item (for a struct: the module it is defined in).
    pub fn defining_module(&self, id: u64) -> Vec<String> {
        match self.parent.get(&id) {
            Some(module) => self.module_path(*module),
            None => {
                let mut path = self.item_path(id);
                path.pop();
                path
            }
        }
    }

    pub fn kind(&self, id: u64) -> Option<&str> {
        self.item(id)?.get("inner")?.as_object()?.keys().next().map(String::as_str)
    }

    pub fn canon(&self, path: &str) -> Option<u64> {
        self.canon.get(path).copied()
    }

    pub fn ext_name(&self, crate_id: u64) -> Option<&str> {
        self.ext_names.get(&crate_id).map(String::as_str)
    }
}

fn path_of(entry: &Value) -> Option<Vec<String>> {
    Some(entry.get("path")?.as_array()?.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
}

/// Every loaded crate.
pub struct Universe {
    pub docs: Vec<Doc>,
}

impl Universe {
    pub fn doc_index(&self, crate_name: &str) -> Option<usize> {
        self.docs.iter().position(|doc| doc.name == crate_name)
    }

    pub fn doc(&self, r: ItemRef) -> &Doc {
        &self.docs[r.doc]
    }

    pub fn item(&self, r: ItemRef) -> Option<&Value> {
        self.docs[r.doc].item(r.id)
    }

    /// Resolves an id that appears in a type of document `doc`.
    pub fn resolve(&self, doc: usize, id: u64) -> Res {
        let d = &self.docs[doc];
        if d.item(id).is_some() {
            return Res::Local(ItemRef { doc, id });
        }
        let Some(entry) = d.paths_entry(id) else {
            return Res::Missing(vec![format!("?{id}")]);
        };
        let path = path_of(entry).unwrap_or_default();
        let crate_id = entry.get("crate_id").and_then(Value::as_u64).unwrap_or(0);
        if crate_id == 0 {
            return Res::Missing(path);
        }
        let crate_name = d.ext_name(crate_id).unwrap_or("").to_string();
        if matches!(crate_name.as_str(), "std" | "core" | "alloc") {
            return Res::Std(path);
        }
        if let Some(other) = self.doc_index(&crate_name) {
            if let Some(found) = self.docs[other].canon(&path.join("::")) {
                return Res::Local(ItemRef { doc: other, id: found });
            }
            // A re-export path: search the loaded crate for a unique type of that name.
            if let Some(leaf) = path.last() {
                let mut matches = self.docs[other].canon.iter().filter(|(key, _)| key.rsplit("::").next() == Some(leaf.as_str()));
                if let (Some((_, id)), None) = (matches.next(), matches.next()) {
                    return Res::Local(ItemRef { doc: other, id: *id });
                }
            }
        }
        Res::Missing(path)
    }
}

/// Orders a map's keys for deterministic output.
pub fn sorted<K: Ord + Clone, V>(map: &HashMap<K, V>) -> BTreeMap<K, &V> {
    map.iter().map(|(k, v)| (k.clone(), v)).collect()
}
