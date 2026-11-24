//! A chunk generator in a guest (F3-8): reading the engine's request and building the answer of a
//! `handle-call` with the method [`METHOD`]. The wire format is in the host's
//! `wasm_mods/level_services.rs` and in `docs/yoake2/MOD-GUIDE.md`.
//!
//! ```ignore
//! fn handle_call(&mut self, caller: String, method: String, payload: Vec<u8>) -> Result<Vec<u8>, String> {
//!     if method == generator::METHOD {
//!         let request = generator::Request::parse(&payload).ok_or("bad request")?;
//!         let mut fill = generator::Fill::new();
//!         fill.biome("minecraft:plains");
//!         fill.uniform(0, "minecraft:stone");
//!         return Ok(fill.to_bytes());
//!     }
//!     Err("unknown method".into())
//! }
//! ```
use crate::prelude::*;

/// The `handle-call` method the engine asks a chunk of.
pub const METHOD: &str = "yoake:generate-chunk";

/// The chunk asked for: the level, its generator, the chunk and the level's build range, the seed and
/// the stem's `settings` object (as JSON text).
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub level: String,
    pub generator: String,
    pub x: i32,
    pub z: i32,
    pub min_y: i32,
    pub height: i32,
    pub seed: i64,
    pub settings: String,
}

impl Request {
    /// Reads the engine's request (a JSON object); `None` when a field is missing.
    pub fn parse(bytes: &[u8]) -> Option<Self> {
        let text = core::str::from_utf8(bytes).ok()?;
        let fields = top_level_fields(text)?;
        let get = |key: &str| fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str());
        let string = |key: &str| get(key).and_then(|v| v.strip_prefix('"')?.strip_suffix('"')).map(String::from);
        let int = |key: &str| get(key).and_then(|v| v.parse::<i64>().ok());
        Some(Self {
            level: string("level")?,
            generator: string("generator")?,
            x: i32::try_from(int("x")?).ok()?,
            z: i32::try_from(int("z")?).ok()?,
            min_y: i32::try_from(int("min_y")?).ok()?,
            height: i32::try_from(int("height")?).ok()?,
            seed: int("seed")?,
            settings: get("settings").unwrap_or("{}").to_owned(),
        })
    }

    /// A whole-number setting of the stem (`"depth": 3`), when it has it at the top of its settings.
    pub fn setting_int(&self, key: &str) -> Option<i64> {
        top_level_fields(&self.settings)?
            .into_iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.parse().ok())
    }
}

/// The `"key": value` pairs at the top of a JSON object, the values as their JSON text. Enough for
/// the engine's request, which has no escapes in its keys.
fn top_level_fields(text: &str) -> Option<Vec<(String, String)>> {
    let b = text.trim().as_bytes();
    if b.first() != Some(&b'{') || b.last() != Some(&b'}') {
        return None;
    }
    let (mut out, mut i) = (Vec::new(), 1);
    while i < b.len() - 1 {
        while i < b.len() && (b[i] == b' ' || b[i] == b',') {
            i += 1;
        }
        if i >= b.len() - 1 {
            break;
        }
        if b[i] != b'"' {
            return None;
        }
        let key_start = i + 1;
        i = key_start;
        while b.get(i)? != &b'"' {
            i += 1;
        }
        let key = core::str::from_utf8(&b[key_start..i]).ok()?.to_owned();
        i += 1;
        while b.get(i)? == &b' ' || b.get(i)? == &b':' {
            i += 1;
        }
        let value_start = i;
        let (mut depth, mut in_string) = (0i32, false);
        while i < b.len() - 1 {
            let c = b[i];
            if in_string {
                if c == b'\\' {
                    i += 1;
                } else if c == b'"' {
                    in_string = false;
                }
            } else {
                match c {
                    b'"' => in_string = true,
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => depth -= 1,
                    b',' if depth == 0 => break,
                    _ => {}
                }
            }
            i += 1;
        }
        out.push((key, core::str::from_utf8(&b[value_start..i]).ok()?.trim().to_owned()));
    }
    Some(out)
}

enum Blocks {
    One(String),
    Palette(Vec<String>, Vec<u16>),
}

/// The answer: blocks and optional biomes by section (16 blocks high, from the bottom of the level).
/// A block section not given is left unchanged (air without a base, the base's blocks with one).
#[derive(Default)]
pub struct Fill {
    biome: Option<String>,
    biome_palette: Vec<String>,
    biome_columns: Option<Vec<u16>>,
    biome_sections: Vec<(u16, Vec<u16>)>,
    sections: Vec<(u16, Blocks)>,
}

impl Fill {
    pub fn new() -> Self {
        Self::default()
    }

    /// The biome of the whole chunk (`namespace:path`), applied before any v2 column or quart
    /// tables. Without those tables the answer keeps the exact v1 bytes.
    pub fn biome(&mut self, biome: &str) -> &mut Self {
        self.biome = Some(biome.to_owned());
        self
    }

    fn biome_index(&mut self, biome: &str) -> u16 {
        if let Some(index) = self.biome_palette.iter().position(|name| name == biome) {
            return index as u16;
        }
        assert!(self.biome_palette.len() < u16::MAX as usize, "too many biomes in one chunk");
        self.biome_palette.push(biome.to_owned());
        (self.biome_palette.len() - 1) as u16
    }

    /// The biome of one 4x4x4 quart cell. `qx` and `qz` are 0..4 within the chunk; `qy` counts
    /// quart cells from the level's bottom. The answer uses format v2. Explicit cells override
    /// `biome_column`, regardless of call order.
    pub fn biome_at(&mut self, qx: u16, qy: u16, qz: u16, biome: &str) -> &mut Self {
        assert!(qx < 4 && qz < 4, "quart x and z are 0..4");
        let palette = self.biome_index(biome);
        let section = qy / 4;
        if !self.biome_sections.iter().any(|(index, _)| *index == section) {
            self.biome_sections.push((section, vec![0; 64]));
        }
        let (_, indices) = self.biome_sections.iter_mut().find(|(index, _)| *index == section).unwrap();
        indices[((qy % 4) * 16 + qz * 4 + qx) as usize] = palette;
        self
    }

    /// The biome of a 4-block-wide column at every height. `qx` and `qz` are 0..4 within the
    /// chunk. The answer uses v2's compact 16-index, all-height form.
    pub fn biome_column(&mut self, qx: u16, qz: u16, biome: &str) -> &mut Self {
        assert!(qx < 4 && qz < 4, "quart x and z are 0..4");
        let palette = self.biome_index(biome);
        let indices = self.biome_columns.get_or_insert_with(|| vec![0; 16]);
        indices[(qz * 4 + qx) as usize] = palette;
        self
    }

    /// Section `index` (from the level's bottom) entirely of one block's default state.
    pub fn uniform(&mut self, index: u16, block: &str) -> &mut Self {
        self.sections.push((index, Blocks::One(block.to_owned())));
        self
    }

    /// Section `index` from a palette and 4096 indices into it, `(y * 16 + z) * 16 + x` inside the section.
    pub fn palette(&mut self, index: u16, palette: &[&str], indices: Vec<u16>) -> &mut Self {
        self.sections.push((
            index,
            Blocks::Palette(palette.iter().map(|s| (*s).to_owned()).collect(), indices),
        ));
        self
    }

    /// The bytes `handle-call` answers with.
    pub fn to_bytes(&self) -> Vec<u8> {
        fn string(out: &mut Vec<u8>, s: &str) {
            out.extend((s.len() as u16).to_le_bytes());
            out.extend(s.as_bytes());
        }
        let version = if self.biome_columns.is_some() || !self.biome_sections.is_empty() { 2 } else { 1 };
        let mut out = vec![version, u8::from(self.biome.is_some()) | u8::from(self.biome_columns.is_some()) << 1];
        if let Some(biome) = &self.biome {
            string(&mut out, biome);
        }
        if version == 2 {
            out.extend((self.biome_palette.len() as u16).to_le_bytes());
            for biome in &self.biome_palette {
                string(&mut out, biome);
            }
            if let Some(columns) = &self.biome_columns {
                for index in columns {
                    out.extend(index.to_le_bytes());
                }
            }
            out.extend((self.biome_sections.len() as u16).to_le_bytes());
            for (index, indices) in &self.biome_sections {
                out.extend(index.to_le_bytes());
                for index in indices {
                    out.extend(index.to_le_bytes());
                }
            }
        }
        out.extend((self.sections.len() as u16).to_le_bytes());
        for (index, blocks) in &self.sections {
            out.extend(index.to_le_bytes());
            match blocks {
                Blocks::One(block) => {
                    out.push(0);
                    string(&mut out, block);
                }
                Blocks::Palette(palette, indices) => {
                    out.push(1);
                    out.extend((palette.len() as u16).to_le_bytes());
                    for block in palette {
                        string(&mut out, block);
                    }
                    for i in indices {
                        out.extend(i.to_le_bytes());
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, value: &str) {
        out.extend((value.len() as u16).to_le_bytes());
        out.extend(value.as_bytes());
    }
    fn u16_at(bytes: &[u8], at: usize) -> u16 {
        u16::from_le_bytes([bytes[at], bytes[at + 1]])
    }

    #[test]
    fn fills_without_quart_biomes_keep_the_v1_bytes() {
        let mut fill = Fill::new();
        fill.biome("minecraft:plains").uniform(0, "minecraft:stone");
        let mut expected = vec![1, 1];
        string(&mut expected, "minecraft:plains");
        expected.extend(1u16.to_le_bytes());
        expected.extend(0u16.to_le_bytes());
        expected.push(0);
        string(&mut expected, "minecraft:stone");
        assert_eq!(fill.to_bytes(), expected);
    }

    #[test]
    fn v2_encodes_columns_and_explicit_quart_overrides() {
        let mut fill = Fill::new();
        fill.biome_at(2, 5, 1, "minecraft:plains");
        fill.biome_column(0, 0, "minecraft:desert");
        fill.biome_at(3, 5, 1, "minecraft:desert");
        let bytes = fill.to_bytes();
        assert_eq!(&bytes[..4], &[2, 2, 2, 0]);
        let columns_at = 2 + 2 + 2 * (2 + "minecraft:plains".len());
        assert_eq!(u16_at(&bytes, columns_at), 1, "the first column is desert");
        let sections_at = columns_at + 32;
        assert_eq!(u16_at(&bytes, sections_at), 1);
        assert_eq!(u16_at(&bytes, sections_at + 2), 1, "quart y 5 is in section 1");
        let indices_at = sections_at + 4;
        assert_eq!(u16_at(&bytes, indices_at + 22 * 2), 0);
        assert_eq!(u16_at(&bytes, indices_at + 23 * 2), 1);
        assert_eq!(u16_at(&bytes, indices_at + 128), 0, "no block sections");
    }

    #[test]
    fn a_column_only_fill_uses_the_sixteen_index_shortcut() {
        let mut fill = Fill::new();
        for qz in 0..4 {
            for qx in 0..4 {
                fill.biome_column(qx, qz, if (qx + qz) % 2 == 0 { "minecraft:plains" } else { "minecraft:desert" });
            }
        }
        let bytes = fill.to_bytes();
        assert_eq!(bytes[0..2], [2, 2]);
        let columns_at = 2 + 2 + 2 * (2 + "minecraft:plains".len());
        for i in 0..16 {
            assert_eq!(u16_at(&bytes, columns_at + i * 2), ((i % 4 + i / 4) % 2) as u16, "a checkerboard of (qx + qz) % 2");
        }
        assert_eq!(u16_at(&bytes, columns_at + 32), 0, "no quart sections");
        assert_eq!(u16_at(&bytes, columns_at + 34), 0, "no block sections");
        assert_eq!(bytes.len(), columns_at + 36);
    }
}
