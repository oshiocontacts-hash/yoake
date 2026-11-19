//! Small, pure math helpers for the procedural constraints (F4M-8): robust rotation arcs, cone and angle limiting,
//! look-at angles and the analytic two-bone IK solution.
//!
//! Everything here is stateless and total: non-finite or degenerate inputs give a defined, finite answer (identity,
//! zero or "no solution"), never NaN and never a panic. Lengths are in model pixels, angles in the unit named by the
//! function (degrees unless the name says radians).

use glam::{EulerRot, Quat, Vec3};

/// Smallest vector length treated as non-zero.
pub const EPS: f32 = 1.0e-6;

/// `v / |v|`, or `None` when `v` is not finite or shorter than [`EPS`].
#[inline]
pub fn try_normalize(v: Vec3) -> Option<Vec3> {
    if !v.is_finite() {
        return None;
    }
    let len = v.length();
    if len > EPS && len.is_finite() {
        Some(v / len)
    } else {
        None
    }
}

/// Some unit vector perpendicular to the unit vector `n` (deterministic).
pub fn any_perpendicular(n: Vec3) -> Vec3 {
    let a = if n.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let p = n.cross(a);
    try_normalize(p).unwrap_or(Vec3::Y)
}

/// The smallest rotation taking direction `from` to direction `to` (both need not be unit). Identity when either is
/// degenerate; a half turn about some perpendicular axis when they are opposite.
pub fn rotation_arc(from: Vec3, to: Vec3) -> Quat {
    let (Some(a), Some(b)) = (try_normalize(from), try_normalize(to)) else {
        return Quat::IDENTITY;
    };
    let d = a.dot(b);
    if d > 1.0 - 1.0e-7 {
        return Quat::IDENTITY;
    }
    if d < -1.0 + 1.0e-7 {
        return Quat::from_axis_angle(any_perpendicular(a), std::f32::consts::PI);
    }
    let c = a.cross(b);
    let q = Quat::from_xyzw(c.x, c.y, c.z, 1.0 + d);
    if q.is_finite() && q.length_squared() > 0.0 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

/// `q` as a unit quaternion; identity if it is not finite or has no length.
#[inline]
pub fn safe_quat(q: Quat) -> Quat {
    if q.is_finite() && q.length_squared() > 1.0e-12 {
        q.normalize()
    } else {
        Quat::IDENTITY
    }
}

/// Spherical interpolation from `a` to `b` by `t` (clamped to 0..1), always the short way.
#[inline]
pub fn slerp_short(a: Quat, b: Quat, t: f32) -> Quat {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    safe_quat(safe_quat(a).slerp(safe_quat(b), t))
}

/// Rotation in degrees `(x, y, z)` to a quaternion, X applied first (the order of `skeleton::bone_matrices`).
pub fn euler_deg_to_quat(rot: Vec3) -> Quat {
    safe_quat(Quat::from_euler(EulerRot::ZYX, rot.z.to_radians(), rot.y.to_radians(), rot.x.to_radians()))
}

/// Angle in degrees between two directions (0 when either is degenerate).
pub fn angle_between_deg(a: Vec3, b: Vec3) -> f32 {
    match (try_normalize(a), try_normalize(b)) {
        (Some(a), Some(b)) => unit_angle_deg(a, b),
        _ => 0.0,
    }
}

/// Angle between two unit vectors in degrees, accurate for tiny angles (`atan2` of cross and dot, not `acos`).
fn unit_angle_deg(a: Vec3, b: Vec3) -> f32 {
    a.cross(b).length().atan2(a.dot(b)).to_degrees()
}

/// Move `current` toward `target` by at most `max_step` (all in the same unit). `max_step` of infinity or NaN-free
/// large values reach the target at once.
#[inline]
pub fn move_toward(current: f32, target: f32, max_step: f32) -> f32 {
    let d = target - current;
    if d.abs() <= max_step {
        target
    } else {
        current + d.signum() * max_step
    }
}

/// Rotate unit direction `from` toward unit direction `to` by at most `max_deg` degrees (the shortest arc).
pub fn rotate_dir_toward(from: Vec3, to: Vec3, max_deg: f32) -> Vec3 {
    let (Some(a), Some(b)) = (try_normalize(from), try_normalize(to)) else {
        return try_normalize(from).unwrap_or(Vec3::NEG_Z);
    };
    let angle = unit_angle_deg(a, b);
    if angle <= max_deg.max(0.0) || angle < 1.0e-4 {
        return b;
    }
    let arc = rotation_arc(a, b);
    let t = (max_deg.max(0.0) / angle).clamp(0.0, 1.0);
    let q = Quat::IDENTITY.slerp(arc, t);
    try_normalize(q * a).unwrap_or(a)
}

/// Clamp the angle between `dir` and `axis` to `max_deg` (a cone around `axis`), keeping the azimuth.
pub fn clamp_to_cone(axis: Vec3, dir: Vec3, max_deg: f32) -> Vec3 {
    let Some(a) = try_normalize(axis) else { return dir };
    let Some(d) = try_normalize(dir) else { return a };
    rotate_dir_toward(a, d, max_deg)
}

/// An orthonormal frame for yaw/pitch maths: `forward` and `up` unit vectors, `right = forward x up`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimFrame {
    pub forward: Vec3,
    pub up: Vec3,
    pub right: Vec3,
}

impl AimFrame {
    /// Blockbench's default: forward `-Z`, up `+Y`, right `+X`.
    pub const DEFAULT: AimFrame = AimFrame { forward: Vec3::NEG_Z, up: Vec3::Y, right: Vec3::X };

    /// Build a frame; `up` is orthogonalised against `forward`. `None` when they are parallel or degenerate.
    pub fn new(forward: Vec3, up: Vec3) -> Option<AimFrame> {
        let f = try_normalize(forward)?;
        let u = try_normalize(up - f * up.dot(f))?;
        let r = try_normalize(f.cross(u))?;
        Some(AimFrame { forward: f, up: u, right: r })
    }

    /// Yaw (positive toward `right`) and pitch (positive toward `up`) of a direction given in the frame's space, in
    /// degrees. A zero direction gives `(0, 0)`.
    pub fn angles_deg(&self, dir: Vec3) -> (f32, f32) {
        let Some(d) = try_normalize(dir) else { return (0.0, 0.0) };
        let x = d.dot(self.right);
        let y = d.dot(self.up);
        let z = d.dot(self.forward);
        let yaw = x.atan2(z).to_degrees();
        let pitch = y.atan2((x * x + z * z).sqrt()).to_degrees();
        (yaw, pitch)
    }

    /// The rotation (in the frame's space) that turns `forward` to the given yaw and pitch (degrees): pitch first
    /// about `right`, then yaw about `up`.
    pub fn rotation_deg(&self, yaw: f32, pitch: f32) -> Quat {
        let pitch_q = Quat::from_axis_angle(self.right, pitch.to_radians());
        // Positive rotation about `up` turns forward toward -right; yaw is positive toward +right.
        let yaw_q = Quat::from_axis_angle(self.up, (-yaw).to_radians());
        safe_quat(yaw_q * pitch_q)
    }
}

/// Result of [`solve_two_bone`]: where the middle joint and the end point have to go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TwoBoneSolution {
    /// New position of the middle joint.
    pub mid: Vec3,
    /// New position of the end point (the target when reachable, else the fully extended point toward it).
    pub end: Vec3,
    /// The target was out of reach (too far, or too close for the two lengths) and was clamped.
    pub clamped: bool,
}

/// Analytic two-bone IK in a plane (law of cosines).
///
/// `a` is the root joint (stays fixed), `b` the middle joint, `c` the end point, `target` where the end point should
/// go. The bone lengths `|b - a|` and `|c - b|` are kept. The bend direction comes from `pole`: a direction from `a`
/// (it is made perpendicular to the root-to-target line); with no usable pole the current bend side of `b` is kept,
/// and a currently straight limb bends toward a fixed perpendicular. A target farther than `l1 + l2` gives the fully
/// extended limb pointing at the target (`clamped`), one closer than `|l1 - l2|` gives the fully folded limb.
///
/// `None` when any input is non-finite or a bone has (almost) no length.
pub fn solve_two_bone(a: Vec3, b: Vec3, c: Vec3, target: Vec3, pole: Option<Vec3>) -> Option<TwoBoneSolution> {
    if !(a.is_finite() && b.is_finite() && c.is_finite() && target.is_finite()) {
        return None;
    }
    let l1 = (b - a).length();
    let l2 = (c - b).length();
    if l1 < 1.0e-5 || l2 < 1.0e-5 || !l1.is_finite() || !l2.is_finite() {
        return None;
    }
    let to_target = target - a;
    let raw_d = to_target.length();
    // Direction to the target; a target on top of the root keeps the current end direction.
    let t_hat = try_normalize(to_target).or_else(|| try_normalize(c - a)).unwrap_or(Vec3::NEG_Y);
    let d_min = (l1 - l2).abs().max(1.0e-5);
    let d_max = l1 + l2;
    let d = raw_d.clamp(d_min, d_max);
    let clamped = raw_d > d_max || raw_d < d_min;

    // Bend direction: perpendicular to t_hat.
    let perp = |v: Vec3| try_normalize(v - t_hat * v.dot(t_hat));
    let bend = pole
        .and_then(perp)
        .or_else(|| perp(b - a))
        .or_else(|| perp(c - b))
        .unwrap_or_else(|| any_perpendicular(t_hat));

    let cos_a = ((l1 * l1 + d * d - l2 * l2) / (2.0 * l1 * d)).clamp(-1.0, 1.0);
    let sin_a = (1.0 - cos_a * cos_a).max(0.0).sqrt();
    let mid = a + (t_hat * cos_a + bend * sin_a) * l1;
    // The end point sits at distance l2 from `mid`, on the line to the (clamped) target point.
    let end = a + t_hat * d;
    let sol = TwoBoneSolution { mid, end, clamped };
    if sol.mid.is_finite() && sol.end.is_finite() {
        Some(sol)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arc_maps_from_to_to() {
        let q = rotation_arc(Vec3::X, Vec3::Y);
        assert!((q * Vec3::X - Vec3::Y).length() < 1e-5);
        let q = rotation_arc(Vec3::X, Vec3::NEG_X);
        assert!((q * Vec3::X - Vec3::NEG_X).length() < 1e-5);
        assert_eq!(rotation_arc(Vec3::ZERO, Vec3::X), Quat::IDENTITY);
        assert_eq!(rotation_arc(Vec3::new(f32::NAN, 0.0, 0.0), Vec3::X), Quat::IDENTITY);
    }

    #[test]
    fn aim_frame_angles_round_trip() {
        let f = AimFrame::DEFAULT;
        let q = f.rotation_deg(30.0, 20.0);
        let (y, p) = f.angles_deg(q * f.forward);
        assert!((y - 30.0).abs() < 1e-3 && (p - 20.0).abs() < 1e-3, "{y} {p}");
        // Right is +X: a target on +X is positive yaw.
        assert!(f.angles_deg(Vec3::new(1.0, 0.0, -1.0)).0 > 44.0);
    }

    #[test]
    fn cone_clamp_limits_the_angle() {
        let d = clamp_to_cone(Vec3::NEG_Z, Vec3::X, 30.0);
        assert!((angle_between_deg(d, Vec3::NEG_Z) - 30.0).abs() < 1e-3);
    }

    #[test]
    fn two_bone_pure() {
        let a = Vec3::ZERO;
        let b = Vec3::new(0.0, -4.0, 1.0);
        let c = Vec3::new(0.0, -8.0, 0.0);
        let s = solve_two_bone(a, b, c, Vec3::new(3.0, -5.0, 0.0), Some(Vec3::Z)).unwrap();
        assert!(!s.clamped);
        assert!(((s.mid - a).length() - (b - a).length()).abs() < 1e-4);
        assert!(((s.end - s.mid).length() - (c - b).length()).abs() < 1e-4);
        assert!((s.end - Vec3::new(3.0, -5.0, 0.0)).length() < 1e-4);
        assert!(solve_two_bone(a, a, c, c, None).is_none());
        assert!(solve_two_bone(a, b, c, Vec3::NAN, None).is_none());
    }
}
