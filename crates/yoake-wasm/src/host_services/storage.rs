use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use super::MAX_MESSAGE_BYTES;
use super::storage_fs::Dir;

pub const MAX_STORAGE_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;
const MAX_KEYS: usize = 65_536;
const MAX_KEY_BYTES: usize = 256;
const MAX_PENDING_BLOBS: usize = 64;
const MAX_PENDING_RANGES: usize = 4096;
const MAGIC: &[u8; 5] = b"YWST\x01";
const HEADER_BYTES: usize = MAGIC.len() + 4;
const ENTRY_HEADER_BYTES: usize = 6;

struct PendingBlob {
    key: String,
    data: Vec<u8>,
    ranges: BTreeMap<usize, usize>,
    reserved: usize,
    total: usize,
}

pub struct Storage {
    entries: BTreeMap<String, Vec<u8>>,
    pending: BTreeMap<u64, PendingBlob>,
    used: usize,
    reserved: usize,
    quota: usize,
    next_ticket: u64,
}

impl Storage {
    pub fn new(quota: usize) -> Result<Self, &'static str> {
        if !(HEADER_BYTES..=MAX_STORAGE_BYTES).contains(&quota) {
            return Err("invalid storage quota");
        }
        Ok(Self {
            entries: BTreeMap::new(),
            pending: BTreeMap::new(),
            used: HEADER_BYTES,
            reserved: 0,
            quota,
            next_ticket: 0,
        })
    }

    pub fn get(&self, key: &str) -> Result<Option<&[u8]>, &'static str> {
        validate_key(key)?;
        match self.entries.get(key) {
            Some(value) if value.len() > MAX_MESSAGE_BYTES => Err("value exceeds the message limit"),
            value => Ok(value.map(Vec::as_slice)),
        }
    }

    pub fn put(&mut self, key: &str, value: &[u8]) -> Result<(), &'static str> {
        validate_key(key)?;
        if value.len() > MAX_MESSAGE_BYTES {
            return Err("value exceeds the message limit");
        }
        self.require_unlocked(key)?;
        let old_cost = self.entries.get(key).map_or(0, |old| entry_cost(key, old.len()));
        if old_cost == 0 && self.entries.len() + self.pending.len() >= MAX_KEYS {
            return Err("too many storage keys");
        }
        let next_used = self.used - old_cost + entry_cost(key, value.len());
        if next_used + self.reserved > self.quota {
            return Err("storage quota exceeded");
        }
        let mut data = Vec::new();
        data.try_reserve_exact(value.len()).map_err(|_| "storage allocation failed")?;
        data.extend_from_slice(value);
        self.entries.insert(key.to_owned(), data);
        self.used = next_used;
        Ok(())
    }

    pub fn delete(&mut self, key: &str) -> Result<bool, &'static str> {
        validate_key(key)?;
        self.require_unlocked(key)?;
        if let Some(value) = self.entries.remove(key) {
            self.used -= entry_cost(key, value.len());
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn begin_blob(&mut self, key: &str, total: usize) -> Result<u64, &'static str> {
        validate_key(key)?;
        self.require_unlocked(key)?;
        if self.pending.len() >= MAX_PENDING_BLOBS
            || (!self.entries.contains_key(key)
                && self.entries.len() + self.pending.len() >= MAX_KEYS)
        {
            return Err("too many storage keys or pending blobs");
        }
        if total > MAX_PENDING_BYTES
            || self.pending.values().map(|blob| blob.total).sum::<usize>()
                > MAX_PENDING_BYTES - total
        {
            return Err("pending blob byte limit exceeded");
        }
        let cost = entry_cost(key, total);
        if cost > self.quota || self.used + self.reserved > self.quota - cost {
            return Err("storage quota exceeded");
        }
        let ticket = self.next_ticket;
        let next_ticket = ticket.checked_add(1).ok_or("storage tickets exhausted")?;
        self.pending.insert(
            ticket,
            PendingBlob {
                key: key.to_owned(),
                data: Vec::new(),
                ranges: BTreeMap::new(),
                reserved: cost,
                total,
            },
        );
        self.reserved += cost;
        self.next_ticket = next_ticket;
        Ok(ticket)
    }

    pub fn write_blob(
        &mut self,
        ticket: u64,
        offset: usize,
        data: &[u8],
    ) -> Result<(), &'static str> {
        let blob = self
            .pending
            .get_mut(&ticket)
            .ok_or("unknown storage ticket")?;
        if data.is_empty() || data.len() > MAX_MESSAGE_BYTES {
            return Err("invalid blob chunk length");
        }
        let end = offset
            .checked_add(data.len())
            .ok_or("invalid blob offset")?;
        if end > blob.total {
            return Err("blob chunk exceeds reserved size");
        }
        let overlaps: Vec<(usize, usize)> = blob.ranges.range(..=end)
            .filter_map(|(&start, &stop)| (stop >= offset).then_some((start, stop)))
            .collect();
        for &(start, stop) in &overlaps {
            let from = start.max(offset);
            let to = stop.min(end);
            if from < to && blob.data[from..to] != data[from - offset..to - offset] {
                return Err("conflicting blob chunk");
            }
        }
        if blob.ranges.len() - overlaps.len() + 1 > MAX_PENDING_RANGES {
            return Err("too many disjoint blob chunks");
        }
        if end > blob.data.len() {
            blob.data
                .try_reserve_exact(end - blob.data.len())
                .map_err(|_| "storage allocation failed")?;
            blob.data.resize(end, 0);
        }
        let mut merged_start = offset;
        let mut merged_end = end;
        for (start, stop) in overlaps {
            merged_start = merged_start.min(start);
            merged_end = merged_end.max(stop);
            blob.ranges.remove(&start);
        }
        blob.data[offset..end].copy_from_slice(data);
        blob.ranges.insert(merged_start, merged_end);
        Ok(())
    }

    pub fn commit_blob(&mut self, ticket: u64) -> Result<(), &'static str> {
        let blob = self.pending.get(&ticket).ok_or("unknown storage ticket")?;
        let complete =
            blob.total == 0 || (blob.ranges.len() == 1 && blob.ranges.get(&0) == Some(&blob.total));
        if !complete {
            return Err("blob is incomplete");
        }
        let blob = self.pending.remove(&ticket).ok_or("unknown storage ticket")?;
        self.reserved -= blob.reserved;
        if let Some(old) = self.entries.insert(blob.key.clone(), blob.data) {
            self.used -= entry_cost(&blob.key, old.len());
        }
        self.used += blob.reserved;
        Ok(())
    }

    pub fn abort_blob(&mut self, ticket: u64) -> bool {
        if let Some(blob) = self.pending.remove(&ticket) {
            self.reserved -= blob.reserved;
            true
        } else {
            false
        }
    }

    pub fn read_blob(&self, key: &str, offset: usize, length: usize) -> Result<Option<&[u8]>, &'static str> {
        validate_key(key)?;
        if length > MAX_MESSAGE_BYTES {
            return Err("blob read exceeds the message limit");
        }
        let Some(data) = self.entries.get(key) else { return Ok(None) };
        if offset > data.len() {
            return Err("invalid blob offset");
        }
        let end = offset.saturating_add(length).min(data.len());
        Ok(Some(&data[offset..end]))
    }

    pub fn write_to(&self, mut writer: impl Write) -> Result<(), String> {
        writer.write_all(MAGIC).and_then(|()| writer.write_all(&(self.entries.len() as u32).to_le_bytes()))
            .map_err(|error| error.to_string())?;
        for (key, data) in &self.entries {
            writer.write_all(&(key.len() as u16).to_le_bytes())
                .and_then(|()| writer.write_all(&(data.len() as u32).to_le_bytes()))
                .and_then(|()| writer.write_all(key.as_bytes()))
                .and_then(|()| writer.write_all(data))
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    pub fn read_from(quota: usize, mut reader: impl Read) -> Result<Self, String> {
        let mut storage = Self::new(quota).map_err(str::to_owned)?;
        let mut magic = [0; 5];
        let mut count_bytes = [0; 4];
        reader.read_exact(&mut magic).and_then(|()| reader.read_exact(&mut count_bytes))
            .map_err(|error| error.to_string())?;
        let count = u32::from_le_bytes(count_bytes) as usize;
        if &magic != MAGIC || count > MAX_KEYS {
            return Err("invalid storage header".into());
        }
        for _ in 0..count {
            let mut key_bytes = [0; 2];
            let mut value_bytes = [0; 4];
            reader.read_exact(&mut key_bytes).and_then(|()| reader.read_exact(&mut value_bytes))
                .map_err(|error| error.to_string())?;
            let key_len = u16::from_le_bytes(key_bytes) as usize;
            let value_len = u32::from_le_bytes(value_bytes) as usize;
            let remaining = quota - storage.used;
            if key_len == 0 || key_len > MAX_KEY_BYTES
                || value_len > remaining || ENTRY_HEADER_BYTES + key_len > remaining - value_len
            {
                return Err("invalid storage entry size".into());
            }
            let mut key = vec![0; key_len];
            reader.read_exact(&mut key).map_err(|error| error.to_string())?;
            let key = String::from_utf8(key).map_err(|_| "invalid storage key")?;
            validate_key(&key).map_err(str::to_owned)?;
            if storage.entries.contains_key(&key) {
                return Err("duplicate storage key".into());
            }
            let mut value = Vec::new();
            value.try_reserve_exact(value_len).map_err(|_| "storage allocation failed")?;
            value.resize(value_len, 0);
            reader.read_exact(&mut value).map_err(|error| error.to_string())?;
            storage.used += entry_cost(&key, value.len());
            storage.entries.insert(key, value);
        }
        let mut extra = [0];
        if reader.read(&mut extra).map_err(|error| error.to_string())? != 0 {
            return Err("trailing storage data".into());
        }
        Ok(storage)
    }

    fn require_unlocked(&self, key: &str) -> Result<(), &'static str> {
        if self.pending.values().any(|blob| blob.key == key) {
            Err("key has a pending blob")
        } else {
            Ok(())
        }
    }
}

pub(crate) fn validate_key(key: &str) -> Result<(), &'static str> {
    if key.is_empty() || key.len() > MAX_KEY_BYTES
        || key.chars().any(|value| value.is_control() || matches!(value, '<' | '>' | ':' | '"' | '\\' | '|' | '?' | '*'))
        || key.split('/').any(|part| part.is_empty() || part == "." || part == ".."
            || part.ends_with('.') || part.ends_with(' ') || reserved_name(part))
    {
        Err("invalid storage key")
    } else {
        Ok(())
    }
}

fn reserved_name(name: &str) -> bool {
    let upper = name.split('.').next().unwrap_or("").trim_end_matches(' ').to_ascii_uppercase();
    let port = upper.strip_prefix("COM").or_else(|| upper.strip_prefix("LPT"));
    matches!(upper.as_str(), "CON" | "CONIN$" | "CONOUT$" | "PRN" | "AUX" | "NUL")
        || matches!(port, Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"))
}

fn entry_cost(key: &str, value_len: usize) -> usize {
    ENTRY_HEADER_BYTES.saturating_add(key.len()).saturating_add(value_len)
}

static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(0);

struct DiskEntry {
    filename: String,
    length: usize,
}

pub struct DiskStorage {
    directory: Dir,
    _lock: File,
    entries: BTreeMap<String, DiskEntry>,
    storage: Storage,
}

impl DiskStorage {
    pub(crate) fn exists(base: &Path, mod_id: &str, scope: Option<&str>) -> Result<bool, String> {
        let base = base.canonicalize().map_err(|error| error.to_string())?;
        let base = Dir::open(&base, false).map_err(|error| error.to_string())?;
        let present = |root: &str, scope: Option<&str>| {
            let directory = base
                .child(OsStr::new(root), false)
                .and_then(|directory| directory.child(OsStr::new(mod_id), false))
                .and_then(|directory| match scope {
                    Some(scope) => directory.child(OsStr::new(scope), false),
                    None => Ok(directory),
                });
            match directory {
                Ok(_) => Ok(true),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
                Err(error) => Err(error.to_string()),
            }
        };
        Ok(present("yoake-data", scope)? || present("yoake-wasm", None)?)
    }

    pub fn open(base: &Path, mod_id: &str) -> Result<Self, String> {
        Self::open_with_quota(base, mod_id, MAX_STORAGE_BYTES)
    }

    pub fn open_with_quota(base: &Path, mod_id: &str, quota: usize) -> Result<Self, String> {
        Self::open_scoped_with_quota(base, mod_id, None, quota)
    }

    pub fn open_scoped(base: &Path, mod_id: &str, scope: &'static str) -> Result<Self, String> {
        if !matches!(scope, "global" | "client") {
            return Err("invalid storage scope".into());
        }
        Self::open_scoped_with_quota(base, mod_id, Some(scope), MAX_STORAGE_BYTES)
    }

    fn open_scoped_with_quota(
        base: &Path,
        mod_id: &str,
        scope: Option<&'static str>,
        quota: usize,
    ) -> Result<Self, String> {
        if !(3..=48).contains(&mod_id.len())
            || reserved_name(mod_id)
            || !mod_id.bytes().all(|value| {
                value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, b'_' | b'-')
            })
        {
            return Err("invalid storage mod ID".into());
        }
        let base = base.canonicalize().map_err(|error| error.to_string())?;
        let base_dir = Dir::open(&base, false).map_err(|error| error.to_string())?;
        let mut directory = base_dir
            .child(OsStr::new("yoake-data"), true)
            .and_then(|directory| directory.child(OsStr::new(mod_id), true))
            .map_err(|error| error.to_string())?;
        if let Some(scope) = scope {
            directory = directory
                .child(OsStr::new(scope), true)
                .map_err(|error| error.to_string())?;
        }
        let lock = directory
            .lock(OsStr::new("store.lock"))
            .map_err(|error| error.to_string())?;
        let mut result = Self {
            directory,
            _lock: lock,
            entries: BTreeMap::new(),
            storage: Storage::new(quota).map_err(str::to_owned)?,
        };
        for filename in result
            .directory
            .entries()
            .map_err(|error| error.to_string())?
        {
            let name = filename.to_string_lossy();
            if name.starts_with(".store-") && name.ends_with(".tmp") {
                result
                    .directory
                    .remove(&filename)
                    .map_err(|error| error.to_string())?;
            } else if name.starts_with("entry-") && name.ends_with(".bin") {
                let mut file = result
                    .directory
                    .read(&filename)
                    .map_err(|error| error.to_string())?
                    .ok_or("storage entry disappeared")?;
                let (key, length) = read_entry_header(&mut file, quota)?;
                if file.metadata().map_err(|error| error.to_string())?.len()
                    != entry_cost(&key, length) as u64
                {
                    return Err("invalid storage entry size".into());
                }
                result.check_size(&key, length)?;
                if result.entries.contains_key(&key) {
                    return Err("duplicate storage key".into());
                }
                result.record(key, name.into_owned(), length);
            }
        }
        if result
            .directory
            .read(OsStr::new("migration-v2.done"))
            .map_err(|error| error.to_string())?
            .is_none()
        {
            if let Some(mut file) = result
                .directory
                .read(OsStr::new("store.bin"))
                .map_err(|error| error.to_string())?
            {
                let mut header = [0; HEADER_BYTES];
                file.read_exact(&mut header)
                    .map_err(|error| error.to_string())?;
                let count = u32::from_le_bytes(
                    header[5..]
                        .try_into()
                        .map_err(|_| "invalid storage header")?,
                ) as usize;
                if &header[..5] != MAGIC || count > MAX_KEYS {
                    return Err("invalid storage header".into());
                }
                let mut keys = std::collections::BTreeSet::new();
                for _ in 0..count {
                    let (key, length) = read_entry_header(&mut file, quota)?;
                    if !keys.insert(key.clone()) {
                        return Err("duplicate storage key".into());
                    }
                    result.import(&key, length, &mut file)?;
                }
                let mut extra = [0];
                if file.read(&mut extra).map_err(|error| error.to_string())? != 0 {
                    return Err("trailing storage data".into());
                }
            }
            match base_dir
                .child(OsStr::new("yoake-wasm"), false)
                .and_then(|directory| directory.child(OsStr::new(mod_id), false))
            {
                Ok(legacy) => result.import_directory(&legacy, "", scope.is_none())?,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
            result.atomic_write("migration-v2.done", |file| {
                file.write_all(b"2").map_err(|error| error.to_string())
            })?;
        }
        Ok(result)
    }

    fn check_size(&self, key: &str, length: usize) -> Result<(), String> {
        validate_key(key).map_err(str::to_owned)?;
        if length > MAX_STORAGE_BYTES {
            return Err("storage value too large".into());
        }
        let old_cost = self
            .entries
            .get(key)
            .map_or(0, |entry| entry_cost(key, entry.length));
        if old_cost == 0 && self.entries.len() + self.storage.pending.len() >= MAX_KEYS {
            return Err("too many storage keys".into());
        }
        if self.storage.used - old_cost + entry_cost(key, length) + self.storage.reserved
            > self.storage.quota
        {
            return Err("storage quota exceeded".into());
        }
        Ok(())
    }

    fn record(&mut self, key: String, filename: String, length: usize) {
        if let Some(old) = self
            .entries
            .insert(key.clone(), DiskEntry { filename, length })
        {
            self.storage.used -= entry_cost(&key, old.length);
        }
        self.storage.used += entry_cost(&key, length);
        self.storage.entries.insert(key, Vec::new());
    }

    fn import(&mut self, key: &str, length: usize, reader: &mut impl Read) -> Result<(), String> {
        if self.entries.contains_key(key) {
            if std::io::copy(&mut reader.take(length as u64), &mut std::io::sink())
                .map_err(|error| error.to_string())?
                != length as u64
            {
                return Err("truncated legacy value".into());
            }
            return Ok(());
        }
        self.check_size(key, length)?;
        let filename = self.persist(key, length, reader)?;
        self.record(key.to_owned(), filename, length);
        Ok(())
    }

    fn import_directory(
        &mut self,
        directory: &Dir,
        prefix: &str,
        binary_suffix: bool,
    ) -> Result<(), String> {
        for name in directory.entries().map_err(|error| error.to_string())? {
            let name_text = name.to_str().ok_or("invalid legacy filename")?;
            let key = format!("{prefix}{name_text}");
            if let Ok(child) = directory.child(&name, false) {
                validate_key(&key).map_err(str::to_owned)?;
                self.import_directory(&child, &format!("{key}/"), binary_suffix)?;
            } else if let Some(mut file) =
                directory.read(&name).map_err(|error| error.to_string())?
            {
                let key = if binary_suffix {
                    let Some(key) = key.strip_suffix(".bin") else {
                        continue;
                    };
                    key
                } else {
                    &key
                };
                let length =
                    usize::try_from(file.metadata().map_err(|error| error.to_string())?.len())
                        .map_err(|_| "legacy value too large")?;
                self.import(key, length, &mut file)?;
            }
        }
        Ok(())
    }

    pub fn get(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        validate_key(key).map_err(str::to_owned)?;
        if self
            .entries
            .get(key)
            .is_some_and(|entry| entry.length > MAX_MESSAGE_BYTES)
        {
            return Err("value exceeds the message limit".into());
        }
        self.read_blob(key, 0, MAX_MESSAGE_BYTES)
    }

    pub fn put(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        if value.len() > MAX_MESSAGE_BYTES {
            return Err("value exceeds the message limit".into());
        }
        self.storage.require_unlocked(key).map_err(str::to_owned)?;
        self.check_size(key, value.len())?;
        let filename = self.persist(key, value.len(), &mut &value[..])?;
        self.record(key.to_owned(), filename, value.len());
        Ok(())
    }

    pub fn delete(&mut self, key: &str) -> Result<bool, String> {
        validate_key(key).map_err(str::to_owned)?;
        self.storage.require_unlocked(key).map_err(str::to_owned)?;
        let Some(entry) = self.entries.get(key) else {
            return Ok(false);
        };
        self.directory
            .remove(OsStr::new(&entry.filename))
            .map_err(|error| error.to_string())?;
        self.storage.used -= entry_cost(key, entry.length);
        self.entries.remove(key);
        self.storage.entries.remove(key);
        Ok(true)
    }

    pub fn begin_blob(&mut self, key: &str, total: usize) -> Result<u64, &'static str> {
        self.storage.begin_blob(key, total)
    }
    pub fn write_blob(&mut self, ticket: u64, offset: usize, data: &[u8]) -> Result<(), &'static str> {
        self.storage.write_blob(ticket, offset, data)
    }
    pub fn pending_bytes(&self) -> usize {
        self.storage.pending.values().map(|blob| blob.total).sum()
    }
    pub fn blob_size(&self, ticket: u64) -> Option<usize> {
        self.storage.pending.get(&ticket).map(|blob| blob.total)
    }
    pub fn abort_all(&mut self) {
        self.storage.pending.clear();
        self.storage.reserved = 0;
    }

    pub fn commit_blob(&mut self, ticket: u64) -> Result<(), String> {
        let blob = self
            .storage
            .pending
            .get(&ticket)
            .ok_or("unknown storage ticket")?;
        if blob.total != 0 && !(blob.ranges.len() == 1 && blob.ranges.get(&0) == Some(&blob.total))
        {
            return Err("blob is incomplete".into());
        }
        let key = blob.key.clone();
        let length = blob.total;
        let filename = self.persist(&key, length, &mut blob.data.as_slice())?;
        self.storage.abort_blob(ticket);
        self.record(key, filename, length);
        Ok(())
    }
    pub fn abort_blob(&mut self, ticket: u64) -> bool {
        self.storage.abort_blob(ticket)
    }

    pub fn read_blob(
        &self,
        key: &str,
        offset: usize,
        length: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        validate_key(key).map_err(str::to_owned)?;
        if length > MAX_MESSAGE_BYTES {
            return Err("blob read exceeds the message limit".into());
        }
        let Some(entry) = self.entries.get(key) else {
            return Ok(None);
        };
        if offset > entry.length {
            return Err("invalid blob offset".into());
        }
        let mut file = self
            .directory
            .read(OsStr::new(&entry.filename))
            .map_err(|error| error.to_string())?
            .ok_or("storage entry disappeared")?;
        file.seek(SeekFrom::Start((entry_cost(key, 0) + offset) as u64))
            .map_err(|error| error.to_string())?;
        let count = length.min(entry.length - offset);
        let mut data = vec![0; count];
        file.read_exact(&mut data)
            .map_err(|error| error.to_string())?;
        Ok(Some(data))
    }

    fn persist(&self, key: &str, length: usize, reader: &mut impl Read) -> Result<String, String> {
        let filename = if let Some(entry) = self.entries.get(key) {
            entry.filename.clone()
        } else {
            loop {
                let candidate = format!(
                    "entry-{}-{}.bin",
                    std::process::id(),
                    NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
                );
                if self
                    .directory
                    .read(OsStr::new(&candidate))
                    .map_err(|error| error.to_string())?
                    .is_none()
                {
                    break candidate;
                }
            }
        };
        self.atomic_write(&filename, |file| {
            file.write_all(&(key.len() as u16).to_le_bytes())
                .and_then(|()| file.write_all(&(length as u32).to_le_bytes()))
                .and_then(|()| file.write_all(key.as_bytes()))
                .map_err(|error| error.to_string())?;
            if std::io::copy(&mut reader.take(length as u64), file)
                .map_err(|error| error.to_string())?
                != length as u64
            {
                return Err("truncated storage value".into());
            }
            Ok(())
        })?;
        Ok(filename)
    }

    fn atomic_write(
        &self,
        target: &str,
        write: impl FnOnce(&mut File) -> Result<(), String>,
    ) -> Result<(), String> {
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let name = format!(".store-{}-{sequence}.tmp", std::process::id());
        let mut file = self
            .directory
            .create(OsStr::new(&name))
            .map_err(|error| error.to_string())?;
        let result =
            write(&mut file).and_then(|()| file.sync_all().map_err(|error| error.to_string()));
        drop(file);
        let result = result.and_then(|()| {
            self.directory
                .replace(OsStr::new(&name), OsStr::new(target))
                .map_err(|error| error.to_string())
        });
        if result.is_err() {
            let _ = self.directory.remove(OsStr::new(&name));
        }
        result
    }
}

fn read_entry_header(reader: &mut impl Read, quota: usize) -> Result<(String, usize), String> {
    let mut header = [0; ENTRY_HEADER_BYTES];
    reader
        .read_exact(&mut header)
        .map_err(|error| error.to_string())?;
    let key_length = u16::from_le_bytes([header[0], header[1]]) as usize;
    let length = u32::from_le_bytes(
        header[2..]
            .try_into()
            .map_err(|_| "invalid storage header")?,
    ) as usize;
    if key_length == 0
        || key_length > MAX_KEY_BYTES
        || length > MAX_STORAGE_BYTES
        || ENTRY_HEADER_BYTES + key_length + length > quota
    {
        return Err("invalid storage entry size".into());
    }
    let mut key = vec![0; key_length];
    reader
        .read_exact(&mut key)
        .map_err(|error| error.to_string())?;
    let key = String::from_utf8(key).map_err(|_| "invalid storage key")?;
    validate_key(&key).map_err(str::to_owned)?;
    Ok((key, length))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    #[test]
    fn pending_blobs_are_lazy_and_bounded() {
        let mut storage = super::Storage::new(super::MAX_STORAGE_BYTES).unwrap();
        let ticket = storage.begin_blob("large", 16 * 1024 * 1024).unwrap();
        assert_eq!(storage.pending[&ticket].data.capacity(), 0);
        assert!(storage.begin_blob("excess", 17 * 1024 * 1024).is_err());
        storage.abort_blob(ticket);
        assert!(storage.begin_blob("replacement", 32 * 1024 * 1024).is_ok());
        assert!(storage.begin_blob("overflow", usize::MAX).is_err());
    }

    use super::*;

    fn temporary_root() -> PathBuf {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap()
            .join(format!("yoake-storage-{}-{time}-{sequence}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        root
    }

    #[test]
    fn disk_storage_persists_and_isolates_scopes() {
        let root = temporary_root();
        let world = root.join("world");
        let global = root.join("global");
        std::fs::create_dir(&world).unwrap();
        std::fs::create_dir(&global).unwrap();
        let mut store = DiskStorage::open_with_quota(&world, "example", 128).unwrap();
        store.put("nest/config", b"hello").unwrap();
        let ticket = store.begin_blob("blob", 6).unwrap();
        store.write_blob(ticket, 3, b"def").unwrap();
        assert!(store.commit_blob(ticket).is_err());
        assert!(DiskStorage::open_with_quota(&world, "example", 128).is_err());
        store.write_blob(ticket, 0, b"abc").unwrap();
        store.commit_blob(ticket).unwrap();
        assert!(store.put("large", &[0; 128]).is_err());
        drop(store);
        let mut restored = DiskStorage::open_with_quota(&world, "example", 128).unwrap();
        assert_eq!(
            restored.get("nest/config").unwrap().as_deref(),
            Some(&b"hello"[..])
        );
        assert_eq!(
            restored.read_blob("blob", 2, 3).unwrap().as_deref(),
            Some(&b"cde"[..])
        );
        assert_eq!(
            DiskStorage::open_with_quota(&global, "example", 128)
                .unwrap()
                .get("blob")
                .unwrap(),
            None
        );
        assert_eq!(
            DiskStorage::open_with_quota(&world, "other", 128)
                .unwrap()
                .get("blob")
                .unwrap(),
            None
        );
        assert!(restored.delete("blob").unwrap());
        drop(restored);
        assert_eq!(
            DiskStorage::open_with_quota(&world, "example", 128)
                .unwrap()
                .get("blob")
                .unwrap(),
            None
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn small_mutations_do_not_rewrite_unrelated_values() {
        let root = temporary_root();
        let mut store = DiskStorage::open(&root, "example").unwrap();
        let value = vec![37; MAX_MESSAGE_BYTES];
        store.put("large", &value).unwrap();
        let filename = store.entries["large"].filename.clone();
        let path = root.join("yoake-data/example").join(&filename);
        #[cfg(windows)]
        let pinned = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&path)
                .unwrap()
        };
        #[cfg(not(windows))]
        let pinned = std::fs::File::open(&path).unwrap();
        let before = pinned.metadata().unwrap().modified().unwrap();
        store.put("small", b"first").unwrap();
        store.put("small", b"second").unwrap();
        assert!(store.delete("small").unwrap());
        assert_eq!(pinned.metadata().unwrap().modified().unwrap(), before);
        assert_eq!(store.get("large").unwrap(), Some(value.clone()));
        drop(pinned);
        drop(store);
        let store = DiskStorage::open(&root, "example").unwrap();
        assert!(
            store
                .storage
                .entries
                .values()
                .all(|value| value.capacity() == 0)
        );
        assert_eq!(store.get("large").unwrap(), Some(value));
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_stores_are_imported_once_with_quota_checks() {
        let root = temporary_root();
        let directory = root.join("yoake-data/example");
        std::fs::create_dir_all(&directory).unwrap();
        let key = b"from-store";
        let value = b"original";
        let mut encoded = MAGIC.to_vec();
        encoded.extend_from_slice(&1u32.to_le_bytes());
        encoded.extend_from_slice(&(key.len() as u16).to_le_bytes());
        encoded.extend_from_slice(&(value.len() as u32).to_le_bytes());
        encoded.extend_from_slice(key);
        encoded.extend_from_slice(value);
        std::fs::write(directory.join("store.bin"), &encoded).unwrap();
        let legacy = root.join("yoake-wasm/example/nested");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("key.bin"), b"legacy").unwrap();
        assert!(DiskStorage::open_with_quota(&root, "example", 12).is_err());
        assert!(!directory.join("migration-v2.done").exists());
        assert_eq!(std::fs::read(directory.join("store.bin")).unwrap(), encoded);
        let mut store = DiskStorage::open(&root, "example").unwrap();
        assert_eq!(store.get("from-store").unwrap(), Some(value.to_vec()));
        assert_eq!(store.get("nested/key").unwrap(), Some(b"legacy".to_vec()));
        store.put("nested/key", b"new").unwrap();
        drop(store);
        std::fs::write(legacy.join("later.bin"), b"not imported").unwrap();
        let store = DiskStorage::open(&root, "example").unwrap();
        assert_eq!(store.get("nested/key").unwrap(), Some(b"new".to_vec()));
        assert_eq!(store.get("nested/later").unwrap(), None);
        assert!(directory.join("migration-v2.done").exists());
        drop(store);
        let config = root.join("other-game");
        std::fs::create_dir_all(config.join("yoake-wasm/example")).unwrap();
        std::fs::write(config.join("yoake-wasm/example/settings.json"), b"{}").unwrap();
        let store = DiskStorage::open_scoped(&config, "example", "global").unwrap();
        assert_eq!(store.get("settings.json").unwrap(), Some(b"{}".to_vec()));
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_temporary_files_are_removed_on_open() {
        let root = temporary_root();
        let store = DiskStorage::open(&root, "example").unwrap();
        drop(store);
        let temporary = root.join("yoake-data/example/.store-interrupted.tmp");
        std::fs::write(&temporary, b"incomplete").unwrap();
        let store = DiskStorage::open(&root, "example").unwrap();
        assert!(!temporary.exists());
        assert_eq!(store.get("missing").unwrap(), None);
        drop(store);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn store_lock_excludes_other_processes() {
        const ROOT_ENV: &str = "YOAKE_STORAGE_PROCESS_TEST";
        if let Some(root) = std::env::var_os(ROOT_ENV) {
            let result = DiskStorage::open(Path::new(&root), "example");
            if std::env::var_os("YOAKE_STORAGE_EXPECT_LOCKED").is_some() {
                assert!(result.err().unwrap().contains("already open"));
            } else {
                assert!(result.is_ok());
            }
            return;
        }
        let root = temporary_root();
        let store = DiskStorage::open(&root, "example").unwrap();
        let child = |locked| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "host_services::storage::tests::store_lock_excludes_other_processes",
                ])
                .env(ROOT_ENV, &root);
            if locked {
                command.env("YOAKE_STORAGE_EXPECT_LOCKED", "1");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        };
        child(true);
        drop(store);
        child(false);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn trusted_base_accepts_links_and_parent_components() {
        let root = temporary_root();
        let real = root.join("real");
        let linked = root.join("linked");
        std::fs::create_dir_all(real.join("inside")).unwrap();
        #[cfg(windows)]
        {
            let junction_root = std::env::temp_dir().join(root.file_name().unwrap());
            let status = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:YOAKE_TEST_JUNCTION -Target $env:YOAKE_TEST_TARGET -ErrorAction Stop | Out-Null"])
                .env("YOAKE_TEST_JUNCTION", junction_root.join("linked"))
                .env("YOAKE_TEST_TARGET", junction_root.join("real")).status().unwrap();
            assert!(status.success());
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &linked).unwrap();
        let base = std::env::temp_dir()
            .join(root.file_name().unwrap())
            .join("linked")
            .join("inside")
            .join("..");
        let mut store = DiskStorage::open(&base, "example").unwrap();
        store.put("key", b"value").unwrap();
        assert_eq!(store.get("key").unwrap(), Some(b"value".to_vec()));
        assert!(real.join("yoake-data/example").is_dir());
        drop(store);
        #[cfg(windows)]
        std::fs::remove_dir(&linked).unwrap();
        #[cfg(unix)]
        std::fs::remove_file(&linked).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disk_storage_rejects_unsafe_names_and_failed_writes() {
        let root = temporary_root();
        for mod_id in ["../evil", "con", "com1", "Bad", "ab"] {
            assert!(DiskStorage::open(&root, mod_id).is_err());
        }
        let mut store = DiskStorage::open_with_quota(&root, "example", 128).unwrap();
        for key in [
            "../x",
            "C:/x",
            "/abs",
            "folder/..",
            "dir\\file",
            "CON",
            "nul.txt",
            "CON .txt",
            "COM¹",
            "lpt².txt",
            "CONIN$",
            "trail.",
        ] {
            assert!(store.put(key, b"value").is_err(), "accepted {key}");
        }
        store.put("key", b"old").unwrap();
        let folder = root.join("yoake-data").join("example");
        let filename = store.entries["key"].filename.clone();
        std::fs::remove_file(folder.join(&filename)).unwrap();
        std::fs::create_dir(folder.join(&filename)).unwrap();
        assert!(store.put("key", b"value").is_err());
        drop(store);
        std::fs::remove_dir(folder.join(&filename)).unwrap();
        std::fs::remove_file(folder.join("migration-v2.done")).unwrap();
        std::fs::write(folder.join("store.bin"), b"broken").unwrap();
        assert!(DiskStorage::open(&root, "example").is_err());
        assert_eq!(std::fs::read(folder.join("store.bin")).unwrap(), b"broken");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn disk_storage_refuses_junctions_before_and_after_open() {
        let root = temporary_root();
        let outside = root.join("outside");
        let data = root.join("yoake-data");
        let junction = data.join("example");
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(&data).unwrap();
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command",
                "New-Item -ItemType Junction -Path $env:YOAKE_TEST_JUNCTION -Target $env:YOAKE_TEST_TARGET -ErrorAction Stop | Out-Null"])
            .env("YOAKE_TEST_JUNCTION", &junction).env("YOAKE_TEST_TARGET", &outside)
            .status().unwrap();
        assert!(status.success(), "junction creation failed");
        assert!(DiskStorage::open(&root, "example").is_err());
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        assert!(junction.starts_with(&root));
        std::fs::remove_dir(&junction).unwrap();
        let mut store = DiskStorage::open(&root, "example").unwrap();
        assert!(std::fs::rename(&junction, data.join("moved")).is_err());
        store.put("key", b"confined").unwrap();
        drop(store);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn disk_storage_refuses_symlinked_mod_directories() {
        use std::os::unix::fs::symlink;

        let root = temporary_root();
        let outside = root.join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(root.join("yoake-data")).unwrap();
        symlink(&outside, root.join("yoake-data").join("example")).unwrap();
        assert!(DiskStorage::open(&root, "example").is_err());
        assert!(std::fs::read_dir(&outside).unwrap().next().is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn roundtrip_preserves_committed_values_and_isolated_stores() {
        let mut world = Storage::new(1024).unwrap();
        let global = Storage::new(1024).unwrap();
        world.put("nest/config", b"hello").unwrap();
        let pending = world.begin_blob("unfinished", 3).unwrap();
        world.write_blob(pending, 0, b"ab").unwrap();
        let mut bytes = Vec::new();
        world.write_to(&mut bytes).unwrap();
        let restored = Storage::read_from(1024, bytes.as_slice()).unwrap();
        assert_eq!(restored.get("nest/config").unwrap(), Some(&b"hello"[..]));
        assert_eq!(restored.get("unfinished").unwrap(), None);
        assert_eq!(global.get("nest/config").unwrap(), None);
    }

    #[test]
    fn failed_replacement_and_aborted_blob_preserve_quota_and_value() {
        let mut storage = Storage::new(41).unwrap();
        storage.put("key", b"old").unwrap();
        assert!(storage.put("key", &[0; 30]).is_err());
        let ticket = storage.begin_blob("blob", 10).unwrap();
        assert!(storage.put("key", &[0; 10]).is_err());
        assert!(storage.abort_blob(ticket));
        assert_eq!(storage.get("key").unwrap(), Some(&b"old"[..]));
        storage.put("key", &[0; 10]).unwrap();
    }

    #[test]
    fn blob_is_private_until_complete_and_retries_must_match() {
        let mut storage = Storage::new(1024).unwrap();
        storage.put("key", b"old").unwrap();
        let ticket = storage.begin_blob("key", 6).unwrap();
        storage.write_blob(ticket, 3, b"def").unwrap();
        assert!(storage.commit_blob(ticket).is_err());
        assert_eq!(storage.get("key").unwrap(), Some(&b"old"[..]));
        assert!(storage.write_blob(ticket, 4, b"XX").is_err());
        storage.write_blob(ticket, 2, b"cde").unwrap();
        storage.write_blob(ticket, 0, b"abc").unwrap();
        storage.commit_blob(ticket).unwrap();
        assert_eq!(storage.read_blob("key", 2, 3).unwrap(), Some(&b"cde"[..]));
        assert_eq!(storage.get("key").unwrap(), Some(&b"abcdef"[..]));
    }

    #[test]
    fn large_blobs_require_bounded_reads() {
        let mut storage = Storage::new(MAX_MESSAGE_BYTES + 128).unwrap();
        let ticket = storage.begin_blob("blob", MAX_MESSAGE_BYTES + 1).unwrap();
        storage.write_blob(ticket, 0, &vec![1; MAX_MESSAGE_BYTES]).unwrap();
        storage.write_blob(ticket, MAX_MESSAGE_BYTES, &[2]).unwrap();
        storage.commit_blob(ticket).unwrap();
        assert!(storage.get("blob").is_err());
        assert_eq!(storage.read_blob("blob", MAX_MESSAGE_BYTES, 1).unwrap(), Some(&[2][..]));
        assert!(storage.read_blob("blob", 0, MAX_MESSAGE_BYTES + 1).is_err());
    }

    #[test]
    fn truncated_oversized_and_trailing_data_are_rejected() {
        let mut storage = Storage::new(1024).unwrap();
        storage.put("key", b"value").unwrap();
        let mut bytes = Vec::new();
        storage.write_to(&mut bytes).unwrap();
        assert!(Storage::read_from(1024, &bytes[..bytes.len() - 1]).is_err());
        assert!(Storage::read_from(HEADER_BYTES, bytes.as_slice()).is_err());
        bytes.push(0);
        assert!(Storage::read_from(1024, bytes.as_slice()).is_err());
    }
}
