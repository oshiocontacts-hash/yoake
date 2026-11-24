//! `cargo horizon-mod datagen [--check]`: a content mod's generated data (`yoake-datagen`).
//!
//! A mod opts in with a `datagen/` crate beside its manifest (its own `Cargo.toml`, a binary
//! whose `main` is `yoake_datagen::main`). The command runs it on the mod folder, which writes
//! the generated files into `assets/` and `data/`; then the mod is built and checked like
//! `cargo horizon-mod build` and `check` do. With `--check`, the generated files are compared
//! with the folder instead of written, and a difference fails (for CI).
//!
//! The mod's root keeps no `Cargo.toml`: the generator is a separate crate, so the mod stays a
//! content mod (docs/mods.md, "Data generation").
use crate::{build_in, check};
use horizon_mods::{Manifest, index::Kind, manifest::MANIFEST};
use std::path::Path;
use std::process::Command;

/// `datagen [--check]` on the current folder.
pub fn main(args: &[String]) -> Result<(), String> {
    let check_only = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err("usage: cargo horizon-mod datagen [--check]".into()),
    };
    let root = std::env::current_dir().map_err(|e| e.to_string())?;
    print!("{}", run(&root, check_only)?);
    Ok(())
}

/// Generates the data of the mod in `root` (or checks it) and validates the result. Returns the
/// report of the built `.hmod`.
pub fn run(root: &Path, check_only: bool) -> Result<String, String> {
    let manifest_text = std::fs::read_to_string(root.join(MANIFEST))
        .map_err(|e| format!("{MANIFEST}: {e} (run this in a mod's folder)"))?;
    let manifest = Manifest::parse(&manifest_text)?;
    if root.join("Cargo.toml").exists() {
        return Err("a Cargo.toml: code mods are native now (docs/native-mods.md); a content mod's generator goes in datagen/".into());
    }
    let generator = root.join("datagen").join("Cargo.toml");
    if !generator.exists() {
        return Err(format!(
            "no {}: a content mod generates its data with a datagen/ crate (docs/mods.md, \"Data generation\")",
            generator.display()
        ));
    }
    // The generator builds into its own target folder, so a `cargo horizon-mod` run inside a
    // workspace does not share (and lock) its target directory.
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command
        .current_dir(root)
        .args(["run", "--quiet", "--manifest-path"])
        .arg(&generator)
        .env("CARGO_TARGET_DIR", root.join("datagen").join("target"))
        .arg("--")
        .arg(root)
        .args(["--id", &manifest.id]);
    if check_only {
        command.arg("--check");
    }
    let status = command
        .status()
        .map_err(|e| format!("could not run cargo for the datagen crate: {e}"))?;
    if !status.success() {
        return Err(if check_only {
            format!("the generated data is out of date ({status}); run cargo horizon-mod datagen")
        } else {
            format!("the datagen crate failed ({status})")
        });
    }
    // The same validation as `build` and `check`: the content is read as the game reads it.
    let hmod = build_in(root)?;
    let kind = check::kind_of(&hmod).unwrap_or(Kind::Mod);
    let report = check::check(&hmod, kind, None, None)?;
    Ok(report.text())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A datagen crate for the test mod, depending on this checkout's `yoake-datagen`.
    fn write_generator(path: &Path) {
        let lib = concat!(env!("CARGO_MANIFEST_DIR"), "/../yoake-datagen").replace('\\', "/");
        std::fs::create_dir_all(path.join("datagen/src")).unwrap();
        std::fs::write(
            path.join("datagen/Cargo.toml"),
            format!(
                "[package]\nname = \"ember-datagen\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n\
                 [dependencies]\nyoake-datagen = {{ path = \"{lib}\" }}\n\n[workspace]\n"
            ),
        )
        .unwrap();
        std::fs::write(
            path.join("datagen/src/main.rs"),
            r##"use yoake_datagen::*;

fn main() -> std::process::ExitCode {
    yoake_datagen::main(generate)
}

fn generate(m: &mut Mod) {
    m.block("ember_ore", &BlockDef::new("minecraft:iron_ore").hardness(3.0).light(7));
    m.blockstate("ember_ore", &BlockState::single("ember:block/ember_ore"));
    m.model("block/ember_ore", &Model::cube_all("minecraft:block/iron_ore"));
    m.item_model("ember_ore", &ItemModel::model("ember:block/ember_ore"));
    m.item("ember_gem", &ItemDef::new("minecraft:emerald").max_stack_size(16).rarity("rare"));
    m.recipe("ember_gem", &Recipe::shapeless(&["ember:ember_ore"], Stack::new("ember:ember_gem", 2)));
    m.loot_table("blocks/ember_ore", &LootTable::drop_item("ember:ember_gem", 2));
    m.tag(TagRegistry::Block, "minecraft:mineable/pickaxe", &Tag::new(&["ember:ember_ore"]));
    m.enchantment(
        "ember_touch",
        &Enchantment::new("enchantment.ember.ember_touch", "#minecraft:enchantable/weapon", &["mainhand"])
            .max_level(3),
    );
    m.damage_type("ember_burn", &DamageType::new("ember_burn", DamageScaling::Never, 0.1).effects(DamageEffects::Burning));
    m.game_rule("ember_fire", &GameRule::bool(true).category("mobs"));
    m.lang(
        Locale::EnUs,
        &[
            ("block.ember.ember_ore", "Ember Ore"),
            ("item.ember.ember_gem", "Ember Gem"),
            ("block.ember.glowing_amethyst", "Glowing Amethyst"),
        ],
    );
}
"##,
        )
        .unwrap();
    }

    #[test]
    fn datagen_writes_checks_and_validates_a_content_mod() {
        let dir = std::env::temp_dir().join(format!("horizon-mod-datagen-{}", std::process::id()));
        let path = dir.join("ember");
        crate::new(&[
            path.display().to_string(),
            "--template".into(),
            "content".into(),
        ])
        .unwrap();
        // A mod with no datagen crate is told so, and its folder is left as it was.
        let err = run(&path, false).unwrap_err();
        assert!(err.contains("datagen"), "{err}");
        write_generator(&path);

        // Generates: the mod's files appear, and the built package checks as content only.
        let report = run(&path, false).unwrap();
        assert!(report.contains("no code: content only"), "{report}");
        for file in [
            "assets/ember/blockstates/ember_ore.json",
            "assets/ember/models/block/ember_ore.json",
            "assets/ember/items/ember_ore.json",
            "assets/ember/lang/en_us.json",
            "data/ember/horizon/block/ember_ore.json",
            "data/ember/horizon/item/ember_gem.json",
            "data/ember/recipe/ember_gem.json",
            "data/ember/loot_table/blocks/ember_ore.json",
            "data/minecraft/tags/block/mineable/pickaxe.json",
            "data/ember/horizon/enchantment/ember_touch.json",
            "data/ember/horizon/damage_type/ember_burn.json",
            "data/ember/horizon/game_rule/ember_fire.json",
        ] {
            assert!(path.join(file).is_file(), "{file} is not written");
        }
        let block: serde_json::Value = serde_json::from_slice(
            &std::fs::read(path.join("data/ember/horizon/block/ember_ore.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(block["template"], "minecraft:iron_ore");
        assert_eq!(block["properties"]["light"], 7);

        // --check passes on the generated folder, and fails once a generated file differs.
        run(&path, true).unwrap();
        std::fs::write(path.join("assets/ember/lang/en_us.json"), "{}\n").unwrap();
        let err = run(&path, true).unwrap_err();
        assert!(err.contains("out of date"), "{err}");
        // Writing again restores it.
        run(&path, false).unwrap();
        run(&path, true).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
