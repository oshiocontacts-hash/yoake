use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::PathBuf;

use super::*;

const MAX_MARKER_BYTES: u64 = 1024 * 1024;
const IDLE_FRAMES: u64 = 120;
static RECOVERY: OnceLock<Result<Arc<Mutex<Recovery>>, String>> = OnceLock::new();

#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Marker {
    incomplete: BTreeSet<String>,
    packages: BTreeMap<String, Package>,
}

#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Package {
    fingerprint: String,
    #[serde(default)]
    quarantines: BTreeMap<String, u8>,
    #[serde(default)]
    quarantined: bool,
}

impl Package {
    fn quarantine(&mut self) {
        if !self.quarantined {
            let count = self
                .quarantines
                .entry(self.fingerprint.clone())
                .or_default();
            *count = count.saturating_add(1).min(2);
            self.quarantined = true;
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredMarker {
    Current(Marker),
    Legacy(BTreeSet<String>),
}

struct Recovery {
    path: PathBuf,
    blocked: BTreeSet<String>,
    owners: BTreeMap<ModId, String>,
    inflight: BTreeMap<ModId, u32>,
    retained: BTreeMap<ModId, u64>,
    packages: BTreeMap<String, Package>,
    persisted: Marker,
    frame: u64,
    retry_frame: u64,
    last_warning: Option<Instant>,
    finished: bool,
}

impl Recovery {
    fn open(path: PathBuf) -> Result<Self, String> {
        let mut marker = match File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_MARKER_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                if bytes.len() as u64 > MAX_MARKER_BYTES {
                    return Err("GPU recovery marker exceeds its size limit".to_owned());
                }
                let marker =
                    match serde_json::from_slice(&bytes).map_err(|error| error.to_string())? {
                        StoredMarker::Current(marker) => marker,
                        StoredMarker::Legacy(incomplete) => Marker {
                            incomplete,
                            ..Marker::default()
                        },
                    };
                if marker.incomplete.len() > 4096
                    || marker.packages.len() > 4096
                    || marker
                        .incomplete
                        .iter()
                        .any(|identity| !valid_identity(identity))
                    || marker.packages.iter().any(|(identity, package)| {
                        !valid_identity(identity)
                            || !valid_fingerprint(&package.fingerprint)
                            || package.quarantines.len() > 4096
                            || package.quarantines.iter().any(|(fingerprint, count)| {
                                !valid_fingerprint(fingerprint) || !(1..=2).contains(count)
                            })
                    })
                {
                    return Err("GPU recovery marker contains invalid mod identities".to_owned());
                }
                marker
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Marker::default(),
            Err(error) => return Err(error.to_string()),
        };
        let persisted = marker.clone();
        for identity in &marker.incomplete {
            if let Some(package) = marker.packages.get_mut(identity) {
                package.quarantine();
            }
        }
        Ok(Self {
            path,
            persisted,
            blocked: marker.incomplete,
            packages: marker.packages,
            owners: BTreeMap::new(),
            inflight: BTreeMap::new(),
            retained: BTreeMap::new(),
            frame: 0,
            retry_frame: 0,
            last_warning: None,
            finished: false,
        })
    }

    #[cfg(test)]
    fn register(&mut self, owner: ModId, identity: &str) -> Result<(), String> {
        self.register_package(owner, identity, "test-package-v1")
    }

    fn register_package(
        &mut self,
        owner: ModId,
        identity: &str,
        fingerprint: &str,
    ) -> Result<(), String> {
        if !valid_identity(identity) || !valid_fingerprint(fingerprint) {
            return Err(
                "GPU recovery requires a valid mod identity and package fingerprint".to_owned(),
            );
        }
        if !self.packages.contains_key(identity) && self.packages.len() >= 4096 {
            return Err("GPU recovery package limit exceeded".to_owned());
        }
        let changed = self
            .packages
            .get(identity)
            .is_some_and(|package| package.fingerprint != fingerprint);
        if changed
            && self
                .inflight
                .keys()
                .any(|owner| self.owners.get(owner).is_some_and(|name| name == identity))
        {
            return Err("cannot replace a package with outstanding GPU work".to_owned());
        }
        self.owners.insert(owner, identity.to_owned());
        let package = self
            .packages
            .entry(identity.to_owned())
            .or_insert_with(|| Package {
                fingerprint: fingerprint.to_owned(),
                quarantines: BTreeMap::new(),
                quarantined: false,
            });
        if self.blocked.contains(identity) {
            package.quarantine();
        }
        if package.fingerprint != fingerprint {
            if self.blocked.contains(identity)
                && package
                    .quarantines
                    .get(&package.fingerprint)
                    .copied()
                    .unwrap_or(0)
                    >= 2
            {
                return Err(
                    "repeated GPU quarantine requires explicitly re-enabling GPU access".to_owned(),
                );
            }
            if !package.quarantines.contains_key(fingerprint) && package.quarantines.len() >= 4096 {
                return Err("GPU recovery fingerprint limit exceeded".to_owned());
            }
            package.fingerprint = fingerprint.to_owned();
            package.quarantined = false;
            self.blocked.remove(identity);
            self.retained
                .retain(|owner, _| self.owners.get(owner).is_none_or(|name| name != identity));
        }
        if self.blocked.contains(identity) {
            return Err("previous GPU submission did not complete; update the package or explicitly re-enable its GPU access".to_owned());
        }
        Ok(())
    }

    fn persist(&mut self) -> Result<(), String> {
        let mut identities = self.blocked.clone();
        for owner in self.retained.keys() {
            identities.insert(
                self.owners
                    .get(owner)
                    .ok_or_else(|| "GPU owner is not registered".to_owned())?
                    .clone(),
            );
        }
        let marker = Marker {
            incomplete: identities,
            packages: self.packages.clone(),
        };
        if marker == self.persisted {
            return Ok(());
        }
        let bytes = serde_json::to_vec(&marker).map_err(|error| error.to_string())?;
        if bytes.len() as u64 > MAX_MARKER_BYTES {
            return Err("GPU recovery marker exceeds its size limit".to_owned());
        }
        let parent = self
            .path
            .parent()
            .ok_or_else(|| "GPU recovery path has no parent".to_owned())?;
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        let temporary = self.path.with_extension("tmp");
        let mut file = File::create(&temporary).map_err(|error| error.to_string())?;
        file.write_all(&bytes).map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(&temporary, &self.path).map_err(|error| error.to_string())?;
        self.persisted = marker;
        Ok(())
    }

    fn save(&mut self) {
        self.save_at(Instant::now());
    }

    fn save_at(&mut self, now: Instant) -> bool {
        if self.frame < self.retry_frame {
            return false;
        }
        match self.persist() {
            Ok(()) => false,
            Err(error) => {
                self.retry_frame = self.frame.saturating_add(IDLE_FRAMES);
                if self
                    .last_warning
                    .is_none_or(|previous| now.saturating_duration_since(previous).as_secs() >= 60)
                {
                    self.last_warning = Some(now);
                    log::warn!("WASM GPU recovery marker could not be saved: {error}");
                    true
                } else {
                    false
                }
            }
        }
    }

    fn begin_frame(&mut self) {
        self.frame = self.frame.saturating_add(1);
        self.retained.retain(|owner, frame| {
            self.inflight.contains_key(owner) || self.frame.saturating_sub(*frame) < IDLE_FRAMES
        });
        self.save();
    }

    fn device_lost(&mut self) {
        for owner in self.inflight.keys() {
            if let Some(identity) = self.owners.get(owner) {
                self.blocked.insert(identity.clone());
                if let Some(package) = self.packages.get_mut(identity) {
                    package.quarantine();
                }
            }
        }
        self.save();
    }

    fn clean_exit(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.retained
            .retain(|owner, _| self.inflight.contains_key(owner));
        self.save();
    }

    fn reenable(&mut self, identity: &str) -> Result<Vec<ModId>, String> {
        if !valid_identity(identity) {
            return Err("GPU recovery requires a valid mod identity".to_owned());
        }
        let owners: Vec<_> = self
            .owners
            .iter()
            .filter_map(|(owner, name)| (name == identity).then_some(*owner))
            .collect();
        if owners.iter().any(|owner| self.inflight.contains_key(owner)) {
            return Err("cannot re-enable a mod with outstanding GPU work".to_owned());
        }
        let blocked = self.blocked.remove(identity);
        let quarantined = self
            .packages
            .get_mut(identity)
            .is_some_and(|package| std::mem::replace(&mut package.quarantined, false));
        let retained: BTreeMap<_, _> = owners
            .iter()
            .filter_map(|owner| self.retained.remove(owner).map(|frame| (*owner, frame)))
            .collect();
        if let Err(error) = self.persist() {
            self.retained.extend(retained);
            if blocked {
                self.blocked.insert(identity.to_owned());
            }
            if let Some(package) = self.packages.get_mut(identity) {
                package.quarantined = quarantined;
            }
            return Err(error);
        }
        Ok(owners)
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
        self.retained.insert(owner, self.frame);
        self.save();
        Ok(())
    }

    fn complete(&mut self, owner: ModId) -> Result<(), String> {
        let Some(count) = self.inflight.get_mut(&owner) else {
            return Ok(());
        };
        *count -= 1;
        if *count == 0 {
            self.inflight.remove(&owner);
        }
        self.retained.insert(owner, self.frame);
        Ok(())
    }
}

fn valid_identity(identity: &str) -> bool {
    !identity.is_empty() && identity.len() <= 256 && !identity.chars().any(char::is_control)
}

fn valid_fingerprint(fingerprint: &str) -> bool {
    !fingerprint.is_empty()
        && fingerprint.len() <= 512
        && !fingerprint.chars().any(char::is_control)
}

fn update(operation: impl FnOnce(&mut Recovery)) {
    if let Some(Ok(state)) = RECOVERY.get() {
        operation(&mut state.lock().unwrap_or_else(|error| error.into_inner()));
    }
}

pub(super) fn begin_frame() {
    update(Recovery::begin_frame);
}

pub(super) fn device_lost() {
    update(Recovery::device_lost);
}

pub(super) fn clean_exit() {
    update(Recovery::clean_exit);
}

#[cfg(any(test, feature = "test-support"))]
pub(super) fn configure(path: PathBuf) -> Result<(), String> {
    let recovery = Recovery::open(path)?;
    RECOVERY
        .set(Ok(Arc::new(Mutex::new(recovery))))
        .map_err(|_| "GPU recovery path is already configured".to_owned())
}

pub(super) fn register(owner: ModId, identity: &str, fingerprint: &str) -> Result<(), String> {
    let recovery = RECOVERY.get_or_init(|| {
        Recovery::open(crate::logging::game_home().join("state/wasm-gpu-inflight.json"))
            .map(|recovery| Arc::new(Mutex::new(recovery)))
    });
    recovery
        .as_ref()
        .map_err(Clone::clone)?
        .lock()
        .map_err(|_| "GPU recovery lock unavailable".to_owned())?
        .register_package(owner, identity, fingerprint)
}

pub(super) fn reenable(identity: &str) -> Result<Vec<ModId>, String> {
    RECOVERY
        .get()
        .ok_or_else(|| "GPU recovery is not initialized".to_owned())?
        .as_ref()
        .map_err(Clone::clone)?
        .lock()
        .map_err(|_| "GPU recovery lock unavailable".to_owned())?
        .reenable(identity)
}

pub(super) fn replace_package(owner: ModId, fingerprint: &str) -> Result<(), String> {
    let mut state = RECOVERY
        .get()
        .ok_or_else(|| "GPU recovery is not initialized".to_owned())?
        .as_ref()
        .map_err(Clone::clone)?
        .lock()
        .map_err(|_| "GPU recovery lock unavailable".to_owned())?;
    let identity = state
        .owners
        .get(&owner)
        .ok_or_else(|| "GPU owner is not registered".to_owned())?
        .clone();
    state.register_package(owner, &identity, fingerprint)
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
                device_lost();
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
            BTreeSet::from(["mesh".to_owned(), "particles".to_owned()])
        );
        state.complete(ModId(2)).unwrap();
        for _ in 0..IDLE_FRAMES {
            state.begin_frame();
        }
        assert!(Recovery::open(path.clone()).unwrap().blocked.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn unchanged_identity_set_does_not_rewrite_marker() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state.register(ModId(1), "particles").unwrap();
        state.before(ModId(1)).unwrap();
        fs::remove_file(&path).unwrap();
        state.before(ModId(1)).unwrap();
        state.complete(ModId(1)).unwrap();
        state.begin_frame();
        assert!(!path.exists());
        state.complete(ModId(1)).unwrap();
        state.clean_exit();
        assert!(Recovery::open(path.clone()).unwrap().blocked.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rename_failure_is_retryable_without_disabling_owner() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state.register(ModId(1), "particles").unwrap();
        fs::create_dir(&path).unwrap();
        state.before(ModId(1)).unwrap();
        assert!(state.blocked.is_empty());
        assert!(state.persisted.incomplete.is_empty());
        fs::remove_dir(&path).unwrap();
        for _ in 0..IDLE_FRAMES - 1 {
            state.begin_frame();
        }
        assert!(!path.exists());
        state.begin_frame();
        assert!(
            Recovery::open(path.clone())
                .unwrap()
                .blocked
                .contains("particles")
        );
        state.complete(ModId(1)).unwrap();
        state.clean_exit();
        assert!(Recovery::open(path.clone()).unwrap().blocked.is_empty());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn device_loss_quarantines_outstanding_work_in_memory_and_on_disk() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state.register(ModId(1), "particles").unwrap();
        state.register(ModId(2), "mesh").unwrap();
        state.before(ModId(1)).unwrap();
        state.before(ModId(2)).unwrap();
        state.complete(ModId(2)).unwrap();
        state.device_lost();
        assert_eq!(state.blocked, BTreeSet::from(["particles".to_owned()]));
        assert!(state.before(ModId(1)).is_err());
        state.complete(ModId(1)).unwrap();
        state.clean_exit();
        assert_eq!(Recovery::open(path.clone()).unwrap().blocked, state.blocked);
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

    #[test]
    fn changed_package_unblocks_only_after_outstanding_work_completes() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state
            .register_package(ModId(1), "particles", "v1:hash1")
            .unwrap();
        state.before(ModId(1)).unwrap();
        state.device_lost();
        assert!(
            state
                .register_package(ModId(1), "particles", "v2:hash2")
                .is_err()
        );
        state.complete(ModId(1)).unwrap();
        assert!(
            state
                .register_package(ModId(1), "particles", "v1:hash1")
                .is_err()
        );
        state
            .register_package(ModId(1), "particles", "v2:hash2")
            .unwrap();
        state.persist().unwrap();
        let mut restarted = Recovery::open(path.clone()).unwrap();
        restarted
            .register_package(ModId(2), "particles", "v2:hash2")
            .unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn blocked_sessions_never_expire_quarantine_or_count_as_new_faults() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state.register(ModId(1), "particles").unwrap();
        state.before(ModId(1)).unwrap();
        state.device_lost();
        state.complete(ModId(1)).unwrap();
        state.clean_exit();
        for _ in 0..5 {
            let mut restarted = Recovery::open(path.clone()).unwrap();
            assert!(restarted.register(ModId(1), "particles").is_err());
            assert!(restarted.before(ModId(1)).is_err());
            restarted.clean_exit();
            restarted.clean_exit();
            assert!(restarted.blocked.contains("particles"));
            assert_eq!(
                restarted.packages["particles"].quarantines["test-package-v1"],
                1
            );
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn repeated_fingerprint_faults_require_owner_action_even_after_package_changes() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state
            .register_package(ModId(1), "particles", "v1:hash1")
            .unwrap();
        state.before(ModId(1)).unwrap();
        state.device_lost();
        state.device_lost();
        state.complete(ModId(1)).unwrap();
        assert_eq!(state.packages["particles"].quarantines["v1:hash1"], 1);
        state
            .register_package(ModId(1), "particles", "v2:hash2")
            .unwrap();
        state
            .register_package(ModId(1), "particles", "v1:hash1")
            .unwrap();
        state.before(ModId(1)).unwrap();
        state.device_lost();
        state.complete(ModId(1)).unwrap();
        state.clean_exit();
        let mut restarted = Recovery::open(path.clone()).unwrap();
        assert!(
            restarted
                .register_package(ModId(99), "particles", "v2:hash2")
                .is_err()
        );
        assert!(restarted.before(ModId(99)).is_err());
        assert_eq!(restarted.packages["particles"].quarantines["v1:hash1"], 2);
        assert_eq!(restarted.reenable("particles").unwrap(), vec![ModId(99)]);
        restarted
            .register_package(ModId(99), "particles", "v2:hash2")
            .unwrap();
        restarted.before(ModId(99)).unwrap();
        restarted.complete(ModId(99)).unwrap();
        restarted.clean_exit();
        assert_eq!(
            Recovery::open(path.clone()).unwrap().packages["particles"].quarantines["v1:hash1"],
            2
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn old_clean_session_markers_remain_quarantined_until_package_change() {
        let path = marker();
        fs::write(&path, br#"{"incomplete":["particles"],"packages":{"particles":{"fingerprint":"v1:hash1","clean_sessions":3}}}"#).unwrap();
        let mut state = Recovery::open(path.clone()).unwrap();
        assert!(
            state
                .register_package(ModId(1), "particles", "v1:hash1")
                .is_err()
        );
        state.clean_exit();
        let mut restarted = Recovery::open(path.clone()).unwrap();
        assert!(
            restarted
                .register_package(ModId(1), "particles", "v1:hash1")
                .is_err()
        );
        assert_eq!(restarted.packages["particles"].quarantines["v1:hash1"], 1);
        restarted
            .register_package(ModId(1), "particles", "v2:hash2")
            .unwrap();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn reenable_is_persisted_and_rejects_outstanding_work() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state.register(ModId(1), "particles").unwrap();
        state.before(ModId(1)).unwrap();
        state.device_lost();
        assert!(state.reenable("particles").is_err());
        state.complete(ModId(1)).unwrap();
        assert_eq!(state.reenable("particles").unwrap(), vec![ModId(1)]);
        assert_eq!(
            state.packages["particles"].quarantines["test-package-v1"],
            1
        );
        assert!(
            !Recovery::open(path.clone())
                .unwrap()
                .blocked
                .contains("particles")
        );
        state.before(ModId(1)).unwrap();
        state.complete(ModId(1)).unwrap();
        state.clean_exit();
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn persistence_failures_back_off_and_warn_at_most_once_per_minute() {
        let path = marker();
        let mut state = Recovery::open(path.clone()).unwrap();
        state.register(ModId(1), "particles").unwrap();
        fs::create_dir(&path).unwrap();
        let now = Instant::now();
        assert!(state.save_at(now));
        assert_eq!(state.retry_frame, IDLE_FRAMES);
        assert!(!state.save_at(now + std::time::Duration::from_secs(61)));
        state.frame = IDLE_FRAMES;
        assert!(!state.save_at(now + std::time::Duration::from_secs(59)));
        assert_eq!(state.retry_frame, IDLE_FRAMES * 2);
        state.frame = IDLE_FRAMES * 2;
        assert!(state.save_at(now + std::time::Duration::from_secs(60)));
        fs::remove_dir(&path).unwrap();
        state.frame = IDLE_FRAMES * 3;
        assert!(!state.save_at(now + std::time::Duration::from_secs(61)));
        fs::remove_file(path).unwrap();
    }
}
