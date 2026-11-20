//! The internal model: what `.geo.json` and `.bbmodel` both load into, and the JSON reading helpers the loaders share.
//!
//! # Coordinate conventions (pinned by tests, see `docs/yoake2/reports/FR-F4M-1.md`)
//!
//! The internal space is **Blockbench's model space**: pixels (1/16 block), +X east, +Y up, -Z north. Bone pivots,
//! cube origins and locator offsets are absolute positions in this space (not parent-relative). A `.bbmodel` is read
//! as is. A Bedrock `.geo.json` is mirrored in X on load, the same way Blockbench imports it:
//!
//! * positions `(x, y, z)` become `(-x, y, z)`; a cube spanning `[ox, ox + sx]` in the file spans `[-(ox + sx), -ox]`;
//! * rotations in degrees `(rx, ry, rz)` become `(-rx, -ry, rz)`;
//! * face names are kept (`east` stays `east`).
//!
//! Rotations are Euler angles in degrees applied as `Rz * Ry * Rx` (X first), for bones (see
//! [`crate::skeleton::bone_matrices`]), cubes (about the cube pivot) and meshes (about the mesh origin).

use crate::skeleton::{BoneIndex, Skeleton};
use glam::{Mat3, Quat, Vec3};
use serde_json::{Map, Value};

/// Locators per model.
pub const MAX_LOCATORS: usize = 1024;
/// Vertices (and faces) per mesh element.
pub const MAX_MESH_VERTICES: usize = 65_536;
/// Embedded textures per model.
pub const MAX_TEXTURES: usize = 64;
/// Largest accepted texture or texture-size edge in pixels.
pub const MAX_TEXTURE_DIM: u32 = 4096;
/// Largest accepted decoded RGBA8 byte size of one embedded texture (16 MiB).
pub const MAX_TEXTURE_BYTES: usize = 16 * 1024 * 1024;
/// Largest accepted sum of decoded RGBA8 byte sizes across all textures in one model (64 MiB).
pub const MAX_MODEL_TEXTURE_BYTES: usize = 64 * 1024 * 1024;
/// Maximum JSON container nesting depth accepted by the pre-scan (strictly below serde_json's 128 recursion limit).
pub(crate) const MAX_JSON_NESTING: usize = 120;
/// Largest accepted absolute value of any number in a model file (pixels or degrees).
pub const MAX_ABS_VALUE: f64 = 1.0e6;

/// A load error: always names the file and, where there is one, the JSON path of the offending value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelError {
    /// The file name given to the loader.
    pub file: String,
    /// JSON path such as `minecraft:geometry[0].bones[3].cubes[1].uv` (empty for whole-file problems).
    pub path: String,
    pub message: String,
}

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(f, "{}: {}: {}", self.file, self.path, self.message)
        }
    }
}

impl std::error::Error for ModelError {}

/// The six cube faces. The numeric value indexes the per-face arrays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    North = 0,
    East = 1,
    South = 2,
    West = 3,
    Up = 4,
    Down = 5,
}

impl Face {
    pub const ALL: [Face; 6] = [Face::North, Face::East, Face::South, Face::West, Face::Up, Face::Down];

    pub fn name(self) -> &'static str {
        match self {
            Face::North => "north",
            Face::East => "east",
            Face::South => "south",
            Face::West => "west",
            Face::Up => "up",
            Face::Down => "down",
        }
    }

    pub fn from_name(name: &str) -> Option<Face> {
        Face::ALL.into_iter().find(|f| f.name() == name)
    }

    /// Outward normal in model space.
    pub fn normal(self) -> Vec3 {
        match self {
            Face::North => Vec3::NEG_Z,
            Face::East => Vec3::X,
            Face::South => Vec3::Z,
            Face::West => Vec3::NEG_X,
            Face::Up => Vec3::Y,
            Face::Down => Vec3::NEG_Y,
        }
    }
}

/// The texture rectangle of one face, in texels: top-left corner `uv`, extent `uv_size` (either may be negative to
/// flip), and a rotation in degrees (0, 90, 180, 270).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceUv {
    pub uv: [f32; 2],
    pub uv_size: [f32; 2],
    pub rotation: u16,
}

/// How a cube is textured.
#[derive(Clone, Debug, PartialEq)]
pub enum CubeUv {
    /// The classic box layout from one offset.
    Box { offset: [f32; 2] },
    /// One rectangle per face (indexed by `Face as usize`); `None` faces are not drawn.
    PerFace([Option<FaceUv>; 6]),
}

/// One cube. All values are in the internal space.
#[derive(Clone, Debug, PartialEq)]
pub struct Cube {
    /// Minimum corner (before inflate).
    pub origin: Vec3,
    /// Extent along +X, +Y, +Z (never negative).
    pub size: Vec3,
    /// Rotation pivot.
    pub pivot: Vec3,
    /// Rotation in degrees about `pivot` (`Rz * Ry * Rx`).
    pub rotation: Vec3,
    /// Grows every face outward by this many pixels.
    pub inflate: f32,
    /// Mirrors a box UV layout (ignored for per-face UV).
    pub mirror: bool,
    pub uv: CubeUv,
    /// Faces that are drawn (indexed by `Face as usize`).
    pub faces_enabled: [bool; 6],
}

/// One corner of a mesh face.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshCorner {
    /// Index into [`Mesh::vertices`].
    pub vertex: u32,
    /// Texture position in texels.
    pub uv: [f32; 2],
    /// Explicit normal (model space); `None` means the flat face normal.
    pub normal: Option<Vec3>,
}

/// A face of 3 or more corners, counter-clockwise seen from outside.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshFace {
    pub corners: Vec<MeshCorner>,
}

/// A polygon mesh. Vertices are final positions in the internal space (element origin and rotation already applied).
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: String,
    pub vertices: Vec<Vec3>,
    pub faces: Vec<MeshFace>,
}

/// What one bone draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BonePart {
    pub never_render: bool,
    pub cubes: Vec<Cube>,
    pub meshes: Vec<Mesh>,
}

/// An attachment point (items, particles). `null_object` marks a bbmodel null object.
#[derive(Clone, Debug, PartialEq)]
pub struct Locator {
    pub name: String,
    pub bone: BoneIndex,
    /// Absolute rest position in the internal space.
    pub offset: Vec3,
    /// Rest rotation in degrees.
    pub rotation: Vec3,
    pub null_object: bool,
}

/// A texture: decoded pixels when embedded, otherwise only a path.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub name: String,
    pub path: Option<String>,
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes of RGBA8, or empty when the texture is not embedded.
    pub rgba: Vec<u8>,
}

/// The `visible_bounds_*` fields of a geo file (used for culling).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibleBounds {
    pub width: f32,
    pub height: f32,
    pub offset: Vec3,
}

/// A loaded model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub identifier: String,
    /// Texture size in texels: UVs in the model are relative to it.
    pub texture_size: [u32; 2],
    pub skeleton: Skeleton,
    /// Geometry per bone, indexed like `skeleton.bones`.
    pub parts: Vec<BonePart>,
    pub locators: Vec<Locator>,
    pub textures: Vec<Texture>,
    pub visible_bounds: Option<VisibleBounds>,
    /// Raw `animations` value of a bbmodel (F4M-2 parses it).
    pub animations: Option<Value>,
    /// Things that were ignored or defaulted.
    pub warnings: Vec<String>,
}

impl Model {
    /// Cubes and meshes in the whole model (what the cube limit counts).
    pub fn element_count(&self) -> usize {
        self.parts.iter().map(|p| p.cubes.len() + p.meshes.len()).sum()
    }
}

/// Rotation matrix for Euler degrees `(x, y, z)`: `Rz * Ry * Rx` (X is applied first).
pub fn euler_matrix(rot: Vec3) -> Mat3 {
    Mat3::from_quat(Quat::from_euler(
        glam::EulerRot::ZYX,
        rot.z.to_radians(),
        rot.y.to_radians(),
        rot.x.to_radians(),
    ))
}

/// Bedrock file position to internal position.
pub fn bedrock_position(p: Vec3) -> Vec3 {
    Vec3::new(0.0 - p.x, p.y, p.z)
}

/// Bedrock file rotation (degrees) to internal rotation.
pub fn bedrock_rotation(r: Vec3) -> Vec3 {
    Vec3::new(0.0 - r.x, 0.0 - r.y, r.z)
}

// ---------------------------------------------------------------------------------------------------------------------
// JSON reading helpers shared by the loaders.

/// `parent.key` (or just `key` for the root).
pub(crate) fn key(path: &str, k: &str) -> String {
    if path.is_empty() {
        k.to_string()
    } else {
        format!("{path}.{k}")
    }
}

/// `parent[i]`.
pub(crate) fn at(path: &str, i: usize) -> String {
    format!("{path}[{i}]")
}

/// Scans raw JSON bytes before `serde_json` runs and rejects nesting deeper than `max_depth`, ignoring `[`/`{`
/// inside JSON string literals while respecting `\"` and `\\` escapes. Zero allocations.
pub(crate) fn check_json_nesting(bytes: &[u8], max_depth: usize) -> Result<(), usize> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for &b in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > max_depth {
                        return Err(depth);
                    }
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Reads typed values out of a `serde_json::Value`, producing errors with the file name and path.
pub(crate) struct Rd<'a> {
    pub file: &'a str,
}

impl<'a> Rd<'a> {
    pub fn err(&self, path: &str, message: impl Into<String>) -> ModelError {
        ModelError { file: self.file.to_string(), path: path.to_string(), message: message.into() }
    }

    pub fn parse(&self, bytes: &[u8]) -> Result<Value, ModelError> {
        self.parse_with_nesting_path(bytes, "json", format!("JSON nested deeper than {MAX_JSON_NESTING} levels"))
    }

    pub fn parse_with_nesting_path(
        &self,
        bytes: &[u8],
        nesting_path: &str,
        nesting_msg: String,
    ) -> Result<Value, ModelError> {
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        if check_json_nesting(bytes, MAX_JSON_NESTING).is_err() {
            return Err(self.err(nesting_path, nesting_msg));
        }
        serde_json::from_slice::<Value>(bytes).map_err(|e| self.err("", format!("invalid JSON: {e}")))
    }

    pub fn object<'v>(&self, path: &str, v: &'v Value) -> Result<&'v Map<String, Value>, ModelError> {
        v.as_object().ok_or_else(|| self.err(path, format!("expected an object, found {}", kind(v))))
    }

    pub fn array<'v>(&self, path: &str, v: &'v Value) -> Result<&'v Vec<Value>, ModelError> {
        v.as_array().ok_or_else(|| self.err(path, format!("expected an array, found {}", kind(v))))
    }

    pub fn string<'v>(&self, path: &str, v: &'v Value) -> Result<&'v str, ModelError> {
        v.as_str().ok_or_else(|| self.err(path, format!("expected a string, found {}", kind(v))))
    }

    pub fn boolean(&self, path: &str, v: &Value) -> Result<bool, ModelError> {
        v.as_bool().ok_or_else(|| self.err(path, format!("expected true or false, found {}", kind(v))))
    }

    pub fn number(&self, path: &str, v: &Value) -> Result<f32, ModelError> {
        let n = v.as_f64().ok_or_else(|| self.err(path, format!("expected a number, found {}", kind(v))))?;
        if !n.is_finite() || n.abs() > MAX_ABS_VALUE {
            return Err(self.err(path, format!("number out of range (|value| must be at most {MAX_ABS_VALUE})")));
        }
        Ok(n as f32)
    }

    pub fn vec3(&self, path: &str, v: &Value) -> Result<Vec3, ModelError> {
        let a = v.as_array().filter(|a| a.len() == 3).ok_or_else(|| {
            self.err(path, format!("expected an array of 3 numbers, found {}", kind(v)))
        })?;
        Ok(Vec3::new(
            self.number(&at(path, 0), &a[0])?,
            self.number(&at(path, 1), &a[1])?,
            self.number(&at(path, 2), &a[2])?,
        ))
    }

    pub fn vec2(&self, path: &str, v: &Value) -> Result<[f32; 2], ModelError> {
        let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| {
            self.err(path, format!("expected an array of 2 numbers, found {}", kind(v)))
        })?;
        Ok([self.number(&at(path, 0), &a[0])?, self.number(&at(path, 1), &a[1])?])
    }

    /// A required member.
    pub fn req<'v>(&self, m: &'v Map<String, Value>, path: &str, k: &str) -> Result<&'v Value, ModelError> {
        match m.get(k) {
            Some(v) if !v.is_null() => Ok(v),
            _ => Err(self.err(&key(path, k), "missing required field")),
        }
    }

    /// An optional member (`null` counts as absent).
    pub fn opt<'v>(m: &'v Map<String, Value>, k: &str) -> Option<&'v Value> {
        m.get(k).filter(|v| !v.is_null())
    }

    pub fn opt_vec3(&self, m: &Map<String, Value>, path: &str, k: &str, default: Vec3) -> Result<Vec3, ModelError> {
        match Self::opt(m, k) {
            Some(v) => self.vec3(&key(path, k), v),
            None => Ok(default),
        }
    }

    pub fn opt_f32(&self, m: &Map<String, Value>, path: &str, k: &str) -> Result<Option<f32>, ModelError> {
        match Self::opt(m, k) {
            Some(v) => Ok(Some(self.number(&key(path, k), v)?)),
            None => Ok(None),
        }
    }

    pub fn opt_bool(&self, m: &Map<String, Value>, path: &str, k: &str) -> Result<Option<bool>, ModelError> {
        match Self::opt(m, k) {
            Some(v) => Ok(Some(self.boolean(&key(path, k), v)?)),
            None => Ok(None),
        }
    }

    pub fn opt_str<'v>(&self, m: &'v Map<String, Value>, path: &str, k: &str) -> Result<Option<&'v str>, ModelError> {
        match Self::opt(m, k) {
            Some(v) => Ok(Some(self.string(&key(path, k), v)?)),
            None => Ok(None),
        }
    }

    /// A whole number in `1..=MAX_TEXTURE_DIM`.
    pub fn dimension(&self, path: &str, v: &Value) -> Result<u32, ModelError> {
        let n = self.number(path, v)?;
        if n.fract() != 0.0 || n < 1.0 || n > MAX_TEXTURE_DIM as f32 {
            return Err(self.err(path, format!("expected a whole number from 1 to {MAX_TEXTURE_DIM}, found {n}")));
        }
        Ok(n as u32)
    }

    /// A UV rotation: a multiple of 90 degrees, normalised to 0, 90, 180 or 270.
    pub fn uv_rotation(&self, path: &str, v: &Value) -> Result<u16, ModelError> {
        let n = self.number(path, v)?;
        if n.fract() != 0.0 || (n as i64) % 90 != 0 {
            return Err(self.err(path, format!("expected a multiple of 90 degrees, found {n}")));
        }
        Ok((n as i64).rem_euclid(360) as u16)
    }
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// Decodes a PNG to RGBA8: `(width, height, pixels)`.
///
/// Reads the IHDR dimensions directly from `bytes` before any allocation, enforces `MAX_TEXTURE_DIM`,
/// `MAX_TEXTURE_BYTES`, and `max_bytes` (the model's remaining texture budget), and allocates the output buffer once.
pub(crate) fn decode_png(bytes: &[u8], max_bytes: usize) -> Result<(u32, u32, Vec<u8>), String> {
    const PNG_SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 33 || !bytes.starts_with(&PNG_SIG) || &bytes[12..16] != b"IHDR" {
        return Err("invalid PNG header".to_string());
    }
    let ihdr_len = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if ihdr_len != 13 {
        return Err("invalid PNG IHDR length".to_string());
    }
    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
    if w == 0 || h == 0 || w > MAX_TEXTURE_DIM || h > MAX_TEXTURE_DIM {
        return Err(format!("texture size {w}x{h} is outside 1..={MAX_TEXTURE_DIM}"));
    }
    let decoded_bytes = (w as u64)
        .checked_mul(h as u64)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| "texture is too large".to_string())?;
    if decoded_bytes > MAX_TEXTURE_BYTES as u64 {
        return Err(format!("decoded texture size ({decoded_bytes} bytes) exceeds {MAX_TEXTURE_BYTES} bytes"));
    }
    if decoded_bytes > max_bytes as u64 {
        return Err(format!("total decoded texture size exceeds {MAX_MODEL_TEXTURE_BYTES} bytes"));
    }

    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    if reader.info().size() != (w, h) {
        return Err("PNG header size mismatch".to_string());
    }
    let px = (w as usize) * (h as usize);
    let target_bytes = px * 4;
    let size = reader.output_buffer_size().ok_or("texture is too large")?;
    if size > MAX_TEXTURE_BYTES || size > max_bytes {
        return Err("texture is too large".to_string());
    }
    let mut rgba = vec![0u8; size.max(target_bytes)];
    let info = reader.next_frame(&mut rgba[..size]).map_err(|e| e.to_string())?;
    if (info.width, info.height) != (w, h) {
        return Err("PNG frame size mismatch".to_string());
    }
    let used = info.buffer_size();
    match info.color_type {
        png::ColorType::Rgba if used >= px * 4 => {}
        png::ColorType::Rgb if used >= px * 3 => {
            for i in (0..px).rev() {
                let (r, g, b) = (rgba[i * 3], rgba[i * 3 + 1], rgba[i * 3 + 2]);
                let d = i * 4;
                rgba[d] = r;
                rgba[d + 1] = g;
                rgba[d + 2] = b;
                rgba[d + 3] = 255;
            }
        }
        png::ColorType::GrayscaleAlpha if used >= px * 2 => {
            for i in (0..px).rev() {
                let (g, a) = (rgba[i * 2], rgba[i * 2 + 1]);
                let d = i * 4;
                rgba[d] = g;
                rgba[d + 1] = g;
                rgba[d + 2] = g;
                rgba[d + 3] = a;
            }
        }
        png::ColorType::Grayscale if used >= px => {
            for i in (0..px).rev() {
                let g = rgba[i];
                let d = i * 4;
                rgba[d] = g;
                rgba[d + 1] = g;
                rgba[d + 2] = g;
                rgba[d + 3] = 255;
            }
        }
        other => return Err(format!("unsupported PNG layout {other:?}")),
    }
    rgba.truncate(target_bytes);
    Ok((w, h, rgba))
}

/// Checks the limits that apply to every model.
pub(crate) fn check_counts(rd: &Rd<'_>, path: &str, bones: usize, elements: usize) -> Result<(), ModelError> {
    if bones > crate::limits::MAX_BONES {
        return Err(rd.err(path, format!("too many bones ({bones}, limit {})", crate::limits::MAX_BONES)));
    }
    if elements > crate::limits::MAX_CUBES {
        return Err(rd.err(path, format!("too many cubes and meshes ({elements}, limit {})", crate::limits::MAX_CUBES)));
    }
    Ok(())
}
