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

/// A range of ids the generator never allocates into (set aside for hand-assigned bindings of a later
/// module). Existing entries inside it are a violation (`reserved_violations`), except an `unbound`
/// sentinel, which marks the range's end.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Reserved {
    pub first: u32,
    pub last: u32,
    pub owner: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Lock {
    pub format: u32,
    pub rustdoc_format_version: u64,
    pub bindings: Vec<LockEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reserved: Vec<Reserved>,
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
        Lock { format: 1, rustdoc_format_version: crate::FORMAT_VERSION, bindings: Vec::new(), reserved: Vec::new() }
    }

    pub fn load(path: &Path) -> R<Lock> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Lock::empty()),
            Err(error) => Err(error.into()),
        }
    }

    /// The first id at or after `id` that is in no reserved range.
    pub fn skip_reserved(&self, mut id: u32) -> u32 {
        while let Some(range) = self.reserved.iter().find(|r| (r.first..=r.last).contains(&id)) {
            id = range.last + 1;
        }
        id
    }

    /// Entries inside a reserved range (an `unbound` sentinel excepted), and ranges that are empty or overlap.
    pub fn reserved_violations(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (i, r) in self.reserved.iter().enumerate() {
            if r.first > r.last {
                out.push(format!("reserved range {}..={} ({}) is empty", r.first, r.last, r.owner));
            }
            for other in &self.reserved[i + 1..] {
                if r.first <= other.last && other.first <= r.last {
                    out.push(format!("reserved ranges of {} and {} overlap", r.owner, other.owner));
                }
            }
            for e in &self.bindings {
                if (r.first..=r.last).contains(&e.id) && e.status != "unbound" {
                    out.push(format!("binding id {} (`{}`) is inside the range reserved for {}", e.id, e.path, r.owner));
                }
            }
        }
        out
    }

    /// Gives every wanted path an id (existing ones keep theirs, new ones get the next free id in
    /// sorted order) and marks the rest `unbound`.
    pub fn assign(&mut self, wanted: &BTreeMap<String, (String, &'static str)>) -> (BTreeMap<String, u32>, LockDelta) {
        let mut delta = LockDelta::default();
        let mut next = self.skip_reserved(self.bindings.iter().map(|e| e.id).max().unwrap_or(0) + 1);
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
                    next = self.skip_reserved(next + 1);
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
        out.extend(self.reserved_violations());
        out
    }

    pub fn to_text(&self) -> R<String> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, path: &str, status: &str) -> LockEntry {
        LockEntry { id, path: path.into(), sig: "s".into(), status: status.into() }
    }

    #[test]
    fn new_ids_skip_a_reserved_range() {
        let mut lock = Lock::empty();
        lock.bindings.push(entry(1, "a", "bound"));
        lock.bindings.push(entry(2, "b", "bound"));
        lock.reserved.push(Reserved { first: 3, last: 6, owner: "later".into() });
        let wanted: BTreeMap<String, (String, &'static str)> =
            ["a", "b", "c", "d"].iter().map(|p| (p.to_string(), ("s".to_string(), "bound"))).collect();
        let (ids, delta) = lock.assign(&wanted);
        assert_eq!((ids["c"], ids["d"]), (7, 8), "{ids:?}");
        assert_eq!(delta.added, ["c", "d"]);
        assert!(lock.reserved_violations().is_empty());
    }

    #[test]
    fn an_entry_inside_a_reserved_range_is_a_violation_but_the_unbound_sentinel_is_not() {
        let mut lock = Lock::empty();
        lock.reserved.push(Reserved { first: 10, last: 20, owner: "later".into() });
        lock.bindings.push(entry(20, "later::__reserved_range_end", "unbound"));
        assert!(lock.reserved_violations().is_empty());
        lock.bindings.push(entry(12, "oops", "bound"));
        let v = lock.reserved_violations();
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].contains("12") && v[0].contains("later"));
        assert!(lock.append_only_violations(&Lock::empty()).iter().any(|m| m.contains("reserved")));
    }
}
