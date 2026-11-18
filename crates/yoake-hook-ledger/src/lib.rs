//! Hook ID persistence and per-crate key tables.
//!
//! Changing a `cfg` expression on a hooked function, its impl, or an enclosing module changes its
//! key. Run the codemod with `--rename OLD=NEW` to keep the existing ID.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const TEST_ID_BASE: u32 = (1 << 16) - 64;

pub mod build;
mod set;

pub use set::{LedgerSet, check_append_only};

#[derive(Clone, Debug)]
pub struct Ledger {
    pub(crate) version: u32,
    pub(crate) next_id: u32,
    pub(crate) crate_name: Option<String>,
    pub(crate) entries: Vec<Entry>,
    pub(crate) excluded: Vec<Excluded>,
}

#[derive(Serialize, Deserialize)]
struct LegacyJson {
    version: u32,
    next_id: u32,
    entries: Vec<Entry>,
    #[serde(default)]
    excluded: Vec<Excluded>,
}

#[derive(Serialize, Deserialize)]
struct ShardJson {
    #[serde(rename = "crate")]
    crate_name: String,
    entries: Vec<Entry>,
    #[serde(default)]
    excluded: Vec<Excluded>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub id: u32,
    pub key: String,
    pub sig: String,
    pub status: Status,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub renamed_from: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    #[default]
    Fn,
    Callsite,
    Store,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Live,
    Tombstone,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Excluded {
    pub key: String,
    pub reason: String,
    #[serde(default)]
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub manual: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Assigned {
    pub id: u32,
    pub new: bool,
    pub sig_changed: bool,
}

#[derive(Debug)]
pub enum LedgerError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Invalid(String),
}

impl fmt::Display for LedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "ledger I/O error: {error}"),
            Self::Json(error) => write!(formatter, "invalid ledger JSON: {error}"),
            Self::Invalid(message) => write!(formatter, "invalid ledger: {message}"),
        }
    }
}

impl std::error::Error for LedgerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Invalid(_) => None,
        }
    }
}

impl From<std::io::Error> for LedgerError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for LedgerError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl Ledger {
    pub fn new() -> Ledger {
        Ledger {
            version: 1,
            next_id: 1,
            crate_name: None,
            entries: Vec::new(),
            excluded: Vec::new(),
        }
    }

    pub(crate) fn new_shard(crate_name: &str, next_id: u32) -> Ledger {
        Ledger {
            version: 2,
            next_id,
            crate_name: Some(crate_name.to_owned()),
            entries: Vec::new(),
            excluded: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Ledger, LedgerError> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), LedgerError> {
        std::fs::write(path, self.to_json())?;
        Ok(())
    }

    pub fn from_json(text: &str) -> Result<Ledger, LedgerError> {
        if let Ok(shard) = serde_json::from_str::<ShardJson>(text) {
            let next_id = shard
                .entries
                .iter()
                .map(|entry| entry.id)
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| LedgerError::Invalid("next_id overflow".into()))?;
            let ledger = Ledger {
                version: 2,
                next_id,
                crate_name: Some(shard.crate_name),
                entries: shard.entries,
                excluded: shard.excluded,
            };
            ledger.validate()?;
            return Ok(ledger);
        }
        let legacy: LegacyJson = serde_json::from_str(text)?;
        let ledger = Ledger {
            version: legacy.version,
            next_id: legacy.next_id,
            crate_name: None,
            entries: legacy.entries,
            excluded: legacy.excluded,
        };
        ledger.validate()?;
        Ok(ledger)
    }

    pub fn to_json(&self) -> String {
        let mut canonical = self.clone();
        canonical.entries.sort_by_key(|entry| entry.id);
        canonical.excluded.sort_by(|left, right| left.key.cmp(&right.key));
        let result = if let Some(crate_name) = canonical.crate_name {
            serde_json::to_string_pretty(&ShardJson {
                crate_name,
                entries: canonical.entries,
                excluded: canonical.excluded,
            })
        } else {
            serde_json::to_string_pretty(&LegacyJson {
                version: canonical.version,
                next_id: canonical.next_id,
                entries: canonical.entries,
                excluded: canonical.excluded,
            })
        };
        let mut json = result.expect("serializing a hook ledger cannot fail");
        json.push('\n');
        json
    }

    pub fn validate(&self) -> Result<(), LedgerError> {
        if !matches!(self.version, 1 | 2) {
            return Err(LedgerError::Invalid(format!(
                "unsupported version {}",
                self.version
            )));
        }
        if self.next_id == 0 {
            return Err(LedgerError::Invalid("next_id must be at least 1".into()));
        }
        let mut ids = BTreeSet::new();
        let mut live_keys = BTreeMap::new();
        for entry in &self.entries {
            if entry.id == 0 || entry.id >= self.next_id {
                return Err(LedgerError::Invalid(format!(
                    "entry id {} is outside 1..{}",
                    entry.id, self.next_id
                )));
            }
            if !ids.insert(entry.id) {
                return Err(LedgerError::Invalid(format!("duplicate id {}", entry.id)));
            }
            if entry.status == Status::Live {
                if let Some(previous) = live_keys.insert(&entry.key, entry.id) {
                    return Err(LedgerError::Invalid(format!(
                        "live key {:?} appears at ids {previous} and {}",
                        entry.key, entry.id
                    )));
                }
            }
        }
        for excluded in &self.excluded {
            if let Some(id) = live_keys.get(&excluded.key) {
                return Err(LedgerError::Invalid(format!(
                    "key {:?} is both live at id {id} and excluded",
                    excluded.key
                )));
            }
        }
        Ok(())
    }

    pub fn assign(&mut self, kind: Kind, key: &str, sig: &str) -> Assigned {
        if let Some(entry) = self
            .entries
            .iter_mut()
            .find(|entry| entry.status == Status::Live && entry.key == key)
        {
            assert_eq!(entry.kind, kind, "live hook key already has another kind: {key}");
            let sig_changed = entry.sig != sig;
            entry.sig = sig.to_owned();
            return Assigned {
                id: entry.id,
                new: false,
                sig_changed,
            };
        }
        assert!(
            !self.excluded.iter().any(|entry| entry.key == key && entry.manual),
            "manual exclusion cannot be overwritten: {key}"
        );
        assert!(self.next_id < TEST_ID_BASE, "hook id space exhausted");
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).expect("hook id space exhausted");
        self.entries.push(Entry {
            id,
            key: key.to_owned(),
            sig: sig.to_owned(),
            status: Status::Live,
            kind,
            renamed_from: Vec::new(),
        });
        self.unexclude(key);
        Assigned {
            id,
            new: true,
            sig_changed: false,
        }
    }

    pub fn rename(&mut self, old_key: &str, new_key: &str) -> Result<u32, LedgerError> {
        let id = self.live_id(old_key).ok_or_else(|| {
            LedgerError::Invalid(format!("cannot rename missing live key {old_key:?}"))
        })?;
        if old_key == new_key {
            return Ok(id);
        }
        if self.live_id(new_key).is_some() {
            return Err(LedgerError::Invalid(format!(
                "rename target {new_key:?} is already live"
            )));
        }
        if self.excluded.iter().any(|entry| entry.key == new_key) {
            return Err(LedgerError::Invalid(format!(
                "rename target {new_key:?} is excluded"
            )));
        }
        let entry = self.entries.iter_mut().find(|entry| entry.id == id).unwrap();
        entry.key = new_key.to_owned();
        entry.renamed_from.push(old_key.to_owned());
        Ok(id)
    }

    pub fn tombstone(&mut self, key: &str) -> Result<u32, LedgerError> {
        let id = self.live_id(key).ok_or_else(|| {
            LedgerError::Invalid(format!("cannot tombstone missing live key {key:?}"))
        })?;
        self.entries.iter_mut().find(|entry| entry.id == id).unwrap().status = Status::Tombstone;
        Ok(id)
    }

    pub fn exclude(&mut self, key: &str, reason: &str) -> Result<(), LedgerError> {
        self.exclude_kind(Kind::Fn, key, reason)
    }

    pub fn exclude_kind(&mut self, kind: Kind, key: &str, reason: &str) -> Result<(), LedgerError> {
        self.exclude_with_manual(kind, key, reason, false)
    }

    pub fn exclude_manual(&mut self, kind: Kind, key: &str, reason: &str) -> Result<(), LedgerError> {
        self.exclude_with_manual(kind, key, reason, true)
    }

    fn exclude_with_manual(
        &mut self,
        kind: Kind,
        key: &str,
        reason: &str,
        manual: bool,
    ) -> Result<(), LedgerError> {
        if self.live_id(key).is_some() {
            return Err(LedgerError::Invalid(format!(
                "cannot exclude live key {key:?}"
            )));
        }
        if let Some(entry) = self.excluded.iter_mut().find(|entry| entry.key == key) {
            if entry.manual && !manual {
                return Ok(());
            }
            entry.kind = kind;
            entry.reason = reason.to_owned();
            entry.manual = manual;
        } else {
            self.excluded.push(Excluded {
                key: key.to_owned(),
                reason: reason.to_owned(),
                kind,
                manual,
            });
        }
        Ok(())
    }

    pub fn unexclude(&mut self, key: &str) {
        self.excluded.retain(|entry| entry.key != key);
    }

    pub fn live_id(&self, key: &str) -> Option<u32> {
        self.entries
            .iter()
            .find(|entry| entry.status == Status::Live && entry.key == key)
            .map(|entry| entry.id)
    }

    pub fn live_id_including_renames(&self, key: &str) -> Option<u32> {
        self.live()
            .find(|entry| {
                entry.key == key || entry.renamed_from.iter().any(|old| old == key)
            })
            .map(|entry| entry.id)
    }

    pub fn live(&self) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(|entry| entry.status == Status::Live)
    }

    pub fn excluded(&self) -> &[Excluded] {
        &self.excluded
    }

    pub fn max_slots(&self) -> u32 {
        self.next_id
    }

    pub fn sig_hash(signature_tokens: &str) -> String {
        let normalized = signature_tokens.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut hash = 0xcbf29ce484222325_u64;
        for byte in normalized.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        format!("{hash:016x}")
    }
}

impl Default for Ledger {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod set_tests;
