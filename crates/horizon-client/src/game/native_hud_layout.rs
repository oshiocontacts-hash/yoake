use super::Game;
use horizon_client::mod_api::{Corner, Corners};

fn place(corners: &mut Corners, corner: Corner, size: [f32; 2], shown: bool) -> Option<[f32; 2]> {
    if !shown
        || !size
            .iter()
            .all(|value| value.is_finite() && *value > 0.0 && *value <= 1_000_000.0)
    {
        return None;
    }
    let (left, top) = corners.place(corner, size[0], size[1]);
    Some([left, top])
}

impl Game<'_> {
    pub fn native_hud_place(
        &self,
        corners: &mut Corners,
        corner: Corner,
        size: [f32; 2],
    ) -> Option<[f32; 2]> {
        place(corners, corner, size, self.native_corner_shown(corner))
    }

    pub fn native_gui_size(&self) -> [f32; 2] {
        let (width, height) = self.gui_size();
        [width, height]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_hud_layout_returns_top_left_and_shares_existing_reservations() {
        let mut corners = Corners::new(400.0, 300.0, 26.0);
        assert_eq!(
            place(&mut corners, Corner::TopLeft, [50.0, 12.0], true),
            Some([2.0, 2.0])
        );
        assert_eq!(
            place(&mut corners, Corner::TopLeft, [70.0, 20.0], true),
            Some([2.0, 16.0])
        );
        assert_eq!(
            place(&mut corners, Corner::TopRight, [50.0, 12.0], true),
            Some([348.0, 28.0])
        );
        assert_eq!(
            place(&mut corners, Corner::BottomRight, [50.0, 12.0], true),
            Some([348.0, 286.0])
        );
        assert_eq!(
            place(&mut corners, Corner::BottomLeft, [50.0, 12.0], true),
            Some([2.0, 286.0])
        );
    }

    #[test]
    fn native_hud_layout_rejected_or_hidden_boxes_do_not_reserve_space() {
        let mut corners = Corners::new(400.0, 300.0, 0.0);
        assert_eq!(
            place(&mut corners, Corner::TopLeft, [50.0, 12.0], false),
            None
        );
        for invalid in [0.0, -1.0, f32::NAN, f32::INFINITY, 1_000_001.0] {
            assert_eq!(
                place(&mut corners, Corner::TopLeft, [invalid, 12.0], true),
                None
            );
            assert_eq!(
                place(&mut corners, Corner::TopLeft, [50.0, invalid], true),
                None
            );
        }
        assert_eq!(
            place(&mut corners, Corner::TopLeft, [50.0, 12.0], true),
            Some([2.0, 2.0])
        );
    }
}
