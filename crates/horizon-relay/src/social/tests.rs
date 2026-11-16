use std::io::{self, Read, Write};
use super::*;
use horizon_friends::social::{Grant, ToRelay};
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
    }, 100, now).unwrap();
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
    relay.provision(&request("a"), &"ab".repeat(32), limits, TICKET_NOW, now).unwrap();
    assert!(matches!(relay.provision(&request("b"), &"ab".repeat(32), limits, TICKET_NOW, now), Err(RelayError::SessionLimit)));
    relay.retire("a", TICKET_NOW).unwrap();
    // The place is free again, so provisioning never fails forever ...
    relay.provision(&request("b"), &"ab".repeat(32), limits, TICKET_NOW, now).unwrap();
    // ... but "a" cannot come back while a grant issued before the retirement could still be valid.
    relay.retire("b", TICKET_NOW).unwrap();
    assert!(matches!(relay.provision(&request("a"), &"ab".repeat(32), limits, TICKET_NOW, now), Err(RelayError::SessionExists)));
    // Retired ids are bounded too: live sessions plus tombstones stay under 4 x max_sessions.
    relay.provision(&request("c"), &"ab".repeat(32), limits, TICKET_NOW, now).unwrap();
    relay.retire("c", TICKET_NOW).unwrap();
    relay.provision(&request("d"), &"ab".repeat(32), limits, TICKET_NOW, now).unwrap();
    relay.retire("d", TICKET_NOW).unwrap();
    assert!(matches!(relay.provision(&request("e"), &"ab".repeat(32), limits, TICKET_NOW, now), Err(RelayError::SessionLimit)));
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
    assert!(matches!(relay.provision(&request, &"ab".repeat(32), limits, 100, Instant::now()), Err(RelayError::InvalidLimits)));
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

const TICKET_NOW: u64 = 1_700_000_000;
const TICKET_TOKEN: &str = "abababababababababababababababababababababababababababababababab";

fn to_hex(bytes: &[u8]) -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() }

fn ticket(session: &str, nonce: u8) -> ProvisionTicket {
    ticket_for(session, TICKET_TOKEN, nonce, TICKET_NOW + 120)
}

fn ticket_for(session: &str, token: &str, nonce: u8, exp: u64) -> ProvisionTicket {
    let digest = ring::digest::digest(&ring::digest::SHA256, &hex::<32>(token));
    ProvisionTicket {
        v: 1,
        session_id: session.into(),
        host_principal: "member:host".into(),
        token_sha256: to_hex(digest.as_ref()),
        bytes_per_second: 1_000_000,
        burst_bytes: 65_536,
        max_connections: 8,
        idle_secs: 60,
        session_exp: TICKET_NOW + 3600,
        relay_id: "relay-1".into(),
        exp,
        nonce: format!("{nonce:02x}").repeat(32),
    }
}

/// Signed with the portal test key; a ticket that cannot be encoded gets a junk signature.
fn sign_ticket(ticket: &ProvisionTicket) -> String {
    match ticket.signing_bytes() {
        Ok(bytes) => {
            let key = Ed25519KeyPair::from_seed_unchecked(&hex::<32>(SEED)).unwrap();
            to_hex(key.sign(&bytes).as_ref())
        }
        Err(_) => "00".repeat(64),
    }
}

fn open(relay: &SocialRelay, ticket: &ProvisionTicket, token: &str, unix: u64) -> Result<(), RelayError> {
    relay.provision_ticket(ticket, &sign_ticket(ticket), token, unix, Instant::now())
}

fn ticket_relay() -> SocialRelay { ticket_relay_with(hex::<32>(PUBLIC), 8) }

fn ticket_relay_with(key: [u8; 32], max_sessions: usize) -> SocialRelay {
    SocialRelay::new(key, max_sessions).with_relay_id("relay-1").unwrap()
}

#[test]
fn a_valid_ticket_opens_a_session_that_pairs_with_the_ticket_token() {
    let relay = ticket_relay();
    let now = Instant::now();
    open(&relay, &ticket("session-1", 1), TICKET_TOKEN, TICKET_NOW).unwrap();
    let grant = signed(1, TICKET_NOW + 60);
    let mut pending = relay.begin_guest(&grant, TestStream::empty(), TICKET_NOW, now).unwrap();
    assert!(matches!(pending.pair(&grant, &"cd".repeat(32), TestStream::empty(), TICKET_NOW, now), Err(RelayError::WrongToken)));
    assert!(pending.pair(&grant, TICKET_TOKEN, TestStream::empty(), TICKET_NOW, now).is_ok());
}

#[test]
fn a_ticket_with_a_bad_signature_is_refused() {
    let relay = ticket_relay();
    let good = ticket("session-1", 1);
    let signature = sign_ticket(&good);
    let mut altered = good.clone();
    altered.max_connections = 9;
    let result = relay.provision_ticket(&altered, &signature, TICKET_TOKEN, TICKET_NOW, Instant::now());
    assert!(matches!(result, Err(RelayError::Grant(GrantError::InvalidSignature))));
    let result = relay.provision_ticket(&good, &"00".repeat(64), TICKET_TOKEN, TICKET_NOW, Instant::now());
    assert!(matches!(result, Err(RelayError::Grant(GrantError::InvalidSignature))));
    let other_key = ticket_relay_with([7; 32], 8);
    assert!(matches!(open(&other_key, &good, TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::InvalidSignature))));
    // Nothing was opened by any of that.
    assert!(open(&relay, &good, TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn expired_and_too_far_ahead_tickets_are_refused() {
    let relay = ticket_relay();
    let at = |exp: u64, nonce: u8| ticket_for("session-1", TICKET_TOKEN, nonce, exp);
    assert!(matches!(open(&relay, &at(TICKET_NOW, 1), TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::Expired))));
    assert!(matches!(open(&relay, &at(TICKET_NOW - 5, 2), TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::Expired))));
    assert!(matches!(open(&relay, &at(TICKET_NOW + 301, 3), TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::LifetimeTooLong))));
    assert!(open(&relay, &at(TICKET_NOW + 300, 4), TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn a_used_nonce_cannot_open_another_session() {
    let relay = ticket_relay();
    open(&relay, &ticket("session-1", 1), TICKET_TOKEN, TICKET_NOW).unwrap();
    let result = open(&relay, &ticket("session-2", 1), TICKET_TOKEN, TICKET_NOW);
    assert!(matches!(result, Err(RelayError::Grant(GrantError::Replay))));
    assert!(open(&relay, &ticket("session-2", 2), TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn a_refused_ticket_does_not_burn_its_nonce() {
    let relay = ticket_relay_with(hex::<32>(PUBLIC), 1);
    open(&relay, &ticket("session-1", 1), TICKET_TOKEN, TICKET_NOW).unwrap();
    let second = ticket("session-2", 2);
    assert!(matches!(open(&relay, &second, TICKET_TOKEN, TICKET_NOW), Err(RelayError::SessionLimit)));
    relay.retire("session-1", TICKET_NOW).unwrap();
    assert!(open(&relay, &second, TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn a_full_nonce_cache_refuses_until_entries_expire() {
    let relay = ticket_relay_with(hex::<32>(PUBLIC), MAX_TICKET_NONCES + 8);
    let numbered = |index: usize, exp: u64| {
        let mut ticket = ticket_for(&format!("s{index}"), TICKET_TOKEN, 0, exp);
        ticket.nonce = format!("{index:064x}");
        ticket.host_principal = format!("member:h{index}"); // one host holds at most 3 sessions
        ticket
    };
    for index in 0..MAX_TICKET_NONCES {
        open(&relay, &numbered(index, TICKET_NOW + 100), TICKET_TOKEN, TICKET_NOW).unwrap();
    }
    let extra = numbered(MAX_TICKET_NONCES, TICKET_NOW + 100);
    assert!(matches!(open(&relay, &extra, TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::ReplayCacheFull))));
    // An idempotent repeat needs no new entry, so a full cache does not stop it.
    assert!(open(&relay, &numbered(0, TICKET_NOW + 100), TICKET_TOKEN, TICKET_NOW).is_ok());
    // Once the remembered tickets have expired they are forgotten.
    let later = numbered(MAX_TICKET_NONCES, TICKET_NOW + 250);
    assert!(open(&relay, &later, TICKET_TOKEN, TICKET_NOW + 150).is_ok());
}

#[test]
fn a_token_that_does_not_match_the_signed_digest_is_refused() {
    let relay = ticket_relay();
    let ticket = ticket("session-1", 1);
    for wrong in [&"cd".repeat(32)[..], "", "not hex", &TICKET_TOKEN.to_uppercase(), &TICKET_TOKEN[2..]] {
        assert!(matches!(open(&relay, &ticket, wrong, TICKET_NOW), Err(RelayError::WrongToken)), "{wrong}");
    }
    assert!(open(&relay, &ticket, TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn invalid_limits_are_refused() {
    let relay = ticket_relay();
    let edits: [(&str, fn(&mut ProvisionTicket)); 11] = [
        ("small burst", |t| t.burst_bytes = 16 * 1024 - 1),
        ("zero burst", |t| t.burst_bytes = 0),
        ("huge burst", |t| t.burst_bytes = MAX_TICKET_BURST_BYTES + 1),
        ("zero rate", |t| t.bytes_per_second = 0),
        ("huge rate", |t| t.bytes_per_second = MAX_TICKET_BYTES_PER_SECOND + 1),
        ("rate max u64", |t| t.bytes_per_second = u64::MAX),
        ("zero connections", |t| t.max_connections = 0),
        ("many connections", |t| t.max_connections = MAX_TICKET_CONNECTIONS + 1),
        ("zero idle", |t| t.idle_secs = 0),
        ("long idle", |t| t.idle_secs = MAX_TICKET_IDLE_SECS + 1),
        ("burst max u64", |t| t.burst_bytes = u64::MAX),
    ];
    for (name, edit) in edits {
        let mut ticket = ticket("session-1", 1);
        edit(&mut ticket);
        assert!(matches!(open(&relay, &ticket, TICKET_TOKEN, TICKET_NOW), Err(RelayError::InvalidLimits)), "{name}");
    }
    let mut edge = ticket("session-1", 1);
    edge.burst_bytes = 16 * 1024;
    assert!(open(&relay, &edge, TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn ids_must_follow_the_identifier_rules() {
    let relay = ticket_relay();
    let long = "x".repeat(129);
    for (index, id) in ["", "has\ncontrol", "tab\there", long.as_str()].into_iter().enumerate() {
        let mut ticket = ticket("session-1", index as u8 + 1);
        ticket.session_id = id.into();
        assert!(matches!(open(&relay, &ticket, TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(_))), "session {id:?}");
        let mut ticket = ticket_for("session-1", TICKET_TOKEN, index as u8 + 10, TICKET_NOW + 120);
        ticket.host_principal = id.into();
        assert!(matches!(open(&relay, &ticket, TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(_))), "host {id:?}");
    }
    let mut longest = ticket("s", 30);
    longest.session_id = "y".repeat(128);
    assert!(open(&relay, &longest, TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn only_version_one_is_accepted() {
    let relay = ticket_relay();
    for version in [0, 2, u32::MAX] {
        let mut ticket = ticket("session-1", 1);
        ticket.v = version;
        assert!(matches!(open(&relay, &ticket, TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::UnsupportedVersion))));
    }
}

#[test]
fn a_repeated_provision_is_idempotent_but_a_different_token_or_host_is_not() {
    let relay = ticket_relay();
    let first = ticket("session-1", 1);
    open(&relay, &first, TICKET_TOKEN, TICKET_NOW).unwrap();
    // The very same ticket again (a retry after a lost reply): provisioned, nothing changes.
    assert!(open(&relay, &first, TICKET_TOKEN, TICKET_NOW + 5).is_ok());
    // A fresh ticket with the same token digest for the same session is a repeat too.
    let mut again = ticket("session-1", 2);
    again.max_connections = 2;
    assert!(open(&relay, &again, TICKET_TOKEN, TICKET_NOW + 5).is_ok());
    // ... and it did not replace the limits: three guests still fit (limit 8, not 2).
    let now = Instant::now();
    let _guests: Vec<_> = (1..=3)
        .map(|nonce| relay.begin_guest(&signed(nonce, TICKET_NOW + 60), TestStream::empty(), TICKET_NOW, now).unwrap())
        .collect();
    // Another token for the same session id, or another host principal, is refused.
    let other_token = "cd".repeat(32);
    let different = ticket_for("session-1", &other_token, 3, TICKET_NOW + 120);
    assert!(matches!(open(&relay, &different, &other_token, TICKET_NOW), Err(RelayError::SessionExists)));
    let mut other_host = ticket("session-1", 4);
    other_host.host_principal = "member:other".into();
    assert!(matches!(open(&relay, &other_host, TICKET_TOKEN, TICKET_NOW), Err(RelayError::SessionExists)));
    // A retired id stays reserved: the repeat no longer succeeds.
    relay.retire("session-1", TICKET_NOW).unwrap();
    assert!(matches!(open(&relay, &first, TICKET_TOKEN, TICKET_NOW), Err(RelayError::SessionExists)));
}

#[test]
fn an_expired_repeat_is_refused_even_for_an_open_session() {
    let relay = ticket_relay();
    let first = ticket("session-1", 1);
    open(&relay, &first, TICKET_TOKEN, TICKET_NOW).unwrap();
    assert!(matches!(open(&relay, &first, TICKET_TOKEN, TICKET_NOW + 120), Err(RelayError::Grant(GrantError::Expired))));
}

#[test]
fn ticket_messages_reject_unknown_fields() {
    let ticket = ticket("session-1", 1);
    let message = ToRelay::Provision { ticket, signature: "00".repeat(64), host_token: TICKET_TOKEN.into() };
    let text = serde_json::to_string(&message).unwrap();
    assert!(serde_json::from_str::<ToRelay>(&text).is_ok());
    assert!(serde_json::from_str::<ToRelay>(&text.replacen('{', "{\"relay\":\"x\",", 1)).is_err());
    assert!(serde_json::from_str::<ToRelay>(&text.replacen("\"v\":1", "\"v\":1,\"limits\":{}", 1)).is_err());
}

/// A ticket for host `host` that expires at `exp` and whose session ends at `session_exp`.
fn ticket_timed(session: &str, host: &str, nonce: u8, exp: u64, session_exp: u64) -> ProvisionTicket {
    let mut ticket = ticket_for(session, TICKET_TOKEN, nonce, exp);
    ticket.host_principal = host.into();
    ticket.session_exp = session_exp;
    ticket
}

fn guest_can_join(relay: &SocialRelay, nonce: u8, unix: u64) -> Result<(), RelayError> {
    relay.begin_guest(&signed(nonce, unix + 60), TestStream::empty(), unix, Instant::now()).map(drop)
}

#[test]
fn a_session_ends_at_its_session_exp_and_frees_its_place() {
    let relay = ticket_relay_with(hex::<32>(PUBLIC), 1);
    let end = TICKET_NOW + 600;
    open(&relay, &ticket_timed("session-1", "member:host", 1, TICKET_NOW + 120, end), TICKET_TOKEN, TICKET_NOW).unwrap();
    assert!(guest_can_join(&relay, 1, end - 1).is_ok());
    let other = |nonce, exp, session_exp| ticket_timed("session-2", "member:other", nonce, exp, session_exp);
    assert!(matches!(open(&relay, &other(2, end + 60, end + 3600), TICKET_TOKEN, end - 1), Err(RelayError::SessionLimit)));
    // At session_exp the room is over: a new session fits, and guests can no longer join the old one.
    assert!(open(&relay, &other(2, end + 60, end + 3600), TICKET_TOKEN, end).is_ok());
    assert!(matches!(guest_can_join(&relay, 2, end), Err(RelayError::UnknownSession)));
}

#[test]
fn an_ended_session_keeps_its_id_reserved_for_300_seconds_then_can_be_opened_again() {
    let relay = ticket_relay();
    let end = TICKET_NOW + 600;
    open(&relay, &ticket_timed("session-1", "member:host", 1, TICKET_NOW + 120, end), TICKET_TOKEN, TICKET_NOW).unwrap();
    let again = |nonce: u8, at: u64| ticket_timed("session-1", "member:host", nonce, at + 120, at + 3600);
    // The reservation, as for `retire`: a grant issued just before the end could still be valid.
    assert!(matches!(open(&relay, &again(2, end), TICKET_TOKEN, end), Err(RelayError::SessionExists)));
    assert!(matches!(open(&relay, &again(2, end + 299), TICKET_TOKEN, end + 299), Err(RelayError::SessionExists)));
    assert!(open(&relay, &again(2, end + 300), TICKET_TOKEN, end + 300).is_ok());
    assert!(guest_can_join(&relay, 3, end + 300).is_ok());
}

#[test]
fn a_repeat_with_a_newer_ticket_extends_the_session_but_never_shortens_it() {
    let relay = ticket_relay();
    let first_end = TICKET_NOW + 600;
    let open_with = |nonce: u8, at: u64, session_exp: u64| {
        open(&relay, &ticket_timed("session-1", "member:host", nonce, at + 120, session_exp), TICKET_TOKEN, at)
    };
    open_with(1, TICKET_NOW, first_end).unwrap();
    open_with(2, TICKET_NOW + 10, TICKET_NOW + 1200).unwrap();
    open_with(3, TICKET_NOW + 20, TICKET_NOW + 400).unwrap();
    assert!(guest_can_join(&relay, 1, first_end + 100).is_ok());
    assert!(guest_can_join(&relay, 2, TICKET_NOW + 1199).is_ok());
    assert!(matches!(guest_can_join(&relay, 3, TICKET_NOW + 1200), Err(RelayError::UnknownSession)));
}

#[test]
fn a_host_may_have_three_sessions_open_at_once() {
    let relay = ticket_relay();
    for index in 0..3u8 {
        open(&relay, &ticket(&format!("room-{index}"), index + 1), TICKET_TOKEN, TICKET_NOW).unwrap();
    }
    assert!(matches!(open(&relay, &ticket("room-3", 4), TICKET_TOKEN, TICKET_NOW), Err(RelayError::HostLimit)));
    // A repeat of an open room is not a fourth room.
    assert!(open(&relay, &ticket("room-0", 5), TICKET_TOKEN, TICKET_NOW).is_ok());
    // Another host is unaffected, and a closed room gives its place back.
    let mut other = ticket("room-x", 6);
    other.host_principal = "member:other".into();
    assert!(open(&relay, &other, TICKET_TOKEN, TICKET_NOW).is_ok());
    relay.retire("room-1", TICKET_NOW).unwrap();
    assert!(open(&relay, &ticket("room-3", 4), TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn the_host_token_retires_a_ticket_session_early() {
    let relay = ticket_relay();
    open(&relay, &ticket("session-1", 1), TICKET_TOKEN, TICKET_NOW).unwrap();
    assert!(matches!(relay.retire_with_token("session-1", &"cd".repeat(32), TICKET_NOW), Err(RelayError::WrongToken)));
    assert!(matches!(relay.retire_with_token("session-1", "", TICKET_NOW), Err(RelayError::WrongToken)));
    assert!(guest_can_join(&relay, 1, TICKET_NOW).is_ok());
    assert!(matches!(relay.retire_with_token("nope", TICKET_TOKEN, TICKET_NOW), Err(RelayError::UnknownSession)));
    relay.retire_with_token("session-1", TICKET_TOKEN, TICKET_NOW).unwrap();
    assert!(matches!(guest_can_join(&relay, 2, TICKET_NOW), Err(RelayError::UnknownSession)));
    assert!(matches!(relay.retire_with_token("session-1", TICKET_TOKEN, TICKET_NOW), Err(RelayError::UnknownSession)));
    // The place is free again but the id is reserved for a while.
    assert!(matches!(open(&relay, &ticket("session-1", 3), TICKET_TOKEN, TICKET_NOW), Err(RelayError::SessionExists)));
}

#[test]
fn a_session_opened_by_function_call_is_not_reachable_by_tickets_or_token_retire() {
    let relay = ticket_relay();
    let request = RegisterSession::relay_only("session-1".into(), "member:host".into());
    let limits = SessionLimits { max_connections: 4, max_nonces: 8, bytes_per_second: 1000, burst_bytes: BURST, idle_timeout: Duration::from_secs(5) };
    relay.provision(&request, TICKET_TOKEN, limits, TICKET_NOW, Instant::now()).unwrap();
    // Same principal, same token: a ticket still does not adopt (or extend) it.
    assert!(matches!(open(&relay, &ticket("session-1", 1), TICKET_TOKEN, TICKET_NOW), Err(RelayError::SessionExists)));
    assert!(matches!(relay.retire_with_token("session-1", TICKET_TOKEN, TICKET_NOW), Err(RelayError::UnknownSession)));
}

#[test]
fn grant_and_ticket_signatures_do_not_cross_over() {
    let relay = ticket_relay();
    let grant = signed(1, TICKET_NOW + 60);
    let good = ticket("session-1", 1);
    let result = relay.provision_ticket(&good, &grant.signature, TICKET_TOKEN, TICKET_NOW, Instant::now());
    assert!(matches!(result, Err(RelayError::Grant(GrantError::InvalidSignature))));
    open(&relay, &good, TICKET_TOKEN, TICKET_NOW).unwrap();
    let forged = SignedGrant { grant: grant.grant.clone(), signature: sign_ticket(&good) };
    let result = relay.begin_guest(&forged, TestStream::empty(), TICKET_NOW, Instant::now());
    assert!(matches!(result, Err(RelayError::Grant(GrantError::InvalidSignature))));
}

#[test]
fn a_ticket_for_another_relay_is_refused() {
    let relay = ticket_relay();
    let mut elsewhere = ticket("session-1", 1);
    elsewhere.relay_id = "relay-staging".into();
    assert!(matches!(open(&relay, &elsewhere, TICKET_TOKEN, TICKET_NOW), Err(RelayError::WrongRelay)));
    // A relay with no id of its own takes no tickets at all.
    let nameless = SocialRelay::new(hex::<32>(PUBLIC), 8);
    assert!(matches!(open(&nameless, &ticket("session-1", 1), TICKET_TOKEN, TICKET_NOW), Err(RelayError::WrongRelay)));
    assert!(SocialRelay::new(hex::<32>(PUBLIC), 8).with_relay_id("").is_err());
    assert!(open(&relay, &ticket("session-1", 1), TICKET_TOKEN, TICKET_NOW).is_ok());
}

#[test]
fn session_exp_is_at_most_twelve_hours_ahead_and_in_the_future() {
    let relay = ticket_relay();
    let at = |nonce: u8, session_exp: u64| ticket_timed("session-1", "member:host", nonce, TICKET_NOW + 120, session_exp);
    assert!(matches!(open(&relay, &at(1, TICKET_NOW + 43_201), TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::LifetimeTooLong))));
    assert!(matches!(open(&relay, &at(2, TICKET_NOW), TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::Expired))));
    assert!(matches!(open(&relay, &at(3, 0), TICKET_TOKEN, TICKET_NOW), Err(RelayError::Grant(GrantError::Expired))));
    assert!(open(&relay, &at(4, TICKET_NOW + 43_200), TICKET_TOKEN, TICKET_NOW).is_ok());
}
