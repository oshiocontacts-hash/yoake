//! Join-time mod transfer (docs/yoake2/JOIN-MODS.md, MX3JN1): a host that runs `.yak` mods a joining player lacks
//! offers them, and the player's game fetches the exact bytes before it quits and lets the launcher install them.
//!
//! One custom payload channel, [`CHANNEL`], in the configuration phase, **only when both peers speak it**: the host
//! sends [`CAPABILITIES`] (and only when it has something to offer), a client that understands answers with the
//! same bytes. A peer that never answers (an old Horizon, a vanilla client) hears nothing else and joins as before.
//!
//! Messages (first byte = kind): `0` capabilities, `1` offer (the host's list), `2` request (a SHA-256), `3` chunk
//! (SHA-256, offset, last flag, at most [`MAX_CHUNK`] bytes), `4` refusal (SHA-256, reason), `5` done (the client
//! needs no more). The client asks for **one SHA-256 at a time**; the host serves only hashes of its own offer, from
//! bytes it reads lazily through a [`Source`], and refuses everything else. Caps: [`MAX_MOD_BYTES`] per mod,
//! [`MAX_JOIN_BYTES`] per join, [`MAX_MODS`] mods; both sides enforce them. This module touches no file: the
//! host's [`Source`] and the client's [`Sink`] are traits, so the whole exchange runs in memory.
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// The custom payload channel, both ways.
pub const CHANNEL: &str = "yoake:mod_transfer";
/// Kind 0 + protocol version 1.
pub const CAPABILITIES: &[u8] = &[KIND_CAPS, 1];
/// Bytes of file data in one chunk at most.
pub const MAX_CHUNK: usize = 256 * 1024;
/// Bytes of one mod at most.
pub const MAX_MOD_BYTES: u64 = 64 * 1024 * 1024;
/// Bytes fetched in one join at most (the sum over the mods asked for).
pub const MAX_JOIN_BYTES: u64 = 256 * 1024 * 1024;
/// Mods in an offer, and mods fetched in one join, at most.
pub const MAX_MODS: usize = 64;
/// Characters in an id, version or catalog id at most (as `mod_list::MAX_TEXT`).
pub const MAX_TEXT: usize = crate::mod_list::MAX_TEXT;

const KIND_CAPS: u8 = 0;
const KIND_LIST: u8 = 1;
const KIND_REQUEST: u8 = 2;
const KIND_CHUNK: u8 = 3;
const KIND_REFUSE: u8 = 4;
const KIND_DONE: u8 = 5;
const LIST_VERSION: u8 = 1;
const FLAG_FULL_ACCESS: u8 = 1;
const FLAG_SIDE_CLIENT: u8 = 2;
const FLAG_CATALOG: u8 = 4;

/// Lowercase hex of a SHA-256.
pub fn hex(hash: &[u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A SHA-256 from 64 lowercase hex digits.
pub fn parse_hex(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(out)
}

/// One mod a host offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferEntry {
    pub id: String,
    pub version: String,
    /// SHA-256 of the `.yak` file.
    pub sha256: [u8; 32],
    /// Bytes of the `.yak` file.
    pub size: u64,
    pub full_access: bool,
    /// The player's game runs a module of it (a client that lacks it should fetch it).
    pub side_client: bool,
    /// Its id in the signed Yoake catalog, when it came from there (the launcher may prefer the catalog).
    pub catalog_id: Option<String>,
}

fn text_ok(text: &str) -> bool {
    !text.is_empty() && text.chars().count() <= MAX_TEXT && !text.chars().any(char::is_control)
}

/// A host's list of mods, within the caps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    entries: Vec<OfferEntry>,
}

impl Offer {
    /// Checks the caps: [`MAX_MODS`] entries, each `1..=MAX_MOD_BYTES` bytes, plain text fields, and no id or
    /// SHA-256 twice.
    pub fn new(entries: Vec<OfferEntry>) -> Result<Self> {
        if entries.len() > MAX_MODS {
            return Err(Error::new(format!("mod transfer: {} mods offered (at most {MAX_MODS})", entries.len())));
        }
        let mut ids = HashSet::new();
        let mut hashes = HashSet::new();
        for entry in &entries {
            if !text_ok(&entry.id) || !text_ok(&entry.version) || entry.catalog_id.as_deref().is_some_and(|id| !text_ok(id)) {
                return Err(Error::new("mod transfer: invalid mod identity"));
            }
            if entry.size == 0 || entry.size > MAX_MOD_BYTES {
                return Err(Error::new(format!("mod transfer: {} is {} bytes (1..={MAX_MOD_BYTES})", entry.id, entry.size)));
            }
            if !ids.insert(entry.id.clone()) || !hashes.insert(entry.sha256) {
                return Err(Error::new("mod transfer: duplicate mod in the offer"));
            }
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[OfferEntry] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn find(&self, sha256: &[u8; 32]) -> Option<&OfferEntry> {
        self.entries.iter().find(|entry| &entry.sha256 == sha256)
    }

    /// The offer as a payload (kind 1).
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.u8(KIND_LIST);
        w.u8(LIST_VERSION);
        w.varint(self.entries.len() as i32);
        for entry in &self.entries {
            w.string(&entry.id);
            w.string(&entry.version);
            w.bytes(&entry.sha256);
            w.varlong(entry.size as i64);
            let mut flags = 0;
            if entry.full_access {
                flags |= FLAG_FULL_ACCESS;
            }
            if entry.side_client {
                flags |= FLAG_SIDE_CLIENT;
            }
            if entry.catalog_id.is_some() {
                flags |= FLAG_CATALOG;
            }
            w.u8(flags);
            if let Some(catalog) = &entry.catalog_id {
                w.string(catalog);
            }
        }
        w.data
    }

    fn decode_body(r: &mut Reader) -> Result<Self> {
        if r.u8()? != LIST_VERSION {
            return Err(Error::new("mod transfer: unsupported offer version"));
        }
        let count = r.varint()?;
        if !(0..=MAX_MODS as i32).contains(&count) {
            return Err(Error::new(format!("mod transfer: {count} mods offered")));
        }
        let mut entries = Vec::new();
        for _ in 0..count {
            let id = r.string(MAX_TEXT)?;
            let version = r.string(MAX_TEXT)?;
            let sha256: [u8; 32] = r.bytes(32)?.try_into().expect("32 bytes");
            let size = u64::try_from(r.varlong()?).map_err(|_| Error::new("mod transfer: negative size"))?;
            let flags = r.u8()?;
            if flags & !(FLAG_FULL_ACCESS | FLAG_SIDE_CLIENT | FLAG_CATALOG) != 0 {
                return Err(Error::new("mod transfer: unknown flags"));
            }
            let catalog_id = if flags & FLAG_CATALOG != 0 { Some(r.string(MAX_TEXT)?) } else { None };
            entries.push(OfferEntry {
                id,
                version,
                sha256,
                size,
                full_access: flags & FLAG_FULL_ACCESS != 0,
                side_client: flags & FLAG_SIDE_CLIENT != 0,
                catalog_id,
            });
        }
        Self::new(entries)
    }
}

/// Why a host did not send a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefuseReason {
    /// The SHA-256 is not in the host's offer.
    NotOffered,
    /// A transfer is still running on this connection.
    Busy,
    /// The file could not be read or is no longer the offered bytes.
    Unavailable,
    /// This connection already took its share (mods or bytes per join).
    Limit,
}

impl RefuseReason {
    fn code(self) -> u8 {
        match self {
            RefuseReason::NotOffered => 1,
            RefuseReason::Busy => 2,
            RefuseReason::Unavailable => 3,
            RefuseReason::Limit => 4,
        }
    }

    fn from_code(code: u8) -> Result<Self> {
        Ok(match code {
            1 => RefuseReason::NotOffered,
            2 => RefuseReason::Busy,
            3 => RefuseReason::Unavailable,
            4 => RefuseReason::Limit,
            _ => return Err(Error::new("mod transfer: unknown refusal")),
        })
    }

    pub fn text(self) -> &'static str {
        match self {
            RefuseReason::NotOffered => "the host does not offer that file",
            RefuseReason::Busy => "the host is busy sending files (to you or to other players)",
            RefuseReason::Unavailable => "the host could not read the file",
            RefuseReason::Limit => "the host's transfer limit for one join was reached",
        }
    }
}

/// One message of the channel, decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Capabilities(u8),
    Offer(Offer),
    Request([u8; 32]),
    Chunk { sha256: [u8; 32], offset: u64, last: bool, data: Vec<u8> },
    Refuse { sha256: [u8; 32], reason: RefuseReason },
    Done,
}

impl Message {
    pub fn decode(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let message = match r.u8()? {
            KIND_CAPS => Message::Capabilities(r.u8()?),
            KIND_LIST => Message::Offer(Offer::decode_body(&mut r)?),
            KIND_REQUEST => Message::Request(r.bytes(32)?.try_into().expect("32 bytes")),
            KIND_CHUNK => {
                let sha256 = r.bytes(32)?.try_into().expect("32 bytes");
                let offset = u64::try_from(r.varlong()?).map_err(|_| Error::new("mod transfer: negative offset"))?;
                let last = r.bool()?;
                let data = r.byte_array(MAX_CHUNK)?;
                Message::Chunk { sha256, offset, last, data }
            }
            KIND_REFUSE => {
                let sha256 = r.bytes(32)?.try_into().expect("32 bytes");
                Message::Refuse { sha256, reason: RefuseReason::from_code(r.u8()?)? }
            }
            KIND_DONE => Message::Done,
            kind => return Err(Error::new(format!("mod transfer: unknown message {kind}"))),
        };
        if r.remaining() != 0 {
            return Err(Error::new("mod transfer: trailing bytes"));
        }
        Ok(message)
    }
}

pub fn request(sha256: &[u8; 32]) -> Vec<u8> {
    let mut out = vec![KIND_REQUEST];
    out.extend_from_slice(sha256);
    out
}

pub fn done() -> Vec<u8> {
    vec![KIND_DONE]
}

fn refuse(sha256: &[u8; 32], reason: RefuseReason) -> Vec<u8> {
    let mut out = vec![KIND_REFUSE];
    out.extend_from_slice(sha256);
    out.push(reason.code());
    out
}

fn chunk(sha256: &[u8; 32], offset: u64, last: bool, data: &[u8]) -> Vec<u8> {
    let mut w = Writer::new();
    w.u8(KIND_CHUNK);
    w.bytes(sha256);
    w.varlong(offset as i64);
    w.bool(last);
    w.byte_array(data);
    w.data
}

/// The capability exchange: both peers must have advertised, like `mod_list::native::Negotiation`.
#[derive(Default)]
pub struct Negotiation {
    supported: bool,
}

impl Negotiation {
    /// Whether `data` is this version's capability message (and remembers it).
    pub fn receive_capabilities(&mut self, data: &[u8]) -> bool {
        if data != CAPABILITIES {
            return false;
        }
        self.supported = true;
        true
    }

    pub fn supported(&self) -> bool {
        self.supported
    }

    /// The offer, only once the peer advertised support.
    pub fn offer_payload(&self, offer: &Offer) -> Option<Vec<u8>> {
        self.supported.then(|| offer.encode())
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Host
// ---------------------------------------------------------------------------------------------------------------

/// Where the host's bytes come from. Read lazily; the host names only hashes of its own offer.
pub trait Source {
    /// Whether this source may start another transfer now (a host serving several joiners limits them all together).
    /// `false` makes the host answer `Busy`. A transfer that was admitted ends with [`Source::close`].
    fn admit(&mut self) -> bool {
        true
    }
    /// Opens the file of the offered `entry` for reading and checks it is still the offered bytes (size and
    /// SHA-256). `Err` when it is not, or cannot be read.
    fn open(&mut self, entry: &OfferEntry) -> std::io::Result<()>;
    /// Reads at most `buf.len()` bytes at `offset` of the file opened last; 0 at its end.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize>;
    /// Releases the file opened last.
    fn close(&mut self);
}

/// What the host's connection loop does next.
#[derive(Debug, PartialEq, Eq)]
pub enum HostEvent {
    /// Nothing to send.
    Idle,
    /// Send these payloads, then call [`HostSession::next_chunk`] until it returns `None`.
    Reply(Vec<Vec<u8>>),
    /// A transfer started: pull chunks.
    Streaming,
    /// The client needs nothing more.
    Finished,
}

struct Active {
    entry: OfferEntry,
    next: u64,
}

/// One client connection's view of a host's offer. At most one transfer at a time; at most [`MAX_MODS`] requests
/// and [`MAX_JOIN_BYTES`] bytes in all.
pub struct HostSession<S: Source> {
    offer: Offer,
    source: S,
    active: Option<Active>,
    requests: usize,
    promised: u64,
}

impl<S: Source> HostSession<S> {
    pub fn new(offer: Offer, source: S) -> Self {
        Self { offer, source, active: None, requests: 0, promised: 0 }
    }

    pub fn offer(&self) -> &Offer {
        &self.offer
    }

    /// Handles one payload from the client. A message that is not a request or `done` is a protocol error.
    pub fn handle(&mut self, payload: &[u8]) -> Result<HostEvent> {
        match Message::decode(payload)? {
            Message::Done => {
                self.finish();
                Ok(HostEvent::Finished)
            }
            Message::Request(sha256) => Ok(self.begin(&sha256)),
            // the client's capability answer arriving late, or a repeated one: harmless
            Message::Capabilities(_) => Ok(HostEvent::Idle),
            _ => Err(Error::new("mod transfer: unexpected message from the client")),
        }
    }

    fn begin(&mut self, sha256: &[u8; 32]) -> HostEvent {
        if self.active.is_some() {
            return HostEvent::Reply(vec![refuse(sha256, RefuseReason::Busy)]);
        }
        // CHECK: only hashes of the host's own offer are ever served; there is no path or name in a request.
        let Some(entry) = self.offer.find(sha256).cloned() else {
            return HostEvent::Reply(vec![refuse(sha256, RefuseReason::NotOffered)]);
        };
        if self.requests >= MAX_MODS || self.promised + entry.size > MAX_JOIN_BYTES {
            return HostEvent::Reply(vec![refuse(sha256, RefuseReason::Limit)]);
        }
        if !self.source.admit() {
            return HostEvent::Reply(vec![refuse(sha256, RefuseReason::Busy)]);
        }
        if self.source.open(&entry).is_err() {
            self.source.close();
            return HostEvent::Reply(vec![refuse(sha256, RefuseReason::Unavailable)]);
        }
        self.requests += 1;
        self.promised += entry.size;
        self.active = Some(Active { entry, next: 0 });
        HostEvent::Streaming
    }

    /// The next payload of the running transfer, read from the source now; `None` when no transfer runs (the last
    /// chunk carries the final marker). A read that comes up short ends the transfer with a refusal.
    pub fn next_chunk(&mut self) -> Option<Vec<u8>> {
        let active = self.active.as_mut()?;
        let remaining = active.entry.size - active.next;
        let length = remaining.min(MAX_CHUNK as u64) as usize;
        let mut buf = vec![0u8; length];
        let mut filled = 0;
        while filled < length {
            match self.source.read_at(active.next + filled as u64, &mut buf[filled..]) {
                Ok(0) | Err(_) => {
                    let sha256 = active.entry.sha256;
                    self.finish();
                    return Some(refuse(&sha256, RefuseReason::Unavailable));
                }
                Ok(read) => filled += read,
            }
        }
        let payload = chunk(&active.entry.sha256, active.next, active.next + length as u64 == active.entry.size, &buf);
        active.next += length as u64;
        if active.next == active.entry.size {
            self.finish();
        }
        Some(payload)
    }

    fn finish(&mut self) {
        self.active = None;
        self.source.close();
    }

    pub fn transferring(&self) -> bool {
        self.active.is_some()
    }
}

// ---------------------------------------------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------------------------------------------

/// Where the client's fetched bytes go (a staging file); nothing here is a final install.
pub trait Sink {
    /// Starts the file for `entry` (creates `<sha256>.part`).
    fn begin(&mut self, entry: &OfferEntry) -> std::io::Result<()>;
    fn write(&mut self, data: &[u8]) -> std::io::Result<()>;
    /// The size and SHA-256 were verified: make the file final (rename to `.yak`).
    fn commit(&mut self, entry: &OfferEntry) -> std::io::Result<()>;
    /// Throws the partial file away.
    fn discard(&mut self, entry: &OfferEntry);
}

/// Where a client sync stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// Fetching; `done` of `total` files are staged.
    Fetching { done: usize, total: usize },
    /// Nothing to fetch (or syncing is off): `done` was sent and the join goes on as before.
    NothingNeeded,
    /// Everything asked for is staged and verified; these entries are in staging.
    Complete(Vec<OfferEntry>),
    /// The join must stop: the text says why. Any partial file is deleted.
    Failed(String),
}

/// What one step gives back: payloads to send and the new state.
#[derive(Debug, PartialEq, Eq)]
pub struct Step {
    pub send: Vec<Vec<u8>>,
    pub state: SyncState,
}

/// The client half: compares the host's offer with what the player has, and fetches what is missing.
pub struct ClientSync<K: Sink> {
    sink: K,
    wanted: Vec<OfferEntry>,
    staged: Vec<OfferEntry>,
    current: Option<Current>,
    failed: bool,
}

struct Current {
    entry: OfferEntry,
    received: u64,
    hasher: Sha256,
}

impl<K: Sink> ClientSync<K> {
    /// Reads the host's offer. `enabled` is the launcher's setting (`--join-mod-sync`), `has` whether the player
    /// already has that exact mod. With nothing missing, or with syncing off, the reply is `done` and nothing is
    /// fetched. A plan over the caps fails here, before any byte is asked for.
    pub fn start(offer_payload: &[u8], enabled: bool, has: impl Fn(&OfferEntry) -> bool, sink: K) -> (Self, Step) {
        let mut sync = Self { sink, wanted: Vec::new(), staged: Vec::new(), current: None, failed: false };
        let offer = match Message::decode(offer_payload) {
            Ok(Message::Offer(offer)) => offer,
            Ok(_) => {
                let step = sync.fail("the host sent something else than its mod list".into());
                return (sync, step);
            }
            Err(error) => {
                let step = sync.fail(format!("the host's mod list is invalid ({error})"));
                return (sync, step);
            }
        };
        let missing: Vec<OfferEntry> = offer.entries().iter().filter(|entry| entry.side_client && !has(entry)).cloned().collect();
        if !enabled || missing.is_empty() {
            return (sync, Step { send: vec![done()], state: SyncState::NothingNeeded });
        }
        let total: u64 = missing.iter().map(|entry| entry.size).sum();
        if missing.len() > MAX_MODS || total > MAX_JOIN_BYTES {
            let step = sync.fail(format!(
                "the host's mods are too big to get automatically ({} mods, {} MiB; the limit is {MAX_MODS} mods and {} MiB)",
                missing.len(),
                total / (1024 * 1024),
                MAX_JOIN_BYTES / (1024 * 1024)
            ));
            return (sync, step);
        }
        sync.wanted = missing;
        let step = sync.next_request();
        (sync, step)
    }

    /// The entries still to fetch (the one in flight first).
    pub fn wanted(&self) -> &[OfferEntry] {
        &self.wanted
    }

    /// The entries staged and verified so far.
    pub fn staged(&self) -> &[OfferEntry] {
        &self.staged
    }

    fn fail(&mut self, text: String) -> Step {
        self.failed = true;
        if let Some(current) = self.current.take() {
            self.sink.discard(&current.entry);
        }
        Step { send: Vec::new(), state: SyncState::Failed(text) }
    }

    fn fetching(&self) -> SyncState {
        SyncState::Fetching { done: self.staged.len(), total: self.staged.len() + self.wanted.len() }
    }

    fn next_request(&mut self) -> Step {
        match self.wanted.first().cloned() {
            None => Step { send: Vec::new(), state: SyncState::Complete(self.staged.clone()) },
            Some(entry) => {
                if let Err(error) = self.sink.begin(&entry) {
                    return self.fail(format!("could not write {} ({error})", entry.id));
                }
                self.current = Some(Current { entry: entry.clone(), received: 0, hasher: Sha256::new() });
                Step { send: vec![request(&entry.sha256)], state: self.fetching() }
            }
        }
    }

    /// Handles one payload from the host while fetching. Anything unexpected, a wrong offset or size, a refusal or
    /// a SHA-256 that is not the offered one fails the sync and deletes the partial file.
    pub fn on_message(&mut self, payload: &[u8]) -> Step {
        if self.failed {
            return Step { send: Vec::new(), state: SyncState::Failed("the transfer already failed".into()) };
        }
        let message = match Message::decode(payload) {
            Ok(message) => message,
            Err(error) => return self.fail(format!("the host sent bad data ({error})")),
        };
        let Some(entry) = self.current.as_ref().map(|current| current.entry.clone()) else {
            return self.fail("the host sent data nobody asked for".into());
        };
        match message {
            Message::Chunk { sha256, offset, last, data } => {
                let received = self.current.as_ref().map_or(0, |current| current.received);
                if sha256 != entry.sha256 || offset != received {
                    return self.fail(format!("the host sent {} out of order", entry.id));
                }
                // CHECK: never more bytes than the offered size, so a host cannot make a client write without end.
                if received + data.len() as u64 > entry.size || (!last && data.is_empty()) {
                    return self.fail(format!("the host sent more of {} than it announced", entry.id));
                }
                if let Some(current) = self.current.as_mut() {
                    current.received += data.len() as u64;
                    current.hasher.update(&data);
                }
                if let Err(error) = self.sink.write(&data) {
                    return self.fail(format!("could not write {} ({error})", entry.id));
                }
                if !last {
                    return Step { send: Vec::new(), state: self.fetching() };
                }
                let current = self.current.take().expect("in flight");
                if current.received != entry.size {
                    self.current = Some(current);
                    return self.fail(format!("{} ended early", entry.id));
                }
                let digest: [u8; 32] = current.hasher.finalize().into();
                if digest != entry.sha256 {
                    self.sink.discard(&entry);
                    self.failed = true;
                    return Step {
                        send: Vec::new(),
                        state: SyncState::Failed(format!("{} from the host is not the file it announced (SHA-256 differs); it was deleted", entry.id)),
                    };
                }
                if let Err(error) = self.sink.commit(&entry) {
                    self.sink.discard(&entry);
                    self.failed = true;
                    return Step { send: Vec::new(), state: SyncState::Failed(format!("could not keep {} ({error})", entry.id)) };
                }
                self.wanted.remove(0);
                self.staged.push(entry);
                self.next_request()
            }
            Message::Refuse { sha256, reason } if sha256 == entry.sha256 => self.fail(format!("the host would not send {}: {}", entry.id, reason.text())),
            _ => self.fail("the host sent an unexpected message".into()),
        }
    }

    /// The connection ended (or the player left) mid-transfer: the partial file goes.
    pub fn abort(&mut self) {
        if let Some(current) = self.current.take() {
            self.sink.discard(&current.entry);
        }
        self.failed = true;
    }

    pub fn into_sink(self) -> K {
        self.sink
    }
}

#[cfg(test)]
mod tests;
