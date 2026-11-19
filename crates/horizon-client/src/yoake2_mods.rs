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
//!   friend** (MX3JN1: join-time mod sync, docs/yoake2/JOIN-MODS.md) is stored apart, in `<mods>/remote/<host dir>/`, which
//!   the plain scan never reads, and runs **only** when the launcher's approved list for a launch that joins exactly that
//!   host names it (`ApprovedList::host` = the `--remote-mods` key, entry origin `server`/`friend`, bytes hashing to the
//!   entry) and only on the client thread. Every other way a server or friend package could arrive (no list, another
//!   host's list, a hand-dropped file, a list without a host scope) is still refused ([`Refusal::FromServer`] /
//!   [`Refusal::FromFriend`]); the MX3L2 rule "never full-access for server/friend origin" now reads "never without the
//!   launcher's approval for that host". `local` and `catalog` work as before.
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

use horizon_mods::yak::{Archive, PublicKey, Wasm};
pub use horizon_mods::yak::control::{
    APPROVED_ARG, APPROVED_ENV, DEV_ARG, AllowEntry, AllowFile, ApprovedEntry, ApprovedList, Control, ListDiff, Origin, OriginFile, OriginRecord,
    control_dir_in, guard, install_package, remove_package, sha256_hex,
};
use yoake_slice::runtime::{self, ModConfig, ModId};

fn short(hash: &str) -> &str {
    hash.get(..12).unwrap_or(hash)
}

/// MX3CL1: the thread a loader call fills. The runtime keeps its mods per thread, so "the server's mods" and "the
/// client's mods" are two separate runtimes (two separate instances of a `both` mod).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadSide {
    /// The simulation thread (integrated or dedicated server).
    Server,
    /// The window's thread.
    Client,
}

impl ThreadSide {
    fn index(self) -> usize {
        match self {
            ThreadSide::Server => 0,
            ThreadSide::Client => 1,
        }
    }

    /// Whether a mod declares a module for this thread: `wasm.server` runs on the server thread, `wasm.client` on the
    /// client thread (a mod may declare both: two separate instances).
    pub fn runs(self, wasm: &Wasm) -> bool {
        match self {
            ThreadSide::Server => wasm.server.is_some(),
            ThreadSide::Client => wasm.client.is_some(),
        }
    }

    fn other(self) -> ThreadSide {
        match self {
            ThreadSide::Server => ThreadSide::Client,
            ThreadSide::Client => ThreadSide::Server,
        }
    }
}

/// Why a package was not loaded as a full-access mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a full-access package: leave it to the sandboxed WebAssembly path.
    Sandboxed,
    /// A full-access package that declares no module for this thread (`wasm.server` / `wasm.client`): not an
    /// error, the other loader takes it.
    OtherSide,
    /// This thread already holds the other side\x27s mods (a server runtime and a client runtime would share one
    /// thread-local runtime, and their hooks would mix): nothing is loaded.
    SharedThread { loading: ThreadSide, holds: ThreadSide },
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
    /// The module's ABI version (`yoake_abi` custom section, ENGINE_CALL_ABI.md "ABI version") is missing, malformed
    /// or of another major version than this engine's. Checked before any guest code runs.
    Abi(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Sandboxed => write!(f, "not a full-access package (left to the sandboxed path)"),
            Refusal::OtherSide => write!(f, "a full-access mod for the other side (left to the other loader)"),
            Refusal::SharedThread { loading, holds } => write!(
                f,
                "this thread already runs {holds:?}-side full-access mods: {loading:?}-side mods would share its runtime (server and client mods need separate threads), not loaded"
            ),
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
            Refusal::FromServer => write!(f, "delivered by a server and not approved for that server (a server's package runs only when the launcher approved it for this host)"),
            Refusal::FromFriend => write!(f, "delivered by a friend and not approved for that host (a friend's package runs only when the launcher approved it for this host)"),
            Refusal::DuplicateId { id, others } => {
                write!(f, "another package has the same id '{id}' ({}): neither is loaded as full-access", others.join(", "))
            }
            Refusal::BadSignature(e) => write!(f, "signature does not verify: {e}"),
            Refusal::Package(e) => write!(f, "malformed package: {e}"),
            Refusal::Init(e) => write!(f, "could not start: {e}"),
            Refusal::Abi(e) => write!(f, "incompatible ABI: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Policy and loading
// ---------------------------------------------------------------------------------------------------------------

/// The policy: may a full-access package of this origin run? `approved` is "the launcher approved this exact
/// content" (for a `Local` package, the player did). `signed_by_trusted` a signature of a key on the trust list.
///
/// MX3JN1: a `Server` or `Friend` package runs **only when approved**, and approval of such a package exists only in
/// the approved list of a launch scoped to its host (`ApprovedList::to_remote_files`, which the loader hands only to the
/// `mods/remote/<host dir>/` folder of the `--remote-mods` host). A signature never replaces that approval.
pub fn decide(origin: Origin, approved: bool, signed_by_trusted: bool) -> Result<(), Refusal> {
    match origin {
        Origin::Server if approved => Ok(()),
        Origin::Friend if approved => Ok(()),
        Origin::Server => Err(Refusal::FromServer),
        Origin::Friend => Err(Refusal::FromFriend),
        // `Dev`: the launcher copied it from the player's own developer folder and approved exactly those bytes
        Origin::Local | Origin::Dev if approved => Ok(()),
        Origin::Catalog if approved || signed_by_trusted => Ok(()),
        Origin::Local | Origin::Catalog | Origin::Dev => Err(Refusal::NotAllowed),
    }
}

/// One mod that was loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// The manifest's id.
    pub id: String,
    /// The manifest's version.
    pub version: String,
    pub module: ModId,
    /// The package's SHA-256.
    pub sha256: String,
    /// The ABI version the module was built for (same major as this engine's).
    pub abi: runtime::AbiVersion,
}

/// Loads one package from `bytes` if it is a full-access mod and the policy lets it run. `origin` is what was
/// recorded when the package arrived (`None`: nothing was, so it is never full-access). Must run on the thread
/// whose hooks it should join.
pub fn load_package(bytes: Vec<u8>, origin: Option<Origin>, allow: &AllowFile, trusted: &[PublicKey], cfg: ModConfig) -> Result<Loaded, Refusal> {
    load_package_on(ThreadSide::Server, bytes, origin, allow, trusted, cfg)
}

/// [`load_package`] for the thread `side`: a mod whose `[wasm] side` does not include it is [`Refusal::OtherSide`]
/// (checked before the policy, so the other thread's mods are never judged, or reported, twice). The policy is the
/// same on both sides.
pub fn load_package_on(side: ThreadSide, bytes: Vec<u8>, origin: Option<Origin>, allow: &AllowFile, trusted: &[PublicKey], cfg: ModConfig) -> Result<Loaded, Refusal> {
    let sha256 = sha256_hex(&bytes);
    let mut archive = Archive::new(Cursor::new(bytes)).map_err(Refusal::Package)?;
    let id = archive.manifest().id.clone();
    let version = archive.manifest().version.to_string();
    match archive.manifest().wasm.as_ref().filter(|wasm| wasm.full_access()) {
        None => return Err(Refusal::Sandboxed),
        Some(wasm) if !side.runs(wasm) => return Err(Refusal::OtherSide),
        Some(_) => {}
    }
    // one runtime per thread: a client runtime and a server runtime must never be filled on the same thread
    if LOADED.with(|loaded| !loaded.borrow()[side.other().index()].is_empty()) {
        let refusal = Refusal::SharedThread { loading: side, holds: side.other() };
        log::warn!("yoake: full-access mod \x27{id}\x27 {refusal}");
        eprintln!("yoake: full-access mod \x27{id}\x27 {refusal}");
        return Err(refusal);
    }
    // an unknown origin is refused before anything else looks at it; a server's or friend's package goes through
    // `decide` like the rest and runs only when approved (MX3JN1)
    let origin = match origin {
        None => return Err(Refusal::UnknownOrigin { sha256 }),
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
    let module = archive.full_access_module(side == ThreadSide::Client).map_err(Refusal::Package)?.ok_or_else(|| Refusal::Package("no full-access module".into()))?;
    // the ABI version is read from the module bytes: no guest code has run yet
    let abi = match runtime::module_abi_version(&module) {
        Err(e) => return Err(Refusal::Abi(e.to_string())),
        Ok(None) => {
            return Err(Refusal::Abi(format!(
                "the module has no `yoake_abi` section (built without the SDK's export_guest!, or for an older engine); this engine speaks {}.{}",
                runtime::ABI_VERSION.major,
                runtime::ABI_VERSION.minor
            )));
        }
        Ok(Some(version)) => version,
    };
    if let runtime::AbiCheck::Refuse(why) = runtime::check_abi(abi) {
        return Err(Refusal::Abi(why));
    }
    let module = runtime::load(&module, cfg, &id).map_err(|e| Refusal::Init(format!("{e:#}")))?;
    Ok(Loaded { id, version, module, sha256, abi })
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
    pub unbridged: Vec<(String, Vec<(u32, String)>)>,
}

impl LoadReport {
    /// One line per thing the player should know: every refusal except a plain sandboxed package, and the problems.
    pub fn messages(&self) -> Vec<String> {
        let mut lines: Vec<String> = self.problems.clone();
        for (path, refusal) in &self.skipped {
            if !matches!(refusal, Refusal::Sandboxed | Refusal::OtherSide) {
                lines.push(format!("full-access mod {} not loaded: {refusal}", path.display()));
            }
        }
        for id in &self.legacy_notes {
            lines.push(format!("full-access.toml names '{id}' without a package hash (an old entry): it approves nothing; approve the package again"));
        }
        for loaded in &self.loaded {
            if loaded.abi.minor > runtime::ABI_VERSION.minor {
                lines.push(format!(
                    "full-access mod '{}' was built for ABI {}.{}, this engine speaks {}.{}: engine calls it makes that this engine does not know are refused",
                    loaded.id,
                    loaded.abi.major,
                    loaded.abi.minor,
                    runtime::ABI_VERSION.major,
                    runtime::ABI_VERSION.minor
                ));
            }
        }
        for (id, hooks) in &self.unbridged {
            let list: Vec<String> = hooks.iter().map(|(hook, why)| format!("{hook} ({why})")).collect();
            lines.push(format!("full-access mod '{id}' registered hooks this engine build cannot call, they will never run: {}", list.join(", ")));
        }
        lines
    }
}

/// Loads every `*.yak` of a mods folder (sorted by file name, so the load order is stable) that is an approved
/// full-access mod. `origins` says where each package came from. Packages that are not full-access are listed as
/// skipped with [`Refusal::Sandboxed`]. Two packages with the same id make every full-access one of them refuse.
pub fn load_dir(dir: &Path, origins: &OriginFile, allow: &AllowFile, trusted: &[PublicKey], cfg: &ModConfig) -> LoadReport {
    load_dir_on(ThreadSide::Server, dir, origins, allow, trusted, cfg)
}

/// [`load_dir`] for the thread `side` (MX3CL1): only mods whose `[wasm] side` includes it are considered; the others
/// are listed as [`Refusal::OtherSide`].
pub fn load_dir_on(side: ThreadSide, dir: &Path, origins: &OriginFile, allow: &AllowFile, trusted: &[PublicKey], cfg: &ModConfig) -> LoadReport {
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
                let wasm = Archive::new(Cursor::new(pack.bytes.clone())).ok().and_then(|a| a.manifest().wasm.clone()).filter(|w| w.full_access());
                let full_access = wasm.is_some();
                let other_side = wasm.as_ref().is_some_and(|w| !side.runs(w));
                let me = pack.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let others = names.iter().filter(|n| **n != me).cloned().collect();
                report.skipped.push((pack.path, if other_side { Refusal::OtherSide } else if full_access { Refusal::DuplicateId { id: id.clone(), others } } else { Refusal::Sandboxed }));
                continue;
            }
        }
        let origin = origins.origin_of(&sha256_hex(&pack.bytes));
        match load_package_on(side, pack.bytes, origin, allow, trusted, cfg.clone()) {
            Ok(loaded) => report.loaded.push(loaded),
            Err(refusal) => report.skipped.push((pack.path, refusal)),
        }
    }
    for id in allow.legacy_ids() {
        report.legacy_notes.push(id.clone());
    }
    report
}

/// MX3JN1: the SHA-256s (lowercase hex) of the full-access packages the server thread runs now, for the connection
/// threads that offer them to joining players (`join_mods::offer_from`). Replaced at every world start.
static SERVED: std::sync::Mutex<std::collections::BTreeSet<String>> = std::sync::Mutex::new(std::collections::BTreeSet::new());

/// Whether the server thread runs the full-access package with this SHA-256 (lowercase hex).
pub fn server_runs_package(sha256_hex: &str) -> bool {
    SERVED.lock().unwrap_or_else(|e| e.into_inner()).contains(sha256_hex)
}

thread_local! {
    /// This thread's loaded mods, per [`ThreadSide`] (indexed by `ThreadSide::index`).
    static LOADED: std::cell::RefCell<[Vec<(ModId, String)>; 2]> = const { std::cell::RefCell::new([Vec::new(), Vec::new()]) };
}

/// Unloads the mods of the previous world; returns what could not be unloaded (those stay in the list).
fn unload_previous(side: ThreadSide) -> Vec<String> {
    LOADED.with(|loaded| {
        let mut errors = Vec::new();
        let mut still: Vec<(ModId, String)> = Vec::new();
        for (id, hash) in loaded.borrow_mut()[side.index()].drain(..) {
            if let Err(e) = runtime::unload(id) {
                errors.push(format!("could not unload mod {id:?} of the previous world: {e:#}"));
                still.push((id, hash));
            }
        }
        // S7: what is offered to joining players is what still runs (the mods that were unloaded no longer do)
        if side == ThreadSide::Server && !errors.is_empty() {
            *SERVED.lock().unwrap_or_else(|e| e.into_inner()) = still.iter().map(|(_, hash)| hash.clone()).collect();
        }
        loaded.borrow_mut()[side.index()].extend(still);
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

/// The list a world or session start loads with: [`process_approved_list`], except that with `--full-access-dev` a newer
/// list read by the dev reload ([`dev_reload_check`]) replaces it (a world started after a reload must not fall back to
/// the launch-time list). A clone: the list is small.
pub fn launch_approved_list() -> Option<ApprovedList> {
    if dev_mode() {
        if let Some(latest) = DEV_LATEST.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            return Some(latest);
        }
    }
    process_approved_list().cloned()
}

/// World start: replaces this thread's full-access mods with those of `<game root>/mods` that this launch's `approved`
/// list names (a package loads only when its bytes hash to an entry and the entry's origin is `local` or `catalog`).
/// **Without a list no mod is full-access.** The compiled-in trust list checks catalog signatures and
/// [`ModConfig::full_access`] gives the limits. Mods of a previous world are unloaded first; when one cannot be
/// unloaded nothing new is loaded (two generations of mods on one hook would be worse) and the report says why.
pub fn load_world_mods(mods_dir: &Path, approved: Option<&ApprovedList>) -> LoadReport {
    load_mods_on(ThreadSide::Server, mods_dir, approved)
}

/// MX3CL1: the client's session start: [`load_world_mods`] for the window's thread. M1: the joined host's mods load only
/// when the connection of this session goes to the host the launch was made for (`join_mods::session_remote_key`). Loads the approved full-access mods
/// whose `[wasm] side` is `client` or `both` into this thread's own runtime (separate from the server's, even for a `both`
/// mod). The same approved list and trust rules; the mods' own `yoake_init` runs here.
pub fn load_client_mods(mods_dir: &Path, approved: Option<&ApprovedList>) -> LoadReport {
    load_client_mods_scoped(mods_dir, approved, horizon_client::join_mods::session_remote_key().as_deref())
}

/// MX3JN1: [`load_client_mods`] for a launch that joins the host `remote_key` (`--remote-mods`): besides `mods_dir`, the
/// packages of `mods_dir/remote/<host dir>/` that the approved list names **for that host** (origin `server`/`friend`,
/// list scoped to the same key) run on the client thread. Another host's folder is never read, and without a key or a
/// matching list nothing remote loads. A remote package whose id a local mod already runs is not loaded (the local one wins).
pub fn load_client_mods_scoped(mods_dir: &Path, approved: Option<&ApprovedList>, remote_key: Option<&str>) -> LoadReport {
    load_mods_scoped(ThreadSide::Client, mods_dir, approved, remote_key)
}

/// M1: [`load_client_mods`] for a session of this process with `process_key` (`--remote-mods`) that is connected to `host`
/// (None: single player, a replay, a pseudo-address): the host's folder loads only when they are the same host.
pub fn load_client_mods_for_host(mods_dir: &Path, approved: Option<&ApprovedList>, process_key: Option<&str>, host: Option<&horizon_client::join_mods::HostId>) -> LoadReport {
    let key = horizon_client::join_mods::session_remote_key_for(process_key, host);
    load_client_mods_scoped(mods_dir, approved, key.as_deref())
}

/// MX3CL1: the client's session end: unloads this thread's client-side mods. Returns what could not be unloaded.
pub fn unload_client_mods() -> Vec<String> {
    DEV_SIDES.with(|sides| sides.borrow_mut()[ThreadSide::Client.index()].in_session = false);
    let errors = unload_previous(ThreadSide::Client);
    for line in &errors {
        log::warn!("yoake: {line}");
        eprintln!("yoake: {line}");
    }
    errors
}

/// Whether this thread has client-side mods loaded (a cheap check for the session hooks).
pub fn client_mods_loaded() -> bool {
    LOADED.with(|loaded| !loaded.borrow()[ThreadSide::Client.index()].is_empty())
}

fn load_mods_on(side: ThreadSide, mods_dir: &Path, approved: Option<&ApprovedList>) -> LoadReport {
    load_mods_scoped(side, mods_dir, approved, None)
}

fn load_mods_scoped(side: ThreadSide, mods_dir: &Path, approved: Option<&ApprovedList>, remote_key: Option<&str>) -> LoadReport {
    let errors = unload_previous(side);
    if !errors.is_empty() {
        let report = LoadReport { problems: errors, ..LoadReport::default() };
        announce(&report);
        return report;
    }
    // MX3HR1: remember which list generation this side runs (the dev reload only acts on a higher one)
    DEV_SIDES.with(|sides| {
        let mut sides = sides.borrow_mut();
        sides[side.index()].in_session = true;
        sides[side.index()].loaded_generation = approved.map_or(0, ApprovedList::generation);
    });
    let (allow, origins) = approved.map(ApprovedList::to_files).unwrap_or_default();
    let trusted = horizon_mods::yak::trusted_keys();
    // before any `yoake_init` runs, so `hook_register` can tell each guest the mode of the ids it asks for
    yoake_slice::bridge::set_mode_resolver(Some(hook_mode));
    let mut report = load_dir_on(side, mods_dir, &origins, &allow, &trusted, &ModConfig::full_access());
    // MX3JN1: the joined host's folder, on the client thread only, with the entries scoped to that exact host
    if let (ThreadSide::Client, Some(key), Some(list)) = (side, remote_key.filter(|key| horizon_mods::yak::control::valid_host_key(key)), approved) {
        if let Some(dir) = horizon_mods::yak::control::host_remote_dir(mods_dir, key) {
            let (remote_allow, remote_origins) = list.to_remote_files(key);
            let remote = load_dir_on(side, &dir, &remote_origins, &remote_allow, &trusted, &ModConfig::full_access());
            merge_remote(&mut report, remote);
        }
    }
    // the "no list" note is only worth a line when there is a full-access package that it affects
    if approved.is_none() && report.skipped.iter().any(|(_, r)| !matches!(r, Refusal::Sandboxed | Refusal::OtherSide)) {
        report.problems.push(format!("the launcher gave no approved list ({APPROVED_ARG} or {APPROVED_ENV}): no full-access mod will run"));
    }
    LOADED.with(|loaded| loaded.borrow_mut()[side.index()].extend(report.loaded.iter().map(|l| (l.module, l.sha256.clone()))));
    if side == ThreadSide::Server {
        *SERVED.lock().unwrap_or_else(|e| e.into_inner()) = report.loaded.iter().map(|l| l.sha256.clone()).collect();
    }
    report.unbridged = install_bridges(&report.loaded);
    announce(&report);
    announce_loaded(side, &report);
    report
}

/// Folds the report of a host's remote folder into the local one: a remote mod with the id of a loaded local mod is
/// unloaded again (the local mod wins), the rest is added.
fn merge_remote(report: &mut LoadReport, remote: LoadReport) {
    for loaded in remote.loaded {
        if report.loaded.iter().any(|local| local.id == loaded.id) {
            let _ = runtime::unload(loaded.module);
            report.problems.push(format!("remote full-access mod '{}' has the id of one of your own mods: your own mod runs, the host's was not loaded", loaded.id));
        } else {
            report.loaded.push(loaded);
        }
    }
    report.skipped.extend(remote.skipped);
    report.problems.extend(remote.problems);
}

// ---------------------------------------------------------------------------------------------------------------
// MX3HR1: dev-mode hot reload (docs/yoake2/ENGINE_CALL_ABI.md, "Dev reload")
// ---------------------------------------------------------------------------------------------------------------

/// Whether this process was started with `--full-access-dev` (read once). Without it the game never re-reads the list.
pub fn dev_mode() -> bool {
    static DEV: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DEV.get_or_init(|| std::env::args().skip(1).any(|a| a == DEV_ARG))
}

/// The newest list the dev reload accepted (any thread), so a world started later loads with it.
static DEV_LATEST: std::sync::Mutex<Option<ApprovedList>> = std::sync::Mutex::new(None);

/// At most one look at the list file per this long, per thread.
pub const DEV_CHECK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);

/// What the dev reload knows about one side of this thread.
struct DevSide {
    /// A loader ran for this side and it was not ended (the client's `unload_client_mods` ends it).
    in_session: bool,
    /// The generation of the list the side's mods were loaded with.
    loaded_generation: u64,
    next_check: Option<std::time::Instant>,
    /// (mtime, size) of the list file when it was last read.
    stamp: Option<(Option<std::time::SystemTime>, u64)>,
}

impl DevSide {
    const fn new() -> Self {
        Self { in_session: false, loaded_generation: 0, next_check: None, stamp: None }
    }
}

thread_local! {
    static DEV_SIDES: std::cell::RefCell<[DevSide; 2]> = const { std::cell::RefCell::new([DevSide::new(), DevSide::new()]) };
}

/// One dev reload that happened.
#[derive(Debug)]
pub struct DevReload {
    pub generation: u64,
    pub report: LoadReport,
}

/// The call the tick paths make: [`dev_reload_check`] for this process (`--full-access-dev`, the list path of this
/// launch, `<game root>/mods`). With no dev flag it is one `OnceLock` read.
pub fn dev_reload_tick(side: ThreadSide) -> Option<DevReload> {
    if !dev_mode() {
        return None;
    }
    let path = ApprovedList::path_from_process()?;
    let root = crate::yak_mods::game_root()?;
    dev_reload_check(true, side, &path, &root.join("mods"), std::time::Instant::now())
}

/// The remote scope a dev reload keeps: the list's host, and only when it is the host of this session.
pub fn dev_remote_key<'a>(list_host: Option<&'a str>, session_key: Option<&str>) -> Option<&'a str> {
    list_host.filter(|host| session_key == Some(*host))
}

/// **Dev reload** for `side` of the calling thread. `enabled` is `--full-access-dev` (false: nothing happens, the file is
/// not even looked at). At most once per [`DEV_CHECK_INTERVAL`] it compares the list file's mtime and size with the last
/// read; a changed file is read and parsed, and when its `generation` is higher than the one this side runs with, and no
/// guest is running ([`runtime::guest_running`]: never mid-activation; it is tried again at the next check), the side's
/// runtime is dropped and loaded again with the new list through the normal loader (same trust rules), `yoake_init`
/// included. A file that cannot be read or parsed is logged and changes nothing. Call it between ticks, outside any hook.
pub fn dev_reload_check(enabled: bool, side: ThreadSide, list_path: &Path, mods_dir: &Path, now: std::time::Instant) -> Option<DevReload> {
    if !enabled {
        return None;
    }
    let (last_stamp, loaded_generation) = DEV_SIDES.with(|sides| {
        let mut sides = sides.borrow_mut();
        let state = &mut sides[side.index()];
        if !state.in_session || state.next_check.is_some_and(|next| now < next) {
            return None;
        }
        state.next_check = Some(now + DEV_CHECK_INTERVAL);
        Some((state.stamp, state.loaded_generation))
    })?;
    let meta = std::fs::metadata(list_path).ok()?;
    let stamp = Some((meta.modified().ok(), meta.len()));
    if stamp == last_stamp {
        return None;
    }
    if runtime::guest_running() {
        // the stamp stays: the change is looked at again at the next check
        let line = format!("yoake: dev reload ({side:?}) postponed: a guest is running (never reload mid-activation)");
        log::warn!("{line}");
        eprintln!("{line}");
        return None;
    }
    let set_stamp = || DEV_SIDES.with(|sides| sides.borrow_mut()[side.index()].stamp = stamp);
    let list = match ApprovedList::read(list_path) {
        Ok(list) => list,
        Err(e) => {
            set_stamp();
            let line = format!("yoake: dev reload ({side:?}): {e}; keeping the loaded mods");
            log::warn!("{line}");
            eprintln!("{line}");
            return None;
        }
    };
    set_stamp();
    if list.generation() <= loaded_generation {
        return None;
    }
    for protected in list.protected() {
        horizon_mods::yak::control::protect(protected);
    }
    {
        let mut latest = DEV_LATEST.lock().unwrap_or_else(|e| e.into_inner());
        if latest.as_ref().is_none_or(|l| l.generation() < list.generation()) {
            *latest = Some(list.clone());
        }
    }
    debug_assert!(!runtime::guest_running());
    // MX3JN1 x MX3HR1: the reload keeps the host-scoped remote mods right: it uses the new list's host (the launcher
    // writes the list of a host launch with `with_host`), but never another host than this launch's `--remote-mods`.
    // S6/M1: a remote scope is kept only when this session really is a join of that host (the process's key equal to the
    // list's host AND the connection going there); a launch without `--remote-mods` never gets one.
    let session_key = horizon_client::join_mods::session_remote_key();
    let remote_key = dev_remote_key(list.host(), session_key.as_deref());
    let report = load_mods_scoped(side, mods_dir, Some(&list), remote_key);
    // S7: an unload that failed halfway left the old mods (the report says so): the stamp goes back, so the next check
    // reads the list again and tries once more
    if DEV_SIDES.with(|sides| sides.borrow()[side.index()].loaded_generation) != list.generation() {
        DEV_SIDES.with(|sides| sides.borrow_mut()[side.index()].stamp = last_stamp);
        let line = format!("yoake: dev reload ({side:?}) generation {} did not run ({}); it is tried again at the next check", list.generation(), report.problems.join("; "));
        log::warn!("{line}");
        eprintln!("{line}");
        return None;
    }
    let refused = report.skipped.iter().filter(|(_, r)| !matches!(r, Refusal::Sandboxed | Refusal::OtherSide)).count();
    let ids: Vec<&str> = report.loaded.iter().map(|l| l.id.as_str()).collect();
    let line = format!("yoake: dev reload ({side:?}) generation {}: {} mod(s) loaded [{}], {refused} refused", list.generation(), ids.len(), ids.join(", "));
    log::info!("{line}");
    eprintln!("{line}");
    Some(DevReload { generation: list.generation(), report })
}

/// Engine-synthetic hooks (ids at and above `SYNTHETIC_HOOK_BASE`) that the engine itself dispatches, with their mode.
/// They have no generated bridge: `hook_mode` answers from this table first and `install_bridges` never lists them
/// as unbridged. MX3CL1: the two client hooks are dispatched on the window's thread (`Game::yoake2_client_tick`, the HUD
/// call in `Game::hud`); a mod that registers one of them on the wrong thread is simply never entered.
pub const ENGINE_DISPATCHED: &[(u32, yoake_slice::bridge::Mode)] = &[
    (runtime::HOOK_SERVER_TICK, yoake_slice::bridge::Mode::Full),
    (runtime::HOOK_CLIENT_TICK, yoake_slice::bridge::Mode::Full),
    (runtime::HOOK_HUD_RENDER, yoake_slice::bridge::Mode::ObserveOnly),
];

/// One generated hook bridge module (`hook_bridge`, yoake-hook-codemod bridge).
pub struct BridgeModule {
    pub name: &'static str,
    /// Its bridgeable ids, sorted.
    pub bridged: &'static [u32],
    pub mode: fn(u32) -> Option<yoake_slice::bridge::Mode>,
    pub install: fn(&[(u32, i32)]) -> yoake_slice::bridge::InstallReport,
}

/// Every bridge module of this engine build. Ledger ids are unique across crates, so at most one knows an id.
pub const BRIDGE_MODULES: [BridgeModule; 4] = [
    BridgeModule {
        name: "entities",
        bridged: crate::entities::hook_bridge::BRIDGED,
        mode: crate::entities::hook_bridge::mode,
        install: crate::entities::hook_bridge::install,
    },
    BridgeModule {
        name: "game::item_use",
        bridged: crate::game::item_use_hook_bridge::BRIDGED,
        mode: crate::game::item_use_hook_bridge::mode,
        install: crate::game::item_use_hook_bridge::install,
    },
    BridgeModule {
        name: "mobs",
        bridged: crate::mobs::hook_bridge::BRIDGED,
        mode: crate::mobs::hook_bridge::mode,
        install: crate::mobs::hook_bridge::install,
    },
    BridgeModule {
        name: "motion",
        bridged: crate::motion::hook_bridge::BRIDGED,
        mode: crate::motion::hook_bridge::mode,
        install: crate::motion::hook_bridge::install,
    },
];

/// The mode of ledger hook `id` in this engine build: the generated bridges' `mode` (ledger ids are unique across
/// crates, so at most one bridge knows `id`). `None`: no bridge can call it. Installed as the process-wide
/// [`yoake_slice::bridge::ModeResolver`], so `hook_register` tells the guest (ENGINE_CALL_ABI.md, "Hook modes").
pub fn hook_mode(id: u32) -> Option<yoake_slice::bridge::Mode> {
    if let Some(&(_, mode)) = ENGINE_DISPATCHED.iter().find(|(hook, _)| *hook == id) {
        return Some(mode);
    }
    BRIDGE_MODULES.iter().find_map(|module| (module.mode)(id))
}

/// After the mods' `yoake_init` registered their hook ids: installs the generated bridge dispatcher
/// (`hook_bridge::install`, docs/yoake2/HOOK-SLICE-BRIDGE.md) of every id any of `loaded` registered. Each id is
/// installed at most once per process (`yoake_slice::bridge::install_once`): a second world, or a second mod on the
/// same id, reuses the dispatcher. Which mods run, and in what order, is the runtime's chain for that id; the
/// dispatcher's own slot order uses the priority of the first mod that registered it. Returns, per mod, the ids no
/// bridge installed, with the reason (not bridgeable, the registration failed, or no bridge has the id).
/// `load_world_mods` calls this; on its own it is for tests that load with [`load_dir`]. Must run on the thread the
/// mods were loaded on.
pub fn install_bridges(loaded: &[Loaded]) -> Vec<(String, Vec<(u32, String)>)> {
    let mut out = Vec::new();
    for l in loaded {
        // id 0 is a legacy single-export guest (no `hook_register`): not a ledger id; engine-dispatched synthetic ids
        // need no bridge
        let registered: Vec<(u32, i32)> = runtime::stats_of(l.module)
            .map(|s| s.registered)
            .unwrap_or_default()
            .into_iter()
            .filter(|(id, _)| *id != 0 && !ENGINE_DISPATCHED.iter().any(|(hook, _)| hook == id))
            .collect();
        let reports: Vec<yoake_slice::bridge::InstallReport> = BRIDGE_MODULES.iter().map(|module| (module.install)(&registered)).collect();
        let missing: Vec<(u32, String)> = registered
            .iter()
            .filter_map(|&(id, _)| {
                if reports.iter().any(|r| r.installed.contains(&id)) {
                    return None;
                }
                let reason = reports
                    .iter()
                    .find_map(|r| r.not_bridgeable.iter().chain(&r.failed).find(|(i, _)| *i == id).map(|(_, why)| (*why).to_string()))
                    .unwrap_or_else(|| "no hook with this id in this engine build".to_string());
                Some((id, reason))
            })
            .collect();
        if !missing.is_empty() {
            out.push((l.id.clone(), missing));
        }
    }
    out
}

/// R2: one line per loaded full-access mod (id, version, side, hooks registered) and one summary per load.
fn announce_loaded(side: ThreadSide, report: &LoadReport) {
    if report.loaded.is_empty() {
        return;
    }
    for loaded in &report.loaded {
        let hooks = runtime::stats_of(loaded.module).map_or(0, |stats| stats.registered.len());
        let line = format!("full-access mod '{}' {} loaded ({side:?} side, {hooks} hook(s) registered)", loaded.id, loaded.version);
        log::info!("yoake: {line}");
        eprintln!("yoake: {line}");
    }
    let refused = report.skipped.iter().filter(|(_, r)| !matches!(r, Refusal::Sandboxed | Refusal::OtherSide)).count();
    let line = format!("{side:?}: {} full-access mod(s) loaded, {refused} refused", report.loaded.len());
    log::info!("yoake: {line}");
    eprintln!("yoake: {line}");
}

fn announce(report: &LoadReport) {
    for line in report.messages() {
        log::warn!("yoake: {line}");
        eprintln!("yoake: {line}");
    }
}

// ---------------------------------------------------------------------------------------------------------------
// MX3CL1: the client-thread hooks
// ---------------------------------------------------------------------------------------------------------------

/// Enters the client-side mods of this thread for [`runtime::HOOK_CLIENT_TICK`] (once per client tick); `root` is the
/// client's `Game`, argument 0 of the hook. With no mod registered for the id this is one thread-local read and the
/// closure is not even built. Must run on the thread the client mods were loaded on ([`load_client_mods`]).
pub fn dispatch_client_tick<G: yoake_slice::activation::PathRoot>(root: &mut G) {
    runtime::dispatch_root_id(runtime::HOOK_CLIENT_TICK, root, &mut |_| 0);
}

/// Enters the client-side mods of this thread for [`runtime::HOOK_HUD_RENDER`] (once per frame, after the vanilla HUD).
/// Observe-only and without handle arguments (argument 0 is a handle to nothing: every field is `BadField`); the guest
/// uses it as a point to call engine functions.
pub fn dispatch_hud_render() {
    static NOTHING: yoake_slice::activation::FieldTable<()> = yoake_slice::activation::FieldTable { get: |_, _| None, set: |_, _, _| false };
    runtime::dispatch_id(runtime::HOOK_HUD_RENDER, &mut (), &NOTHING, &mut |_| 0);
}

#[cfg(test)]
mod tests {
    /// S6: a dev reload keeps a remote scope only for the host of this very session.
    #[test]
    fn a_dev_reload_keeps_the_remote_scope_only_for_the_sessions_host() {
        let h = "server:play.example.com:25565";
        assert_eq!(super::dev_remote_key(Some(h), Some(h)), Some(h));
        assert_eq!(super::dev_remote_key(Some(h), None), None, "single player or a launch without --remote-mods: none");
        assert_eq!(super::dev_remote_key(Some(h), Some("server:other.example.com:25565")), None);
        assert_eq!(super::dev_remote_key(None, Some(h)), None);
    }

    /// `yoake_slice::bridge::install_once` keys by hook id alone: two bridge modules must never claim one id (the
    /// ledger makes ids unique across crates; this checks the generated bridges agree), and no bridge may claim an
    /// engine-dispatched id.
    #[test]
    fn bridge_modules_claim_disjoint_ids() {
        let modules = &super::BRIDGE_MODULES;
        for (i, a) in modules.iter().enumerate() {
            for b in &modules[i + 1..] {
                assert!(a.bridged.iter().all(|id| b.bridged.binary_search(id).is_err()), "an id in both {} and {}", a.name, b.name);
            }
        }
        for (id, mode) in super::ENGINE_DISPATCHED {
            assert!(modules.iter().all(|m| m.bridged.binary_search(id).is_err()));
            assert_eq!(super::hook_mode(*id), Some(*mode));
        }
    }
}
