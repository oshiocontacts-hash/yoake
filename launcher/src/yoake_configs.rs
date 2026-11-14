//! Yoake mods catalog (built-in + .yak) and user configurations shared by offline `YK1-` codes.
//!
//! * The signed built-in catalog (`yoake/mods/builtin/signed.json`, domain `yoake-builtin-v1`) lists the
//!   Yoake mods that are compiled into game builds. It is a separate document from the strict `.yak`
//!   catalog, so launchers that only know that one never read it.
//! * [`merge`] puts both lists together; a `.yak` with the same id wins over the built-in entry.
//! * A [`Configuration`] is a name, a base pack id and mod ids with versions. [`encode`]/[`decode`]
//!   turn it into a share code and back; [`resolve`] checks every id against the catalog.
//!
//! The code format is documented in `docs/yoake-configs.md`.

use crate::yoake_feed::localized;
use horizon_mods::yak::{Catalog, PublicKey, Version, VersionReq};
use horizon_release_sign::{YOAKE_BUILTIN_V1, verify_bytes};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// One file holding the catalog text and its signature, so a publish is a single atomic rename.
pub const BUILTIN_PATH: &str = "yoake/mods/builtin/signed.json";
pub const MAX_BUILTIN: usize = 1024 * 1024;
pub const CODE_PREFIX: &str = "YK1-";
/// A reserved prefix for a future short code that the portal resolves (never decoded here).
pub const SERVER_CODE_PREFIX: &str = "YK2-";
pub const MAX_DECODED: usize = 4096;
/// How much the paste box takes: the longest code `encode` can make (about 8 KiB with its
/// dashes) with room for pasted line breaks and spaces.
pub const MAX_CODE_INPUT: usize = 16 * 1024;
pub const MAX_MODS: usize = 64;
pub const LATEST: &str = "latest";
const MAX_SAVED: usize = 64;
static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
const BUILTIN_KIND: &str = "yoake-builtin-catalog";
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    NotACode,
    ServerCodeUnsupported,
    BadEncoding,
    Checksum,
    TooLarge,
    Malformed,
    Invalid(String),
    NeedsUpdate,
    BadSignature,
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotACode => "This is not a Yoake configuration code",
            Self::ServerCodeUnsupported => "Short codes (YK2-) are not supported yet",
            Self::BadEncoding => "The code contains characters that are not allowed",
            Self::Checksum => "The code is damaged: its checksum does not match",
            Self::TooLarge => "The code is too large",
            Self::Malformed => "The code does not describe a configuration",
            Self::Invalid(reason) => reason,
            Self::NeedsUpdate => "This catalog needs a newer Yoake launcher",
            Self::BadSignature => "The built-in mods catalog signature is invalid",
        })
    }
}

impl std::error::Error for Error {}

// ---- catalog ---------------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
pub struct BuiltinMod {
    pub id: String,
    pub version: String,
    pub author: String,
    pub name: BTreeMap<String, String>,
    pub description: BTreeMap<String, String>,
    #[serde(default)]
    pub requires: Vec<String>,
    #[serde(default)]
    pub requires_engine: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BuiltinCatalog {
    schema: String,
    kind: String,
    /// `YYYY-MM-DDTHH:MM:SSZ`; a launcher never takes a catalog older than the one it has cached.
    pub generated: String,
    /// Features a launcher must understand to read this catalog. A new security-relevant field
    /// has to be listed here too (docs/yoake-configs.md), because unknown fields are ignored.
    #[serde(default)]
    critical_features: Vec<String>,
    pub mods: Vec<BuiltinMod>,
}

/// `YYYY-MM-DDTHH:MM:SSZ`, which also sorts in time order as text.
fn valid_generated(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 20
        && bytes.iter().enumerate().all(|(position, byte)| match position {
            4 | 7 => *byte == b'-',
            10 => *byte == b'T',
            13 | 16 => *byte == b':',
            19 => *byte == b'Z',
            _ => byte.is_ascii_digit(),
        })
}

fn valid_engine_version(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+' | b'_'))
}

impl BuiltinCatalog {
    /// Verifies the exact downloaded bytes against the Yoake mods signature, then parses them.
    pub fn parse_verified(
        bytes: &[u8],
        signature: &str,
        keys: &[PublicKey],
    ) -> Result<Self, Error> {
        if bytes.len() > MAX_BUILTIN || signature.len() > 4096 {
            return Err(Error::TooLarge);
        }
        verify_bytes(YOAKE_BUILTIN_V1, bytes, signature, keys)
            .map_err(|_| Error::BadSignature)?;
        Self::parse(bytes)
    }

    /// Unknown fields and a higher minor version are accepted; a higher major needs a newer launcher.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        let catalog: Self = serde_json::from_slice(bytes).map_err(|_| Error::Malformed)?;
        let (major, minor) = catalog
            .schema
            .split_once('.')
            .ok_or(Error::Malformed)?;
        let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
        if !digits(major) || !digits(minor) || minor.parse::<u32>().is_err() {
            return Err(Error::Malformed);
        }
        match major.parse::<u32>() {
            Ok(1) => {}
            Ok(major) if major > 1 => return Err(Error::NeedsUpdate),
            _ => return Err(Error::Malformed),
        }
        if !catalog.critical_features.is_empty() {
            return Err(Error::NeedsUpdate);
        }
        if catalog.kind != BUILTIN_KIND
            || catalog.mods.len() > 1024
            || !valid_generated(&catalog.generated)
        {
            return Err(Error::Malformed);
        }
        let mut ids = BTreeSet::new();
        for entry in &catalog.mods {
            if !valid_mod_id(&entry.id)
                || !ids.insert(entry.id.as_str())
                || Version::parse(&entry.version).is_err()
                || entry.name.get("en").is_none_or(String::is_empty)
                || entry.description.get("en").is_none_or(String::is_empty)
                || entry.requires.iter().any(|id| !valid_mod_id(id))
                || entry.requires.len() > MAX_MODS
                || entry.requires_engine.len() > 256
                || entry.requires_engine.iter().any(|base| !valid_engine_version(base))
            {
                return Err(Error::Malformed);
            }
        }
        Ok(catalog)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Compiled into a Yoake game build.
    Builtin,
    /// A `.yak` from the signed catalog.
    Yak,
    /// Reserved for WASM modules (branch yoake/wasm-w1); not produced by the catalog yet.
    Wasm,
}

#[derive(Debug, Clone)]
pub struct CatalogMod {
    pub id: String,
    pub kind: Kind,
    /// Newest version.
    pub version: String,
    /// Every version a configuration may pin (newest first).
    pub versions: Vec<String>,
    pub author: String,
    name: BTreeMap<String, String>,
    description: BTreeMap<String, String>,
    pub requires: Vec<String>,
    /// Per version: the dependency requirements of that `.yak` version (`id` -> semver range).
    /// Empty for built-in mods, which only list ids.
    pub requirements: BTreeMap<String, BTreeMap<String, String>>,
    /// Base versions whose build contains a built-in mod (empty for `.yak`).
    pub requires_engine: Vec<String>,
}

impl CatalogMod {
    pub fn name(&self, language: &str) -> &str {
        localized(&self.name, language)
    }

    pub fn description(&self, language: &str) -> &str {
        localized(&self.description, language)
    }
}

/// Built-in entries plus the `.yak` catalog, sorted by id. A `.yak` with the same id replaces the
/// built-in entry.
pub fn merge(builtin: Option<&BuiltinCatalog>, yak: Option<&Catalog>) -> Vec<CatalogMod> {
    let mut mods: BTreeMap<String, CatalogMod> = BTreeMap::new();
    for entry in builtin.into_iter().flat_map(|catalog| &catalog.mods) {
        mods.insert(
            entry.id.clone(),
            CatalogMod {
                id: entry.id.clone(),
                kind: Kind::Builtin,
                version: entry.version.clone(),
                versions: vec![entry.version.clone()],
                author: entry.author.clone(),
                name: entry.name.clone(),
                description: entry.description.clone(),
                requires: entry.requires.clone(),
                requirements: BTreeMap::new(),
                requires_engine: entry.requires_engine.clone(),
            },
        );
    }
    for entry in yak.into_iter().flat_map(|catalog| &catalog.entries) {
        let mut versions: Vec<_> = entry.versions.iter().collect();
        versions.sort_by(|left, right| right.version.cmp(&left.version));
        let Some(newest) = versions.first() else {
            continue;
        };
        let manifest = &newest.manifest;
        let mut name = BTreeMap::from([("en".to_owned(), manifest.name.clone())]);
        if let Some(ja) = &manifest.name_ja {
            name.insert("ja".to_owned(), ja.clone());
        }
        mods.insert(
            entry.id.clone(),
            CatalogMod {
                id: entry.id.clone(),
                kind: Kind::Yak,
                version: newest.version.to_string(),
                versions: versions.iter().map(|v| v.version.to_string()).collect(),
                author: manifest.authors.join(", "),
                name,
                description: BTreeMap::from([("en".to_owned(), manifest.description.clone())]),
                requires: manifest.dependencies.keys().cloned().collect(),
                requirements: versions
                    .iter()
                    .map(|version| {
                        (
                            version.version.to_string(),
                            version
                                .manifest
                                .dependencies
                                .iter()
                                .map(|(id, range)| (id.clone(), range.to_string()))
                                .collect(),
                        )
                    })
                    .collect(),
                requires_engine: Vec::new(),
            },
        );
    }
    mods.into_values().collect()
}

// ---- configuration ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModRef {
    pub id: String,
    /// `latest` or an exact catalog version.
    pub version: String,
}

/// Unknown fields are ignored (a newer launcher may have saved more); [`save`] keeps them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Configuration {
    pub name: String,
    /// The base pack (engine/build) id the instance starts from.
    pub base: String,
    pub mods: Vec<ModRef>,
    /// Per mod, only keys the mod declares shareable. No mod declares any yet, so this stays empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub settings: BTreeMap<String, BTreeMap<String, String>>,
}

pub fn valid_mod_id(value: &str) -> bool {
    (3..=48).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn valid_pack_id(value: &str) -> bool {
    (1..=48).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-' || b == b'.')
}

fn valid_version(value: &str) -> bool {
    value == LATEST || (value.len() <= 64 && Version::parse(value).is_ok())
}

/// Characters that draw nothing or reorder text: Unicode format characters (Cf: zero-width, bidi
/// controls, soft hyphen, tags, ...), line and paragraph separators, and the blank fillers.
fn is_invisible(character: char) -> bool {
    matches!(character as u32,
        0x00AD | 0x034F | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2
        | 0x115F | 0x1160 | 0x17B4 | 0x17B5 | 0x180B..=0x180F | 0x200B..=0x200F
        | 0x2028..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0
        | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F | 0xE0100..=0xE01EF)
}

fn valid_name(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty()
        && trimmed.chars().count() <= 48
        && !trimmed.chars().any(|c| c.is_control() || is_invisible(c))
}

impl Configuration {
    /// Shape checks only (never the catalog): safe characters, sizes, no duplicates.
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = |reason: &str| Err(Error::Invalid(reason.to_owned()));
        if !valid_name(&self.name) {
            return invalid("The configuration name is empty, too long or has control or invisible characters");
        }
        if !valid_pack_id(&self.base) {
            return invalid("The base id is not valid");
        }
        if self.mods.len() > MAX_MODS {
            return invalid("Too many mods");
        }
        let mut ids = BTreeSet::new();
        for entry in &self.mods {
            if !valid_mod_id(&entry.id) || !valid_version(&entry.version) {
                return invalid("A mod id or version is not valid");
            }
            if !ids.insert(entry.id.as_str()) {
                return invalid("A mod is listed twice");
            }
        }
        for (id, values) in &self.settings {
            if !ids.contains(id.as_str())
                || values.len() > 32
                || values.iter().any(|(key, value)| {
                    key.is_empty() || key.len() > 48 || value.len() > 256
                })
            {
                return invalid("A mod setting is not valid");
            }
        }
        Ok(())
    }
}

// ---- share code -------------------------------------------------------------------------------

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Compact {
    v: u8,
    n: String,
    b: String,
    m: Vec<(String, String)>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    s: BTreeMap<String, BTreeMap<String, String>>,
}

fn checksum(payload: &[u8]) -> [u8; 4] {
    let digest = Sha256::digest(payload);
    [digest[0], digest[1], digest[2], digest[3]]
}

fn base32_encode(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(ALPHABET[((buffer >> bits) & 31) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn base32_decode(text: &str) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for character in text.chars() {
        let upper = character.to_ascii_uppercase();
        // Crockford: O is 0, I and L are 1; no U.
        let upper = match upper {
            'O' => '0',
            'I' | 'L' => '1',
            other => other,
        };
        let value = ALPHABET
            .iter()
            .position(|&c| c as char == upper)
            .ok_or(Error::BadEncoding)? as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }
    // The leftover bits are padding and must be zero (one canonical spelling per code).
    if bits > 0 && (buffer & ((1 << bits) - 1)) != 0 {
        return Err(Error::BadEncoding);
    }
    Ok(out)
}

/// `YK1-` + Crockford base32 (no padding, groups of five joined by `-`) of
/// deflate(compact JSON) followed by a 4-byte checksum.
pub fn encode(configuration: &Configuration) -> Result<String, Error> {
    configuration.validate()?;
    let compact = Compact {
        v: 1,
        n: configuration.name.trim().to_owned(),
        b: configuration.base.clone(),
        m: configuration
            .mods
            .iter()
            .map(|entry| (entry.id.clone(), entry.version.clone()))
            .collect(),
        s: configuration.settings.clone(),
    };
    let json = serde_json::to_vec(&compact).map_err(|_| Error::Malformed)?;
    if json.len() > MAX_DECODED {
        return Err(Error::TooLarge);
    }
    let mut encoder =
        flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(&json).map_err(|_| Error::Malformed)?;
    let mut payload = encoder.finish().map_err(|_| Error::Malformed)?;
    let sum = checksum(&payload);
    payload.extend_from_slice(&sum);
    let text = base32_encode(&payload);
    let groups: Vec<&str> = text
        .as_bytes()
        .chunks(5)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect();
    Ok(format!("{CODE_PREFIX}{}", groups.join("-")))
}

/// Reads a code. It only describes a configuration: nothing here is trusted until [`resolve`]
/// has matched every id against the signed catalog.
pub fn decode(code: &str) -> Result<Configuration, Error> {
    let code: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let upper = code.to_ascii_uppercase();
    if upper.starts_with(SERVER_CODE_PREFIX) {
        return Err(Error::ServerCodeUnsupported);
    }
    let body = upper.strip_prefix(CODE_PREFIX).ok_or(Error::NotACode)?;
    // 4 KiB of JSON deflates to far less than this; it bounds the work before decoding.
    if body.len() > 2 * MAX_DECODED {
        return Err(Error::TooLarge);
    }
    let text: String = body.chars().filter(|c| *c != '-').collect();
    let bytes = base32_decode(&text)?;
    if bytes.len() < 5 {
        return Err(Error::Malformed);
    }
    let (payload, sum) = bytes.split_at(bytes.len() - 4);
    if checksum(payload) != sum {
        return Err(Error::Checksum);
    }
    let mut json = Vec::new();
    flate2::read::DeflateDecoder::new(payload)
        .take(MAX_DECODED as u64 + 1)
        .read_to_end(&mut json)
        .map_err(|_| Error::Malformed)?;
    if json.len() > MAX_DECODED {
        return Err(Error::TooLarge);
    }
    let compact: Compact = serde_json::from_slice(&json).map_err(|_| Error::Malformed)?;
    if compact.v != 1 {
        return Err(Error::NeedsUpdate);
    }
    let configuration = Configuration {
        name: compact.n,
        base: compact.b,
        mods: compact
            .m
            .into_iter()
            .map(|(id, version)| ModRef { id, version })
            .collect(),
        settings: compact.s,
    };
    configuration.validate()?;
    Ok(configuration)
}

// ---- resolution -------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The id is not in the signed catalog.
    UnknownMod(String),
    UnknownVersion { id: String, version: String },
    /// The base pack is not in the feed (or the feed is not available).
    UnknownBase(String),
    /// A built-in mod that the base's build does not contain.
    NotInBuild { id: String, base: String },
    /// A mod declares no shareable settings.
    SettingsNotShareable(String),
    /// The selected version of `id` needs `needs` in a range that the selected (or available)
    /// version of `needs` does not satisfy.
    DependencyVersion {
        id: String,
        needs: String,
        requirement: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub id: String,
    /// The exact version (`latest` resolved).
    pub version: String,
    pub kind: Kind,
    /// Added because another selected mod requires it.
    pub auto_added: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub name: String,
    pub base: String,
    pub mods: Vec<Resolved>,
    pub problems: Vec<Problem>,
}

impl Preview {
    pub fn installable(&self) -> bool {
        self.problems.is_empty()
    }

    /// The `.yak` mods to install into the instance after the base pack is in place.
    pub fn yak_set(&self) -> Vec<(String, String)> {
        self.mods
            .iter()
            .filter(|entry| entry.kind == Kind::Yak)
            .map(|entry| (entry.id.clone(), entry.version.clone()))
            .collect()
    }
}

fn satisfies(requirement: &str, version: &str) -> bool {
    VersionReq::parse(requirement)
        .is_ok_and(|range| Version::parse(version).is_ok_and(|version| range.matches(&version)))
}

/// What `entry` at `version` requires: the version-specific `.yak` ranges, else just ids.
fn dependencies_of<'a>(entry: &'a CatalogMod, version: &str) -> Vec<(&'a str, Option<&'a str>)> {
    match entry.requirements.get(version) {
        Some(ranges) => ranges
            .iter()
            .map(|(id, range)| (id.as_str(), Some(range.as_str())))
            .collect(),
        None => entry
            .requires
            .iter()
            .map(|id| (id.as_str(), None))
            .collect(),
    }
}

/// Matches a configuration with the signed catalog. `bases` maps a base pack id to the native
/// (built-in) mods its newest build contains. The versions a configuration names are kept; what
/// they require is added automatically (the newest version that satisfies the range) unless the
/// configuration already names it, in which case its version must satisfy the range.
pub fn resolve(
    configuration: &Configuration,
    catalog: &[CatalogMod],
    bases: &BTreeMap<String, Vec<String>>,
) -> Preview {
    let by_id: BTreeMap<&str, &CatalogMod> =
        catalog.iter().map(|entry| (entry.id.as_str(), entry)).collect();
    let mut problems = Vec::new();
    let mut mods: Vec<Resolved> = Vec::new();
    let mut chosen: BTreeMap<String, String> = BTreeMap::new();
    // The explicit entries first, so a requirement never overrides a pinned version.
    for requested in &configuration.mods {
        if chosen.contains_key(&requested.id) {
            continue;
        }
        let Some(entry) = by_id.get(requested.id.as_str()) else {
            problems.push(Problem::UnknownMod(requested.id.clone()));
            continue;
        };
        let exact = if requested.version == LATEST {
            entry.version.clone()
        } else if entry.versions.contains(&requested.version) {
            requested.version.clone()
        } else {
            problems.push(Problem::UnknownVersion {
                id: requested.id.clone(),
                version: requested.version.clone(),
            });
            continue;
        };
        chosen.insert(entry.id.clone(), exact.clone());
        mods.push(Resolved {
            id: entry.id.clone(),
            version: exact,
            kind: entry.kind,
            auto_added: false,
        });
    }
    let mut queue: VecDeque<(String, String)> = mods
        .iter()
        .map(|entry| (entry.id.clone(), entry.version.clone()))
        .collect();
    while let Some((id, version)) = queue.pop_front() {
        let Some(entry) = by_id.get(id.as_str()) else {
            continue;
        };
        for (needed, range) in dependencies_of(entry, &version) {
            let mismatch = |problems: &mut Vec<Problem>| {
                problems.push(Problem::DependencyVersion {
                    id: id.clone(),
                    needs: needed.to_owned(),
                    requirement: range.unwrap_or_default().to_owned(),
                });
            };
            if let Some(picked) = chosen.get(needed) {
                if range.is_some_and(|range| !satisfies(range, picked)) {
                    mismatch(&mut problems);
                }
                continue;
            }
            let Some(dependency) = by_id.get(needed) else {
                problems.push(Problem::UnknownMod(needed.to_owned()));
                continue;
            };
            let Some(exact) = dependency
                .versions
                .iter()
                .find(|candidate| range.is_none_or(|range| satisfies(range, candidate)))
            else {
                mismatch(&mut problems);
                continue;
            };
            chosen.insert(dependency.id.clone(), exact.clone());
            mods.push(Resolved {
                id: dependency.id.clone(),
                version: exact.clone(),
                kind: dependency.kind,
                auto_added: true,
            });
            queue.push_back((dependency.id.clone(), exact.clone()));
        }
    }
    match bases.get(&configuration.base) {
        None => problems.push(Problem::UnknownBase(configuration.base.clone())),
        Some(native) => {
            for entry in &mods {
                if entry.kind == Kind::Builtin && !native.contains(&entry.id) {
                    problems.push(Problem::NotInBuild {
                        id: entry.id.clone(),
                        base: configuration.base.clone(),
                    });
                }
            }
        }
    }
    for id in configuration.settings.keys() {
        problems.push(Problem::SettingsNotShareable(id.clone()));
    }
    Preview {
        name: configuration.name.trim().to_owned(),
        base: configuration.base.clone(),
        mods,
        problems,
    }
}

/// Selecting `ids` in the editor: adds every requirement, in catalog order, and says which
/// ids were added for that reason.
pub fn with_requirements(ids: &BTreeSet<String>, catalog: &[CatalogMod]) -> (BTreeSet<String>, BTreeSet<String>) {
    let by_id: BTreeMap<&str, &CatalogMod> =
        catalog.iter().map(|entry| (entry.id.as_str(), entry)).collect();
    let mut all = ids.clone();
    let mut stack: Vec<String> = ids.iter().cloned().collect();
    while let Some(id) = stack.pop() {
        for required in by_id.get(id.as_str()).map(|e| &e.requires).into_iter().flatten() {
            if by_id.contains_key(required.as_str()) && all.insert(required.clone()) {
                stack.push(required.clone());
            }
        }
    }
    let added = all.difference(ids).cloned().collect();
    (all, added)
}

// ---- local storage ----------------------------------------------------------------------------

const STORE: &str = "yoake-configs.json";
const MAX_STORE: usize = 1024 * 1024;

/// Why the saved list could not be read.
#[derive(Debug)]
enum Unreadable {
    Io(String),
    Format,
}

/// The raw entries, so unknown fields and entries this launcher cannot use survive a rewrite.
fn read_raw(data_dir: &Path) -> Result<Vec<serde_json::Value>, Unreadable> {
    let path = data_dir.join(STORE);
    let file = match std::fs::File::open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(Unreadable::Io(error.to_string())),
    };
    let mut bytes = Vec::new();
    file.take(MAX_STORE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| Unreadable::Io(error.to_string()))?;
    if bytes.len() > MAX_STORE {
        return Err(Unreadable::Format);
    }
    serde_json::from_slice(&bytes).map_err(|_| Unreadable::Format)
}

pub fn load(data_dir: &Path) -> Vec<Configuration> {
    read_raw(data_dir)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| serde_json::from_value::<Configuration>(value).ok())
        .filter(|configuration| configuration.validate().is_ok())
        .collect()
}

fn entry_name(value: &serde_json::Value) -> Option<&str> {
    value.get("name").and_then(serde_json::Value::as_str)
}

/// The list to change. A file that exists but cannot be parsed is copied to a `.bak` first (the
/// save is refused if that copy fails), so saving never destroys it.
fn writable_list(data_dir: &Path) -> Result<Vec<serde_json::Value>, String> {
    match read_raw(data_dir) {
        Ok(list) => Ok(list),
        Err(Unreadable::Io(error)) => Err(error),
        Err(Unreadable::Format) => {
            let backup = data_dir.join(format!(
                "{STORE}.{}.{}.bak",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |time| time.as_secs()),
                NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::copy(data_dir.join(STORE), &backup)
                .map_err(|error| format!("The saved configurations are damaged and could not be backed up: {error}"))?;
            Ok(Vec::new())
        }
    }
}

/// Saves (or replaces, by name) a configuration.
pub fn save(data_dir: &Path, configuration: Configuration) -> Result<(), String> {
    configuration.validate().map_err(|e| e.to_string())?;
    let mut list = writable_list(data_dir)?;
    let mut value = serde_json::to_value(&configuration).map_err(|e| e.to_string())?;
    // keep what a newer launcher saved next to the fields this one knows
    if let Some(existing) = list
        .iter()
        .find(|entry| entry_name(entry) == Some(configuration.name.as_str()))
        .and_then(serde_json::Value::as_object)
    {
        if let Some(fresh) = value.as_object_mut() {
            for (key, extra) in existing {
                if !matches!(key.as_str(), "name" | "base" | "mods" | "settings") {
                    fresh.entry(key.clone()).or_insert_with(|| extra.clone());
                }
            }
        }
    }
    list.retain(|entry| entry_name(entry) != Some(configuration.name.as_str()));
    list.push(value);
    if list.len() > MAX_SAVED {
        return Err("Too many saved configurations".into());
    }
    write_list(data_dir, &list)
}

pub fn remove(data_dir: &Path, name: &str) -> Result<(), String> {
    let mut list = writable_list(data_dir)?;
    list.retain(|entry| entry_name(entry) != Some(name));
    write_list(data_dir, &list)
}

fn write_list(data_dir: &Path, list: &[serde_json::Value]) -> Result<(), String> {
    std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
    let bytes = serde_json::to_vec_pretty(list).map_err(|e| e.to_string())?;
    let temporary = data_dir.join(format!(
        "{STORE}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temporary, data_dir.join(STORE))
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&temporary);
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
