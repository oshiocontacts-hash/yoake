//! Tests of the behaviour objects of WebAssembly mods (REM-15): the wire, the table, and the example guest
//! `crates/yoake-wasm-sdk/examples/ruby-behaviors` called through the objects the engine builds from it.
use super::*;
use crate::mod_api::{AttachmentError, Attachments, Holder, LegacyRandom, LevelRandomHost, LevelTicksHost, ParticleOptions};
use yoake_wasm::test_guests::{RUBY_BEHAVIORS, load};

fn lock() -> std::sync::MutexGuard<'static, ()> {
    super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

// ----- the wire -----

#[test]
fn answers_are_checked_before_anything_is_applied() {
    let origin = [10, 64, 10];
    let mut e = Enc::new();
    e.u8(1).u16(2);
    e.u8(1).i32(10).i32(64).i32(11).str("lit").str("true");
    e.u8(2).i32(12).i32(64).i32(10).str("").u32(40);
    let good = e.finish();
    let answer = Answer::decode(&good, origin).expect("a good answer");
    assert_eq!(answer.result, NativeInteractionResult::Success);
    assert_eq!(
        answer.commands,
        vec![
            Command::SetProperty { pos: [10, 64, 11], key: "lit".into(), value: "true".into() },
            Command::ScheduleTick { pos: [12, 64, 10], block: String::new(), delay: 40 },
        ]
    );
    // A wrong version, a result that does not exist, bytes left over, a missing end.
    let mut bad = good.clone();
    bad[0] = 2;
    assert!(Answer::decode(&bad, origin).is_none());
    let mut bad = good.clone();
    bad[1] = 9;
    assert!(Answer::decode(&bad, origin).is_none());
    let mut bad = good.clone();
    bad.push(0);
    assert!(Answer::decode(&bad, origin).is_none());
    assert!(Answer::decode(&good[..good.len() - 1], origin).is_none());
    // A command out of reach, a delay past a million ticks, too many commands, a sound at a number that is none.
    let far = {
        let mut e = Enc::new();
        e.u8(0).u16(1).u8(1).i32(10 + MAX_REACH + 1).i32(64).i32(10).str("lit").str("true");
        e.finish()
    };
    assert!(Answer::decode(&far, origin).is_none());
    let slow = {
        let mut e = Enc::new();
        e.u8(0).u16(1).u8(2).i32(10).i32(64).i32(10).str("").u32(2_000_000);
        e.finish()
    };
    assert!(Answer::decode(&slow, origin).is_none());
    let many = {
        let mut e = Enc::new();
        e.u8(0).u16(65);
        for _ in 0..65 {
            e.u8(1).i32(10).i32(64).i32(10).str("a").str("b");
        }
        e.finish()
    };
    assert!(Answer::decode(&many, origin).is_none());
    let nan = {
        let mut e = Enc::new();
        e.u8(0).u16(1).u8(3).str("minecraft:block.note_block.bell").f64(f64::NAN).f64(0.0).f64(0.0);
        e.0.extend_from_slice(&1.0f32.to_le_bytes());
        e.0.extend_from_slice(&1.0f32.to_le_bytes());
        e.finish()
    };
    assert!(Answer::decode(&nan, origin).is_none());
    // A long name is refused, not cut.
    let long = {
        let mut e = Enc::new();
        e.u8(0).u16(1).u8(1).i32(10).i32(64).i32(10).str(&"k".repeat(MAX_NAME + 1)).str("v");
        e.finish()
    };
    assert!(Answer::decode(&long, origin).is_none());
}

#[test]
fn registrations_are_checked_and_stay_the_same_across_worlds() {
    let _serial = lock();
    reset();
    let hooks = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
    // Not from `init`.
    assert!(register("ruby", "block", "ruby:lamp", &hooks(&["use"])).unwrap_err().contains("init"));
    let _init = super::super::full::ServerInitScope::enter("ruby");
    assert!(register("ruby", "block", "ruby:lamp", &hooks(&["use", "step-on"])).is_ok());
    assert!(register("ruby", "block", "ruby:lamp", &hooks(&["step-on", "use"])).is_ok(), "the same set again");
    // What a guest may not do.
    for (kind, id, names) in [
        ("blok", "ruby:lamp", vec!["use"]),
        ("block", "other:lamp", vec!["use"]),
        ("block", "minecraft:stone", vec!["use"]),
        ("block", "lamp", vec!["use"]),
        ("block", "ruby:lamp2", vec![]),
        ("block", "ruby:lamp2", vec!["use", "use"]),
        ("block", "ruby:lamp2", vec!["fly"]),
        ("item", "ruby:wand", vec!["placed"]),
        ("menu", "ruby:vault", vec!["use"]),
    ] {
        assert!(register("ruby", kind, id, &hooks(&names)).is_err(), "{kind} {id} {names:?}");
    }
    // Another package may not take the object over.
    {
        let _other = super::super::full::ServerInitScope::enter("other");
        assert!(register("other", "block", "ruby:lamp", &hooks(&["use"])).is_err());
    }
    // The game reads the table once; after it a new object or other hooks are refused, the same ones are not.
    assert_eq!(block_behaviors().len(), 1);
    assert!(register("ruby", "block", "ruby:lamp", &hooks(&["use", "step-on"])).is_ok());
    assert!(register("ruby", "block", "ruby:lamp", &hooks(&["use"])).unwrap_err().contains("restart"));
    assert!(register("ruby", "block", "ruby:lamp3", &hooks(&["use"])).unwrap_err().contains("already started"));
    assert!(any());
    reset();
    assert!(!any());
}

// ----- the example guest -----

#[derive(Default)]
struct TestHost {
    /// The `lit` property of the lamp.
    lit: bool,
    sets: Vec<([i32; 3], String, String, i32)>,
    /// The recipe type the engine would match recipes with, and the one recipe's data.
    recipes: Option<(Arc<dyn crate::mod_api::NativeRecipeCodec>, serde_json::Value)>,
    /// What `tell` was asked to say, and to whom.
    told: Vec<(u128, String)>,
}

impl Host for TestHost {
    fn acting_player(&self) -> Option<crate::mod_api::NativeServerPlayer> {
        Some(crate::mod_api::NativeServerPlayer {
            uuid: 0x42,
            name: "Tester".into(),
            permission: 4,
            dimension: "minecraft:overworld".into(),
            position: [0.0, 64.0, 0.0],
            game_mode: 0,
            health: 20.0,
            yaw: 0.0,
            pitch: 0.0,
        })
    }
    fn player_sneaking(&self, _uuid: u128) -> Result<bool, String> {
        Ok(false)
    }
    fn tell(&mut self, uuid: u128, line: crate::mod_api::Text) -> bool {
        self.told.push((uuid, line.plain()));
        true
    }
    fn recipe_data(&self, _kind: &str) -> Vec<(String, String)> {
        match &self.recipes {
            Some((_, data)) => vec![("ruby:alloy_shards".to_owned(), data.as_str().unwrap_or_default().to_owned())],
            None => Vec::new(),
        }
    }
    fn recipes_matching(&self, _kind: &str, input: &[ItemStack]) -> Vec<crate::mod_api::NativeRecipeMatch> {
        let Some((codec, data)) = &self.recipes else { return Vec::new() };
        if !codec.matches(data, input) {
            return Vec::new();
        }
        vec![crate::mod_api::NativeRecipeMatch {
            recipe: crate::mod_api::NativeRecipeEntry {
                id: "ruby:alloy_shards".into(),
                recipe_type: "ruby:alloy".into(),
                version: 1,
                category: crate::recipe_book::RecipeBookCategory {
                    id: "ruby:alloy".into(),
                    recipe_type: "ruby:alloy".into(),
                    icon: "minecraft:iron_ingot".into(),
                },
            },
            outputs: codec.assemble(data, input),
        }]
    }
    fn block(&self, _pos: [i32; 3]) -> String {
        "ruby:lamp[lit=false]".to_owned()
    }
    fn block_property(&self, _pos: [i32; 3], property: &str) -> Option<String> {
        (property == "lit").then(|| self.lit.to_string())
    }
    fn set_block_property_with(&mut self, pos: [i32; 3], property: &str, value: &str, flags: i32) -> bool {
        self.sets.push((pos, property.to_owned(), value.to_owned(), flags));
        self.lit = value == "true";
        true
    }
}

#[derive(Default)]
struct TestLevel {
    client: bool,
    sets: Vec<([i32; 3], String, String)>,
    ticks: Vec<([i32; 3], String, i32)>,
    sounds: Vec<String>,
    attachments: Attachments,
}

impl LevelHost for TestLevel {
    fn block_state(&self, _pos: [i32; 3]) -> BlockStateId {
        BlockStateId(0)
    }
    fn is_client_side(&self) -> bool {
        self.client
    }
    fn state_with_property(&self, _state: BlockStateId, _key: &str, _value: &str) -> Option<BlockStateId> {
        None
    }
}
impl LevelRandomHost for TestLevel {
    fn attachments(&mut self, _holder: Holder) -> Result<&mut Attachments, AttachmentError> {
        Ok(&mut self.attachments)
    }
    fn random(&mut self) -> Option<&mut LegacyRandom> {
        None
    }
}
impl LevelTicksHost for TestLevel {
    fn schedule_block_tick(&mut self, pos: [i32; 3], block: &str, delay: i32) -> bool {
        self.ticks.push((pos, block.to_owned(), delay));
        true
    }
    fn schedule_fluid_tick(&mut self, _pos: [i32; 3], _fluid: &str, _delay: i32) -> bool {
        false
    }
}
impl LevelWriteHost for TestLevel {
    fn set_block_property(&mut self, pos: [i32; 3], key: &str, value: &str, _flags: i32) -> bool {
        self.sets.push((pos, key.to_owned(), value.to_owned()));
        true
    }
    fn play_sound(&mut self, sound: &str, _at: [f64; 3], _source: usize, _volume: f32, _pitch: f32) -> bool {
        self.sounds.push(sound.to_owned());
        true
    }
    fn send_particles(
        &mut self,
        _options: &ParticleOptions,
        _state: Option<BlockStateId>,
        _at: [f64; 3],
        _spread: [f64; 3],
        _speed: f64,
        _count: i32,
    ) -> bool {
        false
    }
}

/// The ruby guest, loaded as a world start loads it (its `init` registers), under the id its objects use.
fn ruby() -> Option<Arc<super::super::full::FullMod>> {
    reset();
    let bytes = load(&RUBY_BEHAVIORS)?;
    let source = super::super::full::ServerSource::new("ruby", bytes, false);
    let module = super::super::full::load_server(&source.module()).expect("the guest loads");
    Some(Arc::new(super::super::full::FullMod::new("ruby".into(), None, Some(module)).with_server_source(source)))
}

fn stat(name: &str) -> u32 {
    let mut host = TestHost::default();
    let bytes = invoke("ruby", "ruby:stats", b"", HostAccess::Write(&mut host)).expect("the guest answers");
    let text = String::from_utf8(bytes).unwrap();
    text.split(' ').find_map(|pair| pair.strip_prefix(&format!("{name}="))).unwrap().parse().unwrap()
}

fn block(id: &str) -> Arc<dyn BlockBehavior> {
    block_behaviors().into_iter().find(|b| b.id == id).expect("registered").behavior
}

fn contact(client_side: bool) -> NativeBlockContact<'static> {
    NativeBlockContact {
        dimension: "minecraft:overworld",
        client_side,
        entity: 7,
        kind: "minecraft:pig",
        state: BlockStateId(5),
        position: BlockPos::new(10, 64, 10),
    }
}

#[test]
fn the_guest_registers_only_its_own_objects_and_the_engine_builds_their_behaviours() {
    let _serial = lock();
    let Some(_guest) = ruby() else { return };
    assert_eq!(block_behaviors().into_iter().map(|b| b.id).collect::<Vec<_>>(), ["ruby:lamp"]);
    assert_eq!(item_behaviors().into_iter().map(|b| b.id).collect::<Vec<_>>(), ["ruby:wand"]);
    assert_eq!(menu_behaviors().into_iter().map(|b| b.id).collect::<Vec<_>>(), ["ruby:vault"]);
    assert!(block("ruby:lamp").randomly_ticking(), "the lamp named random-tick");
    reset();
}

#[test]
fn block_hooks_reach_the_guest_for_its_own_block_and_apply_the_answers() {
    let _serial = lock();
    let Some(_guest) = ruby() else { return };
    let lamp = block("ruby:lamp");
    let wand = ItemStack::new("ruby:wand", 1);

    // Right click: the guest flips `lit` with the services of the hook's host.
    let mut host = TestHost::default();
    let context = NativeBlockUse {
        state: BlockStateId(5),
        hit: BlockHit { pos: [10, 64, 10], location: [10.5, 65.0, 10.5], face: Direction::Up },
        hand: Hand::Main,
        stack: Some(&wand),
        client_side: false,
    };
    assert_eq!(lamp.use_block(&mut host, &context), NativeInteractionResult::Success);
    assert_eq!(host.sets, vec![([10, 64, 10], "lit".to_owned(), "true".to_owned(), 3)]);
    assert_eq!(lamp.use_block(&mut host, &context), NativeInteractionResult::Success);
    assert!(!host.lit, "used again, it goes dark");
    assert_eq!(stat("block_uses"), 2);
    // A client that predicts the click calls nothing: the server decides.
    let client = NativeBlockUse { client_side: true, ..context };
    assert_eq!(lamp.use_block(&mut host, &client), NativeInteractionResult::Pass);
    assert_eq!(stat("block_uses"), 2);

    // A tick the level runs: the answer's commands go to the level.
    let mut level = TestLevel::default();
    assert!(!lamp.neighbor_changed(&mut level, BlockPos::new(10, 64, 10), BlockStateId(5), BlockId(3), false));
    assert_eq!(stat("neighbors"), 1);
    assert!(!lamp.random_tick(&mut level, BlockPos::new(10, 64, 10), BlockStateId(5)));
    assert_eq!(stat("random"), 1);
    assert!(lamp.scheduled_tick(&mut level, BlockPos::new(10, 64, 10), BlockStateId(5)));
    assert_eq!(level.sets, vec![([10, 64, 10], "lit".to_owned(), "false".to_owned())]);
    // The same on a client's level calls nothing.
    let mut client_level = TestLevel { client: true, ..Default::default() };
    assert!(!lamp.scheduled_tick(&mut client_level, BlockPos::new(10, 64, 10), BlockStateId(5)));
    assert!(client_level.sets.is_empty());
    assert_eq!(stat("random"), 1);

    // Contact has no game access: the commands wait for the level's next tick and are applied there.
    lamp.entity_inside(&contact(false));
    lamp.step_on(&contact(true));
    assert_eq!((stat("inside"), stat("stepped")), (1, 0), "a client's contact calls nothing");
    lamp.step_on(&contact(false));
    assert_eq!(stat("stepped"), 1);
    let mut host = TestHost::default();
    drain(&mut host, "minecraft:the_nether");
    assert!(host.sets.is_empty(), "another level's tick does not take them");
    drain(&mut host, "minecraft:overworld");
    assert_eq!(host.sets, vec![([10, 64, 10], "lit".to_owned(), "true".to_owned(), 3)]);
    drain(&mut host, "minecraft:overworld");
    assert_eq!(host.sets.len(), 1, "applied once");
    reset();
}

#[test]
fn placed_and_removed_blocks_are_told_to_the_guest() {
    let _serial = lock();
    let Some(_guest) = ruby() else { return };
    let lamp = block("ruby:lamp");
    let dimension = crate::mod_api::LevelDimension::Overworld;
    lamp.placed(dimension, BlockPos::new(1, 2, 3), BlockStateId(5), BlockStateId(0));
    lamp.removed(dimension, BlockPos::new(1, 2, 3), BlockStateId(5), BlockStateId(0));
    lamp.removed(dimension, BlockPos::new(1, 2, 3), BlockStateId(5), BlockStateId(0));
    assert_eq!((stat("placed"), stat("removed")), (1, 2));
    reset();
}

#[test]
fn item_hooks_reach_the_guest_and_the_answer_leaves_the_stack() {
    let _serial = lock();
    let Some(_guest) = ruby() else { return };
    let wand = item_behaviors().into_iter().find(|b| b.id == "ruby:wand").expect("registered").behavior;
    let stack = ItemStack::new("ruby:wand", 1);
    let mut host = TestHost::default();
    let context = NativeItemUse { stack: &stack, hand: Hand::Main, client_side: false, hit: None, entity: None };
    assert_eq!(wand.use_item(&mut host, &context), NativeInteractionResult::Consume);
    assert_eq!(host.told, vec![(0x42, "The ruby wand hums.".to_owned())], "the wand told the player who used it");
    let with_hit = NativeItemUse {
        hit: Some(BlockHit { pos: [4, 70, 4], location: [4.5, 71.0, 4.5], face: Direction::Up }),
        ..context
    };
    assert_eq!(wand.use_on(&mut host, &with_hit), NativeInteractionResult::Success);
    assert_eq!(host.sets, vec![([4, 70, 4], "lit".to_owned(), "true".to_owned(), 3)], "the wand lit the lamp it was used on");
    let on_entity = NativeItemUse { entity: Some((9, "minecraft:cow")), hit: None, ..context };
    assert_eq!(wand.use_on_entity(&mut host, &on_entity), NativeInteractionResult::Fail);
    let mut result = Some(ItemStack::new("minecraft:glass_bottle", 1));
    wand.finish_using(&mut host, &context, &mut result);
    assert_eq!(result, Some(ItemStack::new("ruby:wand", 1)));
    wand.inventory_tick(&mut host, &stack, 2, true, false);
    wand.inventory_tick(&mut host, &stack, 2, true, true);
    assert_eq!(stat("wand_ticks"), 1, "a client's inventory tick calls nothing");
    // A use the client side only predicts is not asked.
    let client = NativeItemUse { client_side: true, ..context };
    assert_eq!(wand.use_item(&mut host, &client), NativeInteractionResult::Pass);
    assert_eq!((stat("wand_uses"), stat("wand_blocks"), stat("wand_entities"), stat("wand_finished")), (1, 1, 1, 1));
    reset();
}

#[test]
fn a_menu_the_guest_creates_stores_what_it_allows_and_reacts_to_clicks() {
    let _serial = lock();
    let Some(_guest) = ruby() else { return };
    let vault = menu_behaviors().into_iter().find(|b| b.id == "ruby:vault").expect("registered").behavior;
    let mut host = TestHost::default();
    assert!(vault.create(&mut host, 1, &[], true).is_none(), "a client only mirrors");
    let inventory = vault.create(&mut host, 0x1234, b"extra", false).expect("the guest opens it");
    assert_eq!(inventory.slots(), 9);
    assert_eq!(inventory.get_stack(0), Some(ItemStack::new("minecraft:emerald", 4)));
    // `may-place` is the guest's: the first slot takes emeralds only.
    assert!(inventory.may_place(0, &ItemStack::new("minecraft:emerald", 1)));
    assert!(!inventory.may_place(0, &ItemStack::new("minecraft:stick", 1)));
    assert!(inventory.may_place(1, &ItemStack::new("minecraft:stick", 1)));
    assert!(!inventory.may_place(9, &ItemStack::new("minecraft:emerald", 1)), "past the last slot");
    assert!(inventory.insert(0, ItemStack::new("minecraft:stick", 1), false).is_some(), "refused: nothing is stored");
    // A click is applied, then the guest crafts: four emeralds became a block.
    inventory.clicked(0, 0, 0);
    assert_eq!(inventory.get_stack(0), None);
    assert_eq!(inventory.get_stack(8), Some(ItemStack::new("minecraft:emerald_block", 1)));
    inventory.clicked(0, 0, 0);
    assert_eq!(inventory.get_stack(8), Some(ItemStack::new("minecraft:emerald_block", 1)), "nothing left to craft");
    // A guest with no `still-valid` hook never traps the player; `closed` reaches the guest.
    assert!(vault.still_valid(&mut host, 0x1234, inventory.as_ref()));
    vault.closed(&mut host, 0x1234, inventory.as_ref());
    assert_eq!(stat("vault_closed"), 1);
    reset();
}

#[test]
fn a_busy_or_missing_guest_leaves_the_object_as_it_was() {
    let _serial = lock();
    let Some(guest) = ruby() else { return };
    let lamp = block("ruby:lamp");
    let held = guest.server.state.lock().unwrap();
    let mut level = TestLevel::default();
    assert!(!lamp.scheduled_tick(&mut level, BlockPos::new(1, 1, 1), BlockStateId(5)));
    assert!(level.sets.is_empty());
    drop(held);
    assert!(lamp.scheduled_tick(&mut level, BlockPos::new(1, 1, 1), BlockStateId(5)));
    drop(guest);
    level.sets.clear();
    assert!(!lamp.scheduled_tick(&mut level, BlockPos::new(1, 1, 1), BlockStateId(5)), "the package is gone");
    assert!(level.sets.is_empty());
    reset();
}

#[test]
fn a_recipe_type_of_the_guest_decodes_matches_assembles_and_displays() {
    let _serial = lock();
    let Some(_guest) = ruby() else { return };
    let types = recipe_types();
    assert_eq!(types.len(), 1);
    let alloy = &types[0];
    assert_eq!((alloy.id.as_str(), alloy.version), ("ruby:alloy", 1));
    assert_eq!(alloy.category.id, "ruby:alloy");
    assert_eq!(alloy.category.recipe_type, "ruby:alloy");
    assert_eq!(alloy.category.icon, "minecraft:iron_ingot");
    let json = serde_json::json!({
        "type": "ruby:alloy", "version": 1,
        "inputs": ["minecraft:iron_ingot", "minecraft:gold_ingot"], "result": "minecraft:amethyst_shard", "count": 2
    });
    let data = alloy.codec.decode(&json).expect("the recipe decodes");
    let iron = ItemStack::new("minecraft:iron_ingot", 1);
    let gold = ItemStack::new("minecraft:gold_ingot", 1);
    assert!(alloy.codec.matches(&data, &[iron.clone(), gold.clone()]));
    assert!(alloy.codec.matches(&data, &[gold.clone(), iron.clone()]), "in any order");
    assert!(!alloy.codec.matches(&data, &[iron.clone()]));
    assert!(!alloy.codec.matches(&data, &[iron.clone(), iron.clone()]));
    assert_eq!(alloy.codec.assemble(&data, &[iron.clone(), gold.clone()]), vec![ItemStack::new("minecraft:amethyst_shard", 2)]);
    let display = alloy.codec.display(&data);
    assert_eq!(display.inputs, vec![vec![iron.clone()], vec![gold.clone()]]);
    assert_eq!(display.outputs, vec![ItemStack::new("minecraft:amethyst_shard", 2)]);
    // Data the guest cannot read matches nothing; data that is no hex string never reaches it.
    let broken = alloy.codec.decode(&serde_json::json!({ "type": "ruby:alloy", "version": 1 })).expect("kept as given");
    assert!(!alloy.codec.matches(&broken, &[iron.clone(), gold.clone()]));
    assert!(!alloy.codec.matches(&serde_json::json!("xyz"), &[iron]));
    reset();
}

#[test]
fn a_menu_asks_the_recipe_type_in_a_hook_that_has_game_access() {
    let _serial = lock();
    let Some(_guest) = ruby() else { return };
    let types = recipe_types();
    let json = serde_json::json!({
        "type": "ruby:alloy", "version": 1,
        "inputs": ["minecraft:iron_ingot", "minecraft:gold_ingot"], "result": "minecraft:amethyst_shard", "count": 2
    });
    let data = types[0].codec.decode(&json).unwrap();
    let mut host = TestHost { recipes: Some((types[0].codec.clone(), data)), ..Default::default() };
    let vault = menu_behaviors().into_iter().find(|b| b.id == "ruby:vault").expect("registered").behavior;
    let inventory = vault.create(&mut host, 1, &[], false).expect("opens");
    // The grid is the first two slots: emeralds are no alloy, so a shift-click leaves it to the engine.
    assert!(!vault.quick_move(&mut host, 1, inventory.as_ref(), 0, false, &ItemStack::new("minecraft:emerald", 4)));
    inventory.set_stack(0, Some(ItemStack::new("minecraft:iron_ingot", 1)));
    inventory.set_stack(1, Some(ItemStack::new("minecraft:gold_ingot", 1)));
    assert!(vault.quick_move(&mut host, 1, inventory.as_ref(), 0, false, &ItemStack::new("minecraft:iron_ingot", 1)));
    assert_eq!((inventory.get_stack(0), inventory.get_stack(1)), (None, None), "the grid was taken");
    assert_eq!(inventory.get_stack(8), Some(ItemStack::new("minecraft:amethyst_shard", 2)));
    reset();
}

#[test]
fn recipe_match_refuses_the_package_that_defines_the_type_and_recipe_data_serves_it() {
    use yoake_wasm::ServerHost as _;
    let _serial = lock();
    let Some(_guest) = ruby() else { return };
    let types = recipe_types();
    let json = serde_json::json!({
        "type": "ruby:alloy", "version": 1,
        "inputs": ["minecraft:iron_ingot", "minecraft:gold_ingot"], "result": "minecraft:amethyst_shard"
    });
    let data = types[0].codec.decode(&json).unwrap();
    assert_eq!(recipe_owner("ruby:alloy").as_deref(), Some("ruby"));
    assert_eq!(recipe_owner("ruby:none"), None);
    let mut own = super::super::full::GameServerHost { id: "ruby".to_owned(), commands: false };
    assert!(own.recipe_match_call(Vec::new(), "ruby:alloy".to_owned()).unwrap_err().contains("recipe-data"));
    // Another package may ask (it is another module; here there is no hook running, so the service says so).
    let mut other = super::super::full::GameServerHost { id: "other".to_owned(), commands: false };
    assert!(other.recipe_match_call(Vec::new(), "ruby:alloy".to_owned()).unwrap_err().contains("unsupported"));
    // The data the game keeps of a recipe reads back as bytes the guest can match with.
    let text = data.as_str().unwrap();
    assert_eq!(recipe_data_bytes(text).map(|bytes| bytes.starts_with(b"{")), Some(true));
    assert_eq!(recipe_data_bytes("zz"), None);
    reset();
}

#[test]
fn a_recipe_type_must_describe_itself_to_be_registered() {
    let _serial = lock();
    reset();
    let hooks = |names: &[&str]| names.iter().map(|name| (*name).to_owned()).collect::<Vec<_>>();
    let _init = super::super::full::ServerInitScope::enter("ruby");
    assert!(register("ruby", "recipe", "ruby:alloy", &hooks(&["matches", "assemble"])).is_err());
    assert!(register("ruby", "recipe", "ruby:alloy", &hooks(&["describe", "matches", "assemble"])).is_ok());
    reset();
}
