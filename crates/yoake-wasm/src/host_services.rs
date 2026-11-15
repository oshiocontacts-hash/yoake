use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

pub mod storage;
pub mod tickets;

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_TRANSFER_BYTES: usize = 64 * 1024 * 1024;
const MAX_IN_FLIGHT: usize = 64;
const MAX_TICKET_OWNERS: usize = 1024;
const MAX_TICKETS_PER_MOD: usize = 4096;

pub struct MessageRateLimit {
    window: Duration,
    max_messages: usize,
    max_bytes: usize,
    window_start: Option<Duration>,
    last_seen: Option<Duration>,
    messages: usize,
    bytes: usize,
}

impl MessageRateLimit {
    pub fn new(window: Duration, max_messages: usize, max_bytes: usize) -> Result<Self, &'static str> {
        if window.is_zero() || max_messages == 0 || max_bytes == 0 {
            return Err("rate limits must be positive");
        }
        Ok(Self {
            window,
            max_messages,
            max_bytes,
            window_start: None,
            last_seen: None,
            messages: 0,
            bytes: 0,
        })
    }

    pub fn try_accept(&mut self, now: Duration, bytes: usize) -> bool {
        if self.last_seen.is_some_and(|last| now < last) {
            return false;
        }
        self.last_seen = Some(now);
        if bytes > MAX_MESSAGE_BYTES {
            return false;
        }
        let expired = self.window_start.is_none_or(|start| now - start >= self.window);
        let used_messages = if expired { 0 } else { self.messages };
        let used_bytes = if expired { 0 } else { self.bytes };
        let Some(next_messages) = used_messages.checked_add(1) else { return false };
        let Some(next_bytes) = used_bytes.checked_add(bytes) else { return false };
        if next_messages > self.max_messages || next_bytes > self.max_bytes {
            return false;
        }
        if expired {
            self.window_start = Some(now);
        }
        self.messages = next_messages;
        self.bytes = next_bytes;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TransferProgress {
    pub received_bytes: usize,
    pub total_bytes: usize,
}

struct IncomingTransfer {
    data: Vec<u8>,
    received: Vec<bool>,
    received_bytes: usize,
}

pub struct IncomingTransfers {
    max_in_flight: usize,
    max_reserved_bytes: usize,
    reserved_bytes: usize,
    transfers: BTreeMap<u64, IncomingTransfer>,
}

impl IncomingTransfers {
    pub fn new(max_in_flight: usize, max_reserved_bytes: usize) -> Result<Self, &'static str> {
        if max_in_flight == 0 || max_in_flight > MAX_IN_FLIGHT
            || max_reserved_bytes == 0 || max_reserved_bytes > MAX_TRANSFER_BYTES
        {
            return Err("invalid transfer limits");
        }
        Ok(Self {
            max_in_flight,
            max_reserved_bytes,
            reserved_bytes: 0,
            transfers: BTreeMap::new(),
        })
    }

    pub fn start(&mut self, id: u64, total_bytes: usize) -> Result<TransferProgress, &'static str> {
        if total_bytes > MAX_TRANSFER_BYTES || self.transfers.contains_key(&id) {
            return Err("invalid transfer or duplicate id");
        }
        let reserved = self.reserved_bytes.checked_add(total_bytes).ok_or("transfer quota exceeded")?;
        if self.transfers.len() >= self.max_in_flight || reserved > self.max_reserved_bytes {
            return Err("transfer quota exceeded");
        }
        let chunks = total_bytes.div_ceil(MAX_MESSAGE_BYTES);
        let mut data = Vec::new();
        data.try_reserve_exact(total_bytes).map_err(|_| "transfer allocation failed")?;
        data.resize(total_bytes, 0);
        self.transfers.insert(id, IncomingTransfer {
            data,
            received: vec![false; chunks],
            received_bytes: 0,
        });
        self.reserved_bytes = reserved;
        Ok(TransferProgress { received_bytes: 0, total_bytes })
    }

    pub fn push_chunk(&mut self, id: u64, index: usize, bytes: &[u8]) -> Result<TransferProgress, &'static str> {
        let transfer = self.transfers.get_mut(&id).ok_or("unknown transfer")?;
        if index >= transfer.received.len() {
            return Err("invalid chunk index");
        }
        let start = index * MAX_MESSAGE_BYTES;
        let end = (start + MAX_MESSAGE_BYTES).min(transfer.data.len());
        if bytes.len() != end - start {
            return Err("invalid chunk length");
        }
        if transfer.received[index] {
            if &transfer.data[start..end] != bytes {
                return Err("conflicting repeated chunk");
            }
        } else {
            transfer.data[start..end].copy_from_slice(bytes);
            transfer.received[index] = true;
            transfer.received_bytes += bytes.len();
        }
        Ok(TransferProgress { received_bytes: transfer.received_bytes, total_bytes: transfer.data.len() })
    }

    pub fn missing_chunks(&self, id: u64) -> Result<Vec<usize>, &'static str> {
        let transfer = self.transfers.get(&id).ok_or("unknown transfer")?;
        Ok(transfer.received.iter().enumerate()
            .filter_map(|(index, received)| (!received).then_some(index))
            .collect())
    }

    pub fn finish(&mut self, id: u64) -> Result<Vec<u8>, &'static str> {
        let transfer = self.transfers.get(&id).ok_or("unknown transfer")?;
        if transfer.received_bytes != transfer.data.len() {
            return Err("transfer is incomplete");
        }
        let transfer = self.transfers.remove(&id).ok_or("unknown transfer")?;
        self.reserved_bytes -= transfer.data.len();
        Ok(transfer.data)
    }

    pub fn cancel(&mut self, id: u64) -> bool {
        if let Some(transfer) = self.transfers.remove(&id) {
            self.reserved_bytes -= transfer.data.len();
            true
        } else {
            false
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ChunkKey {
    pub dimension: String,
    pub x: i32,
    pub z: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketAcquired {
    AlreadyHeld,
    FirstModOwner,
    AdditionalModOwner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TicketReleased {
    NotHeld,
    RemainingModOwners,
    LastModOwner,
}

pub struct ChunkTickets {
    max_owners: usize,
    max_per_mod: usize,
    by_mod: BTreeMap<String, BTreeSet<ChunkKey>>,
    owners: BTreeMap<ChunkKey, usize>,
}

impl ChunkTickets {
    pub fn new(max_owners: usize, max_per_mod: usize) -> Result<Self, &'static str> {
        if max_owners == 0 || max_owners > MAX_TICKET_OWNERS
            || max_per_mod == 0 || max_per_mod > MAX_TICKETS_PER_MOD
        {
            return Err("invalid chunk ticket limits");
        }
        Ok(Self { max_owners, max_per_mod, by_mod: BTreeMap::new(), owners: BTreeMap::new() })
    }

    pub fn acquire(&mut self, mod_id: &str, chunk: ChunkKey) -> Result<TicketAcquired, &'static str> {
        if mod_id.is_empty() || mod_id.len() > 128 || chunk.dimension.is_empty() || chunk.dimension.len() > 256 {
            return Err("invalid chunk ticket owner or dimension");
        }
        if let Some(chunks) = self.by_mod.get(mod_id) {
            if chunks.contains(&chunk) {
                return Ok(TicketAcquired::AlreadyHeld);
            }
            if chunks.len() >= self.max_per_mod {
                return Err("mod chunk ticket quota exceeded");
            }
        } else if self.by_mod.len() >= self.max_owners {
            return Err("chunk ticket owner quota exceeded");
        }
        let count = self.owners.entry(chunk.clone()).or_insert(0);
        let status = if *count == 0 { TicketAcquired::FirstModOwner } else { TicketAcquired::AdditionalModOwner };
        *count += 1;
        self.by_mod.entry(mod_id.to_owned()).or_default().insert(chunk);
        Ok(status)
    }

    pub fn release(&mut self, mod_id: &str, chunk: &ChunkKey) -> TicketReleased {
        let Some(chunks) = self.by_mod.get_mut(mod_id) else { return TicketReleased::NotHeld };
        if !chunks.remove(chunk) {
            return TicketReleased::NotHeld;
        }
        if chunks.is_empty() {
            self.by_mod.remove(mod_id);
        }
        if self.owners.get(chunk) == Some(&1) {
            self.owners.remove(chunk);
            TicketReleased::LastModOwner
        } else {
            if let Some(count) = self.owners.get_mut(chunk) {
                *count -= 1;
            }
            TicketReleased::RemainingModOwners
        }
    }

    pub fn release_mod(&mut self, mod_id: &str) -> Vec<ChunkKey> {
        let chunks = self.by_mod.get(mod_id).cloned().unwrap_or_default();
        chunks.into_iter().filter(|chunk| {
            self.release(mod_id, chunk) == TicketReleased::LastModOwner
        }).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_limits_and_monotonic_windows() {
        let mut limit = MessageRateLimit::new(Duration::from_secs(2), 2, 5).unwrap();
        assert!(limit.try_accept(Duration::from_secs(1), 3));
        assert!(limit.try_accept(Duration::from_secs(2), 2));
        assert!(!limit.try_accept(Duration::from_secs(2), 0));
        assert!(!limit.try_accept(Duration::from_secs(1), 1));
        assert!(!limit.try_accept(Duration::from_secs(3), MAX_MESSAGE_BYTES + 1));
        assert!(limit.try_accept(Duration::from_secs(3), 5));
        assert!(!limit.try_accept(Duration::from_secs(3), 1));
        assert!(MessageRateLimit::new(Duration::ZERO, 1, 1).is_err());
    }

    #[test]
    fn rejected_messages_still_advance_the_monotonic_clock() {
        let mut limit = MessageRateLimit::new(Duration::from_secs(10), 2, 2).unwrap();
        assert!(limit.try_accept(Duration::from_secs(1), 1));
        assert!(!limit.try_accept(Duration::from_secs(2), MAX_MESSAGE_BYTES + 1));
        assert!(!limit.try_accept(Duration::from_millis(1500), 1));
        assert!(limit.try_accept(Duration::from_secs(2), 1));
    }

    #[test]
    fn chunks_can_arrive_out_of_order_and_retransmit() {
        let mut transfers = IncomingTransfers::new(2, MAX_MESSAGE_BYTES + 3).unwrap();
        assert_eq!(transfers.start(17, MAX_MESSAGE_BYTES + 3).unwrap().received_bytes, 0);
        assert_eq!(transfers.missing_chunks(17).unwrap(), vec![0, 1]);
        assert!(transfers.finish(17).is_err());
        assert_eq!(transfers.push_chunk(17, 1, b"end").unwrap().received_bytes, 3);
        assert_eq!(transfers.push_chunk(17, 1, b"end").unwrap().received_bytes, 3);
        assert!(transfers.push_chunk(17, 1, b"bad").is_err());
        assert!(transfers.push_chunk(17, 1, b"short").is_err());
        assert_eq!(transfers.missing_chunks(17).unwrap(), vec![0]);
        transfers.push_chunk(17, 0, &vec![7; MAX_MESSAGE_BYTES]).unwrap();
        let data = transfers.finish(17).unwrap();
        assert_eq!(data.len(), MAX_MESSAGE_BYTES + 3);
        assert_eq!(data[0], 7);
        assert_eq!(&data[MAX_MESSAGE_BYTES..], b"end");
        assert!(transfers.missing_chunks(17).is_err());
    }

    #[test]
    fn transfer_limits_and_cancellation() {
        let mut transfers = IncomingTransfers::new(1, 5).unwrap();
        transfers.start(3, 5).unwrap();
        assert!(transfers.start(3, 1).is_err());
        assert!(transfers.start(4, 1).is_err());
        assert!(transfers.push_chunk(3, 1, b"x").is_err());
        assert!(transfers.cancel(3));
        assert!(!transfers.cancel(3));
        assert!(transfers.start(4, 6).is_err());
        transfers.start(4, 0).unwrap();
        assert!(transfers.finish(4).unwrap().is_empty());
        assert!(IncomingTransfers::new(1, MAX_TRANSFER_BYTES + 1).is_err());
    }

    #[test]
    fn chunk_ticket_ownership_is_capped_and_shared() {
        let mut tickets = ChunkTickets::new(2, 1).unwrap();
        let first = ChunkKey { dimension: "minecraft:overworld".into(), x: 1, z: -2 };
        let second = ChunkKey { dimension: "minecraft:the_nether".into(), x: 1, z: -2 };
        assert_eq!(tickets.acquire("a", first.clone()).unwrap(), TicketAcquired::FirstModOwner);
        assert_eq!(tickets.acquire("a", first.clone()).unwrap(), TicketAcquired::AlreadyHeld);
        assert!(tickets.acquire("a", second.clone()).is_err());
        assert_eq!(tickets.acquire("b", first.clone()).unwrap(), TicketAcquired::AdditionalModOwner);
        assert!(tickets.acquire("c", second.clone()).is_err());
        assert_eq!(tickets.release("c", &first), TicketReleased::NotHeld);
        assert_eq!(tickets.release("a", &first), TicketReleased::RemainingModOwners);
        assert_eq!(tickets.release_mod("b"), vec![first]);
        assert_eq!(tickets.acquire("c", second.clone()).unwrap(), TicketAcquired::FirstModOwner);
        assert_eq!(tickets.release("c", &second), TicketReleased::LastModOwner);
        assert!(ChunkTickets::new(0, 1).is_err());
        assert!(ChunkTickets::new(1, MAX_TICKETS_PER_MOD + 1).is_err());
    }
}
