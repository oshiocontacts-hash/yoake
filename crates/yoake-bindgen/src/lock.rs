//! The id lock file: ids are assigned once and never reused or renumbered.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::doc::R;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LockEntry {
    pub id: u32,
    pub path: String,
    pub sig: String,
    /// `bound`; `ReadOnly` (a setter of a frozen field: the id is reserved and the call is refused);
    /// `unbound` when the generator no longer binds it (the id stays reserved).
    pub status: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Lock {
    pub format: u32,
    pub rustdoc_format_version: u64,
    pub bindings: Vec<LockEntry>,
}

/// What changed in the lock during one run.
#[derive(Default, Debug)]
pub struct LockDelta {
    pub added: Vec<String>,
    /// (path, old signature, new signature): the id is kept, the binding's contract changed.
    pub changed: Vec<(String, String, String)>,
    pub unbound: Vec<String>,
    pub rebound: Vec<String>,
}

impl Lock {
    pub fn empty() -> Lock {
        Lock { format: 1, rustdoc_format_version: crate::FORMAT_VERSION, bindings: Vec::new() }
    }

    pub fn load(path: &Path) -> R<Lock> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Lock::empty()),
            Err(error) => Err(error.into()),
        }
    }

    /// Gives every wanted path an id (existing ones keep theirs, new ones get the next free id in
    /// sorted order) and marks the rest `unbound`.
    pub fn assign(&mut self, wanted: &BTreeMap<String, (String, &'static str)>) -> (BTreeMap<String, u32>, LockDelta) {
        let mut delta = LockDelta::default();
        let mut next = self.bindings.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        let mut ids = BTreeMap::new();
        for (path, (sig, status)) in wanted {
            match self.bindings.iter_mut().find(|e| &e.path == path) {
                Some(entry) => {
                    if &entry.sig != sig {
                        delta.changed.push((path.clone(), entry.sig.clone(), sig.clone()));
                        entry.sig = sig.clone();
                    }
                    if entry.status != *status {
                        if entry.status == "unbound" {
                            delta.rebound.push(path.clone());
                        }
                        entry.status = (*status).to_string();
                    }
                    ids.insert(path.clone(), entry.id);
                }
                None => {
                    self.bindings.push(LockEntry { id: next, path: path.clone(), sig: sig.clone(), status: (*status).to_string() });
                    delta.added.push(path.clone());
                    ids.insert(path.clone(), next);
                    next += 1;
                }
            }
        }
        for entry in &mut self.bindings {
            if !wanted.contains_key(&entry.path) && entry.status != "unbound" {
                entry.status = "unbound".to_string();
                delta.unbound.push(entry.path.clone());
            }
        }
        self.bindings.sort_by_key(|e| e.id);
        self.rustdoc_format_version = crate::FORMAT_VERSION;
        (ids, delta)
    }

    /// The append-only rule against a base lock (what is on the target branch): every base id still
    /// exists with the same path, and no id is used for two paths. Returns the violations.
    pub fn append_only_violations(&self, base: &Lock) -> Vec<String> {
        let mut out = Vec::new();
        for old in &base.bindings {
            match self.bindings.iter().find(|e| e.id == old.id) {
                None => out.push(format!("id {} ({}) was removed", old.id, old.path)),
                Some(now) if now.path != old.path => out.push(format!("id {} was renumbered: `{}` -> `{}`", old.id, old.path, now.path)),
                Some(_) => {}
            }
            if !self.bindings.iter().any(|e| e.path == old.path) {
                out.push(format!("`{}` was removed (it must stay as `unbound`)", old.path));
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        for e in &self.bindings {
            if !ids.insert(e.id) {
                out.push(format!("id {} is used twice", e.id));
            }
        }
        out
    }

    pub fn to_text(&self) -> R<String> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}
