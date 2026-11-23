//! The debug view of custom mobs in the game (F4M-12): `/yoake mobdebug`, `/yoake mobs`, the F3+M
//! chord and the frame glue. What is drawn is planned in `mobs/custom/debug.rs` (no window needed);
//! this file reads the mobs, hands the shapes to `gizmos.rs` and the texts to the name-tag path.
//!
//! Off is free: [`CustomMobDebug`] is a byte and two `Cell`s, every per-frame entry point returns
//! on its first line, and no list is made.
use super::Game;
use glam::DVec3;
use horizon_client::{
    gizmos::{self as g, Gizmos},
    mesher::Vertex,
    mobs::{
        Mob,
        custom::{
            ai::{budget, riding},
            debug::{self, Draw, DebugView, MobView, Shape},
            render_state as rs, stats,
        },
    },
    text::Text,
};
use std::{cell::{Cell, RefCell}, collections::BTreeMap, time::Instant};

/// `TextGizmo.Style.DEFAULT_SCALE`, as the other debug text renderers use it.
const TEXT_SCALE: f32 = 0.32;

/// The measured client cost of one type's mobs (`/yoake mobs`).
#[derive(Default)]
struct Cost {
    pose: std::time::Duration,
    vertices: std::time::Duration,
    /// The frame counter of the last frame this type drew in, and how many it drew there.
    frame: u64,
    drawn: u32,
}

/// The debug state of the game: which parts of the view are on, and the cost meter of `/yoake mobs`.
#[derive(Default)]
pub(super) struct CustomMobDebug {
    pub(super) view: DebugView,
    /// Whether the cost meter runs (`/yoake mobs` starts it, `/yoake mobs stop` ends it).
    timing: Cell<bool>,
    costs: RefCell<BTreeMap<String, Cost>>,
    /// The frame counter when the meter started.
    started_frame: Cell<u64>,
}

impl CustomMobDebug {
    /// Adds what posing `id` and building its vertices took, for the cost meter. Only called while it runs.
    pub(super) fn record(&self, id: &str, pose: std::time::Duration, vertices: std::time::Duration, frame: u64) {
        let mut costs = self.costs.borrow_mut();
        let c = costs.entry(id.to_owned()).or_default();
        c.pose += pose;
        c.vertices += vertices;
        if c.frame != frame {
            c.frame = frame;
            c.drawn = 0;
        }
        c.drawn += 1;
    }
    /// A timestamp for the meter, taken only while it runs.
    pub(super) fn now(&self) -> Option<Instant> {
        self.timing.get().then(Instant::now)
    }
}

impl Game<'_> {
    /// The words after `yoake` of a typed or scripted command, answered here on any server: false
    /// when it is some other `/yoake ...` command (the mods').
    pub(super) fn yoake_command(&mut self, command: &str) -> bool {
        let command = command.trim_start_matches('/');
        let Some(rest) = command.strip_prefix("yoake") else {
            return false;
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return false;
        }
        let rest = rest.trim();
        if !matches!(rest.split_whitespace().next(), Some("mobdebug" | "mobs" | "mob")) {
            return false;
        }
        for line in self.yoake_lines(rest) {
            self.system_message(Text::literal(line.as_str()));
        }
        true
    }

    /// The answer of `yoake <rest>` (`mobdebug ...`, `mobs ...`, `mob reload ...`), one string per line.
    pub(super) fn yoake_lines(&mut self, rest: &str) -> Vec<String> {
        let rest = rest.trim();
        let (word, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        match word {
            "mobdebug" => self.yoake_mobdebug(args.trim()),
            "mobs" => self.yoake_mobs(args.trim()),
            "mob" => self.yoake_mob(args.trim()),
            _ => vec![format!("Unknown yoake command {word:?}")],
        }
    }

    /// `/yoake mobdebug [on|off] [hitboxes] [bones] [anim] [ai]`; also the F3+M chord (a toggle).
    pub(super) fn yoake_mobdebug(&mut self, args: &str) -> Vec<String> {
        match debug::parse(args) {
            Ok(request) => {
                let view = self.custom_mob_debug.view.apply(request);
                if view.is_on() {
                    vec![format!("Custom mob debug view on: {}", view.names().join(", "))]
                } else {
                    vec!["Custom mob debug view off".to_owned()]
                }
            }
            Err(e) => vec![e],
        }
    }

    /// `/yoake mobs [stop]`: the loaded custom types with their live and drawn mobs, the measured client
    /// cost per frame and the path budget.
    pub(super) fn yoake_mobs(&mut self, args: &str) -> Vec<String> {
        let d = &self.custom_mob_debug;
        match args {
            "stop" => {
                d.timing.set(false);
                d.costs.borrow_mut().clear();
                return vec!["Custom mob frame costs are no longer measured".to_owned()];
            }
            "" => {}
            other => return vec![format!("Unknown option {other:?}: use /yoake mobs or /yoake mobs stop")],
        }
        let was_measuring = d.timing.get();
        if !was_measuring {
            d.timing.set(true);
            d.started_frame.set(self.custom_mob_render.borrow().frame());
            d.costs.borrow_mut().clear();
        }
        let loaded = horizon_client::mobs::custom::ids();
        let mut live: BTreeMap<String, u32> = BTreeMap::new();
        for m in &self.mobs {
            if m.kind.custom.is_some() && !m.removed {
                *live.entry(m.kind.id.clone()).or_default() += 1;
            }
        }
        let frame = self.custom_mob_render.borrow().frame();
        let elapsed = frame.saturating_sub(d.started_frame.get());
        let costs = d.costs.borrow();
        let types: Vec<debug::TypeLine> = loaded
            .iter()
            .map(|id| {
                let cost = costs.get(id);
                let per_frame = |t: std::time::Duration| (was_measuring && elapsed >= 10).then(|| t.as_secs_f64() * 1e6 / elapsed as f64);
                debug::TypeLine {
                    id: id.clone(),
                    live: live.get(id).copied().unwrap_or(0),
                    drawn: cost.filter(|c| c.frame + 1 >= frame).map_or(0, |c| c.drawn),
                    pose_us: cost.and_then(|c| per_frame(c.pose)),
                    vertex_us: cost.and_then(|c| per_frame(c.vertices)),
                }
            })
            .collect();
        let mut lines = debug::mobs_report(&types, was_measuring && elapsed >= 10);
        // The server's side: the path searches and the budget (`/horizon custommobs stats`).
        let searches = budget::stats_by_type();
        let input = stats::Input { loaded: &loaded, mobs: &live, searches: &searches, path: budget::stats(), limit: budget::limit() };
        if let Ok(server) = stats::report(&input, None) {
            lines.extend(server.into_iter().skip(1));
        }
        lines
    }

    /// One frame's shapes and texts of the view, nearest mobs first; nothing (and no allocation) when it is off.
    fn custom_mob_debug_draw(&self, alpha: f64) -> Option<Draw> {
        let view = self.custom_mob_debug.view;
        if !view.is_on() {
            return None;
        }
        let player = self.player.as_ref()?;
        let me = DVec3::new(player.position.x, player.position.y, player.position.z);
        let partial = alpha as f32;
        let mut near: Vec<(f64, &Mob)> = self
            .mobs
            .iter()
            .filter(|m| m.kind.custom.is_some() && !m.removed)
            .map(|m| ((DVec3::new(m.position.x, m.position.y, m.position.z) - me).length(), m))
            .filter(|(d, _)| *d <= debug::RANGE)
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near.truncate(debug::MAX_MOBS);
        let mut out = Draw::default();
        let render = self.custom_mob_render.try_borrow().ok();
        for (_, mob) in near {
            let Some(ct) = mob.kind.custom.as_ref() else { continue };
            let feet = mob.previous.lerp(mob.position, alpha);
            let yaw = mob.previous_body_yaw + horizon_client::mobs::living::wrap(mob.body_yaw - mob.previous_body_yaw) * partial;
            let scale = ct.render.as_ref().map_or(1.0, |t| t.scale) * mob.model_scale();
            let roll = rs::death_roll_for(ct, mob.death_time, partial);
            let anim = match &self.remote {
                Some(r) => r.custom_mobs.get(mob.id as i32),
                None => render.as_ref().and_then(|r| r.mirror.get(mob.id)),
            };
            let locators = self.posed_locators(mob);
            let seats: Vec<[f64; 3]> = (0..ct.ride.seats.len())
                .map(|i| {
                    let p = riding::seat_position_of(mob, i, &locators);
                    [p.x, p.y, p.z]
                })
                .collect();
            let values = mob.extra.custom.as_ref().map_or(&[][..], |c| c.synced.as_slice());
            let path: Vec<[f64; 3]> = mob
                .ai
                .nav
                .path
                .as_ref()
                .map(|p| {
                    p.nodes
                        .iter()
                        .skip(p.next)
                        .take(64)
                        .map(|n| [f64::from(n.x) + 0.5, f64::from(n.y), f64::from(n.z) + 0.5])
                        .collect()
                })
                .unwrap_or_default();
            let goals = mob.ai.goals.running_names();
            let target_goals = mob.ai.targets.running_names();
            let activity = mob.extra.custom.as_ref().and_then(|c| c.brain.active_activity.as_deref());
            let eye = [feet.x, feet.y + mob.eye_height() as f64, feet.z];
            let v = MobView {
                id: mob.id,
                ct,
                feet: [feet.x, feet.y, feet.z],
                yaw,
                roll,
                scale,
                matrices: render.as_ref().and_then(|r| r.matrices_of(mob.id)),
                hitbox_mask: mob.extra.custom.as_ref().map_or(u32::MAX, |c| c.hitbox_mask),
                seats: &seats,
                anim,
                values: anim.map_or(values, |a| a.values.as_slice()),
                tick: self.world.game_time,
                health: mob.health,
                eye,
                target: self.beam_target(mob.ai.target, alpha).map(|t| t.to_array()),
                path: &path,
                goals: &goals,
                target_goals: &target_goals,
                activity,
            };
            debug::plan(view.parts(), &v, &mut out);
        }
        Some(out)
    }

    /// The shapes of the view through `gizmos.rs`, with the other debug renderers' lines.
    pub(super) fn custom_mob_debug_gizmos(&self, alpha: f32, gizmos: &mut Gizmos) {
        if !self.custom_mob_debug.view.is_on() {
            return;
        }
        let Some(draw) = self.custom_mob_debug_draw(f64::from(alpha)) else {
            return;
        };
        for shape in draw.shapes {
            match shape {
                Shape::Line { a, b, color, width, on_top: false } => gizmos.line(a, b, color, width),
                Shape::Line { a, b, color, width, on_top: true } => gizmos.on_top.push(g::Line { start: a, end: b, color, width }),
                Shape::Cuboid { min, max, color, width } => gizmos.cuboid(min, max, color, width),
                Shape::Arrow { a, b, color, width } => gizmos.arrow(a, b, color, width),
            }
        }
    }

    /// The overlay texts of the view as billboard texts (`Gizmos.billboardText`, centred, no background).
    pub(super) fn custom_mob_debug_geometry(&mut self, camera: DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        if !self.custom_mob_debug.view.is_on() {
            return;
        }
        let Some(draw) = self.custom_mob_debug_draw(alpha) else {
            return;
        };
        if draw.labels.is_empty() {
            return;
        }
        let start = out.len();
        self.mark_translucent_entity(start, "");
        let mut patches = Vec::new();
        for label in draw.labels {
            patches.extend(self.name_tag_quads(
                DVec3::from_array(label.at) - camera,
                &[(Text::literal(label.text).with_color(label.color & 0xFF_FFFF), 0.0)],
                label.color >> 24,
                false,
                2,
                0,
                g::billboard_text_unit(TEXT_SCALE),
                out,
            ));
        }
        self.map_patches.extend(patches);
    }
}
