use super::{Manifest, Target, manifest::identifier, manifest::web_url};
use horizon_release_sign::{PublicKey, Sha256, YOAKE_MODS_V1, verify_bytes};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::Read;

pub const MAX_CATALOG_SIZE: usize = 16 * 1024 * 1024;
pub const CATALOG_PATH: &str = "/yoake/mods/index.json";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    pub schema: u32,
    pub generated: String,
    pub entries: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogEntry {
    pub id: String,
    pub versions: Vec<CatalogVersion>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogVersion {
    pub version: Version,
    pub url: String,
    pub size: u64,
    pub sha256: String,
    pub manifest: Manifest,
    pub targets: Vec<Target>,
    pub engine: Option<String>,
}

impl Catalog {
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_CATALOG_SIZE {
            return Err("Yoake catalog exceeds 16 MiB".into());
        }
        let catalog: Self = serde_json::from_slice(bytes).map_err(|error| format!("Yoake catalog: {error}"))?;
        catalog.validate()?;
        Ok(catalog)
    }

    /// Verifies the exact downloaded JSON bytes, not a reserialized document.
    pub fn parse_verified(bytes: &[u8], signature: &str, keys: &[PublicKey]) -> Result<Self, String> {
        if bytes.len() > MAX_CATALOG_SIZE || signature.len() > 4096 {
            return Err("Yoake catalog or signature exceeds its size limit".into());
        }
        verify_bytes(YOAKE_MODS_V1, bytes, signature, keys)?;
        Self::parse(bytes)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 {
            return Err(format!("unsupported Yoake catalog schema {}", self.schema));
        }
        validate_timestamp(&self.generated)?;
        if self.entries.len() > 10000 {
            return Err("too many catalog entries".into());
        }
        let mut ids = BTreeSet::new();
        for entry in &self.entries {
            identifier(&entry.id)?;
            if !ids.insert(&entry.id) || entry.versions.is_empty() || entry.versions.len() > 1024 {
                return Err("catalog IDs must be unique, with 1..1024 versions each".into());
            }
            let mut versions = BTreeSet::new();
            for version in &entry.versions {
                if !versions.insert(version.version.to_string()) {
                    return Err(format!("duplicate catalog version {}@{}", entry.id, version.version));
                }
                version.validate(&entry.id)?;
            }
        }
        Ok(())
    }
}

impl CatalogVersion {
    fn validate(&self, id: &str) -> Result<(), String> {
        self.manifest.validate()?;
        if self.manifest.id != id || self.manifest.version != self.version {
            return Err("catalog ID/version differs from yak.toml summary".into());
        }
        if self.size == 0 || self.size > horizon_release_sign::yak::MAX_ARCHIVE_SIZE
            || self.sha256.len() != 64 || !self.sha256.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
            return Err("catalog size or lowercase SHA-256 is invalid".into());
        }
        let expected = format!("/yoake/mods/{id}/{}/{id}-{}.yak", self.version, self.version);
        let url = web_url(&self.url, true)?;
        if url.path() != expected || !self.url.ends_with(&expected) {
            return Err("catalog file URL must be under /yoake/mods/<id>/<version>/".into());
        }
        let expected_targets: BTreeSet<_> = self.manifest.bin.as_ref().map(|bin| bin.targets.iter().copied().collect()).unwrap_or_default();
        if self.targets.iter().copied().collect::<BTreeSet<_>>() != expected_targets
            || self.targets.len() != expected_targets.len()
            || self.engine.as_deref() != self.manifest.bin.as_ref().map(|bin| bin.engine.as_str()) {
            return Err("catalog targets/engine differ from yak.toml summary".into());
        }
        Ok(())
    }

    /// Authenticates an unsigned archive against a previously verified catalog.
    pub fn verify_download(&self, reader: &mut impl Read) -> Result<(), String> {
        if self.size == 0 || self.size > horizon_release_sign::yak::MAX_ARCHIVE_SIZE {
            return Err("invalid catalog download size".into());
        }
        let mut hash = Sha256::default();
        let mut total = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let read = reader.read(&mut buffer).map_err(|error| error.to_string())?;
            if read == 0 {
                break;
            }
            total = total.checked_add(read as u64).ok_or("download size overflow")?;
            if total > self.size {
                return Err("download is larger than the catalog size".into());
            }
            hash.update(&buffer[..read]);
        }
        if total != self.size || hash.hex() != self.sha256 {
            return Err("download size or SHA-256 differs from the catalog".into());
        }
        Ok(())
    }
}

fn validate_timestamp(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.len() != 20 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T'
        || bytes[13] != b':' || bytes[16] != b':' || bytes[19] != b'Z' {
        return Err("generated must be a UTC timestamp YYYY-MM-DDTHH:MM:SSZ".into());
    }
    let number = |start: usize, end: usize| -> Result<u32, String> {
        if !bytes[start..end].iter().all(u8::is_ascii_digit) {
            return Err("invalid generated timestamp".into());
        }
        value[start..end].parse().map_err(|_| "invalid generated timestamp".into())
    };
    let year = number(0, 4)?;
    let month = number(5, 7)?;
    let day = number(8, 10)?;
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => 0,
    };
    if year == 0 || day == 0 || day > days || number(11, 13)? > 23 || number(14, 16)? > 59 || number(17, 19)? > 59 {
        return Err("invalid generated calendar date/time".into());
    }
    Ok(())
}
