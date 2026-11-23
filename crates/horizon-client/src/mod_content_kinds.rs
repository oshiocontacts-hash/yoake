//! The registry-entry content kinds of a content mod (REM-16): `enchantment`, `damage_type`,
//! `game_rule` and `fluid` (`data/<id>/horizon/<kind>/<path>.json`, parsed by
//! `horizon_mods::content_kinds`).
//!
//! Enchantments and damage types are data-pack registries: the mod's JSON goes into the content
//! root as `data/<ns>/enchantment/<path>.json` and `data/<ns>/damage_type/<path>.json`, which the
//! loaders, the registry sync and the commands read besides vanilla's, and a damage type joins
//! the damage catalog (`damage-catalog.json`) the health code reads. Game rules and fluids are
//! registered in the process by the caller (`game/mods.rs`).
use super::{ContentMod, J, Overlay, Result, read_json};
use horizon_mods::content_kinds::{DamageType, Enchantment, Fluid, GameRule};
use std::path::Path;

fn sorted<'a, T>(mods: &'a [ContentMod], of: impl Fn(&'a ContentMod) -> &'a [T], id: impl Fn(&T) -> &str) -> Vec<&'a T> {
    let mut all: Vec<&T> = mods.iter().flat_map(|m| of(m).iter()).collect();
    all.sort_by(|a, b| id(a).cmp(id(b)));
    all
}

/// The mods' enchantments in registry order (by id).
pub fn enchantments(mods: &[ContentMod]) -> Vec<&Enchantment> {
    sorted(mods, |m| m.content.enchantments.as_slice(), |e| &e.id)
}
/// The mods' damage types in registry order (by id).
pub fn damage_types(mods: &[ContentMod]) -> Vec<&DamageType> {
    sorted(mods, |m| m.content.damage_types.as_slice(), |d| &d.id)
}
/// The mods' game rules in registry order (by id).
pub fn game_rules(mods: &[ContentMod]) -> Vec<&GameRule> {
    sorted(mods, |m| m.content.game_rules.as_slice(), |g| &g.id)
}
/// The mods' fluid families in registry order (by id).
pub fn fluids(mods: &[ContentMod]) -> Vec<&Fluid> {
    sorted(mods, |m| m.content.fluids.as_slice(), |f| &f.id)
}

/// The content-root file of a data-pack entry `ns:path` of `kind`.
pub fn data_file(id: &str, kind: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("data/{ns}/{kind}/{path}.json")
}

/// Writes the enchantments' and damage types' files, and the damage types into the catalog.
pub(super) fn write(base: &Path, overlay: &Overlay<'_>, mods: &[ContentMod]) -> Result<()> {
    for e in enchantments(mods) {
        overlay.write(&data_file(&e.id, "enchantment"), &serde_json::to_vec(&e.json)?)?;
    }
    let types = damage_types(mods);
    for d in &types {
        overlay.write(&data_file(&d.id, "damage_type"), &serde_json::to_vec(&d.json)?)?;
    }
    if !types.is_empty() {
        let name = "damage-catalog.json";
        if base.join(name).exists() {
            let mut doc = read_json(&base.join(name))?;
            add_damage_types(&mut doc, &types).map_err(|e| format!("{name}: {e}"))?;
            overlay.write(name, &serde_json::to_vec(&doc)?)?;
        }
    }
    Ok(())
}

/// Appends the types to the damage catalog's `types` (the shape `damage_source::Catalog` reads:
/// `exhaustion` as the eight hex digits of its `f32` bits).
fn add_damage_types(doc: &mut J, types: &[&DamageType]) -> std::result::Result<(), String> {
    let Some(J::Arr(list)) = doc.at("types") else {
        return Err("no types".into());
    };
    for d in types {
        if list.iter().any(|t| t.get("id") == Some(&J::Str(d.id.clone()))) {
            return Err(format!("{} is already a damage type", d.id));
        }
        list.push(J::Obj(vec![
            ("id".into(), J::Str(d.id.clone())),
            ("message_id".into(), J::Str(d.message_id.clone())),
            ("exhaustion".into(), J::Str(format!("{:08x}", d.exhaustion.to_bits()))),
            ("scaling".into(), J::Str(d.scaling.name().into())),
            ("effects".into(), J::Str(d.effects.name().into())),
            ("death_message_type".into(), J::Str(d.death_message_type.name().into())),
        ]));
    }
    Ok(())
}

/// The commands' id lists (`command-registries.json`): the enchantments and damage types the
/// mods bring, for `/enchant`, `/damage` and the resource arguments.
pub(super) fn add_command_ids(doc: &mut J, mods: &[ContentMod]) {
    let enchantments: Vec<J> = enchantments(mods).iter().map(|e| J::Str(e.id.clone())).collect();
    let types: Vec<J> = damage_types(mods).iter().map(|d| J::Str(d.id.clone())).collect();
    for (paths, new) in [
        (["enchantments", "by_registry/minecraft:enchantment/ids"], &enchantments),
        (["damage_types", "by_registry/minecraft:damage_type/ids"], &types),
    ] {
        for path in paths {
            if let Some(J::Arr(ids)) = doc.at(path) {
                for id in new {
                    if !ids.contains(id) {
                        ids.push(id.clone());
                    }
                }
            }
        }
    }
}

/// Where a reader of a resource root finds the data-pack folders of a kind: `data/minecraft/`
/// first, then each content mod's namespace, which exist only in a root built here.
pub fn namespace_dirs(root: &Path, kind: &str) -> Vec<std::path::PathBuf> {
    std::iter::once("minecraft".to_owned())
        .chain(horizon_items::content_namespaces::namespaces(root))
        .map(|ns| root.join("data").join(ns).join(kind))
        .collect()
}

/// The game rules as the process registry takes them.
pub fn rule_declarations(mods: &[ContentMod]) -> Vec<horizon_client_core::gamerules::ModRule> {
    use horizon_client_core::gamerules::{ModRule, Value};
    use horizon_mods::content_kinds::GameRuleValue;
    game_rules(mods)
        .into_iter()
        .map(|g| ModRule {
            name: g.id.clone(),
            default: match g.default {
                GameRuleValue::Bool(b) => Value::Bool(b),
                GameRuleValue::Int(i) => Value::Int(i),
            },
            min: g.min,
            max: g.max,
            category: g.category.clone(),
        })
        .collect()
}

/// A fluid family as the world's fluid registry takes it.
pub fn fluid_definitions(mods: &[ContentMod]) -> Vec<horizon_world::fluid::custom::FluidDefinition> {
    use horizon_mods::content_kinds::FluidMotion as M;
    use horizon_world::fluid::custom::{FluidDefinition, FluidMotion};
    fluids(mods)
        .into_iter()
        .map(|f| FluidDefinition {
            id: f.id.clone(),
            flowing_id: f.flowing_id.clone(),
            still_block: f.still_block.clone(),
            flowing_block: f.flowing_block.clone(),
            bucket: f.bucket.clone(),
            still_texture: f.still_texture.clone(),
            flowing_texture: f.flowing_texture.clone(),
            tint: f.tint,
            viscosity: f.viscosity,
            tick_rate: f.tick_rate,
            light: f.light,
            can_drown: f.can_drown,
            motion: match f.motion {
                M::Water => FluidMotion::Water,
                M::Lava => FluidMotion::Lava,
                M::None => FluidMotion::None,
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_files_follow_the_data_pack_layout() {
        assert_eq!(data_file("ruby:edge", "enchantment"), "data/ruby/enchantment/edge.json");
        assert_eq!(data_file("ruby:a/b", "damage_type"), "data/ruby/damage_type/a/b.json");
    }

    #[test]
    fn damage_types_join_the_catalog_with_their_float_bits() {
        let mut doc: J = serde_json::from_str(r#"{"data_version": 1, "types": [{"id": "minecraft:arrow"}]}"#).unwrap();
        let d = DamageType::parse(
            "ruby:shatter",
            br#"{"message_id": "shatter", "scaling": "always", "exhaustion": 0.1, "effects": "freezing", "death_message_type": "fall_variants"}"#,
        )
        .unwrap();
        add_damage_types(&mut doc, &[&d]).unwrap();
        let v: serde_json::Value = serde_json::to_value(&doc).unwrap();
        // vanilla's own 0.1F is 0x3dcccccd in its catalog.
        assert_eq!(
            v["types"][1],
            serde_json::json!({"id": "ruby:shatter", "message_id": "shatter", "exhaustion": "3dcccccd",
                "scaling": "always", "effects": "freezing", "death_message_type": "fall_variants"})
        );
        assert_eq!(v["types"][0], serde_json::json!({"id": "minecraft:arrow"}));
        assert!(add_damage_types(&mut doc, &[&d]).unwrap_err().contains("already"));
        assert!(add_damage_types(&mut J::Null, &[&d]).is_err());
    }

    #[test]
    fn command_lists_gain_the_mods_ids_once() {
        let mut doc: J = serde_json::from_str(
            r#"{"enchantments": ["minecraft:sharpness"], "damage_types": ["minecraft:arrow"],
            "by_registry": {"minecraft:enchantment": {"ids": ["minecraft:sharpness"]},
            "minecraft:damage_type": {"ids": ["minecraft:arrow"]}}}"#,
        )
        .unwrap();
        add_command_ids(&mut doc, &[]);
        let before = serde_json::to_value(&doc).unwrap();
        assert_eq!(before["enchantments"], serde_json::json!(["minecraft:sharpness"]));
        // a mod's ids come after vanilla's, in every list the commands read.
        let dir = std::env::temp_dir().join(format!("horizon-kinds-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("data/ruby/horizon/damage_type")).unwrap();
        std::fs::create_dir_all(dir.join("data/ruby/horizon/enchantment")).unwrap();
        std::fs::write(dir.join("horizon-mod.toml"), "id = \"ruby\"\nname = \"Ruby\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n").unwrap();
        std::fs::write(dir.join("data/ruby/horizon/damage_type/shatter.json"), br#"{"message_id": "s", "scaling": "never"}"#).unwrap();
        std::fs::write(
            dir.join("data/ruby/horizon/enchantment/keen.json"),
            br#"{"description": "Keen", "supported_items": "minecraft:stick", "weight": 1, "max_level": 1,
            "min_cost": {"base": 1, "per_level_above_first": 0}, "max_cost": {"base": 2, "per_level_above_first": 0},
            "anvil_cost": 1, "slots": ["any"]}"#,
        )
        .unwrap();
        let package = horizon_mods::Package::read(&dir).unwrap();
        let mods = [ContentMod {
            id: "ruby".into(),
            content: horizon_mods::Content::read(&dir, &package.manifest).unwrap(),
        }];
        for _ in 0..2 {
            add_command_ids(&mut doc, &mods);
        }
        let v = serde_json::to_value(&doc).unwrap();
        assert_eq!(v["enchantments"], serde_json::json!(["minecraft:sharpness", "ruby:keen"]));
        assert_eq!(v["by_registry"]["minecraft:enchantment"]["ids"], v["enchantments"]);
        assert_eq!(v["damage_types"], serde_json::json!(["minecraft:arrow", "ruby:shatter"]));
        assert_eq!(v["by_registry"]["minecraft:damage_type"]["ids"], v["damage_types"]);
        crate::test_support::remove_dir_all(dir);
    }

    #[test]
    fn namespace_dirs_are_vanillas_alone_without_a_content_root() {
        let none = std::env::temp_dir().join("horizon-kinds-no-root");
        let dirs = namespace_dirs(&none, "enchantment");
        assert_eq!(dirs, [none.join("data/minecraft/enchantment")]);
    }
}
