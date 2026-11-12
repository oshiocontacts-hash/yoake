# Yoake `.yak` packages and catalog

Level: preview. Package format 1; catalog schema 1. This is the library/tooling stage of
the workspace's `docs/yak-format.md`, not game loading or launcher integration.
The implementation is `horizon_mods::yak` (feature `yak`). It is separate from
[Horizon's upstream registry](mod-index.md), whose JSON and minisign format do not change.

## Distribution and trust

The catalog lives at the portal's `/yoake/mods/index.json`; its detached signature is
`/yoake/mods/index.json.sig`. Each version's URL is exactly
`https://<portal>/yoake/mods/<id>/<version>/<id>-<version>.yak`.
Test-play invitees use this same path, not a second catalog or package format.
Portal origin selection and the proposed `/api/launcher/check` access gate belong to
the launcher/portal, not this offline library. The library validates the HTTPS path,
but does not pin an origin, make requests, follow redirects, or store credentials.

The initial rollout signs **the catalog only**. Individual packages may be unsigned:
a verified catalog authenticates each complete archive through its exact byte size
and SHA-256. Embedded package signatures are supported for later use and are
verified whenever present; their embedded public key is never a trust anchor.
Callers supply pinned Ed25519 public keys, including any explicitly approved rotation keys.

Recommended consumer sequence:

1. Read bounded catalog/signature bytes from the configured portal.
2. Call `Catalog::parse_verified(bytes, signature, pinned_keys)` on the original JSON
   bytes. `Catalog::parse` validates structure only and is **not authentication**.
3. Select a version and obtain its archive in an immutable, caller-controlled cache.
   Call `Archive::from_catalog(reader, version)` to check the complete archive's
   size/hash, structure, and full normalized manifest against that verified entry.
   `CatalogVersion::verify_download` is available for a streaming size/hash check.
4. Run compatibility and dependency checks for the selected set. If the archive has
   `SIGNATURE`, verify it with the caller's pinned keys as well.
5. Call `extract_target` into a new target-view directory; retain the original archive
   as the authenticated cache object.

`CatalogVersion` is an ordinary public data type: passing a hand-built value or one
from `Catalog::parse` to `from_catalog` does not establish catalog trust. Readers and
cached archives must not be concurrently modified between authentication and use.
No catalog freshness, rollback, or revocation policy is inferred from `generated`.

## `index.json`

The example size and digest below are illustrative, not a downloadable artifact.

```json
{
  "schema": 1,
  "generated": "2026-11-13T00:00:00Z",
  "entries": [{
    "id": "example_data",
    "versions": [{
      "version": "1.0.0",
      "url": "https://portal.example/yoake/mods/example_data/1.0.0/example_data-1.0.0.yak",
      "size": 1024,
      "sha256": "0000000000000000000000000000000000000000000000000000000000000000",
      "manifest": {
        "format": 1,
        "id": "example_data",
        "version": "1.0.0",
        "name": "Example data",
        "authors": ["Example author"],
        "description": "An example data-only package.",
        "license": "MIT",
        "yoake": { "version": ">=0.3.0, <0.4.0" },
        "side": { "client": true, "server": true, "required_on_join": false }
      },
      "targets": [],
      "engine": null
    }]
  }]
}
```

| Field | Contract |
| --- | --- |
| `schema` | Exactly `1`. Unknown fields are rejected throughout this schema. |
| `generated` | A valid UTC calendar timestamp, exactly `YYYY-MM-DDTHH:MM:SSZ`. |
| `entries` | At most 10,000 entries; unique IDs, each 3..48 lowercase ASCII letters, digits, `_` or `-`. Empty catalogs are allowed. |
| `versions` | 1..1,024 versions per ID; unique complete semver strings. Array order is preserved; readers do not assume newest-first. |
| `version` | Semver, identical to `manifest.version`. |
| `url` | HTTPS with a host, the exact version path above, no user information, query, or fragment. |
| `size` | Positive complete ZIP byte count, at most 1 GiB + 32 MiB. |
| `sha256` | Exactly 64 lowercase hexadecimal characters, covering all ZIP bytes including any `SIGNATURE`. |
| `manifest` | The full normalized `yak.toml` object in JSON form, not a lossy display summary. Its ID/version must match the entry. |
| `targets` | Unique supported target names, exactly the manifest's `bin.targets`; empty for data-only packages. |
| `engine` | Exactly `bin.engine` for a program package; null/absent for a data-only package. |

Catalog input is limited to 16 MiB and its signature text to 4 KiB. These bounds and
unknown-field rejection apply before the catalog can be used. Change the schema
version rather than silently adding fields that existing readers would reject.
Serialize a validated `Catalog` once, sign those bytes, and publish those exact bytes.
Reserializing, changing whitespace, or adding a newline after signing invalidates the signature.

The JSON manifest uses the same fields as TOML: optional `name_ja`, `license`,
`homepage`, dependency/optional/conflict semver-requirement maps, `bin`, shader arrays
with typed `values`/`default` options, and service arrays. A program's additional fields
look like this:

```json
{
  "bin": {
    "library": "example_program",
    "targets": ["windows-x64", "linux-x64"],
    "engine": "0.3.1+build.example",
    "abi": 2
  }
}
```

That version's catalog `targets` must repeat those two targets and `engine` must be
`"0.3.1+build.example"`. `macos-arm64` and `macos-x64` are also supported.
All service fields are declarative and offline. Unknown or secret-like keys are
rejected recursively, including keys resembling tokens, passwords, private/access/API
keys, credentials, secrets, or authorization. Service URLs reject credentials,
queries, and fragments. This is not a general-purpose detector of secrets hidden in
otherwise ordinary strings; never put secrets anywhere in a package or catalog.

## Signing wire format

Both signatures use Ed25519 through `horizon-release-sign`, not the upstream
registry's minisign. A public key is standard base64 of 32 raw bytes. A detached
signature is standard base64 of 64 raw bytes, with an optional surrounding newline.
Keys are caller-pinned; no certificate chain or key discovery is performed.

The signed bytes are the concatenation of the UTF-8 domain prefix and the payload:

| Domain | Prefix | Payload |
| --- | --- | --- |
| `yoake-mods-v1` | `horizon release yoake-mods-v1\n` | Original `index.json` bytes. |
| `yoake-yak-v1` | `horizon release yoake-yak-v1\n` | Canonical per-file hash list. |

For a package, hash every regular ZIP member except `SIGNATURE`, including
`yak.toml`, metadata, and every target's binaries. Sort by complete UTF-8 path in
ascending byte order and emit one line per file, ending every line with LF:

```text
<64 lowercase SHA-256 hex characters><two ASCII spaces><path><LF>
```

Explicit ZIP directory entries do not enter the list. Paths are not rewritten or
Unicode-normalized. `canonical_hashes` and `Archive::canonical_hash_list` implement
this shared representation. The `SIGNATURE` member is UTF-8 TOML:

```toml
signer = "<base64 public key>"
signature = "<base64 Ed25519 signature>"

[files]
"yak.toml" = "<lowercase SHA-256>"
"content/example.toml" = "<lowercase SHA-256>"
```

The `files` map must equal the computed map exactly: no missing, additional, or
changed files. ZIP entry order, compression, timestamps, and other container
metadata are not part of this per-file signature. The catalog's whole-archive hash
covers those bytes as well. Sign a package **before** recording its catalog size/hash.

## Safe archive and target view

The reader accepts single-disk ZIP/ZIP64 with Stored or Deflate members. It checks
central/local header agreement, CRCs, streamed lengths, hashes, and these limits:

- 256 MiB per regular file and 1 GiB total uncompressed size.
- 200:1 maximum uncompressed/compressed ratio per regular member.
- 16,384 ZIP entries, 32 MiB central metadata, 1 GiB + 32 MiB archive size.
- 1 MiB `yak.toml`, 2 MiB `SIGNATURE`, and 1,024 UTF-8 bytes per member path.

It rejects encryption, symlinks/devices/special entries, invalid UTF-8, absolute or
parent-relative paths, backslashes, control characters, empty/dot components,
Windows-reserved names, trailing dots/spaces, duplicate or case-colliding names,
and regular files used as parent directories. Explicit empty directory entries
are allowed but are not extracted. Only the declared package roots are accepted.
Every declared binary target needs its platform library, and every declared shader
needs files under its own `shaders/<id>/` directory.

Extraction retains `yak.toml`, `LICENSE`/`README.md` when present, common `content/`,
`assets/`, `shaders/`, `config/`, and only the selected `bin/<target>/` subtree,
without flattening the target directory. An embedded `SIGNATURE` is retained as
provenance, but still describes the **original complete archive**, not this subset.
Do not repack or verify a target view as if it were a standalone signed archive.

The destination must not exist. Extraction builds a random sibling temporary
directory, checks/rehashes while copying, and renames it into place only on success.
Failures remove the staging directory. Filesystem symlinks and Windows reparse
points in existing path components are refused. The caller must exclusively control
the destination parent and source paths: these checks are not a sandbox against an
adversary concurrently replacing directories. Packing also refuses an existing
output and publishes via a same-filesystem hard link; filesystems without hard-link
support report an error rather than silently overwriting.

## Library entry points

Enable `horizon-mods` feature `yak`; it adds optional semver, URL, and shared signing
dependencies. The signing crate's default `yak` feature enables archive tooling;
`default-features = false` keeps its pre-existing signing-only dependency surface.

- `Manifest::{read, parse, validate}` and typed `Bin`, `Target`, `Shader`,
  `ShaderOption`, `OptionValue`, `Service`, `Side`, and `Yoake` model package metadata.
- `Archive::{new, from_catalog}` fully inspect a seekable ZIP; `entries`, `hashes`,
  `files`, `parts`, `summary`, and `canonical_hash_list` expose the validated inventory.
- `Archive::{signature, verify_signature, extract_target}` handle optional embedded
  authentication and atomic target views. `new` checks a present signature's shape
  and hash list, but only `verify_signature` or extraction checks its pinned key.
- `pack_directory(root, output, binaries)` writes an unsigned package without running
  a build. `binaries` is a `BTreeMap<Target, PathBuf>` of files or target directories.
- `check_data_compatibility` checks the Yoake semver requirement;
  `check_compatibility` additionally requires an exact engine fingerprint (including
  build metadata), ABI, and target for programs. Neither function loads code.
- `resolve` checks an already selected set, including required/present optional
  dependency ranges, conflicts, duplicates, and cycles. Successful order is
  dependency-first, ID-sorted within each ready level. It does not select versions
  or download missing dependencies; failed resolutions have diagnostics and no order.
- `Catalog::{parse, parse_verified, validate}` and `CatalogVersion::verify_download`
  implement the catalog boundary. `SigningKey`, `verify_bytes`, and
  `horizon_release_sign::yak::{Signature, sign_file}` share domain-separated crypto.

## Developer and release commands

These commands describe producer usage on a machine with a Rust toolchain; they do
not require running Minecraft. `pack` defaults to the current directory and reads
its `yak.toml` before doing anything that may invoke a build.

```text
cargo horizon-mod pack path/to/mod
cargo horizon-mod pack path/to/mod --bin windows-x64=build/example_program.dll --bin linux-x64=build/libexample_program.so
cargo horizon-mod yak verify path/to/mod/example_program-1.0.0.yak
cargo horizon-mod yak verify path/to/mod/example_program-1.0.0.yak --key <base64-public-key>
horizon-release-sign sign --tag yoake-yak-v1 --key-file K path/to/example_program-1.0.0.yak
horizon-release-sign verify --tag yoake-yak-v1 --key <base64-public-key> path/to/example_program-1.0.0.yak
horizon-release-sign sign --tag yoake-mods-v1 --key-file K path/to/index.json
horizon-release-sign verify --tag yoake-mods-v1 --key <base64-public-key> path/to/index.json
```

`pack` writes `<id>-<version>.yak` in the mod directory and collects `content/`,
`assets/`, `shaders/`, `config/`, optional `LICENSE`/`README.md`, and declared binaries.
An explicit binary file is renamed to the target's expected library filename; a
directory contributes its whole target subtree. Relative `--bin` paths are relative
to the mod directory. Without overrides, a program package runs
`cargo build --release --lib` for the discovered Rust host target, using Cargo's
JSON artifact output to locate the library. Other declared targets must already be
available under `bin/<target>/`. Supplying any `--bin` disables automatic builds;
all remaining declared targets must already exist. Data-only packing never builds.

`yak verify` reports package identity/type, counts/parts, path/size/CRC/hash checks,
and unsigned or pinned-signature status. Repeat `--key` to allow multiple pinned
keys. An unsigned result is not authenticated merely because structural checks pass.
An embedded signature without a matching supplied key fails verification.

Package signing rewrites a new sibling archive, verifies its new signature, and
replaces the original only after success. Catalog signing writes `index.json.sig`
and refuses to overwrite an existing sidecar. The release signer validates the ZIP
container or bounded input bytes, not the complete semantic manifest/catalog schema:
producers must also use the full `horizon_mods::yak` validation before publishing.
Keep the production key in `C:\YoakePrivate` and supply it through `--key-file` on
the signing machine; never place it in a mod, catalog, repository, or test fixture.
Tests generate disposable Ed25519 test keys instead.

No command here publishes files, edits the upstream index, chooses trusted keys,
loads a native library, installs mods in a game directory, or contacts a service.
