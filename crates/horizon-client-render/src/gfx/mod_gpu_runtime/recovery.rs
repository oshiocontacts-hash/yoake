use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

use super::*;

const MAX_MARKER_BYTES: u64 = 1024 * 1024;
static RECOVERY: OnceLock<Result<Arc<Mutex<Recovery>>, String>> = OnceLock::new();

struct Recovery {
    path: PathBuf,
    blocked: BTreeSet<String>,
    owners: BTreeMap<ModId, String>,
    inflight: BTreeMap<ModId, u32>,
}

impl Recovery {
    fn open(path: PathBuf) -> Result<Self, String> {
        let blocked = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_MARKER_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                if bytes.len() as u64 > MAX_MARKER_BYTES {
                    return Err("GPU recovery marker exceeds its size limit".to_owned());
                }
                let identities: BTreeSet<String> =
                    serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
                if identities.len() > 4096
                    || identities.iter().any(|identity| !valid_identity(identity))
                {
                    return Err("GPU recovery marker contains invalid mod identities".to_owned());
                }
                identities
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeSet::new(),
            Err(error) => return Err(error.to_string()),
        };
        Ok(Self {
            path,
            blocked,
            owners: BTreeMap::new(),
            inflight: BTreeMap::new(),
        })
    }

    fn register(&mut self, owner: ModId, identity: &str) -> Result<(), String> {
        if !valid_identity(identity) {
            return Err("GPU recovery requires a valid mod identity".to_owned());
        }
        self.owners.insert(owner, identity.to_owned());
        if self.blocked.contains(identity) {
            return Err(format!(
                "previous GPU submission did not complete; inspect and remove {} to retry",
                self.path.display()
            ));
        }
        Ok(())
    }

    fn persist(&self) -> Result<(), String> {
        let mut identities = self.blocked.clone();
        for owner in self.inflight.keys() {
            identities.insert(
                self.owners
                    .get(owner)
                    .ok_or_else(|| "GPU owner is not registered".to_owned())?
                    .clone(),
            );
        }
        let bytes = serde_json::to_vec(&identities).map_err(|error| error.to_string())?;
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "GPU recovery path has no parent".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("tmp");
        let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(&temporary, &self.path).map_err(|error| error.to_string())
    }

    fn before(&mut self, owner: ModId) -> Result<(), String> {
        let identity = self
            .owners
            .get(&owner)
            .ok_or_else(|| "GPU owner is not registered".to_owned())?;
        if self.blocked.contains(identity) {
            return Err("GPU owner is disabled after an incomplete submission".to_owned());
        }
        let count = self.inflight.get(&owner).copied().unwrap_or(0);
        self.inflight.insert(
            owner,
            count
                .checked_add(1)
                .ok_or_else(|| "too many GPU submissions".to_owned())?,
        );
        self.persist()
    }

    fn complete(&mut self, owner: ModId) -> Result<(), String> {
        let Some(count) = self.inflight.get_mut(&owner) else {
            return Ok(());
        };
        *count -= 1;
        if *count == 0 {
            self.inflight.remove(&owner);
        }
        self.persist()
    }
}

fn valid_identity(identity: &str) -> bool {
    !identity.is_empty() && identity.len() <= 256 && !identity.chars().any(char::is_control)
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn configure(path: PathBuf) -> Result<(), String> {
    let recovery = Recovery::open(path)?;
    RECOVERY
        .set(Ok(Arc::new(Mutex::new(recovery))))
        .map_err(|_| "GPU recovery path is already configured".to_owned())
}

pub(super) fn register(owner: ModId, identity: &str) -> Result<(), String> {
    let recovery = RECOVERY.get_or_init(|| {
        Recovery::open(crate::logging::game_home().join("state/wasm-gpu-inflight.json"))
            .map(|recovery| Arc::new(Mutex::new(recovery)))
    });
    recovery
        .as_ref()
        .map_err(Clone::clone)?
        .lock()
        .map_err(|_| "GPU recovery lock unavailable".to_owned())?
        .register(owner, identity)
}

pub(super) fn install(gpu: &mut Gpu, lost: Arc<AtomicBool>) {
    let Some(recovery) = RECOVERY.get() else {
        return;
    };
    let recovery = match recovery {
        Ok(recovery) => recovery.clone(),
        Err(error) => {
            for owner in callbacks()
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .keys()
            {
                gpu.disable(*owner, format!("GPU recovery unavailable: {error}"));
            }
            return;
        }
    };
    {
        let state = recovery.lock().unwrap_or_else(|error| error.into_inner());
        for (owner, identity) in &state.owners {
            if state.blocked.contains(identity) {
                gpu.disable(
                    *owner,
                    "previous GPU submission did not complete".to_owned(),
                );
            }
        }
    }
    let before = recovery.clone();
    gpu.set_submission_hooks(super::super::mod_gpu_resources::SubmissionHooks {
        before: Arc::new(move |owner| {
            before
                .lock()
                .map_err(|_| "GPU recovery lock unavailable".to_owned())?
                .before(owner)
        }),
        complete: Arc::new(move |owner| {
            if lost.load(Ordering::Acquire) {
                return;
            }
            if let Err(error) = recovery
                .lock()
                .map_err(|_| "GPU recovery lock unavailable".to_owned())
                .and_then(|mut state| state.complete(owner))
            {
                log::warn!("WASM GPU completion marker could not be saved: {error}");
            }
        }),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marker() -> PathBuf {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        std::env::temp_dir().join(format!(
            "yoake-gpu-recovery-{}-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn incomplete_work_blocks_stable_identity_on_restart() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state.register(ModId(1), "particles").unwrap();
        state.before(ModId(1)).unwrap();
        let mut restarted = Recovery::open(path.clone()).unwrap();
        assert!(restarted.register(ModId(99), "particles").is_err());
        assert!(restarted.before(ModId(99)).is_err());
        restarted.register(ModId(2), "mesh").unwrap();
        restarted.before(ModId(2)).unwrap();
        restarted.complete(ModId(2)).unwrap();
        assert!(
            Recovery::open(path.clone())
                .unwrap()
                .blocked
                .contains("particles")
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn completion_waits_for_every_submission_and_preserves_other_owners() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state.register(ModId(1), "particles").unwrap();
        state.register(ModId(2), "mesh").unwrap();
        state.before(ModId(1)).unwrap();
        state.before(ModId(1)).unwrap();
        state.before(ModId(2)).unwrap();
        state.complete(ModId(1)).unwrap();
        assert_eq!(Recovery::open(path.clone()).unwrap().blocked.len(), 2);
        state.complete(ModId(1)).unwrap();
        assert_eq!(
            Recovery::open(path.clone()).unwrap().blocked,
            BTreeSet::from(["mesh".to_owned()])
        );
        state.complete(ModId(2)).unwrap();
        assert!(Recovery::open(path.clone()).unwrap().blocked.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn malformed_marker_fails_closed() {
        let path = marker();
        fs::write(&path, b"not json").unwrap();
        assert!(Recovery::open(path.clone()).is_err());
        fs::write(&path, b"[\\]").unwrap();
        assert!(Recovery::open(path.clone()).is_err());
        fs::write(&path, vec![b' '; MAX_MARKER_BYTES as usize + 1]).unwrap();
        assert!(Recovery::open(path.clone()).is_err());
        fs::remove_file(path).unwrap();
    }
}
