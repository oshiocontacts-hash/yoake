use std::collections::BTreeMap;
use std::io::{Read, Write};

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
    if key.is_empty() || key.len() > MAX_KEY_BYTES || key.chars().any(char::is_control) {
        Err("invalid storage key")
    } else {
        Ok(())
    }
}

fn entry_cost(key: &str, value_len: usize) -> usize {
    ENTRY_HEADER_BYTES.saturating_add(key.len()).saturating_add(value_len)
}

#[cfg(test)]
mod tests {
    use super::*;

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
