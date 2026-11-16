# 7. Publishing Your Mod

Once your WebAssembly (Wasm) mod works on your own machine, the next step is packaging and publishing it for other players. This chapter covers how to version `yak.toml`, what goes inside a `.yak` package and its limits, how package and catalog signing work, how mods appear in the launcher's catalog, how updates and compatibility checks work, and what you must never do when publishing.

---

## 7-1. Versioning (`version` in `yak.toml`)

### Format and Validation Rules

The `version` field in `yak.toml` is parsed by the package library (`horizon_mods::yak`) as **`semver::Version` (Semantic Versioning)**[^yak-manifest]. You must write a complete three-part version string (`MAJOR.MINOR.PATCH`), such as `"1.0.0"` or `"0.3.1"`[^yak-manifest][^api-yoake-mods].

The package library, the launcher, and the game client check the following rules on version numbers:

- **Agreement with `index.json` (checked by library and launcher)**: Each package ID (`id` in `yak.toml`) in the catalog can list `1..1,024` unique complete SemVer strings, and each catalog version entry's `version` must equal `manifest.version` (`version` in `yak.toml`)[^api-yoake-mods][^yak-catalog].
- **Agreement with bundled content (checked by library and game)**: When a package includes `content/horizon-mod.toml`, its inner `id` and `version` must match the outer `yak.toml`[^api-yoake-mods][^yak-mods-client].
- **Multiplayer mod identity check (checked by game)**: A `[wasm]` package has a single multiplayer "identity" made up of its `id`, `version`, and `.wasm` file SHA-256 hash (using the SHA-256 of the client module if the package has one, or else of the server module). When `required_on_join = true`, the game verifies that the world host and joining guest have the exact same identity[^wasm-dev][^wasm-mods-client]. A module that is being hot-reloaded for development does not pin a `required_on_join` digest, because its bytes can change.
- **No reusing a version for different package bytes (checked by launcher)**: When installing a mod, if a package with the same `(id, version)` is already installed in the instance with a different `sha256`, the launcher rejects it (`"A different archive already uses this mod version"`)[^launcher-tx]. Any change to the package requires a new `version`.

### What to Bump and When

- The launcher only offers an update when a version greater than the installed version exists in the catalog (`version.version > installed.version`)[^launcher-plan][^launcher-rows]. Every new release must therefore increase the SemVer.
- Policy guidelines for mod authors on when to bump MAJOR vs. MINOR vs. PATCH are not specified in the reference sources (undecided).

---

## 7-2. Inside a `.yak` Archive

### Packing Commands and Allowed Files

For Wasm (`[wasm]`) mods, use the **`yoake pack`** command introduced in [Chapter 2](./02-first-mod.md) and [Chapter 3](./03-play.md)[^yoake-cli]. `yoake pack` builds the `[wasm]` modules, runs validation checks, and calls `horizon_mods::yak::pack_directory` to write an unsigned `.yak` package to **`dist/<id>-<version>.yak`** inside your project (overwriting it on each run)[^yoake-cli][^yak-pack].

```sh
yoake pack
```

Native DLL mods (`[bin]`) are an advanced path for the owner and developers. For `[bin]` packages, data-only packages, and `.yak` verification, `docs/api/yoake-mods.md` provides `cargo horizon-mod pack` and `cargo horizon-mod yak verify` (writing `<id>-<version>.yak` in the mod directory)[^api-yoake-mods]:

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
| `bin/<target>/` | Target-specific dynamic libraries declared under `[bin]` (native mods, an advanced path)[^api-yoake-mods][^yak-root]. |
| `SIGNATURE` | UTF-8 JSON signature member added at the archive root when signed[^api-yoake-mods][^yak-root]. |

For a Wasm mod, place the compiled component at `bin/wasm/<name>.wasm` and declare it in `yak.toml` with `interface = "1.0"`[^wasm-dev]:

```toml
[wasm]
client = "bin/wasm/hello.wasm"
interface = "1.0"
```

When using the `client-full` or `server-full` WIT worlds, declare them in `yak.toml` like this[^wasm-dev]:

```toml
[wasm]
interface = "1.0"
client = "bin/wasm/demo-client.wasm"   # WIT world client-mod (default) ...
world  = "client-full"                  # ... or the full client WIT world
server = "bin/wasm/demo-server.wasm"   # always server-full
```

A Mixin mod instead uses `interface = "1.1"` and `world = "inject-mod"`, with the `[mixin]` section described in [Chapter 8](./08-mixin.md)[^yak-manifest].

### Size and Entry Limits

The `.yak` reader and installer enforce these bounds[^api-yoake-mods][^yak-manifest][^yak-root][^yak-installed]:

- **Complete `.yak` archive size**: at most `1 GiB + 32 MiB` (`MAX_ARCHIVE_SIZE`)[^api-yoake-mods][^yak-root]
- **Total uncompressed size**: at most `1 GiB` (`MAX_TOTAL_SIZE`)[^api-yoake-mods][^yak-root]
- **Per regular file uncompressed size**: at most `256 MiB` (`MAX_FILE_SIZE`)[^api-yoake-mods][^yak-root]
- **Per WebAssembly module (`.wasm`)**: at most `32 MiB` (`MAX_WASM_SIZE`)[^yak-manifest][^yak-root]
- **`yak.toml`**: at most `1 MiB` (`MAX_MANIFEST_SIZE`)[^api-yoake-mods][^yak-manifest]
- **`SIGNATURE`**: at most `2 MiB` (`MAX_SIGNATURE_SIZE`)[^api-yoake-mods][^yak-root]
- **ZIP structure limits**: at most `16,384` ZIP entries, `32 MiB` central directory metadata, `1,024` UTF-8 bytes per member path, `200:1` maximum uncompressed/compressed ratio per regular member, and a maximum directory nesting depth of `64` in installed views[^api-yoake-mods][^yak-root][^yak-installed]

### What Is Rejected and What Must NOT Be Inside

A package is rejected during validation or extraction if it contains any of the following[^api-yoake-mods][^yak-manifest][^yak-root][^yak-pack]:

- Any root file or folder outside `yak.toml`, `SIGNATURE`, `LICENSE`, `README.md`, `content/`, `assets/`, `shaders/`, `config/`, and `bin/`[^api-yoake-mods][^yak-root].
- Undeclared `.wasm` files in `bin/wasm/`, undeclared shader directories under `shaders/<id>/`, or undeclared target binaries under `bin/<target>/`[^yak-root].
- **Both native code (`[bin]`) and WebAssembly (`[wasm]`)**: a package may contain `[bin]` or `[wasm]`, never both[^yak-manifest].
- Encrypted ZIP members, symlinks, Windows reparse points, or device/special files[^api-yoake-mods][^yak-root][^yak-installed].
- Invalid UTF-8, absolute paths, `..` or `.` segments, backslashes (`\`), control characters, empty path components, trailing dots or spaces, Windows-reserved names (`CON`, `PRN`, `AUX`, `NUL`, `COM1`–`COM9`, `LPT1`–`LPT9`), or duplicate/case-colliding paths[^api-yoake-mods][^yak-manifest][^yak-root].
- Secret-like key names in `yak.toml`'s `[[services]]` (such as `secret`, `token`, `password`, `apikey`, or `privatekey`) or URLs containing credentials, queries, or fragments[^api-yoake-mods][^yak-manifest].

> [!WARNING]
> **Never include secrets.** Rejecting secret-like keys in `[[services]]` and URL credentials is not a general-purpose detector of secrets hidden in ordinary strings or files[^api-yoake-mods]. Never put passwords, API keys, tokens, private keys, or any other secrets anywhere in a package or catalog[^api-yoake-mods].

---

## 7-3. What Signing Is and Who Signs

### How Package and Catalog Signatures Work

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

### Who Signs

- A public key embedded in `SIGNATURE.signer` is **never** a trust anchor on its own[^api-yoake-mods].
- Both the game client and the launcher pin **`horizon_mods::yak::trusted_keys()`**: Yoake's pinned signing keys, compiled in at build time from `YOAKE_COMPILED_PUBLIC_KEYS` (`;`-separated), the same keys pinned by the launcher for its own updates and feeds[^api-yoake-mods][^yak-installed][^launcher-mods]. A build without `YOAKE_COMPILED_PUBLIC_KEYS` trusts no key and fails closed[^api-yoake-mods][^yak-installed].
- The production signing key is kept in a private folder outside any repository and supplied via `--key-file` on the signing machine[^api-yoake-mods].

> [!IMPORTANT]
> **Wasm mods for players must be signed with Yoake's key.**
> A `.yak` package created by `yoake pack` (or `cargo horizon-mod pack`) is **unsigned**[^api-yoake-mods][^yoake-cli]. An unsigned `.yak` runs only on the author's machine, with Wasm developer mode enabled (`YOAKE_WASM_UNSIGNED=1`, or the launcher's "Test your own WASM mods")[^yak-mods-client]. For players to run a Wasm mod in a normal game session, the `.yak` must carry a valid `SIGNATURE` signed with Yoake's key[^yak-mods-client][^yak-installed]. How mod authors submit a package to be signed with Yoake's key is (undecided).

<!-- TODO(Yoake): Once the signing-request contact and the portal submission procedure are decided, rewrite this paragraph and the other (undecided) portal-submission spots in chapter 07. -->

### What the Launcher and Game Check

1. **Launcher checks**:
   - Verifies `index.json` against `index.json.sig` using `trusted_keys()` (`Catalog::parse_verified`) and checks that every download URL shares the portal's HTTPS origin[^api-yoake-mods][^launcher-mods].
   - Verifies each downloaded `.yak`'s exact byte size, whole-archive SHA-256, ZIP structure, and full normalized `yak.toml` against the verified catalog entry (`Archive::from_catalog`)[^api-yoake-mods][^launcher-mods].
   - Whenever `SIGNATURE` is present in a package, verifies it against `trusted_keys()`. An invalid or untrusted signature is always rejected, even in developer mode[^api-yoake-mods][^launcher-plan][^launcher-local-tests].
   - Because the launcher's planner (`plan.rs`) blocks unsigned packages only when `manifest.bin.is_some()`, the launcher may install a non-`[bin]` package (data-only or `[wasm]`) from the catalog or from disk even if unsigned (displaying `Unsigned .yak` with a confirmation prompt for local files)[^launcher-plan][^launcher-rows]. **However, a `[wasm]` package without a valid pinned-key `SIGNATURE` will be refused by the game and will not run**, unless Wasm developer mode is on[^yak-mods-client].
2. **Game startup checks**:
   - Reads only `<game dir>/yoake-mods.txt` and the extracted target view `<game dir>/yoake-mods/<id>/<version>/yak.toml` (never `.cache/` or the catalog)[^api-yoake-mods].
   - Checks path safety (refusing symlinks, reparse points, and oversized trees), the Yoake SemVer requirement, and side[^api-yoake-mods][^yak-mods-client].
   - **Before loading any Wasm (`[wasm]`) module**: calls `wasm_mods_from(..., &yak::trusted_keys(), allow_unsigned)` → `wasm_preflight`, which rechecks the installed tree, `yak.toml`, and the `.wasm` file size (at most 32 MiB) and calls `yak::verify_installed` to verify `SIGNATURE` (against `trusted_keys()`) as well as the SHA-256 hashes of `yak.toml` and the `.wasm` module[^yak-mods-client][^yak-installed]. Without a valid pinned-key `SIGNATURE`, the module is refused (`"YAK <id> wasm module refused"`). `allow_unsigned` is true only when Wasm developer mode is on (`YOAKE_WASM_UNSIGNED=1`, or the DLL switch `YOAKE_DLL_MODS_UNSIGNED=1`, which includes it), and even then it covers only a package that has no `SIGNATURE` file at all[^yak-mods-client].
   - **Before loading any native (`[bin]`) library**: rechecks the tree, rereads `yak.toml`, reruns compatibility, and calls `verify_installed` to verify the hashes of the DLL and `yak.toml` against the pinned-key-authenticated `SIGNATURE` map[^api-yoake-mods].

### What "Unsigned" Means and Why It Is Only for Development

- A package without `SIGNATURE` is unsigned. Passing structural checks in `cargo horizon-mod yak verify` does not authenticate an unsigned package[^api-yoake-mods].
- There are **two separate developer-mode switches**, one for Wasm and one for DLL[^api-yoake-mods][^wasm-dev][^launcher-dev]:
  - **Wasm developer mode** is the launcher's hidden Developer options page (revealed by clicking `"Launcher x.y.z"` in the sidebar 7 times) → `"Test your own WASM mods"`, confirmed with the `"Turn on"` button (setting `dev_unsigned_wasm`), or the environment variable `YOAKE_WASM_UNSIGNED=1` (the value must be exactly `1`). It permits `.yak` packages that have no `SIGNATURE` file, declare `[wasm]`, and contain no `[bin]`, and it enables loose `<id>.wasm` files from `YOAKE_WASM_DEV_DIR=<folder>` (their ids get a `dev-` prefix) as well as hot reload. A `SIGNATURE` that is present must still be valid and match the installed files. This switch never authorizes native code; Wasm stays sandboxed, but the service permissions a mod declares (such as `service.commands` on a server module) still apply[^yak-mods-client][^wasm-mods-client].
  - **DLL developer mode** is the launcher's `"Test your own DLL mods (developer mode)"` toggle (setting `dev_unsigned_dll`) or the environment variable `YOAKE_DLL_MODS_UNSIGNED=1` (the value must be exactly `1`). It is a superset: it also permits unsigned native DLL mods (bypassing missing, invalid, or untrusted signatures or checksum failures for native development) and enables the Wasm development features too. Native code is not sandboxed, so use it only for your own native DLL work[^yak-mods-client].
- **Hot reload covers `client-mod` only.** With either switch on, the game polls once a second the unsigned `client-mod` modules in installed `.yak` packages (`developer && !module.signed`) and the modules loaded from `YOAKE_WASM_DEV_DIR`, and swaps in a fresh instance when a file changes. `client-full` and `server-full` modules are loaded once per run and are never watched[^wasm-dev][^wasm-mods-client].
- These switches exist strictly for local development and testing, never for player distribution[^api-yoake-mods][^wasm-dev][^launcher-dev].

---

## 7-4. Getting Into the Launcher's Catalog

### Feeds, Portal, and Catalog Architecture

The launcher relies on two signed indexes:

1. **Base environment feed (`yoake_feed.rs`)**: Configured at build time from `YOAKE_COMPILED_FEED_URL`, `YOAKE_COMPILED_PUBLIC_KEYS`, and `YOAKE_COMPILED_SOURCE_ID` (`"yoake-official"` in schema 2) and signed under `yoake-index-v1`[^launcher-feed]. It records the instance's signed `YoakeBuildIdentity` (`version`, `engine`, `abi`) used to check mod compatibility[^launcher-mods][^launcher-feed].
2. **Mod catalog (`index.json`)**: Fetched from the configured portal (`YOAKE_PORTAL_URL`) at `/yoake/mods/index.json` (up to 16 MiB) with detached signature `/yoake/mods/index.json.sig` (up to 4 KiB, domain `yoake-mods-v1`)[^api-yoake-mods][^launcher-mods]. Each package version lives at `https://<portal>/yoake/mods/<id>/<version>/<id>-<version>.yak`[^api-yoake-mods][^yak-catalog].
   - Fetching the catalog and `.yak` packages requires portal bearer authentication (`yoake-token` obtained through member browser device login or a friend test-play invite code)[^launcher-mods][^launcher-login]. Test-play invitees use the exact same catalog path and package format[^api-yoake-mods].

> [!NOTE]
> As `docs/api/yoake-mods.md` notes, *"No command here publishes files, edits the upstream index, chooses trusted keys..."*[^api-yoake-mods]. The concrete portal URL and the procedure for submitting a mod to the portal/catalog maintainers are not specified in the reference sources (undecided).

### Required Metadata in `yak.toml` and `index.json`

Every published version includes its full normalized `yak.toml` inside `index.json` under `manifest`[^api-yoake-mods][^yak-catalog]:

- **Required `yak.toml` fields**[^yak-manifest]:
  - `format = 1` (unknown fields are rejected throughout)
  - `id`: `3..48` characters of lowercase ASCII `a-z`, `0-9`, `_`, or `-` (not a Windows-reserved name)
  - `version`: complete SemVer string (e.g. `"1.0.0"`)
  - `name`: display name (`1..256` bytes, no control characters)
  - `authors`: `1..128` author names (`1..256` bytes each)
  - `description`: `1..16,384` bytes
  - `[yoake] version`: SemVer requirement for Yoake (e.g. `">=0.3.0, <0.4.0"`). The `version = "*"` that `yoake new` generates is for development; narrow it to a range such as `">=0.3.0, <0.4.0"` in a `.yak` you publish
  - `[side]`: booleans `client`, `server`, `required_on_join` (installing into a launcher client instance requires `client = true`[^launcher-plan])
- **Optional `yak.toml` fields**[^yak-manifest][^wasm-dev]:
  - `name_ja` (`1..256` bytes, preferred by the launcher when the UI language is Japanese[^launcher-mods]), `license` (`1..256` bytes), `homepage` (HTTP/HTTPS URL up to `4,096` bytes with no credentials, query, or fragment)
  - `dependencies`, `optional`, `conflicts` (up to `1,024` entries each, no self-dependencies, disjoint `dependencies` and `optional`)
  - `[wasm]`: `interface` (`"1.0"`, or `"1.1"` for `inject-mod` only), `client` / `server` `.wasm` paths, `world` (`"client-mod"`, `"client-full"`, or `"inject-mod"`), and `permissions` (a subset of the seven: `"service.chat"`, `"service.commands"`, `"service.config"`, `"service.gpu"`, `"service.http"`, `"service.permissions"`, `"service.storage"`)
  - `[mixin]` (`targets`, for `inject-mod`), `[config.<key>]` (up to `256` keys), `[permissions."<node>"]` (up to `256` nodes), `[http]` (`hosts`, requires `service.http`), `[[shaders]]` (up to `128`), `[[services]]` (up to `128`)
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

## 7-5. Publishing Updates and Compatibility

### Publishing a New Version and How the Launcher Sees It

1. Bump `version` in `yak.toml` (and `content/horizon-mod.toml` if present) to a higher SemVer, update `[yoake] version` if needed, and pack the `.yak`[^api-yoake-mods][^yak-manifest].
2. After the `.yak` is signed, record its complete ZIP `size` and `sha256` in a new entry under the mod's `versions` array in `index.json`[^api-yoake-mods]. Serialize the validated `Catalog` once, sign those exact bytes with `yoake-mods-v1`, and publish them without reserializing or changing whitespace[^api-yoake-mods]. How a mod author submits an updated package to the portal is (undecided).
3. The launcher refreshes the catalog when the Mods page opens, when the user clicks `Refresh`, when credentials change, or automatically every **300 seconds (5 minutes)**[^launcher-page].
4. For each installed mod, the launcher checks all catalog versions with the same `id` where `version.version > installed.version` and `environment.check(&version.manifest).is_ok()`, picking the highest SemVer[^launcher-plan][^launcher-rows]. When found, the row shows `Update <version>`; confirming the change (`Review mod changes` → `Confirm`) downloads and verifies the new package into `yoake-mods/.cache/<sha256>.yak`, extracts `<game dir>/yoake-mods/<id>/<new-version>/`, removes `<game dir>/yoake-mods/<id>/<old-version>/`, and rewrites `installed.json` and `yoake-mods.txt` in dependency-first order[^launcher-page][^launcher-rows][^launcher-tx].

### Compatibility (`[yoake] version` and `interface`)

- **`[yoake] version`**: Checked via `check_data_compatibility` against the instance's Yoake version[^api-yoake-mods][^yak-compat]. If a Yoake update moves the instance outside a mod's SemVer requirement, the launcher flags the incompatibility and the client skips the entry at startup with a title-screen notice[^api-yoake-mods][^launcher-mods].
- **`[wasm] interface`**: Must be `"major.minor"` and is checked against `SUPPORTED_WASM_INTERFACE = (1, 1)`. The build accepts `"1.0"`, plus `"1.1"` only when `world = "inject-mod"` (a major other than `1`, or a minor greater than `1`, is rejected)[^wasm-dev][^yak-manifest]. In addition, if a host WIT world changes (for example, when `handle-call` was added to `server-full`), modules compiled before the change cannot be loaded and must be rebuilt with the updated SDK[^wasm-dev].

---

## 7-6. Things You Must NOT Do

Every prohibition below is enforced by or documented in the reference sources:

1. **Never ask players to turn on a developer-mode switch (`YOAKE_WASM_UNSIGNED=1`, `YOAKE_DLL_MODS_UNSIGNED=1`, or either launcher developer toggle)**:
   Those switches bypass signature verification strictly for local development. Wasm stays sandboxed, but the service permissions a mod declares (such as `service.commands`) still apply, and native DLL code runs unsandboxed[^api-yoake-mods][^launcher-dev]. Distributed Wasm mods must be signed with Yoake's key; otherwise players cannot run them in the game[^yak-mods-client].
2. **Never distribute unsigned `[wasm]` or `[bin]` mods to players, and never bundle native DLLs inside a Wasm mod**:
   A `.yak` cannot declare both `[bin]` and `[wasm]`, and undeclared binaries are rejected[^yak-manifest][^yak-root]. Because the game requires a valid Yoake pinned-key `SIGNATURE` for both `[wasm]` and `[bin]` packages, never distribute unsigned code mods to players[^api-yoake-mods][^yak-mods-client][^launcher-plan].
3. **Never reuse another mod's package ID (`id` in `yak.toml`)**:
   The catalog, `installed.json`, `yoake-mods.txt`, and `yak::resolve` require unique IDs, and installing a mod replaces any installed mod with the same `id`[^api-yoake-mods][^yak-catalog][^launcher-tx]. Config (`<game>/config/<id>.toml`), storage (`yoake-data/<id>/...`), and permission nodes (`[mods.<id>]`) are all keyed by `id`[^wasm-dev][^wasm-storage].
4. **Never put secrets, tokens, passwords, or private keys in a `.yak`, `yak.toml`, or catalog**:
   `yak.toml` rejects secret-like keys in `[[services]]` and credentials in URLs, but this is not a general-purpose detector of secrets hidden in ordinary strings; never place secrets or signing private keys anywhere in a package, catalog, or repository[^api-yoake-mods][^yak-manifest].
5. **Never rely on the internal on-disk storage format, edit storage files directly, or try to access them via `std::fs`**:
   Wasm guests have no preopened filesystem directory, so every `std::fs` call fails[^wasm-dev]. Moreover, the on-disk format is internal to the host and may change—never edit the files directly; always use `storage_get` / `storage_put` / `storage_delete` and the `storage_blob_*` APIs (WIT `storage-get` / `storage-put` / `storage-delete` / `storage-blob-*` imports)[^wasm-dev][^wasm-storage]. The storage scopes, folders, quotas, and write budget are explained in [4. Exploring the Example Mods](./04-examples.md).
6. **Never modify a `.yak` or `index.json` after signing, or repack an extracted target view (`<game dir>/yoake-mods/<id>/<version>/`)**:
   Changing a single byte invalidates the signature and SHA-256, and the `SIGNATURE` retained in an extracted directory authenticates the original complete `.yak` archive, not the extracted subset[^api-yoake-mods].

---

## Next Steps

If you want to go further and hook into engine functions themselves, continue to [8. Getting Started with Yoake Mixin (`08-mixin.md`)](./08-mixin.md). To return to the table of contents, see the [README](./README.md).

[^api-yoake-mods]: Source: `docs/api/yoake-mods.md`
[^wasm-dev]: Source: `docs/yoake-wasm-dev.md`
[^wasm-storage]: Source: `crates/yoake-wasm/src/host_services/storage.rs`
[^yoake-cli]: Source: [`02-first-mod.md`](./02-first-mod.md), [`03-play.md`](./03-play.md), and `crates/yoake-cli/src/main.rs` (`yoake pack`)
[^yak-root]: Source: `crates/horizon-mods/src/yak.rs`
[^yak-manifest]: Source: `crates/horizon-mods/src/yak/manifest.rs`
[^yak-catalog]: Source: `crates/horizon-mods/src/yak/catalog.rs`
[^yak-compat]: Source: `crates/horizon-mods/src/yak/compatibility.rs`
[^yak-installed]: Source: `crates/horizon-mods/src/yak/installed.rs`
[^yak-pack]: Source: `crates/horizon-mods/src/yak/pack.rs`
[^yak-mods-client]: Source: `crates/horizon-client/src/yak_mods.rs` (`UnsignedSwitches`, `wasm_mods_from`, `wasm_preflight`)
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
