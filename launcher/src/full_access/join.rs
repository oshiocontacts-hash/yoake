//! MX3JN2: join-time mod sync, the launcher's side (docs/yoake2/JOIN-MODS.md, docs/yoake2/reports/MX3JN1.md).
//!
//! The owner's decision: when a player joins a host and lacks mods the host runs, they are fetched and installed
//! **automatically, with no confirmation, full-access ones included**. The game does the fetching (it stages verified
//! files in `<instance>/mods/remote/.staging/`, writes `join-request.json` and exits with code
//! [`EXIT_CODE`] = 75); this module is everything the launcher does about it:
//!
//! * [`handle_exit`]: read and strictly validate the request, verify every staged file against its SHA-256, install it
//!   into `<instance>/mods/remote/<host dir>/` (`install_remote_package`), record it (and, for a full-access one, its
//!   approval) **for that host only**, and empty the staging folder. All or nothing: any failed check installs nothing
//!   from the request. Runs under the control lock; the launcher calls it from a worker thread.
//! * [`Prepared`](super::Prepared) with a host: later launches that join the same host get `--remote-mods <key>` and an
//!   approved list scoped to it (`ApprovedList::with_host`), made of the player's own accepted entries plus the host's
//!   full-access mods. Launches that join anything else get neither.
//! * [`remote_hosts`] / [`remove_remote_host`]: what the Mods page shows ("from <host>") and "Remove all from this host".
//!
//! # Where the approval lives
//!
//! Not in `full-access.toml` / `yak-origins.toml`: those are keyed by hash only and shared by every host (a remote
//! record there would make a player's own copy of the same bytes "distrusted" for good, and an id approved for the
//! host would show up as "content changed" on the player's own mod of that id). A host's mods are recorded in their
//! own launcher-owned file, `<control folder>/remote-hosts/<instance>/<host dir>.json`, in the control folder the game
//! must never touch (it is in every list's `protected` paths). A launch passes a recorded full-access mod only if the
//! file `<host dir>/<sha256>.yak` still exists and still hashes to the record.
//!
//! # Catalog first
//!
//! The contract says an entry with a `catalog_id` that the signed catalog knows is downloaded from the catalog instead.
//! Hosts do not send `catalog_id` yet, so [`catalog_bytes`] is only the place where that goes; it returns `None` and
//! the staged, hash-checked file is used. Nothing here guesses a catalog entry.
use super::{last_path, lock, mods_dir, open_control, ApprovedEntry, ApprovedList, Control};
use horizon_mods::yak::control::{self, host_dir_name, host_remote_dir, install_remote_package, valid_host_key, Origin};
use horizon_mods::yak::Archive;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The game's exit code for "I fetched mods for this host; install them and start me again".
pub const EXIT_CODE: i32 = 75;
/// Game argument: fetch the host's missing mods when joining.
pub const SYNC_ARG: &str = "--join-mod-sync";
/// Game argument: this launch joins the host with this key, so its remote mods may load.
pub const REMOTE_ARG: &str = "--remote-mods";
/// `<mods>/remote/.staging`, relative to `<mods>/remote`.
const STAGING: &str = ".staging";
const REQUEST_FILE: &str = "join-request.json";
/// Same caps as the game and the protocol (`horizon_protocol::mod_transfer`).
pub const MAX_MODS: usize = 64;
pub const MAX_MOD_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_JOIN_BYTES: u64 = 256 * 1024 * 1024;
/// M2: what one host's folder (`mods/remote/<host dir>`) may hold in total, however many joins filled it.
pub const MAX_HOST_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_HOST_MODS: usize = 64;
const MAX_REQUEST_BYTES: u64 = 1 << 20;
const MAX_TEXT: usize = 64;
const MAX_ADDRESS: usize = control::MAX_HOST_KEY;

// ---------------------------------------------------------------------------------------------------------------
// Host keys
// ---------------------------------------------------------------------------------------------------------------

/// The host key of a direct join (`--server <address>`): `server:<lowercase host>:<port>`, port 25565 when absent,
/// an IPv6 literal in brackets. **The same rule as the game's `join_mods::server_host`**; `None` for the game's
/// pseudo-addresses (`local`, `social`, `replay`, a friend invite code) and for odd characters.
pub fn server_host_key(address: &str) -> Option<String> {
    server_host(address).map(|(key, _)| key)
}

/// The key and the `host:port` the game would pass back for `--server`.
fn server_host(address: &str) -> Option<(String, String)> {
    let address = address.trim();
    if address.is_empty() || matches!(address, "local" | "social" | "replay") || address.starts_with(horizon_friends::ADDRESS_PREFIX) {
        return None;
    }
    let (host, port) = if let Some(rest) = address.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(port) => port.parse::<u16>().ok()?,
            None if tail.is_empty() => 25565,
            None => return None,
        };
        (format!("[{}]", host.to_ascii_lowercase()), port)
    } else {
        match address.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host.to_ascii_lowercase(), port.parse::<u16>().ok()?),
            Some(_) => return None,
            None => (address.to_ascii_lowercase(), 25565),
        }
    };
    let key = format!("server:{host}:{port}");
    valid_host_key(&key).then(|| (key, format!("{host}:{port}")))
}

/// The host key of a social join: `friend:<portal user id>` (the host principal of the grant).
pub fn friend_host_key(portal_user_id: &str) -> Option<String> {
    let key = format!("friend:{}", portal_user_id.to_ascii_lowercase());
    valid_host_key(&key).then_some(key)
}

fn origin_of_key(key: &str) -> Option<Origin> {
    match key.split_once(':')?.0 {
        "server" => Some(Origin::Server),
        "friend" => Some(Origin::Friend),
        _ => None,
    }
}

/// The key a folder name stands for (`server+host+25565` -> `server:host:25565`), when it round-trips.
fn key_of_dir_name(name: &str) -> Option<String> {
    let key = name.replace('+', ":");
    (valid_host_key(&key) && host_dir_name(&key) == name).then_some(key)
}

// ---------------------------------------------------------------------------------------------------------------
// The request
// ---------------------------------------------------------------------------------------------------------------

/// One mod the request names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestMod {
    pub id: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
    pub full_access: bool,
    pub catalog_id: Option<String>,
    /// Always `<sha256>.yak` (validated), a file of the staging folder.
    pub file: String,
}

/// What a host is, in a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestHost {
    pub kind: Origin,
    /// `host:port` for a server, the portal user id for a friend.
    pub address: String,
    pub key: String,
}

/// `join-request.json`, validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRequest {
    pub host: RequestHost,
    pub mods: Vec<RequestMod>,
    pub rejoin: Rejoin,
}

/// How to join the same host again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejoin {
    /// `--server <address>`.
    Server(String),
    /// The social join flow with a fresh grant for this host principal (portal user id).
    Friend(String),
}

impl Rejoin {
    /// The game's own arguments for the join, when it needs no grant: `--server <address>`.
    pub fn quick_play(&self) -> Option<(&'static str, String)> {
        match self {
            Rejoin::Server(address) => Some(("--server", address.clone())),
            Rejoin::Friend(_) => None,
        }
    }
}

fn is_hash(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn plain(text: &str) -> bool {
    !text.is_empty() && text.chars().count() <= MAX_TEXT && !text.chars().any(char::is_control)
}

/// Parses and checks `join-request.json`: field types, the host (kind, address, key and rejoin agree and are plain
/// host-key characters), 1..=[`MAX_MODS`] mods, each with plain id and version, a lowercase 64-digit SHA-256, a size
/// of 1..=[`MAX_MOD_BYTES`], a file name equal to `<sha256>.yak`, no duplicate id or hash, and [`MAX_JOIN_BYTES`] in
/// total. Anything else is an error and the whole request is refused.
pub fn parse_request(text: &str) -> Result<JoinRequest, String> {
    use serde_json::Value;
    let root: Value = serde_json::from_str(text).map_err(|e| format!("join request: {e}"))?;
    let field = |value: &Value, name: &str| value.get(name).and_then(Value::as_str).map(str::to_owned);
    let host = root.get("host").filter(|h| h.is_object()).ok_or("join request: no host")?;
    let kind = match field(host, "kind").as_deref() {
        Some("server") => Origin::Server,
        Some("friend") => Origin::Friend,
        _ => return Err("join request: the host kind must be server or friend".into()),
    };
    let address = field(host, "address").filter(|a| !a.is_empty() && a.len() <= MAX_ADDRESS && !a.chars().any(char::is_control)).ok_or("join request: bad host address")?;
    let key = field(host, "key").filter(|k| valid_host_key(k)).ok_or("join request: the host key is not a host key")?;
    // S2: the key must be exactly the canonical (lowercase) form recomputed from the address, never the host's own spelling
    let canonical = match kind {
        Origin::Server => server_host(&address).filter(|(_, rejoin)| *rejoin == address).map(|(key, _)| key),
        _ => friend_host_key(&address),
    };
    if canonical.as_deref() != Some(key.as_str()) {
        return Err("join request: the host key does not match its kind and address".into());
    }
    let rejoin_text = field(&root, "rejoin").ok_or("join request: no rejoin")?;
    let rejoin = match kind {
        Origin::Server => {
            // what the game passes to `--server`: the address itself
            if rejoin_text != address {
                return Err("join request: rejoin does not match the host address".into());
            }
            Rejoin::Server(rejoin_text)
        }
        _ => {
            if rejoin_text != key {
                return Err("join request: rejoin does not match the host key".into());
            }
            Rejoin::Friend(address.clone())
        }
    };
    let list = root.get("mods").and_then(Value::as_array).ok_or("join request: no mods")?;
    if list.is_empty() || list.len() > MAX_MODS {
        return Err(format!("join request: {} mods (1 to {MAX_MODS} allowed)", list.len()));
    }
    let mut mods: Vec<RequestMod> = Vec::new();
    let mut total = 0u64;
    for (n, item) in list.iter().enumerate() {
        let bad = |what: &str| format!("join request: mods[{n}]: {what}");
        let id = field(item, "id").filter(|t| plain(t)).ok_or_else(|| bad("bad id"))?;
        let version = field(item, "version").filter(|t| plain(t)).ok_or_else(|| bad("bad version"))?;
        let sha256 = field(item, "sha256").filter(|t| is_hash(t)).ok_or_else(|| bad("sha256 must be 64 lowercase hex digits"))?;
        let size = item.get("size").and_then(Value::as_u64).filter(|s| (1..=MAX_MOD_BYTES).contains(s)).ok_or_else(|| bad("size must be 1 to 64 MiB"))?;
        let full_access = item.get("full_access").and_then(Value::as_bool).ok_or_else(|| bad("full_access must be true or false"))?;
        let file = field(item, "file").ok_or_else(|| bad("no file"))?;
        if file != format!("{sha256}.yak") {
            return Err(bad("the file must be named <sha256>.yak"));
        }
        let catalog_id = match item.get("catalog_id") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_str().filter(|t| plain(t)).ok_or_else(|| bad("bad catalog_id"))?.to_owned()),
        };
        if mods.iter().any(|m| m.id == id || m.sha256 == sha256) {
            return Err(bad("duplicate id or hash"));
        }
        total += size;
        if total > MAX_JOIN_BYTES {
            return Err("join request: more than 256 MiB in total".into());
        }
        mods.push(RequestMod { id, version, sha256, size, full_access, catalog_id, file });
    }
    Ok(JoinRequest { host: RequestHost { kind, address, key }, mods, rejoin })
}

/// The hook of the catalog-first path (JOIN-MODS.md, "Catalog first"): bytes of `entry` taken from the signed Yoake
/// catalog, verified there by id and SHA-256 — **only** when the catalog really has it. No host sends `catalog_id`
/// yet, so this is not connected and returns `None`; the caller then uses the staged file (hash-pinned either way).
/// Never make this guess: a wrong answer here would bypass the hash the host announced.
fn catalog_bytes(_entry: &RequestMod) -> Option<Vec<u8>> {
    None
}

// ---------------------------------------------------------------------------------------------------------------
// Folders
// ---------------------------------------------------------------------------------------------------------------

fn remote_root(instance: &Path) -> PathBuf {
    mods_dir(instance).join("remote")
}

fn staging_dir(instance: &Path) -> PathBuf {
    remote_root(instance).join(STAGING)
}

/// A real folder, not a link: the staging folder is where a hostile host's bytes land, so it is checked before use.
fn real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        && match (std::fs::canonicalize(path), path.parent().map(std::fs::canonicalize)) {
            (Ok(path), Some(Ok(parent))) => path.parent() == Some(parent.as_path()),
            _ => false,
        }
}

/// Empties the staging folder (files and folders in it; links are removed, not followed).
fn clear_staging(instance: &Path) {
    let staging = staging_dir(instance);
    let Ok(meta) = std::fs::symlink_metadata(&staging) else { return };
    if meta.file_type().is_symlink() || !meta.is_dir() {
        let _ = std::fs::remove_file(&staging).or_else(|_| std::fs::remove_dir(&staging));
        return;
    }
    let Ok(entries) = std::fs::read_dir(&staging) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink());
        let _ = if is_dir { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path).or_else(|_| std::fs::remove_dir(&path)) };
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The per-host record
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct RecordedMod {
    id: String,
    version: String,
    sha256: String,
    full_access: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct HostRecord {
    host: String,
    display: String,
    mods: Vec<RecordedMod>,
}

fn record_path(control: &Control, instance: &Path, key: &str) -> PathBuf {
    let token = last_path(control, instance).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    control.dir().join("remote-hosts").join(token).join(format!("{}.json", host_dir_name(key)))
}

fn read_record(control: &Control, instance: &Path, key: &str) -> HostRecord {
    let path = record_path(control, instance, key);
    let text = std::fs::File::open(&path).ok().and_then(|f| {
        let mut text = String::new();
        f.take(MAX_REQUEST_BYTES).read_to_string(&mut text).ok().map(|_| text)
    });
    text.and_then(|t| serde_json::from_str::<HostRecord>(&t).ok()).filter(|r| r.host == key).unwrap_or_default()
}

fn write_record(control: &Control, instance: &Path, key: &str, record: &HostRecord) -> Result<(), String> {
    let path = record_path(control, instance, key);
    std::fs::create_dir_all(path.parent().ok_or("no folder")?).map_err(|e| e.to_string())?;
    control::write_atomic(&path, serde_json::to_string_pretty(record).map_err(|e| e.to_string())?.as_bytes())
}

/// The approved-list entries of the host `key` for a launch: its recorded **full-access** mods whose file is still in
/// the host's folder and still hashes to the record (the file is read and hashed). Origin `server` or `friend`.
pub(super) fn host_entries(control: &Control, instance: &Path, key: &str) -> Vec<ApprovedEntry> {
    let (Some(origin), Some(dir)) = (origin_of_key(key), host_remote_dir(&mods_dir(instance), key)) else { return Vec::new() };
    let record = read_record(control, instance, key);
    let mut entries = Vec::new();
    for item in record.mods.iter().filter(|m| m.full_access && is_hash(&m.sha256)) {
        let path = dir.join(format!("{}.yak", item.sha256));
        let intact = std::fs::symlink_metadata(&path).is_ok_and(|m| m.is_file() && m.len() <= MAX_MOD_BYTES)
            && std::fs::read(&path).is_ok_and(|bytes| control::sha256_hex(&bytes) == item.sha256);
        if intact && !entries.iter().any(|e: &ApprovedEntry| e.id == item.id) {
            entries.push(ApprovedEntry { id: item.id.clone(), version: item.version.clone(), sha256: item.sha256.clone(), origin });
        }
    }
    entries
}

/// Whether the host `key` has a remote folder in this instance (the condition for `--remote-mods`).
pub(super) fn host_has_folder(instance: &Path, key: &str) -> bool {
    host_remote_dir(&mods_dir(instance), key).is_some_and(|dir| real_dir(&dir))
}

/// A launch's list with the host's scope: `base` (the player's own accepted entries) plus the host's full-access mods,
/// scoped with `with_host(key)`.
pub(super) fn scoped_list(control: &Control, instance: &Path, key: &str, base: ApprovedList) -> ApprovedList {
    let mut entries = base.entries().to_vec();
    entries.extend(host_entries(control, instance, key));
    ApprovedList::new(entries, base.protected().to_vec()).with_host(key)
}

// ---------------------------------------------------------------------------------------------------------------
// Handling exit code 75
// ---------------------------------------------------------------------------------------------------------------

/// What a successful sync did; the launcher relaunches with [`Rejoin`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinSyncDone {
    pub host_key: String,
    pub host_display: String,
    pub installed: usize,
    pub rejoin: Rejoin,
}

/// The last sync of an instance, for the Mods page / the Play page's notice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinSyncStatus {
    pub host_display: String,
    pub installed: usize,
    pub error: Option<String>,
}

static LAST: Mutex<Option<(PathBuf, JoinSyncStatus)>> = Mutex::new(None);
static LAST_BY_INSTANCE: Mutex<Option<HashMap<PathBuf, JoinSyncStatus>>> = Mutex::new(None);

fn note(instance: &Path, status: JoinSyncStatus) {
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((instance.to_path_buf(), status.clone()));
    LAST_BY_INSTANCE.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(instance.to_path_buf(), status);
}

/// The most recent join sync of any instance in this launcher run, or `None`.
pub fn last_join_sync() -> Option<JoinSyncStatus> {
    LAST.lock().unwrap_or_else(|e| e.into_inner()).as_ref().map(|(_, status)| status.clone())
}

/// The most recent join sync of `instance`.
pub fn last_join_sync_for(instance: &Path) -> Option<JoinSyncStatus> {
    LAST_BY_INSTANCE.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|map| map.get(instance).cloned())
}

/// Whether the text a finished game process gave the launcher (`Msg::Exit`'s error, "Exited with exit code: 75" on
/// Windows, "Exited with exit status: 75" elsewhere) is the join-sync exit code.
pub fn is_exit_text(text: &str) -> bool {
    text.strip_prefix("Exited with ").is_some_and(|rest| rest.ends_with(&format!(": {EXIT_CODE}")))
}

/// **A Yoake game the launcher started exited with 75.** Reads `<instance>/mods/remote/.staging/join-request.json`,
/// validates it, installs and records what it names, and empties the staging folder, whatever the outcome. The status
/// ([`last_join_sync`]) is set either way. Holds the control lock: call it from a worker thread.
pub fn handle_exit(instance: &Path) -> Result<JoinSyncDone, String> {
    handle_exit_in(&crate::backend::data_dir(), instance)
}

pub fn handle_exit_in(data: &Path, instance: &Path) -> Result<JoinSyncDone, String> {
    let _lock = lock();
    let (host, result) = apply(data, instance);
    clear_staging(instance);
    note(
        instance,
        JoinSyncStatus {
            host_display: host.unwrap_or_else(|| "(unknown host)".to_owned()),
            installed: result.as_ref().map_or(0, |done| done.installed),
            error: result.as_ref().err().cloned(),
        },
    );
    result
}

/// M2: a game that **the launcher itself restarted after a join sync** exited with 75 again. At most one automatic
/// rejoin follows a join the player started, so a host that keeps asking (a loop that would add mods every round) is
/// stopped here: nothing is read from the request or installed, the staging folder is emptied, and the status says why.
pub fn refuse_repeated_exit(instance: &Path) -> Result<JoinSyncDone, String> {
    let _lock = lock();
    let host = read_request(&staging_dir(instance)).ok().map(|request| request.host.address);
    clear_staging(instance);
    let error = format!(
        "{} asked for mods again right after they were installed, so the join was stopped (a host may only make the launcher restart the game once per join). Nothing more was installed. Join again yourself if you trust this host.",
        host.as_deref().unwrap_or("the host")
    );
    note(instance, JoinSyncStatus { host_display: host.unwrap_or_else(|| "(unknown host)".to_owned()), installed: 0, error: Some(error.clone()) });
    Err(error)
}

/// Returns the host's display text when the request got that far, and the outcome.
fn apply(data: &Path, instance: &Path) -> (Option<String>, Result<JoinSyncDone, String>) {
    let staging = staging_dir(instance);
    if !real_dir(&staging) {
        return (None, Err("the game left no staging folder for its join request".into()));
    }
    let request = match read_request(&staging) {
        Ok(request) => request,
        Err(e) => return (None, Err(e)),
    };
    let display = request.host.address.clone();
    let result = install_request(data, instance, &staging, &request);
    (Some(display), result)
}

fn read_request(staging: &Path) -> Result<JoinRequest, String> {
    let path = staging.join(REQUEST_FILE);
    let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > MAX_REQUEST_BYTES {
        return Err(format!("{} is not a regular file of at most 1 MiB", path.display()));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_request(&text)
}

/// One verified mod, ready to install.
struct Checked {
    entry: RequestMod,
    bytes: Vec<u8>,
}

/// Takes the file of `entry` from the staging folder (after [`catalog_bytes`]) and proves it is what the request says.
fn verify(staging: &Path, entry: &RequestMod) -> Result<Vec<u8>, String> {
    let bytes = match catalog_bytes(entry) {
        Some(bytes) => bytes,
        None => {
            let path = staging.join(&entry.file);
            let meta = std::fs::symlink_metadata(&path).map_err(|e| format!("{}: {e}", entry.file))?;
            if !meta.is_file() || meta.file_type().is_symlink() {
                return Err(format!("{} is not a regular file", entry.file));
            }
            if meta.len() != entry.size || meta.len() > MAX_MOD_BYTES {
                return Err(format!("{} is {} bytes, the request says {}", entry.file, meta.len(), entry.size));
            }
            let mut bytes = Vec::with_capacity(entry.size as usize);
            std::fs::File::open(&path).map_err(|e| format!("{}: {e}", entry.file))?.take(MAX_MOD_BYTES + 1).read_to_end(&mut bytes).map_err(|e| format!("{}: {e}", entry.file))?;
            bytes
        }
    };
    if bytes.len() as u64 != entry.size {
        return Err(format!("{} does not have the announced size", entry.id));
    }
    if control::sha256_hex(&bytes) != entry.sha256 {
        return Err(format!("{} does not match the SHA-256 the host announced", entry.id));
    }
    // the package itself: valid, and what the request says it is
    let archive = Archive::new(Cursor::new(bytes.as_slice())).map_err(|e| format!("{}: {e}", entry.id))?;
    let manifest = archive.manifest();
    if manifest.id != entry.id || manifest.version.to_string() != entry.version {
        return Err(format!("{} is not the mod {} {} the request names", entry.file, entry.id, entry.version));
    }
    if super::is_full_access(manifest) != entry.full_access {
        return Err(format!("{}: the request's full_access flag is not the package's", entry.id));
    }
    Ok(bytes)
}

fn install_request(data: &Path, instance: &Path, staging: &Path, request: &JoinRequest) -> Result<JoinSyncDone, String> {
    let key = &request.host.key;
    let mods = mods_dir(instance);
    let dir = host_remote_dir(&mods, key).ok_or_else(|| format!("not a host key: {key:?}"))?;
    let control = open_control(data, instance)?;
    // 1. everything is read and proven before anything is installed
    let mut checked = Vec::new();
    for entry in &request.mods {
        checked.push(Checked { bytes: verify(staging, entry)?, entry: entry.clone() });
    }
    // 1b. M2: the host's folder stays within its caps once this request is in (nothing from the request is installed
    // otherwise). Files of the same hash are already there; files this request replaces (same id, other bytes) go away.
    {
        let record = read_record(&control, instance, key);
        let request_hashes: Vec<&str> = request.mods.iter().map(|m| m.sha256.as_str()).collect();
        let replaced_hashes: Vec<&str> = record
            .mods
            .iter()
            .filter(|old| request.mods.iter().any(|m| m.id == old.id) && !request_hashes.contains(&old.sha256.as_str()))
            .map(|old| old.sha256.as_str())
            .collect();
        let (mut count, mut bytes) = (0usize, 0u64);
        let mut present: Vec<String> = Vec::new();
        if let Ok(read) = std::fs::read_dir(&dir) {
            for item in read.flatten() {
                let name = item.file_name().to_string_lossy().into_owned();
                let Some(hash) = name.strip_suffix(".yak") else { continue };
                if replaced_hashes.contains(&hash) {
                    continue;
                }
                count += 1;
                bytes += item.metadata().map_or(0, |m| m.len());
                present.push(hash.to_owned());
            }
        }
        for entry in &request.mods {
            if !present.contains(&entry.sha256) {
                count += 1;
                bytes += entry.size;
            }
        }
        if count > MAX_HOST_MODS || bytes > MAX_HOST_BYTES {
            return Err(format!(
                "this host's mods would be {count} files and {} MiB, over the limit of {MAX_HOST_MODS} files and {} MiB per host; nothing was installed (remove its mods on the Mods page first)",
                bytes / (1024 * 1024),
                MAX_HOST_BYTES / (1024 * 1024)
            ));
        }
    }
    // 2. install; a failure takes back what this request added
    let folder_was_there = dir.exists();
    let mut created: Vec<PathBuf> = Vec::new();
    let mut outcome = Ok(());
    for item in &checked {
        let target = dir.join(format!("{}.yak", item.entry.sha256));
        let existed = target.exists();
        match install_remote_package(&mods, key, &item.bytes, &item.entry.sha256) {
            Ok(path) => {
                if !existed {
                    created.push(path);
                }
            }
            Err(e) => {
                outcome = Err(format!("{}: {e}", item.entry.id));
                break;
            }
        }
    }
    // 3. the record: the approval of this host's full-access mods (for this host only), and drop a replaced version
    let mut replaced: Vec<PathBuf> = Vec::new();
    if outcome.is_ok() {
        let mut record = read_record(&control, instance, key);
        record.host = key.clone();
        record.display = request.host.address.clone();
        for item in &checked {
            for old in record.mods.iter().filter(|m| m.id == item.entry.id && m.sha256 != item.entry.sha256 && !checked.iter().any(|other| other.entry.sha256 == m.sha256)) {
                replaced.push(dir.join(format!("{}.yak", old.sha256)));
            }
            record.mods.retain(|m| m.id != item.entry.id);
            record.mods.push(RecordedMod { id: item.entry.id.clone(), version: item.entry.version.clone(), sha256: item.entry.sha256.clone(), full_access: item.entry.full_access });
        }
        outcome = write_record(&control, instance, key, &record);
    }
    if let Err(e) = outcome {
        for path in created {
            let _ = std::fs::remove_file(path);
        }
        if !folder_was_there {
            let _ = std::fs::remove_dir(&dir);
        }
        return Err(e);
    }
    for path in replaced {
        let _ = std::fs::remove_file(path);
    }
    Ok(JoinSyncDone { host_key: key.clone(), host_display: request.host.address.clone(), installed: checked.len(), rejoin: request.rejoin.clone() })
}

// ---------------------------------------------------------------------------------------------------------------
// Listing and removing a host's mods
// ---------------------------------------------------------------------------------------------------------------

/// One mod fetched from a host.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteMod {
    pub id: String,
    pub version: String,
    /// First 12 hex digits of the file's SHA-256.
    pub sha256_short: String,
    pub full_access: bool,
}

/// A host the instance fetched mods from ("from <host>").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteHost {
    /// `server:<host>:<port>` / `friend:<portal user id>`: what [`remove_remote_host`] takes.
    pub key: String,
    /// What to show: the server's address, or the friend's portal user id (the page may swap in a friend's name).
    pub display: String,
    pub mods: Vec<RemoteMod>,
}

/// The hosts `instance` has remote mods from, with their mods, sorted by key.
pub fn remote_hosts(instance: &Path) -> Vec<RemoteHost> {
    remote_hosts_in(&crate::backend::data_dir(), instance)
}

pub fn remote_hosts_in(data: &Path, instance: &Path) -> Vec<RemoteHost> {
    let _lock = lock();
    let control = open_control(data, instance).ok();
    let Ok(read) = std::fs::read_dir(remote_root(instance)) else { return Vec::new() };
    let mut hosts = Vec::new();
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(key) = key_of_dir_name(&name) else { continue };
        if !entry.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
            continue;
        }
        let record = control.as_ref().map(|c| read_record(c, instance, &key)).unwrap_or_default();
        let mut mods = Vec::new();
        let mut files: Vec<PathBuf> = std::fs::read_dir(entry.path()).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "yak") && p.is_file()).collect()).unwrap_or_default();
        files.sort();
        for path in files {
            let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(known) = record.mods.iter().find(|m| m.sha256 == stem) {
                mods.push(RemoteMod { id: known.id.clone(), version: known.version.clone(), sha256_short: stem.chars().take(12).collect(), full_access: known.full_access });
                continue;
            }
            // not in the record (hand-dropped, or the record is gone): read what it is; it is not approved either way
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let sha = control::sha256_hex(&bytes);
            let (id, version, full_access) = match Archive::new(Cursor::new(bytes)) {
                Ok(archive) => (archive.manifest().id.clone(), archive.manifest().version.to_string(), super::is_full_access(archive.manifest())),
                Err(_) => (stem.clone(), String::new(), false),
            };
            mods.push(RemoteMod { id, version, sha256_short: sha.chars().take(12).collect(), full_access });
        }
        mods.sort_by(|a, b| a.id.cmp(&b.id));
        let display = if record.display.is_empty() { key.split_once(':').map(|(_, rest)| rest.to_owned()).unwrap_or_else(|| key.clone()) } else { record.display.clone() };
        hosts.push(RemoteHost { key, display, mods });
    }
    hosts.sort_by(|a, b| a.key.cmp(&b.key));
    hosts
}

/// "Remove all from this host": deletes the host's folder, its record (what the launcher knew about where those mods
/// came from) and the approvals with it. `Err` for a key that is not a host key or a host the instance has nothing from.
pub fn remove_remote_host(instance: &Path, key: &str) -> Result<(), String> {
    remove_remote_host_in(&crate::backend::data_dir(), instance, key)
}

pub fn remove_remote_host_in(data: &Path, instance: &Path, key: &str) -> Result<(), String> {
    let _lock = lock();
    let dir = host_remote_dir(&mods_dir(instance), key).ok_or_else(|| format!("not a host key: {key:?}"))?;
    let control = open_control(data, instance)?;
    let record = record_path(&control, instance, key);
    let meta = std::fs::symlink_metadata(&dir);
    if meta.is_err() && !record.exists() {
        return Err(format!("no mods from {key} in this instance"));
    }
    // the record (and with it the approval) goes first: whatever happens to the folder, nothing is passed any more
    match std::fs::remove_file(&record) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("{}: {e}", record.display())),
    }
    if let Ok(meta) = meta {
        let result = if meta.file_type().is_symlink() || !meta.is_dir() {
            // never follow a link out of the folder: only the link goes
            std::fs::remove_file(&dir).or_else(|_| std::fs::remove_dir(&dir))
        } else {
            std::fs::remove_dir_all(&dir)
        };
        result.map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    Ok(())
}
