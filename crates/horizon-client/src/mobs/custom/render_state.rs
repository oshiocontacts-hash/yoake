//! Client rendering state of fully custom mobs (FR-F4-MOBS.md sections 3.7 and 4, F4M-6).
//!
//! Two halves, both free of the GPU, the game window and any Minecraft resource:
//!
//! * [`TypeRender`]: what a *type* shares between its mobs for drawing, built once with the type
//!   ([`CustomType::new`]): the baked quads of every bone, locators, the layers, the LOD table, the
//!   cull box, the shadow, the procedural constraints and the bone sets the LOD modes need.
//! * [`MobRender`]: what one *mob* keeps from frame to frame: its rig state (controller layers and
//!   their blends), the procedural state (springs and look-at smoothing), the pose buffers, the bone
//!   matrices and the script's bone visibility. [`MobRender::update`] turns the mob's synced animation
//!   state ([`RemoteCustomMob`]: anchored clocks and triggers) into model-space bone matrices.
//!
//! The vertex output ([`TypeRender::draw`]) lives below; the game's glue is `game/custom_mob_draw.rs`.
//!
//! # Pose pipeline (one call of [`MobRender::update`])
//!
//! 1. Every layer of the rig takes the clocks the server anchored (`Controller::apply_anchor`): a state
//!    change fades like the server's (the client advances the fade by its own frame time); the first
//!    application of a mob snaps (no fade from the type's initial state), so a mob that comes into view
//!    midway shows the same pose as one that was watched all along.
//! 2. The running triggers (one-shot animations) get their elapsed time from their anchored clocks.
//! 3. `Rig::evaluate` writes the animated pose; the type's procedural constraints edit it (look at the
//!    viewer, springs, IK...), then the script's bone writes (a hook point, F4M-10).
//! 4. The skeleton composes the model-space matrices.
//!
//! Animation time never depends on the frame rate: it is the anchored clock at `game tick + partial`.
//! The constraints and the fades are the only parts that integrate over frames (with the game-time
//! delta between two updates, so a paused game pauses them).
//!
//! # LOD
//!
//! `model.lod` is a table of `{distance, mode}`: the last row whose distance the mob has reached
//! applies. `full`: every frame. `half_rate` and `quarter_rate`: the pose is evaluated every second or
//! fourth frame (staggered by entity id) and held in between: the mob itself still moves smoothly, only
//! its limbs update at the lower rate. `low_bones`: full rate, bones whose name ends in `_lod_min` or that
//! `model.lod_min_bones` names are
//! neither evaluated nor drawn. `static`: the rest pose. `hidden`: nothing is drawn.
use super::{
    CustomType, SyncedValue,
    anim_sync::{TICK_SECONDS, TypeAnim},
    remote::{RemoteCustomMob, RemoteCustomMobs},
};
use crate::mesher::Vertex;
use crate::mobs::Mob;
use glam::{Mat4, Vec3};
use horizon_mobmodel::{
    Locator, Model, Skeleton,
    bake::BakedBone,
    expr::{Expr, ExprInputs, Query, Symbols, compile},
    pose::{Rig, RigState, Scratch, TriggerPlay},
    procedural::{
        BoneVisibility, BoneWrite, ProceduralInputs, ProceduralSet, ProceduralState, TargetSource,
        apply_bone_writes_with_visibility, parse_procedural,
    },
    skeleton::{Pose, bone_matrices_into},
};
use horizon_mods::custom_mob::{CustomMobFile, LoadedModel};
use std::{collections::HashMap, ops::Range};

/// Longest game-time step (seconds) one update integrates: a stall or a long pause does not
/// fast-forward a fade or a spring.
const MAX_STEP_SECONDS: f64 = 0.25;
/// The bone-name suffix of the bones `low_bones` leaves out.
pub const LOD_MIN_SUFFIX: &str = "_lod_min";

/// One face of the baked model as the entity pass wants it: a quad (a triangle is a quad with a
/// repeated corner), bone-local positions in model pixels, UVs 0..1 of the model texture, one flat
/// normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quad {
    pub pos: [[f32; 3]; 4],
    pub uv: [[f32; 2]; 4],
    pub normal: [f32; 3],
}

/// A `model.lod` row's mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LodMode {
    Full,
    HalfRate,
    QuarterRate,
    LowBones,
    Static,
    Hidden,
}

impl LodMode {
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "full" => Self::Full,
            "half_rate" => Self::HalfRate,
            "quarter_rate" => Self::QuarterRate,
            "low_bones" => Self::LowBones,
            "static" => Self::Static,
            "hidden" => Self::Hidden,
            _ => return None,
        })
    }
}

/// How a layer is drawn (`model.layers[].mode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerMode {
    /// Alpha tested, lit like the body.
    Cutout,
    /// Blended, drawn after the opaque layers.
    Translucent,
    /// Full light, alpha tested, shaded like the body (OptiFine's emissive overlays).
    Emissive,
    /// Full light, unshaded, blended: glowing eyes.
    Additive,
}

impl LayerMode {
    fn from_name(name: &str) -> Self {
        match name {
            "translucent" => Self::Translucent,
            "emissive" => Self::Emissive,
            "additive" => Self::Additive,
            _ => Self::Cutout,
        }
    }
}

/// A compiled `model.layers[]` row.
#[derive(Clone, Debug)]
pub struct LayerDef {
    pub name: String,
    /// The layer's own texture (`<ns>:entity/...`); the body's when it names none.
    pub texture: Option<String>,
    pub mode: LayerMode,
    /// The bones it draws (the named ones and their descendants); `None` = every bone.
    pub bones: Option<Vec<bool>>,
    /// Packed `0xRRGGBB`: the expression's value as an integer.
    pub tint: Option<Expr>,
    pub visible: Option<Expr>,
}

/// Where the items of a mob go (`model.item_slots`): locator indices into [`TypeRender::locators`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemSlots {
    pub mainhand: Option<usize>,
    pub offhand: Option<usize>,
    pub head: Option<usize>,
}

/// The texture a bbmodel carries in its file, for the entity atlas (`embedded_textures`).
#[derive(Clone, Debug, PartialEq)]
pub struct EmbeddedTexture {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// What a type shares between its mobs for drawing (see the module docs).
#[derive(Debug)]
pub struct TypeRender {
    pub skeleton: Skeleton,
    pub quads: Vec<Quad>,
    /// Per bone, its range in `quads`.
    pub bone_quads: Vec<Range<u32>>,
    pub locators: Vec<Locator>,
    /// The body texture's atlas name (`model.textures.default`).
    pub texture: Option<String>,
    /// The automatic glow-mask layer's texture name: the body texture with `_glowmask`.
    pub glow_texture: Option<String>,
    pub layers: Vec<LayerDef>,
    /// `(distance in blocks, mode)`, distances rising.
    pub lod: Vec<(f32, LodMode)>,
    /// `model.cull`: width, height and the box's offset above the feet (blocks).
    pub cull: Option<[f32; 3]>,
    /// `model.shadow`: radius and strength.
    pub shadow: Option<(f32, f32)>,
    pub scale: f32,
    pub procedural: Option<ProceduralSet>,
    /// `model.procedural_script`: the type's `mob-pose` script writes bones (F4M-10).
    pub script_pose: bool,
    /// Bones whose name ends in [`LOD_MIN_SUFFIX`].
    pub lod_min: Vec<bool>,
    /// The bones `low_bones` evaluates: everything but `lod_min` bones (with all ancestors).
    pub low_needed: Vec<bool>,
    /// Model-space matrices of the rest pose.
    pub rest_matrices: Vec<Mat4>,
    pub item_slots: ItemSlots,
    pub embedded: Vec<EmbeddedTexture>,
}

impl TypeRender {
    /// The drawing data of a type from its file and loaded model; `None` for a type without a model.
    pub fn new(file: &CustomMobFile, loaded: &LoadedModel) -> Option<Self> {
        let model = loaded.model.as_ref()?;
        let mut syms = Symbols::new(&file.synced_names());
        for (name, s) in &file.synced {
            if s.kind == "enum" {
                let values: Vec<&str> = s.values.iter().map(String::as_str).collect();
                syms.set_enum_values(name, &values);
            }
        }
        Some(Self::from_model(file, model, &mut syms))
    }

    /// [`new`](Self::new) with the model and the expression symbols given.
    pub fn from_model(file: &CustomMobFile, model: &Model, syms: &mut Symbols) -> Self {
        let m = &file.model;
        let baked = model.bake();
        let mut quads = Vec::new();
        let mut bone_quads = Vec::with_capacity(baked.bones.len());
        for bone in &baked.bones {
            let start = quads.len() as u32;
            quads_of(bone, &mut quads);
            bone_quads.push(start..quads.len() as u32);
        }
        let n = model.skeleton.len();
        let bone_index = |name: &str| model.skeleton.find(name).map(|b| b.index());
        let lod_min: Vec<bool> = model
            .skeleton
            .bones
            .iter()
            .map(|b| b.name.ends_with(LOD_MIN_SUFFIX) || m.lod_min_bones.iter().any(|n| *n == b.name))
            .collect();
        let keep: Vec<_> = (0..n).filter(|i| !lod_min[*i]).map(|i| horizon_mobmodel::BoneIndex(i as u16)).collect();
        let low_needed = closure(&model.skeleton, &keep);
        let layers = m
            .layers
            .iter()
            .map(|l| {
                let build = |what: &str, src: &Option<String>, syms: &mut Symbols| {
                    src.as_deref().and_then(|s| match compile(s, syms) {
                        Ok(e) => Some(e),
                        Err(e) => {
                            log::warn!("Horizon: {}: layer {}: {what}: {e}; ignored", file.id, l.name);
                            None
                        }
                    })
                };
                let bones = (!l.bones.is_empty()).then(|| {
                    let named: Vec<_> = l
                        .bones
                        .iter()
                        .filter_map(|b| bone_index(b))
                        .map(|i| horizon_mobmodel::BoneIndex(i as u16))
                        .collect();
                    descendants(&model.skeleton, &named)
                });
                LayerDef {
                    name: l.name.clone(),
                    texture: l.texture.clone(),
                    mode: LayerMode::from_name(&l.mode),
                    bones,
                    tint: build("tint", &l.tint, syms),
                    visible: build("visible", &l.visible, syms),
                }
            })
            .collect();
        let mut lod: Vec<(f32, LodMode)> = m
            .lod
            .iter()
            .filter_map(|l| Some((l.distance as f32, LodMode::from_name(&l.mode)?)))
            .collect();
        lod.sort_by(|a, b| a.0.total_cmp(&b.0));
        let procedural = if m.procedural.is_empty() {
            None
        } else {
            match parse_procedural(
                &file.id,
                "model.procedural",
                &serde_json::Value::Array(m.procedural.clone()),
                &model.skeleton,
                &model.locators,
                syms,
            ) {
                Ok(set) => Some(set),
                Err(e) => {
                    log::warn!("Horizon: {}: {e}; the constraints are ignored", file.id);
                    None
                }
            }
        };
        let locator = |name: Option<&String>| name.and_then(|n| model.locators.iter().position(|l| &l.name == n));
        let item_slots = ItemSlots {
            mainhand: locator(m.item_slots.get("mainhand")),
            offhand: locator(m.item_slots.get("offhand")),
            head: locator(m.item_slots.get("head")),
        };
        let mut rest_matrices = Vec::new();
        bone_matrices_into(&model.skeleton, &Pose::rest(&model.skeleton), &mut rest_matrices);
        let texture = m.textures.default.clone();
        let glow_texture = texture.as_ref().map(|t| format!("{t}_glowmask"));
        let embedded = model
            .textures
            .iter()
            .take(1)
            .filter(|t| !t.rgba.is_empty() && t.width > 0 && t.height > 0)
            .map(|t| EmbeddedTexture {
                name: texture.clone().unwrap_or_else(|| default_texture_name(&file.id)),
                width: t.width,
                height: t.height,
                rgba: t.rgba.clone(),
            })
            .collect();
        Self {
            skeleton: model.skeleton.clone(),
            quads,
            bone_quads,
            locators: model.locators.clone(),
            texture,
            glow_texture,
            layers,
            lod,
            cull: m.cull.as_ref().map(|c| [c.width as f32, c.height as f32, c.y_offset as f32]),
            shadow: m.shadow.as_ref().map(|s| (s.radius as f32, s.strength as f32)),
            scale: m.scale as f32,
            procedural,
            script_pose: m.procedural_script,
            lod_min,
            low_needed,
            rest_matrices,
            item_slots,
            embedded,
        }
    }

    /// The LOD mode at `distance` blocks from the camera.
    pub fn lod_mode(&self, distance: f32) -> LodMode {
        self.lod.iter().rev().find(|(d, _)| distance >= *d).map_or(LodMode::Full, |(_, m)| *m)
    }

    /// The culling box relative to the mob's feet, in blocks: `(low, high)`, before the engine's
    /// margin. The type's `cull` box (centred on the feet, `y_offset` above them), else the hitbox
    /// `width` x `height` the engine uses for vanilla mobs. Both times the model scale.
    pub fn cull_box(&self, width: f32, height: f32, scale: f32) -> ([f32; 3], [f32; 3]) {
        let [w, h, y] = self.cull.unwrap_or([width, height, 0.0]);
        let (w, h, y) = (w * scale, h * scale, y * scale);
        ([-w / 2.0, y, -w / 2.0], [w / 2.0, y + h, w / 2.0])
    }

    /// The bone index of locator `i`'s bone's matrix applied to its rest position: a model-space
    /// position (pixels) for the pose `matrices`.
    pub fn locator_point(&self, i: usize, matrices: &[Mat4]) -> Option<Vec3> {
        let l = self.locators.get(i)?;
        Some(matrices.get(l.bone.index())?.transform_point3(l.offset))
    }

    /// The bone matrix's rotation about locator `i` (columns normalised): the frame an item follows.
    pub fn locator_frame(&self, i: usize, matrices: &[Mat4]) -> Option<Mat4> {
        let l = self.locators.get(i)?;
        let m = matrices.get(l.bone.index())?;
        let p = m.transform_point3(l.offset);
        let (x, y, z) = (m.x_axis.truncate().normalize_or_zero(), m.y_axis.truncate().normalize_or_zero(), m.z_axis.truncate().normalize_or_zero());
        Some(Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), p.extend(1.0)))
    }

    /// Whether bone `b` draws for `lod`.
    pub fn bone_drawn(&self, b: usize, lod: LodMode, vis: &BoneVisibility) -> bool {
        vis.is_visible(b) && !(lod == LodMode::LowBones && self.lod_min.get(b).copied().unwrap_or(false))
    }
}

/// The atlas name of a type's body texture when the file names none: `<ns>:entity/<path>`.
pub fn default_texture_name(id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    format!("{ns}:entity/{path}")
}

/// `bones` and their ancestors.
fn closure(skeleton: &Skeleton, bones: &[horizon_mobmodel::BoneIndex]) -> Vec<bool> {
    let mut m = vec![false; skeleton.len()];
    for b in bones {
        let mut cur = Some(*b);
        while let Some(i) = cur {
            if i.index() >= m.len() || m[i.index()] {
                break;
            }
            m[i.index()] = true;
            cur = skeleton.bones[i.index()].parent;
        }
    }
    m
}

/// `bones` and everything below them.
fn descendants(skeleton: &Skeleton, bones: &[horizon_mobmodel::BoneIndex]) -> Vec<bool> {
    let mut m = vec![false; skeleton.len()];
    for (i, b) in skeleton.bones.iter().enumerate() {
        // Parents come first, so one pass sees every ancestor decided.
        m[i] = bones.iter().any(|x| x.index() == i) || b.parent.is_some_and(|p| m[p.index()]);
    }
    m
}

/// The bone's triangles as quads: a cube face is the pair `(a, d, c)`, `(a, c, b)` of its four
/// vertices and becomes one quad `[a, b, c, d]`; a fan of a mesh pairs the same way; a triangle left
/// alone becomes a quad with its last corner repeated.
fn quads_of(bone: &BakedBone, out: &mut Vec<Quad>) {
    let v = &bone.vertices;
    let tri: Vec<[u32; 3]> = bone.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
    let make = |c: [u32; 4]| -> Option<Quad> {
        let at = |i: u32| v.get(i as usize);
        let (a, b, cc, d) = (at(c[0])?, at(c[1])?, at(c[2])?, at(c[3])?);
        Some(Quad {
            pos: [a.position, b.position, cc.position, d.position],
            uv: [a.uv, b.uv, cc.uv, d.uv],
            normal: a.normal,
        })
    };
    let mut i = 0;
    while i < tri.len() {
        let t0 = tri[i];
        if let Some(t1) = tri.get(i + 1).copied()
            && t0[0] == t1[0]
            && t0[2] == t1[1]
            && t0[1] != t1[2]
            && let Some(q) = make([t0[0], t1[2], t0[2], t0[1]])
        {
            out.push(q);
            i += 2;
            continue;
        }
        if let Some(q) = make([t0[0], t0[1], t0[2], t0[2]]) {
            out.push(q);
        }
        i += 1;
    }
}

// ----------------------------------------------------------------------------------------------
// What a frame needs to know of the mob
// ----------------------------------------------------------------------------------------------

/// What the animation expressions read of a mob (`query.*`), taken from the engine's mob.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MobFacts {
    pub health: f32,
    pub max_health: f32,
    pub on_ground: bool,
    pub in_water: bool,
    pub baby: bool,
    pub riding: bool,
    pub has_target: bool,
    pub hurt_time: f32,
    pub death_time: f32,
    /// Seconds since it spawned.
    pub life_time: f32,
    /// Blocks per tick, as the server's `query.ground_speed`.
    pub ground_speed: f32,
    pub vertical_speed: f32,
    pub yaw_speed: f32,
    pub head_x: f32,
    pub head_y: f32,
    pub body_y: f32,
    pub seed: u32,
}

fn wrap(deg: f32) -> f32 {
    let d = deg.rem_euclid(360.0);
    if d > 180.0 { d - 360.0 } else { d }
}

impl MobFacts {
    pub fn of(m: &Mob) -> Self {
        let (dx, dy, dz) = (m.position.x - m.previous.x, m.position.y - m.previous.y, m.position.z - m.previous.z);
        Self {
            health: m.health,
            max_health: m.attrs.max_health,
            on_ground: m.on_ground,
            in_water: m.fluids.in_water,
            baby: m.is_baby(),
            riding: m.extra.mount.is_some(),
            has_target: m.ai.target.is_some(),
            hurt_time: m.hurt_time as f32,
            death_time: m.death_time as f32,
            life_time: m.age as f32 / 20.0,
            ground_speed: (dx * dx + dz * dz).sqrt() as f32,
            vertical_speed: dy as f32,
            yaw_speed: wrap(m.body_yaw - m.previous_body_yaw),
            head_x: m.head_pitch,
            head_y: wrap(m.head_yaw - m.body_yaw),
            body_y: m.body_yaw,
            seed: m.id,
        }
    }

    /// Writes the facts into `i` (its `variables` are left alone: the controllers keep theirs).
    pub fn fill(&self, i: &mut ExprInputs, day_fraction: f32, tick: i64, values: &[SyncedValue]) {
        let flag = |b: bool| if b { 1.0 } else { 0.0 };
        i.set(Query::Health, self.health);
        i.set(Query::MaxHealth, self.max_health);
        i.set(Query::IsOnGround, flag(self.on_ground));
        i.set(Query::IsInWater, flag(self.in_water));
        i.set(Query::IsBaby, flag(self.baby));
        i.set(Query::IsRiding, flag(self.riding));
        i.set(Query::HasTarget, flag(self.has_target));
        i.set(Query::HurtTime, self.hurt_time);
        i.set(Query::DeathTime, self.death_time);
        i.set(Query::LifeTime, self.life_time);
        i.set(Query::GroundSpeed, self.ground_speed);
        i.set(Query::VerticalSpeed, self.vertical_speed);
        i.set(Query::YawSpeed, self.yaw_speed);
        i.set(Query::HeadXRotation, self.head_x);
        i.set(Query::HeadYRotation, self.head_y);
        i.set(Query::BodyYRotation, self.body_y);
        i.set(Query::TimeOfDay, day_fraction);
        i.seed = self.seed;
        i.random_epoch = tick as u32;
        for (slot, v) in values.iter().take(i.synced.len()).enumerate() {
            i.synced[slot] = match v {
                SyncedValue::Bool(b) => flag(*b),
                SyncedValue::I32(n) => *n as f32,
                SyncedValue::F32(f) => *f,
                SyncedValue::Enum(e) => f32::from(*e),
                SyncedValue::Str(_) | SyncedValue::Vec3(_) => 0.0,
            };
        }
    }
}

/// Everything one [`MobRender::update`] needs besides the mob's animation state.
#[derive(Clone, Debug)]
pub struct PoseCtx {
    /// The client's game tick and the fraction of the way to the next one.
    pub tick: i64,
    pub partial: f32,
    /// The render frame counter (LOD stagger).
    pub frame: u64,
    pub lod: LodMode,
    pub facts: MobFacts,
    /// `day_time % 24000 / 24000`.
    pub day_fraction: f32,
    /// Targets in model space, pixels: the viewer (cosmetic, per client), the mob's attack target.
    pub view: Option<Vec3>,
    pub target: Option<Vec3>,
    pub owner: Option<Vec3>,
    /// The mob's full transform times 16 (springs; see `ProceduralInputs::model_to_world`).
    pub model_to_world: Mat4,
}

impl PoseCtx {
    pub fn new(tick: i64, partial: f32, facts: MobFacts) -> Self {
        Self {
            tick,
            partial,
            frame: 0,
            lod: LodMode::Full,
            facts,
            day_fraction: 0.0,
            view: None,
            target: None,
            owner: None,
            model_to_world: Mat4::IDENTITY,
        }
    }
}

// ----------------------------------------------------------------------------------------------
// One mob
// ----------------------------------------------------------------------------------------------

/// What one mob keeps between frames (see the module docs).
pub struct MobRender {
    rig_state: Option<RigState>,
    inputs: ExprInputs,
    scratch: Scratch,
    pose: Pose,
    pstate: Option<ProceduralState>,
    triggers: Vec<TriggerPlay>,
    /// Model-space bone matrices of the pose last evaluated (pixels).
    pub matrices: Vec<Mat4>,
    /// The script's bone visibility (`mob-pose` writes); everything visible by default.
    pub visibility: BoneVisibility,
    /// The script's bone writes for the next update (hook point: F4M-10 fills it), consumed by it.
    pub writes: Vec<BoneWrite>,
    /// `tick + partial` of the last evaluation.
    last_now: Option<f64>,
    /// The frame it ran in (the staggering counts from it).
    pub evaluated_frame: Option<u64>,
    /// The render frame this state was last used in (stale ones are dropped).
    pub used_frame: u64,
    /// How many times the pose was evaluated (tests and the debug view).
    pub evaluations: u64,
}

impl MobRender {
    pub fn new(t: &TypeRender) -> Self {
        Self {
            rig_state: None,
            inputs: ExprInputs::default(),
            scratch: Scratch::default(),
            pose: Pose::default(),
            pstate: t.procedural.as_ref().map(ProceduralState::new),
            triggers: Vec::new(),
            matrices: t.rest_matrices.clone(),
            visibility: BoneVisibility::new(t.skeleton.len()),
            writes: Vec::new(),
            last_now: None,
            evaluated_frame: None,
            used_frame: 0,
            evaluations: 0,
        }
    }

    /// The expression inputs of the last update (layer tints and visibility read them).
    pub fn inputs(&self) -> &ExprInputs {
        &self.inputs
    }

    /// Whether the pose is evaluated this frame at `lod`.
    pub fn due(&self, lod: LodMode, frame: u64, id: u32) -> bool {
        let stagger = |period: u64| (frame + u64::from(id)) % period == 0;
        match lod {
            LodMode::Full | LodMode::LowBones => true,
            LodMode::HalfRate => self.evaluated_frame.is_none() || stagger(2),
            LodMode::QuarterRate => self.evaluated_frame.is_none() || stagger(4),
            LodMode::Static => self.evaluated_frame.is_none(),
            LodMode::Hidden => false,
        }
    }

    /// Brings `matrices` up to date for `ctx` (see the module docs); false when the LOD held the last
    /// pose. `anim` is the mob's synced animation state (`None`: the types initial pose).
    pub fn update(&mut self, ty: &CustomType, tr: &TypeRender, anim: Option<&RemoteCustomMob>, ctx: &PoseCtx) -> bool {
        if !self.due(ctx.lod, ctx.frame, ctx.facts.seed) {
            return false;
        }
        self.evaluated_frame = Some(ctx.frame);
        if ctx.lod == LodMode::Static {
            self.matrices.clone_from(&tr.rest_matrices);
            return true;
        }
        self.evaluations += 1;
        let now = ctx.tick as f64 + f64::from(ctx.partial);
        let dt = self.last_now.map_or(0.0, |l| ((now - l) / 20.0).clamp(0.0, MAX_STEP_SECONDS)) as f32;
        let fresh = self.last_now.is_none();
        self.last_now = Some(now);
        let values: &[SyncedValue] = match anim {
            Some(a) => &a.values,
            None => &[],
        };
        ctx.facts.fill(&mut self.inputs, ctx.day_fraction, ctx.tick, values);
        match &ty.anim {
            Some(ta) => self.evaluate_rig(ta, anim, ctx, dt, fresh, tr),
            None => {
                self.pose.bones.clear();
                self.pose.bones.resize(tr.skeleton.len(), Default::default());
            }
        }
        if let (Some(set), Some(state)) = (&tr.procedural, &mut self.pstate)
            && !matches!(ctx.lod, LodMode::Static)
        {
            let mut pi = ProceduralInputs::new(&self.inputs, dt);
            pi.model_to_world = ctx.model_to_world;
            for (source, point) in [
                (TargetSource::View, ctx.view),
                (TargetSource::Target, ctx.target),
                (TargetSource::Owner, ctx.owner),
            ] {
                if let Some(p) = point {
                    pi = pi.with_target(source, p);
                }
            }
            set.evaluate(&mut self.pose, state, &pi);
        }
        if !self.writes.is_empty() {
            let writes = std::mem::take(&mut self.writes);
            apply_bone_writes_with_visibility(&mut self.pose, &mut self.visibility, &writes);
        }
        bone_matrices_into(&tr.skeleton, &self.pose, &mut self.matrices);
        true
    }

    fn evaluate_rig(&mut self, ta: &TypeAnim, anim: Option<&RemoteCustomMob>, ctx: &PoseCtx, dt: f32, fresh: bool, tr: &TypeRender) {
        let rig: &Rig = &ta.rig;
        let st = self.rig_state.get_or_insert_with(|| rig.initial_state());
        if st.layers.len() != rig.controllers().len() {
            *st = rig.initial_state();
        }
        for (li, (c, ls)) in rig.controllers().iter().zip(st.layers.iter_mut()).enumerate() {
            if let Some(layer) = anim.and_then(|a| a.layers.get(li)).and_then(Option::as_ref) {
                let target = usize::from(layer.state);
                if fresh && target != ls.cur.state {
                    // A mob seen for the first time shows its state at once (the late-join rule).
                    c.set_state(ls, target, false, &mut self.inputs);
                }
                for (slot, clock) in layer.clocks.iter().enumerate() {
                    c.apply_anchor(ls, slot, clock, ctx.tick, ctx.partial, &mut self.inputs);
                }
            }
            // The fade runs on this client's frame time; the old state keeps playing while it fades.
            if dt > 0.0 {
                ls.cur.time += dt;
                if let Some(mut prev) = ls.prev {
                    for i in 0..prev.clocks.len() {
                        prev.clocks[i] += dt * c.slot_speed(&prev, i, &mut self.inputs);
                    }
                    prev.time += dt;
                    ls.blend_elapsed += dt;
                    ls.prev = (ls.blend_elapsed < ls.blend_duration).then_some(prev);
                }
            }
        }
        self.triggers.clear();
        if let Some(a) = anim {
            for t in &a.triggers {
                let Some(def) = ta.triggers.get(usize::from(t.trigger.id)) else {
                    continue;
                };
                let elapsed = t.clock.time_at(ctx.tick, ctx.partial) as f32;
                if elapsed <= def.length {
                    self.triggers.push(TriggerPlay { layer: def.layer as u16, trigger: def.index as u16, elapsed });
                }
            }
        }
        let needed = (ctx.lod == LodMode::LowBones).then_some(tr.low_needed.as_slice());
        if let Some(st) = self.rig_state.as_ref() {
            rig.evaluate(st, &self.triggers, &mut self.inputs, &mut self.scratch, needed, &mut self.pose);
        }
        // A dying mob plays its `death` animation alone, once, and holds its last pose (the engine's
        // tilt is not used then, see `death_roll_for`).
        if let Some(clock) = death_clock(ta, ctx.facts.death_time, ctx.partial)
            && let Some((anim, _)) = ta.death
        {
            rig.sample_animation(anim, clock, &mut self.inputs, &mut self.pose);
        }
    }
}

// ----------------------------------------------------------------------------------------------
// The integrated server's own mobs
// ----------------------------------------------------------------------------------------------

/// What a client keeps of custom mobs that run in its own process (single player, LAN host): their
/// state is read from the mob itself, but it goes through the same [`RemoteCustomMobs`] store a
/// network client fills, so the same code poses them and fires their events (and a late-joined
/// mirror equals a late-joined remote client).
#[derive(Debug, Default)]
pub struct LocalMirror {
    store: RemoteCustomMobs,
    memo: HashMap<u32, Memo>,
}

#[derive(Debug, Default)]
struct Memo {
    anchors: Vec<Option<horizon_protocol::mob_sync::LayerAnchor>>,
    triggers: Vec<(u16, i64)>,
    stamp: u64,
}

impl LocalMirror {
    /// The `synced-changed` notifications of the mirrored mobs since the last call.
    pub fn take_changes(&mut self) -> Vec<super::remote::SyncedChanged> {
        self.store.take_changes()
    }

    pub fn is_empty(&self) -> bool {
        self.store.is_empty() && self.memo.is_empty()
    }

    pub fn get(&self, id: u32) -> Option<&RemoteCustomMob> {
        self.store.get(id as i32)
    }

    pub fn get_mut(&mut self, id: u32) -> Option<&mut RemoteCustomMob> {
        self.store.get_mut(id as i32)
    }

    /// Brings the mirror of `m` up to date at game tick `now` for the window of `viewer` (the host's player uuid): the first call is a snapshot (a late
    /// join), later ones send what the server's sync layer would: anchors that moved past the
    /// tolerance and the triggers that started. `stamp` marks the mob as alive (see [`retain`](Self::retain)).
    pub fn sync(&mut self, m: &Mob, now: i64, stamp: u64, viewer: Option<[i32; 4]>) {
        use horizon_protocol::mob_sync::{Delta, Frame, Message, TriggerStart};
        let (Some(t), Some(state)) = (m.kind.custom.as_ref(), m.extra.custom.as_ref()) else {
            return;
        };
        let id = m.id;
        let kind_of = |_: i32| Some(t.clone());
        let anim = state.anim.as_ref();
        let known = self.memo.contains_key(&id);
        let memo = self.memo.entry(id).or_default();
        memo.stamp = stamp;
        if !known {
            if let Some(s) = crate::lan::mob_sync::snapshot(m) {
                memo.anchors = (0..s.anchors.iter().map(|a| usize::from(a.layer) + 1).max().unwrap_or(0)).map(|_| None).collect();
                for a in &s.anchors {
                    memo.anchors[usize::from(a.layer)] = Some(a.clone());
                }
                memo.triggers = s.triggers.iter().map(|tr| (tr.id, tr.start_tick)).collect();
                let mut frame = Frame::new(now);
                frame.messages.push(Message::Snapshot(s));
                self.store.apply_at(&frame, now, &kind_of);
            }
        } else if let (Some(a), Some(ta)) = (anim, t.anim.as_ref()) {
            let mut frame = Frame::new(now);
            let mut moved = Vec::new();
            for cur in a.anchors() {
                let li = usize::from(cur.layer);
                if memo.anchors.len() <= li {
                    memo.anchors.resize(li + 1, None);
                }
                let sent = memo.anchors[li].as_ref();
                if sent.is_none_or(|s| super::anim_sync::needs_anchor(s, cur, super::anim_sync::ANCHOR_TOLERANCE)) {
                    memo.anchors[li] = Some(cur.clone());
                    moved.push(cur.clone());
                }
            }
            if !moved.is_empty() {
                frame.messages.push(Message::Delta(Delta { entity: id as i32, values: None, anchors: moved, hitbox_mask: None }));
            }
            for tr in a.recent_triggers(ta) {
                if !memo.triggers.contains(&(tr.id, tr.start_tick)) {
                    memo.triggers.push((tr.id, tr.start_tick));
                    frame.messages.push(Message::Trigger(TriggerStart { entity: id as i32, trigger: tr }));
                }
            }
            if memo.triggers.len() > 32 {
                memo.triggers.drain(..16);
            }
            self.store.apply_at(&frame, now, &kind_of);
        }
        // Every value, not only the announced ones (the animations of this process read them all),
        // but as the host's window sees them: a `sync: owner` value of a mob the host's player does
        // not own reads as its default, as for a remote client (`lan::mob_sync::value_visible_to`).
        if let Some(rm) = self.store.get_mut(id as i32) {
            rm.values.clear();
            for (slot, value) in state.synced.iter().enumerate() {
                let visible = crate::lan::mob_sync::value_visible_to(m, slot, viewer);
                rm.values.push(if visible { value.clone() } else { t.synced[slot].default.clone() });
            }
        }
    }

    /// Forgets the mobs not synced since `stamp`.
    pub fn retain(&mut self, stamp: u64) {
        if self.memo.values().all(|m| m.stamp >= stamp) {
            return;
        }
        let gone: Vec<u32> = self.memo.iter().filter(|(_, m)| m.stamp < stamp).map(|(id, _)| *id).collect();
        for id in gone {
            self.memo.remove(&id);
            self.store.remove(id as i32);
        }
    }
}

/// The per-game container: every mob's render state, the mirror of the local mobs and the frame
/// counter. Empty (no allocation) until a custom mob is drawn.
#[derive(Debug, Default)]
pub struct CustomRender {
    mobs: HashMap<u32, MobRender>,
    pub mirror: LocalMirror,
    frame: u64,
    events: Option<horizon_mobmodel::events::EventBuf>,
}

impl CustomRender {
    /// Starts a frame; the counter is what `MobRender::used_frame` and the LOD stagger count.
    pub fn begin_frame(&mut self) -> u64 {
        self.frame += 1;
        // The states of mobs not drawn for 600 frames (gone, or out of range) are dropped.
        if self.frame % 120 == 0 && !self.mobs.is_empty() {
            let frame = self.frame;
            self.mobs.retain(|_, m| frame.saturating_sub(m.used_frame) < 600);
        }
        self.frame
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// The render state of mob `id`, made on first use.
    pub fn mob(&mut self, id: u32, t: &TypeRender) -> &mut MobRender {
        let frame = self.frame;
        let m = self.mobs.entry(id).or_insert_with(|| MobRender::new(t));
        m.used_frame = frame;
        m
    }

    pub fn len(&self) -> usize {
        self.mobs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mobs.is_empty() && self.mirror.is_empty()
    }
}

impl std::fmt::Debug for MobRender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MobRender")
            .field("bones", &self.matrices.len())
            .field("evaluations", &self.evaluations)
            .field("evaluated_frame", &self.evaluated_frame)
            .finish_non_exhaustive()
    }
}

// ----------------------------------------------------------------------------------------------
// The mob's transform
// ----------------------------------------------------------------------------------------------

/// The roll of a dying mob (`LivingEntityRenderer.setupRotations`: it falls on its side over
/// `death_time` ticks, `sqrt(t * 1.6 / 20)` of 90 degrees).
pub fn death_roll_degrees(death_time: i32, partial: f32) -> f32 {
    if death_time <= 0 {
        return 0.0;
    }
    let f = ((death_time as f32 + partial - 1.0) / 20.0 * 1.6).sqrt().min(1.0);
    f * 90.0
}

/// The seconds into the `death` animation of a mob `death_time` ticks dead (held on the last pose), or
/// `None` for a living mob or a type without that animation.
pub fn death_clock(ta: &TypeAnim, death_time: f32, partial: f32) -> Option<f32> {
    let (_, length) = ta.death?;
    (death_time > 0.0).then(|| ((death_time - 1.0 + partial).max(0.0) * TICK_SECONDS).min((length - 1e-3).max(0.0)))
}

/// The roll of a dying mob of `ct`: none when its type plays its own `death` animation, the engine's
/// tilt otherwise.
pub fn death_roll_for(ct: &CustomType, death_time: i32, partial: f32) -> f32 {
    if ct.death_ticks().is_some() { 0.0 } else { death_roll_degrees(death_time, partial) }
}

/// Model space (Blockbench: pixels, -Z north) to the mob's frame in blocks, without its position: the
/// model scale, the yaw (a mob at yaw 0 faces south, +Z), the death roll and the half turn about Y
/// that puts the model's front (-Z) on the mob's front. Multiply by `1/16` for pixels.
pub fn mob_basis(scale: f32, body_yaw: f32, roll: f32) -> Mat4 {
    Mat4::from_scale(Vec3::splat(scale))
        * Mat4::from_rotation_y((-body_yaw).to_radians())
        * Mat4::from_rotation_z(roll.to_radians())
        * Mat4::from_rotation_y(std::f32::consts::PI)
}

/// Model pixels to the render frame (blocks, relative to the camera): `rel` is the mob's feet
/// minus the camera.
pub fn world_from_model(rel: Vec3, scale: f32, body_yaw: f32, roll: f32) -> Mat4 {
    Mat4::from_translation(rel) * mob_basis(scale, body_yaw, roll) * Mat4::from_scale(Vec3::splat(1.0 / 16.0))
}

/// The mob's full transform times 16, the frame the springs simulate in (see
/// `ProceduralInputs::model_to_world`): model pixels to world pixels.
pub fn model_to_world_px(pos: [f64; 3], scale: f32, body_yaw: f32, roll: f32) -> Mat4 {
    Mat4::from_translation(Vec3::new(pos[0] as f32, pos[1] as f32, pos[2] as f32) * 16.0)
        * mob_basis(scale, body_yaw, roll)
}

/// A model-space point (pixels) in the world (blocks), in double precision.
pub fn model_point_to_world(pos: [f64; 3], scale: f32, body_yaw: f32, roll: f32, p: Vec3) -> [f64; 3] {
    use glam::{DMat4, DVec3};
    let m = DMat4::from_translation(DVec3::from(pos))
        * DMat4::from_scale(DVec3::splat(f64::from(scale)))
        * DMat4::from_rotation_y(f64::from(-body_yaw).to_radians())
        * DMat4::from_rotation_z(f64::from(roll).to_radians())
        * DMat4::from_rotation_y(std::f64::consts::PI);
    m.transform_point3(DVec3::new(f64::from(p.x), f64::from(p.y), f64::from(p.z)) / 16.0).to_array()
}

/// A point of the camera (relative to the mob's feet, blocks) in the model's space (pixels): what
/// `look_at: view` aims at. Per client and cosmetic.
pub fn model_point_of(rel_camera: Vec3, scale: f32, body_yaw: f32, roll: f32) -> Vec3 {
    mob_basis(scale, body_yaw, roll).inverse().transform_point3(rel_camera) * 16.0
}

// ----------------------------------------------------------------------------------------------
// Vertex output
// ----------------------------------------------------------------------------------------------

/// Vertex counts of what one [`TypeRender::draw`] produced, by layer kind (tests, the debug view).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DrawStats {
    /// The body and the cutout layers.
    pub cutout: usize,
    /// Translucent and additive layers.
    pub translucent: usize,
    /// Emissive layers, the automatic glow mask included.
    pub emissive: usize,
}

impl DrawStats {
    pub fn total(&self) -> usize {
        self.cutout + self.translucent + self.emissive
    }
}

/// What the vertex output needs besides the type and the pose.
pub struct DrawCtx<'a> {
    /// Model pixels to the render frame ([`world_from_model`]).
    pub world_from_model: Mat4,
    /// Packed sky and block light (the entity pass's `light`).
    pub light: u32,
    /// The hurt overlay word of the body (`overlay::living`): 0, or 1 for the red flash.
    pub overlay: u32,
    pub lod: LodMode,
    /// Texture rectangles by atlas name.
    pub atlas: &'a dyn Fn(&str) -> Option<[f32; 4]>,
}

/// Whether a quad is shaded by its normal or drawn as it is.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shade {
    Lit,
    Flat,
}

/// A face normal as the vertex's `RGBA8_SNORM` (two-sided lighting).
fn pack_normal(n: Vec3) -> [i8; 4] {
    let c = |v: f32| (v.clamp(-1.0, 1.0) * 127.0) as i8;
    [c(n.x), c(n.y), c(n.z), -127]
}

const WHITE: u32 = 0xFFFF_FFFF;

impl TypeRender {
    /// Appends the mob's vertices for the pose in `mr` to `out`: the body, then the opaque layers
    /// (cutout and emissive, in file order, the automatic glow mask after them), then the
    /// translucent and additive ones, which sort after everything opaque. A texture the atlas does
    /// not know draws nothing. See the module docs for LOD, the bone sets and what each mode means.
    pub fn draw(&self, mr: &MobRender, ctx: &DrawCtx, out: &mut Vec<Vertex>) -> DrawStats {
        let mut stats = DrawStats::default();
        if ctx.lod == LodMode::Hidden {
            return stats;
        }
        out.reserve(self.quads.len() * 4 * 2);
        let body = self.texture.as_deref().and_then(|t| (ctx.atlas)(t));
        if let Some(uv) = body {
            stats.cutout += self.emit(mr, ctx, None, uv, WHITE, ctx.light, ctx.overlay, Shade::Lit, out);
        }
        let mut glow_named = false;
        for l in &self.layers {
            if l.mode == LayerMode::Translucent || l.mode == LayerMode::Additive {
                continue;
            }
            glow_named |= l.mode == LayerMode::Emissive && l.texture.is_some() && l.texture == self.glow_texture;
            let Some((uv, color)) = self.layer_paint(l, mr, ctx) else {
                continue;
            };
            if l.mode == LayerMode::Emissive {
                stats.emissive += self.emit(mr, ctx, l.bones.as_deref(), uv, color, FULL_BRIGHT, 0, Shade::Lit, out);
            } else {
                stats.cutout += self.emit(mr, ctx, l.bones.as_deref(), uv, color, ctx.light, ctx.overlay, Shade::Lit, out);
            }
        }
        // `<texture>_glowmask` beside the body texture is an emissive layer of the whole model.
        if !glow_named
            && let Some(uv) = self.glow_texture.as_deref().and_then(|t| (ctx.atlas)(t))
        {
            stats.emissive += self.emit(mr, ctx, None, uv, WHITE, FULL_BRIGHT, 0, Shade::Lit, out);
        }
        for l in &self.layers {
            if l.mode != LayerMode::Translucent && l.mode != LayerMode::Additive {
                continue;
            }
            let Some((uv, color)) = self.layer_paint(l, mr, ctx) else {
                continue;
            };
            let start = out.len();
            let n = if l.mode == LayerMode::Additive {
                let n = self.emit(mr, ctx, l.bones.as_deref(), uv, color, FULL_BRIGHT, 0, Shade::Flat, out);
                for v in &mut out[start..] {
                    v.layer |= u32::from(crate::mesher::BRIGHT) << 16;
                }
                n
            } else {
                self.emit(mr, ctx, l.bones.as_deref(), uv, color, ctx.light, ctx.overlay, Shade::Lit, out)
            };
            for v in &mut out[start..] {
                v.layer |= crate::gfx::OIT_ENTITY;
            }
            stats.translucent += n;
        }
        stats
    }

    /// The texture rectangle and colour a layer draws with this frame, `None` when it is hidden
    /// (`visible` false, no texture in the atlas).
    fn layer_paint(&self, l: &LayerDef, mr: &MobRender, ctx: &DrawCtx) -> Option<([f32; 4], u32)> {
        if l.visible.as_ref().is_some_and(|e| !e.eval_bool(mr.inputs())) {
            return None;
        }
        let name = l.texture.as_deref().or(self.texture.as_deref())?;
        let uv = (ctx.atlas)(name)?;
        let color = match &l.tint {
            Some(e) => {
                let rgb = e.eval(mr.inputs());
                if rgb.is_finite() { crate::mobs::render::abgr((rgb.max(0.0) as u32).min(0xFF_FFFF)) } else { WHITE }
            }
            None => WHITE,
        };
        Some((uv, color))
    }

    /// One pass over the (drawn) bones: every quad transformed by its bone's matrix.
    #[allow(clippy::too_many_arguments)]
    fn emit(
        &self,
        mr: &MobRender,
        ctx: &DrawCtx,
        only: Option<&[bool]>,
        uv: [f32; 4],
        color: u32,
        light: u32,
        overlay: u32,
        shade: Shade,
        out: &mut Vec<Vertex>,
    ) -> usize {
        let start = out.len();
        for (b, range) in self.bone_quads.iter().enumerate() {
            if range.is_empty()
                || !self.bone_drawn(b, ctx.lod, &mr.visibility)
                || only.is_some_and(|o| !o.get(b).copied().unwrap_or(false))
            {
                continue;
            }
            let Some(bone) = mr.matrices.get(b) else {
                continue;
            };
            let m = ctx.world_from_model * *bone;
            let m3 = glam::Mat3::from_mat4(m);
            let det = m3.determinant();
            if !det.is_finite() || det.abs() < 1.0e-18 {
                continue;
            }
            let normals = m3.inverse().transpose();
            for q in &self.quads[range.start as usize..range.end as usize] {
                let normal = match shade {
                    Shade::Lit => pack_normal((normals * Vec3::from(q.normal)).normalize_or_zero()),
                    Shade::Flat => [0; 4],
                };
                for k in 0..4 {
                    let p = m.transform_point3(Vec3::from(q.pos[k]));
                    out.push(Vertex {
                        pos: p.to_array(),
                        uv: [uv[0] + (uv[2] - uv[0]) * q.uv[k][0], uv[1] + (uv[3] - uv[1]) * q.uv[k][1]],
                        layer: overlay,
                        color,
                        light,
                        normal,
                        glint_uv: [0.0; 2],
                    });
                }
            }
        }
        out.len() - start
    }
}

use crate::mobs::render::FULL_BRIGHT;

// ----------------------------------------------------------------------------------------------
// Client animation events
// ----------------------------------------------------------------------------------------------

/// Where the routed client events go (the game plays them; tests record them).
pub trait EventSink {
    /// A sound at a world position; `id` is a vanilla event name or a namespaced one.
    fn sound(&mut self, id: &str, at: [f64; 3], volume: f32, pitch: f32);
    /// A particle at a world position; `id` is the id the type's `particles` table gave.
    fn particle(&mut self, id: &str, at: [f64; 3]);
    /// A `timeline` name (the `mob-anim-event` hook); cosmetic only.
    fn custom(&mut self, name: &str, at: [f64; 3]);
}

/// Hands the client events of one mob to `sink`, each at its locator: the position `event_position`
/// gives in the model's space (pixels, `matrices` being the pose) through the mob's transform
/// (feet `pos`, `scale`, `yaw`, death `roll`) at 1/16 block a pixel. Server-side events are ignored.
#[allow(clippy::too_many_arguments)]
pub fn route_events(
    ta: &TypeAnim,
    tr: &TypeRender,
    events: &[horizon_mobmodel::events::MobAnimEvent],
    matrices: &[Mat4],
    pos: [f64; 3],
    scale: f32,
    yaw: f32,
    roll: f32,
    sink: &mut dyn EventSink,
) {
    use horizon_mobmodel::events::{MobEventKind, event_position};
    for ev in events.iter().filter(|e| e.is_cosmetic()) {
        let at = model_point_to_world(pos, scale, yaw, roll, event_position(&tr.locators, matrices, ev));
        match ev.kind {
            MobEventKind::Sound { sound, volume, pitch, .. } => sink.sound(ta.events.string(sound), at, volume, pitch),
            MobEventKind::Particle { particle, .. } => sink.particle(ta.events.string(particle), at),
            MobEventKind::Custom { name, .. } => sink.custom(ta.events.string(name), at),
        }
    }
}

impl CustomRender {
    /// The render state of mob `id` (made on first use) and the local mirror, split so the pose can
    /// read the mirror's animation state while it updates the state.
    pub fn parts(&mut self, id: u32, t: &TypeRender) -> (&mut MobRender, &LocalMirror) {
        let frame = self.frame;
        let m = self.mobs.entry(id).or_insert_with(|| MobRender::new(t));
        m.used_frame = frame;
        (m, &self.mirror)
    }

    /// The bone matrices of mob `id` as last evaluated.
    pub fn matrices_of(&self, id: u32) -> Option<&[Mat4]> {
        self.mobs.get(&id).map(|m| m.matrices.as_slice())
    }

    /// The reusable event buffer (made on first use), taken out for the caller's loop.
    pub fn take_events(&mut self) -> horizon_mobmodel::events::EventBuf {
        self.events.take().unwrap_or_else(|| horizon_mobmodel::events::EventBuf::with_capacity(64))
    }

    pub fn put_events(&mut self, buf: horizon_mobmodel::events::EventBuf) {
        self.events = Some(buf);
    }
}

/// The embedded textures of every registered type as atlas images `(name, width, height, RGBA8)`.
/// Nothing (and no allocation) while no custom type exists.
pub fn embedded_textures() -> Vec<(String, u32, u32, Vec<u8>)> {
    let Ok(registry) = super::registry::registry().read() else {
        return Vec::new();
    };
    embedded_textures_of(registry.iter().filter_map(|t| t.render.as_ref()))
}

/// [`embedded_textures`] of the given types.
pub fn embedded_textures_of<'a>(types: impl Iterator<Item = &'a TypeRender>) -> Vec<(String, u32, u32, Vec<u8>)> {
    types
        .flat_map(|r| r.embedded.iter())
        .map(|e| (e.name.clone(), e.width, e.height, e.rgba.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::custom::anim_sync::{self, tests::root};
    use horizon_mobmodel::{
        events::{EventBuf, EventSide},
        skeleton::bone_matrices,
    };
    use horizon_protocol::mob_sync::{Frame, LayerAnchor, Message, Snapshot, Trigger, TriggerStart, ValueSet};
    use serde_json::Value;
    use std::sync::Arc;

    const FOX: &str = include_str!(
        "../../../../../tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json"
    );
    const BODY: &str = "sample_mobs:entity/cinder_fox";
    const GLOW: &str = "sample_mobs:entity/cinder_fox_glowmask";
    const ANIMATIONS: [&str; 6] = ["idle", "walk", "run", "sit", "pounce", "death"];
    const ENTITY: i32 = 7;

    /// The Cinder Fox as the game builds it (the real loaders over the fixture's files) with the type
    /// file and the animation file edited first; `geo_json` false makes the bbmodel the model.
    fn fox_with(
        geo_json: bool,
        edit_type: impl FnOnce(&mut Value),
        edit_anim: impl FnOnce(&mut Value),
    ) -> (Arc<CustomType>, LoadedModel) {
        let mut v: Value = serde_json::from_str(FOX).unwrap();
        edit_type(&mut v);
        let file = CustomMobFile::parse("sample_mobs:cinder_fox", v.to_string().as_bytes()).unwrap();
        let mut edit_anim = Some(edit_anim);
        let model = file
            .load_model(&mut |p| {
                if !geo_json && p.ends_with(".geo.json") {
                    return Err("no geo.json".into());
                }
                // The fixture mod's own bbmodel is a one-bone stub; the model crate's is the whole fox
                // with its texture embedded.
                let bytes = if p.ends_with(".bbmodel") {
                    std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../horizon-mobmodel/tests/fixtures/cinder_fox/cinder_fox.bbmodel"))
                } else {
                    std::fs::read(root().join(p))
                }
                .map_err(|e| e.to_string())?;
                if p.ends_with(".animation.json") {
                    let mut a: Value = serde_json::from_slice(&bytes).unwrap();
                    if let Some(edit) = edit_anim.take() {
                        edit(&mut a);
                    }
                    return Ok(a.to_string().into_bytes());
                }
                Ok(bytes)
            })
            .unwrap();
        (Arc::new(CustomType::new(file, Some(&model)).unwrap()), model)
    }

    fn fox() -> Arc<CustomType> {
        fox_with(true, |_| {}, |_| {}).0
    }

    /// The fox with a sound and a particle on the pounce's 0.35 s keyframe, next to its `hit`.
    fn fox_with_effects() -> Arc<CustomType> {
        fox_with(
            true,
            |v| v["model"]["particles"] = serde_json::json!({"flame": "minecraft:flame"}),
            |a| {
                let pounce = &mut a["animations"]["animation.cinder_fox.pounce"];
                pounce["sound_effects"] = serde_json::json!({"0.35": {"effect": "sample_mobs:entity.cinder_fox.pounce", "volume": 0.8, "pitch": 1.1, "locator": "mouth"}});
                pounce["particle_effects"] = serde_json::json!({"0.35": {"effect": "flame", "locator": "mouth", "offset": [0, 1, 0]}});
            },
        )
        .0
    }

    fn atlas(name: &str) -> Option<[f32; 4]> {
        match name {
            BODY => Some([0.0, 0.0, 0.5, 0.5]),
            GLOW => Some([0.5, 0.5, 1.0, 1.0]),
            "x:entity/overlay" => Some([0.25, 0.25, 0.75, 0.75]),
            _ => None,
        }
    }

    fn draw_ctx(lod: LodMode) -> DrawCtx<'static> {
        DrawCtx {
            world_from_model: world_from_model(Vec3::new(1.0, 2.0, 3.0), 1.0, 30.0, 0.0),
            light: 0x00F0_00F0,
            overlay: 0,
            lod,
            atlas: &atlas,
        }
    }

    /// FNV-1a over the vertices, quantised to 1e-4 so the hash does not move with float noise.
    fn hash(vertices: &[Vertex]) -> u64 {
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        let mut eat = |b: &[u8]| {
            for x in b {
                h ^= u64::from(*x);
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        let q = |f: f32| ((f * 1.0e4).round() as i32).to_le_bytes();
        for v in vertices {
            v.pos.iter().for_each(|f| eat(&q(*f)));
            v.uv.iter().for_each(|f| eat(&q(*f)));
            eat(&v.layer.to_le_bytes());
            eat(&v.color.to_le_bytes());
            eat(&v.light.to_le_bytes());
            eat(&v.normal.map(|n| n as u8));
        }
        h
    }

    fn pose_matrices(ct: &CustomType, name: &str, frame: usize) -> Vec<Mat4> {
        let rig = &ct.anim.as_ref().unwrap().rig;
        let idx = rig.find_animation(name).unwrap();
        let length = rig.animation(idx).unwrap().length;
        let mut inputs = ExprInputs::default();
        inputs.set(Query::GroundSpeed, if name == "walk" { 0.12 } else if name == "run" { 0.3 } else { 0.0 });
        let mut pose = Pose::default();
        rig.sample_animation(idx, length * frame as f32 / 30.0, &mut inputs, &mut pose);
        bone_matrices(&rig.skeleton, &pose)
    }

    fn context(tick: i64, frame: u64, lod: LodMode) -> PoseCtx {
        let facts = MobFacts { seed: 7, max_health: 14.0, health: 14.0, on_ground: true, ..Default::default() };
        let mut c = PoseCtx::new(tick, 0.0, facts);
        c.frame = frame;
        c.lod = lod;
        c
    }

    // ----- the baked quads ---------------------------------------------------------------------

    #[test]
    fn every_baked_triangle_pair_becomes_a_quad_of_its_bone() {
        let (ct, loaded) = fox_with(true, |_| {}, |_| {});
        let tr = ct.render.as_ref().expect("the fox has a model");
        let baked = loaded.model.as_ref().unwrap().bake();
        assert_eq!(tr.bone_quads.len(), baked.bones.len());
        let triangles: usize = baked.bones.iter().map(|b| b.indices.len() / 3).sum();
        assert!(tr.quads.len() * 2 >= triangles && tr.quads.len() <= triangles, "{} quads for {triangles} triangles", tr.quads.len());
        let mut end = 0;
        for (r, b) in tr.bone_quads.iter().zip(&baked.bones) {
            assert_eq!(r.start, end, "ranges are contiguous");
            end = r.end;
            assert!((r.end - r.start) as usize <= b.indices.len() / 3);
        }
        assert_eq!(end as usize, tr.quads.len());
        assert!(tr.quads.iter().all(|q| (q.normal.iter().map(|c| c * c).sum::<f32>() - 1.0).abs() < 1e-3));
    }

    // ----- golden vertices ---------------------------------------------------------------------

    /// The vertices of the fox at frame 10 of 30 of each animation, hashed. The hashes were produced
    /// by this code (`F4M6_PRINT=1` shows them) and pin today's pose-to-vertex pipeline: the half turn,
    /// the bone matrices, the UV mapping and the packed normals. They were not checked against Blockbench.
    #[test]
    fn golden_vertex_hashes_of_every_animation() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        let mut hashes = Vec::new();
        let mut counts = Vec::new();
        for name in ANIMATIONS {
            let mut mr = MobRender::new(tr);
            mr.matrices = pose_matrices(&ct, name, 10);
            let mut out = Vec::new();
            let stats = tr.draw(&mr, &draw_ctx(LodMode::Full), &mut out);
            assert_eq!(stats.total(), out.len());
            hashes.push(hash(&out));
            counts.push(out.len());
        }
        if std::env::var("F4M6_PRINT").is_ok() {
            eprintln!("GOLDEN {hashes:#x?} {counts:?}");
        }
        assert!(counts.iter().all(|c| *c == counts[0] && *c > 0), "{counts:?}");
        assert_eq!(hashes, GOLDEN, "actual: {hashes:#x?}");
        for i in 0..hashes.len() {
            for j in i + 1..hashes.len() {
                assert_ne!(hashes[i], hashes[j], "{} and {} draw the same", ANIMATIONS[i], ANIMATIONS[j]);
            }
        }
    }

    const GOLDEN: [u64; 6] = [
        0x353dbaed63db8531,
        0x33c72f6e9876c6d1,
        0x9b7713a88e64bc79,
        0x85bf4e3aa6dca16d,
        0x580fc609b270509d,
        0xe6b3bbd96da64755,
    ];

    // ----- layers ------------------------------------------------------------------------------

    #[test]
    fn the_fox_splits_into_a_cutout_body_and_an_emissive_glow_layer() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        let mr = MobRender::new(tr);
        let mut out = Vec::new();
        let stats = tr.draw(&mr, &draw_ctx(LodMode::Full), &mut out);
        let n = tr.quads.len() * 4;
        assert_eq!(stats, DrawStats { cutout: n, translucent: 0, emissive: n });
        assert_eq!(out.len(), n * 2, "the explicit glow layer is not drawn twice");
        let (body, glow) = out.split_at(n);
        assert!(body.iter().all(|v| v.light == 0x00F0_00F0 && v.normal[3] == -127 && v.color == 0xFFFF_FFFF));
        assert!(glow.iter().all(|v| v.light == FULL_BRIGHT && v.layer == 0));
        let inside = |v: &Vertex, r: [f32; 4]| v.uv[0] >= r[0] - 1e-6 && v.uv[0] <= r[2] + 1e-6 && v.uv[1] >= r[1] - 1e-6 && v.uv[1] <= r[3] + 1e-6;
        assert!(body.iter().all(|v| inside(v, [0.0, 0.0, 0.5, 0.5])));
        assert!(glow.iter().all(|v| inside(v, [0.5, 0.5, 1.0, 1.0])));
    }

    #[test]
    fn a_glowmask_beside_the_texture_is_an_emissive_layer_without_a_layer_row() {
        let ct = fox_with(true, |v| v["model"]["layers"] = serde_json::json!([]), |_| {}).0;
        let tr = ct.render.as_ref().unwrap();
        assert!(tr.layers.is_empty());
        let n = tr.quads.len() * 4;
        let mut out = Vec::new();
        assert_eq!(tr.draw(&MobRender::new(tr), &draw_ctx(LodMode::Full), &mut out), DrawStats { cutout: n, translucent: 0, emissive: n });
        // Without the mask in the atlas there is nothing to add.
        let none = |name: &str| (name == BODY).then_some([0.0, 0.0, 1.0, 1.0]);
        let ctx = DrawCtx { atlas: &none, ..draw_ctx(LodMode::Full) };
        out.clear();
        assert_eq!(tr.draw(&MobRender::new(tr), &ctx, &mut out), DrawStats { cutout: n, translucent: 0, emissive: 0 });
    }

    #[test]
    fn translucent_and_additive_layers_sort_after_the_opaque_ones_and_are_tagged() {
        let layers = serde_json::json!([
            {"name": "ghost", "texture": "x:entity/overlay", "mode": "translucent"},
            {"name": "eyes", "texture": "x:entity/overlay", "mode": "additive", "bones": ["head"]},
            {"name": "glow", "texture": GLOW, "mode": "emissive"},
            {"name": "hidden", "texture": "x:entity/overlay", "mode": "translucent", "visible": "0"},
            {"name": "tinted", "texture": "x:entity/overlay", "mode": "cutout", "tint": "16711680", "bones": ["tail_1"]}
        ]);
        let ct = fox_with(true, |v| v["model"]["layers"] = layers, |_| {}).0;
        let tr = ct.render.as_ref().unwrap();
        let mut out = Vec::new();
        let stats = tr.draw(&MobRender::new(tr), &draw_ctx(LodMode::Full), &mut out);
        let n = tr.quads.len() * 4;
        let share = |name: &str| {
            let bones = tr.layers.iter().find(|l| l.name == name).unwrap().bones.clone().unwrap();
            tr.bone_quads.iter().enumerate().filter(|(i, _)| bones[*i]).map(|(_, r)| (r.end - r.start) as usize * 4).sum::<usize>()
        };
        assert!(share("eyes") > 0 && share("tinted") > 0 && share("eyes") < n);
        assert_eq!(stats.emissive, n);
        assert_eq!(stats.cutout, n + share("tinted"));
        assert_eq!(stats.translucent, n + share("eyes"), "the hidden layer draws nothing");
        let (opaque, blended) = out.split_at(stats.cutout + stats.emissive);
        assert!(opaque.iter().all(|v| v.layer & crate::gfx::OIT_ENTITY == 0));
        assert!(blended.iter().all(|v| v.layer & crate::gfx::OIT_ENTITY != 0));
        let (ghost, eyes) = blended.split_at(n);
        assert!(ghost.iter().all(|v| v.light == 0x00F0_00F0 && v.normal[3] == -127));
        let tier = u32::from(crate::mesher::BRIGHT) << 16;
        assert!(eyes.iter().all(|v| v.light == FULL_BRIGHT && v.normal == [0; 4] && v.layer & tier == tier));
        let tinted = &out[2 * n..2 * n + share("tinted")];
        assert!(tinted.iter().all(|v| v.color == crate::mobs::render::abgr(0xFF0000)));
    }

    #[test]
    fn the_hurt_overlay_goes_to_the_body_and_cutout_layers_not_the_glow() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        let ctx = DrawCtx { overlay: 1, ..draw_ctx(LodMode::Full) };
        let mut out = Vec::new();
        tr.draw(&MobRender::new(tr), &ctx, &mut out);
        let n = tr.quads.len() * 4;
        assert!(out[..n].iter().all(|v| v.layer == 1));
        assert!(out[n..].iter().all(|v| v.layer == 0));
    }

    // ----- LOD ---------------------------------------------------------------------------------

    #[test]
    fn the_lod_table_picks_the_last_row_reached() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        let at = |d: f32| tr.lod_mode(d);
        assert_eq!(at(0.0), LodMode::Full);
        assert_eq!(at(23.99), LodMode::Full);
        assert_eq!(at(24.0), LodMode::HalfRate);
        assert_eq!(at(47.9), LodMode::HalfRate);
        assert_eq!(at(48.0), LodMode::QuarterRate);
        assert_eq!(at(64.0), LodMode::Static);
        assert_eq!(at(10_000.0), LodMode::Static);
    }

    #[test]
    fn half_and_quarter_rate_evaluate_every_second_and_fourth_frame_and_hold_in_between() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        let run = |lod: LodMode| {
            let mut mr = MobRender::new(tr);
            let mut evaluated = Vec::new();
            for frame in 0..16u64 {
                if mr.update(&ct, tr, None, &context(frame as i64, frame, lod)) {
                    evaluated.push(frame);
                }
            }
            (mr.evaluations, evaluated)
        };
        assert_eq!(run(LodMode::Full).0, 16);
        let (half, frames) = run(LodMode::HalfRate);
        // The first frame always evaluates; then the frames of the mob's phase (id 7 is odd).
        assert_eq!(frames[0], 0);
        assert!(frames[1..].iter().all(|f| (f + 7) % 2 == 0), "{frames:?}");
        assert_eq!(half, 1 + 8);
        let (quarter, frames) = run(LodMode::QuarterRate);
        assert!(frames[1..].iter().all(|f| (f + 7) % 4 == 0), "{frames:?}");
        assert_eq!(quarter, 1 + 4);
        assert_eq!(run(LodMode::Hidden).0, 0);
        assert_eq!(run(LodMode::Static).0, 0, "the rest pose is taken once, the rig never runs");
    }

    #[test]
    fn static_is_the_rest_pose_and_hidden_draws_nothing() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        let mut mr = MobRender::new(tr);
        mr.matrices = pose_matrices(&ct, "run", 7);
        assert!(mr.update(&ct, tr, None, &context(0, 0, LodMode::Static)));
        assert_eq!(mr.matrices, tr.rest_matrices);
        let mut out = Vec::new();
        assert_eq!(tr.draw(&mr, &draw_ctx(LodMode::Hidden), &mut out).total(), 0);
        assert!(out.is_empty());
    }

    #[test]
    fn low_bones_leaves_out_the_lod_min_bones_and_hidden_bones_go_at_every_lod() {
        let (ct, loaded) = fox_with(true, |_| {}, |_| {});
        let mut tr = TypeRender::new(&ct.file, &loaded).unwrap();
        let tail = tr.skeleton.find("tail_3").unwrap().index();
        tr.lod_min[tail] = true;
        let mr = MobRender::new(&tr);
        let (mut full, mut low) = (Vec::new(), Vec::new());
        tr.draw(&mr, &draw_ctx(LodMode::Full), &mut full);
        tr.draw(&mr, &draw_ctx(LodMode::LowBones), &mut low);
        let tail_vertices = (tr.bone_quads[tail].end - tr.bone_quads[tail].start) as usize * 4;
        assert!(tail_vertices > 0);
        assert_eq!(full.len() - low.len(), tail_vertices * 2, "body and glow");
        let mut mr = MobRender::new(&tr);
        mr.visibility.set_visible(tail, false);
        let mut hidden = Vec::new();
        tr.draw(&mr, &draw_ctx(LodMode::Full), &mut hidden);
        assert_eq!(hidden.len(), low.len());
        assert!("fur_lod_min".ends_with(LOD_MIN_SUFFIX));
    }

    #[test]
    fn model_lod_min_bones_and_the_name_suffix_are_both_skipped_by_low_bones() {
        let plain = fox().render.as_ref().unwrap().lod_min.iter().filter(|b| **b).count();
        assert_eq!(plain, 0, "the fixture has no tagged bone");
        let (ct, loaded) = fox_with(true, |v| v["model"]["lod_min_bones"] = serde_json::json!(["tail_3", "tail_2"]), |_| {});
        let tr = TypeRender::new(&ct.file, &loaded).unwrap();
        let (t2, t3) = (tr.skeleton.find("tail_2").unwrap().index(), tr.skeleton.find("tail_3").unwrap().index());
        assert!(tr.lod_min[t2] && tr.lod_min[t3]);
        assert_eq!(tr.lod_min.iter().filter(|b| **b).count(), 2);
        let mr = MobRender::new(&tr);
        let (mut full, mut low) = (Vec::new(), Vec::new());
        tr.draw(&mr, &draw_ctx(LodMode::Full), &mut full);
        tr.draw(&mr, &draw_ctx(LodMode::LowBones), &mut low);
        let verts = |b: usize| (tr.bone_quads[b].end - tr.bone_quads[b].start) as usize * 4;
        assert_eq!(full.len() - low.len(), (verts(t2) + verts(t3)) * 2);
        // The suffix still works beside the list: a bone named `*_lod_min` joins the union.
        assert!(!tr.bone_drawn(t3, LodMode::LowBones, &mr.visibility) && tr.bone_drawn(t3, LodMode::Full, &mr.visibility));
    }

    // ----- death ---------------------------------------------------------------------------------

    fn dead_matrices(ct: &Arc<CustomType>, death_time: f32, partial: f32) -> Vec<Mat4> {
        let tr = ct.render.as_ref().unwrap();
        let mut mr = MobRender::new(tr);
        let mut c = context(40, 0, LodMode::Full);
        c.partial = partial;
        c.facts.death_time = death_time;
        c.facts.health = 0.0;
        assert!(mr.update(ct, tr, None, &c));
        mr.matrices.clone()
    }

    #[test]
    fn a_death_animation_replaces_the_tilt_and_holds_its_last_pose() {
        let ct = fox();
        let ta = ct.anim.as_ref().unwrap();
        // The fixture's death is 0.6 s: 12 ticks, kept 20. Alive or without the animation there is no clock.
        assert_eq!(ct.death_ticks(), Some(20));
        assert_eq!(death_clock(ta, 0.0, 0.5), None);
        assert_eq!(death_clock(ta, 1.0, 0.0), Some(0.0));
        assert!((death_clock(ta, 7.0, 0.5).unwrap() - 6.5 * TICK_SECONDS).abs() < 1e-6);
        let end = death_clock(ta, 40.0, 0.0).unwrap();
        assert!(end < 0.6 && end > 0.59, "held on the last pose: {end}");
        // The pose follows the clock, then stays.
        let (a, b, c) = (dead_matrices(&ct, 3.0, 0.0), dead_matrices(&ct, 8.0, 0.0), dead_matrices(&ct, 13.0, 0.0));
        let (d, e) = (dead_matrices(&ct, 20.0, 0.0), dead_matrices(&ct, 59.0, 0.5));
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(c, d, "past the end it holds");
        assert_eq!(d, e);
        let rest = ct.render.as_ref().unwrap().rest_matrices.clone();
        assert_ne!(c, rest, "the last pose is not the rest pose");
        // No tilt for a type with the animation; the engine's tilt for one without.
        assert_eq!(death_roll_for(&ct, 10, 0.0), 0.0);
        let plain = fox_with(
            true,
            |_| {},
            |a| drop(a["animations"].as_object_mut().unwrap().remove("animation.cinder_fox.death")),
        )
        .0;
        assert_eq!(plain.death_ticks(), None);
        assert_eq!(death_roll_for(&plain, 10, 0.25), death_roll_degrees(10, 0.25));
        assert!(death_roll_for(&plain, 10, 0.25) > 0.0);
        // A living mob of either type is not rolled and has no death clock.
        assert_eq!(death_roll_for(&plain, 0, 0.0), 0.0);
    }

    // ----- cull box and shadow -----------------------------------------------------------------

    #[test]
    fn the_cull_box_is_the_types_own_box_or_the_hitbox() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        // `cull: {width: 2, height: 2}` is not the 0.6 x 0.7 hitbox.
        assert_eq!(tr.cull_box(0.6, 0.7, 1.0), ([-1.0, 0.0, -1.0], [1.0, 2.0, 1.0]));
        assert_eq!(tr.cull_box(0.6, 0.7, 0.5), ([-0.5, 0.0, -0.5], [0.5, 1.0, 0.5]));
        let plain = fox_with(true, |v| drop(v["model"].as_object_mut().unwrap().remove("cull")), |_| {}).0;
        assert_eq!(plain.render.as_ref().unwrap().cull_box(0.6, 0.7, 1.0), ([-0.3, 0.0, -0.3], [0.3, 0.7, 0.3]));
        let raised = fox_with(true, |v| v["model"]["cull"] = serde_json::json!({"width": 1, "height": 3, "y_offset": -0.5}), |_| {}).0;
        assert_eq!(raised.render.as_ref().unwrap().cull_box(0.6, 0.7, 1.0), ([-0.5, -0.5, -0.5], [0.5, 2.5, 0.5]));
        assert_eq!(tr.shadow, Some((0.4, 0.8)));
    }

    // ----- locators and the mob's transform ----------------------------------------------------

    #[test]
    fn locators_and_the_mob_transform_put_the_mouth_in_front() {
        let (ct, loaded) = fox_with(true, |_| {}, |_| {});
        let tr = ct.render.as_ref().unwrap();
        let mouth = tr.locators.iter().position(|l| l.name == "mouth").unwrap();
        assert_eq!(tr.item_slots.mainhand, Some(mouth));
        let p = tr.locator_point(mouth, &tr.rest_matrices).unwrap();
        let rest = loaded.locator_rest_position("mouth").unwrap();
        assert!((f64::from(p.x) - rest[0]).abs() < 1e-4 && (f64::from(p.y) - rest[1]).abs() < 1e-4 && (f64::from(p.z) - rest[2]).abs() < 1e-4);
        assert!(p.z < -8.0, "the model's front is -Z: {p:?}");
        // At yaw 0 the mob faces +Z: the half turn puts the mouth ahead of the feet.
        let at = model_point_to_world([10.0, 64.0, 20.0], 1.0, 0.0, 0.0, p);
        assert!((at[0] - 10.0).abs() < 1e-4 && (at[2] - (20.0 - f64::from(p.z) / 16.0)).abs() < 1e-4 && at[2] > 20.5, "{at:?}");
        // A yaw of 90 degrees faces -X (west).
        let west = model_point_to_world([10.0, 64.0, 20.0], 1.0, 90.0, 0.0, p);
        assert!(west[0] < 9.5 && (west[2] - 20.0).abs() < 1e-4, "{west:?}");
        let big = model_point_to_world([0.0; 3], 2.0, 0.0, 0.0, p);
        assert!((big[2] - 2.0 * f64::from(-p.z) / 16.0).abs() < 1e-4);
        let item = tr.locator_frame(mouth, &tr.rest_matrices).unwrap();
        assert!((item.w_axis.truncate() - p).length() < 1e-4);
        assert!((item.x_axis.truncate().length() - 1.0).abs() < 1e-4, "orientation only, no scale");
        // The vertex output's matrix agrees with the point transform.
        let q = world_from_model(Vec3::ZERO, 1.0, 90.0, 0.0).transform_point3(p);
        assert!((f64::from(q.x) - (west[0] - 10.0)).abs() < 1e-4 && (f64::from(q.z) - (west[2] - 20.0)).abs() < 1e-4);
        assert!(death_roll_degrees(0, 0.0) == 0.0 && (death_roll_degrees(20, 0.0) - 90.0).abs() < 1e-3);
    }

    #[test]
    fn the_view_point_is_the_camera_in_model_pixels() {
        // A camera one block in front of a mob facing +Z (yaw 0) is 16 px ahead: model -Z.
        let p = model_point_of(Vec3::new(0.0, 0.5, 1.0), 1.0, 0.0, 0.0);
        assert!((p - Vec3::new(0.0, 8.0, -16.0)).length() < 1e-3, "{p:?}");
        // A quarter turn to the west: a camera at -X is ahead.
        let p = model_point_of(Vec3::new(-1.0, 0.0, 0.0), 1.0, 90.0, 0.0);
        assert!((p - Vec3::new(0.0, 0.0, -16.0)).length() < 1e-3, "{p:?}");
    }

    // ----- embedded textures -------------------------------------------------------------------

    #[test]
    fn a_bbmodels_embedded_texture_goes_to_the_atlas_under_the_default_texture_name() {
        let ct = fox_with(false, |_| {}, |_| {}).0;
        let tr = ct.render.as_ref().unwrap();
        assert_eq!(tr.embedded.len(), 1);
        let e = &tr.embedded[0];
        assert_eq!(e.name, BODY);
        assert!(e.width > 0 && e.rgba.len() == (e.width * e.height * 4) as usize);
        assert_eq!(embedded_textures_of([tr].into_iter())[0].0, BODY);
        assert!(fox().render.as_ref().unwrap().embedded.is_empty(), "the geo.json route has the file");
        assert_eq!(default_texture_name("sample_mobs:cinder_fox"), "sample_mobs:entity/cinder_fox");
    }

    // ----- the pose pipeline -------------------------------------------------------------------

    /// A store holding one fox with `anchors` and `triggers` as of a snapshot received at `arrival`.
    fn store(ct: &Arc<CustomType>, anchors: Vec<LayerAnchor>, triggers: Vec<Trigger>, arrival: i64) -> RemoteCustomMobs {
        let mut s = RemoteCustomMobs::default();
        let mut f = Frame::new(arrival);
        f.messages.push(Message::Snapshot(Snapshot {
            entity: ENTITY,
            type_version: ct.sync_version,
            values: ValueSet::default(),
            anchors,
            triggers,
            hitbox_mask: 3,
        }));
        let t = ct.clone();
        s.apply_at(&f, arrival, &move |_| Some(t.clone()));
        s
    }

    fn anchor(layer: u8, state: u8, tick: i64, time: f32, speed: f32) -> LayerAnchor {
        LayerAnchor { layer, state, tick, slots: vec![(time, speed)] }
    }

    #[test]
    fn a_late_joined_mob_shows_the_pose_of_one_watched_all_along() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        // locomotion idle (state 0) and the pose layer sitting (state 1), anchored at tick 100.
        let anchors = vec![anchor(0, 0, 100, 0.0, 1.0), anchor(2, 1, 100, 0.0, 1.0)];
        let watched = store(&ct, anchors.clone(), vec![], 100);
        let mut a = MobRender::new(tr);
        // Watched from before: first seen with no state at all (the rest), then it sits down.
        for tick in 90..100 {
            a.update(&ct, tr, None, &context(tick, tick as u64, LodMode::Full));
        }
        for tick in 100..=200 {
            a.update(&ct, tr, watched.get(ENTITY), &context(tick, tick as u64, LodMode::Full));
        }
        let joined = store(&ct, anchors, vec![], 200);
        let mut b = MobRender::new(tr);
        b.update(&ct, tr, joined.get(ENTITY), &context(200, 200, LodMode::Full));
        assert_eq!(a.matrices.len(), b.matrices.len());
        for (x, y) in a.matrices.iter().zip(&b.matrices) {
            assert!(x.abs_diff_eq(*y, 1e-4), "{x:?} vs {y:?}");
        }
        // And it is not the rest pose: sitting moves the hips.
        assert!(a.matrices.iter().zip(&tr.rest_matrices).any(|(x, r)| !x.abs_diff_eq(*r, 1e-3)));
    }

    #[test]
    fn a_state_change_fades_over_the_blend_not_instantly() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        // Locomotion: walking (state 1), then idle (state 0), whose `blend` is 0.15 s.
        let before = store(&ct, vec![anchor(0, 1, 0, 0.3, 1.0)], vec![], 0);
        let after = store(&ct, vec![anchor(0, 0, 40, 0.0, 1.0)], vec![], 40);
        let mut m = MobRender::new(tr);
        m.update(&ct, tr, before.get(ENTITY), &context(39, 0, LodMode::Full));
        let old = m.matrices.clone();
        m.update(&ct, tr, after.get(ENTITY), &context(40, 1, LodMode::Full));
        let first = m.matrices.clone();
        for tick in 41..80 {
            m.update(&ct, tr, after.get(ENTITY), &context(tick, tick as u64, LodMode::Full));
        }
        let settled = m.matrices.clone();
        let differs = |a: &[Mat4], b: &[Mat4]| a.iter().zip(b).any(|(x, y)| !x.abs_diff_eq(*y, 1e-3));
        assert!(differs(&old, &first), "the pose has moved toward idle");
        assert!(differs(&first, &settled), "but it is not there yet: the 0.15 s blend runs on the client's frame time");
    }

    #[test]
    fn the_looking_constraint_follows_this_clients_view() {
        // The head follows the viewer (the procedural fixture of horizon-mobmodel names bones of another rig).
        let procedural = serde_json::json!([
            {"type": "look_at", "bone": "head", "target": "view", "yaw": 60, "pitch": 40, "speed": 360}
        ]);
        let ct = fox_with(true, |t| t["model"]["procedural"] = procedural, |_| {}).0;
        let tr = ct.render.as_ref().unwrap();
        assert!(tr.procedural.is_some(), "the fixture constraints bind to the fox");
        let head = tr.skeleton.find("head").unwrap().index();
        let pose_for = |view: Option<Vec3>| {
            let mut mr = MobRender::new(tr);
            for tick in 0..30 {
                let mut c = context(tick, tick as u64, LodMode::Full);
                c.view = view;
                mr.update(&ct, tr, None, &c);
            }
            mr.matrices[head]
        };
        let ahead = pose_for(Some(Vec3::new(0.0, 5.0, -80.0)));
        let side = pose_for(Some(Vec3::new(-80.0, 5.0, 0.0)));
        assert!(!ahead.abs_diff_eq(side, 1e-3), "the head turns toward a viewer on the side");
        assert!(pose_for(None).abs_diff_eq(pose_for(None), 1e-6), "deterministic");
    }

    #[test]
    fn script_writes_hide_and_move_bones() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        let ear = tr.skeleton.find("ear_l").unwrap().index();
        let mut mr = MobRender::new(tr);
        let mut w = BoneWrite::new(ear as u32, horizon_mobmodel::procedural::WriteMode::Additive);
        w.visible = Some(false);
        w.translation = Some(Vec3::new(0.0, 4.0, 0.0));
        mr.writes.push(w);
        mr.update(&ct, tr, None, &context(0, 0, LodMode::Full));
        assert!(mr.writes.is_empty(), "consumed");
        assert!(!mr.visibility.is_visible(ear));
        let moved = mr.matrices[ear].w_axis.y - MobRender::new(tr).matrices[ear].w_axis.y;
        assert!(moved > 3.0, "{moved}");
    }

    #[test]
    fn mob_pose_writes_of_a_script_move_and_hide_bones_and_only_a_flagged_type_asks() {
        use crate::mobs::custom::script::{MobBoneWrite, ScriptVec3, bone_writes};
        let plain = fox();
        assert!(!plain.render.as_ref().unwrap().script_pose, "the fixture does not ask for scripts");
        let (ct, _) = fox_with(true, |v| v["model"]["procedural_script"] = serde_json::json!(true), |_| {});
        let tr = ct.render.as_ref().unwrap();
        assert!(tr.script_pose);
        let ear = tr.skeleton.find("ear_l").unwrap().index();
        let listed = vec![
            MobBoneWrite { bone: ear as u32, additive: true, translation: Some(ScriptVec3 { x: 0.0, y: 4.0, z: 0.0 }), visible: Some(false), ..Default::default() },
            MobBoneWrite { bone: 9999, ..Default::default() },
        ];
        let mut mr = MobRender::new(tr);
        mr.writes = bone_writes(&listed, tr.skeleton.len());
        assert_eq!(mr.writes.len(), 1, "a bone the model lacks is dropped");
        mr.update(&ct, tr, None, &context(0, 0, LodMode::Full));
        assert!(!mr.visibility.is_visible(ear));
        assert!(mr.matrices[ear].w_axis.y - MobRender::new(tr).matrices[ear].w_axis.y > 3.0);
        // The pose is only asked for when it is evaluated: `due` is false for a held LOD frame.
        assert!(mr.due(LodMode::Full, 5, 1));
        assert!(!mr.due(LodMode::Static, 5, 1));
        assert!(!mr.due(LodMode::Hidden, 5, 1));
    }

    // ----- client events -----------------------------------------------------------------------

    #[derive(Default)]
    struct Rec(Vec<String>, Vec<[f64; 3]>);
    impl EventSink for Rec {
        fn sound(&mut self, id: &str, at: [f64; 3], volume: f32, pitch: f32) {
            self.0.push(format!("sound {id} {volume:.2} {pitch:.2}"));
            self.1.push(at);
        }
        fn particle(&mut self, id: &str, at: [f64; 3]) {
            self.0.push(format!("particle {id}"));
            self.1.push(at);
        }
        fn custom(&mut self, name: &str, at: [f64; 3]) {
            self.0.push(format!("custom {name}"));
            self.1.push(at);
        }
    }

    /// The client's per-tick event poll of the store's fox over `ticks`, routed into a recorder:
    /// `(tick, what, where)` for everything that fired.
    fn poll(ct: &Arc<CustomType>, s: &mut RemoteCustomMobs, ticks: std::ops::Range<i64>, pos: [f64; 3]) -> Vec<(i64, String, [f64; 3])> {
        let (tr, ta) = (ct.render.as_ref().unwrap(), ct.anim.as_ref().unwrap());
        let mut buf = EventBuf::with_capacity(32);
        let mut out = Vec::new();
        for tick in ticks {
            buf.clear();
            s.get_mut(ENTITY).unwrap().poll_events(tick, 0.0, &mut buf);
            let mut rec = Rec::default();
            route_events(ta, tr, buf.as_slice(), &tr.rest_matrices, pos, 1.0, 0.0, 0.0, &mut rec);
            out.extend(rec.0.into_iter().zip(rec.1).map(|(w, at)| (tick, w, at)));
        }
        out
    }

    #[test]
    fn a_pounce_plays_its_hit_sound_and_particle_at_the_mouth_on_the_keyframes_tick() {
        let ct = fox_with_effects();
        let id = ct.anim.as_ref().unwrap().trigger_id("pounce").unwrap();
        // The server started the pounce at start tick 99 and the client hears of it in tick 100.
        let mut s = store(&ct, vec![], vec![], 90);
        let mut f = Frame::new(100);
        f.messages.push(Message::Trigger(TriggerStart { entity: ENTITY, trigger: Trigger { id, start_tick: 99, speed: 16 } }));
        let t = ct.clone();
        s.apply_at(&f, 100, &move |_| Some(t.clone()));
        let pos = [10.0, 64.0, 20.0];
        let fired = poll(&ct, &mut s, 100..140, pos);
        let of = |what: &str| fired.iter().filter(|(_, w, _)| w.starts_with(what)).collect::<Vec<_>>();
        let (sound, particle, hit) = (of("sound"), of("particle"), of("custom"));
        assert_eq!((sound.len(), particle.len(), hit.len()), (1, 1, 1), "{fired:?}");
        assert_eq!(sound[0].0, particle[0].0);
        assert_eq!(sound[0].0, hit[0].0);
        assert_eq!(sound[0].0, 99 + 8, "the 8th tick after the start, as the server's frame");
        assert_eq!(sound[0].1, "sound sample_mobs:entity.cinder_fox.pounce 0.80 1.10");
        assert_eq!(particle[0].1, "particle minecraft:flame");
        assert_eq!(hit[0].1, "custom hit");
        // Where: the mouth locator in front of the fox; the particle one pixel above it.
        let tr = ct.render.as_ref().unwrap();
        let mouth = tr.locators.iter().position(|l| l.name == "mouth").unwrap();
        let m = tr.locator_point(mouth, &tr.rest_matrices).unwrap();
        let close = |a: [f64; 3], b: [f64; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
        assert!(close(sound[0].2, model_point_to_world(pos, 1.0, 0.0, 0.0, m)), "{:?}", sound[0].2);
        assert_eq!(hit[0].2, pos, "a timeline name has no locator: the model origin, the feet");
        let bone = tr.locators[mouth].bone.index();
        let above = m + tr.rest_matrices[bone].transform_vector3(Vec3::Y);
        assert!(close(particle[0].2, model_point_to_world(pos, 1.0, 0.0, 0.0, above)), "{:?}", particle[0].2);
        // Nothing plays again afterwards.
        assert!(poll(&ct, &mut s, 140..160, pos).is_empty());
    }

    #[test]
    fn a_pounce_joined_midway_replays_nothing_of_its_past() {
        let ct = fox_with_effects();
        let id = ct.anim.as_ref().unwrap().trigger_id("pounce").unwrap();
        // The client first hears of the pounce (in a snapshot) 0.5 s after its start: the keyframe is past.
        let mut s = store(&ct, vec![], vec![Trigger { id, start_tick: 99, speed: 16 }], 110);
        let fired = poll(&ct, &mut s, 110..140, [0.0; 3]);
        assert!(fired.is_empty(), "{fired:?}");
    }

    #[test]
    fn events_of_the_server_side_are_not_routed() {
        let ct = fox_with_effects();
        let (tr, ta) = (ct.render.as_ref().unwrap(), ct.anim.as_ref().unwrap());
        let mut buf = EventBuf::with_capacity(8);
        let (layer, trig) = (ta.triggers[0].layer, ta.triggers[0].index);
        ta.events.trigger_window(&ta.rig, EventSide::Server, layer, trig, 0.0, 1.0, &mut buf);
        assert!(!buf.is_empty(), "the server's `hit`");
        let mut rec = Rec::default();
        route_events(ta, tr, buf.as_slice(), &tr.rest_matrices, [0.0; 3], 1.0, 0.0, 0.0, &mut rec);
        assert!(rec.0.is_empty(), "gameplay is not cosmetic: {:?}", rec.0);
    }

    // ----- the integrated server's own mobs ----------------------------------------------------

    /// A fox the server ticks and its mirror: the pounce starts on the server and the client side of
    /// the same process plays the same keyframe in the same tick as a remote client would.
    #[test]
    fn a_local_mob_is_mirrored_and_its_pounce_events_fire_once() {
        use crate::mobs::custom::ai::test_support::flat;
        let ct = fox_with_effects();
        let mut mob = anim_sync::tests::fox_mob(&ct, horizon_core::geometry::Vec3::new(0.5, 1.0, 0.5));
        let fx = flat();
        let mut mirror = LocalMirror::default();
        let mut fired = Vec::new();
        let (tr, ta) = (ct.render.as_ref().unwrap(), ct.anim.as_ref().unwrap());
        let mut buf = EventBuf::with_capacity(32);
        for tick in 0..60i64 {
            let mut w = fx.world();
            w.game_time = tick;
            anim_sync::tick(&mut mob, &w);
            if tick == 10 {
                assert!(anim_sync::request_trigger(&mut mob, "pounce", 1.0));
            }
            mirror.sync(&mob, tick, 1, None);
            if let Some(m) = mirror.get_mut(mob.id) {
                buf.clear();
                m.poll_events(tick, 0.0, &mut buf);
                let mut rec = Rec::default();
                route_events(ta, tr, buf.as_slice(), &tr.rest_matrices, [0.5, 1.0, 0.5], 1.0, 0.0, 0.0, &mut rec);
                fired.extend(rec.0.into_iter().map(|w| (tick, w)));
            }
        }
        let sounds: Vec<_> = fired.iter().filter(|(_, w)| w.starts_with("sound")).collect();
        assert_eq!(sounds.len(), 1, "{fired:?}");
        // Asked for in tick 10, started by tick 11 (start tick 10), the keyframe 8 ticks later.
        assert_eq!(sounds[0].0, 10 + 8);
        assert_eq!(mirror.get(mob.id).unwrap().values, mob.extra.custom.as_ref().unwrap().synced);
        mirror.retain(2);
        assert!(mirror.is_empty(), "a mob that is gone is forgotten");
    }

    /// The integrated host reads synced values as a remote client would: a `sync: owner` value of a
    /// mob the host's player does not own is its default, the owner's window sees it (F4M-4c).
    #[test]
    fn the_hosts_mirror_hides_owner_values_of_mobs_the_host_does_not_own() {
        use crate::mobs::custom::{SyncedValue, ai::tame};
        const HOST: [i32; 4] = [7, 8, 9, 10];
        const OTHER: [i32; 4] = [1, 2, 3, 4];
        let ct = fox_with(
            true,
            |v| v["synced"]["mood"] = serde_json::json!({"type": "i32", "min": 0, "max": 100, "default": 5, "sync": "owner"}),
            |_| {},
        )
        .0;
        let slot = ct.synced_slot("mood").unwrap();
        let mut mob = anim_sync::tests::fox_mob(&ct, horizon_core::geometry::Vec3::new(0.5, 1.0, 0.5));
        mob.extra.custom.as_mut().unwrap().synced[slot] = SyncedValue::I32(40);
        let seen = |viewer: Option<[i32; 4]>, mob: &Mob| {
            let mut mirror = LocalMirror::default();
            mirror.sync(mob, 0, 1, viewer);
            mirror.get(mob.id).unwrap().values[slot].clone()
        };
        assert_eq!(seen(Some(HOST), &mob), SyncedValue::I32(5), "unowned: the default");
        tame::tame(&mut mob, OTHER);
        assert_eq!(seen(Some(HOST), &mob), SyncedValue::I32(5), "owned by somebody else");
        tame::tame(&mut mob, HOST);
        assert_eq!(seen(Some(HOST), &mob), SyncedValue::I32(40), "owned by the host's player");
        assert_eq!(seen(None, &mob), SyncedValue::I32(5), "no player uuid: nothing owned");
    }

    // ----- nothing without a custom mob --------------------------------------------------------

    #[test]
    fn a_world_without_custom_mobs_does_no_work_and_holds_nothing() {
        let mut render = CustomRender::default();
        assert!(render.is_empty());
        for _ in 0..1000 {
            render.begin_frame();
        }
        assert!(render.is_empty() && render.len() == 0 && render.matrices_of(1).is_none());
        assert_eq!(render.frame(), 1000);
        let none = embedded_textures_of(std::iter::empty());
        assert!(none.is_empty() && none.capacity() == 0, "no atlas image, no allocation");
        // A vanilla mob is never mirrored.
        let vanilla = crate::mobs::Mob::new(
            1,
            crate::mobs::sensor_scan_tests::kind(),
            horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            0.0,
        );
        let mut mirror = LocalMirror::default();
        mirror.sync(&vanilla, 5, 1, None);
        assert!(mirror.is_empty());
    }

    #[test]
    fn mob_states_of_mobs_not_drawn_for_a_while_are_dropped() {
        let ct = fox();
        let tr = ct.render.as_ref().unwrap();
        let mut render = CustomRender::default();
        render.begin_frame();
        render.parts(1, tr);
        render.parts(2, tr);
        assert_eq!(render.len(), 2);
        for _ in 0..800 {
            render.begin_frame();
            render.parts(2, tr);
        }
        assert_eq!(render.len(), 1, "mob 1 was not drawn for 600 frames");
    }
}
