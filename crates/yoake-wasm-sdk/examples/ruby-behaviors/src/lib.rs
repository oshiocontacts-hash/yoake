//! Behaviours of a mod's own content (REM-15), the sample of `behavior-register`:
//!
//! * the block `ruby:lamp` (a redstone lamp's states: `lit`) toggles `lit` when used (right click, with
//!   the ordinary `block-property` and `set-block-property` services), lights when an entity steps on it
//!   and goes dark again 40 ticks later (a scheduled tick, with commands), and counts what happens to it;
//! * the item `ruby:wand` toggles the lamp it is used on, counts its uses and ticks in the inventory,
//!   and leaves a fresh wand when it finishes being used;
//! * the menu `ruby:vault` has nine slots, takes only emeralds in its first one, and turns four emeralds
//!   in that slot into an emerald block in the last when clicked (a crafting-like menu); a shift-click asks
//!   the recipes of the type below (their data, matched here) what its first two slots make and puts it in the last;
//! * the recipe type `ruby:alloy` (`data/ruby/recipe/alloy.json`: `inputs` in any order and a `result`),
//!   matched and assembled here.
//!
//! `handle_call("ruby:stats")` answers the counts as text, for the tests.
use yoake_wasm_sdk::full::behaviors::{
    self, Behaviors, Block, BlockAnswer, BlockUse, Command, Interaction, ItemAnswer, ItemCall, ItemTick, ItemUseOnBlock,
    Leave, MenuChange, MenuContents, MenuOpen, RecipeDescription, RecipeDisplay, Stack, block, item, json_fields,
    json_strings, menu, recipe,
};
use yoake_wasm_sdk::full::server::{ServerMod, ext, services, types::BlockPos};

const LAMP: &str = "ruby:lamp";
const WAND: &str = "ruby:wand";
const VAULT: &str = "ruby:vault";
const ALLOY: &str = "ruby:alloy";

#[derive(Default)]
struct Guest {
    placed: u32,
    removed: u32,
    neighbors: u32,
    random: u32,
    inside: u32,
    stepped: u32,
    block_uses: u32,
    wand_uses: u32,
    wand_blocks: u32,
    wand_entities: u32,
    wand_finished: u32,
    wand_ticks: u32,
    vault_closed: u32,
}

fn pos(p: [i32; 3]) -> BlockPos {
    BlockPos { x: p[0], y: p[1], z: p[2] }
}

/// Flips the `lit` property of the block at `at` and tells the player who did it what the lamp does now: whether it
/// was changed.
fn toggle(at: [i32; 3]) -> bool {
    let Ok(Some(lit)) = services::block_property(pos(at), "lit") else { return false };
    let lighting = lit != "true";
    let changed = services::set_block_property(3, pos(at), "lit", if lighting { "true" } else { "false" }).unwrap_or(false);
    if changed && let Some(player) = services::acting_player() {
        let _ = services::tell(if lighting { "The ruby lamp lights." } else { "The ruby lamp goes dark." }, &player.uuid);
    }
    changed
}

/// What a recipe's data (its JSON text) asks: the stacks it takes, the item it makes and how many.
fn alloy(data: &[u8]) -> Option<(Vec<String>, String, u32)> {
    let fields = json_fields(core::str::from_utf8(data).ok()?)?;
    let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
    let inputs = json_strings(get("inputs")?)?;
    let result = get("result")?.strip_prefix('"')?.strip_suffix('"')?.to_owned();
    let count = get("count").map_or(Some(1), |text| text.parse().ok())?;
    Some((inputs, result, count))
}

fn count(contents: &[(u16, Stack)], slot: u16) -> u32 {
    contents.iter().find(|(s, _)| *s == slot).map_or(0, |(_, stack)| stack.count)
}

impl Behaviors for Guest {
    fn on_block_use(&mut self, call: &BlockUse) -> BlockAnswer {
        self.block_uses += 1;
        if toggle(call.block.pos) { BlockAnswer::result(Interaction::Success) } else { BlockAnswer::result(Interaction::Fail) }
    }
    fn on_block_placed(&mut self, _block: &Block, _previous: u32) -> BlockAnswer {
        self.placed += 1;
        BlockAnswer::default()
    }
    fn on_block_removed(&mut self, _block: &Block, _new: u32) -> BlockAnswer {
        self.removed += 1;
        BlockAnswer::default()
    }
    fn on_block_neighbor_changed(&mut self, _block: &Block, _changed: u32, _moved: bool) -> BlockAnswer {
        self.neighbors += 1;
        BlockAnswer::default()
    }
    fn on_block_random_tick(&mut self, _block: &Block) -> BlockAnswer {
        self.random += 1;
        BlockAnswer::default()
    }
    fn on_block_scheduled_tick(&mut self, block: &Block) -> BlockAnswer {
        BlockAnswer::default().with(Command::SetProperty { pos: block.pos, key: "lit".into(), value: "false".into() })
    }
    fn on_block_entity_inside(&mut self, _block: &Block, _entity: u32, _kind: &str) -> BlockAnswer {
        self.inside += 1;
        BlockAnswer::default()
    }
    fn on_block_step_on(&mut self, block: &Block, _entity: u32, _kind: &str) -> BlockAnswer {
        self.stepped += 1;
        BlockAnswer::default()
            .with(Command::SetProperty { pos: block.pos, key: "lit".into(), value: "true".into() })
            .with(Command::ScheduleTick { pos: block.pos, block: String::new(), delay: 40 })
    }

    fn on_item_use(&mut self, _call: &ItemCall) -> ItemAnswer {
        self.wand_uses += 1;
        if let Some(player) = services::acting_player() {
            let _ = services::tell("The ruby wand hums.", &player.uuid);
        }
        ItemAnswer::result(Interaction::Consume)
    }
    fn on_item_use_on_block(&mut self, call: &ItemUseOnBlock) -> ItemAnswer {
        self.wand_blocks += 1;
        if services::block(pos(call.pos)).is_ok_and(|name| name.starts_with(LAMP)) && toggle(call.pos) {
            ItemAnswer::result(Interaction::Success)
        } else {
            ItemAnswer::result(Interaction::Pass)
        }
    }
    fn on_item_use_on_entity(&mut self, _call: &behaviors::ItemUseOnEntity) -> ItemAnswer {
        self.wand_entities += 1;
        ItemAnswer::result(Interaction::Fail)
    }
    fn on_item_finish_using(&mut self, _call: &ItemCall) -> ItemAnswer {
        self.wand_finished += 1;
        ItemAnswer { result: Interaction::Success, leave: Leave::Stack(Stack::new(WAND, 1)) }
    }
    fn on_item_inventory_tick(&mut self, _call: &ItemTick) {
        self.wand_ticks += 1;
    }

    fn on_menu_create(&mut self, _open: &MenuOpen) -> Option<MenuContents> {
        Some(MenuContents { slots: 9, stacks: vec![(0, Stack::new("minecraft:emerald", 4))] })
    }
    fn on_menu_may_place(&mut self, _menu: &str, slot: u16, stack: &Stack) -> bool {
        slot != 0 || stack.id == "minecraft:emerald"
    }
    fn on_menu_closed(&mut self, _menu: &str, _player: &str, _contents: &[(u16, Stack)]) {
        self.vault_closed += 1;
    }
    fn on_menu_quick_move(
        &mut self,
        _menu: &str,
        _player: &str,
        _slot: u16,
        _inventory: bool,
        _stack: &Stack,
        contents: &[(u16, Stack)],
    ) -> MenuChange {
        // The first two slots are the grid. The recipes of the type are this package's own to match: the engine
        // cannot call a package that is running a hook, so it gives their data (`recipe-data`) and the guest
        // asks its own `on_recipe_matches`.
        let grid: Vec<Stack> = contents.iter().filter(|(slot, _)| *slot < 2).map(|(_, stack)| stack.clone()).collect();
        let Ok(recipes) = ext::recipe_data(ALLOY) else { return MenuChange::default() };
        let Some(data) = recipes.iter().map(|(_, data)| data).find(|data| self.on_recipe_matches(ALLOY, data, &grid)) else {
            return MenuChange::default();
        };
        let Some(made) = self.on_recipe_assemble(ALLOY, data, &grid).into_iter().next() else {
            return MenuChange::default();
        };
        MenuChange { handled: true, set: vec![(0, None), (1, None), (8, Some(made))] }
    }
    fn on_menu_clicked(&mut self, _menu: &str, _player: &str, slot: u16, _button: u8, _mode: u8, contents: &[(u16, Stack)]) -> MenuChange {
        let emeralds = count(contents, 0);
        if slot == 0 && emeralds >= 4 {
            let left = emeralds - 4;
            let rest = (left > 0).then(|| Stack::new("minecraft:emerald", left));
            MenuChange { handled: false, set: vec![(0, rest), (8, Some(Stack::new("minecraft:emerald_block", count(contents, 8) + 1)))] }
        } else {
            MenuChange::default()
        }
    }

    fn on_recipe_describe(&mut self, recipe_type: &str) -> Option<RecipeDescription> {
        (recipe_type == ALLOY).then(|| RecipeDescription { version: 1, category: "ruby:alloy".into(), icon: "minecraft:iron_ingot".into() })
    }
    fn on_recipe_matches(&mut self, _recipe_type: &str, data: &[u8], input: &[Stack]) -> bool {
        let Some((mut wanted, _, _)) = alloy(data) else { return false };
        let mut given: Vec<&str> = input.iter().filter(|stack| stack.count > 0).map(|stack| stack.id.as_str()).collect();
        wanted.sort();
        given.sort();
        wanted.iter().map(String::as_str).eq(given.iter().copied())
    }
    fn on_recipe_assemble(&mut self, _recipe_type: &str, data: &[u8], _input: &[Stack]) -> Vec<Stack> {
        alloy(data).map(|(_, result, count)| vec![Stack::new(&result, count)]).unwrap_or_default()
    }
    fn on_recipe_display(&mut self, _recipe_type: &str, data: &[u8]) -> RecipeDisplay {
        match alloy(data) {
            Some((inputs, result, count)) => RecipeDisplay {
                inputs: inputs.iter().map(|id| vec![Stack::new(id, 1)]).collect(),
                outputs: vec![Stack::new(&result, count)],
            },
            None => RecipeDisplay::default(),
        }
    }
}

impl ServerMod for Guest {
    fn subscriptions() -> &'static [&'static str] {
        &[]
    }

    fn init(&mut self) {
        let results = [
            ext::register_behavior(
                "block",
                LAMP,
                &[
                    block::USE,
                    block::PLACED,
                    block::REMOVED,
                    block::NEIGHBOR_CHANGED,
                    block::RANDOM_TICK,
                    block::SCHEDULED_TICK,
                    block::ENTITY_INSIDE,
                    block::STEP_ON,
                ],
            ),
            ext::register_behavior(
                "item",
                WAND,
                &[item::USE, item::USE_ON_BLOCK, item::USE_ON_ENTITY, item::FINISH_USING, item::INVENTORY_TICK],
            ),
            ext::register_behavior("menu", VAULT, &[menu::CREATE, menu::MAY_PLACE, menu::CLOSED, menu::CLICKED, menu::QUICK_MOVE]),
            ext::register_behavior(
                "recipe",
                ALLOY,
                &[recipe::DESCRIBE, recipe::MATCHES, recipe::ASSEMBLE, recipe::DISPLAY],
            ),
        ];
        for result in results {
            if let Err(error) = result {
                println!("ruby-behaviors: registering: {error}");
            }
        }
    }

    fn handle_call(&mut self, _caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
        if method == "ruby:stats" {
            return Ok(format!(
                "placed={} removed={} neighbors={} random={} inside={} stepped={} block_uses={} wand_uses={} wand_blocks={} wand_entities={} wand_finished={} wand_ticks={} vault_closed={}",
                self.placed, self.removed, self.neighbors, self.random, self.inside, self.stepped, self.block_uses,
                self.wand_uses, self.wand_blocks, self.wand_entities, self.wand_finished, self.wand_ticks, self.vault_closed
            )
            .into_bytes());
        }
        behaviors::handle(self, &method, &payload).unwrap_or_else(|| Err(format!("unknown method {method}")))
    }
}

yoake_wasm_sdk::export_server_full!(Guest);
