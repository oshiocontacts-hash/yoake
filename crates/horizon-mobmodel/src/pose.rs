//! Pose evaluation: animations, controllers and layers to one [`Pose`] per frame, server-side timeline events and
//! sparse evaluation (FR-F4-MOBS.md section 3.6, F4M-2).
//!
//! Evaluation model:
//! * Every animation produces, per bone, optional rotation (Euler degrees), position (pixels) and scale (factor)
//!   channels. They are accumulated in that "Euler form" and only turned into a quaternion at the very end.
//! * Inside one state the animations stack in list order. A normal animation adds (rotation and position add scaled by
//!   its weight, scale multiplies as `1 + w * (s - 1)`); an animation with `override_previous_animation` instead
//!   replaces what was accumulated on the channels it animates (a lerp by its weight, weight capped at 1). The weight
//!   is `blend_weight` times the state entry's `weight`.
//! * During a state cross-fade the previous and the current state's stacks are each evaluated, then lerped (a channel
//!   one side does not animate counts as the rest value on that side).
//! * One-shot trigger animations are applied on top of their layer's result.
//! * Layers combine in order into the final pose, see [`crate::controller`] for `override` and `add`.
//! * The rotation order is the one of [`crate::skeleton::bone_matrices`]: `Quat::from_euler(ZYX, z, y, x)`, X applied
//!   first.

use crate::anim::{Animation, AnimationLibrary, ChannelKind, LoadError, Warning};
use crate::controller::{Controller, ControllerState, LayerMode, StateRun};
use crate::expr::{ExprInputs, Query};
use crate::skeleton::{bone_matrices, BoneIndex, BoneTransform, Pose, Skeleton};
use glam::{EulerRot, Mat4, Quat, Vec3};

/// Rotation in degrees `(x, y, z)` to a quaternion, X applied first.
pub fn euler_degrees_to_quat(rot: Vec3) -> Quat {
    Quat::from_euler(EulerRot::ZYX, rot.z.to_radians(), rot.y.to_radians(), rot.x.to_radians())
}

const ROT: u8 = 1;
const POS: u8 = 2;
const SCALE: u8 = 4;

/// One bone's accumulated channels.
#[derive(Clone, Copy, Debug)]
struct Slot {
    rot: Vec3,
    pos: Vec3,
    scale: Vec3,
    /// Which channels were written.
    mask: u8,
}

impl Slot {
    const REST: Slot = Slot { rot: Vec3::ZERO, pos: Vec3::ZERO, scale: Vec3::ONE, mask: 0 };

    fn channel(&mut self, kind: ChannelKind) -> (&mut Vec3, u8) {
        match kind {
            ChannelKind::Rotation => (&mut self.rot, ROT),
            ChannelKind::Position => (&mut self.pos, POS),
            ChannelKind::Scale => (&mut self.scale, SCALE),
        }
    }
}

/// Reusable buffers for [`Rig::evaluate`] (keep one per thread to avoid allocating per frame).
#[derive(Default)]
pub struct Scratch {
    cur: Vec<Slot>,
    prev: Vec<Slot>,
    fin: Vec<Slot>,
}

impl Scratch {
    fn prepare(&mut self, n: usize) {
        for v in [&mut self.cur, &mut self.prev, &mut self.fin] {
            v.clear();
            v.resize(n, Slot::REST);
        }
    }
}

/// A one-shot animation started by a trigger (synced as `(trigger_id, start_tick, speed)`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriggerPlay {
    pub layer: u16,
    /// Index into the layer's `triggers` list.
    pub trigger: u16,
    /// Seconds since the trigger started, speed already applied: the animation's clock.
    pub elapsed: f32,
}

/// Per-mob controller state: one entry per layer.
#[derive(Clone, Debug, PartialEq)]
pub struct RigState {
    pub layers: Vec<ControllerState>,
}

/// An event that fired during [`Rig::update`] or [`Rig::trigger_events`].
#[derive(Clone, Copy, Debug)]
pub struct RigEvent<'a> {
    pub layer: usize,
    /// Index into the animation library.
    pub animation: usize,
    pub event: &'a crate::anim::AnimEvent,
    pub loop_index: u32,
}

/// A skeleton with its animations and controller layers, bound by bone and animation names.
#[derive(Clone, Debug)]
pub struct Rig {
    pub skeleton: Skeleton,
    animations: AnimationLibrary,
    controllers: Vec<Controller>,
    /// Per animation, per `BoneAnim`: the skeleton bone, if the name exists.
    anim_bones: Vec<Vec<Option<BoneIndex>>>,
    /// Per layer: the bones it may drive (`None` = all).
    masks: Vec<Option<Vec<bool>>>,
}

impl Rig {
    /// Bind names. Errors: a state or trigger names an animation that does not exist. Warnings: an animation animates
    /// a bone the skeleton does not have, a mask names an unknown bone.
    pub fn new(
        skeleton: Skeleton,
        animations: AnimationLibrary,
        mut controllers: Vec<Controller>,
    ) -> Result<(Rig, Vec<Warning>), LoadError> {
        let mut warnings = Vec::new();
        for c in &mut controllers {
            for s in &mut c.states {
                for a in &mut s.animations {
                    a.index = animations.find(&a.name).ok_or_else(|| LoadError {
                        context: c.name.clone(),
                        message: format!("state '{}' uses unknown animation '{}'", s.name, a.name),
                        column: None,
                    })?;
                }
            }
            for t in &mut c.triggers {
                t.index = animations.find(&t.animation).ok_or_else(|| LoadError {
                    context: c.name.clone(),
                    message: format!("trigger '{}' uses unknown animation '{}'", t.name, t.animation),
                    column: None,
                })?;
            }
        }
        let mut anim_bones = Vec::new();
        for a in animations.iter() {
            let mut v = Vec::new();
            for b in &a.bones {
                let idx = skeleton.find(&b.bone);
                if idx.is_none() {
                    warnings.push(Warning {
                        context: a.name.clone(),
                        message: format!("animates bone '{}' which the model does not have (ignored)", b.bone),
                    });
                }
                v.push(idx);
            }
            anim_bones.push(v);
        }
        let mut masks = Vec::new();
        for c in &controllers {
            if c.bones_mask.is_empty() {
                masks.push(None);
                continue;
            }
            let mut m = vec![false; skeleton.len()];
            for name in &c.bones_mask {
                match skeleton.find(name) {
                    Some(i) => m[i.index()] = true,
                    None => warnings.push(Warning {
                        context: c.name.clone(),
                        message: format!("bones_mask names unknown bone '{name}' (ignored)"),
                    }),
                }
            }
            if c.include_children {
                for (i, b) in skeleton.bones.iter().enumerate() {
                    if let Some(p) = b.parent {
                        if m[p.index()] {
                            m[i] = true;
                        }
                    }
                }
            }
            masks.push(Some(m));
        }
        Ok((Rig { skeleton, animations, controllers, anim_bones, masks }, warnings))
    }

    pub fn animations(&self) -> &AnimationLibrary {
        &self.animations
    }

    pub fn controllers(&self) -> &[Controller] {
        &self.controllers
    }

    pub fn initial_state(&self) -> RigState {
        RigState { layers: self.controllers.iter().map(Controller::initial_state).collect() }
    }

    /// `needed` for sparse evaluation: the given bones and all their ancestors.
    pub fn bone_closure(&self, bones: &[BoneIndex]) -> Vec<bool> {
        let mut m = vec![false; self.skeleton.len()];
        for b in bones {
            let mut cur = Some(*b);
            while let Some(i) = cur {
                if i.index() >= m.len() || m[i.index()] {
                    break;
                }
                m[i.index()] = true;
                cur = self.skeleton.bones[i.index()].parent;
            }
        }
        m
    }

    /// Advance every layer by `dt` seconds (0 while paused) and run the transitions. Timeline, sound and particle
    /// events whose time was crossed by a playing animation's clock go to `sink` (once per loop; nothing at dt = 0).
    /// `inputs.query[anim_time]` is clobbered; `on_enter` may write `inputs.variables`.
    pub fn update(
        &self,
        st: &mut RigState,
        dt: f32,
        inputs: &mut ExprInputs,
        sink: &mut dyn FnMut(RigEvent<'_>),
    ) {
        if st.layers.len() != self.controllers.len() {
            *st = self.initial_state();
        }
        for (li, (c, ls)) in self.controllers.iter().zip(st.layers.iter_mut()).enumerate() {
            c.update(ls, dt, inputs, &mut |state, slot, old, new| {
                let Some(a) = c.states.get(state).and_then(|s| s.animations.get(slot)) else { return };
                if let Some(anim) = self.animations.get(a.index) {
                    anim.emit_events(old, new, |event, loop_index| {
                        sink(RigEvent { layer: li, animation: a.index, event, loop_index })
                    });
                }
            });
        }
    }

    /// Events of a trigger animation for the clock window `[e0, e1)` (the caller tracks the trigger's clock).
    pub fn trigger_events(
        &self,
        layer: usize,
        trigger: usize,
        e0: f32,
        e1: f32,
        sink: &mut dyn FnMut(RigEvent<'_>),
    ) {
        let Some(t) = self.controllers.get(layer).and_then(|c| c.triggers.get(trigger)) else { return };
        if let Some(anim) = self.animations.get(t.index) {
            anim.emit_events(e0, e1, |event, loop_index| sink(RigEvent { layer, animation: t.index, event, loop_index }));
        }
    }

    fn apply_anim(
        &self,
        ai: usize,
        clock: f32,
        weight_mul: f32,
        inputs: &mut ExprInputs,
        slots: &mut [Slot],
        needed: Option<&[bool]>,
    ) {
        let Some(anim) = self.animations.get(ai) else { return };
        let Some(t) = anim.sample_time(clock, inputs) else { return };
        let w = anim.weight(inputs) * weight_mul;
        if w <= 0.0 || !w.is_finite() {
            return;
        }
        let Some(bones) = self.anim_bones.get(ai) else { return };
        for (ba, bone) in anim.bones.iter().zip(bones) {
            let Some(bone) = bone else { continue };
            if needed.is_some_and(|n| !n.get(bone.index()).copied().unwrap_or(false)) {
                continue;
            }
            let Some(slot) = slots.get_mut(bone.index()) else { continue };
            for kind in [ChannelKind::Rotation, ChannelKind::Position, ChannelKind::Scale] {
                let Some(ch) = ba.channel(kind) else { continue };
                let v = ch.sample(t, inputs);
                put(slot, kind, v, w, anim.override_previous);
            }
        }
    }

    fn apply_state(
        &self,
        c: &Controller,
        run: &StateRun,
        inputs: &mut ExprInputs,
        slots: &mut [Slot],
        needed: Option<&[bool]>,
    ) {
        let Some(state) = c.states.get(run.state) else { return };
        for (i, a) in state.animations.iter().enumerate() {
            let clock = run.clocks.get(i).copied().unwrap_or(0.0);
            inputs.set(Query::AnimTime, clock);
            let wm = a.weight.as_ref().map_or(1.0, |w| w.eval(inputs).max(0.0));
            self.apply_anim(a.index, clock, wm, inputs, slots, needed);
        }
    }

    /// Evaluate the whole rig at its current state into `out` (resized to the skeleton). `needed` restricts the work
    /// to some bones (see [`Rig::bone_closure`]); the others stay at identity. `inputs.query[anim_time]` is restored.
    pub fn evaluate(
        &self,
        st: &RigState,
        triggers: &[TriggerPlay],
        inputs: &mut ExprInputs,
        scratch: &mut Scratch,
        needed: Option<&[bool]>,
        out: &mut Pose,
    ) {
        let saved = inputs.get(Query::AnimTime);
        let n = self.skeleton.len();
        scratch.prepare(n);
        let Scratch { cur, prev, fin } = scratch;
        for (li, (c, ls)) in self.controllers.iter().zip(st.layers.iter()).enumerate() {
            cur.fill(Slot::REST);
            self.apply_state(c, &ls.cur, inputs, cur, needed);
            if let Some(p) = &ls.prev {
                prev.fill(Slot::REST);
                self.apply_state(c, p, inputs, prev, needed);
                let alpha = c.blend_alpha(ls);
                for (i, (cs, ps)) in cur.iter_mut().zip(prev.iter()).enumerate() {
                    if needed.is_some_and(|nd| !nd[i]) {
                        continue;
                    }
                    *cs = lerp_slot(ps, cs, alpha);
                }
            }
            for tp in triggers.iter().filter(|t| t.layer as usize == li) {
                if let Some(t) = c.triggers.get(tp.trigger as usize) {
                    self.apply_anim(t.index, tp.elapsed, 1.0, inputs, cur, needed);
                }
            }
            inputs.set(Query::AnimTime, 0.0);
            let lw = c.weight.eval(inputs).max(0.0);
            if lw <= 0.0 {
                continue;
            }
            let mask = self.masks.get(li).and_then(|m| m.as_deref());
            for i in 0..n {
                if needed.is_some_and(|nd| !nd[i]) || cur[i].mask == 0 {
                    continue;
                }
                if mask.is_some_and(|m| !m[i]) {
                    continue;
                }
                combine(&mut fin[i], &cur[i], lw, c.mode);
            }
        }
        out.bones.clear();
        out.bones.resize(n, BoneTransform::IDENTITY);
        for (i, s) in fin.iter().enumerate() {
            if s.mask == 0 {
                continue;
            }
            out.bones[i] = BoneTransform { translation: s.pos, rotation: euler_degrees_to_quat(s.rot), scale: s.scale };
        }
        inputs.set(Query::AnimTime, saved);
    }

    /// Sample one animation alone at `clock` seconds, weight 1 (golden poses, tools, previews). A finished `Once`
    /// animation gives the rest pose.
    pub fn sample_animation(&self, anim: usize, clock: f32, inputs: &mut ExprInputs, out: &mut Pose) {
        let saved = inputs.get(Query::AnimTime);
        let n = self.skeleton.len();
        let mut slots = vec![Slot::REST; n];
        self.apply_anim(anim, clock, 1.0, inputs, &mut slots, None);
        out.bones.clear();
        out.bones.resize(n, BoneTransform::IDENTITY);
        for (i, s) in slots.iter().enumerate() {
            if s.mask != 0 {
                out.bones[i] =
                    BoneTransform { translation: s.pos, rotation: euler_degrees_to_quat(s.rot), scale: s.scale };
            }
        }
        inputs.set(Query::AnimTime, saved);
    }

    /// The model-space matrix of one bone from a (possibly sparse) pose, computed from its ancestor chain only, with
    /// [`bone_matrices`] on the chain so the convention is the skeleton's. For hitboxes that follow a bone.
    pub fn bone_matrix(&self, pose: &Pose, bone: BoneIndex) -> Mat4 {
        let mut chain = Vec::new();
        let mut cur = Some(bone);
        while let Some(i) = cur {
            let Some(b) = self.skeleton.bones.get(i.index()) else { break };
            chain.push(i);
            cur = b.parent;
        }
        chain.reverse();
        let sub = Skeleton {
            bones: chain
                .iter()
                .enumerate()
                .map(|(k, i)| {
                    let mut b = self.skeleton.bones[i.index()].clone();
                    b.parent = if k == 0 { None } else { Some(BoneIndex((k - 1) as u16)) };
                    b
                })
                .collect(),
        };
        let sub_pose = Pose { bones: chain.iter().map(|i| pose.bones.get(i.index()).copied().unwrap_or_default()).collect() };
        bone_matrices(&sub, &sub_pose).last().copied().unwrap_or(Mat4::IDENTITY)
    }

    /// Look up an animation by name (exact, or unique suffix).
    pub fn find_animation(&self, name: &str) -> Option<usize> {
        self.animations.find(name)
    }

    pub fn animation(&self, index: usize) -> Option<&Animation> {
        self.animations.get(index)
    }
}

/// Apply one channel value with weight `w` to a slot.
fn put(slot: &mut Slot, kind: ChannelKind, v: Vec3, w: f32, replace: bool) {
    let (dst, bit) = slot.channel(kind);
    if replace {
        let w = w.min(1.0);
        *dst += (v - *dst) * w;
    } else if kind == ChannelKind::Scale {
        *dst *= Vec3::ONE + (v - Vec3::ONE) * w;
    } else {
        *dst += v * w;
    }
    slot.mask |= bit;
}

fn lerp_slot(a: &Slot, b: &Slot, t: f32) -> Slot {
    Slot {
        rot: a.rot.lerp(b.rot, t),
        pos: a.pos.lerp(b.pos, t),
        scale: a.scale.lerp(b.scale, t),
        mask: a.mask | b.mask,
    }
}

/// Fold a layer's slot into the final slot.
fn combine(fin: &mut Slot, layer: &Slot, w: f32, mode: LayerMode) {
    for (bit, kind) in [(ROT, ChannelKind::Rotation), (POS, ChannelKind::Position), (SCALE, ChannelKind::Scale)] {
        if layer.mask & bit == 0 {
            continue;
        }
        let v = match kind {
            ChannelKind::Rotation => layer.rot,
            ChannelKind::Position => layer.pos,
            ChannelKind::Scale => layer.scale,
        };
        put(fin, kind, v, w, mode == LayerMode::Override);
    }
}
