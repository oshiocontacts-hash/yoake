use crate::social::{SignedGrant, decode_hex};
use ring::signature::{ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProvisionTicket {
    pub v: u32,
    pub session_id: String,
    pub host_principal: String,
    pub token_sha256: String,
    pub bytes_per_second: u64,
    pub burst_bytes: u64,
    pub max_connections: u32,
    pub idle_secs: u32,
    pub session_exp: u64,
    pub relay_id: String,
    pub exp: u64,
    pub nonce: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedProvision {
    pub ticket: ProvisionTicket,
    pub signature: String,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Provision {
        ticket: ProvisionTicket,
        signature: String,
        host_token: String,
    },
    Retire {
        session_id: String,
        host_token: String,
    },
    Guest {
        grant: SignedGrant,
    },
    Host {
        grant: SignedGrant,
        token: String,
    },
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Reply {
    Waiting {},
    Bridged {},
    Rejected {},
    Provisioned {},
    Retired {},
}

pub(crate) fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "social control request rejected",
    )
}

fn identifier(value: &str) -> io::Result<()> {
    if value.is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}

fn append_text(output: &mut Vec<u8>, value: &str) -> io::Result<()> {
    identifier(value)?;
    output.extend_from_slice(&(value.len() as u16).to_be_bytes());
    output.extend_from_slice(value.as_bytes());
    Ok(())
}

impl ProvisionTicket {
    pub fn signing_bytes(&self) -> io::Result<Vec<u8>> {
        if self.v != 1 {
            return Err(invalid());
        }
        let mut output = b"yoake-social-provision-v1\0".to_vec();
        output.extend_from_slice(&self.v.to_be_bytes());
        append_text(&mut output, &self.session_id)?;
        append_text(&mut output, &self.host_principal)?;
        output.extend_from_slice(&decode_hex::<32>(&self.token_sha256).ok_or_else(invalid)?);
        output.extend_from_slice(&self.bytes_per_second.to_be_bytes());
        output.extend_from_slice(&self.burst_bytes.to_be_bytes());
        output.extend_from_slice(&self.max_connections.to_be_bytes());
        output.extend_from_slice(&self.idle_secs.to_be_bytes());
        output.extend_from_slice(&self.session_exp.to_be_bytes());
        append_text(&mut output, &self.relay_id)?;
        output.extend_from_slice(&self.exp.to_be_bytes());
        output.extend_from_slice(&decode_hex::<32>(&self.nonce).ok_or_else(invalid)?);
        Ok(output)
    }
}

impl SignedProvision {
    pub fn verify(&self, public_key: &[u8; 32], token: &str, now: u64) -> io::Result<()> {
        let signature = decode_hex::<64>(&self.signature).ok_or_else(invalid)?;
        UnparsedPublicKey::new(&ED25519, public_key)
            .verify(&self.ticket.signing_bytes()?, &signature)
            .map_err(|_| invalid())?;
        let ticket = &self.ticket;
        if ticket.exp <= now
            || ticket.exp > now.saturating_add(300)
            || ticket.session_exp <= now
            || ticket.session_exp > now.saturating_add(43_200)
            || ticket.bytes_per_second > 1 << 30
            || !(16_384..=67_108_864).contains(&ticket.burst_bytes)
            || !(1..=256).contains(&ticket.max_connections)
            || !(1..=3600).contains(&ticket.idle_secs)
        {
            return Err(invalid());
        }
        let token = decode_hex::<32>(token).ok_or_else(invalid)?;
        let expected = decode_hex::<32>(&ticket.token_sha256).ok_or_else(invalid)?;
        let actual: [u8; 32] = Sha256::digest(token).into();
        if actual != expected {
            return Err(invalid());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::Ed25519KeyPair;

    const NOW: u64 = 1_700_000_000;
    const TOKEN: &str = "abababababababababababababababababababababababababababababababab";
    const PUBLIC_KEY: &str = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn ticket() -> ProvisionTicket {
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

    fn signed(ticket: ProvisionTicket) -> SignedProvision {
        let seed =
            decode_hex::<32>("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
                .unwrap();
        let key = Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
        let signature = key
            .sign(&ticket.signing_bytes().unwrap())
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        SignedProvision { ticket, signature }
    }

    #[test]
    fn social_provision_matches_independent_canonical_vector() {
        let expected = concat!(
            "796f616b652d736f6369616c2d70726f766973696f6e2d763100",
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
            "1111111111111111111111111111111111111111111111111111111111111111"
        );
        assert_eq!(
            ticket().signing_bytes().unwrap(),
            decode_hex::<167>(expected).unwrap()
        );
        let provision = SignedProvision {
            ticket: ticket(),
            signature: "be08b6aa9568a50ce5437cefdb5b0786a03ca454edb6d492c25181052aa8894631bfa72daae84a22473abd95c39666610c385fb2323c93b4c6efda8e0cb0f205".into(),
        };
        let public_key = decode_hex::<32>(PUBLIC_KEY).unwrap();
        assert!(provision.verify(&public_key, TOKEN, NOW).is_ok());
        assert!(
            provision
                .verify(&public_key, &"cd".repeat(32), NOW)
                .is_err()
        );
        let mut altered = provision;
        altered.ticket.host_principal = "another-host".into();
        assert!(altered.verify(&public_key, TOKEN, NOW).is_err());
    }

    #[test]
    fn social_provision_enforces_signed_limits() {
        let public_key = decode_hex::<32>(PUBLIC_KEY).unwrap();
        let invalid_changes: &[fn(&mut ProvisionTicket)] = &[
            |ticket| ticket.exp = NOW,
            |ticket| ticket.exp = NOW + 301,
            |ticket| ticket.session_exp = NOW,
            |ticket| ticket.session_exp = NOW + 43_201,
            |ticket| ticket.bytes_per_second = (1 << 30) + 1,
            |ticket| ticket.burst_bytes = 16_383,
            |ticket| ticket.burst_bytes = 67_108_865,
            |ticket| ticket.max_connections = 0,
            |ticket| ticket.max_connections = 257,
            |ticket| ticket.idle_secs = 0,
            |ticket| ticket.idle_secs = 3601,
            |ticket| ticket.token_sha256 = "ff".repeat(32),
        ];
        for change in invalid_changes {
            let mut ticket = ticket();
            change(&mut ticket);
            let error = signed(ticket).verify(&public_key, TOKEN, NOW).unwrap_err();
            assert_eq!(error.to_string(), "social control request rejected");
        }
        for (rate, burst, connections, idle) in
            [(0, 16_384, 1, 1), (1 << 30, 67_108_864, 256, 3600)]
        {
            let mut ticket = ticket();
            ticket.bytes_per_second = rate;
            ticket.burst_bytes = burst;
            ticket.max_connections = connections;
            ticket.idle_secs = idle;
            ticket.exp = NOW + 300;
            assert!(signed(ticket).verify(&public_key, TOKEN, NOW).is_ok());
        }
    }

    #[test]
    fn social_control_uses_strict_spec_wire_shapes() {
        let request = Request::Provision {
            ticket: ticket(),
            signature: "signed".into(),
            host_token: TOKEN.into(),
        };
        let json = serde_json::to_value(request).unwrap();
        assert_eq!(json["type"], "provision");
        assert_eq!(json["host_token"], TOKEN);
        assert!(json.get("grant").is_none());
        assert!(
            serde_json::from_str::<Request>(
                r#"{"type":"retire","session_id":"session-1","host_token":"secret","extra":true}"#
            )
            .is_err()
        );
        assert!(serde_json::from_str::<Reply>(r#"{"type":"bridged","extra":true}"#).is_err());
        for reply in ["waiting", "bridged", "rejected", "provisioned", "retired"] {
            assert!(serde_json::from_str::<Reply>(&format!(r#"{{"type":"{reply}"}}"#)).is_ok());
        }
        for change in [
            |ticket: &mut ProvisionTicket| ticket.v = 2,
            |ticket: &mut ProvisionTicket| ticket.session_id.clear(),
            |ticket: &mut ProvisionTicket| ticket.relay_id = "bad\nrelay".into(),
        ] {
            let mut ticket = ticket();
            change(&mut ticket);
            assert!(ticket.signing_bytes().is_err());
        }
    }
}
