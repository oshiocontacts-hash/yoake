mod social_support;
use horizon_friends::{social::{FromRelay, ToRelay}, social_tls::{ClientChannel, ControlChannel, client_channel}};
use horizon_relay::social_server::SocialServer;
use rustls::{CertificateError, Error as TlsError};
use social_support::*;
use std::{net::TcpStream, time::Duration};

fn bridge(address: std::net::SocketAddr, tls: &TestTls, grant: &horizon_friends::social::SignedGrant) -> (ClientChannel, ClientChannel) {
    let mut guest = guest(address, tls, grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    let mut host = host(address, tls, grant, TOKEN);
    assert!(matches!(recv_from(&mut host), FromRelay::Bridged {}));
    assert!(matches!(recv_from(&mut guest), FromRelay::Bridged {}));
    (guest, host)
}

#[test]
fn valid_guest_and_host_bridge_payloads_both_ways() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 1);
    let (mut guest, mut host) = bridge(address, &tls, &grant);
    guest.send_message(b"hello host").unwrap();
    assert_eq!(host.recv_message().unwrap(), b"hello host");
    host.send_message(b"hello guest").unwrap();
    assert_eq!(guest.recv_message().unwrap(), b"hello guest");
}

#[test]
fn data_still_flows_after_the_channels_were_idle_for_300_ms() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 20);
    let (mut guest, mut host) = bridge(address, &tls, &grant);
    std::thread::sleep(Duration::from_millis(300));
    guest.send_message(b"late").unwrap();
    assert_eq!(host.recv_message().unwrap(), b"late");
    std::thread::sleep(Duration::from_millis(300));
    host.send_message(b"later").unwrap();
    assert_eq!(guest.recv_message().unwrap(), b"later");
}

#[test]
fn waiting_always_arrives_before_bridged() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    for nonce in 30..50 {
        let grant = signed("session-1", unix_time() + 60, nonce);
        let mut guest = guest(address, &tls, &grant);
        // No wait for `waiting`: a host racing the guest is either refused or comes second.
        let mut host = host(address, &tls, &grant, TOKEN);
        let first = recv_from(&mut guest);
        match recv_from(&mut host) {
            FromRelay::Bridged {} => {
                assert!(matches!(first, FromRelay::Waiting {}));
                assert!(matches!(recv_from(&mut guest), FromRelay::Bridged {}));
            }
            other => assert!(matches!(other, FromRelay::Rejected {})),
        }
    }
}

#[test]
fn wrong_host_token_is_rejected_and_the_guest_keeps_waiting() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 2);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    let mut bad = host(address, &tls, &grant, &"cd".repeat(32));
    assert_rejected(&mut bad);
    assert_peer_closed(&mut bad);
    // Whoever holds only the grant cannot evict the guest.
    assert!(guest.try_recv_message(Duration::from_millis(300)).unwrap().is_none());
    let mut good = host(address, &tls, &grant, TOKEN);
    assert!(matches!(recv_from(&mut good), FromRelay::Bridged {}));
    assert!(matches!(recv_from(&mut guest), FromRelay::Bridged {}));
}

#[test]
fn a_host_with_a_different_grant_for_the_same_nonce_does_not_evict_the_guest() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 3);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    let mut forged = grant.clone();
    forged.grant.guest_principal = "guest:mallory".into();
    let mut bad = host(address, &tls, &forged, TOKEN);
    assert_rejected(&mut bad);
    assert!(guest.try_recv_message(Duration::from_millis(200)).unwrap().is_none());
}

#[test]
fn expired_grants_and_wrong_sessions_are_rejected_and_closed() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let expired = signed("session-1", unix_time() - 1, 4);
    let mut channel = guest(address, &tls, &expired);
    assert_rejected(&mut channel);
    assert_peer_closed(&mut channel);
    let wrong = signed("session-2", unix_time() + 60, 5);
    let mut channel = guest(address, &tls, &wrong);
    assert_rejected(&mut channel);
    assert_peer_closed(&mut channel);
}

#[test]
fn a_grant_cannot_be_used_twice() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 6);
    let mut first = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut first), FromRelay::Waiting {}));
    // While the first guest waits ...
    let mut second = guest(address, &tls, &grant);
    assert_rejected(&mut second);
    assert!(first.try_recv_message(Duration::from_millis(200)).unwrap().is_none());
    // ... and after it was bridged and closed.
    let mut host = host(address, &tls, &grant, TOKEN);
    assert!(matches!(recv_from(&mut host), FromRelay::Bridged {}));
    drop((first, host));
    let mut third = guest(address, &tls, &grant);
    assert_rejected(&mut third);
}

#[test]
fn two_hosts_with_the_same_grant_get_one_bridge_and_one_rejection() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 7);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    let answers: Vec<bool> = std::thread::scope(|scope| {
        let tasks: Vec<_> = (0..2).map(|_| scope.spawn(|| {
            let mut channel = host(address, &tls, &grant, TOKEN);
            matches!(recv_from(&mut channel), FromRelay::Bridged {})
        })).collect();
        tasks.into_iter().map(|task| task.join().unwrap()).collect()
    });
    assert_eq!(answers.iter().filter(|bridged| **bridged).count(), 1);
    assert!(matches!(recv_from(&mut guest), FromRelay::Bridged {}));
    // The relay did not panic: it still serves new grants.
    let fresh = signed("session-1", unix_time() + 60, 8);
    bridge(address, &tls, &fresh);
}

#[test]
fn a_host_that_arrives_first_is_rejected_and_does_not_burn_the_grant() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let grant = signed("session-1", unix_time() + 60, 9);
    let mut early = host(address, &tls, &grant, TOKEN);
    assert_rejected(&mut early);
    assert_peer_closed(&mut early);
    bridge(address, &tls, &grant);
}

#[test]
fn malformed_nonces_are_rejected_without_panicking() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let exp = unix_time() + 60;
    let nonces = ["\u{e9}".repeat(32), "AB".repeat(32), format!("+f{}", "00".repeat(31)), "ab".repeat(31)];
    for nonce in nonces {
        let grant = grant_with_nonce(&nonce, exp);
        let mut channel = guest(address, &tls, &grant);
        assert_rejected(&mut channel);
        let mut channel = host(address, &tls, &grant, TOKEN);
        assert_rejected(&mut channel);
    }
    bridge(address, &tls, &signed("session-1", exp, 10));
}

#[test]
fn a_grant_that_expires_between_guest_and_host_is_refused() {
    let (_server, address, tls) = setup(Duration::from_secs(30));
    let exp = unix_time() + 3;
    let grant = signed("session-1", exp, 11);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    while unix_time() < exp {
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut host = host(address, &tls, &grant, TOKEN);
    assert_rejected(&mut host);
    assert_peer_closed(&mut host);
}

#[test]
fn wrong_relay_ca_fails_with_a_certificate_error() {
    let (_server, address, _tls) = setup(Duration::from_secs(5));
    let other = tls("relay.test");
    let error = client_channel(TcpStream::connect(address).unwrap(), &[&other.ca], "relay.test", Duration::from_secs(2))
        .err().unwrap();
    let inner = error.get_ref().and_then(|inner| inner.downcast_ref::<TlsError>());
    assert!(matches!(inner, Some(TlsError::InvalidCertificate(CertificateError::UnknownIssuer | CertificateError::BadSignature))), "{error:?}");
}

#[test]
fn either_of_two_trusted_cas_works_for_a_rotation_overlap() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let retired = self::tls("relay.test");
    for cas in [[&retired.ca, &tls.ca], [&tls.ca, &retired.ca]] {
        let result = client_channel(TcpStream::connect(address).unwrap(), &cas, "relay.test", Duration::from_secs(2));
        assert!(result.is_ok(), "{:?}", result.err());
    }
}

#[test]
fn pending_guest_is_closed_after_the_idle_timeout() {
    let (_server, address, tls) = setup(Duration::from_millis(100));
    let grant = signed("session-1", unix_time() + 60, 12);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    assert_peer_closed(&mut guest);
}

#[test]
fn idle_bridged_pair_is_closed() {
    let (_server, address, tls) = setup(Duration::from_millis(100));
    let grant = signed("session-1", unix_time() + 60, 13);
    let (mut guest, mut host) = bridge(address, &tls, &grant);
    assert_peer_closed(&mut host);
    assert_peer_closed(&mut guest);
}

#[test]
fn no_social_configuration_does_not_start_a_listener() {
    assert!(SocialServer::from_options(None, None, None, None).is_none());
    // Something set but unusable disables it too.
    let key = Some("ab".repeat(32));
    assert!(SocialServer::from_options(Some("127.0.0.1:0".into()), Some("nope.der".into()), Some("nope.key".into()), key).is_none());
    assert!(SocialServer::from_options(Some("127.0.0.1:0".into()), None, None, Some("\u{e9}".repeat(32))).is_none());
}

#[test]
fn a_bridged_pair_idle_for_longer_than_the_first_message_timeout_stays_alive() {
    let (_server, address, tls) = setup(Duration::from_secs(60));
    let grant = signed("session-1", unix_time() + 120, 60);
    let (mut guest, mut host) = bridge(address, &tls, &grant);
    std::thread::sleep(Duration::from_millis(5600)); // longer than FIRST_MESSAGE_TIMEOUT (5 s)
    guest.send_message(b"still here").unwrap();
    assert_eq!(host.recv_message().unwrap(), b"still here");
    host.send_message(b"yes").unwrap();
    assert_eq!(guest.recv_message().unwrap(), b"yes");
}

fn numbered(index: usize) -> Vec<u8> {
    let mut bytes = vec![(index % 251) as u8; 4096];
    bytes[..4].copy_from_slice(&(index as u32).to_be_bytes());
    bytes
}

#[test]
fn a_slow_peer_behind_a_bandwidth_limited_bridge_gets_whole_messages_in_order() {
    let (_server, address, tls) = setup_with(Duration::from_secs(30), 200_000, 20_000, 64);
    let grant = signed("session-1", unix_time() + 120, 61);
    let (mut guest, mut host) = bridge(address, &tls, &grant);
    let started = std::time::Instant::now();
    let sender = std::thread::spawn(move || {
        for index in 0..40 { guest.send_message(&numbered(index)).unwrap(); }
        guest
    });
    std::thread::sleep(Duration::from_millis(400)); // the host is slow to start reading
    for index in 0..40 {
        assert_eq!(host.recv_message().unwrap(), numbered(index), "message {index} arrived split, lost or out of order");
    }
    let _guest = sender.join().unwrap();
    // ~164 kB at 200 kB/s behind a 20 kB burst: a real wait, but nowhere near a stall.
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn idle_pairs_do_not_take_the_sessions_bandwidth_from_an_active_pair() {
    let (_server, address, tls) = setup_with(Duration::from_secs(60), 100_000, 32_768, 64);
    let idle: Vec<_> = (100..108).map(|nonce| bridge(address, &tls, &signed("session-1", unix_time() + 120, nonce))).collect();
    let (mut guest, mut host) = bridge(address, &tls, &signed("session-1", unix_time() + 120, 110));
    let started = std::time::Instant::now();
    let sender = std::thread::spawn(move || {
        for index in 0..20 { guest.send_message(&numbered(index)).unwrap(); }
        guest
    });
    for index in 0..20 {
        assert_eq!(host.recv_message().unwrap(), numbered(index));
    }
    let _guest = sender.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
    drop(idle);
}

#[test]
fn an_empty_message_closes_the_bridge() {
    let (_server, address, tls) = setup(Duration::from_secs(5));
    let (mut guest, mut host) = bridge(address, &tls, &signed("session-1", unix_time() + 60, 62));
    guest.send_message(&[]).unwrap();
    assert_peer_closed(&mut host);
    assert_peer_closed(&mut guest);
}

#[test]
fn repeated_wrong_tokens_lock_hosts_out_for_a_while_but_never_evict_the_guest() {
    let (_server, address, tls) = setup(Duration::from_secs(30));
    let grant = signed("session-1", unix_time() + 60, 63);
    let mut guest = guest(address, &tls, &grant);
    assert!(matches!(recv_from(&mut guest), FromRelay::Waiting {}));
    for _ in 0..3 {
        let mut bad = host(address, &tls, &grant, &"cd".repeat(32));
        assert_rejected(&mut bad);
    }
    // Locked out for a while, even for the right token; the guest is still waiting.
    let mut locked = host(address, &tls, &grant, TOKEN);
    assert_rejected(&mut locked);
    assert!(guest.try_recv_message(Duration::from_millis(300)).unwrap().is_none());
}

#[test]
fn many_idle_pairs_cost_little_cpu_and_stay_responsive() {
    let (server, address, tls) = setup_with(Duration::from_secs(120), 1_000_000, 64 * 1024, 256);
    let mut pairs: Vec<_> = (0..100u8)
        .map(|index| bridge(address, &tls, &signed("session-1", unix_time() + 200, 100 + index)))
        .collect();
    std::thread::sleep(Duration::from_millis(500)); // let every loop settle into its idle nap
    let before = server.forward_ticks();
    std::thread::sleep(Duration::from_secs(3));
    let ticks = server.forward_ticks() - before;
    // Napping up to 16 ms: about 19k loop passes for 100 pairs in 3 s. A 1 ms tick would be over 90k.
    assert!(ticks < 45_000, "{ticks} forwarding passes in 3 s");
    let (guest, host) = &mut pairs[50];
    let started = std::time::Instant::now();
    guest.send_message(b"ping").unwrap();
    assert_eq!(host.recv_message().unwrap(), b"ping");
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn a_host_opens_its_session_with_a_ticket_and_a_guest_then_bridges_with_a_grant() {
    let (_server, address, tls) = setup_unprovisioned();
    // Without a session, a guest grant finds nothing.
    let grant = signed("session-1", unix_time() + 60, 61);
    let mut early = guest(address, &tls, &grant);
    assert_rejected(&mut early);

    let mut control = client(address, &tls);
    send_to(&mut control, &provision_message("session-1", unix_time() + 60, 1, TOKEN));
    assert!(matches!(recv_from(&mut control), FromRelay::Provisioned {}));
    assert_peer_closed(&mut control);

    let (mut guest, mut host) = bridge(address, &tls, &signed("session-1", unix_time() + 60, 62));
    guest.send_message(b"hello host").unwrap();
    assert_eq!(host.recv_message().unwrap(), b"hello host");
    host.send_message(b"hello guest").unwrap();
    assert_eq!(guest.recv_message().unwrap(), b"hello guest");
}

#[test]
fn repeating_a_ticket_is_provisioned_again_and_leaves_the_session_alone() {
    let (_server, address, tls) = setup_unprovisioned();
    let message = provision_message("session-1", unix_time() + 60, 2, TOKEN);
    assert!(matches!(ask(address, &tls, &message), FromRelay::Provisioned {}));
    let (mut guest, _host) = bridge(address, &tls, &signed("session-1", unix_time() + 60, 63));
    assert!(matches!(ask(address, &tls, &message), FromRelay::Provisioned {}));
    // The bridge made before the repeat still works.
    guest.send_message(b"still here").unwrap();
}

#[test]
fn bad_tickets_get_rejected_and_close_the_connection() {
    let (_server, address, tls) = setup_unprovisioned();
    let other_token = "cd".repeat(32);
    let ToRelay::Provision { ticket, signature, host_token } = provision_message("session-1", unix_time() + 60, 3, TOKEN) else { unreachable!() };
    let wrong_token = ToRelay::Provision { ticket: ticket.clone(), signature: signature.clone(), host_token: other_token };
    let mut tampered = ticket.clone();
    tampered.max_connections = 65;
    let tampered = ToRelay::Provision { ticket: tampered, signature: signature.clone(), host_token: host_token.clone() };
    let expired = provision_message("session-1", unix_time().saturating_sub(5), 4, TOKEN);
    let too_far = provision_message("session-1", unix_time() + 400, 5, TOKEN);
    for message in [wrong_token, tampered, expired, too_far] {
        let mut channel = client(address, &tls);
        send_to(&mut channel, &message);
        assert_rejected(&mut channel);
        assert_peer_closed(&mut channel);
    }
    // None of that opened the session or used the nonces.
    assert!(matches!(ask(address, &tls, &ToRelay::Provision { ticket, signature, host_token }), FromRelay::Provisioned {}));
}

#[test]
fn a_ticket_nonce_works_once_across_sessions_and_unknown_fields_are_refused() {
    let (_server, address, tls) = setup_unprovisioned();
    assert!(matches!(ask(address, &tls, &provision_message("session-1", unix_time() + 60, 6, TOKEN)), FromRelay::Provisioned {}));
    assert!(matches!(ask(address, &tls, &provision_message("session-2", unix_time() + 60, 6, TOKEN)), FromRelay::Rejected {}));
    // A different token for the open session is refused.
    assert!(matches!(ask(address, &tls, &provision_message("session-1", unix_time() + 60, 7, &"cd".repeat(32))), FromRelay::Rejected {}));
    // A ticket with an extra field is not even decoded.
    let mut text = serde_json::to_string(&provision_message("session-3", unix_time() + 60, 8, TOKEN)).unwrap();
    text = text.replacen('{', "{\"candidate\":\"192.0.2.1:1\",", 1);
    let mut channel = client(address, &tls);
    channel.send_message(text.as_bytes()).unwrap();
    assert_peer_closed(&mut channel);
}
