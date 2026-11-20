//! A WebAssembly package as the chunk generator of a level (F3-8).
//!
//! A package registers a generator by id (`generator-register`, from `init`) and a level whose stem is
//! `{"type": ..., "generator": {"type": "yoake:mod", "id": "<generator id>", "settings": {...}}}`. When
//! that level generates a chunk, the engine calls the package's `handle-call` export, on the worker that
//! generates the chunk, with the caller `yoake:engine` and the method [`METHOD`]:
//!
//! * the request is a JSON object: `level` (the level's id), `generator`, `x` and `z` (the chunk), `min_y`
//!   and `height` (the level's build range), `seed` and `settings` (the stem's object);
//! * the answer is the binary fill below, little endian. `str` is a `u16` length and UTF-8 bytes.
//!
//! ```text
//! u8   version = 1
//! u8   flags            bit 0: a biome for the whole chunk follows
//! str  biome            (only with the flag) `namespace:path`
//! u16  sections         how many sections follow
//! each section:
//!   u16 index           from the bottom of the level, 0 .. height / 16
//!   u8  kind            0: one block, 1: a palette
//!   kind 0: str block   `namespace:path` (its default state)
//!   kind 1: u16 n (1..=4096), n * str block, then 4096 * u16 palette indices,
//!           index = (y * 16 + z) * 16 + x inside the section
//! ```
//!
//! Sections not mentioned stay air. The whole answer is checked before anything is written: a bad version,
//! a section out of range or given twice, an unknown block, a palette index past the palette or bytes left
//! over fail the chunk. A failed call, an answer that does not check or a guest that ran out of its limits
//! leaves the chunk empty (air) and is logged once for the level; the guest is then disabled when it
//! trapped or ran out of its limits, as for any hook. The guest's own limits are the budget of a
//! `handle-call`; the generating thread waits up to two seconds for the module when a hook holds it.
use super::*;
use horizon_client::mod_generator::{ModChunk, ModColumnSource, ModGeneratorContext, ModGeneratorFactory};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The method a generating package's `handle-call` is called with.
pub(crate) const METHOD: &str = "yoake:generate-chunk";
/// Who the package is told is calling.
const CALLER: &str = "yoake:engine";
/// How long a generating thread waits for a module a hook is running.
const LOCK_WAIT: Duration = Duration::from_secs(2);
/// A call slower than this is logged (it still counts: the guest's limits cut it off).
const SLOW: Duration = Duration::from_millis(100);

/// The factory for generator ids package `mod_id` registered.
pub(crate) fn generator_factory(mod_id: &str) -> Arc<dyn ModGeneratorFactory> {
    Arc::new(WasmGeneratorFactory { mod_id: mod_id.to_owned() })
}

struct WasmGeneratorFactory {
    mod_id: String,
}

impl ModGeneratorFactory for WasmGeneratorFactory {
    fn create(&self, c: &ModGeneratorContext) -> Result<Arc<dyn ModColumnSource>, String> {
        let slot = super::server_services::endpoint(&self.mod_id)
            .ok_or_else(|| format!("package {:.64} has no running server module", self.mod_id))?;
        Ok(Arc::new(WasmColumnSource {
            slot,
            mod_id: self.mod_id.clone(),
            level: c.level.clone(),
            generator: c.generator.clone(),
            seed: c.seed,
            min_y: c.shape.min_y,
            height: c.shape.height,
            settings: c.settings.clone(),
            block: c.block_lookup(),
            warned: AtomicBool::new(false),
        }))
    }
}

struct WasmColumnSource {
    slot: Arc<super::server_services::ServerSlot>,
    mod_id: String,
    level: String,
    generator: String,
    seed: i64,
    min_y: i32,
    height: i32,
    settings: serde_json::Value,
    block: Arc<dyn Fn(&str) -> Option<u32> + Send + Sync>,
    warned: AtomicBool,
}

impl WasmColumnSource {
    fn request(&self, x: i32, z: i32) -> Vec<u8> {
        serde_json::json!({
            "level": self.level, "generator": self.generator, "x": x, "z": z,
            "min_y": self.min_y, "height": self.height, "seed": self.seed, "settings": self.settings,
        })
        .to_string()
        .into_bytes()
    }

    /// The package's answer for the chunk, or why there is none.
    fn call(&self, request: &[u8]) -> Result<Vec<u8>, String> {
        let started = Instant::now();
        let mut state = loop {
            match self.slot.state.try_lock() {
                Ok(state) => break state,
                Err(std::sync::TryLockError::Poisoned(e)) => break e.into_inner(),
                Err(std::sync::TryLockError::WouldBlock) => {
                    if started.elapsed() > LOCK_WAIT {
                        return Err(format!("{:.64} was busy for {LOCK_WAIT:?}", self.mod_id));
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        };
        let Some(module) = state.module.as_mut() else {
            return Err(format!("{:.64} has no running server module", self.mod_id));
        };
        self.slot.dirty.store(true, Ordering::Relaxed);
        let mut inner = Invocation { host: HostAccess::None, batch: None, batch_start: 0, client: ClientExtras::default() };
        let answer = {
            let _scope = Scope::enter(&mut inner);
            module.handle_generator_call(CALLER, METHOD, request)
        };
        let answer = match answer {
            None => Err(format!("{:.64} did not answer (disabled, trapped or over its limits)", self.mod_id)),
            Some(Ok(bytes)) if bytes.len() > super::server_services::MAX_BLOB => Err("the answer is larger than 1 MiB".to_owned()),
            Some(answer) => answer,
        };
        if started.elapsed() > SLOW {
            log::warn!("wasm mod {} took {:?} to generate a chunk of {}", self.mod_id, started.elapsed(), self.level);
        }
        answer
    }
}

impl ModColumnSource for WasmColumnSource {
    fn generate(&self, chunk: &mut ModChunk<'_>) {
        let (x, z) = (chunk.chunk_x, chunk.chunk_z);
        let result = self
            .call(&self.request(x, z))
            .and_then(|bytes| Fill::decode(&bytes, self.height, &*self.block));
        match result {
            Ok(fill) => fill.apply(self.min_y, chunk),
            Err(why) => {
                if !self.warned.swap(true, Ordering::Relaxed) {
                    log::error!(
                        "wasm generator {} of level {}: chunk {x} {z} is left empty: {why} (further chunks of this level are logged at debug level)",
                        self.generator, self.level
                    );
                } else {
                    log::debug!("wasm generator {} of level {}: chunk {x} {z} left empty: {why}", self.generator, self.level);
                }
            }
        }
    }
}

/// One section of a fill: a block, or a palette and its 4096 indices.
#[derive(Debug, PartialEq)]
enum Blocks {
    One(u32),
    Palette(Vec<u32>, Vec<u16>),
}

/// A decoded and checked answer.
#[derive(Debug, PartialEq)]
pub(super) struct Fill {
    biome: Option<String>,
    sections: Vec<(u16, Blocks)>,
}

struct Cursor<'a>(&'a [u8]);
impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        if self.0.len() < n {
            return Err("the answer ends early".into());
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn str(&mut self) -> Result<&'a str, String> {
        let n = usize::from(self.u16()?);
        std::str::from_utf8(self.take(n)?).map_err(|_| "a name is not UTF-8".to_owned())
    }
}

impl Fill {
    /// Checks and decodes an answer; `blocks` gives a block's default state (None: unknown).
    pub(super) fn decode(
        bytes: &[u8],
        height: i32,
        blocks: &dyn Fn(&str) -> Option<u32>,
    ) -> Result<Self, String> {
        let mut c = Cursor(bytes);
        if c.u8()? != 1 {
            return Err("unknown fill version".into());
        }
        let flags = c.u8()?;
        if flags & !1 != 0 {
            return Err("unknown fill flags".into());
        }
        let biome = (flags & 1 != 0).then(|| c.str().map(str::to_owned)).transpose()?;
        let count = usize::from(c.u16()?);
        let limit = usize::try_from(height / 16).unwrap_or(0);
        if count > limit {
            return Err(format!("{count} sections for a level of {limit}"));
        }
        let state = |name: &str| blocks(name).ok_or_else(|| format!("unknown block {name:.64}"));
        let mut seen = vec![false; limit];
        let mut sections = Vec::with_capacity(count);
        for _ in 0..count {
            let index = c.u16()?;
            let slot = seen.get_mut(usize::from(index)).ok_or_else(|| format!("section {index} is outside the level"))?;
            if std::mem::replace(slot, true) {
                return Err(format!("section {index} is given twice"));
            }
            let section = match c.u8()? {
                0 => Blocks::One(state(c.str()?)?),
                1 => {
                    let n = usize::from(c.u16()?);
                    if n == 0 || n > 4096 {
                        return Err(format!("a palette of {n}"));
                    }
                    let palette = (0..n).map(|_| state(c.str()?)).collect::<Result<Vec<_>, _>>()?;
                    let raw = c.take(4096 * 2)?;
                    let indices: Vec<u16> = raw.chunks_exact(2).map(|b| u16::from_le_bytes([b[0], b[1]])).collect();
                    if indices.iter().any(|&i| usize::from(i) >= n) {
                        return Err("a palette index is past the palette".into());
                    }
                    Blocks::Palette(palette, indices)
                }
                other => return Err(format!("unknown section kind {other}")),
            };
            sections.push((index, section));
        }
        if !c.0.is_empty() {
            return Err("bytes left over after the sections".into());
        }
        Ok(Self { biome, sections })
    }

    fn apply(&self, min_y: i32, chunk: &mut ModChunk<'_>) {
        if let Some(biome) = &self.biome
            && !chunk.set_biome(biome)
        {
            log::debug!("a generator named the unknown biome {biome:.64}");
        }
        for (index, blocks) in &self.sections {
            let bottom = min_y + i32::from(*index) * 16;
            match blocks {
                Blocks::One(0) => {}
                Blocks::One(state) => {
                    for y in 0..16 {
                        chunk.fill_layer(bottom + y, *state);
                    }
                }
                Blocks::Palette(palette, indices) => {
                    for (i, &p) in indices.iter().enumerate() {
                        let state = palette[usize::from(p)];
                        if state != 0 {
                            let (x, z, y) = ((i & 15) as i32, ((i >> 4) & 15) as i32, (i >> 8) as i32);
                            chunk.set_block(x, bottom + y, z, state);
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod service_tests {
    use super::super::full::GameServerHost;
    use yoake_wasm::ServerHost as _;

    fn host() -> GameServerHost {
        GameServerHost { id: "levels-test".to_owned(), commands: false }
    }

    /// What a guest passes is checked before the game sees it, and the services that need the server's hook say so
    /// outside one (nothing here has a world).
    #[test]
    fn the_level_services_check_what_a_guest_passes() {
        let mut host = host();
        for bad in ["", "nocolon", "A:upper", "a:../x", "a: space"] {
            assert!(host.level_create(bad.to_owned(), true, "{}".to_owned()).is_err(), "{bad}");
            assert!(host.level_delete(bad.to_owned()).is_err());
            assert!(host.generator_register(bad.to_owned()).is_err());
        }
        // A stem or a type must be a JSON object, and not huge.
        assert!(host.level_create("a:b".into(), true, "[]".into()).unwrap_err().contains("not a JSON object"));
        assert!(host.level_create("a:b".into(), true, "{".into()).unwrap_err().contains("level stem"));
        assert!(host.level_register_type("a:b".into(), "3".into()).unwrap_err().contains("not a JSON object"));
        let big = format!("{{\"x\": \"{}\"}}", "y".repeat(1 << 16));
        assert!(host.level_create("a:b".into(), true, big).unwrap_err().contains("64 KiB"));
        // A type must decode as a dimension type (a height that is no multiple of 16 does not).
        assert!(host.level_register_type("a:t".into(), "{\"min_y\": 1, \"height\": 7}".into()).is_err());
        // Outside a server hook the rest answer so, and change nothing.
        for answer in [
            host.level_create("a:b".into(), true, "{}".into()),
            host.level_delete("a:b".into()),
            host.level_unload("a:b".into()),
            host.level_regenerate("a:b".into(), Some(1)),
            host.level_register("a:b".into(), "{}".into()),
            host.generator_register("a:gen".into()),
        ] {
            assert!(answer.unwrap_err().contains("unsupported"), "not inside a server hook");
        }
        assert!(host.level_list().unwrap_err().contains("unsupported"));
        assert!(horizon_client::mod_generator::staged().0.is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(name: &str) -> Option<u32> {
        match name {
            "minecraft:air" => Some(0),
            "minecraft:stone" => Some(1),
            "minecraft:dirt" => Some(2),
            _ => None,
        }
    }
    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u16).to_le_bytes());
        out.extend(s.as_bytes());
    }
    /// A fill with a uniform stone section 0, a palette section 1 (dirt on stone) and a biome.
    fn fill() -> Vec<u8> {
        let mut b = vec![1, 1];
        string(&mut b, "minecraft:plains");
        b.extend(2u16.to_le_bytes());
        b.extend(0u16.to_le_bytes());
        b.push(0);
        string(&mut b, "minecraft:stone");
        b.extend(1u16.to_le_bytes());
        b.push(1);
        b.extend(2u16.to_le_bytes());
        string(&mut b, "minecraft:stone");
        string(&mut b, "minecraft:dirt");
        for i in 0..4096u16 {
            b.extend(u16::from(i >> 8 == 3).to_le_bytes());
        }
        b
    }

    #[test]
    fn a_good_answer_decodes() {
        let fill = Fill::decode(&fill(), 64, &block).unwrap();
        assert_eq!(fill.biome.as_deref(), Some("minecraft:plains"));
        assert_eq!(fill.sections.len(), 2);
        assert_eq!(fill.sections[0], (0, Blocks::One(1)));
        let Blocks::Palette(p, i) = &fill.sections[1].1 else { panic!() };
        assert_eq!((p.as_slice(), i.len(), i[3 * 256], i[0]), (&[1, 2][..], 4096, 1, 0));
    }

    /// The fuzz the plan asks for: bad palette index, wrong section count, truncation at every length, junk.
    #[test]
    fn bad_answers_are_refused_whole() {
        let good = fill();
        assert!(Fill::decode(&good, 64, &block).is_ok());
        // Every truncation fails (and none panics).
        for n in 0..good.len() {
            assert!(Fill::decode(&good[..n], 64, &block).is_err(), "cut at {n}");
        }
        // Bytes left over.
        let mut more = good.clone();
        more.push(0);
        assert!(Fill::decode(&more, 64, &block).unwrap_err().contains("left over"));
        // A palette index past the palette.
        let mut bad = good.clone();
        let at = bad.len() - 2;
        bad[at..].copy_from_slice(&5u16.to_le_bytes());
        assert!(Fill::decode(&bad, 64, &block).unwrap_err().contains("past the palette"));
        // More sections than the level has (height 16: one section).
        assert!(Fill::decode(&good, 16, &block).unwrap_err().contains("sections for a level"));
        // A section outside the level, and one given twice.
        let mut outside = good.clone();
        let first = 2 + 2 + "minecraft:plains".len() + 2;
        outside[first..first + 2].copy_from_slice(&9u16.to_le_bytes());
        assert!(Fill::decode(&outside, 64, &block).unwrap_err().contains("outside the level"));
        let mut twice = good.clone();
        let second = first + 2 + 1 + 2 + "minecraft:stone".len();
        twice[second..second + 2].copy_from_slice(&0u16.to_le_bytes());
        assert!(Fill::decode(&twice, 64, &block).unwrap_err().contains("twice"));
        // An unknown block, version, flags and kind.
        let mut stone = Vec::new();
        string(&mut stone, "minecraft:bedrock");
        let unknown = [vec![1, 0, 1, 0, 0, 0, 0], stone.clone()].concat();
        assert!(Fill::decode(&unknown, 64, &block).unwrap_err().contains("unknown block"));
        assert!(Fill::decode(&[2, 0, 0, 0], 64, &block).unwrap_err().contains("version"));
        assert!(Fill::decode(&[1, 2, 0, 0], 64, &block).unwrap_err().contains("flags"));
        assert!(Fill::decode(&[1, 0, 1, 0, 0, 0, 7], 64, &block).unwrap_err().contains("kind"));
        // A palette of none, and of too many.
        assert!(Fill::decode(&[1, 0, 1, 0, 0, 0, 1, 0, 0], 64, &block).unwrap_err().contains("palette of 0"));
        assert!(Fill::decode(&[1, 0, 1, 0, 0, 0, 1, 0xff, 0xff], 64, &block).unwrap_err().contains("palette of"));
        // Anything at all: no panic.
        let mut seed = 0x1234_5678u32;
        for _ in 0..2000 {
            let junk: Vec<u8> = (0..(seed % 64) as usize)
                .map(|_| {
                    seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                    (seed >> 24) as u8
                })
                .collect();
            let _ = Fill::decode(&junk, 384, &block);
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        }
    }
}
