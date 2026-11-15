use glam::Mat4;
use horizon_client::mod_api::NativeCamera;

pub(super) struct CameraFrame<'a> {
    pub position: [f64; 3],
    pub eye_height: [f32; 2],
    pub partial_tick: f32,
    pub angles: [f32; 3],
    pub fov_degrees: f32,
    pub far: f32,
    pub size: [u32; 2],
    pub bob: Mat4,
    pub spin: Mat4,
    pub dimension: &'a str,
}

pub(super) fn snapshot(frame: CameraFrame<'_>) -> NativeCamera {
    let [yaw, pitch, roll] = frame.angles;
    NativeCamera {
        position: frame.position,
        eye_height: frame.eye_height[0]
            + (frame.eye_height[1] - frame.eye_height[0]) * frame.partial_tick,
        partial_tick: frame.partial_tick,
        yaw,
        pitch,
        fov_degrees: frame.fov_degrees,
        near: 0.05,
        far: frame.far,
        view: (frame.bob
            * frame.spin
            * horizon_client::camera::view_matrix_with_roll(yaw, pitch, roll))
        .to_cols_array(),
        projection: Mat4::perspective_rh(
            frame.fov_degrees.to_radians(),
            frame.size[0] as f32 / frame.size[1].max(1) as f32,
            frame.far,
            0.05,
        )
        .to_cols_array(),
        dimension: frame.dimension.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Vec3, Vec4};

    fn frame() -> CameraFrame<'static> {
        CameraFrame {
            position: [30_000_000.25, 64.5, -30_000_000.75],
            eye_height: [1.0, 1.6],
            partial_tick: 0.25,
            angles: [180.0, 0.0, 0.0],
            fov_degrees: 90.0,
            far: 1024.0,
            size: [1600, 800],
            bob: Mat4::IDENTITY,
            spin: Mat4::IDENTITY,
            dimension: "minecraft:overworld",
        }
    }

    fn close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
    }

    #[test]
    fn native_camera_is_relative_and_interpolates_eye_height() {
        let camera = snapshot(frame());
        assert_eq!(camera.position, [30_000_000.25, 64.5, -30_000_000.75]);
        close(camera.eye_height, 1.15);
        close(camera.partial_tick, 0.25);
        assert_eq!(camera.dimension, "minecraft:overworld");
        for (actual, expected) in camera.view.into_iter().zip(Mat4::IDENTITY.to_cols_array()) {
            close(actual, expected);
        }
    }

    #[test]
    fn native_camera_projection_uses_reversed_depth_and_viewport_aspect() {
        let camera = snapshot(frame());
        let projection = Mat4::from_cols_array(&camera.projection);
        for (distance, expected) in [(0.05, 1.0), (1024.0, 0.0)] {
            let clip = projection * Vec4::new(0.0, 0.0, -distance, 1.0);
            close(clip.z / clip.w, expected);
        }
        let clip = projection * Vec4::new(2.0, 1.0, -1.0, 1.0);
        close(clip.x / clip.w, 1.0);
        close(clip.y / clip.w, 1.0);
        assert!(
            snapshot(CameraFrame {
                size: [800, 0],
                ..frame()
            })
            .projection
            .iter()
            .all(|value| value.is_finite())
        );
    }

    #[test]
    fn native_camera_combines_roll_spin_and_bob_in_render_order() {
        let camera = snapshot(CameraFrame {
            angles: [180.0, 0.0, 90.0],
            bob: Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
            spin: Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ..frame()
        });
        let transformed = Mat4::from_cols_array(&camera.view) * Vec4::new(2.0, 0.0, 0.0, 1.0);
        for (actual, expected) in transformed.to_array().into_iter().zip([1.0, 0.0, 3.0, 1.0]) {
            close(actual, expected);
        }
        let turned = snapshot(CameraFrame {
            angles: [90.0, 0.0, 0.0],
            ..frame()
        });
        let forward = Mat4::from_cols_array(&turned.view) * Vec4::new(0.0, 0.0, -1.0, 0.0);
        close(forward.x, 1.0);
        close(forward.y, 0.0);
        close(forward.z, 0.0);
    }
}
