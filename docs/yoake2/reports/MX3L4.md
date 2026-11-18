# MX3L4: the launcher is the authority; the game only receives approved hashes

Branch `yoake2/slice` on top of 4dce73050 (MX3L3), working tree. No git writes, no installs, no game launch. Edits only inside `horizon-integ`; **`launcher/` untouched** (the launcher wiring is listed below). Feature `yoake2-slice`; default builds unchanged. Design: `docs/yoake2/ENGINE_CALL_ABI.md`, section "The launcher is the authority (MX3L3, MX3L4)" (replaces the MX3L3 section); `YOAKE_MIXIN2.md` 2.5 updated.

## Progress checklist

- [x] 1a the game no longer reads the control files; it takes this launch's approved list `(id, version, sha256, origin)` from a launcher-written file (`--full-access-approved <path>`)
- [x] 1b the game loads only packages whose bytes hash to an entry (origin local/catalog; catalog signatures still verified); no list: nothing is full-access
- [x] 1c `--full-access-dir` removed from the game side; `control.rs` is the launcher's library
- [x] 1d the seal is now the **checksum file** (`<folder>.checksum`), documented as a corruption check, not tamper protection; `reseal` is `accept_current_files`, for the launcher after the player confirmed a shown diff
- [x] 2a `guard` is bidirectional (inside **or ancestor of** a protected path)
- [x] 2b `guard` is applied to the paths `DiskStorage` really opens (resolved base, storage directory, legacy import directories, each file written)
- [x] 2c `..` components are rejected outright (also in verbatim Windows paths, where `..` is an ordinary name)
- [x] 3a remote packages are named by their hash
- [x] 3b `write_atomic` fsyncs before the rename (best effort)
- [x] tests, docs, both reports

## 1. What the launcher must pass (to wire in `launcher/`)

Full text with the table is in `ENGINE_CALL_ABI.md`; in short:

- **Build it**: `Control::create(&control_dir_in(data_dir()), &game_root)` (once), then per launch `Control::approved_list(&mods_dir)`. It reads and corruption-checks `full-access.toml` and `yak-origins.toml` (an `Err` means: show the problem, pass **no** list) and returns an `ApprovedList` with one entry per top-level `*.yak` of `<game root>/mods` whose SHA-256 is approved under its id and whose origin record is `local` or `catalog`, plus the control folder and its checksum file as `protected` paths.
- **Write it**: `list.write_for_launch(&run_dir)` into a fresh, launch-specific folder outside the game folder (for example `<launcher data>/run/<launch id>/`); it creates `approved.toml` (never overwrites) and returns the absolute path.
- **Pass the path**: `--full-access-approved <absolute path>` on the game's command line (or `YOAKE_FULL_ACCESS_APPROVED`; the option wins). Command line chosen as the simplest robust way on Windows: no inherited handle across `CreateProcess`.
- **Format**: TOML, UTF-8, at most 1 MiB: `format = 1`, `protected = [paths]`, `[[package]]` with `id`, `version`, `sha256` (64 lowercase hex of the `.yak`), `origin` (`"local"` | `"catalog"`; anything else invalidates the whole file).
- **Lifetime**: one launch. The file must exist and stay unchanged until the game's first world start (the game reads it once and keeps it in memory); the launcher deletes it and its folder when the game exits. A mod that rewrites the control files changes nothing in the running game.
- **At the next launch**: keep the last accepted list, compute `last.diff(&new)` (`ListDiff { added, removed, changed }`) and show it. If a control file fails the corruption check (changed behind the launcher's back), show what changed and call `Control::accept_current_files()` **only after the player confirmed**.
- **Install and approve** (unchanged from MX3L3): `Control::install_local(mods_dir, source)`, `Control::approve(bytes)`, `install_package(.., Origin::Catalog, signer)`.

## 2. Game side

`yoake2_mods::process_approved_list()` reads the file named by the option/variable **once** (at the first world start, before any mod runs), registers the file and each `protected` path with `guard`, and caches the list. `load_world_mods(mods_dir, Option<&ApprovedList>)` turns it into the allow/origin pair and uses the existing `load_dir`, so signature checks, duplicate-id refusal and every other rule are unchanged. The game never opens the control folder.

## 3. `guard` fixes

- (a) refuses a path inside a protected path **or an ancestor of one** (the same rule `Control::open` applies to the game root).
- (b) `DiskStorage` canonicalizes its trusted base (the OS resolves links and `..` there, correctly, links first) and then guards the **resolved** base, the storage directory, the legacy directories it imports from, and every file it writes (`atomic_write`). Note: the pre-existing test `trusted_base_accepts_links_and_parent_components` requires the engine's own base to be allowed to hold a `..`, so the `..` rejection applies to every path derived from it, not to the raw trusted base (which is resolved by the OS first).
- (c) `..` in any guarded path is refused outright, including in verbatim `\\?\` paths where Rust treats it as a plain name; `Control::open` refuses it too, so nothing is resolved lexically before links.

## 4. Nits

- Remote packages are stored as `<mods>/remote/<origin>/<sha256>.yak` (the sender's name is ignored): a file pre-placed under the sender's name cannot block a delivery, the same package delivered twice is fine, a file of the hash name with other bytes is refused.
- `write_atomic`: write, `sync_all` (best effort), rename.

## Tests

| Suite | Result |
|---|---|
| horizon-mods `--features yak` (incl. 7 `yak::control` unit tests) | 73 passed (lib; incl. 7 control tests) + 40 integration, all passed |
| yoake-slice (`--features wasm`) | unit 25, `multi` 6, `policy` 6, `slice` 21, doctest 1, all passed |
| yoake-wasm storage tests (incl. the ancestor / `..` test) | 25 passed |
| horizon-client `yoake2_loader` | 16 passed (was 15) |
| lib, feature on: sandboxed-path skip + real `Game` (2, ignored; `HORIZON_TEST_RESOURCES=C:\Users\USER\NestgateHorizon\horizon\.local\horizon-client\26.3`), adapted to the approved-list input | 3 passed (real Game two-mod test and single-mod test, 17 s) |
| horizon-client-lib `yoake2_engine_call` / `yoake2_glue` / `yoake2_slice` | 8 / 5 / 7 passed (+1 ignored bench) |
| yoake-guest-sdk | unit 3, `guest_runtime` 7, all passed |
| feature-off `cargo test -p horizon-client --lib` | **1193 passed, 0 failed, 40 ignored** |

New or changed tests: the game loads only matching bytes (unapproved, hand-dropped and replaced-bytes packages refused, list entries of origin server/friend do not parse, loading again replaces); a tampered control file (including a forged checksum) has no effect on the running launch and shows up as a diff for the next, and a damaged one is refused by the launcher; an ancestor storage root and `..` are refused (unit tests and `DiskStorage`); remote naming by hash; list format, option/env parsing, diff, one list per launch; the checksum file's corruption check. Not re-run: the yoake-bindgen `engine` gate (no binding changed). Note: two test runs sharing the same scratch folders at the same time (two cargo runs in parallel) make the loader tests fail spuriously; run them one at a time.

## Open

- **The launcher is not wired** (nothing in `launcher/` sets `--full-access-approved`, so a launcher-started game runs no full-access mod): build and write the list, pass the path, keep and diff the last accepted list, delete the run folder on exit.
- The checksum file is a corruption check only. A keyed MAC is not worth adding while the game cannot read the folder.
- `guard` is only as strong as its callers: `DiskStorage` is the only one; the first path-taking binding must call it.
- Packages in `remote/<origin>/` are stored but nothing in this tree delivers or uses them yet.
