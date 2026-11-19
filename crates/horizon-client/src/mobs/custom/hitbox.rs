//! Hit and pick parts of a custom mob (`hitboxes`, FR-F4-MOBS.md section 3.4).
//!
//! A mob has exactly one collision box, `size` (world collision, spawn space, pushing: the engine's
//! `Mob::bounds`). The listed parts are extra boxes that are only hurt and pick boxes (the way the
//! dragon's wings and heads are), each with a damage multiplier.
//!
//! Part boxes: a box of `size = [width, height]` whose bottom centre is at `offset` (blocks) from
//! the mob's feet, `+z` forward and `+x` right-handed as the model faces (at yaw 0 the mob faces
//! `+z` and `+x` is east), turned with the body yaw when `rotation_yaw_follow`. A part with `bone`
//! has its offset added to the bone's pivot instead of to the feet; the pivot is the bone's rest
//! position (the model turned half a circle: Blockbench's front is north, the mob's is south). The
//! synced-clock pose of the bone (the animated position) belongs to the animation sync item (F4M-5b).
//!
//! Damage: [`scale_damage`] picks the part a blow's aim ray enters first (a part wins a tie with
//! the main box) and multiplies the damage by its `damage_multiplier`; no aim (a fall, `/damage`)
//! or a ray that misses every box is plain damage. A projectile's aim is its owner's, so a part
//! hit by an arrow is the part the owner was looking at. [`pick`] is the crosshair's distance to
//! the nearest pick part or to the main box.
use super::CustomType;
use crate::mobs::{Aim, Mob};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_mods::custom_mob::{Hitbox, LoadedModel};

/// One hurt and pick part, resolved against the model.
#[derive(Clone, Debug, PartialEq)]
pub struct HitPart {
    pub name: String,
    pub width: f64,
    pub height: f64,
    /// From the feet (no bone) or the bone pivot, in the mob's frame.
    pub offset: [f64; 3],
    /// The bone's rest pivot in the mob's frame (blocks), when the part follows a bone.
    pub bone: Option<[f64; 3]>,
    pub damage_multiplier: f64,
    pub pick: bool,
    pub hurt: bool,
    pub yaw_follow: bool,
}

/// Model space (pixels, front north) to the mob's frame (blocks, front south): half a turn.
pub fn model_to_mob(p: [f64; 3]) -> [f64; 3] {
    [-p[0] / 16.0, p[1] / 16.0, -p[2] / 16.0]
}

impl HitPart {
    pub fn resolve(h: &Hitbox, model: Option<&LoadedModel>) -> Result<Self, String> {
        let bone = match (&h.bone, model) {
            (Some(name), Some(m)) => Some(model_to_mob(m.bone_rest_position(name).ok_or_else(
                || format!("hitbox {}: the model has no bone {name:?}", h.name),
            )?)),
            (Some(name), None) => {
                return Err(format!("hitbox {}: bone {name:?} needs the model", h.name));
            }
            (None, _) => None,
        };
        Ok(Self {
            name: h.name.clone(),
            width: h.size[0],
            height: h.size[1],
            offset: h.offset,
            bone,
            damage_multiplier: h.damage_multiplier,
            pick: h.pick,
            hurt: h.hurt,
            yaw_follow: h.rotation_yaw_follow,
        })
    }

    /// The part's box for a mob with its feet at `feet` and body yaw `yaw` (degrees).
    pub fn bounds(&self, feet: [f64; 3], yaw: f32) -> Aabb {
        let anchor = self.bone.unwrap_or([0.0; 3]);
        let local = [
            anchor[0] + self.offset[0],
            anchor[1] + self.offset[1],
            anchor[2] + self.offset[2],
        ];
        let (x, z) = if self.yaw_follow {
            let a = f64::from(yaw).to_radians();
            let (s, c) = a.sin_cos();
            (local[0] * c - local[2] * s, local[0] * s + local[2] * c)
        } else {
            (local[0], local[2])
        };
        let (cx, cy, cz) = (feet[0] + x, feet[1] + local[1], feet[2] + z);
        let w = self.width / 2.0;
        Aabb::new(
            Vec3::new(cx - w, cy, cz - w),
            Vec3::new(cx + w, cy + self.height, cz + w),
        )
    }
}

/// The distance along `dir` (a unit vector) from `eye` to where the ray enters `b`, within `reach`.
pub fn ray_enter(eye: [f64; 3], dir: [f64; 3], reach: f64, b: &Aabb) -> Option<f64> {
    let (lo, hi) = (b.min_position(), b.max_position());
    let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
    let (mut near, mut far) = (0.0f64, reach);
    for a in 0..3 {
        if dir[a].abs() < 1e-12 {
            if eye[a] < lo[a] || eye[a] > hi[a] {
                return None;
            }
            continue;
        }
        let (t0, t1) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
        near = near.max(t0.min(t1));
        far = far.min(t0.max(t1));
        if near > far {
            return None;
        }
    }
    Some(near)
}

fn normalized(look: [f64; 3]) -> Option<[f64; 3]> {
    let n = (look[0] * look[0] + look[1] * look[1] + look[2] * look[2]).sqrt();
    (n.is_finite() && n > 1e-9).then(|| [look[0] / n, look[1] / n, look[2] / n])
}

/// Which part a ray enters first: `(name, multiplier, distance)`. The main box is `None`, with
/// multiplier 1. Parts with `hurt: false` are not candidates.
pub fn hurt_part<'a>(
    t: &'a CustomType,
    main: &Aabb,
    feet: [f64; 3],
    yaw: f32,
    eye: [f64; 3],
    look: [f64; 3],
    reach: f64,
) -> Option<(Option<&'a str>, f64, f64)> {
    let dir = normalized(look)?;
    let mut best = ray_enter(eye, dir, reach, main).map(|d| (None, 1.0, d));
    for p in t.hitboxes.iter().filter(|p| p.hurt) {
        if let Some(d) = ray_enter(eye, dir, reach, &p.bounds(feet, yaw))
            && best.is_none_or(|b| d <= b.2 + 1e-9)
        {
            best = Some((Some(p.name.as_str()), p.damage_multiplier, d));
        }
    }
    best
}

/// A blow's damage after the part its aim hit (see the module).
pub fn scale_damage(m: &Mob, amount: f32, aim: Option<Aim>) -> f32 {
    let (Some(t), Some(aim)) = (&m.kind.custom, aim) else {
        return amount;
    };
    if t.hitboxes.is_empty() {
        return amount;
    }
    let feet = [m.position.x, m.position.y, m.position.z];
    match hurt_part(t, &m.bounds(), feet, m.yaw, aim.eye, aim.look, 64.0) {
        Some((_, multiplier, _)) => (f64::from(amount) * multiplier) as f32,
        None => amount,
    }
}

/// The crosshair's distance to a custom mob: its main box, or a part with `pick` (which may
/// stick out of the box).
pub fn pick(m: &Mob, main: &Aabb, eye: [f64; 3], dir: [f64; 3], reach: f64) -> Option<f64> {
    let t = m.kind.custom.as_ref()?;
    let feet = [m.position.x, m.position.y, m.position.z];
    let mut best = ray_enter(eye, dir, reach, main);
    for p in t.hitboxes.iter().filter(|p| p.pick) {
        if let Some(d) = ray_enter(eye, dir, reach, &p.bounds(feet, m.yaw))
            && best.is_none_or(|b| d < b)
        {
            best = Some(d);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str, size: [f64; 2], offset: [f64; 3], multiplier: f64) -> HitPart {
        HitPart {
            name: name.into(),
            width: size[0],
            height: size[1],
            offset,
            bone: None,
            damage_multiplier: multiplier,
            pick: true,
            hurt: true,
            yaw_follow: true,
        }
    }
    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_part_follows_the_body_yaw() {
        // A head 0.5 in front of the feet, 0.4 wide: at yaw 0 the mob faces +z.
        let head = part("head", [0.4, 0.4], [0.0, 0.5, 0.5], 1.5);
        let b = head.bounds([10.0, 64.0, 20.0], 0.0);
        let (lo, hi) = (b.min_position(), b.max_position());
        assert!(near(lo.x, 9.8) && near(hi.x, 10.2));
        assert!(near(lo.y, 64.5) && near(hi.y, 64.9));
        assert!(near(lo.z, 20.3) && near(hi.z, 20.7));
        // A quarter turn (yaw 90 faces -x): the head is in front, now at -x.
        let b = head.bounds([10.0, 64.0, 20.0], 90.0);
        let (lo, hi) = (b.min_position(), b.max_position());
        assert!(near(lo.x, 9.3) && near(hi.x, 9.7), "{lo:?} {hi:?}");
        assert!(near(lo.z, 19.8) && near(hi.z, 20.2));
        // Without rotation_yaw_follow it stays where the type put it.
        let still = HitPart { yaw_follow: false, ..head };
        let b = still.bounds([10.0, 64.0, 20.0], 90.0);
        assert!(near(b.min_position().z, 20.3));
    }

    #[test]
    fn a_bone_part_hangs_from_the_bone_pivot() {
        // A pivot 12 px up and 8 px behind the model's origin along -z (the model's front):
        // in the mob's frame that is in front, since the model is turned half a circle.
        assert_eq!(model_to_mob([0.0, 12.0, -8.0]), [-0.0, 0.75, 0.5]);
        let mut p = part("head", [0.4, 0.4], [0.0, 0.0, 0.0], 1.5);
        p.bone = Some(model_to_mob([0.0, 12.0, -8.0]));
        let b = p.bounds([0.0, 0.0, 0.0], 0.0);
        assert!(near(b.min_position().y, 0.75) && near(b.max_position().y, 1.15));
        assert!(near(b.min_position().z, 0.3) && near(b.max_position().z, 0.7));
    }

    #[test]
    fn rays_enter_boxes_where_they_should() {
        let b = Aabb::new(Vec3::new(-0.5, 0.0, -0.5), Vec3::new(0.5, 1.0, 0.5));
        let d = ray_enter([0.0, 0.5, -3.0], [0.0, 0.0, 1.0], 10.0, &b).unwrap();
        assert!(near(d, 2.5));
        assert!(ray_enter([0.0, 0.5, -3.0], [0.0, 0.0, 1.0], 2.0, &b).is_none());
        assert!(ray_enter([2.0, 0.5, -3.0], [0.0, 0.0, 1.0], 10.0, &b).is_none());
        // Inside the box: distance 0.
        assert!(near(ray_enter([0.0, 0.5, 0.0], [1.0, 0.0, 0.0], 10.0, &b).unwrap(), 0.0));
    }
}
