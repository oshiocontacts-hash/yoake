# Yoake social transport: draft protocol

This document specifies the social transport primitives. The relay has an opt-in
social listener, but the launcher and game do not use it yet. Social joins must
remain unavailable until the portal, host connection handling, and end-to-end
encryption are integrated; the existing invite-code and LAN paths remain separate.

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
rejected.

The control channel is TLS 1.3 only, implemented by rustls with ring; the crypto
provider is named explicitly in every config, so another provider in the
dependency graph cannot change or break it. A client trusts only the
caller-supplied Yoake private CA certificates and verifies the relay's DNS name
(SNI); it does not use public WebPKI roots or a custom verifier. Several CA
certificates may be given at once so a CA can be rotated with an overlap. The
relay's leaf certificate (with a SAN for the relay name and the serverAuth
extended key usage; the CA should have pathLen 0) and private key are separate
from the portal's grant signing key. `HORIZON_RELAY_SOCIAL_CERT` and
`HORIZON_RELAY_SOCIAL_CERT_KEY` are paths to a DER leaf certificate and PKCS#8
key, never inline contents. The key file is opened once and checked on that
handle: it must be a regular file and, on Unix, not group/other accessible
(Windows ACLs are managed by the deployer). The owner is not checked, because
that needs the current uid and no crate for it is in the build. At startup the
relay refuses a certificate that does not match the key or is outside its
validity period, and says which. With none of the social variables set the relay
only logs that the listener is disabled; a partial or unusable setting is
logged with the reason. The production CA is not created or embedded yet.

Messages use a four-byte big-endian length and at most 16 KiB of payload, with
the length checked before allocation. Sockets are nonblocking and every wait is a
loop against an `Instant` deadline, with no per-call threads and no kernel socket
timeouts. The handshake has a deadline (4 s by default). The first control
message must arrive within 5 s of the handshake. A whole `send_message` has a
10 s deadline; missing it closes the channel, because bytes already handed to TLS
are never handed over again. Waiting for a message that does not come is not an
error, so an idle bridged connection stays open until the session's idle limit.
TCP EOF without TLS `close_notify` is `UnexpectedEof`; a clean close is
`ConnectionAborted`. There is no raw stream accessor or plaintext fallback.

The TLS exporter output for label `EXPORTER-yoake-social-binding-v1`, with no
context, is available as an input to the future end-to-end layer. It is
different on every hop: the guest-relay and host-relay channels are separate TLS
sessions, so their `channel_binding` values differ. The later end-to-end design
must not assume both sides share it; it needs a value that both ends derive
from their own secret.

Handshake admission counts in-progress handshakes per source group (an IPv4
address, or an IPv6 /64, after mapping IPv4-in-IPv6 back to IPv4): at most 4 per
group, 256 in total. When the total is full the oldest in-progress handshake is
dropped to admit the new one, so stalled connections cannot lock real clients
out. The slot is released as soon as the handshake ends. These limits only mean
something when clients reach the relay directly: behind a proxy or load
balancer every client shares one address. The social listener must run without a
proxy, or the PROXY protocol would have to be supported first (it is not).

The relay state machine verifies grants and the host token before producing a
paired forwarding handle. It decrypts one outer TLS channel and re-encrypts into
the other while preserving per-session connection, nonce, bandwidth, and
inactivity limits; its locks are held only to check limits and reserve or settle
budget, never across network I/O. Pending guests are keyed by session and
grant nonce, and a second guest with the same grant is refused. The guest gets
`waiting` before it can be paired. A host with a wrong token, a different grant,
or an expired grant is rejected without disturbing the waiting guest; a host
that arrives before the guest is rejected (hosts do not queue). Nonces are
accepted only as 64 lowercase hexadecimal characters. Known limits: a guest that
disconnects while waiting is noticed only at the session's idle limit; a bridged
connection is not cut when the grant's `exp` passes (`exp` is checked only at
admission and pairing); and there is no clock-skew allowance, so the portal and
relay clocks must agree. The listener actively expires pending and idle
connections and closes both sockets. Retired session identifiers cannot be
reused during the relay process lifetime and count toward the configured session
limit after retirement.

A grant is a bearer authorization, not proof of possession of the guest's
Minecraft account. Gameplay does not use the verified principal yet. The future
integration must bind that principal to the host-side player, prove the Minecraft
identity, and establish end-to-end encryption using a separate per-guest secret.
Neither the relay token, signed grant, nor TLS channel binding alone is that
secret. Portal provisioning, unavailable credentials, failed verification, and
unavailable social transport must never fall back to an unauthenticated join.
