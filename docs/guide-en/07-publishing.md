# 7. Publishing your mod

Once your WebAssembly mod works on your own machine, the next step is packaging and publishing it for other players. This chapter covers how to version `yak.toml`, what goes inside a `.yak` archive and its limits, how package and catalog signing work, how mods appear in the launcher's catalog, how updates and compatibility checks work, and what you must never do when publishing.

---

## 7-1. Versioning (`version` in `yak.toml`)

### Format and validation rules

The `version` field in `yak.toml` is parsed by the package library (`horizon_mods::yak`) as **`semver::Version` (Semantic Versioning)**[^yak-manifest]. You must write a complete three-part version string (`MAJOR.MINOR.PATCH`), such as `"1.0.0"` or `"0.3.1"`[^yak-manifest][^api-yoake-mods].

The package library, the launcher, and the game client check the following rules on version numbers:

- **Agreement with `index.json` (checked by library and launcher)**: Each mod `id` in the catalog can list `1..1,024` unique complete SemVer strings, and each catalog version entry's `version` must equal `manifest.version` (`version` in `yak.toml`)[^api-yoake-mods][^yak-catalog].
- **Agreement with bundled content (checked by library and game)**: When an archive includes `content/horizon-mod.toml`, its inner `id` and `version` must match the outer `yak.toml`[^api-yoake-mods][^yak-mods-client].
- **Multiplayer mod identity check (checked by game)**: A `[wasm]` package has a single multiplayer "identity" made up of its `id`, `version`, and `.wasm` file SHA-256 hash (using the client `.wasm`'s SHA-256 if the package has a client module, or else the server `.wasm`'s SHA-256). When `required_on_join = true`, the game verifies that the world host and joining guest have the exact same identity[^wasm-dev][^wasm-mods-client].
- **No reusing a version for different archive bytes (checked by launcher)**: When installing a mod, if an archive with the same `(id, version)` is already installed in the instance with a different `sha256`, the launcher rejects it (`"A different archive already uses this mod version"`)[^launcher-tx]. Any change to the archive requires a new `version`.

### What to bump and when

- The launcher only offers an update when a version greater than the installed version exists in the catalog (`version.version > installed.version`)[^launcher-plan][^launcher-rows]. Every new release must therefore increase the SemVer.
- Policy guidelines for mod authors on when to bump MAJOR vs. MINOR vs. PATCH are not specified in the reference sources (undecided).

---

## 7-2. Inside a `.yak` archive

### Packing commands and allowed files

For WebAssembly (`[wasm]`) mods, use the **`yoake pack`** command introduced in Chapters 2 and 3[^yoake-cli]. `yoake pack` builds the `[wasm]` modules, runs validation checks, and calls `horizon_mods::yak::pack_directory` to write an unsigned `.yak` archive to **`dist/<id>-<version>.yak`** inside your project (overwriting it on each run)[^yoake-cli][^yak-pack].

```sh
yoake pack
```

For native (`[bin]`) packages, data-only packages, and `.yak` archive verification, `docs/api/yoake-mods.md` provides `cargo horizon-mod pack` and `cargo horizon-mod yak verify` (writing `<id>-<version>.yak` in the mod directory)[^api-yoake-mods]:

```text
cargo horizon-mod pack path/to/mod
cargo horizon-mod yak verify path/to/mod/example_program-1.0.0.yak
```

Packing (`pack_directory`) writes a single-disk ZIP/ZIP64 archive using uncompressed (`Stored`) entries only[^yak-pack]; the archive reader accepts both `Stored` and `Deflate` members[^api-yoake-mods][^yak-root]. Only the following package roots are accepted inside a `.yak`[^api-yoake-mods][^yak-root][^yak-pack]:

| Path | Purpose |
| --- | --- |
| `yak.toml` | Required root manifest[^yak-root][^yak-pack]. |
| `LICENSE` / `README.md` | Optional root metadata files[^api-yoake-mods][^yak-root]. |
| `bin/wasm/<name>.wasm` | WebAssembly components declared under `[wasm]` (must be `.wasm` files directly under `bin/wasm/`, with no subdirectories)[^wasm-dev][^yak-manifest][^yak-root]. |
| `content/` | Unpacked `.hmod` content containing `horizon-mod.toml`[^api-yoake-mods][^yak-root]. |
| `assets/` | Asset files[^api-yoake-mods][^yak-root]. |
| `shaders/<id>/` | Files for each shader pack declared under `[[shaders]]`[^api-yoake-mods][^yak-root]. |
| `config/` | Configuration files[^api-yoake-mods][^yak-root]. |
| `bin/<target>/` | Target-specific dynamic libraries declared under `[bin]` (for native mods)[^api-yoake-mods][^yak-root]. |
| `SIGNATURE` | UTF-8 JSON signature member added at the archive root when signed[^api-yoake-mods][^yak-root]. |

For a WebAssembly mod, place the compiled component at `bin/wasm/<name>.wasm` and declare it in `yak.toml` with `interface = "1.0"`[^wasm-dev]:

```toml
[wasm]
client = "bin/wasm/hello.wasm"
interface = "1.0"
```

When using the `client-full` or `server-full` worlds, declare them in `yak.toml` like this[^wasm-dev]:

```toml
[wasm]
interface = "1.0"
client = "bin/wasm/demo-client.wasm"   # world client-mod (default) ...
world  = "client-full"                  # ... or the full client world
server = "bin/wasm/demo-server.wasm"   # always server-full
```

### Size and entry limits

The `.yak` reader and installer enforce these bounds[^api-yoake-mods][^yak-manifest][^yak-root][^yak-installed]:

- **Complete `.yak` archive size**: at most `1 GiB + 32 MiB` (`MAX_ARCHIVE_SIZE`)[^api-yoake-mods][^yak-root]
- **Total uncompressed size**: at most `1 GiB` (`MAX_TOTAL_SIZE`)[^api-yoake-mods][^yak-root]
- **Per regular file uncompressed size**: at most `256 MiB` (`MAX_FILE_SIZE`)[^api-yoake-mods][^yak-root]
- **Per WebAssembly module (`.wasm`)**: at most `32 MiB` (`MAX_WASM_SIZE`)[^yak-manifest][^yak-root]
- **`yak.toml`**: at most `1 MiB` (`MAX_MANIFEST_SIZE`)[^api-yoake-mods][^yak-manifest]
- **`SIGNATURE`**: at most `2 MiB` (`MAX_SIGNATURE_SIZE`)[^api-yoake-mods][^yak-root]
- **ZIP structure limits**: at most `16,384` ZIP entries, `32 MiB` central directory metadata, `1,024` UTF-8 bytes per member path, `200:1` maximum uncompressed/compressed ratio per regular member, and a maximum directory nesting depth of `64` in installed views[^api-yoake-mods][^yak-root][^yak-installed]

### What is rejected and what must NOT be inside

An archive is rejected during validation or extraction if it contains any of the following[^api-yoake-mods][^yak-manifest][^yak-root][^yak-pack]:

- Any root file or folder outside `yak.toml`, `SIGNATURE`, `LICENSE`, `README.md`, `content/`, `assets/`, `shaders/`, `config/`, and `bin/`[^api-yoake-mods][^yak-root].
- Undeclared `.wasm` files in `bin/wasm/`, undeclared shader directories under `shaders/<id>/`, or undeclared target binaries under `bin/<target>/`[^yak-root].
- **Both native code (`[bin]`) and WebAssembly (`[wasm]`)**: a package may contain `[bin]` or `[wasm]`, never both[^yak-manifest].
- Encrypted ZIP members, symlinks, Windows reparse points, or device/special files[^api-yoake-mods][^yak-root][^yak-installed].
- Invalid UTF-8, absolute paths, `..` or `.` segments, backslashes (`\`), control characters, empty path components, trailing dots or spaces, Windows-reserved names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`), or duplicate/case-colliding paths[^api-yoake-mods][^yak-manifest][^yak-root].
- Secret-like key names in `yak.toml`'s `[[services]]` (such as `secret`, `token`, `password`, `apikey`, or `privatekey`) or URLs containing credentials, queries, or fragments[^api-yoake-mods][^yak-manifest].

> **Warning (never include secrets)**: Rejecting secret-like keys in `[[services]]` and URL credentials is not a general-purpose detector of secrets hidden in ordinary strings or files[^api-yoake-mods]. Never put passwords, API keys, tokens, private keys, or any other secrets anywhere in a package or catalog[^api-yoake-mods].

---

## 7-3. What signing is and who signs

### How package and catalog signatures work

Yoake signatures use **Ed25519** via `horizon-release-sign` (a public key is 32 raw bytes in standard base64; a signature is 64 raw bytes in standard base64)[^api-yoake-mods]. Two signing domains are used[^api-yoake-mods]:

| Domain | Prefix | Payload and location |
| --- | --- | --- |
| `yoake-yak-v1` | `horizon release yoake-yak-v1\n` | Canonical sorted per-file SHA-256 list (`<64 lowercase hex>  <path>\n`) for every regular ZIP member except `SIGNATURE`, stored as UTF-8 JSON in `SIGNATURE` at the ZIP root[^api-yoake-mods]. |
| `yoake-mods-v1` | `horizon release yoake-mods-v1\n` | Original `index.json` bytes, stored as the detached signature `index.json.sig`[^api-yoake-mods]. |

The signing and verification commands are[^api-yoake-mods]:

```text
horizon-release-sign sign --tag yoake-yak-v1 --key-file K path/to/example_program-1.0.0.yak
horizon-release-sign verify --tag yoake-yak-v1 --key <base64-public-key> path/to/example_program-1.0.0.yak
horizon-release-sign sign --tag yoake-mods-v1 --key-file K path/to/index.json
horizon-release-sign verify --tag yoake-mods-v1 --key <base64-public-key> path/to/index.json
```

Because the catalog's `size` and `sha256` cover all bytes of the `.yak` ZIP including `SIGNATURE`, you must **sign a package before** recording its catalog size and hash[^api-yoake-mods].

### Who signs

- A public key embedded in `SIGNATURE.signer` is **never** a trust anchor on its own[^api-yoake-mods].
- Both the game client and the launcher pin **`horizon_mods::yak::trusted_keys()`**: Yoake's signing keys compiled in at build time from `YOAKE_COMPILED_PUBLIC_KEYS` (`;`-separated), the same keys pinned by the launcher for its own updates and feeds[^api-yoake-mods][^yak-installed][^launcher-mods]. A build without `YOAKE_COMPILED_PUBLIC_KEYS` trusts no key and fails closed[^api-yoake-mods][^yak-installed].
- The production signing key is kept in a private folder outside any repository and supplied via `--key-file` on the signing machine[^api-yoake-mods].

> **Important: WebAssembly mods for players must be signed with Yoake's key**
> A `.yak` archive created by `yoake pack` (or `cargo horizon-mod pack`) is **unsigned**[^api-yoake-mods][^yoake-cli]. An unsigned `.yak` runs only on the author's machine with developer mode (`YOAKE_DLL_MODS_UNSIGNED=1`) enabled[^yak-mods-client]. For players to run a WebAssembly mod in a normal game session, the `.yak` must carry a valid `SIGNATURE` signed with Yoake's key[^yak-mods-client][^yak-installed]. How mod authors submit a package to be signed with Yoake's key is (undecided).

### What the launcher and game check

1. **Launcher checks**:
   - Verifies `index.json` against `index.json.sig` using `trusted_keys()` (`Catalog::parse_verified`) and checks that every download URL shares the portal's HTTPS origin[^api-yoake-mods][^launcher-mods].
   - Verifies each downloaded `.yak`'s exact byte size, whole-archive SHA-256, ZIP structure, and full normalized `yak.toml` against the verified catalog entry (`Archive::from_catalog`)[^api-yoake-mods][^launcher-mods].
   - Whenever `SIGNATURE` is present in an archive, verifies it against `trusted_keys()`. An invalid or untrusted signature is always rejected, even in developer mode[^api-yoake-mods][^launcher-plan][^launcher-local-tests].
   - Because the launcher's planner (`plan.rs`) blocks unsigned archives only when `manifest.bin.is_some()`, the launcher may install a non-`[bin]` package (data-only or `[wasm]`) from the catalog or from disk even if unsigned (displaying `Unsigned .yak` with a confirmation prompt for local files)[^launcher-plan][^launcher-rows]. **However, a `[wasm]` package without a valid pinned-key `SIGNATURE` will be refused by the game and will not run**[^yak-mods-client].
2. **Game startup checks**:
   - Reads only `<game dir>/yoake-mods.txt` and the extracted target view `<game dir>/yoake-mods/<id>/<version>/yak.toml` (never `.cache/` or the catalog)[^api-yoake-mods].
   - Checks path safety (refusing symlinks, reparse points, and oversized trees), the Yoake SemVer requirement, and side[^api-yoake-mods][^yak-mods-client].
   - **Before loading any WebAssembly (`[wasm]`) module**: calls `wasm_mods_from(..., &yak::trusted_keys(), unsigned_allowed())` → `wasm_preflight`, which rechecks the installed tree, `yak.toml`, and the `.wasm` file size (at most 32 MiB) and calls `yak::verify_installed` to verify `SIGNATURE` (against `trusted_keys()`) as well as the SHA-256 hashes of `yak.toml` and the `.wasm` module[^yak-mods-client][^yak-installed]. Without a valid pinned-key `SIGNATURE`, the module is refused (`"YAK <id> wasm module refused"`) unless `YOAKE_DLL_MODS_UNSIGNED=1` is set[^yak-mods-client].
   - **Before loading any native (`[bin]`) library**: rechecks the tree, rereads `yak.toml`, reruns compatibility, and calls `verify_installed` to verify the hashes of the DLL and `yak.toml` against the pinned-key-authenticated `SIGNATURE` map[^api-yoake-mods].

### What "unsigned" means and why it is only for development

- An archive without `SIGNATURE` is unsigned. Passing structural checks in `cargo horizon-mod yak verify` does not authenticate an unsigned archive[^api-yoake-mods].
- Testing a local unsigned package launched from the launcher requires opening the hidden Developer Options page (clicking `"Launcher x.y.z"` in the sidebar 7 times), enabling `"Test your own DLL mods (developer mode)"`, and confirming the warning dialog[^launcher-dev].
- In the game client, only the exact environment variable `YOAKE_DLL_MODS_UNSIGNED=1` bypasses missing, invalid, or untrusted signatures or checksum failures for development[^api-yoake-mods][^yak-mods-client]. `YOAKE_DLL_MODS_UNSIGNED=1` also marks every unsigned WebAssembly module in installed `.yak` packages for hot-reload watching (`watch: (developer && !module.signed)`, polled once a second)[^wasm-dev][^wasm-mods-client] and enables loading loose `<id>.wasm` files from `YOAKE_WASM_DEV_DIR=<folder>`[^wasm-dev][^wasm-mods-client]. These switches exist strictly for local development and testing, never for player distribution[^api-yoake-mods][^wasm-dev][^launcher-dev].

---

## 7-4. Getting into the launcher's catalog

### Feeds, portal, and catalog architecture

The launcher relies on two signed indexes:

1. **Base environment feed (`yoake_feed.rs`)**: Configured at build time from `YOAKE_COMPILED_FEED_URL`, `YOAKE_COMPILED_PUBLIC_KEYS`, and `YOAKE_COMPILED_SOURCE_ID` (`"yoake-official"` in schema 2) and signed under `yoake-index-v1`[^launcher-feed]. It records the instance's signed `YoakeBuildIdentity` (`version`, `engine`, `abi`) used to check mod compatibility[^launcher-mods][^launcher-feed].
2. **Mod catalog (`index.json`)**: Fetched from the configured portal (`YOAKE_PORTAL_URL`) at `/yoake/mods/index.json` (up to 16 MiB) with detached signature `/yoake/mods/index.json.sig` (up to 4 KiB, domain `yoake-mods-v1`)[^api-yoake-mods][^launcher-mods]. Each package version lives at `https://<portal>/yoake/mods/<id>/<version>/<id>-<version>.yak`[^api-yoake-mods][^yak-catalog].
   - Fetching the catalog and `.yak` archives requires portal bearer authentication (`yoake-token` obtained through member browser device login or a friend test-play invite code)[^launcher-mods][^launcher-login]. Test-play invitees use the exact same catalog path and package format[^api-yoake-mods].

> As `docs/api/yoake-mods.md` notes, *"No command here publishes files, edits the upstream index, chooses trusted keys..."*[^api-yoake-mods]. The concrete portal URL and the procedure for submitting a mod to the portal/catalog maintainers are not specified in the reference sources (undecided).

### Required metadata in `yak.toml` and `index.json`

Every published version includes its full normalized `yak.toml` inside `index.json` under `manifest`[^api-yoake-mods][^yak-catalog]:

- **Required `yak.toml` fields**[^yak-manifest]:
  - `format = 1` (unknown fields are rejected throughout)
  - `id`: `3..48` characters of lowercase ASCII `a-z`, `0-9`, `_`, or `-` (not a Windows-reserved name)
  - `version`: complete SemVer string (e.g. `"1.0.0"`)
  - `name`: display name (`1..256` bytes, no control characters)
  - `authors`: `1..128` author names (`1..256` bytes each)
  - `description`: `1..16,384` bytes
  - `[yoake] version`: SemVer requirement for Yoake (e.g. `">=0.3.0, <0.4.0"`)
  - `[side]`: booleans `client`, `server`, `required_on_join` (installing into a launcher client instance requires `client = true`[^launcher-plan])
- **Optional `yak.toml` fields**[^yak-manifest][^wasm-dev]:
  - `name_ja` (`1..256` bytes, preferred by the launcher when the UI language is Japanese[^launcher-mods]), `license` (`1..256` bytes), `homepage` (HTTP/HTTPS URL up to `4,096` bytes with no credentials, query, or fragment)
  - `dependencies`, `optional`, `conflicts` (up to `1,024` entries each, no self-dependencies, disjoint `dependencies` and `optional`)
  - `[wasm]`: `interface = "1.0"`, `client` / `server` `.wasm` paths, `world` (`"client-mod"` or `"client-full"`), and `permissions` (from `"service.commands"`, `"service.config"`, `"service.http"`, `"service.permissions"`, `"service.storage"`)
  - `[config.<key>]` (up to `256` keys), `[permissions.<node>]` (up to `256` nodes), `[http]` (`hosts`, requires `service.http`), `[[shaders]]` (up to `128`), `[[services]]` (up to `128`)
- **`index.json` format** (from `docs/api/yoake-mods.md`)[^api-yoake-mods]:

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

---

## 7-5. Publishing updates and compatibility

### Publishing a new version and how the launcher sees it

1. Bump `version` in `yak.toml` (and `content/horizon-mod.toml` if present) to a higher SemVer, update `[yoake] version` if needed, and pack the `.yak`[^api-yoake-mods][^yak-manifest].
2. After the `.yak` is signed, record its complete ZIP `size` and `sha256` in a new entry under the mod's `versions` array in `index.json`[^api-yoake-mods]. Serialize the validated `Catalog` once, sign those exact bytes with `yoake-mods-v1`, and publish them without reserializing or changing whitespace[^api-yoake-mods]. How a mod author submits an updated package to the portal is (undecided).
3. The launcher refreshes the catalog when the Mods page opens, when the user clicks `Refresh`, when credentials change, or automatically every **300 seconds (5 minutes)**[^launcher-page].
4. For each installed mod, the launcher checks all catalog versions with the same `id` where `version.version > installed.version` and `environment.check(&version.manifest).is_ok()`, picking the highest SemVer[^launcher-plan][^launcher-rows]. When found, the row shows `Update <version>`; confirming the change (`Review mod changes` → `Confirm`) downloads and verifies the new archive into `yoake-mods/.cache/<sha256>.yak`, extracts `<game dir>/yoake-mods/<id>/<new-version>/`, removes `<game dir>/yoake-mods/<id>/<old-version>/`, and rewrites `installed.json` and `yoake-mods.txt` in dependency-first order[^launcher-page][^launcher-rows][^launcher-tx].

### Compatibility (`[yoake] version` and `interface`)

- **`[yoake] version`**: Checked via `check_data_compatibility` against the instance's Yoake version[^api-yoake-mods][^yak-compat]. If a Yoake update moves the instance outside a mod's SemVer requirement, the launcher flags the incompatibility and the client skips the entry at startup with a title-screen notice[^api-yoake-mods][^launcher-mods].
- **`[wasm] interface`**: Must be `"major.minor"` matching `SUPPORTED_WASM_INTERFACE = (1, 0)`, i.e. `interface = "1.0"` (`major != 1` or `minor > 0` is rejected)[^wasm-dev][^yak-manifest]. In addition, if a host WIT world changes (for example, when `handle-call` was added to `server-full`), modules compiled before the change cannot be loaded and must be rebuilt with the updated SDK[^wasm-dev].

---

## 7-6. Things you must NOT do

Every prohibition below is enforced by or documented in the reference sources:

1. **Never ask players to set `YOAKE_DLL_MODS_UNSIGNED=1` or turn on the launcher's unsigned-DLL developer mode**:
   Those switches bypass signature verification strictly for local development, and native DLL code runs unsandboxed[^api-yoake-mods][^launcher-dev]. Distributed WebAssembly mods must be signed with Yoake's key; otherwise players cannot run them in the game[^yak-mods-client].
2. **Never bundle native DLLs inside a WebAssembly mod or distribute unsigned `[wasm]` or `[bin]` mods to players**:
   A `.yak` cannot declare both `[bin]` and `[wasm]`, and undeclared binaries are rejected[^yak-manifest][^yak-root]. Because the game requires a valid Yoake pinned-key `SIGNATURE` for both `[wasm]` and `[bin]` packages, never distribute unsigned code mods to players[^api-yoake-mods][^yak-mods-client][^launcher-plan].
3. **Never reuse another mod's `id`**:
   The catalog, `installed.json`, `yoake-mods.txt`, and `yak::resolve` require unique IDs, and installing a mod replaces any installed mod with the same `id`[^api-yoake-mods][^yak-catalog][^launcher-tx]. Config (`<game>/config/<mod-id>.toml`), storage (`yoake-data/<mod-id>/...`), and permissions (`[mods.<mod-id>]`) are all keyed by `id`[^wasm-dev][^wasm-storage].
4. **Never put secrets, tokens, passwords, or private keys in a `.yak`, `yak.toml`, or catalog**:
   `yak.toml` rejects secret-like keys in `[[services]]` and credentials in URLs, but this is not a general-purpose detector of secrets hidden in ordinary strings; never place secrets or signing private keys anywhere in a package, catalog, or repository[^api-yoake-mods][^yak-manifest].
5. **Never rely on the internal on-disk storage format, edit storage files directly, or try to access them via `std::fs`**:
   WebAssembly guests have no preopened filesystem directory, so every `std::fs` call fails[^wasm-dev]. Moreover, the on-disk format is internal to the host and may change—never edit the files directly; always use `storage_get` / `storage_put` / `storage_delete` and the `storage_blob_*` APIs (WIT `storage-get` / `storage-put` / `storage-delete` / `storage-blob-*` imports)[^wasm-dev][^wasm-storage].
   - **Scope folders and per-key files**: Stores live under `<world>/yoake-data/<mod-id>/` (`world`, server-only), `<game>/yoake-data/<mod-id>/global/` (`global`, server-only), and `<game>/yoake-data/<mod-id>/client/` (`client`, client-only)[^wasm-dev][^wasm-storage]. Instead of a single `store.bin`, each store folder uses an exclusive lock file (`store.lock`) and one atomic entry file per key (`entry-<pid>-<seq>.bin`, keeping only keys and lengths resident in memory while reading values and blob slices on demand)[^wasm-dev][^wasm-storage]. Writes go temp file (`.store-<pid>-<seq>.tmp`) → `fsync` (`sync_all`) → atomic rename, and stale temp files are removed on open[^wasm-dev][^wasm-storage].
   - **Quotas, blobs, and write budget**: Each store has a `256 MiB` quota (including serialized metadata and reserved pending uploads) and at most `65,536` keys (up to `256` bytes per key)[^wasm-dev][^wasm-storage]. Inline reads/writes (`storage_get` / `storage_put`) and blob chunks (`storage_blob_write` / `storage_blob_read`) are capped at `1 MiB`[^wasm-dev][^wasm-storage]. Blob uploads (`storage-blob-begin` / `write` / `commit` / `abort` / `read`) allow at most `64` outstanding uploads per store and a **32 MiB aggregate pending-blob limit per mod** across all scopes; buffers grow lazily on write, chunks may arrive out of order (up to `4,096` disjoint ranges), commit requires complete coverage, and a failed commit retains the upload for completion or abort[^wasm-dev][^wasm-storage]. Across scopes, encoded writes—including config writes—share a per-mod token bucket of **8 MiB per second sustained, 32 MiB burst**, returning an error when exceeded[^wasm-dev].
   - **Automatic legacy import and corrupted-entry handling**: On first bind (when `migration-v2.done` is absent), `world` and `global` stores automatically import old `store.bin` data and legacy `<world>/yoake-wasm/<mod-id>/<key>.bin` (`world`) or `<game>/config/yoake-wasm/<mod-id>/<key>.bin` (`global`) entries (stripping `.bin`; `client` stores import neither) within quota and without replacing newer data[^wasm-dev][^wasm-storage]. The old files are kept as recovery backups, and the `migration-v2.done` marker prevents repeated imports[^wasm-dev][^wasm-storage]. Corrupted or unreadable entry files (or legacy items) are skipped with a warning (`[yoake] storage ...: skipping ...`) so healthy entries remain usable[^wasm-dev][^wasm-storage].
   - **Note**: This storage design requires an engine version from `yoake/w2d-services` (`@4ec9e65b2`) or later[^wasm-dev][^wasm-storage].
6. **Never modify a `.yak` or `index.json` after signing, or repack an extracted target view (`<game dir>/yoake-mods/<id>/<version>/`)**:
   Changing a single byte invalidates the signature and SHA-256, and the `SIGNATURE` retained in an extracted directory authenticates the original complete `.yak` archive, not the extracted subset[^api-yoake-mods].

---

[^api-yoake-mods]: Source: `docs/api/yoake-mods.md`
[^wasm-dev]: Source: `docs/yoake-wasm-dev.md`
[^wasm-storage]: Source: `crates/yoake-wasm/src/host_services/storage.rs`
[^yoake-cli]: Source: `docs/guide-ja/02-first-mod.md` / `docs/guide-ja/03-play.md` (`yoake pack` in `yoake/p10-tools` CLI)
[^yak-root]: Source: `crates/horizon-mods/src/yak.rs`
[^yak-manifest]: Source: `crates/horizon-mods/src/yak/manifest.rs`
[^yak-catalog]: Source: `crates/horizon-mods/src/yak/catalog.rs`
[^yak-compat]: Source: `crates/horizon-mods/src/yak/compatibility.rs`
[^yak-installed]: Source: `crates/horizon-mods/src/yak/installed.rs`
[^yak-pack]: Source: `crates/horizon-mods/src/yak/pack.rs`
[^yak-mods-client]: Source: `crates/horizon-client/src/yak_mods.rs`
[^wasm-mods-client]: Source: `crates/horizon-client/src/wasm_mods.rs`
[^launcher-mods]: Source: `launcher/src/yoake_mods.rs`
[^launcher-plan]: Source: `launcher/src/yoake_mods/plan.rs`
[^launcher-tx]: Source: `launcher/src/yoake_mods/plan/transaction.rs`
[^launcher-local-tests]: Source: `launcher/src/yoake_mods/plan/tests/local.rs`
[^launcher-page]: Source: `launcher/src/yoake_mods_page.rs`
[^launcher-rows]: Source: `launcher/src/yoake_mods_page/rows.rs`
[^launcher-feed]: Source: `launcher/src/yoake_feed.rs`
[^launcher-login]: Source: `launcher/src/yoake_login.rs`
[^launcher-dev]: Source: `launcher/src/dev_options.rs`
