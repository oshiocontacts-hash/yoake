//! The script side of fully custom mobs (FR-F4-MOBS.md sections 3.3 and 6.2, F4M-10): the types a
//! WASM guest sees, the command bus its `mobs-server` imports write to, and the [`Runtime`] the
//! game installs to call the guest's exports.
//!
//! Nothing here runs without a [`Runtime`]: with no mod that declares a server module, `installed`
//! is false, no `ScriptGoal` ever says yes and no custom mob asks a guest anything.
//!
//! # How a guest call reaches a mob that is being ticked
//!
//! A mob is ticked as `&mut Mob` while the game holds the mob list apart, so a guest service cannot
//! reach it by handle. Instead every guest call is made through [`call`]: the mob's [`MobView`] is
//! published first (services that read answer from it), and what the guest asks for is written to a
//! **command queue** ([`Command`]). When the call returns, the caller applies the commands for
//! its own mob ([`take_for`], then [`apply`]) before the engine goes on, so a goal's `start` that
//! says "navigate to" has navigated when it returns. Commands for another mob, or made outside any
//! call, wait in the queue and are applied at the start of that mob's next tick
//! ([`pre_tick`]). `spawn` is the exception: it needs the whole game and only works from a hook
//! that has one.
use super::{CustomState, CustomType, SyncedValue};
use crate::mobs::{Mob, MobEvent, MobWorld};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// Script goal evaluations (`can-use`, `can-continue`, `start`, `tick`, `stop`) one mob may make in
/// one game tick (FR-F4-MOBS.md section 8). Past it the goal answers its fallback: `can-use`
/// false, `can-continue` true (a running goal is not cut off by the budget), `tick` skipped.
pub const MAX_EVALUATIONS_PER_MOB_TICK: u32 = 32;
/// Most commands waiting for mobs that are not being ticked; the oldest are dropped (and counted).
pub const MAX_QUEUED_COMMANDS: usize = 4096;
/// Most bytes of a mob's script data (`data-put`, `mob-save`).
pub const MAX_DATA_BYTES: usize = 64 * 1024;
/// Most bone writes a `mob-pose` call may return.
pub const MAX_POSE_WRITES: usize = 256;

// ---------------------------------------------------------------------------------------------
// Types of the hooks (the generator maps these to WIT records)
// ---------------------------------------------------------------------------------------------

/// A point or a vector.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ScriptVec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

/// A tracked value as a script sees it.
#[derive(Clone, Debug, PartialEq)]
pub enum MobScriptValue {
    Bool(bool),
    Int(i32),
    Float(f32),
    Text(String),
    Vec3(ScriptVec3),
    /// An enum value, by name.
    Choice(String),
}

/// One bone's change a `mob-pose` call returns (see `horizon-mobmodel`'s `BoneWrite`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MobBoneWrite {
    /// The bone index (`bone-index(type, name)`).
    pub bone: u32,
    /// Translation adds, rotation multiplies on the right, scale multiplies; else the channels
    /// replace the pose's.
    pub additive: bool,
    /// Pixels.
    pub translation: Option<ScriptVec3>,
    /// Degrees about X, Y, Z (X applied first).
    pub rotation_degrees: Option<ScriptVec3>,
    pub scale: Option<ScriptVec3>,
    /// Show or hide the bone.
    pub visible: Option<bool>,
}

/// What `mob-pose` is told about the frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MobPoseFrame {
    /// The mob's age in seconds, with the frame's partial tick.
    pub time: f32,
    pub partial: f32,
    /// Blocks per tick on the ground, and the mob's head turn against its body (degrees).
    pub ground_speed: f32,
    pub head_yaw: f32,
    pub head_pitch: f32,
    pub on_ground: bool,
    /// The mob's attack target (the point the procedural `target` source reads), if it has one.
    pub target: Option<ScriptVec3>,
}

// ---------------------------------------------------------------------------------------------
// Views and commands
// ---------------------------------------------------------------------------------------------

/// What `info` answers about a mob.
#[derive(Clone, Debug, PartialEq)]
pub struct MobView {
    pub id: u32,
    pub type_id: String,
    pub dimension: String,
    pub position: [f64; 3],
    pub velocity: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub max_health: f32,
    pub alive: bool,
    pub on_ground: bool,
    /// The attack target: a mob's id, or the player.
    pub target_mob: Option<u32>,
    pub target_player: bool,
    pub owner: Option<[i32; 4]>,
    pub sitting: bool,
    pub baby: bool,
    pub age: i32,
    pub controlled: bool,
    /// Where the attack target is, and whether the mob has a line of sight to it (known when the
    /// view is made with a world: inside a guest call of the mob's own goals and hooks).
    pub target_position: Option<[f64; 3]>,
    pub target_visible: bool,
    /// The tracked values by name, and the custom attributes by name (base, value).
    pub synced: Vec<(String, MobScriptValue)>,
    pub attributes: Vec<(String, f64, f64)>,
    pub data: Vec<u8>,
}

impl MobView {
    /// The view of `m` (`None` for a mob that is not custom).
    pub fn of(m: &Mob) -> Option<Self> {
        Self::of_in(m, None, false)
    }

    /// [`of`](Self::of) with the world, which gives the target's place and sight.
    pub fn of_in(m: &Mob, w: Option<&MobWorld>, sees_target: bool) -> Option<Self> {
        let t = m.kind.custom.as_ref()?;
        let state = m.extra.custom.as_ref()?;
        let mut attrs = state.attrs.clone();
        let attributes = t
            .attributes
            .iter()
            .filter_map(|d| Some((d.id.clone(), attrs.base(&d.id)?, attrs.value(&d.id)?)))
            .collect();
        Some(Self {
            id: m.id,
            type_id: t.id.clone(),
            dimension: String::new(),
            position: [m.position.x, m.position.y, m.position.z],
            velocity: [m.velocity.x, m.velocity.y, m.velocity.z],
            yaw: m.body_yaw,
            pitch: m.head_pitch,
            health: m.health,
            max_health: m.attrs.max_health,
            alive: m.alive(),
            on_ground: m.on_ground,
            target_mob: match m.ai.target {
                Some(crate::mobs::Who::Mob(id)) => Some(id),
                _ => None,
            },
            target_player: matches!(m.ai.target, Some(crate::mobs::Who::Player)),
            owner: m.extra.owner,
            sitting: m.extra.sitting,
            baby: m.is_baby(),
            age: m.extra.age,
            controlled: state.script.claimed,
            target_position: w.and_then(|w| m.target_info(w)).map(|t| [t.position.x, t.position.y, t.position.z]),
            target_visible: sees_target,
            synced: t
                .synced
                .iter()
                .zip(&state.synced)
                .map(|(d, v)| (d.name.clone(), script_value(d, v)))
                .collect(),
            attributes,
            data: state.script.data.clone(),
        })
    }
}

/// A tracked value as the script sees it.
pub fn script_value(def: &super::SyncedDef, v: &SyncedValue) -> MobScriptValue {
    match v {
        SyncedValue::Bool(b) => MobScriptValue::Bool(*b),
        SyncedValue::I32(n) => MobScriptValue::Int(*n),
        SyncedValue::F32(n) => MobScriptValue::Float(*n),
        SyncedValue::Str(s) => MobScriptValue::Text(s.clone()),
        SyncedValue::Vec3(p) => MobScriptValue::Vec3(ScriptVec3 { x: p[0], y: p[1], z: p[2] }),
        SyncedValue::Enum(i) => MobScriptValue::Choice(def.values.get(usize::from(*i)).cloned().unwrap_or_default()),
    }
}

/// A script value as the type's tracked value `def` (`None` when it is of another type).
pub fn synced_value(def: &super::SyncedDef, v: &MobScriptValue) -> Option<SyncedValue> {
    let v = match (def.kind.as_str(), v) {
        ("bool", MobScriptValue::Bool(b)) => SyncedValue::Bool(*b),
        ("i32", MobScriptValue::Int(n)) => SyncedValue::I32(*n),
        ("f32", MobScriptValue::Float(n)) if n.is_finite() => SyncedValue::F32(*n),
        ("string", MobScriptValue::Text(s)) => SyncedValue::Str(s.clone()),
        ("vec3", MobScriptValue::Vec3(p)) if [p.x, p.y, p.z].iter().all(|n| n.is_finite()) => {
            SyncedValue::Vec3([p.x, p.y, p.z])
        }
        ("enum", MobScriptValue::Choice(name)) => SyncedValue::Enum(def.values.iter().position(|n| n == name)? as u8),
        _ => return None,
    };
    def.coerce(v)
}

/// What a guest asked a mob to do.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SetSynced { key: String, value: MobScriptValue },
    TriggerAnim { name: String, speed: f32 },
    SetControllerState { controller: String, state: String },
    NavigateTo { position: [f64; 3], speed: f64 },
    NavigateStop,
    LookAt { position: [f64; 3], yaw_speed: f32, pitch_speed: f32 },
    Jump,
    SetTarget { mob: Option<u32>, player: bool },
    Attack { mob: Option<u32>, player: bool, damage: f32, knockback: f32 },
    SetAttributeBase { attribute: String, value: f64 },
    AddModifier { attribute: String, modifier: String, amount: f64, operation: String },
    RemoveModifier { attribute: String, modifier: String },
    SetHitbox { name: String, enabled: bool },
    PutData(Vec<u8>),
    Despawn,
    Claim(bool),
    Impulse([f64; 3]),
}

/// A command for one mob.
#[derive(Clone, Debug, PartialEq)]
pub struct Queued {
    pub mob: u32,
    pub command: Command,
}

#[derive(Default)]
struct Bus {
    views: HashMap<u32, Arc<MobView>>,
    queue: Vec<Queued>,
    /// The mob whose guest call is running (services that name another mob still queue).
    current: Option<u32>,
    dropped: u64,
}

static BUS: std::sync::LazyLock<Mutex<Bus>> = std::sync::LazyLock::new(|| Mutex::new(Bus::default()));

fn bus() -> std::sync::MutexGuard<'static, Bus> {
    BUS.lock().unwrap_or_else(|p| p.into_inner())
}

/// Publishes the view of a mob (the game does it for every custom mob at the start of a tick, and
/// the engine again for the mob whose hook is about to run).
pub fn publish(view: MobView) {
    let id = view.id;
    bus().views.insert(id, Arc::new(view));
}

/// Forgets a mob's view (it was removed).
pub fn forget(id: u32) {
    bus().views.remove(&id);
}

/// The last published view of a mob.
pub fn view(id: u32) -> Option<Arc<MobView>> {
    bus().views.get(&id).cloned()
}

/// Queues `command` for `mob` (what the `mobs-server` services do).
pub fn push(mob: u32, command: Command) {
    let mut b = bus();
    if b.queue.len() >= MAX_QUEUED_COMMANDS {
        b.queue.remove(0);
        b.dropped += 1;
    }
    b.queue.push(Queued { mob, command });
}

/// Takes the commands waiting for `mob`, oldest first.
pub fn take_for(mob: u32) -> Vec<Command> {
    let mut b = bus();
    if b.queue.is_empty() {
        return Vec::new();
    }
    let mut mine = Vec::new();
    b.queue.retain(|q| {
        if q.mob == mob {
            mine.push(q.command.clone());
            false
        } else {
            true
        }
    });
    mine
}

/// How many commands wait.
pub fn queued() -> usize {
    bus().queue.len()
}

/// Commands dropped because the queue was full.
pub fn dropped() -> u64 {
    bus().dropped
}

/// The mob whose guest call is running.
pub fn current() -> Option<u32> {
    bus().current
}

/// Empties the bus (a new world; tests).
pub fn reset() {
    let mut b = bus();
    b.views.clear();
    b.queue.clear();
    b.current = None;
    b.dropped = 0;
}

// ---------------------------------------------------------------------------------------------
// The runtime
// ---------------------------------------------------------------------------------------------

/// What `mob-interact` answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InteractReply {
    Pass,
    Consume,
    Success,
}

/// What `mob-hurt` answers: the (maybe changed) amount, or a cancel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HurtReply {
    pub amount: f32,
    pub cancel: bool,
}

/// The guest side the game installs (F4M-10): one method per export of `yoake:mods/server-hooks`
/// the engine calls. Every method answers its fallback when no guest runs, subscribed or has not
/// trapped; `goal_*` return `None` then so the caller can tell "no" from "nobody answered".
pub trait Runtime: Send + Sync {
    /// Whether any module answers (a module that trapped or timed out does not count).
    fn live(&self) -> bool;
    fn goal_can_use(&self, mob: u32, goal: &str) -> Option<bool>;
    fn goal_can_continue(&self, mob: u32, goal: &str) -> Option<bool>;
    fn goal_start(&self, mob: u32, goal: &str);
    fn goal_tick(&self, mob: u32, goal: &str);
    fn goal_stop(&self, mob: u32, goal: &str);
    fn mob_tick(&self, mob: u32);
    fn mob_spawn_check(&self, type_id: &str, dimension: &str, pos: [i32; 3]) -> Option<bool>;
    fn mob_spawned(&self, mob: u32, type_id: &str, reason: &str);
    fn mob_hurt(&self, mob: u32, part: &str, source: &str, amount: f32) -> Option<HurtReply>;
    fn mob_died(&self, mob: u32, type_id: &str, attacker: Option<u32>);
    fn mob_interact(&self, mob: u32, player: &str, hand: &str, item: &str) -> InteractReply;
    fn mob_anim_event(&self, mob: u32, name: &str);
    fn mob_load(&self, mob: u32, data: &[u8]);
    fn mob_save(&self, mob: u32, data: &[u8]) -> Option<Vec<u8>>;
    fn mob_tamed(&self, mob: u32, owner: &str);
    fn mob_bred(&self, mob: u32, partner: u32, child: u32);
    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue);
}

static RUNTIME: RwLock<Option<Arc<dyn Runtime>>> = RwLock::new(None);

/// Installs (or removes, with `None`) the guest runtime; returns the one it replaced.
pub fn set_runtime(r: Option<Arc<dyn Runtime>>) -> Option<Arc<dyn Runtime>> {
    match RUNTIME.write() {
        Ok(mut slot) => std::mem::replace(&mut *slot, r),
        Err(_) => None,
    }
}

/// The installed runtime, if it has a live module.
pub fn runtime() -> Option<Arc<dyn Runtime>> {
    let r = RUNTIME.read().ok()?.clone()?;
    r.live().then_some(r)
}

/// Whether a runtime is installed (cheap; the game asks it before it publishes views).
pub fn installed() -> bool {
    RUNTIME.read().map(|r| r.is_some()).unwrap_or(false)
}

/// Runs `f` (a call into the guest) for `m`: the mob's view is published, and the commands the
/// guest queued for `m` are applied afterwards, in order. `w` and `events` are what the commands
/// that move or hurt need; a command they cannot do waits for the mob's next tick.
pub fn call<R>(
    m: &mut Mob,
    w: Option<&MobWorld>,
    events: Option<&mut Vec<MobEvent>>,
    f: impl FnOnce(&dyn Runtime) -> R,
) -> Option<R> {
    let rt = runtime()?;
    let sees = match (w, m.ai.target) {
        (Some(w), Some(who)) => m.sees(w, who),
        _ => false,
    };
    if let Some(v) = MobView::of_in(m, w, sees) {
        let mut b = bus();
        b.views.insert(v.id, Arc::new(v));
        b.current = Some(m.id);
    }
    let r = f(&*rt);
    bus().current = None;
    apply_pending(m, w, events);
    Some(r)
}

/// Applies what is queued for `m`.
pub fn apply_pending(m: &mut Mob, w: Option<&MobWorld>, mut events: Option<&mut Vec<MobEvent>>) {
    for c in take_for(m.id) {
        if !super::script_apply::apply_in(m, w, events.as_deref_mut(), c.clone()) {
            push(m.id, c);
        }
    }
}

/// `mob-hurt` for a custom mob: the guests' answer (`None` when nobody answered).
pub fn hurt(m: &mut Mob, part: &str, source: &str, amount: f32) -> Option<HurtReply> {
    let id = m.id;
    call(m, None, None, |rt| rt.mob_hurt(id, part, source, amount)).flatten()
}

/// `mob-died` for a custom mob.
pub fn died(m: &mut Mob, attacker: Option<u32>) {
    let id = m.id;
    let kind = m.kind.custom.as_ref().map(|t| t.id.clone()).unwrap_or_default();
    call(m, None, None, |rt| rt.mob_died(id, &kind, attacker));
}

/// `mob-spawned` for a custom mob (`reason`: natural, egg, command, script, breeding or load); once
/// per mob.
pub fn spawned(m: &mut Mob, reason: &str) {
    if !installed() {
        return;
    }
    let Some(s) = state(m) else { return };
    if std::mem::replace(&mut s.spawned, true) {
        return;
    }
    let id = m.id;
    let kind = m.kind.custom.as_ref().map(|t| t.id.clone()).unwrap_or_default();
    call(m, None, None, |rt| rt.mob_spawned(id, &kind, reason));
}

/// `mob-save`: the script data to write with the mob (what the guests made of it).
pub fn saved_data(m: &mut Mob) -> Option<Vec<u8>> {
    if !installed() {
        return None;
    }
    let id = m.id;
    let data = state(m)?.data.clone();
    let out = call(m, None, None, |rt| rt.mob_save(id, &data)).flatten()?;
    if let Some(s) = state(m) {
        s.data = out.clone();
    }
    Some(out)
}

/// The script data to write with `m`: what the guests make of it with `mob-save`, else what it has.
pub fn data_to_save(m: &Mob) -> Vec<u8> {
    let Some(state) = m.extra.custom.as_ref() else {
        return Vec::new();
    };
    match runtime() {
        Some(rt) => rt.mob_save(m.id, &state.script.data).unwrap_or_else(|| state.script.data.clone()),
        None => state.script.data.clone(),
    }
}

/// `mob-interact` for a custom mob.
pub fn interact(m: &mut Mob, player: &str, hand: &str, item: &str) -> InteractReply {
    let id = m.id;
    call(m, None, None, |rt| rt.mob_interact(id, player, hand, item)).unwrap_or(InteractReply::Pass)
}

/// A player or owner UUID as the canonical text a guest gets.
pub fn uuid_text(u: [i32; 4]) -> String {
    let n = u.iter().fold(0u128, |acc, p| (acc << 32) | u128::from(*p as u32));
    let h = format!("{n:032x}");
    format!("{}-{}-{}-{}-{}", &h[..8], &h[8..12], &h[12..16], &h[16..20], &h[20..])
}

/// Puts the claim of `claim-control` on the mob's goal selector (the goals that need `move`, `look`
/// or `jump` stop), or takes it off again. Called when the claim changes and at the start of every
/// tick of the mob (the selector is held apart while goals run, so a change made then is lost).
pub fn sync_claim(m: &mut Mob) {
    use crate::mobs::goals::{JUMP, LOOK, MOVE};
    let disabled = m.ai.goals.disabled;
    let Some(s) = m.extra.custom.as_mut().map(|c| &mut c.script) else { return };
    if s.claimed && !s.applied {
        s.saved_disabled = disabled;
        s.applied = true;
    }
    if s.claimed {
        m.ai.goals.disabled = s.saved_disabled | MOVE | LOOK | JUMP;
    } else if s.applied {
        m.ai.goals.disabled = s.saved_disabled;
        s.applied = false;
    }
}

/// What a custom mob's tick tells the guests, before its goals (called by `Mob::tick`): commands
/// that waited for the mob are applied, then `mob-load` (once), `mob-anim-event`,
/// `synced-changed-server`, `mob-tamed` / `mob-bred`, and `mob-tick`. Nothing happens without a
/// runtime. Returns false when a script despawned the mob.
pub fn pre_tick(m: &mut Mob, w: &MobWorld, events: &mut Vec<MobEvent>) -> bool {
    if m.kind.custom.is_none() {
        return true;
    }
    sync_claim(m);
    if !installed() {
        return true;
    }
    apply_pending(m, Some(w), Some(events));
    let now = w.game_time;
    let id = m.id;
    let (load, evs, changes, notices) = {
        let Some(c) = m.extra.custom.as_mut() else { return true };
        let load = !c.script.loaded;
        c.script.loaded = true;
        let data = if load && !c.script.data.is_empty() { Some(c.script.data.clone()) } else { None };
        (data, std::mem::take(&mut c.script.events), std::mem::take(&mut c.script.changes), c.take_notices())
    };
    let Some(t) = m.kind.custom.clone() else { return true };
    if let Some(data) = load
        && charge(m, now)
    {
        call(m, Some(w), Some(events), |rt| rt.mob_load(id, &data));
    }
    for name in evs {
        if charge(m, now) {
            call(m, Some(w), Some(events), |rt| rt.mob_anim_event(id, &name));
        }
    }
    for (slot, old, new) in changes {
        let Some(def) = t.synced.get(usize::from(slot)) else { continue };
        let (o, n) = (script_value(def, &old), script_value(def, &new));
        if charge(m, now) {
            call(m, Some(w), Some(events), |rt| rt.synced_changed(id, &def.name, &o, &n));
        }
    }
    for n in notices {
        if !charge(m, now) {
            break;
        }
        match n {
            super::Notice::Tamed { owner } => {
                let who = uuid_text(owner);
                call(m, Some(w), Some(events), |rt| rt.mob_tamed(id, &who));
            }
            super::Notice::Bred { partner, child } => {
                call(m, Some(w), Some(events), |rt| rt.mob_bred(id, partner.unwrap_or(0), child.unwrap_or(0)));
            }
        }
    }
    if charge(m, now) {
        call(m, Some(w), Some(events), |rt| rt.mob_tick(id));
    }
    !m.removed
}

fn charge(m: &mut Mob, now: i64) -> bool {
    state(m).is_some_and(|s| s.charge(now))
}


// ---------------------------------------------------------------------------------------------
// The client side: `mob-pose`, `mob-anim-event-client`, `synced-changed-client`
// ---------------------------------------------------------------------------------------------

/// The guest side of the client exports (the game installs it with [`set_client_runtime`]).
pub trait ClientRuntime: Send + Sync {
    fn live(&self) -> bool;
    /// The bone writes the guests give a mob of a `procedural_script` type for this frame.
    fn mob_pose(&self, mob: u32, kind: &str, frame: &MobPoseFrame) -> Vec<MobBoneWrite>;
    fn mob_anim_event(&self, mob: u32, name: &str);
    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue);
}

static CLIENT: RwLock<Option<Arc<dyn ClientRuntime>>> = RwLock::new(None);

/// Installs (or removes) the client runtime; returns the one it replaced.
pub fn set_client_runtime(r: Option<Arc<dyn ClientRuntime>>) -> Option<Arc<dyn ClientRuntime>> {
    match CLIENT.write() {
        Ok(mut slot) => std::mem::replace(&mut *slot, r),
        Err(_) => None,
    }
}

/// The installed client runtime, if a client module is live.
pub fn client_runtime() -> Option<Arc<dyn ClientRuntime>> {
    let r = CLIENT.read().ok()?.clone()?;
    r.live().then_some(r)
}

/// Converts a script's bone writes to the engine's, dropping the invalid ones and keeping at most
/// [`MAX_POSE_WRITES`]: a write is `bone` plus channels (see `BoneWrite`); non-finite numbers drop the
/// whole write.
pub fn bone_writes(list: &[MobBoneWrite], bones: usize) -> Vec<horizon_mobmodel::procedural::BoneWrite> {
    use horizon_mobmodel::procedural::{BoneWrite, WriteMode, WriteRotation};
    let v3 = |p: &ScriptVec3| glam::Vec3::new(p.x, p.y, p.z);
    let ok = |p: &ScriptVec3| [p.x, p.y, p.z].iter().all(|n| n.is_finite());
    let mut out = Vec::new();
    for w in list.iter().take(MAX_POSE_WRITES) {
        if (w.bone as usize) >= bones
            || ![w.translation.as_ref(), w.rotation_degrees.as_ref(), w.scale.as_ref()].into_iter().flatten().all(ok)
        {
            continue;
        }
        let mut b = BoneWrite::new(w.bone, if w.additive { WriteMode::Additive } else { WriteMode::Absolute });
        b.translation = w.translation.as_ref().map(v3);
        b.rotation = w.rotation_degrees.as_ref().map(|r| WriteRotation::EulerDegrees(v3(r)));
        b.scale = w.scale.as_ref().map(v3);
        b.visible = w.visible;
        out.push(b);
    }
    out
}

/// A locator's place in the world as the client last drew the mob.
#[derive(Clone, Debug, PartialEq)]
pub struct LocatorPlace {
    pub name: String,
    pub position: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
}

static LOCATORS: Mutex<Option<HashMap<u32, Vec<LocatorPlace>>>> = Mutex::new(None);

/// Stores the locators of mob `id` as drawn this frame (only for `procedural_script` types).
pub fn set_locators(id: u32, places: Vec<LocatorPlace>) {
    let mut g = LOCATORS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(id, places);
}

/// Where locator `name` of mob `id` was drawn last.
pub fn locator(id: u32, name: &str) -> Option<LocatorPlace> {
    let g = LOCATORS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref()?.get(&id)?.iter().find(|p| p.name == name).cloned()
}

/// Forgets the drawn locators (a new world; tests).
pub fn clear_locators() {
    *LOCATORS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The bone index a script writes for bone `name` of type `type_id` (`bone-index`).
pub fn bone_index(type_id: &str, name: &str) -> Option<u32> {
    let t = super::get(type_id)?;
    let tr = t.render.as_ref()?;
    tr.skeleton.find(name).map(|i| u32::from(i.0))
}

/// The state a script keeps on a mob (`CustomState.script`).
#[derive(Clone, Debug, Default)]
pub struct ScriptState {
    /// `claim-control`: the script drives the mob; its own goals are suspended.
    pub claimed: bool,
    /// The opaque bytes of `data-get` and `data-put`, saved with the mob.
    pub data: Vec<u8>,
    /// The tick `evaluations` counts for.
    pub eval_tick: i64,
    /// Script goal calls made in `eval_tick` (the budget of [`MAX_EVALUATIONS_PER_MOB_TICK`]).
    pub evaluations: u32,
    /// Server timeline events delivered and not yet handed to `mob-anim-event`.
    pub events: Vec<String>,
    /// Tracked value changes (slot, old, new) not yet handed to `synced-changed-server`.
    pub changes: Vec<(u8, SyncedValue, SyncedValue)>,
    /// `mob-load` was called for this mob.
    pub loaded: bool,
    /// `mob-spawned` was told for this mob.
    pub spawned: bool,
    /// The bits of the goal selector's `disabled` mask before `claim-control`.
    pub saved_disabled: u8,
    /// The selector's flags carry the claim now.
    pub applied: bool,
}

impl ScriptState {
    /// Notes a server timeline event for `mob-anim-event` (the oldest are dropped past 16).
    pub fn push_event(&mut self, name: &str) {
        if self.events.len() >= 16 {
            self.events.remove(0);
        }
        self.events.push(name.to_owned());
    }

    /// Notes a tracked value change for `synced-changed-server` (the oldest are dropped past 16).
    pub fn note_change(&mut self, slot: usize, old: SyncedValue, new: SyncedValue) {
        if self.changes.len() >= 16 {
            self.changes.remove(0);
        }
        self.changes.push((slot as u8, old, new));
    }

    /// Counts one guest call of this mob in tick `now`; false when the budget of the tick is spent.
    pub fn charge(&mut self, now: i64) -> bool {
        if self.eval_tick != now {
            self.eval_tick = now;
            self.evaluations = 0;
        }
        if self.evaluations >= MAX_EVALUATIONS_PER_MOB_TICK {
            return false;
        }
        self.evaluations += 1;
        true
    }
}

/// Whether the mob's script state exists (it is made with the custom state).
pub fn state(m: &mut Mob) -> Option<&mut ScriptState> {
    m.extra.custom.as_mut().map(|c| &mut c.script)
}

/// The type of a custom mob (helper for the services).
pub fn type_of(m: &Mob) -> Option<&Arc<CustomType>> {
    m.kind.custom.as_ref()
}

#[allow(dead_code)]
fn _uses(_: &CustomState) {}
