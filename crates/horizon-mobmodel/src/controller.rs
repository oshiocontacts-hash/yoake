//! Animation controllers (FR-F4-MOBS.md section 6.1): states, animations per state, transitions on expressions,
//! blend times and per-state speed. Each controller is one **layer** with a weight and a bone mask; the pose
//! evaluator ([`crate::pose`]) stacks the layers.
//!
//! File format (our own; everything but `states` is optional):
//!
//! ```json
//! { "controllers": {
//!   "locomotion": {
//!     "initial": "idle",
//!     "weight": 1, "bones_mask": [], "include_children": true, "mode": "override",
//!     "states": {
//!       "idle": { "animations": ["idle"], "blend": 0.15,
//!                 "transitions": [ { "to": "walk", "when": "q.ground_speed > 0.05", "blend": 0.1 } ] },
//!       "walk": { "animations": [ { "name": "walk", "speed": "q.ground_speed * 4", "weight": 1 } ],
//!                 "speed": 1, "on_enter": { "v.steps": "0" },
//!                 "transitions": [ { "to": "idle", "when": "q.ground_speed < 0.03" } ] } } },
//!   "action": { "weight": 1, "bones_mask": ["arm_l"], "triggers": { "pounce": "pounce" } } } }
//! ```
//!
//! Conventions (decided here):
//! * Layers run in file order (the first is the bottom layer). `mode` is `override` (default: the layer replaces
//!   what is below on the channels it animates, scaled by its weight) or `add` (its values are added, scaled).
//! * `bones_mask` lists bone names the layer may drive (empty: all bones). With `include_children` (default true) the
//!   descendants of listed bones are included. The mask is binary; the layer weight does the fading.
//! * `blend` (alias `blend_transition`) on a state is the cross-fade in seconds used when entering that state; a
//!   transition's own `blend` overrides it. Default 0 (a cut). The state being left keeps playing while it fades out.
//!   A transition during a fade drops the older state at once.
//! * Transitions are tested in order every update, after the clocks advanced; the first non-zero `when` fires, and at
//!   most one transition fires per update. In `when`, `query.anim_time` is the seconds spent in the state.
//! * `speed` (state) and `speed` (per animation) are expressions, clamped to `[0, 100]`; each animation has its own
//!   clock that advances by `dt * state_speed * animation_speed`, so a changing speed never makes time jump.
//! * `on_enter` assigns `variable.*` slots when the state is entered (evaluated in order).
//! * A layer without states is allowed: it has one implicit empty state and exists for its `triggers`
//!   (one-shot animations: trigger name to animation name; the trigger id is its position in the file).
//! * A state with `"inactive": true` is "no state": it plays nothing and the layer is off while it is current. Blending
//!   into it fades the layer's weight from 1 to 0 over the blend time (the layer then stays off); blending out of it
//!   fades the weight in. With blend 0 the switch is instant.
//! * At most [`MAX_STATE_ANIMS`] animations per state, [`MAX_STATES`] states and [`MAX_LAYERS`] layers.

use crate::clock::AnimClock;
use crate::anim::{load_err, Ctx, LoadError, OrderedMap, Scalar, Warning};
use crate::expr::{Expr, ExprInputs, Query, Symbols};
use serde::Deserialize;
use serde_json::Value;

pub const MAX_STATE_ANIMS: usize = 8;
/// State ids are one byte on the wire.
pub const MAX_STATES: usize = 255;
pub const MAX_LAYERS: usize = 16;
/// `update` clamps `dt` to this many seconds.
pub const MAX_DT: f32 = 10.0;

#[derive(Clone, Debug, PartialEq)]
pub struct StateAnim {
    pub name: String,
    /// Index into the animation library; set by `Rig::new` (`usize::MAX` until then).
    pub index: usize,
    pub speed: Option<Scalar>,
    pub weight: Option<Scalar>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Transition {
    pub to: usize,
    pub when: Expr,
    pub blend: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct State {
    pub name: String,
    pub animations: Vec<StateAnim>,
    pub speed: Option<Scalar>,
    pub transitions: Vec<Transition>,
    /// Fade-in seconds when this state is entered.
    pub blend: f32,
    /// `(variable slot, value)` assignments on entry.
    pub on_enter: Vec<(usize, Expr)>,
    /// "No state": the layer contributes nothing while this state is current. A blend into it fades the layer's weight
    /// to 0 (and a blend out of it fades the weight in) instead of lerping towards the rest pose. Blend 0 is instant.
    pub inactive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LayerMode {
    #[default]
    Override,
    Add,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TriggerDef {
    pub name: String,
    pub animation: String,
    /// Index into the animation library; set by `Rig::new`.
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Controller {
    pub name: String,
    pub initial: usize,
    pub states: Vec<State>,
    pub weight: Scalar,
    pub mode: LayerMode,
    pub bones_mask: Vec<String>,
    pub include_children: bool,
    pub triggers: Vec<TriggerDef>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedControllers {
    pub controllers: Vec<Controller>,
    pub warnings: Vec<Warning>,
}

#[derive(Deserialize)]
struct CtrlFile {
    #[serde(default)]
    controllers: Option<OrderedMap<RawLayer>>,
}

#[derive(Deserialize, Default)]
struct RawLayer {
    initial: Option<String>,
    states: Option<OrderedMap<Value>>,
    weight: Option<Value>,
    bones_mask: Option<Vec<String>>,
    include_children: Option<bool>,
    mode: Option<String>,
    triggers: Option<OrderedMap<Value>>,
    #[serde(flatten)]
    extra: serde_json::Map<String, Value>,
}

/// Parse a controller file. Layers come out in file order; pass several files' results to `Rig::new` in the order
/// the layers should stack.
pub fn parse_controllers(json: &str, syms: &mut Symbols) -> Result<ParsedControllers, LoadError> {
    let file: CtrlFile = serde_json::from_str(json).map_err(|e| load_err("", format!("invalid JSON: {e}")))?;
    let Some(OrderedMap(layers)) = file.controllers else {
        return Err(load_err("", "no \"controllers\" object"));
    };
    if layers.len() > MAX_LAYERS {
        return Err(load_err("", format!("more than {MAX_LAYERS} controllers")));
    }
    let mut out = ParsedControllers::default();
    for (name, raw) in layers {
        let mut ctx = Ctx { syms, warnings: &mut out.warnings, context: name.clone() };
        let c = parse_layer(&name, raw, &mut ctx)?;
        out.controllers.push(c);
    }
    Ok(out)
}

fn blend_seconds(v: &Value, what: &str, ctx: &Ctx) -> Result<f32, LoadError> {
    match v.as_f64() {
        Some(b) if b.is_finite() && b >= 0.0 => Ok(b as f32),
        _ => Err(ctx.err(format!("{what} must be a number of seconds >= 0"))),
    }
}

fn parse_layer(name: &str, raw: RawLayer, ctx: &mut Ctx) -> Result<Controller, LoadError> {
    for k in raw.extra.keys() {
        ctx.warn(format!("unsupported field '{k}' (ignored)"));
    }
    let mode = match raw.mode.as_deref() {
        None | Some("override") => LayerMode::Override,
        Some("add") => LayerMode::Add,
        Some(other) => return Err(ctx.err(format!("mode must be \"override\" or \"add\", got '{other}'"))),
    };
    let weight = match &raw.weight {
        Some(v) => ctx.scalar(v, "weight")?,
        None => Scalar::Const(1.0),
    };
    // Pass 1: state names, so transitions can point forward.
    let state_list: Vec<(String, Value)> = raw.states.map_or_else(Vec::new, |m| m.0);
    if state_list.len() > MAX_STATES {
        return Err(ctx.err(format!("more than {MAX_STATES} states")));
    }
    let names: Vec<&str> = state_list.iter().map(|(n, _)| n.as_str()).collect();
    let mut states = Vec::new();
    for (sname, sv) in &state_list {
        states.push(parse_state(sname, sv, &names, ctx)?);
    }
    if states.is_empty() {
        states.push(State {
            name: "default".into(),
            animations: Vec::new(),
            speed: None,
            transitions: Vec::new(),
            blend: 0.0,
            on_enter: Vec::new(),
            inactive: false,
        });
    }
    let initial = match &raw.initial {
        None => 0,
        Some(n) => match states.iter().position(|s| s.name == *n) {
            Some(i) => i,
            None => return Err(ctx.err(format!("initial state '{n}' does not exist"))),
        },
    };
    let mut triggers = Vec::new();
    if let Some(OrderedMap(list)) = raw.triggers {
        for (tname, tv) in list {
            let Some(anim) = tv.as_str() else {
                return Err(ctx.err(format!("trigger '{tname}' must name an animation")));
            };
            triggers.push(TriggerDef { name: tname, animation: anim.to_string(), index: usize::MAX });
        }
    }
    Ok(Controller {
        name: name.to_string(),
        initial,
        states,
        weight,
        mode,
        bones_mask: raw.bones_mask.unwrap_or_default(),
        include_children: raw.include_children.unwrap_or(true),
        triggers,
    })
}

fn parse_state(name: &str, v: &Value, names: &[&str], ctx: &mut Ctx) -> Result<State, LoadError> {
    let Some(obj) = v.as_object() else { return Err(ctx.err(format!("state '{name}' must be an object"))) };
    let mut st = State {
        name: name.to_string(),
        animations: Vec::new(),
        speed: None,
        transitions: Vec::new(),
        blend: 0.0,
        on_enter: Vec::new(),
        inactive: false,
    };
    for (key, val) in obj {
        let what = format!("state '{name}' {key}");
        match key.as_str() {
            "animations" => {
                let Some(list) = val.as_array() else { return Err(ctx.err(format!("{what} must be an array"))) };
                if list.len() > MAX_STATE_ANIMS {
                    return Err(ctx.err(format!("state '{name}' has more than {MAX_STATE_ANIMS} animations")));
                }
                for item in list {
                    st.animations.push(parse_state_anim(item, &what, ctx)?);
                }
            }
            "transitions" => {
                let Some(list) = val.as_array() else { return Err(ctx.err(format!("{what} must be an array"))) };
                for item in list {
                    st.transitions.push(parse_transition(item, name, names, ctx)?);
                }
            }
            "inactive" => match val.as_bool() {
                Some(b) => st.inactive = b,
                None => return Err(ctx.err(format!("{what} must be true or false"))),
            },
            "blend" | "blend_transition" => st.blend = blend_seconds(val, &what, ctx)?,
            "speed" => st.speed = Some(ctx.scalar(val, &what)?),
            "on_enter" => {
                let Some(map) = val.as_object() else { return Err(ctx.err(format!("{what} must be an object"))) };
                for (var, ev) in map {
                    let bare = var
                        .strip_prefix("variable.")
                        .or_else(|| var.strip_prefix("v."))
                        .unwrap_or(var.as_str());
                    let src = match ev {
                        Value::String(s) => s.clone(),
                        Value::Number(n) => n.to_string(),
                        Value::Bool(b) => (if *b { "1" } else { "0" }).to_string(),
                        _ => return Err(ctx.err(format!("{what}: '{var}' needs a number or an expression"))),
                    };
                    // Interning through a throwaway expression keeps the slot rules in one place.
                    let slot_expr = ctx.compile(&format!("v.{bare}"), &what)?;
                    let slot = match slot_expr.ops().first() {
                        Some(crate::expr::Op::Var(i)) if slot_expr.len() == 1 => *i as usize,
                        _ => return Err(ctx.err(format!("{what}: '{var}' is not a variable name"))),
                    };
                    let e = ctx.compile(&src, &format!("{what}.{var}"))?;
                    st.on_enter.push((slot, e));
                }
            }
            other => ctx.warn(format!("state '{name}': unsupported field '{other}' (ignored)")),
        }
    }
    Ok(st)
}

fn parse_state_anim(v: &Value, what: &str, ctx: &mut Ctx) -> Result<StateAnim, LoadError> {
    match v {
        Value::String(s) => Ok(StateAnim { name: s.clone(), index: usize::MAX, speed: None, weight: None }),
        Value::Object(o) => {
            let Some(name) = o.get("name").and_then(Value::as_str) else {
                return Err(ctx.err(format!("{what}: an animation entry needs a \"name\"")));
            };
            let mut a = StateAnim { name: name.to_string(), index: usize::MAX, speed: None, weight: None };
            for (k, val) in o {
                match k.as_str() {
                    "name" => {}
                    "speed" => a.speed = Some(ctx.scalar(val, &format!("{what} '{name}' speed"))?),
                    "weight" => a.weight = Some(ctx.scalar(val, &format!("{what} '{name}' weight"))?),
                    other => ctx.warn(format!("{what} '{name}': unsupported field '{other}' (ignored)")),
                }
            }
            Ok(a)
        }
        _ => Err(ctx.err(format!("{what}: an animation entry is a name or an object"))),
    }
}

fn parse_transition(v: &Value, from: &str, names: &[&str], ctx: &mut Ctx) -> Result<Transition, LoadError> {
    let what = format!("state '{from}' transition");
    let Some(o) = v.as_object() else { return Err(ctx.err(format!("{what} must be an object"))) };
    let Some(to_name) = o.get("to").and_then(Value::as_str) else {
        return Err(ctx.err(format!("{what} needs a \"to\" state")));
    };
    let Some(to) = names.iter().position(|n| *n == to_name) else {
        return Err(ctx.err(format!("{what} goes to unknown state '{to_name}'")));
    };
    let Some(when) = o.get("when") else { return Err(ctx.err(format!("{what} to '{to_name}' needs a \"when\""))) };
    let when = match ctx.scalar(when, &format!("{what} to '{to_name}' when"))? {
        Scalar::Const(c) => Expr::constant(c),
        Scalar::Expr(e) => e,
    };
    let mut blend = None;
    for (k, val) in o {
        match k.as_str() {
            "to" | "when" => {}
            "blend" | "blend_transition" => blend = Some(blend_seconds(val, &format!("{what} blend"), ctx)?),
            other => ctx.warn(format!("{what}: unsupported field '{other}' (ignored)")),
        }
    }
    Ok(Transition { to, when, blend })
}

// ---------------------------------------------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------------------------------------------

/// One state being played: its clocks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StateRun {
    pub state: usize,
    /// Seconds since the state was entered.
    pub time: f32,
    /// Per-animation clocks (speed applied), by position in the state's animation list.
    pub clocks: [f32; MAX_STATE_ANIMS],
}

impl StateRun {
    fn new(state: usize) -> Self {
        Self { state, time: 0.0, clocks: [0.0; MAX_STATE_ANIMS] }
    }
}

/// Runtime state of one controller (one per mob and layer).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ControllerState {
    pub cur: StateRun,
    /// The state being faded out, if a blend is running.
    pub prev: Option<StateRun>,
    pub blend_elapsed: f32,
    pub blend_duration: f32,
}

fn clamp_speed(v: f32) -> f32 {
    v.clamp(0.0, 100.0)
}

impl Controller {
    pub fn initial_state(&self) -> ControllerState {
        ControllerState { cur: StateRun::new(self.initial), prev: None, blend_elapsed: 0.0, blend_duration: 0.0 }
    }

    /// The synced state id (one byte).
    pub fn state_id(&self, st: &ControllerState) -> u8 {
        st.cur.state.min(MAX_STATES) as u8
    }

    /// Fade weight of the current state against the previous one: 1 when no fade runs.
    pub fn blend_alpha(&self, st: &ControllerState) -> f32 {
        match st.prev {
            Some(_) if st.blend_duration > 0.0 => (st.blend_elapsed / st.blend_duration).clamp(0.0, 1.0),
            _ => 1.0,
        }
    }

    /// Enter `to` (a client applying a synced state id uses this; `blend` chooses whether to fade).
    /// Does nothing if `to` is the current state or does not exist.
    pub fn set_state(&self, st: &mut ControllerState, to: usize, blend: bool, inputs: &mut ExprInputs) {
        if to == st.cur.state || to >= self.states.len() {
            return;
        }
        let dur = if blend { self.states[to].blend } else { 0.0 };
        self.enter(st, to, dur, inputs);
    }

    fn enter(&self, st: &mut ControllerState, to: usize, blend: f32, inputs: &mut ExprInputs) {
        if blend > 0.0 {
            st.prev = Some(st.cur);
            st.blend_elapsed = 0.0;
            st.blend_duration = blend;
        } else {
            st.prev = None;
        }
        st.cur = StateRun::new(to);
        inputs.set(Query::AnimTime, 0.0);
        for (slot, e) in &self.states[to].on_enter {
            let v = e.eval(inputs);
            if let Some(var) = inputs.variables.get_mut(*slot) {
                *var = v;
            }
        }
    }

    fn advance_run(
        &self,
        run: &mut StateRun,
        dt: f32,
        inputs: &mut ExprInputs,
        events: &mut dyn FnMut(usize, usize, f32, f32),
    ) {
        let Some(state) = self.states.get(run.state) else { return };
        for (i, _) in state.animations.iter().enumerate().take(MAX_STATE_ANIMS) {
            let sp = self.slot_speed(run, i, inputs);
            let old = run.clocks[i];
            let new = old + dt * sp;
            run.clocks[i] = new;
            events(run.state, i, old, new);
        }
        run.time += dt;
    }

    /// The effective clock speed of animation `slot` of a running state: state speed times animation speed, each an
    /// expression clamped to `[0, 100]`. `query.anim_time` is overwritten.
    pub fn slot_speed(&self, run: &StateRun, slot: usize, inputs: &mut ExprInputs) -> f32 {
        let Some(state) = self.states.get(run.state) else { return 0.0 };
        let Some(a) = state.animations.get(slot).filter(|_| slot < MAX_STATE_ANIMS) else { return 0.0 };
        inputs.set(Query::AnimTime, run.time);
        let state_speed = state.speed.as_ref().map_or(1.0, |s| clamp_speed(s.eval(inputs)));
        inputs.set(Query::AnimTime, run.clocks[slot]);
        a.speed.as_ref().map_or(1.0, |s| clamp_speed(s.eval(inputs))) * state_speed
    }

    /// The server's anchor for animation `slot` of the current state at game tick `tick`: the integrated clock and its
    /// current speed. Send it whenever [`Controller::anchor_stale`] says so (and when the state changes).
    pub fn anchor(&self, st: &ControllerState, slot: usize, tick: i64, inputs: &mut ExprInputs) -> AnimClock {
        let time = st.cur.clocks.get(slot).copied().unwrap_or(0.0) as f64;
        AnimClock::new(self.state_id(st), tick, time, self.slot_speed(&st.cur, slot, inputs))
    }

    /// Whether the state or speed of `slot` moved away from the last sent anchor.
    pub fn anchor_stale(&self, st: &ControllerState, slot: usize, last: &AnimClock, inputs: &mut ExprInputs) -> bool {
        last.is_stale(self.state_id(st), self.slot_speed(&st.cur, slot, inputs))
    }

    /// Client side: apply a received anchor. A different state id enters that state (with its fade), then the clock of
    /// `slot` is set to the anchor's time at `tick + partial`. At the anchor tick the value equals the server's clock
    /// exactly (f32 to f64 and back is lossless).
    pub fn apply_anchor(
        &self,
        st: &mut ControllerState,
        slot: usize,
        clock: &AnimClock,
        tick: i64,
        partial: f32,
        inputs: &mut ExprInputs,
    ) {
        if clock.state as usize != st.cur.state {
            self.set_state(st, clock.state as usize, true, inputs);
        }
        if let Some(c) = st.cur.clocks.get_mut(slot) {
            *c = clock.time_at(tick, partial) as f32;
        }
    }

    /// Advance the clocks by `dt` seconds (0 while paused), then test the transitions. `events(state, slot,
    /// clock_old, clock_new)` is called for every playing animation (also the fading one) so the caller can emit
    /// its timeline events for the clock window. Returns whether a transition fired.
    ///
    /// `inputs` supplies the expression values and receives `on_enter` variable writes; its `query.anim_time` is
    /// overwritten.
    pub fn update(
        &self,
        st: &mut ControllerState,
        dt: f32,
        inputs: &mut ExprInputs,
        events: &mut dyn FnMut(usize, usize, f32, f32),
    ) -> bool {
        let dt = if dt.is_finite() { dt.clamp(0.0, MAX_DT) } else { 0.0 };
        let mut cur = st.cur;
        self.advance_run(&mut cur, dt, inputs, events);
        st.cur = cur;
        if let Some(mut prev) = st.prev {
            self.advance_run(&mut prev, dt, inputs, events);
            st.blend_elapsed += dt;
            st.prev = if st.blend_elapsed >= st.blend_duration { None } else { Some(prev) };
        }
        let Some(state) = self.states.get(st.cur.state) else { return false };
        inputs.set(Query::AnimTime, st.cur.time);
        for t in &state.transitions {
            if t.when.eval_bool(inputs) {
                let dur = t.blend.unwrap_or(self.states[t.to].blend);
                self.enter(st, t.to, dur, inputs);
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = r#"{ "controllers": {
      "locomotion": { "initial": "idle", "states": {
        "walk": { "animations": [ { "name": "walk", "speed": "q.ground_speed * 4" } ],
                  "transitions": [ { "to": "idle", "when": "q.ground_speed < 0.03" } ] },
        "idle": { "animations": ["idle"], "blend": 0.2,
                  "transitions": [ { "to": "walk", "when": "q.ground_speed > 0.05", "blend": 0.1 },
                                   { "to": "jump", "when": "!q.is_on_ground" } ] },
        "jump": { "animations": ["jump", "wobble"], "speed": 2, "on_enter": { "v.jumps": "v.jumps + 1" },
                  "transitions": [ { "to": "idle", "when": "q.is_on_ground" } ] }
      } },
      "action": { "weight": 0.5, "bones_mask": ["arm"], "include_children": false, "mode": "add",
                  "triggers": { "pounce": "pounce", "bite": "bite" } }
    } }"#;

    fn load() -> (Vec<Controller>, Symbols) {
        let mut s = Symbols::new(&[]);
        (parse_controllers(FILE, &mut s).unwrap().controllers, s)
    }

    fn noop() -> impl FnMut(usize, usize, f32, f32) {
        |_, _, _, _| {}
    }

    #[test]
    fn parses_layers_states_and_triggers_in_file_order() {
        let (c, s) = load();
        assert_eq!(c.len(), 2);
        let loco = &c[0];
        let names: Vec<_> = loco.states.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["walk", "idle", "jump"]);
        assert_eq!(loco.initial, 1);
        assert_eq!(loco.states[1].blend, 0.2);
        assert_eq!(loco.states[1].transitions[0].blend, Some(0.1));
        assert_eq!(loco.states[1].transitions[0].to, 0);
        assert_eq!(loco.states[2].on_enter.len(), 1);
        assert_eq!(s.variables(), &["jumps".to_string()]);
        let act = &c[1];
        assert_eq!(act.mode, LayerMode::Add);
        assert_eq!(act.weight, Scalar::Const(0.5));
        assert_eq!(act.bones_mask, ["arm"]);
        assert!(!act.include_children);
        assert_eq!(act.states.len(), 1); // the implicit one
        let t: Vec<_> = act.triggers.iter().map(|t| (t.name.as_str(), t.animation.as_str())).collect();
        assert_eq!(t, [("pounce", "pounce"), ("bite", "bite")]);
    }

    #[test]
    fn transitions_fire_in_order_with_hysteresis() {
        let (c, _) = load();
        let c = &c[0];
        let mut st = c.initial_state();
        let mut inp = ExprInputs::default();
        inp.set(Query::IsOnGround, 1.0);
        assert!(!c.update(&mut st, 0.05, &mut inp, &mut noop()));
        assert_eq!(c.state_id(&st), 1);
        inp.set(Query::GroundSpeed, 0.04); // between the thresholds: stays idle
        assert!(!c.update(&mut st, 0.05, &mut inp, &mut noop()));
        inp.set(Query::GroundSpeed, 0.2);
        assert!(c.update(&mut st, 0.05, &mut inp, &mut noop()));
        assert_eq!(c.state_id(&st), 0);
        inp.set(Query::GroundSpeed, 0.04); // still walking: 0.04 is not < 0.03
        assert!(!c.update(&mut st, 0.05, &mut inp, &mut noop()));
        assert_eq!(c.state_id(&st), 0);
        inp.set(Query::GroundSpeed, 0.0);
        assert!(c.update(&mut st, 0.05, &mut inp, &mut noop()));
        assert_eq!(c.state_id(&st), 1);
    }

    #[test]
    fn first_matching_transition_wins_and_only_one_per_update() {
        let (c, _) = load();
        let c = &c[0];
        let mut st = c.initial_state();
        let mut inp = ExprInputs::default();
        inp.set(Query::GroundSpeed, 0.5); // walk (first listed) and jump both true
        inp.set(Query::IsOnGround, 0.0);
        assert!(c.update(&mut st, 0.0, &mut inp, &mut noop()));
        assert_eq!(c.state_id(&st), 0); // walk
        // walk -> idle needs speed < 0.03: not true, so it stays; no chain.
        assert!(!c.update(&mut st, 0.0, &mut inp, &mut noop()));
        assert_eq!(c.state_id(&st), 0);
    }

    #[test]
    fn blend_durations_and_alpha() {
        let (c, _) = load();
        let c = &c[0];
        let mut st = c.initial_state();
        let mut inp = ExprInputs::default();
        inp.set(Query::IsOnGround, 1.0);
        inp.set(Query::GroundSpeed, 0.5);
        assert!(c.update(&mut st, 0.0, &mut inp, &mut noop()));
        // idle -> walk used the transition's own blend 0.1, not walk's default 0.
        assert_eq!((st.prev.map(|p| p.state), st.blend_duration), (Some(1), 0.1));
        assert_eq!(c.blend_alpha(&st), 0.0);
        inp.set(Query::GroundSpeed, 0.1);
        c.update(&mut st, 0.05, &mut inp, &mut noop());
        assert!((c.blend_alpha(&st) - 0.5).abs() < 1e-6);
        c.update(&mut st, 0.06, &mut inp, &mut noop());
        assert!(st.prev.is_none());
        assert_eq!(c.blend_alpha(&st), 1.0);
        // walk -> idle uses idle's blend 0.2.
        inp.set(Query::GroundSpeed, 0.0);
        assert!(c.update(&mut st, 0.0, &mut inp, &mut noop()));
        assert_eq!(st.blend_duration, 0.2);
        // A transition during a fade drops the older state: the previous one is the one just left.
        inp.set(Query::GroundSpeed, 1.0);
        assert!(c.update(&mut st, 0.01, &mut inp, &mut noop()));
        assert_eq!(st.prev.map(|p| p.state), Some(1));
    }

    #[test]
    fn speed_integrates_the_clock() {
        let (c, _) = load();
        let c = &c[0];
        let mut st = c.initial_state();
        let mut inp = ExprInputs::default();
        c.set_state(&mut st, 0, false, &mut inp); // walk, clock speed = ground_speed * 4
        let mut seen = Vec::new();
        inp.set(Query::GroundSpeed, 0.5);
        c.update(&mut st, 1.0, &mut inp, &mut |s, i, a, b| seen.push((s, i, a, b)));
        assert_eq!(seen, [(0, 0, 0.0, 2.0)]);
        inp.set(Query::GroundSpeed, 0.25);
        c.update(&mut st, 1.0, &mut inp, &mut noop());
        assert_eq!(st.cur.clocks[0], 3.0); // 2 + 1, no jump when the speed changed
        assert_eq!(st.cur.time, 2.0);
        // A paused update (dt = 0, or a bad dt) changes nothing.
        let before = st;
        c.update(&mut st, 0.0, &mut inp, &mut noop());
        assert_eq!(st, before);
        c.update(&mut st, f32::NAN, &mut inp, &mut noop());
        assert_eq!(st, before);
    }

    #[test]
    fn state_speed_and_on_enter() {
        let (c, _) = load();
        let c = &c[0];
        let mut st = c.initial_state();
        let mut inp = ExprInputs::default();
        inp.set(Query::IsOnGround, 0.0);
        assert!(c.update(&mut st, 0.0, &mut inp, &mut noop())); // idle -> jump
        assert_eq!(c.state_id(&st), 2);
        assert_eq!(inp.variables[0], 1.0); // on_enter ran
        c.update(&mut st, 0.5, &mut inp, &mut noop());
        assert_eq!(st.cur.clocks[..2], [1.0, 1.0]); // state speed 2 on both animations
        inp.set(Query::IsOnGround, 1.0);
        assert!(c.update(&mut st, 0.0, &mut inp, &mut noop()));
        inp.set(Query::IsOnGround, 0.0);
        assert!(c.update(&mut st, 0.0, &mut inp, &mut noop()));
        assert_eq!(inp.variables[0], 2.0);
    }

    #[test]
    fn anchors_agree_with_the_integrated_clock() {
        let (c, _) = load();
        let c = &c[0];
        let mut server = c.initial_state();
        let mut client = c.initial_state();
        let mut inp = ExprInputs::default();
        c.set_state(&mut server, 0, false, &mut inp); // walk: speed = ground_speed * 4
        inp.set(Query::GroundSpeed, 0.5);
        let mut tick = 0i64;
        let mut last = c.anchor(&server, 0, tick, &mut inp);
        assert_eq!(last.speed, 2.0);
        c.apply_anchor(&mut client, 0, &last, tick, 0.0, &mut inp);
        for _ in 0..100 {
            tick += 1;
            c.update(&mut server, 0.05, &mut inp, &mut noop());
            // constant speed: the client's interpolated time tracks the server's integrated clock
            let t = last.time_at(tick, 0.0);
            assert!((t - server.cur.clocks[0] as f64).abs() < 1e-4, "tick {tick}");
        }
        // speed change at tick 100: the server re-anchors, time stays continuous
        inp.set(Query::GroundSpeed, 0.25);
        assert!(c.anchor_stale(&server, 0, &last, &mut inp));
        let before = last.time_at(tick, 0.0);
        let next = c.anchor(&server, 0, tick, &mut inp);
        assert_eq!(next.speed, 1.0);
        assert!((next.anchor_time - before).abs() < 1e-4);
        last = next;
        assert!(!c.anchor_stale(&server, 0, &last, &mut inp));
        // exact agreement at the anchor, bit for bit
        c.apply_anchor(&mut client, 0, &last, tick, 0.0, &mut inp);
        assert_eq!(client.cur.clocks[0], server.cur.clocks[0]);
        for _ in 0..40 {
            tick += 1;
            c.update(&mut server, 0.05, &mut inp, &mut noop());
        }
        let mut c2 = client;
        c.apply_anchor(&mut c2, 0, &last, tick, 0.0, &mut inp);
        assert!((c2.cur.clocks[0] - server.cur.clocks[0]).abs() < 1e-3);
        // a state id in the anchor makes the client enter that state
        let idle = AnimClock::new(1, tick, 0.0, 1.0);
        c.apply_anchor(&mut c2, 0, &idle, tick, 0.0, &mut inp);
        assert_eq!(c.state_id(&c2), 1);
        assert!(c.anchor_stale(&c2, 0, &last, &mut inp));
    }

    #[test]
    fn set_state_for_synced_ids() {
        let (c, _) = load();
        let c = &c[0];
        let mut st = c.initial_state();
        let mut inp = ExprInputs::default();
        c.set_state(&mut st, 2, true, &mut inp);
        assert_eq!(c.state_id(&st), 2);
        assert_eq!(st.blend_duration, 0.0); // jump has no blend of its own
        assert!(st.prev.is_none());
        c.set_state(&mut st, 1, true, &mut inp);
        assert_eq!(st.blend_duration, 0.2);
        assert!(st.prev.is_some());
        c.set_state(&mut st, 99, true, &mut inp); // unknown: ignored
        c.set_state(&mut st, 1, true, &mut inp); // same: ignored
        assert_eq!(c.state_id(&st), 1);
    }

    #[test]
    fn errors_and_warnings() {
        let bad = |s: &str| parse_controllers(s, &mut Symbols::new(&[]));
        assert!(bad("{}").is_err());
        assert!(bad(r#"{"controllers":{"a":{"initial":"nope","states":{"x":{}}}}}"#).is_err());
        assert!(bad(r#"{"controllers":{"a":{"states":{"x":{"transitions":[{"to":"y","when":"1"}]}}}}}"#).is_err());
        assert!(bad(r#"{"controllers":{"a":{"states":{"x":{"transitions":[{"to":"x"}]}}}}}"#).is_err());
        assert!(bad(r#"{"controllers":{"a":{"states":{"x":{"transitions":[{"to":"x","when":"1 +"}]}}}}}"#).is_err());
        assert!(bad(r#"{"controllers":{"a":{"mode":"mix"}}}"#).is_err());
        assert!(bad(r#"{"controllers":{"a":{"states":{"x":{"blend":-1}}}}}"#).is_err());
        let nine = ["\"a\""; 9].join(",");
        assert!(bad(&format!(r#"{{"controllers":{{"a":{{"states":{{"x":{{"animations":[{nine}]}}}}}}}}}}"#)).is_err());
        let many = (0..17).map(|i| format!("\"c{i}\":{{}}")).collect::<Vec<_>>().join(",");
        assert!(bad(&format!(r#"{{"controllers":{{{many}}}}}"#)).is_err());
        let p = bad(
            r#"{"controllers":{"a":{"colour":1,"states":{"x":{"transitions":[{"to":"x","when":"q.is_sneaking"}],"foo":1}}}}}"#,
        )
        .unwrap();
        let w: Vec<_> = p.warnings.iter().map(|w| w.to_string()).collect();
        assert!(w.iter().any(|w| w.contains("colour")), "{w:?}");
        assert!(w.iter().any(|w| w.contains("is_sneaking")));
        assert!(w.iter().any(|w| w.contains("foo")));
        // the unsupported "when" became constant 0: never fires
        let c = &p.controllers[0];
        let mut st = c.initial_state();
        assert!(!c.update(&mut st, 0.1, &mut ExprInputs::default(), &mut |_, _, _, _| {}));
    }
}
