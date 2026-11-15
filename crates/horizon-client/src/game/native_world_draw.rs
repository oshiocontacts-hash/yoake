//! Frame-local, bounded world drawing requests for native source mods.
use horizon_client::{gizmos, hud::HudAtlas, screen_effects};

use super::Game;

const MAX_LINES: usize = 256;
const MAX_POST_EFFECTS: usize = 16;
const MAX_DISTANCE: f64 = 256.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostKind {
    Tint,
    Vignette,
}

impl PostKind {
    fn sprite(self) -> &'static str {
        match self {
            Self::Tint => "horizon/white",
            Self::Vignette => "misc/vignette",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PostEffect {
    kind: PostKind,
    color: u32,
}

#[derive(Default)]
pub(super) struct WorldLines {
    accepting: bool,
    lines: Vec<gizmos::Line>,
    post_effects: Vec<PostEffect>,
}

impl WorldLines {
    pub(super) fn begin_frame(&mut self, in_world: bool) {
        self.lines.clear();
        self.post_effects.clear();
        self.accepting = in_world;
    }

    pub(super) fn end_frame(&mut self) {
        self.accepting = false;
    }

    fn push(&mut self, line: gizmos::Line) -> Result<(), String> {
        if !self.accepting {
            return Err("world lines may only be submitted from the client world frame hook".into());
        }
        if !line
            .start
            .iter()
            .chain(line.end.iter())
            .all(|value| value.is_finite())
            || !line.width.is_finite()
            || !(0.5..=4.0).contains(&line.width)
            || line.start == line.end
        {
            return Err("world lines need distinct finite endpoints and width from 0.5 to 4".into());
        }
        if self.lines.len() >= MAX_LINES {
            return Err("world line frame limit reached".into());
        }
        self.lines.push(line);
        Ok(())
    }

    fn push_post_effect(&mut self, effect: PostEffect) -> Result<(), String> {
        if !self.accepting {
            return Err("post effects may only be submitted from the client world frame hook".into());
        }
        if self.post_effects.len() >= MAX_POST_EFFECTS {
            return Err("post effect frame limit reached".into());
        }
        self.post_effects.push(effect);
        Ok(())
    }

    pub(super) fn append_visible(&mut self, camera: [f64; 3], output: &mut Vec<gizmos::Line>) {
        output.extend(self.lines.drain(..).filter(|line| {
            [line.start, line.end].iter().all(|point| {
                point
                    .iter()
                    .zip(camera.iter())
                    .map(|(value, origin)| (value - origin).powi(2))
                    .sum::<f64>()
                    <= MAX_DISTANCE * MAX_DISTANCE
            })
        }));
    }

    pub(super) fn append_post_effects(
        &mut self,
        atlas: &HudAtlas,
        output: &mut Vec<screen_effects::ScreenQuad>,
    ) {
        output.extend(self.post_effects.drain(..).filter_map(|effect| {
            let name = effect.kind.sprite();
            let rect = atlas.sprites.get(name)?;
            let sampling = atlas.flags.get(name).copied().map_or(0, screen_effects::sampling);
            Some(post_quad(
                effect,
                [rect.u0, rect.v0, rect.u1, rect.v1],
                sampling,
            ))
        }));
    }
}

fn post_quad(effect: PostEffect, rect: [f32; 4], sampling: u32) -> screen_effects::ScreenQuad {
    let [alpha, red, green, blue] = effect.color.to_be_bytes();
    let [red, green, blue, alpha] = [red, green, blue, alpha].map(|value| f32::from(value) / 255.0);
    let (color, blend) = match effect.kind {
        PostKind::Tint => ([red, green, blue, alpha], screen_effects::Blend::Alpha),
        PostKind::Vignette => (
            [
                (1.0 - red) * alpha,
                (1.0 - green) * alpha,
                (1.0 - blue) * alpha,
                1.0,
            ],
            screen_effects::Blend::Vignette,
        ),
    };
    screen_effects::ScreenQuad {
        vertices: std::array::from_fn(|index| screen_effects::ScreenVertex {
            pos: [[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]][index],
            uv: [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]][index],
            rect,
            color,
        }),
        blend,
        sampling,
        world_tint: false,
    }
}

#[allow(dead_code)]
impl Game<'_> {
    /// Submit a depth-tested line to the current world frame. Coordinates are in world blocks,
    /// color is ARGB, and width is in pixels. Calls are accepted only during the client frame hook;
    /// requests are bounded and consumed by that frame. API @API:world_lines@.
    pub(super) fn native_draw_world_line(
        &mut self,
        start: [f64; 3],
        end: [f64; 3],
        color: u32,
        width: f32,
    ) -> Result<(), String> {
        self.native_world_lines.push(gizmos::Line {
            start,
            end,
            color,
            width,
        })
    }

    /// Add a full-screen ARGB tint after the world, before the HUD, for this client frame.
    /// API @API:world_lines@.
    pub(super) fn native_draw_post_tint(&mut self, color: u32) -> Result<(), String> {
        self.native_post_effect(PostKind::Tint, color)
    }

    /// Add an ARGB vignette after the world, before the HUD, for this client frame.
    /// API @API:world_lines@.
    pub(super) fn native_draw_post_vignette(&mut self, color: u32) -> Result<(), String> {
        self.native_post_effect(PostKind::Vignette, color)
    }

    fn native_post_effect(&mut self, kind: PostKind, color: u32) -> Result<(), String> {
        if !self.gui.atlas.sprites.contains_key(kind.sprite()) {
            return Err("post effect texture is unavailable".into());
        }
        self.native_world_lines
            .push_post_effect(PostEffect { kind, color })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(start: [f64; 3], end: [f64; 3]) -> gizmos::Line {
        gizmos::Line {
            start,
            end,
            color: 0xff12_3456,
            width: 2.0,
        }
    }

    #[test]
    fn requests_are_only_accepted_in_a_world_frame() {
        let mut lines = WorldLines::default();
        assert!(lines.push(line([0.0; 3], [1.0; 3])).is_err());
        lines.begin_frame(false);
        assert!(lines.push(line([0.0; 3], [1.0; 3])).is_err());
        lines.begin_frame(true);
        assert!(lines.push(line([0.0; 3], [1.0; 3])).is_ok());
        lines.end_frame();
        assert!(lines.push(line([0.0; 3], [1.0; 3])).is_err());
    }

    #[test]
    fn rejects_invalid_geometry_and_limits_each_frame() {
        let mut lines = WorldLines::default();
        lines.begin_frame(true);
        assert!(lines.push(line([f64::NAN, 0.0, 0.0], [0.0; 3])).is_err());
        assert!(lines.push(line([0.0; 3], [0.0; 3])).is_err());
        let mut invalid = line([0.0; 3], [1.0; 3]);
        invalid.width = f32::INFINITY;
        assert!(lines.push(invalid).is_err());
        invalid.width = 4.1;
        assert!(lines.push(invalid).is_err());
        for _ in 0..MAX_LINES {
            assert!(lines.push(line([0.0; 3], [1.0; 3])).is_ok());
        }
        assert!(lines.push(line([0.0; 3], [1.0; 3])).is_err());
        lines.begin_frame(true);
        assert!(lines.push(line([0.0; 3], [1.0; 3])).is_ok());
    }

    #[test]
    fn renders_nearby_lines_once_and_discards_distant_lines() {
        let mut lines = WorldLines::default();
        let near = line([12.0, 0.0, 0.0], [15.0, 0.0, 0.0]);
        lines.begin_frame(true);
        lines.push(near).unwrap();
        lines
            .push(line([300.0, 0.0, 0.0], [301.0, 0.0, 0.0]))
            .unwrap();
        lines.push(line([0.0; 3], [f64::MAX, 0.0, 0.0])).unwrap();
        let mut output = Vec::new();
        lines.append_visible([0.0; 3], &mut output);
        assert_eq!(output, vec![near]);
        lines.append_visible([0.0; 3], &mut output);
        assert_eq!(output, vec![near]);
    }

    #[test]
    fn post_effects_are_frame_scoped_and_bounded() {
        let mut lines = WorldLines::default();
        let effect = PostEffect {
            kind: PostKind::Tint,
            color: 0x80ff_0000,
        };
        assert!(lines.push_post_effect(effect).is_err());
        lines.begin_frame(true);
        for _ in 0..MAX_POST_EFFECTS {
            assert!(lines.push_post_effect(effect).is_ok());
        }
        assert!(lines.push_post_effect(effect).is_err());
        lines.end_frame();
        assert!(lines.push_post_effect(effect).is_err());
        lines.begin_frame(true);
        assert!(lines.post_effects.is_empty());
        assert!(lines.push_post_effect(effect).is_ok());
    }

    #[test]
    fn post_quads_cover_screen_and_apply_argb_opacity() {
        let rect = [0.1, 0.2, 0.3, 0.4];
        let effect = PostEffect {
            kind: PostKind::Tint,
            color: 0x80ff_0000,
        };
        let tint = post_quad(effect, rect, 0);
        assert_eq!(tint.blend, screen_effects::Blend::Alpha);
        assert_eq!(tint.vertices[0].pos, [-1.0, 1.0]);
        assert_eq!(tint.vertices[2].pos, [1.0, -1.0]);
        assert_eq!(tint.vertices[0].rect, rect);
        assert_eq!(tint.vertices[0].color, [1.0, 0.0, 0.0, 128.0 / 255.0]);
        let vignette = post_quad(PostEffect { kind: PostKind::Vignette, ..effect }, rect, 0);
        assert_eq!(vignette.blend, screen_effects::Blend::Vignette);
        assert_eq!(vignette.vertices[0].color, [0.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]);
        let black_vignette = post_quad(
            PostEffect {
                kind: PostKind::Vignette,
                color: 0x8000_0000,
            },
            rect,
            0,
        );
        assert_eq!(
            black_vignette.vertices[0].color,
            [128.0 / 255.0, 128.0 / 255.0, 128.0 / 255.0, 1.0]
        );
    }
}
