//! MX3L1/MX3L2: the full-access mod loader (feature `yoake2-slice`).
//!
//! A full-access mod is a `.yak` whose manifest says `[wasm] world = "full-access"` with a `server` module that
//! is a core WebAssembly module (not a component) speaking the `engine_call` ABI (docs/yoake2/ENGINE_CALL_ABI.md).
//! It has the powers of the engine itself, so permission is bound to **content** and the origin is **metadata**:
//!
//! * The player's approval is an entry in `<mods folder>/full-access.toml` holding the mod id, its version and the
//!   **SHA-256 of the package file**. A package whose bytes changed (an update, a replaced file) no longer matches,
//!   is not loaded and is reported as needing re-approval ([`Refusal::ContentChanged`]).
//! * Where a package came from is recorded **when it arrives** in `<mods folder>/yak-origins.toml` ([`OriginFile`],
//!   keyed by the package hash; written by [`install_package`] and by whatever delivers packages), never guessed from
//!   the folder it sits in. A package with no record (or an unreadable one) has an unknown origin and is **never**
//!   full-access ([`Refusal::UnknownOrigin`]). A package that came from a **server or a friend** is never
//!   full-access either, even if its hash is on the allow list (design 2.5). Only `local` (installed by the player)
//!   and `catalog` (with a signer on the trust list, or approved) packages can run.
//! * Two packages with the same id make both refuse ([`Refusal::DuplicateId`]): nobody can slip a second file in
//!   under an approved id.
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
use yoake_slice::runtime::{self, ModConfig, ModId};

/// Where a package came from. Only `Local` and `Catalog` can be full-access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Installed by the player from a file on their own computer.
    Local,
    /// Downloaded from the catalog (its signature is checked).
    Catalog,
    /// Delivered by a server the player joined.
    Server,
    /// Delivered by a friend.
    Friend,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Local => "local",
            Origin::Catalog => "catalog",
            Origin::Server => "server",
            Origin::Friend => "friend",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "local" => Origin::Local,
            "catalog" => Origin::Catalog,
            "server" => Origin::Server,
            "friend" => Origin::Friend,
            _ => return None,
        })
    }

    /// Whether a package of this origin may ever be full-access (still needs approval or a trusted signer).
    pub fn can_be_full_access(self) -> bool {
        matches!(self, Origin::Local | Origin::Catalog)
    }
}

/// Lowercase hex SHA-256 of `bytes` (the identity of a package's content).
pub fn sha256_hex(bytes: &[u8]) -> String {
    <sha2::Sha256 as sha2::Digest>::digest(bytes).iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

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
// The player's approvals: id + version + package hash
// ---------------------------------------------------------------------------------------------------------------

/// One approval: this exact package content may run with full access.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowEntry {
    pub id: String,
    pub version: String,
    /// Lowercase hex SHA-256 of the `.yak` file.
    pub sha256: String,
}

/// The player's approvals, kept in `<mods folder>/full-access.toml`:
///
/// ```toml
/// [[allow]]
/// id = "my_mod"
/// version = "1.0.0"
/// sha256 = "<64 hex digits of the .yak file>"
/// ```
///
/// The MX3L1 form (`[allow]` with `id = true`) named only the id, so anything could run under it. Such lines are
/// read but never allow anything: [`AllowFile::legacy_ids`] lists them so the loader can say the mod needs approving
/// again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowFile {
    entries: Vec<AllowEntry>,
    legacy: Vec<String>,
}

impl AllowFile {
    pub const FILE_NAME: &'static str = "full-access.toml";

    pub fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = text.parse().map_err(|e| format!("{}: {e}", Self::FILE_NAME))?;
        let mut file = Self::default();
        match table.get("allow") {
            None => {}
            Some(toml::Value::Table(old)) => {
                // MX3L1: `id = true`. No content is named, so no content is approved.
                for (id, value) in old {
                    if value.as_bool().ok_or_else(|| format!("allow.{id} must be true or false"))? {
                        file.legacy.push(id.clone());
                    }
                }
            }
            Some(toml::Value::Array(items)) => {
                for (n, item) in items.iter().enumerate() {
                    let item = item.as_table().ok_or_else(|| format!("allow[{n}] must be a table"))?;
                    let text = |key: &str| -> Result<String, String> {
                        item.get(key)
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                            .ok_or_else(|| format!("allow[{n}].{key} is missing (a string)"))
                    };
                    let (id, version, sha256) = (text("id")?, text("version")?, text("sha256")?.to_ascii_lowercase());
                    if id.is_empty() {
                        return Err(format!("allow[{n}].id is empty"));
                    }
                    if !is_sha256_hex(&sha256) {
                        return Err(format!("allow[{n}].sha256 must be 64 hex digits"));
                    }
                    if item.get("enabled").is_some_and(|v| v.as_bool() == Some(false)) {
                        continue;
                    }
                    file.entries.push(AllowEntry { id, version, sha256 });
                }
            }
            Some(_) => return Err("allow must be an array of tables ([[allow]])".into()),
        }
        Ok(file)
    }

    /// The file of `dir`; a missing file allows nothing.
    pub fn load(dir: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(dir.join(Self::FILE_NAME)) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Whether this exact content of `id` was approved.
    pub fn is_allowed(&self, id: &str, sha256: &str) -> bool {
        self.entries.iter().any(|e| e.id == id && e.sha256 == sha256)
    }

    /// The hashes approved for `id` (empty when the id was never approved).
    pub fn approved_hashes(&self, id: &str) -> Vec<String> {
        self.entries.iter().filter(|e| e.id == id).map(|e| e.sha256.clone()).collect()
    }

    /// Ids of MX3L1-style lines, which approve nothing.
    pub fn legacy_ids(&self) -> &[String] {
        &self.legacy
    }

    pub fn entries(&self) -> &[AllowEntry] {
        &self.entries
    }

    /// Approves this content. Any earlier approval of the same id is replaced (an update is approved anew, and
    /// the old content stops matching).
    pub fn allow(&mut self, id: &str, version: &str, sha256: &str) {
        self.entries.retain(|e| e.id != id);
        self.legacy.retain(|l| l != id);
        self.entries.push(AllowEntry { id: id.to_owned(), version: version.to_owned(), sha256: sha256.to_ascii_lowercase() });
    }

    /// Approves the package in `bytes` (reads its id and version from the manifest).
    pub fn allow_package(&mut self, bytes: &[u8]) -> Result<AllowEntry, String> {
        let archive = Archive::new(Cursor::new(bytes.to_vec()))?;
        let manifest = archive.manifest();
        let (id, version) = (manifest.id.clone(), manifest.version.to_string());
        let sha256 = sha256_hex(bytes);
        self.allow(&id, &version, &sha256);
        Ok(AllowEntry { id, version, sha256 })
    }

    /// Withdraws every approval of `id`.
    pub fn revoke(&mut self, id: &str) {
        self.entries.retain(|e| e.id != id);
        self.legacy.retain(|l| l != id);
    }

    pub fn to_text(&self) -> String {
        let mut text = String::from("# Full-access mods the player approved: they run with the engine's own powers.\n# An entry names the exact package file (SHA-256); a changed file needs approving again.\n");
        for e in &self.entries {
            text.push_str(&format!("\n[[allow]]\nid = {:?}\nversion = {:?}\nsha256 = {:?}\n", e.id, e.version, e.sha256));
        }
        text
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::write(dir.join(Self::FILE_NAME), self.to_text()).map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Origin metadata
// ---------------------------------------------------------------------------------------------------------------

/// What was recorded when a package arrived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OriginRecord {
    pub origin: Origin,
    pub id: String,
    pub version: String,
    /// Catalog: who signed it (informational; the loader verifies the signature itself).
    pub signer: Option<String>,
}

/// The arrival records of the packages in a mods folder, kept in `<mods folder>/yak-origins.toml` and keyed by the
/// SHA-256 of the package file:
///
/// ```toml
/// [[package]]
/// sha256 = "<64 hex digits>"
/// id = "my_mod"
/// version = "1.0.0"
/// origin = "local"        # local | catalog | server | friend
/// signer = "..."          # catalog only
/// ```
///
/// Whatever stores a package must record it here: [`install_package`] does, for the player's own installs, the
/// catalog download and the server and friend deliveries (pass the origin). A hash with no record, or with an
/// origin this build does not know, is an unknown origin and never full-access.
///
/// Records are sticky towards distrust: when the same bytes arrive again from a server or a friend the record
/// becomes `server`/`friend`, and a later local or catalog arrival does not lift it (delete the record to start
/// over). A mod file cannot write this file: it is written by the game and the launcher only.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OriginFile {
    records: BTreeMap<String, OriginRecord>,
}

impl OriginFile {
    pub const FILE_NAME: &'static str = "yak-origins.toml";

    pub fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = text.parse().map_err(|e| format!("{}: {e}", Self::FILE_NAME))?;
        let mut records = BTreeMap::new();
        let Some(packages) = table.get("package") else { return Ok(Self { records }) };
        let packages = packages.as_array().ok_or("package must be an array of tables ([[package]])")?;
        for (n, item) in packages.iter().enumerate() {
            let item = item.as_table().ok_or_else(|| format!("package[{n}] must be a table"))?;
            let get = |key: &str| item.get(key).and_then(|v| v.as_str());
            let Some(sha256) = get("sha256").map(str::to_ascii_lowercase).filter(|h| is_sha256_hex(h)) else {
                return Err(format!("package[{n}].sha256 must be 64 hex digits"));
            };
            // an origin this build does not know is as good as none: the entry is dropped
            let Some(origin) = get("origin").and_then(Origin::parse) else { continue };
            records.insert(
                sha256,
                OriginRecord {
                    origin,
                    id: get("id").unwrap_or_default().to_owned(),
                    version: get("version").unwrap_or_default().to_owned(),
                    signer: get("signer").map(str::to_owned),
                },
            );
        }
        Ok(Self { records })
    }

    /// The file of `dir`; a missing file records nothing.
    pub fn load(dir: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(dir.join(Self::FILE_NAME)) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn get(&self, sha256: &str) -> Option<&OriginRecord> {
        self.records.get(sha256)
    }

    pub fn origin_of(&self, sha256: &str) -> Option<Origin> {
        self.get(sha256).map(|r| r.origin)
    }

    /// Records an arrival. Distrust is sticky (see the type docs).
    pub fn record(&mut self, sha256: &str, record: OriginRecord) {
        let sha256 = sha256.to_ascii_lowercase();
        if let Some(existing) = self.records.get(&sha256) {
            if !existing.origin.can_be_full_access() && record.origin.can_be_full_access() {
                return;
            }
        }
        self.records.insert(sha256, record);
    }

    /// Forgets a record (the package becomes unknown-origin until it arrives again).
    pub fn forget(&mut self, sha256: &str) {
        self.records.remove(&sha256.to_ascii_lowercase());
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn to_text(&self) -> String {
        let mut text = String::from("# Where each package in this folder came from, recorded when it arrived (never guessed from the folder).\n# A package with no record here is never a full-access mod.\n");
        for (sha256, r) in &self.records {
            text.push_str(&format!("\n[[package]]\nsha256 = {sha256:?}\nid = {:?}\nversion = {:?}\norigin = {:?}\n", r.id, r.version, r.origin.as_str()));
            if let Some(signer) = &r.signer {
                text.push_str(&format!("signer = {signer:?}\n"));
            }
        }
        text
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::write(dir.join(Self::FILE_NAME), self.to_text()).map_err(|e| e.to_string())
    }
}

/// Stores a `.yak` in the mods folder `dir` as `<file_name>` and records where it came from. This is the one way a
/// package gets into the folder with an origin: the player's own install (`Origin::Local`), a catalog download
/// (`Origin::Catalog`, with the signer's name) and a delivery by a server or a friend (`Origin::Server`,
/// `Origin::Friend`) all go through it. Returns the file's SHA-256.
pub fn install_package(dir: &Path, file_name: &str, bytes: &[u8], origin: Origin, signer: Option<&str>) -> Result<String, String> {
    if file_name.is_empty() || file_name.contains(['/', '\\']) || file_name.starts_with('.') || !file_name.ends_with(".yak") {
        return Err(format!("not a plain .yak file name: {file_name:?}"));
    }
    let archive = Archive::new(Cursor::new(bytes.to_vec()))?;
    let (id, version) = (archive.manifest().id.clone(), archive.manifest().version.to_string());
    let sha256 = sha256_hex(bytes);
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let mut origins = OriginFile::load(dir)?;
    std::fs::write(dir.join(file_name), bytes).map_err(|e| e.to_string())?;
    origins.record(&sha256, OriginRecord { origin, id, version, signer: signer.map(str::to_owned) });
    origins.save(dir)?;
    Ok(sha256)
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

/// World start: replaces this thread's full-access mods with those of `<game root>/mods` (the player's approvals in
/// `full-access.toml` and the arrival records in `yak-origins.toml` there, the compiled-in trust list for
/// signatures, [`ModConfig::full_access`] limits). Mods of a previous world are unloaded first; when one cannot be
/// unloaded nothing new is loaded (two generations of mods on one hook would be worse) and the report says why.
pub fn load_world_mods(mods_dir: &Path) -> LoadReport {
    let errors = unload_previous();
    if !errors.is_empty() {
        let report = LoadReport { problems: errors, ..LoadReport::default() };
        announce(&report);
        return report;
    }
    let mut problems = Vec::new();
    let allow = match AllowFile::load(mods_dir) {
        Ok(allow) => allow,
        Err(e) => {
            problems.push(format!("{e}; no full-access mod will run"));
            AllowFile::default()
        }
    };
    let origins = match OriginFile::load(mods_dir) {
        Ok(origins) => origins,
        Err(e) => {
            problems.push(format!("{e}; no package has a known origin, so no full-access mod will run"));
            OriginFile::default()
        }
    };
    let trusted = horizon_mods::yak::trusted_keys();
    let mut report = load_dir(mods_dir, &origins, &allow, &trusted, &ModConfig::full_access());
    report.problems.extend(problems);
    LOADED.with(|loaded| loaded.borrow_mut().extend(report.loaded.iter().map(|l| l.module)));
    announce(&report);
    report
}

fn announce(report: &LoadReport) {
    for line in report.messages() {
        log::warn!("yoake: {line}");
        eprintln!("yoake: {line}");
    }
}
