# Yoake social relay operations

The social transport is still a draft: the launcher and game do not use it yet.
Do not expose it as a working player-join service until the portal integration and
end-to-end encryption described in [yoake-social-transport.md](yoake-social-transport.md)
are complete. The existing invite-code listener is independent.

## Certificate layout

Use a DNS name that clients can reach directly, for example
`relay.example.org`. On an administrative machine with no production player data,
generate a private CA and a 30-day leaf:

```sh
horizon-relay --gen-social-certs /secure/yoake-social-ca --host relay.example.org
```

The host name is lower-cased; a trailing dot is refused. On Windows, create the
folder first and lock it down before generating anything (key files inherit the
folder's ACL):

```bat
mkdir C:\yoake-social-ca
icacls C:\yoake-social-ca /inheritance:r /grant:r "%USERNAME%:(OI)(CI)F"
```

Do not use a synced folder (Google Drive, OneDrive, Dropbox): the keys would be
copied to the cloud. On Unix the command creates a missing output directory with
mode `0700`; `--renew-social-leaf` refuses a directory writable by group or others.
If generation fails part-way it removes the files it created.

The command refuses to overwrite any output. It creates:

| File | Purpose | Where it belongs |
| --- | --- | --- |
| `ca.der` | CA certificate pinned by clients | Distribute to clients; keep a backup |
| `ca-key.der` | CA signing key, PKCS#8 DER | Offline backup/signing machine only |
| `leaf.der` | 30-day relay certificate | Relay |
| `leaf-key.der` | Relay private key, PKCS#8 DER | Relay only |

The CA is valid for five years, has `pathLen: 0`, `keyCertSign` and `cRLSign`,
and constrains DNS names to the selected hostname (excluding its subdomains).
The leaf has that DNS SAN, `serverAuth`, and `digitalSignature`. Its validity
starts one hour in the past, so players whose clock runs slightly behind still
accept it; it ends 30 days after issue.

**Never copy `ca-key.der` to the relay or include it in a client.** Keep it
offline, encrypted, and backed up. It is needed only to renew a leaf. On Unix the
command creates both key files with mode `0600` at open time; it does not create
them and chmod them later. Protect backups similarly. On Windows, restrict the
files' ACLs to the signing/relay service account as appropriate.

Send `leaf-key.der` to the relay only over a secure channel (scp/SSH, not chat or
e-mail), then delete it from the CA machine, including `leaf-key.der.old`.

**A leaked leaf key means impersonation for up to 30 days:** clients do not check
revocation lists. Either rotate the CA (the emergency procedure below) or accept
up to 30 days of exposure until the leaf expires. `leaf-key.der.old` is still a
valid secret key until its leaf expires; protect and delete it like the current key.

Copy only `leaf.der` and `leaf-key.der` to the relay. Give the service user read
access, keep the key a regular file with mode `0600`, and make the certificate
mode `0644`. The relay checks key permissions on startup and rejects a key that
is accessible by group or others on Unix.

## Environment

All four social settings are required to enable the listener:

```sh
HORIZON_RELAY_SOCIAL_ADDR=0.0.0.0:7443
HORIZON_RELAY_SOCIAL_CERT=/var/lib/horizon-relay/leaf.der
HORIZON_RELAY_SOCIAL_CERT_KEY=/var/lib/horizon-relay/leaf-key.der
HORIZON_RELAY_PORTAL_KEY=<64 lowercase hex characters>
HORIZON_RELAY_ID=<this relay's name, for example relay-eu-1>
```

The certificate and key variables are **file paths**, not inline DER. The portal
variable is the portal's public key in hex, not a file path; it verifies both guest
grants and host provision tickets (the portal never connects to the relay, so
there is no relay credential for it; see "Session registration" in
[yoake-social-transport.md](yoake-social-transport.md)).
`HORIZON_RELAY_ID` is required whenever the social listener is configured: it is
this relay's name, with the same rules as every social identifier (1 to 128 bytes
of UTF-8, no control characters). A provision ticket carries the relay it is for
and the relay refuses any other name, so give every relay that shares the portal
key (staging, other regions) its own ID, and have the portal issue tickets for
the ID of the relay it hands to the host. Changing it invalidates tickets issued
for the old name.
The social address is a socket address. `PORT`, `RELAY_BIND`, and the other
`RELAY_*` variables continue to configure the separate invite relay; see
[play-with-friends.md](play-with-friends.md).

The relay refuses to start the social listener if settings are incomplete, the
leaf and key do not match, or the leaf is outside its validity period. It logs a
warning when the leaf has seven days or less remaining, at startup and during
accept checks (at most once an hour). Once expired, new handshakes are refused.

## Example social-enabled systemd unit

This example uses a different unit name and does not alter the existing
invite-only `horizon-relay.service` (stop that one first if both would use the
same `PORT`). Create a dedicated system user, install the binary and leaf pair,
then use `/etc/systemd/system/horizon-relay-social.service`:

```ini
[Unit]
Description=Horizon invite and social relay
After=network-online.target
Wants=network-online.target

[Service]
User=horizon-relay
Group=horizon-relay
StateDirectory=horizon-relay
StateDirectoryMode=0700
ExecStart=/usr/local/bin/horizon-relay
Environment=PORT=7000
Environment=HORIZON_RELAY_SOCIAL_ADDR=0.0.0.0:7443
Environment=HORIZON_RELAY_SOCIAL_CERT=/var/lib/horizon-relay/leaf.der
Environment=HORIZON_RELAY_SOCIAL_CERT_KEY=/var/lib/horizon-relay/leaf-key.der
Environment=HORIZON_RELAY_PORTAL_KEY=replace-with-the-64-character-public-key
Environment=HORIZON_RELAY_ID=replace-with-this-relays-name
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
Restart=on-failure
RestartSec=5

[Install]
WantedBy=multi-user.target
```

For example, create the user with `useradd --system --home
/var/lib/horizon-relay --shell /usr/sbin/nologin horizon-relay`. Install files
with `install -o horizon-relay -g horizon-relay -m 0644 leaf.der
/var/lib/horizon-relay/leaf.der` and the corresponding `install -m 0600` command
for `leaf-key.der`. Run `systemctl daemon-reload`, then `systemctl enable --now
horizon-relay-social`. Open both required TCP ports in the firewall as applicable.

## Routine leaf rotation

Start before the seven-day warning. Clients pin the CA, so an ordinary leaf
renewal requires no client change.

1. On the offline CA machine, run:

   ```sh
   horizon-relay --renew-social-leaf /secure/yoake-social-ca --host relay.example.org
   ```

2. The command reads `ca.der` and `ca-key.der`, checks the CA, issues a new
   30-day pair, and verifies it against `ca.der` and `--host` the way a client
   would. A wrong `--host` (outside the CA's name constraint) fails and changes
   nothing. The new files are written as `.new`, the old ones are kept as
   `leaf.der.old` and `leaf-key.der.old` (hard link or copy), and each current file
   is then replaced by a single rename, so `leaf.der` is never missing. Leftover
   `.new` files are cleaned up. A later renewal replaces the `.old` backups.
   It prints a warning when the CA has less than 90 days left.
3. After the command finishes, copy only the new `leaf.der` and `leaf-key.der`
   to the relay, retaining the ownership and permissions above.
4. Run `systemctl restart horizon-relay-social`. There is no certificate reload or
   reload command: a running relay keeps its old in-memory TLS configuration.
5. Check the journal for startup errors and expiry warnings, and make a real
   pinned-CA TLS connection. Keep the previous offline pair until this succeeds.

## CA rotation and compromise

The CA lives five years, and nothing warns clients when it ends: webpki does not
check the trust anchor's expiry, so only `--renew-social-leaf` starts failing.
Start the rotation months before the CA expires (the renew command warns at
90 days, which is too late for a client update to reach everyone). The relay's
host name is part of the CA's name constraint: changing the host name needs a new
CA, which is a client update.

For planned CA rotation, generate a new CA in a separate directory. Distribute
its `ca.der` alongside the old CA: clients accept several pinned CAs. Wait for
all clients to receive both, issue a leaf from the new CA, install it, restart,
and verify. Remove the old CA from clients only after the overlap is complete.

If the CA key leaks, treat the CA as compromised. Stop the social listener,
generate a new CA and leaf, distribute the new CA, and remove the compromised CA
from every client as quickly as possible. Do not leave a compromised CA trusted
for a routine overlap. Replace any leaf keys that may also have been exposed,
restart, and review logs for unexpected TLS use. The TLS CA is separate from the
portal grant-signing key, but investigate other credentials separately.

## Network requirements and limits

The social listener must receive client TCP connections **directly**, without a
reverse proxy or load balancer. Its source limits use the socket peer address;
a proxy would put all clients behind one address. PROXY protocol support is not
implemented for this listener. TLS 1.3 is mandatory and there is no plaintext
fallback.

Default admission limits are 256 in-flight handshakes/first-message waits, 4 per
IPv4 address or IPv6 `/64`, and 32 per IPv4 `/24` or IPv6 `/48`. The TLS handshake
deadline is 4 seconds, the first control message deadline is 5 seconds, and a
whole write has a 10-second deadline. The listener caps live connection threads
at 2048 and closes additional sockets unread. These are fixed defaults, not
currently environment settings. Session bandwidth, nonce, connection, and idle
limits also apply after pairing.

Monitor service restarts and listener startup failures, certificate-expiry and
outside-validity messages, connection-limit errors, thread/file-descriptor and
memory use, TCP reachability on the social port, and successful pinned-CA
handshakes. Monitor system clock accuracy: certificate and grant validity have
no clock-skew allowance. The existing five-minute invite-relay count output does
not measure social handshake health, so journal and connection checks are needed.
