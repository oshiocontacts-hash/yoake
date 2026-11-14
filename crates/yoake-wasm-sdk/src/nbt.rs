//! NBT for guests: the `nbt` type of `yoake-full.wit` is a `list<u8>` holding the network
//! encoding of one tag (a tag id byte, then the payload, no root name; big-endian; strings in
//! Java's modified UTF-8). [`parse`] and [`write`] convert it to and from a [`Tag`] tree;
//! [`Compound`] keeps its entries in insertion order.
//!
//! Lists are homogeneous, as in vanilla. Horizon's own wrapper encoding for mixed lists reads
//! back here as a list of compounds, which writes out unchanged.

use alloc::{string::String, vec::Vec};
use core::fmt;

/// How deep [`parse`] follows nested lists and compounds.
pub const MAX_DEPTH: usize = 64;

pub const END: u8 = 0;
pub const BYTE: u8 = 1;
pub const SHORT: u8 = 2;
pub const INT: u8 = 3;
pub const LONG: u8 = 4;
pub const FLOAT: u8 = 5;
pub const DOUBLE: u8 = 6;
pub const BYTE_ARRAY: u8 = 7;
pub const STRING: u8 = 8;
pub const LIST: u8 = 9;
pub const COMPOUND: u8 = 10;
pub const INT_ARRAY: u8 = 11;
pub const LONG_ARRAY: u8 = 12;

/// One NBT value.
#[derive(Clone, Debug, PartialEq)]
pub enum Tag {
    Byte(i8),
    Short(i16),
    Int(i32),
    Long(i64),
    Float(f32),
    Double(f64),
    ByteArray(Vec<i8>),
    String(String),
    /// The elements share one tag id; an empty list has none.
    List(Vec<Tag>),
    Compound(Compound),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
}

/// A compound: named tags in the order they were added or read.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound {
    entries: Vec<(String, Tag)>,
}

impl Compound {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn get(&self, key: &str) -> Option<&Tag> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut Tag> {
        self.entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Sets `key`, keeping the place of an existing entry; returns the replaced tag.
    pub fn insert(&mut self, key: impl Into<String>, tag: Tag) -> Option<Tag> {
        let key = key.into();
        match self.entries.iter_mut().find(|(k, _)| *k == key) {
            Some((_, slot)) => Some(core::mem::replace(slot, tag)),
            None => {
                self.entries.push((key, tag));
                None
            }
        }
    }

    pub fn remove(&mut self, key: &str) -> Option<Tag> {
        let at = self.entries.iter().position(|(k, _)| k == key)?;
        Some(self.entries.remove(at).1)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Tag)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Builder form of [`Compound::insert`].
    pub fn with(mut self, key: impl Into<String>, tag: Tag) -> Self {
        self.insert(key, tag);
        self
    }

    /// The compound for a guest-facing `nbt` value; empty bytes are the empty compound.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.is_empty() {
            return Ok(Self::new());
        }
        match parse(bytes)? {
            Tag::Compound(compound) => Ok(compound),
            _ => Err(Error::NotACompound),
        }
    }

    /// This compound as an `nbt` value.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.push(COMPOUND);
        write_compound(&mut out, self);
        out
    }
}

impl Tag {
    /// The tag id of this value.
    pub fn id(&self) -> u8 {
        match self {
            Tag::Byte(_) => BYTE,
            Tag::Short(_) => SHORT,
            Tag::Int(_) => INT,
            Tag::Long(_) => LONG,
            Tag::Float(_) => FLOAT,
            Tag::Double(_) => DOUBLE,
            Tag::ByteArray(_) => BYTE_ARRAY,
            Tag::String(_) => STRING,
            Tag::List(_) => LIST,
            Tag::Compound(_) => COMPOUND,
            Tag::IntArray(_) => INT_ARRAY,
            Tag::LongArray(_) => LONG_ARRAY,
        }
    }

    pub fn as_compound(&self) -> Option<&Compound> {
        match self {
            Tag::Compound(c) => Some(c),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Tag::String(s) => Some(s),
            _ => None,
        }
    }

    /// Any integer tag as `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Tag::Byte(v) => Some(i64::from(*v)),
            Tag::Short(v) => Some(i64::from(*v)),
            Tag::Int(v) => Some(i64::from(*v)),
            Tag::Long(v) => Some(*v),
            _ => None,
        }
    }

    /// Any numeric tag as `f64`.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Tag::Float(v) => Some(f64::from(*v)),
            Tag::Double(v) => Some(*v),
            other => other.as_i64().map(|v| v as f64),
        }
    }

    /// This tag as an `nbt` value.
    pub fn to_bytes(&self) -> Vec<u8> {
        write(self)
    }
}

/// Why an `nbt` value did not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The bytes end inside a tag.
    Truncated,
    /// A tag id outside 1..=12 where a value was expected.
    BadTag(u8),
    /// A negative length.
    BadLength,
    /// Lists or compounds nest deeper than [`MAX_DEPTH`].
    TooDeep,
    /// Bytes remain after the tag.
    Trailing,
    /// A compound was expected.
    NotACompound,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Truncated => f.write_str("NBT ends inside a tag"),
            Error::BadTag(id) => write!(f, "bad NBT tag id {id}"),
            Error::BadLength => f.write_str("negative NBT length"),
            Error::TooDeep => f.write_str("NBT nests too deeply"),
            Error::Trailing => f.write_str("bytes after the NBT tag"),
            Error::NotACompound => f.write_str("expected a compound tag"),
        }
    }
}

// --------------------------------------------------------------------------------------------
// Reading

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.at.checked_add(n).ok_or(Error::Truncated)?;
        let slice = self.bytes.get(self.at..end).ok_or(Error::Truncated)?;
        self.at = end;
        Ok(slice)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Error> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn length(&mut self) -> Result<usize, Error> {
        let n = i32::from_be_bytes(self.array()?);
        usize::try_from(n).map_err(|_| Error::BadLength)
    }

    fn string(&mut self) -> Result<String, Error> {
        let n = usize::from(u16::from_be_bytes(self.array()?));
        Ok(decode_modified_utf8(self.take(n)?))
    }

    fn payload(&mut self, id: u8, depth: usize) -> Result<Tag, Error> {
        Ok(match id {
            BYTE => Tag::Byte(i8::from_be_bytes(self.array()?)),
            SHORT => Tag::Short(i16::from_be_bytes(self.array()?)),
            INT => Tag::Int(i32::from_be_bytes(self.array()?)),
            LONG => Tag::Long(i64::from_be_bytes(self.array()?)),
            FLOAT => Tag::Float(f32::from_be_bytes(self.array()?)),
            DOUBLE => Tag::Double(f64::from_be_bytes(self.array()?)),
            BYTE_ARRAY => {
                let n = self.length()?;
                Tag::ByteArray(self.take(n)?.iter().map(|b| *b as i8).collect())
            }
            STRING => Tag::String(self.string()?),
            LIST => {
                if depth >= MAX_DEPTH {
                    return Err(Error::TooDeep);
                }
                let element = self.array::<1>()?[0];
                let n = self.length()?;
                if n == 0 {
                    return Ok(Tag::List(Vec::new()));
                }
                if !(BYTE..=LONG_ARRAY).contains(&element) {
                    return Err(Error::BadTag(element));
                }
                // The capacity is bounded by the input, so a huge claimed length costs nothing.
                let mut items = Vec::with_capacity(n.min(self.bytes.len() - self.at));
                for _ in 0..n {
                    items.push(self.payload(element, depth + 1)?);
                }
                Tag::List(items)
            }
            COMPOUND => {
                if depth >= MAX_DEPTH {
                    return Err(Error::TooDeep);
                }
                let mut compound = Compound::new();
                loop {
                    let id = self.array::<1>()?[0];
                    if id == END {
                        break;
                    }
                    if id > LONG_ARRAY {
                        return Err(Error::BadTag(id));
                    }
                    let key = self.string()?;
                    let value = self.payload(id, depth + 1)?;
                    compound.insert(key, value);
                }
                Tag::Compound(compound)
            }
            INT_ARRAY => {
                let n = self.length()?;
                let bytes = self.take(n.checked_mul(4).ok_or(Error::BadLength)?)?;
                Tag::IntArray(bytes.chunks_exact(4).map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect())
            }
            LONG_ARRAY => {
                let n = self.length()?;
                let bytes = self.take(n.checked_mul(8).ok_or(Error::BadLength)?)?;
                Tag::LongArray(
                    bytes
                        .chunks_exact(8)
                        .map(|c| i64::from_be_bytes([c[0], c[1], c[2], c[3], c[4], c[5], c[6], c[7]]))
                        .collect(),
                )
            }
            other => return Err(Error::BadTag(other)),
        })
    }
}

/// Java's modified UTF-8: NUL as C0 80 and supplementary characters as surrogate pairs.
/// Unpaired surrogates (which a Java string may hold) read as U+FFFD.
fn decode_modified_utf8(bytes: &[u8]) -> String {
    let mut units: Vec<u16> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let (unit, used) = if b & 0x80 == 0 {
            (u16::from(b), 1)
        } else if b & 0xE0 == 0xC0 && i + 1 < bytes.len() {
            ((u16::from(b & 0x1F) << 6) | u16::from(bytes[i + 1] & 0x3F), 2)
        } else if b & 0xF0 == 0xE0 && i + 2 < bytes.len() {
            ((u16::from(b & 0x0F) << 12) | (u16::from(bytes[i + 1] & 0x3F) << 6) | u16::from(bytes[i + 2] & 0x3F), 3)
        } else {
            (0xFFFD, 1)
        };
        units.push(unit);
        i += used;
    }
    char::decode_utf16(units).map(|c| c.unwrap_or('\u{FFFD}')).collect()
}

/// Parses an `nbt` value: one tag, nothing after it.
pub fn parse(bytes: &[u8]) -> Result<Tag, Error> {
    let mut reader = Reader { bytes, at: 0 };
    let id = reader.array::<1>()?[0];
    let tag = reader.payload(id, 0)?;
    if reader.at != bytes.len() {
        return Err(Error::Trailing);
    }
    Ok(tag)
}

// --------------------------------------------------------------------------------------------
// Writing

fn encode_modified_utf8(text: &str, out: &mut Vec<u8>) {
    let start = out.len();
    out.extend([0, 0]);
    for unit in text.encode_utf16() {
        match unit {
            1..=0x7F => out.push(unit as u8),
            0 | 0x80..=0x7FF => out.extend([0xC0 | (unit >> 6) as u8, 0x80 | (unit & 0x3F) as u8]),
            _ => out.extend([0xE0 | (unit >> 12) as u8, 0x80 | ((unit >> 6) & 0x3F) as u8, 0x80 | (unit & 0x3F) as u8]),
        }
    }
    // A string longer than 65535 bytes cannot be written: it is cut at a character boundary.
    let mut len = out.len() - start - 2;
    if len > usize::from(u16::MAX) {
        let mut cut = start + 2 + usize::from(u16::MAX);
        while out[cut] & 0xC0 == 0x80 {
            cut -= 1;
        }
        out.truncate(cut);
        len = cut - start - 2;
    }
    out[start..start + 2].copy_from_slice(&(len as u16).to_be_bytes());
}

fn write_payload(out: &mut Vec<u8>, tag: &Tag) {
    match tag {
        Tag::Byte(v) => out.push(*v as u8),
        Tag::Short(v) => out.extend(v.to_be_bytes()),
        Tag::Int(v) => out.extend(v.to_be_bytes()),
        Tag::Long(v) => out.extend(v.to_be_bytes()),
        Tag::Float(v) => out.extend(v.to_be_bytes()),
        Tag::Double(v) => out.extend(v.to_be_bytes()),
        Tag::ByteArray(v) => {
            out.extend((v.len() as i32).to_be_bytes());
            out.extend(v.iter().map(|b| *b as u8));
        }
        Tag::String(v) => encode_modified_utf8(v, out),
        Tag::List(items) => {
            out.push(items.first().map_or(END, Tag::id));
            out.extend((items.len() as i32).to_be_bytes());
            for item in items {
                write_payload(out, item);
            }
        }
        Tag::Compound(c) => write_compound(out, c),
        Tag::IntArray(v) => {
            out.extend((v.len() as i32).to_be_bytes());
            for n in v {
                out.extend(n.to_be_bytes());
            }
        }
        Tag::LongArray(v) => {
            out.extend((v.len() as i32).to_be_bytes());
            for n in v {
                out.extend(n.to_be_bytes());
            }
        }
    }
}

fn write_compound(out: &mut Vec<u8>, compound: &Compound) {
    for (key, value) in &compound.entries {
        out.push(value.id());
        encode_modified_utf8(key, out);
        write_payload(out, value);
    }
    out.push(END);
}

/// Writes a tag as an `nbt` value: the tag id, then the payload.
pub fn write(tag: &Tag) -> Vec<u8> {
    let mut out = Vec::new();
    out.push(tag.id());
    write_payload(&mut out, tag);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::{string::ToString, vec};

    fn sample() -> Tag {
        Tag::Compound(
            Compound::new()
                .with("byte", Tag::Byte(-1))
                .with("short", Tag::Short(300))
                .with("int", Tag::Int(-70000))
                .with("long", Tag::Long(1 << 40))
                .with("float", Tag::Float(1.5))
                .with("double", Tag::Double(-2.25))
                .with("bytes", Tag::ByteArray(vec![1, -2, 3]))
                .with("text", Tag::String("hi \u{e9} \u{1F600} \0".to_string()))
                .with("list", Tag::List(vec![Tag::Int(1), Tag::Int(2)]))
                .with("compounds", Tag::List(vec![Tag::Compound(Compound::new().with("a", Tag::Byte(1)))]))
                .with("empty", Tag::List(Vec::new()))
                .with("ints", Tag::IntArray(vec![1, -2, 3]))
                .with("longs", Tag::LongArray(vec![i64::MIN, 0, i64::MAX])),
        )
    }

    #[test]
    fn every_kind_round_trips() {
        let tag = sample();
        let bytes = write(&tag);
        assert_eq!(bytes[0], COMPOUND);
        assert_eq!(parse(&bytes).unwrap(), tag);
    }

    #[test]
    fn compound_keeps_insertion_order_and_replaces_in_place() {
        let mut c = Compound::new().with("z", Tag::Byte(1)).with("a", Tag::Byte(2));
        c.insert("z", Tag::Byte(9));
        let keys: Vec<&str> = c.iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["z", "a"]);
        let back = Compound::from_bytes(&c.to_bytes()).unwrap();
        assert_eq!(back, c);
        assert_eq!(back.get("z").and_then(Tag::as_i64), Some(9));
        assert_eq!(c.remove("a"), Some(Tag::Byte(2)));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn empty_bytes_are_an_empty_compound() {
        assert!(Compound::from_bytes(&[]).unwrap().is_empty());
        assert_eq!(Compound::from_bytes(&[3, 0, 0, 0, 1]), Err(Error::NotACompound));
    }

    #[test]
    fn known_bytes_match_the_wire_format() {
        // {"a": 1b} in the network encoding.
        let tag = Tag::Compound(Compound::new().with("a", Tag::Byte(1)));
        assert_eq!(write(&tag), [10, 1, 0, 1, b'a', 1, 0]);
        // Java's modified UTF-8: NUL is C0 80, U+1F600 is a surrogate pair of two 3-byte units.
        let nul = write(&Tag::String("\0".to_string()));
        assert_eq!(nul, [8, 0, 2, 0xC0, 0x80]);
        let emoji = write(&Tag::String("\u{1F600}".to_string()));
        assert_eq!(emoji, [8, 0, 6, 0xED, 0xA0, 0xBD, 0xED, 0xB8, 0x80]);
        assert_eq!(parse(&emoji).unwrap(), Tag::String("\u{1F600}".to_string()));
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        assert_eq!(parse(&[]), Err(Error::Truncated));
        assert_eq!(parse(&[10, 1, 0, 1, b'a']), Err(Error::Truncated));
        assert_eq!(parse(&[3, 0, 0, 0]), Err(Error::Truncated));
        assert_eq!(parse(&[3, 0, 0, 0, 1, 7]), Err(Error::Trailing));
        assert_eq!(parse(&[99]), Err(Error::BadTag(99)));
        assert_eq!(parse(&[7, 0xFF, 0xFF, 0xFF, 0xFF]), Err(Error::BadLength));
        // A list claiming a billion elements with no data behind it.
        assert_eq!(parse(&[9, 3, 0x3B, 0x9A, 0xCA, 0x00]), Err(Error::Truncated));
        // Lists of lists of lists, 200 deep.
        let mut deep = vec![9];
        for _ in 0..200 {
            deep.extend([9, 0, 0, 0, 1]);
        }
        assert_eq!(parse(&deep), Err(Error::TooDeep));
    }
}
