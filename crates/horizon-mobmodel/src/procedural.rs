//! Procedural constraints and the script pose interface (FR-F4-MOBS.md sections 3.6, 4, 6.2; F4M-8).
//!
//! A [`ProceduralSet`] is the bound, immutable list of constraints of one mob type (shared by all mobs of the type);
//! a [`ProceduralState`] is the per-mob mutable part (spring particles, follow history, smoothed angles, scratch
//! matrices). [`ProceduralSet::evaluate`] runs after the animation system has produced the [`Pose`] and edits that pose
//! in place, in list order. [`apply_bone_writes`] (the `mob-pose` script hook) runs after it.
//!
//! No allocation happens per frame: everything is sized when the state is created.
//!
//! # Conventions
//!
//! * **Space**: model space, pixels (1/16 block), Blockbench axes (+X east/right, +Y up, -Z forward). All points the
//!   caller passes in (look-at targets, IK targets, `model_to_world`) are in this space. A bone's *pivot position*
//!   is where the bone's joint is in the posed model (`bone_matrices[i] * pivot`; it does not depend on the bone's own
//!   rotation). A bone's *frame* is the geometry frame (the axes the cubes were authored in, before the bone's rest
//!   rotation); `forward`/`axis` options of `look_at`/`aim`/`counter_rotate` are expressed in that frame.
//! * **What is edited**: only `Pose::bones[i].rotation` (constraints never touch translation or scale). A constraint
//!   that has `weight < 1` (or is fading) blends the rotation it computes with the rotation it found, by spherical
//!   interpolation.
//! * **Order**: constraints run in array order; every constraint sees the result of the ones before it (the matrices
//!   are updated after each edit). Put `spring` after anything that moves its bones, `follow` after its source.
//! * **Activation**: every constraint has an optional `when` expression (compiled with [`crate::expr`], evaluated with
//!   the per-frame [`ExprInputs`]) and a `fade` time in seconds (default 0.15; 0 = instant): the constraint's strength is
//!   `weight * activation`, activation moving linearly toward 1 (condition true) or 0 (false). The first frame of a
//!   mob starts at its target value (no fade-in at spawn). With activation 0 a constraint costs one expression
//!   evaluation. Synced enum values are plain numbers in expressions: the **index of the value in the type's `synced`
//!   `values` list**, so the fox's "sitting" is `q.synced('state') == 1` (`idle` 0, `sit` 1, `pounce` 2). The expression
//!   language has no enum-name literals.
//! * **Time**: `dt` seconds per frame, sanitised: NaN, infinite or negative become 0; values above [`MAX_DT`] (0.1 s)
//!   are clamped to it (a stalled frame loses time instead of exploding the simulation).
//!
//! # JSON (`model.procedural` of the type file, FR-F4-MOBS.md 6.1)
//!
//! An array of objects, `type` selects the kind. Unknown fields are errors. Common optional fields: `when`
//! (expression string), `fade` (seconds, 0..10), `weight` (0..1, default 1).
//!
//! ```text
//! { "type": "look_at", "bone": "head", "target": "view", "yaw": 60, "pitch": 40, "speed": 360,
//!   "forward": "-z", "up": "y", "offset": [0,0,0] }
//! { "type": "aim", "bone": "arm", "axis": [0,0,-1], "target": "target", "max_angle": 90, "speed": 360, "offset": [0,0,0] }
//! { "type": "two_bone_ik", "root": "body", "mid": "leg_fl", "tip": "leg_fl", "tip_offset": [0,-3,0],
//!   "target": {"input": 0}, "pole": {"dir": [0,0,-1]} }
//! { "type": "spring", "bones": ["tail1","tail2","tail3"], "stiffness": 60, "damping": 4, "gravity": 160,
//!   "max_angle": 60, "tip_length": 6, "tip_dir": [0,0,1] }
//! { "type": "follow", "bone": "ear_r", "source": "ear_l", "delay_ticks": 3 }
//! { "type": "counter_rotate", "bone": "head", "axis": [0,1,0], "up": [0,1,0] }
//! ```
//!
//! Targets (`look_at`, `aim`, `two_bone_ik`): `"view"`, `"target"`, `"owner"` (points the caller resolves into
//! [`ProceduralInputs::targets`], in model space; `None` means "not available"), `{"input": n}` (n in 0..5, caller
//! slot `3 + n`), `{"point": [x,y,z]}` (constant, model space), `{"locator": "name"}` (a model locator, posed with its
//! bone) or `{"bone": "name"}` (the bone's pivot, posed). `two_bone_ik` poles: `{"dir": [x,y,z]}` (model-space
//! direction), `{"point": [x,y,z]}` or `{"bone": "name"}` (a bone's posed pivot).
//!
//! * `look_at` yaws and pitches the bone toward the target, relative to the bone's animated orientation. `yaw` and
//!   `pitch` are symmetric limits in degrees (0..180), `speed` the maximum angular speed in degrees per second for each
//!   of the two angles (0 = unlimited). `forward` and `up` pick the geometry-frame axes (`"x"`, `"-x"`, `"y"`, ...,
//!   default `-z` and `y`); `offset` moves the eye point away from the pivot (geometry frame, pixels). Without a target
//!   the bone relaxes back to its animated orientation, at the same speed.
//! * `aim` turns an arbitrary geometry-frame `axis` of the bone toward the target by the shortest arc, limited to a
//!   cone of `max_angle` degrees (0..180) and to `speed` degrees per second.
//! * `two_bone_ik` bends `root`, `mid` and the end point (`tip` bone's pivot plus `tip_offset`, or `tip_locator`) so the
//!   end point reaches the target. `mid` must be a descendant of `root` and the tip bone a descendant of (or equal to)
//!   `mid`. Bone lengths are kept; a target out of reach gives the fully extended (or folded) limb pointing at it. The
//!   bend side follows `pole`, else the current bend. If the target is unavailable the constraint fades out using the
//!   last target it had.
//! * `spring` simulates the tip of every chain bone as a damped spring particle and swings the bone toward it (see
//!   [`SpringSpec`]). The chain `bones` must each be a descendant of the one before.
//! * `follow` copies the *local rotation* of `source` into `bone`, `delay_ticks` game ticks (0.05 s) late.
//! * `counter_rotate` swings the bone (minimal arc) until its `axis` points along the model-space direction `up`.
//!
//! # Limits
//!
//! At most [`limits::MAX_CONSTRAINTS`] constraints per type, [`limits::MAX_LOOK_AT`] `look_at`, [`limits::MAX_AIM`] `aim`,
//! [`limits::MAX_IK`] `two_bone_ik`, [`limits::MAX_SPRING`] `spring` ([`limits::MAX_SPRING_CHAIN`] bones per chain,
//! [`limits::MAX_SPRING_BONES`] in all), [`limits::MAX_FOLLOW`] `follow` (delay up to [`limits::MAX_FOLLOW_DELAY_TICKS`]
//! ticks), [`limits::MAX_COUNTER_ROTATE`] `counter_rotate`. Numbers must be finite and within the ranges named above.
//! Script bone writes: at most [`limits::MAX_BONE_WRITES`] per call.

use crate::expr::{compile, Expr, ExprInputs, Symbols};
use crate::ik::{
    clamp_to_cone, euler_deg_to_quat, move_toward, rotate_dir_toward, rotation_arc, safe_quat, slerp_short,
    solve_two_bone, try_normalize, AimFrame,
};
use crate::model::{Locator, ModelError};
use crate::skeleton::{BoneIndex, Pose, Skeleton};
use glam::{Mat4, Quat, Vec3};
use serde_json::{Map, Value};

/// Limits of the procedural system (documented in the module header).
pub mod limits {
    pub const MAX_CONSTRAINTS: usize = 64;
    pub const MAX_LOOK_AT: usize = 16;
    pub const MAX_AIM: usize = 16;
    pub const MAX_IK: usize = 8;
    pub const MAX_SPRING: usize = 16;
    pub const MAX_SPRING_CHAIN: usize = 16;
    /// Spring bones over all `spring` constraints of a type.
    pub const MAX_SPRING_BONES: usize = 64;
    pub const MAX_FOLLOW: usize = 16;
    pub const MAX_FOLLOW_DELAY_TICKS: u32 = 40;
    pub const MAX_COUNTER_ROTATE: usize = 16;
    /// `BoneWrite`s accepted per `apply_bone_writes` call; the rest are dropped and counted.
    pub const MAX_BONE_WRITES: usize = 1024;
    /// Largest absolute value accepted in a constraint file or a bone write (pixels, degrees, factors).
    pub const MAX_ABS_VALUE: f32 = 1.0e6;
    pub const MAX_STIFFNESS: f32 = 10_000.0;
    pub const MAX_DAMPING: f32 = 1_000.0;
    pub const MAX_GRAVITY: f32 = 100_000.0;
    pub const MAX_SPEED_DEG: f32 = 100_000.0;
}

/// Largest `dt` (seconds) one evaluation simulates.
pub const MAX_DT: f32 = 0.1;
/// Fixed step of the spring integration (seconds).
pub const SPRING_STEP: f32 = 1.0 / 120.0;
/// Seconds per game tick (the unit of `follow` delays).
pub const TICK_SECONDS: f32 = 0.05;
/// Number of target slots in [`ProceduralInputs::targets`].
pub const TARGET_SLOTS: usize = 8;
/// Passes of the two-bone solve per frame (see the IK branch of `evaluate`) and the distance that ends them early.
const IK_PASSES: usize = 4;
const IK_TOLERANCE: f32 = 1.0e-4;
/// Default fade time in seconds.
pub const DEFAULT_FADE: f32 = 0.15;

// ---------------------------------------------------------------------------------------------------------------
// Specification types
// ---------------------------------------------------------------------------------------------------------------

/// A caller-resolved target point, an index into [`ProceduralInputs::targets`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetSource {
    /// Slot 0: where the viewer (camera) is, for "look at the player".
    View,
    /// Slot 1: the mob's current attack or AI target.
    Target,
    /// Slot 2: the owner.
    Owner,
    /// Slots 3..8, `{"input": n}`.
    Custom(u8),
}

impl TargetSource {
    pub fn slot(self) -> usize {
        match self {
            TargetSource::View => 0,
            TargetSource::Target => 1,
            TargetSource::Owner => 2,
            TargetSource::Custom(n) => 3 + n as usize,
        }
    }
}

/// Where a constraint aims.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TargetSpec {
    /// A point supplied by the caller each frame (model space).
    Source(TargetSource),
    /// A constant model-space point.
    Point(Vec3),
    /// A model locator: `rest` is its rest position, posed with `bone`.
    Locator { bone: BoneIndex, rest: Vec3 },
    /// The posed pivot of a bone.
    Bone(BoneIndex),
}

/// The bend side of a two-bone limb.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PoleSpec {
    /// A model-space direction.
    Dir(Vec3),
    /// A model-space point.
    Point(Vec3),
    /// The posed pivot of a bone.
    Bone(BoneIndex),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LookAtSpec {
    pub bone: BoneIndex,
    pub target: TargetSpec,
    /// Symmetric yaw limit in degrees.
    pub yaw: f32,
    /// Symmetric pitch limit in degrees.
    pub pitch: f32,
    /// Maximum angular speed in degrees per second, per angle; 0 = unlimited.
    pub speed: f32,
    pub frame: AimFrame,
    /// Eye point relative to the bone's pivot, in the bone's geometry frame (pixels).
    pub offset: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AimSpec {
    pub bone: BoneIndex,
    pub target: TargetSpec,
    /// The geometry-frame axis that is turned toward the target (unit).
    pub axis: Vec3,
    /// Cone half angle in degrees.
    pub max_angle: f32,
    /// Degrees per second; 0 = unlimited.
    pub speed: f32,
    pub offset: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TwoBoneIkSpec {
    pub root: BoneIndex,
    pub mid: BoneIndex,
    /// The bone the end point belongs to (a descendant of `mid` or `mid` itself).
    pub tip: BoneIndex,
    /// The end point in rest model coordinates (the tip's pivot plus its offset, or a locator's rest position).
    pub tip_rest: Vec3,
    pub target: TargetSpec,
    pub pole: Option<PoleSpec>,
}

/// A chain of bones whose tips are damped spring particles.
///
/// Per chain bone the *tip* is the pivot of the next chain bone; the tip of the last bone is `tip_length` pixels from its
/// pivot along `tip_dir` (default: the direction of the previous bone, or straight down for a one-bone chain; with
/// `tip_length` of `None` the length of the previous bone, or 6 pixels for a one-bone chain). The particle is pulled to
/// where the animation puts the tip with `stiffness` (1/s^2: the spring's omega squared), slowed by `damping` (1/s), and
/// pushed by `gravity` (pixels/s^2 along world -Y of the [`ProceduralInputs::model_to_world`] frame). After each frame
/// the particle is projected onto the sphere of the bone's length around its pivot, so lengths never change, and the bone
/// swings toward it, never farther than `max_angle` degrees from the animated direction. Integration uses a fixed step of
/// [`SPRING_STEP`] with at most 16 steps per call, semi-implicit with exponential damping: stable for any `dt`
/// (clamped to [`MAX_DT`]) and stiffness up to [`limits::MAX_STIFFNESS`].
#[derive(Clone, Debug, PartialEq)]
pub struct SpringSpec {
    pub bones: Vec<BoneIndex>,
    pub stiffness: f32,
    pub damping: f32,
    pub gravity: f32,
    pub max_angle: f32,
    pub tip_length: Option<f32>,
    pub tip_dir: Option<Vec3>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FollowSpec {
    pub bone: BoneIndex,
    pub source: BoneIndex,
    pub delay_ticks: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CounterRotateSpec {
    pub bone: BoneIndex,
    /// Geometry-frame axis of the bone that should point along `up`.
    pub axis: Vec3,
    /// Model-space direction.
    pub up: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConstraintKind {
    LookAt(LookAtSpec),
    Aim(AimSpec),
    TwoBoneIk(TwoBoneIkSpec),
    Spring(SpringSpec),
    Follow(FollowSpec),
    CounterRotate(CounterRotateSpec),
}

impl ConstraintKind {
    pub fn name(&self) -> &'static str {
        match self {
            ConstraintKind::LookAt(_) => "look_at",
            ConstraintKind::Aim(_) => "aim",
            ConstraintKind::TwoBoneIk(_) => "two_bone_ik",
            ConstraintKind::Spring(_) => "spring",
            ConstraintKind::Follow(_) => "follow",
            ConstraintKind::CounterRotate(_) => "counter_rotate",
        }
    }
}

/// One constraint with its activation.
#[derive(Clone, Debug, PartialEq)]
pub struct Constraint {
    pub kind: ConstraintKind,
    /// Activation condition; `None` = always on.
    pub when: Option<Expr>,
    /// Fade time in seconds (0 = instant).
    pub fade: f32,
    /// Strength 0..1.
    pub weight: f32,
}

impl Constraint {
    pub fn new(kind: ConstraintKind) -> Self {
        Constraint { kind, when: None, fade: DEFAULT_FADE, weight: 1.0 }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The bound set
// ---------------------------------------------------------------------------------------------------------------

/// Per spring chain bone, derived at bind time.
#[derive(Clone, Copy, Debug, PartialEq)]
struct SpringTip {
    rest: Vec3,
    length: f32,
}

/// The constraints of one mob type bound to its skeleton. Immutable; share between mobs.
#[derive(Clone, Debug)]
pub struct ProceduralSet {
    constraints: Vec<Constraint>,
    n: usize,
    rest: Vec<Quat>,
    pivots: Vec<Vec3>,
    parents: Vec<Option<usize>>,
    /// Per constraint (empty for non-springs).
    spring_tips: Vec<Vec<SpringTip>>,
}

fn is_ancestor_or_self(parents: &[Option<usize>], ancestor: usize, mut node: usize) -> bool {
    loop {
        if node == ancestor {
            return true;
        }
        match parents[node] {
            Some(p) => node = p,
            None => return false,
        }
    }
}

fn berr(message: String) -> ModelError {
    ModelError { file: String::new(), path: String::new(), message }
}

impl ProceduralSet {
    /// Bind `constraints` to `skeleton`. Checks bone indices, ancestry, limits and numeric ranges; the error names no
    /// file (use [`parse_procedural`] for file/path errors).
    pub fn new(skeleton: &Skeleton, constraints: Vec<Constraint>) -> Result<ProceduralSet, ModelError> {
        let n = skeleton.len();
        let parents: Vec<Option<usize>> = skeleton.bones.iter().map(|b| b.parent.map(|p| p.index())).collect();
        for (i, p) in parents.iter().enumerate() {
            if let Some(p) = p {
                if *p >= i {
                    return Err(berr(format!("bone {i} has parent {p}: parents must come before children")));
                }
            }
        }
        let rest: Vec<Quat> = skeleton.bones.iter().map(|b| euler_deg_to_quat(b.rotation)).collect();
        let pivots: Vec<Vec3> = skeleton.bones.iter().map(|b| b.pivot).collect();
        let mut spring_tips = Vec::with_capacity(constraints.len());
        let mut counts = [0usize; 6];
        let mut spring_bones = 0usize;
        if constraints.len() > limits::MAX_CONSTRAINTS {
            return Err(berr(format!("{} constraints (at most {})", constraints.len(), limits::MAX_CONSTRAINTS)));
        }
        let bone_ok = |b: BoneIndex, what: &str| -> Result<usize, ModelError> {
            if b.index() < n {
                Ok(b.index())
            } else {
                Err(berr(format!("{what}: bone index {} out of range (skeleton has {n} bones)", b.0)))
            }
        };
        let check = |v: f32, what: &str, lo: f32, hi: f32| -> Result<(), ModelError> {
            if v.is_finite() && v >= lo && v <= hi {
                Ok(())
            } else {
                Err(berr(format!("{what} must be a number from {lo} to {hi}, got {v}")))
            }
        };
        let check_vec = |v: Vec3, what: &str| -> Result<(), ModelError> {
            if v.is_finite() && v.abs().max_element() <= limits::MAX_ABS_VALUE {
                Ok(())
            } else {
                Err(berr(format!("{what} must be finite and within +-{}", limits::MAX_ABS_VALUE)))
            }
        };
        let check_target = |t: &TargetSpec| -> Result<(), ModelError> {
            match t {
                TargetSpec::Source(TargetSource::Custom(k)) if *k >= (TARGET_SLOTS - 3) as u8 => {
                    Err(berr(format!("target input {k} out of range (0..{})", TARGET_SLOTS - 3)))
                }
                TargetSpec::Source(_) => Ok(()),
                TargetSpec::Point(p) => check_vec(*p, "target point"),
                TargetSpec::Locator { bone, rest } => {
                    bone_ok(*bone, "target locator")?;
                    check_vec(*rest, "target locator")
                }
                TargetSpec::Bone(b) => bone_ok(*b, "target bone").map(|_| ()),
            }
        };
        for c in &constraints {
            check(c.weight, "weight", 0.0, 1.0)?;
            check(c.fade, "fade", 0.0, 10.0)?;
            let mut tips = Vec::new();
            match &c.kind {
                ConstraintKind::LookAt(s) => {
                    counts[0] += 1;
                    bone_ok(s.bone, "look_at")?;
                    check_target(&s.target)?;
                    check(s.yaw, "look_at yaw", 0.0, 180.0)?;
                    check(s.pitch, "look_at pitch", 0.0, 180.0)?;
                    check(s.speed, "look_at speed", 0.0, limits::MAX_SPEED_DEG)?;
                    check_vec(s.offset, "look_at offset")?;
                }
                ConstraintKind::Aim(s) => {
                    counts[1] += 1;
                    bone_ok(s.bone, "aim")?;
                    check_target(&s.target)?;
                    check(s.max_angle, "aim max_angle", 0.0, 180.0)?;
                    check(s.speed, "aim speed", 0.0, limits::MAX_SPEED_DEG)?;
                    check_vec(s.offset, "aim offset")?;
                    if try_normalize(s.axis).is_none() {
                        return Err(berr("aim axis must not be zero".into()));
                    }
                }
                ConstraintKind::TwoBoneIk(s) => {
                    counts[2] += 1;
                    let r = bone_ok(s.root, "two_bone_ik root")?;
                    let m = bone_ok(s.mid, "two_bone_ik mid")?;
                    let t = bone_ok(s.tip, "two_bone_ik tip")?;
                    if r == m || !is_ancestor_or_self(&parents, r, m) {
                        return Err(berr("two_bone_ik: mid must be a descendant of root".into()));
                    }
                    if !is_ancestor_or_self(&parents, m, t) {
                        return Err(berr("two_bone_ik: tip must be mid or a descendant of mid".into()));
                    }
                    check_vec(s.tip_rest, "two_bone_ik tip point")?;
                    check_target(&s.target)?;
                    match s.pole {
                        Some(PoleSpec::Dir(v)) | Some(PoleSpec::Point(v)) => check_vec(v, "two_bone_ik pole")?,
                        Some(PoleSpec::Bone(b)) => {
                            bone_ok(b, "two_bone_ik pole")?;
                        }
                        None => {}
                    }
                }
                ConstraintKind::Spring(s) => {
                    counts[3] += 1;
                    if s.bones.is_empty() || s.bones.len() > limits::MAX_SPRING_CHAIN {
                        return Err(berr(format!(
                            "spring chain has {} bones (1 to {})",
                            s.bones.len(),
                            limits::MAX_SPRING_CHAIN
                        )));
                    }
                    spring_bones += s.bones.len();
                    check(s.stiffness, "spring stiffness", 0.0, limits::MAX_STIFFNESS)?;
                    check(s.damping, "spring damping", 0.0, limits::MAX_DAMPING)?;
                    check(s.gravity, "spring gravity", -limits::MAX_GRAVITY, limits::MAX_GRAVITY)?;
                    check(s.max_angle, "spring max_angle", 0.0, 180.0)?;
                    let mut idx = Vec::with_capacity(s.bones.len());
                    for b in &s.bones {
                        idx.push(bone_ok(*b, "spring")?);
                    }
                    for w in idx.windows(2) {
                        if w[0] == w[1] || !is_ancestor_or_self(&parents, w[0], w[1]) {
                            return Err(berr(format!(
                                "spring chain: bone {} is not a descendant of bone {}",
                                skeleton.bones[w[1]].name, skeleton.bones[w[0]].name
                            )));
                        }
                    }
                    let mut dir_prev: Option<Vec3> = None;
                    let mut len_prev: Option<f32> = None;
                    for k in 0..idx.len() {
                        let piv = pivots[idx[k]];
                        if k + 1 < idx.len() {
                            let next = pivots[idx[k + 1]];
                            let len = (next - piv).length();
                            if len < 0.01 {
                                return Err(berr(format!(
                                    "spring chain: bones {} and {} have the same pivot",
                                    skeleton.bones[idx[k]].name,
                                    skeleton.bones[idx[k + 1]].name
                                )));
                            }
                            tips.push(SpringTip { rest: next, length: len });
                            dir_prev = try_normalize(next - piv);
                            len_prev = Some(len);
                        } else {
                            let dir = match s.tip_dir {
                                Some(d) => try_normalize(d).ok_or_else(|| berr("spring tip_dir must not be zero".into()))?,
                                None => dir_prev.unwrap_or(Vec3::NEG_Y),
                            };
                            let len = s.tip_length.or(len_prev).unwrap_or(6.0);
                            check(len, "spring tip_length", 0.01, limits::MAX_ABS_VALUE)?;
                            tips.push(SpringTip { rest: piv + dir * len, length: len });
                        }
                    }
                }
                ConstraintKind::Follow(s) => {
                    counts[4] += 1;
                    let b = bone_ok(s.bone, "follow bone")?;
                    let src = bone_ok(s.source, "follow source")?;
                    if b == src {
                        return Err(berr("follow: bone and source are the same".into()));
                    }
                    if s.delay_ticks > limits::MAX_FOLLOW_DELAY_TICKS {
                        return Err(berr(format!(
                            "follow delay_ticks {} (at most {})",
                            s.delay_ticks,
                            limits::MAX_FOLLOW_DELAY_TICKS
                        )));
                    }
                }
                ConstraintKind::CounterRotate(s) => {
                    counts[5] += 1;
                    bone_ok(s.bone, "counter_rotate")?;
                    if try_normalize(s.axis).is_none() || try_normalize(s.up).is_none() {
                        return Err(berr("counter_rotate axis and up must not be zero".into()));
                    }
                    check_vec(s.axis, "counter_rotate axis")?;
                    check_vec(s.up, "counter_rotate up")?;
                }
            }
            spring_tips.push(tips);
        }
        let caps = [
            ("look_at", limits::MAX_LOOK_AT),
            ("aim", limits::MAX_AIM),
            ("two_bone_ik", limits::MAX_IK),
            ("spring", limits::MAX_SPRING),
            ("follow", limits::MAX_FOLLOW),
            ("counter_rotate", limits::MAX_COUNTER_ROTATE),
        ];
        for (i, (name, cap)) in caps.iter().enumerate() {
            if counts[i] > *cap {
                return Err(berr(format!("{} {name} constraints (at most {cap})", counts[i])));
            }
        }
        if spring_bones > limits::MAX_SPRING_BONES {
            return Err(berr(format!("{spring_bones} spring bones in all (at most {})", limits::MAX_SPRING_BONES)));
        }
        Ok(ProceduralSet { constraints, n, rest, pivots, parents, spring_tips })
    }

    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    pub fn is_empty(&self) -> bool {
        self.constraints.is_empty()
    }

    /// Number of skeleton bones the set was bound to.
    pub fn bone_count(&self) -> usize {
        self.n
    }
}

// ---------------------------------------------------------------------------------------------------------------
// State and inputs
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum SlotData {
    None,
    Look { yaw: f32, pitch: f32 },
    Aim { dir: Vec3 },
    Ik { last_target: Option<Vec3> },
    Spring { acc: f32, p: Vec<Vec3>, v: Vec<Vec3> },
    Follow { acc: f32, primed: bool, prev: Quat, hist: Vec<Quat> },
}

#[derive(Clone, Debug)]
struct Slot {
    activation: f32,
    /// The activation has been set once (the first frame starts at its goal).
    init: bool,
    /// Spring particles hold a valid state.
    ready: bool,
    data: SlotData,
}

/// Scratch matrices and rotations, kept consistent with the pose while constraints edit it.
#[derive(Clone, Debug)]
struct Work {
    mats: Vec<Mat4>,
    wrot: Vec<Quat>,
    stamp: Vec<u32>,
    cur: u32,
}

/// Per-mob mutable state of a [`ProceduralSet`]. Created once; evaluation never allocates.
#[derive(Clone, Debug)]
pub struct ProceduralState {
    n: usize,
    work: Work,
    slots: Vec<Slot>,
}

impl ProceduralState {
    pub fn new(set: &ProceduralSet) -> ProceduralState {
        let n = set.n;
        let slots = set
            .constraints
            .iter()
            .enumerate()
            .map(|(i, c)| Slot {
                activation: 0.0,
                init: false,
                ready: false,
                data: match &c.kind {
                    ConstraintKind::LookAt(_) => SlotData::Look { yaw: 0.0, pitch: 0.0 },
                    ConstraintKind::Aim(s) => SlotData::Aim { dir: try_normalize(s.axis).unwrap_or(Vec3::NEG_Z) },
                    ConstraintKind::TwoBoneIk(_) => SlotData::Ik { last_target: None },
                    ConstraintKind::Spring(_) => {
                        let k = set.spring_tips[i].len();
                        SlotData::Spring { acc: 0.0, p: vec![Vec3::ZERO; k], v: vec![Vec3::ZERO; k] }
                    }
                    ConstraintKind::Follow(f) => {
                        SlotData::Follow { acc: 0.0, primed: false, prev: Quat::IDENTITY, hist: vec![Quat::IDENTITY; f.delay_ticks as usize + 2] }
                    }
                    ConstraintKind::CounterRotate(_) => SlotData::None,
                },
            })
            .collect();
        ProceduralState {
            n,
            work: Work { mats: vec![Mat4::IDENTITY; n], wrot: vec![Quat::IDENTITY; n], stamp: vec![0; n], cur: 0 },
            slots,
        }
    }

    /// Forget everything (springs restart from the animated pose, histories refill, activations jump).
    pub fn reset(&mut self) {
        for s in &mut self.slots {
            s.init = false;
            s.ready = false;
            s.activation = 0.0;
            match &mut s.data {
                SlotData::Look { yaw, pitch } => {
                    *yaw = 0.0;
                    *pitch = 0.0;
                }
                SlotData::Aim { .. } => {}
                SlotData::Ik { last_target } => *last_target = None,
                SlotData::Spring { acc, .. } => *acc = 0.0,
                SlotData::Follow { acc, primed, .. } => {
                    *acc = 0.0;
                    *primed = false;
                }
                SlotData::None => {}
            }
        }
    }
}

/// Per-frame inputs of [`ProceduralSet::evaluate`].
#[derive(Clone, Copy, Debug)]
pub struct ProceduralInputs<'a> {
    /// Seconds since the previous evaluation of this mob (sanitised, see the module header).
    pub dt: f32,
    /// For `when` conditions.
    pub expr: &'a ExprInputs,
    /// Target points in model space, by [`TargetSource::slot`]; `None` = unavailable.
    pub targets: [Option<Vec3>; TARGET_SLOTS],
    /// Maps model space (pixels) to the frame the springs simulate in (also pixels, gravity along its -Y). Identity
    /// makes springs react only to the animation; pass the mob's real transform times 16 to get inertia when it moves
    /// and turns. A singular or non-finite matrix is replaced by identity.
    pub model_to_world: Mat4,
}

impl<'a> ProceduralInputs<'a> {
    pub fn new(expr: &'a ExprInputs, dt: f32) -> Self {
        ProceduralInputs { dt, expr, targets: [None; TARGET_SLOTS], model_to_world: Mat4::IDENTITY }
    }

    pub fn with_target(mut self, source: TargetSource, point: Vec3) -> Self {
        if let Some(t) = self.targets.get_mut(source.slot()) {
            *t = Some(point);
        }
        self
    }
}

/// What one [`ProceduralSet::evaluate`] call did (diagnostics; nothing here is an error).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EvalReport {
    /// Constraints that edited the pose.
    pub applied: u32,
    /// Constraints skipped because their activation is 0.
    pub inactive: u32,
    /// Active aiming constraints whose target was unavailable.
    pub missing_target: u32,
    /// Spring particles reset because they became non-finite.
    pub non_finite_resets: u32,
    /// The pose has fewer bones than the set: nothing was done.
    pub pose_mismatch: bool,
    /// The state does not belong to the set: nothing was done.
    pub state_mismatch: bool,
}

/// `dt` as the evaluation uses it.
pub fn sanitize_dt(dt: f32) -> f32 {
    if dt.is_finite() && dt > 0.0 {
        dt.min(MAX_DT)
    } else {
        0.0
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------------------------------------------

impl Work {
    fn compute_one(&mut self, set: &ProceduralSet, pose: &Pose, j: usize) {
        let t = pose.bones[j];
        let local_rot = safe_quat(set.rest[j] * safe_quat(t.rotation));
        let piv = set.pivots[j];
        let (pm, pw) = match set.parents[j] {
            Some(p) => (self.mats[p], self.wrot[p]),
            None => (Mat4::IDENTITY, Quat::IDENTITY),
        };
        self.mats[j] = pm
            * Mat4::from_translation(piv + t.translation)
            * Mat4::from_quat(local_rot)
            * Mat4::from_scale(t.scale)
            * Mat4::from_translation(-piv);
        self.wrot[j] = safe_quat(pw * local_rot);
    }

    fn recompute_all(&mut self, set: &ProceduralSet, pose: &Pose) {
        for j in 0..set.n {
            self.compute_one(set, pose, j);
        }
    }

    /// Recompute `b` and everything below it.
    fn recompute_from(&mut self, set: &ProceduralSet, pose: &Pose, b: usize) {
        self.cur = self.cur.wrapping_add(1);
        if self.cur == 0 {
            self.stamp.iter_mut().for_each(|s| *s = 0);
            self.cur = 1;
        }
        self.compute_one(set, pose, b);
        self.stamp[b] = self.cur;
        for j in b + 1..set.n {
            if let Some(p) = set.parents[j] {
                if self.stamp[p] == self.cur {
                    self.compute_one(set, pose, j);
                    self.stamp[j] = self.cur;
                }
            }
        }
    }

    fn pivot_pos(&self, set: &ProceduralSet, b: usize) -> Vec3 {
        self.mats[b].transform_point3(set.pivots[b])
    }

    fn set_local(&mut self, set: &ProceduralSet, pose: &mut Pose, b: usize, q: Quat) {
        if !q.is_finite() {
            return;
        }
        pose.bones[b].rotation = safe_quat(q);
        self.recompute_from(set, pose, b);
    }

    /// Make bone `b`'s world rotation `w`.
    fn set_world(&mut self, set: &ProceduralSet, pose: &mut Pose, b: usize, w: Quat) {
        let pw = set.parents[b].map_or(Quat::IDENTITY, |p| self.wrot[p]);
        let local_total = pw.conjugate() * safe_quat(w);
        self.set_local(set, pose, b, set.rest[b].conjugate() * local_total);
    }

    fn resolve_target(&self, set: &ProceduralSet, spec: &TargetSpec, inp: &ProceduralInputs) -> Option<Vec3> {
        let p = match spec {
            TargetSpec::Source(s) => inp.targets.get(s.slot()).copied().flatten()?,
            TargetSpec::Point(p) => *p,
            TargetSpec::Locator { bone, rest } => self.mats[bone.index()].transform_point3(*rest),
            TargetSpec::Bone(b) => self.pivot_pos(set, b.index()),
        };
        p.is_finite().then_some(p)
    }
}

fn step_activation(slot: &mut Slot, want: bool, fade: f32, dt: f32) {
    let goal = if want { 1.0 } else { 0.0 };
    if !slot.init {
        slot.activation = goal;
        slot.init = true;
    } else if fade <= 0.0 {
        slot.activation = goal;
    } else {
        slot.activation = move_toward(slot.activation, goal, dt / fade);
    }
}

fn frame_matrices(m: Mat4) -> (Mat4, Mat4) {
    let det = m.determinant();
    if m.is_finite() && det.is_finite() && det.abs() > 1.0e-12 {
        let inv = m.inverse();
        if inv.is_finite() {
            return (m, inv);
        }
    }
    (Mat4::IDENTITY, Mat4::IDENTITY)
}

impl ProceduralSet {
    /// Apply every constraint to `pose` (which holds the animated pose) in order.
    pub fn evaluate(&self, pose: &mut Pose, state: &mut ProceduralState, inp: &ProceduralInputs) -> EvalReport {
        let mut rep = EvalReport::default();
        if self.constraints.is_empty() {
            return rep;
        }
        if state.n != self.n || state.slots.len() != self.constraints.len() {
            rep.state_mismatch = true;
            return rep;
        }
        if pose.bones.len() < self.n {
            rep.pose_mismatch = true;
            return rep;
        }
        let dt = sanitize_dt(inp.dt);
        let (m2w, w2m) = frame_matrices(inp.model_to_world);
        let ProceduralState { work, slots, .. } = state;
        work.recompute_all(self, pose);
        for (i, c) in self.constraints.iter().enumerate() {
            let slot = &mut slots[i];
            let cond = c.when.as_ref().is_none_or(|e| e.eval_bool(inp.expr));
            // Targets: aiming constraints relax without one; IK fades out.
            match &c.kind {
                ConstraintKind::LookAt(s) => {
                    step_activation(slot, cond, c.fade, dt);
                    let w = c.weight * slot.activation;
                    if slot.activation <= 0.0 {
                        rep.inactive += 1;
                        continue;
                    }
                    let target = work.resolve_target(self, &s.target, inp);
                    if target.is_none() {
                        rep.missing_target += 1;
                    }
                    let SlotData::Look { yaw, pitch } = &mut slot.data else { continue };
                    let b = s.bone.index();
                    let wb = work.wrot[b];
                    let eye = work.pivot_pos(self, b) + wb * s.offset;
                    let (mut dy, mut dp) = (0.0, 0.0);
                    if let Some(t) = target {
                        let local = wb.conjugate() * (t - eye);
                        if try_normalize(local).is_some() {
                            let (y, p) = s.frame.angles_deg(local);
                            dy = y.clamp(-s.yaw, s.yaw);
                            dp = p.clamp(-s.pitch, s.pitch);
                        } else {
                            dy = *yaw;
                            dp = *pitch;
                        }
                    }
                    let step = if s.speed > 0.0 { s.speed * dt } else { f32::INFINITY };
                    *yaw = move_toward(*yaw, dy, step);
                    *pitch = move_toward(*pitch, dp, step);
                    let q = slerp_short(Quat::IDENTITY, s.frame.rotation_deg(*yaw, *pitch), w);
                    let old = pose.bones[b].rotation;
                    work.set_local(self, pose, b, safe_quat(old) * q);
                    rep.applied += 1;
                }
                ConstraintKind::Aim(s) => {
                    step_activation(slot, cond, c.fade, dt);
                    let w = c.weight * slot.activation;
                    if slot.activation <= 0.0 {
                        rep.inactive += 1;
                        continue;
                    }
                    let target = work.resolve_target(self, &s.target, inp);
                    if target.is_none() {
                        rep.missing_target += 1;
                    }
                    let SlotData::Aim { dir } = &mut slot.data else { continue };
                    let axis = try_normalize(s.axis).unwrap_or(Vec3::NEG_Z);
                    let b = s.bone.index();
                    let wb = work.wrot[b];
                    let eye = work.pivot_pos(self, b) + wb * s.offset;
                    let desired = match target.and_then(|t| try_normalize(wb.conjugate() * (t - eye))) {
                        Some(l) => clamp_to_cone(axis, l, s.max_angle),
                        None => axis,
                    };
                    let step = if s.speed > 0.0 { s.speed * dt } else { 180.0 };
                    let moved = rotate_dir_toward(*dir, desired, step);
                    *dir = clamp_to_cone(axis, moved, s.max_angle);
                    let q = slerp_short(Quat::IDENTITY, rotation_arc(axis, *dir), w);
                    let old = pose.bones[b].rotation;
                    work.set_local(self, pose, b, safe_quat(old) * q);
                    rep.applied += 1;
                }
                ConstraintKind::TwoBoneIk(s) => {
                    let target = work.resolve_target(self, &s.target, inp);
                    let SlotData::Ik { last_target } = &mut slot.data else { continue };
                    if target.is_some() {
                        *last_target = target;
                    }
                    step_activation(slot, cond && target.is_some(), c.fade, dt);
                    let w = c.weight * slot.activation;
                    let SlotData::Ik { last_target } = &slot.data else { continue };
                    if slot.activation <= 0.0 {
                        rep.inactive += 1;
                        continue;
                    }
                    let Some(tgt) = target.or(*last_target) else {
                        rep.missing_target += 1;
                        continue;
                    };
                    let (r, m, t) = (s.root.index(), s.mid.index(), s.tip.index());
                    let old_r = pose.bones[r].rotation;
                    let old_m = pose.bones[m].rotation;
                    // One pass is exact when the limb is rigid (no non-uniform scale on the bones or their parents);
                    // a few more passes absorb the error that a squashed parent adds.
                    for iter in 0..IK_PASSES {
                        let a = work.pivot_pos(self, r);
                        let bpos = work.pivot_pos(self, m);
                        let cpos = work.mats[t].transform_point3(s.tip_rest);
                        let pole = s.pole.map(|p| match p {
                            PoleSpec::Dir(d) => d,
                            PoleSpec::Point(q) => q - a,
                            PoleSpec::Bone(b) => work.pivot_pos(self, b.index()) - a,
                        });
                        let Some(sol) = solve_two_bone(a, bpos, cpos, tgt, pole) else { break };
                        if iter > 0 && (cpos - sol.end).length() < IK_TOLERANCE {
                            break;
                        }
                        let r1 = rotation_arc(bpos - a, sol.mid - a);
                        let new_w = r1 * work.wrot[r];
                        work.set_world(self, pose, r, new_w);
                        let b1 = work.pivot_pos(self, m);
                        let c1 = work.mats[t].transform_point3(s.tip_rest);
                        let q2 = rotation_arc(c1 - b1, sol.end - b1);
                        let new_w = q2 * work.wrot[m];
                        work.set_world(self, pose, m, new_w);
                    }
                    if w < 1.0 {
                        let nr = pose.bones[r].rotation;
                        let nm = pose.bones[m].rotation;
                        pose.bones[r].rotation = slerp_short(old_r, nr, w);
                        pose.bones[m].rotation = slerp_short(old_m, nm, w);
                        work.recompute_from(self, pose, r);
                    }
                    rep.applied += 1;
                }
                ConstraintKind::Spring(s) => {
                    step_activation(slot, cond, c.fade, dt);
                    let w = c.weight * slot.activation;
                    let SlotData::Spring { acc, p, v } = &mut slot.data else { continue };
                    if slot.activation <= 0.0 {
                        slot.ready = false; // restart from the animated pose on the next activation
                        rep.inactive += 1;
                        continue;
                    }
                    let first = !slot.ready || p.iter().chain(v.iter()).any(|x| !x.is_finite());
                    let steps = if first {
                        *acc = 0.0;
                        0
                    } else {
                        *acc += dt;
                        let n = ((*acc / SPRING_STEP) as usize).min(16);
                        *acc = (*acc - n as f32 * SPRING_STEP).clamp(0.0, SPRING_STEP);
                        n
                    };
                    let k = s.stiffness.clamp(0.0, limits::MAX_STIFFNESS);
                    let decay = (-s.damping.clamp(0.0, limits::MAX_DAMPING) * SPRING_STEP).exp();
                    let g = Vec3::new(0.0, -s.gravity.clamp(-limits::MAX_GRAVITY, limits::MAX_GRAVITY), 0.0);
                    for (j, bone) in s.bones.iter().enumerate() {
                        let b = bone.index();
                        let tip = self.spring_tips[i][j];
                        let pivot = work.pivot_pos(self, b);
                        let tip_anim = work.mats[b].transform_point3(tip.rest);
                        let Some(cur) = try_normalize(tip_anim - pivot) else { continue };
                        let tw = m2w.transform_point3(tip_anim);
                        if first || !p[j].is_finite() || !v[j].is_finite() {
                            if !first {
                                rep.non_finite_resets += 1;
                            }
                            p[j] = tw;
                            v[j] = Vec3::ZERO;
                        }
                        // A particle far from its rest tip (teleport) is pulled back at once.
                        let reach = 2.0 * tip.length;
                        let off = p[j] - tw;
                        if off.length() > reach {
                            p[j] = tw + off.normalize_or_zero() * reach;
                            v[j] = Vec3::ZERO;
                        }
                        for _ in 0..steps {
                            let acc_v = (tw - p[j]) * k + g;
                            v[j] = (v[j] + acc_v * SPRING_STEP) * decay;
                            p[j] += v[j] * SPRING_STEP;
                        }
                        if !p[j].is_finite() || !v[j].is_finite() {
                            rep.non_finite_resets += 1;
                            p[j] = tw;
                            v[j] = Vec3::ZERO;
                        }
                        let pm = w2m.transform_point3(p[j]);
                        let desired = match try_normalize(pm - pivot) {
                            Some(d) => clamp_to_cone(cur, d, s.max_angle),
                            None => cur,
                        };
                        let delta = slerp_short(Quat::IDENTITY, rotation_arc(cur, desired), w);
                        let new_w = delta * work.wrot[b];
                        work.set_world(self, pose, b, new_w);
                        // The particle lives on the sphere around the pivot (the bone's length never changes).
                        p[j] = m2w.transform_point3(pivot + desired * tip.length);
                    }
                    slot.ready = true;
                    rep.applied += 1;
                }
                ConstraintKind::Follow(s) => {
                    step_activation(slot, cond, c.fade, dt);
                    let w = c.weight * slot.activation;
                    let SlotData::Follow { acc, primed, prev, hist } = &mut slot.data else { continue };
                    let src = safe_quat(pose.bones[s.source.index()].rotation);
                    if !*primed {
                        hist.iter_mut().for_each(|h| *h = src);
                        *prev = src;
                        *acc = 0.0;
                        *primed = true;
                    }
                    // Tick boundaries that fall inside this frame sample the source by interpolating between the
                    // previous frame's value and this one's, so a frame length that is not a tick multiple adds no lag.
                    let mut t_into = TICK_SECONDS - *acc;
                    *acc += dt;
                    let mut ticks = 0;
                    while *acc >= TICK_SECONDS && ticks < hist.len() {
                        *acc -= TICK_SECONDS;
                        let f = if dt > 0.0 { (t_into / dt).clamp(0.0, 1.0) } else { 1.0 };
                        hist.rotate_right(1);
                        hist[0] = slerp_short(*prev, src, f);
                        t_into += TICK_SECONDS;
                        ticks += 1;
                    }
                    *prev = src;
                    *acc = acc.min(TICK_SECONDS * 0.999);
                    if slot.activation <= 0.0 {
                        rep.inactive += 1;
                        continue;
                    }
                    let d = s.delay_ticks as usize;
                    let delayed = if d == 0 {
                        src
                    } else {
                        let u = d as f32 - *acc / TICK_SECONDS;
                        let i0 = (u.floor().max(0.0) as usize).min(d);
                        slerp_short(hist[i0], hist[i0 + 1], u - i0 as f32)
                    };
                    let b = s.bone.index();
                    let own = pose.bones[b].rotation;
                    work.set_local(self, pose, b, slerp_short(own, delayed, w));
                    rep.applied += 1;
                }
                ConstraintKind::CounterRotate(s) => {
                    step_activation(slot, cond, c.fade, dt);
                    let w = c.weight * slot.activation;
                    if slot.activation <= 0.0 {
                        rep.inactive += 1;
                        continue;
                    }
                    let b = s.bone.index();
                    let wb = work.wrot[b];
                    let cur = wb * try_normalize(s.axis).unwrap_or(Vec3::Y);
                    let delta = slerp_short(Quat::IDENTITY, rotation_arc(cur, s.up), w);
                    work.set_world(self, pose, b, delta * wb);
                    rep.applied += 1;
                }
            }
        }
        rep
    }

    /// Model-space joint positions of the posed skeleton (`bone_matrices[i] * pivot`), for tools and tests.
    pub fn pivot_positions(&self, pose: &Pose) -> Vec<Vec3> {
        let mut w = Work {
            mats: vec![Mat4::IDENTITY; self.n],
            wrot: vec![Quat::IDENTITY; self.n],
            stamp: vec![0; self.n],
            cur: 0,
        };
        if pose.bones.len() < self.n {
            return Vec::new();
        }
        w.recompute_all(self, pose);
        (0..self.n).map(|b| w.pivot_pos(self, b)).collect()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// JSON parsing
// ---------------------------------------------------------------------------------------------------------------

struct Parser<'a> {
    file: &'a str,
    skeleton: &'a Skeleton,
    locators: &'a [Locator],
    syms: &'a mut Symbols,
}

type PResult<T> = Result<T, ModelError>;

impl Parser<'_> {
    fn err(&self, path: &str, message: impl Into<String>) -> ModelError {
        ModelError { file: self.file.to_string(), path: path.to_string(), message: message.into() }
    }

    fn object<'v>(&self, v: &'v Value, path: &str) -> PResult<&'v Map<String, Value>> {
        v.as_object().ok_or_else(|| self.err(path, "expected an object"))
    }

    fn check_keys(&self, m: &Map<String, Value>, allowed: &[&str], path: &str) -> PResult<()> {
        for k in m.keys() {
            if !allowed.contains(&k.as_str()) {
                return Err(self.err(&format!("{path}.{k}"), format!("unknown field; allowed: {}", allowed.join(", "))));
            }
        }
        Ok(())
    }

    fn num(&self, v: &Value, path: &str, lo: f32, hi: f32) -> PResult<f32> {
        let x = v.as_f64().ok_or_else(|| self.err(path, "expected a number"))?;
        if !x.is_finite() || x < lo as f64 || x > hi as f64 {
            return Err(self.err(path, format!("{x} is out of range ({lo} to {hi})")));
        }
        Ok(x as f32)
    }

    fn opt_num(&self, m: &Map<String, Value>, key: &str, path: &str, lo: f32, hi: f32, default: f32) -> PResult<f32> {
        match m.get(key) {
            None => Ok(default),
            Some(v) => self.num(v, &format!("{path}.{key}"), lo, hi),
        }
    }

    fn vec3(&self, v: &Value, path: &str) -> PResult<Vec3> {
        let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| self.err(path, "expected [x, y, z]"))?;
        let mut out = [0.0f32; 3];
        for (i, x) in a.iter().enumerate() {
            out[i] = self.num(x, &format!("{path}[{i}]"), -limits::MAX_ABS_VALUE, limits::MAX_ABS_VALUE)?;
        }
        Ok(Vec3::from(out))
    }

    fn opt_vec3(&self, m: &Map<String, Value>, key: &str, path: &str) -> PResult<Option<Vec3>> {
        m.get(key).map(|v| self.vec3(v, &format!("{path}.{key}"))).transpose()
    }

    /// An axis: `"x"`, `"-y"`, `"+z"` or `[x, y, z]` (non-zero).
    fn axis(&self, v: &Value, path: &str) -> PResult<Vec3> {
        if let Some(s) = v.as_str() {
            return match s.to_ascii_lowercase().as_str() {
                "x" | "+x" => Ok(Vec3::X),
                "-x" => Ok(Vec3::NEG_X),
                "y" | "+y" => Ok(Vec3::Y),
                "-y" => Ok(Vec3::NEG_Y),
                "z" | "+z" => Ok(Vec3::Z),
                "-z" => Ok(Vec3::NEG_Z),
                _ => Err(self.err(path, format!("unknown axis '{s}' (x, -x, y, -y, z, -z or [x, y, z])"))),
            };
        }
        let a = self.vec3(v, path)?;
        if try_normalize(a).is_none() {
            return Err(self.err(path, "axis must not be zero"));
        }
        Ok(a)
    }

    fn string<'v>(&self, v: &'v Value, path: &str) -> PResult<&'v str> {
        v.as_str().ok_or_else(|| self.err(path, "expected a string"))
    }

    fn bone(&self, v: &Value, path: &str) -> PResult<BoneIndex> {
        let name = self.string(v, path)?;
        self.skeleton.find(name).ok_or_else(|| {
            let names: Vec<&str> = self.skeleton.bones.iter().take(24).map(|b| b.name.as_str()).collect();
            self.err(path, format!("unknown bone '{name}' (bones: {})", names.join(", ")))
        })
    }

    fn req<'v>(&self, m: &'v Map<String, Value>, key: &str, path: &str) -> PResult<&'v Value> {
        m.get(key).ok_or_else(|| self.err(path, format!("missing field '{key}'")))
    }

    fn req_bone(&self, m: &Map<String, Value>, key: &str, path: &str) -> PResult<BoneIndex> {
        self.bone(self.req(m, key, path)?, &format!("{path}.{key}"))
    }

    fn target(&self, v: &Value, path: &str) -> PResult<TargetSpec> {
        if let Some(s) = v.as_str() {
            return match s {
                "view" => Ok(TargetSpec::Source(TargetSource::View)),
                "target" => Ok(TargetSpec::Source(TargetSource::Target)),
                "owner" => Ok(TargetSpec::Source(TargetSource::Owner)),
                _ => Err(self.err(
                    path,
                    format!(
                        "unknown target '{s}' (\"view\", \"target\", \"owner\", {{\"input\": n}}, {{\"point\": [..]}}, {{\"locator\": name}}, {{\"bone\": name}})"
                    ),
                )),
            };
        }
        let m = self.object(v, path)?;
        if m.len() != 1 {
            return Err(self.err(path, "expected exactly one of input, point, locator, bone"));
        }
        let (k, val) = m.iter().next().expect("one entry");
        let p = format!("{path}.{k}");
        match k.as_str() {
            "input" => {
                let n = self.num(val, &p, 0.0, (TARGET_SLOTS - 3 - 1) as f32)?;
                if n.fract() != 0.0 {
                    return Err(self.err(&p, "expected an integer"));
                }
                Ok(TargetSpec::Source(TargetSource::Custom(n as u8)))
            }
            "point" => Ok(TargetSpec::Point(self.vec3(val, &p)?)),
            "locator" => {
                let name = self.string(val, &p)?;
                let loc = self.locators.iter().find(|l| l.name == name).ok_or_else(|| {
                    let names: Vec<&str> = self.locators.iter().map(|l| l.name.as_str()).collect();
                    self.err(&p, format!("unknown locator '{name}' (locators: {})", names.join(", ")))
                })?;
                Ok(TargetSpec::Locator { bone: loc.bone, rest: loc.offset })
            }
            "bone" => Ok(TargetSpec::Bone(self.bone(val, &p)?)),
            _ => Err(self.err(&p, "unknown target kind (input, point, locator, bone)")),
        }
    }

    fn pole(&self, v: &Value, path: &str) -> PResult<PoleSpec> {
        let m = self.object(v, path)?;
        if m.len() != 1 {
            return Err(self.err(path, "expected exactly one of dir, point, bone"));
        }
        let (k, val) = m.iter().next().expect("one entry");
        let p = format!("{path}.{k}");
        match k.as_str() {
            "dir" => {
                let d = self.vec3(val, &p)?;
                if try_normalize(d).is_none() {
                    return Err(self.err(&p, "pole direction must not be zero"));
                }
                Ok(PoleSpec::Dir(d))
            }
            "point" => Ok(PoleSpec::Point(self.vec3(val, &p)?)),
            "bone" => Ok(PoleSpec::Bone(self.bone(val, &p)?)),
            _ => Err(self.err(&p, "unknown pole kind (dir, point, bone)")),
        }
    }

    fn constraint(&mut self, v: &Value, path: &str) -> PResult<Constraint> {
        let m = self.object(v, path)?;
        let ty = self.string(self.req(m, "type", path)?, &format!("{path}.type"))?;
        const COMMON: [&str; 5] = ["type", "when", "fade", "weight", "comment"];
        let kind_keys: &[&str] = match ty {
            "look_at" => &["bone", "target", "yaw", "pitch", "speed", "forward", "up", "offset"],
            "aim" => &["bone", "target", "axis", "max_angle", "speed", "offset"],
            "two_bone_ik" => &["root", "mid", "tip", "tip_offset", "tip_locator", "target", "pole"],
            "spring" => &["bones", "stiffness", "damping", "gravity", "max_angle", "tip_length", "tip_dir"],
            "follow" => &["bone", "source", "delay_ticks"],
            "counter_rotate" => &["bone", "axis", "up"],
            other => {
                return Err(self.err(
                    &format!("{path}.type"),
                    format!("unknown constraint type '{other}' (look_at, aim, two_bone_ik, spring, follow, counter_rotate)"),
                ))
            }
        };
        let allowed: Vec<&str> = COMMON.iter().chain(kind_keys.iter()).copied().collect();
        self.check_keys(m, &allowed, path)?;
        let weight = self.opt_num(m, "weight", path, 0.0, 1.0, 1.0)?;
        let fade = self.opt_num(m, "fade", path, 0.0, 10.0, DEFAULT_FADE)?;
        let when = match m.get("when") {
            None => None,
            Some(w) => {
                let p = format!("{path}.when");
                let src = self.string(w, &p)?;
                Some(compile(src, self.syms).map_err(|e| self.err(&p, format!("expression: {e}")))?)
            }
        };
        let target_or = |this: &Self, default: TargetSpec| -> PResult<TargetSpec> {
            match m.get("target") {
                None => Ok(default),
                Some(t) => this.target(t, &format!("{path}.target")),
            }
        };
        let kind = match ty {
            "look_at" => {
                let forward = match m.get("forward") {
                    Some(f) => self.axis(f, &format!("{path}.forward"))?,
                    None => Vec3::NEG_Z,
                };
                let up = match m.get("up") {
                    Some(f) => self.axis(f, &format!("{path}.up"))?,
                    None => Vec3::Y,
                };
                let frame = AimFrame::new(forward, up)
                    .ok_or_else(|| self.err(path, "forward and up must not be parallel"))?;
                ConstraintKind::LookAt(LookAtSpec {
                    bone: self.req_bone(m, "bone", path)?,
                    target: target_or(self, TargetSpec::Source(TargetSource::View))?,
                    yaw: self.opt_num(m, "yaw", path, 0.0, 180.0, 60.0)?,
                    pitch: self.opt_num(m, "pitch", path, 0.0, 180.0, 40.0)?,
                    speed: self.opt_num(m, "speed", path, 0.0, limits::MAX_SPEED_DEG, 360.0)?,
                    frame,
                    offset: self.opt_vec3(m, "offset", path)?.unwrap_or(Vec3::ZERO),
                })
            }
            "aim" => ConstraintKind::Aim(AimSpec {
                bone: self.req_bone(m, "bone", path)?,
                target: target_or(self, TargetSpec::Source(TargetSource::Target))?,
                axis: match m.get("axis") {
                    Some(a) => try_normalize(self.axis(a, &format!("{path}.axis"))?).unwrap_or(Vec3::NEG_Z),
                    None => Vec3::NEG_Z,
                },
                max_angle: self.opt_num(m, "max_angle", path, 0.0, 180.0, 90.0)?,
                speed: self.opt_num(m, "speed", path, 0.0, limits::MAX_SPEED_DEG, 360.0)?,
                offset: self.opt_vec3(m, "offset", path)?.unwrap_or(Vec3::ZERO),
            }),
            "two_bone_ik" => {
                let root = self.req_bone(m, "root", path)?;
                let mid = self.req_bone(m, "mid", path)?;
                let (tip, tip_rest) = match (m.get("tip"), m.get("tip_locator")) {
                    (Some(_), Some(_)) => {
                        return Err(self.err(path, "give either 'tip' (with optional 'tip_offset') or 'tip_locator', not both"))
                    }
                    (None, None) => return Err(self.err(path, "missing field 'tip' (or 'tip_locator')")),
                    (Some(t), None) => {
                        let bone = self.bone(t, &format!("{path}.tip"))?;
                        let off = self.opt_vec3(m, "tip_offset", path)?.unwrap_or(Vec3::ZERO);
                        (bone, self.skeleton.bones[bone.index()].pivot + off)
                    }
                    (None, Some(l)) => {
                        if m.contains_key("tip_offset") {
                            return Err(self.err(&format!("{path}.tip_offset"), "tip_offset belongs to 'tip', not 'tip_locator'"));
                        }
                        let lp = format!("{path}.tip_locator");
                        let name = self.string(l, &lp)?;
                        let loc = self.locators.iter().find(|x| x.name == name).ok_or_else(|| {
                            let names: Vec<&str> = self.locators.iter().map(|x| x.name.as_str()).collect();
                            self.err(&lp, format!("unknown locator '{name}' (locators: {})", names.join(", ")))
                        })?;
                        (loc.bone, loc.offset)
                    }
                };
                ConstraintKind::TwoBoneIk(TwoBoneIkSpec {
                    root,
                    mid,
                    tip,
                    tip_rest,
                    target: self.target(self.req(m, "target", path)?, &format!("{path}.target"))?,
                    pole: m.get("pole").map(|p| self.pole(p, &format!("{path}.pole"))).transpose()?,
                })
            }
            "spring" => {
                let bp = format!("{path}.bones");
                let arr = self
                    .req(m, "bones", path)?
                    .as_array()
                    .ok_or_else(|| self.err(&bp, "expected an array of bone names"))?;
                if arr.is_empty() || arr.len() > limits::MAX_SPRING_CHAIN {
                    return Err(self.err(&bp, format!("{} bones in the chain (1 to {})", arr.len(), limits::MAX_SPRING_CHAIN)));
                }
                let bones = arr
                    .iter()
                    .enumerate()
                    .map(|(i, b)| self.bone(b, &format!("{bp}[{i}]")))
                    .collect::<PResult<Vec<_>>>()?;
                ConstraintKind::Spring(SpringSpec {
                    bones,
                    stiffness: self.opt_num(m, "stiffness", path, 0.0, limits::MAX_STIFFNESS, 60.0)?,
                    damping: self.opt_num(m, "damping", path, 0.0, limits::MAX_DAMPING, 4.0)?,
                    gravity: self.opt_num(m, "gravity", path, -limits::MAX_GRAVITY, limits::MAX_GRAVITY, 0.0)?,
                    max_angle: self.opt_num(m, "max_angle", path, 0.0, 180.0, 90.0)?,
                    tip_length: match m.get("tip_length") {
                        Some(v) => Some(self.num(v, &format!("{path}.tip_length"), 0.01, limits::MAX_ABS_VALUE)?),
                        None => None,
                    },
                    tip_dir: match m.get("tip_dir") {
                        Some(v) => Some(self.axis(v, &format!("{path}.tip_dir"))?),
                        None => None,
                    },
                })
            }
            "follow" => {
                let d = self.opt_num(m, "delay_ticks", path, 0.0, limits::MAX_FOLLOW_DELAY_TICKS as f32, 0.0)?;
                if d.fract() != 0.0 {
                    return Err(self.err(&format!("{path}.delay_ticks"), "expected a whole number of ticks"));
                }
                ConstraintKind::Follow(FollowSpec {
                    bone: self.req_bone(m, "bone", path)?,
                    source: self.req_bone(m, "source", path)?,
                    delay_ticks: d as u32,
                })
            }
            _ => ConstraintKind::CounterRotate(CounterRotateSpec {
                bone: self.req_bone(m, "bone", path)?,
                axis: match m.get("axis") {
                    Some(a) => self.axis(a, &format!("{path}.axis"))?,
                    None => Vec3::Y,
                },
                up: match m.get("up") {
                    Some(a) => self.axis(a, &format!("{path}.up"))?,
                    None => Vec3::Y,
                },
            }),
        };
        Ok(Constraint { kind, when, fade, weight })
    }
}

/// Parse the `procedural` array of a type file (`value` is the array) and bind it to `skeleton`.
///
/// `file` and `path` (for example `"model.procedural"`) prefix the error messages: `file: path[2].yaw: message`.
/// `locators` come from the loaded model; `syms` is the type's expression symbol table (its synced values), which
/// `when` conditions are compiled against. Every problem is a hard error (unknown names and fields included).
pub fn parse_procedural(
    file: &str,
    path: &str,
    value: &Value,
    skeleton: &Skeleton,
    locators: &[Locator],
    syms: &mut Symbols,
) -> Result<ProceduralSet, ModelError> {
    let mut p = Parser { file, skeleton, locators, syms };
    let arr = value.as_array().ok_or_else(|| p.err(path, "expected an array of constraints"))?;
    if arr.len() > limits::MAX_CONSTRAINTS {
        return Err(p.err(path, format!("{} constraints (at most {})", arr.len(), limits::MAX_CONSTRAINTS)));
    }
    let mut constraints = Vec::with_capacity(arr.len());
    let mut kinds: Vec<&'static str> = Vec::new();
    for (i, v) in arr.iter().enumerate() {
        let c = p.constraint(v, &format!("{path}[{i}]"))?;
        kinds.push(c.kind.name());
        constraints.push(c);
    }
    let caps = [
        ("look_at", limits::MAX_LOOK_AT),
        ("aim", limits::MAX_AIM),
        ("two_bone_ik", limits::MAX_IK),
        ("spring", limits::MAX_SPRING),
        ("follow", limits::MAX_FOLLOW),
        ("counter_rotate", limits::MAX_COUNTER_ROTATE),
    ];
    for (name, cap) in caps {
        let count = kinds.iter().filter(|k| **k == name).count();
        if count > cap {
            return Err(p.err(path, format!("{count} {name} constraints (at most {cap})")));
        }
    }
    // Remaining cross-checks (ancestry, spring totals) give errors that name the offending constraint.
    for (i, c) in constraints.iter().enumerate() {
        ProceduralSet::new(skeleton, vec![c.clone()]).map_err(|e| p.err(&format!("{path}[{i}]"), e.message))?;
    }
    ProceduralSet::new(skeleton, constraints).map_err(|e| p.err(path, e.message))
}

/// Like [`parse_procedural`] from JSON text holding the array (error path root is `procedural`).
pub fn parse_procedural_str(
    file: &str,
    json: &str,
    skeleton: &Skeleton,
    locators: &[Locator],
    syms: &mut Symbols,
) -> Result<ProceduralSet, ModelError> {
    let v: Value = serde_json::from_str(json).map_err(|e| ModelError {
        file: file.to_string(),
        path: String::new(),
        message: format!("invalid JSON: {e}"),
    })?;
    parse_procedural(file, "procedural", &v, skeleton, locators, syms)
}

// ---------------------------------------------------------------------------------------------------------------
// The script pose interface (`mob-pose`)
// ---------------------------------------------------------------------------------------------------------------

/// How a [`BoneWrite`] combines with the pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteMode {
    /// Translation adds, rotation multiplies on the right (`old * delta`, delta in the bone's local frame), scale
    /// multiplies.
    Additive,
    /// The given channels replace the pose's.
    Absolute,
}

/// A rotation in a [`BoneWrite`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum WriteRotation {
    /// Degrees about X, Y, Z, X applied first (the order of the animation system).
    EulerDegrees(Vec3),
    /// Any non-zero finite quaternion (it is normalised).
    Quat(Quat),
}

/// One bone's change from a script. Unset channels are left alone. A write with any invalid value is rejected as a
/// whole.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneWrite {
    /// Bone index, resolved from names at load (`bone-index(type, name)`).
    pub bone: u32,
    pub mode: WriteMode,
    /// Pixels.
    pub translation: Option<Vec3>,
    pub rotation: Option<WriteRotation>,
    /// Factor.
    pub scale: Option<Vec3>,
    /// Show or hide the bone (not part of [`Pose`]; see [`BoneVisibility`]).
    pub visible: Option<bool>,
}

impl BoneWrite {
    pub fn new(bone: u32, mode: WriteMode) -> Self {
        BoneWrite { bone, mode, translation: None, rotation: None, scale: None, visible: None }
    }
}

/// Per-mob bone visibility set by scripts (all visible at creation). A bitset sized once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoneVisibility {
    words: Vec<u64>,
    len: usize,
}

impl BoneVisibility {
    pub fn new(bones: usize) -> Self {
        BoneVisibility { words: vec![0; bones.div_ceil(64)], len: bones }
    }

    /// Everything visible again.
    pub fn reset(&mut self) {
        self.words.iter_mut().for_each(|w| *w = 0);
    }

    /// Out-of-range bones are reported visible.
    pub fn is_visible(&self, bone: usize) -> bool {
        bone >= self.len || self.words[bone / 64] & (1 << (bone % 64)) == 0
    }

    pub fn set_visible(&mut self, bone: usize, visible: bool) {
        if bone < self.len {
            if visible {
                self.words[bone / 64] &= !(1 << (bone % 64));
            } else {
                self.words[bone / 64] |= 1 << (bone % 64);
            }
        }
    }
}

/// What [`apply_bone_writes`] did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WriteReport {
    pub applied: u32,
    /// The bone index does not exist (the whole write is ignored).
    pub ignored_unknown: u32,
    /// A NaN, an infinity, an out-of-range value or a zero quaternion (the whole write is rejected).
    pub rejected_invalid: u32,
    /// Writes beyond [`limits::MAX_BONE_WRITES`].
    pub dropped_over_limit: u32,
    /// Visibility values that could not be stored because no [`BoneVisibility`] was given.
    pub visibility_ignored: u32,
}

fn finite_vec(v: Vec3) -> bool {
    v.is_finite() && v.abs().max_element() <= limits::MAX_ABS_VALUE
}

fn write_valid(w: &BoneWrite) -> bool {
    w.translation.is_none_or(finite_vec)
        && w.scale.is_none_or(finite_vec)
        && match w.rotation {
            None => true,
            Some(WriteRotation::EulerDegrees(e)) => finite_vec(e),
            Some(WriteRotation::Quat(q)) => q.is_finite() && q.length_squared() > 1.0e-12 && q.length_squared().is_finite(),
        }
}

/// Apply script writes to `pose`, in order, after the constraints. Unknown bones are ignored and counted, invalid
/// values reject their write; visibility values are dropped (counted). Never panics, never allocates.
pub fn apply_bone_writes(pose: &mut Pose, writes: &[BoneWrite]) -> WriteReport {
    apply_inner(pose, None, writes)
}

/// [`apply_bone_writes`] that also stores `visible` values in `visibility`.
pub fn apply_bone_writes_with_visibility(
    pose: &mut Pose,
    visibility: &mut BoneVisibility,
    writes: &[BoneWrite],
) -> WriteReport {
    apply_inner(pose, Some(visibility), writes)
}

fn apply_inner(pose: &mut Pose, mut vis: Option<&mut BoneVisibility>, writes: &[BoneWrite]) -> WriteReport {
    let mut rep = WriteReport::default();
    for (i, w) in writes.iter().enumerate() {
        if i >= limits::MAX_BONE_WRITES {
            rep.dropped_over_limit = (writes.len() - i) as u32;
            break;
        }
        let b = w.bone as usize;
        if b >= pose.bones.len() {
            rep.ignored_unknown += 1;
            continue;
        }
        if !write_valid(w) {
            rep.rejected_invalid += 1;
            continue;
        }
        let t = &mut pose.bones[b];
        match w.mode {
            WriteMode::Additive => {
                if let Some(v) = w.translation {
                    t.translation += v;
                }
                if let Some(r) = w.rotation {
                    let d = match r {
                        WriteRotation::EulerDegrees(e) => euler_deg_to_quat(e),
                        WriteRotation::Quat(q) => safe_quat(q),
                    };
                    t.rotation = safe_quat(safe_quat(t.rotation) * d);
                }
                if let Some(v) = w.scale {
                    t.scale *= v;
                }
            }
            WriteMode::Absolute => {
                if let Some(v) = w.translation {
                    t.translation = v;
                }
                if let Some(r) = w.rotation {
                    t.rotation = match r {
                        WriteRotation::EulerDegrees(e) => euler_deg_to_quat(e),
                        WriteRotation::Quat(q) => safe_quat(q),
                    };
                }
                if let Some(v) = w.scale {
                    t.scale = v;
                }
            }
        }
        if let Some(v) = w.visible {
            match vis.as_deref_mut() {
                Some(bv) => bv.set_visible(b, v),
                None => rep.visibility_ignored += 1,
            }
        }
        rep.applied += 1;
    }
    rep
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skeleton::Bone;

    fn skel() -> Skeleton {
        let b = |n: &str, p: Option<u16>, piv: [f32; 3]| Bone {
            name: n.into(),
            parent: p.map(BoneIndex),
            pivot: Vec3::from(piv),
            rotation: Vec3::ZERO,
        };
        Skeleton { bones: vec![b("a", None, [0.0, 10.0, 0.0]), b("b", Some(0), [0.0, 6.0, 0.0]), b("c", Some(1), [0.0, 2.0, 0.0])] }
    }

    #[test]
    fn work_matches_bone_matrices() {
        let s = skel();
        let set = ProceduralSet::new(&s, vec![Constraint::new(ConstraintKind::Follow(FollowSpec {
            bone: BoneIndex(2),
            source: BoneIndex(1),
            delay_ticks: 0,
        }))])
        .unwrap();
        let mut pose = Pose::rest(&s);
        pose.bones[0].rotation = euler_deg_to_quat(Vec3::new(10.0, 20.0, 30.0));
        pose.bones[1].translation = Vec3::new(1.0, 2.0, 3.0);
        pose.bones[1].rotation = euler_deg_to_quat(Vec3::new(-40.0, 5.0, 0.0));
        pose.bones[2].scale = Vec3::new(1.0, 2.0, 1.0);
        let mut w = Work { mats: vec![Mat4::IDENTITY; 3], wrot: vec![Quat::IDENTITY; 3], stamp: vec![0; 3], cur: 0 };
        w.recompute_all(&set, &pose);
        let expect = crate::skeleton::bone_matrices(&s, &pose);
        for i in 0..3 {
            assert!(w.mats[i].abs_diff_eq(expect[i], 1e-4), "bone {i}");
        }
    }

    #[test]
    fn visibility_bitset() {
        let mut v = BoneVisibility::new(70);
        assert!(v.is_visible(69) && v.is_visible(500));
        v.set_visible(69, false);
        assert!(!v.is_visible(69) && v.is_visible(68));
        v.reset();
        assert!(v.is_visible(69));
    }
}
