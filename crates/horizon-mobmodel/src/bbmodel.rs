//! Blockbench `.bbmodel` loader.
//!
//! A bbmodel is already in the internal space (see [`crate::model`]); nothing is mirrored. Choices where the format is
//! loose (all written down in the F4M-1 report):
//! * Bones are the outliner groups, in depth-first order with children in outliner order. Elements that sit at the
//!   top level of the outliner (outside any group) go into one synthetic bone named `root` that is added last.
//! * `cube`, `mesh`, `locator` and `null_object` elements are read. Other element types are skipped with a warning.
//!   Elements and groups with `"export": false` are skipped. Elements missing from the outliner are skipped with a
//!   warning.
//! * A cube pivots about its `origin` and rotates `Rz * Ry * Rx`. Cube faces with `"texture": null` are not drawn. With
//!   `box_uv` the classic box layout is used from `uv_offset`; otherwise each face's `uv` is `[x1, y1, x2, y2]` in
//!   texels and `rotation` is its UV rotation (0, 90, 180, 270).
//! * Mesh vertex positions are relative to the mesh `origin`; the mesh `rotation` turns them about the origin. Faces
//!   and vertices are taken sorted by their id string (deterministic whatever the file order).
//! * `animations` is kept as the raw JSON value for the animation loader.

use crate::model::*;
use crate::skeleton::{Bone, BoneIndex};
use base64::Engine;
use glam::Vec3;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};

const MAX_DEPTH: usize = 64;
const MAX_TEXTURE_SOURCE_LEN: usize = 48 * 1024 * 1024;

/// Loads a `.bbmodel`. `file` is only used in error messages.
pub fn load_bbmodel(file: &str, bytes: &[u8]) -> Result<Model, ModelError> {
    let rd = Rd { file };
    let root = rd.parse(bytes)?;
    let obj = rd.object("", &root)?;
    let mut warnings = Vec::new();

    let mut texture_size = [64u32, 64u32];
    if let Some(r) = Rd::opt(obj, "resolution") {
        let ro = rd.object("resolution", r)?;
        texture_size[0] = rd.dimension("resolution.width", rd.req(ro, "resolution", "width")?)?;
        texture_size[1] = rd.dimension("resolution.height", rd.req(ro, "resolution", "height")?)?;
    } else {
        warnings.push(format!("{file}: no resolution, assuming 64x64"));
    }

    let identifier = match (rd.opt_str(obj, "", "model_identifier")?, rd.opt_str(obj, "", "name")?) {
        (Some(id), _) if !id.is_empty() => id.to_string(),
        (_, Some(n)) if !n.is_empty() => n.to_string(),
        _ => file.split('.').next().unwrap_or("model").to_string(),
    };

    // Textures.
    let mut textures = Vec::new();
    if let Some(tv) = Rd::opt(obj, "textures") {
        let arr = rd.array("textures", tv)?;
        if arr.len() > MAX_TEXTURES {
            return Err(rd.err("textures", format!("too many textures ({}, limit {MAX_TEXTURES})", arr.len())));
        }
        for (i, t) in arr.iter().enumerate() {
            textures.push(parse_texture(&rd, &at("textures", i), t)?);
        }
    }

    // Elements by uuid.
    let empty = Vec::new();
    let elements: &Vec<Value> = match Rd::opt(obj, "elements") {
        Some(v) => rd.array("elements", v)?,
        None => &empty,
    };
    if elements.len() > crate::limits::MAX_CUBES + MAX_LOCATORS + 1024 {
        return Err(rd.err("elements", format!("too many elements ({})", elements.len())));
    }
    let mut by_uuid: HashMap<&str, usize> = HashMap::new();
    for (i, e) in elements.iter().enumerate() {
        let p = at("elements", i);
        let eo = rd.object(&p, e)?;
        let uuid = rd.string(&key(&p, "uuid"), rd.req(eo, &p, "uuid")?)?;
        if by_uuid.insert(uuid, i).is_some() {
            return Err(rd.err(&key(&p, "uuid"), format!("duplicate uuid {uuid:?}")));
        }
    }

    let mut b = Builder {
        rd: &rd,
        elements,
        by_uuid,
        used: vec![false; elements.len()],
        model: Model::default(),
        names: HashSet::new(),
        element_count: 0,
        locator_count: 0,
        orphans: Vec::new(),
    };
    if let Some(ov) = Rd::opt(obj, "outliner") {
        let items = rd.array("outliner", ov)?;
        b.walk(items, "outliner", None, 0)?;
    }
    if !b.orphans.is_empty() {
        let mut name = "root".to_string();
        while b.names.contains(&name) {
            name.insert(0, '_');
        }
        check_counts(&rd, "outliner", b.model.skeleton.bones.len() + 1, 0)?;
        let idx = b.push_bone(name, None, Vec3::ZERO, Vec3::ZERO);
        let orphans = std::mem::take(&mut b.orphans);
        for (ei, path) in orphans {
            b.element(ei, &path, idx)?;
        }
    }
    for (i, used) in b.used.iter().enumerate() {
        if !used {
            warnings.push(format!("{file}: {}: element is not in the outliner and was skipped", at("elements", i)));
        }
    }

    let mut model = b.model;
    warnings.append(&mut model.warnings);
    model.identifier = identifier;
    model.texture_size = texture_size;
    model.textures = textures;
    model.animations = obj.get("animations").filter(|v| !v.is_null()).cloned();
    model.warnings = warnings;
    Ok(model)
}

struct Builder<'a> {
    rd: &'a Rd<'a>,
    elements: &'a Vec<Value>,
    by_uuid: HashMap<&'a str, usize>,
    used: Vec<bool>,
    model: Model,
    names: HashSet<String>,
    element_count: usize,
    locator_count: usize,
    orphans: Vec<(usize, String)>,
}

impl<'a> Builder<'a> {
    fn warn(&mut self, path: &str, msg: &str) {
        let w = format!("{}: {path}: {msg}", self.rd.file);
        self.model.warnings.push(w);
    }

    fn push_bone(&mut self, name: String, parent: Option<BoneIndex>, pivot: Vec3, rotation: Vec3) -> BoneIndex {
        let idx = BoneIndex(self.model.skeleton.bones.len() as u16);
        self.names.insert(name.clone());
        self.model.skeleton.bones.push(Bone { name, parent, pivot, rotation });
        self.model.parts.push(BonePart::default());
        idx
    }

    fn walk(&mut self, items: &'a [Value], path: &str, parent: Option<BoneIndex>, depth: usize) -> Result<(), ModelError> {
        if depth > MAX_DEPTH {
            return Err(self.rd.err(path, format!("outliner is nested deeper than {MAX_DEPTH} groups")));
        }
        for (i, item) in items.iter().enumerate() {
            let p = at(path, i);
            match item {
                Value::String(uuid) => {
                    let Some(&ei) = self.by_uuid.get(uuid.as_str()) else {
                        return Err(self.rd.err(&p, format!("no element with uuid {uuid:?}")));
                    };
                    match parent {
                        Some(bone) => self.element(ei, &p, bone)?,
                        None => {
                            self.used[ei] = true;
                            self.orphans.push((ei, p));
                        }
                    }
                }
                Value::Object(g) => self.group(g, &p, parent, depth)?,
                other => {
                    return Err(self.rd.err(&p, format!("expected an element uuid or a group, found {other}")));
                }
            }
        }
        Ok(())
    }

    fn group(&mut self, g: &'a Map<String, Value>, p: &str, parent: Option<BoneIndex>, depth: usize) -> Result<(), ModelError> {
        let rd = self.rd;
        if rd.opt_bool(g, p, "export")? == Some(false) {
            self.mark_used(g, 0);
            return Ok(());
        }
        let name_path = key(p, "name");
        let name = rd.string(&name_path, rd.req(g, p, "name")?)?.to_string();
        if name.is_empty() {
            return Err(rd.err(&name_path, "group name must not be empty"));
        }
        if self.names.contains(&name) {
            return Err(rd.err(&name_path, format!("duplicate bone name {name:?}")));
        }
        let origin = rd.opt_vec3(g, p, "origin", Vec3::ZERO)?;
        let rotation = rd.opt_vec3(g, p, "rotation", Vec3::ZERO)?;
        check_counts(rd, p, self.model.skeleton.bones.len() + 1, 0)?;
        let idx = self.push_bone(name, parent, origin, rotation);
        if let Some(cv) = Rd::opt(g, "children") {
            let cp = key(p, "children");
            let arr = rd.array(&cp, cv)?;
            self.walk(arr, &cp, Some(idx), depth + 1)?;
        }
        Ok(())
    }

    /// Marks every element below a skipped group as seen, so it is not reported as missing from the outliner.
    fn mark_used(&mut self, g: &Map<String, Value>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let Some(Value::Array(children)) = g.get("children") else { return };
        for c in children {
            match c {
                Value::String(uuid) => {
                    if let Some(&ei) = self.by_uuid.get(uuid.as_str()) {
                        self.used[ei] = true;
                    }
                }
                Value::Object(sub) => self.mark_used(sub, depth + 1),
                _ => {}
            }
        }
    }

    fn element(&mut self, ei: usize, ref_path: &str, bone: BoneIndex) -> Result<(), ModelError> {
        self.used[ei] = true;
        let rd = self.rd;
        let p = at("elements", ei);
        let e = rd.object(&p, &self.elements[ei])?;
        if rd.opt_bool(e, &p, "export")? == Some(false) {
            return Ok(());
        }
        let kind = rd.opt_str(e, &p, "type")?.unwrap_or("cube");
        match kind {
            "cube" => {
                check_counts(rd, &p, 0, self.element_count + 1)?;
                self.element_count += 1;
                let cube = parse_cube(rd, &p, e)?;
                self.model.parts[bone.index()].cubes.push(cube);
            }
            "mesh" => {
                check_counts(rd, &p, 0, self.element_count + 1)?;
                self.element_count += 1;
                let mesh = parse_mesh(rd, &p, e, &mut self.model.warnings)?;
                self.model.parts[bone.index()].meshes.push(mesh);
            }
            "locator" | "null_object" => {
                self.locator_count += 1;
                if self.locator_count > MAX_LOCATORS {
                    return Err(rd.err(&p, format!("too many locators (limit {MAX_LOCATORS})")));
                }
                let name = rd.opt_str(e, &p, "name")?.unwrap_or("").to_string();
                let offset = rd.vec3(&key(&p, "from"), rd.req(e, &p, "from")?)?;
                let rotation = rd.opt_vec3(e, &p, "rotation", Vec3::ZERO)?;
                self.model.locators.push(Locator { name, bone, offset, rotation, null_object: kind == "null_object" });
            }
            other => {
                let msg = format!("element type {other:?} is not supported and was skipped (outliner {ref_path})");
                self.warn(&p, &msg);
            }
        }
        Ok(())
    }
}

fn vec4(rd: &Rd<'_>, path: &str, v: &Value) -> Result<[f32; 4], ModelError> {
    let a = v
        .as_array()
        .filter(|a| a.len() == 4)
        .ok_or_else(|| rd.err(path, "expected an array of 4 numbers [x1, y1, x2, y2]"))?;
    Ok([
        rd.number(&at(path, 0), &a[0])?,
        rd.number(&at(path, 1), &a[1])?,
        rd.number(&at(path, 2), &a[2])?,
        rd.number(&at(path, 3), &a[3])?,
    ])
}

fn parse_cube(rd: &Rd<'_>, p: &str, e: &Map<String, Value>) -> Result<Cube, ModelError> {
    let from = rd.vec3(&key(p, "from"), rd.req(e, p, "from")?)?;
    let to = rd.vec3(&key(p, "to"), rd.req(e, p, "to")?)?;
    let pivot = rd.opt_vec3(e, p, "origin", Vec3::ZERO)?;
    let rotation = rd.opt_vec3(e, p, "rotation", Vec3::ZERO)?;
    let inflate = rd.opt_f32(e, p, "inflate")?.unwrap_or(0.0);
    let box_uv = rd.opt_bool(e, p, "box_uv")?.unwrap_or(false);
    let mirror = rd.opt_bool(e, p, "mirror_uv")?.unwrap_or(false);
    let offset = match Rd::opt(e, "uv_offset") {
        Some(v) => rd.vec2(&key(p, "uv_offset"), v)?,
        None => [0.0, 0.0],
    };
    let faces_obj = match Rd::opt(e, "faces") {
        Some(f) => Some(rd.object(&key(p, "faces"), f)?),
        None => None,
    };

    let lo = from.min(to);
    let hi = from.max(to);
    let mut faces_enabled = [true; 6];
    let mut faces = [None; 6];
    for f in Face::ALL {
        let fp = key(&key(p, "faces"), f.name());
        let fv = faces_obj.and_then(|m| m.get(f.name()));
        let fo = match fv {
            Some(v) if !v.is_null() => Some(rd.object(&fp, v)?),
            _ => None,
        };
        let textured = fo.is_some_and(|o| o.get("texture") != Some(&Value::Null));
        if box_uv {
            // Without a faces object every face is drawn; with one, only those that have a texture.
            faces_enabled[f as usize] = faces_obj.is_none() || textured;
        } else {
            faces_enabled[f as usize] = textured;
            if let (true, Some(fo)) = (textured, fo) {
                let uv = match Rd::opt(fo, "uv") {
                    Some(v) => vec4(rd, &key(&fp, "uv"), v)?,
                    None => [0.0; 4],
                };
                let rot = match Rd::opt(fo, "rotation") {
                    Some(v) => rd.uv_rotation(&key(&fp, "rotation"), v)?,
                    None => 0,
                };
                faces[f as usize] =
                    Some(FaceUv { uv: [uv[0], uv[1]], uv_size: [uv[2] - uv[0], uv[3] - uv[1]], rotation: rot });
            }
        }
    }
    let uv = if box_uv { CubeUv::Box { offset } } else { CubeUv::PerFace(faces) };
    Ok(Cube { origin: lo, size: hi - lo, pivot, rotation, inflate, mirror, uv, faces_enabled })
}

fn parse_mesh(rd: &Rd<'_>, p: &str, e: &Map<String, Value>, warnings: &mut Vec<String>) -> Result<Mesh, ModelError> {
    let name = rd.opt_str(e, p, "name")?.unwrap_or("mesh").to_string();
    let origin = rd.opt_vec3(e, p, "origin", Vec3::ZERO)?;
    let rotation = rd.opt_vec3(e, p, "rotation", Vec3::ZERO)?;
    let rot = euler_matrix(rotation);

    let vp = key(p, "vertices");
    let vobj = rd.object(&vp, rd.req(e, p, "vertices")?)?;
    if vobj.len() > MAX_MESH_VERTICES {
        return Err(rd.err(&vp, format!("too many vertices (limit {MAX_MESH_VERTICES})")));
    }
    let mut ids: Vec<&String> = vobj.keys().collect();
    ids.sort();
    let mut index: HashMap<&str, u32> = HashMap::new();
    let mut vertices = Vec::with_capacity(ids.len());
    for (i, id) in ids.iter().enumerate() {
        let v = rd.vec3(&key(&vp, id), &vobj[id.as_str()])?;
        vertices.push(origin + rot * v);
        index.insert(id.as_str(), i as u32);
    }

    let mut faces = Vec::new();
    if let Some(fv) = Rd::opt(e, "faces") {
        let fp = key(p, "faces");
        let fobj = rd.object(&fp, fv)?;
        if fobj.len() > MAX_MESH_VERTICES {
            return Err(rd.err(&fp, format!("too many faces (limit {MAX_MESH_VERTICES})")));
        }
        let mut fids: Vec<&String> = fobj.keys().collect();
        fids.sort();
        for fid in fids {
            let path = key(&fp, fid);
            let fo = rd.object(&path, &fobj[fid.as_str()])?;
            let vlist_path = key(&path, "vertices");
            let vlist = rd.array(&vlist_path, rd.req(fo, &path, "vertices")?)?;
            if vlist.len() < 3 {
                warnings.push(format!("{}: {path}: face with fewer than 3 vertices skipped", rd.file));
                continue;
            }
            if vlist.len() > MAX_MESH_VERTICES {
                return Err(rd.err(&vlist_path, format!("too many vertices (limit {MAX_MESH_VERTICES})")));
            }
            let uv_obj = match Rd::opt(fo, "uv") {
                Some(u) => Some(rd.object(&key(&path, "uv"), u)?),
                None => None,
            };
            let mut corners = Vec::with_capacity(vlist.len());
            for (ci, cv) in vlist.iter().enumerate() {
                let cp = at(&vlist_path, ci);
                let id = rd.string(&cp, cv)?;
                let Some(&vi) = index.get(id) else {
                    return Err(rd.err(&cp, format!("unknown vertex id {id:?}")));
                };
                let uv = match uv_obj.and_then(|m| m.get(id)) {
                    Some(u) => rd.vec2(&key(&key(&path, "uv"), id), u)?,
                    None => [0.0, 0.0],
                };
                corners.push(MeshCorner { vertex: vi, uv, normal: None });
            }
            faces.push(MeshFace { corners });
        }
    }
    Ok(Mesh { name, vertices, faces })
}

fn parse_texture(rd: &Rd<'_>, p: &str, v: &Value) -> Result<Texture, ModelError> {
    let t = rd.object(p, v)?;
    let name = rd.opt_str(t, p, "name")?.unwrap_or("").to_string();
    let path = match rd.opt_str(t, p, "path")? {
        Some(s) => Some(s.to_string()),
        None => rd.opt_str(t, p, "relative_path")?.map(str::to_string),
    };
    let (mut width, mut height, mut rgba) = (0, 0, Vec::new());
    if let Some(src) = rd.opt_str(t, p, "source")? {
        if let Some(rest) = src.strip_prefix("data:") {
            if src.len() > MAX_TEXTURE_SOURCE_LEN {
                return Err(rd.err(&key(p, "source"), "embedded texture is too large"));
            }
            let b64 = rest.split_once(',').map_or(rest, |(_, d)| d);
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|e| rd.err(&key(p, "source"), format!("invalid base64: {e}")))?;
            let (w, h, px) = decode_png(&bytes).map_err(|e| rd.err(&key(p, "source"), format!("invalid PNG: {e}")))?;
            width = w;
            height = h;
            rgba = px;
        }
    }
    Ok(Texture { name, path, width, height, rgba })
}
