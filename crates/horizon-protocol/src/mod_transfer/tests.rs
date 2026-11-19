use super::*;
use std::collections::HashMap;

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn entry(id: &str, bytes: &[u8]) -> OfferEntry {
    OfferEntry {
        id: id.into(),
        version: "1.0.0".into(),
        sha256: sha(bytes),
        size: bytes.len() as u64,
        full_access: true,
        side_client: true,
        catalog_id: None,
    }
}

/// The host's files in memory; `lie` serves other bytes than the offered ones (a changed file).
#[derive(Default)]
struct Files {
    files: HashMap<[u8; 32], Vec<u8>>,
    lie: HashMap<[u8; 32], Vec<u8>>,
    open: Option<[u8; 32]>,
    opened: usize,
}

impl Source for Files {
    fn open(&mut self, entry: &OfferEntry) -> std::io::Result<()> {
        self.opened += 1;
        if !self.files.contains_key(&entry.sha256) {
            return Err(std::io::ErrorKind::NotFound.into());
        }
        self.open = Some(entry.sha256);
        Ok(())
    }

    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> std::io::Result<usize> {
        let key = self.open.unwrap();
        let bytes = self.files.get(&key).map(|honest| self.lie.get(&key).unwrap_or(honest)).unwrap();
        let start = (offset as usize).min(bytes.len());
        let end = (start + buf.len()).min(bytes.len());
        buf[..end - start].copy_from_slice(&bytes[start..end]);
        Ok(end - start)
    }

    fn close(&mut self) {
        self.open = None;
    }
}

#[derive(Default)]
struct Staging {
    parts: HashMap<[u8; 32], Vec<u8>>,
    done: HashMap<[u8; 32], Vec<u8>>,
    current: Option<[u8; 32]>,
    discarded: Vec<[u8; 32]>,
}

impl Sink for Staging {
    fn begin(&mut self, entry: &OfferEntry) -> std::io::Result<()> {
        self.parts.insert(entry.sha256, Vec::new());
        self.current = Some(entry.sha256);
        Ok(())
    }

    fn write(&mut self, data: &[u8]) -> std::io::Result<()> {
        self.parts.get_mut(&self.current.unwrap()).unwrap().extend_from_slice(data);
        Ok(())
    }

    fn commit(&mut self, entry: &OfferEntry) -> std::io::Result<()> {
        let part = self.parts.remove(&entry.sha256).unwrap();
        self.done.insert(entry.sha256, part);
        Ok(())
    }

    fn discard(&mut self, entry: &OfferEntry) {
        self.parts.remove(&entry.sha256);
        self.discarded.push(entry.sha256);
    }
}

fn host(files: &[(&str, &[u8])]) -> HostSession<Files> {
    let entries = files.iter().map(|(id, bytes)| entry(id, bytes)).collect();
    let mut source = Files::default();
    for (_, bytes) in files {
        source.files.insert(sha(bytes), bytes.to_vec());
    }
    HostSession::new(Offer::new(entries).unwrap(), source)
}

fn host_with(bytes: &[u8]) -> HostSession<Files> {
    host(&[("m", bytes)])
}

/// Runs a client against the host in memory until the client stops asking.
fn run(host: &mut HostSession<Files>, enabled: bool, has: impl Fn(&OfferEntry) -> bool) -> (SyncState, Staging) {
    let (mut sync, mut step) = ClientSync::start(&host.offer().encode(), enabled, has, Staging::default());
    let mut rounds = 0;
    while matches!(step.state, SyncState::Fetching { .. }) {
        rounds += 1;
        assert!(rounds < 1000, "stalled");
        let mut replies = Vec::new();
        for payload in std::mem::take(&mut step.send) {
            match host.handle(&payload).unwrap() {
                HostEvent::Reply(more) => replies.extend(more),
                HostEvent::Streaming => {
                    while let Some(chunk) = host.next_chunk() {
                        replies.push(chunk);
                    }
                }
                HostEvent::Idle | HostEvent::Finished => {}
            }
        }
        for payload in replies {
            let next = sync.on_message(&payload);
            step.send.extend(next.send);
            step.state = next.state;
            if !matches!(step.state, SyncState::Fetching { .. }) {
                break;
            }
        }
    }
    (step.state, sync.into_sink())
}

#[test]
fn capabilities_gate_the_offer_and_old_peers_hear_nothing() {
    let offer = Offer::new(vec![entry("a", b"abc")]).unwrap();
    let mut negotiation = Negotiation::default();
    assert!(negotiation.offer_payload(&offer).is_none());
    // an old peer answers with something else, or nothing at all
    assert!(!negotiation.receive_capabilities(&[0, 2]));
    assert!(!negotiation.receive_capabilities(&[1, 0]));
    assert!(!negotiation.receive_capabilities(&[]));
    assert!(!negotiation.supported());
    assert!(negotiation.offer_payload(&offer).is_none());
    assert!(negotiation.receive_capabilities(CAPABILITIES));
    let payload = negotiation.offer_payload(&offer).unwrap();
    assert_eq!(Message::decode(&payload).unwrap(), Message::Offer(offer));
    assert_eq!(CHANNEL, "yoake:mod_transfer");
    // the channel is not one of the old exchange's
    assert_ne!(CHANNEL, crate::mod_list::CHANNEL);
    assert_ne!(CHANNEL, crate::mod_list::native::CHANNEL);
    assert_ne!(CHANNEL, crate::mod_net::CHANNEL);
}

#[test]
fn messages_round_trip_and_reject_garbage() {
    let mut full = entry("a", b"abc");
    full.catalog_id = Some("catalog-a".into());
    full.side_client = false;
    let offer = Offer::new(vec![full, entry("b", b"defg")]).unwrap();
    assert_eq!(Message::decode(&offer.encode()).unwrap(), Message::Offer(offer.clone()));
    let hash = sha(b"abc");
    assert_eq!(Message::decode(&request(&hash)).unwrap(), Message::Request(hash));
    assert_eq!(Message::decode(&done()).unwrap(), Message::Done);
    assert_eq!(Message::decode(CAPABILITIES).unwrap(), Message::Capabilities(1));
    assert_eq!(
        Message::decode(&chunk(&hash, 7, true, b"xyz")).unwrap(),
        Message::Chunk { sha256: hash, offset: 7, last: true, data: b"xyz".to_vec() }
    );
    assert_eq!(
        Message::decode(&refuse(&hash, RefuseReason::Busy)).unwrap(),
        Message::Refuse { sha256: hash, reason: RefuseReason::Busy }
    );
    let encoded = offer.encode();
    for length in 0..encoded.len() {
        assert!(Message::decode(&encoded[..length]).is_err(), "truncated at {length}");
    }
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(Message::decode(&trailing).is_err());
    assert!(Message::decode(&[9]).is_err());
    assert!(Message::decode(&[KIND_REQUEST, 1, 2]).is_err());
    // a chunk bigger than the cap is not decoded
    assert!(Message::decode(&chunk(&hash, 0, false, &vec![0; MAX_CHUNK + 1])).is_err());
    assert!(Message::decode(&chunk(&hash, 0, false, &vec![0; MAX_CHUNK])).is_ok());
}

#[test]
fn a_transfer_happy_path_streams_chunks_with_a_final_marker() {
    let big: Vec<u8> = (0..MAX_CHUNK * 2 + 1000).map(|i| (i % 251) as u8).collect();
    let small = b"tiny mod".to_vec();
    let mut both = host(&[("big_mod", &big), ("small_mod", &small)]);
    let (state, staging) = run(&mut both, true, |_| false);
    let SyncState::Complete(staged) = state else { panic!("{state:?}") };
    assert_eq!(staged.len(), 2);
    assert_eq!(staging.done[&sha(&big)], big);
    assert_eq!(staging.done[&sha(&small)], small);
    assert!(staging.parts.is_empty() && staging.discarded.is_empty());
    assert!(!both.transferring());
    // the chunk sizes and the marker on the wire
    let mut one = host_with(&big);
    assert_eq!(one.handle(&request(&sha(&big))).unwrap(), HostEvent::Streaming);
    let mut seen = Vec::new();
    while let Some(payload) = one.next_chunk() {
        let Message::Chunk { offset, last, data, .. } = Message::decode(&payload).unwrap() else { panic!() };
        assert!(data.len() <= MAX_CHUNK);
        seen.push((offset, data.len(), last));
    }
    assert_eq!(seen, vec![(0, MAX_CHUNK, false), (MAX_CHUNK as u64, MAX_CHUNK, false), (2 * MAX_CHUNK as u64, 1000, true)]);
}

#[test]
fn nothing_is_fetched_when_the_player_has_it_or_syncing_is_off_or_it_is_not_for_the_client() {
    let mut server_only = entry("srv", b"server only");
    server_only.side_client = false;
    let have = entry("have", b"already here");
    let mut files = Files::default();
    files.files.insert(have.sha256, b"already here".to_vec());
    let mut session = HostSession::new(Offer::new(vec![server_only, have]).unwrap(), files);
    let (state, staging) = run(&mut session, true, |e| e.id == "have");
    assert_eq!(state, SyncState::NothingNeeded);
    assert!(staging.done.is_empty() && staging.parts.is_empty());
    // syncing off: the same list, a missing mod, and still no request
    let mut off = host_with(b"missing");
    let (sync, step) = ClientSync::start(&off.offer().encode(), false, |_| false, Staging::default());
    assert_eq!(step, Step { send: vec![done()], state: SyncState::NothingNeeded });
    assert!(sync.wanted().is_empty());
    assert_eq!(off.handle(&done()).unwrap(), HostEvent::Finished);
}

#[test]
fn a_sha256_mismatch_aborts_and_deletes_the_partial_file() {
    let real = b"the real bytes of the mod".to_vec();
    let mut session = host_with(&real);
    // same length, other content: the host serves a changed file
    session.source.lie.insert(sha(&real), b"the fake bytes of the mod".to_vec());
    let (state, staging) = run(&mut session, true, |_| false);
    let SyncState::Failed(text) = state else { panic!("{state:?}") };
    assert!(text.contains("SHA-256 differs") && text.contains("deleted"), "{text}");
    assert!(staging.done.is_empty() && staging.parts.is_empty());
    assert_eq!(staging.discarded, vec![sha(&real)]);
}

#[test]
fn a_host_that_sends_too_much_or_out_of_order_is_cut_off() {
    let bytes = b"0123456789".to_vec();
    let hash = sha(&bytes);
    let offer = Offer::new(vec![entry("m", &bytes)]).unwrap();
    let start = || ClientSync::start(&offer.encode(), true, |_| false, Staging::default());
    // more than the announced size
    let (mut sync, _) = start();
    assert!(matches!(sync.on_message(&chunk(&hash, 0, false, &[1; 11])).state, SyncState::Failed(_)));
    assert!(sync.into_sink().parts.is_empty());
    // wrong offset
    let (mut sync, _) = start();
    assert!(matches!(sync.on_message(&chunk(&hash, 3, false, b"abc")).state, SyncState::Failed(_)));
    // ends early
    let (mut sync, _) = start();
    assert!(matches!(sync.on_message(&chunk(&hash, 0, true, b"abc")).state, SyncState::Failed(_)));
    // another file's chunk, a refusal, noise
    let (mut sync, _) = start();
    assert!(matches!(sync.on_message(&chunk(&[7; 32], 0, false, b"abc")).state, SyncState::Failed(_)));
    let (mut sync, _) = start();
    let SyncState::Failed(text) = sync.on_message(&refuse(&hash, RefuseReason::NotOffered)).state else { panic!() };
    assert!(text.contains("does not offer"));
    let (mut sync, _) = start();
    assert!(matches!(sync.on_message(&[99]).state, SyncState::Failed(_)));
    // once failed, nothing revives it, and no file was kept
    let (mut sync, _) = start();
    sync.on_message(&chunk(&hash, 5, false, b"x"));
    assert!(matches!(sync.on_message(&chunk(&hash, 0, true, &bytes)).state, SyncState::Failed(_)));
    assert!(sync.into_sink().done.is_empty());
}

#[test]
fn the_host_refuses_a_sha256_it_did_not_offer() {
    let mut session = host_with(b"offered");
    let other = sha(b"a file on the host's disk that is not offered");
    let HostEvent::Reply(replies) = session.handle(&request(&other)).unwrap() else { panic!() };
    assert_eq!(Message::decode(&replies[0]).unwrap(), Message::Refuse { sha256: other, reason: RefuseReason::NotOffered });
    // the source was never even asked to open anything
    assert_eq!(session.source.opened, 0);
    assert!(!session.transferring());
    // a client message that is not a request is a protocol error, not a file
    assert!(session.handle(&chunk(&other, 0, true, b"x")).is_err());
    assert!(session.handle(&[]).is_err());
    assert!(session.handle(&[KIND_REQUEST, 1]).is_err());
}

#[test]
fn one_transfer_at_a_time_and_the_per_join_limits() {
    let mut session = host(&[("a", b"aaaa"), ("b", b"bbbb")]);
    assert_eq!(session.handle(&request(&sha(b"aaaa"))).unwrap(), HostEvent::Streaming);
    let HostEvent::Reply(busy) = session.handle(&request(&sha(b"bbbb"))).unwrap() else { panic!() };
    assert_eq!(Message::decode(&busy[0]).unwrap(), Message::Refuse { sha256: sha(b"bbbb"), reason: RefuseReason::Busy });
    assert!(session.transferring());
    while session.next_chunk().is_some() {}
    assert_eq!(session.handle(&request(&sha(b"bbbb"))).unwrap(), HostEvent::Streaming);
    while session.next_chunk().is_some() {}
    // a file read that dies mid-way ends with a refusal
    let mut broken = host_with(b"abcdef");
    assert_eq!(broken.handle(&request(&sha(b"abcdef"))).unwrap(), HostEvent::Streaming);
    broken.source.files.insert(sha(b"abcdef"), b"abc".to_vec());
    let payload = broken.next_chunk().unwrap();
    assert_eq!(Message::decode(&payload).unwrap(), Message::Refuse { sha256: sha(b"abcdef"), reason: RefuseReason::Unavailable });
    assert!(!broken.transferring() && broken.next_chunk().is_none());
    // 64 requests, then the host says no (the same small mod over and over)
    let mut again = host_with(b"x");
    for _ in 0..MAX_MODS {
        assert_eq!(again.handle(&request(&sha(b"x"))).unwrap(), HostEvent::Streaming);
        while again.next_chunk().is_some() {}
    }
    let HostEvent::Reply(limit) = again.handle(&request(&sha(b"x"))).unwrap() else { panic!() };
    assert_eq!(Message::decode(&limit[0]).unwrap(), Message::Refuse { sha256: sha(b"x"), reason: RefuseReason::Limit });
}

#[test]
fn the_caps_hold_on_both_sides() {
    let too_many: Vec<_> = (0..=MAX_MODS).map(|i| entry(&format!("m{i}"), format!("bytes {i}").as_bytes())).collect();
    assert!(Offer::new(too_many.clone()).is_err());
    assert!(Offer::new(too_many[..MAX_MODS].to_vec()).is_ok());
    let mut huge = entry("huge", b"x");
    huge.size = MAX_MOD_BYTES + 1;
    assert!(Offer::new(vec![huge.clone()]).is_err());
    huge.size = MAX_MOD_BYTES;
    assert!(Offer::new(vec![huge]).is_ok());
    let mut empty = entry("empty", b"x");
    empty.size = 0;
    assert!(Offer::new(vec![empty]).is_err());
    assert!(Offer::new(vec![entry("a", b"1"), entry("a", b"2")]).is_err(), "duplicate id");
    assert!(Offer::new(vec![entry("a", b"1"), entry("b", b"1")]).is_err(), "duplicate hash");
    assert!(Offer::new(vec![entry("", b"1")]).is_err());
    assert!(Offer::new(vec![entry(&"x".repeat(MAX_TEXT + 1), b"1")]).is_err());
    // a hand-built payload over the caps is refused by the client's decoder
    let mut w = Writer::new();
    w.u8(KIND_LIST);
    w.u8(LIST_VERSION);
    w.varint(MAX_MODS as i32 + 1);
    assert!(Message::decode(&w.data).is_err());
    // a plan over 256 MiB in all (five 64 MiB mods) is refused before any request
    let big: Vec<_> = (0..5)
        .map(|i| {
            let mut e = entry(&format!("big{i}"), format!("{i}").as_bytes());
            e.size = MAX_MOD_BYTES;
            e
        })
        .collect();
    let offer = Offer::new(big).unwrap();
    let (_, step) = ClientSync::start(&offer.encode(), true, |_| false, Staging::default());
    let SyncState::Failed(text) = step.state else { panic!() };
    assert!(text.contains("too big"), "{text}");
    assert!(step.send.is_empty());
    // four fit exactly
    let four = Offer::new(offer.entries()[..4].to_vec()).unwrap();
    let (_, step) = ClientSync::start(&four.encode(), true, |_| false, Staging::default());
    assert!(matches!(step.state, SyncState::Fetching { done: 0, total: 4 }));
    // the host stops at the join cap too, whatever the client asks
    let mut files = Files::default();
    for e in four.entries() {
        files.files.insert(e.sha256, vec![0]);
    }
    let mut session = HostSession::new(four.clone(), files);
    session.promised = MAX_JOIN_BYTES - 1;
    let HostEvent::Reply(limit) = session.handle(&request(&four.entries()[0].sha256)).unwrap() else { panic!() };
    assert!(matches!(Message::decode(&limit[0]).unwrap(), Message::Refuse { reason: RefuseReason::Limit, .. }));
}

#[test]
fn a_file_that_changed_on_the_host_after_the_offer_is_not_served() {
    let mut session = host_with(b"offered bytes");
    session.source.files.clear();
    let HostEvent::Reply(replies) = session.handle(&request(&sha(b"offered bytes"))).unwrap() else { panic!() };
    assert_eq!(
        Message::decode(&replies[0]).unwrap(),
        Message::Refuse { sha256: sha(b"offered bytes"), reason: RefuseReason::Unavailable }
    );
    assert!(!session.transferring());
}

#[test]
fn hex_helpers_round_trip() {
    let hash = sha(b"x");
    assert_eq!(parse_hex(&hex(&hash)), Some(hash));
    assert_eq!(parse_hex(&"A".repeat(64)), None);
    assert_eq!(parse_hex("abc"), None);
    assert_eq!(parse_hex(&"g".repeat(64)), None);
}
