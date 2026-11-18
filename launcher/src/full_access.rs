//! Full-access mods, the launcher's side (MX3U1): the one place that installs them, lists them, approves them
//! and builds the list the game receives. No UI code: the pages call the functions below.
//!
//! A full-access mod (`[wasm] world = "full-access"`) runs with the engine's own powers, so it never goes through the
//! sandboxed store (`yoake_mods`). It lives as a top-level `.yak` in `<instance>/mods/` with an origin record
//! (`local` or `catalog`), and it runs only when the player approved that exact file. The two control files
//! (`full-access.toml`, `yak-origins.toml`) belong to the launcher and live in `<launcher data>/yoake-full-access/`;
//! the game never opens that folder and receives, per launch, a list of what is approved
//! ([`prepare`], `--full-access-approved <file>`).
//!
//! # The launch rule (pending changes)
//!
//! For each instance the launcher keeps the **last accepted list** (`yoake-full-access/last-accepted/<instance>.toml`):
//! what the player has seen and confirmed. A launch never passes the *current* list on its own; it passes
//! **the last accepted list intersected with what still matches on disk**: an entry of the current list
//! (`id`, `version`, `sha256`, `origin` all equal) that is also in the last accepted list. So:
//!
//! * a mod whose bytes changed is dropped (never upgraded silently): its old entry no longer matches a file;
//! * an approval or an install that appeared behind the player's back (a mod rewrote the control files, a file was put
//!   into `mods/`) is not passed until [`accept_changes`] ran after the player confirmed the diff
//!   ([`pending_changes`]);
//! * the player's own explicit acts are already confirmations: [`approve`] adds that one entry to the last accepted
//!   list, [`revoke`] and [`remove`] take it out. Nothing else changes it.
//! * a control file that fails the corruption check (or cannot be read) means **no list at all**: nothing is
//!   full-access for that launch, and the reason is in [`Prepared::problem`] / [`control_problem`].
//!
//! # Run folders
//!
//! Each launch gets `<launcher data>/run/<launch id>/approved.toml`. The folder is deleted when the game exits
//! (dropping the [`RunFolder`] does it), and [`remove_stale_runs`] at startup removes the folders of launchers that
//! crashed (a folder whose `owner.lock` nobody holds). The game reads the file once, at its first world start.
use horizon_mods::yak::control::{
    self, AllowFile, ApprovedEntry, ApprovedList, Control, ListDiff, Origin, OriginFile, APPROVED_ARG,
};
use horizon_mods::yak::{Archive, Manifest};
use std::collections::HashMap;
use std::fs::File;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// The folder of per-launch lists, inside the launcher's data folder.
pub const RUN_SUBDIR: &str = "run";
/// The per-instance last accepted lists, inside the control folder.
const LAST_SUBDIR: &str = "last-accepted";
/// Held open by the launcher that owns a run folder; a crashed launcher releases it.
const OWNER_LOCK: &str = "owner.lock";
/// A run folder without a lock file younger than this is left alone (it is being created).
const YOUNG: std::time::Duration = std::time::Duration::from_secs(60);

/// Where a mod stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowState {
    /// This exact file is approved.
    Approved,
    /// Never approved.
    NotApproved,
    /// The id was approved, but for other bytes: the file changed, or it is another version. Not run until approved again.
    ContentChanged,
    /// Two files in `mods/` carry this id; the game refuses both.
    DuplicateId,
    /// Cannot be full-access: the reason is for the player (unreadable package, no install record, a control file
    /// that failed its check, ...).
    Refused(String),
}

/// One full-access package in `<instance>/mods/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullAccessRow {
    pub id: String,
    pub name: String,
    pub version: String,
    /// First 12 hex digits of the file's SHA-256.
    pub sha256_short: String,
    /// The whole SHA-256 (what [`approve`] takes; a prefix of at least 8 digits is accepted too).
    pub sha256: String,
    /// `"local"`, `"catalog"`, or `"unknown"` (no install record).
    pub origin: String,
    /// Who signed a catalog package (informational).
    pub signer: Option<String>,
    /// The file's name in `mods/`.
    pub file: String,
    /// Same as `state == Approved`.
    pub approved: bool,
    pub state: RowState,
    /// Whether the next launch passes this file to the game: approved **and** in the last accepted list.
    /// `approved && !passed_at_launch` is a change waiting for [`accept_changes`].
    pub passed_at_launch: bool,
}

/// `<instance>/mods`.
pub fn mods_dir(instance: &Path) -> PathBuf {
    instance.join("mods")
}

/// Every public function that reads or writes the control files holds this, so the launcher never runs two of them at
/// once (an install and an approval both rewrite the checksum file: interleaved, the check would fail and stop every
/// full-access mod). Other processes are not covered.
static CONTROL_LOCK: Mutex<()> = Mutex::new(());

fn lock() -> std::sync::MutexGuard<'static, ()> {
    CONTROL_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn open_control(data: &Path, instance: &Path) -> Result<Control, String> {
    Control::create(&control::control_dir_in(data), instance)
}

// ---------------------------------------------------------------------------------------------------------------
// Scanning the mods folder
// ---------------------------------------------------------------------------------------------------------------

struct Found {
    path: PathBuf,
    file: String,
    sha256: String,
    manifest: Result<Manifest, String>,
}

impl Found {
    fn full_access(&self) -> bool {
        self.manifest.as_ref().map_or(true, |m| m.wasm.as_ref().is_some_and(|w| w.full_access()))
    }

    fn id(&self) -> String {
        self.manifest.as_ref().map(|m| m.id.clone()).unwrap_or_else(|_| self.file.trim_end_matches(".yak").to_owned())
    }
}

/// The top-level `*.yak` files that are full-access packages (or cannot be read, which is shown as refused).
fn scan(mods: &Path) -> Vec<Found> {
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(mods) {
        Ok(entries) => entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "yak") && p.is_file()).collect(),
        Err(_) => Vec::new(),
    };
    paths.sort();
    let mut found = Vec::new();
    for path in paths {
        let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let Ok(bytes) = std::fs::read(&path) else { continue };
        let sha256 = control::sha256_hex(&bytes);
        let manifest = Archive::new(Cursor::new(bytes)).map(|a| a.manifest().clone());
        let item = Found { path, file, sha256, manifest };
        if item.full_access() {
            found.push(item);
        }
    }
    found
}

fn short(sha256: &str) -> String {
    sha256.chars().take(12).collect()
}

// ---------------------------------------------------------------------------------------------------------------
// The last accepted list
// ---------------------------------------------------------------------------------------------------------------

fn last_path(control: &Control, instance: &Path) -> PathBuf {
    let canonical = control::canonical_lenient(instance).to_string_lossy().replace('\\', "/");
    let key = if cfg!(windows) { canonical.to_lowercase() } else { canonical };
    control.dir().join(LAST_SUBDIR).join(format!("{}.toml", &control::sha256_hex(key.as_bytes())[..16]))
}

/// The list the player last accepted for `instance`; empty when none (or unreadable: nothing is then passed).
fn load_last(control: &Control, instance: &Path) -> ApprovedList {
    std::fs::read_to_string(last_path(control, instance)).ok().and_then(|text| ApprovedList::parse(&text).ok()).unwrap_or_default()
}

fn save_last(control: &Control, instance: &Path, list: &ApprovedList) -> Result<(), String> {
    let path = last_path(control, instance);
    std::fs::create_dir_all(path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
    control::write_atomic(&path, list.to_text().as_bytes())
}

/// The last accepted list with `id` replaced by what `current` has for it (nothing: removed).
fn last_with(control: &Control, instance: &Path, id: &str, current: Option<&ApprovedList>) -> Result<(), String> {
    let last = load_last(control, instance);
    let mut entries: Vec<ApprovedEntry> = last.entries().iter().filter(|e| e.id != id).cloned().collect();
    if let Some(current) = current {
        entries.extend(current.entries().iter().filter(|e| e.id == id).cloned());
    }
    save_last(control, instance, &ApprovedList::new(entries, last.protected().to_vec()))
}

// ---------------------------------------------------------------------------------------------------------------
// The current list, and the one a launch passes
// ---------------------------------------------------------------------------------------------------------------

/// What the next launch would pass if nothing were held back: [`Control::approved_list`].
fn current_list(control: &Control, mods: &Path) -> Result<ApprovedList, String> {
    control.approved_list(mods)
}

/// The same list built from the control files **without** the corruption check, to show the player what changed
/// behind the launcher's back. `Err` when a file cannot even be parsed.
fn unchecked_list(control: &Control, mods: &Path) -> Result<ApprovedList, String> {
    let read = |name: &str| -> Result<Option<String>, String> {
        match std::fs::read_to_string(control.dir().join(name)) {
            Ok(text) => Ok(Some(text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{name}: {e}")),
        }
    };
    let allow = read(AllowFile::FILE_NAME)?.map(|t| AllowFile::parse(&t)).transpose()?.unwrap_or_default();
    let origins = read(OriginFile::FILE_NAME)?.map(|t| OriginFile::parse(&t)).transpose()?.unwrap_or_default();
    let mut entries = Vec::new();
    for item in scan(mods) {
        let Some(origin) = origins.origin_of(&item.sha256).filter(|o| o.can_be_full_access()) else { continue };
        let Some(entry) = allow.entries().iter().find(|e| e.sha256 == item.sha256) else { continue };
        entries.push(ApprovedEntry { id: entry.id.clone(), version: entry.version.clone(), sha256: item.sha256.clone(), origin });
    }
    Ok(ApprovedList::new(entries, vec![control.dir().to_path_buf(), control.checksum_path()]))
}

/// The list a launch passes: the entries of `current` that the player accepted (equal entry in `last`).
fn effective_list(last: &ApprovedList, current: &ApprovedList) -> ApprovedList {
    let entries = current.entries().iter().filter(|e| last.entries().contains(e)).cloned().collect();
    ApprovedList::new(entries, current.protected().to_vec())
}

// ---------------------------------------------------------------------------------------------------------------
// The API for the pages
// ---------------------------------------------------------------------------------------------------------------

/// Every full-access package in `instance`'s `mods/` folder, with its state.
pub fn list(instance: &Path) -> Vec<FullAccessRow> {
    list_in(&crate::backend::data_dir(), instance)
}

pub fn list_in(data: &Path, instance: &Path) -> Vec<FullAccessRow> {
    let _lock = lock();
    let mods = mods_dir(instance);
    let found = scan(&mods);
    if found.is_empty() {
        return Vec::new();
    }
    let control = open_control(data, instance);
    let checked = control.as_ref().map_err(Clone::clone).and_then(|c| Ok((c.allow()?, c.origins()?)));
    let effective = control.as_ref().ok().and_then(|c| {
        let current = current_list(c, &mods).ok()?;
        Some(effective_list(&load_last(c, instance), &current))
    });
    let mut rows = Vec::new();
    for item in &found {
        let id = item.id();
        let (name, version, mut state) = match &item.manifest {
            Ok(m) => (m.name.clone(), m.version.to_string(), RowState::NotApproved),
            Err(e) => (id.clone(), String::new(), RowState::Refused(format!("not a valid .yak: {e}"))),
        };
        let mut origin = "unknown".to_owned();
        let mut signer = None;
        match &checked {
            Err(e) => state = RowState::Refused(e.clone()),
            Ok((allow, origins)) => {
                let record = origins.get(&item.sha256);
                if let Some(record) = record {
                    origin = record.origin.as_str().to_owned();
                    signer = record.signer.clone();
                }
                if matches!(state, RowState::Refused(_)) {
                    // the package itself is unreadable: that is the reason shown
                } else if !record.is_some_and(|r| r.origin.can_be_full_access()) {
                    state = RowState::Refused(match record {
                        None => "the launcher has no install record for this file (it was put into the folder by hand); remove it and install it again".to_owned(),
                        Some(r) => format!("a package from a {} cannot be full-access", r.origin.as_str()),
                    });
                } else if found.iter().filter(|other| other.manifest.is_ok() && other.id() == id).count() > 1 {
                    state = RowState::DuplicateId;
                } else if allow.is_allowed(&id, &item.sha256) {
                    state = RowState::Approved;
                } else if !allow.approved_hashes(&id).is_empty() {
                    state = RowState::ContentChanged;
                }
            }
        }
        let approved = state == RowState::Approved;
        let passed_at_launch = approved && effective.as_ref().is_some_and(|l| l.entries().iter().any(|e| e.sha256 == item.sha256));
        rows.push(FullAccessRow {
            id,
            name,
            version,
            sha256_short: short(&item.sha256),
            sha256: item.sha256.clone(),
            origin,
            signer,
            file: item.file.clone(),
            approved,
            state,
            passed_at_launch,
        });
    }
    rows
}

/// Approves exactly the package file `id` with this SHA-256 (full, or a prefix of at least 8 digits). The player's own
/// act: it is also recorded as accepted for the next launch. `Err` (nothing changed) when the file is not there, its
/// bytes differ, it has no install record, its id is duplicated, or a control file is damaged.
pub fn approve(instance: &Path, id: &str, sha256: &str) -> Result<(), String> {
    approve_in(&crate::backend::data_dir(), instance, id, sha256)
}

pub fn approve_in(data: &Path, instance: &Path, id: &str, sha256: &str) -> Result<(), String> {
    let _lock = lock();
    let prefix = sha256.trim().to_ascii_lowercase();
    if prefix.len() < 8 || !prefix.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("give the package's SHA-256 (at least 8 hex digits)".into());
    }
    let control = open_control(data, instance)?;
    let mods = mods_dir(instance);
    let found = scan(&mods);
    let same_id: Vec<&Found> = found.iter().filter(|f| f.manifest.as_ref().is_ok_and(|m| m.id == id)).collect();
    let item = match same_id.iter().filter(|f| f.sha256.starts_with(&prefix)).collect::<Vec<_>>().as_slice() {
        [one] => **one,
        [] => return Err(format!("no full-access package {id} with that SHA-256 in {}", mods.display())),
        _ => return Err("that SHA-256 prefix matches more than one file".into()),
    };
    if same_id.len() > 1 {
        return Err(format!("{id} is in the mods folder more than once; remove the extra file first"));
    }
    control
        .origins()?
        .origin_of(&item.sha256)
        .filter(|o| o.can_be_full_access())
        .ok_or_else(|| format!("{} was not installed by the launcher; remove it and install it again", item.file))?;
    let bytes = std::fs::read(&item.path).map_err(|e| format!("{}: {e}", item.path.display()))?;
    if control::sha256_hex(&bytes) != item.sha256 {
        return Err(format!("{} changed while approving; try again", item.file));
    }
    control.approve(&bytes)?;
    let current = current_list(&control, &mods)?;
    last_with(&control, instance, id, Some(&current))
}

/// Withdraws every approval of `id`.
pub fn revoke(instance: &Path, id: &str) -> Result<(), String> {
    revoke_in(&crate::backend::data_dir(), instance, id)
}

pub fn revoke_in(data: &Path, instance: &Path, id: &str) -> Result<(), String> {
    let _lock = lock();
    let control = open_control(data, instance)?;
    let mut allow = control.allow()?;
    allow.revoke(id);
    control.save_allow(&allow)?;
    last_with(&control, instance, id, None)
}

/// Deletes every full-access package with this id from `mods/`, forgets where it came from and withdraws its approval.
pub fn remove(instance: &Path, id: &str) -> Result<(), String> {
    remove_in(&crate::backend::data_dir(), instance, id)
}

pub fn remove_in(data: &Path, instance: &Path, id: &str) -> Result<(), String> {
    let _lock = lock();
    let control = open_control(data, instance)?;
    let found = scan(&mods_dir(instance));
    let targets: Vec<&Found> = found.iter().filter(|f| f.id() == id).collect();
    if targets.is_empty() {
        return Err(format!("no full-access package {id} in {}", mods_dir(instance).display()));
    }
    let (mut allow, mut origins) = (control.allow()?, control.origins()?);
    allow.revoke(id);
    for item in &targets {
        origins.forget(&item.sha256);
    }
    control.save_allow(&allow)?;
    control.save_origins(&origins)?;
    last_with(&control, instance, id, None)?;
    for item in targets {
        std::fs::remove_file(&item.path).map_err(|e| format!("{}: {e}", item.path.display()))?;
    }
    Ok(())
}

/// Deletes the single full-access package `file` (its name in `mods/`) and forgets where it came from. Its id stays
/// approved only when another file with the same id is still there (the player removed the extra copy of a
/// duplicate); otherwise the approval of the id is withdrawn as in [`remove`].
pub fn remove_file(instance: &Path, file: &str) -> Result<(), String> {
    remove_file_in(&crate::backend::data_dir(), instance, file)
}

pub fn remove_file_in(data: &Path, instance: &Path, file: &str) -> Result<(), String> {
    let _lock = lock();
    let control = open_control(data, instance)?;
    let found = scan(&mods_dir(instance));
    let item = found
        .iter()
        .find(|f| f.file == file)
        .ok_or_else(|| format!("no full-access package {file} in {}", mods_dir(instance).display()))?;
    let id = item.id();
    let others = found.iter().any(|f| f.file != file && f.id() == id);
    // A copy with the same bytes keeps the install record (it is keyed by SHA-256).
    let same_bytes = found.iter().any(|f| f.file != file && f.sha256 == item.sha256);
    let (mut allow, mut origins) = (control.allow()?, control.origins()?);
    if !same_bytes {
        origins.forget(&item.sha256);
    }
    if !others {
        allow.revoke(&id);
    }
    control.save_allow(&allow)?;
    control.save_origins(&origins)?;
    if !same_bytes {
        let last = load_last(&control, instance);
        let entries = last.entries().iter().filter(|e| e.sha256 != item.sha256).cloned().collect();
        save_last(&control, instance, &ApprovedList::new(entries, last.protected().to_vec()))?;
    }
    std::fs::remove_file(&item.path).map_err(|e| format!("{}: {e}", item.path.display()))
}

/// What the next launch would pass that the player has not accepted yet, compared with the last accepted list:
/// `added`, `removed` (also a file that vanished or whose bytes changed) and `changed`. `None` when there is nothing
/// to show. When a control file fails its check but can still be read, the diff is built from what is on disk (the
/// player sees what changed behind the launcher's back); [`control_problem`] says why it is held back.
pub fn pending_changes(instance: &Path) -> Option<ListDiff> {
    pending_changes_in(&crate::backend::data_dir(), instance)
}

pub fn pending_changes_in(data: &Path, instance: &Path) -> Option<ListDiff> {
    let _lock = lock();
    pending_unlocked(data, instance)
}

fn pending_unlocked(data: &Path, instance: &Path) -> Option<ListDiff> {
    let control = open_control(data, instance).ok()?;
    let mods = mods_dir(instance);
    let current = current_list(&control, &mods).or_else(|_| unchecked_list(&control, &mods)).ok()?;
    let diff = load_last(&control, instance).diff(&current);
    (!diff.is_empty()).then_some(diff)
}

/// Why no list can be built right now (a control file failed its corruption check or does not parse), or `None`.
pub fn control_problem(instance: &Path) -> Option<String> {
    control_problem_in(&crate::backend::data_dir(), instance)
}

pub fn control_problem_in(data: &Path, instance: &Path) -> Option<String> {
    let _lock = lock();
    let control = match open_control(data, instance) {
        Ok(control) => control,
        Err(e) => return Some(e),
    };
    control.allow().err().or_else(|| control.origins().err())
}

/// **Only after the player confirmed the diff [`pending_changes`] showed.** Records the control files as they are now
/// ([`Control::accept_current_files`]) and the current list as the last accepted one. `Err` (nothing changed) when
/// a control file cannot be read at all.
pub fn accept_changes(instance: &Path) -> Result<(), String> {
    accept_changes_in(&crate::backend::data_dir(), instance, None)
}

/// [`accept_changes`], refusing when what is pending is no longer what the player was shown.
pub fn accept_changes_shown(instance: &Path, shown: &ListDiff) -> Result<(), String> {
    accept_changes_in(&crate::backend::data_dir(), instance, Some(shown))
}

pub fn accept_changes_in(data: &Path, instance: &Path, shown: Option<&ListDiff>) -> Result<(), String> {
    let _lock = lock();
    let control = open_control(data, instance)?;
    let mods = mods_dir(instance);
    // The list the diff is built from (unchecked when a control file failed its check: accepting re-seals it).
    let compared = current_list(&control, &mods).or_else(|_| unchecked_list(&control, &mods))?;
    if let Some(shown) = shown {
        let now = load_last(&control, instance).diff(&compared);
        if &now != shown {
            return Err("the pending changes are not what you were shown; look at them again".into());
        }
    }
    control.accept_current_files()?;
    let after = current_list(&control, &mods)?;
    // Saved: the entries that were compared (and shown), not a list read again afterwards. A launch passes only
    // entries that are in both, so anything that changed in between stays held back until shown.
    save_last(&control, instance, &ApprovedList::new(compared.entries().to_vec(), after.protected().to_vec()))
}

// ---------------------------------------------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------------------------------------------

fn check_new(found: &[Found], bytes: &[u8]) -> Result<Manifest, String> {
    let archive = Archive::new(Cursor::new(bytes.to_vec()))?;
    let manifest = archive.manifest().clone();
    if !manifest.wasm.as_ref().is_some_and(|w| w.full_access()) {
        return Err(format!("{} is not a full-access package", manifest.id));
    }
    let sha256 = control::sha256_hex(bytes);
    if found.iter().any(|f| f.manifest.as_ref().is_ok_and(|m| m.id == manifest.id) && f.sha256 != sha256) {
        return Err(format!("{} is already installed in another version; remove it first", manifest.id));
    }
    Ok(manifest)
}

/// Whether `manifest` is a full-access package.
pub fn is_full_access(manifest: &Manifest) -> bool {
    manifest.wasm.as_ref().is_some_and(|w| w.full_access())
}

/// Installs a full-access `.yak` the player chose: copied into `<instance>/mods/` through [`Control::install_local`]
/// with origin `local`. **Not approved**: approval is [`approve`]. Returns the file's SHA-256.
pub fn install_local(instance: &Path, source: &Path) -> Result<String, String> {
    install_local_in(&crate::backend::data_dir(), instance, source, None)
}

/// `expected`: the SHA-256 the caller planned (a different file is removed again and refused).
pub fn install_local_in(data: &Path, instance: &Path, source: &Path, expected: Option<&str>) -> Result<String, String> {
    let _lock = lock();
    let control = open_control(data, instance)?;
    let mods = mods_dir(instance);
    let bytes = std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
    check_new(&scan(&mods), &bytes)?;
    let sha256 = control.install_local(&mods, source)?;
    if expected.is_some_and(|expected| expected != sha256) {
        let _ = undo_install(&control, &mods, source.file_name().and_then(|n| n.to_str()).unwrap_or_default(), &sha256);
        return Err("the file changed while installing; review it again".into());
    }
    Ok(sha256)
}

/// Installs a full-access `.yak` downloaded from the catalog (`bytes` already checked against the signed catalog)
/// with origin `catalog`, recording the signer. **Not approved.** Returns the file's SHA-256.
pub fn install_catalog_in(data: &Path, instance: &Path, file_name: &str, bytes: &[u8], signer: Option<&str>) -> Result<String, String> {
    let _lock = lock();
    let control = open_control(data, instance)?;
    let mods = mods_dir(instance);
    check_new(&scan(&mods), bytes)?;
    control::install_package(&control, &mods, file_name, bytes, Origin::Catalog, signer)
}

fn undo_install(control: &Control, mods: &Path, file_name: &str, sha256: &str) -> Result<(), String> {
    let mut origins = control.origins()?;
    origins.forget(sha256);
    control.save_origins(&origins)?;
    control::remove_package(mods, file_name)
}

// ---------------------------------------------------------------------------------------------------------------
// Launch
// ---------------------------------------------------------------------------------------------------------------

/// A launch's run folder. Dropping it (when the game exits) deletes the folder.
#[derive(Debug)]
pub struct RunFolder {
    dir: PathBuf,
    approved: PathBuf,
    lock: Option<File>,
}

impl RunFolder {
    /// The list file the game was given.
    pub fn approved_path(&self) -> &Path {
        &self.approved
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Deletes the folder now (same as dropping).
    pub fn finish(self) {}
}

impl Drop for RunFolder {
    fn drop(&mut self) {
        self.lock = None;
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// What a launch needs from [`prepare`].
#[derive(Debug, Default)]
pub struct Prepared {
    /// The run folder; keep it until the game exits, then drop it. `None` when no list was written.
    pub run: Option<RunFolder>,
    /// Why no list was passed (the game starts without full-access mods), when so.
    pub problem: Option<String>,
    /// Changes waiting for the player (held back from this launch); same as [`pending_changes`].
    pub pending: Option<ListDiff>,
    /// How many mods this launch passes.
    pub passed: usize,
}

impl Prepared {
    /// Adds `--full-access-approved <abs path>` to the game's command line (nothing when there is no list).
    pub fn apply(&self, cmd: &mut Command) {
        if let Some(run) = &self.run {
            cmd.arg(APPROVED_ARG).arg(run.approved_path());
        }
    }

    /// The argument pair, for callers that build their own command line.
    pub fn args(&self) -> Vec<std::ffi::OsString> {
        self.run.iter().flat_map(|run| [APPROVED_ARG.into(), run.approved_path().as_os_str().to_owned()]).collect()
    }
}

static PROBLEMS: Mutex<Option<HashMap<PathBuf, String>>> = Mutex::new(None);

/// The problem the last [`prepare`] for `instance` met (its game started without full-access mods), if any.
pub fn last_launch_problem(instance: &Path) -> Option<String> {
    PROBLEMS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|map| map.get(instance).cloned())
}

fn note_problem(instance: &Path, problem: Option<&str>) {
    let mut guard = PROBLEMS.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    match problem {
        Some(text) => map.insert(instance.to_path_buf(), text.to_owned()),
        None => map.remove(instance),
    };
}

static NEXT_RUN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn new_run_folder(data: &Path, list: &ApprovedList) -> Result<RunFolder, String> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let id = format!("{}-{nanos}-{}", std::process::id(), NEXT_RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let dir = data.join(RUN_SUBDIR).join(id);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let lock = File::create(dir.join(OWNER_LOCK)).map_err(|e| format!("{}: {e}", dir.display()))?;
    lock.try_lock().map_err(|e| format!("{}: {e}", dir.display()))?;
    match list.write_for_launch(&dir) {
        Ok(approved) => Ok(RunFolder { dir, approved, lock: Some(lock) }),
        Err(e) => {
            drop(lock);
            let _ = std::fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

/// **Call before spawning the Yoake game** for `instance` (its game folder). Builds the list this launch passes
/// (the launch rule is in the module documentation), writes it into a fresh run folder and returns it; add the
/// argument with [`Prepared::apply`] and keep [`Prepared::run`] until the game exits. On a problem nothing is passed
/// and [`Prepared::problem`] says why (also kept for [`last_launch_problem`]).
pub fn prepare(instance: &Path) -> Prepared {
    prepare_in(&crate::backend::data_dir(), instance)
}

pub fn prepare_in(data: &Path, instance: &Path) -> Prepared {
    let _lock = lock();
    let result = (|| {
        let control = open_control(data, instance)?;
        let mods = mods_dir(instance);
        let current = current_list(&control, &mods)?;
        let last = load_last(&control, instance);
        let pending = last.diff(&current);
        let effective = effective_list(&last, &current);
        let run = new_run_folder(data, &effective)?;
        Ok::<_, String>(Prepared { passed: effective.entries().len(), run: Some(run), problem: None, pending: (!pending.is_empty()).then_some(pending) })
    })();
    let prepared = result.unwrap_or_else(|problem| Prepared {
        problem: Some(problem),
        pending: pending_unlocked(data, instance),
        ..Prepared::default()
    });
    note_problem(instance, prepared.problem.as_deref());
    prepared
}

/// Startup: removes the run folders of launchers that crashed or were killed. A folder whose `owner.lock` is held
/// by a live launcher is left alone. Returns how many folders it removed.
pub fn remove_stale_runs() -> usize {
    remove_stale_runs_in(&crate::backend::data_dir())
}

pub fn remove_stale_runs_in(data: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(data.join(RUN_SUBDIR)) else { return 0 };
    let mut removed = 0;
    for entry in entries.flatten() {
        let dir = entry.path();
        if !dir.is_dir() {
            continue;
        }
        let owned = match File::options().write(true).open(dir.join(OWNER_LOCK)) {
            // nobody holds it: the owner is gone
            Ok(file) => file.try_lock().is_ok().then_some(file),
            Err(_) => {
                let young = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < YOUNG);
                if young {
                    continue;
                }
                None
            }
        };
        let stale = owned.is_some() || !dir.join(OWNER_LOCK).exists();
        if stale {
            drop(owned);
            if std::fs::remove_dir_all(&dir).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}
