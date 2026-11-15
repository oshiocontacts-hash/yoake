use std::io::{self, Read, Write};
use super::*;
use horizon_friends::social::Grant;
use ring::signature::Ed25519KeyPair;
use std::cell::RefCell;
use std::io::Cursor;
use std::rc::Rc;

struct TestStream {
    incoming: Cursor<Vec<u8>>,
    outgoing: Rc<RefCell<Vec<u8>>>,
}

impl TestStream {
    fn new(input: &[u8]) -> (Self, Rc<RefCell<Vec<u8>>>) {
        let outgoing = Rc::new(RefCell::new(Vec::new()));
        (Self { incoming: Cursor::new(input.to_vec()), outgoing: outgoing.clone() }, outgoing)
    }

    fn empty() -> Self {
        Self::new(&[]).0
    }
}

impl Read for TestStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.incoming.read(bytes)
    }
}

impl Write for TestStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.outgoing.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

const BURST: usize = 16 * 1024;
const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

fn hex<const LENGTH: usize>(value: &str) -> [u8; LENGTH] {
    let mut output = [0; LENGTH];
    for (index, byte) in output.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16).unwrap();
    }
    output
}

fn signed(nonce: u8, exp: u64) -> SignedGrant {
    let grant = Grant {
        v: 1,
        session_id: "session-1".into(),
        host_principal: "member:host".into(),
        guest_principal: "guest:alice".into(),
        mc_profile: Some("profile-1".into()),
        relay_only: true,
        exp,
        nonce: format!("{nonce:02x}").repeat(32),
    };
    let key = Ed25519KeyPair::from_seed_unchecked(&hex::<32>(SEED)).unwrap();
    let signature = key.sign(&grant.signing_bytes().unwrap()).as_ref().iter()
        .map(|byte| format!("{byte:02x}")).collect();
    SignedGrant { grant, signature }
}

fn setup(max_connections: usize, idle_timeout: Duration, burst_bytes: usize, rate: u64) -> (SocialRelay, Instant) {
    let now = Instant::now();
    let relay = SocialRelay::new(hex::<32>(PUBLIC), 4);
    let request = RegisterSession::relay_only("session-1".into(), "member:host".into());
    relay.provision(&request, &"ab".repeat(32), SessionLimits {
        max_connections,
        max_nonces: 8,
        bytes_per_second: rate,
        burst_bytes,
        idle_timeout,
    }, now).unwrap();
    (relay, now)
}

#[test]
fn pairing_requires_the_host_token_and_the_exact_guest_grant() {
    let (relay, now) = setup(2, Duration::from_secs(30), BURST, 32);
    let grant = signed(1, 200);
    let (guest, guest_output) = TestStream::new(b"hello");
    let (host, host_output) = TestStream::new(b"welcome");
    let mut pending = relay.begin_guest(&grant, guest, 100, now).unwrap();
    assert_eq!(pending.verified_grant().guest_principal(), "guest:alice");
    assert!(matches!(pending.pair(&grant, &"cd".repeat(32), TestStream::empty(), 100, now), Err(RelayError::WrongToken)));
    assert!(matches!(pending.pair(&signed(2, 200), &"ab".repeat(32), TestStream::empty(), 100, now), Err(RelayError::PairingMismatch)));
    let mut pair = pending.pair(&grant, &"ab".repeat(32), host, 100, now).unwrap();
    assert_eq!(pair.verified_grant().mc_profile(), Some("profile-1"));
    assert_eq!(pair.forward_once(Direction::GuestToHost, now).unwrap(), ForwardProgress::Transferred(5));
    assert_eq!(pair.forward_once(Direction::HostToGuest, now).unwrap(), ForwardProgress::Transferred(7));
    assert_eq!(host_output.borrow().as_slice(), b"hello");
    assert_eq!(guest_output.borrow().as_slice(), b"welcome");
}

#[test]
fn connection_capacity_is_released_without_allowing_grant_replay() {
    let (relay, now) = setup(1, Duration::from_secs(30), BURST, 32);
    let first = signed(1, 200);
    let second = signed(2, 200);
    let mut pending = relay.begin_guest(&first, TestStream::empty(), 100, now).unwrap();
    assert!(matches!(relay.begin_guest(&second, TestStream::empty(), 100, now), Err(RelayError::ConnectionLimit)));
    let pair = pending.pair(&first, &"ab".repeat(32), TestStream::empty(), 100, now).unwrap();
    assert!(matches!(relay.begin_guest(&second, TestStream::empty(), 100, now), Err(RelayError::ConnectionLimit)));
    drop(pair);
    assert!(matches!(relay.begin_guest(&first, TestStream::empty(), 100, now), Err(RelayError::Grant(GrantError::Replay))));
    assert!(relay.begin_guest(&second, TestStream::empty(), 100, now).is_ok());
}

#[test]
fn idle_pending_and_paired_connections_expire() {
    let (relay, now) = setup(1, Duration::from_secs(5), BURST, 32);
    let first = signed(1, 200);
    let second = signed(2, 200);
    let mut stale = relay.begin_guest(&first, TestStream::empty(), 100, now).unwrap();
    let later = now + Duration::from_secs(6);
    let (guest, _) = TestStream::new(b"late");
    let (host, host_output) = TestStream::new(&[]);
    let mut pending = relay.begin_guest(&second, guest, 106, later).unwrap();
    assert!(matches!(stale.pair(&first, &"ab".repeat(32), TestStream::empty(), 106, later), Err(RelayError::TimedOut)));
    let mut pair = pending.pair(&second, &"ab".repeat(32), host, 106, later).unwrap();
    assert!(matches!(pair.forward_once(Direction::GuestToHost, later + Duration::from_secs(5)), Err(ForwardError::Relay(RelayError::TimedOut))));
    assert!(host_output.borrow().is_empty());
    assert!(relay.begin_guest(&signed(3, 200), TestStream::empty(), 111, later + Duration::from_secs(5)).is_ok());
}

#[test]
fn bandwidth_budget_is_shared_across_directions() {
    let (relay, now) = setup(1, Duration::from_secs(30), BURST, BURST as u64);
    let grant = signed(1, 200);
    let (guest, guest_output) = TestStream::new(&[b'1'; BURST]);
    let (host, host_output) = TestStream::new(&[b'a'; BURST]);
    let mut pending = relay.begin_guest(&grant, guest, 100, now).unwrap();
    let mut pair = pending.pair(&grant, &"ab".repeat(32), host, 100, now).unwrap();
    assert_eq!(pair.forward_once(Direction::GuestToHost, now).unwrap(), ForwardProgress::Transferred(BURST));
    assert_eq!(pair.forward_once(Direction::HostToGuest, now).unwrap(), ForwardProgress::Blocked);
    assert_eq!(pair.forward_once(Direction::HostToGuest, now + Duration::from_secs(1)).unwrap(), ForwardProgress::Transferred(BURST));
    assert_eq!(host_output.borrow().as_slice(), &[b'1'; BURST][..]);
    assert_eq!(guest_output.borrow().as_slice(), &[b'a'; BURST][..]);
}

#[test]
fn stale_handle_cannot_remove_a_reused_slot() {
    let (relay, now) = setup(1, Duration::from_secs(5), BURST, 32);
    let stale = relay.begin_guest(&signed(1, 105), TestStream::empty(), 100, now).unwrap();
    let later = now + Duration::from_secs(6);
    let fresh_grant = signed(1, 200);
    let mut fresh = relay.begin_guest(&fresh_grant, TestStream::empty(), 106, later).unwrap();
    drop(stale);
    assert!(fresh.pair(&fresh_grant, &"ab".repeat(32), TestStream::empty(), 106, later).is_ok());
}

#[test]
fn retiring_a_session_frees_its_place_but_not_its_id_for_a_while() {
    let now = Instant::now();
    let relay = SocialRelay::new(hex::<32>(PUBLIC), 1);
    let limits = SessionLimits { max_connections: 1, max_nonces: 1, bytes_per_second: 1, burst_bytes: BURST, idle_timeout: Duration::from_secs(5) };
    let request = |id: &str| RegisterSession::relay_only(id.into(), "member:host".into());
    relay.provision(&request("a"), &"ab".repeat(32), limits, now).unwrap();
    assert!(matches!(relay.provision(&request("b"), &"ab".repeat(32), limits, now), Err(RelayError::SessionLimit)));
    relay.retire("a").unwrap();
    // The place is free again, so provisioning never fails forever ...
    relay.provision(&request("b"), &"ab".repeat(32), limits, now).unwrap();
    // ... but "a" cannot come back while a grant issued before the retirement could still be valid.
    relay.retire("b").unwrap();
    assert!(matches!(relay.provision(&request("a"), &"ab".repeat(32), limits, now), Err(RelayError::SessionExists)));
    // Retired ids are bounded too: live sessions plus tombstones stay under 4 x max_sessions.
    relay.provision(&request("c"), &"ab".repeat(32), limits, now).unwrap();
    relay.retire("c").unwrap();
    relay.provision(&request("d"), &"ab".repeat(32), limits, now).unwrap();
    relay.retire("d").unwrap();
    assert!(matches!(relay.provision(&request("e"), &"ab".repeat(32), limits, now), Err(RelayError::SessionLimit)));
}

#[test]
fn repeated_wrong_tokens_lock_the_slot_for_a_while_but_never_drop_the_guest() {
    let (relay, now) = setup(2, Duration::from_secs(600), BURST, 32);
    let grant = signed(1, 300);
    let mut pending = relay.begin_guest(&grant, TestStream::empty(), 100, now).unwrap();
    for _ in 0..3 {
        assert!(matches!(pending.pair(&grant, &"cd".repeat(32), TestStream::empty(), 100, now), Err(RelayError::WrongToken)));
    }
    // Locked: even the right token is refused, but the guest is still there.
    assert!(!pending.is_expired(now));
    assert!(matches!(pending.pair(&grant, &"ab".repeat(32), TestStream::empty(), 100, now + Duration::from_secs(29)), Err(RelayError::WrongToken)));
    assert!(!pending.is_expired(now + Duration::from_secs(29)));
    assert!(pending.pair(&grant, &"ab".repeat(32), TestStream::empty(), 130, now + Duration::from_secs(31)).is_ok());
}

#[test]
fn a_burst_smaller_than_one_message_is_not_a_valid_limit() {
    let relay = SocialRelay::new(hex::<32>(PUBLIC), 1);
    let limits = SessionLimits { max_connections: 1, max_nonces: 1, bytes_per_second: 1, burst_bytes: BURST - 1, idle_timeout: Duration::from_secs(5) };
    let request = RegisterSession::relay_only("a".into(), "member:host".into());
    assert!(matches!(relay.provision(&request, &"ab".repeat(32), limits, Instant::now()), Err(RelayError::InvalidLimits)));
}

#[test]
fn bandwidth_is_charged_per_whole_message_and_nothing_is_held_while_idle() {
    let (relay, now) = setup(1, Duration::from_secs(30), BURST, 10);
    let grant = signed(1, 200);
    let (guest, _) = TestStream::new(b"0123456789");
    let host_output = Rc::new(RefCell::new(Vec::new()));
    let host = Idle(host_output.clone());
    let mut pending = relay.begin_guest(&grant, guest, 100, now).unwrap();
    let mut pair = pending.pair(&grant, &"ab".repeat(32), host, 100, now).unwrap();
    // Host side has nothing: no budget is taken for the empty read, however often it is polled.
    for _ in 0..50 {
        assert_eq!(pair.forward_once(Direction::HostToGuest, now).unwrap(), ForwardProgress::Blocked);
    }
    assert_eq!(pair.forward_once(Direction::GuestToHost, now).unwrap(), ForwardProgress::Transferred(10));
    assert_eq!(host_output.borrow().as_slice(), b"0123456789");
}

/// A peer that never has anything to say.
struct Idle(Rc<RefCell<Vec<u8>>>);

impl Read for Idle {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> { Err(io::ErrorKind::WouldBlock.into()) }
}

impl Write for Idle {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
