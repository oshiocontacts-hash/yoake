# Yoake social transport: draft protocol

This document specifies the code-only social transport primitives, not a deployed
protocol. The running relay, launcher, and game do not yet use them. Social joins
must remain unavailable until the portal, authenticated relay transport, host
connection handling, and end-to-end encryption are integrated; the existing
invite-code and LAN paths remain separate.

## Portal grant

The portal issues a distinct grant for one guest and one host session. A grant
is a JSON object with exactly these fields:

| Field | Type and meaning |
| --- | --- |
| `v` | Protocol version, currently the integer `1`. |
| `session_id` | Opaque portal session identifier. |
| `host_principal` | Opaque portal principal hosting the session. |
| `guest_principal` | Opaque portal principal authorized to join. |
| `mc_profile` | Optional Minecraft profile identifier; a string, `null`, or absent. Its semantics are not yet specified. |
| `relay_only` | Must be `true`. |
| `exp` | Unsigned Unix expiration time in seconds; invalid at or after this time. |
| `nonce` | 32 cryptographically random bytes, encoded as 64 lowercase hexadecimal characters. |

All identifiers are nonempty UTF-8 strings of at most 128 bytes and cannot
contain Unicode control characters. They must be opaque and must not encode an
IP address, port, hostname, or other direct connection information. Unknown
fields and versions are rejected. The verifier requires `exp` to be no more
than 300 seconds in the future, but this alone does not establish the time of
issuance. Issuers and verifiers need trustworthy clocks.

The signed envelope has exactly `grant` and `signature` fields. The signature
is the 64-byte Ed25519 signature encoded as 128 lowercase hexadecimal
characters. It covers this binary encoding, not serialized JSON:

1. ASCII `yoake-social-join-v1` followed by one zero byte.
2. `v` as an unsigned 32-bit big-endian integer.
3. `session_id`, `host_principal`, and `guest_principal`, in that order: a
   two-byte unsigned big-endian UTF-8 byte length followed by the UTF-8 bytes.
4. `mc_profile`: `00` when absent or null, otherwise `01` followed by the same
   length-prefixed UTF-8 encoding.
5. `relay_only`: one byte, `01`.
6. `exp` as an unsigned 64-bit big-endian integer.
7. The 32 decoded nonce bytes.

The host pins the portal public key independently of the grant and checks the
signature, session, host, expiration, and nonce before accepting a guest. Its
bounded nonce cache rejects repeated grants and fails closed when full. The
relay independently checks the signature, session, host, expiration, and nonce.
Both replay caches are in memory: a production integration must invalidate
old sessions and grants after restart, or provide durable replay protection.

### Test vectors

For a grant with version `1`, session `s`, host `h`, guest `g`, no Minecraft
profile, `relay_only` true, expiration `120`, and 32 zero nonce bytes, the
signed message is the following hexadecimal byte string, with lines concatenated:

```text
796f616b652d736f6369616c2d6a6f696e2d763100
00000001
000173000168000167
00
01
0000000000000078
0000000000000000000000000000000000000000000000000000000000000000
```

The Ed25519 implementation is also checked against the standard RFC 8032
empty-message vector. Its public key is
`d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a`
and its signature is the concatenation of:

```text
e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155
5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b
```

This second vector is for an empty message, not for the example grant. A
portal-generated signed grant vector is still needed for cross-implementation
interoperability testing.

## Session registration

The host registration request contains `session_id`, `host_principal`, and
`relay_only: true`. A successful response contains those same fields and
`host_token`; unregistering identifies the session. The session
token is 32 random bytes encoded as lowercase hexadecimal, and must be handled
as a secret. Registration is represented by a client trait; the repository
does not provide a production HTTP client or portal authentication.

Registration with the portal and provisioning the relay must be authenticated,
confidential, tied to the same host and session, and fail closed. Portal-to-relay
provisioning, token delivery, key rotation, and authorization are not specified
by this code and must be agreed before deployment.

## Relay messages and pairing

Social relay control messages are a separate, strictly decoded protocol, not
an extension of the legacy invite-code wire format. A guest presents a signed
grant; the host presents that same signed grant and its session token. Responses
are `waiting`, `bridged`, or `rejected`. These message types have no fields for
direct addresses, observed addresses, or ICE candidates, and unknown fields are
rejected. They must only be used over an authenticated, confidential channel;
the message types alone do not provide TLS or prevent identifying information
from being placed inside an otherwise valid opaque identifier.

The relay state machine verifies grants and the host token before producing a
paired forwarding handle. The handle owns the guest and host streams supplied
during authentication and forwards only between those streams. Per-session
configuration bounds connections, remembered nonces, forwarding bandwidth, and
inactivity. Forwarding holds the shared session lock during I/O and requires
nonblocking streams. Idle expiration is enforced when the state is serviced or a
new request arrives; an integration must drive it and close expired sockets.
Retired session identifiers cannot be reused during the relay process lifetime;
they count toward the configured session limit even after retirement.

A grant is a bearer authorization, not proof of possession of the guest's
Minecraft account. The host verifier provides a stream wrapper retaining the
verified grant, but gameplay does not use it yet. The future integration must
bind that principal to the actual host-side player, prove the Minecraft identity as
required by the portal contract, and establish end-to-end encryption using a
separate per-guest secret. Neither the relay token nor the signed grant is that
secret. Portal unavailability, missing credentials, failed verification, and
unavailable social transport must never fall back to an unauthenticated join.
