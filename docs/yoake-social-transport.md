# Yoake social transport: draft protocol

This document specifies the social transport primitives. The relay has an opt-in
social listener, but the launcher and game do not use it yet. Social joins must
remain unavailable until the portal, host connection handling, and end-to-end
encryption are integrated; the existing invite-code and LAN paths remain separate.
For certificate generation, deployment, rotation, and monitoring, see
[Yoake social relay operations](yoake-social-ops.md).

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

The portal never connects to the relay. A host opens its own session on the relay
with a portal-signed **provision ticket**, so no portal-to-relay channel, relay
credential or relay-side portal client exists.

1. The host asks the portal to host a session (`POST /api/social/v1/sessions`).
   The portal answers with `session_id`, a `host_token` and a signed provision
   ticket. The `host_token` is 32 random bytes as 64 lowercase hex characters, is
   returned once, and the portal keeps only its SHA-256.
2. The host connects to the relay (TLS control channel) and sends `provision`
   with the ticket, its signature and the `host_token`. The relay replies
   `provisioned` (or `rejected`) and closes that connection.
3. A guest asks the portal to join (`POST /api/social/v1/sessions/:id/join`). The
   portal checks friendship, group membership and blocks, and returns a grant
   (240 s, as the portal issues it; the relay accepts up to 300 s) with the relay's address.
4. The guest connects to the relay with the grant; the host pairs it over a
   `host` connection with the grant and `host_token`, exactly as in
   [Relay messages and pairing](#relay-messages-and-pairing).
5. The session ends at its `session_exp`, or earlier when the host sends `retire`
   (below). The portal's own `DELETE` and 12-hour limit never reach the relay, so
   the relay enforces `session_exp` itself.

### Provision message

JSON like the other social messages: internally tagged by `type`, and unknown
fields are refused at every level.

```json
{
  "type": "provision",
  "ticket": {
    "v": 1,
    "session_id": "session-1",
    "host_principal": "member:host",
    "token_sha256": "<64 lowercase hex>",
    "bytes_per_second": 1000000,
    "burst_bytes": 65536,
    "max_connections": 8,
    "idle_secs": 60,
    "session_exp": 1700043200,
    "relay_id": "relay-1",
    "exp": 1700000120,
    "nonce": "<64 lowercase hex>"
  },
  "signature": "<128 lowercase hex>",
  "host_token": "<64 lowercase hex>"
}
```

The reply is `{"type":"provisioned"}` or `{"type":"rejected"}`; like every other
rejection, `rejected` carries no reason. Representation follows the grant: byte
strings (`token_sha256`, `nonce`, `signature`, `host_token`) are lowercase hex
only, the signed values are plain JSON fields (the ticket is a struct, not an
opaque blob), and the signature is over the binary encoding below, never over the
JSON text.

`exp` is the ticket's own lifetime (when it may still be presented);
`session_exp` is when the session it opens ends.

### Signed bytes

All integers are big-endian. The Ed25519 signature covers, in this order:

1. ASCII `yoake-social-provision-v1` followed by one zero byte.
2. `v`, `u32`, currently `1`.
3. `session_id`: `u16` length, then the UTF-8 bytes.
4. `host_principal`: `u16` length, then the UTF-8 bytes.
5. `token_sha256`: the raw 32 bytes.
6. `bytes_per_second`, `u64`.
7. `burst_bytes`, `u64` (at least 16384).
8. `max_connections`, `u32`.
9. `idle_secs`, `u32`.
10. `session_exp`, `u64`, Unix seconds.
11. `relay_id`: `u16` length, then the UTF-8 bytes.
12. `exp`, `u64`, Unix seconds.
13. `nonce`: the raw 32 bytes.

The key is the same pinned portal key as for grants (`HORIZON_RELAY_PORTAL_KEY`).
The domain strings differ (`yoake-social-join-v1` for grants), so a signature for
one kind of message never verifies as the other. `token_sha256` is SHA-256 of the
**32 raw bytes** the `host_token` hex decodes to (not of the hex text).

### What the relay checks, in order

Any failure is `rejected`, and nothing is stored:

1. Signature and format: `v` is `1`, hex fields are well formed, `session_id`,
   `host_principal` and `relay_id` follow the identifier rules (nonempty, at most
   128 UTF-8 bytes, no control characters), and the Ed25519 signature verifies.
2. Time: `exp` is in the future and at most 300 s ahead; `session_exp` is in the
   future and at most 12 hours ahead. There is no clock-skew allowance, so portal
   and relay clocks must agree.
3. `relay_id` equals this relay's `HORIZON_RELAY_ID`. A ticket for another relay
   (staging, say) that happens to share the portal key is refused.
4. `host_token` is 64 lowercase hex characters and SHA-256 of its bytes equals
   `token_sha256`.
5. The limits are valid: `burst_bytes` at least 16384 (one maximum message), and
   every value nonzero and no larger than the relay's caps (`bytes_per_second` at
   most 2^30, `burst_bytes` at most 64 MiB, `max_connections` at most 256,
   `idle_secs` at most 3600).
6. The session id: if it is already open with the same `host_principal` and the
   same `token_sha256` (and was itself opened by ticket), the reply is
   `provisioned` and the session is extended (below). Any other existing id,
   including a retired one still reserved and one opened by function call, is
   refused.
7. Per-host cap: one `host_principal` may have at most 3 sessions open at once;
   a fourth is refused.
8. The relay's session limit is not reached, the nonce was not used to open a
   session before, and the nonce cache is not full.

### Expiry, extension and early retire

- **Expiry.** A session whose `session_exp` has passed is ended like a retired one:
  its place is freed, waiting and bridged guests are dropped, and its id becomes a
  tombstone. The relay checks with the current Unix time (passed in, never read
  inside the check) whenever a session is provisioned, a guest arrives, or
  `retire` is called, and once a second inside every running bridge. A session
  nobody touches therefore lingers in memory until the next such event, but never
  holds capacity against a new session, because the check runs first.
- **Extension.** Presenting a new ticket for the same open session (same principal
  and token digest) moves `session_exp` to the later of the two values. It never
  shortens it, and it changes no limit. This is how a host keeps a long session
  going: the portal issues a fresh ticket and the host sends it again.
- **Retire.** `{"type":"retire","session_id":"...","host_token":"..."}` ends a
  session opened by ticket before its `session_exp`. The token authenticates it.
  The reply is `{"type":"retired"}`, or `{"type":"rejected"}` for a wrong token,
  an unknown or already ended session, or a session not opened by ticket. The
  connection is closed after the reply.
- **Tombstone.** An ended session's id stays reserved for 300 s (the longest grant
  lifetime), so no grant issued before the end can be valid for a new session
  with the same id. After that the id is free and a new ticket can open it again;
  this is safe, since every grant for the old session has expired by then.

### Idempotence and the nonce

A host may repeat a `provision` (for example when the reply was lost). Steps 1 to
5 above run first; then step 6 decides. For an open session with the same
principal and token digest the reply is `provisioned`, the nonce is neither stored
nor checked, and the only change is a possible extension of `session_exp`. So
resending the same ticket is accepted, and so is a new ticket for the same session
and token. For a new session the nonce must be unused, and it is stored (with the
ticket's `exp`) only when the session is actually created, so a refused ticket
does not burn its nonce.

This is the safe choice for a lost reply: a captured ticket on its own opens
nothing without the `host_token`, a repeat cannot change a session's limits or
principal, and a nonce can create at most one session. The nonce cache holds at
most 4096 entries, dropped once their `exp` has passed; because a ticket's `exp`
is at most 300 s ahead and a host holds at most 3 sessions, the cache is a second
line of defence rather than a limit anyone meets. A full cache refuses new
sessions (repeats still work). It is in memory: after a relay restart a ticket
still within its 300 s can be used again, which is why the lifetime is short and
the ticket is useless without the token.

Registration with the portal, token delivery and key rotation remain the portal's
concern. The relay also keeps a function-call `provision` used by tests and
embedding; such sessions never end by `session_exp`, cannot be adopted or
extended by tickets, and cannot be retired with `retire`.

### Provision test vector

Portal key: RFC 8032 TEST 1.

```text
secret key  9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60
public key  d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a
```

Input values:

```text
v                 1
session_id        "session-1"
host_principal    "member:host"
host_token        abababababababababababababababababababababababababababababababab
token_sha256      9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885
bytes_per_second  1000000
burst_bytes       65536
max_connections   8
idle_secs         60
session_exp       1700043200
relay_id          "relay-1"
exp               1700000120
nonce             1111111111111111111111111111111111111111111111111111111111111111
```

(Valid when the verifier's clock is 1700000000: `exp` is 120 s ahead and
`session_exp` exactly 12 hours ahead.)

Signed bytes (167 bytes; each line is one field in order, to be concatenated):

```text
796f616b652d736f6369616c2d70726f766973696f6e2d763100
00000001
000973657373696f6e2d31
000b6d656d6265723a686f7374
9a2db2e23f1504cd056606553ac049c5e718e8f9ce9233876df1a7a1821af885
00000000000f4240
0000000000010000
00000008
0000003c
00000000655499c0
000772656c61792d31
000000006553f178
1111111111111111111111111111111111111111111111111111111111111111
```

Signature (Ed25519 is deterministic), 128 hex characters:

```text
be08b6aa9568a50ce5437cefdb5b0786a03ca454edb6d492c25181052aa8894631bfa72daae84a22473abd95c39666610c385fb2323c93b4c6efda8e0cb0f205
```

## Relay messages and pairing

Social relay control messages are a separate, strictly decoded protocol, not
an extension of the legacy invite-code wire format. A host opens its session with
`provision` and may close it with `retire` (see above); a guest presents a signed
grant; the host presents that same signed grant and its session token. Responses
are `waiting`, `bridged`, `provisioned`, `retired`, or `rejected`. These message types have no fields for
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
logged with the reason. The binary can create a private CA and leaf and renew
the leaf offline; no production CA is embedded in clients yet.

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

Connection admission holds a slot from accept until the first control message
arrives, so it covers the TLS handshake and the wait for that message. Slots are
counted per source group (an IPv4 address, or an IPv6 /64, after mapping
IPv4-in-IPv6 back to IPv4): at most 4 per group; per coarser net (IPv4 /24, IPv6
/48): at most 32; and 256 in total. When the total is full the oldest connection
of the busiest net inside the busiest wide group (IPv4 /16, IPv6 /32) is dropped to admit the new one, so stalled
connections, even spread over many sources, cannot lock real clients out. The
listener also caps live connection threads (2048) and closes further sockets
unread. The relay refuses new connections while its certificate is outside its
validity period (checked on every accept, logged once per episode). A seven-day
expiry warning is logged at startup and on accept checks, at most once an hour.
The relay does not reload a renewed certificate, so it must be restarted after
one. These limits only mean
something when clients reach the relay directly: behind a proxy or load
balancer every client shares one address. The social listener must run without
a proxy, or the PROXY protocol would have to be supported first (it is not).

The relay state machine verifies grants and the host token before producing a
paired forwarding handle. It decrypts one outer TLS channel and re-encrypts into
the other while preserving per-session connection, nonce, bandwidth, and
inactivity limits. Its locks are held only to check limits and take budget, never
across network I/O. The forwarder reads first, without waiting, and charges
bandwidth for a whole message only once it holds one (a session's burst must be at least
one maximum message, 16 KiB, or its limits are invalid), so idle pairs hold no budget and messages are never
split. A message of zero length is a protocol violation and closes the bridge. The
forwarding loop naps up to 16 ms when both directions are idle, so an idle pair
costs little and the first message after a long silence can be delayed by that
much; a readiness-based wait (mio or poll/WSAPoll) would remove the delay but
would need a shared poller across channels and is not done. Pending guests are keyed by session and
grant nonce, and a second guest with the same grant is refused. The guest gets
`waiting` before it can be paired. A host with a wrong token, a different grant,
or an expired grant is rejected without disturbing the waiting guest, except that
three wrong tokens against one waiting guest lock that guest's slot against every host for 30 s (the guest itself keeps waiting); a host
that arrives before the guest is rejected (hosts do not queue). Nonces are
accepted only as 64 lowercase hexadecimal characters. Known limits: a guest that
disconnects while waiting is noticed only at the session's idle limit; a bridged
connection is not cut when the grant's `exp` passes (`exp` is checked only at
admission and pairing); and there is no clock-skew allowance, so the portal and
relay clocks must agree. The listener actively expires pending and idle
connections and closes both sockets. Closing is not a faithful signal: when one
hop drops (TCP reset or EOF without close_notify) the relay closes the other hop
with a proper close_notify, so the end-to-end layer must not treat a clean TLS
close as proof that the peer ended the session deliberately or that all its data
arrived. Before closing, the relay sends close_notify, half-closes, and waits
briefly for the peer to finish, so unread data does not turn its close into a
reset. A retired session's place is freed at once, but its identifier stays
reserved for 300 s (the longest grant lifetime) so a grant issued before the
retirement cannot be replayed against a re-provisioned session of the same id.
Live sessions plus these reservations are capped at four times the session limit.

A grant is a bearer authorization, not proof of possession of the guest's
Minecraft account. Gameplay does not use the verified principal yet. The future
integration must bind that principal to the host-side player, prove the Minecraft
identity, and establish end-to-end encryption using a separate per-guest secret.
Neither the relay token, signed grant, nor TLS channel binding alone is that
secret. Portal provisioning, unavailable credentials, failed verification, and
unavailable social transport must never fall back to an unauthenticated join.
