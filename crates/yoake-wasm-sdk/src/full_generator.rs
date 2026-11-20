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

/// The answer: blocks by section (16 high, from the bottom of the level) and, if wanted, one biome for
/// the whole chunk. A section not given stays air.
#[derive(Default)]
pub struct Fill {
    biome: Option<String>,
    sections: Vec<(u16, Blocks)>,
}

impl Fill {
    pub fn new() -> Self {
        Self::default()
    }

    /// The biome of the whole chunk (`namespace:path`).
    pub fn biome(&mut self, biome: &str) -> &mut Self {
        self.biome = Some(biome.to_owned());
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
        let mut out = vec![1, u8::from(self.biome.is_some())];
        if let Some(biome) = &self.biome {
            string(&mut out, biome);
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
