use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Assigned, Entry, Kind, Ledger, LedgerError, Status, TEST_ID_BASE};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Meta {
    pub(crate) version: u32,
    pub(crate) next_id: u32,
}

#[derive(Clone, Debug)]
pub struct LedgerSet {
    pub(crate) meta: Meta,
    pub(crate) shards: BTreeMap<String, Ledger>,
}

impl LedgerSet {
    pub fn new() -> Self {
        Self {
            meta: Meta {
                version: 2,
                next_id: 1,
            },
            shards: BTreeMap::new(),
        }
    }

    pub fn load(dir: &Path) -> Result<Self, LedgerError> {
        let meta_path = dir.join("meta.json");
        let meta: Meta = serde_json::from_str(&std::fs::read_to_string(meta_path)?)?;
        let mut shards = BTreeMap::new();
        for item in std::fs::read_dir(dir)? {
            let path = item?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json")
                || path.file_name().and_then(|value| value.to_str()) == Some("meta.json")
            {
                continue;
            }
            let mut shard = Ledger::load(&path)?;
            let crate_name = shard.crate_name.clone().ok_or_else(|| {
                LedgerError::Invalid(format!("{} is not a crate shard", path.display()))
            })?;
            if path.file_stem().and_then(|value| value.to_str()) != Some(crate_name.as_str()) {
                return Err(LedgerError::Invalid(format!(
                    "shard {} names crate {crate_name}",
                    path.display()
                )));
            }
            shard.next_id = meta.next_id;
            if shards.insert(crate_name.clone(), shard).is_some() {
                return Err(LedgerError::Invalid(format!(
                    "duplicate shard for {crate_name}"
                )));
            }
        }
        let set = Self { meta, shards };
        set.validate()?;
        Ok(set)
    }

    pub fn save(&self, dir: &Path) -> Result<(), LedgerError> {
        self.validate()?;
        std::fs::create_dir_all(dir)?;
        let mut meta = serde_json::to_string_pretty(&self.meta)?;
        meta.push('\n');
        std::fs::write(dir.join("meta.json"), meta)?;
        for (crate_name, shard) in &self.shards {
            shard.save(&dir.join(format!("{crate_name}.json")))?;
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), LedgerError> {
        if self.meta.version != 2 {
            return Err(LedgerError::Invalid(format!(
                "unsupported ledger set version {}",
                self.meta.version
            )));
        }
        if self.meta.next_id == 0 || self.meta.next_id > TEST_ID_BASE {
            return Err(LedgerError::Invalid(format!(
                "next_id {} is outside 1..={TEST_ID_BASE}",
                self.meta.next_id
            )));
        }
        let mut ids = BTreeMap::new();
        let mut live_keys = BTreeMap::new();
        let mut excluded_keys = BTreeSet::new();
        for (crate_name, shard) in &self.shards {
            if shard.crate_name.as_deref() != Some(crate_name.as_str()) {
                return Err(LedgerError::Invalid(format!(
                    "shard map key {crate_name} does not match its crate field"
                )));
            }
            for entry in &shard.entries {
                if entry.id == 0 || entry.id >= self.meta.next_id {
                    return Err(LedgerError::Invalid(format!(
                        "entry id {} is outside 1..{}",
                        entry.id, self.meta.next_id
                    )));
                }
                if let Some(previous) = ids.insert(entry.id, crate_name) {
                    return Err(LedgerError::Invalid(format!(
                        "id {} is shared by crates {previous} and {crate_name}",
                        entry.id
                    )));
                }
                if entry.status == Status::Live
                    && let Some(previous) = live_keys.insert(&entry.key, crate_name)
                {
                    return Err(LedgerError::Invalid(format!(
                        "live key {:?} is shared by crates {previous} and {crate_name}",
                        entry.key
                    )));
                }
            }
            for excluded in &shard.excluded {
                excluded_keys.insert(&excluded.key);
            }
        }
        for (key, crate_name) in live_keys {
            if excluded_keys.contains(key) {
                return Err(LedgerError::Invalid(format!(
                    "key {key:?} is live in {crate_name} and also excluded"
                )));
            }
        }
        Ok(())
    }

    pub fn shard(&self, crate_name: &str) -> Option<&Ledger> {
        self.shards.get(crate_name)
    }

    pub fn shard_mut(&mut self, crate_name: &str) -> &mut Ledger {
        let next_id = self.meta.next_id;
        self.shards
            .entry(crate_name.to_owned())
            .or_insert_with(|| Ledger::new_shard(crate_name, next_id))
    }

    pub fn assign(&mut self, crate_name: &str, kind: Kind, key: &str, sig: &str) -> Assigned {
        if let Some(owner) = self.shards.iter().find_map(|(name, shard)| {
            shard.live_id(key).map(|_| name.clone())
        }) {
            assert_eq!(owner, crate_name, "live hook key cannot move between crates: {key}");
            return self.shards.get_mut(&owner).unwrap().assign(kind, key, sig);
        }
        assert!(self.meta.next_id < TEST_ID_BASE, "hook id space exhausted");
        let next_id = self.meta.next_id;
        let assigned = {
            let shard = self.shard_mut(crate_name);
            shard.next_id = next_id;
            shard.assign(kind, key, sig)
        };
        self.meta.next_id = next_id.checked_add(1).expect("hook id space exhausted");
        assigned
    }

    pub fn next_id(&self) -> u32 {
        self.meta.next_id
    }
}

impl Default for LedgerSet {
    fn default() -> Self {
        Self::new()
    }
}

pub fn check_append_only(base: &LedgerSet, current: &LedgerSet) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if current.meta.next_id < base.meta.next_id {
        errors.push(format!(
            "next_id decreased from {} to {}",
            base.meta.next_id, current.meta.next_id
        ));
    }
    let current_entries = entries_by_id(current);
    for (id, (base_crate, base_entry)) in entries_by_id(base) {
        let Some((current_crate, current_entry)) = current_entries.get(&id).copied() else {
            errors.push(format!("id {id} from {base_crate} was removed"));
            continue;
        };
        if current_crate != base_crate {
            errors.push(format!("id {id} moved from {base_crate} to {current_crate}"));
        }
        if current_entry.key != base_entry.key
            && !current_entry.renamed_from.contains(&base_entry.key)
        {
            errors.push(format!("id {id} changed key without renamed_from"));
        }
        if base_entry.status == Status::Tombstone && current_entry.status != Status::Tombstone {
            errors.push(format!("tombstone id {id} is no longer a tombstone"));
        }
    }
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

fn entries_by_id(set: &LedgerSet) -> BTreeMap<u32, (&str, &Entry)> {
    set.shards
        .iter()
        .flat_map(|(crate_name, shard)| {
            shard
                .entries
                .iter()
                .map(move |entry| (entry.id, (crate_name.as_str(), entry)))
        })
        .collect()
}
