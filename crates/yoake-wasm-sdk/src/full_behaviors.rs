//! Behaviours of a package's own blocks, items and menus, written in a guest (REM-15): what a Forge mod
//! writes by subclassing `Block`, `Item` and `MenuType`.
//!
//! A server module registers each object it acts on from `init` (`ext::register_behavior`), then
//! answers the engine's `handle-call` for it. The wire format is described in the host's
//! `wasm_mods/behaviors.rs` and in `docs/yoake2/MOD-GUIDE.md`; this module reads the requests and builds
//! the answers, so a guest only implements the hooks it registered:
//!
//! ```ignore
//! use yoake_wasm_sdk::full::behaviors::{self, Behaviors, Block, BlockUse, BlockAnswer, Interaction};
//! use yoake_wasm_sdk::full::server::{ServerMod, ext, services};
//!
//! #[derive(Default)]
//! struct Guest;
//!
//! impl Behaviors for Guest {
//!     fn on_block_use(&mut self, call: &BlockUse) -> BlockAnswer {
//!         // flip the block's `lit` property with the ordinary services
//!         BlockAnswer::result(Interaction::Success)
//!     }
//! }
//!
//! impl ServerMod for Guest {
//!     fn subscriptions() -> &'static [&'static str] { &[] }
//!     fn init(&mut self) {
//!         ext::register_behavior("block", "ruby:lamp", &[behaviors::block::USE]).ok();
//!     }
//!     fn handle_call(&mut self, _caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
//!         behaviors::handle(self, &method, &payload).unwrap_or_else(|| Err("unknown method".into()))
//!     }
//! }
//! ```
use crate::prelude::*;

/// The caller the engine's behaviour calls come from.
pub const CALLER: &str = "yoake:engine";

/// The hook names of a block, for `register_behavior("block", ...)`.
pub mod block {
    pub const USE: &str = "use";
    pub const PLACED: &str = "placed";
    pub const REMOVED: &str = "removed";
    pub const NEIGHBOR_CHANGED: &str = "neighbor-changed";
    pub const RANDOM_TICK: &str = "random-tick";
    pub const SCHEDULED_TICK: &str = "scheduled-tick";
    pub const ENTITY_INSIDE: &str = "entity-inside";
    pub const STEP_ON: &str = "step-on";
}

/// The hook names of an item.
pub mod item {
    pub const USE: &str = "use";
    pub const USE_ON_BLOCK: &str = "use-on-block";
    pub const USE_ON_ENTITY: &str = "use-on-entity";
    pub const FINISH_USING: &str = "finish-using";
    pub const INVENTORY_TICK: &str = "inventory-tick";
}

/// The hook names of a menu.
pub mod menu {
    pub const CREATE: &str = "create";
    pub const MAY_PLACE: &str = "may-place";
    pub const STILL_VALID: &str = "still-valid";
    pub const CLOSED: &str = "closed";
    pub const CLICKED: &str = "clicked";
    pub const QUICK_MOVE: &str = "quick-move";
}

/// The hook names of a recipe type.
pub mod recipe {
    pub const DESCRIBE: &str = "describe";
    pub const DECODE: &str = "decode";
    pub const MATCHES: &str = "matches";
    pub const ASSEMBLE: &str = "assemble";
    pub const DISPLAY: &str = "display";
}

/// What the hook of an interaction answers: the engine's `InteractionResult`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Interaction {
    /// Not handled: the engine carries on as if the hook was not there.
    #[default]
    Pass,
    /// Handled; the hand swings.
    Success,
    /// Handled, without a swing.
    Consume,
    /// Refused.
    Fail,
}

impl Interaction {
    fn byte(self) -> u8 {
        match self {
            Self::Pass => 0,
            Self::Success => 1,
            Self::Consume => 2,
            Self::Fail => 3,
        }
    }
}

/// Which hand does it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Main,
    Off,
}

/// An item stack as the engine sends it: `components` is the network NBT of its components (empty for
/// the item's defaults).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stack {
    pub id: String,
    pub count: u32,
    pub components: Vec<u8>,
}

impl Stack {
    pub fn new(id: &str, count: u32) -> Self {
        Self { id: id.to_owned(), count, components: Vec::new() }
    }
}

/// A block, where it is and the state it is in.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Block {
    /// The registered id (`<package id>:<path>`).
    pub id: String,
    /// The level's id; empty when the hook does not know it.
    pub dimension: String,
    pub pos: [i32; 3],
    /// The block state's id (the engine's registry; read properties with `services::block_property`).
    pub state: u32,
}

/// A right click on the block.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockUse {
    pub block: Block,
    /// The face hit: down, up, north, south, west, east (0 to 5).
    pub face: u8,
    /// Where on the block (world coordinates).
    pub point: [f64; 3],
    pub hand: Hand,
    pub held: Option<Stack>,
}

/// What the engine does to the level after a hook (bounded: at most 64, within 32 blocks of the object).
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// `set_block_property`: the block at `pos` takes `value` for property `key`.
    SetProperty { pos: [i32; 3], key: String, value: String },
    /// Schedules a tick of `block` (empty: this block) at `pos` in `delay` ticks.
    ScheduleTick { pos: [i32; 3], block: String, delay: u32 },
    /// Plays a sound at a position.
    Sound { sound: String, at: [f64; 3], volume: f32, pitch: f32 },
}

/// The answer of a block hook.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockAnswer {
    pub result: Interaction,
    pub commands: Vec<Command>,
}

impl BlockAnswer {
    pub fn result(result: Interaction) -> Self {
        Self { result, commands: Vec::new() }
    }
    pub fn with(mut self, command: Command) -> Self {
        self.commands.push(command);
        self
    }
    fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.result.byte()).u16(self.commands.len().min(usize::from(u16::MAX)) as u16);
        for command in &self.commands {
            match command {
                Command::SetProperty { pos, key, value } => {
                    w.u8(1).i32(pos[0]).i32(pos[1]).i32(pos[2]).str(key).str(value);
                }
                Command::ScheduleTick { pos, block, delay } => {
                    w.u8(2).i32(pos[0]).i32(pos[1]).i32(pos[2]).str(block).u32(*delay);
                }
                Command::Sound { sound, at, volume, pitch } => {
                    w.u8(3).str(sound).f64(at[0]).f64(at[1]).f64(at[2]).f32(*volume).f32(*pitch);
                }
            }
        }
        w.finish()
    }
}

/// An item hook's call.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemCall {
    /// The registered item id.
    pub item: String,
    pub stack: Stack,
    pub hand: Hand,
}

/// A use of the item on a block.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemUseOnBlock {
    pub call: ItemCall,
    pub pos: [i32; 3],
    pub face: u8,
    pub point: [f64; 3],
}

/// A use of the item on an entity.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemUseOnEntity {
    pub call: ItemCall,
    pub entity: u32,
    pub kind: String,
}

/// An item's inventory tick.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemTick {
    pub call: ItemCall,
    pub slot: i32,
    pub selected: bool,
}

/// What `finish-using` leaves in the hand.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Leave {
    /// Whatever the engine leaves.
    #[default]
    Keep,
    /// This stack.
    Stack(Stack),
    /// Nothing.
    Empty,
}

/// The answer of an item hook.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ItemAnswer {
    pub result: Interaction,
    /// Only `finish-using` reads it.
    pub leave: Leave,
}

impl ItemAnswer {
    pub fn result(result: Interaction) -> Self {
        Self { result, leave: Leave::Keep }
    }
    fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(self.result.byte());
        match &self.leave {
            Leave::Keep => {
                w.u8(0);
            }
            Leave::Stack(stack) => {
                w.u8(1).stack(stack);
            }
            Leave::Empty => {
                w.u8(2);
            }
        }
        w.finish()
    }
}

/// A menu being opened: the contents the guest gives it.
#[derive(Clone, Debug, PartialEq)]
pub struct MenuOpen {
    pub menu: String,
    /// The player's UUID.
    pub player: String,
    /// What the opener passed along (`open_menu`'s extra bytes).
    pub extra: Vec<u8>,
}

/// The new menu's size and first contents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuContents {
    pub slots: u16,
    pub stacks: Vec<(u16, Stack)>,
}

/// What a click or a shift-click did, and what the guest wants changed in the menu's storage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuChange {
    /// `quick-move`: the guest took the move over (the engine does not run its own).
    pub handled: bool,
    /// Slots to set after the click (`None` empties a slot).
    pub set: Vec<(u16, Option<Stack>)>,
}

/// The hooks a guest implements; each does nothing by default. Register exactly the hooks you override.
pub trait Behaviors {
    fn on_block_use(&mut self, _call: &BlockUse) -> BlockAnswer {
        BlockAnswer::default()
    }
    /// The block was placed (or arrived by any change of block) where `previous` state was. No game
    /// access: answer commands.
    fn on_block_placed(&mut self, _block: &Block, _previous: u32) -> BlockAnswer {
        BlockAnswer::default()
    }
    /// The block left (broken, replaced, burnt) and `new` is the state there now.
    fn on_block_removed(&mut self, _block: &Block, _new: u32) -> BlockAnswer {
        BlockAnswer::default()
    }
    fn on_block_neighbor_changed(&mut self, _block: &Block, _changed: u32, _moved: bool) -> BlockAnswer {
        BlockAnswer::default()
    }
    fn on_block_random_tick(&mut self, _block: &Block) -> BlockAnswer {
        BlockAnswer::default()
    }
    fn on_block_scheduled_tick(&mut self, _block: &Block) -> BlockAnswer {
        BlockAnswer::default()
    }
    fn on_block_entity_inside(&mut self, _block: &Block, _entity: u32, _kind: &str) -> BlockAnswer {
        BlockAnswer::default()
    }
    fn on_block_step_on(&mut self, _block: &Block, _entity: u32, _kind: &str) -> BlockAnswer {
        BlockAnswer::default()
    }

    fn on_item_use(&mut self, _call: &ItemCall) -> ItemAnswer {
        ItemAnswer::default()
    }
    fn on_item_use_on_block(&mut self, _call: &ItemUseOnBlock) -> ItemAnswer {
        ItemAnswer::default()
    }
    fn on_item_use_on_entity(&mut self, _call: &ItemUseOnEntity) -> ItemAnswer {
        ItemAnswer::default()
    }
    fn on_item_finish_using(&mut self, _call: &ItemCall) -> ItemAnswer {
        ItemAnswer::default()
    }
    fn on_item_inventory_tick(&mut self, _call: &ItemTick) {}

    /// A player opens the menu: its size and first contents (`None` refuses to open it).
    fn on_menu_create(&mut self, _open: &MenuOpen) -> Option<MenuContents> {
        None
    }
    /// May `stack` go into `slot` (asked for every insertion the engine tries).
    fn on_menu_may_place(&mut self, _menu: &str, _slot: u16, _stack: &Stack) -> bool {
        true
    }
    /// Is the menu still usable by the player (asked every tick it is open).
    fn on_menu_still_valid(&mut self, _menu: &str, _player: &str) -> bool {
        true
    }
    /// The menu closed; `contents` is what its storage holds.
    fn on_menu_closed(&mut self, _menu: &str, _player: &str, _contents: &[(u16, Stack)]) {}
    /// A click in the menu's own slots was applied: `slot`, `button` and the click `mode` (the engine's
    /// `ClickType` ordinal). `contents` is the storage after it.
    fn on_menu_clicked(&mut self, _menu: &str, _player: &str, _slot: u16, _button: u8, _mode: u8, _contents: &[(u16, Stack)]) -> MenuChange {
        MenuChange::default()
    }
    /// A shift-click on `slot` of the menu (`inventory` true: a slot of the player's own inventory, by its
    /// index). `contents` is the storage.
    fn on_menu_quick_move(&mut self, _menu: &str, _player: &str, _slot: u16, _inventory: bool, _stack: &Stack, _contents: &[(u16, Stack)]) -> MenuChange {
        MenuChange::default()
    }

    /// A recipe type is asked what it is: the version its recipes carry (`"version"` in their JSON) and its
    /// recipe book category. Required of a registered recipe type.
    fn on_recipe_describe(&mut self, _recipe_type: &str) -> Option<RecipeDescription> {
        None
    }
    /// A recipe of the type was read (its JSON text): answer the data kept for it, or why it is refused.
    /// Without this hook the data is the JSON text itself.
    fn on_recipe_decode(&mut self, _recipe_type: &str, json: &str) -> Result<Vec<u8>, String> {
        Ok(json.as_bytes().to_vec())
    }
    /// Whether the crafting grid `input` makes the recipe whose data is `data`.
    fn on_recipe_matches(&mut self, _recipe_type: &str, _data: &[u8], _input: &[Stack]) -> bool {
        false
    }
    /// What the recipe makes of the grid.
    fn on_recipe_assemble(&mut self, _recipe_type: &str, _data: &[u8], _input: &[Stack]) -> Vec<Stack> {
        Vec::new()
    }
    /// What the recipe book shows: groups of alternatives per slot, and the outputs.
    fn on_recipe_display(&mut self, _recipe_type: &str, _data: &[u8]) -> RecipeDisplay {
        RecipeDisplay::default()
    }
}

/// What a recipe type says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecipeDescription {
    pub version: u32,
    /// The recipe book category's id (`<package id>:<path>`).
    pub category: String,
    /// The item that shows the category.
    pub icon: String,
}

/// A recipe as the recipe book shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecipeDisplay {
    /// One group per slot: the stacks that fit it.
    pub inputs: Vec<Vec<Stack>>,
    pub outputs: Vec<Stack>,
}

/// The `"key": value` pairs at the top of a JSON object, the values as their JSON text (strings keep
/// their quotes): enough to read a recipe without a JSON library.
pub fn json_fields(text: &str) -> Option<Vec<(String, String)>> {
    let b = text.trim().as_bytes();
    if b.first() != Some(&b'{') || b.last() != Some(&b'}') {
        return None;
    }
    let (mut out, mut i) = (Vec::new(), 1);
    while i < b.len() - 1 {
        while i < b.len() && matches!(b[i], b' ' | b',' | b'\n' | b'\r' | b'\t') {
            i += 1;
        }
        if i >= b.len() - 1 {
            break;
        }
        if b[i] != b'"' {
            return None;
        }
        let key_start = i + 1;
        i = key_start;
        while b.get(i)? != &b'"' {
            i += 1;
        }
        let key = core::str::from_utf8(&b[key_start..i]).ok()?.to_owned();
        i += 1;
        while b.get(i)? == &b' ' || b.get(i)? == &b':' {
            i += 1;
        }
        let value_start = i;
        let (mut depth, mut in_string) = (0i32, false);
        while i < b.len() - 1 {
            let c = b[i];
            if in_string {
                if c == b'\\' {
                    i += 1;
                } else if c == b'"' {
                    in_string = false;
                }
            } else {
                match c {
                    b'"' => in_string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => depth -= 1,
                    b',' if depth == 0 => break,
                    _ => {}
                }
            }
            i += 1;
        }
        out.push((key, core::str::from_utf8(&b[value_start..i]).ok()?.trim().to_owned()));
    }
    Some(out)
}

/// The strings of a JSON array of plain strings (`["a", "b"]`), or `None` for anything else.
pub fn json_strings(text: &str) -> Option<Vec<String>> {
    let inner = text.trim().strip_prefix('[')?.strip_suffix(']')?;
    inner
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(|part| part.strip_prefix('"')?.strip_suffix('"').map(String::from))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The wire
// ---------------------------------------------------------------------------------------------

/// The request/answer version byte.
pub const VERSION: u8 = 1;

struct Writer(Vec<u8>);

impl Writer {
    fn new() -> Self {
        Self(vec![VERSION])
    }
    fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(&mut self, v: f32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f64(&mut self, v: f64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn str(&mut self, v: &str) -> &mut Self {
        let bytes = &v.as_bytes()[..v.len().min(usize::from(u16::MAX))];
        self.u16(bytes.len() as u16);
        self.0.extend_from_slice(bytes);
        self
    }
    fn stack(&mut self, stack: &Stack) -> &mut Self {
        self.str(&stack.id).u32(stack.count).u32(stack.components.len() as u32);
        self.0.extend_from_slice(&stack.components);
        self
    }
    fn finish(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.0)
    }
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8]) -> Option<Self> {
        match bytes.split_first() {
            Some((&VERSION, rest)) => Some(Self(rest)),
            _ => None,
        }
    }
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = usize::from(self.u16()?);
        core::str::from_utf8(self.take(n)?).ok().map(String::from)
    }
    fn stack(&mut self) -> Option<Stack> {
        let id = self.str()?;
        let count = self.u32()?;
        let n = self.u32()? as usize;
        let components = self.take(n)?.to_vec();
        Some(Stack { id, count, components })
    }
    fn hand(&mut self) -> Option<Hand> {
        Some(if self.u8()? == 0 { Hand::Main } else { Hand::Off })
    }
    fn block(&mut self) -> Option<Block> {
        Some(Block {
            id: self.str()?,
            dimension: self.str()?,
            pos: [self.i32()?, self.i32()?, self.i32()?],
            state: self.u32()?,
        })
    }
    fn contents(&mut self) -> Option<Vec<(u16, Stack)>> {
        let _slots = self.u16()?;
        let count = usize::from(self.u16()?);
        let mut out = Vec::new();
        for _ in 0..count {
            out.push((self.u16()?, self.stack()?));
        }
        Some(out)
    }
}

/// Answers the engine's behaviour call `method` (`yoake:<kind>/<hook>`) with `payload`. `None` when
/// `method` is not a behaviour method (answer your own methods then); `Some(Err(..))` for a request this
/// SDK cannot read.
pub fn handle<B: Behaviors + ?Sized>(guest: &mut B, method: &str, payload: &[u8]) -> Option<Result<Vec<u8>, String>> {
    let hook = method.strip_prefix("yoake:")?;
    let (kind, hook) = hook.split_once('/')?;
    let bad = || Err(format!("a {kind}/{hook} request this SDK cannot read"));
    let result = match kind {
        "block" => block_call(guest, hook, payload),
        "item" => item_call(guest, hook, payload),
        "menu" => menu_call(guest, hook, payload),
        "recipe" => recipe_call(guest, hook, payload),
        _ => return None,
    };
    Some(result.ok_or(()).or_else(|()| bad()))
}

fn block_call<B: Behaviors + ?Sized>(guest: &mut B, hook: &str, payload: &[u8]) -> Option<Vec<u8>> {
    let mut r = Reader::new(payload)?;
    let block = r.block()?;
    let answer = match hook {
        "use" => {
            let face = r.u8()?;
            let point = [r.f64()?, r.f64()?, r.f64()?];
            let hand = r.hand()?;
            let held = if r.u8()? == 1 { Some(r.stack()?) } else { None };
            guest.on_block_use(&BlockUse { block, face, point, hand, held })
        }
        "placed" => guest.on_block_placed(&block, r.u32()?),
        "removed" => guest.on_block_removed(&block, r.u32()?),
        "neighbor-changed" => {
            let changed = r.u32()?;
            guest.on_block_neighbor_changed(&block, changed, r.u8()? != 0)
        }
        "random-tick" => guest.on_block_random_tick(&block),
        "scheduled-tick" => guest.on_block_scheduled_tick(&block),
        "entity-inside" => {
            let entity = r.u32()?;
            guest.on_block_entity_inside(&block, entity, &r.str()?)
        }
        "step-on" => {
            let entity = r.u32()?;
            guest.on_block_step_on(&block, entity, &r.str()?)
        }
        _ => return None,
    };
    Some(answer.encode())
}

fn item_call<B: Behaviors + ?Sized>(guest: &mut B, hook: &str, payload: &[u8]) -> Option<Vec<u8>> {
    let mut r = Reader::new(payload)?;
    let item = r.str()?;
    let stack = r.stack()?;
    let hand = r.hand()?;
    let call = ItemCall { item, stack, hand };
    let answer = match hook {
        "use" => guest.on_item_use(&call),
        "use-on-block" => {
            let pos = [r.i32()?, r.i32()?, r.i32()?];
            let face = r.u8()?;
            let point = [r.f64()?, r.f64()?, r.f64()?];
            guest.on_item_use_on_block(&ItemUseOnBlock { call, pos, face, point })
        }
        "use-on-entity" => {
            let entity = r.u32()?;
            guest.on_item_use_on_entity(&ItemUseOnEntity { call, entity, kind: r.str()? })
        }
        "finish-using" => guest.on_item_finish_using(&call),
        "inventory-tick" => {
            let slot = r.i32()?;
            guest.on_item_inventory_tick(&ItemTick { call, slot, selected: r.u8()? != 0 });
            ItemAnswer::default()
        }
        _ => return None,
    };
    Some(answer.encode())
}

fn menu_call<B: Behaviors + ?Sized>(guest: &mut B, hook: &str, payload: &[u8]) -> Option<Vec<u8>> {
    let mut r = Reader::new(payload)?;
    let mut w = Writer::new();
    match hook {
        "create" => {
            let menu = r.str()?;
            let player = r.str()?;
            let n = r.u32()? as usize;
            let extra = r.take(n)?.to_vec();
            let contents = guest.on_menu_create(&MenuOpen { menu, player, extra }).unwrap_or_default();
            // slots 0: the engine refuses to open it
            w.u16(contents.slots).u16(contents.stacks.len().min(usize::from(u16::MAX)) as u16);
            for (slot, stack) in &contents.stacks {
                w.u16(*slot).stack(stack);
            }
        }
        "may-place" => {
            let menu = r.str()?;
            let slot = r.u16()?;
            let stack = r.stack()?;
            w.u8(u8::from(guest.on_menu_may_place(&menu, slot, &stack)));
        }
        "still-valid" => {
            let menu = r.str()?;
            let player = r.str()?;
            w.u8(u8::from(guest.on_menu_still_valid(&menu, &player)));
        }
        "closed" => {
            let menu = r.str()?;
            let player = r.str()?;
            let contents = r.contents()?;
            guest.on_menu_closed(&menu, &player, &contents);
        }
        "clicked" => {
            let menu = r.str()?;
            let player = r.str()?;
            let slot = r.u16()?;
            let button = r.u8()?;
            let mode = r.u8()?;
            let contents = r.contents()?;
            write_change(&mut w, &guest.on_menu_clicked(&menu, &player, slot, button, mode, &contents));
        }
        "quick-move" => {
            let menu = r.str()?;
            let player = r.str()?;
            let slot = r.u16()?;
            let inventory = r.u8()? != 0;
            let stack = r.stack()?;
            let contents = r.contents()?;
            write_change(&mut w, &guest.on_menu_quick_move(&menu, &player, slot, inventory, &stack, &contents));
        }
        _ => return None,
    }
    Some(w.finish())
}

fn recipe_call<B: Behaviors + ?Sized>(guest: &mut B, hook: &str, payload: &[u8]) -> Option<Vec<u8>> {
    let mut r = Reader::new(payload)?;
    let recipe_type = r.str()?;
    let mut w = Writer::new();
    let items = |r: &mut Reader<'_>| -> Option<Vec<Stack>> {
        let count = usize::from(r.u16()?);
        (0..count).map(|_| r.stack()).collect()
    };
    let write_items = |w: &mut Writer, stacks: &[Stack]| {
        w.u16(stacks.len().min(64) as u16);
        for stack in stacks.iter().take(64) {
            w.stack(stack);
        }
    };
    match hook {
        "describe" => {
            let description = guest.on_recipe_describe(&recipe_type)?;
            w.u32(description.version).str(&description.category).str(&description.icon);
        }
        "decode" => {
            let n = r.u32()? as usize;
            let json = core::str::from_utf8(r.take(n)?).ok()?;
            match guest.on_recipe_decode(&recipe_type, json) {
                Ok(data) => {
                    w.u8(1).u32(data.len() as u32);
                    w.0.extend_from_slice(&data);
                }
                Err(message) => {
                    w.u8(0).str(&message);
                }
            }
        }
        "matches" | "assemble" => {
            let n = r.u32()? as usize;
            let data = r.take(n)?.to_vec();
            let input = items(&mut r)?;
            if hook == "matches" {
                w.u8(u8::from(guest.on_recipe_matches(&recipe_type, &data, &input)));
            } else {
                write_items(&mut w, &guest.on_recipe_assemble(&recipe_type, &data, &input));
            }
        }
        "display" => {
            let n = r.u32()? as usize;
            let data = r.take(n)?.to_vec();
            let display = guest.on_recipe_display(&recipe_type, &data);
            w.u16(display.inputs.len().min(64) as u16);
            for group in display.inputs.iter().take(64) {
                write_items(&mut w, group);
            }
            write_items(&mut w, &display.outputs);
        }
        _ => return None,
    }
    Some(w.finish())
}

fn write_change(w: &mut Writer, change: &MenuChange) {
    w.u8(u8::from(change.handled)).u16(change.set.len().min(usize::from(u16::MAX)) as u16);
    for (slot, stack) in &change.set {
        w.u16(*slot);
        match stack {
            Some(stack) => w.u8(1).stack(stack),
            None => w.u8(0),
        };
    }
}
