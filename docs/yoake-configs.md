# Yoake mod catalog and configurations (構成) with share codes

Launcher: `launcher/src/yoake_configs.rs` (logic, tested), `launcher/src/yoake_configs_page.rs`
(the section at the bottom of the Yoake page). Publisher: `tools/yoake/catalog/mods.toml`,
`tools/yoake/yoake_catalog.py`.

## Catalog kinds

| Kind | Where it comes from | What enabling it means |
|---|---|---|
| `builtin` | signed `yoake/mods/builtin/signed.json` | The mod is compiled into a Yoake game build. It runs whenever that build runs, so "enabling" = starting from a base pack whose build contains it (`native_mods` of the base, the same list a pack's `enabled_native_mods` must equal). |
| `.yak` | signed `yoake/mods/index.json` (the existing catalog) | Installed into the instance by the launcher (`yoake_mods::install_set`). |
| WASM | reserved | Not produced yet (branch `yoake/wasm-w1`). The badge exists. |

When the same id is in both lists, the `.yak` entry wins (`yoake_configs::merge`), so a built-in mod
can move to a `.yak` later without changing any configuration or code.

### `builtin/signed.json` (signed with the `yoake-builtin-v1` domain)

It is a separate document from the `.yak` catalog on purpose: the `.yak` catalog parser is strict
(`deny_unknown_fields`), so launchers that only know it never read this one.

The file is an envelope with the exact catalog text and its signature, so publishing it is one
rename and a launcher can never see a catalog next to another catalog's signature:

```json
{"json": "<the catalog text below, as a string>", "signature": "<base64 Ed25519>"}
```

The signature covers `horizon release yoake-builtin-v1\n` followed by the exact bytes of `json`.
That domain is its own: a `.yak` catalog (`yoake-mods-v1`), index (`yoake-index-v1`) or `.yak`
signature never verifies as a built-in catalog, nor the reverse.

```json
{
  "schema": "1.0",
  "kind": "yoake-builtin-catalog",
  "generated": "2026-11-14T00:00:00Z",
  "mods": [{
    "id": "yasagashi",
    "version": "0.1.0",
    "author": "Yoake",
    "name": {"en": "Yasagashi Werewolf", "ja": "家探し人狼"},
    "description": {"en": "...", "ja": "..."},
    "requires": ["nestgate"],
    "requires_engine": ["0.2.0-jinro.1"]
  }]
}
```

* `schema` is `major.minor` (digits only): unknown fields and a higher minor are accepted, a higher
  major shows "needs a newer Yoake".
* `critical_features` (optional list): any entry makes a launcher that reads this show "needs a
  newer Yoake" instead of reading the catalog. **Rule: a new field that changes how a catalog must
  be treated (a security-relevant one) must also be named in `critical_features`**, because older
  launchers ignore fields they do not know and would otherwise act as if it were not there.
* `generated` (`YYYY-MM-DDTHH:MM:SSZ`, required) only moves forward. A launcher keeps the last
  verified catalog and refuses one with an older `generated` (rollback), showing a message and
  keeping the cached one; `make_catalog` refuses to write one older than the staged one.
* `requires_engine` lists the base versions (index `bases[].version`) whose build contains the mod
  (letters, digits and `.-+_`, at most 64 characters each); the launcher checks the base's
  `native_mods` list.
* A problem with the built-in catalog (bad signature, newer major, rollback, offline) keeps the
  last verified copy on the page and never hides the `.yak` list.

Built by `python tools/yoake/yoake_pack.py catalog --staging <dir> [--key <private key> --public-key <pub>]`
from `tools/yoake/catalog/mods.toml`; `publish` uploads `signed.json` (one rename) before
`current.json`. Signing needs a `horizon-release-sign` that knows `--tag yoake-builtin-v1` (or the
`cryptography` package).

## Configuration

```
name      1..48 characters, no control characters
base      pack id of the base pack (a Yoake build/engine to start from)
mods      [{id, version}]  version is "latest" or an exact catalog version; at most 64
settings  {mod id: {key: value}}   only keys a mod declares shareable; none do yet, so it is empty
```

Names also refuse invisible characters (Unicode format characters such as zero-width and bidi
controls, line and paragraph separators, blank fillers).

Saved locally in `yoake-configs.json` in the launcher data folder (max 64). Unknown fields (a newer
launcher may have saved more) are ignored when reading and kept when saving; entries this launcher
cannot use stay in the file. A file that cannot be parsed is copied to
`yoake-configs.json.<time>.<n>.bak` before anything is saved over it (the save is refused if the copy
fails).

## Share code, version 1 (`YK1-`)

```
YK1-  base32( deflate( compact JSON ) || checksum )  in groups of 5 separated by "-"
```

* compact JSON: `{"v":1,"n":<name>,"b":<base>,"m":[[<id>,<version>],...],"s":{...}}` (`s` only when
  non-empty). `v` is the code version; unknown fields are refused.
* deflate: raw DEFLATE (RFC 1951), best compression.
* checksum: the first 4 bytes of SHA-256 of the deflate stream, appended after it.
* base32: Crockford alphabet `0123456789ABCDEFGHJKMNPQRSTVWXYZ`, no padding. On input the
  launcher ignores case, spaces, line breaks and `-`, and reads `O` as `0` and `I`/`L` as `1`;
  non-zero padding bits are refused, so a code has one canonical spelling.
* limits (the paste box takes 16 KiB, a code is at most about 8 KiB): at most 4 KiB of decoded JSON (the inflater stops at 4 KiB + 1), at most 64 mods, the
  encoded body is refused beyond 8 KiB before any decoding.
* content: ids match `[a-z0-9_-]{3,48}`, versions are `latest` or semver, the base matches
  `[a-z0-9_.-]{1,48}`. A code never contains a URL or a file path; anything else is refused.

Importing never trusts the code: `resolve` matches every id and version against the signed
catalog and reports each problem (unknown mod, unknown version, unknown base, built-in mod missing
from the base's build, settings that no mod allows, a dependency range that the chosen version does
not satisfy). Versions the configuration names are kept; requirements (`requires`, or a `.yak`'s
`dependencies`, of the chosen version) are added automatically as the newest version that satisfies
the range, and marked as required. Only a configuration without
problems can be installed: the launcher creates a new instance from the base pack, then installs
the `.yak` mods into it.

### `YK2-` (reserved)

A future short code (8 characters) for which the portal stores the configuration. The launcher
refuses `YK2-` codes with a clear message; nothing of the server part exists.

## 要確認

* Built-in mods cannot be switched per instance: they are part of the base build. A configuration
  therefore picks the base pack; a built-in mod that the chosen build lacks is an error. If the
  game should later load built-in mods selectively, the base/pack rules (`enabled_native_mods` must
  equal the base's `native_mods`) and the game need a switch first.
* The yak catalog is empty today; `.yak` entries in configurations are tested with fixtures only.
