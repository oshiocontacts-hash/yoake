//! The debug view of custom mobs (FR-F4-MOBS.md section 3.11, F4M-12): what `/yoake mobdebug` and the
//! F3+M chord draw, and what `/yoake mobs` prints.
//!
//! This file is free of the window and the GPU. [`plan`] turns one mob's facts ([`MobView`]) into
//! gizmo shapes ([`Shape`]: lines, boxes, arrows, drawn through `gizmos.rs`) and text labels
//! ([`Label`]: billboard text), so what the view shows can be tested without a game. The glue is
//! `game/custom_mob_debug.rs`.
//!
//! Off by default and free while off: [`DebugView`] is one byte, and the game does nothing at all
//! (no list, no allocation) while it is zero.
use super::{CustomType, SyncedValue, remote::RemoteCustomMob, render_state as rs};
use glam::{Mat4, Vec3};

/// The part switches of `/yoake mobdebug`.
pub const HITBOXES: u8 = 1;
pub const BONES: u8 = 2;
pub const ANIM: u8 = 4;
pub const AI: u8 = 8;
pub const ALL: u8 = HITBOXES | BONES | ANIM | AI;

/// ARGB colours of the shapes (the roles of a hitbox part, the skeleton, the AI).
pub const HURT_AND_PICK: u32 = 0xFF3F_FF00;
pub const HURT_ONLY: u32 = 0xFFFF_3030;
pub const PICK_ONLY: u32 = 0xFF55_99FF;
pub const DISABLED: u32 = 0xFF80_8080;
pub const BONE: u32 = 0xFFFF_FF00;
pub const LOCATOR: u32 = 0xFF00_FFFF;
pub const SEAT: u32 = 0xFFFF_AA00;
pub const TARGET: u32 = 0xFFFF_55FF;
pub const PATH: u32 = 0xFF55_FF55;
pub const TEXT: u32 = 0xFFFF_FFFF;
pub const TEXT_DIM: u32 = 0xFFAA_AAAA;

/// The most mobs the view draws in one frame, and how far from the camera (blocks).
pub const MAX_MOBS: usize = 48;
pub const RANGE: f64 = 48.0;
/// Bone names are drawn only for a skeleton this small (a big one would be a wall of text).
const MAX_BONE_LABELS: usize = 48;
/// Blocks between two lines of the overlay text.
const LINE_HEIGHT: f64 = 0.17;

/// Which parts of the view are on; zero is off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DebugView(u8);

/// What a `/yoake mobdebug ...` asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Request {
    Toggle,
    On(u8),
    Off,
}

impl DebugView {
    pub fn is_on(self) -> bool {
        self.0 != 0
    }
    pub fn parts(self) -> u8 {
        self.0
    }
    pub fn has(self, part: u8) -> bool {
        self.0 & part != 0
    }
    /// Applies `request`; the new state.
    pub fn apply(&mut self, request: Request) -> Self {
        self.0 = match request {
            Request::Toggle if self.is_on() => 0,
            Request::Toggle => ALL,
            Request::On(parts) => parts,
            Request::Off => 0,
        };
        *self
    }
    /// The part names that are on, in the command's order.
    pub fn names(self) -> Vec<&'static str> {
        [(HITBOXES, "hitboxes"), (BONES, "bones"), (ANIM, "anim"), (AI, "ai")]
            .into_iter()
            .filter(|(bit, _)| self.0 & bit != 0)
            .map(|(_, name)| name)
            .collect()
    }
}

/// `/yoake mobdebug [on|off] [hitboxes] [bones] [anim] [ai]` (the words after `mobdebug`): nothing
/// toggles, `on` alone is every part, part names alone switch exactly those on.
pub fn parse(args: &str) -> Result<Request, String> {
    let mut words = args.split_whitespace().peekable();
    let state = match words.peek().copied() {
        Some("on") => {
            words.next();
            Some(true)
        }
        Some("off") => {
            words.next();
            Some(false)
        }
        _ => None,
    };
    let mut parts = 0u8;
    for w in words {
        parts |= match w {
            "hitboxes" => HITBOXES,
            "bones" => BONES,
            "anim" => ANIM,
            "ai" => AI,
            other => return Err(format!("Unknown option {other:?}: use on, off, hitboxes, bones, anim, ai")),
        };
    }
    Ok(match (state, parts) {
        (Some(false), 0) => Request::Off,
        (Some(false), _) => return Err("off takes no part names".to_owned()),
        (Some(true), 0) => Request::On(ALL),
        (_, parts) if parts != 0 => Request::On(parts),
        (None, _) => Request::Toggle,
        (Some(true), _) => unreachable!("handled above"),
    })
}

/// One thing drawn through `gizmos.rs`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Shape {
    Line { a: [f64; 3], b: [f64; 3], color: u32, width: f32, on_top: bool },
    Cuboid { min: [f64; 3], max: [f64; 3], color: u32, width: f32 },
    Arrow { a: [f64; 3], b: [f64; 3], color: u32, width: f32 },
}

/// One billboard text.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub at: [f64; 3],
    pub text: String,
    pub color: u32,
}

/// What a frame of the view draws.
#[derive(Debug, Default)]
pub struct Draw {
    pub shapes: Vec<Shape>,
    pub labels: Vec<Label>,
}

/// Everything [`plan`] reads about one mob.
pub struct MobView<'a> {
    pub id: u32,
    pub ct: &'a CustomType,
    /// Where it is drawn: the interpolated feet, body yaw (degrees), roll and total scale.
    pub feet: [f64; 3],
    pub yaw: f32,
    pub roll: f32,
    pub scale: f32,
    /// The pose as last drawn (model pixels); the rest pose when it has not been drawn yet.
    pub matrices: Option<&'a [Mat4]>,
    pub hitbox_mask: u32,
    /// The seats' world positions (blocks), in the type's seat order.
    pub seats: &'a [[f64; 3]],
    pub anim: Option<&'a RemoteCustomMob>,
    pub values: &'a [SyncedValue],
    pub tick: i64,
    pub health: f32,
    pub eye: [f64; 3],
    pub target: Option<[f64; 3]>,
    /// The navigator's path from the next node on (block centres).
    pub path: &'a [[f64; 3]],
    pub goals: &'a [&'static str],
    pub target_goals: &'a [&'static str],
    pub activity: Option<&'a str>,
}

/// The text of a synced value (`warmth=2.50`, an enum by its name).
pub fn value_text(def: &super::SyncedDef, v: &SyncedValue) -> String {
    let shown = match v {
        SyncedValue::Bool(b) => b.to_string(),
        SyncedValue::I32(i) => i.to_string(),
        SyncedValue::F32(f) => format!("{f:.2}"),
        SyncedValue::Str(s) => format!("{s:?}"),
        SyncedValue::Vec3(p) => format!("({:.2}, {:.2}, {:.2})", p[0], p[1], p[2]),
        SyncedValue::Enum(i) => def.values.get(usize::from(*i)).cloned().unwrap_or_else(|| format!("#{i}")),
    };
    format!("{}={shown}", def.name)
}

/// The shapes and texts of the parts in `parts` for mob `v`, appended to `out`. The caller has
/// already decided the mob is worth drawing (in range, within the cap).
pub fn plan(parts: u8, v: &MobView, out: &mut Draw) {
    let top = v.feet[1] + f64::from(v.ct.file.size.height) * f64::from(v.scale.max(1.0));
    let mut stack = Stack { x: v.feet[0], z: v.feet[2], y: top + 0.35, lines: Vec::new() };
    stack.add(format!("{} #{}  hp {:.1}", v.ct.id, v.id, v.health), TEXT);
    if parts & HITBOXES != 0 {
        hitboxes(v, out);
    }
    if parts & BONES != 0 {
        skeleton(v, out);
    }
    if parts & ANIM != 0 {
        anim(v, &mut stack);
    }
    if parts & AI != 0 {
        ai(v, out, &mut stack);
    }
    stack.finish(out);
}

/// The overlay's lines, drawn from the top down.
struct Stack {
    x: f64,
    z: f64,
    y: f64,
    lines: Vec<(String, u32)>,
}

impl Stack {
    fn add(&mut self, text: String, color: u32) {
        self.lines.push((text, color));
    }
    fn finish(self, out: &mut Draw) {
        let n = self.lines.len();
        for (i, (text, color)) in self.lines.into_iter().enumerate() {
            let y = self.y + (n - 1 - i) as f64 * LINE_HEIGHT;
            out.labels.push(Label { at: [self.x, y, self.z], text, color });
        }
    }
}

fn hitboxes(v: &MobView, out: &mut Draw) {
    for (i, part) in v.ct.hitboxes.iter().enumerate() {
        let enabled = i >= 32 || v.hitbox_mask & (1 << i) != 0;
        let color = match (enabled, part.hurt, part.pick) {
            (false, _, _) => DISABLED,
            (_, true, true) => HURT_AND_PICK,
            (_, true, false) => HURT_ONLY,
            (_, false, _) => PICK_ONLY,
        };
        let b = part.bounds(v.feet, v.yaw);
        let (lo, hi) = (b.min_position(), b.max_position());
        out.shapes.push(Shape::Cuboid { min: [lo.x, lo.y, lo.z], max: [hi.x, hi.y, hi.z], color, width: 2.0 });
        let note = if enabled { String::new() } else { " (off)".to_owned() };
        out.labels.push(Label {
            at: [(lo.x + hi.x) / 2.0, hi.y + 0.08, (lo.z + hi.z) / 2.0],
            text: format!("{} x{}{note}", part.name, part.damage_multiplier),
            color,
        });
    }
}

fn world_of(v: &MobView, p: Vec3) -> [f64; 3] {
    rs::model_point_to_world(v.feet, v.scale, v.yaw, v.roll, p)
}

/// The skeleton as lines parent origin to child origin, the locators and the seats.
fn skeleton(v: &MobView, out: &mut Draw) {
    if let Some(tr) = v.ct.render.as_ref() {
        let matrices = v.matrices.unwrap_or(tr.rest_matrices.as_slice());
        let origin = |b: usize| -> Option<[f64; 3]> {
            let bone = tr.skeleton.bones.get(b)?;
            Some(world_of(v, matrices.get(b)?.transform_point3(bone.pivot)))
        };
        let label_bones = tr.skeleton.len() <= MAX_BONE_LABELS;
        for (b, bone) in tr.skeleton.bones.iter().enumerate() {
            let Some(here) = origin(b) else { continue };
            if let Some(parent) = bone.parent.and_then(|p| origin(p.index())) {
                out.shapes.push(Shape::Line { a: parent, b: here, color: BONE, width: 2.0, on_top: true });
            }
            if label_bones {
                out.labels.push(Label { at: here, text: bone.name.clone(), color: BONE });
            }
        }
        for (i, l) in tr.locators.iter().enumerate() {
            let Some(p) = tr.locator_point(i, matrices) else { continue };
            let at = world_of(v, p);
            let r = 0.04;
            out.shapes.push(Shape::Cuboid {
                min: [at[0] - r, at[1] - r, at[2] - r],
                max: [at[0] + r, at[1] + r, at[2] + r],
                color: LOCATOR,
                width: 2.0,
            });
            out.labels.push(Label { at: [at[0], at[1] + 0.1, at[2]], text: l.name.clone(), color: LOCATOR });
        }
    }
    for (i, at) in v.seats.iter().enumerate() {
        let r = 0.1;
        out.shapes.push(Shape::Cuboid {
            min: [at[0] - r, at[1] - r, at[2] - r],
            max: [at[0] + r, at[1] + r, at[2] + r],
            color: SEAT,
            width: 2.0,
        });
        out.labels.push(Label { at: [at[0], at[1] + 0.15, at[2]], text: format!("seat {i}"), color: SEAT });
    }
}

/// Per controller layer: its state and the time of its first clock; then the synced values.
fn anim(v: &MobView, stack: &mut Stack) {
    if let (Some(ta), Some(a)) = (v.ct.anim.as_ref(), v.anim) {
        for (li, c) in ta.rig.controllers().iter().enumerate() {
            let Some(Some(layer)) = a.layers.get(li) else {
                stack.add(format!("{}: (no anchor yet)", c.name), TEXT_DIM);
                continue;
            };
            let state = c.states.get(usize::from(layer.state)).map_or("?", |s| s.name.as_str());
            let time = a.time_at(li, 0, v.tick, 0.0).map_or(String::new(), |t| format!("  t={t:.2}s"));
            stack.add(format!("{}: {state}{time}", c.name), TEXT);
        }
    }
    if !v.ct.synced.is_empty() {
        let shown: Vec<String> = v
            .ct
            .synced
            .iter()
            .zip(v.values)
            .map(|(def, value)| value_text(def, value))
            .collect();
        stack.add(shown.join(" "), TEXT_DIM);
    }
}

/// The target, the path and the running goals.
fn ai(v: &MobView, out: &mut Draw, stack: &mut Stack) {
    if let Some(t) = v.target {
        out.shapes.push(Shape::Arrow { a: v.eye, b: t, color: TARGET, width: 2.5 });
    }
    let mut at = v.feet;
    for node in v.path {
        out.shapes.push(Shape::Line { a: at, b: *node, color: PATH, width: 2.0, on_top: false });
        at = *node;
    }
    if !v.goals.is_empty() {
        stack.add(format!("goals: {}", v.goals.join(", ")), TEXT);
    }
    if !v.target_goals.is_empty() {
        stack.add(format!("targets: {}", v.target_goals.join(", ")), TEXT_DIM);
    }
    if let Some(a) = v.activity {
        stack.add(format!("activity: {a}"), TEXT);
    }
    if v.path.is_empty() && v.target.is_none() && v.goals.is_empty() && v.activity.is_none() {
        stack.add("idle".to_owned(), TEXT_DIM);
    }
}

/// One type's line of `/yoake mobs`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TypeLine {
    pub id: String,
    pub live: u32,
    /// Drawn in the last frame.
    pub drawn: u32,
    /// Mean microseconds a frame went into posing and into building vertices (None: not measured yet).
    pub pose_us: Option<f64>,
    pub vertex_us: Option<f64>,
}

/// The lines of `/yoake mobs` for `types`: counts, and the measured client cost per frame.
pub fn mobs_report(types: &[TypeLine], measuring: bool) -> Vec<String> {
    if types.is_empty() {
        return vec!["No custom mob types are loaded".to_owned()];
    }
    let mut lines = Vec::new();
    let (mut live, mut drawn) = (0, 0);
    for t in types {
        let cost = match (t.pose_us, t.vertex_us) {
            (Some(p), Some(v)) => format!(", per frame: pose {p:.0} us, vertices {v:.0} us"),
            _ => String::new(),
        };
        lines.push(format!("{}: {} live, {} drawn{cost}", t.id, t.live, t.drawn));
        live += t.live;
        drawn += t.drawn;
    }
    if types.len() > 1 {
        lines.push(format!("Total: {live} live, {drawn} drawn"));
    }
    if !measuring {
        lines.push("Frame costs are measured from now on: run /yoake mobs again in a few seconds (/yoake mobs stop ends it)".to_owned());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::custom::tests::fox_type;

    fn view<'a>(ct: &'a CustomType, goals: &'a [&'static str], path: &'a [[f64; 3]], seats: &'a [[f64; 3]]) -> MobView<'a> {
        MobView {
            id: 7,
            ct,
            feet: [10.0, 64.0, 5.0],
            yaw: 0.0,
            roll: 0.0,
            scale: 1.0,
            matrices: None,
            hitbox_mask: u32::MAX,
            seats,
            anim: None,
            values: &[],
            tick: 100,
            health: 14.0,
            eye: [10.0, 64.5, 5.0],
            target: Some([12.0, 64.5, 5.0]),
            path,
            goals,
            target_goals: &[],
            activity: Some("idle"),
        }
    }

    #[test]
    fn mobdebug_words_parse_to_requests() {
        assert_eq!(parse(""), Ok(Request::Toggle));
        assert_eq!(parse("on"), Ok(Request::On(ALL)));
        assert_eq!(parse("off"), Ok(Request::Off));
        assert_eq!(parse("on bones ai"), Ok(Request::On(BONES | AI)));
        assert_eq!(parse("hitboxes"), Ok(Request::On(HITBOXES)));
        assert!(parse("off bones").is_err());
        assert!(parse("sideways").is_err());
        let mut v = DebugView::default();
        assert!(!v.is_on());
        assert!(v.apply(Request::Toggle).is_on());
        assert_eq!(v.parts(), ALL);
        assert!(!v.apply(Request::Toggle).is_on());
        assert_eq!(v.apply(Request::On(BONES)).names(), ["bones"]);
        assert_eq!(std::mem::size_of::<DebugView>(), 1, "the off state is one byte");
    }

    #[test]
    fn hitboxes_become_boxes_with_role_colours_and_names() {
        let ct = fox_type();
        let mut out = Draw::default();
        plan(HITBOXES, &view(&ct, &[], &[], &[]), &mut out);
        let boxes: Vec<_> = out.shapes.iter().filter(|s| matches!(s, Shape::Cuboid { .. })).collect();
        assert_eq!(boxes.len(), ct.hitboxes.len());
        assert!(out.labels.iter().any(|l| l.text.starts_with(&ct.hitboxes[0].name)));
        // A disabled part is grey and says so.
        let mut off = Draw::default();
        let mut v = view(&ct, &[], &[], &[]);
        v.hitbox_mask = 0;
        plan(HITBOXES, &v, &mut off);
        assert!(off.shapes.iter().all(|s| matches!(s, Shape::Cuboid { color, .. } if *color == DISABLED)));
        assert!(off.labels.iter().any(|l| l.text.ends_with("(off)")));
    }

    #[test]
    fn bones_locators_and_seats_are_drawn() {
        let ct = fox_type();
        let tr = ct.render.as_ref().unwrap();
        let seats = [[10.0, 65.0, 5.0]];
        let mut out = Draw::default();
        plan(BONES, &view(&ct, &[], &[], &seats), &mut out);
        let lines = out.shapes.iter().filter(|s| matches!(s, Shape::Line { .. })).count();
        let parented = tr.skeleton.bones.iter().filter(|b| b.parent.is_some()).count();
        assert_eq!(lines, parented, "one line per bone with a parent");
        for l in &tr.locators {
            assert!(out.labels.iter().any(|x| x.text == l.name), "locator {}", l.name);
        }
        assert!(out.labels.iter().any(|x| x.text == "seat 0"));
        for b in &tr.skeleton.bones {
            assert!(out.labels.iter().any(|x| x.text == b.name), "bone {}", b.name);
        }
    }

    #[test]
    fn ai_shows_target_path_goals_and_activity() {
        let ct = fox_type();
        let path = [[11.0, 64.0, 5.0], [12.0, 64.0, 6.0]];
        let mut out = Draw::default();
        plan(AI, &view(&ct, &["Float", "Stroll"], &path, &[]), &mut out);
        assert_eq!(out.shapes.iter().filter(|s| matches!(s, Shape::Arrow { .. })).count(), 1);
        assert_eq!(out.shapes.iter().filter(|s| matches!(s, Shape::Line { .. })).count(), 2);
        assert!(out.labels.iter().any(|l| l.text == "goals: Float, Stroll"));
        assert!(out.labels.iter().any(|l| l.text == "activity: idle"));
        // No part is on: only the header.
        let mut none = Draw::default();
        plan(0, &view(&ct, &["Float"], &path, &[]), &mut none);
        assert!(none.shapes.is_empty());
        assert_eq!(none.labels.len(), 1);
    }

    #[test]
    fn anim_lists_layers_and_synced_values() {
        use horizon_protocol::mob_sync::{Frame, LayerAnchor, Message, Snapshot, ValueSet};
        let ct = std::sync::Arc::new(fox_type());
        let values: Vec<SyncedValue> = ct.synced.iter().map(|s| s.default.clone()).collect();
        // Layer 0 (locomotion) anchored in state 1 at tick 90; the other layers never heard of.
        let mut store = crate::mobs::custom::remote::RemoteCustomMobs::default();
        let mut frame = Frame::new(90);
        frame.messages.push(Message::Snapshot(Snapshot {
            entity: 7,
            type_version: ct.sync_version,
            values: ValueSet::default(),
            anchors: vec![LayerAnchor { layer: 0, state: 1, tick: 90, slots: vec![(0.0, 1.0)] }],
            triggers: vec![],
            hitbox_mask: 3,
        }));
        let t = ct.clone();
        store.apply_at(&frame, 90, &move |_| Some(t.clone()));
        let mut v = view(&ct, &[], &[], &[]);
        v.values = &values;
        v.anim = store.get(7);
        let mut out = Draw::default();
        plan(ANIM, &v, &mut out);
        let text: Vec<&str> = out.labels.iter().map(|l| l.text.as_str()).collect();
        assert!(text.iter().any(|t| t.contains("warmth=")), "{text:?}");
        let controllers = ct.anim.as_ref().unwrap().rig.controllers();
        let first = &controllers[0];
        let state = &first.states[1].name;
        assert!(text.iter().any(|t| t.starts_with(&format!("{}: {state}", first.name)) && t.contains("t=0.50s")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("(no anchor yet)")), "a layer with no anchor says so: {text:?}");
        // The lines stack upwards from the header.
        let ys: Vec<f64> = out.labels.iter().map(|l| l.at[1]).collect();
        assert!(ys.windows(2).all(|w| w[0] > w[1]), "{ys:?}");
        assert!(out.labels[0].text.starts_with("sample_mobs:cinder_fox #7"));
    }

    #[test]
    fn the_report_lists_counts_and_asks_for_a_second_run_to_measure() {
        assert_eq!(mobs_report(&[], false), ["No custom mob types are loaded"]);
        let a = TypeLine { id: "a:x".into(), live: 3, drawn: 2, pose_us: Some(12.4), vertex_us: Some(30.0) };
        let b = TypeLine { id: "a:y".into(), live: 1, drawn: 0, ..Default::default() };
        let lines = mobs_report(&[a, b], true);
        assert_eq!(lines[0], "a:x: 3 live, 2 drawn, per frame: pose 12 us, vertices 30 us");
        assert_eq!(lines[1], "a:y: 1 live, 0 drawn");
        assert_eq!(lines[2], "Total: 4 live, 2 drawn");
        assert_eq!(lines.len(), 3);
        assert!(mobs_report(&[TypeLine::default()], false).last().unwrap().contains("again"));
    }
}
