# FR-F4M-1: model loaders and bake (`horizon-mobmodel`)

Branch `yoake-free/f4m-1`. Files: `crates/horizon-mobmodel/src/{model,geo,bbmodel,bake}.rs`, `lib.rs` (module lines and
re-exports only), `tests/{model_tests,fuzz}.rs`, `tests/fixtures/*`. `skeleton.rs` is unchanged.

## API

```rust
load_geo_json(file, bytes) -> Result<Model, ModelError>
load_geo_json_with(file, bytes, Some("geometry.x"))
load_bbmodel(file, bytes) -> Result<Model, ModelError>
model.bake() -> BakedModel            // bones[i] = { vertices: Vec<BakedVertex{position, normal, uv}>, indices: Vec<u32> }
```

`Model` = identifier, `texture_size`, `Skeleton`, `parts[bone]` (cubes, meshes, `never_render`), `locators`, `textures`
(decoded RGBA8 when embedded), `visible_bounds`, raw bbmodel `animations` (`Option<serde_json::Value>`), `warnings`.
Baked vertices are in model rest space per bone (cube rotation and inflate applied); the renderer multiplies them by
`bone_matrices()[bone]`. Triangle lists, CCW from outside, UVs normalised by the texture size.

## Supported

* geo.json 1.12.0+ (`minecraft:geometry`) and 1.8 (`geometry.<name>`, `texturewidth/textureheight`): description
  (identifier, texture size, `visible_bounds_*`), bones (`name`, `parent`, `pivot`, `rotation`, `bind_pose_rotation`,
  `mirror`, `inflate`, `neverRender`, `locators` as vector or `{offset, rotation}`, `cubes`, `poly_mesh`), cubes
  (`origin`, `size`, `pivot`, `rotation`, `inflate`, `mirror`, `uv` as box offset or per-face `{uv, uv_size, uv_rotation}`).
* bbmodel: `meta`, `resolution`, `elements` (cube with `from/to/origin/rotation/inflate/box_uv/uv_offset/mirror_uv/faces`,
  mesh, locator, null_object), `outliner` (nested groups with origin, rotation, child order), `textures` (embedded
  base64 PNG decoded with `png`, or path only), `animations` kept raw.
* Mesh triangulation: fan `(0, i, i+1)` for quads and n-gons; flat normals by Newell's method (or the file's normals).

## Conventions chosen (all pinned by tests)

1. **Internal space is Blockbench space** (pixels, +X east, +Y up, -Z north). A bbmodel loads unchanged. A Bedrock geo.json
   is converted the way Blockbench imports it: positions `(x,y,z) -> (-x,y,z)` (a cube `[ox, ox+sx]` becomes
   `[-(ox+sx), -ox]`), rotations `(rx,ry,rz) -> (-rx,-ry,rz)`, face names kept. Locator offsets/rotations likewise.
   **F4M-2 must apply the same conversion to Bedrock `animation.json` (position `(-x,y,z)`, rotation `(-x,-y,z)`)**; bbmodel
   animations need none.
2. Rotation order everywhere: degrees, matrix `Rz * Ry * Rx` (X applied first), bones, cubes (about the cube pivot) and
   meshes (about the mesh origin). This is what `skeleton::bone_matrices` already does.
3. Bone pivots, cube origins, locators are absolute model-space positions (not parent relative).
4. Bone order: depth first from the roots, siblings in file order (geo) or outliner order (bbmodel); parents always first.
   Top-level bbmodel elements go to a synthetic bone `root` added last (name made unique if taken).
5. Box UV (offset u,v; size w,h,d for X,Y,Z): east `(u,v+d)-(u+d,v+d+h)`, north `(u+d,v+d)-(u+d+w,..)`, west
   `(u+d+w,v+d)-(u+2d+w,..)`, south `(u+2d+w,v+d)-(u+2d+2w,..)`, up `(u+d,v)-(u+d+w,v+d)`, down `(u+d+w,v+d)-(u+d+2w,v)`
   (V reversed). `mirror` swaps east/west and flips each rectangle left-right; geometry unchanged; ignored for per-face UV.
6. Face orientation seen from outside: north has +X on the left, south +X right, east +Z left, west +Z right, up has north
   at the top, down has south at the top. Vertex order per face: TL, TR, BR, BL; indices `[0,3,2, 0,2,1]`.
7. UV rotation 90 = image turned clockwise on the face (corner `i` takes texture corner `i - rot/90`). Negative `uv_size`
   flips. Per-face `uv_size` defaults to the face's natural size (geo). bbmodel face `uv` is `[x1,y1,x2,y2]`.
8. Inflate grows all faces outwards and does not change the box UV layout. Cube `mirror`/`inflate` default to the bone's (geo).
9. Cube without `pivot` rotates about `[0,0,0]` (geo). `bind_pose_rotation` is used when `rotation` is absent.
10. bbmodel mesh vertices are relative to the mesh `origin`; faces and vertices are processed sorted by id string, so output
    is independent of JSON key order. geo `poly_mesh` corner order is reversed on load (the X mirror flips winding);
    `normalized_uvs` is multiplied by the texture size. Only explicit polygon arrays are supported (`"tri_list"` /
    `"quad_list"` strings are an error).
11. bbmodel: faces with `"texture": null` are not drawn (per-face cubes: also absent faces; box-UV cubes: only when a `faces`
    object exists). Elements/groups with `"export": false` are skipped (with their subtrees). geo locators are sorted by name.
12. Only one geometry per geo file is loaded (first, or by identifier). Inheritance (`geometry.a:geometry.b`) is ignored
    with a warning. Unknown fields are ignored; unsupported bbmodel element types give a warning.

## Errors and limits

`ModelError { file, path, message }`, `Display` = `fox.geo.json: minecraft:geometry[0].bones[3].cubes[1].uv: missing required
field`. All parsing goes through typed readers; no `unwrap` on input data; serde_json's recursion limit (128) bounds nesting.
Limits: bones 256 and cubes+meshes 4096 (`limits`), locators 1024, textures 64, texture edge 8192, mesh vertices/faces 65536,
outliner depth 64, every number `|x| <= 1e6`, embedded texture source 48 MiB. Duplicate bone names, unknown parents, parent
cycles, unknown uuids/vertex ids, bad base64/PNG are errors with paths.

## Tests (`cargo test -p horizon-mobmodel`: 31 pass, 0 fail)

* 1 unit test (skeleton, from the skeleton commit).
* `tests/model_tests.rs`: 24 tests: golden vertex sets (cube, mirrored cube, per-face UV with size/rotation/flip, bbmodel vs
  geo per-face, mesh quad/tri/pentagon with origin+rotation, geo poly_mesh winding), winding/normal check on all cube faces,
  nested pivots (hand-computed point), rotation order, Bedrock-to-internal mapping, inheritance of bone mirror/inflate,
  1.8 layout (child before parent, `bind_pose_rotation`), identifier selection, bbmodel root/null_object/export/warnings,
  errors with exact messages (geo and bbmodel), texture errors, limits (256/257 bones, 4096/4097 cubes), deep nesting, BOM,
  and **the test fox as bbmodel and geo.json equal at rest** (same skeleton, same `bone_matrices`, same baked vertices,
  normals, UVs, indices within 1e-4, same locators) plus a posed comparison.
* `tests/fuzz.rs`: 6 tests, seed-fixed: 3000 / 3000 / 2000 structural mutations of the fox bbmodel, fox geo and 1.8 geo
  (each fed to both loaders), 1800 byte-level corruptions, 500 random byte documents plus odd documents, hostile sizes. Anything
  that loads is baked and checked finite, in range, parents first.
* Clippy (`--all-targets`): clean for the crate. Cargo reports `serde` as an unused dependency (from the skeleton commit;
  F4M-2 probably uses it, I did not touch `Cargo.toml`).

Fixtures are original: `tests/fixtures/test_fox.bbmodel` and `test_fox.geo.json` are generated from one description by
`gen_fox.py` (10 bones, 12 cubes, 1 mesh, 2 locators, 8x8 embedded PNG; covers nested pivots, rest rotations on all axes,
cube rotation, inflate, bone- and cube-level mirror, per-face UV with rotation/negative size/hidden face). `cube_old.geo.json`
is a 1.8 sample.

## Open questions

1. The Blockbench Bedrock mapping (`x -> -x`, rotation `(-x,-y,z)`) is from memory; check against a real Blockbench export
   (a model with a rotated bone) before F4M-6 ships. The loaders keep it in two functions (`bedrock_position/rotation`).
2. Default cube `pivot` when absent, `down` face UV orientation, and the sign of UV rotation follow my reading of the formats;
   untested against Blockbench renders.
3. Blockbench mesh vertices are assumed relative to the mesh origin (as in Blockbench's own mesh data).
4. Textures referenced by `texture` index on faces are not tracked (one texture per model assumed by the renderer).
5. `Cargo.toml` still lists `serde` unused.
