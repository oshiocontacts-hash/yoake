use std::collections::HashMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Mutex;

use ring::{hmac, signature};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_GRANT_LIFETIME_SECS: u64 = 300;
pub const MAX_IDENTIFIER_BYTES: usize = 128;
const DOMAIN: &[u8] = b"yoake-social-join-v1\0";
const TOKEN_DOMAIN: &[u8] = b"yoake-social-host-token-v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Grant {
    pub v: u32,
    pub session_id: String,
    pub host_principal: String,
    pub guest_principal: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mc_profile: Option<String>,
    pub relay_only: bool,
    pub exp: u64,
    pub nonce: String,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedGrant {
    pub grant: Grant,
    pub signature: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantError {
    InvalidFormat,
    UnsupportedVersion,
    RelayOnlyRequired,
    InvalidSignature,
    Expired,
    LifetimeTooLong,
    WrongSession,
    WrongHost,
    Replay,
    ReplayCacheFull,
    Unavailable,
}

impl fmt::Display for GrantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidFormat => "invalid social join grant",
            Self::UnsupportedVersion => "unsupported social join grant version",
            Self::RelayOnlyRequired => "social sessions require relay-only mode",
            Self::InvalidSignature => "social join grant signature is invalid",
            Self::Expired => "social join grant has expired",
            Self::LifetimeTooLong => "social join grant lifetime is too long",
            Self::WrongSession => "social join grant is for another session",
            Self::WrongHost => "social join grant is for another host",
            Self::Replay => "social join grant was already used",
            Self::ReplayCacheFull => "social join replay protection is full",
            Self::Unavailable => "social join verification is unavailable",
        })
    }
}

impl std::error::Error for GrantError {}

fn decode_hex<const LENGTH: usize>(value: &str) -> Option<[u8; LENGTH]> {
    fn digit(value: u8) -> Option<u8> {
        match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            _ => None,
        }
    }

    let bytes = value.as_bytes();
    if bytes.len() != LENGTH * 2 {
        return None;
    }
    let mut result = [0; LENGTH];
    for (index, byte) in result.iter_mut().enumerate() {
        *byte = (digit(bytes[index * 2])? << 4) | digit(bytes[index * 2 + 1])?;
    }
    Some(result)
}

fn append_identifier(output: &mut Vec<u8>, value: &str) -> Result<(), GrantError> {
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES || value.chars().any(char::is_control) {
        return Err(GrantError::InvalidFormat);
    }
    output.extend_from_slice(&(value.len() as u16).to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

impl Grant {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, GrantError> {
        if self.v != VERSION {
            return Err(GrantError::UnsupportedVersion);
        }
        if !self.relay_only {
            return Err(GrantError::RelayOnlyRequired);
        }
        let nonce = decode_hex::<32>(&self.nonce).ok_or(GrantError::InvalidFormat)?;
        let mut output = DOMAIN.to_vec();
        output.extend_from_slice(&self.v.to_be_bytes());
        append_identifier(&mut output, &self.session_id)?;
        append_identifier(&mut output, &self.host_principal)?;
        append_identifier(&mut output, &self.guest_principal)?;
        match &self.mc_profile {
            Some(profile) => {
                output.push(1);
                append_identifier(&mut output, profile)?;
            }
            None => output.push(0),
        }
        output.push(1);
        output.extend_from_slice(&self.exp.to_be_bytes());
        output.extend_from_slice(&nonce);
        Ok(output)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedGrant {
    grant: Grant,
    nonce: [u8; 32],
}

impl VerifiedGrant {
    pub fn session_id(&self) -> &str { &self.grant.session_id }
    pub fn host_principal(&self) -> &str { &self.grant.host_principal }
    pub fn guest_principal(&self) -> &str { &self.grant.guest_principal }
    pub fn mc_profile(&self) -> Option<&str> { self.grant.mc_profile.as_deref() }
    pub fn expires_at(&self) -> u64 { self.grant.exp }
    pub fn nonce(&self) -> [u8; 32] { self.nonce }
}

pub struct AuthorizedConnection<STREAM> {
    stream: STREAM,
    grant: VerifiedGrant,
}

impl<STREAM> AuthorizedConnection<STREAM> {
    pub fn grant(&self) -> &VerifiedGrant {
        &self.grant
    }
}

impl<STREAM: Read> Read for AuthorizedConnection<STREAM> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buffer)
    }
}

impl<STREAM: Write> Write for AuthorizedConnection<STREAM> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.stream.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

pub struct GrantVerifier {
    public_key: [u8; 32],
}

impl GrantVerifier {
    pub fn new(public_key: [u8; 32]) -> Self { Self { public_key } }

    pub fn verify_at(&self, signed: &SignedGrant, session_id: &str, host_principal: &str, now: u64) -> Result<VerifiedGrant, GrantError> {
        let bytes = signed.grant.signing_bytes()?;
        let signature = decode_hex::<64>(&signed.signature).ok_or(GrantError::InvalidFormat)?;
        signature::UnparsedPublicKey::new(&signature::ED25519, self.public_key)
            .verify(&bytes, &signature)
            .map_err(|_| GrantError::InvalidSignature)?;
        if signed.grant.exp <= now { return Err(GrantError::Expired); }
        if signed.grant.exp - now > MAX_GRANT_LIFETIME_SECS { return Err(GrantError::LifetimeTooLong); }
        if signed.grant.session_id != session_id { return Err(GrantError::WrongSession); }
        if signed.grant.host_principal != host_principal { return Err(GrantError::WrongHost); }
        Ok(VerifiedGrant {
            grant: signed.grant.clone(),
            nonce: decode_hex::<32>(&signed.grant.nonce).ok_or(GrantError::InvalidFormat)?,
        })
    }
}

pub struct HostGrantVerifier {
    verifier: GrantVerifier,
    used: Mutex<HashMap<[u8; 32], u64>>,
    max_nonces: usize,
}

impl HostGrantVerifier {
    pub fn new(public_key: [u8; 32], max_nonces: usize) -> Self {
        Self { verifier: GrantVerifier::new(public_key), used: Mutex::new(HashMap::new()), max_nonces }
    }

    pub fn authorize_at(&self, signed: &SignedGrant, session_id: &str, host_principal: &str, now: u64) -> Result<VerifiedGrant, GrantError> {
        let grant = self.verifier.verify_at(signed, session_id, host_principal, now)?;
        let mut used = self.used.lock().map_err(|_| GrantError::Unavailable)?;
        used.retain(|_, expiration| *expiration > now);
        if used.contains_key(&grant.nonce) { return Err(GrantError::Replay); }
        if used.len() >= self.max_nonces { return Err(GrantError::ReplayCacheFull); }
        used.insert(grant.nonce, grant.expires_at());
        Ok(grant)
    }

    pub fn bind_connection_at<STREAM>(
        &self,
        stream: STREAM,
        signed: &SignedGrant,
        session_id: &str,
        host_principal: &str,
        now: u64,
    ) -> Result<AuthorizedConnection<STREAM>, GrantError> {
        let grant = self.authorize_at(signed, session_id, host_principal, now)?;
        Ok(AuthorizedConnection { stream, grant })
    }
}

pub struct HostTokenVerifier {
    tag: hmac::Tag,
}

impl HostTokenVerifier {
    pub fn new(token: &str) -> Result<Self, GrantError> {
        let token = decode_hex::<32>(token).ok_or(GrantError::InvalidFormat)?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, &token);
        Ok(Self { tag: hmac::sign(&key, TOKEN_DOMAIN) })
    }

    pub fn verify(&self, token: &str) -> bool {
        let Some(token) = decode_hex::<32>(token) else { return false; };
        let key = hmac::Key::new(hmac::HMAC_SHA256, &token);
        hmac::verify(&key, TOKEN_DOMAIN, self.tag.as_ref()).is_ok()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterSession {
    pub session_id: String,
    pub host_principal: String,
    pub relay_only: bool,
}

impl RegisterSession {
    pub fn relay_only(session_id: String, host_principal: String) -> Self {
        Self { session_id, host_principal, relay_only: true }
    }

    pub fn validate(&self) -> Result<(), GrantError> {
        if !self.relay_only { return Err(GrantError::RelayOnlyRequired); }
        let mut bytes = Vec::new();
        append_identifier(&mut bytes, &self.session_id)?;
        append_identifier(&mut bytes, &self.host_principal)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisteredSession {
    pub session_id: String,
    pub host_principal: String,
    pub relay_only: bool,
    pub host_token: String,
}

impl RegisteredSession {
    pub fn validate_for(&self, request: &RegisterSession) -> Result<(), GrantError> {
        request.validate()?;
        if !self.relay_only { return Err(GrantError::RelayOnlyRequired); }
        if self.session_id != request.session_id { return Err(GrantError::WrongSession); }
        if self.host_principal != request.host_principal { return Err(GrantError::WrongHost); }
        HostTokenVerifier::new(&self.host_token)?;
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnregisterSession {
    pub session_id: String,
}

pub trait SessionPortal {
    type Error;
    fn register(&self, request: &RegisterSession) -> Result<RegisteredSession, Self::Error>;
    fn unregister(&self, request: &UnregisterSession) -> Result<(), Self::Error>;
}

#[derive(Debug)]
pub enum RegistrationError<ERROR> {
    Portal(ERROR),
    InvalidResponse(GrantError),
}

pub fn register_session<PORTAL: SessionPortal>(portal: &PORTAL, request: &RegisterSession) -> Result<RegisteredSession, RegistrationError<PORTAL::Error>> {
    request.validate().map_err(RegistrationError::InvalidResponse)?;
    let registered = portal.register(request).map_err(RegistrationError::Portal)?;
    registered.validate_for(request).map_err(RegistrationError::InvalidResponse)?;
    Ok(registered)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToRelay {
    Guest { grant: SignedGrant },
    Host { grant: SignedGrant, token: String },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FromRelay {
    Waiting,
    Bridged,
    Rejected,
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn signed_grant() -> SignedGrant {
        let grant = Grant { v: 1, session_id: "session-1".into(), host_principal: "member:host".into(), guest_principal: "guest:alice".into(), mc_profile: Some("profile-1".into()), relay_only: true, exp: 1_700_000_120, nonce: "00".repeat(32) };
        let key = Ed25519KeyPair::from_seed_unchecked(&decode_hex::<32>(SEED).unwrap()).unwrap();
        assert_eq!(key.public_key().as_ref(), decode_hex::<32>(PUBLIC).unwrap());
        let signature = key.sign(&grant.signing_bytes().unwrap()).as_ref().iter().map(|byte| format!("{byte:02x}")).collect();
        SignedGrant { grant, signature }
    }

    #[test]
    fn canonical_grant_encoding_matches_the_documented_vector() {
        let grant = Grant {
            v: 1,
            session_id: "s".into(),
            host_principal: "h".into(),
            guest_principal: "g".into(),
            mc_profile: None,
            relay_only: true,
            exp: 120,
            nonce: "00".repeat(32),
        };
        let mut expected = b"yoake-social-join-v1\0".to_vec();
        expected.extend_from_slice(&[
            0, 0, 0, 1, 0, 1, b's', 0, 1, b'h', 0, 1, b'g', 0, 1, 0, 0, 0, 0, 0, 0,
            0, 120,
        ]);
        expected.extend_from_slice(&[0; 32]);
        assert_eq!(grant.signing_bytes().unwrap(), expected);
    }

    #[test]
    fn verifies_rfc8032_ed25519_test_vector() {
        let signature = concat!(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155",
            "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
        signature::UnparsedPublicKey::new(&signature::ED25519, decode_hex::<32>(PUBLIC).unwrap())
            .verify(b"", &decode_hex::<64>(signature).unwrap())
            .unwrap();
    }

    #[test]
    fn verifies_grants_and_rejects_expired_wrong_key_host_and_replay() {
        let signed = signed_grant();
        let key = decode_hex::<32>(PUBLIC).unwrap();
        let verifier = HostGrantVerifier::new(key, 4);
        let verified = verifier.authorize_at(&signed, "session-1", "member:host", 1_700_000_000).unwrap();
        assert_eq!(verified.guest_principal(), "guest:alice");
        assert_eq!(verified.mc_profile(), Some("profile-1"));
        assert!(matches!(verifier.authorize_at(&signed, "session-1", "member:host", 1_700_000_000), Err(GrantError::Replay)));
        let verifier = GrantVerifier::new(key);
        assert!(matches!(verifier.verify_at(&signed, "session-1", "member:host", 1_700_000_120), Err(GrantError::Expired)));
        assert!(matches!(verifier.verify_at(&signed, "session-1", "member:other", 1_700_000_000), Err(GrantError::WrongHost)));
        assert!(matches!(verifier.verify_at(&signed, "session-2", "member:host", 1_700_000_000), Err(GrantError::WrongSession)));
        assert!(matches!(verifier.verify_at(&signed, "session-1", "member:host", 1_699_999_819), Err(GrantError::LifetimeTooLong)));
        assert!(matches!(GrantVerifier::new([1; 32]).verify_at(&signed, "session-1", "member:host", 1_700_000_000), Err(GrantError::InvalidSignature)));
        let mut altered = signed.clone();
        altered.grant.guest_principal = "guest:mallory".into();
        assert!(matches!(verifier.verify_at(&altered, "session-1", "member:host", 1_700_000_000), Err(GrantError::InvalidSignature)));
        assert!(matches!(HostGrantVerifier::new(key, 0).authorize_at(&signed, "session-1", "member:host", 1_700_000_000), Err(GrantError::ReplayCacheFull)));
    }

    #[test]
    fn accepted_stream_retains_the_verified_guest_principal() {
        let signed = signed_grant();
        let verifier = HostGrantVerifier::new(decode_hex::<32>(PUBLIC).unwrap(), 4);
        let stream = std::io::Cursor::new(b"hello".to_vec());
        let mut connection = verifier
            .bind_connection_at(stream, &signed, "session-1", "member:host", 1_700_000_000)
            .unwrap();
        assert_eq!(connection.grant().guest_principal(), "guest:alice");
        let mut bytes = [0; 5];
        connection.read_exact(&mut bytes).unwrap();
        assert_eq!(&bytes, b"hello");
        assert!(matches!(
            verifier.bind_connection_at(std::io::empty(), &signed, "session-1", "member:host", 1_700_000_000),
            Err(GrantError::Replay)
        ));
    }

    #[test]
    fn social_messages_do_not_expose_or_accept_peer_addresses() {
        let signed = signed_grant();
        let requests = [
            ToRelay::Guest { grant: signed.clone() },
            ToRelay::Host { grant: signed, token: "ab".repeat(32) },
        ];
        for request in requests {
            let serialized = serde_json::to_string(&request).unwrap();
            assert!(!serialized.contains("candidate"));
            assert!(!serialized.contains("observed"));
            assert!(!serialized.contains("192.0.2.1"));
        }
        let serialized = serde_json::to_string(&ToRelay::Guest { grant: signed_grant() }).unwrap();
        let injected = serialized.replacen('{', "{\"candidate\":\"192.0.2.1:1234\",", 1);
        assert!(serde_json::from_str::<ToRelay>(&injected).is_err());
        assert!(serde_json::from_str::<FromRelay>(r#"{"type":"bridged","observed":"192.0.2.1:1234"}"#).is_err());
        assert_eq!(serde_json::to_string(&FromRelay::Bridged).unwrap(), r#"{"type":"bridged"}"#);
    }

    struct FakePortal { available: bool }

    impl SessionPortal for FakePortal {
        type Error = &'static str;
        fn register(&self, request: &RegisterSession) -> Result<RegisteredSession, Self::Error> {
            if !self.available { return Err("offline"); }
            Ok(RegisteredSession { session_id: request.session_id.clone(), host_principal: request.host_principal.clone(), relay_only: true, host_token: "ab".repeat(32) })
        }
        fn unregister(&self, _: &UnregisterSession) -> Result<(), Self::Error> { Ok(()) }
    }

    #[test]
    fn registration_fails_closed_when_portal_is_unavailable() {
        let request = RegisterSession::relay_only("session-1".into(), "member:host".into());
        assert!(register_session(&FakePortal { available: true }, &request).is_ok());
        assert!(matches!(register_session(&FakePortal { available: false }, &request), Err(RegistrationError::Portal("offline"))));
    }
}
