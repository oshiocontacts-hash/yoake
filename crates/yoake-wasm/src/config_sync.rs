use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use crate::package_services::MAX_SYNC_BYTES;

pub const CHANNEL: &str = "yoake:config_sync";
pub const MAX_TOTAL_BYTES: usize = 8 * MAX_SYNC_BYTES;
pub const MAX_MODS: usize = 256;
const CHUNK_BYTES: usize = 28_000;
const PACKETS_PER_TICK: usize = 4;

pub struct Snapshot {
    pub id: String,
    pub revision: u64,
    pub data: Arc<Vec<u8>>,
}

struct Transfer {
    id: String,
    revision: u64,
    data: Arc<Vec<u8>>,
    next: usize,
}

#[derive(Default)]
struct Recipient {
    sent: BTreeMap<String, u64>,
    active: Option<Transfer>,
}

#[derive(Default)]
pub struct Outbox {
    recipients: BTreeMap<u32, Recipient>,
}

impl Outbox {
    pub fn retain(&mut self, connections: &BTreeSet<u32>) {
        self.recipients
            .retain(|connection, _| connections.contains(connection));
    }

    pub fn packets(
        &mut self,
        connection: u32,
        snapshots: &[Snapshot],
    ) -> Result<Vec<Vec<u8>>, String> {
        let total = snapshots
            .iter()
            .try_fold(0usize, |total, snapshot| {
                if !valid_id(&snapshot.id) || snapshot.data.len() > MAX_SYNC_BYTES {
                    return None;
                }
                total.checked_add(snapshot.data.len())
            })
            .ok_or("invalid config snapshots")?;
        if snapshots.len() > MAX_MODS || total > MAX_TOTAL_BYTES {
            return Err("config sync aggregate limit exceeded".into());
        }
        if snapshots.is_empty() {
            return Ok(Vec::new());
        }
        let recipient = self.recipients.entry(connection).or_default();
        let mut packets = Vec::new();
        for _ in 0..PACKETS_PER_TICK {
            if recipient.active.is_none() {
                let Some(snapshot) = snapshots
                    .iter()
                    .find(|snapshot| recipient.sent.get(&snapshot.id) != Some(&snapshot.revision))
                else {
                    break;
                };
                recipient.active = Some(Transfer {
                    id: snapshot.id.clone(),
                    revision: snapshot.revision,
                    data: snapshot.data.clone(),
                    next: 0,
                });
            }
            let transfer = recipient.active.as_mut().ok_or("missing config transfer")?;
            let count = transfer.data.len().div_ceil(CHUNK_BYTES).max(1);
            let start = transfer.next * CHUNK_BYTES;
            let end = (start + CHUNK_BYTES).min(transfer.data.len());
            let mut packet = vec![1, transfer.id.len() as u8];
            packet.extend_from_slice(transfer.id.as_bytes());
            packet.extend_from_slice(&transfer.revision.to_le_bytes());
            packet.extend_from_slice(&(transfer.next as u16).to_le_bytes());
            packet.extend_from_slice(&(count as u16).to_le_bytes());
            packet.extend_from_slice(&(transfer.data.len() as u32).to_le_bytes());
            packet.extend_from_slice(&transfer.data[start..end]);
            packets.push(packet);
            transfer.next += 1;
            if transfer.next == count {
                recipient
                    .sent
                    .insert(transfer.id.clone(), transfer.revision);
                recipient.active = None;
            }
        }
        Ok(packets)
    }
}

struct Partial {
    id: String,
    revision: u64,
    total: usize,
    next: usize,
    data: Vec<u8>,
}

#[derive(Default)]
pub struct Inbox {
    active: Option<Partial>,
}

impl Inbox {
    pub fn receive(&mut self, packet: &[u8]) -> Result<Option<(String, Vec<u8>)>, String> {
        let result = self.receive_inner(packet);
        if result.is_err() {
            self.active = None;
        }
        result
    }

    fn receive_inner(&mut self, packet: &[u8]) -> Result<Option<(String, Vec<u8>)>, String> {
        let id_len = usize::from(*packet.get(1).ok_or("short config packet")?);
        let header = 18 + id_len;
        if packet.first() != Some(&1)
            || packet.len() < header
            || packet.len() > header + CHUNK_BYTES
        {
            return Err("invalid config packet".into());
        }
        let id = std::str::from_utf8(&packet[2..2 + id_len]).map_err(|_| "invalid mod id")?;
        if !valid_id(id) {
            return Err("invalid mod id".into());
        }
        let meta = &packet[2 + id_len..header];
        let revision = u64::from_le_bytes(meta[..8].try_into().map_err(|_| "short revision")?);
        let index = usize::from(u16::from_le_bytes([meta[8], meta[9]]));
        let count = usize::from(u16::from_le_bytes([meta[10], meta[11]]));
        let total =
            u32::from_le_bytes(meta[12..16].try_into().map_err(|_| "short length")?) as usize;
        let payload = &packet[header..];
        if total > MAX_SYNC_BYTES || count != total.div_ceil(CHUNK_BYTES).max(1) || index >= count {
            return Err("invalid config transfer size".into());
        }
        let expected = total.saturating_sub(index * CHUNK_BYTES).min(CHUNK_BYTES);
        if payload.len() != expected {
            return Err("invalid config chunk size".into());
        }
        if index == 0 {
            self.active = Some(Partial {
                id: id.into(),
                revision,
                total,
                next: 0,
                data: Vec::with_capacity(total),
            });
        }
        let active = self.active.as_mut().ok_or("config transfer not started")?;
        if active.id != id
            || active.revision != revision
            || active.total != total
            || active.next != index
        {
            return Err("out of order config packet".into());
        }
        active.data.extend_from_slice(payload);
        active.next += 1;
        if active.next == count {
            return Ok(self.active.take().map(|active| (active.id, active.data)));
        }
        Ok(None)
    }
}

fn valid_id(id: &str) -> bool {
    (3..=48).contains(&id.len())
        && id.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_changes_rejoin_and_bounded_delivery() {
        let mut outbox = Outbox::default();
        let mut inbox = Inbox::default();
        let mut snapshots = vec![Snapshot {
            id: "example".into(),
            revision: 0,
            data: Arc::new(vec![23; MAX_SYNC_BYTES]),
        }];
        let mut completed = None;
        for _ in 0..10 {
            let packets = outbox.packets(1, &snapshots).unwrap();
            assert!(packets.len() <= 4);
            for packet in packets {
                assert!(packet.len() < 29_000);
                completed = inbox.receive(&packet).unwrap().or(completed);
            }
        }
        assert_eq!(
            completed.unwrap(),
            ("example".into(), vec![23; MAX_SYNC_BYTES])
        );
        assert!(outbox.packets(1, &snapshots).unwrap().is_empty());
        snapshots[0].revision = 1;
        snapshots[0].data = Arc::new(b"changed".to_vec());
        assert_eq!(
            inbox
                .receive(&outbox.packets(1, &snapshots).unwrap()[0])
                .unwrap()
                .unwrap()
                .1,
            b"changed"
        );
        outbox.retain(&BTreeSet::new());
        assert_eq!(outbox.packets(1, &snapshots).unwrap().len(), 1);
        assert_eq!(outbox.packets(2, &snapshots).unwrap().len(), 1);
    }

    #[test]
    fn rejects_corrupt_oversized_and_out_of_order_packets() {
        let mut outbox = Outbox::default();
        let snapshots = [Snapshot {
            id: "example".into(),
            revision: 0,
            data: Arc::new(vec![0; CHUNK_BYTES + 1]),
        }];
        let packets = outbox.packets(1, &snapshots).unwrap();
        let mut inbox = Inbox::default();
        assert!(inbox.receive(&packets[1]).is_err());
        inbox.receive(&packets[0]).unwrap();
        let mut corrupt = packets[1].clone();
        corrupt[17] = 255;
        assert!(inbox.receive(&corrupt).is_err());
        assert!(inbox.receive(&packets[1]).is_err());
        for length in 0..packets[0].len() {
            assert!(inbox.receive(&packets[0][..length]).is_err());
        }
        let snapshots = [Snapshot {
            id: "example".into(),
            revision: 0,
            data: Arc::new(vec![0; MAX_SYNC_BYTES + 1]),
        }];
        assert!(outbox.packets(2, &snapshots).is_err());
        assert!(outbox.recipients.get(&2).is_none());
    }
}
