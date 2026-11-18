//! MX3L1/MX3L2: the full-access mod loader (feature `yoake2-slice`).
//!
//! A full-access mod is a `.yak` whose manifest says `[wasm] world = "full-access"` with a `server` module that
//! is a core WebAssembly module (not a component) speaking the `engine_call` ABI (docs/yoake2/ENGINE_CALL_ABI.md).
//! It has the powers of the engine itself, so permission is bound to **content** and the origin is **metadata**:
//!
//! * The player's approval is an entry (id, version and the **SHA-256 of the package file**) in the launcher's
//!   `full-access.toml`. A package whose bytes changed (an update, a replaced file) no longer matches, is not loaded
//!   and is reported ([`Refusal::ContentChanged`] / [`Refusal::UnknownOrigin`]).
//! * Where a package came from is recorded **when it arrives** in the launcher's `yak-origins.toml` (keyed by the package
//!   hash; written by [`install_package`]), never guessed from the folder it sits in. A package with no record has an
//!   unknown origin and is **never** full-access ([`Refusal::UnknownOrigin`]). A package that came from a **server or a
//!   friend** is never full-access, even if its hash is approved (design 2.5); such packages are stored apart, in
//!   `<mods>/remote/<origin>/`, which this loader never scans. Only `local` and `catalog` packages can run.
//! * **The game does not read those files** (MX3L4). The launcher verifies them and passes this launch's approved list
//!   ([`ApprovedList`]: `(id, version, sha256, origin)` entries) in a file named by `--full-access-approved <path>`
//!   (or `YOAKE_FULL_ACCESS_APPROVED`); the loader turns it into the same allow/origin pair and loads only packages whose
//!   bytes hash to an entry. No list: nothing is full-access. A mod that rewrites the control files changes nothing
//!   in this launch. The format, the path and the lifetime are documented on [`ApprovedList`].
//! * Two packages with the same id in the mods folder make both refuse ([`Refusal::DuplicateId`]): nobody can slip a
//!   second file in under an approved id.
//!
//! A manifest without `world = "full-access"` is not this loader's business: it stays on the sandboxed WebAssembly
//! path ([`Refusal::Sandboxed`] says so; that path in turn skips full-access packages, `yak_mods::wasm_mods_from`).
//!
//! Mods load on the thread the loader runs on (the integrated-server thread at world start): the runtime keeps
//! its mods per thread, each in its own store; for a hook id they run in priority order, each wrapping the next.
//! Full-access mods run with [`ModConfig::full_access`]: no fuel limit, a watchdog that only logs slow activations.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use horizon_mods::yak::{Archive, PublicKey};
pub use horizon_mods::yak::control::{
    APPROVED_ARG, APPROVED_ENV, AllowEntry, AllowFile, ApprovedEntry, ApprovedList, Control, ListDiff, Origin, OriginFile, OriginRecord,
    control_dir_in, guard, install_package, remove_package, sha256_hex,
};
use yoake_slice::runtime::{self, ModConfig, ModId};

fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

/// Why a package was not loaded as a full-access mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a full-access package: leave it to the sandboxed WebAssembly path.
    Sandboxed,
    /// A full-access package nobody allowed.
    NotAllowed,
    /// The id was allowed, but for other content: the package changed since the player approved it (an update, a
    /// replaced file). It needs approving again. `allowed` are the approved hashes, `actual` this package's.
    ContentChanged { id: String, allowed: Vec<String>, actual: String },
    /// No origin was recorded for this package (or the record is unusable): never full-access.
    UnknownOrigin { sha256: String },
    FromServer,
    FromFriend,
    /// Another package in the folder has the same id: neither is loaded as full-access.
    DuplicateId { id: String, others: Vec<String> },
    /// A catalog package whose signature does not verify against the trust list.
    BadSignature(String),
    /// The package is malformed.
    Package(String),
    /// `yoake_init` failed or the module did not instantiate.
    Init(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Sandboxed => write!(f, "not a full-access package (left to the sandboxed path)"),
            Refusal::NotAllowed => write!(f, "full-access mod nobody approved (add it to {})", AllowFile::FILE_NAME),
            Refusal::ContentChanged { id, allowed, actual } => write!(
                f,
                "the package of '{id}' changed since it was approved (approved {}, this file is {}); approve it again in {}",
                allowed.iter().map(|h| short(h)).collect::<Vec<_>>().join(", "),
                short(actual),
                AllowFile::FILE_NAME
            ),
            Refusal::UnknownOrigin { sha256 } => write!(
                f,
                "no origin is recorded for this package ({}) in {}: a package of unknown origin is never full-access",
                short(sha256),
                OriginFile::FILE_NAME
            ),
            Refusal::FromServer => write!(f, "delivered by a server: a server's package is never full-access"),
            Refusal::FromFriend => write!(f, "delivered by a friend: a friend's package is never full-access"),
            Refusal::DuplicateId { id, others } => {
                write!(f, "another package has the same id '{id}' ({}): neither is loaded as full-access", others.join(", "))
            }
            Refusal::BadSignature(e) => write!(f, "signature does not verify: {e}"),
            Refusal::Package(e) => write!(f, "malformed package: {e}"),
            Refusal::Init(e) => write!(f, "could not start: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Policy and loading
// ---------------------------------------------------------------------------------------------------------------

/// The policy: may a full-access package of this origin run? `approved` is "the player approved this exact
/// content"; `signed_by_trusted` a signature of a key on the trust list.
pub fn decide(origin: Origin, approved: bool, signed_by_trusted: bool) -> Result<(), Refusal> {
    match origin {
        Origin::Server => Err(Refusal::FromServer),
        Origin::Friend => Err(Refusal::FromFriend),
        Origin::Local if approved => Ok(()),
        Origin::Catalog if approved || signed_by_trusted => Ok(()),
        Origin::Local | Origin::Catalog => Err(Refusal::NotAllowed),
    }
}

/// One mod that was loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// The manifest's id.
    pub id: String,
    pub module: ModId,
    /// The package's SHA-256.
    pub sha256: String,
}

/// Loads one package from `bytes` if it is a full-access mod and the policy lets it run. `origin` is what was
/// recorded when the package arrived (`None`: nothing was, so it is never full-access). Must run on the thread
/// whose hooks it should join.
pub fn load_package(bytes: Vec<u8>, origin: Option<Origin>, allow: &AllowFile, trusted: &[PublicKey], cfg: ModConfig) -> Result<Loaded, Refusal> {
    let sha256 = sha256_hex(&bytes);
    let mut archive = Archive::new(Cursor::new(bytes)).map_err(Refusal::Package)?;
    let id = archive.manifest().id.clone();
    if !archive.manifest().wasm.as_ref().is_some_and(|wasm| wasm.full_access()) {
        return Err(Refusal::Sandboxed);
    }
    // an unknown origin, a server's or a friend's package is refused before anything else looks at it
    let origin = match origin {
        None => return Err(Refusal::UnknownOrigin { sha256 }),
        Some(Origin::Server) => return Err(Refusal::FromServer),
        Some(Origin::Friend) => return Err(Refusal::FromFriend),
        Some(origin) => origin,
    };
    let signed = match archive.verify_signature(trusted) {
        Ok(signer) => signer.is_some(),
        // a signature that does not verify only matters for a catalog package: a local file is the player's own
        Err(e) if origin == Origin::Catalog => return Err(Refusal::BadSignature(e)),
        Err(_) => false,
    };
    if let Err(refusal) = decide(origin, allow.is_allowed(&id, &sha256), signed) {
        // the id is on the list but this content is not: say so, it is an update or a replaced file
        let approved = allow.approved_hashes(&id);
        return Err(match refusal {
            Refusal::NotAllowed if !approved.is_empty() => Refusal::ContentChanged { id, allowed: approved, actual: sha256 },
            other => other,
        });
    }
    let module = archive.full_access_module().map_err(Refusal::Package)?.ok_or_else(|| Refusal::Package("no full-access module".into()))?;
    let module = runtime::load(&module, cfg, &id).map_err(|e| Refusal::Init(format!("{e:#}")))?;
    Ok(Loaded { id, module, sha256 })
}

/// What a folder scan did.
#[derive(Debug, Default)]
pub struct LoadReport {
    pub loaded: Vec<Loaded>,
    pub skipped: Vec<(PathBuf, Refusal)>,
    /// Things that are not about one package (an unreadable switch file, a mod that would not unload).
    pub problems: Vec<String>,
    /// Ids of old-style approvals (no package hash) found in the switch file; they approve nothing.
    pub legacy_notes: Vec<String>,
    /// Per loaded mod (manifest id), the hook ids it registered that no generated bridge could install
    /// ([`install_bridges`]): the runtime keeps those registrations, but the engine never enters them.
    pub unbridged: Vec<(String, Vec<u32>)>,
}

impl LoadReport {
    /// One line per thing the player should know: every refusal except a plain sandboxed package, and the problems.
    pub fn messages(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.problems.clone();
        for (path, refusal) in &self.skipped {
            if *refusal != Refusal::Sandboxed {
                lines.push(format!("full-access mod {} not loaded: {refusal}", path.display()));
            }
        }
        for id in &self.legacy_notes {
            lines.push(format!("full-access.toml names '{id}' without a package hash (an old entry): it approves nothing; approve the package again"));
        }
        for (id, hooks) in &self.unbridged {
            lines.push(format!("full-access mod '{id}' registered hook ids {hooks:?} that this engine build cannot call (no bridge): they will never run"));
        }
        lines
    }
}

/// Loads every `*.yak` of a mods folder (sorted by file name, so the load order is stable) that is an approved
/// full-access mod. `origins` says where each package came from. Packages that are not full-access are listed as
/// skipped with [`Refusal::Sandboxed`]. Two packages with the same id make every full-access one of them refuse.
pub fn load_dir(dir: &Path, origins: &OriginFile, allow: &AllowFile, trusted: &[PublicKey], cfg: &ModConfig) -> LoadReport {
    let mut report = LoadReport::default();
    let Ok(entries) = std::fs::read_dir(dir) else { return report };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "yak")).collect();
    files.sort();
    // pass 1: read every package, find the ids that appear twice
    struct Pack {
        path: PathBuf,
        bytes: Vec<u8>,
        /// The manifest's id when the package parses.
        id: Option<String>,
    }
    let mut packs = Vec::new();
    for path in files {
        match std::fs::read(&path) {
            Ok(bytes) => {
                let id = Archive::new(Cursor::new(bytes.clone())).ok().map(|a| a.manifest().id.clone());
                packs.push(Pack { path, bytes, id });
            }
            Err(e) => report.skipped.push((path, Refusal::Package(e.to_string()))),
        }
    }
    let mut by_id: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for pack in &packs {
        if let Some(id) = &pack.id {
            by_id.entry(id.clone()).or_default().push(pack.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        }
    }
    // pass 2: load
    for pack in packs {
        if let Some(id) = &pack.id {
            let names = &by_id[id];
            if names.len() > 1 {
                // a duplicate id: every full-access package of the group refuses (a sandboxed one is not our business)
                let full_access = Archive::new(Cursor::new(pack.bytes.clone()))
                    .ok()
                    .is_some_and(|a| a.manifest().wasm.as_ref().is_some_and(|w| w.full_access()));
                let me = pack.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let others = names.iter().filter(|n| **n != me).cloned().collect();
                report.skipped.push((pack.path, if full_access { Refusal::DuplicateId { id: id.clone(), others } } else { Refusal::Sandboxed }));
                continue;
            }
        }
        let origin = origins.origin_of(&sha256_hex(&pack.bytes));
        match load_package(pack.bytes, origin, allow, trusted, cfg.clone()) {
            Ok(loaded) => report.loaded.push(loaded),
            Err(refusal) => report.skipped.push((pack.path, refusal)),
        }
    }
    for id in allow.legacy_ids() {
        report.legacy_notes.push(id.clone());
    }
    report
}

thread_local! {
    static LOADED: std::cell::RefCell<Vec<ModId>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Unloads the mods of the previous world; returns what could not be unloaded (those stay in the list).
fn unload_previous() -> Vec<String> {
    LOADED.with(|loaded| {
        let mut errors = Vec::new();
        let mut still = Vec::new();
        for id in loaded.borrow_mut().drain(..) {
            if let Err(e) = runtime::unload(id) {
                errors.push(format!("could not unload mod {id:?} of the previous world: {e:#}"));
                still.push(id);
            }
        }
        loaded.borrow_mut().extend(still);
        errors
    })
}

/// The list of this launch (`--full-access-approved` / `YOAKE_FULL_ACCESS_APPROVED`), read **once**, at the first call,
/// before any mod can run, and kept in memory. The list file and every path it names as protected are registered with
/// [`guard`]. `None`: no list was given or it could not be read, so nothing is full-access (the reason is printed).
pub fn process_approved_list() -> Option<&'static ApprovedList> {
    static LIST: std::sync::OnceLock<Option<ApprovedList>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        let path = ApprovedList::path_from_process()?;
        horizon_mods::yak::control::protect(&path);
        match ApprovedList::read(&path) {
            Ok(list) => {
                for protected in list.protected() {
                    horizon_mods::yak::control::protect(protected);
                }
                Some(list)
            }
            Err(e) => {
                log::warn!("yoake: {e}; no full-access mod will run");
                eprintln!("yoake: {e}; no full-access mod will run");
                None
            }
        }
    })
    .as_ref()
}

/// World start: replaces this thread's full-access mods with those of `<game root>/mods` that this launch's `approved`
/// list names (a package loads only when its bytes hash to an entry and the entry's origin is `local` or `catalog`).
/// **Without a list no mod is full-access.** The compiled-in trust list checks catalog signatures and
/// [`ModConfig::full_access`] gives the limits. Mods of a previous world are unloaded first; when one cannot be
/// unloaded nothing new is loaded (two generations of mods on one hook would be worse) and the report says why.
pub fn load_world_mods(mods_dir: &Path, approved: Option<&ApprovedList>) -> LoadReport {
    let errors = unload_previous();
    if !errors.is_empty() {
        let report = LoadReport { problems: errors, ..LoadReport::default() };
        announce(&report);
        return report;
    }
    let (allow, origins) = approved.map(ApprovedList::to_files).unwrap_or_default();
    let trusted = horizon_mods::yak::trusted_keys();
    let mut report = load_dir(mods_dir, &origins, &allow, &trusted, &ModConfig::full_access());
    // the "no list" note is only worth a line when there is a full-access package that it affects
    if approved.is_none() && report.skipped.iter().any(|(_, r)| !matches!(r, Refusal::Sandboxed)) {
        report.problems.push(format!("the launcher gave no approved list ({APPROVED_ARG} or {APPROVED_ENV}): no full-access mod will run"));
    }
    LOADED.with(|loaded| loaded.borrow_mut().extend(report.loaded.iter().map(|l| l.module)));
    report.unbridged = install_bridges(&report.loaded);
    announce(&report);
    report
}

/// Process-wide: ledger hook id -> whether its generated dispatcher is installed. Dispatchers go into the engine's
/// global hook slots and registering one appends to the slot's chain, so each id is tried **once per process**
/// (a second world, or a second mod with the same id, must not add a second dispatcher: the guests would run
/// twice). An installed dispatcher whose id no loaded mod registers (the mod was unloaded, or it runs on another
/// thread) only looks up the empty chain and runs the original.
static BRIDGES: std::sync::Mutex<BTreeMap<u32, bool>> = std::sync::Mutex::new(BTreeMap::new());

/// After the mods' `yoake_init` registered their hook ids: installs the generated bridge dispatcher
/// (`hook_bridge::install`, docs/yoake2/HOOK-SLICE-BRIDGE.md) of every id any of `loaded` registered, the union
/// over the mods, each id once per process. Which mods run, and in what order, is the runtime's chain for that id;
/// the dispatcher's own slot order uses the priority of the first mod that registered it. Returns, per mod, the
/// ids no bridge could install (not in a bridged module, arg0 not bridgeable, see each bridge's `UNBRIDGED`).
/// `load_world_mods` calls this; on its own it is for tests that load with [`load_dir`]. Must run on the thread
/// the mods were loaded on.
pub fn install_bridges(loaded: &[Loaded]) -> Vec<(String, Vec<u32>)> {
    // id 0 is a legacy single-export guest (no `hook_register`): not a ledger id
    let per_mod: Vec<(String, Vec<(u32, i32)>)> = loaded
        .iter()
        .map(|l| {
            let registered = runtime::stats_of(l.module).map(|s| s.registered).unwrap_or_default();
            (l.id.clone(), registered.into_iter().filter(|(id, _)| *id != 0).collect())
        })
        .collect();
    let mut bridges = BRIDGES.lock().unwrap_or_else(|e| e.into_inner());
    for (_, registered) in &per_mod {
        for &(id, priority) in registered {
            if !bridges.contains_key(&id) {
                bridges.insert(id, install_bridge(id, priority));
            }
        }
    }
    per_mod
        .into_iter()
        .filter_map(|(mod_id, registered)| {
            let missing: Vec<u32> = registered.iter().map(|(id, _)| *id).filter(|id| !bridges[id]).collect();
            (!missing.is_empty()).then_some((mod_id, missing))
        })
        .collect()
}

/// Tries the generated bridges in turn (ledger ids are unique across crates, so at most one knows `id`).
fn install_bridge(id: u32, priority: i32) -> bool {
    crate::entities::hook_bridge::install(&[(id, priority)]).is_empty() || crate::game::item_use_hook_bridge::install(&[(id, priority)]).is_empty()
}

fn announce(report: &LoadReport) {
    for line in report.messages() {
        log::warn!("yoake: {line}");
        eprintln!("yoake: {line}");
    }
}
