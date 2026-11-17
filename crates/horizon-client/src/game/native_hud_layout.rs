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
    use horizon_client::hud::{GUI_SCALE, HudBatch};
    use std::sync::atomic::Ordering;

    #[test]
    fn gui_size_matches_hud_batch_at_window_sizes_and_scales() {
        if std::env::var_os("YOAKE_TEST_GUI_SCALE_CHILD").is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .arg("gui_size_matches_hud_batch_at_window_sizes_and_scales")
                .env("YOAKE_TEST_GUI_SCALE_CHILD", "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "GUI scale test subprocess failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        struct RestoreScale(u32);
        impl Drop for RestoreScale {
            fn drop(&mut self) {
                GUI_SCALE.store(self.0, Ordering::Relaxed);
            }
        }

        let _restore = RestoreScale(GUI_SCALE.load(Ordering::Relaxed));
        for (window, scale, expected) in [
            ([319, 239], 0, (319.0, 239.0)),
            ([1280, 720], 0, (427.0, 240.0)),
            ([2560, 1440], 0, (427.0, 240.0)),
            ([1921, 1081], 0, (481.0, 271.0)),
            ([1280, 720], 1, (1280.0, 720.0)),
            ([1280, 720], 2, (640.0, 360.0)),
            ([1920, 1080], 5, (480.0, 270.0)),
        ] {
            GUI_SCALE.store(scale, Ordering::Relaxed);
            let batch = HudBatch::new(window);
            let size = Game::gui_size_for_window(window);
            assert_eq!(size, (batch.width, batch.height));
            assert_eq!(size, expected, "window {window:?}, scale {scale}");
        }
    }

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
