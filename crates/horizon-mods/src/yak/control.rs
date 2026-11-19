//! MX3L3/MX3L4: the launcher is the authority over full-access mods; the game only receives approved hashes.
//!
//! Two files decide what may run with the engine's own powers, and both belong to the **launcher**:
//!
//! * `full-access.toml` ([`AllowFile`]): the player's approvals (id + version + SHA-256 of the `.yak` file);
//! * `yak-origins.toml` ([`OriginFile`]): where each package came from, recorded when it arrived.
//!
//! They live in the launcher-owned `yoake-full-access/` folder of the launcher's data folder ([`control_dir_in`]),
//! outside the game root. **The game never opens that folder.** At each launch the launcher reads and checks the files
//! ([`Control::approved_list`]), builds the list of packages approved for *this launch* as
//! `(id, version, sha256, origin)` entries ([`ApprovedList`]), writes it to a fresh file ([`ApprovedList::write_for_launch`])
//! and hands the game its path with [`APPROVED_ARG`] (or [`APPROVED_ENV`]). The game loads only a package whose bytes hash
//! to an entry of that list with origin `local` or `catalog` (catalog signatures are still verified). Without the list
//! nothing is full-access. A full-access mod that rewrites the control files therefore changes nothing in the running
//! game; it shows up at the *next* launch, where the launcher compares the new list with the last one the player accepted
//! ([`ApprovedList::diff`]) and shows the difference before anything is approved.
//!
//! **What the launcher must pass** (format, path, lifetime) is documented on [`ApprovedList`].
//!
//! **The checksum file is a corruption check, not tamper protection.** Each save is atomic (temp file, fsync, rename) and
//! then recorded in `<control folder>.checksum` (a sibling of the folder), holding the SHA-256 of both files. The
//! launcher refuses a file that does not match (half-written, damaged, edited by hand, or a checksum missing for an
//! existing file) and builds no list from it. Anything that can write the control files can rewrite the checksum too, so
//! this does not stop an attacker; what stops a running mod is that the game never reads the folder and that the folder is
//! unreachable by storage roots and file bindings ([`guard`]). [`Control::accept_current_files`] (the old `reseal`) rewrites
//! the checksum from whatever is on disk: **the launcher calls it only after the player confirmed a diff it showed**.
//! A keyed MAC is not a next step worth taking while the game cannot read the folder.
//!
//! **Packages from servers and friends** never enter the folder of approved mods: [`install_package`] puts them in
//! `<mods>/remote/<origin>/<sha256>.yak` (named by their hash; not scanned by the loader, so they take no part in
//! duplicate-id detection), never overwrites a file with other content, and never touches the control files.
//!
//! **The launcher's entry points** are [`Control::install_local`] (a `.yak` the player chose: copy it into the mods
//! folder and record origin `local`) and [`Control::approve`] (the player's approval of that exact file). A `.yak` a
//! player drops into the mods folder by hand has no origin record and stays not-full-access until the launcher records it.
//!
//! **Protected paths.** [`protect`] / [`guard`] keep every storage root and file binding of the game away from the
//! control folder and the approved-list file ([`guard`] refuses a path inside a protected path, an ancestor of one, and
//! any path with a `..` component).

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use super::Archive;

/// Command-line option that gives the game this launch's approved list: `--full-access-approved <path>`.
pub const APPROVED_ARG: &str = "--full-access-approved";
/// MX3HR1: with this flag (dev mode only) the game re-reads the approved list and reloads when its `generation` grows.
pub const DEV_ARG: &str = "--full-access-dev";
/// Environment variable with the same meaning (the option wins).
pub const APPROVED_ENV: &str = "YOAKE_FULL_ACCESS_APPROVED";
/// The subfolder of the launcher's data folder.
pub const CONTROL_SUBDIR: &str = "yoake-full-access";

/// Where the launcher keeps the control folder, given its own data folder (`backend::data_dir()`).
pub fn control_dir_in(launcher_data_dir: &Path) -> PathBuf {
    launcher_data_dir.join(CONTROL_SUBDIR)
}

/// Lowercase hex SHA-256 of `bytes` (the identity of a package's content).
pub fn sha256_hex(bytes: &[u8]) -> String {
    horizon_release_sign::sha256_hex(bytes)
}

fn is_sha256_hex(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Where a package came from. Only `Local`, `Catalog` and `Dev` can be full-access.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Installed by the player from a file on their own computer.
    Local,
    /// Downloaded from the catalog (its signature is checked).
    Catalog,
    /// Copied by the launcher from the player's own developer folder while a game it started runs (`dev`, shown as
    /// "dev folder"); the launcher approves exactly those bytes itself.
    Dev,
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
            Origin::Dev => "dev",
            Origin::Server => "server",
            Origin::Friend => "friend",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Some(match text {
            "local" => Origin::Local,
            "catalog" => Origin::Catalog,
            "dev" => Origin::Dev,
            "server" => Origin::Server,
            "friend" => Origin::Friend,
            _ => return None,
        })
    }

    /// Whether a package of this origin may ever be full-access (still needs approval or a trusted signer).
    pub fn can_be_full_access(self) -> bool {
        matches!(self, Origin::Local | Origin::Catalog | Origin::Dev)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Protected paths
// ---------------------------------------------------------------------------------------------------------------

static PROTECTED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Whether `path` has a `..` component.
pub fn has_parent_dir(path: &Path) -> bool {
    // a verbatim path (`\?\C:\...`, what `canonicalize` gives on Windows) does not parse `..` as a parent component
    // but as an ordinary name, so look at the names too
    path.components().any(|c| matches!(c, Component::ParentDir) || matches!(c, Component::Normal(name) if name == ".."))
}

/// A path with `.` resolved and symlinks followed as far as the path exists: the deepest existing ancestor is
/// canonicalized and the rest (which may not exist yet) is appended. **Callers refuse `..` before calling this**
/// ([`guard`], [`Control::open`]): a `..` is only resolved lexically here, which is wrong when a link comes before it.
pub fn canonical_lenient(path: &Path) -> PathBuf {
    let mut absolute = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    // apply `..` and `.` lexically first, so that a missing directory cannot hide a traversal
    let mut parts: Vec<Component<'_>> = Vec::new();
    let owned = std::mem::take(&mut absolute);
    for component in owned.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(parts.last(), Some(Component::Normal(_))) {
                    parts.pop();
                }
            }
            other => parts.push(other),
        }
    }
    let lexical: PathBuf = parts.iter().collect();
    // then resolve links through the longest existing prefix
    let mut tail = Vec::new();
    let mut head = lexical.as_path();
    loop {
        if let Ok(real) = head.canonicalize() {
            let mut out = real;
            out.extend(tail.iter().rev());
            return out;
        }
        match (head.parent(), head.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_owned());
                head = parent;
            }
            _ => return lexical,
        }
    }
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

/// Registers `path` (a folder, or a single file) as protected for this process. Idempotent.
pub fn protect(path: &Path) {
    let canonical = canonical_lenient(path);
    let mut list = PROTECTED.lock().unwrap_or_else(|e| e.into_inner());
    if !list.contains(&canonical) {
        list.push(canonical);
    }
}

/// Whether `path` is, is inside, or is an **ancestor of** a protected path (symlinks resolved), or has a `..` component.
pub fn is_protected(path: &Path) -> bool {
    if has_parent_dir(path) {
        return true;
    }
    let canonical = canonical_lenient(path);
    PROTECTED.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|p| is_inside(&canonical, p) || is_inside(p, &canonical))
}

/// `Err` when `path` is inside a protected path, **an ancestor of one** (a storage root that contains the control
/// folder would reach it), or contains a `..` component (refused outright, never resolved). **Every storage root and
/// every file a binding is about to open calls this** (`yoake-wasm`'s `DiskStorage` does, for its root and each path it
/// opens).
pub fn guard(path: &Path) -> Result<(), String> {
    if has_parent_dir(path) {
        return Err(format!("{} has a `..` component: refused", path.display()));
    }
    if is_protected(path) {
        Err(format!("{} is, is inside, or contains a protected folder (the full-access control files)", path.display()))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Atomic writes
// ---------------------------------------------------------------------------------------------------------------

/// Writes `bytes` to a temporary file next to `path`, fsyncs it (best effort), then renames it over `path`: a reader sees
/// the old file or the new one, never half of it.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    let temp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    {
        use std::io::Write;
        let mut file = std::fs::File::create(&temp).map_err(|e| format!("{}: {e}", temp.display()))?;
        file.write_all(bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
        // best effort: the data is on disk before the rename makes it the file
        let _ = file.sync_all();
    }
    std::fs::rename(&temp, path).map_err(|e| {
        let _ = std::fs::remove_file(&temp);
        format!("{}: {e}", path.display())
    })
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

/// The player's approvals (`full-access.toml` in the control folder):
///
/// ```toml
/// [[allow]]
/// id = "my_mod"
/// version = "1.0.0"
/// sha256 = "<64 hex digits of the .yak file>"
/// ```
///
/// The MX3L1 form (`[allow]` with `id = true`) named only the id: such lines are read but never allow anything
/// ([`AllowFile::legacy_ids`]).
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

    /// Approves this content. Any earlier approval of the same id is replaced.
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

/// The arrival records of the packages in the mods folder (`yak-origins.toml` in the control folder), keyed by the
/// SHA-256 of the package file:
///
/// ```toml
/// [[package]]
/// sha256 = "<64 hex digits>"
/// id = "my_mod"
/// version = "1.0.0"
/// origin = "local"        # local | catalog (server and friend packages are not recorded here)
/// signer = "..."          # catalog only
/// ```
///
/// A hash with no record, or with an origin this build does not know, is an unknown origin and never full-access.
/// Packages that arrive from a server or a friend live in `<mods>/remote/<origin>/` and are **not** recorded here, so
/// they cannot change a record of a local or catalog package. A record of `server`/`friend` (older files) stays
/// sticky towards distrust.
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

    pub fn get(&self, sha256: &str) -> Option<&OriginRecord> {
        self.records.get(sha256)
    }

    pub fn origin_of(&self, sha256: &str) -> Option<Origin> {
        self.get(sha256).map(|r| r.origin)
    }

    /// Records an arrival. Distrust is sticky: a `server`/`friend` record is not lifted by a later local record.
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
        let mut text = String::from("# Where each package in the mods folder came from, recorded when it arrived (never guessed from the folder).\n# A package with no record here is never a full-access mod.\n");
        for (sha256, r) in &self.records {
            text.push_str(&format!("\n[[package]]\nsha256 = {sha256:?}\nid = {:?}\nversion = {:?}\norigin = {:?}\n", r.id, r.version, r.origin.as_str()));
            if let Some(signer) = &r.signer {
                text.push_str(&format!("signer = {signer:?}\n"));
            }
        }
        text
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The control folder (launcher side)
// ---------------------------------------------------------------------------------------------------------------

/// The launcher-owned folder holding the two control files and their checksum file. **Launcher code only**: the game
/// never opens it.
#[derive(Clone, Debug)]
pub struct Control {
    dir: PathBuf,
}

impl Control {
    pub const CHECK_KEY_ALLOW: &'static str = "allow";
    pub const CHECK_KEY_ORIGINS: &'static str = "origins";

    /// Opens the control folder `dir` for a game whose root is `game_root`. Refuses a relative path, any path with a `..`
    /// component, a folder inside the game root (or one that contains it). The folder is not created (see
    /// [`Control::create`]). Registers the folder and its checksum file as protected in this process (a launcher
    /// process does not run mods; this is for tests and for tools that embed both).
    pub fn open(dir: &Path, game_root: &Path) -> Result<Self, String> {
        if !dir.is_absolute() {
            return Err(format!("the full-access control folder must be an absolute path, got {}", dir.display()));
        }
        if has_parent_dir(dir) || has_parent_dir(game_root) {
            return Err(format!("the full-access control folder and the game folder must not contain `..` ({}, {})", dir.display(), game_root.display()));
        }
        let (real_dir, real_root) = (canonical_lenient(dir), canonical_lenient(game_root));
        if is_inside(&real_dir, &real_root) || is_inside(&real_root, &real_dir) {
            return Err(format!(
                "the full-access control folder {} is inside the game folder {}: it must be launcher-owned, outside it",
                dir.display(),
                game_root.display()
            ));
        }
        let control = Self { dir: real_dir };
        protect(&control.dir);
        protect(&control.checksum_path());
        Ok(control)
    }

    /// [`Control::open`], creating the folder.
    pub fn create(dir: &Path, game_root: &Path) -> Result<Self, String> {
        let control = Self::open(dir, game_root)?;
        std::fs::create_dir_all(&control.dir).map_err(|e| format!("{}: {e}", control.dir.display()))?;
        Ok(control)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `<control folder>.checksum`: next to the folder, in the launcher's data folder.
    pub fn checksum_path(&self) -> PathBuf {
        let mut name = self.dir.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".checksum");
        self.dir.with_file_name(name)
    }

    fn checksums(&self) -> Result<BTreeMap<String, String>, String> {
        let text = match std::fs::read_to_string(self.checksum_path()) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(format!("{}: {e}", self.checksum_path().display())),
        };
        let table: toml::Table = text.parse().map_err(|e| format!("checksum file: {e}"))?;
        Ok(table.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned()))).collect())
    }

    fn read_checked(&self, file_name: &str, key: &str) -> Result<Option<String>, String> {
        let bytes = match std::fs::read(self.dir.join(file_name)) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{file_name}: {e}")),
        };
        let recorded = self.checksums()?.get(key).cloned();
        match (bytes, recorded) {
            (None, None) => Ok(None),
            (Some(bytes), Some(hash)) if sha256_hex(&bytes) == hash => String::from_utf8(bytes).map(Some).map_err(|e| format!("{file_name}: {e}")),
            (Some(_), Some(_)) => Err(format!("{file_name} does not match the checksum the launcher recorded (damaged or changed): refused until the player accepts it")),
            (Some(_), None) => Err(format!("{file_name} has no recorded checksum (the launcher did not write it): refused until the player accepts it")),
            (None, Some(_)) => Err(format!("{file_name} is missing although the launcher recorded it: refused")),
        }
    }

    /// The approvals; an empty file when none was ever saved. `Err` when the file fails the corruption check or does not parse.
    pub fn allow(&self) -> Result<AllowFile, String> {
        match self.read_checked(AllowFile::FILE_NAME, Self::CHECK_KEY_ALLOW)? {
            Some(text) => AllowFile::parse(&text),
            None => Ok(AllowFile::default()),
        }
    }

    /// The arrival records; see [`Control::allow`].
    pub fn origins(&self) -> Result<OriginFile, String> {
        match self.read_checked(OriginFile::FILE_NAME, Self::CHECK_KEY_ORIGINS)? {
            Some(text) => OriginFile::parse(&text),
            None => Ok(OriginFile::default()),
        }
    }

    fn write_checksum(&self, key: &str, hash: &str) -> Result<(), String> {
        let mut sums = self.checksums().unwrap_or_default();
        sums.insert(key.to_owned(), hash.to_owned());
        let mut text = String::from("# SHA-256 of the full-access control files as the launcher last wrote them (a corruption check, not tamper protection). Do not edit.\n");
        for (k, v) in &sums {
            text.push_str(&format!("{k} = {v:?}\n"));
        }
        write_atomic(&self.checksum_path(), text.as_bytes())
    }

    fn save_text(&self, file_name: &str, key: &str, text: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        write_atomic(&self.dir.join(file_name), text.as_bytes())?;
        self.write_checksum(key, &sha256_hex(text.as_bytes()))
    }

    /// Saves the approvals: atomic, then recorded in the checksum file.
    pub fn save_allow(&self, allow: &AllowFile) -> Result<(), String> {
        self.save_text(AllowFile::FILE_NAME, Self::CHECK_KEY_ALLOW, &allow.to_text())
    }

    /// Saves the arrival records: atomic, then recorded in the checksum file.
    pub fn save_origins(&self, origins: &OriginFile) -> Result<(), String> {
        self.save_text(OriginFile::FILE_NAME, Self::CHECK_KEY_ORIGINS, &origins.to_text())
    }

    /// Records the checksums of the files **as they are on disk now**. Only for the launcher, **after the player confirmed
    /// a diff it showed** (the files changed behind the launcher's back, for example by a mod of the last session, and
    /// the player looked at what changed and accepted it). Never call it to make an error go away.
    pub fn accept_current_files(&self) -> Result<(), String> {
        for (name, key) in [(AllowFile::FILE_NAME, Self::CHECK_KEY_ALLOW), (OriginFile::FILE_NAME, Self::CHECK_KEY_ORIGINS)] {
            if let Ok(bytes) = std::fs::read(self.dir.join(name)) {
                self.write_checksum(key, &sha256_hex(&bytes))?;
            }
        }
        Ok(())
    }

    /// **The launcher's entry point for "install a local .yak"**: copies the package the player chose into the mods
    /// folder `mods_dir` (never over an existing file) and records origin `local`. Returns the package's SHA-256.
    /// Approval is a separate act ([`Control::approve`]).
    pub fn install_local(&self, mods_dir: &Path, source: &Path) -> Result<String, String> {
        let name = source.file_name().and_then(|n| n.to_str()).ok_or("the source has no file name")?;
        let bytes = std::fs::read(source).map_err(|e| format!("{}: {e}", source.display()))?;
        install_package(self, mods_dir, name, &bytes, Origin::Local, None)
    }

    /// Records the player's approval of exactly this package file.
    pub fn approve(&self, bytes: &[u8]) -> Result<AllowEntry, String> {
        let mut allow = self.allow()?;
        let entry = allow.allow_package(bytes)?;
        self.save_allow(&allow)?;
        Ok(entry)
    }

    /// **Builds this launch's approved list** from the verified control files and the packages in `mods_dir`: every
    /// top-level `*.yak` whose SHA-256 is approved under its id **and** has an origin record `local` or `catalog`.
    /// `Err` when a control file fails the corruption check or does not parse (the launcher then shows the problem and
    /// passes no list, so nothing is full-access). The list also names the folders the game must keep its storage and
    /// bindings away from (the control folder and its checksum file).
    pub fn approved_list(&self, mods_dir: &Path) -> Result<ApprovedList, String> {
        let (allow, origins) = (self.allow()?, self.origins()?);
        let mut files: Vec<PathBuf> = match std::fs::read_dir(mods_dir) {
            Ok(entries) => entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "yak") && p.is_file()).collect(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(format!("{}: {e}", mods_dir.display())),
        };
        files.sort();
        let mut list = ApprovedList { entries: Vec::new(), protected: vec![self.dir.clone(), self.checksum_path()], generation: 0, host: None };
        for path in files {
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let sha256 = sha256_hex(&bytes);
            let Some(origin) = origins.origin_of(&sha256).filter(|o| o.can_be_full_access()) else { continue };
            let Some(entry) = allow.entries().iter().find(|e| e.sha256 == sha256) else { continue };
            list.entries.push(ApprovedEntry { id: entry.id.clone(), version: entry.version.clone(), sha256, origin });
        }
        Ok(list)
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The approved list: what the launcher hands the game, for one launch
// ---------------------------------------------------------------------------------------------------------------

/// One package approved for this launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovedEntry {
    pub id: String,
    pub version: String,
    /// Lowercase hex SHA-256 of the `.yak` file.
    pub sha256: String,
    /// `Local` or `Catalog` (a catalog package still has its signature verified by the game).
    pub origin: Origin,
}

/// **The one input the game takes about full-access mods.** The launcher builds it ([`Control::approved_list`]) and the
/// game reads it once, at its first world start, then never again (a mod that rewrites the file changes nothing).
///
/// **Format** (TOML, UTF-8, at most 1 MiB):
///
/// ```toml
/// format = 1
/// protected = ["C:\\Users\\me\\AppData\\Roaming\\horizon-launcher\\yoake-full-access", "...\\yoake-full-access.checksum"]
///
/// [[package]]
/// id = "my_mod"
/// version = "1.0.0"
/// sha256 = "<64 lowercase hex digits of the .yak file>"
/// origin = "local"          # or "catalog"; anything else makes the whole file invalid
/// ```
///
/// An optional top-level `generation = <integer>` (absent = 0) is raised by the launcher on every rewrite in dev mode
/// (MX3HR1); a game started with `--full-access-dev` reloads its full-access mods when it grows. Written only when non-zero.
///
/// **How to pass it.** Write it with [`ApprovedList::write_for_launch`] into a fresh, launch-specific file *outside the
/// game folder* (for example `<launcher data>/run/<launch id>/approved.toml`), and pass its absolute path on the game's
/// command line as `--full-access-approved <path>` (or `--full-access-approved=<path>`); the environment variable
/// `YOAKE_FULL_ACCESS_APPROVED` means the same and the option wins. A command-line path is the simplest robust way on
/// Windows: no inherited handle to keep alive across `CreateProcess`, and the launcher already builds the game's
/// command line. **No option and no variable, an unreadable or invalid file: nothing is full-access.**
///
/// **Lifetime.** One launch. The file must exist and stay unchanged from the game's start until its first world start
/// (the game reads it once and keeps it in memory); the launcher deletes it (and its fresh folder) when the game process
/// exits, and writes a new one for the next launch. The game registers the file and every `protected` path as protected
/// from its storage roots and file bindings, and never opens the control folder itself.
///
/// **What the game does with it.** It loads a package only if the SHA-256 of the file's bytes equals an entry's `sha256`
/// (and the manifest id equals the entry's `id`), with the entry's origin. Everything else (changed bytes, a second file
/// with the same id, a server or friend package, a hand-dropped file) is not full-access.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ApprovedList {
    entries: Vec<ApprovedEntry>,
    protected: Vec<PathBuf>,
    /// MX3HR1: dev-mode hot reload. The launcher raises it on every rewrite of the file; absent = 0. Only a game started
    /// with `--full-access-dev` looks at it (it reloads when it grows); otherwise it is ignored.
    generation: u64,
    /// MX3JN1: the host this launch joins (`server:<host>:<port>` / `friend:<portal user id>`). Entries with origin
    /// `server` or `friend` are valid only in a list that names their host; the game loads them only from that
    /// host's folder and only when its own `--remote-mods` names the same host. Absent = no remote mods.
    host: Option<String>,
}

/// What changed between two lists (the launcher shows it to the player before approving a new launch).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListDiff {
    pub added: Vec<ApprovedEntry>,
    pub removed: Vec<ApprovedEntry>,
    /// Same id, other version or bytes: `(before, after)`.
    pub changed: Vec<(ApprovedEntry, ApprovedEntry)>,
}

impl ListDiff {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.changed.is_empty()
    }
}

impl ApprovedList {
    pub const FORMAT: i64 = 1;
    pub const MAX_BYTES: u64 = 1 << 20;

    pub fn new(entries: Vec<ApprovedEntry>, protected: Vec<PathBuf>) -> Self {
        Self { entries, protected, generation: 0, host: None }
    }

    /// The host scope of this list (MX3JN1); see the `host` field.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// Scopes the list to the host the launch joins. Entries with origin `server`/`friend` need it.
    pub fn with_host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// The rewrite counter (0 when the file has none). See the `generation` field.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    pub fn entries(&self) -> &[ApprovedEntry] {
        &self.entries
    }

    pub fn protected(&self) -> &[PathBuf] {
        &self.protected
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn to_text(&self) -> String {
        let mut text = format!("# This launch's approved full-access packages (written by the launcher; the game reads it once).\nformat = {}\n", Self::FORMAT);
        let paths: Vec<String> = self.protected.iter().map(|p| format!("{:?}", p.to_string_lossy())).collect();
        text.push_str(&format!("protected = [{}]\n", paths.join(", ")));
        if let Some(host) = &self.host {
            text.push_str(&format!("host = {host:?}\n"));
        }
        if self.generation != 0 {
            text.push_str(&format!("generation = {}\n", self.generation));
        }
        for e in &self.entries {
            text.push_str(&format!("\n[[package]]\nid = {:?}\nversion = {:?}\nsha256 = {:?}\norigin = {:?}\n", e.id, e.version, e.sha256, e.origin.as_str()));
        }
        text
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let table: toml::Table = text.parse().map_err(|e| format!("approved list: {e}"))?;
        if table.get("format").and_then(|v| v.as_integer()) != Some(Self::FORMAT) {
            return Err(format!("approved list: format must be {}", Self::FORMAT));
        }
        let mut list = Self::default();
        if let Some(generation) = table.get("generation") {
            list.generation = generation.as_integer().and_then(|g| u64::try_from(g).ok()).ok_or("approved list: generation must be a non-negative integer")?;
        }
        if let Some(protected) = table.get("protected") {
            for item in protected.as_array().ok_or("approved list: protected must be an array of paths")? {
                list.protected.push(PathBuf::from(item.as_str().ok_or("approved list: protected holds strings")?));
            }
        }
        if let Some(host) = table.get("host") {
            let host = host.as_str().filter(|h| valid_host_key(h)).ok_or("approved list: host must be a host key (server:<host>:<port> or friend:<id>)")?;
            list.host = Some(host.to_owned());
        }
        if let Some(packages) = table.get("package") {
            for (n, item) in packages.as_array().ok_or("approved list: package must be an array of tables")?.iter().enumerate() {
                let item = item.as_table().ok_or_else(|| format!("approved list: package[{n}] must be a table"))?;
                let get = |key: &str| item.get(key).and_then(|v| v.as_str()).ok_or_else(|| format!("approved list: package[{n}].{key} is missing"));
                let sha256 = get("sha256")?.to_ascii_lowercase();
                if !is_sha256_hex(&sha256) {
                    return Err(format!("approved list: package[{n}].sha256 must be 64 hex digits"));
                }
                let origin = Origin::parse(get("origin")?).ok_or_else(|| format!("approved list: package[{n}].origin must be local, catalog, dev, server or friend"))?;
                // MX3JN1: a server's or friend's package is valid only in a list scoped to its host, and the host's kind matches.
                if !origin.can_be_full_access() {
                    let scoped = list.host.as_deref().is_some_and(|host| host.starts_with(&format!("{}:", origin.as_str())));
                    if !scoped {
                        return Err(format!("approved list: package[{n}].origin {} needs a matching top-level host", origin.as_str()));
                    }
                }
                list.entries.push(ApprovedEntry { id: get("id")?.to_owned(), version: get("version")?.to_owned(), sha256, origin });
            }
        }
        Ok(list)
    }

    /// Reads a list from `path` (at most [`Self::MAX_BYTES`]).
    pub fn read(path: &Path) -> Result<Self, String> {
        use std::io::Read;
        let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut text = String::new();
        file.take(Self::MAX_BYTES + 1).read_to_string(&mut text).map_err(|e| format!("{}: {e}", path.display()))?;
        if text.len() as u64 > Self::MAX_BYTES {
            return Err("approved list: larger than 1 MiB".into());
        }
        Self::parse(&text)
    }

    /// The path the game was given: `--full-access-approved <path>` (or `=<path>`), else [`APPROVED_ENV`].
    pub fn path_from_process() -> Option<PathBuf> {
        Self::path_from(std::env::args().skip(1), std::env::var_os(APPROVED_ENV))
    }

    pub fn path_from(args: impl IntoIterator<Item = String>, env: Option<std::ffi::OsString>) -> Option<PathBuf> {
        let args: Vec<String> = args.into_iter().collect();
        for (i, arg) in args.iter().enumerate() {
            if arg == APPROVED_ARG {
                return args.get(i + 1).filter(|v| !v.is_empty()).map(PathBuf::from);
            }
            if let Some(value) = arg.strip_prefix(&format!("{APPROVED_ARG}=")) {
                return (!value.is_empty()).then(|| PathBuf::from(value));
            }
        }
        env.filter(|v| !v.is_empty()).map(PathBuf::from)
    }

    /// Launcher side: writes the list into `run_dir` (a fresh folder for this launch, outside the game folder, created
    /// if needed) as `approved.toml` and returns its absolute path. Refuses to overwrite: a launch gets a new folder.
    pub fn write_for_launch(&self, run_dir: &Path) -> Result<PathBuf, String> {
        use std::io::Write;
        std::fs::create_dir_all(run_dir).map_err(|e| format!("{}: {e}", run_dir.display()))?;
        let path = run_dir.join("approved.toml");
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        file.write_all(self.to_text().as_bytes()).map_err(|e| e.to_string())?;
        let _ = file.sync_all();
        Ok(path)
    }

    /// The two files the loader (`load_dir`) works with: approvals by hash, and the origin of each hash. Game side.
    /// Only the player's own (`local`) and catalog packages: what the plain `mods/` folder may run. Entries of a host
    /// scope are never in here ([`ApprovedList::to_remote_files`]).
    pub fn to_files(&self) -> (AllowFile, OriginFile) {
        self.files_where(|e| e.origin.can_be_full_access())
    }

    /// MX3JN1: the pair for the folder of the host this launch joins (`mods/remote/<host dir>/`): the `server`/`friend`
    /// entries, and only when this list names exactly `host_key` as its scope. Empty for any other key.
    pub fn to_remote_files(&self, host_key: &str) -> (AllowFile, OriginFile) {
        if self.host.as_deref() != Some(host_key) {
            return (AllowFile::default(), OriginFile::default());
        }
        self.files_where(|e| !e.origin.can_be_full_access())
    }

    fn files_where(&self, keep: impl Fn(&ApprovedEntry) -> bool) -> (AllowFile, OriginFile) {
        let (mut allow, mut origins) = (AllowFile::default(), OriginFile::default());
        for e in self.entries.iter().filter(|e| keep(e)) {
            allow.allow(&e.id, &e.version, &e.sha256);
            origins.record(&e.sha256, OriginRecord { origin: e.origin, id: e.id.clone(), version: e.version.clone(), signer: None });
        }
        (allow, origins)
    }

    /// What `newer` changes compared with `self`: added, removed, and changed (same id, other version or bytes).
    pub fn diff(&self, newer: &ApprovedList) -> ListDiff {
        let mut diff = ListDiff::default();
        for e in &newer.entries {
            match self.entries.iter().find(|o| o.id == e.id) {
                None => diff.added.push(e.clone()),
                Some(old) if old != e => diff.changed.push((old.clone(), e.clone())),
                Some(_) => {}
            }
        }
        for old in &self.entries {
            if !newer.entries.iter().any(|e| e.id == old.id) {
                diff.removed.push(old.clone());
            }
        }
        diff
    }
}

/// MX3JN1: a host key as the launcher and the game pass it around: `server:<host>:<port>` or `friend:<portal user id>`,
/// plain characters only (it names a folder and sits on a command line), at most [`MAX_HOST_KEY`] of them.
pub fn valid_host_key(key: &str) -> bool {
    let Some((kind, rest)) = key.split_once(':') else { return false };
    matches!(kind, "server" | "friend")
        && !rest.is_empty()
        && key.len() <= MAX_HOST_KEY
        && !rest.contains("..")
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'.' | b'-' | b'_' | b'[' | b']'))
}

/// Characters in a host key at most.
pub const MAX_HOST_KEY: usize = 200;

/// The folder name of a host key under `<mods>/remote/`: the key with `:` written as `+` (a colon is not allowed in a
/// Windows folder name, and neither a host name nor a portal id holds a `+`).
pub fn host_dir_name(key: &str) -> String {
    key.replace(':', "+")
}

/// MX3JN1: `<mods>/remote/<host dir>/`, where the packages fetched from one host live; `None` for an invalid key.
pub fn host_remote_dir(mods_dir: &Path, host_key: &str) -> Option<PathBuf> {
    valid_host_key(host_key).then(|| mods_dir.join("remote").join(host_dir_name(host_key)))
}

/// MX3JN1, for the launcher: stores a package fetched from the host `host_key` as `<mods>/remote/<host dir>/<sha256>.yak`
/// (named by its hash; never overwrites a file with other content; the control files are not touched) after checking
/// that the bytes hash to `expected_sha256` and form a valid package. Returns the stored path. Approval is the launcher's
/// separate act ([`ApprovedList::with_host`] with `server`/`friend` entries).
pub fn install_remote_package(mods_dir: &Path, host_key: &str, bytes: &[u8], expected_sha256: &str) -> Result<PathBuf, String> {
    let dir = host_remote_dir(mods_dir, host_key).ok_or_else(|| format!("not a host key: {host_key:?}"))?;
    let sha256 = sha256_hex(bytes);
    if !is_sha256_hex(expected_sha256) || sha256 != expected_sha256 {
        return Err("the package does not match the SHA-256 the host announced".into());
    }
    Archive::new(Cursor::new(bytes.to_vec()))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let target = dir.join(format!("{sha256}.yak"));
    {
        use std::io::Write;
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(mut file) => {
                file.write_all(bytes).map_err(|e| format!("{}: {e}", target.display()))?;
                let _ = file.sync_all();
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if std::fs::read(&target).map(|existing| sha256_hex(&existing) != sha256).unwrap_or(true) {
                    return Err(format!("{} already exists with other content", target.display()));
                }
            }
            Err(e) => return Err(format!("{}: {e}", target.display())),
        }
    }
    Ok(target)
}

/// The folder of packages delivered by a server or a friend: `<mods>/remote/<origin>/`.
pub fn remote_dir(mods_dir: &Path, origin: Origin) -> Option<PathBuf> {
    (!origin.can_be_full_access()).then(|| mods_dir.join("remote").join(origin.as_str()))
}

fn plain_yak_name(file_name: &str) -> bool {
    !(file_name.is_empty() || file_name.contains(['/', '\\', ':']) || file_name.starts_with('.') || !file_name.ends_with(".yak"))
}

/// Stores a `.yak` and, for the player's own installs and catalog downloads, records where it came from. **Never
/// overwrites a file with other content.**
///
/// * `Local` and `Catalog`: the file goes into `mods_dir` under `file_name` (refused if that name exists) and the origin
///   is recorded in `control`'s origin file. Returns the SHA-256.
/// * `Server` and `Friend`: the file goes into `<mods_dir>/remote/<origin>/<sha256>.yak` (named by its hash, so another
///   file already sitting under the sender's name cannot block it; `file_name` is only checked, not used), which the
///   full-access loader never scans. If a file of that name already exists its bytes must hash to the name (the same
///   package delivered twice is fine); anything else is refused. The control files are not touched at all.
pub fn install_package(control: &Control, mods_dir: &Path, file_name: &str, bytes: &[u8], origin: Origin, signer: Option<&str>) -> Result<String, String> {
    if !plain_yak_name(file_name) {
        return Err(format!("not a plain .yak file name: {file_name:?}"));
    }
    let archive = Archive::new(Cursor::new(bytes.to_vec()))?;
    let (id, version) = (archive.manifest().id.clone(), archive.manifest().version.to_string());
    let sha256 = sha256_hex(bytes);
    let target_dir = remote_dir(mods_dir, origin).unwrap_or_else(|| mods_dir.to_path_buf());
    std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
    let remote = !origin.can_be_full_access();
    let target = target_dir.join(if remote { format!("{sha256}.yak") } else { file_name.to_owned() });
    // `create_new`: refuse when it exists (also when two installs race)
    let created = {
        use std::io::Write;
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&target) {
            Ok(mut file) => {
                file.write_all(bytes).map_err(|e| format!("{}: {e}", target.display()))?;
                let _ = file.sync_all();
                true
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // the same package delivered twice is fine; any other content under that name is not
                let same = remote && std::fs::read(&target).map(|existing| sha256_hex(&existing) == sha256).unwrap_or(false);
                if same {
                    false
                } else {
                    return Err(format!("{} already exists: an installed package is never overwritten", target.display()));
                }
            }
            Err(e) => return Err(format!("{}: {e}", target.display())),
        }
    };
    if origin.can_be_full_access() {
        let recorded = (|| {
            let mut origins = control.origins()?;
            origins.record(&sha256, OriginRecord { origin, id, version, signer: signer.map(str::to_owned) });
            control.save_origins(&origins)
        })();
        if let Err(e) = recorded {
            // no record, no package: a file without a record is never full-access anyway, but do not leave it behind
            if created {
                let _ = std::fs::remove_file(&target);
            }
            return Err(e);
        }
    }
    Ok(sha256)
}

/// Removes an installed package file from `mods_dir` (an update first removes the old file, then installs the new).
pub fn remove_package(mods_dir: &Path, file_name: &str) -> Result<(), String> {
    if !plain_yak_name(file_name) {
        return Err(format!("not a plain .yak file name: {file_name:?}"));
    }
    std::fs::remove_file(mods_dir.join(file_name)).map_err(|e| e.to_string())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("yak_control_tests").join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    #[test]
    fn the_control_folder_must_be_outside_the_game_root_and_have_no_dotdot() {
        let base = scratch("outside");
        let root = base.join("game");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        let outside = base.join("launcher").join(CONTROL_SUBDIR);
        assert!(Control::open(&outside, &root).is_ok());
        for inside in [root.join("mods"), root.join("mods").join(CONTROL_SUBDIR), root.clone()] {
            let error = Control::open(&inside, &root).unwrap_err();
            assert!(error.contains("inside the game folder"), "{inside:?}: {error}");
        }
        // a `..` is refused outright, whatever it resolves to
        let dotdot = with_dotdot(&root, "a/../mods");
        assert!(Control::open(&dotdot, &root).unwrap_err().contains("`..`"));
        assert!(Control::open(&with_dotdot(&base, "x/../launcher/c"), &root).unwrap_err().contains("`..`"));
        // a folder that contains the game root is refused too
        assert!(Control::open(&base, &root).is_err());
        assert!(Control::open(Path::new("relative/dir"), &root).unwrap_err().contains("absolute"));
        assert_eq!(control_dir_in(&base.join("launcher")), outside);
    }

    #[test]
    fn guard_refuses_inside_ancestor_and_dotdot() {
        let base = scratch("protect");
        let control = base.join("launcher").join(CONTROL_SUBDIR);
        std::fs::create_dir_all(&control).unwrap();
        protect(&control);
        // inside
        assert!(guard(&control).is_err());
        assert!(guard(&control.join("full-access.toml")).is_err());
        assert!(guard(&control.join("not").join("there").join("yet")).is_err(), "a path that does not exist yet");
        // an ancestor of the protected folder: a storage root that contains it
        assert!(guard(&base.join("launcher")).is_err(), "the parent");
        assert!(guard(&base).is_err(), "a grandparent");
        // `..` anywhere is refused, even when it would resolve to a harmless place
        assert!(guard(&with_dotdot(&base, "game/../elsewhere")).is_err());
        assert!(guard(&with_dotdot(&base, &format!("game/mods/../../launcher/{CONTROL_SUBDIR}/x"))).is_err());
        // unrelated paths and a sibling with the same prefix are fine
        assert!(guard(&base.join("game")).is_ok());
        assert!(guard(&base.join("launcher").join("other")).is_ok());
        assert!(guard(&base.join("launcher").join(format!("{CONTROL_SUBDIR}-sibling"))).is_ok());
        assert!(guard(&base.join("gamer")).is_ok());
    }

    #[test]
    fn a_damaged_or_unrecorded_file_fails_the_corruption_check() {
        let base = scratch("checksum");
        let root = base.join("game");
        let control = Control::create(&base.join("launcher").join(CONTROL_SUBDIR), &root).unwrap();
        assert_eq!(control.allow().unwrap(), AllowFile::default(), "nothing saved yet: empty, fine");
        let mut allow = AllowFile::default();
        allow.allow("one", "1.0.0", &"a".repeat(64));
        control.save_allow(&allow).unwrap();
        assert_eq!(control.allow().unwrap(), allow);
        assert!(control.checksum_path().ends_with("yoake-full-access.checksum"));
        // the file is edited behind the launcher's back: the launcher refuses it
        let path = control.dir().join(AllowFile::FILE_NAME);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&format!("\n[[allow]]\nid = \"evil\"\nversion = \"1\"\nsha256 = \"{}\"\n", "b".repeat(64)));
        std::fs::write(&path, &text).unwrap();
        assert!(control.allow().unwrap_err().contains("does not match the checksum"));
        // after the player confirmed the diff, the launcher records the current files
        control.accept_current_files().unwrap();
        assert!(control.allow().unwrap().is_allowed("evil", &"b".repeat(64)));
        // a file with no recorded checksum, a deleted file and a deleted checksum file are refused
        std::fs::write(control.dir().join(OriginFile::FILE_NAME), "").unwrap();
        assert!(control.origins().unwrap_err().contains("no recorded checksum"));
        std::fs::remove_file(&path).unwrap();
        assert!(control.allow().unwrap_err().contains("missing"));
        control.save_allow(&allow).unwrap();
        std::fs::remove_file(control.checksum_path()).unwrap();
        assert!(control.allow().unwrap_err().contains("no recorded checksum"));
    }

    #[test]
    fn atomic_writes_replace_and_leave_no_temp_files() {
        let dir = scratch("atomic");
        let file = dir.join("x.toml");
        write_atomic(&file, b"one").unwrap();
        write_atomic(&file, b"two").unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["x.toml"], "no temp file is left");
    }

    /// `base` joined with `rest` as plain text: `PathBuf::join` would resolve a `..` on a verbatim (`\?\`) path.
    fn with_dotdot(base: &Path, rest: &str) -> PathBuf {
        let base = base.to_string_lossy();
        PathBuf::from(format!("{}/{rest}", base.trim_start_matches(r"\\?\")))
    }

    fn entry(id: &str, hash_char: char, origin: Origin) -> ApprovedEntry {
        ApprovedEntry { id: id.into(), version: "1.0.0".into(), sha256: hash_char.to_string().repeat(64), origin }
    }

    #[test]
    fn the_approved_list_round_trips_and_only_local_and_catalog_are_valid() {
        let list = ApprovedList::new(vec![entry("one", 'a', Origin::Local), entry("two", 'b', Origin::Catalog)], vec![PathBuf::from(r"C:\Users\me\AppData\Roaming\horizon-launcher\yoake-full-access")]);
        assert_eq!(ApprovedList::parse(&list.to_text()).unwrap(), list, "paths with backslashes survive");
        for bad_origin in ["server", "friend", "usb"] {
            let text = format!("format = 1\n[[package]]\nid = \"x\"\nversion = \"1\"\nsha256 = \"{}\"\norigin = \"{bad_origin}\"\n", "a".repeat(64));
            assert!(ApprovedList::parse(&text).is_err(), "{bad_origin}");
        }
        assert!(ApprovedList::parse("format = 2\n").is_err());
        assert_eq!(ApprovedList::parse(&list.to_text()).unwrap().generation(), 0, "old files have no generation");
        let g = list.clone().with_generation(7);
        assert!(g.to_text().contains("generation = 7"));
        assert_eq!(ApprovedList::parse(&g.to_text()).unwrap(), g);
        assert!(ApprovedList::parse("format = 1\ngeneration = -1\n").is_err());
        assert!(ApprovedList::parse("format = 1\ngeneration = \"x\"\n").is_err());
        assert!(ApprovedList::parse("").is_err(), "no format line");
        assert!(ApprovedList::parse("format = 1\n[[package]]\nid = \"x\"\nversion = \"1\"\nsha256 = \"short\"\norigin = \"local\"\n").is_err());
        // one list per launch, never overwritten
        let dir = scratch("listfile").join("run-1");
        let path = list.write_for_launch(&dir).unwrap();
        assert_eq!(ApprovedList::read(&path).unwrap(), list);
        assert!(list.write_for_launch(&dir).is_err());
        // the allow/origin pair the loader works with
        let (allow, origins) = list.to_files();
        assert!(allow.is_allowed("one", &"a".repeat(64)) && !allow.is_allowed("one", &"b".repeat(64)));
        assert_eq!(origins.origin_of(&"b".repeat(64)), Some(Origin::Catalog));
    }

    #[test]
    fn the_dev_origin_round_trips_can_be_full_access_and_old_files_still_read() {
        assert_eq!(Origin::parse("dev"), Some(Origin::Dev));
        assert_eq!(Origin::Dev.as_str(), "dev");
        assert!(Origin::Dev.can_be_full_access());
        assert!(!Origin::Server.can_be_full_access() && !Origin::Friend.can_be_full_access());
        assert!(remote_dir(Path::new("mods"), Origin::Dev).is_none(), "a dev package sits top-level like a local one");
        // the approved list, with a dev entry and a generation
        let list = ApprovedList::new(vec![entry("d", 'd', Origin::Dev), entry("l", 'f', Origin::Local)], vec![]).with_generation(7);
        let text = list.to_text();
        assert!(text.contains("generation = 7") && text.contains("origin = \"dev\""), "{text}");
        let back = ApprovedList::parse(&text).unwrap();
        assert_eq!(back, list);
        assert_eq!(back.generation(), 7);
        // generation 0 is not written, and a file without it (every older launcher) reads as 0
        let plain = ApprovedList::new(vec![entry("l", 'a', Origin::Local)], vec![]);
        assert!(!plain.to_text().contains("generation"));
        assert_eq!(ApprovedList::parse(&plain.to_text()).unwrap().generation(), 0);
        for bad in ["generation = -1", "generation = \"x\""] {
            assert!(ApprovedList::parse(&format!("format = 1\n{bad}\n")).is_err(), "{bad}");
        }
        // the origin file: a dev record round-trips, and a file written before `dev` existed still reads
        let mut origins = OriginFile::default();
        origins.record(&"d".repeat(64), OriginRecord { origin: Origin::Dev, id: "d".into(), version: "1.0.0".into(), signer: None });
        origins.record(&"a".repeat(64), OriginRecord { origin: Origin::Catalog, id: "a".into(), version: "1.0.0".into(), signer: Some("me".into()) });
        let again = OriginFile::parse(&origins.to_text()).unwrap();
        assert_eq!(again.origin_of(&"d".repeat(64)), Some(Origin::Dev));
        assert_eq!(again.origin_of(&"a".repeat(64)), Some(Origin::Catalog));
        let old = format!("[[package]]\nsha256 = \"{}\"\norigin = \"local\"\nid = \"x\"\nversion = \"1\"\n", "c".repeat(64));
        assert_eq!(OriginFile::parse(&old).unwrap().origin_of(&"c".repeat(64)), Some(Origin::Local));
    }

    #[test]
    fn the_diff_shows_added_removed_and_changed() {
        let old = ApprovedList::new(vec![entry("keep", 'a', Origin::Local), entry("gone", 'b', Origin::Local), entry("upd", 'c', Origin::Local)], vec![]);
        let new = ApprovedList::new(vec![entry("keep", 'a', Origin::Local), entry("upd", 'd', Origin::Local), entry("fresh", 'e', Origin::Catalog)], vec![]);
        let diff = old.diff(&new);
        assert_eq!(diff.added, [entry("fresh", 'e', Origin::Catalog)]);
        assert_eq!(diff.removed, [entry("gone", 'b', Origin::Local)]);
        assert_eq!(diff.changed, [(entry("upd", 'c', Origin::Local), entry("upd", 'd', Origin::Local))]);
        assert!(old.diff(&old).is_empty());
    }

    #[test]
    fn the_list_path_comes_from_the_option_or_the_environment() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(ApprovedList::path_from(args(&["--x", "--full-access-approved", "/a/b"]), None), Some(PathBuf::from("/a/b")));
        assert_eq!(ApprovedList::path_from(args(&["--full-access-approved=/c"]), Some("/env".into())), Some(PathBuf::from("/c")), "the option wins");
        assert_eq!(ApprovedList::path_from(args(&[]), Some("/env".into())), Some(PathBuf::from("/env")));
        assert_eq!(ApprovedList::path_from(args(&["--full-access-approved"]), None), None);
        assert_eq!(ApprovedList::path_from(args(&[]), None), None, "no list: nothing is full-access");
        assert_eq!(ApprovedList::path_from(args(&[]), Some("".into())), None);
    }

    fn data_package(dir: &Path, id: &str) -> Vec<u8> {
        let src = dir.join(format!("src_{id}"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("yak.toml"),
            format!("format = 1\nid = '{id}'\nversion = '1.0.0'\nname = '{id}'\nauthors = ['t']\ndescription = 'd'\n[yoake]\nversion = '*'\n[side]\nclient = true\nserver = false\nrequired_on_join = false\n"),
        )
        .unwrap();
        let out = dir.join(format!("{id}.yak"));
        super::super::pack_directory(&src, &out, &BTreeMap::new()).unwrap();
        std::fs::read(out).unwrap()
    }

    #[test]
    fn a_list_keeps_its_host_and_its_generation_together() {
        let key = "friend:user_7";
        let list = ApprovedList::new(vec![entry("d", 'd', Origin::Dev), entry("l", 'a', Origin::Local), entry("f", 'f', Origin::Friend)], vec![]).with_host(key).with_generation(4);
        let again = ApprovedList::parse(&list.to_text()).unwrap();
        assert_eq!((again.host(), again.generation()), (Some(key), 4));
        assert_eq!(again, list);
        let error = ApprovedList::parse(&list.to_text().replace(r#"origin = "dev""#, r#"origin = "nope""#)).unwrap_err();
        for word in ["local", "catalog", "dev", "server", "friend"] {
            assert!(error.contains(word), "{error}");
        }
    }

    #[test]
    fn a_servers_or_friends_package_is_valid_only_in_a_list_scoped_to_its_host() {
        let server_key = "server:play.example.com:25565";
        let friend_key = "friend:user_7";
        let remote = |id: &str, hash: char, origin: Origin| entry(id, hash, origin);
        let list = ApprovedList::new(vec![entry("mine", 'a', Origin::Local), remote("theirs", 'b', Origin::Server)], vec![]).with_host(server_key);
        assert_eq!(list.host(), Some(server_key));
        assert_eq!(ApprovedList::parse(&list.to_text()).unwrap(), list, "the host survives the file");
        // the plain mods folder never sees the host's entries; the host's folder sees only them, and only for its own key
        let (allow, origins) = list.to_files();
        assert!(allow.is_allowed("mine", &"a".repeat(64)) && !allow.is_allowed("theirs", &"b".repeat(64)));
        assert_eq!(origins.origin_of(&"b".repeat(64)), None);
        let (allow, origins) = list.to_remote_files(server_key);
        assert!(allow.is_allowed("theirs", &"b".repeat(64)) && !allow.is_allowed("mine", &"a".repeat(64)));
        assert_eq!(origins.origin_of(&"b".repeat(64)), Some(Origin::Server));
        for other in ["server:other.example.com:25565", friend_key, "server:play.example.com:25566", ""] {
            let (allow, origins) = list.to_remote_files(other);
            assert!(allow.entries().is_empty() && origins.is_empty(), "{other:?}");
        }
        // a list without a host cannot carry a remote entry, nor can one scoped to the other kind of host
        let unscoped = ApprovedList::new(vec![remote("theirs", 'b', Origin::Server)], vec![]);
        assert!(ApprovedList::parse(&unscoped.to_text()).is_err());
        let wrong_kind = ApprovedList::new(vec![remote("theirs", 'b', Origin::Friend)], vec![]).with_host(server_key);
        assert!(ApprovedList::parse(&wrong_kind.to_text()).is_err());
        let friend = ApprovedList::new(vec![remote("theirs", 'b', Origin::Friend)], vec![]).with_host(friend_key);
        assert_eq!(ApprovedList::parse(&friend.to_text()).unwrap(), friend);
        // a bad host is no list; a list with no remote entries may still name one or none
        for bad in ["", "server", "server:../x", "usb:x", "server:a b"] {
            let text = format!("format = 1\nhost = {bad:?}\n");
            assert!(ApprovedList::parse(&text).is_err(), "{bad:?}");
        }
        assert!(ApprovedList::parse("format = 1\n").unwrap().host().is_none());
        // a launch with no host scope has no remote files at all
        let (allow, _) = ApprovedList::new(vec![entry("mine", 'a', Origin::Local)], vec![]).to_remote_files(server_key);
        assert!(allow.entries().is_empty());
    }

    #[test]
    fn host_keys_and_their_folder_names() {
        for good in ["server:a.example:25565", "server:[::1]:25565", "friend:u-1_x", "server:192.168.0.2:1"] {
            assert!(valid_host_key(good), "{good}");
        }
        for bad in ["", "server", "server:", "friend", "x:y", "server:a/b", "server:a b", "server:..", "server:a..b", "server:é:1"] {
            assert!(!valid_host_key(bad), "{bad:?}");
        }
        assert!(!valid_host_key(&format!("friend:{}", "a".repeat(MAX_HOST_KEY))));
        assert_eq!(host_dir_name("server:a.example:25565"), "server+a.example+25565");
        let mods = Path::new("mods");
        assert_eq!(host_remote_dir(mods, "friend:u1"), Some(mods.join("remote").join("friend+u1")));
        assert_eq!(host_remote_dir(mods, "../x"), None);
    }

    #[test]
    fn a_remote_package_is_stored_by_its_hash_under_its_host_and_never_overwritten() {
        let base = scratch("install_remote");
        let bytes = data_package(&base, "demo_remote");
        let hash = sha256_hex(&bytes);
        let mods = base.join("mods");
        let key = "server:play.example.com:25565";
        let path = install_remote_package(&mods, key, &bytes, &hash).unwrap();
        assert_eq!(path, mods.join("remote").join("server+play.example.com+25565").join(format!("{hash}.yak")));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(install_remote_package(&mods, key, &bytes, &hash).unwrap(), path, "the same package twice is fine");
        // another host gets its own folder
        let other = install_remote_package(&mods, "friend:u1", &bytes, &hash).unwrap();
        assert_ne!(other.parent(), path.parent());
        // wrong hash, bad key, not a package: nothing is written
        assert!(install_remote_package(&mods, key, &bytes, &"0".repeat(64)).is_err());
        assert!(install_remote_package(&mods, "server:../x", &bytes, &hash).is_err());
        assert!(install_remote_package(&mods, "server:b.example:1", b"not a package", &sha256_hex(b"not a package")).is_err());
        assert!(!mods.join("remote").join("server+b.example+1").exists());
        // the plain mods folder is untouched and the full-access loader's scan finds nothing
        assert!(std::fs::read_dir(&mods).unwrap().flatten().all(|item| item.file_name() == "remote"));
        // a file under that name with other bytes is refused
        std::fs::write(&path, b"other").unwrap();
        assert!(install_remote_package(&mods, key, &bytes, &hash).is_err());
    }
}
