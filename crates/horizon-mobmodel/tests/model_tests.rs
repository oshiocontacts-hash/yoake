//! Golden and behaviour tests for the loaders and the bake (F4M-1).

use glam::{Mat3, Mat4, Vec3};
use horizon_mobmodel::skeleton::{bone_matrices, Pose};
use horizon_mobmodel::*;
use serde_json::{json, Value};

const FOX_BB: &[u8] = include_bytes!("fixtures/test_fox.bbmodel");
const FOX_GEO: &[u8] = include_bytes!("fixtures/test_fox.geo.json");
const OLD_GEO: &[u8] = include_bytes!("fixtures/cube_old.geo.json");

fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}

/// A 1.12 geo file with one geometry of the given texture size and bones.
fn geo(tex: u32, bones: Value) -> Vec<u8> {
    bytes(&json!({
        "format_version": "1.12.0",
        "minecraft:geometry": [{
            "description": {"identifier": "geometry.t", "texture_width": tex, "texture_height": tex},
            "bones": bones
        }]
    }))
}

fn load_geo(tex: u32, bones: Value) -> Model {
    load_geo_json("t.geo.json", &geo(tex, bones)).expect("geo loads")
}

/// A bbmodel with the given outliner and elements.
fn bb(tex: u32, outliner: Value, elements: Value) -> Vec<u8> {
    bytes(&json!({
        "meta": {"format_version": "4.10"},
        "name": "t",
        "resolution": {"width": tex, "height": tex},
        "elements": elements,
        "outliner": outliner
    }))
}

fn approx(a: f32, b: f32, eps: f32) -> bool {
    (a - b).abs() <= eps
}

fn assert_v3(got: [f32; 3], want: [f32; 3], what: &str) {
    assert!(got.iter().zip(&want).all(|(g, w)| approx(*g, *w, 1e-4)), "{what}: got {got:?}, want {want:?}");
}

/// Vertices of bone `i` as (position, uv in texels).
fn texel_verts(baked: &BakedModel, bone: usize, tex: f32) -> Vec<([f32; 3], [f32; 2])> {
    baked.bones[bone].vertices.iter().map(|v| (v.position, [v.uv[0] * tex, v.uv[1] * tex])).collect()
}

fn assert_verts(got: &[([f32; 3], [f32; 2])], want: &[([f32; 3], [f32; 2])]) {
    assert_eq!(got.len(), want.len(), "vertex count");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert_v3(g.0, w.0, &format!("vertex {i} position"));
        assert!(approx(g.1[0], w.1[0], 1e-3) && approx(g.1[1], w.1[1], 1e-3), "vertex {i} uv: got {:?}, want {:?}", g.1, w.1);
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// Golden vertex sets

/// Cube origin [1,0,0] size [2,3,4] in a geo file is x in [-3,-1] internally. Texture 16x16, uv [0,0].
fn golden_cube_geo(extra: Value) -> Model {
    let mut cube = json!({"origin": [1, 0, 0], "size": [2, 3, 4], "uv": [0, 0]});
    if let (Some(c), Some(e)) = (cube.as_object_mut(), extra.as_object()) {
        for (k, v) in e {
            c.insert(k.clone(), v.clone());
        }
    }
    load_geo(16, json!([{"name": "b", "pivot": [0, 0, 0], "cubes": [cube]}]))
}

#[test]
fn golden_cube_vertices() {
    let m = golden_cube_geo(json!({}));
    let baked = m.bake();
    let got = texel_verts(&baked, 0, 16.0);
    let want: Vec<([f32; 3], [f32; 2])> = vec![
        // north (z = 0): +X on the left
        ([-1.0, 3.0, 0.0], [4.0, 4.0]),
        ([-3.0, 3.0, 0.0], [6.0, 4.0]),
        ([-3.0, 0.0, 0.0], [6.0, 7.0]),
        ([-1.0, 0.0, 0.0], [4.0, 7.0]),
        // east (x = -1): +Z on the left
        ([-1.0, 3.0, 4.0], [0.0, 4.0]),
        ([-1.0, 3.0, 0.0], [4.0, 4.0]),
        ([-1.0, 0.0, 0.0], [4.0, 7.0]),
        ([-1.0, 0.0, 4.0], [0.0, 7.0]),
        // south (z = 4)
        ([-3.0, 3.0, 4.0], [10.0, 4.0]),
        ([-1.0, 3.0, 4.0], [12.0, 4.0]),
        ([-1.0, 0.0, 4.0], [12.0, 7.0]),
        ([-3.0, 0.0, 4.0], [10.0, 7.0]),
        // west (x = -3)
        ([-3.0, 3.0, 0.0], [6.0, 4.0]),
        ([-3.0, 3.0, 4.0], [10.0, 4.0]),
        ([-3.0, 0.0, 4.0], [10.0, 7.0]),
        ([-3.0, 0.0, 0.0], [6.0, 7.0]),
        // up (y = 3): V runs backwards, north at the top
        ([-3.0, 3.0, 0.0], [4.0, 4.0]),
        ([-1.0, 3.0, 0.0], [6.0, 4.0]),
        ([-1.0, 3.0, 4.0], [6.0, 0.0]),
        ([-3.0, 3.0, 4.0], [4.0, 0.0]),
        // down (y = 0): V runs backwards, south at the top
        ([-3.0, 0.0, 4.0], [6.0, 4.0]),
        ([-1.0, 0.0, 4.0], [8.0, 4.0]),
        ([-1.0, 0.0, 0.0], [8.0, 0.0]),
        ([-3.0, 0.0, 0.0], [6.0, 0.0]),
    ];
    assert_verts(&got, &want);
    assert_eq!(baked.bones[0].indices.len(), 36);
    assert_eq!(&baked.bones[0].indices[..6], &[0, 3, 2, 0, 2, 1]);
}

#[test]
fn cube_triangles_wind_counter_clockwise_outward_with_matching_normals() {
    for extra in [json!({}), json!({"rotation": [20, 35, 50], "pivot": [1, 1, 1]}), json!({"mirror": true})] {
        let baked = golden_cube_geo(extra).bake();
        let b = &baked.bones[0];
        for tri in b.indices.chunks(3) {
            let p: Vec<Vec3> = tri.iter().map(|&i| Vec3::from(b.vertices[i as usize].position)).collect();
            let n = Vec3::from(b.vertices[tri[0] as usize].normal);
            let face_n = (p[1] - p[0]).cross(p[2] - p[0]).normalize();
            assert!(face_n.dot(n) > 0.999, "triangle normal {face_n} vs vertex normal {n}");
        }
    }
}

#[test]
fn golden_mirrored_cube_swaps_and_flips_box_uv() {
    let plain = golden_cube_geo(json!({})).bake();
    let mirrored = golden_cube_geo(json!({"mirror": true})).bake();
    let got = texel_verts(&mirrored, 0, 16.0);
    // Geometry is unchanged.
    for (a, b) in plain.bones[0].vertices.iter().zip(&mirrored.bones[0].vertices) {
        assert_v3(a.position, b.position, "mirror keeps geometry");
    }
    let uvs: Vec<[f32; 2]> = got.iter().map(|v| v.1).collect();
    let want: Vec<[f32; 2]> = vec![
        // north, east, south, west, up, down; four corners each (TL, TR, BR, BL)
        [6.0, 4.0], [4.0, 4.0], [4.0, 7.0], [6.0, 7.0],
        [10.0, 4.0], [6.0, 4.0], [6.0, 7.0], [10.0, 7.0],
        [12.0, 4.0], [10.0, 4.0], [10.0, 7.0], [12.0, 7.0],
        [4.0, 4.0], [0.0, 4.0], [0.0, 7.0], [4.0, 7.0],
        [6.0, 4.0], [4.0, 4.0], [4.0, 0.0], [6.0, 0.0],
        [8.0, 4.0], [6.0, 4.0], [6.0, 0.0], [8.0, 0.0],
    ];
    assert_eq!(uvs, want);
}

#[test]
fn bone_mirror_and_inflate_are_inherited_unless_the_cube_sets_its_own() {
    let m = load_geo(
        16,
        json!([{"name": "b", "pivot": [0, 0, 0], "mirror": true, "inflate": 0.5, "cubes": [
            {"origin": [0, 0, 0], "size": [2, 2, 2], "uv": [0, 0]},
            {"origin": [0, 0, 0], "size": [2, 2, 2], "uv": [0, 0], "mirror": false, "inflate": 0.0}
        ]}]),
    );
    let c = &m.parts[0].cubes;
    assert!(c[0].mirror && c[0].inflate == 0.5);
    assert!(!c[1].mirror && c[1].inflate == 0.0);
    // inflate 0.5 grows the cube: x in [-2.5, 0.5]
    let (lo, hi) = m.bake().aabb().unwrap();
    assert_v3(lo.to_array(), [-2.5, -0.5, -0.5], "inflated lo");
    assert_v3(hi.to_array(), [0.5, 2.5, 2.5], "inflated hi");
}

#[test]
fn golden_per_face_uv_with_size_rotation_and_flip() {
    let m = golden_cube_geo(json!({"uv": {
        "north": {"uv": [1, 2], "uv_size": [2, 3]},
        "east": {"uv": [8, 8], "uv_size": [4, 3], "uv_rotation": 90},
        "west": {"uv": [6, 6], "uv_size": [-4, -3]},
        "up": {"uv": [0, 0]}
    }}));
    let cube = &m.parts[0].cubes[0];
    assert_eq!(cube.faces_enabled, [true, true, false, true, true, false]);
    let got = texel_verts(&m.bake(), 0, 16.0);
    let want: Vec<([f32; 3], [f32; 2])> = vec![
        // north: plain rectangle (1,2)-(3,5)
        ([-1.0, 3.0, 0.0], [1.0, 2.0]),
        ([-3.0, 3.0, 0.0], [3.0, 2.0]),
        ([-3.0, 0.0, 0.0], [3.0, 5.0]),
        ([-1.0, 0.0, 0.0], [1.0, 5.0]),
        // east: rectangle (8,8)-(12,11) turned 90 degrees clockwise
        ([-1.0, 3.0, 4.0], [8.0, 11.0]),
        ([-1.0, 3.0, 0.0], [8.0, 8.0]),
        ([-1.0, 0.0, 0.0], [12.0, 8.0]),
        ([-1.0, 0.0, 4.0], [12.0, 11.0]),
        // west: negative size flips both ways: (6,6)-(2,3)
        ([-3.0, 3.0, 0.0], [6.0, 6.0]),
        ([-3.0, 3.0, 4.0], [2.0, 6.0]),
        ([-3.0, 0.0, 4.0], [2.0, 3.0]),
        ([-3.0, 0.0, 0.0], [6.0, 3.0]),
        // up: geo uv_size defaults to the natural size (2 x 4) and up/down are stored flipped on both axes, so
        // geo {uv [0,0], size [2,4]} is the bbmodel rectangle (2,4)-(0,0)
        ([-3.0, 3.0, 0.0], [2.0, 4.0]),
        ([-1.0, 3.0, 0.0], [0.0, 4.0]),
        ([-1.0, 3.0, 4.0], [0.0, 0.0]),
        ([-3.0, 3.0, 4.0], [2.0, 0.0]),
    ];
    assert_verts(&got, &want);
}

#[test]
fn bbmodel_per_face_uv_matches_geo_per_face_uv() {
    let elements = json!([{
        "type": "cube", "uuid": "c1", "name": "c", "from": [-3, 0, 0], "to": [-1, 3, 4], "box_uv": false,
        "faces": {
            "north": {"uv": [1, 2, 3, 5], "texture": 0},
            "east": {"uv": [8, 8, 12, 11], "texture": 0, "rotation": 90},
            "west": {"uv": [6, 6, 2, 3], "texture": 0},
            "up": {"uv": [2, 4, 0, 0], "texture": 0},
            "south": {"uv": [0, 0, 1, 1], "texture": null}
        }
    }]);
    let m = load_bbmodel("t.bbmodel", &bb(16, json!([{"name": "b", "origin": [0, 0, 0], "children": ["c1"]}]), elements))
        .unwrap();
    let want = golden_cube_geo(json!({"uv": {
        "north": {"uv": [1, 2], "uv_size": [2, 3]},
        "east": {"uv": [8, 8], "uv_size": [4, 3], "uv_rotation": 90},
        "west": {"uv": [6, 6], "uv_size": [-4, -3]},
        "up": {"uv": [0, 0]}
    }}));
    assert_eq!(m.bake(), want.bake());
}

#[test]
fn golden_mesh_triangulation_origin_and_rotation() {
    // Mesh origin (1,2,3), rotated 90 degrees about Y: Ry(90) maps (x, y, z) to (z, y, -x).
    let elements = json!([{
        "type": "mesh", "uuid": "m1", "name": "m", "origin": [1, 2, 3], "rotation": [0, 90, 0],
        "vertices": {
            "a": [0, 0, 0], "b": [1, 0, 0], "c": [1, 1, 0], "d": [0, 1, 0], "e": [-1, 0.5, 0]
        },
        "faces": {
            "q": {"vertices": ["a", "b", "c", "d"], "uv": {"a": [0, 0], "b": [8, 0], "c": [8, 8], "d": [0, 8]}},
            "t": {"vertices": ["a", "b", "c"], "uv": {"a": [0, 0], "b": [8, 0], "c": [8, 8]}},
            "p": {"vertices": ["a", "b", "c", "d", "e"]},
            "line": {"vertices": ["a", "b"]}
        }
    }]);
    let m = load_bbmodel("t.bbmodel", &bb(16, json!([{"name": "g", "origin": [0, 0, 0], "children": ["m1"]}]), elements))
        .unwrap();
    assert!(m.warnings.iter().any(|w| w.contains("fewer than 3 vertices")), "{:?}", m.warnings);
    let baked = m.bake();
    let b = &baked.bones[0];
    // Faces are taken sorted by id: p (5 corners), q (4), t (3).
    assert_eq!(b.vertices.len(), 5 + 4 + 3);
    assert_eq!(b.indices, vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 5, 6, 7, 5, 7, 8, 9, 10, 11]);
    // Vertex a = origin + Ry(90)*(0,0,0) = (1,2,3); b = (1,0,0) -> (0,0,-1) -> (1,2,2); c = (1,1,0) -> (1,3,2);
    // e = (-1,0.5,0) -> (0,0.5,1) -> (1,2.5,4).
    assert_v3(b.vertices[0].position, [1.0, 2.0, 3.0], "p.a");
    assert_v3(b.vertices[1].position, [1.0, 2.0, 2.0], "p.b");
    assert_v3(b.vertices[2].position, [1.0, 3.0, 2.0], "p.c");
    assert_v3(b.vertices[4].position, [1.0, 2.5, 4.0], "p.e");
    // The flat normal of a counter-clockwise (a, b, c, d) quad in the XY plane is +Z, turned to +X by the rotation.
    for v in &b.vertices {
        assert_v3(v.normal, [1.0, 0.0, 0.0], "flat normal");
    }
    // Quad uv: (8, 8) of a 16 texture is 0.5.
    assert_eq!(b.vertices[7].uv, [0.5, 0.5]);
}

#[test]
fn golden_geo_poly_mesh_reverses_winding_and_mirrors_x() {
    let m = load_geo(
        16,
        json!([{"name": "b", "pivot": [0, 0, 0], "poly_mesh": {
            "normalized_uvs": true,
            "positions": [[0, 0, 0], [-2, 0, 0], [-2, 2, 0]],
            "normals": [[0, 0, -1]],
            "uvs": [[0, 0], [0.5, 0], [0.5, 0.5]],
            "polys": [[[0, 0, 0], [1, 0, 1], [2, 0, 2]]]
        }}]),
    );
    let mesh = &m.parts[0].meshes[0];
    assert_eq!(mesh.vertices[1], Vec3::new(2.0, 0.0, 0.0));
    // Reversed corner order: 2, 1, 0; uv is normalised to texels (x16).
    let order: Vec<u32> = mesh.faces[0].corners.iter().map(|c| c.vertex).collect();
    assert_eq!(order, vec![2, 1, 0]);
    assert_eq!(mesh.faces[0].corners[0].uv, [8.0, 8.0]);
    assert_eq!(mesh.faces[0].corners[0].normal, Some(Vec3::new(0.0, 0.0, -1.0)));
    // After the mirror the triangle (2,2,0), (2,0,0), (0,0,0) is counter-clockwise looking from -Z... its flat normal
    // points to -Z, which is the stated normal, so the winding survived the mirror.
    let b = &m.bake().bones[0];
    let p: Vec<Vec3> = b.indices.iter().map(|&i| Vec3::from(b.vertices[i as usize].position)).collect();
    assert!((p[1] - p[0]).cross(p[2] - p[0]).normalize().dot(Vec3::NEG_Z) > 0.999);
}

#[test]
fn golden_nested_pivots_bb_and_geo_agree() {
    // a: rotated 90 about Z at the origin. b (child of a): pivot (0,10,0), rotated 90 about X.
    // The point (0,10,-2) goes to (0,12,0) by b and then to (-12,0,0) by a.
    let bbm = load_bbmodel(
        "t.bbmodel",
        &bb(
            16,
            json!([{"name": "a", "origin": [0, 0, 0], "rotation": [0, 0, 90], "children": [
                {"name": "b", "origin": [0, 10, 0], "rotation": [90, 0, 0], "children": ["c1"]}]}]),
            json!([{"type": "cube", "uuid": "c1", "from": [-1, 9, -3], "to": [1, 11, -1], "box_uv": true}]),
        ),
    )
    .unwrap();
    let geom = load_geo(
        16,
        json!([
            {"name": "a", "pivot": [0, 0, 0], "rotation": [0, 0, 90]},
            {"name": "b", "parent": "a", "pivot": [0, 10, 0], "rotation": [-90, 0, 0], "cubes": [
                {"origin": [-1, 9, -3], "size": [2, 2, 2], "uv": [0, 0]}]}
        ]),
    );
    assert_eq!(bbm.skeleton, geom.skeleton);
    for m in [&bbm, &geom] {
        let mats = bone_matrices(&m.skeleton, &Pose::rest(&m.skeleton));
        let p = mats[1].transform_point3(Vec3::new(0.0, 10.0, -2.0));
        assert_v3(p.to_array(), [-12.0, 0.0, 0.0], "nested pivot point");
        let q = mats[0].transform_point3(Vec3::new(0.0, 10.0, -2.0));
        assert_v3(q.to_array(), [-10.0, 0.0, -2.0], "parent only");
    }
}

#[test]
fn rotation_order_is_z_then_y_then_x_applied_x_first() {
    let bbm = load_bbmodel(
        "t.bbmodel",
        &bb(16, json!([{"name": "a", "origin": [0, 0, 0], "rotation": [30, 40, 50]}]), json!([])),
    )
    .unwrap();
    let geom = load_geo(16, json!([{"name": "a", "pivot": [0, 0, 0], "rotation": [-30, -40, 50]}]));
    assert_eq!(bbm.skeleton.bones[0].rotation, Vec3::new(30.0, 40.0, 50.0));
    assert_eq!(geom.skeleton.bones[0].rotation, Vec3::new(30.0, 40.0, 50.0));
    let want = Mat3::from_rotation_z(50f32.to_radians())
        * Mat3::from_rotation_y(40f32.to_radians())
        * Mat3::from_rotation_x(30f32.to_radians());
    let m = bone_matrices(&bbm.skeleton, &Pose::rest(&bbm.skeleton))[0];
    for v in [Vec3::X, Vec3::Y, Vec3::Z, Vec3::new(1.0, 2.0, 3.0)] {
        assert!((m.transform_vector3(v) - want * v).length() < 1e-5);
    }
    // The cube rotation uses the same order.
    let c = load_bbmodel(
        "t.bbmodel",
        &bb(
            16,
            json!([{"name": "a", "origin": [0, 0, 0], "children": ["c1"]}]),
            json!([{"type": "cube", "uuid": "c1", "from": [1, 0, 0], "to": [2, 1, 1], "origin": [0, 0, 0],
                    "rotation": [0, 0, 90], "box_uv": true}]),
        ),
    )
    .unwrap();
    let (lo, hi) = c.bake().aabb().unwrap();
    assert_v3(lo.to_array(), [-1.0, 1.0, 0.0], "rotated cube lo");
    assert_v3(hi.to_array(), [0.0, 2.0, 1.0], "rotated cube hi");
}

#[test]
fn geo_bedrock_to_internal_mapping() {
    let m = load_geo(
        16,
        json!([{"name": "a", "pivot": [2, 3, 4], "rotation": [10, 20, 30],
                "locators": {"l": {"offset": [1, 2, 3], "rotation": [5, 6, 7]}, "p": [1, 1, 1]},
                "cubes": [{"origin": [1, 0, 0], "size": [2, 3, 4], "pivot": [5, 6, 7], "rotation": [1, 2, 3], "uv": [0, 0]}]}]),
    );
    assert_eq!(m.skeleton.bones[0].pivot, Vec3::new(-2.0, 3.0, 4.0));
    assert_eq!(m.skeleton.bones[0].rotation, Vec3::new(-10.0, -20.0, 30.0));
    let c = &m.parts[0].cubes[0];
    assert_eq!(c.origin, Vec3::new(-3.0, 0.0, 0.0));
    assert_eq!(c.size, Vec3::new(2.0, 3.0, 4.0));
    assert_eq!(c.pivot, Vec3::new(-5.0, 6.0, 7.0));
    assert_eq!(c.rotation, Vec3::new(-1.0, -2.0, 3.0));
    // Locators come out sorted by name.
    assert_eq!(m.locators[0].name, "l");
    assert_eq!(m.locators[0].offset, Vec3::new(-1.0, 2.0, 3.0));
    assert_eq!(m.locators[0].rotation, Vec3::new(-5.0, -6.0, 7.0));
    assert_eq!(m.locators[1].name, "p");
    assert_eq!(m.locators[1].rotation, Vec3::ZERO);
}

// ---------------------------------------------------------------------------------------------------------------------
// The test fox in both formats

#[test]
fn fox_bbmodel_and_geo_are_equal_at_rest() {
    let bbm = load_bbmodel("test_fox.bbmodel", FOX_BB).unwrap();
    let geom = load_geo_json("test_fox.geo.json", FOX_GEO).unwrap();

    assert_eq!(bbm.identifier, geom.identifier);
    assert_eq!(bbm.texture_size, geom.texture_size);
    assert_eq!(bbm.skeleton, geom.skeleton, "same skeleton");
    assert_eq!(bbm.skeleton.len(), 10);
    assert_eq!(bbm.element_count(), 13);
    assert_eq!(geom.element_count(), 13);

    let rest_bb = bone_matrices(&bbm.skeleton, &Pose::rest(&bbm.skeleton));
    let rest_geo = bone_matrices(&geom.skeleton, &Pose::rest(&geom.skeleton));
    assert!(rest_bb.iter().zip(&rest_geo).all(|(a, b)| a.abs_diff_eq(*b, 1e-5)), "same bone matrices at rest");
    let tail = bbm.skeleton.find("tail").unwrap().index();
    assert!(!rest_bb[tail].abs_diff_eq(Mat4::IDENTITY, 1e-3), "the tail really is rotated");

    let (ba, bb_) = (bbm.bake(), geom.bake());
    assert_eq!(ba.bones.len(), bb_.bones.len());
    assert!(ba.vertex_count() > 200);
    for (i, (x, y)) in ba.bones.iter().zip(&bb_.bones).enumerate() {
        let name = &bbm.skeleton.bones[i].name;
        assert_eq!(x.indices, y.indices, "indices of {name}");
        assert_eq!(x.vertices.len(), y.vertices.len(), "vertex count of {name}");
        for (j, (p, q)) in x.vertices.iter().zip(&y.vertices).enumerate() {
            assert_v3(p.position, q.position, &format!("{name} vertex {j} position"));
            assert_v3(p.normal, q.normal, &format!("{name} vertex {j} normal"));
            assert!(approx(p.uv[0], q.uv[0], 1e-4) && approx(p.uv[1], q.uv[1], 1e-4), "{name} vertex {j} uv");
        }
    }

    // Locators (compare sorted by name: bbmodel keeps outliner order, geo sorts).
    let key = |m: &Model| {
        let mut v: Vec<(String, usize, [f32; 3])> =
            m.locators.iter().map(|l| (l.name.clone(), l.bone.index(), l.offset.to_array())).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(key(&bbm), key(&geom));
    assert_eq!(key(&bbm).len(), 2);

    // Format-specific extras.
    assert_eq!(bbm.textures.len(), 1);
    assert_eq!((bbm.textures[0].width, bbm.textures[0].height), (8, 8));
    assert_eq!(bbm.textures[0].rgba.len(), 8 * 8 * 4);
    assert_eq!(&bbm.textures[0].rgba[..4], &[0, 0, 128, 255]);
    assert!(bbm.animations.as_ref().is_some_and(|a| a.as_array().is_some_and(|a| a.len() == 1)));
    assert!(geom.visible_bounds.is_some_and(|b| b.width == 4.0 && b.height == 3.0 && b.offset == Vec3::Y));
    assert!(bbm.warnings.is_empty(), "{:?}", bbm.warnings);
    assert!(geom.warnings.is_empty(), "{:?}", geom.warnings);
}

#[test]
fn fox_poses_agree_too() {
    let bbm = load_bbmodel("test_fox.bbmodel", FOX_BB).unwrap();
    let geom = load_geo_json("test_fox.geo.json", FOX_GEO).unwrap();
    let mut pose = Pose::rest(&bbm.skeleton);
    pose.bones[1].rotation = glam::Quat::from_rotation_y(0.4);
    pose.bones[4].translation = Vec3::new(0.0, 1.0, 0.5);
    let a = bone_matrices(&bbm.skeleton, &pose);
    let b = bone_matrices(&geom.skeleton, &pose);
    assert!(a.iter().zip(&b).all(|(x, y)| x.abs_diff_eq(*y, 1e-5)));
}

#[test]
fn old_1_8_layout_loads_and_orders_parents_first() {
    let m = load_geo_json("cube_old.geo.json", OLD_GEO).unwrap();
    assert_eq!(m.identifier, "geometry.old_box");
    assert_eq!(m.texture_size, [32, 32]);
    // "top" is listed before its parent "root" in the file.
    let names: Vec<&str> = m.skeleton.bones.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, vec!["root", "top"]);
    assert_eq!(m.skeleton.bones[1].parent, Some(BoneIndex(0)));
    // bind_pose_rotation is read as the rest rotation: (0,0,90) -> (-0,-0,90).
    assert_eq!(m.skeleton.bones[1].rotation, Vec3::new(0.0, 0.0, 90.0));
    assert_eq!(m.locators.len(), 1);
    assert_eq!(m.locators[0].bone, BoneIndex(1));
    assert_eq!(m.locators[0].offset, Vec3::new(-1.0, 9.0, 0.0));
    // root cube: mirror and inflate 0.5; 8x8x8 at x -4..4 inflated.
    let c = &m.parts[0].cubes[0];
    assert!(c.mirror && c.inflate == 0.5);
    let baked = m.bake();
    let (lo, hi) = baked.bones[0].vertices.iter().fold((Vec3::splat(1e9), Vec3::splat(-1e9)), |(l, h), v| {
        (l.min(Vec3::from(v.position)), h.max(Vec3::from(v.position)))
    });
    assert_v3(lo.to_array(), [-4.5, -0.5, -4.5], "old cube lo");
    assert_v3(hi.to_array(), [4.5, 8.5, 4.5], "old cube hi");
    assert_eq!(m.visible_bounds.unwrap().height, 2.0);
}

#[test]
fn identifier_selection_and_multiple_geometries() {
    let text = bytes(&json!({
        "format_version": "1.12.0",
        "minecraft:geometry": [
            {"description": {"identifier": "geometry.a", "texture_width": 16, "texture_height": 16},
             "bones": [{"name": "one"}]},
            {"description": {"identifier": "geometry.b", "texture_width": 32, "texture_height": 32},
             "bones": [{"name": "two"}]}
        ]
    }));
    assert_eq!(load_geo_json("x.geo.json", &text).unwrap().skeleton.bones[0].name, "one");
    let b = load_geo_json_with("x.geo.json", &text, Some("geometry.b")).unwrap();
    assert_eq!((b.skeleton.bones[0].name.as_str(), b.texture_size), ("two", [32, 32]));
    let e = load_geo_json_with("x.geo.json", &text, Some("geometry.zzz")).unwrap_err();
    assert!(e.to_string().contains("no geometry with identifier"), "{e}");
}

// ---------------------------------------------------------------------------------------------------------------------
// bbmodel specifics

#[test]
fn bbmodel_top_level_elements_go_to_a_synthetic_root_and_unsupported_things_warn() {
    let m = load_bbmodel(
        "t.bbmodel",
        &bb(
            16,
            json!(["c1", {"name": "g", "origin": [0, 0, 0], "children": ["n1"]},
                   {"name": "hidden", "origin": [0, 0, 0], "export": false, "children": ["c2"]}, "s1"]),
            json!([
                {"type": "cube", "uuid": "c1", "from": [0, 0, 0], "to": [1, 1, 1], "box_uv": true},
                {"type": "cube", "uuid": "c2", "from": [0, 0, 0], "to": [1, 1, 1], "box_uv": true},
                {"type": "null_object", "uuid": "n1", "name": "hand", "from": [1, 2, 3]},
                {"type": "special", "uuid": "s1"},
                {"type": "cube", "uuid": "lost", "from": [0, 0, 0], "to": [1, 1, 1]}
            ]),
        ),
    )
    .unwrap();
    let names: Vec<&str> = m.skeleton.bones.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, vec!["g", "root"]);
    assert_eq!(m.parts[1].cubes.len(), 1);
    assert_eq!(m.parts[0].cubes.len(), 0);
    assert!(m.locators[0].null_object && m.locators[0].name == "hand" && m.locators[0].bone == BoneIndex(0));
    assert!(m.warnings.iter().any(|w| w.contains("not supported")), "{:?}", m.warnings);
    assert!(m.warnings.iter().any(|w| w.contains("elements[4]") && w.contains("not in the outliner")), "{:?}", m.warnings);
}

#[test]
fn bbmodel_box_uv_matches_geo_box_uv_and_texture_null_hides_a_face() {
    let elements = json!([{"type": "cube", "uuid": "c1", "from": [-3, 0, 0], "to": [-1, 3, 4],
        "box_uv": true, "uv_offset": [0, 0], "mirror_uv": true,
        "faces": {"north": {"uv": [0, 0, 0, 0], "texture": 0}, "south": {"uv": [0, 0, 0, 0], "texture": null}}}]);
    let m = load_bbmodel("t.bbmodel", &bb(16, json!([{"name": "b", "children": ["c1"]}]), elements)).unwrap();
    // A faces object is present: only north has a texture, every other face is hidden.
    assert_eq!(m.parts[0].cubes[0].faces_enabled, [true, false, false, false, false, false]);
    let full = load_bbmodel(
        "t.bbmodel",
        &bb(
            16,
            json!([{"name": "b", "children": ["c1"]}]),
            json!([{"type": "cube", "uuid": "c1", "from": [-3, 0, 0], "to": [-1, 3, 4], "box_uv": true, "mirror_uv": true}]),
        ),
    )
    .unwrap();
    assert_eq!(full.bake(), golden_cube_geo(json!({"mirror": true})).bake());
}

#[test]
fn bbmodel_texture_by_path_has_no_pixels() {
    let text = bytes(&json!({
        "resolution": {"width": 16, "height": 16}, "elements": [], "outliner": [],
        "textures": [{"name": "a.png", "path": "C:/x/a.png"}]
    }));
    let m = load_bbmodel("t.bbmodel", &text).unwrap();
    assert_eq!(m.textures[0].path.as_deref(), Some("C:/x/a.png"));
    assert!(m.textures[0].rgba.is_empty());
}

// ---------------------------------------------------------------------------------------------------------------------
// Errors and limits

fn geo_err(bones: Value) -> String {
    load_geo_json("fox.geo.json", &geo(16, bones)).unwrap_err().to_string()
}

#[test]
fn errors_carry_file_and_json_path() {
    assert_eq!(
        geo_err(json!([{"name": "a"}, {"name": "b", "cubes": [
            {"origin": [0, 0, 0], "size": [1, 1, 1], "uv": [0, 0]},
            {"origin": [0, 0, 0], "size": [1, 1, 1]}]}])),
        "fox.geo.json: minecraft:geometry[0].bones[1].cubes[1].uv: missing required field"
    );
    assert_eq!(
        geo_err(json!([{"name": "b", "cubes": [{"origin": [0, 0, 0], "size": [1, 1], "uv": [0, 0]}]}])),
        "fox.geo.json: minecraft:geometry[0].bones[0].cubes[0].size: expected an array of 3 numbers, found an array"
    );
    assert_eq!(
        geo_err(json!([{"name": "b", "cubes": [{"origin": [0, "x", 0], "size": [1, 1, 1], "uv": [0, 0]}]}])),
        "fox.geo.json: minecraft:geometry[0].bones[0].cubes[0].origin[1]: expected a number, found a string"
    );
    assert_eq!(
        geo_err(json!([{"name": "b", "cubes": [{"origin": [0, 0, 0], "size": [1, 1, 1], "uv": 5}]}])),
        "fox.geo.json: minecraft:geometry[0].bones[0].cubes[0].uv: expected [u, v] or an object of faces, found a number"
    );
    assert!(geo_err(json!([{"name": "b", "cubes": [{"origin": [0, 0, 0], "size": [1, 1, 1],
        "uv": {"north": {"uv": [0, 0], "uv_rotation": 45}}}]}]))
        .contains("cubes[0].uv.north.uv_rotation: expected a multiple of 90"));
    assert!(geo_err(json!([{"name": "b", "pivot": [1e30, 0, 0]}])).contains("pivot[0]: number out of range"));
    assert!(geo_err(json!([{"pivot": [0, 0, 0]}])).ends_with("bones[0].name: missing required field"));
    assert!(geo_err(json!([{"name": "a", "parent": "zz"}])).ends_with("bones[0].parent: unknown parent bone \"zz\""));
    assert!(geo_err(json!([{"name": "a"}, {"name": "a"}])).contains("bones[1].name: duplicate bone name"));
    assert!(geo_err(json!([{"name": "a", "parent": "b"}, {"name": "b", "parent": "a"}])).contains("parent cycle"));
    assert!(geo_err(json!([{"name": "a", "parent": "a"}])).contains("parent cycle"));
    let e = load_geo_json("broken.geo.json", b"{ nope").unwrap_err();
    assert!(e.to_string().starts_with("broken.geo.json: invalid JSON:"), "{e}");
    let e = load_geo_json("empty.geo.json", b"{}").unwrap_err();
    assert!(e.to_string().starts_with("empty.geo.json: no geometry found"), "{e}");
    let e = load_geo_json("arr.geo.json", b"[]").unwrap_err();
    assert!(e.to_string().contains("expected an object"), "{e}");
}

#[test]
fn bbmodel_errors_carry_file_and_json_path() {
    let e = load_bbmodel("fox.bbmodel", &bb(16, json!([{"name": "g", "children": ["nope"]}]), json!([]))).unwrap_err();
    assert_eq!(e.to_string(), "fox.bbmodel: outliner[0].children[0]: no element with uuid \"nope\"");
    let e = load_bbmodel(
        "fox.bbmodel",
        &bb(16, json!([{"name": "g", "children": ["c"]}]), json!([{"type": "cube", "uuid": "c", "from": [0, 0], "to": [1, 1, 1]}])),
    )
    .unwrap_err();
    assert_eq!(e.to_string(), "fox.bbmodel: elements[0].from: expected an array of 3 numbers, found an array");
    let e = load_bbmodel("fox.bbmodel", &bb(16, json!([{"children": []}]), json!([]))).unwrap_err();
    assert_eq!(e.to_string(), "fox.bbmodel: outliner[0].name: missing required field");
    let e = load_bbmodel("fox.bbmodel", &bb(16, json!([{"name": "a"}, {"name": "a"}]), json!([]))).unwrap_err();
    assert!(e.to_string().contains("outliner[1].name: duplicate bone name"), "{e}");
    let e = load_bbmodel(
        "fox.bbmodel",
        &bb(16, json!([]), json!([{"type": "cube", "uuid": "c"}, {"type": "cube", "uuid": "c"}])),
    )
    .unwrap_err();
    assert!(e.to_string().contains("elements[1].uuid: duplicate uuid"), "{e}");
    let mesh_elements = json!([{"type": "mesh", "uuid": "m", "vertices": {"a": [0, 0, 0]},
        "faces": {"f": {"vertices": ["a", "b", "c"]}}}]);
    let e = load_bbmodel("fox.bbmodel", &bb(16, json!([{"name": "g", "children": ["m"]}]), mesh_elements)).unwrap_err();
    assert_eq!(e.to_string(), "fox.bbmodel: elements[0].faces.f.vertices[1]: unknown vertex id \"b\"");
    let e = load_bbmodel("fox.bbmodel", br#"{"resolution": {"width": 0, "height": 16}}"#).unwrap_err();
    assert!(e.to_string().starts_with("fox.bbmodel: resolution.width: expected a whole number from 1 to"), "{e}");
}

#[test]
fn bbmodel_texture_errors() {
    let texture = |source: &str| {
        bytes(&json!({"resolution": {"width": 16, "height": 16}, "elements": [], "outliner": [],
                      "textures": [{"name": "t", "source": source}]}))
    };
    let e = load_bbmodel("t.bbmodel", &texture("data:image/png;base64,@@@@")).unwrap_err();
    assert!(e.to_string().starts_with("t.bbmodel: textures[0].source: invalid base64"), "{e}");
    let e = load_bbmodel("t.bbmodel", &texture("data:image/png;base64,AAAA")).unwrap_err();
    assert!(e.to_string().starts_with("t.bbmodel: textures[0].source: invalid PNG"), "{e}");
}

#[test]
fn limits_bones_and_cubes() {
    let bones = |n: usize| -> Value { (0..n).map(|i| json!({"name": format!("b{i}")})).collect() };
    assert!(load_geo_json("m.geo.json", &geo(16, bones(256))).is_ok());
    let e = load_geo_json("m.geo.json", &geo(16, bones(257))).unwrap_err();
    assert_eq!(e.to_string(), "m.geo.json: minecraft:geometry[0].bones: too many bones (257, limit 256)");

    let cube = json!({"origin": [0, 0, 0], "size": [1, 1, 1], "uv": [0, 0]});
    let cubes = |n: usize| -> Value { json!([{"name": "b", "cubes": vec![cube.clone(); n]}]) };
    assert!(load_geo_json("m.geo.json", &geo(16, cubes(4096))).is_ok());
    let e = load_geo_json("m.geo.json", &geo(16, cubes(4097))).unwrap_err();
    assert!(e.to_string().contains("too many cubes and meshes (4097, limit 4096)"), "{e}");

    // The same in a bbmodel.
    let elements: Value = (0..4097).map(|i| json!({"type": "cube", "uuid": format!("c{i}"), "from": [0, 0, 0], "to": [1, 1, 1]})).collect();
    let children: Vec<Value> = (0..4097).map(|i| json!(format!("c{i}"))).collect();
    let e = load_bbmodel("m.bbmodel", &bb(16, json!([{"name": "g", "children": children}]), elements)).unwrap_err();
    assert!(e.to_string().contains("too many cubes and meshes (4097, limit 4096)"), "{e}");
    let groups: Vec<Value> = (0..257).map(|i| json!({"name": format!("g{i}")})).collect();
    let e = load_bbmodel("m.bbmodel", &bb(16, json!(groups), json!([]))).unwrap_err();
    assert!(e.to_string().contains("too many bones (257, limit 256)"), "{e}");
}

#[test]
fn deeply_nested_outliner_is_an_error_not_a_stack_overflow() {
    let mut node = json!({"name": "leaf"});
    for i in 0..100 {
        node = json!({"name": format!("g{i}"), "children": [node]});
    }
    // 100 groups exceed the JSON nesting pre-scan (120) before serde_json's recursion limit (128).
    let e = load_bbmodel("deep.bbmodel", &bb(16, json!([node]), json!([]))).unwrap_err();
    assert!(e.to_string().contains("nested deeper than"), "{e}");
    // 49 groups stay under the JSON pre-scan and hit bbmodel's 48-group limit with an element path.
    let mut node49 = json!({"name": "leaf"});
    for i in 0..49 {
        node49 = json!({"name": format!("g{i}"), "children": [node49]});
    }
    let e49 = load_bbmodel("deep.bbmodel", &bb(16, json!([node49]), json!([]))).unwrap_err();
    assert!(e49.path.starts_with("outliner[0].children[0]") && e49.message.contains("nested deeper than 48"), "{e49}");
    let deep = format!("{}{}", "[".repeat(5000), "]".repeat(5000));
    assert!(load_bbmodel("deep.bbmodel", deep.as_bytes()).is_err());
    assert!(load_geo_json("deep.geo.json", deep.as_bytes()).is_err());
}

fn png_chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in tag.iter().chain(data) {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    out.extend_from_slice(&(!crc).to_be_bytes());
    out
}

fn stored_zlib(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let mut chunks = data.chunks(65535).peekable();
    if chunks.peek().is_none() {
        out.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(c) = chunks.next() {
        let last = u8::from(chunks.peek().is_none());
        let len = c.len() as u16;
        out.push(last);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(c);
    }
    let (mut s1, mut s2) = (1u32, 0u32);
    for &b in data {
        s1 = (s1 + b as u32) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    out.extend_from_slice(&((s2 << 16) | s1).to_be_bytes());
    out
}

fn encode_uncompressed_png(w: u32, h: u32, color_type: u8, bpp: usize, pixels: &[u8]) -> String {
    use base64::Engine;
    let row_bytes = w as usize * bpp;
    let mut raw = Vec::with_capacity(h as usize * (1 + row_bytes));
    for y in 0..h as usize {
        raw.push(0); // filter None
        raw.extend_from_slice(&pixels[y * row_bytes..(y + 1) * row_bytes]);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, color_type, 0, 0, 0]);
    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    png.extend_from_slice(&png_chunk(b"IDAT", &stored_zlib(&raw)));
    png.extend_from_slice(&png_chunk(b"IEND", &[]));
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(&png))
}

#[test]
fn bbmodel_png_color_types_expand_in_place_and_model_budget_caps_at_64_mib() {
    // 2x1 Grayscale, GrayscaleAlpha, Rgb, Rgba all decode to exact RGBA8.
    let gs = encode_uncompressed_png(2, 1, 0, 1, &[10, 200]);
    let ga = encode_uncompressed_png(2, 1, 4, 2, &[10, 128, 200, 64]);
    let rgb = encode_uncompressed_png(2, 1, 2, 3, &[1, 2, 3, 4, 5, 6]);
    let rgba = encode_uncompressed_png(2, 1, 6, 4, &[1, 2, 3, 4, 5, 6, 7, 8]);
    let doc = bytes(&json!({
        "resolution": {"width": 16, "height": 16}, "elements": [], "outliner": [],
        "textures": [
            {"name": "gs", "source": gs},
            {"name": "ga", "source": ga},
            {"name": "rgb", "source": rgb},
            {"name": "rgba", "source": rgba},
        ]
    }));
    let m = load_bbmodel("t.bbmodel", &doc).unwrap();
    assert_eq!(m.textures[0].rgba, vec![10, 10, 10, 255, 200, 200, 200, 255]);
    assert_eq!(m.textures[1].rgba, vec![10, 10, 10, 128, 200, 200, 200, 64]);
    assert_eq!(m.textures[2].rgba, vec![1, 2, 3, 255, 4, 5, 6, 255]);
    assert_eq!(m.textures[3].rgba, vec![1, 2, 3, 4, 5, 6, 7, 8]);

    // Four 2048x2048 textures = 64 MiB decoded (accepted); a fifth pushes total to 80 MiB (rejected at textures[4].source).
    let tile_src = encode_uncompressed_png(2048, 2048, 0, 1, &vec![7u8; 2048 * 2048]);
    let tex4: Vec<Value> = (0..4).map(|i| json!({"name": format!("t{i}"), "source": tile_src})).collect();
    let ok_doc = bytes(&json!({"resolution": {"width": 16, "height": 16}, "elements": [], "outliner": [], "textures": tex4}));
    assert_eq!(load_bbmodel("ok.bbmodel", &ok_doc).unwrap().textures.len(), 4);

    let mut tex5 = tex4;
    tex5.push(json!({"name": "t4", "source": tile_src}));
    let bad_doc = bytes(&json!({"resolution": {"width": 16, "height": 16}, "elements": [], "outliner": [], "textures": tex5}));
    let e = load_bbmodel("bad.bbmodel", &bad_doc).unwrap_err();
    assert_eq!((e.file.as_str(), e.path.as_str()), ("bad.bbmodel", "textures[4].source"), "{e}");
}

#[test]
fn utf8_bom_is_accepted() {
    let mut text = vec![0xEF, 0xBB, 0xBF];
    text.extend_from_slice(FOX_GEO);
    assert!(load_geo_json("bom.geo.json", &text).is_ok());
}

/// Data from an independently authored Blockbench pair: geo up/down are the bbmodel rectangles flipped on both axes.
#[test]
fn geo_up_down_flip_matches_bbmodel_and_side_faces_do_not_flip() {
    let cube = json!({"type": "cube", "uuid": "c", "from": [-19, 0, 0], "to": [0, 5, 13], "box_uv": false, "faces": {
        "north": {"uv": [57, 0, 38, 10], "texture": 0},
        "up": {"uv": [0, 13, 19, 0], "texture": 0},
        "down": {"uv": [19, 13, 38, 0], "texture": 0}}});
    let b = load_bbmodel("t.bbmodel", &bb(64, json!([{"name": "b", "children": ["c"]}]), json!([cube]))).unwrap();
    let g = load_geo(64, json!([{"name": "b", "cubes": [{"origin": [0, 0, 0], "size": [19, 5, 13], "uv": {
        "north": {"uv": [57, 0], "uv_size": [-19, 10]},
        "up": {"uv": [19, 0], "uv_size": [-19, 13]},
        "down": {"uv": [38, 0], "uv_size": [-19, 13]}}}]}]));
    assert_eq!(b.parts[0].cubes[0].uv, g.parts[0].cubes[0].uv);
    assert_eq!(b.bake(), g.bake());
    let uvs = texel_verts(&g.bake(), 0, 64.0);
    // north: top-left of the rectangle is (57,0); up: V reversed, TL is (0,13); down: TL is (19,13).
    assert_eq!(uvs[0].1, [57.0, 0.0]);
    assert_eq!(uvs[4].1, [0.0, 13.0]);
    assert_eq!(uvs[8].1, [19.0, 13.0]);
}
