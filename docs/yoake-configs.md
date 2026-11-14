# Yoake mod catalog and configurations (構成) with share codes

Launcher: `launcher/src/yoake_configs.rs` (logic, tested), `launcher/src/yoake_configs_page.rs`
(the section at the bottom of the Yoake page). Publisher: `tools/yoake/catalog/mods.toml`,
`tools/yoake/yoake_catalog.py`.

## Catalog kinds

| Kind | Where it comes from | What enabling it means |
|---|---|---|
| `builtin` | signed `yoake/mods/builtin/index.json` | The mod is compiled into a Yoake game build. It runs whenever that build runs, so "enabling" = starting from a base pack whose build contains it (`native_mods` of the base, the same list a pack's `enabled_native_mods` must equal). |
| `.yak` | signed `yoake/mods/index.json` (the existing catalog) | Installed into the instance by the launcher (`yoake_mods::install_set`). |
| WASM | reserved | Not produced yet (branch `yoake/wasm-w1`). The badge exists. |

When the same id is in both lists, the `.yak` entry wins (`yoake_configs::merge`), so a built-in mod
can move to a `.yak` later without changing any configuration or code.

### `builtin/index.json` (signed with the `yoake-mods-v1` domain, `index.json.sig` beside it)

It is a separate document from the `.yak` catalog on purpose: the `.yak` catalog parser is strict
(`deny_unknown_fields`), so launchers that only know it never read this one.

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

`schema` is `major.minor`: unknown fields and a higher minor are accepted, a higher major shows
"needs a newer Yoake". `requires_engine` lists the base versions (index `bases[].version`) whose
build contains the mod; the launcher checks the base's `native_mods` list. Built by
`python tools/yoake/yoake_pack.py catalog --staging <dir> [--key <private key> --public-key <pub>]`
from `tools/yoake/catalog/mods.toml`; `publish` uploads it before `current.json`.

## Configuration

```
name      1..48 characters, no control characters
base      pack id of the base pack (a Yoake build/engine to start from)
mods      [{id, version}]  version is "latest" or an exact catalog version; at most 64
settings  {mod id: {key: value}}   only keys a mod declares shareable; none do yet, so it is empty
```

Saved locally in `yoake-configs.json` in the launcher data folder (max 64).

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
* limits: at most 4 KiB of decoded JSON (the inflater stops at 4 KiB + 1), at most 64 mods, the
  encoded body is refused beyond 8 KiB before any decoding.
* content: ids match `[a-z0-9_-]{3,48}`, versions are `latest` or semver, the base matches
  `[a-z0-9_.-]{1,48}`. A code never contains a URL or a file path; anything else is refused.

Importing never trusts the code: `resolve` matches every id and version against the signed
catalog and reports each problem (unknown mod, unknown version, unknown base, built-in mod missing
from the base's build, settings that no mod allows). Requirements (`requires`, or a `.yak`'s
`dependencies`) are added automatically and marked as required. Only a configuration without
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
