//! Native identities, exchanged only after both peers advertise support.
use super::{Entry, Side};
use crate::{
    Error, Result,
    buf::{Reader, Writer},
};
use sha2::{Digest, Sha256};
use std::sync::OnceLock;

pub const CHANNEL: &str = "horizon:native_mods";
pub const CAPABILITIES: &[u8] = &[1, 0];
const SNAPSHOT: &[u8] = &[1, 1];
const MAX_LIST_BYTES: i32 = 128 * 1024;

/// Build-time identity for one native bundle, including mixin-only bundles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mod {
    pub id: &'static str,
    pub version: &'static str,
    pub side: Side,
}

static LOCAL: OnceLock<Vec<Entry>> = OnceLock::new();

pub fn install(mods: &'static [Mod]) -> Result<()> {
    let entries: Vec<_> = mods
        .iter()
        .map(|native| Entry {
            id: native.id.into(),
            version: native.version.into(),
            sha256: None,
            side: native.side,
        })
        .collect();
    validate(&entries, true)?;
    LOCAL
        .set(entries)
        .map_err(|_| Error::new("native mod identities already installed"))
}

pub fn local() -> &'static [Entry] {
    LOCAL.get().map(Vec::as_slice).unwrap_or_default()
}

pub fn has_shared(entries: &[Entry]) -> bool {
    entries.iter().any(|entry| entry.side == Side::Both)
}

fn validate(entries: &[Entry], native: bool) -> Result<()> {
    if entries.len() > super::MAX_ENTRIES {
        return Err(Error::new("native mod check: too many entries"));
    }
    let mut ids = std::collections::HashSet::new();
    for entry in entries {
        if [&entry.id, &entry.version].iter().any(|text| {
            text.is_empty()
                || text.chars().count() > super::MAX_TEXT
                || text.chars().any(char::is_control)
        }) || !ids.insert(&entry.id)
            || (native && entry.sha256.is_some())
        {
            return Err(Error::new(
                "native mod check: invalid or duplicate identity",
            ));
        }
    }
    Ok(())
}

/// SHA-256 of the count and length-prefixed, sorted shared native id/version pairs.
pub fn digest(entries: &[Entry]) -> [u8; 32] {
    let mut entries = super::shared(entries);
    entries.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.version.cmp(&right.version))
    });
    let mut writer = Writer::new();
    writer.varint(entries.len() as i32);
    for entry in entries {
        writer.string(&entry.id);
        writer.string(&entry.version);
    }
    Sha256::digest(&writer.data).into()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub content: Vec<Entry>,
    pub native: Vec<Entry>,
    pub digest: [u8; 32],
}

impl Snapshot {
    pub fn new(content: &[Entry], native: &[Entry]) -> Result<Self> {
        let content = super::shared(content);
        let mut native = super::shared(native);
        validate(&content, false)?;
        validate(&native, true)?;
        native.sort_by(|left, right| left.id.cmp(&right.id));
        let digest = digest(&native);
        Ok(Self {
            content,
            native,
            digest,
        })
    }
}

#[derive(Default)]
pub struct Negotiation {
    supported: bool,
}

impl Negotiation {
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

    // CHECK: Identity-bearing data is never sent before the peer advertises this version.
    pub fn payload(&self, snapshot: &Snapshot) -> Option<Vec<u8>> {
        self.supported.then(|| encode(snapshot))
    }
}

fn encode(snapshot: &Snapshot) -> Vec<u8> {
    let mut writer = Writer::new();
    writer.bytes(SNAPSHOT);
    for entries in [&snapshot.content, &snapshot.native] {
        let list = super::encode(entries);
        writer.varint(list.len() as i32);
        writer.bytes(&list);
    }
    writer.bytes(&snapshot.digest);
    writer.data
}

pub fn decode(data: &[u8]) -> Result<Snapshot> {
    let mut reader = Reader::new(data);
    if reader.bytes(SNAPSHOT.len())? != SNAPSHOT {
        return Err(Error::new("native mod check: unsupported snapshot"));
    }
    let mut read_list = || -> Result<Vec<Entry>> {
        let length = reader.varint()?;
        if !(0..=MAX_LIST_BYTES).contains(&length) {
            return Err(Error::new("native mod check: list too large"));
        }
        super::decode(reader.bytes(length as usize)?)
    };
    let content = read_list()?;
    let native = read_list()?;
    let digest = reader.bytes(32)?;
    if reader.remaining() != 0
        || content
            .iter()
            .chain(&native)
            .any(|entry| entry.side != Side::Both)
    {
        return Err(Error::new("native mod check: invalid shared snapshot"));
    }
    let snapshot = Snapshot::new(&content, &native)?;
    if digest != snapshot.digest {
        return Err(Error::new("native mod check: pack digest differs"));
    }
    Ok(snapshot)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Difference {
    pub content: super::Difference,
    pub native: super::Difference,
    pub server_digest: [u8; 32],
    pub client_digest: [u8; 32],
}

pub fn compare(server: &Snapshot, client: &Snapshot) -> Difference {
    Difference {
        content: super::compare(&server.content, &client.content),
        native: super::compare(&server.native, &client.native),
        server_digest: server.digest,
        client_digest: client.digest,
    }
}

impl Difference {
    pub fn is_empty(&self) -> bool {
        self.content.is_empty() && self.native.is_empty()
    }

    pub fn reason(&self) -> String {
        let short = |digest: &[u8; 32]| -> String {
            digest[..6]
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        };
        let mut reasons = Vec::new();
        if !self.native.is_empty() {
            reasons.push(format!(
                "Different Yoake pack (here {}, server {}). {}",
                short(&self.client_digest),
                short(&self.server_digest),
                self.native.reason()
            ));
        }
        if !self.content.is_empty() {
            reasons.push(self.content.reason());
        }
        reasons.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, version: &str, side: Side) -> Entry {
        Entry {
            id: id.into(),
            version: version.into(),
            sha256: None,
            side,
        }
    }

    fn shared(id: &str, version: &str) -> Entry {
        entry(id, version, Side::Both)
    }

    fn payload(snapshot: &Snapshot) -> Vec<u8> {
        let mut negotiation = Negotiation::default();
        assert!(negotiation.receive_capabilities(CAPABILITIES));
        negotiation.payload(snapshot).unwrap()
    }

    #[test]
    fn legacy_packets_are_byte_identical_without_shared_native_mods() {
        let content = vec![shared("x", "1")];
        let legacy = vec![1, 1, b'x', 1, b'1', 0, 0];
        assert_eq!(super::super::encode(&content), legacy);
        assert_eq!(super::super::decode(&legacy).unwrap(), content);
        assert_eq!(super::super::encode(&[]), vec![0]);
        for native in [
            Vec::new(),
            vec![entry("minimap", "1", Side::Client)],
            vec![entry("admin", "1", Side::Server)],
        ] {
            assert!(!has_shared(&native));
            let snapshot = Snapshot::new(&content, &native).unwrap();
            assert!(snapshot.native.is_empty());
            assert!(Negotiation::default().payload(&snapshot).is_none());
            assert_eq!(super::super::encode(&snapshot.content), legacy);
        }
    }

    #[test]
    fn snapshots_require_supported_capabilities_and_round_trip() {
        for native in [Vec::new(), vec![shared("kigurumi", "0.4.1")]] {
            let snapshot = Snapshot::new(&[], &native).unwrap();
            let mut negotiation = Negotiation::default();
            assert!(!negotiation.receive_capabilities(&[2, 0]));
            assert!(!negotiation.supported());
            assert!(negotiation.payload(&snapshot).is_none());
            assert!(negotiation.receive_capabilities(CAPABILITIES));
            let encoded = negotiation.payload(&snapshot).unwrap();
            assert_eq!(decode(&encoded).unwrap(), snapshot);
            assert!(super::super::decode(&encoded).is_err());
        }
    }

    #[test]
    fn content_and_native_namespaces_and_content_hashes_are_preserved() {
        let mut content = shared("kagerou", "3");
        content.sha256 = Some([7; 32]);
        let snapshot = Snapshot::new(&[content], &[shared("kagerou", "4")]).unwrap();
        assert_eq!(decode(&payload(&snapshot)).unwrap(), snapshot);
        assert_eq!(snapshot.content[0].sha256, Some([7; 32]));
        assert_eq!(snapshot.native[0].version, "4");
    }

    #[test]
    fn same_pack_ignores_order_and_keeps_private_mods_off_the_wire() {
        let server = Snapshot::new(
            &[],
            &[
                shared("kigurumi", "0.4.1"),
                entry("private_admin", "1", Side::Server),
                shared("animagic", "2"),
            ],
        )
        .unwrap();
        let client = Snapshot::new(
            &[],
            &[
                shared("animagic", "2"),
                shared("kigurumi", "0.4.1"),
                entry("private_minimap", "9", Side::Client),
            ],
        )
        .unwrap();
        assert_eq!(server.digest, client.digest);
        assert!(compare(&server, &client).is_empty());
        assert_eq!(payload(&server), payload(&client));
        assert!(
            !payload(&server)
                .windows(b"private_admin".len())
                .any(|window| window == b"private_admin")
        );
        assert!(
            !payload(&client)
                .windows(b"private_minimap".len())
                .any(|window| window == b"private_minimap")
        );
    }

    #[test]
    fn a_version_mismatch_names_both_versions_and_the_pack() {
        let server = Snapshot::new(&[], &[shared("kigurumi", "0.4.2")]).unwrap();
        let client = Snapshot::new(&[], &[shared("kigurumi", "0.4.1")]).unwrap();
        let difference = compare(&server, &client);
        assert!(!difference.is_empty());
        assert_ne!(server.digest, client.digest);
        let reason = difference.reason();
        assert!(reason.starts_with("Different Yoake pack (here "));
        assert!(reason.contains(", server "));
        assert!(reason.ends_with(
            "Your mods don't match this server's. Different version or file: kigurumi (server 0.4.2, yours 0.4.1)"
        ));
    }

    #[test]
    fn missing_mods_are_refused_in_both_directions() {
        let pack = Snapshot::new(&[], &[shared("animagic", "1")]).unwrap();
        let empty = Snapshot::new(&[], &[]).unwrap();
        assert!(
            compare(&pack, &empty)
                .reason()
                .ends_with("Missing: animagic 1")
        );
        assert!(
            compare(&empty, &pack)
                .reason()
                .ends_with("Not on this server: animagic 1")
        );
        for side in [Side::Client, Side::Server] {
            let private = Snapshot::new(&[], &[entry("animagic", "1", side)]).unwrap();
            assert!(compare(&private, &empty).is_empty());
            assert!(compare(&empty, &private).is_empty());
            assert!(!compare(&pack, &private).is_empty());
            assert!(!compare(&private, &pack).is_empty());
        }
    }

    #[test]
    fn content_mismatches_still_use_the_existing_refusal() {
        let mut server_content = shared("ruby", "1");
        server_content.sha256 = Some([1; 32]);
        let mut client_content = server_content.clone();
        client_content.sha256 = Some([2; 32]);
        let native = [shared("kagerou", "1")];
        let server = Snapshot::new(&[server_content], &native).unwrap();
        let client = Snapshot::new(&[client_content], &native).unwrap();
        let difference = compare(&server, &client);
        assert!(!difference.is_empty());
        assert_eq!(
            difference.reason(),
            "Your mods don't match this server's. Different version or file: ruby 1 (your file was changed)"
        );
    }

    #[test]
    fn digest_frames_identity_boundaries() {
        assert_ne!(digest(&[shared("ab", "c")]), digest(&[shared("a", "bc")]));
    }

    #[test]
    fn invalid_identities_and_malformed_snapshots_are_rejected() {
        assert!(Snapshot::new(&[], &[shared("x", "1"), shared("x", "2")]).is_err());
        assert!(Snapshot::new(&[], &[shared("", "1")]).is_err());
        assert!(Snapshot::new(&[], &[shared("x", "\n")]).is_err());
        assert!(Snapshot::new(&[], &[shared(&"x".repeat(65), "1")]).is_err());
        assert!(Snapshot::new(&[], &[shared("x", &"1".repeat(65))]).is_err());
        let snapshot = Snapshot::new(&[], &[shared("x", "1")]).unwrap();
        let encoded = payload(&snapshot);
        for length in 0..encoded.len() {
            assert!(decode(&encoded[..length]).is_err());
        }
        let mut corrupt = encoded.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(decode(&corrupt).is_err());
        let mut trailing = encoded.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_err());
        let mut unsupported = encoded;
        unsupported[0] = 2;
        assert!(decode(&unsupported).is_err());
        let mut oversized = Writer::new();
        oversized.bytes(SNAPSHOT);
        oversized.varint(MAX_LIST_BYTES + 1);
        assert!(decode(&oversized.data).is_err());
    }

    #[test]
    fn private_or_hashed_native_wire_entries_are_rejected() {
        for side in [Side::Client, Side::Server, Side::Both] {
            let mut native = entry("secret", "1", side);
            native.sha256 = Some([1; 32]);
            let snapshot = Snapshot {
                content: Vec::new(),
                digest: digest(&[native.clone()]),
                native: vec![native],
            };
            assert!(decode(&payload(&snapshot)).is_err());
        }
    }
}
