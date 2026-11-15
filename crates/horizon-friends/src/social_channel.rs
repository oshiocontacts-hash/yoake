//! Authenticated, confidential control messages between a client and a relay.
//! The pinned Ed25519 key authenticates a signed, ephemeral X25519 exchange;
//! there is deliberately no plaintext fallback.
use std::io::{self, Read, Write};

use chacha20poly1305::{ChaCha20Poly1305, KeyInit, aead::{Aead, Payload}};
use hkdf::Hkdf;
use ring::{agreement, rand::SystemRandom, signature::{self, Ed25519KeyPair, KeyPair}};
use sha2::{Digest, Sha256};

use crate::social::{FromRelay, ToRelay};

pub const MAX_MESSAGE_SIZE: usize = 16 * 1024;
pub const MAX_FRAME_SIZE: usize = MAX_MESSAGE_SIZE + 1 + TAG;
const LABEL: &[u8] = b"yoake-social-channel-v1";
const CONFIRM: &[u8] = b"yoake-social-channel-v1 confirm";
const VERSION: u8 = 1;
const TAG: usize = 16;
const CLIENT_HELLO: usize = 1 + 32 + 32;
const RELAY_HELLO: usize = 32 + 32 + 64;
const DATA: u8 = 0;
const CONFIRMATION: u8 = 1;

/// The relay's long-term signing identity. The PKCS#8 document is validated by ring.
pub struct RelayIdentity {
    key: Ed25519KeyPair,
}

impl RelayIdentity {
    pub fn from_pkcs8(pkcs8: &[u8]) -> io::Result<Self> {
        let key = Ed25519KeyPair::from_pkcs8(pkcs8)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid relay Ed25519 PKCS#8 key"))?;
        Ok(Self { key })
    }

    #[cfg(test)]
    pub fn from_seed(seed: &[u8; 32]) -> io::Result<Self> {
        let key = Ed25519KeyPair::from_seed_unchecked(seed)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid relay Ed25519 seed"))?;
        Ok(Self { key })
    }

    pub fn public_key(&self) -> [u8; 32] {
        let mut output = [0; 32];
        output.copy_from_slice(self.key.public_key().as_ref());
        output
    }
}

#[derive(Clone, Copy)]
enum Role { Client, Relay }

pub fn client_handshake<S: Read + Write>(
    mut stream: S,
    pinned_relay_key: &[u8; 32],
) -> io::Result<SecureChannel<S>> {
    let rng = SystemRandom::new();
    let private = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &rng)
        .map_err(|_| io::Error::other("could not generate the client ephemeral key"))?;
    let public = private.compute_public_key()
        .map_err(|_| io::Error::other("could not compute the client ephemeral public key"))?;
    let mut body = [0; CLIENT_HELLO];
    body[0] = VERSION;
    body[1..33].copy_from_slice(public.as_ref());
    crate::fill_random(&mut body[33..])?;
    let mut hello = Vec::with_capacity(4 + body.len());
    hello.extend_from_slice(&(body.len() as u32).to_be_bytes());
    hello.extend_from_slice(&body);
    stream.write_all(&hello)?;
    stream.flush()?;

    let relay = read_hello::<RELAY_HELLO, _>(&mut stream)?;
    let transcript = transcript(&hello, &relay[..64]);
    signature::UnparsedPublicKey::new(&signature::ED25519, pinned_relay_key)
        .verify(&transcript, &relay[64..])
        .map_err(|_| io::Error::new(io::ErrorKind::PermissionDenied, "relay signature does not match the pinned key"))?;
    let channel = agree(stream, private, &relay[..32], &transcript, Role::Client)?;
    confirm(channel)
}

pub fn relay_handshake<S: Read + Write>(
    mut stream: S,
    identity: &RelayIdentity,
) -> io::Result<SecureChannel<S>> {
    let hello = read_prefixed(&mut stream)?;
    if hello.len() != CLIENT_HELLO || hello[0] != VERSION {
        return Err(invalid("invalid social channel client hello"));
    }
    let rng = SystemRandom::new();
    let private = agreement::EphemeralPrivateKey::generate(&agreement::X25519, &rng)
        .map_err(|_| io::Error::other("could not generate the relay ephemeral key"))?;
    let public = private.compute_public_key()
        .map_err(|_| io::Error::other("could not compute the relay ephemeral public key"))?;
    let mut body = [0; 64];
    body[..32].copy_from_slice(public.as_ref());
    crate::fill_random(&mut body[32..])?;
    let mut prefixed_client = Vec::with_capacity(4 + hello.len());
    prefixed_client.extend_from_slice(&(hello.len() as u32).to_be_bytes());
    prefixed_client.extend_from_slice(&hello);
    let transcript = transcript(&prefixed_client, &body);
    let signature = identity.key.sign(&transcript);
    let mut response = Vec::with_capacity(4 + RELAY_HELLO);
    response.extend_from_slice(&(RELAY_HELLO as u32).to_be_bytes());
    response.extend_from_slice(&body);
    response.extend_from_slice(signature.as_ref());
    stream.write_all(&response)?;
    stream.flush()?;
    let channel = agree(stream, private, &hello[1..33], &transcript, Role::Relay)?;
    confirm(channel)
}

fn transcript(client_hello: &[u8], relay_body: &[u8]) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(LABEL);
    hash.update(client_hello);
    hash.update((RELAY_HELLO as u32).to_be_bytes());
    hash.update(relay_body);
    hash.finalize().into()
}

fn read_prefixed<S: Read>(stream: &mut S) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length != CLIENT_HELLO {
        return Err(invalid("invalid social channel hello length"));
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(body)
}

fn read_hello<const N: usize, S: Read>(stream: &mut S) -> io::Result<[u8; N]> {
    let mut header = [0; 4];
    stream.read_exact(&mut header)?;
    if u32::from_be_bytes(header) as usize != N {
        return Err(invalid("invalid social channel relay hello length"));
    }
    let mut body = [0; N];
    stream.read_exact(&mut body)?;
    Ok(body)
}

fn agree<S: Read + Write>(
    stream: S,
    private: agreement::EphemeralPrivateKey,
    peer: &[u8],
    transcript: &[u8; 32],
    role: Role,
) -> io::Result<SecureChannel<S>> {
    agreement::agree_ephemeral(
        private,
        &agreement::UnparsedPublicKey::new(&agreement::X25519, peer),
        |secret| {
            let hkdf = Hkdf::<Sha256>::new(Some(transcript), secret);
            let (client, relay) = (expand(&hkdf, b"client->relay"), expand(&hkdf, b"relay->client"));
            match role {
                Role::Client => SecureChannel::new(stream, client, relay),
                Role::Relay => SecureChannel::new(stream, relay, client),
            }
        },
    ).map_err(|_| invalid("social channel X25519 agreement failed"))
}

fn expand(hkdf: &Hkdf<Sha256>, direction: &[u8]) -> ChaCha20Poly1305 {
    let mut key = [0; 32];
    hkdf.expand(&[LABEL, b" ", direction].concat(), &mut key)
        .expect("32 bytes is a valid HKDF output");
    ChaCha20Poly1305::new(&key.into())
}

fn confirm<S: Read + Write>(mut channel: SecureChannel<S>) -> io::Result<SecureChannel<S>> {
    channel.write_frame(CONFIRMATION, CONFIRM)?;
    channel.stream.flush()?;
    let (kind, payload) = channel.read_frame()?;
    if kind != CONFIRMATION || payload != CONFIRM {
        return Err(invalid("invalid social channel key confirmation"));
    }
    Ok(channel)
}

fn nonce(counter: u64) -> [u8; 12] {
    let mut output = [0; 12];
    output[4..].copy_from_slice(&counter.to_be_bytes());
    output
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

pub struct SecureChannel<S> {
    stream: S,
    send: ChaCha20Poly1305,
    recv: ChaCha20Poly1305,
    send_counter: u64,
    recv_counter: u64,
    failed: bool,
}

impl<S: Read + Write> SecureChannel<S> {
    fn new(stream: S, send: ChaCha20Poly1305, recv: ChaCha20Poly1305) -> Self {
        Self { stream, send, recv, send_counter: 0, recv_counter: 0, failed: false }
    }

    pub fn send_message(&mut self, bytes: &[u8]) -> io::Result<()> {
        let result = if bytes.len() > MAX_MESSAGE_SIZE {
            Err(io::Error::new(io::ErrorKind::InvalidInput, "social channel message is too large"))
        } else if self.failed {
            Err(invalid("social channel is closed after an earlier error"))
        } else {
            self.write_frame(DATA, bytes).and_then(|()| self.stream.flush())
        };
        self.track(result)
    }

    pub fn recv_message(&mut self) -> io::Result<Vec<u8>> {
        let result = if self.failed {
            Err(invalid("social channel is closed after an earlier error"))
        } else {
            self.read_frame().and_then(|(kind, payload)| {
                if kind == DATA { Ok(payload) } else { Err(invalid("unexpected social channel frame")) }
            })
        };
        self.track(result)
    }

    pub fn send_to_relay(&mut self, message: &ToRelay) -> io::Result<()> {
        let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
        self.send_message(&bytes)
    }

    pub fn recv_to_relay(&mut self) -> io::Result<ToRelay> {
        let bytes = self.recv_message()?;
        let result = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("invalid ToRelay JSON message"));
        self.track(result)
    }

    pub fn send_from_relay(&mut self, message: &FromRelay) -> io::Result<()> {
        let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
        self.send_message(&bytes)
    }

    pub fn recv_from_relay(&mut self) -> io::Result<FromRelay> {
        let bytes = self.recv_message()?;
        let result = serde_json::from_slice(&bytes)
            .map_err(|_| invalid("invalid FromRelay JSON message"));
        self.track(result)
    }

    /// Recover the stream only while the channel is error-free. The peer must
    /// have finished using encryption; raw plaintext must never be exchanged.
    pub fn into_inner(self) -> io::Result<S> {
        if self.failed { Err(invalid("cannot recover a failed social channel")) } else { Ok(self.stream) }
    }

    fn track<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() { self.failed = true; }
        result
    }

    fn write_frame(&mut self, kind: u8, bytes: &[u8]) -> io::Result<()> {
        let mut plain = Vec::with_capacity(bytes.len() + 1);
        plain.push(kind);
        plain.extend_from_slice(bytes);
        let header = u32::try_from(plain.len() + TAG).map_err(io::Error::other)?.to_be_bytes();
        let sealed = self.send.encrypt(&nonce(self.send_counter).into(), Payload { msg: &plain, aad: &header })
            .map_err(|_| io::Error::other("social channel encryption failed"))?;
        self.send_counter = self.send_counter.checked_add(1)
            .ok_or_else(|| io::Error::other("social channel send counter exhausted"))?;
        self.stream.write_all(&header)?;
        self.stream.write_all(&sealed)
    }

    fn read_frame(&mut self) -> io::Result<(u8, Vec<u8>)> {
        let mut header = [0; 4];
        self.stream.read_exact(&mut header)?;
        let length = u32::from_be_bytes(header) as usize;
        if !(TAG + 1..=MAX_FRAME_SIZE).contains(&length) {
            return Err(invalid("invalid social channel frame length"));
        }
        let mut sealed = vec![0; length];
        self.stream.read_exact(&mut sealed)?;
        let plain = self.recv.decrypt(&nonce(self.recv_counter).into(), Payload { msg: &sealed, aad: &header })
            .map_err(|_| invalid("social channel frame authentication failed"))?;
        self.recv_counter = self.recv_counter.checked_add(1)
            .ok_or_else(|| io::Error::other("social channel receive counter exhausted"))?;
        Ok((plain[0], plain[1..].to_vec()))
    }
}

#[cfg(test)]
#[path = "social_channel_tests.rs"]
mod tests;
