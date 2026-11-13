use crate::yoake_auth::YoakeAuth;
use crate::yoake_feed::{
    self, FeedClient, FeedIndexV2, PackVersionV2, ParsedFeedIndex, Policy, VerifiedFeed,
};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::ffi::OsStr;
use std::fmt;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;
use url::Url;

#[path = "yoake_install_fs.rs"]
mod secure_fs;

use secure_fs::Dir;

const MANIFEST: &str = "yoake-pack.json";
const PREVIOUS_MANIFEST: &str = "yoake-pack.previous.json";
const MOD_LIST: &str = "horizon-mods.txt";
const STAGING: &str = ".yoake-staging";
const JOURNAL: &str = "journal.json";
const MAX_METADATA: u64 = 8 * 1024 * 1024;
const MAX_FILE: u64 = 8 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallError {
    UnsupportedSchema,
    InvalidFeed,
    PackNotFound,
    VersionNotFound,
    UnsupportedTarget,
    LauncherTooOld,
    InvalidMetadata,
    RecoveryRequired,
    Conflict(String),
    BaseConflict(String),
    UnsafePath,
    NeedsLogin,
    FeedRequired,
    NoRollback,
    MissingBase,
    LaunchMetadataUnavailable,
    UnsupportedNativeMods,
    Authentication,
    HttpStatus(u16),
    Network,
    InvalidObject,
    Cancelled,
    Busy,
    Io(String),
}

impl fmt::Display for InstallError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for InstallError {}

impl From<io::Error> for InstallError {
    fn from(error: io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedFile {
    pub destination: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstalledPack {
    pub pack_id: String,
    pub pack_version: String,
    pub base_version: String,
    pub sequence: u64,
    pub target: String,
    pub managed_files: Vec<ManagedFile>,
    pub transaction: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_index: Option<FeedIndexV2>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rollback_proof: Option<yoake_feed::FeedProof>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileScope {
    Base,
    Instance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallPolicy {
    Managed,
    DefaultOnCreate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    Install,
    Skip,
    PreserveDefault,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    pub scope: FileScope,
    pub destination: String,
    pub sha256: String,
    pub size: u64,
    pub policy: InstallPolicy,
    pub action: FileAction,
    expected_before: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    pub destination: String,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub pack_id: String,
    pub pack_version: String,
    pub base_version: String,
    pub sequence: u64,
    pub target: String,
    pub files: Vec<PlannedFile>,
    pub removals: Vec<Removal>,
    rollback_index: FeedIndexV2,
    rollback_proof: Option<yoake_feed::FeedProof>,
    previous: Option<InstalledPack>,
    instance_dir: PathBuf,
    data_dir: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub struct InstallSnapshot {
    pub previous: Option<InstalledPack>,
    pub instance_files: BTreeMap<String, String>,
    pub base_files: BTreeMap<String, String>,
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn valid_transaction(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_installed(manifest: &InstalledPack) -> Result<(), InstallError> {
    for component in [
        &manifest.pack_id,
        &manifest.pack_version,
        &manifest.base_version,
        &manifest.target,
    ] {
        yoake_feed::validate_destination(component).map_err(|_| InstallError::InvalidMetadata)?;
        if component.contains('/') {
            return Err(InstallError::InvalidMetadata);
        }
    }
    if manifest.sequence == 0 || !valid_transaction(&manifest.transaction) {
        return Err(InstallError::InvalidMetadata);
    }
    let mut paths = BTreeSet::new();
    for file in &manifest.managed_files {
        yoake_feed::validate_pack_destination(&file.destination, Policy::Managed)
            .map_err(|_| InstallError::InvalidMetadata)?;
        if !valid_hash(&file.sha256) || !paths.insert(file.destination.to_ascii_lowercase()) {
            return Err(InstallError::InvalidMetadata);
        }
    }
    if let Some(index) = &manifest.rollback_index {
        yoake_feed::validate_index_v2(index).map_err(|_| InstallError::InvalidMetadata)?;
        if index.sequence != manifest.sequence || index.packs.len() != 1 || index.bases.len() != 1 {
            return Err(InstallError::InvalidMetadata);
        }
        let pack = &index.packs[0];
        if pack.id != manifest.pack_id || pack.versions.len() != 1 {
            return Err(InstallError::InvalidMetadata);
        }
        let version = &pack.versions[0];
        if version.version != manifest.pack_version
            || !version
                .base_version
                .eq_ignore_ascii_case(&manifest.base_version)
            || index.bases[0].version != manifest.base_version
            || !version.targets.contains_key(&manifest.target)
        {
            return Err(InstallError::InvalidMetadata);
        }
        let expected: BTreeSet<_> = version.targets[&manifest.target]
            .files
            .iter()
            .filter(|file| matches!(file.policy, Policy::Managed))
            .map(|file| (&file.destination, &file.sha256))
            .collect();
        let actual: BTreeSet<_> = manifest
            .managed_files
            .iter()
            .map(|file| (&file.destination, &file.sha256))
            .collect();
        if expected != actual {
            return Err(InstallError::InvalidMetadata);
        }
    }
    Ok(())
}

fn read_limited(mut file: File) -> Result<Vec<u8>, InstallError> {
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_METADATA + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_METADATA {
        return Err(InstallError::InvalidMetadata);
    }
    Ok(bytes)
}

fn parent_dir(
    root: &Path,
    destination: &str,
    create: bool,
) -> Result<Option<(Dir, String)>, InstallError> {
    yoake_feed::validate_destination(destination).map_err(|_| InstallError::UnsafePath)?;
    let mut parts: Vec<_> = destination.split('/').collect();
    let filename = parts.pop().ok_or(InstallError::UnsafePath)?.to_owned();
    let mut dir = match Dir::open(root, create) {
        Ok(dir) => dir,
        Err(error) if !create && error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    for part in parts {
        dir = match dir.child(OsStr::new(part), create) {
            Ok(dir) => dir,
            Err(error) if !create && error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
    }
    Ok(Some((dir, filename)))
}

fn read_file(root: &Path, destination: &str) -> Result<Option<File>, InstallError> {
    let Some((dir, filename)) = parent_dir(root, destination, false)? else {
        return Ok(None);
    };
    Ok(dir.read(OsStr::new(&filename))?)
}

fn file_hash(mut file: File) -> Result<String, InstallError> {
    if file.metadata()?.len() > MAX_FILE {
        return Err(InstallError::InvalidObject);
    }
    let mut hasher = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        total = total
            .checked_add(count as u64)
            .ok_or(InstallError::InvalidObject)?;
        if total > MAX_FILE {
            return Err(InstallError::InvalidObject);
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub trait ObjectSource: Send + Sync {
    fn open(&self, sha256: &str) -> Result<Box<dyn Read + Send>, InstallError>;
}

struct CacheOnlySource;

impl ObjectSource for CacheOnlySource {
    fn open(&self, _sha256: &str) -> Result<Box<dyn Read + Send>, InstallError> {
        Err(InstallError::FeedRequired)
    }
}

pub struct HttpObjectSource {
    root: Url,
    auth: Arc<dyn YoakeAuth>,
}

impl HttpObjectSource {
    pub fn from_feed_client(client: &FeedClient) -> Self {
        let (root, auth, _) = client.install_connection();
        Self { root, auth }
    }
}

impl ObjectSource for HttpObjectSource {
    fn open(&self, sha256: &str) -> Result<Box<dyn Read + Send>, InstallError> {
        if !valid_hash(sha256) {
            return Err(InstallError::InvalidObject);
        }
        let url = self
            .root
            .join(&format!("objects/sha256/{sha256}"))
            .map_err(|_| InstallError::InvalidFeed)?;
        let builder = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(15))
            .timeout_read(Duration::from_secs(30));
        let agent = crate::proxy::agent(builder, url.as_str()).build();
        let mut request = agent.get(url.as_str());
        if let Some(authorization) = self
            .auth
            .authorization()
            .map_err(|_| InstallError::Authentication)?
        {
            request = request.set("Authorization", &authorization);
        }
        let response = match request.call() {
            Ok(response) => response,
            Err(ureq::Error::Status(401 | 403, _)) => return Err(InstallError::NeedsLogin),
            Err(ureq::Error::Status(status, _)) => return Err(InstallError::HttpStatus(status)),
            Err(ureq::Error::Transport(_)) => return Err(InstallError::Network),
        };
        if !(200..300).contains(&response.status()) {
            return Err(InstallError::HttpStatus(response.status()));
        }
        Ok(response.into_reader())
    }
}

#[derive(Debug, Clone)]
pub enum InstallEvent {
    Download {
        bytes_done: u64,
        total_bytes: u64,
        current_file: String,
    },
    Applying {
        current_file: String,
    },
    Finished(Result<InstalledPack, InstallError>),
}

pub struct InstallTask {
    pub events: mpsc::Receiver<InstallEvent>,
    cancelled: Arc<AtomicBool>,
}

impl InstallTask {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<(), InstallError> {
    if cancelled.load(Ordering::SeqCst) {
        Err(InstallError::Cancelled)
    } else {
        Ok(())
    }
}

fn object_matches(file: File, sha256: &str, size: u64) -> Result<bool, InstallError> {
    if file.metadata()?.len() != size {
        return Ok(false);
    }
    Ok(file_hash(file)? == sha256)
}

fn missing_objects(plan: &InstallPlan) -> Result<(u64, bool), InstallError> {
    let cache = plan.data_dir.join("yoake-cache").join("objects");
    let mut objects = BTreeMap::new();
    for file in plan
        .files
        .iter()
        .filter(|file| file.action == FileAction::Install)
    {
        if let Some(size) = objects.insert(&file.sha256, file.size) {
            if size != file.size {
                return Err(InstallError::InvalidFeed);
            }
        }
    }
    let mut total = 0_u64;
    let mut missing = false;
    for (sha256, size) in objects {
        let valid = match read_file(&cache, sha256)? {
            Some(file) => object_matches(file, sha256, size)?,
            None => false,
        };
        if !valid {
            missing = true;
            total = total.checked_add(size).ok_or(InstallError::InvalidFeed)?;
        }
    }
    Ok((total, missing))
}

pub fn download_bytes(plan: &InstallPlan) -> Result<u64, InstallError> {
    Ok(missing_objects(plan)?.0)
}

fn cache_object(
    cache: &Path,
    file: &PlannedFile,
    source: &dyn ObjectSource,
    cancelled: &AtomicBool,
    done: &AtomicU64,
    total: u64,
    events: &mpsc::Sender<InstallEvent>,
) -> Result<(), InstallError> {
    check_cancelled(cancelled)?;
    let directory = Dir::open(cache, true)?;
    let hash = OsStr::new(&file.sha256);
    let lock_name = format!("{}.lock", file.sha256);
    let lock = directory.lock_file(OsStr::new(&lock_name))?;
    lock.try_lock().map_err(|_| InstallError::Busy)?;
    let partial = format!("{}.partial", file.sha256);
    directory.remove(OsStr::new(&partial))?;
    if let Some(existing) = directory.read(hash)? {
        if object_matches(existing, &file.sha256, file.size)? {
            let bytes_done = done.fetch_add(file.size, Ordering::SeqCst) + file.size;
            let _ = events.send(InstallEvent::Download {
                bytes_done,
                total_bytes: total,
                current_file: file.destination.clone(),
            });
            return Ok(());
        }
        directory.remove(hash)?;
    }
    let result = (|| {
        let mut input = source.open(&file.sha256)?;
        let mut output = directory.create(OsStr::new(&partial))?;
        let mut digest = Sha256::new();
        let mut count = 0_u64;
        let mut buffer = [0_u8; 65536];
        loop {
            check_cancelled(cancelled)?;
            let read = input.read(&mut buffer).map_err(|_| InstallError::Network)?;
            if read == 0 {
                break;
            }
            count = count
                .checked_add(read as u64)
                .ok_or(InstallError::InvalidObject)?;
            if count > file.size {
                return Err(InstallError::InvalidObject);
            }
            output.write_all(&buffer[..read])?;
            digest.update(&buffer[..read]);
            let bytes_done = done.fetch_add(read as u64, Ordering::SeqCst) + read as u64;
            let _ = events.send(InstallEvent::Download {
                bytes_done,
                total_bytes: total,
                current_file: file.destination.clone(),
            });
        }
        if count != file.size || format!("{:x}", digest.finalize()) != file.sha256 {
            return Err(InstallError::InvalidObject);
        }
        output.sync_all()?;
        drop(output);
        check_cancelled(cancelled)?;
        directory.rename_new(OsStr::new(&partial), hash)?;
        directory.sync()?;
        Ok(())
    })();
    directory.remove(OsStr::new(&partial))?;
    result
}

fn download_objects(
    plan: &InstallPlan,
    source: &dyn ObjectSource,
    cancelled: &AtomicBool,
    events: &mpsc::Sender<InstallEvent>,
) -> Result<(), InstallError> {
    let mut unique: BTreeMap<String, PlannedFile> = BTreeMap::new();
    for file in plan
        .files
        .iter()
        .filter(|file| file.action == FileAction::Install)
    {
        if let Some(previous) = unique.insert(file.sha256.clone(), file.clone()) {
            if previous.size != file.size {
                return Err(InstallError::InvalidFeed);
            }
        }
    }
    let total = unique
        .values()
        .try_fold(0_u64, |sum, file| sum.checked_add(file.size))
        .ok_or(InstallError::InvalidFeed)?;
    let queue = Mutex::new(VecDeque::from_iter(unique.into_values()));
    let errors = Mutex::new(Vec::new());
    let stop = AtomicBool::new(false);
    let done = AtomicU64::new(0);
    let cache = plan.data_dir.join("yoake-cache").join("objects");
    thread::scope(|scope| {
        for _ in 0..2 {
            scope.spawn(|| loop {
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                let file = queue.lock().expect("download queue poisoned").pop_front();
                let Some(file) = file else { break };
                if let Err(error) =
                    cache_object(&cache, &file, source, cancelled, &done, total, events)
                {
                    stop.store(true, Ordering::SeqCst);
                    errors.lock().expect("download errors poisoned").push(error);
                    break;
                }
            });
        }
    });
    if let Some(error) = errors
        .into_inner()
        .expect("download errors poisoned")
        .into_iter()
        .next()
    {
        return Err(error);
    }
    check_cancelled(cancelled)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    transaction: String,
    operations: Vec<JournalOperation>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JournalOperation {
    destination: String,
    before: Option<String>,
    after: Option<String>,
    staged: Option<String>,
    backup: String,
    default_on_create: bool,
}

fn validate_journal(journal: &Journal) -> Result<(), InstallError> {
    if !valid_transaction(&journal.transaction) || journal.operations.is_empty() {
        return Err(InstallError::InvalidMetadata);
    }
    let mut destinations = BTreeSet::new();
    for (index, operation) in journal.operations.iter().enumerate() {
        let is_manifest = index + 1 == journal.operations.len();
        let is_previous_manifest = operation.destination == PREVIOUS_MANIFEST;
        let is_mod_list = operation.destination == MOD_LIST;
        if is_manifest != (operation.destination == MANIFEST) {
            return Err(InstallError::InvalidMetadata);
        }
        if is_previous_manifest
            && (index + 2 != journal.operations.len()
                || operation.default_on_create
                || operation.after.is_none())
        {
            return Err(InstallError::InvalidMetadata);
        }
        if is_mod_list && (operation.default_on_create || operation.after.is_none()) {
            return Err(InstallError::InvalidMetadata);
        }
        if !is_manifest && !is_previous_manifest && !is_mod_list {
            let policy = if operation.default_on_create {
                Policy::DefaultOnCreate
            } else {
                Policy::Managed
            };
            yoake_feed::validate_pack_destination(&operation.destination, policy)
                .map_err(|_| InstallError::InvalidMetadata)?;
        }
        if !destinations.insert(operation.destination.to_ascii_lowercase())
            || operation
                .before
                .as_ref()
                .is_some_and(|hash| !valid_hash(hash))
            || operation
                .after
                .as_ref()
                .is_some_and(|hash| !valid_hash(hash))
            || operation.backup != format!("old-{}-{index}", journal.transaction)
            || operation
                .staged
                .as_ref()
                .is_some_and(|name| name != &format!("new-{}-{index}", journal.transaction))
            || operation.staged.is_some() != operation.after.is_some()
            || (operation.default_on_create && operation.before.is_some())
            || (is_manifest && operation.after.is_none())
        {
            return Err(InstallError::InvalidMetadata);
        }
    }
    Ok(())
}

fn transaction_id() -> Result<String, InstallError> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| InstallError::Io("randomness unavailable".to_owned()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn write_staged(directory: &Dir, name: &str, bytes: &[u8]) -> Result<String, InstallError> {
    let mut file = directory.create(OsStr::new(name))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn copy_object_to(
    cache: &Path,
    source: &PlannedFile,
    directory: &Dir,
    name: &str,
    cancelled: &AtomicBool,
) -> Result<(), InstallError> {
    let cache_dir = Dir::open(cache, false)?;
    let mut input = cache_dir
        .read(OsStr::new(&source.sha256))?
        .ok_or(InstallError::InvalidObject)?;
    let mut output = directory.create(OsStr::new(name))?;
    let mut digest = Sha256::new();
    let mut count = 0_u64;
    let mut buffer = [0_u8; 65536];
    loop {
        check_cancelled(cancelled)?;
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or(InstallError::InvalidObject)?;
        if count > source.size {
            return Err(InstallError::InvalidObject);
        }
        output.write_all(&buffer[..read])?;
        digest.update(&buffer[..read]);
    }
    if count != source.size || format!("{:x}", digest.finalize()) != source.sha256 {
        return Err(InstallError::InvalidObject);
    }
    output.sync_all()?;
    Ok(())
}

fn prepare_base(plan: &InstallPlan, cancelled: &AtomicBool) -> Result<(), InstallError> {
    let cache = plan.data_dir.join("yoake-cache").join("objects");
    let base = plan.data_dir.join("yoake-bases").join(&plan.base_version);
    let directory = Dir::open(&base, true)?;
    for file in plan
        .files
        .iter()
        .filter(|file| file.scope == FileScope::Base)
    {
        check_cancelled(cancelled)?;
        if let Some(existing) = directory.read(OsStr::new(&file.destination))? {
            if !object_matches(existing, &file.sha256, file.size)? {
                return Err(InstallError::BaseConflict(file.destination.clone()));
            }
            continue;
        }
        if file.action == FileAction::Skip {
            return Err(InstallError::BaseConflict(file.destination.clone()));
        }
        let name = format!("base-{}", transaction_id()?);
        let result = (|| {
            copy_object_to(&cache, file, &directory, &name, cancelled)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;

                let staged = directory
                    .read(OsStr::new(&name))?
                    .ok_or(InstallError::InvalidObject)?;
                staged.set_permissions(std::fs::Permissions::from_mode(0o700))?;
                staged.sync_all()?;
            }
            match directory.link_new(OsStr::new(&name), &directory, OsStr::new(&file.destination)) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let existing = directory
                        .read(OsStr::new(&file.destination))?
                        .ok_or(InstallError::RecoveryRequired)?;
                    if !object_matches(existing, &file.sha256, file.size)? {
                        return Err(InstallError::BaseConflict(file.destination.clone()));
                    }
                }
                Err(error) => return Err(error.into()),
            }
            directory.sync()?;
            Ok(())
        })();
        directory.remove(OsStr::new(&name))?;
        result?;
    }
    Ok(())
}

fn cleanup_journal(staging: &Dir, journal: &Journal) -> Result<(), InstallError> {
    for operation in &journal.operations {
        if let Some(name) = &operation.staged {
            staging.remove(OsStr::new(name))?;
        }
        staging.remove(OsStr::new(&operation.backup))?;
    }
    staging.remove(OsStr::new(JOURNAL))?;
    staging.sync()?;
    Ok(())
}

fn recover_locked(
    instance: &Path,
    staging: &Dir,
    rollback_committed: bool,
) -> Result<(), InstallError> {
    let Some(file) = staging.read(OsStr::new(JOURNAL))? else {
        return Ok(());
    };
    let journal: Journal =
        serde_json::from_slice(&read_limited(file)?).map_err(|_| InstallError::InvalidMetadata)?;
    validate_journal(&journal)?;
    if !rollback_committed
        && read_manifest(instance)?
            .is_some_and(|manifest| manifest.transaction == journal.transaction)
    {
        return cleanup_journal(staging, &journal);
    }
    for operation in journal.operations.iter().rev() {
        let backup = staging.read(OsStr::new(&operation.backup))?;
        let Some((directory, filename)) =
            parent_dir(instance, &operation.destination, backup.is_some())?
        else {
            if operation.before.is_some() {
                return Err(InstallError::RecoveryRequired);
            }
            continue;
        };
        let current_file = directory.read(OsStr::new(&filename))?;
        let staged_file = match &operation.staged {
            Some(name) => staging.read(OsStr::new(name))?,
            None => None,
        };
        let is_staged = match (&current_file, &staged_file) {
            (Some(current), Some(staged)) => secure_fs::same_file(current, staged)?,
            _ => false,
        };
        let is_backup = match (&current_file, &backup) {
            (Some(current), Some(backup)) => secure_fs::same_file(current, backup)?,
            _ => false,
        };
        let current = current_file.map(file_hash).transpose()?;
        drop(staged_file);
        if let Some(backup_file) = &backup {
            if Some(file_hash(backup_file.try_clone()?)?) != operation.before {
                return Err(InstallError::RecoveryRequired);
            }
            if is_backup {
                drop(backup);
                continue;
            }
            if current.is_some() {
                if !is_staged || current != operation.after {
                    return Err(InstallError::RecoveryRequired);
                }
                directory.remove(OsStr::new(&filename))?;
            }
            drop(backup);
            staging.link_new(
                OsStr::new(&operation.backup),
                &directory,
                OsStr::new(&filename),
            )?;
            directory.sync()?;
        } else if operation.before.is_none() {
            if is_staged {
                if current != operation.after {
                    return Err(InstallError::RecoveryRequired);
                }
                directory.remove(OsStr::new(&filename))?;
                directory.sync()?;
            }
        } else if current != operation.before {
            return Err(InstallError::RecoveryRequired);
        }
    }
    cleanup_journal(staging, &journal)
}

pub fn recover_install(instance: &Path) -> Result<(), InstallError> {
    let instance_dir = Dir::open(instance, true)?;
    let staging = instance_dir.child(OsStr::new(STAGING), true)?;
    let lock = staging.lock_file(OsStr::new("install.lock"))?;
    lock.try_lock().map_err(|_| InstallError::Busy)?;
    recover_locked(instance, &staging, false)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchProgram {
    pub executable: PathBuf,
    pub resource_tag: String,
}

pub fn has_manifest_or_pending(instance: &Path) -> bool {
    [
        instance.join(MANIFEST),
        instance.join(STAGING).join(JOURNAL),
    ]
    .iter()
    .any(|path| match std::fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != io::ErrorKind::NotFound,
    })
}

pub fn prepare_launch(
    instance: &Path,
    data_dir: &Path,
) -> Result<Option<LaunchProgram>, InstallError> {
    match std::fs::symlink_metadata(instance.join(STAGING)) {
        Ok(_) => recover_install(instance)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    match std::fs::symlink_metadata(instance.join(MANIFEST)) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let Some(manifest) = read_manifest(instance)? else {
        return Ok(None);
    };
    let stored_index = manifest
        .rollback_index
        .as_ref()
        .ok_or(InstallError::LaunchMetadataUnavailable)?;
    let proof = manifest
        .rollback_proof
        .as_ref()
        .ok_or(InstallError::LaunchMetadataUnavailable)?;
    let verified =
        yoake_feed::verify_saved_feed(proof).map_err(|_| InstallError::InvalidMetadata)?;
    let index = schema(&verified)?;
    let version = select(index, &manifest.pack_id, &manifest.pack_version)?;
    let base = index
        .bases
        .iter()
        .find(|base| base.version == manifest.base_version)
        .ok_or(InstallError::InvalidMetadata)?;
    if index.sequence != manifest.sequence
        || version.base_version != manifest.base_version
        || stored_index.source_id != index.source_id
        || &stored_index.packs[0].versions[0] != version
        || &stored_index.bases[0] != base
    {
        return Err(InstallError::InvalidMetadata);
    }
    let target = base
        .targets
        .get(&manifest.target)
        .ok_or(InstallError::InvalidMetadata)?;
    let [base_file] = target.files.as_slice() else {
        return Err(InstallError::InvalidMetadata);
    };
    let base_dir = data_dir.join("yoake-bases").join(&manifest.base_version);
    let file = read_file(&base_dir, &base_file.destination)?.ok_or(InstallError::MissingBase)?;
    if !object_matches(file, &base_file.sha256, base_file.size)? {
        return Err(InstallError::InvalidObject);
    }
    if !crate::yoake_feed::same_native_mods(&version.enabled_native_mods, &base.native_mods) {
        return Err(InstallError::UnsupportedNativeMods);
    }
    Ok(Some(LaunchProgram {
        executable: base_dir.join(&base_file.destination),
        resource_tag: base.horizon_release.clone(),
    }))
}

fn apply_plan(
    plan: &InstallPlan,
    staging: &Dir,
    cancelled: &AtomicBool,
    events: &mpsc::Sender<InstallEvent>,
) -> Result<InstalledPack, InstallError> {
    let transaction = transaction_id()?;
    let cache = plan.data_dir.join("yoake-cache").join("objects");
    let mut journal = Journal {
        transaction: transaction.clone(),
        operations: Vec::new(),
    };
    let mut temporary = Vec::new();
    let prepare = (|| {
        let previous_bytes = read_file(&plan.instance_dir, MANIFEST)?
            .map(read_limited)
            .transpose()?;
        let previous = previous_bytes
            .as_ref()
            .map(|bytes| {
                let manifest =
                    serde_json::from_slice(bytes).map_err(|_| InstallError::InvalidMetadata)?;
                validate_installed(&manifest)?;
                Ok::<_, InstallError>(manifest)
            })
            .transpose()?;
        if previous != plan.previous {
            return Err(InstallError::Conflict(MANIFEST.to_owned()));
        }
        let version_changed = previous.as_ref().is_some_and(|manifest| {
            manifest.pack_id != plan.pack_id || manifest.pack_version != plan.pack_version
        });
        let manifest_before = previous_bytes
            .as_ref()
            .map(|bytes| format!("{:x}", Sha256::digest(bytes)));
        for file in plan
            .files
            .iter()
            .filter(|file| file.scope == FileScope::Instance)
        {
            check_cancelled(cancelled)?;
            let now = hash_at(&plan.instance_dir, &file.destination)?;
            if file.policy == InstallPolicy::Managed && now != file.expected_before {
                return Err(InstallError::Conflict(file.destination.clone()));
            }
            if file.action != FileAction::Install
                || (file.policy == InstallPolicy::DefaultOnCreate && now.is_some())
            {
                continue;
            }
            let index = journal.operations.len();
            let name = format!("new-{transaction}-{index}");
            temporary.push(name.clone());
            copy_object_to(&cache, file, staging, &name, cancelled)?;
            journal.operations.push(JournalOperation {
                destination: file.destination.clone(),
                before: now,
                after: Some(file.sha256.clone()),
                staged: Some(name),
                backup: format!("old-{transaction}-{index}"),
                default_on_create: file.policy == InstallPolicy::DefaultOnCreate,
            });
        }
        for removal in &plan.removals {
            if hash_at(&plan.instance_dir, &removal.destination)? != Some(removal.sha256.clone()) {
                return Err(InstallError::Conflict(removal.destination.clone()));
            }
            let index = journal.operations.len();
            journal.operations.push(JournalOperation {
                destination: removal.destination.clone(),
                before: Some(removal.sha256.clone()),
                after: None,
                staged: None,
                backup: format!("old-{transaction}-{index}"),
                default_on_create: false,
            });
        }
        let index = journal.operations.len();
        let name = format!("new-{transaction}-{index}");
        temporary.push(name.clone());
        let content_mods = select(&plan.rollback_index, &plan.pack_id, &plan.pack_version)?
            .enabled_content_mods
            .iter()
            .map(|id| format!("mod.{id}:true\n"))
            .collect::<String>();
        let after = write_staged(staging, &name, content_mods.as_bytes())?;
        journal.operations.push(JournalOperation {
            destination: MOD_LIST.to_owned(),
            before: hash_at(&plan.instance_dir, MOD_LIST)?,
            after: Some(after),
            staged: Some(name),
            backup: format!("old-{transaction}-{index}"),
            default_on_create: false,
        });
        if let Some(bytes) = previous_bytes.filter(|_| version_changed) {
            let index = journal.operations.len();
            let name = format!("new-{transaction}-{index}");
            temporary.push(name.clone());
            let after = write_staged(staging, &name, &bytes)?;
            journal.operations.push(JournalOperation {
                destination: PREVIOUS_MANIFEST.to_owned(),
                before: hash_at(&plan.instance_dir, PREVIOUS_MANIFEST)?,
                after: Some(after),
                staged: Some(name),
                backup: format!("old-{transaction}-{index}"),
                default_on_create: false,
            });
        }
        let manifest = InstalledPack {
            pack_id: plan.pack_id.clone(),
            pack_version: plan.pack_version.clone(),
            base_version: plan.base_version.clone(),
            sequence: plan.sequence,
            target: plan.target.clone(),
            managed_files: plan
                .files
                .iter()
                .filter(|file| {
                    file.scope == FileScope::Instance && file.policy == InstallPolicy::Managed
                })
                .map(|file| ManagedFile {
                    destination: file.destination.clone(),
                    sha256: file.sha256.clone(),
                })
                .collect(),
            transaction: transaction.clone(),
            rollback_index: Some(plan.rollback_index.clone()),
            rollback_proof: plan.rollback_proof.clone(),
        };
        let index = journal.operations.len();
        let name = format!("new-{transaction}-{index}");
        temporary.push(name.clone());
        let bytes =
            serde_json::to_vec_pretty(&manifest).map_err(|_| InstallError::InvalidMetadata)?;
        if bytes.len() as u64 > MAX_METADATA {
            return Err(InstallError::InvalidMetadata);
        }
        let after = write_staged(staging, &name, &bytes)?;
        journal.operations.push(JournalOperation {
            destination: MANIFEST.to_owned(),
            before: manifest_before,
            after: Some(after),
            staged: Some(name),
            backup: format!("old-{transaction}-{index}"),
            default_on_create: false,
        });
        validate_journal(&journal)?;
        let journal_bytes =
            serde_json::to_vec(&journal).map_err(|_| InstallError::InvalidMetadata)?;
        if journal_bytes.len() as u64 > MAX_METADATA {
            return Err(InstallError::InvalidMetadata);
        }
        let journal_temp = format!("journal-{transaction}");
        temporary.push(journal_temp.clone());
        write_staged(staging, &journal_temp, &journal_bytes)?;
        staging.link_new(OsStr::new(&journal_temp), staging, OsStr::new(JOURNAL))?;
        staging.sync()?;
        staging.remove(OsStr::new(&journal_temp))?;
        Ok(manifest)
    })();
    let manifest = match prepare {
        Ok(manifest) => manifest,
        Err(error) => {
            if staging.read(OsStr::new(JOURNAL))?.is_some() {
                recover_locked(&plan.instance_dir, staging, false)?;
            }
            for name in temporary {
                staging.remove(OsStr::new(&name))?;
            }
            return Err(error);
        }
    };
    let commit = (|| {
        for operation in &journal.operations {
            check_cancelled(cancelled)?;
            let _ = events.send(InstallEvent::Applying {
                current_file: operation.destination.clone(),
            });
            let (directory, filename) =
                parent_dir(&plan.instance_dir, &operation.destination, true)?
                    .ok_or(InstallError::UnsafePath)?;
            let now = directory
                .read(OsStr::new(&filename))?
                .map(file_hash)
                .transpose()?;
            if now != operation.before {
                return Err(InstallError::Conflict(operation.destination.clone()));
            }
            if operation.before.is_some() {
                directory.link_new(
                    OsStr::new(&filename),
                    staging,
                    OsStr::new(&operation.backup),
                )?;
                staging.sync()?;
                let original = directory
                    .read(OsStr::new(&filename))?
                    .ok_or(InstallError::RecoveryRequired)?;
                let backup = staging
                    .read(OsStr::new(&operation.backup))?
                    .ok_or(InstallError::RecoveryRequired)?;
                if !secure_fs::same_file(&original, &backup)? {
                    return Err(InstallError::RecoveryRequired);
                }
                drop(original);
                drop(backup);
                directory.remove(OsStr::new(&filename))?;
                directory.sync()?;
            }
            if let Some(name) = &operation.staged {
                staging.link_new(OsStr::new(name), &directory, OsStr::new(&filename))?;
                directory.sync()?;
            }
        }
        Ok(())
    })();
    if let Err(error) = commit {
        recover_locked(&plan.instance_dir, staging, true)?;
        return Err(error);
    }
    let _ = cleanup_journal(staging, &journal);
    Ok(manifest)
}

pub fn start_install(
    client: &FeedClient,
    feed: VerifiedFeed,
    pack_id: String,
    pack_version: String,
    instance_dir: PathBuf,
) -> InstallTask {
    let (_, _, data_dir) = client.install_connection();
    let source = Arc::new(HttpObjectSource::from_feed_client(client));
    start_install_with_source(
        feed,
        pack_id,
        pack_version,
        instance_dir,
        data_dir,
        current_target(),
        env!("CARGO_PKG_VERSION").to_owned(),
        source,
    )
}

pub fn start_install_with_source(
    feed: VerifiedFeed,
    pack_id: String,
    pack_version: String,
    instance_dir: PathBuf,
    data_dir: PathBuf,
    target: Result<String, InstallError>,
    launcher_version: String,
    source: Arc<dyn ObjectSource>,
) -> InstallTask {
    let (sender, events) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    thread::spawn(move || {
        let result = (|| {
            if !instance_dir.is_absolute() || !data_dir.is_absolute() {
                return Err(InstallError::UnsafePath);
            }
            let root = Dir::open(&instance_dir, true)?;
            let staging = root.child(OsStr::new(STAGING), true)?;
            let lock = staging.lock_file(OsStr::new("install.lock"))?;
            lock.try_lock().map_err(|_| InstallError::Busy)?;
            recover_locked(&instance_dir, &staging, false)?;
            let plan = plan_install_at(
                &feed,
                &pack_id,
                &pack_version,
                &instance_dir,
                &data_dir,
                &target?,
                &launcher_version,
            )?;
            download_objects(&plan, source.as_ref(), &worker_cancelled, &sender)?;
            prepare_base(&plan, &worker_cancelled)?;
            apply_plan(&plan, &staging, &worker_cancelled, &sender)
        })();
        let _ = sender.send(InstallEvent::Finished(result));
    });
    InstallTask { events, cancelled }
}

pub fn start_rollback(
    client: Option<&FeedClient>,
    feed: Option<VerifiedFeed>,
    instance_dir: PathBuf,
) -> InstallTask {
    let (data_dir, source): (PathBuf, Arc<dyn ObjectSource>) = match client {
        Some(client) => (
            client.install_connection().2,
            Arc::new(HttpObjectSource::from_feed_client(client)),
        ),
        None => (crate::backend::data_dir(), Arc::new(CacheOnlySource)),
    };
    start_rollback_with_source(
        feed,
        instance_dir,
        data_dir,
        current_target(),
        env!("CARGO_PKG_VERSION").to_owned(),
        source,
    )
}

pub fn rollback(instance_dir: PathBuf) -> InstallTask {
    let (sender, events) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    thread::spawn(move || {
        let mut task = start_rollback(None, None, instance_dir.clone());
        let mut attempted_feed = false;
        loop {
            if worker_cancelled.load(Ordering::SeqCst) {
                task.cancel();
            }
            match task.events.recv_timeout(Duration::from_millis(100)) {
                Ok(InstallEvent::Finished(Err(InstallError::FeedRequired)))
                    if worker_cancelled.load(Ordering::SeqCst) =>
                {
                    let _ = sender.send(InstallEvent::Finished(Err(InstallError::Cancelled)));
                    break;
                }
                Ok(InstallEvent::Finished(Err(InstallError::FeedRequired)))
                    if !attempted_feed && !worker_cancelled.load(Ordering::SeqCst) =>
                {
                    attempted_feed = true;
                    let online = (|| {
                        let client = FeedClient::configured()
                            .map_err(|_| InstallError::InvalidFeed)?
                            .ok_or(InstallError::FeedRequired)?;
                        let (_, auth, _) = client.install_connection();
                        if auth
                            .authorization()
                            .map_err(|_| InstallError::Authentication)?
                            .is_none()
                        {
                            return Err(InstallError::NeedsLogin);
                        }
                        check_cancelled(&worker_cancelled)?;
                        let feed = client.fetch_feed().map_err(|error| match error {
                            yoake_feed::FeedError::NeedsLogin
                            | yoake_feed::FeedError::HttpStatus(401 | 403) => {
                                InstallError::NeedsLogin
                            }
                            yoake_feed::FeedError::Authentication(_) => {
                                InstallError::Authentication
                            }
                            yoake_feed::FeedError::HttpStatus(status) => {
                                InstallError::HttpStatus(status)
                            }
                            yoake_feed::FeedError::Network => InstallError::Network,
                            _ => InstallError::InvalidFeed,
                        })?;
                        check_cancelled(&worker_cancelled)?;
                        Ok(start_rollback(
                            Some(&client),
                            Some(feed),
                            instance_dir.clone(),
                        ))
                    })();
                    match online {
                        Ok(next) => task = next,
                        Err(error) => {
                            let _ = sender.send(InstallEvent::Finished(Err(error)));
                            break;
                        }
                    }
                }
                Ok(event @ InstallEvent::Finished(_)) => {
                    let _ = sender.send(event);
                    break;
                }
                Ok(event) => {
                    let _ = sender.send(event);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = sender.send(InstallEvent::Finished(Err(InstallError::Io(
                        "rollback worker stopped unexpectedly".to_owned(),
                    ))));
                    break;
                }
            }
        }
    });
    InstallTask { events, cancelled }
}

pub fn start_rollback_with_source(
    feed: Option<VerifiedFeed>,
    instance_dir: PathBuf,
    data_dir: PathBuf,
    target: Result<String, InstallError>,
    launcher_version: String,
    source: Arc<dyn ObjectSource>,
) -> InstallTask {
    let (sender, events) = mpsc::channel();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = Arc::clone(&cancelled);
    thread::spawn(move || {
        let result = (|| {
            if !instance_dir.is_absolute() || !data_dir.is_absolute() {
                return Err(InstallError::UnsafePath);
            }
            let root = Dir::open(&instance_dir, false)?;
            let staging = root.child(OsStr::new(STAGING), true)?;
            let lock = staging.lock_file(OsStr::new("install.lock"))?;
            lock.try_lock().map_err(|_| InstallError::Busy)?;
            recover_locked(&instance_dir, &staging, false)?;
            check_cancelled(&worker_cancelled)?;
            let file =
                read_file(&instance_dir, PREVIOUS_MANIFEST)?.ok_or(InstallError::NoRollback)?;
            let previous: InstalledPack = serde_json::from_slice(&read_limited(file)?)
                .map_err(|_| InstallError::InvalidMetadata)?;
            validate_installed(&previous)?;
            let target = target?;
            if previous.target != target {
                return Err(InstallError::UnsupportedTarget);
            }
            let snapshot_feed = match &previous.rollback_proof {
                Some(proof) => yoake_feed::verify_saved_feed(proof)
                    .map_err(|_| InstallError::InvalidMetadata)?,
                None => feed.clone().ok_or(InstallError::FeedRequired)?,
            };
            let saved_index = schema(&snapshot_feed)?.clone();
            if previous.rollback_proof.is_some() && saved_index.sequence != previous.sequence {
                return Err(InstallError::InvalidMetadata);
            }
            let plan = plan_install_at(
                &snapshot_feed,
                &previous.pack_id,
                &previous.pack_version,
                &instance_dir,
                &data_dir,
                &target,
                &launcher_version,
            )?;
            if plan.base_version != previous.base_version {
                return Err(InstallError::InvalidMetadata);
            }
            let cache_only = CacheOnlySource;
            let download_source: &dyn ObjectSource = if missing_objects(&plan)?.1 {
                let live = schema(feed.as_ref().ok_or(InstallError::FeedRequired)?)?;
                let live_version = select(live, &previous.pack_id, &previous.pack_version)?;
                let saved_version =
                    select(&saved_index, &previous.pack_id, &previous.pack_version)?;
                let live_base = live
                    .bases
                    .iter()
                    .find(|base| base.version == plan.base_version);
                let saved_base = saved_index
                    .bases
                    .iter()
                    .find(|base| base.version == plan.base_version);
                if live_version != saved_version || live_base != saved_base {
                    return Err(InstallError::InvalidFeed);
                }
                source.as_ref()
            } else {
                &cache_only
            };
            download_objects(&plan, download_source, &worker_cancelled, &sender)?;
            prepare_base(&plan, &worker_cancelled)?;
            apply_plan(&plan, &staging, &worker_cancelled, &sender)
        })();
        let _ = sender.send(InstallEvent::Finished(result));
    });
    InstallTask { events, cancelled }
}

fn hash_at(root: &Path, destination: &str) -> Result<Option<String>, InstallError> {
    read_file(root, destination)?.map(file_hash).transpose()
}

pub fn read_manifest(instance: &Path) -> Result<Option<InstalledPack>, InstallError> {
    let Some(file) = read_file(instance, MANIFEST)? else {
        return Ok(None);
    };
    let manifest: InstalledPack =
        serde_json::from_slice(&read_limited(file)?).map_err(|_| InstallError::InvalidMetadata)?;
    validate_installed(&manifest)?;
    Ok(Some(manifest))
}

fn schema(feed: &VerifiedFeed) -> Result<&FeedIndexV2, InstallError> {
    let ParsedFeedIndex::V2(index) = &feed.index else {
        return Err(InstallError::UnsupportedSchema);
    };
    yoake_feed::validate_index_v2(index).map_err(|_| InstallError::InvalidFeed)?;
    Ok(index)
}

fn select<'a>(
    index: &'a FeedIndexV2,
    pack_id: &str,
    version: &str,
) -> Result<&'a PackVersionV2, InstallError> {
    index
        .packs
        .iter()
        .find(|pack| pack.id == pack_id)
        .ok_or(InstallError::PackNotFound)?
        .versions
        .iter()
        .find(|candidate| candidate.version == version)
        .ok_or(InstallError::VersionNotFound)
}

pub fn launcher_version(value: &str) -> Result<Version, InstallError> {
    if let Ok(version) = Version::parse(value) {
        return Ok(version);
    }
    let parts: Vec<_> = value.split('.').collect();
    if !(1..=2).contains(&parts.len())
        || parts
            .iter()
            .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return Err(InstallError::InvalidFeed);
    }
    let expanded = if parts.len() == 1 {
        format!("{value}.0.0")
    } else {
        format!("{value}.0")
    };
    Version::parse(&expanded).map_err(|_| InstallError::InvalidFeed)
}

pub fn current_target() -> Result<String, InstallError> {
    let os = match std::env::consts::OS {
        "windows" => "windows",
        "linux" => "linux",
        "macos" => "macos",
        _ => return Err(InstallError::UnsupportedTarget),
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return Err(InstallError::UnsupportedTarget),
    };
    Ok(format!("{os}-{arch}"))
}

pub fn plan_install(
    feed: &VerifiedFeed,
    pack_id: &str,
    pack_version: &str,
    instance_dir: &Path,
) -> Result<InstallPlan, InstallError> {
    plan_install_at(
        feed,
        pack_id,
        pack_version,
        instance_dir,
        &crate::backend::data_dir(),
        &current_target()?,
        env!("CARGO_PKG_VERSION"),
    )
}

pub fn plan_install_at(
    feed: &VerifiedFeed,
    pack_id: &str,
    pack_version: &str,
    instance_dir: &Path,
    data_dir: &Path,
    target: &str,
    current_launcher: &str,
) -> Result<InstallPlan, InstallError> {
    let index = schema(feed)?;
    let pack = select(index, pack_id, pack_version)?;
    let base = index
        .bases
        .iter()
        .find(|base| base.version.eq_ignore_ascii_case(&pack.base_version))
        .ok_or(InstallError::InvalidFeed)?;
    let base_target = base
        .targets
        .get(target)
        .ok_or(InstallError::UnsupportedTarget)?;
    let pack_target = pack
        .targets
        .get(target)
        .ok_or(InstallError::UnsupportedTarget)?;
    if read_file(instance_dir, &format!("{STAGING}/{JOURNAL}"))?.is_some() {
        return Err(InstallError::RecoveryRequired);
    }
    let previous = read_manifest(instance_dir)?;
    let mut snapshot = InstallSnapshot {
        previous,
        ..InstallSnapshot::default()
    };
    let mut paths = BTreeSet::new();
    paths.extend(
        pack_target
            .files
            .iter()
            .map(|file| file.destination.clone()),
    );
    if let Some(previous) = &snapshot.previous {
        paths.extend(
            previous
                .managed_files
                .iter()
                .map(|file| file.destination.clone()),
        );
    }
    for path in paths {
        if let Some(hash) = hash_at(instance_dir, &path)? {
            snapshot.instance_files.insert(path, hash);
        }
    }
    let base_dir = data_dir.join("yoake-bases").join(&base.version);
    for file in &base_target.files {
        if let Some(hash) = hash_at(&base_dir, &file.destination)? {
            snapshot.base_files.insert(file.destination.clone(), hash);
        }
    }
    plan_install_with_snapshot(
        feed,
        pack_id,
        pack_version,
        instance_dir,
        data_dir,
        target,
        current_launcher,
        &snapshot,
    )
}

pub fn plan_install_with_snapshot(
    feed: &VerifiedFeed,
    pack_id: &str,
    pack_version: &str,
    instance_dir: &Path,
    data_dir: &Path,
    target: &str,
    current_launcher: &str,
    snapshot: &InstallSnapshot,
) -> Result<InstallPlan, InstallError> {
    if !instance_dir.is_absolute() || !data_dir.is_absolute() {
        return Err(InstallError::UnsafePath);
    }
    let index = schema(feed)?;
    let pack = select(index, pack_id, pack_version)?;
    if launcher_version(current_launcher)? < launcher_version(&pack.minimum_launcher)? {
        return Err(InstallError::LauncherTooOld);
    }
    let base = index
        .bases
        .iter()
        .find(|base| base.version.eq_ignore_ascii_case(&pack.base_version))
        .ok_or(InstallError::InvalidFeed)?;
    let base_target = base
        .targets
        .get(target)
        .ok_or(InstallError::UnsupportedTarget)?;
    let pack_target = pack
        .targets
        .get(target)
        .ok_or(InstallError::UnsupportedTarget)?;
    if let Some(previous) = &snapshot.previous {
        validate_installed(previous)?;
        if previous.pack_id != pack_id {
            return Err(InstallError::Conflict(MANIFEST.to_owned()));
        }
    }
    let old: BTreeMap<_, _> = snapshot
        .previous
        .iter()
        .flat_map(|manifest| &manifest.managed_files)
        .map(|file| (file.destination.to_ascii_lowercase(), file))
        .collect();
    let mut files = Vec::new();
    for file in &base_target.files {
        let existing = snapshot.base_files.get(&file.destination);
        let action = match existing {
            Some(hash) if hash == &file.sha256 => FileAction::Skip,
            Some(_) => return Err(InstallError::BaseConflict(file.destination.clone())),
            None => FileAction::Install,
        };
        files.push(PlannedFile {
            scope: FileScope::Base,
            destination: file.destination.clone(),
            sha256: file.sha256.clone(),
            size: file.size,
            policy: InstallPolicy::Managed,
            action,
            expected_before: existing.cloned(),
        });
    }
    let mut next_paths = BTreeSet::new();
    for file in &pack_target.files {
        let existing = snapshot.instance_files.get(&file.destination);
        let policy = match file.policy {
            Policy::Managed => InstallPolicy::Managed,
            Policy::DefaultOnCreate => InstallPolicy::DefaultOnCreate,
        };
        let action = match (policy, existing) {
            (_, Some(hash)) if hash == &file.sha256 => FileAction::Skip,
            (InstallPolicy::DefaultOnCreate, Some(_)) => FileAction::PreserveDefault,
            _ => FileAction::Install,
        };
        let key = file.destination.to_ascii_lowercase();
        next_paths.insert(key.clone());
        if let Some(previous) = old.get(&key) {
            if previous.destination != file.destination {
                return Err(InstallError::Conflict(file.destination.clone()));
            }
        }
        files.push(PlannedFile {
            scope: FileScope::Instance,
            destination: file.destination.clone(),
            sha256: file.sha256.clone(),
            size: file.size,
            policy,
            action,
            expected_before: existing.cloned(),
        });
    }
    let mut removals = Vec::new();
    for (key, file) in old {
        if !next_paths.contains(&key) {
            if let Some(current) = snapshot.instance_files.get(&file.destination) {
                removals.push(Removal {
                    destination: file.destination.clone(),
                    sha256: current.clone(),
                });
            }
        }
    }
    let mut rollback_index = index.clone();
    rollback_index
        .packs
        .retain(|candidate| candidate.id == pack_id);
    rollback_index.packs[0]
        .versions
        .retain(|candidate| candidate.version == pack_version);
    rollback_index
        .bases
        .retain(|candidate| candidate.version == base.version);
    Ok(InstallPlan {
        pack_id: pack_id.to_owned(),
        pack_version: pack_version.to_owned(),
        base_version: base.version.clone(),
        sequence: index.sequence,
        target: target.to_owned(),
        files,
        removals,
        rollback_index,
        rollback_proof: feed.proof.clone(),
        previous: snapshot.previous.clone(),
        instance_dir: instance_dir.to_owned(),
        data_dir: data_dir.to_owned(),
    })
}

#[cfg(test)]
#[path = "yoake_install_tests.rs"]
mod tests;
