//! The server's animation runtime of a custom mob (FR-F4-MOBS.md sections 3.5 and 3.6, F4M-5b).
//!
//! Every mob of a type with a model, animations and controllers keeps a [`MobAnim`]: its controller
//! rig state (`horizon-mobmodel`), the high-water marks of the timeline events it emitted, and the
//! one-shot triggers it plays. [`tick`] runs it once per server tick, before the mob's goals:
//!
//! 1. the trigger names the attack goals asked for (`attacks::take_triggers`) and the ones a script
//!    asked for ([`request_trigger`]) start playing;
//! 2. the rig advances by one tick (0.05 s) from the mob's inputs: ground speed, health, the tracked
//!    values..., the controllers' transitions run, and the **server timeline events** of the
//!    animations flagged `model.server_events` come out of `MobEvents::update_marked` and
//!    `trigger_window_marked`;
//! 3. every `Custom` event is handed to `attacks::deliver_frame`, so a goal that waits for its
//!    `damage_frame` strikes at the keyframe (the goal's `damage_delay` is only the safety net, see
//!    [`frame_deadline`]);
//! 4. the anchors of the layers' current states are taken from the integrated clocks.
//!
//! # What the clients are told
//!
//! Animation time is piecewise linear (the reviewer's decision): the server sends, per controller
//! layer, `(state id, anchor tick, anchor time, speed)` for each animation slot of the current state
//! and sends a **new anchor whenever the state changes or the speed has changed so much that a client
//! extrapolating from the old anchor would be more than [`ANCHOR_TOLERANCE`] seconds away from the
//! server's clock** ([`needs_anchor`]; with a tolerance of 0 it is "whenever the speed or state
//! changes" exactly). A client computes `time = anchor_time + (now_tick + partial - anchor_tick) * speed / 20`
//! (`AnimClock`). Triggers are `(trigger id, start tick, speed)`: the global index of the trigger over
//! the layers' trigger lists in file order.
//!
//! # Ticks
//!
//! The anchor of tick `T` holds the clocks after the update of tick `T`, and a trigger started in tick
//! `T` has `start_tick = T - 1`, so the client's time at tick `T` is the server's clock after tick `T`'s
//! update and an event of tick `T` is the one in the client window that ends at `T`. A trigger the goal
//! asked for during tick `N` (goals run after the animation) starts in tick `N + 1`; its keyframe at
//! 0.35 s is emitted in tick `N + 8`, which is also the tick of the goal's blow.
use super::CustomType;
use crate::mobs::{Mob, MobWorld, custom::ai::attacks};
use horizon_mods::custom_mob::LoadedModel;
use horizon_mobmodel::{
    anim::EventKind,
    controller::MAX_STATE_ANIMS,
    events::{EventBuf, EventMarks, EventSide, MobEventKind, MobEvents},
    expr::{ExprInputs, Query},
    pose::{Rig, RigState},
};
use horizon_protocol::mob_sync::{LayerAnchor, TRIGGER_RING, Trigger, speed_byte, speed_from_byte};
use std::cell::RefCell;

/// Seconds of animation time per game tick.
pub const TICK_SECONDS: f32 = 0.05;
/// **Re-anchor extrapolation tolerance**: how far (seconds of animation time) a client that keeps
/// extrapolating from the last anchor may be from the server's clock before the server sends a new
/// anchor (`needs_anchor`). 20 ms is less than half a tick (50 ms), so a client is never visibly
/// off, and it stops a walking mob whose `q.ground_speed * 4` jitters by a hair every tick from
/// sending a message every tick (about 10 anchors a second, over the per-player budget with a
/// couple of dozen mobs). A state change always re-anchors at once, and `0.0` means "whenever the
/// speed changes at all" (tests pin both). Reviewer decision on F4M-5b.
pub const ANCHOR_TOLERANCE: f64 = 0.02;
/// Ticks added to an animation's length to make the safety deadline of a damage frame.
const FRAME_SLACK_TICKS: i64 = 4;
/// The animation a dying mob plays (`death`); a type without one keeps the engine's 20 ticks and tilt.
pub const DEATH_ANIMATION: &str = "death";
/// The fewest ticks a custom mob with a death animation is kept (the engine's own 20), so XP/drops and
/// other `death_time` timing stay vanilla-like.
pub const MIN_DEATH_TICKS: i32 = 20;
/// The most ticks a custom mob is kept for its death animation.
pub const MAX_DEATH_TICKS: i32 = 60;

/// One trigger of a type: the layer and its place in the layer's list.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggerDef {
    pub layer: usize,
    pub index: usize,
    pub name: String,
    /// The length in seconds of the animation it plays.
    pub length: f32,
    /// The animation it plays (index into the library).
    pub animation: usize,
}

/// A server timeline event of an animation flagged `server_events`.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerFrame {
    pub name: String,
    pub animation: usize,
    pub length: f32,
}

/// What a type shares between its mobs: the rig and its events, the trigger table.
#[derive(Debug)]
pub struct TypeAnim {
    pub rig: Rig,
    pub events: MobEvents,
    /// Every trigger of every layer, in file order: a trigger's wire id is its index here.
    pub triggers: Vec<TriggerDef>,
    pub frames: Vec<ServerFrame>,
    /// The type's `death` animation (index into the library) and its length in seconds: a dying mob is
    /// kept until it ends (F4M-6), and the client plays it instead of the engine's tilt.
    pub death: Option<(usize, f32)>,
}

impl TypeAnim {
    /// The type's runtime from its loaded model; `None` when the type has no rig.
    pub fn new(loaded: &LoadedModel) -> Option<Self> {
        let (rig, events) = (loaded.rig.as_ref()?, loaded.events.as_ref()?);
        let mut triggers = Vec::new();
        for (layer, c) in rig.controllers().iter().enumerate() {
            for (index, t) in c.triggers.iter().enumerate() {
                let length = rig.animation(t.index).map_or(0.0, |a| a.length);
                triggers.push(TriggerDef {
                    layer,
                    index,
                    name: t.name.clone(),
                    length,
                    animation: t.index,
                });
            }
        }
        let mut frames = Vec::new();
        for (animation, a) in rig.animations().iter().enumerate() {
            if !events.is_server_animation(animation) {
                continue;
            }
            for e in a.events.iter().filter(|e| e.kind == EventKind::Timeline) {
                frames.push(ServerFrame {
                    name: e.name.clone(),
                    animation,
                    length: a.length,
                });
            }
        }
        let death = rig
            .find_animation(DEATH_ANIMATION)
            .and_then(|i| rig.animation(i).map(|a| (i, a.length)))
            .filter(|(_, length)| length.is_finite() && *length > 0.0);
        Some(Self {
            rig: rig.clone(),
            events: events.clone(),
            triggers,
            frames,
            death,
        })
    }

    /// How many ticks a dying mob is kept: the `death` animation's length clamped to
    /// [`MIN_DEATH_TICKS`]..=[`MAX_DEATH_TICKS`] (a shorter clip holds its last pose); `None` without it.
    pub fn death_ticks(&self) -> Option<i32> {
        self.death.map(|(_, length)| ((length / TICK_SECONDS - 1e-3).ceil() as i32).clamp(MIN_DEATH_TICKS, MAX_DEATH_TICKS))
    }

    /// The wire id of the first trigger named `name`.
    pub fn trigger_id(&self, name: &str) -> Option<u16> {
        self.triggers
            .iter()
            .position(|t| t.name == name)
            .and_then(|i| u16::try_from(i).ok())
    }

    /// Feeds `h` the shape of what clients read: the layers, their states and animation slots, and
    /// the trigger names. A client whose type differs from the server's must not read the mob's data.
    pub fn signature(&self, h: &mut impl FnMut(&[u8])) {
        for c in self.rig.controllers() {
            h(c.name.as_bytes());
            h(&[c.states.len().min(255) as u8]);
            for s in &c.states {
                h(&[s.animations.len().min(255) as u8]);
            }
        }
        for t in &self.triggers {
            h(t.name.as_bytes());
        }
    }
}

/// The version of a type's sync layout: its id, tracked values and animation shape.
pub fn type_version(t: &CustomType) -> u32 {
    let mut hash = 0x811c_9dc5u32;
    let mut feed = |bytes: &[u8]| {
        for b in bytes.iter().chain(&[0xFF]) {
            hash ^= u32::from(*b);
            hash = hash.wrapping_mul(0x0100_0193);
        }
    };
    feed(t.id.as_bytes());
    for s in &t.synced {
        feed(s.name.as_bytes());
        feed(s.kind.as_bytes());
        s.values.iter().for_each(|v| feed(v.as_bytes()));
        feed(&s.min.unwrap_or(i32::MIN).to_be_bytes());
        feed(&s.max.unwrap_or(i32::MAX).to_be_bytes());
    }
    if let Some(a) = &t.anim {
        a.signature(&mut feed);
    }
    hash
}

/// A trigger being played.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    id: u16,
    start_tick: i64,
    speed: f32,
    /// Windows processed so far.
    ticks: u32,
}

impl Run {
    fn wire(&self) -> Trigger {
        Trigger {
            id: self.id,
            start_tick: self.start_tick,
            speed: speed_byte(self.speed),
        }
    }
}

/// What the previous tick left, for the motion queries.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Motion {
    position: [f64; 3],
    body_yaw: f32,
}

/// One mob's animation state (in `CustomState.anim`).
#[derive(Clone, Debug)]
pub struct MobAnim {
    state: RigState,
    marks: EventMarks,
    inputs: ExprInputs,
    /// The layers' anchors as of the last tick.
    current: Vec<LayerAnchor>,
    /// The anchors the sync layer last sent.
    sent: Vec<Option<LayerAnchor>>,
    runs: Vec<Run>,
    /// The last [`TRIGGER_RING`] triggers started, oldest first.
    ring: Vec<Trigger>,
    /// Triggers started since the sync layer last took them.
    started: Vec<Trigger>,
    /// Triggers asked for by name, started by the next tick.
    pending: Vec<(String, f32)>,
    motion: Option<Motion>,
    /// The game time of the last tick.
    now: i64,
}

thread_local! {
    static EVENTS: RefCell<EventBuf> = RefCell::new(EventBuf::with_capacity(64));
}

impl MobAnim {
    pub fn new(t: &TypeAnim, seed: u32, now: i64) -> Self {
        let state = t.rig.initial_state();
        let layers = t.rig.controllers().len();
        let mut inputs = ExprInputs::default();
        inputs.seed = seed;
        let mut me = Self {
            marks: EventMarks::new(&t.rig),
            state,
            inputs,
            current: (0..layers)
                .map(|l| LayerAnchor {
                    layer: l as u8,
                    state: 0,
                    tick: now,
                    slots: Vec::new(),
                })
                .collect(),
            sent: vec![None; layers],
            runs: Vec::new(),
            ring: Vec::new(),
            started: Vec::new(),
            pending: Vec::new(),
            motion: None,
            now,
        };
        me.refresh_anchors(t, now);
        me
    }

    /// Recomputes `current` from the integrated clocks at `tick`.
    fn refresh_anchors(&mut self, t: &TypeAnim, tick: i64) {
        for (li, c) in t.rig.controllers().iter().enumerate() {
            let (Some(ls), Some(out)) = (self.state.layers.get(li), self.current.get_mut(li)) else {
                continue;
            };
            let n = c
                .states
                .get(ls.cur.state)
                .map_or(0, |s| s.animations.len())
                .min(MAX_STATE_ANIMS);
            out.state = c.state_id(ls);
            out.tick = tick;
            out.slots.clear();
            for slot in 0..n {
                let a = c.anchor(ls, slot, tick, &mut self.inputs);
                out.slots.push((a.anchor_time as f32, a.speed));
            }
        }
    }

    /// The anchors of the layers as of the last tick.
    pub fn anchors(&self) -> &[LayerAnchor] {
        &self.current
    }

    /// The anchors that must be sent now: the layers whose state changed or whose clock a client
    /// would extrapolate wrongly from the last anchor sent ([`needs_anchor`]); they count as sent.
    pub fn take_anchor_changes(&mut self, tolerance: f64) -> Vec<LayerAnchor> {
        let mut out = Vec::new();
        for (cur, sent) in self.current.iter().zip(&mut self.sent) {
            if sent.as_ref().is_none_or(|s| needs_anchor(s, cur, tolerance)) {
                *sent = Some(cur.clone());
                out.push(cur.clone());
            }
        }
        out
    }

    /// Whether [`take_anchor_changes`](Self::take_anchor_changes) would give anything.
    pub fn anchors_changed(&self, tolerance: f64) -> bool {
        self.current
            .iter()
            .zip(&self.sent)
            .any(|(cur, sent)| sent.as_ref().is_none_or(|s| needs_anchor(s, cur, tolerance)))
    }

    /// The triggers started since the last call, oldest first.
    pub fn take_started(&mut self) -> Vec<Trigger> {
        std::mem::take(&mut self.started)
    }

    /// The triggers a client that joins now should still see playing (the ring, minus the ones that
    /// ended), oldest first.
    pub fn recent_triggers(&self, t: &TypeAnim) -> Vec<Trigger> {
        self.ring
            .iter()
            .filter(|tr| {
                let length = t.triggers.get(usize::from(tr.id)).map_or(0.0, |d| d.length);
                let elapsed = (self.now - tr.start_tick) as f32 * TICK_SECONDS * speed_from_byte(tr.speed);
                elapsed < length + TICK_SECONDS
            })
            .copied()
            .collect()
    }

    fn start(&mut self, t: &TypeAnim, name: &str, speed: f32, now: i64) -> bool {
        let Some(id) = t.trigger_id(name) else {
            return false;
        };
        let def = &t.triggers[usize::from(id)];
        // The wire speed is a byte of sixteenths: the server plays exactly what it sends.
        let speed = speed_from_byte(speed_byte(speed));
        self.marks.reset_trigger(def.layer, def.index);
        let run = Run {
            id,
            start_tick: now - 1,
            speed,
            ticks: 0,
        };
        self.runs.retain(|r| r.id != id);
        self.runs.push(run);
        self.ring.retain(|r| r.id != id);
        if self.ring.len() >= TRIGGER_RING {
            self.ring.remove(0);
        }
        self.ring.push(run.wire());
        self.started.push(run.wire());
        true
    }
}

/// Whether a client that extrapolates from `sent` is wrong about `cur` (the same layer, a later
/// tick): a different state or number of slots, or a slot whose clock is more than `tolerance`
/// seconds from the extrapolation, or one that stopped or started.
pub fn needs_anchor(sent: &LayerAnchor, cur: &LayerAnchor, tolerance: f64) -> bool {
    if sent.state != cur.state || sent.slots.len() != cur.slots.len() {
        return true;
    }
    let ticks = cur.tick.wrapping_sub(sent.tick) as f64;
    sent.slots.iter().zip(&cur.slots).any(|(s, c)| {
        let predicted = f64::from(s.0) + ticks * f64::from(s.1) / 20.0;
        (s.1 == 0.0) != (c.1 == 0.0)
            || (predicted - f64::from(c.0)).abs() > tolerance
            // A tolerance of 0 means every change of speed.
            || (tolerance <= 0.0 && s.1 != c.1)
    })
}

/// What the expressions of a mob's animation read each tick (the engine's inputs plus the motion
/// queries).
fn fill_inputs(m: &Mob, w: &MobWorld, anim: &mut MobAnim) {
    super::ai::expr::fill(&mut anim.inputs, m, w);
    let i = &mut anim.inputs;
    let at = [m.position.x, m.position.y, m.position.z];
    let (ground, vertical, yaw) = match anim.motion {
        Some(last) => (
            ((at[0] - last.position[0]).powi(2) + (at[2] - last.position[2]).powi(2)).sqrt() as f32,
            (at[1] - last.position[1]) as f32,
            wrap(m.body_yaw - last.body_yaw),
        ),
        None => (0.0, 0.0, 0.0),
    };
    anim.motion = Some(Motion {
        position: at,
        body_yaw: m.body_yaw,
    });
    i.set(Query::GroundSpeed, ground);
    i.set(Query::VerticalSpeed, vertical);
    i.set(Query::YawSpeed, yaw);
    i.set(Query::HeadXRotation, m.head_pitch);
    i.set(Query::HeadYRotation, wrap(m.head_yaw - m.body_yaw));
    i.set(Query::BodyYRotation, m.body_yaw);
    i.set(Query::DeathTime, m.death_time as f32);
}

fn wrap(deg: f32) -> f32 {
    let d = deg.rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

/// Asks for the trigger `name` to play at `speed` (1 is real time) from the next tick. False for a
/// mob that is not a custom one or whose type has no such trigger.
pub fn request_trigger(m: &mut Mob, name: &str, speed: f32) -> bool {
    let Some(t) = m.kind.custom.clone() else {
        return false;
    };
    let (Some(a), Some(state)) = (&t.anim, m.extra.custom.as_mut()) else {
        return false;
    };
    if a.trigger_id(name).is_none() {
        return false;
    }
    state
        .anim
        .get_or_insert_with(|| Box::new(MobAnim::new(a, m.id, 0)))
        .pending
        .push((name.to_owned(), speed));
    true
}

/// Forces the animation controller `controller` of `m` into its state `state` (the script
/// `set-controller-state`): the state is entered at once with its fade, and the controller's own
/// transitions may leave it again on a later tick. False for a mob without animations or a name
/// the type does not have.
pub fn force_state(m: &mut Mob, controller: &str, state: &str) -> bool {
    let Some(t) = m.kind.custom.clone() else {
        return false;
    };
    let (Some(a), Some(cs)) = (&t.anim, m.extra.custom.as_mut()) else {
        return false;
    };
    let Some(li) = a.rig.controllers().iter().position(|c| c.name == controller) else {
        return false;
    };
    let Some(si) = a.rig.controllers()[li].states.iter().position(|s| s.name == state) else {
        return false;
    };
    let anim = cs.anim.get_or_insert_with(|| Box::new(MobAnim::new(a, m.id, 0)));
    let MobAnim { state: rig, inputs, .. } = &mut **anim;
    let Some(ls) = rig.layers.get_mut(li) else {
        return false;
    };
    a.rig.controllers()[li].set_state(ls, si, true, inputs);
    true
}

/// One server tick of the mob's animation (see the module docs). Nothing happens for a mob whose
/// type has no animations. `w.game_time` is the tick.
pub fn tick(m: &mut Mob, w: &MobWorld) {
    let Some(t) = m.kind.custom.clone() else {
        return;
    };
    let Some(ta) = &t.anim else {
        return;
    };
    let Some(state) = m.extra.custom.as_mut() else {
        return;
    };
    let now = w.game_time;
    let mut anim = state
        .anim
        .take()
        .unwrap_or_else(|| Box::new(MobAnim::new(ta, m.id, now)));
    anim.now = now;
    // The animations the goals asked for, and the ones a script did.
    let mut names = attacks::take_triggers(m);
    for name in names.drain(..) {
        if !anim.start(ta, &name, 1.0, now) {
            log::debug!("Horizon: {}: no trigger named {name:?}", t.id);
        }
    }
    for (name, speed) in std::mem::take(&mut anim.pending) {
        anim.start(ta, &name, speed, now);
    }
    fill_inputs(m, w, &mut anim);
    // The rig and the triggers, one tick; the server's timeline events collect in the buffer.
    let frames: Vec<String> = EVENTS.with(|buf| {
        let mut buf = buf.borrow_mut();
        buf.clear();
        let MobAnim {
            state,
            marks,
            inputs,
            runs,
            ..
        } = &mut *anim;
        ta.events
            .update_marked(&ta.rig, EventSide::Server, state, TICK_SECONDS, inputs, marks, &mut buf);
        for run in runs.iter_mut() {
            let def = &ta.triggers[usize::from(run.id)];
            let step = TICK_SECONDS * run.speed;
            // Computed from the tick count, not summed: no drift, the same at every speed.
            let (c0, c1) = (run.ticks as f32 * step, (run.ticks + 1) as f32 * step);
            if c1 > c0 {
                ta.events
                    .trigger_window_marked(&ta.rig, EventSide::Server, def.layer, def.index, c0, c1, marks, &mut buf);
            }
            run.ticks += 1;
        }
        buf.as_slice()
            .iter()
            .filter_map(|e| match e.kind {
                MobEventKind::Custom { name, .. } => Some(ta.events.string(name).to_owned()),
                _ => None,
            })
            .collect()
    });
    // A trigger is over once its clock has passed its animation.
    anim.runs.retain(|r| {
        let def = &ta.triggers[usize::from(r.id)];
        r.speed > 0.0 && r.ticks as f32 * TICK_SECONDS * r.speed < def.length
    });
    anim.refresh_anchors(ta, now);
    if let Some(state) = m.extra.custom.as_mut() {
        state.anim = Some(anim);
    }
    for name in frames {
        log::info!("Horizon custom-mob server frame: mob {} {name:?} tick {}", m.id, w.game_time);
        attacks::deliver_frame(m, &name);
    }
}

/// The tick after which an attack that waits for the server timeline event `frame` gives up and
/// strikes anyway, counted from the tick it began: `Some` when the type's server animations have such
/// an event (the keyframe decides when the blow lands; this is only the safety net, the length of
/// the animation that has it, preferring the attack's own `trigger`, plus a few ticks), `None` when
/// they have not (then the goal's own `damage_delay` is the whole mechanism, as in F4M-4a).
pub fn frame_deadline(t: &CustomType, trigger: Option<&str>, frame: &str) -> Option<i64> {
    let a = t.anim.as_ref()?;
    let mut candidates = a.frames.iter().filter(|f| f.name == frame);
    let first = candidates.next()?;
    let own = trigger
        .and_then(|name| a.triggers.iter().find(|d| d.name == name))
        .and_then(|d| a.frames.iter().find(|f| f.name == frame && f.animation == d.animation));
    let length = match own {
        Some(f) => f.length,
        None => candidates.fold(first.length, |l, f| l.max(f.length)),
    };
    Some((length / TICK_SECONDS).ceil() as i64 + FRAME_SLACK_TICKS)
}

#[cfg(test)]
#[path = "anim_sync_tests.rs"]
pub(crate) mod tests;
