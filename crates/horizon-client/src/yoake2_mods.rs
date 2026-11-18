//! MX3L1: the full-access mod loader (feature `yoake2-slice`).
//!
//! A full-access mod is a `.yak` whose manifest says `[wasm] world = "full-access"` with a `server` module that
//! is a core WebAssembly module (not a component) speaking the `engine_call` ABI (docs/yoake2/ENGINE_CALL_ABI.md).
//! It has the powers of the engine itself, so it runs only when
//!
//! * it is a local package in the instance's mods folder and the player switched it on in `full-access.toml`
//!   (a launcher UI comes later; today the file and [`AllowFile`] are the record), or
//! * it is a catalog package signed by a key on the trust list (or switched on as above).
//!
//! A package that came from a **server or a friend is never full-access** (design 2.5): such an origin is refused
//! even with the switch on. A manifest without `world = "full-access"` is not this loader's business: it stays on
//! the sandboxed WebAssembly path ([`Refusal::Sandboxed`] says so).
//!
//! Mods load on the thread the loader runs on (the integrated-server thread at world start): the runtime keeps
//! its mods per thread, each in its own store; for a hook id they run in priority order, each wrapping the next.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use horizon_mods::yak::{Archive, PublicKey};
use yoake_slice::runtime::{self, ModConfig, ModId};

/// Where a package came from. Only the first two can be full-access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A file in the instance's mods folder.
    LocalFolder,
    /// Downloaded from the catalog (its signature is checked).
    Catalog,
    /// Delivered by a server the player joined.
    Server,
    /// Delivered by a friend.
    Friend,
}

/// Why a package was not loaded as a full-access mod.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not a full-access package: leave it to the sandboxed WebAssembly path.
    Sandboxed,
    /// A full-access package nobody allowed.
    NotAllowed,
    FromServer,
    FromFriend,
    /// A catalog package whose signature does not verify against the trust list.
    BadSignature(String),
    /// The package is malformed.
    Package(String),
    /// `yoake_init` failed or the module did not instantiate.
    Init(String),
}

/// The player's per-mod switches, kept in `<mods folder>/full-access.toml`:
///
/// ```toml
/// [allow]
/// my_mod_id = true
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AllowFile {
    allow: BTreeMap<String, bool>,
}

impl AllowFile {
    pub const FILE_NAME: &'static str = "full-access.toml";

    pub fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = text.parse().map_err(|e| format!("{}: {e}", Self::FILE_NAME))?;
        let mut allow = BTreeMap::new();
        if let Some(section) = table.get("allow") {
            let section = section.as_table().ok_or("[allow] must be a table")?;
            for (id, value) in section {
                let on = value.as_bool().ok_or_else(|| format!("allow.{id} must be true or false"))?;
                allow.insert(id.clone(), on);
            }
        }
        Ok(Self { allow })
    }

    /// The file of `dir`; a missing file allows nothing.
    pub fn load(dir: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(dir.join(Self::FILE_NAME)) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn is_allowed(&self, id: &str) -> bool {
        self.allow.get(id).copied().unwrap_or(false)
    }

    pub fn set(&mut self, id: &str, allowed: bool) {
        self.allow.insert(id.to_owned(), allowed);
    }

    pub fn to_text(&self) -> String {
        let mut text = String::from("# Full-access mods the player allowed (they run with the engine's own powers).\n[allow]\n");
        for (id, on) in &self.allow {
            text.push_str(&format!("{id} = {on}\n"));
        }
        text
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::write(dir.join(Self::FILE_NAME), self.to_text()).map_err(|e| e.to_string())
    }
}

/// The policy: may a full-access package of this origin run?
pub fn decide(origin: Origin, switched_on: bool, signed_by_trusted: bool) -> Result<(), Refusal> {
    match origin {
        Origin::Server => Err(Refusal::FromServer),
        Origin::Friend => Err(Refusal::FromFriend),
        Origin::LocalFolder if switched_on => Ok(()),
        Origin::Catalog if switched_on || signed_by_trusted => Ok(()),
        Origin::LocalFolder | Origin::Catalog => Err(Refusal::NotAllowed),
    }
}

/// One mod that was loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    /// The manifest's id.
    pub id: String,
    pub module: ModId,
}

/// Loads one package from `bytes` if it is a full-access mod and the policy lets it run. Must run on the thread
/// whose hooks it should join.
pub fn load_package(bytes: Vec<u8>, origin: Origin, allow: &AllowFile, trusted: &[PublicKey], cfg: ModConfig) -> Result<Loaded, Refusal> {
    let mut archive = Archive::new(Cursor::new(bytes)).map_err(Refusal::Package)?;
    let id = archive.manifest().id.clone();
    if !archive.manifest().wasm.as_ref().is_some_and(|wasm| wasm.full_access()) {
        return Err(Refusal::Sandboxed);
    }
    // a server's or a friend's package is refused before anything else looks at it
    match origin {
        Origin::Server => return Err(Refusal::FromServer),
        Origin::Friend => return Err(Refusal::FromFriend),
        Origin::LocalFolder | Origin::Catalog => {}
    }
    let signed = match archive.verify_signature(trusted) {
        Ok(signer) => signer.is_some(),
        // a signature that does not verify only matters for a catalog package: a local file is the player's own
        Err(e) if origin == Origin::Catalog => return Err(Refusal::BadSignature(e)),
        Err(_) => false,
    };
    decide(origin, allow.is_allowed(&id), signed)?;
    let module = archive.full_access_module().map_err(Refusal::Package)?.ok_or_else(|| Refusal::Package("no full-access module".into()))?;
    let module = runtime::load(&module, cfg, &id).map_err(|e| Refusal::Init(format!("{e:#}")))?;
    Ok(Loaded { id, module })
}

/// What a folder scan did.
#[derive(Debug, Default)]
pub struct LoadReport {
    pub loaded: Vec<Loaded>,
    pub skipped: Vec<(PathBuf, Refusal)>,
}

/// Loads every `*.yak` of a mods folder (sorted by file name, so the load order is stable) that is an allowed
/// full-access mod. Packages that are not full-access are listed as skipped with [`Refusal::Sandboxed`].
pub fn load_dir(dir: &Path, origin: Origin, allow: &AllowFile, trusted: &[PublicKey], cfg: &ModConfig) -> LoadReport {
    let mut report = LoadReport::default();
    let Ok(entries) = std::fs::read_dir(dir) else { return report };
    let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "yak")).collect();
    files.sort();
    for path in files {
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) => {
                report.skipped.push((path, Refusal::Package(e.to_string())));
                continue;
            }
        };
        match load_package(bytes, origin, allow, trusted, cfg.clone()) {
            Ok(loaded) => report.loaded.push(loaded),
            Err(refusal) => report.skipped.push((path, refusal)),
        }
    }
    report
}

thread_local! {
    static LOADED: std::cell::RefCell<Vec<ModId>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// World start: replaces this thread's full-access mods with those of `<game root>/mods` (the player's switches
/// in `full-access.toml` there, the compiled-in trust list for signatures). Mods of a previous world are unloaded.
pub fn load_world_mods(mods_dir: &Path) -> LoadReport {
    LOADED.with(|loaded| {
        for id in loaded.borrow_mut().drain(..) {
            let _ = runtime::unload(id);
        }
    });
    let allow = match AllowFile::load(mods_dir) {
        Ok(allow) => allow,
        Err(e) => {
            eprintln!("yoake: {e}; no full-access mod will run");
            AllowFile::default()
        }
    };
    let trusted = horizon_mods::yak::trusted_keys();
    let report = load_dir(mods_dir, Origin::LocalFolder, &allow, &trusted, &ModConfig::new(""));
    LOADED.with(|loaded| loaded.borrow_mut().extend(report.loaded.iter().map(|l| l.module)));
    for (path, refusal) in &report.skipped {
        if *refusal != Refusal::Sandboxed {
            eprintln!("yoake: full-access mod {} not loaded: {refusal:?}", path.display());
        }
    }
    report
}
