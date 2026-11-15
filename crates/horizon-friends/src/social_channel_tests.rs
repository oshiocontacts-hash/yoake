use super::*;
use crate::social::{Grant, SignedGrant};
use std::io::Cursor;
use std::net::{TcpListener, TcpStream};

const SEED: [u8; 32] = [7; 32];

fn pair() -> (TcpStream, TcpStream) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (relay, _) = listener.accept().unwrap();
    (client, relay)
}

fn identity() -> RelayIdentity {
    RelayIdentity::from_seed(&SEED).unwrap()
}

fn bare(bytes: Vec<u8>) -> SecureChannel<Cursor<Vec<u8>>> {
    SecureChannel::new(
        Cursor::new(bytes),
        ChaCha20Poly1305::new(&[9; 32].into()),
        ChaCha20Poly1305::new(&[9; 32].into()),
    )
}

fn encode(messages: &[&[u8]]) -> Vec<u8> {
    let mut channel = bare(Vec::new());
    for message in messages {
        channel.send_message(message).unwrap();
    }
    channel.into_inner().unwrap().into_inner()
}

fn frame_len(bytes: &[u8]) -> usize {
    4 + u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize
}

fn signed_grant() -> SignedGrant {
    let grant = Grant {
        v: 1,
        session_id: "private-session-name".into(),
        host_principal: "member:host".into(),
        guest_principal: "guest:alice".into(),
        mc_profile: Some("profile-1".into()),
        relay_only: true,
        exp: 1_700_000_120,
        nonce: "42".repeat(32),
    };
    let signature = identity().key.sign(&grant.signing_bytes().unwrap());
    SignedGrant {
        grant,
        signature: signature.as_ref().iter().map(|b| format!("{b:02x}")).collect(),
    }
}

#[test]
fn handshake_and_messages_round_trip_in_both_directions() {
    let (client, relay) = pair();
    let identity = identity();
    let pinned = identity.public_key();
    let task = std::thread::spawn(move || relay_handshake(relay, &identity).unwrap());
    let mut client = client_handshake(client, &pinned).unwrap();
    let mut relay = task.join().unwrap();
    for message in [b"first".as_slice(), b"second", b""] {
        client.send_message(message).unwrap();
        assert_eq!(relay.recv_message().unwrap(), message);
    }
    for message in [b"waiting".as_slice(), b"bridged", b"done"] {
        relay.send_message(message).unwrap();
        assert_eq!(client.recv_message().unwrap(), message);
    }
}

#[test]
fn wrong_pinned_key_is_permission_denied() {
    let (client, relay) = pair();
    let identity = identity();
    let task = std::thread::spawn(move || relay_handshake(relay, &identity));
    let error = client_handshake(client, &[99; 32]).err().expect("wrong pinned key was accepted");
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(task.join().unwrap().is_err());
}

#[test]
fn flipped_data_frame_is_rejected_and_closes_the_channel() {
    let mut bytes = encode(&[b"authenticated payload"]);
    bytes[7] ^= 1;
    let mut channel = bare(bytes);
    let error = channel.recv_message().unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(channel.recv_message().is_err());
    assert!(channel.into_inner().is_err());
}

#[test]
fn replayed_and_reordered_frames_are_rejected() {
    let bytes = encode(&[b"first", b"second"]);
    let first = frame_len(&bytes);
    let mut replayed = bytes[..first].to_vec();
    replayed.extend_from_slice(&bytes[..first]);
    let mut reader = bare(replayed);
    assert_eq!(reader.recv_message().unwrap(), b"first");
    assert_eq!(reader.recv_message().unwrap_err().kind(), io::ErrorKind::InvalidData);

    let mut reordered = bytes[first..].to_vec();
    reordered.extend_from_slice(&bytes[..first]);
    assert_eq!(bare(reordered).recv_message().unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn oversized_frame_is_rejected_before_its_body_is_read() {
    let bytes = ((MAX_FRAME_SIZE as u32) + 1).to_be_bytes().to_vec();
    let mut channel = bare(bytes);
    assert_eq!(channel.recv_message().unwrap_err().kind(), io::ErrorKind::InvalidData);
}

#[test]
fn guest_grant_round_trips_without_plaintext_on_the_wire() {
    let message = ToRelay::Guest { grant: signed_grant() };
    let clear_grant = serde_json::to_vec(match &message {
        ToRelay::Guest { grant } => grant,
        ToRelay::Host { .. } => unreachable!(),
    }).unwrap();
    let mut sender = bare(Vec::new());
    sender.send_to_relay(&message).unwrap();
    let wire = sender.into_inner().unwrap().into_inner();
    assert!(!wire.windows(clear_grant.len()).any(|window| window == clear_grant));
    assert!(!wire.windows(b"private-session-name".len()).any(|window| window == b"private-session-name"));
    let mut receiver = bare(wire);
    match receiver.recv_to_relay().unwrap() {
        ToRelay::Guest { grant } => assert!(grant == match message {
            ToRelay::Guest { grant } => grant,
            ToRelay::Host { .. } => unreachable!(),
        }),
        ToRelay::Host { .. } => panic!("wrong message variant"),
    }
}
