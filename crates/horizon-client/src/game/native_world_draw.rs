//! Frame-local, bounded world drawing requests for native source mods.
use horizon_client::hud::native_textures::NativeGuiTexture;
use horizon_client::{gfx::native_services::WorldDraws, hud::HudAtlas, screen_effects};

use super::Game;

const MAX_POST_EFFECTS: usize = 16;

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
    post_effects: Vec<PostEffect>,
}

impl WorldLines {
    pub(super) fn begin_frame(&mut self, in_world: bool) {
        self.post_effects.clear();
        self.accepting = in_world;
    }

    pub(super) fn end_frame(&mut self) {
        self.accepting = false;
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
        self.native_world_draws()?.line(start, end, color, width)
    }

    pub fn native_draw_world_quad(
        &mut self,
        corners: [[f64; 3]; 4],
        color: u32,
    ) -> Result<(), String> {
        self.native_world_draws()?.quad(corners, color)
    }

    pub fn native_draw_world_textured_quad(
        &mut self,
        corners: [[f64; 3]; 4],
        uv: [f32; 4],
        color: u32,
        texture: &NativeGuiTexture,
    ) -> Result<(), String> {
        self.native_world_texture(texture)?;
        self.native_world_draws()?
            .textured_quad(corners, uv, color, texture)
    }

    pub fn native_draw_world_billboard(
        &mut self,
        center: [f64; 3],
        size: [f32; 2],
        uv: [f32; 4],
        color: u32,
        texture: Option<&NativeGuiTexture>,
    ) -> Result<(), String> {
        if let Some(texture) = texture {
            self.native_world_texture(texture)?;
        }
        self.native_world_draws()?
            .billboard(center, size, uv, color, texture)
    }

    fn native_world_draws(&mut self) -> Result<&mut WorldDraws, String> {
        if !self.native_world_lines.accepting {
            return Err("world drawing requires the client world frame hook".into());
        }
        self.renderer
            .as_mut()
            .map(|renderer| &mut renderer.native_services.draws)
            .ok_or("renderer unavailable".into())
    }

    fn native_world_texture(&self, texture: &NativeGuiTexture) -> Result<(), String> {
        if !self
            .renderer
            .as_ref()
            .is_some_and(|renderer| renderer.hud.native_gui_texture_owns(texture))
        {
            return Err("world texture must belong to the current renderer".into());
        }
        Ok(())
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

    #[test]
    fn requests_are_only_accepted_in_a_world_frame() {
        let mut lines = WorldLines::default();
        assert!(!lines.accepting);
        lines.begin_frame(false);
        assert!(!lines.accepting);
        lines.begin_frame(true);
        assert!(lines.accepting);
        lines.end_frame();
        assert!(!lines.accepting);
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
