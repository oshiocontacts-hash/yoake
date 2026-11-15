use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::storage_fs::Dir;
use super::MAX_MESSAGE_BYTES;

pub const MAX_STORAGE_BYTES: usize = 256 * 1024 * 1024;
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
        let cost = entry_cost(key, total);
        if cost > self.quota || self.used + self.reserved > self.quota - cost {
            return Err("storage quota exceeded");
        }
        let ticket = self.next_ticket;
        let next_ticket = ticket.checked_add(1).ok_or("storage tickets exhausted")?;
        let mut data = Vec::new();
        data.try_reserve_exact(total).map_err(|_| "storage allocation failed")?;
        data.resize(total, 0);
        self.pending.insert(ticket, PendingBlob {
            key: key.to_owned(),
            data,
            ranges: BTreeMap::new(),
            reserved: cost,
        });
        self.reserved += cost;
        self.next_ticket = next_ticket;
        Ok(ticket)
    }

    pub fn write_blob(&mut self, ticket: u64, offset: usize, data: &[u8]) -> Result<(), &'static str> {
        let blob = self.pending.get_mut(&ticket).ok_or("unknown storage ticket")?;
        if data.is_empty() || data.len() > MAX_MESSAGE_BYTES {
            return Err("invalid blob chunk length");
        }
        let end = offset.checked_add(data.len()).ok_or("invalid blob offset")?;
        if end > blob.data.len() {
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
        let complete = blob.data.is_empty()
            || (blob.ranges.len() == 1 && blob.ranges.get(&0) == Some(&blob.data.len()));
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

fn validate_key(key: &str) -> Result<(), &'static str> {
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

pub struct DiskStorage {
    base: PathBuf,
    mod_id: String,
    storage: Storage,
}

impl DiskStorage {
    pub fn open(base: &Path, mod_id: &str) -> Result<Self, String> {
        Self::open_with_quota(base, mod_id, MAX_STORAGE_BYTES)
    }

    pub fn open_with_quota(base: &Path, mod_id: &str, quota: usize) -> Result<Self, String> {
        if !(3..=48).contains(&mod_id.len()) || reserved_name(mod_id)
            || !mod_id.bytes().all(|value| value.is_ascii_lowercase() || value.is_ascii_digit() || matches!(value, b'_' | b'-'))
        {
            return Err("invalid storage mod ID".into());
        }
        let mut result = Self {
            base: base.to_path_buf(),
            mod_id: mod_id.to_owned(),
            storage: Storage::new(quota).map_err(str::to_owned)?,
        };
        match result.directory(false) {
            Ok(directory) => {
                if let Some(file) = directory.read(OsStr::new("store.bin")).map_err(|error| error.to_string())? {
                    result.storage = Storage::read_from(quota, file)?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Dir::open(base, false).map_err(|error| error.to_string())?;
            }
            Err(error) => return Err(error.to_string()),
        }
        Ok(result)
    }

    fn directory(&self, create: bool) -> std::io::Result<Dir> {
        Dir::open(&self.base, false)?
            .child(OsStr::new("yoake-data"), create)?
            .child(OsStr::new(&self.mod_id), create)
    }

    pub fn get(&self, key: &str) -> Result<Option<&[u8]>, &'static str> {
        self.storage.get(key)
    }

    pub fn put(&mut self, key: &str, value: &[u8]) -> Result<(), String> {
        validate_key(key).map_err(str::to_owned)?;
        if value.len() > MAX_MESSAGE_BYTES {
            return Err("value exceeds the message limit".into());
        }
        self.storage.require_unlocked(key).map_err(str::to_owned)?;
        let old_cost = self.storage.entries.get(key).map_or(0, |old| entry_cost(key, old.len()));
        if old_cost == 0 && self.storage.entries.len() + self.storage.pending.len() >= MAX_KEYS {
            return Err("too many storage keys".into());
        }
        let next_used = self.storage.used - old_cost + entry_cost(key, value.len());
        if next_used + self.storage.reserved > self.storage.quota {
            return Err("storage quota exceeded".into());
        }
        let mut data = Vec::new();
        data.try_reserve_exact(value.len()).map_err(|_| "storage allocation failed")?;
        data.extend_from_slice(value);
        self.persist(key, Some(&data))?;
        self.storage.entries.insert(key.to_owned(), data);
        self.storage.used = next_used;
        Ok(())
    }

    pub fn delete(&mut self, key: &str) -> Result<bool, String> {
        validate_key(key).map_err(str::to_owned)?;
        self.storage.require_unlocked(key).map_err(str::to_owned)?;
        let Some(old) = self.storage.entries.get(key) else { return Ok(false) };
        let old_cost = entry_cost(key, old.len());
        self.persist(key, None)?;
        self.storage.entries.remove(key);
        self.storage.used -= old_cost;
        Ok(true)
    }

    pub fn begin_blob(&mut self, key: &str, total: usize) -> Result<u64, &'static str> {
        self.storage.begin_blob(key, total)
    }

    pub fn write_blob(&mut self, ticket: u64, offset: usize, data: &[u8]) -> Result<(), &'static str> {
        self.storage.write_blob(ticket, offset, data)
    }

    pub fn commit_blob(&mut self, ticket: u64) -> Result<(), String> {
        let blob = self.storage.pending.get(&ticket).ok_or("unknown storage ticket")?;
        let complete = blob.data.is_empty()
            || (blob.ranges.len() == 1 && blob.ranges.get(&0) == Some(&blob.data.len()));
        if !complete {
            return Err("blob is incomplete".into());
        }
        self.persist(&blob.key, Some(&blob.data))?;
        self.storage.commit_blob(ticket).map_err(str::to_owned)
    }

    pub fn abort_blob(&mut self, ticket: u64) -> bool {
        self.storage.abort_blob(ticket)
    }

    pub fn read_blob(&self, key: &str, offset: usize, length: usize) -> Result<Option<&[u8]>, &'static str> {
        self.storage.read_blob(key, offset, length)
    }

    fn persist(&self, key: &str, replacement: Option<&[u8]>) -> Result<(), String> {
        let directory = self.directory(true).map_err(|error| error.to_string())?;
        let mut created = None;
        for _ in 0..32 {
            let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
            let name = format!(".store-{}-{sequence}.tmp", std::process::id());
            match directory.create(OsStr::new(&name)) {
                Ok(file) => { created = Some((name, file)); break; }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        let (name, mut file) = created.ok_or("could not create a storage temporary file")?;
        let result = (|| -> Result<(), String> {
            let count = self.storage.entries.len() - self.storage.entries.contains_key(key) as usize
                + replacement.is_some() as usize;
            file.write_all(MAGIC).and_then(|()| file.write_all(&(count as u32).to_le_bytes()))
                .map_err(|error| error.to_string())?;
            for (existing_key, data) in &self.storage.entries {
                if existing_key != key {
                    write_entry(&mut file, existing_key, data)?;
                }
            }
            if let Some(data) = replacement {
                write_entry(&mut file, key, data)?;
            }
            file.sync_all().map_err(|error| error.to_string())
        })();
        drop(file);
        let result = result.and_then(|()| directory.replace(OsStr::new(&name), OsStr::new("store.bin"))
            .map_err(|error| error.to_string()));
        if result.is_err() {
            let _ = directory.remove(OsStr::new(&name));
        }
        result
    }
}

fn write_entry(file: &mut File, key: &str, data: &[u8]) -> Result<(), String> {
    file.write_all(&(key.len() as u16).to_le_bytes())
        .and_then(|()| file.write_all(&(data.len() as u32).to_le_bytes()))
        .and_then(|()| file.write_all(key.as_bytes()))
        .and_then(|()| file.write_all(data))
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
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
        assert!(!world.join("yoake-data").exists());
        store.put("nest/config", b"hello").unwrap();
        let ticket = store.begin_blob("blob", 6).unwrap();
        store.write_blob(ticket, 3, b"def").unwrap();
        assert!(store.commit_blob(ticket).is_err());
        let restored = DiskStorage::open_with_quota(&world, "example", 128).unwrap();
        assert_eq!(restored.get("blob").unwrap(), None);
        store.write_blob(ticket, 0, b"abc").unwrap();
        store.commit_blob(ticket).unwrap();
        assert!(store.put("large", &[0; 128]).is_err());
        let mut restored = DiskStorage::open_with_quota(&world, "example", 128).unwrap();
        assert_eq!(restored.get("nest/config").unwrap(), Some(&b"hello"[..]));
        assert_eq!(restored.read_blob("blob", 2, 3).unwrap(), Some(&b"cde"[..]));
        assert_eq!(DiskStorage::open_with_quota(&global, "example", 128).unwrap().get("blob").unwrap(), None);
        assert_eq!(DiskStorage::open_with_quota(&world, "other", 128).unwrap().get("blob").unwrap(), None);
        assert!(restored.delete("blob").unwrap());
        assert_eq!(DiskStorage::open_with_quota(&world, "example", 128).unwrap().get("blob").unwrap(), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn disk_storage_rejects_unsafe_names_and_failed_writes() {
        let root = temporary_root();
        for mod_id in ["../evil", "con", "com1", "Bad", "ab"] {
            assert!(DiskStorage::open(&root, mod_id).is_err());
        }
        let mut store = DiskStorage::open_with_quota(&root, "example", 128).unwrap();
        for key in ["../x", "C:/x", "/abs", "folder/..", "dir\\file", "CON", "nul.txt", "CON .txt", "COM¹", "lpt².txt", "CONIN$", "trail."] {
            assert!(store.put(key, b"value").is_err(), "accepted {key}");
        }
        let folder = root.join("yoake-data").join("example");
        std::fs::create_dir_all(folder.join("store.bin")).unwrap();
        assert!(store.put("key", b"value").is_err());
        assert_eq!(store.get("key").unwrap(), None);
        std::fs::remove_dir(folder.join("store.bin")).unwrap();
        std::fs::write(folder.join("store.bin"), b"broken").unwrap();
        assert!(DiskStorage::open(&root, "example").is_err());
        assert_eq!(std::fs::read(folder.join("store.bin")).unwrap(), b"broken");
        std::fs::remove_dir_all(root).unwrap();
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
