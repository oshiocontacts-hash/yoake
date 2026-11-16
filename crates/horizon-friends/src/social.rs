use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Mutex;

use ring::{digest, hmac, signature};
use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
pub const MAX_GRANT_LIFETIME_SECS: u64 = 300;
const CLIENT_CLOCK_SKEW_SECS: u64 = 60;
pub const MAX_IDENTIFIER_BYTES: usize = 128;
const DOMAIN: &[u8] = b"yoake-social-join-v1\0";
const TOKEN_DOMAIN: &[u8] = b"yoake-social-host-token-v1";
pub const PROVISION_VERSION: u32 = 1;
/// A provision ticket may not expire further ahead than this.
pub const MAX_PROVISION_LIFETIME_SECS: u64 = 300;
/// A provisioned session may not be set to end further ahead than this.
pub const MAX_SESSION_LIFETIME_SECS: u64 = 12 * 60 * 60;
/// Smallest burst a ticket may grant: one maximum control message.
pub const MIN_PROVISION_BURST_BYTES: u64 = 16 * 1024;
const PROVISION_DOMAIN: &[u8] = b"yoake-social-provision-v1\0";

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

/// Lowercase hexadecimal only (`[0-9a-f]`), exact length, byte-wise so multibyte text cannot panic.
pub fn decode_hex<const LENGTH: usize>(value: &str) -> Option<[u8; LENGTH]> {
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

/// The identifier rule shared by every id in the social protocol.
pub fn validate_identifier(value: &str) -> Result<(), GrantError> {
    append_identifier(&mut Vec::new(), value)
}

fn append_identifier(output: &mut Vec<u8>, value: &str) -> Result<(), GrantError> {
    if value.is_empty() || value.len() > MAX_IDENTIFIER_BYTES || value.chars().any(char::is_control)
    {
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

/// What the portal signs so a host can open its own session on the relay. JSON like `Grant`:
/// the 32-byte values are lowercase hex, and unknown fields are refused.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionTicket {
    pub v: u32,
    pub session_id: String,
    pub host_principal: String,
    /// SHA-256 of the 32 raw host token bytes: 64 lowercase hex characters.
    pub token_sha256: String,
    pub bytes_per_second: u64,
    pub burst_bytes: u64,
    pub max_connections: u32,
    pub idle_secs: u32,
    /// Unix time at which the session itself ends (at most 12 hours ahead).
    pub session_exp: u64,
    /// The relay this ticket is for; the relay refuses any other id.
    pub relay_id: String,
    pub exp: u64,
    pub nonce: String,
}

impl ProvisionTicket {
    /// The exact bytes the portal signs. The format and version checks live here.
    pub fn signing_bytes(&self) -> Result<Vec<u8>, GrantError> {
        if self.v != PROVISION_VERSION {
            return Err(GrantError::UnsupportedVersion);
        }
        let token_sha256 = decode_hex::<32>(&self.token_sha256).ok_or(GrantError::InvalidFormat)?;
        let nonce = decode_hex::<32>(&self.nonce).ok_or(GrantError::InvalidFormat)?;
        let mut output = PROVISION_DOMAIN.to_vec();
        output.extend_from_slice(&self.v.to_be_bytes());
        append_identifier(&mut output, &self.session_id)?;
        append_identifier(&mut output, &self.host_principal)?;
        output.extend_from_slice(&token_sha256);
        output.extend_from_slice(&self.bytes_per_second.to_be_bytes());
        output.extend_from_slice(&self.burst_bytes.to_be_bytes());
        output.extend_from_slice(&self.max_connections.to_be_bytes());
        output.extend_from_slice(&self.idle_secs.to_be_bytes());
        output.extend_from_slice(&self.session_exp.to_be_bytes());
        append_identifier(&mut output, &self.relay_id)?;
        output.extend_from_slice(&self.exp.to_be_bytes());
        output.extend_from_slice(&nonce);
        Ok(output)
    }
}

/// A ticket whose signature, version and time window were checked.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedTicket {
    ticket: ProvisionTicket,
    token_sha256: [u8; 32],
    nonce: [u8; 32],
}

impl VerifiedTicket {
    pub fn ticket(&self) -> &ProvisionTicket {
        &self.ticket
    }
    pub fn nonce(&self) -> [u8; 32] {
        self.nonce
    }
    pub fn expires_at(&self) -> u64 {
        self.ticket.exp
    }

    /// True when `host_token` (64 lowercase hex characters) hashes to the signed digest.
    pub fn token_matches(&self, host_token: &str) -> bool {
        let Some(token) = decode_hex::<32>(host_token) else {
            return false;
        };
        let digest = digest::digest(&digest::SHA256, &token);
        // Both sides are public digests, so a plain comparison leaks nothing.
        digest.as_ref() == self.token_sha256
    }

    /// The same digest of the stored token, for the idempotent repeat of a provision.
    pub fn token_sha256(&self) -> [u8; 32] {
        self.token_sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedGrant {
    grant: Grant,
    nonce: [u8; 32],
}

impl VerifiedGrant {
    pub fn session_id(&self) -> &str {
        &self.grant.session_id
    }
    pub fn host_principal(&self) -> &str {
        &self.grant.host_principal
    }
    pub fn guest_principal(&self) -> &str {
        &self.grant.guest_principal
    }
    pub fn mc_profile(&self) -> Option<&str> {
        self.grant.mc_profile.as_deref()
    }
    pub fn expires_at(&self) -> u64 {
        self.grant.exp
    }
    pub fn nonce(&self) -> [u8; 32] {
        self.nonce
    }
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
    pub fn new(public_key: [u8; 32]) -> Self {
        Self { public_key }
    }

    /// Format and Ed25519 signature only; run this before looking anything else up.
    pub fn verify_signature<'a>(
        &self,
        signed: &'a SignedGrant,
    ) -> Result<SignatureChecked<'a>, GrantError> {
        let bytes = signed.grant.signing_bytes()?;
        let signature = decode_hex::<64>(&signed.signature).ok_or(GrantError::InvalidFormat)?;
        signature::UnparsedPublicKey::new(&signature::ED25519, self.public_key)
            .verify(&bytes, &signature)
            .map_err(|_| GrantError::InvalidSignature)?;
        Ok(SignatureChecked { signed })
    }

    /// Signature, version, format and time window of a provision ticket, with the same pinned
    /// portal key as grants. No state is touched; the relay checks nonce, token and limits next.
    pub fn verify_ticket(
        &self,
        ticket: &ProvisionTicket,
        signature_hex: &str,
        now: u64,
    ) -> Result<VerifiedTicket, GrantError> {
        self.verify_ticket_with_skew(ticket, signature_hex, now, 0)
    }

    pub fn verify_client_ticket(
        &self,
        ticket: &ProvisionTicket,
        signature_hex: &str,
        now: u64,
    ) -> Result<VerifiedTicket, GrantError> {
        self.verify_ticket_with_skew(ticket, signature_hex, now, CLIENT_CLOCK_SKEW_SECS)
    }

    fn verify_ticket_with_skew(
        &self,
        ticket: &ProvisionTicket,
        signature_hex: &str,
        now: u64,
        skew: u64,
    ) -> Result<VerifiedTicket, GrantError> {
        let bytes = ticket.signing_bytes()?;
        let signature = decode_hex::<64>(signature_hex).ok_or(GrantError::InvalidFormat)?;
        signature::UnparsedPublicKey::new(&signature::ED25519, self.public_key)
            .verify(&bytes, &signature)
            .map_err(|_| GrantError::InvalidSignature)?;
        if ticket.exp <= now {
            return Err(GrantError::Expired);
        }
        if ticket.exp - now > MAX_PROVISION_LIFETIME_SECS + skew {
            return Err(GrantError::LifetimeTooLong);
        }
        if ticket.session_exp <= now {
            return Err(GrantError::Expired);
        }
        if ticket.session_exp - now > MAX_SESSION_LIFETIME_SECS + skew {
            return Err(GrantError::LifetimeTooLong);
        }
        Ok(VerifiedTicket {
            ticket: ticket.clone(),
            token_sha256: decode_hex::<32>(&ticket.token_sha256)
                .ok_or(GrantError::InvalidFormat)?,
            nonce: decode_hex::<32>(&ticket.nonce).ok_or(GrantError::InvalidFormat)?,
        })
    }

    pub fn verify_at(
        &self,
        signed: &SignedGrant,
        session_id: &str,
        host_principal: &str,
        now: u64,
    ) -> Result<VerifiedGrant, GrantError> {
        self.verify_signature(signed)?
            .check(session_id, host_principal, now)
    }

    pub fn verify_client_at(
        &self,
        signed: &SignedGrant,
        session_id: &str,
        host_principal: &str,
        now: u64,
    ) -> Result<VerifiedGrant, GrantError> {
        self.verify_signature(signed)?.check_with_skew(
            session_id,
            host_principal,
            now,
            CLIENT_CLOCK_SKEW_SECS,
        )
    }
}

/// A grant whose signature has been verified; only this can produce a `VerifiedGrant`.
pub struct SignatureChecked<'a> {
    signed: &'a SignedGrant,
}

impl SignatureChecked<'_> {
    /// Time, session and host claims. There is no clock-skew allowance.
    pub fn check(
        &self,
        session_id: &str,
        host_principal: &str,
        now: u64,
    ) -> Result<VerifiedGrant, GrantError> {
        self.check_with_skew(session_id, host_principal, now, 0)
    }

    fn check_with_skew(
        &self,
        session_id: &str,
        host_principal: &str,
        now: u64,
        skew: u64,
    ) -> Result<VerifiedGrant, GrantError> {
        let grant = &self.signed.grant;
        if grant.exp <= now {
            return Err(GrantError::Expired);
        }
        if grant.exp - now > MAX_GRANT_LIFETIME_SECS + skew {
            return Err(GrantError::LifetimeTooLong);
        }
        if grant.session_id != session_id {
            return Err(GrantError::WrongSession);
        }
        if grant.host_principal != host_principal {
            return Err(GrantError::WrongHost);
        }
        Ok(VerifiedGrant {
            grant: grant.clone(),
            nonce: decode_hex::<32>(&grant.nonce).ok_or(GrantError::InvalidFormat)?,
        })
    }
}

pub struct HostGrantVerifier {
    verifier: GrantVerifier,
    used: Mutex<HashMap<[u8; 32], u64>>,
    pending: Mutex<HashSet<[u8; 32]>>,
    max_nonces: usize,
}

impl HostGrantVerifier {
    pub fn new(public_key: [u8; 32], max_nonces: usize) -> Self {
        Self {
            verifier: GrantVerifier::new(public_key),
            used: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashSet::new()),
            max_nonces,
        }
    }

    pub fn authorize_at(
        &self,
        signed: &SignedGrant,
        session_id: &str,
        host_principal: &str,
        now: u64,
    ) -> Result<VerifiedGrant, GrantError> {
        let grant = self
            .verifier
            .verify_at(signed, session_id, host_principal, now)?;
        let mut used = self.used.lock().map_err(|_| GrantError::Unavailable)?;
        let pending = self.pending.lock().map_err(|_| GrantError::Unavailable)?;
        used.retain(|_, expiration| *expiration > now);
        if used.contains_key(&grant.nonce) || pending.contains(&grant.nonce) {
            return Err(GrantError::Replay);
        }
        if used.len() + pending.len() >= self.max_nonces {
            return Err(GrantError::ReplayCacheFull);
        }
        used.insert(grant.nonce, grant.expires_at());
        Ok(grant)
    }

    pub fn begin_client_at(
        &self,
        signed: &SignedGrant,
        session_id: &str,
        host_principal: &str,
        now: u64,
    ) -> Result<PendingGrant<'_>, GrantError> {
        let grant = self
            .verifier
            .verify_client_at(signed, session_id, host_principal, now)?;
        let mut used = self.used.lock().map_err(|_| GrantError::Unavailable)?;
        let mut pending = self.pending.lock().map_err(|_| GrantError::Unavailable)?;
        used.retain(|_, expiration| *expiration > now);
        if used.contains_key(&grant.nonce) || pending.contains(&grant.nonce) {
            return Err(GrantError::Replay);
        }
        if used.len() + pending.len() >= self.max_nonces {
            return Err(GrantError::ReplayCacheFull);
        }
        pending.insert(grant.nonce);
        Ok(PendingGrant {
            verifier: self,
            grant,
        })
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

pub struct PendingGrant<'a> {
    verifier: &'a HostGrantVerifier,
    grant: VerifiedGrant,
}

impl PendingGrant<'_> {
    pub fn expires_at(&self) -> u64 {
        self.grant.expires_at()
    }

    pub fn bridged(self) -> Result<(), GrantError> {
        self.verifier
            .used
            .lock()
            .map_err(|_| GrantError::Unavailable)?
            .insert(self.grant.nonce, self.grant.expires_at());
        Ok(())
    }
}

impl Drop for PendingGrant<'_> {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.verifier.pending.lock() {
            pending.remove(&self.grant.nonce);
        }
    }
}

pub struct HostTokenVerifier {
    tag: hmac::Tag,
}

impl HostTokenVerifier {
    pub fn new(token: &str) -> Result<Self, GrantError> {
        let token = decode_hex::<32>(token).ok_or(GrantError::InvalidFormat)?;
        let key = hmac::Key::new(hmac::HMAC_SHA256, &token);
        Ok(Self {
            tag: hmac::sign(&key, TOKEN_DOMAIN),
        })
    }

    pub fn verify(&self, token: &str) -> bool {
        let Some(token) = decode_hex::<32>(token) else {
            return false;
        };
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
        Self {
            session_id,
            host_principal,
            relay_only: true,
        }
    }

    pub fn validate(&self) -> Result<(), GrantError> {
        if !self.relay_only {
            return Err(GrantError::RelayOnlyRequired);
        }
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
        if !self.relay_only {
            return Err(GrantError::RelayOnlyRequired);
        }
        if self.session_id != request.session_id {
            return Err(GrantError::WrongSession);
        }
        if self.host_principal != request.host_principal {
            return Err(GrantError::WrongHost);
        }
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

pub fn register_session<PORTAL: SessionPortal>(
    portal: &PORTAL,
    request: &RegisterSession,
) -> Result<RegisteredSession, RegistrationError<PORTAL::Error>> {
    request
        .validate()
        .map_err(RegistrationError::InvalidResponse)?;
    let registered = portal
        .register(request)
        .map_err(RegistrationError::Portal)?;
    registered
        .validate_for(request)
        .map_err(RegistrationError::InvalidResponse)?;
    Ok(registered)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToRelay {
    Guest {
        grant: SignedGrant,
    },
    Host {
        grant: SignedGrant,
        token: String,
    },
    /// A host opens its own session with a portal-signed ticket and the token the ticket commits to.
    Provision {
        ticket: ProvisionTicket,
        signature: String,
        host_token: String,
    },
    /// A host closes a session it opened with a ticket, before its `session_exp`.
    Retire {
        session_id: String,
        host_token: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
// Empty struct variants, not unit variants: serde ignores deny_unknown_fields on internally tagged unit variants.
pub enum FromRelay {
    Waiting {},
    Bridged {},
    Rejected {},
    Provisioned {},
    Retired {},
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const SEED: &str = "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60";
    const PUBLIC: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn signed_grant() -> SignedGrant {
        let grant = Grant {
            v: 1,
            session_id: "session-1".into(),
            host_principal: "member:host".into(),
            guest_principal: "guest:alice".into(),
            mc_profile: Some("profile-1".into()),
            relay_only: true,
            exp: 1_700_000_120,
            nonce: "00".repeat(32),
        };
        let key = Ed25519KeyPair::from_seed_unchecked(&decode_hex::<32>(SEED).unwrap()).unwrap();
        assert_eq!(key.public_key().as_ref(), decode_hex::<32>(PUBLIC).unwrap());
        let signature = key
            .sign(&grant.signing_bytes().unwrap())
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        SignedGrant { grant, signature }
    }

    fn vector_ticket() -> ProvisionTicket {
        ProvisionTicket {
            v: 1,
            session_id: "session-1".into(),
            host_principal: "member:host".into(),
            token_sha256: "9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885".into(),
            bytes_per_second: 1_000_000,
            burst_bytes: 65_536,
            max_connections: 8,
            idle_secs: 60,
            session_exp: 1_700_043_200,
            relay_id: "relay-1".into(),
            exp: 1_700_000_120,
            nonce: "11".repeat(32),
        }
    }

    const VECTOR_BYTES: &str = concat!(
        "796f616b652d736f6369616c2d70726f766973696f6e2d7631",
        "00",
        "00000001",
        "000973657373696f6e2d31",
        "000b6d656d6265723a686f7374",
        "9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885",
        "00000000000f4240",
        "0000000000010000",
        "00000008",
        "0000003c",
        "00000000655499c0",
        "000772656c61792d31",
        "000000006553f178",
        "1111111111111111111111111111111111111111111111111111111111111111",
    );
    const VECTOR_SIGNATURE: &str = concat!(
        "be08b6aa9568a50ce5437cefdb5b0786a03ca454edb6d492c25181052aa88946",
        "31bfa72daae84a22473abd95c39666610c385fb2323c93b4c6efda8e0cb0f205",
    );

    fn to_hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn provision_ticket_matches_the_documented_vector() {
        let ticket = vector_ticket();
        let bytes = ticket.signing_bytes().unwrap();
        assert_eq!(to_hex(&bytes), VECTOR_BYTES);
        // SHA-256 of the 32 raw token bytes 0xab..ab.
        let token = "ab".repeat(32);
        let digest = digest::digest(&digest::SHA256, &decode_hex::<32>(&token).unwrap());
        assert_eq!(to_hex(digest.as_ref()), ticket.token_sha256);
        // RFC 8032 TEST 1 key signs it; Ed25519 is deterministic, so the signature is fixed.
        let key = Ed25519KeyPair::from_seed_unchecked(&decode_hex::<32>(SEED).unwrap()).unwrap();
        assert_eq!(to_hex(key.sign(&bytes).as_ref()), VECTOR_SIGNATURE);
        let verifier = GrantVerifier::new(decode_hex::<32>(PUBLIC).unwrap());
        let verified = verifier
            .verify_ticket(&ticket, VECTOR_SIGNATURE, 1_700_000_000)
            .unwrap();
        assert!(verified.token_matches(&token));
        assert!(!verified.token_matches(&"cd".repeat(32)));
        assert!(!verified.token_matches("not hex"));
    }

    #[test]
    fn provision_ticket_checks_signature_version_and_time_window() {
        let verifier = GrantVerifier::new(decode_hex::<32>(PUBLIC).unwrap());
        let ticket = vector_ticket();
        let now = 1_700_000_000;
        assert!(matches!(
            verifier.verify_ticket(&ticket, VECTOR_SIGNATURE, 1_700_000_120),
            Err(GrantError::Expired)
        ));
        assert!(matches!(
            verifier.verify_ticket(&ticket, VECTOR_SIGNATURE, 1_699_999_819),
            Err(GrantError::LifetimeTooLong)
        ));
        assert!(
            verifier
                .verify_ticket(&ticket, VECTOR_SIGNATURE, 1_700_000_000)
                .is_ok()
        );
        // session_exp: not past, and at most 12 hours ahead. It is signed, so test it by re-signing.
        let key = Ed25519KeyPair::from_seed_unchecked(&decode_hex::<32>(SEED).unwrap()).unwrap();
        let resign = |edit: &dyn Fn(&mut ProvisionTicket)| {
            let mut changed = ticket.clone();
            edit(&mut changed);
            let signature = to_hex(
                key.sign(&changed.signing_bytes().unwrap_or_default())
                    .as_ref(),
            );
            verifier.verify_ticket(&changed, &signature, now)
        };
        assert!(matches!(
            resign(&|t| t.session_exp = now),
            Err(GrantError::Expired)
        ));
        assert!(matches!(
            resign(&|t| t.session_exp = now + 43_201),
            Err(GrantError::LifetimeTooLong)
        ));
        assert!(resign(&|t| t.session_exp = now + 43_200).is_ok());
        assert!(matches!(
            resign(&|t| t.relay_id = String::new()),
            Err(GrantError::InvalidFormat)
        ));
        let mut altered = ticket.clone();
        altered.relay_id = "relay-2".into();
        assert!(matches!(
            verifier.verify_ticket(&altered, VECTOR_SIGNATURE, now),
            Err(GrantError::InvalidSignature)
        ));
        let mut altered = ticket.clone();
        altered.max_connections = 9;
        assert!(matches!(
            verifier.verify_ticket(&altered, VECTOR_SIGNATURE, now),
            Err(GrantError::InvalidSignature)
        ));
        assert!(matches!(
            GrantVerifier::new([1; 32]).verify_ticket(&ticket, VECTOR_SIGNATURE, now),
            Err(GrantError::InvalidSignature)
        ));
        assert!(matches!(
            verifier.verify_ticket(&ticket, &"00".repeat(64), now),
            Err(GrantError::InvalidSignature)
        ));
        assert!(matches!(
            verifier.verify_ticket(&ticket, "zz", now),
            Err(GrantError::InvalidFormat)
        ));
        altered = ticket.clone();
        altered.v = 2;
        assert!(matches!(
            verifier.verify_ticket(&altered, VECTOR_SIGNATURE, now),
            Err(GrantError::UnsupportedVersion)
        ));
        altered = ticket;
        altered.host_principal = String::new();
        assert!(matches!(
            verifier.verify_ticket(&altered, VECTOR_SIGNATURE, now),
            Err(GrantError::InvalidFormat)
        ));
    }

    #[test]
    fn provision_messages_are_strict_json() {
        let message = ToRelay::Provision {
            ticket: vector_ticket(),
            signature: VECTOR_SIGNATURE.into(),
            host_token: "ab".repeat(32),
        };
        let text = serde_json::to_string(&message).unwrap();
        assert!(
            text.starts_with(r#"{"type":"provision","ticket":{"v":1,"session_id":"session-1""#),
            "{text}"
        );
        assert!(matches!(
            serde_json::from_str::<ToRelay>(&text).unwrap(),
            ToRelay::Provision { .. }
        ));
        assert!(serde_json::from_str::<ToRelay>(&text.replacen('{', "{\"extra\":1,", 1)).is_err());
        assert!(
            serde_json::from_str::<ToRelay>(&text.replacen("\"v\":1,", "\"v\":1,\"extra\":1,", 1))
                .is_err()
        );
        assert_eq!(
            serde_json::to_string(&FromRelay::Provisioned {}).unwrap(),
            r#"{"type":"provisioned"}"#
        );
        assert!(serde_json::from_str::<FromRelay>(r#"{"type":"provisioned","x":1}"#).is_err());
    }

    #[test]
    fn grant_and_ticket_signatures_are_not_interchangeable() {
        let verifier = GrantVerifier::new(decode_hex::<32>(PUBLIC).unwrap());
        let grant = signed_grant();
        // A grant's signature does not pass as a ticket's ...
        assert!(matches!(
            verifier.verify_ticket(&vector_ticket(), &grant.signature, 1_700_000_000),
            Err(GrantError::InvalidSignature)
        ));
        // ... and a ticket's signature does not pass as a grant's.
        let forged = SignedGrant {
            grant: grant.grant.clone(),
            signature: VECTOR_SIGNATURE.into(),
        };
        assert!(matches!(
            verifier.verify_at(&forged, "session-1", "member:host", 1_700_000_000),
            Err(GrantError::InvalidSignature)
        ));
        // The domains differ, so no encoding of one can equal an encoding of the other.
        assert_ne!(
            grant.grant.signing_bytes().unwrap()[..21],
            vector_ticket().signing_bytes().unwrap()[..21]
        );
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
            0, 0, 0, 1, 0, 1, b's', 0, 1, b'h', 0, 1, b'g', 0, 1, 0, 0, 0, 0, 0, 0, 0, 120,
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
        let verified = verifier
            .authorize_at(&signed, "session-1", "member:host", 1_700_000_000)
            .unwrap();
        assert_eq!(verified.guest_principal(), "guest:alice");
        assert_eq!(verified.mc_profile(), Some("profile-1"));
        assert!(matches!(
            verifier.authorize_at(&signed, "session-1", "member:host", 1_700_000_000),
            Err(GrantError::Replay)
        ));
        let verifier = GrantVerifier::new(key);
        assert!(matches!(
            verifier.verify_at(&signed, "session-1", "member:host", 1_700_000_120),
            Err(GrantError::Expired)
        ));
        assert!(matches!(
            verifier.verify_at(&signed, "session-1", "member:other", 1_700_000_000),
            Err(GrantError::WrongHost)
        ));
        assert!(matches!(
            verifier.verify_at(&signed, "session-2", "member:host", 1_700_000_000),
            Err(GrantError::WrongSession)
        ));
        assert!(matches!(
            verifier.verify_at(&signed, "session-1", "member:host", 1_699_999_819),
            Err(GrantError::LifetimeTooLong)
        ));
        assert!(matches!(
            GrantVerifier::new([1; 32]).verify_at(
                &signed,
                "session-1",
                "member:host",
                1_700_000_000
            ),
            Err(GrantError::InvalidSignature)
        ));
        let mut altered = signed.clone();
        altered.grant.guest_principal = "guest:mallory".into();
        assert!(matches!(
            verifier.verify_at(&altered, "session-1", "member:host", 1_700_000_000),
            Err(GrantError::InvalidSignature)
        ));
        assert!(matches!(
            HostGrantVerifier::new(key, 0).authorize_at(
                &signed,
                "session-1",
                "member:host",
                1_700_000_000
            ),
            Err(GrantError::ReplayCacheFull)
        ));
    }

    #[test]
    fn client_clock_skew_does_not_relax_relay_verification() {
        let now = 1_700_000_000;
        let verifier = GrantVerifier::new(decode_hex::<32>(PUBLIC).unwrap());
        let key = Ed25519KeyPair::from_seed_unchecked(&decode_hex::<32>(SEED).unwrap()).unwrap();
        let mut ticket = vector_ticket();
        ticket.exp = now + MAX_PROVISION_LIFETIME_SECS + CLIENT_CLOCK_SKEW_SECS;
        ticket.session_exp = now + MAX_SESSION_LIFETIME_SECS + CLIENT_CLOCK_SKEW_SECS;
        let signature = to_hex(key.sign(&ticket.signing_bytes().unwrap()).as_ref());
        assert!(
            verifier
                .verify_client_ticket(&ticket, &signature, now)
                .is_ok()
        );
        assert!(matches!(
            verifier.verify_ticket(&ticket, &signature, now),
            Err(GrantError::LifetimeTooLong)
        ));
        assert!(matches!(
            verifier.verify_client_ticket(&ticket, &signature, now - 1),
            Err(GrantError::LifetimeTooLong)
        ));
        let signed = signed_grant();
        let early = signed.grant.exp - MAX_GRANT_LIFETIME_SECS - CLIENT_CLOCK_SKEW_SECS;
        assert!(
            verifier
                .verify_client_at(&signed, "session-1", "member:host", early)
                .is_ok()
        );
        assert!(matches!(
            verifier.verify_at(&signed, "session-1", "member:host", early),
            Err(GrantError::LifetimeTooLong)
        ));
        assert!(matches!(
            verifier.verify_client_at(&signed, "session-1", "member:host", early - 1),
            Err(GrantError::LifetimeTooLong)
        ));
        assert!(matches!(
            verifier.verify_client_at(&signed, "session-1", "member:host", signed.grant.exp),
            Err(GrantError::Expired)
        ));
    }

    #[test]
    fn failed_host_pairs_release_the_nonce_but_bridged_pairs_burn_it() {
        let verifier = HostGrantVerifier::new(decode_hex::<32>(PUBLIC).unwrap(), 1);
        let signed = signed_grant();
        let now = 1_700_000_000;
        let pending = verifier
            .begin_client_at(&signed, "session-1", "member:host", now)
            .unwrap();
        assert!(matches!(
            verifier.begin_client_at(&signed, "session-1", "member:host", now),
            Err(GrantError::Replay)
        ));
        let mut another = signed.clone();
        another.grant.nonce = "12".repeat(32);
        let key = Ed25519KeyPair::from_seed_unchecked(&decode_hex::<32>(SEED).unwrap()).unwrap();
        another.signature = to_hex(key.sign(&another.grant.signing_bytes().unwrap()).as_ref());
        assert!(matches!(
            verifier.begin_client_at(&another, "session-1", "member:host", now),
            Err(GrantError::ReplayCacheFull)
        ));
        drop(pending);
        verifier
            .begin_client_at(&signed, "session-1", "member:host", now)
            .unwrap()
            .bridged()
            .unwrap();
        assert!(matches!(
            verifier.begin_client_at(&signed, "session-1", "member:host", now),
            Err(GrantError::Replay)
        ));
    }

    #[test]
    fn hex_decoding_is_lowercase_ascii_only_and_cannot_panic() {
        assert!(decode_hex::<2>("00ff").is_some());
        assert!(decode_hex::<2>("00FF").is_none());
        assert!(decode_hex::<2>("+f00").is_none());
        assert!(decode_hex::<2>("0 f0").is_none());
        assert!(decode_hex::<2>("00f").is_none());
        // 4 bytes, but two multibyte characters: slicing by byte index would panic.
        assert!(decode_hex::<2>("\u{e9}\u{e9}").is_none());
        assert!(decode_hex::<32>(&"\u{e9}".repeat(32)).is_none());
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
            verifier.bind_connection_at(
                std::io::empty(),
                &signed,
                "session-1",
                "member:host",
                1_700_000_000
            ),
            Err(GrantError::Replay)
        ));
    }

    #[test]
    fn social_messages_do_not_expose_or_accept_peer_addresses() {
        let signed = signed_grant();
        let requests = [
            ToRelay::Guest {
                grant: signed.clone(),
            },
            ToRelay::Host {
                grant: signed,
                token: "ab".repeat(32),
            },
        ];
        for request in requests {
            let serialized = serde_json::to_string(&request).unwrap();
            assert!(!serialized.contains("candidate"));
            assert!(!serialized.contains("observed"));
            assert!(!serialized.contains("192.0.2.1"));
        }
        let serialized = serde_json::to_string(&ToRelay::Guest {
            grant: signed_grant(),
        })
        .unwrap();
        let injected = serialized.replacen('{', "{\"candidate\":\"192.0.2.1:1234\",", 1);
        assert!(serde_json::from_str::<ToRelay>(&injected).is_err());
        assert!(
            serde_json::from_str::<FromRelay>(r#"{"type":"bridged","observed":"192.0.2.1:1234"}"#)
                .is_err()
        );
        assert_eq!(
            serde_json::to_string(&FromRelay::Bridged {}).unwrap(),
            r#"{"type":"bridged"}"#
        );
    }

    struct FakePortal {
        available: bool,
    }

    impl SessionPortal for FakePortal {
        type Error = &'static str;
        fn register(&self, request: &RegisterSession) -> Result<RegisteredSession, Self::Error> {
            if !self.available {
                return Err("offline");
            }
            Ok(RegisteredSession {
                session_id: request.session_id.clone(),
                host_principal: request.host_principal.clone(),
                relay_only: true,
                host_token: "ab".repeat(32),
            })
        }
        fn unregister(&self, _: &UnregisterSession) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn registration_fails_closed_when_portal_is_unavailable() {
        let request = RegisterSession::relay_only("session-1".into(), "member:host".into());
        assert!(register_session(&FakePortal { available: true }, &request).is_ok());
        assert!(matches!(
            register_session(&FakePortal { available: false }, &request),
            Err(RegistrationError::Portal("offline"))
        ));
    }
}
