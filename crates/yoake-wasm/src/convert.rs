//! Conversions between Horizon's engine values and the WIT value types of `yoake-full.wit`
//! (`nbt`, `item-stack`, `text-json`, `json`).
//!
//! * `nbt` is `list<u8>` holding the network NBT encoding: a tag id byte, then the payload, no
//!   root name (`horizon_world::nbt::write_network` / `read_network`). An empty list reads as an
//!   empty compound, so a guest can leave `components` out.
//! * `item-stack` carries the id, the count and the stack's data components as an NBT compound,
//!   exactly the `components` compound Horizon writes to the network and to disk
//!   (`horizon_items::item_data::encode_components`); bundle and container contents stay inside.
//! * `text-json` is vanilla's text component JSON; `json` any JSON value as text.
//!
//! Everything read from a guest is bounded ([`GUEST_LIMITS`]) so a mod cannot allocate without end.
#![allow(dead_code)]

use horizon_client_core::{component, text::Text};
use horizon_items::{item_data, items::ItemStack};
use horizon_world::nbt::{self, Compound, Limits, Tag};
use std::sync::Arc;

/// What a guest's NBT may cost to decode: 4 MiB, 200 000 tags, 64 levels deep.
pub const GUEST_LIMITS: Limits = Limits { max_bytes: 4 << 20, max_nodes: 200_000, max_depth: 64 };

/// The host's limits for values it hands to a guest.
const HOST_LIMITS: Limits = Limits { max_bytes: 64 << 20, max_nodes: 4_000_000, max_depth: 512 };

/// A guest's item stack, with the `components` still encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WireItemStack {
    pub id: String,
    pub count: u32,
    pub components: Vec<u8>,
}

pub fn tag_to_nbt(tag: &Tag) -> Result<Vec<u8>, String> {
    nbt::write_network(tag, HOST_LIMITS).map_err(|e| e.to_string())
}

/// A tag from a guest's bytes; trailing bytes are an error.
pub fn nbt_to_tag(bytes: &[u8]) -> Result<Tag, String> {
    let (tag, used) = nbt::read_network(bytes, GUEST_LIMITS).map_err(|e| e.to_string())?;
    if used != bytes.len() {
        return Err(format!("NBT has {} trailing bytes", bytes.len() - used));
    }
    Ok(tag)
}

pub fn compound_to_nbt(compound: &Compound) -> Result<Vec<u8>, String> {
    tag_to_nbt(&Tag::Compound(compound.clone()))
}

/// A compound from a guest's bytes: empty bytes are the empty compound, any other tag an error.
pub fn nbt_to_compound(bytes: &[u8]) -> Result<Compound, String> {
    if bytes.is_empty() {
        return Ok(Compound::new());
    }
    match nbt_to_tag(bytes)? {
        Tag::Compound(compound) => Ok(compound),
        other => Err(format!("expected a compound tag, found tag id {}", other.id())),
    }
}

pub fn item_to_wire(stack: &ItemStack) -> Result<WireItemStack, String> {
    let components = match item_data::encode_components(stack) {
        Some(tag) => tag_to_nbt(&tag)?,
        None => Vec::new(),
    };
    Ok(WireItemStack { id: stack.item.to_string(), count: stack.count, components })
}

pub fn item_from_wire(wire: &WireItemStack) -> Result<ItemStack, String> {
    if wire.id.is_empty() {
        return Err("item id is empty".into());
    }
    let (damage, extra) = if wire.components.is_empty() {
        (0, None)
    } else {
        let tag = nbt_to_tag(&wire.components)?;
        if !matches!(tag, Tag::Compound(_)) {
            return Err("item components must be a compound".into());
        }
        item_data::decode_components(Some(&tag))
    };
    let mut stack = ItemStack { item: Arc::from(wire.id.as_str()), count: wire.count, damage, extra };
    stack.tidy();
    Ok(stack)
}

pub fn text_to_json(text: &Text) -> String {
    text.component().to_string()
}

/// Text from component JSON; `selector` components stay literal here.
pub fn text_from_json(json: &str) -> Result<Text, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(component::to_text(&value, &mut |selector| Text::literal(selector)))
}

pub fn value_to_json(value: &serde_json::Value) -> String {
    value.to_string()
}

pub fn value_from_json(json: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(json).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Tag {
        Tag::compound([
            ("name", Tag::String("a\u{e9}\u{1f600}".into())),
            ("n", Tag::Int(-7)),
            ("list", Tag::List(vec![Tag::Long(1), Tag::Long(2)])),
            ("nest", Tag::compound([("b", Tag::Byte(1)), ("arr", Tag::IntArray(vec![1, 2, 3]))])),
        ])
    }

    #[test]
    fn nbt_round_trips() {
        let bytes = tag_to_nbt(&sample()).unwrap();
        assert_eq!(bytes[0], 10);
        assert_eq!(nbt_to_tag(&bytes).unwrap(), sample());
        assert!(nbt_to_compound(&bytes).unwrap().contains_key(&"n".into()));
        assert!(nbt_to_compound(&[]).unwrap().is_empty());
        assert!(nbt_to_compound(&[3, 0, 0, 0, 1]).is_err());
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(nbt_to_tag(&trailing).is_err());
    }

    #[test]
    fn nbt_matches_the_guest_sdk_bytes() {
        // The same literals the guest SDK's tests pin: {"a": 1b}, a NUL string, an emoji string.
        let tag = Tag::compound([("a", Tag::Byte(1))]);
        assert_eq!(tag_to_nbt(&tag).unwrap(), [10, 1, 0, 1, b'a', 1, 0]);
        assert_eq!(tag_to_nbt(&Tag::String("\0".into())).unwrap(), [8, 0, 2, 0xC0, 0x80]);
        let emoji = [8, 0, 6, 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80];
        assert_eq!(tag_to_nbt(&Tag::String("\u{1F600}".into())).unwrap(), emoji);
        assert_eq!(nbt_to_tag(&emoji).unwrap(), Tag::String("\u{1F600}".into()));
    }

    #[test]
    fn nbt_from_guest_is_bounded() {
        // 100 nested compounds exceed the guest's depth limit.
        let mut bytes = Vec::new();
        for _ in 0..100 {
            bytes.extend([10, 0, 1, b'a']);
        }
        bytes.push(0);
        let mut deep = vec![10];
        deep.extend(&bytes);
        assert!(nbt_to_tag(&deep).is_err());
    }

    #[test]
    fn item_stack_round_trips_with_components() {
        let mut stack = ItemStack::new("minecraft:diamond_sword", 1);
        stack.damage = 12;
        stack.enchant("minecraft:sharpness", 5);
        let wire = item_to_wire(&stack).unwrap();
        assert_eq!(wire.id, "minecraft:diamond_sword");
        assert!(!wire.components.is_empty());
        let back = item_from_wire(&wire).unwrap();
        assert_eq!(back.item, stack.item);
        assert_eq!(back.damage, 12);
        assert_eq!(back.enchantment_level("minecraft:sharpness"), 5);
    }

    #[test]
    fn plain_item_has_no_components() {
        let stack = ItemStack::new("minecraft:stone", 64);
        let wire = item_to_wire(&stack).unwrap();
        assert!(wire.components.is_empty());
        let back = item_from_wire(&wire).unwrap();
        assert_eq!((back.count, back.damage), (64, 0));
        assert!(item_from_wire(&WireItemStack { id: String::new(), count: 1, components: Vec::new() }).is_err());
    }

    #[test]
    fn text_round_trips_through_json() {
        let text = Text::colored("hi", 0xFF5555);
        let json = text_to_json(&text);
        assert!(json.contains("hi"));
        let back = text_from_json(&json).unwrap();
        assert_eq!(back.spans, text.spans);
        assert!(text_from_json("{").is_err());
    }

    #[test]
    fn json_values_round_trip() {
        let value = serde_json::json!({"a": [1, 2, {"b": null}]});
        assert_eq!(value_from_json(&value_to_json(&value)).unwrap(), value);
    }
}
