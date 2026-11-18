//! How a handle type is reached: as a hook argument (any method of it that is hooked takes it as
//! `self`), or by an accessor path of fields from the `Game` root. This is reporting only: the glue
//! takes the receiver from the activation (`with_arg`); accessor-path resolution from `Game` is not
//! built yet (MX3V1 open problem 5), but the report says which types it would serve and through what.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde_json::Value;

use crate::doc::{ItemRef, Res, Universe};

fn type_ids(value: &Value, out: &mut Vec<u64>) {
    match value {
        Value::Object(map) => {
            if let Some(id) = map.get("resolved_path").and_then(|p| p.get("id")).and_then(Value::as_u64) {
                out.push(id);
            }
            for child in map.values() {
                type_ids(child, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                type_ids(item, out);
            }
        }
        _ => {}
    }
}

fn field_types(uni: &Universe, owner: ItemRef) -> Vec<(String, u64)> {
    let doc = uni.doc(owner);
    let Some(item) = doc.item(owner.id) else { return Vec::new() };
    let Some(definition) = item.pointer("/inner/struct") else { return Vec::new() };
    let ids: Vec<(String, u64)> = if let Some(fields) = definition.pointer("/kind/plain/fields").and_then(Value::as_array) {
        fields.iter().filter_map(Value::as_u64).map(|f| (doc.item_name(f).unwrap_or("?").to_string(), f)).collect()
    } else if let Some(fields) = definition.pointer("/kind/tuple").and_then(Value::as_array) {
        fields.iter().enumerate().filter_map(|(i, f)| f.as_u64().map(|f| (i.to_string(), f))).collect()
    } else {
        Vec::new()
    };
    ids
}

/// For every type in `targets`, the shortest chain of fields from the root struct `root_name`
/// of document `root_doc` whose type mentions it (up to `max_depth` structs deep).
pub fn game_paths(uni: &Universe, root_doc: usize, root_name: &str, targets: &BTreeSet<ItemRef>, max_depth: usize) -> BTreeMap<ItemRef, String> {
    let mut found = BTreeMap::new();
    let doc = &uni.docs[root_doc];
    let Some(index) = doc.v.get("index").and_then(Value::as_object) else { return found };
    let root = index
        .iter()
        .filter(|(_, item)| item.get("name").and_then(Value::as_str) == Some(root_name) && item.pointer("/inner/struct").is_some())
        .filter_map(|(id, _)| id.parse::<u64>().ok())
        .min();
    let Some(root) = root else { return found };
    let root_ref = ItemRef { doc: root_doc, id: root };
    let mut seen: BTreeSet<ItemRef> = BTreeSet::from([root_ref]);
    let mut queue: VecDeque<(ItemRef, String, usize)> = VecDeque::from([(root_ref, root_name.to_string(), 0)]);
    while let Some((owner, path, depth)) = queue.pop_front() {
        let owner_doc = uni.doc(owner);
        for (name, field) in field_types(uni, owner) {
            let Some(ty) = owner_doc.item(field).and_then(|f| f.pointer("/inner/struct_field")) else { continue };
            let mut ids = Vec::new();
            type_ids(ty, &mut ids);
            let field_path = format!("{path}.{name}");
            for id in ids {
                let Res::Local(child) = uni.resolve(owner.doc, id) else { continue };
                if targets.contains(&child) && !found.contains_key(&child) {
                    found.insert(child, field_path.clone());
                }
                let is_struct = uni.doc(child).kind(child.id) == Some("struct");
                if is_struct && depth + 1 < max_depth && seen.insert(child) {
                    queue.push_back((child, field_path.clone(), depth + 1));
                }
            }
        }
    }
    found
}
