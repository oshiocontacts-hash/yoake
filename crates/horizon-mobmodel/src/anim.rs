//! Animations: the `animation.json` parser, the bbmodel `animations[]` converter, keyframe sampling, easing and
//! timeline events (FR-F4-MOBS.md section 3.6).
//!
//! Conventions (decided here, see the F4M-2 report):
//! * `animation.json` is Bedrock: on load position `(x, y, z)` becomes `(-x, y, z)` and rotation `(rx, ry, rz)` becomes
//!   `(-rx, -ry, rz)`, the same mapping as the geo.json loader, so everything internal is in Blockbench space. bbmodel
//!   animations are used as is.
//! * Times are seconds. Rotations are degrees `[x, y, z]`, positions are model pixels, scale is a factor. Values are
//!   taken as written in the file (no axis flips); the pose evaluator turns rotations into quaternions with the same
//!   Euler order as `skeleton::bone_matrices` (X first, then Y, then Z).
//! * A keyframe's `lerp_mode` and `easing` control the segment that **starts** at that keyframe. Easing shapes the
//!   segment's progress for `linear` and `catmullrom`; it is ignored for `step` and `bezier`.
//! * `pre` is the value approached from the previous keyframe, `post` the value left towards the next one. At exactly
//!   a keyframe's time the sample is its `post`. Before the first keyframe the first `pre` holds; after the last the
//!   last `post` holds.
//! * Rotation is interpolated per Euler component (as Bedrock does), not as quaternions.
//! * `animation_length` missing: the largest keyframe or event time. A zero-length animation is a constant pose and
//!   behaves like `hold_on_last_frame`.
//! * `anim_time_update` is a stateless expression: it sees `query.anim_time` = the clock (seconds since the animation
//!   started, speed applied) and returns the time to sample, before looping. `blend_weight` multiplies the weight.
//!   Keyframe expressions see `query.anim_time` = the sampled (wrapped) time.

use crate::expr::{compile, Expr, ExprError, ExprErrorKind, ExprInputs, Query, Symbols};
use crate::limits::MAX_ANIMATIONS;
use glam::Vec3;
use serde::de::{Deserializer, MapAccess, Visitor};
use serde::Deserialize;
use serde_json::Value;
use std::fmt;

/// A non-fatal finding while loading, for the loader to list with the animation name.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    /// Animation or controller name (empty for the file).
    pub context: String,
    pub message: String,
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.context.is_empty() {
            write!(f, "{}", self.message)
        } else {
            write!(f, "'{}': {}", self.context, self.message)
        }
    }
}

/// A fatal load error.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadError {
    pub context: String,
    pub message: String,
    /// Column in an expression, if the error came from one.
    pub column: Option<usize>,
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.context.is_empty() {
            write!(f, "'{}': ", self.context)?;
        }
        write!(f, "{}", self.message)?;
        if let Some(c) = self.column {
            write!(f, " (column {c})")?;
        }
        Ok(())
    }
}

impl std::error::Error for LoadError {}

pub(crate) fn load_err(context: &str, message: impl Into<String>) -> LoadError {
    LoadError { context: context.to_string(), message: message.into(), column: None }
}

/// Loading context: symbols, warnings, and the name used in messages.
pub(crate) struct Ctx<'a> {
    pub syms: &'a mut Symbols,
    pub warnings: &'a mut Vec<Warning>,
    pub context: String,
}

impl Ctx<'_> {
    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(Warning { context: self.context.clone(), message: message.into() });
    }

    pub fn err(&self, message: impl Into<String>) -> LoadError {
        load_err(&self.context, message)
    }

    /// Compile an expression; unsupported Molang becomes a warning and the constant 0.
    pub fn compile(&mut self, src: &str, what: &str) -> Result<Expr, LoadError> {
        match compile(src, self.syms) {
            Ok(e) => Ok(e),
            Err(e) if e.kind == ExprErrorKind::Unsupported => {
                self.warn(format!("{what}: unsupported expression '{src}' ({e}); using 0"));
                Ok(Expr::constant(0.0))
            }
            Err(e) => Err(expr_load_error(&self.context, what, src, &e)),
        }
    }

    pub fn scalar(&mut self, v: &Value, what: &str) -> Result<Scalar, LoadError> {
        match v {
            Value::Number(n) => Ok(Scalar::Const(n.as_f64().unwrap_or(0.0) as f32)),
            Value::Bool(b) => Ok(Scalar::Const(if *b { 1.0 } else { 0.0 })),
            Value::String(s) => {
                if let Ok(x) = s.trim().parse::<f32>() {
                    if x.is_finite() {
                        return Ok(Scalar::Const(x));
                    }
                }
                let e = self.compile(s, what)?;
                Ok(Scalar::from_expr(e))
            }
            _ => Err(self.err(format!("{what}: expected a number or an expression string"))),
        }
    }

    pub fn vec3(&mut self, v: &Value, what: &str) -> Result<Vec3Expr, LoadError> {
        match v {
            Value::Array(a) => {
                if a.len() != 3 {
                    return Err(self.err(format!("{what}: a vector needs 3 values, got {}", a.len())));
                }
                Ok(Vec3Expr([self.scalar(&a[0], what)?, self.scalar(&a[1], what)?, self.scalar(&a[2], what)?]))
            }
            other => {
                let s = self.scalar(other, what)?;
                Ok(Vec3Expr([s.clone(), s.clone(), s]))
            }
        }
    }
}

pub(crate) fn expr_load_error(context: &str, what: &str, src: &str, e: &ExprError) -> LoadError {
    LoadError { context: context.to_string(), message: format!("{what}: '{src}': {}", e.message), column: Some(e.column) }
}

/// A JSON object read with its key order kept (serde_json's own map sorts keys).
pub(crate) struct OrderedMap<T>(pub Vec<(String, T)>);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OrderedMap<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
            type Value = OrderedMap<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("an object")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut m: A) -> Result<OrderedMap<T>, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = m.next_entry::<String, T>()? {
                    out.push((k, v));
                }
                Ok(OrderedMap(out))
            }
        }
        d.deserialize_map(V(std::marker::PhantomData))
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Easing
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EaseFamily {
    Sine,
    Quad,
    Cubic,
    Quart,
    Quint,
    Expo,
    Circ,
    Back,
    Elastic,
    Bounce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EaseDir {
    In,
    Out,
    InOut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Easing {
    #[default]
    Linear,
    Ease(EaseFamily, EaseDir),
}

fn bounce_out(x: f32) -> f32 {
    const N1: f32 = 7.5625;
    const D1: f32 = 2.75;
    if x < 1.0 / D1 {
        N1 * x * x
    } else if x < 2.0 / D1 {
        let x = x - 1.5 / D1;
        N1 * x * x + 0.75
    } else if x < 2.5 / D1 {
        let x = x - 2.25 / D1;
        N1 * x * x + 0.9375
    } else {
        let x = x - 2.625 / D1;
        N1 * x * x + 0.984375
    }
}

impl Easing {
    /// Names as written in animation files: `linear`, `easeInSine`, `easeOutBack`, `easeInOutBounce` and so on;
    /// case, underscores and dashes are ignored (`ease_in_out_sine` works too).
    pub fn parse(name: &str) -> Option<Easing> {
        let n: String = name.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_ascii_lowercase();
        if n == "linear" {
            return Some(Easing::Linear);
        }
        let n = n.strip_prefix("ease").unwrap_or(&n);
        let (dir, rest) = if let Some(r) = n.strip_prefix("inout") {
            (EaseDir::InOut, r)
        } else if let Some(r) = n.strip_prefix("in") {
            (EaseDir::In, r)
        } else {
            (EaseDir::Out, n.strip_prefix("out")?)
        };
        let family = match rest {
            "sine" => EaseFamily::Sine,
            "quad" => EaseFamily::Quad,
            "cubic" => EaseFamily::Cubic,
            "quart" => EaseFamily::Quart,
            "quint" => EaseFamily::Quint,
            "expo" => EaseFamily::Expo,
            "circ" => EaseFamily::Circ,
            "back" => EaseFamily::Back,
            "elastic" => EaseFamily::Elastic,
            "bounce" => EaseFamily::Bounce,
            _ => return None,
        };
        Some(Easing::Ease(family, dir))
    }

    /// Map progress `x` in `[0, 1]` (clamped) to eased progress. 0 maps to 0 and 1 to 1 for every curve.
    pub fn apply(self, x: f32) -> f32 {
        let x = if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 };
        let Easing::Ease(fam, dir) = self else { return x };
        if x == 0.0 || x == 1.0 {
            return x;
        }
        use std::f32::consts::PI;
        let y = match (fam, dir) {
            (EaseFamily::Sine, EaseDir::In) => 1.0 - (x * PI / 2.0).cos(),
            (EaseFamily::Sine, EaseDir::Out) => (x * PI / 2.0).sin(),
            (EaseFamily::Sine, EaseDir::InOut) => -((PI * x).cos() - 1.0) / 2.0,
            (EaseFamily::Quad | EaseFamily::Cubic | EaseFamily::Quart | EaseFamily::Quint, d) => {
                let n = match fam {
                    EaseFamily::Quad => 2,
                    EaseFamily::Cubic => 3,
                    EaseFamily::Quart => 4,
                    _ => 5,
                };
                match d {
                    EaseDir::In => x.powi(n),
                    EaseDir::Out => 1.0 - (1.0 - x).powi(n),
                    EaseDir::InOut => {
                        if x < 0.5 {
                            2.0f32.powi(n - 1) * x.powi(n)
                        } else {
                            1.0 - (-2.0 * x + 2.0).powi(n) / 2.0
                        }
                    }
                }
            }
            (EaseFamily::Expo, EaseDir::In) => 2.0f32.powf(10.0 * x - 10.0),
            (EaseFamily::Expo, EaseDir::Out) => 1.0 - 2.0f32.powf(-10.0 * x),
            (EaseFamily::Expo, EaseDir::InOut) => {
                if x < 0.5 {
                    2.0f32.powf(20.0 * x - 10.0) / 2.0
                } else {
                    (2.0 - 2.0f32.powf(-20.0 * x + 10.0)) / 2.0
                }
            }
            (EaseFamily::Circ, EaseDir::In) => 1.0 - (1.0 - x * x).sqrt(),
            (EaseFamily::Circ, EaseDir::Out) => (1.0 - (x - 1.0) * (x - 1.0)).sqrt(),
            (EaseFamily::Circ, EaseDir::InOut) => {
                if x < 0.5 {
                    (1.0 - (1.0 - (2.0 * x) * (2.0 * x)).sqrt()) / 2.0
                } else {
                    ((1.0 - (-2.0 * x + 2.0) * (-2.0 * x + 2.0)).sqrt() + 1.0) / 2.0
                }
            }
            (EaseFamily::Back, d) => {
                const C1: f32 = 1.70158;
                const C2: f32 = C1 * 1.525;
                const C3: f32 = C1 + 1.0;
                match d {
                    EaseDir::In => C3 * x * x * x - C1 * x * x,
                    EaseDir::Out => 1.0 + C3 * (x - 1.0).powi(3) + C1 * (x - 1.0).powi(2),
                    EaseDir::InOut => {
                        if x < 0.5 {
                            ((2.0 * x).powi(2) * ((C2 + 1.0) * 2.0 * x - C2)) / 2.0
                        } else {
                            ((2.0 * x - 2.0).powi(2) * ((C2 + 1.0) * (2.0 * x - 2.0) + C2) + 2.0) / 2.0
                        }
                    }
                }
            }
            (EaseFamily::Elastic, d) => {
                let c4 = 2.0 * PI / 3.0;
                let c5 = 2.0 * PI / 4.5;
                match d {
                    EaseDir::In => -(2.0f32.powf(10.0 * x - 10.0)) * ((10.0 * x - 10.75) * c4).sin(),
                    EaseDir::Out => 2.0f32.powf(-10.0 * x) * ((10.0 * x - 0.75) * c4).sin() + 1.0,
                    EaseDir::InOut => {
                        if x < 0.5 {
                            -(2.0f32.powf(20.0 * x - 10.0) * ((20.0 * x - 11.125) * c5).sin()) / 2.0
                        } else {
                            2.0f32.powf(-20.0 * x + 10.0) * ((20.0 * x - 11.125) * c5).sin() / 2.0 + 1.0
                        }
                    }
                }
            }
            (EaseFamily::Bounce, EaseDir::In) => 1.0 - bounce_out(1.0 - x),
            (EaseFamily::Bounce, EaseDir::Out) => bounce_out(x),
            (EaseFamily::Bounce, EaseDir::InOut) => {
                if x < 0.5 {
                    (1.0 - bounce_out(1.0 - 2.0 * x)) / 2.0
                } else {
                    (1.0 + bounce_out(2.0 * x - 1.0)) / 2.0
                }
            }
        };
        if y.is_finite() {
            y
        } else {
            x
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LerpMode {
    #[default]
    Linear,
    CatmullRom,
    Bezier,
    Step,
}

impl LerpMode {
    pub fn parse(s: &str) -> Option<LerpMode> {
        match s.to_ascii_lowercase().as_str() {
            "linear" => Some(LerpMode::Linear),
            "catmullrom" | "catmull_rom" => Some(LerpMode::CatmullRom),
            "bezier" => Some(LerpMode::Bezier),
            "step" => Some(LerpMode::Step),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// Plays once, then stops affecting the pose.
    #[default]
    Once,
    Loop,
    /// Plays once, then holds the last frame.
    Hold,
}

/// A number or a compiled expression.
#[derive(Clone, Debug, PartialEq)]
pub enum Scalar {
    Const(f32),
    Expr(Expr),
}

impl Scalar {
    /// The negated scalar (an expression that cannot take one more instruction becomes 0).
    pub fn negated(&self) -> Scalar {
        match self {
            Scalar::Const(v) => Scalar::Const(-v),
            Scalar::Expr(e) => e.negated().map_or(Scalar::Const(0.0), Scalar::Expr),
        }
    }

    pub fn from_expr(e: Expr) -> Scalar {
        match e.as_constant() {
            Some(v) => Scalar::Const(v),
            None => Scalar::Expr(e),
        }
    }

    pub fn eval(&self, inp: &ExprInputs) -> f32 {
        match self {
            Scalar::Const(v) => *v,
            Scalar::Expr(e) => e.eval(inp),
        }
    }
}

/// Three scalars (x, y, z).
#[derive(Clone, Debug, PartialEq)]
pub struct Vec3Expr(pub [Scalar; 3]);

impl Vec3Expr {
    fn flip(&mut self, axes: [bool; 3]) {
        for (s, f) in self.0.iter_mut().zip(axes) {
            if f {
                *s = s.negated();
            }
        }
    }

    pub fn constant(v: Vec3) -> Self {
        Vec3Expr([Scalar::Const(v.x), Scalar::Const(v.y), Scalar::Const(v.z)])
    }

    pub fn eval(&self, inp: &ExprInputs) -> Vec3 {
        Vec3::new(self.0[0].eval(inp), self.0[1].eval(inp), self.0[2].eval(inp))
    }
}

/// Bezier handles of one keyframe, per axis, as offsets from the keyframe: a time offset and a value offset.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BezierHandles {
    /// Negative or zero time offsets.
    pub left_time: Vec3,
    pub left_value: Vec3,
    /// Positive or zero time offsets.
    pub right_time: Vec3,
    pub right_value: Vec3,
}

impl Default for BezierHandles {
    /// Blockbench's defaults: 0.1 s each side, flat.
    fn default() -> Self {
        Self {
            left_time: Vec3::splat(-0.1),
            left_value: Vec3::ZERO,
            right_time: Vec3::splat(0.1),
            right_value: Vec3::ZERO,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Keyframe {
    pub time: f32,
    pub pre: Vec3Expr,
    pub post: Vec3Expr,
    /// Interpolation of the segment that starts here.
    pub lerp: LerpMode,
    pub easing: Easing,
    pub bezier: BezierHandles,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Channel {
    Constant(Vec3Expr),
    /// At least one keyframe, sorted by time.
    Keys(Vec<Keyframe>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelKind {
    Rotation,
    Position,
    Scale,
}

impl ChannelKind {
    pub fn default_value(self) -> Vec3 {
        match self {
            ChannelKind::Scale => Vec3::ONE,
            _ => Vec3::ZERO,
        }
    }
}

fn finite3(v: Vec3) -> Vec3 {
    Vec3::new(
        if v.x.is_finite() { v.x } else { 0.0 },
        if v.y.is_finite() { v.y } else { 0.0 },
        if v.z.is_finite() { v.z } else { 0.0 },
    )
}

/// Cubic Bezier of one axis in (time, value), solved for the value at `t`. Handle times are clamped inside the
/// segment so time stays monotonic.
#[allow(clippy::too_many_arguments)]
fn bezier_axis(t0: f32, v0: f32, rt: f32, rv: f32, t1: f32, v1: f32, lt: f32, lv: f32, t: f32) -> f32 {
    let dur = t1 - t0;
    if dur <= 0.0 {
        return v1;
    }
    let x1 = t0 + rt.clamp(0.0, dur);
    let x2 = t1 + lt.clamp(-dur, 0.0);
    let y1 = v0 + rv;
    let y2 = v1 + lv;
    let cubic = |p0: f32, p1: f32, p2: f32, p3: f32, s: f32| {
        let u = 1.0 - s;
        u * u * u * p0 + 3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s * p3
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..32 {
        let mid = 0.5 * (lo + hi);
        if cubic(t0, x1, x2, t1, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cubic(v0, y1, y2, v1, 0.5 * (lo + hi))
}

fn catmull(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3, s: f32) -> Vec3 {
    let s2 = s * s;
    let s3 = s2 * s;
    0.5 * (2.0 * p1
        + (p2 - p0) * s
        + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * s2
        + (3.0 * p1 - p0 - 3.0 * p2 + p3) * s3)
}

impl Channel {
    /// The channel's value at `t` seconds.
    pub fn sample(&self, t: f32, inp: &ExprInputs) -> Vec3 {
        let v = match self {
            Channel::Constant(v) => v.eval(inp),
            Channel::Keys(keys) => sample_keys(keys, t, inp),
        };
        finite3(v)
    }

    /// Largest keyframe time (0 for a constant).
    pub fn end_time(&self) -> f32 {
        match self {
            Channel::Constant(_) => 0.0,
            Channel::Keys(k) => k.last().map_or(0.0, |k| k.time),
        }
    }
}

fn sample_keys(keys: &[Keyframe], t: f32, inp: &ExprInputs) -> Vec3 {
    let (Some(first), Some(last)) = (keys.first(), keys.last()) else { return Vec3::ZERO };
    if t < first.time {
        return first.pre.eval(inp);
    }
    if t >= last.time {
        return last.post.eval(inp);
    }
    let i = keys.partition_point(|k| k.time <= t).saturating_sub(1).min(keys.len() - 2);
    let (a, b) = (&keys[i], &keys[i + 1]);
    let span = b.time - a.time;
    if t == a.time {
        return a.post.eval(inp); // exactly at a key: its leaving value, with no solver error
    }
    let s = if span > 0.0 { ((t - a.time) / span).clamp(0.0, 1.0) } else { 1.0 };
    match a.lerp {
        LerpMode::Step => a.post.eval(inp),
        LerpMode::Linear => {
            let (p1, p2) = (a.post.eval(inp), b.pre.eval(inp));
            p1 + (p2 - p1) * a.easing.apply(s)
        }
        LerpMode::CatmullRom => {
            let (p1, p2) = (a.post.eval(inp), b.pre.eval(inp));
            let p0 = if i > 0 { keys[i - 1].post.eval(inp) } else { p1 };
            let p3 = keys.get(i + 2).map_or(p2, |k| k.pre.eval(inp));
            catmull(p0, p1, p2, p3, a.easing.apply(s))
        }
        LerpMode::Bezier => {
            let (p1, p2) = (a.post.eval(inp), b.pre.eval(inp));
            let (ha, hb) = (&a.bezier, &b.bezier);
            Vec3::new(
                bezier_axis(a.time, p1.x, ha.right_time.x, ha.right_value.x, b.time, p2.x, hb.left_time.x, hb.left_value.x, t),
                bezier_axis(a.time, p1.y, ha.right_time.y, ha.right_value.y, b.time, p2.y, hb.left_time.y, hb.left_value.y, t),
                bezier_axis(a.time, p1.z, ha.right_time.z, ha.right_value.z, b.time, p2.z, hb.left_time.z, hb.left_value.z, t),
            )
        }
    }
}

/// All channels of one bone in one animation.
#[derive(Clone, Debug, PartialEq)]
pub struct BoneAnim {
    pub bone: String,
    pub rotation: Option<Channel>,
    pub position: Option<Channel>,
    pub scale: Option<Channel>,
}

impl BoneAnim {
    pub fn channel(&self, kind: ChannelKind) -> Option<&Channel> {
        match kind {
            ChannelKind::Rotation => self.rotation.as_ref(),
            ChannelKind::Position => self.position.as_ref(),
            ChannelKind::Scale => self.scale.as_ref(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Timeline,
    Sound,
    Particle,
}

/// A timeline, sound or particle event at a time in the animation.
#[derive(Clone, Debug, PartialEq)]
pub struct AnimEvent {
    pub time: f32,
    pub kind: EventKind,
    /// Timeline event name, sound id or particle id.
    pub name: String,
    pub locator: Option<String>,
}

/// Most loop iterations one `emit_events` call walks; a longer stall only emits the last ones.
pub const MAX_EVENT_LOOPS_PER_CALL: u32 = 64;

#[derive(Clone, Debug, PartialEq)]
pub struct Animation {
    pub name: String,
    pub loop_mode: LoopMode,
    pub length: f32,
    pub override_previous: bool,
    pub anim_time_update: Option<Expr>,
    pub blend_weight: Option<Expr>,
    pub bones: Vec<BoneAnim>,
    /// Sorted by time (stable).
    pub events: Vec<AnimEvent>,
}

impl Animation {
    /// The time to sample for `clock` seconds on the animation's own clock, or `None` when a `Once` animation has
    /// finished. Sets `query.anim_time` in `inp` to the clock while `anim_time_update` runs, and to the returned time
    /// afterwards (so keyframe expressions see the wrapped time).
    pub fn sample_time(&self, clock: f32, inp: &mut ExprInputs) -> Option<f32> {
        let clock = if clock.is_finite() { clock.max(0.0) } else { 0.0 };
        inp.set(Query::AnimTime, clock);
        let mut t = match &self.anim_time_update {
            Some(e) => e.eval(inp),
            None => clock,
        };
        if t < 0.0 {
            t = 0.0;
        }
        let l = self.length;
        let t = if l <= 0.0 {
            0.0
        } else {
            match self.loop_mode {
                LoopMode::Loop => t - (t / l).floor() * l,
                LoopMode::Hold => t.min(l),
                LoopMode::Once => {
                    if t >= l {
                        return None;
                    }
                    t
                }
            }
        };
        inp.set(Query::AnimTime, t);
        Some(t)
    }

    /// The animation's own weight (`blend_weight`, default 1), at least 0.
    pub fn weight(&self, inp: &ExprInputs) -> f32 {
        self.blend_weight.as_ref().map_or(1.0, |e| e.eval(inp).max(0.0))
    }

    /// Call `cb(event, loop_index)` for every event whose time falls in the clock window `[c0, c1)`, in time order.
    /// A loop animation repeats its events every loop, once each. Nothing is emitted when the clock did not advance
    /// (a paused animation). Events past a non-looping animation's length never fire.
    pub fn emit_events(&self, c0: f32, c1: f32, mut cb: impl FnMut(&AnimEvent, u32)) {
        if !(c0.is_finite() && c1.is_finite()) || c1 <= c0 || self.events.is_empty() {
            return;
        }
        let c0 = c0.max(0.0);
        if c1 <= c0 {
            return;
        }
        let l = self.length;
        if self.loop_mode == LoopMode::Loop && l > 0.0 {
            let k_end = (c1 / l).floor().max(0.0) as u32;
            let mut k = (c0 / l).floor().max(0.0) as u32;
            if k_end.saturating_sub(k) > MAX_EVENT_LOOPS_PER_CALL {
                k = k_end - MAX_EVENT_LOOPS_PER_CALL;
            }
            while k <= k_end {
                let base = k as f32 * l;
                for ev in &self.events {
                    if ev.time >= l {
                        continue;
                    }
                    let te = base + ev.time;
                    if te >= c0 && te < c1 {
                        cb(ev, k);
                    }
                }
                k += 1;
            }
        } else {
            for ev in &self.events {
                if ev.time >= c0 && ev.time < c1 && (l <= 0.0 || ev.time <= l) {
                    cb(ev, 0);
                }
            }
        }
    }
}

/// The animations of a mob type, in load order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AnimationLibrary {
    animations: Vec<Animation>,
}

impl AnimationLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace (by exact name). Errors past [`MAX_ANIMATIONS`].
    pub fn add(&mut self, a: Animation) -> Result<(), LoadError> {
        if let Some(i) = self.animations.iter().position(|x| x.name == a.name) {
            self.animations[i] = a;
            return Ok(());
        }
        if self.animations.len() >= MAX_ANIMATIONS {
            return Err(load_err(&a.name, format!("more than {MAX_ANIMATIONS} animations")));
        }
        self.animations.push(a);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.animations.len()
    }

    pub fn is_empty(&self) -> bool {
        self.animations.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<&Animation> {
        self.animations.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Animation> {
        self.animations.iter()
    }

    /// Index by exact name, else the one animation whose name ends in `.query` (so `walk` finds
    /// `animation.fox.walk` when that is unambiguous).
    pub fn find(&self, query: &str) -> Option<usize> {
        if let Some(i) = self.animations.iter().position(|a| a.name == query) {
            return Some(i);
        }
        let suffix = format!(".{query}");
        let mut hits = self.animations.iter().enumerate().filter(|(_, a)| a.name.ends_with(&suffix));
        match (hits.next(), hits.next()) {
            (Some((i, _)), None) => Some(i),
            _ => None,
        }
    }
}

/// What a parse returns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParsedAnimations {
    pub animations: Vec<Animation>,
    pub warnings: Vec<Warning>,
}

// ---------------------------------------------------------------------------------------------------------------
// animation.json
// ---------------------------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct AnimFile {
    #[serde(default)]
    animations: Option<OrderedMap<Value>>,
}

/// Parse an `animation.json` document. Names, flags and keyframes as in section 3.6; see the module docs for the
/// conventions. Unsupported fields and Molang constructs become warnings.
pub fn parse_animation_file(json: &str, syms: &mut Symbols) -> Result<ParsedAnimations, LoadError> {
    let file: AnimFile = serde_json::from_str(json).map_err(|e| load_err("", format!("invalid JSON: {e}")))?;
    let mut out = ParsedAnimations::default();
    let Some(OrderedMap(list)) = file.animations else {
        return Err(load_err("", "no \"animations\" object"));
    };
    for (name, value) in list {
        let mut ctx = Ctx { syms, warnings: &mut out.warnings, context: name.clone() };
        let anim = parse_animation(&name, &value, &mut ctx)?;
        out.animations.push(anim);
    }
    Ok(out)
}

fn parse_time_key(key: &str, ctx: &Ctx, what: &str) -> Result<f32, LoadError> {
    match key.trim().parse::<f32>() {
        Ok(t) if t.is_finite() && t >= 0.0 => Ok(t),
        _ => Err(ctx.err(format!("{what}: '{key}' is not a time in seconds (a number >= 0)"))),
    }
}

fn parse_animation(name: &str, v: &Value, ctx: &mut Ctx) -> Result<Animation, LoadError> {
    let Some(obj) = v.as_object() else { return Err(ctx.err("animation must be an object")) };
    let mut anim = Animation {
        name: name.to_string(),
        loop_mode: LoopMode::Once,
        length: 0.0,
        override_previous: false,
        anim_time_update: None,
        blend_weight: None,
        bones: Vec::new(),
        events: Vec::new(),
    };
    let mut explicit_length = None;
    for (key, val) in obj {
        match key.as_str() {
            "loop" => {
                anim.loop_mode = match val {
                    Value::Bool(true) => LoopMode::Loop,
                    Value::Bool(false) => LoopMode::Once,
                    Value::String(s) if s == "hold_on_last_frame" => LoopMode::Hold,
                    Value::String(s) if s == "true" => LoopMode::Loop,
                    Value::String(s) if s == "false" => LoopMode::Once,
                    _ => return Err(ctx.err("loop must be true, false or \"hold_on_last_frame\"")),
                }
            }
            "animation_length" => match val.as_f64() {
                Some(l) if l.is_finite() && l >= 0.0 => explicit_length = Some(l as f32),
                _ => return Err(ctx.err("animation_length must be a number >= 0")),
            },
            "override_previous_animation" => match val.as_bool() {
                Some(b) => anim.override_previous = b,
                None => return Err(ctx.err("override_previous_animation must be true or false")),
            },
            "anim_time_update" => {
                let s = ctx.scalar(val, "anim_time_update")?;
                anim.anim_time_update = Some(match s {
                    Scalar::Const(c) => Expr::constant(c),
                    Scalar::Expr(e) => e,
                });
            }
            "blend_weight" => {
                let s = ctx.scalar(val, "blend_weight")?;
                anim.blend_weight = Some(match s {
                    Scalar::Const(c) => Expr::constant(c),
                    Scalar::Expr(e) => e,
                });
            }
            "bones" => {
                let Some(bones) = val.as_object() else { return Err(ctx.err("bones must be an object")) };
                for (bone, bv) in bones {
                    anim.bones.push(parse_bone(bone, bv, ctx)?);
                }
            }
            "timeline" => parse_events(EventKind::Timeline, val, &mut anim.events, ctx)?,
            "sound_effects" => parse_events(EventKind::Sound, val, &mut anim.events, ctx)?,
            "particle_effects" => parse_events(EventKind::Particle, val, &mut anim.events, ctx)?,
            "format_version" | "start_delay" | "loop_delay" | "reset" | "animation_name" => {
                if matches!(key.as_str(), "start_delay" | "loop_delay") && val.as_f64().is_none_or(|x| x != 0.0) {
                    ctx.warn(format!("{key} is not supported (ignored)"));
                }
            }
            other => ctx.warn(format!("unsupported field '{other}' (ignored)")),
        }
    }
    anim.length = match explicit_length {
        Some(l) => l,
        None => derived_length(&anim),
    };
    anim.events.sort_by(|a, b| a.time.total_cmp(&b.time));
    check_events(&mut anim, ctx);
    Ok(anim)
}

fn derived_length(a: &Animation) -> f32 {
    let mut l = 0.0f32;
    for b in &a.bones {
        for c in [&b.rotation, &b.position, &b.scale].into_iter().flatten() {
            l = l.max(c.end_time());
        }
    }
    for e in &a.events {
        l = l.max(e.time);
    }
    l
}

fn check_events(a: &mut Animation, ctx: &mut Ctx) {
    if a.length > 0.0 {
        for e in &a.events {
            if e.time > a.length {
                ctx.warn(format!("event '{}' at {}s is past animation_length {}s and never fires", e.name, e.time, a.length));
            }
        }
    }
}

fn parse_events(kind: EventKind, v: &Value, out: &mut Vec<AnimEvent>, ctx: &mut Ctx) -> Result<(), LoadError> {
    let what = match kind {
        EventKind::Timeline => "timeline",
        EventKind::Sound => "sound_effects",
        EventKind::Particle => "particle_effects",
    };
    let Some(obj) = v.as_object() else { return Err(ctx.err(format!("{what} must be an object keyed by time"))) };
    for (key, val) in obj {
        let time = parse_time_key(key, ctx, what)?;
        let items: Vec<&Value> = match val {
            Value::Array(a) => a.iter().collect(),
            other => vec![other],
        };
        for item in items {
            let (name, locator) = match (kind, item) {
                (_, Value::String(s)) => (s.clone(), None),
                (EventKind::Timeline, _) => return Err(ctx.err(format!("{what} at {key}: expected a name or a list of names"))),
                (_, Value::Object(o)) => {
                    let Some(effect) = o.get("effect").and_then(Value::as_str) else {
                        return Err(ctx.err(format!("{what} at {key}: missing \"effect\"")));
                    };
                    (effect.to_string(), o.get("locator").and_then(Value::as_str).map(str::to_string))
                }
                _ => return Err(ctx.err(format!("{what} at {key}: expected an object with \"effect\""))),
            };
            out.push(AnimEvent { time, kind, name, locator });
        }
    }
    Ok(())
}

/// Bedrock animation values to the internal Blockbench space (same mapping as the geo.json loader): position
/// `(x, y, z)` to `(-x, y, z)`, rotation `(rx, ry, rz)` to `(-rx, -ry, rz)`. Scale is unchanged.
fn bedrock_to_internal(ch: &mut Channel, kind: ChannelKind) {
    let axes = match kind {
        ChannelKind::Rotation => [true, true, false],
        ChannelKind::Position => [true, false, false],
        ChannelKind::Scale => return,
    };
    let flip3 = |v: &mut Vec3| {
        for (i, f) in axes.iter().enumerate() {
            if *f {
                v[i] = -v[i];
            }
        }
    };
    match ch {
        Channel::Constant(v) => v.flip(axes),
        Channel::Keys(keys) => {
            for k in keys {
                k.pre.flip(axes);
                k.post.flip(axes);
                flip3(&mut k.bezier.left_value);
                flip3(&mut k.bezier.right_value);
            }
        }
    }
}

fn parse_bone(bone: &str, v: &Value, ctx: &mut Ctx) -> Result<BoneAnim, LoadError> {
    let Some(obj) = v.as_object() else { return Err(ctx.err(format!("bone '{bone}' must be an object"))) };
    let mut b = BoneAnim { bone: bone.to_string(), rotation: None, position: None, scale: None };
    for (key, val) in obj {
        let kind = match key.as_str() {
            "rotation" => ChannelKind::Rotation,
            "position" => ChannelKind::Position,
            "scale" => ChannelKind::Scale,
            other => {
                ctx.warn(format!("bone '{bone}': unsupported field '{other}' (ignored)"));
                continue;
            }
        };
        let what = format!("bone '{bone}' {key}");
        let mut ch = parse_channel(val, &what, ctx)?;
        bedrock_to_internal(&mut ch, kind);
        match kind {
            ChannelKind::Rotation => b.rotation = Some(ch),
            ChannelKind::Position => b.position = Some(ch),
            ChannelKind::Scale => b.scale = Some(ch),
        }
    }
    Ok(b)
}

fn parse_channel(v: &Value, what: &str, ctx: &mut Ctx) -> Result<Channel, LoadError> {
    let Value::Object(obj) = v else { return Ok(Channel::Constant(ctx.vec3(v, what)?)) };
    let mut keys: Vec<Keyframe> = Vec::new();
    for (tk, kv) in obj {
        let time = parse_time_key(tk, ctx, what)?;
        let kw = format!("{what} at {tk}");
        let mut key = Keyframe {
            time,
            pre: Vec3Expr::constant(Vec3::ZERO),
            post: Vec3Expr::constant(Vec3::ZERO),
            lerp: LerpMode::Linear,
            easing: Easing::Linear,
            bezier: BezierHandles::default(),
        };
        if let Value::Object(o) = kv {
            let (mut pre, mut post) = (None, None);
            for (f, fv) in o {
                match f.as_str() {
                    "pre" => pre = Some(ctx.vec3(fv, &kw)?),
                    "post" => post = Some(ctx.vec3(fv, &kw)?),
                    "lerp_mode" => match fv.as_str().and_then(LerpMode::parse) {
                        Some(m) => key.lerp = m,
                        None => {
                            ctx.warn(format!("{kw}: unknown lerp_mode {fv}, using linear"));
                        }
                    },
                    "easing" => match fv.as_str().and_then(Easing::parse) {
                        Some(e) => key.easing = e,
                        None => ctx.warn(format!("{kw}: unknown easing {fv}, using linear")),
                    },
                    "bezier_left_time" => key.bezier.left_time = f32_vec3(fv, &kw, ctx)?,
                    "bezier_left_value" => key.bezier.left_value = f32_vec3(fv, &kw, ctx)?,
                    "bezier_right_time" => key.bezier.right_time = f32_vec3(fv, &kw, ctx)?,
                    "bezier_right_value" => key.bezier.right_value = f32_vec3(fv, &kw, ctx)?,
                    other => ctx.warn(format!("{kw}: unsupported field '{other}' (ignored)")),
                }
            }
            match (pre, post) {
                (Some(a), Some(b)) => {
                    key.pre = a;
                    key.post = b;
                }
                (Some(a), None) => {
                    key.post = a.clone();
                    key.pre = a;
                }
                (None, Some(b)) => {
                    key.pre = b.clone();
                    key.post = b;
                }
                (None, None) => return Err(ctx.err(format!("{kw}: needs \"pre\" and/or \"post\""))),
            }
        } else {
            let v3 = ctx.vec3(kv, &kw)?;
            key.pre = v3.clone();
            key.post = v3;
        }
        keys.push(key);
    }
    if keys.is_empty() {
        return Err(ctx.err(format!("{what}: empty keyframe table")));
    }
    // Two keys that parse to the same time (for example "1" and "1.0") are an authoring mistake: keep the later.
    keys.sort_by(|a, b| a.time.total_cmp(&b.time));
    let before = keys.len();
    let mut dedup: Vec<Keyframe> = Vec::with_capacity(keys.len());
    for k in keys {
        if dedup.last().is_some_and(|p| p.time == k.time) {
            dedup.pop();
        }
        dedup.push(k);
    }
    if dedup.len() != before {
        ctx.warn(format!("{what}: duplicate keyframe times, kept the last of each"));
    }
    Ok(Channel::Keys(dedup))
}

fn f32_vec3(v: &Value, what: &str, ctx: &Ctx) -> Result<Vec3, LoadError> {
    let bad = || ctx.err(format!("{what}: expected three numbers"));
    match v {
        Value::Array(a) if a.len() == 3 => {
            let g = |i: usize| a[i].as_f64().map(|x| x as f32).ok_or_else(bad);
            Ok(Vec3::new(g(0)?, g(1)?, g(2)?))
        }
        Value::Number(n) => Ok(Vec3::splat(n.as_f64().unwrap_or(0.0) as f32)),
        _ => Err(bad()),
    }
}

// ---------------------------------------------------------------------------------------------------------------
// bbmodel animations[]
// ---------------------------------------------------------------------------------------------------------------

/// Convert the `animations` array of a `.bbmodel` into the same [`Animation`] type, keyed by bone (group) name.
///
/// Read per animation: `name`, `length`, `loop` (`once`, `hold`, `loop`), `override`, `anim_time_update`,
/// `blend_weight`, and `animators` (an object; each animator has `name` = the group name and `keyframes`). A bone
/// keyframe has `channel` (`rotation`, `position`, `scale`), `time`, `interpolation`
/// (`linear`, `catmullrom`, `bezier`, `step`), `data_points` (one point, or two for pre and post, each `{x, y, z}` of
/// numbers or expression strings; an empty string means the channel default), and for bezier
/// `bezier_left_time`, `bezier_left_value`, `bezier_right_time`, `bezier_right_value`. An animator of
/// `type: "effect"` has keyframes with `channel` `sound`, `particle` or `timeline` and `data_points` of
/// `{effect, locator}` (sound, particle) or `{script}` (timeline; the script text is the event name).
pub fn animations_from_bbmodel(animations: &Value, syms: &mut Symbols) -> Result<ParsedAnimations, LoadError> {
    let mut out = ParsedAnimations::default();
    let Some(list) = animations.as_array() else { return Err(load_err("", "animations must be an array")) };
    for (n, av) in list.iter().enumerate() {
        let name = av.get("name").and_then(Value::as_str).map_or_else(|| format!("animation_{n}"), str::to_string);
        let mut ctx = Ctx { syms, warnings: &mut out.warnings, context: name.clone() };
        let anim = convert_bb_animation(&name, av, &mut ctx)?;
        out.animations.push(anim);
    }
    Ok(out)
}

fn convert_bb_animation(name: &str, v: &Value, ctx: &mut Ctx) -> Result<Animation, LoadError> {
    let mut anim = Animation {
        name: name.to_string(),
        loop_mode: LoopMode::Once,
        length: 0.0,
        override_previous: v.get("override").and_then(Value::as_bool).unwrap_or(false),
        anim_time_update: None,
        blend_weight: None,
        bones: Vec::new(),
        events: Vec::new(),
    };
    anim.loop_mode = match v.get("loop").and_then(Value::as_str) {
        None | Some("once") => LoopMode::Once,
        Some("loop") => LoopMode::Loop,
        Some("hold") => LoopMode::Hold,
        Some(other) => return Err(ctx.err(format!("unknown loop mode '{other}' (once, hold, loop)"))),
    };
    for (field, slot) in [("anim_time_update", 0), ("blend_weight", 1)] {
        if let Some(s) = v.get(field).and_then(Value::as_str) {
            if !s.trim().is_empty() {
                let e = ctx.compile(s, field)?;
                if slot == 0 {
                    anim.anim_time_update = Some(e);
                } else {
                    anim.blend_weight = Some(e);
                }
            }
        }
    }
    let explicit_length = v.get("length").and_then(Value::as_f64).filter(|l| l.is_finite() && *l >= 0.0);
    if let Some(animators) = v.get("animators").and_then(Value::as_object) {
        let mut entries: Vec<(String, &Value)> = Vec::new();
        for (id, a) in animators {
            let bone = a.get("name").and_then(Value::as_str).unwrap_or(id).to_string();
            entries.push((bone, a));
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        for (bone, a) in entries {
            let kfs = a.get("keyframes").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            if a.get("type").and_then(Value::as_str) == Some("effect") {
                convert_bb_effects(kfs, &mut anim.events, ctx)?;
            } else {
                if anim.bones.iter().any(|b| b.bone == bone) {
                    ctx.warn(format!("two animators for bone '{bone}', the second is ignored"));
                    continue;
                }
                if let Some(b) = convert_bb_bone(&bone, kfs, ctx)? {
                    anim.bones.push(b);
                }
            }
        }
    }
    anim.length = explicit_length.map_or_else(|| derived_length(&anim), |l| l as f32);
    anim.events.sort_by(|a, b| a.time.total_cmp(&b.time));
    check_events(&mut anim, ctx);
    Ok(anim)
}

fn bb_time(k: &Value, ctx: &Ctx, what: &str) -> Result<f32, LoadError> {
    match k.get("time").and_then(Value::as_f64) {
        Some(t) if t.is_finite() && t >= 0.0 => Ok(t as f32),
        _ => Err(ctx.err(format!("{what}: keyframe without a valid \"time\""))),
    }
}

fn bb_point(p: &Value, kind: ChannelKind, what: &str, ctx: &mut Ctx) -> Result<Vec3Expr, LoadError> {
    let def = kind.default_value();
    let mut comps = [Scalar::Const(def.x), Scalar::Const(def.y), Scalar::Const(def.z)];
    for (i, axis) in ["x", "y", "z"].iter().enumerate() {
        match p.get(*axis) {
            None | Some(Value::Null) => {}
            Some(Value::String(s)) if s.trim().is_empty() => {}
            Some(v) => comps[i] = ctx.scalar(v, &format!("{what} {axis}"))?,
        }
    }
    Ok(Vec3Expr(comps))
}

fn bb_vec3(k: &Value, field: &str, default: Vec3, what: &str, ctx: &Ctx) -> Result<Vec3, LoadError> {
    match k.get(field) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => f32_vec3(v, &format!("{what} {field}"), ctx),
    }
}

fn convert_bb_bone(bone: &str, kfs: &[Value], ctx: &mut Ctx) -> Result<Option<BoneAnim>, LoadError> {
    let mut b = BoneAnim { bone: bone.to_string(), rotation: None, position: None, scale: None };
    for kind in [ChannelKind::Rotation, ChannelKind::Position, ChannelKind::Scale] {
        let cname = match kind {
            ChannelKind::Rotation => "rotation",
            ChannelKind::Position => "position",
            ChannelKind::Scale => "scale",
        };
        let what = format!("bone '{bone}' {cname}");
        let mut keys = Vec::new();
        for k in kfs.iter().filter(|k| k.get("channel").and_then(Value::as_str) == Some(cname)) {
            let time = bb_time(k, ctx, &what)?;
            let pts = k.get("data_points").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            let (pre, post) = match pts {
                [] => return Err(ctx.err(format!("{what} at {time}: no data_points"))),
                [p] => {
                    let v = bb_point(p, kind, &what, ctx)?;
                    (v.clone(), v)
                }
                [a, b, ..] => (bb_point(a, kind, &what, ctx)?, bb_point(b, kind, &what, ctx)?),
            };
            let lerp = match k.get("interpolation").and_then(Value::as_str) {
                None => LerpMode::Linear,
                Some(s) => LerpMode::parse(s).unwrap_or_else(|| {
                    ctx.warn(format!("{what} at {time}: unknown interpolation '{s}', using linear"));
                    LerpMode::Linear
                }),
            };
            let d = BezierHandles::default();
            let bezier = BezierHandles {
                left_time: bb_vec3(k, "bezier_left_time", d.left_time, &what, ctx)?,
                left_value: bb_vec3(k, "bezier_left_value", d.left_value, &what, ctx)?,
                right_time: bb_vec3(k, "bezier_right_time", d.right_time, &what, ctx)?,
                right_value: bb_vec3(k, "bezier_right_value", d.right_value, &what, ctx)?,
            };
            keys.push(Keyframe { time, pre, post, lerp, easing: Easing::Linear, bezier });
        }
        if keys.is_empty() {
            continue;
        }
        keys.sort_by(|a, b| a.time.total_cmp(&b.time));
        let ch = Channel::Keys(keys);
        match kind {
            ChannelKind::Rotation => b.rotation = Some(ch),
            ChannelKind::Position => b.position = Some(ch),
            ChannelKind::Scale => b.scale = Some(ch),
        }
    }
    if b.rotation.is_none() && b.position.is_none() && b.scale.is_none() {
        return Ok(None);
    }
    Ok(Some(b))
}

fn convert_bb_effects(kfs: &[Value], out: &mut Vec<AnimEvent>, ctx: &mut Ctx) -> Result<(), LoadError> {
    for k in kfs {
        let kind = match k.get("channel").and_then(Value::as_str) {
            Some("sound") => EventKind::Sound,
            Some("particle") => EventKind::Particle,
            Some("timeline") => EventKind::Timeline,
            other => {
                ctx.warn(format!("effect keyframe with unsupported channel {other:?} (ignored)"));
                continue;
            }
        };
        let time = bb_time(k, ctx, "effects")?;
        for p in k.get("data_points").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]) {
            let text = |f: &str| p.get(f).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
            let name = match kind {
                EventKind::Timeline => text("script").or_else(|| text("effect")),
                _ => text("effect"),
            };
            let Some(name) = name else {
                ctx.warn(format!("effect keyframe at {time}s has no effect name (ignored)"));
                continue;
            };
            out.push(AnimEvent { time, kind, name: name.to_string(), locator: text("locator").map(str::to_string) });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syms() -> Symbols {
        Symbols::new(&["mode"])
    }

    fn parse(json: &str) -> ParsedAnimations {
        parse_animation_file(json, &mut syms()).unwrap()
    }

    fn inp() -> ExprInputs {
        ExprInputs::default()
    }

    fn near(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-4, "{a} vs {b}");
    }

    fn ch(json: &str) -> Channel {
        let p = parse(&format!(r#"{{"animations":{{"a":{{"bones":{{"b":{{"scale":{json}}}}}}}}}}}"#));
        p.animations[0].bones[0].scale.clone().unwrap()
    }

    #[test]
    fn easing_table() {
        use EaseDir::*;
        use EaseFamily::*;
        let e = |f, d, x: f32| Easing::Ease(f, d).apply(x);
        near(e(Quad, In, 0.5), 0.25);
        near(e(Quad, Out, 0.5), 0.75);
        near(e(Quad, InOut, 0.25), 0.125);
        near(e(Quad, InOut, 0.75), 0.875);
        near(e(Cubic, In, 0.5), 0.125);
        near(e(Cubic, Out, 0.5), 0.875);
        near(e(Cubic, InOut, 0.25), 0.0625);
        near(e(Quart, In, 0.5), 0.0625);
        near(e(Quart, InOut, 0.25), 0.03125);
        near(e(Quint, In, 0.5), 0.03125);
        near(e(Quint, Out, 0.5), 0.96875);
        near(e(Sine, In, 0.5), 0.292893);
        near(e(Sine, Out, 0.5), std::f32::consts::FRAC_1_SQRT_2);
        near(e(Sine, InOut, 0.5), 0.5);
        near(e(Expo, In, 0.5), 0.03125);
        near(e(Expo, Out, 0.5), 0.96875);
        near(e(Expo, InOut, 0.25), 0.015625);
        near(e(Circ, In, 0.5), 0.133975);
        near(e(Circ, Out, 0.5), 0.866025);
        near(e(Circ, InOut, 0.25), 0.0669873);
        near(e(Back, In, 0.5), -0.0876975);
        near(e(Back, Out, 0.5), 1.0876975);
        near(e(Elastic, In, 0.5), -0.015625);
        near(e(Elastic, Out, 0.5), 1.015625);
        near(e(Bounce, Out, 0.5), 0.765625);
        near(e(Bounce, Out, 0.25), 0.47265625);
        near(e(Bounce, In, 0.5), 0.234375);
        near(e(Bounce, InOut, 0.25), 0.1171875);
        for f in [Sine, Quad, Cubic, Quart, Quint, Expo, Circ, Back, Elastic, Bounce] {
            for d in [In, Out, InOut] {
                assert_eq!(Easing::Ease(f, d).apply(0.0), 0.0, "{f:?} {d:?}");
                assert_eq!(Easing::Ease(f, d).apply(1.0), 1.0, "{f:?} {d:?}");
                assert!(Easing::Ease(f, d).apply(0.37).is_finite());
            }
        }
        assert_eq!(Easing::Linear.apply(0.3), 0.3);
        assert_eq!(Easing::Linear.apply(f32::NAN), 0.0);
        assert_eq!(Easing::Linear.apply(2.0), 1.0);
    }

    #[test]
    fn easing_names() {
        assert_eq!(Easing::parse("linear"), Some(Easing::Linear));
        assert_eq!(Easing::parse("easeInSine"), Some(Easing::Ease(EaseFamily::Sine, EaseDir::In)));
        assert_eq!(Easing::parse("easeOutBack"), Some(Easing::Ease(EaseFamily::Back, EaseDir::Out)));
        assert_eq!(Easing::parse("easeInOutBounce"), Some(Easing::Ease(EaseFamily::Bounce, EaseDir::InOut)));
        assert_eq!(Easing::parse("ease_in_out_quint"), Some(Easing::Ease(EaseFamily::Quint, EaseDir::InOut)));
        assert_eq!(Easing::parse("EASEINEXPO"), Some(Easing::Ease(EaseFamily::Expo, EaseDir::In)));
        assert_eq!(Easing::parse("easeInWobble"), None);
        assert_eq!(Easing::parse("nonsense"), None);
    }

    #[test]
    fn linear_keys_golden() {
        let c = ch(r#"{"0": [0, 0, 0], "1": [10, -4, 2], "3": [10, 4, 2]}"#);
        let at = |t| c.sample(t, &inp());
        assert_eq!(at(0.0), Vec3::ZERO);
        assert_eq!(at(0.5), Vec3::new(5.0, -2.0, 1.0));
        assert_eq!(at(1.0), Vec3::new(10.0, -4.0, 2.0));
        assert_eq!(at(2.0), Vec3::new(10.0, 0.0, 2.0));
        assert_eq!(at(3.0), Vec3::new(10.0, 4.0, 2.0));
        assert_eq!(at(9.0), Vec3::new(10.0, 4.0, 2.0)); // holds the last
        assert_eq!(at(-1.0), Vec3::ZERO); // holds the first
    }

    #[test]
    fn pre_post_makes_a_jump() {
        let c = ch(r#"{"0": [0,0,0], "1": {"pre": [10,0,0], "post": [0,20,0]}, "2": [0,20,0]}"#);
        let at = |t| c.sample(t, &inp());
        assert_eq!(at(0.5), Vec3::new(5.0, 0.0, 0.0));
        near(at(0.999).x, 9.99);
        assert_eq!(at(1.0), Vec3::new(0.0, 20.0, 0.0)); // at the key itself: post
        assert_eq!(at(1.5), Vec3::new(0.0, 20.0, 0.0));
    }

    #[test]
    fn step_edge_cases() {
        let c = ch(r#"{"0": {"post": [1,0,0], "lerp_mode": "step"}, "1": {"post": [2,0,0], "lerp_mode": "step"}, "2": [3,0,0]}"#);
        let at = |t| c.sample(t, &inp()).x;
        assert_eq!(at(0.0), 1.0);
        assert_eq!(at(0.999), 1.0);
        assert_eq!(at(1.0), 2.0); // the next key takes over exactly at its time
        assert_eq!(at(1.999), 2.0);
        assert_eq!(at(2.0), 3.0);
        // step into a key whose pre differs: the step ignores pre
        let c = ch(r#"{"0": {"post": [1,0,0], "lerp_mode": "step"}, "1": {"pre": [5,0,0], "post": [6,0,0]}}"#);
        assert_eq!(c.sample(0.5, &inp()).x, 1.0);
        assert_eq!(c.sample(1.0, &inp()).x, 6.0);
        // single key
        let c = ch(r#"{"2": [7,7,7]}"#);
        assert_eq!(c.sample(0.0, &inp()), Vec3::splat(7.0));
        assert_eq!(c.sample(5.0, &inp()), Vec3::splat(7.0));
    }

    #[test]
    fn catmullrom_golden() {
        // 0, 10, 0, 10 at t = 0..3; the segment 1..2 uses all four points.
        let c = ch(r#"{"0": [0,0,0], "1": {"post": [10,0,0], "lerp_mode": "catmullrom"}, "2": [0,0,0], "3": [10,0,0]}"#);
        let at = |t| c.sample(t, &inp()).x;
        near(at(1.0), 10.0);
        near(at(1.25), 8.4375);
        near(at(1.5), 5.0);
        near(at(1.75), 1.5625);
        near(at(2.0), 0.0);
        // The first segment has the segment start's catmullrom mode? No: key 0 is linear.
        near(at(0.5), 5.0);
    }

    #[test]
    fn catmullrom_ends_duplicate_the_endpoint() {
        // Two keys only, catmullrom: p0 = p1 and p3 = p2, a smooth ease: 0.5 * (2*0 + 0 + (0-0+40-10... ) computed
        // below by hand: p0=p1=0, p2=p3=10, s=0.5 -> 0.5*(0 + 10*0.5 + (0 -0 +40 -10)*0.25 + (0-0-30+10)*0.125)
        // = 0.5*(5 + 7.5 - 2.5) = 5.
        let c = ch(r#"{"0": {"post": [0,0,0], "lerp_mode": "catmullrom"}, "1": [10,0,0]}"#);
        near(c.sample(0.5, &inp()).x, 5.0);
        // s = 0.25: 0.5*(2.5 + 30*0.0625 + (-20)*0.015625) = 0.5*(2.5+1.875-0.3125) = 2.03125
        near(c.sample(0.25, &inp()).x, 2.03125);
    }

    #[test]
    fn bezier_golden() {
        // Flat handles at one third: time is linear, value is 10 * (3s^2 - 2s^3).
        let b = r#""bezier_right_time": [0.3333333,0.3333333,0.3333333], "bezier_right_value": [0,0,0]"#;
        let l = r#""bezier_left_time": [-0.3333333,-0.3333333,-0.3333333], "bezier_left_value": [0,0,0]"#;
        let c = ch(&format!(r#"{{"0": {{"post": [0,0,0], "lerp_mode": "bezier", {b}}}, "1": {{"post": [10,0,0], {l}}}}}"#));
        let at = |t| c.sample(t, &inp()).x;
        near(at(0.25), 1.5625);
        near(at(0.5), 5.0);
        near(at(0.75), 8.4375);
        // Value handles: 3 up on the right of key 0, 3 down on the left of key 1 -> y(0.25) = 2.40625.
        let b = r#""bezier_right_time": [0.3333333,0.3333333,0.3333333], "bezier_right_value": [3,3,3]"#;
        let l = r#""bezier_left_time": [-0.3333333,-0.3333333,-0.3333333], "bezier_left_value": [-3,-3,-3]"#;
        let c = ch(&format!(r#"{{"0": {{"post": [0,0,0], "lerp_mode": "bezier", {b}}}, "1": {{"post": [10,0,0], {l}}}}}"#));
        near(c.sample(0.25, &inp()).x, 2.40625);
        near(c.sample(0.5, &inp()).x, 5.0);
    }

    #[test]
    fn bezier_edge_cases() {
        // Handle times beyond the segment are clamped, so time stays monotonic and the curve stays finite.
        let b = r#""bezier_right_time": [5,5,5]"#;
        let l = r#""bezier_left_time": [-5,-5,-5]"#;
        let c = ch(&format!(r#"{{"0": {{"post": [0,0,0], "lerp_mode": "bezier", {b}}}, "1": {{"post": [10,0,0], {l}}}}}"#));
        let mut prev = -1.0;
        for i in 0..=20 {
            let v = c.sample(i as f32 / 20.0, &inp()).x;
            assert!((0.0..=10.0).contains(&v) && v >= prev - 1e-4);
            prev = v;
        }
        // Default handles (Blockbench: 0.1 s) on a 0.05 s segment are clamped to the segment.
        let c = ch(r#"{"0": {"post": [0,0,0], "lerp_mode": "bezier"}, "0.05": [1,0,0]}"#);
        assert!((c.sample(0.025, &inp()).x - 0.5).abs() < 0.01); // flat time curve at the middle: ill-conditioned
        // Exactly at both ends.
        assert_eq!(c.sample(0.0, &inp()).x, 0.0);
        assert_eq!(c.sample(0.05, &inp()).x, 1.0);
        // Missing handle fields use defaults and an eased shape differs from linear at the quarter point.
        let c = ch(r#"{"0": {"post": [0,0,0], "lerp_mode": "bezier"}, "1": [10,0,0]}"#);
        let v = c.sample(0.25, &inp()).x;
        assert!(v > 0.0 && v < 10.0);
    }

    #[test]
    fn easing_applies_to_linear_segments() {
        let c = ch(r#"{"0": {"post": [0,0,0], "easing": "easeInQuad"}, "1": [8,0,0]}"#);
        near(c.sample(0.5, &inp()).x, 2.0);
        near(c.sample(0.25, &inp()).x, 0.5);
    }

    #[test]
    fn expressions_in_keyframes_and_constants() {
        let p = parse(
            r#"{"animations":{"a":{"animation_length":2,"loop":true,"bones":{"b":{
                "rotation":["math.sin(query.anim_time * 90) * 10", 0, "query.synced('mode') * 2"],
                "scale": 2}}}}}"#,
        );
        let a = &p.animations[0];
        let b = &a.bones[0];
        let mut i = inp();
        i.set(Query::AnimTime, 1.0);
        i.synced[0] = 3.0;
        let r = b.rotation.as_ref().unwrap().sample(0.0, &i);
        near(r.x, -10.0); // rotation x is flipped on load
        near(r.z, 6.0);
        assert_eq!(b.scale.as_ref().unwrap().sample(0.0, &i), Vec3::splat(2.0));
    }

    #[test]
    fn bedrock_axes_are_converted_to_blockbench_space() {
        let p = parse(
            r#"{"animations":{"a":{"bones":{"b":{
                "position":{"0":[1,2,3],"1":{"post":["q.health",5,6],"lerp_mode":"bezier","bezier_right_value":[1,1,1]}},
                "rotation":[10,20,30],"scale":[1,2,3]}}}}}"#,
        );
        let b = &p.animations[0].bones[0];
        let mut i = inp();
        i.set(Query::Health, 4.0);
        assert_eq!(b.position.as_ref().unwrap().sample(0.0, &i), Vec3::new(-1.0, 2.0, 3.0));
        assert_eq!(b.position.as_ref().unwrap().sample(1.0, &i), Vec3::new(-4.0, 5.0, 6.0));
        let Channel::Keys(k) = b.position.as_ref().unwrap() else { panic!() };
        assert_eq!(k[0].bezier.right_value, Vec3::ZERO);
        assert_eq!(k[1].bezier.right_value, Vec3::new(-1.0, 1.0, 1.0));
        assert_eq!(b.rotation.as_ref().unwrap().sample(0.0, &i), Vec3::new(-10.0, -20.0, 30.0));
        assert_eq!(b.scale.as_ref().unwrap().sample(0.0, &i), Vec3::new(1.0, 2.0, 3.0));
    }

    #[test]
    fn loop_modes_and_sample_time() {
        let p = parse(
            r#"{"animations":{
              "l":{"loop":true,"animation_length":2,"bones":{"b":{"position":{"0":[0,0,0],"2":[2,0,0]}}}},
              "h":{"loop":"hold_on_last_frame","animation_length":2},
              "o":{"loop":false,"animation_length":2},
              "z":{"loop":false}
            }}"#,
        );
        let a = |n: usize| &p.animations[n];
        let mut i = inp();
        assert_eq!(a(0).sample_time(5.0, &mut i), Some(1.0));
        assert_eq!(a(0).sample_time(2.0, &mut i), Some(0.0));
        assert_eq!(a(1).sample_time(5.0, &mut i), Some(2.0));
        assert_eq!(a(2).sample_time(1.5, &mut i), Some(1.5));
        assert_eq!(a(2).sample_time(2.0, &mut i), None);
        assert_eq!(a(3).sample_time(9.0, &mut i), Some(0.0)); // zero length: constant pose
        assert_eq!(a(0).sample_time(f32::NAN, &mut i), Some(0.0));
        assert_eq!(a(0).sample_time(-3.0, &mut i), Some(0.0));
    }

    #[test]
    fn anim_time_update_and_blend_weight() {
        let p = parse(
            r#"{"animations":{"a":{"loop":true,"animation_length":4,"anim_time_update":"query.anim_time * 2",
                "blend_weight":"query.health / 10"}}}"#,
        );
        let a = &p.animations[0];
        let mut i = inp();
        assert_eq!(a.sample_time(1.5, &mut i), Some(3.0));
        assert_eq!(a.sample_time(2.5, &mut i), Some(1.0));
        i.set(Query::Health, 5.0);
        assert_eq!(a.weight(&i), 0.5);
        i.set(Query::Health, -5.0);
        assert_eq!(a.weight(&i), 0.0);
    }

    #[test]
    fn length_defaults_to_last_key() {
        let p = parse(r#"{"animations":{"a":{"bones":{"b":{"rotation":{"0":[0,0,0],"1.5":[1,0,0]}}}, "timeline":{"2.0":"x"}}}}"#);
        assert_eq!(p.animations[0].length, 2.0);
    }

    #[test]
    fn events_in_windows() {
        let p = parse(
            r#"{"animations":{
              "l":{"loop":true,"animation_length":1,"timeline":{"0":"start","0.5":["half","half2"]},
                   "sound_effects":{"0.25":{"effect":"step"}},"particle_effects":{"0.75":{"effect":"dust","locator":"foot"}}},
              "o":{"loop":false,"animation_length":1,"timeline":{"0":"a","1":"end","2":"late"}}
            }}"#,
        );
        let collect = |a: &Animation, c0: f32, c1: f32| {
            let mut v = Vec::new();
            a.emit_events(c0, c1, |e, k| v.push((e.name.clone(), k)));
            v
        };
        let l = &p.animations[0];
        let names = |v: Vec<(String, u32)>| v.into_iter().map(|(n, k)| format!("{n}#{k}")).collect::<Vec<_>>();
        assert_eq!(names(collect(l, 0.0, 0.3)), ["start#0", "step#0"]);
        assert_eq!(names(collect(l, 0.3, 0.6)), ["half#0", "half2#0"]);
        assert_eq!(names(collect(l, 0.6, 1.0)), ["dust#0"]);
        // Window edges are [c0, c1): no duplicates between consecutive windows, events exactly once per loop.
        assert_eq!(names(collect(l, 1.0, 1.1)), ["start#1"]);
        assert_eq!(collect(l, 0.0, 3.0).len(), 3 * 5);
        // Paused (no clock progress) or time going backwards: nothing.
        assert!(collect(l, 0.5, 0.5).is_empty());
        assert!(collect(l, 0.9, 0.2).is_empty());
        assert!(collect(l, f32::NAN, 1.0).is_empty());
        // A long stall is bounded.
        let mut n = 0;
        l.emit_events(0.0, 1.0e6, |_, _| n += 1);
        assert!(n <= 5 * (MAX_EVENT_LOOPS_PER_CALL as usize + 1));
        // A non-looping animation fires each event once, up to its length; the late one never fires.
        let o = &p.animations[1];
        assert_eq!(names(collect(o, 0.0, 10.0)), ["a#0", "end#0"]);
        assert_eq!(names(collect(o, 1.0, 1.5)), ["end#0"]);
        assert!(p.warnings.iter().any(|w| w.message.contains("late")));
        let ev = &l.events;
        assert_eq!(ev.iter().find(|e| e.name == "dust").unwrap().locator.as_deref(), Some("foot"));
        assert_eq!(ev.iter().find(|e| e.name == "step").unwrap().kind, EventKind::Sound);
    }

    #[test]
    fn unsupported_things_warn() {
        let p = parse(
            r#"{"format_version":"1.8.0","animations":{"a":{"bones":{"b":{
                "rotation":["math.hermite_blend(1)", 0, 0], "relative_to": {"rotation":"entity"}}},"weird":1}}}"#,
        );
        let w: Vec<String> = p.warnings.iter().map(|w| w.to_string()).collect();
        assert!(w.iter().any(|w| w.contains("hermite_blend") && w.contains("'a'")), "{w:?}");
        assert!(w.iter().any(|w| w.contains("relative_to")));
        assert!(w.iter().any(|w| w.contains("weird")));
        let r = p.animations[0].bones[0].rotation.as_ref().unwrap();
        assert_eq!(r.sample(0.0, &inp()), Vec3::ZERO);
    }

    #[test]
    fn errors_are_reported_with_context() {
        let e = parse_animation_file(r#"{"animations":{"a":{"bones":{"b":{"rotation":["1 +", 0, 0]}}}}}"#, &mut syms()).unwrap_err();
        assert_eq!(e.context, "a");
        assert_eq!(e.column, Some(4));
        let e = parse_animation_file(r#"{"animations":{"a":{"loop":"maybe"}}}"#, &mut syms()).unwrap_err();
        assert!(e.message.contains("loop"));
        let e = parse_animation_file(r#"{"animations":{"a":{"bones":{"b":{"rotation":[1,2]}}}}}"#, &mut syms()).unwrap_err();
        assert!(e.message.contains("3 values"));
        let e = parse_animation_file(r#"{"animations":{"a":{"bones":{"b":{"rotation":{"x":[1,2,3]}}}}}}"#, &mut syms()).unwrap_err();
        assert!(e.message.contains("time"));
        let e = parse_animation_file(
            r#"{"animations":{"a":{"bones":{"b":{"rotation":["query.synced('missing')",0,0]}}}}}"#,
            &mut syms(),
        )
        .unwrap_err();
        assert!(e.message.contains("missing"));
        assert!(parse_animation_file("{", &mut syms()).is_err());
        assert!(parse_animation_file("{}", &mut syms()).is_err());
    }

    #[test]
    fn file_order_is_kept() {
        let p = parse(r#"{"animations":{"zeta":{},"alpha":{},"mid":{}}}"#);
        let names: Vec<_> = p.animations.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["zeta", "alpha", "mid"]);
    }

    #[test]
    fn library_lookup() {
        let p = parse(r#"{"animations":{"animation.fox.walk":{},"animation.fox.idle":{},"animation.cat.idle":{}}}"#);
        let mut lib = AnimationLibrary::new();
        for a in p.animations {
            lib.add(a).unwrap();
        }
        assert_eq!(lib.find("animation.fox.walk"), Some(0));
        assert_eq!(lib.find("walk"), Some(0));
        assert_eq!(lib.find("idle"), None); // ambiguous
        assert_eq!(lib.find("fox.idle"), Some(1));
        assert_eq!(lib.find("run"), None);
        let mut lib = AnimationLibrary::new();
        for i in 0..MAX_ANIMATIONS {
            lib.add(Animation { name: format!("a{i}"), ..empty() }).unwrap();
        }
        assert!(lib.add(Animation { name: "extra".into(), ..empty() }).is_err());
        assert!(lib.add(Animation { name: "a3".into(), ..empty() }).is_ok()); // replace is fine
    }

    fn empty() -> Animation {
        Animation {
            name: String::new(),
            loop_mode: LoopMode::Once,
            length: 0.0,
            override_previous: false,
            anim_time_update: None,
            blend_weight: None,
            bones: vec![],
            events: vec![],
        }
    }

    #[test]
    fn bbmodel_conversion() {
        let v: Value = serde_json::from_str(
            r#"[{
              "name": "walk", "length": 1.0, "loop": "loop", "override": true,
              "animators": {
                "uuid-1": {"name": "arm", "type": "bone", "keyframes": [
                  {"channel": "rotation", "time": 0, "interpolation": "linear", "data_points": [{"x": "0", "y": "0", "z": "0"}]},
                  {"channel": "rotation", "time": 1, "interpolation": "catmullrom", "data_points": [{"x": "90", "y": "", "z": "math.sin(query.anim_time)"}]},
                  {"channel": "position", "time": 0.5, "interpolation": "step", "data_points": [{"x": 1, "y": 2, "z": 3}, {"x": 4, "y": 5, "z": 6}]},
                  {"channel": "scale", "time": 0, "interpolation": "bezier", "data_points": [{"x": "", "y": "", "z": ""}],
                   "bezier_left_time": [-0.2,-0.2,-0.2], "bezier_right_value": [0.5,0.5,0.5]}
                ]},
                "uuid-2": {"name": "Effects", "type": "effect", "keyframes": [
                  {"channel": "sound", "time": 0.5, "data_points": [{"effect": "thud", "locator": ""}]},
                  {"channel": "particle", "time": 0.25, "data_points": [{"effect": "dust", "locator": "foot"}]},
                  {"channel": "timeline", "time": 0.75, "data_points": [{"script": "hit"}]}
                ]}
              }
            },
            {"name": "idle", "loop": "hold", "length": 2, "animators": {}}]"#,
        )
        .unwrap();
        let p = animations_from_bbmodel(&v, &mut syms()).unwrap();
        assert_eq!(p.animations.len(), 2);
        let walk = &p.animations[0];
        assert_eq!((walk.loop_mode, walk.length, walk.override_previous), (LoopMode::Loop, 1.0, true));
        assert_eq!(p.animations[1].loop_mode, LoopMode::Hold);
        assert_eq!(walk.bones.len(), 1);
        let arm = &walk.bones[0];
        assert_eq!(arm.bone, "arm");
        let Channel::Keys(rot) = arm.rotation.as_ref().unwrap() else { panic!() };
        assert_eq!(rot[1].lerp, LerpMode::CatmullRom);
        let mut i = inp();
        i.set(Query::AnimTime, 90.0);
        near(rot[1].post.eval(&i).x, 90.0);
        assert_eq!(rot[1].post.eval(&i).y, 0.0); // empty = channel default
        near(rot[1].post.eval(&i).z, 1.0);
        let Channel::Keys(pos) = arm.position.as_ref().unwrap() else { panic!() };
        assert_eq!(pos[0].lerp, LerpMode::Step);
        assert_eq!(pos[0].pre.eval(&i), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(pos[0].post.eval(&i), Vec3::new(4.0, 5.0, 6.0));
        let Channel::Keys(sc) = arm.scale.as_ref().unwrap() else { panic!() };
        assert_eq!(sc[0].post.eval(&i), Vec3::ONE);
        assert_eq!(sc[0].bezier.left_time, Vec3::splat(-0.2));
        assert_eq!(sc[0].bezier.right_value, Vec3::splat(0.5));
        assert_eq!(sc[0].bezier.right_time, Vec3::splat(0.1)); // default
        let names: Vec<_> = walk.events.iter().map(|e| (e.time, e.kind, e.name.as_str(), e.locator.as_deref())).collect();
        assert_eq!(
            names,
            [
                (0.25, EventKind::Particle, "dust", Some("foot")),
                (0.5, EventKind::Sound, "thud", None),
                (0.75, EventKind::Timeline, "hit", None)
            ]
        );
    }

    #[test]
    fn bbmodel_conversion_errors() {
        let bad = |s: &str| animations_from_bbmodel(&serde_json::from_str(s).unwrap(), &mut syms());
        assert!(bad(r#"{}"#).is_err());
        assert!(bad(r#"[{"name":"a","loop":"forever"}]"#).is_err());
        assert!(bad(r#"[{"name":"a","animators":{"x":{"name":"b","keyframes":[{"channel":"rotation","data_points":[{}]}]}}}]"#).is_err());
        assert!(bad(r#"[{"name":"a","animators":{"x":{"name":"b","keyframes":[{"channel":"rotation","time":0,"data_points":[]}]}}}]"#).is_err());
        assert!(bad(r#"[{"name":"a","animators":{"x":{"name":"b","keyframes":[{"channel":"rotation","time":0,"data_points":[{"x":"1 +"}]}]}}}]"#).is_err());
    }
}
