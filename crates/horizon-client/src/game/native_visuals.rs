use super::Game;
use crate::mod_api::NativeColoredLight;
use glam::DVec3;
use horizon_client::{dynamic_light, gizmos};

fn rim_target_valid(
    loaded: bool,
    state: horizon_world::registry::BlockStateId,
    air: horizon_world::registry::BlockStateId,
) -> bool {
    loaded && state != air
}

fn rim_position_valid(at: [f64; 3], player: DVec3) -> bool {
    at.iter().all(|v| v.is_finite())
        && (at[0] - player.x).powi(2) + (at[1] - player.y).powi(2) + (at[2] - player.z).powi(2)
            <= 1024.0
}

/// Native coloured-light limits for hooks @API:visual_light_limits@ and newer.
fn colored_light_valid(light: &NativeColoredLight, camera: DVec3, loaded: bool) -> bool {
    light.at.iter().all(|v| v.is_finite())
        && DVec3::from(light.at).distance_squared(camera) <= 4096.0
        && light.rgb <= 0xffffff
        && light.radius.is_finite()
        && (0.25..=32.0).contains(&light.radius)
        && light.intensity.is_finite()
        && (0.0..=1.0).contains(&light.intensity)
        && loaded
}

fn colored_light_requests(
    lights: impl Iterator<Item = NativeColoredLight>,
    camera: DVec3,
    mut loaded: impl FnMut(&NativeColoredLight) -> bool,
) -> impl Iterator<Item = NativeColoredLight> {
    lights
        .take(8)
        .filter(move |light| colored_light_valid(light, camera, loaded(light)))
}

impl Game<'_> {
    /// Frame-local requests never persist after a mod is removed or a world changes.
    pub(super) fn native_frame_visuals(
        &self,
        camera: DVec3,
        alpha: f64,
        lines: &mut Vec<gizmos::Line>,
    ) -> Vec<dynamic_light::Colored> {
        let mut particle_lights = self
            .renderer
            .as_ref()
            .and_then(|r| r.particle_lights())
            .map_or_else(Vec::new, |entries| {
                self.particles
                    .lights(entries, self.clock.camera_fraction() as f32)
            });
        // Held and dropped items whose block the pack's `[[lights]]` colour shine in that colour,
        // as far as their dynamic light reaches.
        if let Some(renderer) = self.renderer.as_ref().filter(|r| r.has_light_colors()) {
            for (source, item) in self.dynamic_light_sources(alpha) {
                let Some(color) = item.and_then(|i| renderer.light_color(&i)) else {
                    continue;
                };
                let top = color[0].max(color[1]).max(color[2]);
                if source.level == 0 || top <= 0.0 {
                    continue;
                }
                let [r, g, b] = color.map(|c| (c / top * 255.0).round() as u32);
                particle_lights.push((
                    source.at.to_array(),
                    r << 16 | g << 8 | b,
                    f32::from(source.level),
                    1.0,
                ));
            }
        }
        let Some(player) = self.native_player() else {
            return dynamic_light::select_colored(particle_lights.into_iter(), camera);
        };
        let mut rims = Vec::new();
        let mut lights = Vec::new();
        self.native_visuals(&mut rims, &mut lights);
        let mut edges = 0;
        for rim in rims.into_iter().take(8) {
            let pos = rim.pos;
            let state = self.world.block(pos[0], pos[1], pos[2]);
            if edges >= 256
                || !rim.width.is_finite()
                || !(0.5..=4.0).contains(&rim.width)
                || !rim_position_valid(
                    pos.map(|v| f64::from(v) + 0.5),
                    DVec3::new(player.x, player.y, player.z),
                )
                || !rim_target_valid(
                    self.world
                        .chunks
                        .contains_key(&(pos[0].div_euclid(16), pos[2].div_euclid(16))),
                    state,
                    self.world.air,
                )
            {
                continue;
            }
            let facts = self.res.facts().get(state);
            let offset = if facts.outline_offset {
                self.pack
                    .context
                    .tables
                    .offset(state, pos[0], pos[2])
                    .unwrap_or([0.0; 3])
            } else {
                [0.0; 3]
            };
            let origin = pos.map(f64::from);
            let box_shape = self.context_outline(pos, state);
            let cube = horizon_client::blocks::cube_edges();
            let segments = if box_shape.is_some() {
                cube.as_slice()
            } else {
                facts.outline_edges.as_ref()
            };
            for edge in segments.iter().take(256 - edges) {
                let point = |end: usize| {
                    std::array::from_fn(|i| {
                        let v = edge[i + 3 * end];
                        if let Some(b) = box_shape {
                            origin[i] + b[i] + v * (b[i + 3] - b[i])
                        } else {
                            origin[i] + offset[i] + v
                        }
                    })
                };
                lines.push(gizmos::Line {
                    start: point(0),
                    end: point(1),
                    color: rim.argb,
                    width: rim.width,
                });
                edges += 1;
            }
        }
        let sources = colored_light_requests(lights.into_iter(), camera, |light| {
            self.world.chunks.contains_key(&(
                (light.at[0].floor() as i32).div_euclid(16),
                (light.at[2].floor() as i32).div_euclid(16),
            ))
        });
        dynamic_light::select_colored(
            sources
                .map(|l| (l.at, l.rgb, l.radius, l.intensity))
                .chain(particle_lights),
            camera,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        colored_light_requests, colored_light_valid, rim_position_valid, rim_target_valid,
    };
    use crate::mod_api::NativeColoredLight;
    use glam::DVec3;
    use horizon_world::registry::BlockStateId;

    fn colored_light(at: [f64; 3]) -> NativeColoredLight {
        NativeColoredLight {
            at,
            rgb: 0xff8040,
            radius: 1.0,
            intensity: 0.5,
        }
    }

    #[test]
    fn colored_light_radius_boundaries() {
        for radius in [0.25, 32.0] {
            let light = NativeColoredLight {
                radius,
                ..colored_light([0.0; 3])
            };
            assert!(colored_light_valid(&light, DVec3::ZERO, true));
        }
        for radius in [0.24, 32.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let light = NativeColoredLight {
                radius,
                ..colored_light([0.0; 3])
            };
            assert!(!colored_light_valid(&light, DVec3::ZERO, true));
        }
    }

    #[test]
    fn colored_lights_follow_the_render_camera_not_the_player() {
        let player = DVec3::new(0.0, 64.0, 0.0);
        let camera = DVec3::new(128.0, 64.0, 0.0);
        let near_camera = colored_light([127.0, 64.0, 0.0]);
        let near_player = colored_light([1.0, 64.0, 0.0]);
        assert!(colored_light_valid(&near_camera, camera, true));
        assert!(!colored_light_valid(&near_player, camera, true));
        assert!(!rim_position_valid(near_camera.at, player));
        assert!(rim_position_valid(near_player.at, player));
    }

    #[test]
    fn colored_light_camera_distance_is_inclusive_and_euclidean() {
        let camera = DVec3::new(100.0, 64.0, -200.0);
        for offset in [
            [64.0, 0.0, 0.0],
            [-64.0, 0.0, 0.0],
            [0.0, 64.0, 0.0],
            [0.0, 0.0, 64.0],
            [32.0, 32.0, 32.0],
        ] {
            let light = colored_light((camera + DVec3::from(offset)).to_array());
            assert!(colored_light_valid(&light, camera, true));
        }
        for offset in [
            [64.01, 0.0, 0.0],
            [-64.01, 0.0, 0.0],
            [0.0, 64.01, 0.0],
            [0.0, 0.0, 64.01],
            [48.0, 48.0, 0.0],
        ] {
            let light = colored_light((camera + DVec3::from(offset)).to_array());
            assert!(!colored_light_valid(&light, camera, true));
        }
    }

    #[test]
    fn colored_lights_require_finite_positions_and_loaded_sources() {
        let light = colored_light([0.0; 3]);
        assert!(!colored_light_valid(&light, DVec3::ZERO, false));
        for axis in 0..3 {
            for coordinate in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut light = light;
                light.at[axis] = coordinate;
                assert!(!colored_light_valid(&light, DVec3::ZERO, true));
            }
        }
    }

    #[test]
    fn colored_light_rgb_and_intensity_limits_are_unchanged() {
        for rgb in [0, 0xffffff] {
            for intensity in [0.0, 1.0] {
                let light = NativeColoredLight {
                    rgb,
                    intensity,
                    ..colored_light([0.0; 3])
                };
                assert!(colored_light_valid(&light, DVec3::ZERO, true));
            }
        }
        let light = NativeColoredLight {
            rgb: 0x1000000,
            ..colored_light([0.0; 3])
        };
        assert!(!colored_light_valid(&light, DVec3::ZERO, true));
        for intensity in [-0.01, 1.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let light = NativeColoredLight {
                intensity,
                ..colored_light([0.0; 3])
            };
            assert!(!colored_light_valid(&light, DVec3::ZERO, true));
        }
    }

    #[test]
    fn native_light_budget_is_applied_before_validation() {
        let invalid = NativeColoredLight {
            radius: 32.01,
            ..colored_light([0.0; 3])
        };
        let requests = [invalid; 8]
            .into_iter()
            .chain(std::iter::once(colored_light([0.0; 3])));
        let mut examined = 0;
        let accepted = colored_light_requests(requests, DVec3::ZERO, |_| {
            examined += 1;
            true
        })
        .count();
        assert_eq!(accepted, 0);
        assert_eq!(examined, 8);
    }

    #[test]
    fn empty_native_light_requests_do_no_work() {
        let mut examined = 0;
        let accepted = colored_light_requests(std::iter::empty(), DVec3::ZERO, |_| {
            examined += 1;
            true
        })
        .count();
        assert_eq!(accepted, 0);
        assert_eq!(examined, 0);
    }

    #[test]
    fn rim_distance_remains_within_32_blocks_of_the_player() {
        let player = DVec3::new(100.0, 64.0, -200.0);
        assert!(rim_position_valid([132.0, 64.0, -200.0], player));
        assert!(!rim_position_valid([132.01, 64.0, -200.0], player));
        assert!(!rim_position_valid([124.0, 88.0, -200.0], player));
        assert!(!rim_position_valid([f64::NAN, 64.0, -200.0], player));
    }

    #[test]
    fn ordinary_loaded_blocks_can_have_rims() {
        let air = BlockStateId(0);
        let stone = BlockStateId(1); // No emission requirement.
        assert!(rim_target_valid(true, stone, air));
        assert!(!rim_target_valid(false, stone, air));
        assert!(!rim_target_valid(true, air, air));
    }
}
