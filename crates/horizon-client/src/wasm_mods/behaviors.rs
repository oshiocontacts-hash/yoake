//! Behaviour objects of WebAssembly mods (REM-15): the way a Forge mod subclasses `Block`, `Item` and
//! `MenuType`, for the package's own content.
//!
//! A server module calls `behavior-register(kind, id, hooks)` from `init` for each content object it
//! wants to act on (`kind` is `block`, `item`, `menu` or `recipe`; `id` is `<package id>:<path>`; `hooks`
//! names what the guest implements). The game reads the table once, when it starts, and builds an
//! ordinary [`BlockBehavior`], [`ItemBehavior`] or [`MenuBehavior`] for each object: the same objects
//! native mods give, called from the same places in the engine, so the cost of every other block, item or
//! menu is exactly what it was (one table lookup that finds nothing, and none at all while no mod
//! registered anything). When one of those places is reached for a registered id, the engine calls the
//! package's `handle-call` export on the server module with the caller `yoake:engine` and the method
//! `yoake:<kind>/<hook>`, under the guest's own per-call limits. The wire format is below and in the SDK's
//! `behaviors` module.
//!
//! ```text
//! request            u8 version = 1, then the fields of the hook (little endian)
//! str                u16 length, UTF-8 bytes
//! item               str id, u32 count, u32 length + the components' network NBT
//! block              str block, str dimension ("" when unknown), i32 x, y, z, u32 state, then:
//!   use              u8 face (the order of Direction::ALL), f64 x, y, z (the hit), u8 hand (0 main, 1 off),
//!                    u8 held (0, or 1 and an item)
//!   placed           u32 previous state
//!   removed          u32 new state
//!   neighbor-changed u32 changed block, u8 moved
//!   random-tick, scheduled-tick   -
//!   entity-inside, step-on        u32 entity, str kind
//! item               str item, item stack, u8 hand, then:
//!   use              -
//!   use-on-block     i32 x, y, z, u8 face, f64 x, y, z
//!   use-on-entity    u32 entity, str kind
//!   finish-using     -
//!   inventory-tick   i32 slot, u8 selected
//! answer             u8 version = 1, u8 result (0 pass, 1 success, 2 consume, 3 fail), then:
//!   block hooks      u16 count of commands: 1 set-property (i32 x, y, z, str key, str value),
//!                    2 schedule-tick (i32 x, y, z, str block ("" is this block), u32 delay),
//!                    3 play-sound (str sound, f64 x, y, z, f32 volume, f32 pitch)
//!   item hooks       u8 stack (0 none, 1 an item follows, 2 an empty stack) - what finish-using leaves
//! ```
//!
//! A hook never sees another package's objects, nor an object that has no such hook registered. A guest
//! that is busy (a hook of its own caused this one), disabled, over its limits or that answers with
//! something that does not check, leaves the object doing what it does without the hook. A hook that has
//! no game access of its own (a tick the level runs, a block observed changing, an entity touching a
//! block) answers with commands that the engine applies, bounded and checked, instead of calling the
//! write services.
use super::full::{item_from, item_w, valid_item_id};
use super::server_services::endpoint;
use super::*;
use crate::mod_api::{
    BlockBehavior, BlockHit, BlockId, BlockPos, BlockStateId, Direction, Hand, ItemBehavior, ItemHandler,
    LevelHost, LevelWriteHost, MenuBehavior, NativeBlockBehavior, NativeBlockContact, NativeBlockUse, NativeInteractionResult,
    NativeItemBehavior, NativeItemUse, NativeMenuBehavior,
};
use std::collections::BTreeMap;
use yoake_wasm::full_bindings::client::yoake::mods::engine_types as et;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Who the package is told is calling.
pub(crate) const CALLER: &str = "yoake:engine";
const VERSION: u8 = 1;
/// Most objects one package registers.
const MAX_PER_PACKAGE: usize = 256;
/// Most commands one answer carries, and how far from the object they may reach.
const MAX_COMMANDS: usize = 64;
const MAX_REACH: i32 = 32;
/// Most commands waiting for the next tick of their level.
const MAX_PENDING: usize = 4096;
/// Longest key, value, block or sound name of a command.
const MAX_NAME: usize = 128;

pub(crate) const BLOCK_HOOKS: &[&str] = &[
    "use",
    "placed",
    "removed",
    "neighbor-changed",
    "random-tick",
    "scheduled-tick",
    "entity-inside",
    "step-on",
];
pub(crate) const ITEM_HOOKS: &[&str] = &["use", "use-on-block", "use-on-entity", "finish-using", "inventory-tick"];
pub(crate) const MENU_HOOKS: &[&str] = &["create", "may-place", "still-valid", "closed", "clicked", "quick-move"];
pub(crate) const RECIPE_HOOKS: &[&str] = &["describe", "decode", "matches", "assemble", "display"];

const B_USE: u32 = 1 << 0;
const B_PLACED: u32 = 1 << 1;
const B_REMOVED: u32 = 1 << 2;
const B_NEIGHBOR: u32 = 1 << 3;
const B_RANDOM: u32 = 1 << 4;
const B_SCHEDULED: u32 = 1 << 5;
const B_INSIDE: u32 = 1 << 6;
const B_STEP: u32 = 1 << 7;

const I_USE: u32 = 1 << 0;
const I_USE_ON: u32 = 1 << 1;
const I_USE_ENTITY: u32 = 1 << 2;
const I_FINISH: u32 = 1 << 3;
const I_TICK: u32 = 1 << 4;

const M_CREATE: u32 = 1 << 0;
const M_MAY_PLACE: u32 = 1 << 1;
const M_VALID: u32 = 1 << 2;
const M_CLOSED: u32 = 1 << 3;
const M_CLICKED: u32 = 1 << 4;
const M_QUICK: u32 = 1 << 5;

const R_DESCRIBE: u32 = 1 << 0;
const R_DECODE: u32 = 1 << 1;
const R_MATCHES: u32 = 1 << 2;
const R_ASSEMBLE: u32 = 1 << 3;
const R_DISPLAY: u32 = 1 << 4;

// ---------------------------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Block,
    Item,
    Menu,
    Recipe,
}

impl Kind {
    pub(crate) fn parse(text: &str) -> Result<Self, String> {
        Ok(match text {
            "block" => Self::Block,
            "item" => Self::Item,
            "menu" => Self::Menu,
            "recipe" => Self::Recipe,
            _ => return Err("a behaviour kind is block, item, menu or recipe".into()),
        })
    }

    fn hooks(self) -> &'static [&'static str] {
        match self {
            Self::Block => BLOCK_HOOKS,
            Self::Item => ITEM_HOOKS,
            Self::Menu => MENU_HOOKS,
            Self::Recipe => RECIPE_HOOKS,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    owner: String,
    hooks: u32,
}

#[derive(Default)]
struct Table {
    blocks: BTreeMap<String, Entry>,
    items: BTreeMap<String, Entry>,
    menus: BTreeMap<String, Entry>,
    recipes: BTreeMap<String, Entry>,
    /// The game has read the table: nothing new may be added (what a world start repeats is accepted).
    frozen: bool,
}

impl Table {
    fn of(&mut self, kind: Kind) -> &mut BTreeMap<String, Entry> {
        match kind {
            Kind::Block => &mut self.blocks,
            Kind::Item => &mut self.items,
            Kind::Menu => &mut self.menus,
            Kind::Recipe => &mut self.recipes,
        }
    }
}

static TABLE: Mutex<Table> = Mutex::new(Table {
    blocks: BTreeMap::new(),
    items: BTreeMap::new(),
    menus: BTreeMap::new(),
    recipes: BTreeMap::new(),
    frozen: false,
});
/// Something is registered: the engine's places for these objects look at the table only then.
static ANY: AtomicBool = AtomicBool::new(false);

fn table() -> std::sync::MutexGuard<'static, Table> {
    TABLE.lock().unwrap_or_else(|error| error.into_inner())
}

/// Whether any package registered a behaviour (one atomic load).
pub(crate) fn any() -> bool {
    ANY.load(Ordering::Relaxed)
}

fn own_id(owner: &str, id: &str) -> Result<(), String> {
    if !valid_item_id(id) {
        return Err(format!("not a content id: {id:.64}"));
    }
    match id.split_once(':') {
        Some((namespace, _)) if namespace == owner => Ok(()),
        _ => Err(format!("{id:.64} is not one of package {owner:.64}'s own objects (its ids start with `{owner:.64}:`)")),
    }
}

/// `behavior-register`: records that package `owner`'s guest implements `hooks` for object `id`. Only
/// from the module's `init`; the same call again (a new world starts the module afresh) changes nothing.
pub(crate) fn register(owner: &str, kind: &str, id: &str, hooks: &[String]) -> Result<(), String> {
    if !super::full::initializing_server(owner) {
        return Err("behaviours are registered from `init`: the game reads them once, when it starts".into());
    }
    let kind = Kind::parse(kind)?;
    own_id(owner, id)?;
    if hooks.is_empty() || hooks.len() > 32 {
        return Err("name between 1 and 32 hooks".into());
    }
    let known = kind.hooks();
    let mut mask = 0u32;
    for hook in hooks {
        let Some(index) = known.iter().position(|name| name == hook) else {
            return Err(format!("unknown {kind:?} hook {hook:.32} (they are {})", known.join(", ")));
        };
        if mask & (1 << index) != 0 {
            return Err(format!("hook {hook:.32} named twice"));
        }
        mask |= 1 << index;
    }
    if kind == Kind::Recipe && mask & (R_DESCRIBE | R_MATCHES | R_ASSEMBLE) != (R_DESCRIBE | R_MATCHES | R_ASSEMBLE) {
        return Err("a recipe type implements describe, matches and assemble".into());
    }
    let mut table = table();
    let frozen = table.frozen;
    let map = table.of(kind);
    let entry = Entry { owner: owner.to_owned(), hooks: mask };
    match map.get(id) {
        Some(existing) if existing.owner != owner => {
            return Err(format!("{id:.64} already has behaviour from package {:.64}", existing.owner));
        }
        Some(existing) if *existing == entry => return Ok(()),
        Some(_) if frozen => {
            return Err("the game already started with other hooks for this object; restart the game to change them".into());
        }
        None if frozen => {
            return Err("the game already started: register behaviours in `init`, which runs before it".into());
        }
        None if map.values().filter(|e| e.owner == owner).count() >= MAX_PER_PACKAGE => {
            return Err(format!("a package registers at most {MAX_PER_PACKAGE} objects of a kind"));
        }
        _ => {}
    }
    map.insert(id.to_owned(), entry);
    ANY.store(true, Ordering::Relaxed);
    Ok(())
}

/// Forgets everything (tests).
#[cfg(test)]
pub(crate) fn reset() {
    *table() = Table::default();
    ANY.store(false, Ordering::Relaxed);
    PENDING.lock().unwrap_or_else(|error| error.into_inner()).clear();
}

fn snapshot(kind: Kind) -> Vec<(String, Entry)> {
    let mut table = table();
    table.frozen = true;
    table.of(kind).iter().map(|(id, entry)| (id.clone(), entry.clone())).collect()
}

/// The block behaviours of the registered blocks, for the game's start.
pub(crate) fn block_behaviors() -> Vec<NativeBlockBehavior> {
    snapshot(Kind::Block)
        .into_iter()
        .map(|(id, entry)| NativeBlockBehavior {
            behavior: Arc::new(WasmBlock { owner: entry.owner, id: id.clone(), hooks: entry.hooks }),
            id,
        })
        .collect()
}

/// The item behaviours of the registered items.
pub(crate) fn item_behaviors() -> Vec<NativeItemBehavior> {
    snapshot(Kind::Item)
        .into_iter()
        .map(|(id, entry)| NativeItemBehavior {
            behavior: Arc::new(WasmItem { owner: entry.owner, id: id.clone(), hooks: entry.hooks }),
            id,
        })
        .collect()
}

/// The menu behaviours of the registered menus.
pub(crate) fn menu_behaviors() -> Vec<NativeMenuBehavior> {
    snapshot(Kind::Menu)
        .into_iter()
        .map(|(id, entry)| NativeMenuBehavior {
            behavior: Arc::new(WasmMenu { owner: entry.owner, id: id.clone(), hooks: entry.hooks }),
            id,
        })
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The wire
// ---------------------------------------------------------------------------------------------

pub(crate) struct Enc(Vec<u8>);

impl Enc {
    pub(crate) fn new() -> Self {
        Self(vec![VERSION])
    }
    pub(crate) fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }
    pub(crate) fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub(crate) fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub(crate) fn i32(&mut self, v: i32) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub(crate) fn f64(&mut self, v: f64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    pub(crate) fn str(&mut self, v: &str) -> &mut Self {
        let bytes = &v.as_bytes()[..v.len().min(usize::from(u16::MAX))];
        self.u16(bytes.len() as u16);
        self.0.extend_from_slice(bytes);
        self
    }
    pub(crate) fn item(&mut self, stack: &ItemStack) -> &mut Self {
        let wire = item_w(stack);
        self.str(&wire.id).u32(wire.count).u32(wire.components.len() as u32);
        self.0.extend_from_slice(&wire.components);
        self
    }
    pub(crate) fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

pub(crate) struct Dec<'a>(&'a [u8]);

impl<'a> Dec<'a> {
    /// Checks the version byte.
    pub(crate) fn new(bytes: &'a [u8]) -> Option<Self> {
        match bytes.split_first() {
            Some((&VERSION, rest)) => Some(Self(rest)),
            _ => None,
        }
    }
    pub(crate) fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Some(head)
    }
    pub(crate) fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    pub(crate) fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    pub(crate) fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub(crate) fn i32(&mut self) -> Option<i32> {
        Some(i32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    pub(crate) fn f64(&mut self) -> Option<f64> {
        let v = f64::from_le_bytes(self.take(8)?.try_into().ok()?);
        v.is_finite().then_some(v)
    }
    pub(crate) fn f32(&mut self) -> Option<f32> {
        let v = f32::from_le_bytes(self.take(4)?.try_into().ok()?);
        v.is_finite().then_some(v)
    }
    pub(crate) fn str(&mut self) -> Option<String> {
        let n = usize::from(self.u16()?);
        std::str::from_utf8(self.take(n)?).ok().map(str::to_owned)
    }
    pub(crate) fn item(&mut self) -> Option<ItemStack> {
        let id = self.str()?;
        let count = self.u32()?;
        let n = self.u32()? as usize;
        if n > 64 * 1024 {
            return None;
        }
        let components = self.take(n)?.to_vec();
        item_from(&et::ItemStack { id, count, components })
    }
    pub(crate) fn done(&self) -> bool {
        self.0.is_empty()
    }
}

/// What a guest's answer asks the engine to do to the level.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    SetProperty { pos: [i32; 3], key: String, value: String },
    ScheduleTick { pos: [i32; 3], block: String, delay: u32 },
    Sound { sound: String, at: [f64; 3], volume: f32, pitch: f32 },
}

#[derive(Debug, PartialEq)]
pub(crate) struct Answer {
    pub(crate) result: NativeInteractionResult,
    pub(crate) commands: Vec<Command>,
}

impl Answer {
    /// Reads and checks a block hook's answer; every position must lie within [`MAX_REACH`] of `origin`.
    pub(crate) fn decode(bytes: &[u8], origin: [i32; 3]) -> Option<Self> {
        let mut d = Dec::new(bytes)?;
        let result = match d.u8()? {
            0 => NativeInteractionResult::Pass,
            1 => NativeInteractionResult::Success,
            2 => NativeInteractionResult::Consume,
            3 => NativeInteractionResult::Fail,
            _ => return None,
        };
        let count = usize::from(d.u16()?);
        if count > MAX_COMMANDS {
            return None;
        }
        let near = |pos: [i32; 3]| pos.iter().zip(origin).all(|(a, b)| a.abs_diff(b) <= MAX_REACH as u32);
        let name = |d: &mut Dec<'_>| d.str().filter(|text| text.len() <= MAX_NAME);
        let mut commands = Vec::with_capacity(count);
        for _ in 0..count {
            let command = match d.u8()? {
                1 => {
                    let pos = [d.i32()?, d.i32()?, d.i32()?];
                    let (key, value) = (name(&mut d)?, name(&mut d)?);
                    near(pos).then_some(())?;
                    Command::SetProperty { pos, key, value }
                }
                2 => {
                    let pos = [d.i32()?, d.i32()?, d.i32()?];
                    let block = name(&mut d)?;
                    let delay = d.u32()?;
                    near(pos).then_some(())?;
                    (delay <= 1_000_000).then_some(())?;
                    Command::ScheduleTick { pos, block, delay }
                }
                3 => {
                    let sound = name(&mut d)?;
                    let at = [d.f64()?, d.f64()?, d.f64()?];
                    let (volume, pitch) = (d.f32()?, d.f32()?);
                    Command::Sound { sound, at, volume: volume.clamp(0.0, 16.0), pitch: pitch.clamp(0.0, 2.0) }
                }
                _ => return None,
            };
            commands.push(command);
        }
        d.done().then_some(Self { result, commands })
    }
}

// ---------------------------------------------------------------------------------------------
// Calling the guest
// ---------------------------------------------------------------------------------------------

static BUSY: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicBool = AtomicBool::new(false);

/// One `handle-call` into package `owner`'s server module with the game access of this hook. `None` when
/// the module is not there, busy (this hook was caused by one of its own), disabled, trapped, over its
/// limits or answered an error: the object then behaves as if it had no such hook.
pub(crate) fn invoke(owner: &str, method: &str, payload: &[u8], host: HostAccess<'_>) -> Option<Vec<u8>> {
    let slot = endpoint(owner)?;
    let mut state = match slot.state.try_lock() {
        Ok(state) => state,
        Err(std::sync::TryLockError::WouldBlock) => {
            if BUSY.fetch_add(1, Ordering::Relaxed) == 0 {
                log::warn!("wasm mod {owner}: {method} dropped, the module was busy (a hook of its own caused it); further drops are only counted");
            }
            return None;
        }
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
    };
    let module = state.module.as_mut()?;
    slot.dirty.store(true, Ordering::Relaxed);
    let mut invocation = Invocation { host, batch: None, batch_start: 0, client: ClientExtras::default() };
    let answer = {
        let _scope = Scope::enter(&mut invocation);
        module.handle_call(CALLER, method, payload)
    };
    match answer? {
        Ok(bytes) if bytes.len() <= super::server_services::MAX_BLOB => Some(bytes),
        Ok(_) => None,
        Err(error) => {
            if !FAILED.swap(true, Ordering::Relaxed) {
                log::warn!("wasm mod {owner}: {method} failed: {error:.200} (further failures are logged at debug level)");
            } else {
                log::debug!("wasm mod {owner}: {method} failed: {error:.200}");
            }
            None
        }
    }
}

fn direction_index(direction: Direction) -> u8 {
    Direction::ALL.iter().position(|d| *d == direction).unwrap_or(0) as u8
}

fn hand_index(hand: Hand) -> u8 {
    match hand {
        Hand::Main => 0,
        Hand::Off => 1,
    }
}

fn level_point(origin: [i32; 3]) -> [f64; 3] {
    origin.map(|c| f64::from(c) + 0.5)
}

// ---------------------------------------------------------------------------------------------
// Blocks
// ---------------------------------------------------------------------------------------------

/// What a pending block command waits for: the level of this id (`None`: the next level that ticks).
struct Pending {
    owner: String,
    dimension: Option<String>,
    command: Command,
}

static PENDING: Mutex<Vec<Pending>> = Mutex::new(Vec::new());
static ANY_PENDING: AtomicBool = AtomicBool::new(false);

fn queue(owner: &str, dimension: Option<&str>, commands: Vec<Command>) {
    if commands.is_empty() {
        return;
    }
    let mut pending = PENDING.lock().unwrap_or_else(|error| error.into_inner());
    for command in commands {
        if pending.len() >= MAX_PENDING {
            log::debug!("wasm mod {owner}: too many block commands waiting; dropped");
            break;
        }
        pending.push(Pending { owner: owner.to_owned(), dimension: dimension.map(str::to_owned), command });
    }
    ANY_PENDING.store(true, Ordering::Relaxed);
}

/// Applies the commands the hooks without game access of their own left, when the level `dimension` is
/// about to tick (`native_level_tick_pre`). One atomic load while there are none.
pub(crate) fn drain(host: &mut dyn Host, dimension: &str) {
    if !ANY_PENDING.load(Ordering::Relaxed) {
        return;
    }
    let taken: Vec<Pending> = {
        let mut pending = PENDING.lock().unwrap_or_else(|error| error.into_inner());
        let (now, later): (Vec<_>, Vec<_>) =
            pending.drain(..).partition(|p| p.dimension.as_deref().is_none_or(|d| d == dimension));
        *pending = later;
        ANY_PENDING.store(!pending.is_empty(), Ordering::Relaxed);
        now
    };
    for pending in taken {
        match pending.command {
            Command::SetProperty { pos, key, value } => {
                host.set_block_property_with(pos, &key, &value, 3);
            }
            other => log::debug!("wasm mod {}: {other:?} needs a level that ticks; dropped", pending.owner),
        }
    }
}

fn dimension_name(dimension: crate::mod_api::LevelDimension) -> &'static str {
    match dimension {
        crate::mod_api::LevelDimension::Overworld => "minecraft:overworld",
        crate::mod_api::LevelDimension::Nether => "minecraft:the_nether",
        crate::mod_api::LevelDimension::End => "minecraft:the_end",
    }
}

struct WasmBlock {
    owner: String,
    id: String,
    hooks: u32,
}

impl WasmBlock {
    fn has(&self, hook: u32) -> bool {
        self.hooks & hook != 0
    }

    fn request(&self, dimension: &str, pos: [i32; 3], state: BlockStateId) -> Enc {
        let mut enc = Enc::new();
        enc.str(&self.id).str(dimension).i32(pos[0]).i32(pos[1]).i32(pos[2]).u32(state.0);
        enc
    }

    fn call(&self, hook: &str, payload: &[u8], pos: [i32; 3], host: HostAccess<'_>) -> Option<Answer> {
        let bytes = invoke(&self.owner, &format!("yoake:block/{hook}"), payload, host)?;
        let answer = Answer::decode(&bytes, pos);
        if answer.is_none() {
            log::debug!("wasm mod {}: the answer of block/{hook} for {} does not check", self.owner, self.id);
        }
        answer
    }

    /// A hook of the level's ticks: the answer's commands go straight to the level.
    fn level_hook(
        &self,
        level: &mut dyn LevelWriteHost,
        hook: &str,
        hook_bit: u32,
        pos: BlockPos,
        payload: Enc,
    ) -> bool {
        if !self.has(hook_bit) || level.is_client_side() {
            return false;
        }
        let at = [pos.x, pos.y, pos.z];
        let mut payload = payload;
        let Some(answer) = self.call(hook, &payload.finish(), at, HostAccess::None) else {
            return false;
        };
        let touched = !answer.commands.is_empty();
        for command in answer.commands {
            match command {
                Command::SetProperty { pos, key, value } => {
                    level.set_block_property(pos, &key, &value, 3);
                }
                Command::ScheduleTick { pos, block, delay } => {
                    let block = if block.is_empty() { self.id.as_str() } else { block.as_str() };
                    level.schedule_block_tick(pos, block, delay.try_into().unwrap_or(i32::MAX));
                }
                Command::Sound { sound, at, volume, pitch } => {
                    level.play_sound(&sound, at, 0, volume, pitch);
                }
            }
        }
        touched || answer.result != NativeInteractionResult::Pass
    }

    fn contact(&self, hook: &str, hook_bit: u32, context: &NativeBlockContact<'_>) {
        if !self.has(hook_bit) || context.client_side {
            return;
        }
        let pos = [context.position.x, context.position.y, context.position.z];
        let mut payload = self.request(context.dimension, pos, context.state);
        payload.u32(context.entity as u32).str(context.kind);
        if let Some(answer) = self.call(hook, &payload.finish(), pos, HostAccess::None) {
            queue(&self.owner, Some(context.dimension), answer.commands);
        }
    }
}

impl BlockBehavior for WasmBlock {
    fn randomly_ticking(&self) -> bool {
        self.has(B_RANDOM)
    }

    fn use_block(&self, host: &mut dyn Host, context: &NativeBlockUse<'_>) -> NativeInteractionResult {
        if !self.has(B_USE) || context.client_side {
            return NativeInteractionResult::Pass;
        }
        let mut payload = self.request("", context.hit.pos, context.state);
        payload
            .u8(direction_index(context.hit.face))
            .f64(context.hit.location[0])
            .f64(context.hit.location[1])
            .f64(context.hit.location[2])
            .u8(hand_index(context.hand));
        match context.stack {
            Some(stack) => payload.u8(1).item(stack),
            None => payload.u8(0),
        };
        let Some(answer) = self.call("use", &payload.finish(), context.hit.pos, HostAccess::Write(host)) else {
            return NativeInteractionResult::Pass;
        };
        queue(&self.owner, None, answer.commands);
        answer.result
    }

    fn neighbor_changed(
        &self,
        level: &mut dyn LevelWriteHost,
        position: BlockPos,
        state: BlockStateId,
        changed: BlockId,
        moved: bool,
    ) -> bool {
        if !self.has(B_NEIGHBOR) {
            return false;
        }
        let mut payload = self.request("", [position.x, position.y, position.z], state);
        payload.u32(changed.0 as u32).u8(u8::from(moved));
        self.level_hook(level, "neighbor-changed", B_NEIGHBOR, position, payload)
    }

    fn random_tick(&self, level: &mut dyn LevelWriteHost, position: BlockPos, state: BlockStateId) -> bool {
        let payload = self.request("", [position.x, position.y, position.z], state);
        self.level_hook(level, "random-tick", B_RANDOM, position, payload)
    }

    fn scheduled_tick(&self, level: &mut dyn LevelWriteHost, position: BlockPos, state: BlockStateId) -> bool {
        let payload = self.request("", [position.x, position.y, position.z], state);
        self.level_hook(level, "scheduled-tick", B_SCHEDULED, position, payload)
    }

    fn entity_inside(&self, context: &NativeBlockContact<'_>) {
        self.contact("entity-inside", B_INSIDE, context);
    }

    fn step_on(&self, context: &NativeBlockContact<'_>) {
        self.contact("step-on", B_STEP, context);
    }

    fn placed(&self, dimension: crate::mod_api::LevelDimension, position: BlockPos, state: BlockStateId, previous: BlockStateId) {
        if !self.has(B_PLACED) {
            return;
        }
        let name = dimension_name(dimension);
        let at = [position.x, position.y, position.z];
        let mut payload = self.request(name, at, state);
        payload.u32(previous.0);
        if let Some(answer) = self.call("placed", &payload.finish(), at, HostAccess::None) {
            queue(&self.owner, Some(name), answer.commands);
        }
    }

    fn removed(&self, dimension: crate::mod_api::LevelDimension, position: BlockPos, state: BlockStateId, new: BlockStateId) {
        if !self.has(B_REMOVED) {
            return;
        }
        let name = dimension_name(dimension);
        let at = [position.x, position.y, position.z];
        let mut payload = self.request(name, at, state);
        payload.u32(new.0);
        if let Some(answer) = self.call("removed", &payload.finish(), at, HostAccess::None) {
            queue(&self.owner, Some(name), answer.commands);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Items
// ---------------------------------------------------------------------------------------------

struct WasmItem {
    owner: String,
    id: String,
    hooks: u32,
}

impl WasmItem {
    fn has(&self, hook: u32) -> bool {
        self.hooks & hook != 0
    }

    fn request(&self, stack: &ItemStack, hand: Hand) -> Enc {
        let mut enc = Enc::new();
        enc.str(&self.id).item(stack).u8(hand_index(hand));
        enc
    }

    /// The answer's result and, for a hook that has one, the stack it leaves.
    fn call(
        &self,
        hook: &str,
        payload: &[u8],
        host: &mut dyn Host,
    ) -> Option<(NativeInteractionResult, Option<Option<ItemStack>>)> {
        let bytes = invoke(&self.owner, &format!("yoake:item/{hook}"), payload, HostAccess::Write(host))?;
        let mut d = Dec::new(&bytes)?;
        let result = match d.u8()? {
            0 => NativeInteractionResult::Pass,
            1 => NativeInteractionResult::Success,
            2 => NativeInteractionResult::Consume,
            3 => NativeInteractionResult::Fail,
            _ => return None,
        };
        let stack = match d.u8()? {
            0 => None,
            1 => Some(Some(d.item()?)),
            2 => Some(None),
            _ => return None,
        };
        d.done().then_some((result, stack))
    }
}

impl ItemBehavior for WasmItem {
    fn use_item(&self, host: &mut dyn Host, context: &NativeItemUse<'_>) -> NativeInteractionResult {
        if !self.has(I_USE) || context.client_side {
            return NativeInteractionResult::Pass;
        }
        let payload = self.request(context.stack, context.hand).finish();
        self.call("use", &payload, host).map_or(NativeInteractionResult::Pass, |(result, _)| result)
    }

    fn use_on(&self, host: &mut dyn Host, context: &NativeItemUse<'_>) -> NativeInteractionResult {
        let (true, false, Some(hit)) = (self.has(I_USE_ON), context.client_side, context.hit.as_ref()) else {
            return NativeInteractionResult::Pass;
        };
        let BlockHit { pos, location, face } = hit;
        let mut payload = self.request(context.stack, context.hand);
        payload
            .i32(pos[0])
            .i32(pos[1])
            .i32(pos[2])
            .u8(direction_index(*face))
            .f64(location[0])
            .f64(location[1])
            .f64(location[2]);
        self.call("use-on-block", &payload.finish(), host).map_or(NativeInteractionResult::Pass, |(result, _)| result)
    }

    fn use_on_entity(&self, host: &mut dyn Host, context: &NativeItemUse<'_>) -> NativeInteractionResult {
        let (true, false, Some((entity, kind))) = (self.has(I_USE_ENTITY), context.client_side, context.entity) else {
            return NativeInteractionResult::Pass;
        };
        let mut payload = self.request(context.stack, context.hand);
        payload.u32(entity).str(kind);
        self.call("use-on-entity", &payload.finish(), host).map_or(NativeInteractionResult::Pass, |(result, _)| result)
    }

    fn finish_using(&self, host: &mut dyn Host, context: &NativeItemUse<'_>, result: &mut Option<ItemStack>) {
        if !self.has(I_FINISH) || context.client_side {
            return;
        }
        let payload = self.request(context.stack, context.hand).finish();
        if let Some((_, Some(stack))) = self.call("finish-using", &payload, host) {
            *result = stack;
        }
    }

    fn inventory_tick(&self, host: &mut dyn Host, stack: &ItemStack, slot: i32, selected: bool, client_side: bool) {
        if !self.has(I_TICK) || client_side {
            return;
        }
        let mut payload = self.request(stack, Hand::Main);
        payload.i32(slot).u8(u8::from(selected));
        self.call("inventory-tick", &payload.finish(), host);
    }
}

// ---------------------------------------------------------------------------------------------
// Menus
// ---------------------------------------------------------------------------------------------

struct WasmMenu {
    owner: String,
    id: String,
    hooks: u32,
}

impl MenuBehavior for WasmMenu {
    fn create(&self, host: &mut dyn Host, player: u128, extra: &[u8], client_side: bool) -> Option<Arc<dyn ItemHandler>> {
        // A menu's contents live on the server: a client only mirrors what it is sent.
        if client_side || self.hooks & M_CREATE == 0 {
            return None;
        }
        let mut payload = Enc::new();
        payload.str(&self.id).str(&super::full::uuid_text(player)).u32(extra.len() as u32);
        payload.0.extend_from_slice(extra);
        let bytes = invoke(&self.owner, "yoake:menu/create", &payload.finish(), HostAccess::Write(host))?;
        let mut d = Dec::new(&bytes)?;
        let slots = usize::from(d.u16()?);
        let count = usize::from(d.u16()?);
        if slots > 256 || count > slots {
            return None;
        }
        let mut contents = vec![None; slots];
        for _ in 0..count {
            let slot = usize::from(d.u16()?);
            *contents.get_mut(slot)? = Some(d.item()?);
        }
        d.done().then_some(())?;
        Some(Arc::new(WasmInventory {
            owner: self.owner.clone(),
            menu: self.id.clone(),
            player: super::full::uuid_text(player),
            hooks: self.hooks,
            contents: Mutex::new(contents),
        }))
    }

    fn quick_move(
        &self,
        host: &mut dyn Host,
        player: u128,
        inventory: &dyn ItemHandler,
        slot: usize,
        from_inventory: bool,
        stack: &ItemStack,
    ) -> bool {
        if self.hooks & M_QUICK == 0 {
            return false;
        }
        let mut payload = Enc::new();
        payload.str(&self.id).str(&super::full::uuid_text(player)).u16(slot as u16).u8(u8::from(from_inventory)).item(stack);
        write_contents(&mut payload, inventory);
        let Some(bytes) = invoke(&self.owner, "yoake:menu/quick-move", &payload.finish(), HostAccess::Write(host)) else {
            return false;
        };
        match decode_change(&bytes, inventory.slots()) {
            Some((handled, sets)) => {
                apply_sets(inventory, sets);
                handled
            }
            None => false,
        }
    }

    fn still_valid(&self, host: &mut dyn Host, player: u128, _inventory: &dyn ItemHandler) -> bool {
        if self.hooks & M_VALID == 0 {
            return true;
        }
        let mut payload = Enc::new();
        payload.str(&self.id).str(&super::full::uuid_text(player));
        invoke(&self.owner, "yoake:menu/still-valid", &payload.finish(), HostAccess::Write(host))
            .and_then(|bytes| {
                let mut d = Dec::new(&bytes)?;
                let valid = d.u8()?;
                d.done().then_some(valid != 0)
            })
            // A guest that cannot answer must not trap the player in a menu.
            .unwrap_or(true)
    }

    fn closed(&self, host: &mut dyn Host, player: u128, inventory: &dyn ItemHandler) {
        if self.hooks & M_CLOSED == 0 {
            return;
        }
        let mut payload = Enc::new();
        payload.str(&self.id).str(&super::full::uuid_text(player));
        write_contents(&mut payload, inventory);
        invoke(&self.owner, "yoake:menu/closed", &payload.finish(), HostAccess::Write(host));
    }
}

/// A guest's `menu-change` answer: whether it took the move over, and the slots it sets.
type Change = (bool, Vec<(usize, Option<ItemStack>)>);

fn decode_change(bytes: &[u8], slots: usize) -> Option<Change> {
    let mut d = Dec::new(bytes)?;
    let handled = d.u8()? != 0;
    let count = usize::from(d.u16()?);
    if count > slots {
        return None;
    }
    let mut sets = Vec::with_capacity(count);
    for _ in 0..count {
        let slot = usize::from(d.u16()?);
        if slot >= slots {
            return None;
        }
        let stack = match d.u8()? {
            0 => None,
            1 => Some(d.item()?),
            _ => return None,
        };
        sets.push((slot, stack));
    }
    d.done().then_some((handled, sets))
}

fn apply_sets(inventory: &dyn ItemHandler, sets: Vec<(usize, Option<ItemStack>)>) {
    for (slot, stack) in sets {
        inventory.set_stack(slot, stack);
    }
}

fn write_contents(payload: &mut Enc, inventory: &dyn ItemHandler) {
    let filled: Vec<(usize, ItemStack)> =
        (0..inventory.slots()).filter_map(|slot| inventory.get_stack(slot).map(|stack| (slot, stack))).collect();
    payload.u16(inventory.slots() as u16).u16(filled.len() as u16);
    for (slot, stack) in &filled {
        payload.u16(*slot as u16).item(stack);
    }
}

/// The storage of a menu the guest created: a plain slot list the engine's menu moves stacks through;
/// the guest decides what may go where (`may-place`) and reacts in `clicked` and `quick-move`.
struct WasmInventory {
    owner: String,
    menu: String,
    /// The player the menu was opened for.
    player: String,
    hooks: u32,
    contents: Mutex<Vec<Option<ItemStack>>>,
}

impl ItemHandler for WasmInventory {
    fn slots(&self) -> usize {
        self.contents.lock().unwrap_or_else(|error| error.into_inner()).len()
    }

    fn get_stack(&self, slot: usize) -> Option<ItemStack> {
        self.contents.lock().unwrap_or_else(|error| error.into_inner()).get(slot).cloned().flatten()
    }

    fn set_stack(&self, slot: usize, stack: Option<ItemStack>) {
        if let Some(place) = self.contents.lock().unwrap_or_else(|error| error.into_inner()).get_mut(slot) {
            *place = stack;
        }
    }

    fn stack_limit(&self, stack: &ItemStack) -> u32 {
        horizon_items::items::published_stack_limit(&stack.item).flatten().unwrap_or(64).clamp(1, 99)
    }

    fn clicked(&self, slot: i32, button: i32, mode: i32) {
        if self.hooks & M_CLICKED == 0 {
            return;
        }
        let mut payload = Enc::new();
        payload.str(&self.menu).str(&self.player).u16(slot.clamp(0, i32::from(u16::MAX)) as u16).u8(button.clamp(0, 255) as u8).u8(mode.clamp(0, 255) as u8);
        write_contents(&mut payload, self);
        let answer = invoke(&self.owner, "yoake:menu/clicked", &payload.finish(), HostAccess::None)
            .and_then(|bytes| decode_change(&bytes, self.slots()));
        if let Some((_, sets)) = answer {
            apply_sets(self, sets);
        }
    }

    fn may_place(&self, slot: usize, stack: &ItemStack) -> bool {
        if slot >= self.slots() {
            return false;
        }
        if self.hooks & M_MAY_PLACE == 0 {
            return true;
        }
        let mut payload = Enc::new();
        payload.str(&self.menu).u16(slot as u16).item(stack);
        invoke(&self.owner, "yoake:menu/may-place", &payload.finish(), HostAccess::None)
            .and_then(|bytes| {
                let mut d = Dec::new(&bytes)?;
                let allowed = d.u8()?;
                d.done().then_some(allowed != 0)
            })
            // A guest that cannot answer refuses nothing the slot would take.
            .unwrap_or(true)
    }
}

// ---------------------------------------------------------------------------------------------
// Recipe types
// ---------------------------------------------------------------------------------------------
//
// ```text
// recipe             str type id, then:
//   describe         -                              answer: u32 version, str category id, str icon item
//   decode           u32 length + the recipe's JSON answer: u8 ok; ok: u32 length + data, else: str error
//   matches          u32 length + data, u16 count + items          answer: u8
//   assemble         the same                                      answer: u16 count + items
//   display          u32 length + data                  answer: u16 groups (u16 count + the items that fit
//                    one slot), u16 count + the items it makes
// ```
//
// The data of a recipe is kept as `decode` answered it (the recipe's own JSON text when the guest has no
// `decode`), at most 64 KiB; it is the guest's to read again in `matches`, `assemble` and `display`.

/// Largest data of one recipe.
const MAX_RECIPE_DATA: usize = 64 * 1024;
/// Most items in a crafting grid, an answer or a display.
const MAX_RECIPE_ITEMS: usize = 64;

/// The recipe types of the registered `recipe` objects: each asks its guest to `describe` it (the version
/// its recipes carry and its recipe book category), which can wait a little for a busy module.
pub(crate) fn recipe_types() -> Vec<crate::mod_api::NativeRecipeType> {
    use crate::mod_api::NativeRecipeType;
    use crate::recipe_book::RecipeBookCategory;
    let mut types = Vec::new();
    for (id, entry) in snapshot(Kind::Recipe) {
        let mut payload = Enc::new();
        payload.str(&id);
        let described = invoke_waiting("yoake:recipe/describe", &entry.owner, &payload.finish()).and_then(|bytes| {
            let mut d = Dec::new(&bytes)?;
            let (version, category, icon) = (d.u32()?, d.str()?, d.str()?);
            d.done().then_some((version, category, icon))
        });
        let Some((version, category, icon)) = described else {
            log::warn!("wasm mod {}: recipe type {id} did not describe itself; it is left out", entry.owner);
            continue;
        };
        types.push(NativeRecipeType {
            category: RecipeBookCategory { id: category, recipe_type: id.clone(), icon },
            codec: Arc::new(WasmRecipe { owner: entry.owner, id: id.clone(), hooks: entry.hooks }),
            id,
            version,
        });
    }
    types
}

/// Like [`invoke`] for a call the engine cannot drop: a module another thread holds is waited for, a little.
fn invoke_waiting(method: &str, owner: &str, payload: &[u8]) -> Option<Vec<u8>> {
    let started = std::time::Instant::now();
    loop {
        if let Some(slot) = endpoint(owner)
            && slot.state.try_lock().is_ok()
        {
            return invoke(owner, method, payload, HostAccess::None);
        }
        if started.elapsed() > std::time::Duration::from_secs(2) {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

struct WasmRecipe {
    owner: String,
    id: String,
    hooks: u32,
}

fn data_bytes(data: &serde_json::Value) -> Option<Vec<u8>> {
    let text = data.as_str()?;
    if text.len() % 2 != 0 || text.len() > MAX_RECIPE_DATA * 2 {
        return None;
    }
    (0..text.len() / 2).map(|i| u8::from_str_radix(text.get(i * 2..i * 2 + 2)?, 16).ok()).collect()
}

/// The package that defines recipe type `id`, if one does.
pub(crate) fn recipe_owner(id: &str) -> Option<String> {
    table().recipes.get(id).map(|entry| entry.owner.clone())
}

/// The bytes of a recipe's data as the game keeps it (a JSON string of hex digits): what `recipe-data` gives a guest.
pub(crate) fn recipe_data_bytes(data: &str) -> Option<Vec<u8>> {
    data_bytes(&serde_json::Value::String(data.to_owned()))
}

fn data_value(bytes: &[u8]) -> serde_json::Value {
    serde_json::Value::String(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl WasmRecipe {
    fn request(&self, data: &serde_json::Value) -> Option<Enc> {
        let bytes = data_bytes(data)?;
        let mut enc = Enc::new();
        enc.str(&self.id).u32(bytes.len() as u32);
        enc.0.extend_from_slice(&bytes);
        Some(enc)
    }

    fn items(d: &mut Dec<'_>) -> Option<Vec<ItemStack>> {
        let count = usize::from(d.u16()?);
        if count > MAX_RECIPE_ITEMS {
            return None;
        }
        (0..count).map(|_| d.item()).collect()
    }

    fn write_items(enc: &mut Enc, items: &[ItemStack]) {
        enc.u16(items.len().min(MAX_RECIPE_ITEMS) as u16);
        for stack in items.iter().take(MAX_RECIPE_ITEMS) {
            enc.item(stack);
        }
    }
}

impl crate::mod_api::NativeRecipeCodec for WasmRecipe {
    fn decode(&self, json: &serde_json::Value) -> Result<serde_json::Value, String> {
        let text = json.to_string();
        if text.len() > MAX_RECIPE_DATA {
            return Err("the recipe is larger than 64 KiB".into());
        }
        if self.hooks & R_DECODE == 0 {
            return Ok(data_value(text.as_bytes()));
        }
        let mut enc = Enc::new();
        enc.str(&self.id).u32(text.len() as u32);
        enc.0.extend_from_slice(text.as_bytes());
        let bytes = invoke_waiting("yoake:recipe/decode", &self.owner, &enc.finish())
            .ok_or_else(|| format!("package {:.64} did not answer for {}", self.owner, self.id))?;
        let mut d = Dec::new(&bytes).ok_or("the answer does not check")?;
        match d.u8() {
            Some(1) => {
                let n = d.u32().ok_or("the answer does not check")? as usize;
                if n > MAX_RECIPE_DATA {
                    return Err("the recipe's data is larger than 64 KiB".into());
                }
                let data = d.take(n).ok_or("the answer does not check")?.to_vec();
                d.done().then(|| data_value(&data)).ok_or_else(|| "the answer does not check".into())
            }
            Some(0) => Err(d.str().unwrap_or_else(|| "refused".into())),
            _ => Err("the answer does not check".into()),
        }
    }

    fn matches(&self, data: &serde_json::Value, input: &[ItemStack]) -> bool {
        let Some(mut enc) = self.request(data) else { return false };
        Self::write_items(&mut enc, input);
        invoke(&self.owner, "yoake:recipe/matches", &enc.finish(), HostAccess::None)
            .and_then(|bytes| {
                let mut d = Dec::new(&bytes)?;
                let matched = d.u8()?;
                d.done().then_some(matched != 0)
            })
            .unwrap_or(false)
    }

    fn assemble(&self, data: &serde_json::Value, input: &[ItemStack]) -> Vec<ItemStack> {
        let Some(mut enc) = self.request(data) else { return Vec::new() };
        Self::write_items(&mut enc, input);
        invoke(&self.owner, "yoake:recipe/assemble", &enc.finish(), HostAccess::None)
            .and_then(|bytes| {
                let mut d = Dec::new(&bytes)?;
                let items = Self::items(&mut d)?;
                d.done().then_some(items)
            })
            .unwrap_or_default()
    }

    fn display(&self, data: &serde_json::Value) -> crate::mod_api::NativeRecipeDisplay {
        let empty = crate::mod_api::NativeRecipeDisplay { inputs: Vec::new(), outputs: Vec::new() };
        if self.hooks & R_DISPLAY == 0 {
            return empty;
        }
        let Some(mut enc) = self.request(data) else { return empty };
        invoke(&self.owner, "yoake:recipe/display", &enc.finish(), HostAccess::None)
            .and_then(|bytes| {
                let mut d = Dec::new(&bytes)?;
                let groups = usize::from(d.u16()?);
                if groups > MAX_RECIPE_ITEMS {
                    return None;
                }
                let inputs = (0..groups).map(|_| Self::items(&mut d)).collect::<Option<Vec<_>>>()?;
                let outputs = Self::items(&mut d)?;
                d.done().then_some(crate::mod_api::NativeRecipeDisplay { inputs, outputs })
            })
            .unwrap_or(empty)
    }
}

#[cfg(test)]
#[path = "behaviors/tests.rs"]
mod tests;
