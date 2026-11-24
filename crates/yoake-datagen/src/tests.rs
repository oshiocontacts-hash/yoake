//! Each builder writes the exact JSON the loader and the game read (the expected texts are
//! written out here, not derived from the builders).
use super::*;
use serde_json::json;

fn text(m: &Mod, path: &str) -> String {
    m.output()
        .text(path)
        .unwrap_or_else(|| panic!("{path} is not generated"))
        .to_owned()
}

#[test]
fn blockstate_single_and_variants_are_exact() {
    let mut m = Mod::new("ember");
    m.blockstate("ember_ore", &BlockState::single("ember:block/ember_ore"));
    m.blockstate(
        "pipe",
        &BlockState::default()
            .variant("axis=x", "ember:block/pipe_x")
            .variant("axis=y", "ember:block/pipe_y"),
    );
    assert_eq!(
        text(&m, "assets/ember/blockstates/ember_ore.json"),
        "{\n  \"variants\": {\n    \"\": {\n      \"model\": \"ember:block/ember_ore\"\n    }\n  }\n}\n"
    );
    let pipe: Value = serde_json::from_str(&text(&m, "assets/ember/blockstates/pipe.json")).unwrap();
    assert_eq!(
        pipe,
        json!({"variants": {"axis=x": {"model": "ember:block/pipe_x"}, "axis=y": {"model": "ember:block/pipe_y"}}})
    );
}

#[test]
fn models_name_their_parent_and_texture_slots() {
    let mut m = Mod::new("ember");
    m.model("block/ember_ore", &Model::cube_all("minecraft:block/iron_ore"));
    m.model("block/ember_fern", &Model::cross("ember:block/ember_fern"));
    m.model("item/ember_gem", &Model::item_generated("ember:item/ember_gem"));
    m.model("item/ember_sword", &Model::item_handheld("ember:item/ember_sword"));
    m.model(
        "block/ember_lamp",
        &Model::parent("minecraft:block/cube_column").texture("end", "ember:block/lamp_top").texture("side", "ember:block/lamp"),
    );
    assert_eq!(
        text(&m, "assets/ember/models/block/ember_ore.json"),
        "{\n  \"parent\": \"minecraft:block/cube_all\",\n  \"textures\": {\n    \"all\": \"minecraft:block/iron_ore\"\n  }\n}\n"
    );
    let value = |p: &str| -> Value { serde_json::from_str(&text(&m, p)).unwrap() };
    assert_eq!(value("assets/ember/models/block/ember_fern.json"), json!({"parent": "minecraft:block/cross", "textures": {"cross": "ember:block/ember_fern"}}));
    assert_eq!(value("assets/ember/models/item/ember_gem.json"), json!({"parent": "minecraft:item/generated", "textures": {"layer0": "ember:item/ember_gem"}}));
    assert_eq!(value("assets/ember/models/item/ember_sword.json"), json!({"parent": "minecraft:item/handheld", "textures": {"layer0": "ember:item/ember_sword"}}));
    assert_eq!(
        value("assets/ember/models/block/ember_lamp.json"),
        json!({"parent": "minecraft:block/cube_column", "textures": {"end": "ember:block/lamp_top", "side": "ember:block/lamp"}})
    );
}

#[test]
fn item_model_definition_is_exact() {
    let mut m = Mod::new("ember");
    m.item_model("ember_ore", &ItemModel::model("ember:block/ember_ore"));
    assert_eq!(
        text(&m, "assets/ember/items/ember_ore.json"),
        "{\n  \"model\": {\n    \"model\": \"ember:block/ember_ore\",\n    \"type\": \"minecraft:model\"\n  }\n}\n"
    );
}

#[test]
fn lang_files_per_locale() {
    let mut m = Mod::new("ember");
    m.lang(Locale::EnUs, &[("block.ember.ember_ore", "Ember Ore")]);
    m.lang(Locale::JaJp, &[("block.ember.ember_ore", "エンバー鉱石")]);
    assert_eq!(text(&m, "assets/ember/lang/en_us.json"), "{\n  \"block.ember.ember_ore\": \"Ember Ore\"\n}\n");
    assert_eq!(text(&m, "assets/ember/lang/ja_jp.json"), "{\n  \"block.ember.ember_ore\": \"エンバー鉱石\"\n}\n");
}

#[test]
fn shaped_and_shapeless_recipes_are_exact() {
    let mut m = Mod::new("ember");
    m.recipe(
        "ember_block",
        &Recipe::shaped(&["EE", "EE"], &[('E', "ember:ember_gem")], Stack::new("ember:ember_block", 1)),
    );
    m.recipe(
        "ember_dust",
        &Recipe::shapeless(&["ember:ember_gem", "minecraft:gunpowder"], Stack::new("ember:ember_dust", 2)).category("misc"),
    );
    assert_eq!(
        text(&m, "data/ember/recipe/ember_block.json"),
        concat!(
            "{\n",
            "  \"category\": \"misc\",\n",
            "  \"key\": {\n",
            "    \"E\": \"ember:ember_gem\"\n",
            "  },\n",
            "  \"pattern\": [\n",
            "    \"EE\",\n",
            "    \"EE\"\n",
            "  ],\n",
            "  \"result\": {\n",
            "    \"id\": \"ember:ember_block\"\n",
            "  },\n",
            "  \"type\": \"minecraft:crafting_shaped\"\n",
            "}\n"
        )
    );
    let dust: Value = serde_json::from_str(&text(&m, "data/ember/recipe/ember_dust.json")).unwrap();
    assert_eq!(
        dust,
        json!({"type": "minecraft:crafting_shapeless", "category": "misc",
               "ingredients": ["ember:ember_gem", "minecraft:gunpowder"],
               "result": {"id": "ember:ember_dust", "count": 2}})
    );
}

#[test]
fn cooking_recipes_use_their_own_type() {
    let mut m = Mod::new("ember");
    m.recipe("smelt", &Recipe::cooking(Cooking::Smelting, "ember:ore_raw", Stack::new("ember:ingot", 1), 0.7, 200));
    m.recipe("blast", &Recipe::cooking(Cooking::Blasting, "ember:ore_raw", Stack::new("ember:ingot", 1), 0.7, 100));
    m.recipe("smoke", &Recipe::cooking(Cooking::Smoking, "minecraft:beef", Stack::new("ember:steak", 1), 0.35, 100));
    m.recipe("camp", &Recipe::cooking(Cooking::CampfireCooking, "minecraft:beef", Stack::new("ember:steak", 1), 0.35, 600));
    let value = |p: &str| -> Value { serde_json::from_str(&text(&m, p)).unwrap() };
    assert_eq!(
        value("data/ember/recipe/smelt.json"),
        json!({"type": "minecraft:smelting", "category": "misc", "ingredient": "ember:ore_raw",
               "result": {"id": "ember:ingot"}, "experience": 0.7, "cookingtime": 200})
    );
    assert_eq!(value("data/ember/recipe/blast.json")["type"], "minecraft:blasting");
    assert_eq!(value("data/ember/recipe/smoke.json")["type"], "minecraft:smoking");
    assert_eq!(value("data/ember/recipe/camp.json")["type"], "minecraft:campfire_cooking");
    assert_eq!(value("data/ember/recipe/camp.json")["cookingtime"], 600);
}

#[test]
fn loot_tables_drop_self_or_a_count() {
    let mut m = Mod::new("ember");
    m.loot_table("blocks/ember_ore", &LootTable::drop_self("ember:ember_ore"));
    m.loot_table("blocks/ember_block", &LootTable::drop_item("ember:ember_gem", 4));
    let value = |p: &str| -> Value { serde_json::from_str(&text(&m, p)).unwrap() };
    assert_eq!(
        value("data/ember/loot_table/blocks/ember_ore.json"),
        json!({
            "type": "minecraft:block",
            "pools": [{
                "rolls": 1,
                "conditions": [{"condition": "minecraft:survives_explosion"}],
                "entries": [{"type": "minecraft:item", "name": "ember:ember_ore"}]
            }]
        })
    );
    assert_eq!(
        value("data/ember/loot_table/blocks/ember_block.json")["pools"][0]["entries"],
        json!([{"type": "minecraft:item", "name": "ember:ember_gem",
                "functions": [{"function": "minecraft:set_count", "count": 4, "add": false}]}])
    );
}

#[test]
fn tags_add_or_replace_under_their_namespace() {
    let mut m = Mod::new("ember");
    m.tag(TagRegistry::Block, "minecraft:mineable/pickaxe", &Tag::new(&["ember:ember_ore"]));
    m.tag(TagRegistry::Item, "c:gems", &Tag::new(&["ember:ember_gem"]).replace());
    assert_eq!(
        text(&m, "data/minecraft/tags/block/mineable/pickaxe.json"),
        "{\n  \"values\": [\n    \"ember:ember_ore\"\n  ]\n}\n"
    );
    assert_eq!(
        text(&m, "data/c/tags/item/gems.json"),
        "{\n  \"replace\": true,\n  \"values\": [\n    \"ember:ember_gem\"\n  ]\n}\n"
    );
}

#[test]
fn block_and_item_declarations_are_exact() {
    let mut m = Mod::new("ember");
    m.block("ember_ore", &BlockDef::new("minecraft:iron_ore").hardness(3.0).light(7).requires_tool(true));
    m.block("ember_fern", &BlockDef::new("minecraft:fern").without_item());
    m.item("ember_gem", &ItemDef::new("minecraft:emerald").max_stack_size(16).rarity("rare").crafting_remainder("minecraft:bucket").component("minecraft:max_damage", json!(300)));
    let value = |p: &str| -> Value { serde_json::from_str(&text(&m, p)).unwrap() };
    assert_eq!(
        value("data/ember/horizon/block/ember_ore.json"),
        json!({"template": "minecraft:iron_ore", "properties": {"hardness": 3.0, "light": 7, "requires_tool": true}})
    );
    assert_eq!(
        value("data/ember/horizon/block/ember_fern.json"),
        json!({"template": "minecraft:fern", "item": false})
    );
    assert_eq!(
        value("data/ember/horizon/item/ember_gem.json"),
        json!({"template": "minecraft:emerald", "max_stack_size": 16, "rarity": "rare",
               "crafting_remainder": "minecraft:bucket", "components": {"minecraft:max_damage": 300}})
    );
}

#[test]
fn enchantment_damage_type_and_game_rule_are_exact() {
    let mut m = Mod::new("ember");
    m.enchantment(
        "ember_touch",
        &Enchantment::new("enchantment.ember.ember_touch", "#minecraft:enchantable/weapon", &["mainhand"])
            .weight(5)
            .max_level(3)
            .min_cost(5, 8)
            .max_cost(25, 8)
            .anvil_cost(4),
    );
    m.damage_type("ember_burn", &DamageType::new("ember_burn", DamageScaling::Never, 0.1).effects(DamageEffects::Burning).death_message_type(DeathMessageType::IntentionalGameDesign));
    m.game_rule("ember_fire", &GameRule::bool(true).category("mobs"));
    m.game_rule("ember_limit", &GameRule::int(4, 0, 64));
    let value = |p: &str| -> Value { serde_json::from_str(&text(&m, p)).unwrap() };
    assert_eq!(
        value("data/ember/horizon/enchantment/ember_touch.json"),
        json!({
            "description": {"translate": "enchantment.ember.ember_touch"},
            "supported_items": "#minecraft:enchantable/weapon",
            "slots": ["mainhand"], "weight": 5, "max_level": 3,
            "min_cost": {"base": 5, "per_level_above_first": 8},
            "max_cost": {"base": 25, "per_level_above_first": 8},
            "anvil_cost": 4
        })
    );
    assert_eq!(
        value("data/ember/horizon/damage_type/ember_burn.json"),
        json!({"message_id": "ember_burn", "scaling": "never", "exhaustion": 0.1, "effects": "burning",
               "death_message_type": "intentional_game_design"})
    );
    assert_eq!(
        value("data/ember/horizon/game_rule/ember_fire.json"),
        json!({"type": "bool", "default": true, "category": "mobs"})
    );
    assert_eq!(
        value("data/ember/horizon/game_rule/ember_limit.json"),
        json!({"type": "int", "default": 4, "min": 0, "max": 64, "category": "misc"})
    );
}

#[test]
fn fluid_declaration_keeps_only_what_is_set() {
    let mut m = Mod::new("ember");
    m.fluid("ember", &Fluid::new().tint(0xFF8800).viscosity(6).motion("lava").light(12));
    assert_eq!(
        serde_json::from_str::<Value>(&text(&m, "data/ember/horizon/fluid/ember.json")).unwrap(),
        json!({"tint": 0xFF8800, "viscosity": 6, "motion": "lava", "light": 12})
    );
}

#[test]
fn run_writes_then_check_passes_until_a_file_changes() {
    let dir = std::env::temp_dir().join(format!("yoake-datagen-run-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let args: Vec<String> = [dir.display().to_string(), "--id".into(), "ember".into()].to_vec();
    let generate = |m: &mut Mod| {
        m.block("ember_ore", &BlockDef::new("minecraft:iron_ore"));
        m.lang(Locale::EnUs, &[("block.ember.ember_ore", "Ember Ore")]);
    };
    assert_eq!(run(&args, generate).unwrap(), "datagen: wrote 2 files");
    let mut check = args.clone();
    check.push("--check".into());
    assert_eq!(run(&check, generate).unwrap(), "datagen: 2 files up to date");
    std::fs::write(dir.join("assets/ember/lang/en_us.json"), "{}\n").unwrap();
    let err = run(&check, generate).unwrap_err();
    assert!(err.contains("assets/ember/lang/en_us.json"), "{err}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn parse_args_needs_a_folder_and_an_id() {
    let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    assert_eq!(
        parse_args(&s(&["mod", "--id", "ember", "--check"])).unwrap(),
        Args { root: "mod".into(), id: "ember".into(), check: true }
    );
    assert!(parse_args(&s(&["mod"])).is_err());
    assert!(parse_args(&s(&["--id", "ember"])).is_err());
}

#[test]
#[should_panic(expected = "twice")]
fn writing_one_file_twice_is_refused() {
    let mut m = Mod::new("ember");
    m.blockstate("a", &BlockState::single("ember:block/a"));
    m.blockstate("a", &BlockState::single("ember:block/b"));
}
