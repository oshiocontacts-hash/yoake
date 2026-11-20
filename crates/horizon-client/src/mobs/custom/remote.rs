//! What a client keeps of the custom mobs a server tells it about with `horizon:mob_sync` (F4M-5b):
//! per entity the tracked values, the animation anchors of every controller layer and the ring of
//! recent triggers, applied from decoded [`Frame`]s. The game's glue is `game/remote_custom_mob.rs`;
//! the integrated server needs none of this (it reads the mob's own state) but the same apply code
//! runs in the tests against a server mob's messages.
//!
//! # Apply rules
//!
//! Every message is absolute and carries the server game time of its frame, so applying is
//! last-writer-wins by frame time, per piece:
//!
//! * a **value** slot takes a value only from a frame at least as new as the one that set it, so a
//!   delta that arrives after a newer one (reorder) or after a newer snapshot changes nothing, and a
//!   lost delta is repaired by the next delta of that slot or by a snapshot;
//! * a **layer's anchors** are replaced whole by a frame at least as new as the last one for that layer;
//! * a **trigger** is added to the ring once (`(id, start tick)` identifies it), whatever the order;
//! * the **hitbox mask** follows the newest frame.
//!
//! A **snapshot** applies the same way (its pieces are as new as its frame) and creates the entity's
//! store; a delta or trigger for an entity without a store is dropped (and counted): the server only
//! sends them after a snapshot, on the same ordered connection. A snapshot whose `type_version` is not
//! the client type's means the client's copy of the type differs from the server's: the mob is marked
//! incompatible and everything for it is ignored.
//!
//! # Events (`EventCursor`, FR-F4M-9's rule)
//!
//! Every anchored clock and every trigger has an `EventCursor`. A clock that arrives in a **snapshot** is
//! `joined` (a late join never replays the past). A layer's first clocks from a **delta** are `fresh` only
//! when the client receives them at or before their anchor tick with the animation at time 0, else
//! `joined`; later anchors go straight to `advance`, which classifies them itself (continuous, seek,
//! restart). A **trigger message** is `fresh` when it is heard of within [`FRESH_TRIGGER_SECONDS`] of its
//! start (a one-shot that just began: its first events, at time 0 included, still fire), `joined`
//! otherwise (and always `joined` when it comes in a snapshot).
use super::{CustomType, SyncedValue};
use crate::lan::mob_sync::from_wire;
use horizon_mobmodel::{
    clock::AnimClock,
    events::{EventBuf, EventCursor, EventSide},
    pose::RigSource,
};
use horizon_protocol::mob_sync::{
    self as wire, Delta, Frame, LayerAnchor, MAX_LAYERS, MAX_VALUES, Message, Snapshot, Trigger, TriggerStart,
    TRIGGER_RING, speed_from_byte,
};
use std::{collections::HashMap, sync::Arc};

/// How old a trigger may be when the client first hears of it and still get its first events
/// (seconds of its own clock).
pub const FRESH_TRIGGER_SECONDS: f64 = 0.25;

/// A tracked value changed on the client (the `synced-changed` notification: the script hook and the
/// animation inputs read it later).
#[derive(Clone, Debug, PartialEq)]
pub struct SyncedChanged {
    pub entity: i32,
    pub slot: usize,
    pub name: String,
    pub old: SyncedValue,
    pub new: SyncedValue,
}

/// The clocks of one controller layer's current state, one per animation slot.
#[derive(Debug)]
pub struct RemoteLayer {
    /// The controller state id.
    pub state: u8,
    pub clocks: Vec<AnimClock>,
    cursors: Vec<EventCursor>,
    /// The frame time of the anchors.
    time: i64,
}

/// A trigger on the ring.
#[derive(Debug)]
pub struct RemoteTrigger {
    pub trigger: Trigger,
    pub clock: AnimClock,
    cursor: EventCursor,
}

/// What a client holds of one custom mob.
#[derive(Debug)]
pub struct RemoteCustomMob {
    pub kind: Arc<CustomType>,
    /// The tracked values, by slot (the type's defaults until a snapshot).
    pub values: Vec<SyncedValue>,
    value_time: [i64; MAX_VALUES],
    pub layers: Vec<Option<RemoteLayer>>,
    /// The recent triggers, oldest first.
    pub triggers: Vec<RemoteTrigger>,
    pub hitbox_mask: u32,
    hitbox_time: i64,
    /// The server's type does not match this client's: nothing is applied.
    pub incompatible: bool,
}

impl RemoteCustomMob {
    fn new(kind: Arc<CustomType>) -> Self {
        Self {
            values: kind.synced.iter().map(|d| d.default.clone()).collect(),
            hitbox_mask: match kind.hitboxes.len() {
                n if n >= 32 => u32::MAX,
                n => (1u32 << n) - 1,
            },
            kind,
            value_time: [i64::MIN; MAX_VALUES],
            layers: (0..MAX_LAYERS).map(|_| None).collect(),
            triggers: Vec::new(),
            hitbox_time: i64::MIN,
            incompatible: false,
        }
    }

    /// The animation time (seconds) of slot `slot` of layer `layer` at `tick` plus `partial`.
    pub fn time_at(&self, layer: usize, slot: usize, tick: i64, partial: f32) -> Option<f64> {
        let l = self.layers.get(layer)?.as_ref()?;
        Some(l.clocks.get(slot)?.time_at(tick, partial))
    }

    /// The value named `name`.
    pub fn value(&self, name: &str) -> Option<&SyncedValue> {
        self.values.get(self.kind.synced_slot(name)?)
    }

    /// The events the client side of the animations gives between the last call and `tick` plus
    /// `partial` (sounds, particles and custom events of every animation, for the cosmetic hook), into
    /// `buf`. Triggers that have ended are dropped from the ring's cursors.
    pub fn poll_events(&mut self, tick: i64, partial: f32, buf: &mut EventBuf) {
        let Some(ta) = self.kind.anim.as_ref() else {
            return;
        };
        for (li, layer) in self.layers.iter_mut().enumerate() {
            let (Some(layer), Some(c)) = (layer, ta.rig.controllers().get(li)) else {
                continue;
            };
            let Some(state) = c.states.get(usize::from(layer.state)) else {
                continue;
            };
            for (slot, (clock, cursor)) in layer.clocks.iter().zip(&mut layer.cursors).enumerate() {
                let w = cursor.advance(clock, tick, partial);
                if let Some(a) = state.animations.get(slot).filter(|_| !w.is_empty()) {
                    ta.events.animation_window(
                        &ta.rig,
                        EventSide::Client,
                        li,
                        RigSource::State { state: u16::from(layer.state), slot: slot as u16 },
                        a.index,
                        w.c0,
                        w.c1,
                        buf,
                    );
                }
            }
        }
        for t in &mut self.triggers {
            let Some(def) = ta.triggers.get(usize::from(t.trigger.id)) else {
                continue;
            };
            let w = t.cursor.advance(&t.clock, tick, partial);
            if !w.is_empty() {
                ta.events
                    .trigger_window(&ta.rig, EventSide::Client, def.layer, def.index, w.c0, w.c1, buf);
            }
        }
        // A trigger a second past its end is forgotten (the server's ring drops them too).
        let kind = &self.kind;
        self.triggers.retain(|t| {
            let length = kind
                .anim
                .as_ref()
                .and_then(|a| a.triggers.get(usize::from(t.trigger.id)))
                .map_or(0.0, |d| f64::from(d.length));
            t.clock.time_at(tick, partial) < length + 1.0
        });
    }
}

/// What applying a frame did, for tests and the debug view.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Applied {
    pub snapshots: usize,
    pub deltas: usize,
    pub triggers: usize,
    /// Messages for an entity the client has no store for (or a type it does not have).
    pub dropped: usize,
    /// Messages for a mob whose type differs from the server's.
    pub incompatible: usize,
}

/// Every custom mob the client knows from `horizon:mob_sync`, by entity id.
#[derive(Debug, Default)]
pub struct RemoteCustomMobs {
    mobs: HashMap<i32, RemoteCustomMob>,
    changes: Vec<SyncedChanged>,
}

impl RemoteCustomMobs {
    pub fn get(&self, entity: i32) -> Option<&RemoteCustomMob> {
        self.mobs.get(&entity)
    }
    pub fn get_mut(&mut self, entity: i32) -> Option<&mut RemoteCustomMob> {
        self.mobs.get_mut(&entity)
    }
    pub fn len(&self) -> usize {
        self.mobs.len()
    }
    pub fn is_empty(&self) -> bool {
        self.mobs.is_empty()
    }
    /// The entity left the client (`RemoveEntities`).
    pub fn remove(&mut self, entity: i32) {
        self.mobs.remove(&entity);
    }
    /// A new world or connection.
    pub fn clear(&mut self) {
        self.mobs.clear();
        self.changes.clear();
    }
    /// The `synced-changed` notifications since the last call, in order.
    pub fn take_changes(&mut self) -> Vec<SyncedChanged> {
        std::mem::take(&mut self.changes)
    }

    /// Decodes a payload and applies it. `kind_of` gives the custom type of an entity the client
    /// has (by its `AddEntity`). An undecodable payload is an error and changes nothing.
    pub fn apply_payload(
        &mut self,
        data: &[u8],
        kind_of: &dyn Fn(i32) -> Option<Arc<CustomType>>,
    ) -> Result<Applied, horizon_protocol::Error> {
        let frame = wire::decode(data)?;
        Ok(self.apply(&frame, kind_of))
    }

    /// Applies a frame (see the module docs for the rules).
    pub fn apply(&mut self, frame: &Frame, kind_of: &dyn Fn(i32) -> Option<Arc<CustomType>>) -> Applied {
        self.apply_at(frame, frame.game_time, kind_of)
    }

    /// [`apply`](Self::apply) for a frame that reaches the client when its own time is `arrival` (the
    /// server game time as the client estimates it): how late a trigger is heard of, and where a joined
    /// clock starts, depend on it. The frame's own time orders the pieces.
    pub fn apply_at(
        &mut self,
        frame: &Frame,
        arrival: i64,
        kind_of: &dyn Fn(i32) -> Option<Arc<CustomType>>,
    ) -> Applied {
        let mut done = Applied::default();
        for m in &frame.messages {
            match m {
                Message::Snapshot(s) => self.snapshot(frame.game_time, arrival, s, kind_of, &mut done),
                Message::Delta(d) => match self.mobs.get_mut(&d.entity) {
                    Some(mob) if mob.incompatible => done.incompatible += 1,
                    Some(mob) => {
                        apply_delta(mob, frame.game_time, arrival, d, &mut self.changes);
                        done.deltas += 1;
                    }
                    None => done.dropped += 1,
                },
                Message::Trigger(t) => match self.mobs.get_mut(&t.entity) {
                    Some(mob) if mob.incompatible => done.incompatible += 1,
                    Some(mob) => {
                        add_trigger(mob, arrival, t, false);
                        done.triggers += 1;
                    }
                    None => done.dropped += 1,
                },
            }
        }
        done
    }

    fn snapshot(
        &mut self,
        now: i64,
        arrival: i64,
        s: &Snapshot,
        kind_of: &dyn Fn(i32) -> Option<Arc<CustomType>>,
        done: &mut Applied,
    ) {
        if !self.mobs.contains_key(&s.entity) {
            let Some(kind) = kind_of(s.entity) else {
                done.dropped += 1;
                return;
            };
            self.mobs.insert(s.entity, RemoteCustomMob::new(kind));
        }
        let Some(mob) = self.mobs.get_mut(&s.entity) else {
            return;
        };
        mob.incompatible = mob.kind.sync_version != s.type_version;
        if mob.incompatible {
            done.incompatible += 1;
            return;
        }
        // Everything in a snapshot is as new as its frame; the clocks are late-joined.
        let as_delta = Delta {
            entity: s.entity,
            values: Some(s.values.clone()),
            anchors: s.anchors.clone(),
            hitbox_mask: Some(s.hitbox_mask),
        };
        apply_values(mob, now, &as_delta, &mut self.changes);
        for a in &s.anchors {
            apply_anchor(mob, now, arrival, a, true);
        }
        mob.hitbox_mask = if now >= mob.hitbox_time { s.hitbox_mask } else { mob.hitbox_mask };
        mob.hitbox_time = mob.hitbox_time.max(now);
        for t in &s.triggers {
            add_trigger(mob, arrival, &TriggerStart { entity: s.entity, trigger: *t }, true);
        }
        done.snapshots += 1;
    }
}

fn apply_values(mob: &mut RemoteCustomMob, now: i64, d: &Delta, changes: &mut Vec<SyncedChanged>) {
    let Some(set) = &d.values else {
        return;
    };
    for (slot, v) in set.iter() {
        let (Some(def), Some(held)) = (mob.kind.synced.get(slot), mob.values.get_mut(slot)) else {
            continue;
        };
        if now < mob.value_time[slot] {
            continue;
        }
        let Some(new) = from_wire(def, v) else {
            continue;
        };
        mob.value_time[slot] = now;
        if *held != new {
            let old = std::mem::replace(held, new.clone());
            changes.push(SyncedChanged { entity: d.entity, slot, name: def.name.clone(), old, new });
        }
    }
}

fn apply_delta(mob: &mut RemoteCustomMob, now: i64, arrival: i64, d: &Delta, changes: &mut Vec<SyncedChanged>) {
    apply_values(mob, now, d, changes);
    for a in &d.anchors {
        apply_anchor(mob, now, arrival, a, false);
    }
    if let Some(mask) = d.hitbox_mask
        && now >= mob.hitbox_time
    {
        mob.hitbox_mask = mask;
        mob.hitbox_time = now;
    }
}

fn clock(a: &LayerAnchor, i: usize) -> AnimClock {
    let (time, speed) = a.slots[i];
    AnimClock::new(a.state, a.tick, f64::from(time), speed)
}

fn apply_anchor(mob: &mut RemoteCustomMob, now: i64, arrival: i64, a: &LayerAnchor, joined: bool) {
    let Some(slot) = mob.layers.get_mut(usize::from(a.layer)) else {
        return;
    };
    if slot.as_ref().is_some_and(|l| now < l.time) {
        return;
    }
    let clocks: Vec<AnimClock> = (0..a.slots.len()).map(|i| clock(a, i)).collect();
    // Cursors survive an anchor for the same number of slots (`advance` classifies the change);
    // otherwise they begin: late joined, or fresh for a clock the client sees start.
    let cursors = match slot.take() {
        Some(old) if old.cursors.len() == clocks.len() => old.cursors,
        _ => {
            let fresh = !joined && arrival <= a.tick && a.slots.iter().all(|s| s.0 == 0.0);
            clocks
                .iter()
                .map(|c| if fresh { EventCursor::fresh(c) } else { EventCursor::joined(c, arrival, 0.0) })
                .collect()
        }
    };
    *slot = Some(RemoteLayer { state: a.state, clocks, cursors, time: now });
}

fn add_trigger(mob: &mut RemoteCustomMob, arrival: i64, t: &TriggerStart, from_snapshot: bool) {
    let tr = t.trigger;
    if mob.triggers.iter().any(|x| x.trigger.id == tr.id && x.trigger.start_tick == tr.start_tick) {
        return;
    }
    let clock = AnimClock::new(0, tr.start_tick, 0.0, speed_from_byte(tr.speed));
    let age = clock.time_at(arrival, 0.0);
    let cursor = if !from_snapshot && age <= FRESH_TRIGGER_SECONDS {
        EventCursor::fresh(&clock)
    } else {
        EventCursor::joined(&clock, arrival, 0.0)
    };
    // The same trigger played again replaces its older run (an older one than ours is a stale copy);
    // the ring keeps the newest four.
    if mob.triggers.iter().any(|x| x.trigger.id == tr.id && x.trigger.start_tick > tr.start_tick) {
        return;
    }
    mob.triggers.retain(|x| x.trigger.id != tr.id);
    if mob.triggers.len() >= TRIGGER_RING {
        mob.triggers.remove(0);
    }
    // A trigger older than one the ring already has is placed by start tick.
    let at = mob.triggers.iter().position(|x| x.trigger.start_tick > tr.start_tick).unwrap_or(mob.triggers.len());
    mob.triggers.insert(at, RemoteTrigger { trigger: tr, clock, cursor });
}

#[cfg(test)]
mod tests;
