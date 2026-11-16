# Yoake social transport and game handoff

This document specifies the social transport primitives and the opt-in game
client. Without an explicit social handoff the game does not contact the social
relay or open additional listeners. The existing invite-code and LAN paths remain
separate. The launcher/portal handoff producer and production relay provisioning
must be integrated before exposing this flow to players. TLS protects each hop;
it does not provide end-to-end encryption against the relay.
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
than 300 seconds in the future; the provisioning contract asks the portal to
issue grants expiring after 240 seconds. This alone does not establish the time
of issuance. Issuers and verifiers need trustworthy clocks.

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

## Session provisioning

The portal creates the session and returns its opaque `session_id`,
`host_principal`, secret `host_token`, and signed provision ticket. The token is
32 random bytes encoded as 64 lowercase hexadecimal characters. The game does
not implement portal HTTP endpoints or authenticate to the portal itself.

The ticket has exactly `v`, `session_id`, `host_principal`, `token_sha256`,
`bytes_per_second`, `burst_bytes`, `max_connections`, `idle_secs`, `session_exp`,
`relay_id`, `exp`, and `nonce`. `token_sha256` is SHA-256 of the decoded token
bytes, not its hexadecimal text. Signature and nonce use the same lowercase
hexadecimal formats as grants. The signed provision envelope contains `ticket`
and `signature`. Its Ed25519 message is:

1. ASCII `yoake-social-provision-v1` followed by one zero byte.
2. `v` as unsigned 32-bit big-endian.
3. `session_id` and `host_principal`, each length-prefixed as for grants.
4. The 32 decoded `token_sha256` bytes.
5. `bytes_per_second` and `burst_bytes`, each unsigned 64-bit big-endian.
6. `max_connections` and `idle_secs`, each unsigned 32-bit big-endian.
7. `session_exp` as unsigned 64-bit big-endian.
8. Length-prefixed `relay_id`.
9. `exp` as unsigned 64-bit big-endian, then the 32 decoded nonce bytes.

Ticket expiry must be after now and at most 300 seconds ahead; session expiry
must be after now and at most 12 hours ahead. Bandwidth is at most 1 GiB/s
(`0` means unlimited), burst is 16 KiB through 64 MiB, connections are 1 through
256, and idle timeout is 1 through 3600 seconds. The game checks the signature,
limits, token digest, session and principal before sending `provision` over TLS.
The relay must additionally enforce its own identity, authorization, ticket
replay, principal session cap and renewal rules.

The final provisioning contract closes the initial TLS connection after
`provisioned`. This is not a persistent host socket: each guest needs a fresh
`host` TLS connection presenting that guest's grant. The launcher must deliver
the grant only after the guest is waiting at the relay; the current pairing
implementation does not accept host-first pairing. The client does not renew
sessions automatically.

## Launcher to game handoff

Pass only file paths on the command line:

```text
--yoake-social-handoff <one-shot-file> --yoake-social-trust <trust-file>
```

For hosts, also pass `--yoake-social-control-stdin` and keep the game's stdin
open for lifecycle commands. Guests omit that flag. Microsoft authentication
continues to use the existing `--session-file` handoff. Do not combine
`--session-stdin` with `--yoake-social-control-stdin`; startup rejects this
combination before reading stdin. Tokens, tickets, grants and signatures must
never be command-line arguments or log messages.

Create the one-shot file in the game's private temporary directory with
`horizon_friends::social_handoff::write_new`, then pass its path. It exclusively
creates a new file with Unix mode `0600`, or on Windows an owner-only protected
DACL. The startup reader requires a private, single-link regular file owned by
the current user and rejects symlinks/reparse points and permissive access. It
reads at most 64 KiB and deletes the file before parsing; malformed and oversized
private files are also consumed. A failed permission check leaves the file
untouched for its owner to remove. Errors contain no credential data. The
launcher must remove an unused handoff if game startup never reads it.

The strict JSON host shape is:

```json
{
  "role": "host",
  "session_id": "<portal session>",
  "host_principal": "<portal principal>",
  "host_token": "<64 lowercase hex characters>",
  "provision": { "ticket": {}, "signature": "<128 lowercase hex characters>" },
  "relay": "<relay DNS name or IP>:<TLS port>"
}
```

Here `ticket` must be the complete ticket described above, not an empty object.
The guest shape has `role: "guest"`, `session_id`, `grant` (the complete grant
object), `signature`, and `relay`. All unknown fields are rejected. A guest
handoff takes priority over ordinary startup world/server quick-play options.

The separate trust JSON contains `portal_public_key` (32 bytes as lowercase
hexadecimal) and `ca_certificates` (one through eight DER CA certificates, each
encoded as lowercase hexadecimal). It is bounded to 1 MiB and is not consumed.
The launcher must authenticate and protect this configuration independently of
the portal response or guest grant. TLS verifies the relay hostname/IP against
these roots; no system-root fallback is used.

Host stdin accepts newline-delimited strict JSON commands:

```json
{"type":"pair","grant":{"grant":{},"signature":"<signed guest grant signature>"}}
{"type":"end"}
```

Replace the inner grant with the full signed grant. Each line is bounded to
64 KiB, and the pending command queue is bounded to 256. EOF, malformed or
oversized input, or queue overflow ends the session rather than continuing with
an ambiguous control state. The launcher owns coordination with the portal and
the waiting guest; there is no game-side portal polling.

Bridged hosts feed the normal LAN login handler without creating a listener.
An unpublished integrated server accepts this transport without opening LAN
access. Normal Minecraft online login, capacity, duplicate-player and local-seat
checks remain in place; social principals do not grant owner permissions.
The integrated server does not pause while the social host is active. Guests
run the normal client login over the bridge. Leaving/closing the world, host
stdin ending, explicit `end`, or session expiry cancels active and pending
sockets. Retirement is sent asynchronously and is best effort; a hard-killed
process still relies on relay expiry/idle policy.

The loopback TLS tests use real relay guest/host pairing and forwarding, but
adapt `provision`/`retire` in a test-only handler. Production support for these
two messages is being implemented separately. These tests are not a game or
launcher runtime test.

## Relay messages and pairing

Social relay control messages are a separate, strictly decoded protocol, not
an extension of the legacy invite-code wire format. `provision` contains the flat
`ticket`, `signature`, and `host_token` fields and returns `provisioned` or
`rejected`. `retire` contains `session_id` and `host_token` and returns `retired`
or `rejected`. A `guest` presents a signed `grant`; a `host` presents that same
signed `grant` and its session `token`. Pairing responses are `waiting`, `bridged`,
or `rejected`. These message types have no fields for
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
Minecraft account. Social streams retain the normal Minecraft login and online
authentication path, but gameplay does not bind the verified portal principal to
that Minecraft identity yet. A future integration must add that binding and
establish end-to-end encryption using a separate per-guest secret.
Neither the relay token, signed grant, nor TLS channel binding alone is that
secret. Portal provisioning, unavailable credentials, failed verification, and
unavailable social transport must never fall back to an unauthenticated join.
