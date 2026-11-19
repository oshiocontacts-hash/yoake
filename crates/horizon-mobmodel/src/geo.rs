//! Bedrock `.geo.json` loader: `format_version` 1.12.0 and newer (`minecraft:geometry`) and the older 1.8 layout
//! (`geometry.<name>` with `texturewidth` / `textureheight`).
//!
//! Choices where the format is loose (all written down in the F4M-1 report):
//! * Only one geometry per file is loaded: by identifier if one is given, else the first (1.12) or the
//!   alphabetically first `geometry.*` key (1.8). Geometry inheritance (`geometry.a:geometry.b`) is ignored.
//! * Bones are re-ordered depth first from the roots (file order among siblings) so parents come first.
//! * A cube without `pivot` rotates about `[0, 0, 0]`. A cube `mirror` / `inflate` defaults to the bone's.
//! * Per-face `up` / `down` UV in geo is the bbmodel rectangle flipped on both axes (as Blockbench exports it):
//!   `uv = [x2, y2]`, `uv_size = [x1 - x2, y1 - y2]`; the loader flips it back.
//! * Per-face `uv_size` defaults to the face's natural size. `bind_pose_rotation` is read when `rotation` is absent.
//! * `mirror` flips the box UV layout only (geometry is unchanged); it is ignored for per-face UV.
//! * Locators come out sorted by name within each bone.

use crate::model::*;
use crate::skeleton::{Bone, BoneIndex, Skeleton};
use glam::Vec3;
use serde_json::{Map, Value};
use std::collections::HashMap;

/// Loads the first geometry of a `.geo.json`. `file` is only used in error messages.
pub fn load_geo_json(file: &str, bytes: &[u8]) -> Result<Model, ModelError> {
    load_geo_json_with(file, bytes, None)
}

/// Loads the geometry whose identifier is `identifier` (or the first one when `None`).
pub fn load_geo_json_with(file: &str, bytes: &[u8], identifier: Option<&str>) -> Result<Model, ModelError> {
    let rd = Rd { file };
    let root = rd.parse(bytes)?;
    let root_obj = rd.object("", &root)?;
    let mut warnings = Vec::new();

    let geo_path: String;
    let geo: &Map<String, Value>;
    let desc: &Map<String, Value>;
    let ident: String;
    if let Some(list) = root_obj.get("minecraft:geometry") {
        let arr = rd.array("minecraft:geometry", list)?;
        if arr.is_empty() {
            return Err(rd.err("minecraft:geometry", "no geometry in the file"));
        }
        let mut idx = 0;
        if let Some(want) = identifier {
            idx = arr
                .iter()
                .position(|g| {
                    g.get("description").and_then(|d| d.get("identifier")).and_then(Value::as_str) == Some(want)
                })
                .ok_or_else(|| rd.err("minecraft:geometry", format!("no geometry with identifier {want:?}")))?;
        }
        geo_path = at("minecraft:geometry", idx);
        geo = rd.object(&geo_path, &arr[idx])?;
        let desc_path = key(&geo_path, "description");
        desc = rd.object(&desc_path, rd.req(geo, &geo_path, "description")?)?;
        ident = rd.string(&key(&desc_path, "identifier"), rd.req(desc, &desc_path, "identifier")?)?.to_string();
    } else {
        let mut keys: Vec<&String> = root_obj.keys().filter(|k| k.starts_with("geometry.")).collect();
        keys.sort();
        let k = keys.first().copied().ok_or_else(|| {
            rd.err("", "no geometry found (expected \"minecraft:geometry\" or a \"geometry.<name>\" key)")
        })?;
        let name = match identifier {
            Some(want) => root_obj
                .keys()
                .find(|k| k.split(':').next() == Some(want))
                .ok_or_else(|| rd.err("", format!("no geometry with identifier {want:?}")))?,
            None => k,
        };
        geo_path = name.clone();
        geo = rd.object(&geo_path, &root_obj[name])?;
        desc = geo;
        if name.contains(':') {
            warnings.push(format!("{file}: {geo_path}: geometry inheritance is not supported, parent ignored"));
        }
        ident = name.split(':').next().unwrap_or(name).to_string();
    }

    // Texture size.
    let mut texture_size = [64u32, 64u32];
    let mut have_size = false;
    for (i, names) in [["texture_width", "texturewidth"], ["texture_height", "textureheight"]].iter().enumerate() {
        for n in names {
            if let Some(v) = Rd::opt(desc, n) {
                let p = if std::ptr::eq(desc, geo) { key(&geo_path, n) } else { key(&key(&geo_path, "description"), n) };
                texture_size[i] = rd.dimension(&p, v)?;
                have_size = true;
            }
        }
    }
    if !have_size {
        warnings.push(format!("{file}: {geo_path}: no texture size, assuming 64x64"));
    }

    // Visible bounds.
    let desc_path = if std::ptr::eq(desc, geo) { geo_path.clone() } else { key(&geo_path, "description") };
    let bw = rd.opt_f32(desc, &desc_path, "visible_bounds_width")?;
    let bh = rd.opt_f32(desc, &desc_path, "visible_bounds_height")?;
    let visible_bounds = if bw.is_some() || bh.is_some() {
        Some(VisibleBounds {
            width: bw.unwrap_or(0.0),
            height: bh.unwrap_or(0.0),
            offset: rd.opt_vec3(desc, &desc_path, "visible_bounds_offset", Vec3::ZERO)?,
        })
    } else {
        None
    };

    // Bones.
    let bones_path = key(&geo_path, "bones");
    let empty = Vec::new();
    let raw_bones: &Vec<Value> = match Rd::opt(geo, "bones") {
        Some(v) => rd.array(&bones_path, v)?,
        None => {
            warnings.push(format!("{file}: {geo_path}: geometry has no bones"));
            &empty
        }
    };
    check_counts(&rd, &bones_path, raw_bones.len(), 0)?;

    let mut raws: Vec<RawBone> = Vec::with_capacity(raw_bones.len());
    let mut names: HashMap<String, usize> = HashMap::new();
    let mut elements = 0usize;
    let mut locator_count = 0usize;
    for (i, b) in raw_bones.iter().enumerate() {
        let p = at(&bones_path, i);
        let raw = parse_bone(&rd, &p, b, texture_size, &mut elements, &mut locator_count, &mut warnings)?;
        if names.insert(raw.name.clone(), i).is_some() {
            return Err(rd.err(&key(&p, "name"), format!("duplicate bone name {:?}", raw.name)));
        }
        raws.push(raw);
    }

    // Parents and order.
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); raws.len()];
    let mut roots = Vec::new();
    for (i, raw) in raws.iter().enumerate() {
        match &raw.parent {
            None => roots.push(i),
            Some(parent) => match names.get(parent) {
                Some(&pi) => children[pi].push(i),
                None => {
                    return Err(rd.err(
                        &key(&at(&bones_path, i), "parent"),
                        format!("unknown parent bone {parent:?}"),
                    ))
                }
            },
        }
    }
    let mut order: Vec<usize> = Vec::with_capacity(raws.len());
    let mut stack: Vec<usize> = roots.iter().rev().copied().collect();
    while let Some(i) = stack.pop() {
        order.push(i);
        stack.extend(children[i].iter().rev());
    }
    if order.len() != raws.len() {
        let seen: std::collections::HashSet<usize> = order.iter().copied().collect();
        let bad = (0..raws.len()).find(|i| !seen.contains(i)).unwrap_or(0);
        return Err(rd.err(
            &key(&at(&bones_path, bad), "parent"),
            format!("bone {:?} is part of a parent cycle", raws[bad].name),
        ));
    }
    let mut new_index = vec![0usize; raws.len()];
    for (n, &old) in order.iter().enumerate() {
        new_index[old] = n;
    }

    let mut skeleton = Skeleton::default();
    let mut parts = Vec::with_capacity(raws.len());
    let mut locators = Vec::new();
    let mut slots: Vec<Option<RawBone>> = raws.into_iter().map(Some).collect();
    for (n, &old) in order.iter().enumerate() {
        let raw = slots[old].take().expect("each bone is visited once");
        let parent = raw.parent.as_ref().and_then(|p| names.get(p)).map(|&pi| BoneIndex(new_index[pi] as u16));
        skeleton.bones.push(Bone { name: raw.name, parent, pivot: raw.pivot, rotation: raw.rotation });
        parts.push(BonePart { never_render: raw.never_render, cubes: raw.cubes, meshes: raw.meshes });
        let mut locs = raw.locators;
        locs.sort_by(|a, b| a.0.cmp(&b.0));
        for (name, offset, rotation) in locs {
            locators.push(Locator { name, bone: BoneIndex(n as u16), offset, rotation, null_object: false });
        }
    }

    Ok(Model {
        identifier: ident,
        texture_size,
        skeleton,
        parts,
        locators,
        textures: Vec::new(),
        visible_bounds,
        animations: None,
        warnings,
    })
}

struct RawBone {
    name: String,
    parent: Option<String>,
    pivot: Vec3,
    rotation: Vec3,
    never_render: bool,
    cubes: Vec<Cube>,
    meshes: Vec<Mesh>,
    locators: Vec<(String, Vec3, Vec3)>,
}

fn parse_bone(
    rd: &Rd<'_>,
    p: &str,
    v: &Value,
    tex: [u32; 2],
    elements: &mut usize,
    locator_count: &mut usize,
    warnings: &mut Vec<String>,
) -> Result<RawBone, ModelError> {
    let b = rd.object(p, v)?;
    let name_path = key(p, "name");
    let name = rd.string(&name_path, rd.req(b, p, "name")?)?.to_string();
    if name.is_empty() {
        return Err(rd.err(&name_path, "bone name must not be empty"));
    }
    let parent = match Rd::opt(b, "parent") {
        Some(pv) => Some(rd.string(&key(p, "parent"), pv)?.to_string()),
        None => None,
    };
    let pivot = bedrock_position(rd.opt_vec3(b, p, "pivot", Vec3::ZERO)?);
    let rot_key = if Rd::opt(b, "rotation").is_some() { "rotation" } else { "bind_pose_rotation" };
    let rotation = bedrock_rotation(rd.opt_vec3(b, p, rot_key, Vec3::ZERO)?);
    let mirror = rd.opt_bool(b, p, "mirror")?.unwrap_or(false);
    let inflate = rd.opt_f32(b, p, "inflate")?.unwrap_or(0.0);
    let never_render = rd.opt_bool(b, p, "neverRender")?.unwrap_or(false);

    let mut cubes = Vec::new();
    if let Some(cv) = Rd::opt(b, "cubes") {
        let cp = key(p, "cubes");
        let arr = rd.array(&cp, cv)?;
        check_counts(rd, &cp, 0, *elements + arr.len())?;
        for (i, c) in arr.iter().enumerate() {
            cubes.push(parse_cube(rd, &at(&cp, i), c, mirror, inflate, warnings)?);
        }
        *elements += arr.len();
    }

    let mut meshes = Vec::new();
    if let Some(mv) = Rd::opt(b, "poly_mesh") {
        check_counts(rd, &key(p, "poly_mesh"), 0, *elements + 1)?;
        *elements += 1;
        meshes.push(parse_poly_mesh(rd, &key(p, "poly_mesh"), mv, &name, tex)?);
    }

    let mut locators = Vec::new();
    if let Some(lv) = Rd::opt(b, "locators") {
        let lp = key(p, "locators");
        let lo = rd.object(&lp, lv)?;
        for (lname, l) in lo {
            *locator_count += 1;
            if *locator_count > MAX_LOCATORS {
                return Err(rd.err(&lp, format!("too many locators (limit {MAX_LOCATORS})")));
            }
            let path = key(&lp, lname);
            let (off, rot) = match l {
                Value::Array(_) => (rd.vec3(&path, l)?, Vec3::ZERO),
                Value::Object(m) => (
                    rd.vec3(&key(&path, "offset"), rd.req(m, &path, "offset")?)?,
                    rd.opt_vec3(m, &path, "rotation", Vec3::ZERO)?,
                ),
                _ => return Err(rd.err(&path, "expected [x, y, z] or an object with \"offset\"")),
            };
            locators.push((lname.clone(), bedrock_position(off), bedrock_rotation(rot)));
        }
    }

    Ok(RawBone { name, parent, pivot, rotation, never_render, cubes, meshes, locators })
}

fn parse_cube(
    rd: &Rd<'_>,
    p: &str,
    v: &Value,
    bone_mirror: bool,
    bone_inflate: f32,
    warnings: &mut Vec<String>,
) -> Result<Cube, ModelError> {
    let c = rd.object(p, v)?;
    let origin = rd.vec3(&key(p, "origin"), rd.req(c, p, "origin")?)?;
    let size = rd.vec3(&key(p, "size"), rd.req(c, p, "size")?)?;
    let pivot = rd.opt_vec3(c, p, "pivot", Vec3::ZERO)?;
    let rotation = rd.opt_vec3(c, p, "rotation", Vec3::ZERO)?;
    let inflate = rd.opt_f32(c, p, "inflate")?.unwrap_or(bone_inflate);
    let mirror = rd.opt_bool(c, p, "mirror")?.unwrap_or(bone_mirror);

    // Normalise a negative size, then mirror X into the internal space.
    let a = origin;
    let b = origin + size;
    let lo = a.min(b);
    let hi = a.max(b);
    let ext = hi - lo;
    let int_origin = Vec3::new(0.0 - hi.x, lo.y, lo.z);

    let uv_path = key(p, "uv");
    let uv_v = rd.req(c, p, "uv")?;
    let mut faces_enabled = [true; 6];
    let uv = match uv_v {
        Value::Array(_) => {
            let o = rd.vec2(&uv_path, uv_v)?;
            CubeUv::Box { offset: o }
        }
        Value::Object(m) => {
            let mut faces = [None; 6];
            for (fname, fv) in m {
                let Some(face) = Face::from_name(fname) else {
                    warnings.push(format!("{}: {}: unknown face {fname:?} ignored", rd.file, uv_path));
                    continue;
                };
                let fp = key(&uv_path, fname);
                let fo = rd.object(&fp, fv)?;
                let uv = rd.vec2(&key(&fp, "uv"), rd.req(fo, &fp, "uv")?)?;
                let natural = match face {
                    Face::North | Face::South => [ext.x, ext.y],
                    Face::East | Face::West => [ext.z, ext.y],
                    Face::Up | Face::Down => [ext.x, ext.z],
                };
                let uv_size = match Rd::opt(fo, "uv_size") {
                    Some(sv) => rd.vec2(&key(&fp, "uv_size"), sv)?,
                    None => natural,
                };
                let rotation = match Rd::opt(fo, "uv_rotation") {
                    Some(rv) => rd.uv_rotation(&key(&fp, "uv_rotation"), rv)?,
                    None => 0,
                };
                // Blockbench's Bedrock exporter writes up and down with both axes flipped; undo it.
                let (uv, uv_size) = if matches!(face, Face::Up | Face::Down) {
                    ([uv[0] + uv_size[0], uv[1] + uv_size[1]], [0.0 - uv_size[0], 0.0 - uv_size[1]])
                } else {
                    (uv, uv_size)
                };
                faces[face as usize] = Some(FaceUv { uv, uv_size, rotation });
            }
            for f in Face::ALL {
                faces_enabled[f as usize] = faces[f as usize].is_some();
            }
            CubeUv::PerFace(faces)
        }
        other => {
            return Err(rd.err(&uv_path, format!("expected [u, v] or an object of faces, found {}", describe(other))));
        }
    };

    Ok(Cube {
        origin: int_origin,
        size: ext,
        pivot: bedrock_position(pivot),
        rotation: bedrock_rotation(rotation),
        inflate,
        mirror,
        uv,
        faces_enabled,
    })
}

fn describe(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

fn parse_poly_mesh(rd: &Rd<'_>, p: &str, v: &Value, bone: &str, tex: [u32; 2]) -> Result<Mesh, ModelError> {
    let m = rd.object(p, v)?;
    let normalized = rd.opt_bool(m, p, "normalized_uvs")?.unwrap_or(false);

    let pos_path = key(p, "positions");
    let pos_arr = rd.array(&pos_path, rd.req(m, p, "positions")?)?;
    if pos_arr.len() > MAX_MESH_VERTICES {
        return Err(rd.err(&pos_path, format!("too many vertices (limit {MAX_MESH_VERTICES})")));
    }
    let mut vertices = Vec::with_capacity(pos_arr.len());
    for (i, pv) in pos_arr.iter().enumerate() {
        vertices.push(bedrock_position(rd.vec3(&at(&pos_path, i), pv)?));
    }

    let mut normals = Vec::new();
    if let Some(nv) = Rd::opt(m, "normals") {
        let np = key(p, "normals");
        let arr = rd.array(&np, nv)?;
        if arr.len() > MAX_MESH_VERTICES {
            return Err(rd.err(&np, format!("too many normals (limit {MAX_MESH_VERTICES})")));
        }
        for (i, n) in arr.iter().enumerate() {
            normals.push(bedrock_position(rd.vec3(&at(&np, i), n)?));
        }
    }

    let uv_path = key(p, "uvs");
    let uv_arr = rd.array(&uv_path, rd.req(m, p, "uvs")?)?;
    if uv_arr.len() > MAX_MESH_VERTICES {
        return Err(rd.err(&uv_path, format!("too many uvs (limit {MAX_MESH_VERTICES})")));
    }
    let scale = if normalized { [tex[0] as f32, tex[1] as f32] } else { [1.0, 1.0] };
    let mut uvs = Vec::with_capacity(uv_arr.len());
    for (i, u) in uv_arr.iter().enumerate() {
        let t = rd.vec2(&at(&uv_path, i), u)?;
        uvs.push([t[0] * scale[0], t[1] * scale[1]]);
    }

    let polys_path = key(p, "polys");
    let polys_v = rd.req(m, p, "polys")?;
    let polys = match polys_v {
        Value::Array(a) => a,
        Value::String(s) => {
            return Err(rd.err(&polys_path, format!("polys {s:?} is not supported, write the polygons as arrays")))
        }
        other => return Err(rd.err(&polys_path, format!("expected an array of polygons, found {}", describe(other)))),
    };
    if polys.len() > MAX_MESH_VERTICES {
        return Err(rd.err(&polys_path, format!("too many polygons (limit {MAX_MESH_VERTICES})")));
    }
    let mut faces = Vec::with_capacity(polys.len());
    for (fi, poly) in polys.iter().enumerate() {
        let pp = at(&polys_path, fi);
        let corners_v = rd.array(&pp, poly)?;
        if corners_v.len() < 3 || corners_v.len() > MAX_MESH_VERTICES {
            return Err(rd.err(&pp, "a polygon needs at least 3 corners"));
        }
        let mut corners = Vec::with_capacity(corners_v.len());
        for (ci, cv) in corners_v.iter().enumerate() {
            let cp = at(&pp, ci);
            let t = rd.array(&cp, cv)?;
            if t.len() != 3 {
                return Err(rd.err(&cp, "expected [position index, normal index, uv index]"));
            }
            let index = |slot: usize, len: usize, what: &str| -> Result<usize, ModelError> {
                let sp = at(&cp, slot);
                let n = rd.number(&sp, &t[slot])?;
                if n.fract() != 0.0 || n < 0.0 || n as usize >= len {
                    return Err(rd.err(&sp, format!("{what} index {n} is out of range (0..{len})")));
                }
                Ok(n as usize)
            };
            let vi = index(0, vertices.len(), "position")?;
            let ui = index(2, uvs.len(), "uv")?;
            let normal = if normals.is_empty() {
                None
            } else {
                Some(normals[index(1, normals.len(), "normal")?])
            };
            corners.push(MeshCorner { vertex: vi as u32, uv: uvs[ui], normal });
        }
        // The X mirror reverses the winding; restore counter-clockwise-from-outside.
        corners.reverse();
        faces.push(MeshFace { corners });
    }
    Ok(Mesh { name: format!("{bone}.poly_mesh"), vertices, faces })
}
