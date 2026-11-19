//! MX3JN1: join-time mod sync, the game's side (docs/yoake2/JOIN-MODS.md).
//!
//! A host that runs `.yak` mods a joining player lacks offers them on `yoake:mod_transfer`
//! (`horizon_protocol::mod_transfer`). With the launcher's `--join-mod-sync` flag the joining game fetches the
//! missing files into `<game dir>/mods/remote/.staging/<sha256>.part`, verifies size and SHA-256, renames them to
//! `.yak`, writes `join-request.json` and quits with exit code [`EXIT_JOIN_NEEDS_MODS`]. **The game installs
//! nothing and approves nothing**: the launcher reads the request, installs the files under
//! `mods/remote/<host dir>/` and relaunches with `--remote-mods <host key>` (and an approved list for full-access
//! mods). Without the flag nothing here runs and a mismatch behaves as before.
//!
//! This module holds the parts that do not need a connection (arguments, host identity, the staging sink, the request
//! file, the host's file source and offer, what the player already has), so they run in tests without a game.
use horizon_mods::yak::Archive;
use horizon_protocol::mod_transfer::{self as transfer, MAX_MOD_BYTES, Offer, OfferEntry, Sink, Source};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::File,
    io::{Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::Mutex,
    time::SystemTime,
};

/// The game's exit code that says "I fetched mods for this host; install them and start me again".
pub const EXIT_JOIN_NEEDS_MODS: i32 = 75;
/// Launcher flag: fetch the host's missing mods when joining.
pub const SYNC_ARG: &str = "--join-mod-sync";
/// Launcher option: this launch joins the host with this key, so its remote mods may load.
pub const REMOTE_ARG: &str = "--remote-mods";
/// `<game dir>/mods/remote`.
pub const REMOTE_DIR: &str = "remote";
/// `<game dir>/mods/remote/.staging`.
pub const STAGING_DIR: &str = ".staging";
/// The request the launcher reads, in the staging folder.
pub const REQUEST_FILE: &str = "join-request.json";

/// Whether `--join-mod-sync` is among `args`.
pub fn sync_enabled_from<I: IntoIterator<Item = String>>(args: I) -> bool {
    args.into_iter().any(|argument| argument == SYNC_ARG)
}

/// The value of `--remote-mods <key>` (or `--remote-mods=<key>`), when it is a valid host key.
pub fn remote_key_from<I: IntoIterator<Item = String>>(args: I) -> Option<String> {
    let args: Vec<String> = args.into_iter().collect();
    for (index, argument) in args.iter().enumerate() {
        let value = if argument == REMOTE_ARG {
            args.get(index + 1).cloned()
        } else {
            argument.strip_prefix(&format!("{REMOTE_ARG}=")).map(str::to_owned)
        };
        if let Some(value) = value {
            return valid_host_key(&value).then_some(value);
        }
    }
    None
}

/// This process's `--join-mod-sync`.
pub fn sync_enabled() -> bool {
    sync_enabled_from(std::env::args().skip(1))
}

/// This process's `--remote-mods`.
pub fn remote_key() -> Option<String> {
    remote_key_from(std::env::args().skip(1))
}

pub use horizon_mods::yak::control::{MAX_HOST_KEY as MAX_KEY, host_dir_name, valid_host_key};

/// `<game dir>/mods/remote/<host dir>` (see `control::host_dir_name`).
pub fn host_dir(game_root: &Path, key: &str) -> PathBuf {
    game_root.join("mods").join(REMOTE_DIR).join(host_dir_name(key))
}

/// `<game dir>/mods/remote/.staging`.
pub fn staging_dir(game_root: &Path) -> PathBuf {
    game_root.join("mods").join(REMOTE_DIR).join(STAGING_DIR)
}

// ---------------------------------------------------------------------------------------------------------------
// Which host this join is
// ---------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostKind {
    Server,
    Friend,
}

impl HostKind {
    fn as_str(self) -> &'static str {
        match self {
            HostKind::Server => "server",
            HostKind::Friend => "friend",
        }
    }
}

/// The host this connection goes to, as the request file names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostId {
    pub kind: HostKind,
    /// `host:port` for a server, the portal user id for a friend.
    pub address: String,
    pub key: String,
    /// What the launcher must pass to join again: `host:port` (the value of `--server`) for a server,
    /// `friend:<portal user id>` for a friend (the launcher makes a fresh social grant itself).
    pub rejoin: String,
}

/// A server address as typed (`host`, `host:port`, `[v6]:port`) as a host identity. None for the game's own
/// pseudo-addresses (`local`, `social`, a friend invite code) and for anything with unusual characters.
pub fn server_host(address: &str) -> Option<HostId> {
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
    valid_host_key(&key).then(|| HostId { kind: HostKind::Server, address: format!("{host}:{port}"), key, rejoin: format!("{host}:{port}") })
}

/// A social/relay host by its portal user id.
pub fn friend_host(portal_user_id: &str) -> Option<HostId> {
    let key = format!("friend:{portal_user_id}");
    valid_host_key(&key).then(|| HostId { kind: HostKind::Friend, address: portal_user_id.to_owned(), rejoin: key.clone(), key })
}

static HOST: Mutex<Option<HostId>> = Mutex::new(None);

/// Names the host the next connection goes to (the game sets it before it connects; None for single player and for
/// connections that have no stable host identity).
pub fn set_host(host: Option<HostId>) {
    *HOST.lock().unwrap_or_else(|e| e.into_inner()) = host;
}

pub fn host() -> Option<HostId> {
    HOST.lock().unwrap_or_else(|e| e.into_inner()).clone()
}

// ---------------------------------------------------------------------------------------------------------------
// The client: staging and the request
// ---------------------------------------------------------------------------------------------------------------

/// Writes fetched bytes to `<staging>/<sha256>.part` and renames them to `<sha256>.yak` once verified. File names
/// are hex of the hash, never anything the host sent.
pub struct StagingSink {
    dir: PathBuf,
    current: Option<(File, PathBuf)>,
}

impl StagingSink {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, current: None }
    }

    fn names(&self, entry: &OfferEntry) -> (PathBuf, PathBuf) {
        let hash = transfer::hex(&entry.sha256);
        (self.dir.join(format!("{hash}.part")), self.dir.join(format!("{hash}.yak")))
    }
}

impl Sink for StagingSink {
    fn begin(&mut self, entry: &OfferEntry) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let (part, _) = self.names(entry);
        let _ = std::fs::remove_file(&part);
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&part)?;
        self.current = Some((file, part));
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        match &mut self.current {
            Some((file, _)) => file.write_all(data),
            None => Err(std::io::Error::other("no file in progress")),
        }
    }

    fn commit(&mut self, entry: &OfferEntry) -> std::io::Result<()> {
        let (file, part) = self.current.take().ok_or_else(|| std::io::Error::other("no file in progress"))?;
        file.sync_all()?;
        drop(file);
        // CHECK: the size was verified by the protocol (sum of chunks == offered size); the file on disk says the same.
        if std::fs::metadata(&part)?.len() != entry.size {
            let _ = std::fs::remove_file(&part);
            return Err(std::io::Error::other("size differs"));
        }
        let (_, done) = self.names(entry);
        let _ = std::fs::remove_file(&done);
        std::fs::rename(&part, &done)
    }

    fn discard(&mut self, entry: &OfferEntry) {
        self.current = None;
        let (part, _) = self.names(entry);
        let _ = std::fs::remove_file(part);
    }
}

/// The request as the contract has it: `{ host: {kind, address, key}, mods: [ {id, version, sha256, size,
/// full_access, catalog_id?, file} ], rejoin }`.
pub fn request_json(host: &HostId, mods: &[OfferEntry]) -> serde_json::Value {
    let mods: Vec<serde_json::Value> = mods
        .iter()
        .map(|entry| {
            let hash = transfer::hex(&entry.sha256);
            let mut object = serde_json::json!({
                "id": entry.id,
                "version": entry.version,
                "sha256": hash,
                "size": entry.size,
                "full_access": entry.full_access,
                "file": format!("{hash}.yak"),
            });
            if let Some(catalog) = &entry.catalog_id {
                object["catalog_id"] = serde_json::Value::String(catalog.clone());
            }
            object
        })
        .collect();
    serde_json::json!({
        "host": { "kind": host.kind.as_str(), "address": host.address, "key": host.key },
        "mods": mods,
        "rejoin": host.rejoin,
    })
}

/// Writes `join-request.json` into the staging folder (temp file, then rename, so the launcher never reads half of it).
pub fn write_request(staging: &Path, host: &HostId, mods: &[OfferEntry]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(staging)?;
    let path = staging.join(REQUEST_FILE);
    let temp = staging.join(format!(".{REQUEST_FILE}.tmp"));
    let text = serde_json::to_string_pretty(&request_json(host, mods)).map_err(std::io::Error::other)?;
    {
        let mut file = File::create(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    let _ = std::fs::remove_file(&path);
    std::fs::rename(&temp, &path)?;
    Ok(path)
}

/// Removes a stale request (a failed or abandoned join must leave none behind).
pub fn clear_request(staging: &Path) {
    let _ = std::fs::remove_file(staging.join(REQUEST_FILE));
}

/// What the game does once every file is staged: the request file, then the exit code. Pure apart from the file, so
/// a test can check both without ending the process.
pub fn finish_join(staging: &Path, host: &HostId, staged: &[OfferEntry]) -> Result<i32, String> {
    write_request(staging, host, staged).map_err(|error| format!("could not write {REQUEST_FILE}: {error}"))?;
    Ok(EXIT_JOIN_NEEDS_MODS)
}

/// Ends the process with the exit code (not testable on purpose; [`finish_join`] is).
pub fn exit_with(code: i32) -> ! {
    std::process::exit(code)
}

/// The exit path of a finished sync, for the net thread: write the request, then quit with 75. A failure to write
/// the request returns the message instead (the join is then refused with it).
pub fn complete_and_exit(game_root: &Path, host: &HostId, staged: &[OfferEntry]) -> String {
    match finish_join(&staging_dir(game_root), host, staged) {
        Ok(code) => exit_with(code),
        Err(message) => message,
    }
}

/// Whether the player already has this exact mod: a `.yak` of that SHA-256 in `mods/` or in this host's remote
/// folder, or (for a mod that is not full-access) an installed package of the same id and version. Sizes are compared
/// before anything is hashed.
pub fn has_local(game_root: &Path, host_key: Option<&str>, installed: &[(String, String)], entry: &OfferEntry) -> bool {
    let mut folders = vec![game_root.join("mods")];
    if let Some(key) = host_key.filter(|key| valid_host_key(key)) {
        folders.push(host_dir(game_root, key));
    }
    for folder in folders {
        let Ok(read) = std::fs::read_dir(&folder) else { continue };
        for item in read.flatten() {
            let path = item.path();
            if path.extension().is_none_or(|extension| extension != "yak") || !item.metadata().is_ok_and(|meta| meta.is_file() && meta.len() == entry.size) {
                continue;
            }
            if hash_file(&path).is_ok_and(|hash| hash == entry.sha256) {
                return true;
            }
        }
    }
    !entry.full_access && installed.iter().any(|(id, version)| *id == entry.id && *version == entry.version)
}

fn hash_file(path: &Path) -> std::io::Result<[u8; 32]> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buf)?;
        if read == 0 {
            return Ok(hasher.finalize().into());
        }
        hasher.update(&buf[..read]);
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The host: what it offers and where the bytes come from
// ---------------------------------------------------------------------------------------------------------------

/// One `.yak` of the host's `mods/` folder, read once and remembered by (length, modified time).
#[derive(Clone)]
struct Scanned {
    sha256: [u8; 32],
    size: u64,
    id: String,
    version: String,
    full_access: bool,
    /// A client module exists (a full-access `wasm.client`) or the manifest says the client runs it.
    side_client: bool,
    required_on_join: bool,
}

type ScanKey = (u64, Option<SystemTime>);

static SCANNED: Mutex<Option<HashMap<PathBuf, (ScanKey, Option<Scanned>)>>> = Mutex::new(None);

fn scan_file(path: &Path, size: u64) -> Option<Scanned> {
    if size == 0 || size > MAX_MOD_BYTES {
        log::warn!("join mods: {} is {size} bytes, not offered (1..={MAX_MOD_BYTES})", path.display());
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path).ok()?.take(MAX_MOD_BYTES + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 != size {
        return None;
    }
    let sha256: [u8; 32] = Sha256::digest(&bytes).into();
    let archive = Archive::new(Cursor::new(bytes)).ok()?;
    let manifest = archive.manifest();
    let wasm = manifest.wasm.as_ref();
    let full_access = wasm.is_some_and(|wasm| wasm.full_access());
    Some(Scanned {
        sha256,
        size,
        id: manifest.id.clone(),
        version: manifest.version.to_string(),
        full_access,
        side_client: manifest.side.client || wasm.is_some_and(|wasm| wasm.client.is_some()),
        required_on_join: manifest.side.required_on_join,
    })
}

/// What a host runs now, for [`offer_from`]: the SHA-256s of its loaded full-access mods, and the (id, version) of
/// its installed sandboxed packages.
pub struct Running<'a> {
    pub full_access_hashes: &'a dyn Fn(&str) -> bool,
    pub installed: &'a [(String, String)],
}

/// The host's offer: every top-level `mods/*.yak` that is running here and that a joining client needs (`required_on_join`,
/// or full-access with a client module). A file the host does not run is never offered, and no other path is read.
/// The files' hashes are remembered by length and modified time, so a join after the first costs a directory listing.
/// `None` when there is nothing to offer, or when the list would break a cap (the host then behaves as before).
pub fn offer_from(game_root: &Path, running: &Running<'_>) -> Option<(Offer, FileSource)> {
    let read = std::fs::read_dir(game_root.join("mods")).ok()?;
    let mut paths: Vec<PathBuf> = read.flatten().map(|item| item.path()).filter(|path| path.extension().is_some_and(|extension| extension == "yak")).collect();
    paths.sort();
    let mut entries = Vec::new();
    let mut files = HashMap::new();
    let mut cache = SCANNED.lock().unwrap_or_else(|e| e.into_inner());
    let cache = cache.get_or_insert_with(HashMap::new);
    for path in paths {
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        if !meta.is_file() {
            continue;
        }
        let key: ScanKey = (meta.len(), meta.modified().ok());
        let scanned = match cache.get(&path) {
            Some((cached, scanned)) if *cached == key => scanned.clone(),
            _ => {
                let scanned = scan_file(&path, meta.len());
                cache.insert(path.clone(), (key, scanned.clone()));
                scanned
            }
        };
        let Some(scanned) = scanned else { continue };
        let hash = transfer::hex(&scanned.sha256);
        let running_here = if scanned.full_access {
            scanned.side_client && (running.full_access_hashes)(&hash)
        } else {
            scanned.required_on_join && running.installed.iter().any(|(id, version)| *id == scanned.id && *version == scanned.version)
        };
        if !running_here {
            continue;
        }
        files.insert(scanned.sha256, path);
        entries.push(OfferEntry {
            id: scanned.id,
            version: scanned.version,
            sha256: scanned.sha256,
            size: scanned.size,
            full_access: scanned.full_access,
            side_client: scanned.side_client,
            catalog_id: None,
        });
    }
    if entries.is_empty() {
        return None;
    }
    match Offer::new(entries) {
        Ok(offer) => Some((offer, FileSource { files, open: None })),
        Err(error) => {
            log::warn!("join mods: not offering the host's mods: {error}");
            None
        }
    }
}

/// The offer of this process as a host: its `mods/` folder, what its server thread runs now (full-access packages by
/// hash, installed packages by id and version). `None` when there is nothing to offer.
pub fn host_offer() -> Option<(Offer, FileSource)> {
    let installed = installed_pairs();
    let runs = |hash: &str| RUNS.get().is_some_and(|runs| runs(hash));
    offer_from(&game_root(), &Running { full_access_hashes: &runs, installed: &installed })
}

/// The host's file source: only the files of its own offer, found by SHA-256, opened and re-hashed when a transfer
/// starts (a file changed since the offer is refused), read lazily in chunks.
pub struct FileSource {
    files: HashMap<[u8; 32], PathBuf>,
    open: Option<File>,
}

impl Source for FileSource {
    fn open(&mut self, entry: &OfferEntry) -> std::io::Result<()> {
        // CHECK: a path is only ever looked up by a hash that is in the offer, never built from the request.
        let path = self.files.get(&entry.sha256).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.is_file() || meta.len() != entry.size || hash_file(path)? != entry.sha256 {
            return Err(std::io::Error::other("the file is no longer the offered one"));
        }
        self.open = Some(File::open(path)?);
        Ok(())
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let file = self.open.as_mut().ok_or_else(|| std::io::Error::other("no file open"))?;
        file.seek(SeekFrom::Start(offset))?;
        file.read(buf)
    }

    fn close(&mut self) {
        self.open = None;
    }
}

// ---------------------------------------------------------------------------------------------------------------
// The connection's half: what the net thread does with the channel
// ---------------------------------------------------------------------------------------------------------------

/// The game folder of this process: `--game-dir`, else the working directory.
pub fn game_root() -> PathBuf {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = args
        .windows(2)
        .rev()
        .find(|pair| matches!(pair[0].as_str(), "--game-dir" | "--gameDir"))
        .map(|pair| PathBuf::from(&pair[1]))
        .unwrap_or_else(|| PathBuf::from("."));
    std::path::absolute(&dir).unwrap_or(dir)
}

type InstalledFn = fn() -> Vec<(String, String)>;
type RunsFn = fn(&str) -> bool;
static INSTALLED: std::sync::OnceLock<InstalledFn> = std::sync::OnceLock::new();
static RUNS: std::sync::OnceLock<RunsFn> = std::sync::OnceLock::new();

/// The game crate tells this one (which holds the connection code) what it runs: `installed` lists the (id, version) of
/// the installed packages of this launch, `runs` whether the server thread runs the full-access package of a SHA-256
/// (lowercase hex). Without it nothing is offered and nothing counts as installed.
pub fn set_providers(installed: InstalledFn, runs: RunsFn) {
    let _ = INSTALLED.set(installed);
    let _ = RUNS.set(runs);
}

/// (id, version) of the installed `.yak` packages of this launch.
pub fn installed_pairs() -> Vec<(String, String)> {
    INSTALLED.get().map(|installed| installed()).unwrap_or_default()
}

/// What the net thread does after one payload of the channel.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reaction {
    /// Payloads to send on the channel.
    pub send: Vec<Vec<u8>>,
    /// Text for the connecting screen ("Getting mods from the host…").
    pub status: Option<String>,
    /// The join must stop with this message (a partial file is already deleted, no request was left).
    pub fatal: Option<String>,
    /// Everything is staged and the request is written: quit with this exit code.
    pub exit: Option<i32>,
}

/// One connection's client half of the exchange. It exists for every join; with no `yoake:mod_transfer` message from
/// the host it does nothing at all.
pub struct ClientJoin {
    negotiation: transfer::Negotiation,
    sync: Option<transfer::ClientSync<StagingSink>>,
    finished: bool,
    game_root: PathBuf,
    host: Option<HostId>,
    enabled: bool,
    remote_key: Option<String>,
    installed: Vec<(String, String)>,
}

impl ClientJoin {
    /// For a join of this process: the launcher's flags, the host the game set, the installed packages.
    pub fn from_process() -> Self {
        Self::new(game_root(), host(), sync_enabled(), remote_key(), installed_pairs())
    }

    pub fn new(game_root: PathBuf, host: Option<HostId>, enabled: bool, remote_key: Option<String>, installed: Vec<(String, String)>) -> Self {
        Self { negotiation: transfer::Negotiation::default(), sync: None, finished: false, game_root, host, enabled, remote_key, installed }
    }

    /// Handles one payload of the channel from the host.
    pub fn on_payload(&mut self, data: &[u8]) -> Reaction {
        // the capability exchange first; nothing else is read before the host advertised
        if self.negotiation.receive_capabilities(data) {
            return Reaction { send: vec![transfer::CAPABILITIES.to_vec()], ..Reaction::default() };
        }
        if !self.negotiation.supported() || self.finished {
            return Reaction::default();
        }
        let step = match &mut self.sync {
            Some(sync) => sync.on_message(data),
            None => {
                let staging = staging_dir(&self.game_root);
                // a request of an earlier, abandoned join must never be taken for this one
                clear_request(&staging);
                // CHECK: without a host identity (single player, an invite code) nothing is fetched.
                let enabled = self.enabled && self.host.is_some();
                let (root, key, installed) = (self.game_root.clone(), self.remote_key.clone(), self.installed.clone());
                let (sync, step) = transfer::ClientSync::start(data, enabled, |entry| has_local(&root, key.as_deref(), &installed, entry), StagingSink::new(staging));
                self.sync = Some(sync);
                step
            }
        };
        self.react(step)
    }

    fn react(&mut self, step: transfer::Step) -> Reaction {
        let mut reaction = Reaction { send: step.send, ..Reaction::default() };
        match step.state {
            transfer::SyncState::Fetching { done, total } => {
                reaction.status = Some(format!("ホストから MOD を取得中… / Getting mods from the host… ({}/{total})", done + 1));
            }
            transfer::SyncState::NothingNeeded => self.finished = true,
            transfer::SyncState::Failed(text) => {
                self.finished = true;
                clear_request(&staging_dir(&self.game_root));
                reaction.fatal = Some(format!("Could not get the host's mods: {text}"));
            }
            transfer::SyncState::Complete(staged) => {
                self.finished = true;
                // `host` is Some here: a sync only fetches with a host identity
                let host = self.host.clone().expect("a transfer ran for a known host");
                match finish_join(&staging_dir(&self.game_root), &host, &staged) {
                    Ok(code) => {
                        reaction.status = Some("ホストから MOD を取得しました / Got the host's mods".into());
                        reaction.exit = Some(code);
                    }
                    Err(message) => reaction.fatal = Some(message),
                }
            }
        }
        reaction
    }

    /// The connection ended before the transfer did: the partial file goes and no request is left.
    pub fn abort(&mut self) {
        if let Some(sync) = &mut self.sync {
            if !self.finished {
                sync.abort();
            }
        }
        if !self.finished && self.sync.is_some() {
            clear_request(&staging_dir(&self.game_root));
        }
        self.finished = true;
    }
}

impl Drop for ClientJoin {
    /// However the configuration phase ends (an error, a refusal, a lost connection), no partial file or request stays.
    fn drop(&mut self) {
        self.abort();
    }
}

#[cfg(test)]
mod tests;
