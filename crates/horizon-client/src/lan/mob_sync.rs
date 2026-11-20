//! The server half of `horizon:mob_sync` (F4M-5b; the wire format is `horizon_protocol::mob_sync`).
//!
//! A custom mob keeps its tracked values, controller anchors, triggers and hitbox mask in its
//! `CustomState`. Once per tick, after the mobs ticked, [`collect`] turns what changed into messages:
//!
//! * **triggers** go at once (never merged, never rate limited);
//! * one **delta** per mob carries every value that changed, the anchors that must be re-sent (state
//!   or speed change, see `anim_sync::needs_anchor`) and a changed hitbox mask, all merged over the
//!   ticks since the last delta;
//! * a **snapshot** ([`snapshot`]) is the whole state, sent to a player right after the mob's
//!   `AddEntity` (when it starts tracking the mob).
//!
//! # Limits
//!
//! * At most 256 bytes of values in a message (a type that could exceed it does not load,
//!   `CustomType::new`).
//! * At most 10 deltas per second and mob unless a value asks for more: each value has a `sync_rate`
//!   (1 to 20 per second, default 10) that is the most often it is sent; the mob's deltas are no
//!   closer together than the fastest of what they carry (anchors and the hitbox mask count as 10).
//!   A value changed again before its turn is sent once, as its latest value (merged).
//! * Only the players who track the entity get anything (the tracking range is the distance gate),
//!   only Horizon clients ([`reads_mob_sync`]), and nothing at all when no custom type is loaded.
//! * Values with `sync` of `none` or `owner` are not sent (the owner state is F4M-4c).
use super::guest::Guest;
use horizon_protocol::packets::{configuration::CustomPayload, play};
use crate::mobs::{
    Mob,
    custom::{CustomState, CustomType, SyncedDef, SyncedValue, anim_sync, attrs::Sync},
};
use horizon_protocol::mob_sync::{
    self as wire, Delta, Frame, MAX_FRAME, Message, Snapshot, TICKS_PER_SECOND, TriggerStart, Value, ValueSet,
};

/// The custom payload channel.
pub use wire::CHANNEL;

/// What the sync layer remembers of one mob's sending (`CustomState.sync_out`).
#[derive(Clone, Debug, PartialEq)]
pub struct SyncOut {
    /// The tick of the mob's last delta.
    pub last_delta: i64,
    /// The tick each value was last sent.
    pub last_value: [i64; wire::MAX_VALUES],
}

impl Default for SyncOut {
    fn default() -> Self {
        Self {
            last_delta: i64::MIN / 2,
            last_value: [i64::MIN / 2; wire::MAX_VALUES],
        }
    }
}

/// The ticks between two sends of something that may go `rate` times a second.
pub fn interval(rate: u32) -> i64 {
    i64::from(TICKS_PER_SECOND.div_ceil(rate.clamp(1, TICKS_PER_SECOND)))
}

/// The wire form of a tracked value.
pub fn to_wire(v: &SyncedValue) -> Value {
    match v {
        SyncedValue::Bool(b) => Value::Bool(*b),
        SyncedValue::I32(n) => Value::I32(*n),
        SyncedValue::F32(f) => Value::F32(*f),
        SyncedValue::Str(s) => Value::Str(s.clone()),
        SyncedValue::Vec3(v) => Value::Vec3(*v),
        SyncedValue::Enum(i) => Value::Enum(*i),
    }
}

/// A received value as the tracked value `def` (cut to its range): `None` for a value of another
/// kind, a non-finite number or an enum index the type does not have.
pub fn from_wire(def: &SyncedDef, v: &Value) -> Option<SyncedValue> {
    let v = match v {
        Value::Bool(b) => SyncedValue::Bool(*b),
        Value::I32(n) => SyncedValue::I32(*n),
        Value::F32(f) => SyncedValue::F32(*f),
        Value::Str(s) => SyncedValue::Str(s.clone()),
        Value::Vec3(v) => SyncedValue::Vec3(*v),
        Value::Enum(i) => SyncedValue::Enum(*i),
    };
    def.coerce(v)
}

/// The slots whose values go to the clients.
pub fn announced_mask(t: &CustomType) -> u32 {
    t.synced
        .iter()
        .take(wire::MAX_VALUES)
        .enumerate()
        .filter(|(_, d)| d.sync == Sync::All)
        .fold(0, |m, (i, _)| m | 1 << i)
}

fn value_set(t: &CustomType, state: &CustomState, mask: u32) -> ValueSet {
    let mut set = ValueSet::default();
    for (slot, v) in state.synced.iter().enumerate().take(wire::MAX_VALUES) {
        if mask & (1 << slot) != 0 && t.synced.get(slot).is_some() {
            set.mask |= 1 << slot;
            set.values.push(to_wire(v));
        }
    }
    set
}

/// What one mob wants sent this tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collected {
    pub entity: i32,
    pub triggers: Vec<TriggerStart>,
    pub delta: Option<Delta>,
}

impl Collected {
    pub fn messages(&self) -> impl Iterator<Item = Message> + '_ {
        self.triggers
            .iter()
            .copied()
            .map(Message::Trigger)
            .chain(self.delta.clone().map(Message::Delta))
    }
}

/// Takes what changed about custom mob `m` since the last call, at game tick `now`; `None` when
/// there is nothing to send (or `m` is not a custom mob). Values that are not told to anyone are
/// forgotten here; values whose turn has not come stay changed for a later tick.
pub fn collect(m: &mut Mob, now: i64) -> Option<Collected> {
    let t = m.kind.custom.clone()?;
    let entity = m.id as i32;
    let state = m.extra.custom.as_mut()?;
    let announced = announced_mask(&t);
    // What nobody is told is not pending.
    state.synced_dirty &= announced;
    let triggers: Vec<TriggerStart> = state
        .anim
        .as_mut()
        .map(|a| a.take_started())
        .unwrap_or_default()
        .into_iter()
        .map(|trigger| TriggerStart { entity, trigger })
        .collect();
    // The values whose turn it is, and the fastest rate among everything pending.
    let mut ready = 0u32;
    let mut rate = 0;
    for (slot, def) in t.synced.iter().enumerate().take(wire::MAX_VALUES) {
        let bit = 1 << slot;
        if state.synced_dirty & bit != 0 && now - state.sync_out.last_value[slot] >= interval(def.rate) {
            ready |= bit;
            rate = rate.max(def.rate);
        }
    }
    let anchors_pending = state
        .anim
        .as_ref()
        .is_some_and(|a| a.anchors_changed(anim_sync::ANCHOR_TOLERANCE));
    if anchors_pending || state.hitbox_dirty {
        rate = rate.max(wire::DEFAULT_SYNC_RATE);
    }
    let mut out = Collected { entity, triggers, delta: None };
    if rate == 0 || now - state.sync_out.last_delta < interval(rate) {
        return (!out.triggers.is_empty()).then_some(out);
    }
    let anchors = match state.anim.as_mut() {
        Some(a) => a.take_anchor_changes(anim_sync::ANCHOR_TOLERANCE),
        None => Vec::new(),
    };
    let values = (ready != 0).then(|| value_set(&t, state, ready));
    let hitbox_mask = state.hitbox_dirty.then_some(state.hitbox_mask);
    state.synced_dirty &= !ready;
    state.hitbox_dirty = false;
    state.sync_out.last_delta = now;
    for slot in (0..wire::MAX_VALUES).filter(|s| ready & (1 << s) != 0) {
        state.sync_out.last_value[slot] = now;
    }
    let delta = Delta { entity, values, anchors, hitbox_mask };
    out.delta = (!delta.is_empty()).then_some(delta);
    (out.delta.is_some() || !out.triggers.is_empty()).then_some(out)
}

/// The whole state of custom mob `m`, for a player that starts tracking it: every announced
/// value, the anchors of every layer, the triggers still playing and the hitbox mask. `None` for
/// a mob that is not a custom one.
pub fn snapshot(m: &Mob) -> Option<Snapshot> {
    let t = m.kind.custom.as_ref()?;
    let state = m.extra.custom.as_ref()?;
    let (anchors, triggers) = match (&state.anim, &t.anim) {
        (Some(a), Some(ta)) => (a.anchors().to_vec(), a.recent_triggers(ta)),
        _ => (Vec::new(), Vec::new()),
    };
    Some(Snapshot {
        entity: m.id as i32,
        type_version: t.sync_version,
        values: value_set(t, state, announced_mask(t)),
        anchors,
        triggers,
        hitbox_mask: state.hitbox_mask,
    })
}

/// Frames for `messages` at game tick `now`: as few as fit [`MAX_FRAME`] bytes each, in order.
/// A message that cannot be encoded is dropped (it would be a bug: the limits are the type's).
pub fn frames(now: i64, messages: impl IntoIterator<Item = Message>) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let mut frame = Frame::new(now);
    let mut size = 0;
    let flush = |frame: &mut Frame, out: &mut Vec<Vec<u8>>| {
        if !frame.messages.is_empty() {
            match wire::encode(frame) {
                Ok(bytes) => out.push(bytes),
                Err(e) => log::warn!("Horizon: mob sync frame dropped: {e}"),
            }
        }
        frame.messages.clear();
    };
    for m in messages {
        let Ok(len) = wire::message_len(&m) else {
            log::warn!("Horizon: a mob sync message was not encodable and was dropped");
            continue;
        };
        if size + len > MAX_FRAME || frame.messages.len() >= wire::MAX_MESSAGES {
            flush(&mut frame, &mut out);
            size = 0;
        }
        size += len;
        frame.messages.push(m);
    }
    flush(&mut frame, &mut out);
    out
}

/// Whether any custom type is loaded: with none, nothing of `horizon:mob_sync` is ever built or sent.
pub fn active() -> bool {
    crate::mobs::custom::registry().read().is_ok_and(|r| !r.is_empty())
}

/// What one player is sent this tick. `tracking` says which entities the player tracks (the
/// distance gate), `starts` are the ones it started tracking since the last tick (they get a
/// snapshot, and nothing else about the mob this tick: the snapshot is the whole state), `collected`
/// is what the mobs want sent and `snap` builds a snapshot by entity id.
pub fn for_viewer(
    tracking: impl Fn(i32) -> bool,
    starts: &[i32],
    collected: &[Collected],
    snap: impl Fn(i32) -> Option<Snapshot>,
) -> Vec<Message> {
    let mut out = Vec::new();
    for &entity in starts {
        if tracking(entity)
            && let Some(s) = snap(entity)
        {
            out.push(Message::Snapshot(s));
        }
    }
    for c in collected {
        if tracking(c.entity) && !starts.contains(&c.entity) {
            out.extend(c.messages());
        }
    }
    out
}

/// Whether the guest is placed and on a Horizon client: the only one that reads `horizon:mob_sync`.
pub fn reads_mob_sync(g: &Guest) -> bool {
    g.placed && g.brand == "horizon"
}

/// The payload packets a guest is sent for `messages`.
pub fn packets(now: i64, messages: impl IntoIterator<Item = Message>) -> Vec<play::Clientbound> {
    frames(now, messages)
        .into_iter()
        .map(|data| {
            play::Clientbound::CustomPayload(CustomPayload {
                channel: CHANNEL.into(),
                data,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests;
