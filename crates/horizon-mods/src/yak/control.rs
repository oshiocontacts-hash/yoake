//! MX3L3: the control files of full-access mods, kept where no mod can reach them.
//!
//! Two files decide what may run with the engine's own powers:
//!
//! * `full-access.toml` ([`AllowFile`]): the player's approvals (id + version + SHA-256 of the `.yak` file);
//! * `yak-origins.toml` ([`OriginFile`]): where each package came from, recorded when it arrived.
//!
//! A running full-access mod has engine-level power, so these files must not live in the game folder where a
//! file-writing binding could reach them. They live in a **launcher-owned folder outside the game root**
//! (`<launcher data dir>/yoake-full-access/`, see [`control_dir_in`]) that the launcher passes to the game as an
//! explicit path ([`CONTROL_ARG`] or [`CONTROL_ENV`]). Without that path nothing is full-access. [`Control::open`]
//! refuses a folder inside the game root and registers the folder (and its seal file) as **protected**
//! ([`protect`], [`guard`]): every storage root or file binding must call [`guard`] on the paths it is about to
//! touch and refuse when it fails (`yoake-wasm`'s disk storage does).
//!
//! **Tamper detection.** Each save is atomic (temp file, then rename) and then recorded in a *seal*,
//! `<control folder>.seal` next to the folder (`yoake-full-access.seal`), holding the SHA-256 of both files. A game
//! that reads a file whose hash differs from the seal (hand-edited, replaced, half-written, or a seal that is
//! missing for an existing file) refuses it: [`Control::allow`] / [`Control::origins`] return an error and the
//! loader runs nothing as full-access. Only the launcher side ([`Control::save_allow`], [`Control::save_origins`],
//! [`Control::reseal`]) writes seals. **This is a hash, not a MAC**: it catches edits by anything that does not also
//! rewrite the seal. The next step is an HMAC keyed by a secret the launcher keeps in the OS credential store
//! (`launcher/src/account.rs` has the keychain and an encrypted-file fallback) so that even a process that can write
//! both files cannot forge a seal; the game would need that key handed to it, which is why it is not done yet.
//!
//! **Packages from servers and friends** never enter the folder of approved mods: [`install_package`] puts them in
//! `<mods>/remote/<origin>/` (not scanned by the loader, so they take no part in duplicate-id detection), refuses to
//! overwrite an existing file, and never touches the control files.
//!
//! **The launcher's entry point** is [`Control::install_local`] (a `.yak` the player chose: copy it into the mods
//! folder and record origin `local`) plus [`Control::approve`] (the player's approval of that exact file). A `.yak` a
//! player drops into the mods folder by hand has no origin record and stays not-full-access until the launcher records
//! it.

use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

use super::Archive;

/// Command-line option that gives the game the control folder: `--full-access-dir <path>`.
pub const CONTROL_ARG: &str = "--full-access-dir";
/// Environment variable with the same meaning (the option wins).
pub const CONTROL_ENV: &str = "YOAKE_FULL_ACCESS_DIR";
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

// ---------------------------------------------------------------------------------------------------------------
// Protected paths
// ---------------------------------------------------------------------------------------------------------------

static PROTECTED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// A path with `.` and `..` resolved and symlinks followed as far as the path exists: the deepest existing
/// ancestor is canonicalized and the rest (which may not exist yet) is appended with `..` applied.
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

/// Whether `path` is, or is inside, a protected path (symlinks and `..` resolved).
pub fn is_protected(path: &Path) -> bool {
    let canonical = canonical_lenient(path);
    PROTECTED.lock().unwrap_or_else(|e| e.into_inner()).iter().any(|p| is_inside(&canonical, p))
}

/// `Err` when `path` is inside a protected path. **Every storage root and file-taking binding calls this before it
/// touches a path.**
pub fn guard(path: &Path) -> Result<(), String> {
    if is_protected(path) {
        Err(format!("{} is inside a protected folder (the full-access control files)", path.display()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn unprotect_all_for_test() {
    PROTECTED.lock().unwrap_or_else(|e| e.into_inner()).clear();
}

// ---------------------------------------------------------------------------------------------------------------
// Atomic writes
// ---------------------------------------------------------------------------------------------------------------

/// Writes `bytes` to a temporary file next to `path`, then renames it over `path`: a reader sees the old file or the
/// new one, never half of it.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = path.file_name().ok_or("no file name")?.to_string_lossy().into_owned();
    let temp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    std::fs::write(&temp, bytes).map_err(|e| format!("{}: {e}", temp.display()))?;
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
// The control folder
// ---------------------------------------------------------------------------------------------------------------

/// The launcher-owned folder holding the two control files and their seal.
#[derive(Clone, Debug)]
pub struct Control {
    dir: PathBuf,
}

impl Control {
    pub const SEAL_KEY_ALLOW: &'static str = "allow";
    pub const SEAL_KEY_ORIGINS: &'static str = "origins";

    /// Opens the control folder `dir` for a game whose root is `game_root`. Refuses a relative path, a folder inside
    /// the game root (or the game root inside it) and registers the folder and its seal as protected. The folder is
    /// not created (see [`Control::create`]).
    pub fn open(dir: &Path, game_root: &Path) -> Result<Self, String> {
        if !dir.is_absolute() {
            return Err(format!("the full-access control folder must be an absolute path, got {}", dir.display()));
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
        protect(&control.seal_path());
        Ok(control)
    }

    /// [`Control::open`], creating the folder (launcher side).
    pub fn create(dir: &Path, game_root: &Path) -> Result<Self, String> {
        let control = Self::open(dir, game_root)?;
        std::fs::create_dir_all(&control.dir).map_err(|e| format!("{}: {e}", control.dir.display()))?;
        Ok(control)
    }

    /// The control folder given to this process: `--full-access-dir <path>` (or `--full-access-dir=<path>`), else
    /// [`CONTROL_ENV`]. `None`: nothing is full-access.
    pub fn path_from_process() -> Option<PathBuf> {
        Self::path_from(std::env::args().skip(1), std::env::var_os(CONTROL_ENV))
    }

    pub fn path_from(args: impl IntoIterator<Item = String>, env: Option<std::ffi::OsString>) -> Option<PathBuf> {
        let args: Vec<String> = args.into_iter().collect();
        for (i, arg) in args.iter().enumerate() {
            if arg == CONTROL_ARG {
                return args.get(i + 1).filter(|v| !v.is_empty()).map(PathBuf::from);
            }
            if let Some(value) = arg.strip_prefix(&format!("{CONTROL_ARG}=")) {
                return (!value.is_empty()).then(|| PathBuf::from(value));
            }
        }
        env.filter(|v| !v.is_empty()).map(PathBuf::from)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `<control folder>.seal`: next to the folder, in the launcher's data folder.
    pub fn seal_path(&self) -> PathBuf {
        let mut name = self.dir.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        name.push(".seal");
        self.dir.with_file_name(name)
    }

    fn seal(&self) -> Result<BTreeMap<String, String>, String> {
        let text = match std::fs::read_to_string(self.seal_path()) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(format!("{}: {e}", self.seal_path().display())),
        };
        let table: toml::Table = text.parse().map_err(|e| format!("seal: {e}"))?;
        Ok(table.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned()))).collect())
    }

    fn read_checked(&self, file_name: &str, key: &str) -> Result<Option<String>, String> {
        let bytes = match std::fs::read(self.dir.join(file_name)) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("{file_name}: {e}")),
        };
        let sealed = self.seal()?.get(key).cloned();
        match (bytes, sealed) {
            (None, None) => Ok(None),
            (Some(bytes), Some(hash)) if sha256_hex(&bytes) == hash => String::from_utf8(bytes).map(Some).map_err(|e| format!("{file_name}: {e}")),
            (Some(_), Some(_)) => Err(format!("{file_name} changed since the launcher last wrote it (it does not match its seal): refused")),
            (Some(_), None) => Err(format!("{file_name} has no seal (the launcher did not write it): refused")),
            (None, Some(_)) => Err(format!("{file_name} is missing although the launcher sealed it: refused")),
        }
    }

    /// The approvals; an empty file when none was ever saved. `Err` when the file does not match its seal or does not parse.
    pub fn allow(&self) -> Result<AllowFile, String> {
        match self.read_checked(AllowFile::FILE_NAME, Self::SEAL_KEY_ALLOW)? {
            Some(text) => AllowFile::parse(&text),
            None => Ok(AllowFile::default()),
        }
    }

    /// The arrival records; see [`Control::allow`].
    pub fn origins(&self) -> Result<OriginFile, String> {
        match self.read_checked(OriginFile::FILE_NAME, Self::SEAL_KEY_ORIGINS)? {
            Some(text) => OriginFile::parse(&text),
            None => Ok(OriginFile::default()),
        }
    }

    fn write_seal(&self, key: &str, hash: &str) -> Result<(), String> {
        let mut seal = self.seal().unwrap_or_default();
        seal.insert(key.to_owned(), hash.to_owned());
        let mut text = String::from("# SHA-256 of the full-access control files as the launcher last wrote them. Do not edit.\n");
        for (k, v) in &seal {
            text.push_str(&format!("{k} = {v:?}\n"));
        }
        write_atomic(&self.seal_path(), text.as_bytes())
    }

    fn save_text(&self, file_name: &str, key: &str, text: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        write_atomic(&self.dir.join(file_name), text.as_bytes())?;
        self.write_seal(key, &sha256_hex(text.as_bytes()))
    }

    /// Saves the approvals (launcher side): atomic, then sealed.
    pub fn save_allow(&self, allow: &AllowFile) -> Result<(), String> {
        self.save_text(AllowFile::FILE_NAME, Self::SEAL_KEY_ALLOW, &allow.to_text())
    }

    /// Saves the arrival records (launcher side): atomic, then sealed.
    pub fn save_origins(&self, origins: &OriginFile) -> Result<(), String> {
        self.save_text(OriginFile::FILE_NAME, Self::SEAL_KEY_ORIGINS, &origins.to_text())
    }

    /// Seals the files as they are now (launcher side, after the player confirmed a repair of a refused file).
    pub fn reseal(&self) -> Result<(), String> {
        for (name, key) in [(AllowFile::FILE_NAME, Self::SEAL_KEY_ALLOW), (OriginFile::FILE_NAME, Self::SEAL_KEY_ORIGINS)] {
            if let Ok(bytes) = std::fs::read(self.dir.join(name)) {
                self.write_seal(key, &sha256_hex(&bytes))?;
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

    /// Records the player's approval of exactly this package file (launcher side).
    pub fn approve(&self, bytes: &[u8]) -> Result<AllowEntry, String> {
        let mut allow = self.allow()?;
        let entry = allow.allow_package(bytes)?;
        self.save_allow(&allow)?;
        Ok(entry)
    }
}

/// The folder of packages delivered by a server or a friend: `<mods>/remote/<origin>/`.
pub fn remote_dir(mods_dir: &Path, origin: Origin) -> Option<PathBuf> {
    (!origin.can_be_full_access()).then(|| mods_dir.join("remote").join(origin.as_str()))
}

fn plain_yak_name(file_name: &str) -> bool {
    !(file_name.is_empty() || file_name.contains(['/', '\\', ':']) || file_name.starts_with('.') || !file_name.ends_with(".yak"))
}

/// Stores a `.yak` and, for the player's own installs and catalog downloads, records where it came from. **Never
/// overwrites an existing file.**
///
/// * `Local` and `Catalog`: the file goes into `mods_dir` and the origin is recorded (in `control`'s origin file,
///   sealed). Returns the SHA-256.
/// * `Server` and `Friend`: the file goes into `<mods_dir>/remote/<origin>/`, which the full-access loader never
///   scans, so it cannot take part in duplicate-id detection or disturb an approved mod; the control files are not
///   touched at all.
pub fn install_package(control: &Control, mods_dir: &Path, file_name: &str, bytes: &[u8], origin: Origin, signer: Option<&str>) -> Result<String, String> {
    if !plain_yak_name(file_name) {
        return Err(format!("not a plain .yak file name: {file_name:?}"));
    }
    let archive = Archive::new(Cursor::new(bytes.to_vec()))?;
    let (id, version) = (archive.manifest().id.clone(), archive.manifest().version.to_string());
    let sha256 = sha256_hex(bytes);
    let target_dir = remote_dir(mods_dir, origin).unwrap_or_else(|| mods_dir.to_path_buf());
    std::fs::create_dir_all(&target_dir).map_err(|e| e.to_string())?;
    let target = target_dir.join(file_name);
    // `create_new`: refuse when it exists (also when two installs race)
    {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map_err(|e| {
            if e.kind() == std::io::ErrorKind::AlreadyExists {
                format!("{} already exists: an installed package is never overwritten", target.display())
            } else {
                format!("{}: {e}", target.display())
            }
        })?;
        file.write_all(bytes).map_err(|e| format!("{}: {e}", target.display()))?;
    }
    if origin.can_be_full_access() {
        let recorded = (|| {
            let mut origins = control.origins()?;
            origins.record(&sha256, OriginRecord { origin, id, version, signer: signer.map(str::to_owned) });
            control.save_origins(&origins)
        })();
        if let Err(e) = recorded {
            // no record, no package: a file without a record is never full-access anyway, but do not leave it behind
            let _ = std::fs::remove_file(&target);
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
    fn the_control_folder_must_be_outside_the_game_root() {
        let base = scratch("outside");
        let root = base.join("game");
        std::fs::create_dir_all(root.join("mods")).unwrap();
        let outside = base.join("launcher").join(CONTROL_SUBDIR);
        assert!(Control::open(&outside, &root).is_ok());
        for inside in [root.join("mods"), root.join("mods").join(CONTROL_SUBDIR), root.join("a").join("..").join("mods"), root.clone()] {
            let error = Control::open(&inside, &root).unwrap_err();
            assert!(error.contains("inside the game folder"), "{inside:?}: {error}");
        }
        // a folder that contains the game root is refused too (the game would sit inside the launcher's secrets)
        assert!(Control::open(&base, &root).is_err());
        assert!(Control::open(Path::new("relative/dir"), &root).unwrap_err().contains("absolute"));
        assert_eq!(control_dir_in(&base.join("launcher")), outside);
    }

    #[test]
    fn protected_paths_refuse_traversal_and_missing_children() {
        let base = scratch("protect");
        let control = base.join("launcher").join(CONTROL_SUBDIR);
        std::fs::create_dir_all(&control).unwrap();
        protect(&control);
        assert!(guard(&control).is_err());
        assert!(guard(&control.join("full-access.toml")).is_err());
        assert!(guard(&control.join("not").join("there").join("yet")).is_err(), "a path that does not exist yet");
        let sneaky = base.join("game").join("mods").join("..").join("..").join("launcher").join(CONTROL_SUBDIR).join("x");
        assert!(guard(&sneaky).is_err(), "a `..` traversal from a folder that does not exist");
        assert!(guard(&base.join("game")).is_ok());
        assert!(guard(&base.join("launcher").join("other")).is_ok());
        assert!(guard(&base.join("launcher").join(format!("{CONTROL_SUBDIR}-sibling"))).is_ok(), "a sibling with the same prefix is not inside");
    }

    #[test]
    fn a_tampered_or_unsealed_file_is_refused() {
        let base = scratch("seal");
        let root = base.join("game");
        let control = Control::create(&base.join("launcher").join(CONTROL_SUBDIR), &root).unwrap();
        assert_eq!(control.allow().unwrap(), AllowFile::default(), "nothing saved yet: empty, fine");
        let mut allow = AllowFile::default();
        allow.allow("one", "1.0.0", &"a".repeat(64));
        control.save_allow(&allow).unwrap();
        assert_eq!(control.allow().unwrap(), allow);
        // someone edits the file (adds an approval) without the launcher: refused
        let path = control.dir().join(AllowFile::FILE_NAME);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str(&format!("\n[[allow]]\nid = \"evil\"\nversion = \"1\"\nsha256 = \"{}\"\n", "b".repeat(64)));
        std::fs::write(&path, &text).unwrap();
        assert!(control.allow().unwrap_err().contains("changed since the launcher last wrote it"));
        // the launcher repairs it on purpose
        control.reseal().unwrap();
        assert!(control.allow().unwrap().is_allowed("evil", &"b".repeat(64)));
        // a file with no seal at all, a deleted file and a deleted seal are refused
        std::fs::write(control.dir().join(OriginFile::FILE_NAME), "").unwrap();
        assert!(control.origins().unwrap_err().contains("no seal"));
        std::fs::remove_file(&path).unwrap();
        assert!(control.allow().unwrap_err().contains("missing"));
        control.save_allow(&allow).unwrap();
        std::fs::remove_file(control.seal_path()).unwrap();
        assert!(control.allow().unwrap_err().contains("no seal"));
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

    #[test]
    fn the_path_comes_from_the_option_or_the_environment() {
        let args = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(Control::path_from(args(&["--x", "--full-access-dir", "/a/b"]), None), Some(PathBuf::from("/a/b")));
        assert_eq!(Control::path_from(args(&["--full-access-dir=/c"]), Some("/env".into())), Some(PathBuf::from("/c")), "the option wins");
        assert_eq!(Control::path_from(args(&[]), Some("/env".into())), Some(PathBuf::from("/env")));
        assert_eq!(Control::path_from(args(&["--full-access-dir"]), None), None);
        assert_eq!(Control::path_from(args(&[]), None), None, "no path: nothing is full-access");
        assert_eq!(Control::path_from(args(&[]), Some("".into())), None);
    }
}
