//! The skeleton interface shared by the model loaders (F4M-1) and the animation runtime (F4M-2).
//!
//! Units and conventions follow Blockbench's Bedrock export: positions in model pixels (1/16 block), rotations in
//! degrees about X, then Y, then Z as Blockbench applies them; the loaders and the golden poses pin the exact order.

use glam::{Mat4, Quat, Vec3};

/// Index of a bone in [`Skeleton::bones`]. Parents always come before their children.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BoneIndex(pub u16);

impl BoneIndex {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// One bone at rest.
#[derive(Clone, Debug, PartialEq)]
pub struct Bone {
    pub name: String,
    pub parent: Option<BoneIndex>,
    /// Pivot in model space (pixels).
    pub pivot: Vec3,
    /// Rest rotation in degrees (x, y, z).
    pub rotation: Vec3,
}

/// Bones in parent-before-child order, with a name lookup.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    /// The bone named `name`.
    pub fn find(&self, name: &str) -> Option<BoneIndex> {
        self.bones.iter().position(|b| b.name == name).map(|i| BoneIndex(i as u16))
    }

    pub fn len(&self) -> usize {
        self.bones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bones.is_empty()
    }
}

/// An animated bone's offset from its rest state, in the bone's local frame (what animations produce).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneTransform {
    /// Translation in pixels.
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl BoneTransform {
    pub const IDENTITY: Self = Self { translation: Vec3::ZERO, rotation: Quat::IDENTITY, scale: Vec3::ONE };
}

impl Default for BoneTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

/// One frame's per-bone local transforms, indexed like [`Skeleton::bones`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    pub bones: Vec<BoneTransform>,
}

impl Pose {
    /// The rest pose of `skeleton` (every bone identity).
    pub fn rest(skeleton: &Skeleton) -> Self {
        Self { bones: vec![BoneTransform::IDENTITY; skeleton.len()] }
    }
}

/// Model-space matrices of every bone for `pose` (rest rotation and pivot composed with the pose, in parent order).
/// F4M-1 pins the exact convention with golden tests; F4M-2 uses this to check poses.
pub fn bone_matrices(skeleton: &Skeleton, pose: &Pose) -> Vec<Mat4> {
    let mut out: Vec<Mat4> = Vec::with_capacity(skeleton.len());
    for (i, bone) in skeleton.bones.iter().enumerate() {
        let t = pose.bones.get(i).copied().unwrap_or_default();
        let rest = Quat::from_euler(
            glam::EulerRot::ZYX,
            bone.rotation.z.to_radians(),
            bone.rotation.y.to_radians(),
            bone.rotation.x.to_radians(),
        );
        let local = Mat4::from_translation(bone.pivot + t.translation)
            * Mat4::from_quat(rest * t.rotation)
            * Mat4::from_scale(t.scale)
            * Mat4::from_translation(-bone.pivot);
        let parent = bone.parent.map_or(Mat4::IDENTITY, |p| out[p.index()]);
        out.push(parent * local);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_pose_gives_identity_matrices_without_rest_rotation() {
        let skeleton = Skeleton {
            bones: vec![
                Bone { name: "body".into(), parent: None, pivot: Vec3::new(0.0, 8.0, 0.0), rotation: Vec3::ZERO },
                Bone {
                    name: "head".into(),
                    parent: Some(BoneIndex(0)),
                    pivot: Vec3::new(0.0, 12.0, -4.0),
                    rotation: Vec3::ZERO,
                },
            ],
        };
        assert_eq!(skeleton.find("head"), Some(BoneIndex(1)));
        let m = bone_matrices(&skeleton, &Pose::rest(&skeleton));
        assert!(m.iter().all(|m| m.abs_diff_eq(Mat4::IDENTITY, 1e-6)));
    }
}
