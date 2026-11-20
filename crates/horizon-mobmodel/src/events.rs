//! Typed animation events for the engine (FR-F4-MOBS.md sections 3.6, 4 "Animation events", 6.1, 6.2; F4M-9).
//!
//! [`crate::anim`] keeps the raw timeline (`timeline`, `sound_effects`, `particle_effects` with names and locator
//! names) and [`crate::pose::Rig`] says which of them fired in a clock window. This module turns that into values the
//! engine consumes without parsing anything:
//!
//! * [`MobAnimEvent`] with a [`MobEventKind`]: `Sound { sound, volume, pitch, locator }`, `Particle { particle,
//!   locator, offset }` and `Custom { name, locator }` (a `timeline` name). Strings are interned in the [`MobEvents`]
//!   table ([`StrId`]), so an event is `Copy` and emitting one never allocates. Every event carries the animation index,
//!   the exact keyframe time, the layer and the slot it came from ([`RigSource`]: a state's animation list or a
//!   trigger) and the loop index.
//! * **Binding at load** ([`MobEvents::build`]): locator names become [`LocatorIndex`] values (an unknown locator is a
//!   load error naming the file and the path), particle names go through the type's `particles` table to a vanilla or
//!   mod particle id, and ids are checked. Parsing of the type JSON: [`parse_particles`], [`parse_server_events`],
//!   [`MobEvents::from_model_json`].
//! * **Sides** ([`EventSide`]): the **client** gets sound, particle and custom events of every animation; they are
//!   *cosmetic*. The **server** gets only custom (`timeline`) events, and only of animations flagged in the type's
//!   `server_events`; never sounds or particles. Gameplay (the damage frame, a leap impulse) must be driven by the
//!   server's custom event only; the client's copy of the same name goes to the cosmetic hook `mob-anim-event-client`
//!   and must not change game state, so nothing is applied twice.
//! * **Exactly once**: the server drives [`MobEvents::update_marked`] / [`MobEvents::trigger_window_marked`] with its
//!   integrated clock (`[c0, c1)` windows, once per loop, nothing while `dt = 0`, deterministic: same inputs give the
//!   same events in the same order) and an [`EventMarks`] high-water mark per (layer, slot), so even a repeated or
//!   rewound window never emits an event twice (no double hits). The unmarked variants are the plain windows. A client that derives time from [`AnimClock`] anchors uses an [`EventCursor`] per clock; see its
//!   rules (a late joiner never replays the past; a re-anchor that jumps backwards or forwards fires nothing).
//! * **Positions**: [`locator_position`] gives the model-space position (pixels) of a locator for a pose, from matrices
//!   computed once per frame with [`crate::skeleton::bone_matrices_into`].
//!
//! Event order inside one window is the animation's: by time, then kind (timeline, sound, particle), then declaration.
//! Across layers the order is layer order, within a layer the order of the controller's callbacks.
//!
//! Limits (hard errors at load): 256 particle table entries, 256 events per animation, 4096 events per type, ids at
//! most 128 characters of `a-z 0-9 _ . : / -` with at most one `:`, timeline names 1 to 64 characters without control
//! characters, volume 0 to 4, pitch 0.5 to 2, particle offset at most 10000 pixels per axis.

use crate::anim::{AnimEvent, AnimationLibrary, EventKind};
use crate::clock::AnimClock;
use crate::expr::ExprInputs;
use crate::model::{Locator, ModelError};
use crate::pose::{Rig, RigEvent, RigSource, RigState};
use glam::{Mat4, Vec3};
use serde_json::Value;

/// Limits of this module (see the module docs).
pub mod limits {
    pub const MAX_PARTICLES: usize = 256;
    pub const MAX_EVENTS_PER_ANIMATION: usize = 256;
    pub const MAX_EVENTS_PER_TYPE: usize = 4096;
    pub const MAX_ID_LEN: usize = 128;
    pub const MAX_NAME_LEN: usize = 64;
    /// Capacity limit of an [`super::EventBuf`].
    pub const MAX_EVENT_BUFFER: usize = 1024;
}

/// Pixels per block: locator positions are in model pixels.
pub const PIXELS_PER_BLOCK: f32 = 16.0;

/// Which side consumes the events.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EventSide {
    /// Cosmetic: sound, particle and custom events of all animations.
    Client,
    /// Gameplay: custom events of animations flagged `server_events` only.
    Server,
}

/// An interned string of a [`MobEvents`] table (sound id, particle id or custom event name).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct StrId(pub u32);

/// Index into the model's `locators`, resolved at load.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LocatorIndex(pub u16);

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MobEventKind {
    Sound { sound: StrId, volume: f32, pitch: f32, locator: Option<LocatorIndex> },
    /// `offset` is in pixels in the locator's frame (model space when there is no locator).
    Particle { particle: StrId, locator: Option<LocatorIndex>, offset: Vec3 },
    /// A `timeline` name.
    Custom { name: StrId, locator: Option<LocatorIndex> },
}

/// One event, ready for the engine.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MobAnimEvent {
    pub side: EventSide,
    /// Index into the animation library.
    pub animation: u16,
    pub layer: u16,
    /// The state animation slot or trigger it played from.
    pub source: RigSource,
    /// The keyframe's time in seconds on the animation.
    pub time: f32,
    pub loop_index: u32,
    pub kind: MobEventKind,
}

impl MobAnimEvent {
    /// Client events are cosmetic: they must not change game state.
    pub fn is_cosmetic(&self) -> bool {
        self.side == EventSide::Client
    }
}

/// A fixed-capacity event buffer: pushing never allocates, a full buffer drops the event and counts it.
#[derive(Clone, Debug)]
pub struct EventBuf {
    events: Vec<MobAnimEvent>,
    cap: usize,
    dropped: u32,
}

impl EventBuf {
    /// `capacity` is clamped to `1..=MAX_EVENT_BUFFER`. This is the only allocation.
    pub fn with_capacity(capacity: usize) -> Self {
        let cap = capacity.clamp(1, limits::MAX_EVENT_BUFFER);
        Self { events: Vec::with_capacity(cap), cap, dropped: 0 }
    }

    pub fn clear(&mut self) {
        self.events.clear();
        self.dropped = 0;
    }

    pub fn push(&mut self, e: MobAnimEvent) {
        if self.events.len() < self.cap {
            self.events.push(e);
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    pub fn as_slice(&self) -> &[MobAnimEvent] {
        &self.events
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Events dropped since the last [`EventBuf::clear`] because the buffer was full.
    pub fn dropped(&self) -> u32 {
        self.dropped
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Type JSON: particles and server_events
// ---------------------------------------------------------------------------------------------------------------

fn merr(file: &str, path: &str, message: impl Into<String>) -> ModelError {
    ModelError { file: file.to_string(), path: path.to_string(), message: message.into() }
}

fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= limits::MAX_ID_LEN
        && s.matches(':').count() <= 1
        && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | ':' | '/' | '-'))
        && !s.starts_with(':')
        && !s.ends_with(':')
}

/// `particles` of the type: the animation's particle effect name to a vanilla or mod particle id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParticleTable {
    entries: Vec<(String, String)>,
}

impl ParticleTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an entry (the id gets `minecraft:` when it has no namespace). For tests and code-built types.
    pub fn insert(&mut self, name: &str, id: &str) {
        let id = if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
        match self.entries.iter_mut().find(|(n, _)| n == name) {
            Some(e) => e.1 = id,
            None => self.entries.push((name.to_string(), id)),
        }
    }

    pub fn resolve(&self, name: &str) -> Option<&str> {
        self.entries.iter().find(|(n, _)| n == name).map(|(_, id)| id.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Parse the type's `model.particles` (`value` is the object; `path` such as `model.particles` prefixes errors).
///
/// ```text
/// "particles": { "cinder_fox_embers": "sample_mobs:ember", "puff": { "id": "minecraft:cloud" }, "dust": "smoke" }
/// ```
///
/// A value without a namespace gets `minecraft:`. Errors: not an object, more than 256 entries, an empty or overlong
/// name, an invalid id, unknown fields in the object form.
pub fn parse_particles(file: &str, path: &str, value: &Value) -> Result<ParticleTable, ModelError> {
    let obj = value.as_object().ok_or_else(|| merr(file, path, "expected an object of particle name to particle id"))?;
    if obj.len() > limits::MAX_PARTICLES {
        return Err(merr(file, path, format!("{} particles (at most {})", obj.len(), limits::MAX_PARTICLES)));
    }
    let mut t = ParticleTable::new();
    for (name, v) in obj {
        let p = format!("{path}[{name:?}]");
        if name.is_empty() || name.chars().count() > limits::MAX_NAME_LEN || name.chars().any(char::is_control) {
            return Err(merr(file, &p, format!("particle name {name:?} must be 1 to {} characters", limits::MAX_NAME_LEN)));
        }
        let id = match v {
            Value::String(s) => s.as_str(),
            Value::Object(o) => {
                if let Some(k) = o.keys().find(|k| k.as_str() != "id") {
                    return Err(merr(file, &p, format!("unknown field '{k}' (only \"id\")")));
                }
                match o.get("id").and_then(Value::as_str) {
                    Some(s) => s,
                    None => return Err(merr(file, &p, "missing \"id\" (a string)")),
                }
            }
            _ => return Err(merr(file, &p, "expected a particle id string or {\"id\": ...}")),
        };
        if !valid_id(id) {
            return Err(merr(
                file,
                &p,
                format!("invalid particle id {id:?} (1 to {} chars of a-z 0-9 _ . : / -, one ':' at most)", limits::MAX_ID_LEN),
            ));
        }
        t.insert(name, id);
    }
    Ok(t)
}

/// Parse the type's `model.server_events` into one flag per animation of `library` (library order).
///
/// Forms: `true` (all animations), `false`, an array of animation names (same lookup as controllers: exact name or
/// unique suffix) or an object `{"pounce": true, "idle": false}`. An unknown animation is an error at its path.
pub fn parse_server_events(
    file: &str,
    path: &str,
    value: &Value,
    library: &AnimationLibrary,
) -> Result<Vec<bool>, ModelError> {
    let mut flags = vec![false; library.len()];
    let find = |name: &str, p: &str| -> Result<usize, ModelError> {
        library.find(name).ok_or_else(|| merr(file, p, format!("unknown animation '{name}'")))
    };
    match value {
        Value::Bool(b) => flags.iter_mut().for_each(|f| *f = *b),
        Value::Array(a) => {
            for (i, v) in a.iter().enumerate() {
                let p = format!("{path}[{i}]");
                let name = v.as_str().ok_or_else(|| merr(file, &p, "expected an animation name"))?;
                flags[find(name, &p)?] = true;
            }
        }
        Value::Object(o) => {
            for (name, v) in o {
                let p = format!("{path}[{name:?}]");
                let b = v.as_bool().ok_or_else(|| merr(file, &p, "expected true or false"))?;
                flags[find(name, &p)?] = b;
            }
        }
        _ => return Err(merr(file, path, "expected true, false, an array of animation names or an object of flags")),
    }
    Ok(flags)
}

// ---------------------------------------------------------------------------------------------------------------
// The bound event table
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
struct Compiled {
    time: f32,
    kind: MobEventKind,
}

/// Typed events of every animation of a type, bound to the model's locators and the type's particle table.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MobEvents {
    strings: Vec<String>,
    /// Per animation, per `Animation::events` index.
    anims: Vec<Vec<Compiled>>,
    server: Vec<bool>,
}

impl MobEvents {
    /// Bind every event. `file` names the animation file in errors (path
    /// `animations["pounce"].events[2].locator`). `server_flags` has one entry per animation (missing entries count as
    /// false). A particle effect name that is not in `particles` is accepted when it already is a namespaced id
    /// (contains `:`), otherwise an error.
    pub fn build(
        file: &str,
        library: &AnimationLibrary,
        locators: &[Locator],
        particles: &ParticleTable,
        server_flags: &[bool],
    ) -> Result<MobEvents, ModelError> {
        let mut me = MobEvents::default();
        let mut total = 0usize;
        for (ai, anim) in library.iter().enumerate() {
            let base = format!("animations[{:?}]", anim.name);
            if anim.events.len() > limits::MAX_EVENTS_PER_ANIMATION {
                return Err(merr(
                    file,
                    &base,
                    format!("{} events (at most {})", anim.events.len(), limits::MAX_EVENTS_PER_ANIMATION),
                ));
            }
            total += anim.events.len();
            if total > limits::MAX_EVENTS_PER_TYPE {
                return Err(merr(file, &base, format!("more than {} events in the type", limits::MAX_EVENTS_PER_TYPE)));
            }
            let mut v = Vec::with_capacity(anim.events.len());
            for (ei, e) in anim.events.iter().enumerate() {
                let p = format!("{base}.events[{ei}]");
                v.push(me.compile(file, &p, e, locators, particles)?);
            }
            me.anims.push(v);
            me.server.push(server_flags.get(ai).copied().unwrap_or(false));
        }
        Ok(me)
    }

    /// Read `particles` and `server_events` from the type's `model` object (both optional) and [`MobEvents::build`].
    /// `type_file` names the type file in errors of those two fields (`model.particles`, `model.server_events`);
    /// `anim_file` names the animation file for the event errors.
    pub fn from_model_json(
        type_file: &str,
        anim_file: &str,
        model: &Value,
        library: &AnimationLibrary,
        locators: &[Locator],
    ) -> Result<MobEvents, ModelError> {
        let obj = model.as_object().ok_or_else(|| merr(type_file, "model", "expected a model object"))?;
        let particles = match obj.get("particles") {
            Some(v) => parse_particles(type_file, "model.particles", v)?,
            None => ParticleTable::new(),
        };
        let flags = match obj.get("server_events") {
            Some(v) => parse_server_events(type_file, "model.server_events", v, library)?,
            None => vec![false; library.len()],
        };
        Self::build(anim_file, library, locators, &particles, &flags)
    }

    fn intern(&mut self, s: &str) -> StrId {
        if let Some(i) = self.strings.iter().position(|x| x == s) {
            return StrId(i as u32);
        }
        self.strings.push(s.to_string());
        StrId((self.strings.len() - 1) as u32)
    }

    fn compile(
        &mut self,
        file: &str,
        path: &str,
        e: &AnimEvent,
        locators: &[Locator],
        particles: &ParticleTable,
    ) -> Result<Compiled, ModelError> {
        let locator = match &e.locator {
            None => None,
            Some(name) => match locators.iter().position(|l| &l.name == name) {
                Some(i) => Some(LocatorIndex(i as u16)),
                None => {
                    return Err(merr(
                        file,
                        &format!("{path}.locator"),
                        format!(
                            "unknown locator '{name}' (the model has: {})",
                            locators.iter().map(|l| l.name.as_str()).collect::<Vec<_>>().join(", ")
                        ),
                    ))
                }
            },
        };
        let kind = match e.kind {
            EventKind::Timeline => {
                if e.name.is_empty() || e.name.chars().count() > limits::MAX_NAME_LEN || e.name.chars().any(char::is_control) {
                    return Err(merr(
                        file,
                        path,
                        format!("timeline name {:?} must be 1 to {} characters, no control characters", e.name, limits::MAX_NAME_LEN),
                    ));
                }
                MobEventKind::Custom { name: self.intern(&e.name), locator }
            }
            EventKind::Sound => {
                if !valid_id(&e.name) {
                    return Err(merr(file, path, format!("invalid sound id {:?}", e.name)));
                }
                MobEventKind::Sound { sound: self.intern(&e.name), volume: e.volume, pitch: e.pitch, locator }
            }
            EventKind::Particle => {
                let id = match particles.resolve(&e.name) {
                    Some(id) => id.to_string(),
                    None if e.name.contains(':') && valid_id(&e.name) => e.name.clone(),
                    None => {
                        return Err(merr(
                            file,
                            path,
                            format!("particle effect '{}' is not in the type's particles table (and is not a namespaced id)", e.name),
                        ))
                    }
                };
                MobEventKind::Particle { particle: self.intern(&id), locator, offset: Vec3::from(e.offset) }
            }
        };
        Ok(Compiled { time: e.time, kind })
    }

    /// The text of an interned string.
    pub fn string(&self, id: StrId) -> &str {
        self.strings.get(id.0 as usize).map_or("", String::as_str)
    }

    /// The id of an already interned string (tests, hosts that map names to handlers).
    pub fn find_string(&self, s: &str) -> Option<StrId> {
        self.strings.iter().position(|x| x == s).map(|i| StrId(i as u32))
    }

    /// Whether `animation` is flagged `server_events`.
    pub fn is_server_animation(&self, animation: usize) -> bool {
        self.server.get(animation).copied().unwrap_or(false)
    }

    /// The typed event for one raw one, or `None` when `side` does not get it (server: not custom, or the animation is
    /// not flagged).
    pub fn resolve(&self, side: EventSide, layer: usize, ev: &RigEvent<'_>) -> Option<MobAnimEvent> {
        let c = self.anims.get(ev.animation)?.get(ev.event_index)?;
        if side == EventSide::Server && !(matches!(c.kind, MobEventKind::Custom { .. }) && self.is_server_animation(ev.animation)) {
            return None;
        }
        Some(MobAnimEvent {
            side,
            animation: ev.animation as u16,
            layer: layer as u16,
            source: ev.source,
            time: c.time,
            loop_index: ev.loop_index,
            kind: c.kind,
        })
    }

    /// Advance the rig by `dt` ([`Rig::update`]) and append the events `side` gets to `buf` (not cleared).
    pub fn update(
        &self,
        rig: &Rig,
        side: EventSide,
        st: &mut RigState,
        dt: f32,
        inputs: &mut ExprInputs,
        buf: &mut EventBuf,
    ) {
        rig.update(st, dt, inputs, &mut |ev| {
            if let Some(e) = self.resolve(side, ev.layer, &ev) {
                buf.push(e);
            }
        });
    }

    /// Events of a trigger animation for the clock window `[c0, c1)` ([`Rig::trigger_events`]).
    pub fn trigger_window(
        &self,
        rig: &Rig,
        side: EventSide,
        layer: usize,
        trigger: usize,
        c0: f32,
        c1: f32,
        buf: &mut EventBuf,
    ) {
        rig.trigger_events(layer, trigger, c0, c1, &mut |ev| {
            if let Some(e) = self.resolve(side, ev.layer, &ev) {
                buf.push(e);
            }
        });
    }

    /// Events of one animation for the clock window `[c0, c1)`, for a client that plays state animations from
    /// [`AnimClock`]s (`source` and `layer` are only copied into the events).
    #[allow(clippy::too_many_arguments)]
    pub fn animation_window(
        &self,
        rig: &Rig,
        side: EventSide,
        layer: usize,
        source: RigSource,
        animation: usize,
        c0: f32,
        c1: f32,
        buf: &mut EventBuf,
    ) {
        let Some(anim) = rig.animation(animation) else { return };
        anim.emit_events_indexed(c0, c1, |event_index, event, loop_index| {
            let ev = RigEvent { layer, animation, event, event_index, loop_index, source };
            if let Some(e) = self.resolve(side, layer, &ev) {
                buf.push(e);
            }
        });
    }
}

// ---------------------------------------------------------------------------------------------------------------
// High-water marks (server and any window-driven emission)
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
struct Mark {
    has: bool,
    loop_index: u32,
    time: f32,
    p_has: bool,
    p_loop: u32,
    p_time: f32,
}

fn after(l: u32, t: f32, ml: u32, mt: f32) -> bool {
    l > ml || (l == ml && t > mt)
}

#[derive(Clone, Debug)]
struct LayerMarks {
    /// Offset of each state's first animation slot in `marks`.
    state_base: Vec<usize>,
    trig_base: usize,
    end: usize,
}

/// Per-mob high-water marks for window-driven emission ([`MobEvents::update_marked`], `trigger_window_marked`,
/// `animation_window_marked`): for every (layer, state animation slot or trigger) the furthest `(loop_index, keyframe
/// time)` that has been emitted. An event fires only when it is **past** the mark, so a window that is repeated or
/// goes back over ground already covered (a rewound trigger clock, a replayed window) never emits an event twice; the
/// most important user is the server's damage-frame `hit`. A new loop is past the mark and fires again.
///
/// Events of one call that share a keyframe time (a `hit` with its sound and particle) are all compared with the mark
/// as it was before the call, so they fire together. The mark is committed at the end of the call.
///
/// Resets: [`EventMarks::reset_layer`] (done automatically by `update_marked` when a layer entered a new state),
/// [`EventMarks::reset_trigger`] (call it when a trigger animation is **started again**), [`EventMarks::reset_all`]
/// (a seek, teleport or reload). The allocation happens in [`EventMarks::new`] only.
#[derive(Clone, Debug)]
pub struct EventMarks {
    layers: Vec<LayerMarks>,
    marks: Vec<Mark>,
}

impl EventMarks {
    /// Marks for every state animation slot and trigger of the rig's controllers.
    pub fn new(rig: &Rig) -> Self {
        let mut layers = Vec::new();
        let mut n = 0usize;
        for c in rig.controllers() {
            let mut state_base = Vec::with_capacity(c.states.len());
            for s in &c.states {
                state_base.push(n);
                n += s.animations.len();
            }
            let trig_base = n;
            n += c.triggers.len();
            layers.push(LayerMarks { state_base, trig_base, end: n });
        }
        EventMarks { layers, marks: vec![Mark::default(); n] }
    }

    fn index(&self, layer: usize, source: RigSource) -> Option<usize> {
        let l = self.layers.get(layer)?;
        match source {
            RigSource::State { state, slot } => {
                let base = *l.state_base.get(state as usize)?;
                let next = l.state_base.get(state as usize + 1).copied().unwrap_or(l.trig_base);
                let i = base + slot as usize;
                (i < next).then_some(i)
            }
            RigSource::Trigger { trigger } => {
                let i = l.trig_base + trigger as usize;
                (i < l.end).then_some(i)
            }
        }
    }

    /// Whether an event at `(loop_index, time)` is past the mark; records it as pending when it is.
    fn allow(&mut self, layer: usize, source: RigSource, loop_index: u32, time: f32) -> bool {
        let Some(i) = self.index(layer, source) else { return true };
        let m = &mut self.marks[i];
        let fire = !m.has || after(loop_index, time, m.loop_index, m.time);
        if fire && (!m.p_has || after(loop_index, time, m.p_loop, m.p_time)) {
            m.p_has = true;
            m.p_loop = loop_index;
            m.p_time = time;
        }
        fire
    }

    fn commit(&mut self) {
        for m in &mut self.marks {
            if m.p_has {
                m.has = true;
                m.loop_index = m.p_loop;
                m.time = m.p_time;
                m.p_has = false;
            }
        }
    }

    /// Forget the marks of one layer (a new state was entered, or a seek).
    pub fn reset_layer(&mut self, layer: usize) {
        if let Some(l) = self.layers.get(layer) {
            let (a, b) = (l.state_base.first().copied().unwrap_or(l.trig_base), l.end);
            self.marks[a..b].fill(Mark::default());
        }
    }

    /// Forget the mark of one trigger: it is being played again from its start.
    pub fn reset_trigger(&mut self, layer: usize, trigger: usize) {
        if let Some(i) = self.index(layer, RigSource::Trigger { trigger: trigger as u16 }) {
            self.marks[i] = Mark::default();
        }
    }

    /// Forget everything.
    pub fn reset_all(&mut self) {
        self.marks.fill(Mark::default());
    }
}

impl MobEvents {
    /// [`MobEvents::update`] with high-water marks: an event fires only when it is past the mark of its (layer, slot).
    /// A layer that entered a new state during the call has its marks reset (its clocks restarted).
    pub fn update_marked(
        &self,
        rig: &Rig,
        side: EventSide,
        st: &mut RigState,
        dt: f32,
        inputs: &mut ExprInputs,
        marks: &mut EventMarks,
        buf: &mut EventBuf,
    ) {
        rig.update(st, dt, inputs, &mut |ev| {
            if marks.allow(ev.layer, ev.source, ev.loop_index, ev.event.time) {
                if let Some(e) = self.resolve(side, ev.layer, &ev) {
                    buf.push(e);
                }
            }
        });
        marks.commit();
        for (li, l) in st.layers.iter().enumerate() {
            if l.cur.time == 0.0 {
                marks.reset_layer(li);
            }
        }
    }

    /// [`MobEvents::trigger_window`] with high-water marks (call [`EventMarks::reset_trigger`] when the trigger starts
    /// again).
    #[allow(clippy::too_many_arguments)]
    pub fn trigger_window_marked(
        &self,
        rig: &Rig,
        side: EventSide,
        layer: usize,
        trigger: usize,
        c0: f32,
        c1: f32,
        marks: &mut EventMarks,
        buf: &mut EventBuf,
    ) {
        rig.trigger_events(layer, trigger, c0, c1, &mut |ev| {
            if marks.allow(ev.layer, ev.source, ev.loop_index, ev.event.time) {
                if let Some(e) = self.resolve(side, ev.layer, &ev) {
                    buf.push(e);
                }
            }
        });
        marks.commit();
    }

    /// [`MobEvents::animation_window`] with high-water marks.
    #[allow(clippy::too_many_arguments)]
    pub fn animation_window_marked(
        &self,
        rig: &Rig,
        side: EventSide,
        layer: usize,
        source: RigSource,
        animation: usize,
        c0: f32,
        c1: f32,
        marks: &mut EventMarks,
        buf: &mut EventBuf,
    ) {
        let Some(anim) = rig.animation(animation) else { return };
        anim.emit_events_indexed(c0, c1, |event_index, event, loop_index| {
            let ev = RigEvent { layer, animation, event, event_index, loop_index, source };
            if marks.allow(layer, source, loop_index, event.time) {
                if let Some(e) = self.resolve(side, layer, &ev) {
                    buf.push(e);
                }
            }
        });
        marks.commit();
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Locator positions
// ---------------------------------------------------------------------------------------------------------------

/// Model-space position in pixels of `locator` plus `extra` (pixels, in the locator's frame: it rotates and scales with
/// the bone) for the bone matrices of a pose ([`crate::skeleton::bone_matrices_into`]). No locator: `extra` in model
/// space. A locator index or bone outside the arrays gives `extra` as well.
pub fn locator_position(locators: &[Locator], matrices: &[Mat4], locator: Option<LocatorIndex>, extra: Vec3) -> Vec3 {
    let Some(l) = locator.and_then(|i| locators.get(i.0 as usize)) else { return extra };
    match matrices.get(l.bone.index()) {
        Some(m) => m.transform_point3(l.offset + extra),
        None => extra,
    }
}

/// Where the engine puts `ev`'s sound or particle: its locator (plus the particle offset) in model space, pixels.
/// Custom events use the locator without offset.
pub fn event_position(locators: &[Locator], matrices: &[Mat4], ev: &MobAnimEvent) -> Vec3 {
    match ev.kind {
        MobEventKind::Sound { locator, .. } | MobEventKind::Custom { locator, .. } => {
            locator_position(locators, matrices, locator, Vec3::ZERO)
        }
        MobEventKind::Particle { locator, offset, .. } => locator_position(locators, matrices, locator, offset),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Client cursor over AnimClock
// ---------------------------------------------------------------------------------------------------------------

/// A client's cursor over one [`AnimClock`] (one animation clock, for example a trigger or a state animation): turns
/// the clock's time at each rendered tick into the half-open window to feed to [`MobEvents::animation_window`] or
/// [`MobEvents::trigger_window`].
///
/// Rules (all tested):
/// * The window of a call is `[last, now)` in clock seconds, so consecutive calls never repeat or skip an event, and a
///   paused clock (speed 0, or the same tick again) gives an empty window.
/// * [`EventCursor::joined`] (a client that starts watching a clock midway: late join, a mob coming into view) starts at
///   the current time and the first window is **exclusive** of that time: only events strictly after it fire; nothing in
///   the past is replayed. [`EventCursor::fresh`] (a clock the client saw start) includes events at the anchor time.
/// * A new anchor that is **continuous** (same state, and the old clock at the new anchor's tick gives the new anchor
///   time within 1 microsecond: what [`AnimClock::reanchor`] makes) changes nothing.
/// * A new anchor that is not continuous is a **seek**: no event fires for the jump, the cursor moves to the new time
///   and only events strictly after it fire later. This covers re-anchoring backwards (nothing replays), forwards
///   (nothing is skipped in a burst) and a state change. Exception: an anchor at time 0 whose tick is at or after the
///   last observed tick is a **restart** (the server started the animation since we last looked): events from time 0 up
///   to now fire, once.
/// * A step of more than [`MAX_CATCH_UP_SECONDS`] of clock time in one call (a stall, a tick jump) is a seek too.
/// * **High-water mark**: the cursor remembers the furthest clock time it has given events up to. Time moving
///   backwards (negative speed, a rewind) fires nothing and moves the cursor back, and moving forward again fires
///   nothing until the clock passes the mark: no event is emitted twice. A new loop (clock time past the mark) fires
///   again. A seek, a restart and a state change reset the mark.
/// * Windows are f32 like the animation's event times; a boundary closer than f32 resolution (about 1e-7 s at 1 s)
///   counts as equal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventCursor {
    last: f64,
    /// Furthest time events were given out to (the high-water mark).
    high: f64,
    exclusive: bool,
    prev: AnimClock,
    last_tick: i64,
}

/// A clock window `[c0, c1)` in seconds for the emit functions (`c1 <= c0` is empty).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EventWindow {
    pub c0: f32,
    pub c1: f32,
}

impl EventWindow {
    pub const EMPTY: EventWindow = EventWindow { c0: 0.0, c1: 0.0 };

    pub fn is_empty(&self) -> bool {
        self.c1.is_nan() || self.c0.is_nan() || self.c1 <= self.c0
    }
}

/// The most clock time (seconds, speed already applied) one [`EventCursor::advance`] call may cover. A larger step (a
/// frame stall, a tick jump, a long pause of the client) is treated as a **seek**: nothing fires for it and the
/// high-water mark moves to the new time, so a stall never bursts a pile of sounds and particles.
pub const MAX_CATCH_UP_SECONDS: f64 = 1.0;

fn f32_next_up(x: f32) -> f32 {
    if !x.is_finite() || x < 0.0 {
        return x;
    }
    f32::from_bits(x.to_bits() + 1)
}

impl EventCursor {
    /// Start watching `clock` at `tick`/`partial`: events at or before this moment never fire.
    pub fn joined(clock: &AnimClock, tick: i64, partial: f32) -> Self {
        let t = clock.time_at(tick, partial);
        EventCursor { last: t, high: t, exclusive: true, prev: *clock, last_tick: tick }
    }

    /// Start at the clock's anchor: events at the anchor time (0 for a new animation) do fire.
    pub fn fresh(clock: &AnimClock) -> Self {
        EventCursor { last: clock.anchor_time, high: clock.anchor_time, exclusive: false, prev: *clock, last_tick: clock.anchor_tick }
    }

    /// The time (seconds) up to which events have been given out.
    pub fn time(&self) -> f64 {
        self.last
    }

    /// The window for `clock` at `tick` plus `partial` of a tick. Call it once per rendered frame (or tick) with the
    /// current clock; pass the same clock again when nothing changed.
    pub fn advance(&mut self, clock: &AnimClock, tick: i64, partial: f32) -> EventWindow {
        let now = clock.time_at(tick, partial);
        if self.prev != *clock {
            let continuous = self.prev.state == clock.state
                && (self.prev.time_at(clock.anchor_tick, 0.0) - clock.anchor_time).abs() <= 1.0e-6;
            if !continuous {
                if clock.anchor_time == 0.0 && clock.anchor_tick >= self.last_tick && clock.anchor_tick <= tick {
                    self.last = 0.0;
                    self.high = 0.0;
                    self.exclusive = false;
                } else {
                    self.last = now;
                    self.high = now;
                    self.exclusive = true;
                }
            }
            self.prev = *clock;
        }
        self.last_tick = tick;
        if now > self.last {
            if now - self.last > MAX_CATCH_UP_SECONDS {
                self.last = now;
                self.high = now;
                self.exclusive = true;
                return EventWindow::EMPTY;
            }
            // Behind the mark (after a rewind) only the part past the mark is new; the mark is an exclusive end of
            // the previous window, so events exactly at it have not fired yet.
            let (start, exclusive) = if self.high > self.last { (self.high, false) } else { (self.last, self.exclusive) };
            self.last = now;
            self.exclusive = false;
            if now <= start {
                return EventWindow::EMPTY;
            }
            self.high = now;
            let c0 = start as f32;
            let c0 = if exclusive { f32_next_up(c0) } else { c0 };
            EventWindow { c0, c1: now as f32 }
        } else {
            if now < self.last {
                self.last = now;
                self.exclusive = true;
            }
            EventWindow::EMPTY
        }
    }
}
