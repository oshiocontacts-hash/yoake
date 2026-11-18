# MX3U1: the launcher passes the approved full-access list (backend only, no UI)

Branch `yoake2/slice` on top of 2d9b00366 (MX3L4), working tree. No git writes, no installs, no game or launcher GUI started. Edits only inside `horizon-integ`: `launcher/` plus this report. `crates/` untouched.

## Progress checklist

- [x] 1 launch wiring: GUI Yoake launch (`main.rs`) and CLI `play` (`cli.rs`) call `full_access::prepare`, pass `--full-access-approved <abs path>`, delete the run folder on exit, remove stale run folders at startup
- [x] 1b last accepted list kept per instance in the control folder; `pending_changes` = `last.diff(current)`
- [x] 2 local install: `plan_local` detects `world = "full-access"`, routes through `Control::install_local` into `<instance>/mods/` (origin `local`), not approved
- [x] 3 catalog: a full-access catalog version is downloaded, checked against the signed catalog, installed with `install_package(.., Origin::Catalog, signer)`, not approved
- [x] 4 API module `launcher/src/full_access.rs` (plain types, no UI)
- [x] 5 tests (11 new, in `launcher/src/yoake_mods/plan/tests/full_access.rs`), full launcher suite green
- [x] both reports

## What changed

| File | Change |
|---|---|
| `launcher/src/full_access.rs` (new, `pub mod` in `lib.rs`) | the API below, scanning, last-accepted list, run folders, install helpers |
| `launcher/src/yoake_mods/plan.rs` | `InstallPlan.full_access: Vec<Manifest>` (pub) + private `full_chosen`; `split_full_access` takes full-access candidates (local file or catalog version) out of the sandboxed resolver; they skip `InstanceEnvironment::check` (no `side.client` needed) and use `check_compatibility` instead |
| `launcher/src/yoake_mods/plan/transaction.rs` | `apply_for` runs the old sandboxed commit only when the plan has sandboxed changes (or no full-access part), then `install_full_access` for each full-access candidate (local: hash re-checked, `full_access::install_local_in` = `Control::install_local`; catalog: `http.open`, size + sha256 check, `Archive::from_catalog`, signer hex recorded, `install_catalog_in`) |
| `launcher/src/main.rs` | field `client_full_access: Option<RunFolder>`; Yoake launches call `prepare` after `limit_memory`, before `wrap`; cleared on exit and on spawn failure; `remove_stale_runs()` at startup next to the social handoff sweep |
| `launcher/src/cli.rs` | `play` for a Yoake program: `prepare`, `apply`, dropped right after the game exits; a problem is printed to stderr |
| tests | fixture `full_access()` manifest + wasm stub file (`bin/wasm/server.wasm`); `tests/full_access.rs` |

Not wired on purpose: Java instances (`java_game`, the cli java path, main `JavaGame`) refuse to launch when the instance is Yoake, and mcp `play` starts plain Horizon clients (no Yoake program). If one of them ever starts a Yoake program it must call `prepare` too.

## The launch rule (pending changes)

Per instance the launcher keeps the **last accepted list** in `<launcher data>/yoake-full-access/last-accepted/<hash of instance path>.toml` (same format as the game's list). A launch passes **last accepted intersected with current**: an entry of the list that `Control::approved_list` builds now (`id`, `version`, `sha256`, `origin` all equal) that is also in the last accepted list. Consequences:

- a mod whose bytes changed is dropped (its old entry has no file any more), never upgraded silently;
- an approval or file that appeared behind the player's back (control files rewritten by a mod, with or without a matching checksum, or a file put into `mods/`) is not passed until `accept_changes` after the player confirmed the diff;
- the player's own explicit acts count as confirmation of that one id: `approve` records that entry as accepted, `revoke` and `remove` take it out; nothing else changes the accepted list;
- a control file that fails the corruption check (or does not parse) means **no list and no argument**: nothing is full-access for that launch; the reason is in `Prepared::problem` and `last_launch_problem`. `pending_changes` still shows the diff built from the files as they are on disk (unchecked parse), so the player sees what changed; an unparseable file cannot be accepted;
- no last-accepted file (or an unreadable one) means an empty list: nothing passed.

The game receives the argument on every Yoake launch (even with an empty list) so that it always registers the protected control paths.

## Run folders

`<launcher data>/run/<pid>-<nanos>-<n>/approved.toml` plus `owner.lock`, an exclusively locked file the launcher holds while the run folder lives. Dropping `RunFolder` (game exit) deletes the folder. `remove_stale_runs` at startup removes folders whose lock nobody holds (crashed launcher) and folders with no lock file older than a minute; a folder of another live launcher is left alone. A game that outlives a crashed launcher before its first world start would find its file gone, which only means no full-access mods.

## Final API (`horizon_launcher::full_access`)

```rust
pub const RUN_SUBDIR: &str = "run";

pub enum RowState { Approved, NotApproved, ContentChanged, DuplicateId, Refused(String) }

pub struct FullAccessRow {
    pub id: String, pub name: String, pub version: String,
    pub sha256_short: String,      // 12 hex digits
    pub sha256: String,            // full; what approve takes (a prefix of >= 8 digits works too)
    pub origin: String,            // "local" | "catalog" | "unknown"
    pub signer: Option<String>,    // catalog: hex of the signing key
    pub file: String,              // file name in mods/
    pub approved: bool,            // state == Approved
    pub state: RowState,
    pub passed_at_launch: bool,    // approved AND in the last accepted list; approved && !passed = waiting for accept_changes
}

pub fn mods_dir(instance: &Path) -> PathBuf;                       // <instance>/mods
pub fn list(instance: &Path) -> Vec<FullAccessRow>;
pub fn approve(instance: &Path, id: &str, sha256: &str) -> Result<(), String>;
pub fn revoke(instance: &Path, id: &str) -> Result<(), String>;
pub fn remove(instance: &Path, id: &str) -> Result<(), String>;    // deletes the file(s), forgets origin + approval
pub fn pending_changes(instance: &Path) -> Option<ListDiff>;       // last accepted vs what the next launch would pass
pub fn control_problem(instance: &Path) -> Option<String>;         // a control file is damaged / unreadable
pub fn accept_changes(instance: &Path) -> Result<(), String>;      // accept_current_files + record the list; only after the player confirmed
pub fn accept_changes_shown(instance: &Path, shown: &ListDiff) -> Result<(), String>; // refuses if pending != shown
pub fn install_local(instance: &Path, source: &Path) -> Result<String, String>;       // direct install (the plan path does the same); returns sha256
pub fn is_full_access(manifest: &Manifest) -> bool;

pub struct RunFolder { /* Drop deletes the folder */ }  // .approved_path(), .dir(), .finish()
pub struct Prepared { pub run: Option<RunFolder>, pub problem: Option<String>, pub pending: Option<ListDiff>, pub passed: usize }
impl Prepared { pub fn apply(&self, cmd: &mut Command); pub fn args(&self) -> Vec<OsString>; }
pub fn prepare(instance: &Path) -> Prepared;                       // call before spawning the Yoake game
pub fn last_launch_problem(instance: &Path) -> Option<String>;     // what the last prepare() reported
pub fn remove_stale_runs() -> usize;                               // at startup
```

Every function has an `_in(data: &Path, ...)` twin (`list_in`, `approve_in`, `prepare_in`, `accept_changes_in(data, instance, shown: Option<&ListDiff>)`, `install_local_in(data, instance, source, expected_sha)`, `install_catalog_in`, `remove_stale_runs_in`, ...) that takes the launcher data folder explicitly; the short ones use `backend::data_dir()`. `ListDiff { added, removed, changed }` and `ApprovedEntry` are `horizon_mods::yak::control` types (`id, version, sha256, origin`).

Plan API (for the install pages): `InstallPlan` has a new `pub full_access: Vec<Manifest>` (the part of `changes` that will be copied into `mods/`, origin local or catalog, **not approved**). `Client::plan_local`, `plan_install`, `plan_set` and `apply` keep their signatures. Installing never approves: show the row, then call `approve`.

Deviations from the task text: `pending_changes` returns `Option<ListDiff>` as asked, but a damaged control file is reported by the separate `control_problem` / `Prepared::problem`; `approve` accepts a hash prefix (at least 8 digits); installing a different version of an installed full-access id is refused (remove the old one first); `remove` removes all files with that id.

## Tests

`cd launcher && cargo test --offline -j 4`: lib **595 passed**, 0 failed, 6 ignored; bin 170 passed, 1 ignored; integration: hidden_processes 6, logs 1, owner_gate 2, desktop 0, startup 0 (1 ignored); doctests 0. 11 of the lib tests are new (`yoake_mods::plan::tests::full_access::*`):

1. local full-access install lands in `mods/` byte for byte, origin `local`, not approved, not in the sandboxed store, empty list at launch, another version refused;
2. approve: launch args contain `--full-access-approved <abs path>`, the file has the entry; revoke and remove take it out;
3. changed bytes: dropped from the launch, shown in `pending_changes` (removed), row refused, `accept_changes` acknowledges, the new bytes are still not approved;
4. an update needs approving again (`ContentChanged` when the id is approved for other bytes);
5. control files changed behind the back: an approval written with an honest checksum is not passed until accepted (with the shown-diff check); a hand edit without checksum gives no list, a problem text, all rows refused, the diff shown; accepting restores; an unparseable file cannot be accepted;
6. a stale shown diff is refused by `accept_changes_shown`;
7. run folders: written, deleted on drop, a dead one removed at startup, a live one and a fresh unlocked one left alone;
8. catalog full-access install: origin `catalog`, signer recorded, not approved, then approve and launch;
9. a catalog download that differs from the catalog is refused, nothing in `mods/`;
10. hand-dropped (no install record) and duplicated files are refused and cannot be approved;
11. a sandboxed `.yak` still goes through the old path (state file written, nothing in `mods/`), and a full-access install next to it leaves the sandboxed store alone.


## Open

- The GUI keeps `Prepared::problem` only for `last_launch_problem`; nothing sets `client_error` for it (the UI is yours).
- Version updates of a full-access mod are remove + install + approve (no in-place replace).
- The last accepted list is keyed by the instance's canonical path; moving or renaming an instance makes its approved mods pending again (the safe direction).
- Not run: the real game with the list (no game launch by instruction); the loader side is covered by MX3L4's tests.
