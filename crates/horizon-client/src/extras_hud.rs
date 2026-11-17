//! HUD boxes in the corners: the minimap's place and mods' panels, small
//! boxes in GUI pixels stacked per corner (`docs/extras.md`).
use crate::{
    extras::Corner,
    hud::{HudAtlas, HudBatch, Rect, rgba},
};

/// The gap between an overlay and the screen edge or the next overlay.
const MARGIN: f32 = 2.0;
const BACKGROUND: u32 = 0x7800_0000;

/// Where the next overlay goes in each corner.
pub struct Corners {
    width: f32,
    height: f32,
    /// Distance from the corner's top or bottom edge, per `Corner::ALL`.
    used: [f32; 4],
    top_center_used: f32,
    top_center_limit: Option<f32>,
    top_center_boss_count: usize,
}
impl Corners {
    /// `top_right` reserves the status effect icons' rows.
    pub fn new(width: f32, height: f32, top_right: f32) -> Self {
        Self {
            width,
            height,
            used: [0.0, top_right, 0.0, 0.0],
            top_center_used: 0.0,
            top_center_limit: None,
            top_center_boss_count: 0,
        }
    }
    pub fn reserve_top_center(&mut self, boss_count: usize) {
        self.top_center_boss_count = boss_count;
    }
    pub fn place_top_center(&mut self, width: f32, height: f32) -> Option<(f32, f32)> {
        if !(width.is_finite()
            && height.is_finite()
            && width > 0.0
            && height > 0.0
            && width <= 1_000_000.0
            && height <= 1_000_000.0)
        {
            return None;
        }
        let limit = match self.top_center_limit {
            Some(limit) => limit,
            None => {
                let mut y = 12;
                for _ in 0..self.top_center_boss_count {
                    self.top_center_used = (y + 5) as f32;
                    y += 19;
                    if y >= self.height as i32 / 3 {
                        break;
                    }
                }
                let limit = ((self.height as i32 / 2) as f32 - 48.0).max(0.0);
                self.top_center_limit = Some(limit);
                limit
            }
        };
        let top = self.top_center_used + MARGIN;
        if width > self.width - MARGIN * 2.0 || top + height > limit {
            return None;
        }
        self.top_center_used = top + height;
        Some((((self.width - width) / 2.0).floor(), top))
    }
    /// The top-left position of a `w` x `h` box in `corner`, below (or
    /// above) what the corner already holds.
    pub fn place(&mut self, corner: Corner, w: f32, h: f32) -> (f32, f32) {
        let i = Corner::ALL.iter().position(|c| *c == corner).unwrap_or(0);
        let edge = self.used[i] + MARGIN;
        self.used[i] = edge + h;
        let x = if corner.left() {
            MARGIN
        } else {
            self.width - MARGIN - w
        };
        let y = if corner.top() {
            edge
        } else {
            self.height - edge - h
        };
        (x, y)
    }
}

/// How the minimap shows: its corner and side in GUI pixels, its image (an
/// atlas rect), the player arrow (the map item's decoration sprite) turned
/// `arrow` degrees clockwise from up, and the rim's N toward `north` when the
/// map turns.
pub struct MinimapLook {
    pub corner: Corner,
    pub size: f32,
    pub image: Rect,
    pub arrow: f32,
    pub north: Option<(f32, f32)>,
}
pub fn draw_minimap(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    corners: &mut Corners,
    look: &MinimapLook,
) {
    let MinimapLook {
        corner,
        size,
        image,
        arrow,
        north,
    } = *look;
    let (x, y) = corners.place(corner, size, size);
    batch.rect_uv(x, y, size, size, image, u32::MAX);
    let (cx, cy) = (x + size / 2.0, y + size / 2.0);
    if let Some(r) = atlas.sprites.get("map/decorations/player").copied() {
        // MapRenderer's decorations draw the sprite flipped: its arrow
        // points down the texture, so up on screen is half a turn.
        let (s, c) = (arrow + 180.0).to_radians().sin_cos();
        let p = |dx: f32, dy: f32| [cx + dx * c - dy * s, cy + dx * s + dy * c];
        let h = 4.0;
        batch.quad(
            [p(-h, h), p(h, h), p(h, -h), p(-h, -h)],
            [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]],
            u32::MAX,
        );
    }
    if let Some((nx, ny)) = north {
        let reach = size / 2.0 - 6.0;
        let tw = HudBatch::text_width(atlas, "N");
        let (tx, ty) = (cx + nx * reach - tw / 2.0, cy + ny * reach - 4.0);
        batch.text(
            atlas,
            "N",
            tx.round(),
            ty.round(),
            rgba(255, 255, 255, 255),
            true,
        );
    }
}

/// A box of text lines (a mod's HUD panel): `(text, 0xRRGGBB)` each.
pub fn draw_lines(
    batch: &mut HudBatch,
    atlas: &HudAtlas,
    corners: &mut Corners,
    corner: Corner,
    lines: &[(&str, u32)],
) {
    if lines.is_empty() {
        return;
    }
    let w = lines
        .iter()
        .map(|(text, _)| HudBatch::text_width(atlas, text))
        .fold(0.0, f32::max)
        + 4.0;
    let h = 10.0 * lines.len() as f32 + 2.0;
    let (x, y) = corners.place(corner, w, h);
    batch.fill(atlas, x, y, w, h, BACKGROUND);
    for (i, (text, color)) in lines.iter().enumerate() {
        let [_, r, g, b] = color.to_be_bytes();
        let y = y + 2.0 + 10.0 * i as f32;
        batch.text(atlas, text, x + 2.0, y, rgba(r, g, b, 255), true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_stack_away_from_their_edge() {
        let mut c = Corners::new(400.0, 300.0, 26.0);
        assert_eq!(c.place(Corner::TopLeft, 50.0, 12.0), (2.0, 2.0));
        assert_eq!(c.place(Corner::TopLeft, 70.0, 20.0), (2.0, 16.0));
        // Top right starts under the effect icons.
        assert_eq!(c.place(Corner::TopRight, 50.0, 12.0), (348.0, 28.0));
        assert_eq!(c.place(Corner::BottomRight, 50.0, 12.0), (348.0, 286.0));
        assert_eq!(c.place(Corner::BottomRight, 50.0, 10.0), (348.0, 274.0));
        assert_eq!(c.place(Corner::BottomLeft, 10.0, 10.0), (2.0, 288.0));
    }

    #[test]
    fn top_center_stacks_down_without_moving_corners() {
        let mut corners = Corners::new(400.0, 300.0, 26.0);
        assert_eq!(corners.place_top_center(50.0, 12.0), Some((175.0, 2.0)));
        assert_eq!(corners.place_top_center(70.0, 12.0), Some((165.0, 16.0)));
        assert_eq!(corners.place(Corner::TopRight, 50.0, 12.0), (348.0, 28.0));
    }

    #[test]
    fn top_center_avoids_boss_bars_and_title_area() {
        let mut corners = Corners::new(400.0, 300.0, 0.0);
        corners.reserve_top_center(2);
        assert_eq!(corners.place_top_center(80.0, 10.0), Some((160.0, 38.0)));
        assert_eq!(corners.place_top_center(80.0, 57.0), None);
        assert_eq!(corners.place_top_center(80.0, 52.0), Some((160.0, 50.0)));
        assert_eq!(corners.place_top_center(80.0, 1.0), None);

        let mut corners = Corners::new(400.0, 300.0, 0.0);
        corners.reserve_top_center(100);
        assert_eq!(corners.place_top_center(80.0, 7.0), Some((160.0, 95.0)));
    }

    #[test]
    fn top_center_rejects_invalid_sizes() {
        let mut corners = Corners::new(400.0, 300.0, 0.0);
        for size in [(0.0, 10.0), (10.0, f32::NAN), (397.0, 10.0)] {
            assert_eq!(corners.place_top_center(size.0, size.1), None);
        }
        assert_eq!(corners.place_top_center(20.0, 10.0), Some((190.0, 2.0)));
    }

    #[test]
    fn top_center_rounds_horizontal_position_down() {
        let mut corners = Corners::new(401.0, 300.0, 0.0);
        assert_eq!(corners.place_top_center(50.0, 10.0), Some((175.0, 2.0)));
    }
}
