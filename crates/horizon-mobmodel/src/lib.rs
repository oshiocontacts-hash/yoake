//! Custom mob models and animation (FR-F4-MOB, `docs/yoake2/FR-F4-MOBS.md` section 3.6).
//!
//! No GPU and no game dependencies: the client and the server both use it.
//!
//! - F4M-1 (`model`, `geo`, `bbmodel`, `bake`): `.geo.json` and `.bbmodel` to [`Model`], bone-local baked vertices.
//! - F4M-2 (`anim`, `expr`, `controller`, `pose`): animations, the expression language, controllers, pose evaluation.
//!
//! The shared interface between the two is this file: [`BoneIndex`], [`Bone`], [`Skeleton`], [`BoneTransform`] and
//! [`Pose`]. Change it only in agreement (both branches build on it).

pub mod anim;
pub mod clock;
pub mod controller;
pub mod expr;
pub mod pose;
pub mod skeleton;

pub use skeleton::{Bone, BoneIndex, BoneTransform, Pose, Skeleton};

/// Limits per mob type (FR-F4-MOBS.md F4M-0).
pub mod limits {
    /// Bones in one model.
    pub const MAX_BONES: usize = 256;
    /// Cubes (and mesh elements) in one model.
    pub const MAX_CUBES: usize = 4096;
    /// Animations per mob type.
    pub const MAX_ANIMATIONS: usize = 128;
    /// Synced values per mob type.
    pub const MAX_SYNCED: usize = 32;
    /// Bytecode instructions in one compiled expression.
    pub const MAX_EXPR_INSTRUCTIONS: usize = 256;
}
