//! MX3DF1: the developer folder for full-access mods (hot reload, the launcher's side).
//!
//! With developer mode on (`Settings::dev_unsigned_wasm`, "Test your own WASM mods") and a folder chosen
//! (`Settings::full_access_dev_folder`, set through [`set_dev_folder`]), a launch of the Yoake game passes
//! `--full-access-dev` and the launcher watches that folder for as long as the game it started runs: every second a
//! worker thread looks at the top-level `*.yak` files. A file that is new, changed or gone, and has stayed the same for
//! 500 ms ([`STABLE`]), is applied by [`DevSync::tick`]:
//!
//! * a full-access package is installed into `<instance>/mods/` with origin `dev`, **approved** (exactly those bytes, and
//!   accepted for the instance: the owner-approved exception for the player's own folder) and the run's approved list
//!   is rewritten in place and atomically with `generation` + 1;
//! * an older dev copy of the same id (or of the same file name) is replaced; the same id from any other origin is
//!   refused and reported, and never overwritten;
//! * a removed file removes its dev copy and its approval, and rewrites the list the same way.
//!
//! Nothing from any other source is approved here. Every control-file write holds the control lock of the parent
//! module. The watcher belongs to the [`super::RunFolder`]: dropping the run folder (the game exited) stops it.
use super::*;
use std::collections::{BTreeSet, HashMap};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

/// The game's command-line flag for a launch with an active developer folder.
pub const DEV_ARG: &str = "--full-access-dev";
/// How often the folder is looked at.
const POLL: Duration = Duration::from_secs(1);
/// A file must look the same for this long before it is taken (it may still be being written).
pub const STABLE: Duration = Duration::from_millis(500);

/// What the Mods page shows for an instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DevStatus {
    pub folder: PathBuf,
    /// A watcher is running for a game of this instance.
    pub watching: bool,
    /// The last thing that happened ("installed foo 1.0.0 (generation 3)", "removed bar.yak", ...).
    pub last_event: Option<String>,
    /// The last problem (a refused file, a folder that cannot be used), until a later event succeeds.
    pub last_error: Option<String>,
    /// The `generation` of the run's approved list (0 before the first rewrite).
    pub generation: u64,
}

#[derive(Clone, Debug, Default)]
struct DevState {
    watching: bool,
    last_event: Option<String>,
    last_error: Option<String>,
    generation: u64,
}

static STATES: Mutex<Option<HashMap<PathBuf, DevState>>> = Mutex::new(None);

fn with_state<T>(instance: &Path, f: impl FnOnce(&mut DevState) -> T) -> T {
    let mut guard = STATES.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(HashMap::new).entry(instance.to_path_buf()).or_default())
}

/// The developer folder that applies to a launch: only while developer mode is on (`dev_unsigned_wasm`, the same switch
/// as "Test your own WASM mods") and a folder is chosen. Not validated here ([`validate_dev_folder`]).
pub fn active_dev_folder(settings: &crate::backend::Settings) -> Option<PathBuf> {
    settings.dev_unsigned_wasm.then(|| settings.full_access_dev_folder.clone()).flatten()
}

/// The developer folder's status for `instance`, or `None` when it is not active (developer mode off or no folder).
pub fn dev_folder_status(instance: &Path) -> Option<DevStatus> {
    status_for(instance, active_dev_folder(&crate::backend::Settings::load()))
}

/// [`dev_folder_status`] for a given active folder.
pub fn status_for(instance: &Path, folder: Option<PathBuf>) -> Option<DevStatus> {
    let folder = folder?;
    let state = with_state(instance, |s| s.clone());
    Some(DevStatus { folder, watching: state.watching, last_event: state.last_event, last_error: state.last_error, generation: state.generation })
}

/// Chooses (`Some`) or clears (`None`) the developer folder in the launcher's settings. `Err` (nothing changed) for a
/// path that is not an existing absolute folder, has a `..`, is inside a `mods` folder, the launcher's data folder or
/// any instance's folder, or contains the launcher's data folder. It takes effect for launches while developer mode is
/// on. The UI keeps its own copy of the settings in step (`Settings::load` after this call).
pub fn set_dev_folder(path: Option<PathBuf>) -> Result<(), String> {
    let mut settings = crate::backend::Settings::load();
    settings.full_access_dev_folder = match path {
        None => None,
        Some(path) => {
            let homes: Vec<PathBuf> = crate::instances::list().into_iter().flat_map(|i| [i.home, i.work]).collect();
            Some(validate_dev_folder(&path, &crate::backend::data_dir(), &homes)?)
        }
    };
    settings.save()
}

fn comparable(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).trim_end_matches('/').to_owned();
    if cfg!(windows) { text.to_lowercase() } else { text }
}

fn is_inside(path: &Path, folder: &Path) -> bool {
    let (path, folder) = (comparable(path), comparable(folder));
    path == folder || path.strip_prefix(&folder).is_some_and(|rest| rest.starts_with('/'))
}

/// Checks a developer folder choice and returns its canonical path. Refused: a relative path, a `..`, a path that is
/// not an existing folder, one with a `mods` component, one inside (or equal to) the launcher's `data` folder or any of
/// `homes` (instance folders), and one that contains `data`.
pub fn validate_dev_folder(path: &Path, data: &Path, homes: &[PathBuf]) -> Result<PathBuf, String> {
    if control::has_parent_dir(path) {
        return Err(format!("{} has a `..` component", path.display()));
    }
    if !path.is_absolute() {
        return Err(format!("{} is not an absolute path", path.display()));
    }
    if !path.is_dir() {
        return Err(format!("{} is not an existing folder", path.display()));
    }
    let canonical = control::canonical_lenient(path);
    if canonical.components().any(|c| matches!(c, std::path::Component::Normal(n) if n.to_string_lossy().eq_ignore_ascii_case("mods"))) {
        return Err(format!("{} is a mods folder (or inside one): choose a separate folder for your own builds", path.display()));
    }
    let data = control::canonical_lenient(data);
    if is_inside(&canonical, &data) || is_inside(&data, &canonical) {
        return Err(format!("{} is the launcher's data folder, inside it, or contains it", path.display()));
    }
    for home in homes {
        if is_inside(&canonical, &control::canonical_lenient(home)) {
            return Err(format!("{} is an instance's folder or inside one: choose a separate folder", path.display()));
        }
    }
    Ok(canonical)
}

/// One file's identity for "has it changed": size and modification time (`None`: the file is not there).
type Sig = Option<(u64, Option<SystemTime>)>;

struct Obs {
    sig: Sig,
    since: Instant,
}

/// What one applied change did.
struct Outcome {
    event: String,
    /// The approved list needs rewriting.
    changed: bool,
}

/// The engine of the watcher: compares the folder with `<instance>/mods/` and applies stable changes. Public so the
/// tests can drive it with a clock of their own; the launcher runs it on a worker thread ([`start`]).
pub struct DevSync {
    data: PathBuf,
    instance: PathBuf,
    folder: PathBuf,
    approved: PathBuf,
    obs: HashMap<String, Obs>,
    acted: HashMap<String, Sig>,
    /// Names of the dev copies in `mods/` (from the start, and the ones this run installed).
    installed: BTreeSet<String>,
}

fn plain_yak(name: &str) -> bool {
    !(name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') || !name.ends_with(".yak"))
}

impl DevSync {
    /// `approved`: the run's approved list, rewritten in place. `folder` must already be validated.
    pub fn new(data: &Path, instance: &Path, folder: &Path, approved: &Path) -> Self {
        let mut sync = Self {
            data: data.to_path_buf(),
            instance: instance.to_path_buf(),
            folder: folder.to_path_buf(),
            approved: approved.to_path_buf(),
            obs: HashMap::new(),
            acted: HashMap::new(),
            installed: BTreeSet::new(),
        };
        let generation = ApprovedList::read(approved).map(|l| l.generation()).unwrap_or(0);
        sync.installed = sync.dev_copies();
        with_state(instance, |s| {
            *s = DevState { watching: true, last_event: Some(format!("watching {}", folder.display())), last_error: None, generation };
        });
        sync
    }

    /// The names of the dev-origin packages in `mods/`.
    fn dev_copies(&self) -> BTreeSet<String> {
        let _lock = lock();
        let Ok(control) = open_control(&self.data, &self.instance) else { return BTreeSet::new() };
        let Ok(origins) = control.origins() else { return BTreeSet::new() };
        scan(&mods_dir(&self.instance)).into_iter().filter(|f| origins.origin_of(&f.sha256) == Some(Origin::Dev)).map(|f| f.file).collect()
    }

    fn signatures(&self) -> HashMap<String, Sig> {
        let mut out = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(&self.folder) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !plain_yak(&name) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                if meta.is_file() {
                    out.insert(name, Some((meta.len(), meta.modified().ok())));
                }
            }
        }
        out
    }

    /// One look at the folder at time `now`: applies every change that has been the same for [`STABLE`].
    pub fn tick(&mut self, now: Instant) {
        let current = self.signatures();
        let names: BTreeSet<String> = current.keys().cloned().chain(self.installed.iter().cloned()).chain(self.obs.keys().cloned()).collect();
        for name in names {
            let sig = current.get(&name).cloned().unwrap_or(None);
            let fresh = !self.obs.get(&name).is_some_and(|o| o.sig == sig);
            if fresh {
                self.obs.insert(name.clone(), Obs { sig, since: now });
            }
            let stable = now.saturating_duration_since(self.obs[&name].since) >= STABLE;
            if !stable || self.acted.get(&name) == Some(&sig) {
                continue;
            }
            match self.apply(&name, sig.is_some()) {
                Ok((event, retry)) => {
                    if !retry {
                        self.acted.insert(name.clone(), sig);
                    }
                    if let Some(event) = event {
                        with_state(&self.instance, |s| {
                            s.last_event = Some(event);
                            s.last_error = None;
                        });
                    }
                }
                Err((error, retry)) => {
                    if !retry {
                        self.acted.insert(name.clone(), sig);
                    }
                    with_state(&self.instance, |s| s.last_error = Some(format!("{name}: {error}")));
                }
            }
            if sig.is_none() && self.acted.get(&name) == Some(&None) && !self.installed.contains(&name) {
                self.obs.remove(&name);
                self.acted.remove(&name);
            }
        }
    }

    /// Applies one stable change. `Ok((event, retry))` / `Err((problem, retry))`: `retry` when the file could not be
    /// read (it is tried again on the next look even if it does not change).
    fn apply(&mut self, name: &str, present: bool) -> Result<(Option<String>, bool), (String, bool)> {
        let _lock = lock();
        let result = if present { self.install(name) } else { self.remove(name) };
        let outcome = result?;
        if outcome.changed {
            self.rewrite().map_err(|e| (format!("the approved list could not be rewritten: {e}"), true))?;
        }
        let generation = ApprovedList::read(&self.approved).map(|l| l.generation()).unwrap_or(0);
        with_state(&self.instance, |s| s.generation = generation);
        Ok((Some(format!("{} (generation {generation})", outcome.event)), false))
    }

    /// Rewrites the run's approved list in place (temp file, then rename) with the next generation: the list a launch
    /// would pass now (the accepted entries that still match the files).
    fn rewrite(&self) -> Result<(), String> {
        let control = open_control(&self.data, &self.instance)?;
        let mods = mods_dir(&self.instance);
        let current = current_list(&control, &mods)?;
        let effective = effective_list(&load_last(&control, &self.instance), &current);
        let generation = ApprovedList::read(&self.approved).map(|l| l.generation()).unwrap_or(0) + 1;
        control::write_atomic(&self.approved, effective.with_generation(generation).to_text().as_bytes())
    }

    fn install(&mut self, name: &str) -> Result<Outcome, (String, bool)> {
        let path = self.folder.join(name);
        let bytes = std::fs::read(&path).map_err(|e| (format!("{}: {e}", path.display()), true))?;
        let fail = |e: String| (e, false);
        let manifest = Archive::new(Cursor::new(bytes.clone())).map(|a| a.manifest().clone()).map_err(|e| fail(format!("not a valid .yak: {e}")))?;
        let control = open_control(&self.data, &self.instance).map_err(fail)?;
        let mods = mods_dir(&self.instance);
        let origins = control.origins().map_err(fail)?;
        let found = scan(&mods);
        let is_dev = |f: &Found| origins.origin_of(&f.sha256) == Some(Origin::Dev);
        if !is_full_access(&manifest) {
            // a dev copy under this name that is no longer a full-access package goes away with it
            if found.iter().any(|f| f.file == name && is_dev(f)) {
                return self.remove(name);
            }
            return Ok(Outcome { event: format!("ignored {name}: not a full-access package"), changed: false });
        }
        let sha256 = control::sha256_hex(&bytes);
        // the same id from anywhere but the dev folder is never touched
        if let Some(other) = found.iter().find(|f| f.manifest.as_ref().is_ok_and(|m| m.id == manifest.id) && !is_dev(f)) {
            let origin = origins.origin_of(&other.sha256).map_or("unknown", Origin::as_str);
            return Err(fail(format!("{} is already installed from origin {origin} ({}): the dev folder never overwrites it; remove that one first", manifest.id, other.file)));
        }
        let replaced: Vec<&Found> = found.iter().filter(|f| is_dev(f) && (f.file == name || f.id() == manifest.id)).collect();
        let unchanged = replaced.len() == 1 && replaced[0].file == name && replaced[0].sha256 == sha256;
        let mut changed = !unchanged;
        if !unchanged {
            let (mut allow, mut origins) = (control.allow().map_err(fail)?, control.origins().map_err(fail)?);
            for old in &replaced {
                if !found.iter().any(|f| f.sha256 == old.sha256 && !replaced.iter().any(|r| r.file == f.file)) {
                    origins.forget(&old.sha256);
                }
                allow.revoke(&old.id());
            }
            control.save_allow(&allow).map_err(fail)?;
            control.save_origins(&origins).map_err(fail)?;
            for old in &replaced {
                std::fs::remove_file(&old.path).map_err(|e| fail(format!("{}: {e}", old.path.display())))?;
            }
            control::install_package(&control, &mods, name, &bytes, Origin::Dev, None).map_err(fail)?;
        }
        // approve exactly these bytes and accept them for the instance (idempotent for an unchanged file)
        let allowed = control.allow().map_err(fail)?.is_allowed(&manifest.id, &sha256);
        if !allowed {
            control.approve(&bytes).map_err(fail)?;
        }
        let current = current_list(&control, &mods).map_err(fail)?;
        if !load_last(&control, &self.instance).entries().iter().any(|e| e.sha256 == sha256) {
            changed = true;
        }
        last_with(&control, &self.instance, &manifest.id, Some(&current)).map_err(fail)?;
        changed |= !allowed;
        self.installed.insert(name.to_owned());
        let verb = if replaced.is_empty() { "installed" } else if unchanged { "kept" } else { "updated" };
        Ok(Outcome { event: format!("{verb} {} {} from the dev folder", manifest.id, manifest.version), changed })
    }

    fn remove(&mut self, name: &str) -> Result<Outcome, (String, bool)> {
        let fail = |e: String| (e, false);
        let control = open_control(&self.data, &self.instance).map_err(fail)?;
        let origins = control.origins().map_err(fail)?;
        let found = scan(&mods_dir(&self.instance));
        let Some(item) = found.iter().find(|f| f.file == name) else {
            self.installed.remove(name);
            return Ok(Outcome { event: format!("{name} was removed (nothing installed)"), changed: false });
        };
        // never a package from another origin, whatever its name
        if origins.origin_of(&item.sha256) != Some(Origin::Dev) {
            self.installed.remove(name);
            return Err(fail(format!("{name} in mods/ is not a dev-folder copy; left alone")));
        }
        let id = item.id();
        remove_file_unlocked(&control, &self.instance, name).map_err(fail)?;
        self.installed.remove(name);
        Ok(Outcome { event: format!("removed {id} ({name}) with the dev folder's file"), changed: true })
    }
}

/// A running watcher: a worker thread that looks at the folder every [`POLL`]. Dropping it stops the thread.
#[derive(Debug)]
pub struct DevWatcher {
    instance: PathBuf,
    stop: Option<mpsc::Sender<()>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for DevWatcher {
    fn drop(&mut self) {
        self.stop = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        with_state(&self.instance, |s| s.watching = false);
    }
}

/// Starts the watcher for a launch: refuses a folder that [`validate_dev_folder`] does not allow for this instance.
pub(super) fn start(data: &Path, instance: &Path, folder: &Path, approved: &Path) -> Result<DevWatcher, String> {
    let folder = validate_dev_folder(folder, data, &[instance.to_path_buf()])?;
    let mods = mods_dir(instance);
    if is_inside(&folder, &mods) {
        return Err(format!("{} is the instance's mods folder", folder.display()));
    }
    let mut sync = DevSync::new(data, instance, &folder, approved);
    let (stop, rx) = mpsc::channel::<()>();
    let handle = std::thread::Builder::new()
        .name("full-access-dev-folder".into())
        .spawn(move || loop {
            sync.tick(Instant::now());
            if !matches!(rx.recv_timeout(POLL), Err(mpsc::RecvTimeoutError::Timeout)) {
                break;
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(DevWatcher { instance: instance.to_path_buf(), stop: Some(stop), handle: Some(handle) })
}

/// A launch whose dev folder could not be used: no flag, no watcher, and the status says why.
pub(super) fn note_start_failed(instance: &Path, folder: &Path, error: &str) {
    with_state(instance, |s| {
        s.watching = false;
        s.last_error = Some(format!("{}: {error}", folder.display()));
    });
}
