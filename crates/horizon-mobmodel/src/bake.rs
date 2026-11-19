//! Bakes a [`Model`] into bone-local vertex arrays.
//!
//! "Bone-local" means the vertices are stored per bone in the model's rest space (absolute positions, cube rotations
//! and inflate already applied). The renderer transforms them with that bone's matrix from
//! [`crate::skeleton::bone_matrices`], which already contains the pivot translation.
//!
//! # Conventions (pinned by golden tests)
//!
//! * Each cube face gives 4 vertices in the order top-left, top-right, bottom-right, bottom-left of its texture
//!   rectangle as seen from outside the cube, with indices `[0, 3, 2, 0, 2, 1]` (counter-clockwise from outside).
//! * Box UV layout (offset `(u, v)`, size `(w, h, d)` = X, Y, Z): east `(u, v+d)..(u+d, v+d+h)`; north
//!   `(u+d, v+d)..(u+d+w, v+d+h)`; west `(u+d+w, v+d)..(u+2d+w, v+d+h)`; south `(u+2d+w, v+d)..(u+2d+2w, v+d+h)`;
//!   up `(u+d, v+d)..(u+d+w, v)` (V reversed); down `(u+d+w, v+d)..(u+d+2w, v)` (V runs backwards).
//! * `mirror` swaps the east and west rectangles and flips every rectangle left to right.
//! * Viewed from outside, a north face has +X on its left, south +X on its right, east +Z on its left, west +Z on its
//!   right, up and down +X on the right; up has north at the top of its (V reversed) rectangle, down has south at the top.
//! * A UV rotation of 90 degrees turns the image clockwise on the face: the corner `i` takes the texture corner
//!   `(i - rotation / 90) mod 4`.
//! * UVs are normalised by the texture size (0..1). Mesh faces are fan-triangulated `(0, i, i + 1)`.

use crate::model::*;
use crate::skeleton::BoneIndex;
use glam::{Vec2, Vec3};

/// One vertex ready for the GPU.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BakedVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// Normalised texture coordinates.
    pub uv: [f32; 2],
}

/// The triangles of one bone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedBone {
    pub vertices: Vec<BakedVertex>,
    /// Triangle list indices into `vertices`.
    pub indices: Vec<u32>,
}

/// All bones of a model, indexed like the skeleton.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BakedModel {
    pub bones: Vec<BakedBone>,
}

impl BakedModel {
    pub fn vertex_count(&self) -> usize {
        self.bones.iter().map(|b| b.vertices.len()).sum()
    }

    pub fn triangle_count(&self) -> usize {
        self.bones.iter().map(|b| b.indices.len() / 3).sum()
    }

    pub fn bone(&self, bone: BoneIndex) -> Option<&BakedBone> {
        self.bones.get(bone.index())
    }

    /// Bounding box of every baked vertex at rest.
    pub fn aabb(&self) -> Option<(Vec3, Vec3)> {
        let mut it = self.bones.iter().flat_map(|b| b.vertices.iter()).map(|v| Vec3::from(v.position));
        let first = it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| (lo.min(p), hi.max(p))))
    }
}

impl Model {
    /// Bakes the model (see the module docs). Bones with `never_render` come out empty.
    pub fn bake(&self) -> BakedModel {
        bake(self)
    }
}

/// Bakes `model`.
pub fn bake(model: &Model) -> BakedModel {
    let tex = Vec2::new(model.texture_size[0].max(1) as f32, model.texture_size[1].max(1) as f32);
    let mut out = BakedModel { bones: Vec::with_capacity(model.parts.len()) };
    for part in &model.parts {
        let mut bone = BakedBone::default();
        if !part.never_render {
            for cube in &part.cubes {
                bake_cube(cube, tex, &mut bone);
            }
            for mesh in &part.meshes {
                bake_mesh(mesh, tex, &mut bone);
            }
        }
        out.bones.push(bone);
    }
    out
}

/// Texture rectangles `(u1, v1, u2, v2)` in texels for a box-UV cube, indexed by `Face as usize`.
pub fn box_uv_rects(offset: [f32; 2], size: Vec3, mirror: bool) -> [[f32; 4]; 6] {
    let (u, v) = (offset[0], offset[1]);
    let (w, h, d) = (size.x, size.y, size.z);
    let mut r = [[0.0; 4]; 6];
    r[Face::East as usize] = [u, v + d, u + d, v + d + h];
    r[Face::North as usize] = [u + d, v + d, u + d + w, v + d + h];
    r[Face::West as usize] = [u + d + w, v + d, u + 2.0 * d + w, v + d + h];
    r[Face::South as usize] = [u + 2.0 * d + w, v + d, u + 2.0 * d + 2.0 * w, v + d + h];
    r[Face::Up as usize] = [u + d, v + d, u + d + w, v];
    r[Face::Down as usize] = [u + d + w, v + d, u + d + 2.0 * w, v];
    if mirror {
        r.swap(Face::East as usize, Face::West as usize);
        for rect in &mut r {
            rect.swap(0, 2);
        }
    }
    r
}

/// The four corners (top-left, top-right, bottom-right, bottom-left as seen from outside) of `face`.
fn face_corners(face: Face, lo: Vec3, hi: Vec3) -> [Vec3; 4] {
    let (x0, y0, z0) = (lo.x, lo.y, lo.z);
    let (x1, y1, z1) = (hi.x, hi.y, hi.z);
    match face {
        Face::North => [Vec3::new(x1, y1, z0), Vec3::new(x0, y1, z0), Vec3::new(x0, y0, z0), Vec3::new(x1, y0, z0)],
        Face::South => [Vec3::new(x0, y1, z1), Vec3::new(x1, y1, z1), Vec3::new(x1, y0, z1), Vec3::new(x0, y0, z1)],
        Face::East => [Vec3::new(x1, y1, z1), Vec3::new(x1, y1, z0), Vec3::new(x1, y0, z0), Vec3::new(x1, y0, z1)],
        Face::West => [Vec3::new(x0, y1, z0), Vec3::new(x0, y1, z1), Vec3::new(x0, y0, z1), Vec3::new(x0, y0, z0)],
        Face::Up => [Vec3::new(x0, y1, z0), Vec3::new(x1, y1, z0), Vec3::new(x1, y1, z1), Vec3::new(x0, y1, z1)],
        Face::Down => [Vec3::new(x0, y0, z1), Vec3::new(x1, y0, z1), Vec3::new(x1, y0, z0), Vec3::new(x0, y0, z0)],
    }
}

fn bake_cube(cube: &Cube, tex: Vec2, out: &mut BakedBone) {
    let lo = cube.origin - Vec3::splat(cube.inflate);
    let hi = cube.origin + cube.size + Vec3::splat(cube.inflate);
    let rot = euler_matrix(cube.rotation);
    let rotated = cube.rotation != Vec3::ZERO;
    let box_rects = match &cube.uv {
        CubeUv::Box { offset } => Some(box_uv_rects(*offset, cube.size, cube.mirror)),
        CubeUv::PerFace(_) => None,
    };
    for face in Face::ALL {
        let fi = face as usize;
        if !cube.faces_enabled[fi] {
            continue;
        }
        let (rect, rotation) = match (&cube.uv, &box_rects) {
            (_, Some(r)) => (r[fi], 0u16),
            (CubeUv::PerFace(faces), None) => match faces[fi] {
                Some(f) => ([f.uv[0], f.uv[1], f.uv[0] + f.uv_size[0], f.uv[1] + f.uv_size[1]], f.rotation),
                None => continue,
            },
            _ => continue,
        };
        // Texture corners in the same order as the face corners: top-left, top-right, bottom-right, bottom-left.
        let tc = [[rect[0], rect[1]], [rect[2], rect[1]], [rect[2], rect[3]], [rect[0], rect[3]]];
        let shift = (rotation / 90) as usize % 4;
        let corners = face_corners(face, lo, hi);
        let normal = if rotated { rot * face.normal() } else { face.normal() };
        let base = out.vertices.len() as u32;
        for (i, c) in corners.iter().enumerate() {
            let p = if rotated { cube.pivot + rot * (*c - cube.pivot) } else { *c };
            let t = tc[(i + 4 - shift) % 4];
            out.vertices.push(BakedVertex {
                position: p.to_array(),
                normal: normal.to_array(),
                uv: [t[0] / tex.x, t[1] / tex.y],
            });
        }
        out.indices.extend_from_slice(&[base, base + 3, base + 2, base, base + 2, base + 1]);
    }
}

fn bake_mesh(mesh: &Mesh, tex: Vec2, out: &mut BakedBone) {
    for face in &mesh.faces {
        let n = face.corners.len();
        if n < 3 {
            continue;
        }
        let positions: Vec<Vec3> =
            face.corners.iter().map(|c| mesh.vertices.get(c.vertex as usize).copied().unwrap_or(Vec3::ZERO)).collect();
        // Newell's method: robust for n-gons.
        let mut normal = Vec3::ZERO;
        for i in 0..n {
            let a = positions[i];
            let b = positions[(i + 1) % n];
            normal += Vec3::new((a.y - b.y) * (a.z + b.z), (a.z - b.z) * (a.x + b.x), (a.x - b.x) * (a.y + b.y));
        }
        let flat = normal.try_normalize().unwrap_or(Vec3::Y);
        let base = out.vertices.len() as u32;
        for (c, p) in face.corners.iter().zip(&positions) {
            let nrm = c.normal.and_then(Vec3::try_normalize).unwrap_or(flat);
            out.vertices.push(BakedVertex {
                position: p.to_array(),
                normal: nrm.to_array(),
                uv: [c.uv[0] / tex.x, c.uv[1] / tex.y],
            });
        }
        for i in 1..(n as u32 - 1) {
            out.indices.extend_from_slice(&[base, base + i, base + i + 1]);
        }
    }
}
