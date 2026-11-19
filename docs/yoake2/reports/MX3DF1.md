# MX3DF1: launcher backend for the dev folder (hot reload of full-access mods)

Branch `yoake2/dev-folder`, from bc0cd95f2. Backend only: no UI, the launcher GUI and the game were never started.

## Checklist

- [x] `Origin::Dev` in `crates/horizon-mods/src/yak/control.rs` ("dev"; `can_be_full_access()` is true for Local, Catalog, Dev; round trip; old files still read)
- [x] `generation` on `ApprovedList` (optional top-level integer, absent = 0, not written while 0; `generation()`, `with_generation()`). **Not on vps/yoake2/hot-reload** (no such branch on the vps remote when I looked), so I added it as specified; keep the merge trivial (see 要確認 1)
- [x] Setting `full_access_dev_folder: Option<PathBuf>` in `Settings` (`launcher/src/backend.rs`), absent from the file while unset
- [x] Gate: active only while `dev_unsigned_wasm` ("Test your own WASM mods") is on: `active_dev_folder(&Settings)`
- [x] `--full-access-dev` (`DEV_ARG`) added by `Prepared::apply()` / `Prepared::args()` when the dev folder is active at launch; nothing otherwise (no flag, no watcher)
- [x] Worker thread, polls every 1 s, std only (no new dependency), stops with the run folder
- [x] Stability: a file (new, changed or gone) must look the same for 500 ms before it is taken
- [x] Install through `control::install_package` with origin `Dev`; replaces an older dev copy of the same id (or same file name); refuses the same id from any other origin (never overwritten)
- [x] Approves exactly those bytes and records them as accepted for the instance (`Control::approve` + `last_with`)
- [x] Removed dev file: dev copy removed, origin record and approval withdrawn (never a non-dev copy)
- [x] Run's approved list rewritten in place, atomically (`control::write_atomic`: temp file then rename), `generation` + 1; the content is what a launch would pass now (accepted entries that still match the files)
- [x] Safety: folder validation (`validate_dev_folder`), control lock held across every control-file write (install, approve, remove, rewrite), nothing from another source approved
- [x] `FullAccessRow.origin == "dev"` (comes from the origin record; state Approved like normal)
- [x] API: `dev_folder_status`, `set_dev_folder`
- [x] Tests (below), full launcher suite and horizon-mods --features yak green
- [x] Committed and pushed to `vps yoake2/dev-folder`

## Files

- `crates/horizon-mods/src/yak/control.rs`: `Origin::Dev`, `ApprovedList.generation`, tests.
- `crates/horizon-client/src/yoake2_mods.rs`: **one match arm** (`decide`): the new enum variant made it non-exhaustive. `Local | Dev` run when approved, `Dev` otherwise `NotAllowed`. This is the game side's file; see 要確認 2.
- `launcher/src/backend.rs`: the setting.
- `launcher/src/full_access.rs`: `mod dev;` + re-exports, `RunFolder.dev` (watcher, stopped before the run folder is deleted), `Prepared.dev`, `prepare_with(data, instance, dev_folder)`, `remove_file_unlocked` (the body of `remove_file_in`, so the watcher can call it while holding the lock).
- `launcher/src/full_access/dev.rs` (new): everything else.
- `launcher/src/yoake_mods/plan/tests/full_access_dev.rs` (new) + one `mod` line in `tests.rs`.

## API (`horizon_launcher::full_access`)

```rust
pub const DEV_ARG: &str = "--full-access-dev";

pub struct DevStatus { pub folder: PathBuf, pub watching: bool, pub last_event: Option<String>,
                       pub last_error: Option<String>, pub generation: u64 }

/// None when the dev folder is not active (developer mode off or no folder chosen).
pub fn dev_folder_status(instance: &Path) -> Option<DevStatus>;

/// Some(path): validates and stores the canonical path in settings.json; None clears it. Err = nothing changed.
pub fn set_dev_folder(path: Option<PathBuf>) -> Result<(), String>;

// supporting
pub fn active_dev_folder(settings: &Settings) -> Option<PathBuf>;           // dev_unsigned_wasm && folder set
pub fn validate_dev_folder(path: &Path, data: &Path, homes: &[PathBuf]) -> Result<PathBuf, String>;
pub fn status_for(instance: &Path, folder: Option<PathBuf>) -> Option<DevStatus>;   // testable form
pub fn prepare_with(data: &Path, instance: &Path, dev_folder: Option<&Path>) -> Prepared;
pub struct DevSync { .. }   // the engine: new(data, instance, folder, approved), tick(now: Instant); the thread runs it
```

`prepare(instance)` loads the settings itself and calls `prepare_with`; the call sites in `main.rs` and `cli.rs` need no change. `Prepared` has a new `dev: bool`.

Status values: `last_event` examples: "watching <folder>", "installed foo 1.0.0 from the dev folder (generation 2)", "updated ...", "removed foo (foo.yak) with the dev folder's file", "ignored x.yak: not a full-access package". `last_error` examples: "foo.yak: foo is already installed from origin local (...): the dev folder never overwrites it; remove that one first", "x.yak: not a valid .yak: ...". `last_error` is cleared by the next successful event. `status.watching` is true from the launch until the run folder is dropped. After a launch whose folder was refused (`validate_dev_folder` for that instance), `watching` is false and `last_error` says why. `generation` is the run list's generation (0 before the first rewrite).

Because the status is keyed by the instance path and filled by the watcher, a page can call `dev_folder_status(home)` on its normal refresh.

## Behaviour details

- **What is watched**: top-level `*.yak` of the chosen folder (no subfolders). Also the dev copies already in `<instance>/mods/` (origin record `dev`), so a dev copy whose source file is gone is removed when the next run starts looking. A file present at the start counts as new (installed/approved if not yet; a no-op if already installed with the same bytes).
- **Replace**: a dev copy with the same id or the same file name is removed (file, origin record, approval) and the new one installed, approved, accepted. The same id from local, catalog or an unknown origin: refused with `last_error`, nothing changed (also nothing when that file is later removed from the dev folder).
- **Not full-access**: ignored (event only); if a dev copy by that name existed it is removed with the file. An unreadable file (still locked) is retried every second; a broken package is reported once and tried again only when the file changes.
- **The list rewrite** reads the old run list's `generation` and writes generation + 1, only when something changed (an unchanged file causes no rewrite). Other pending, not yet accepted changes stay held back (the same rule as a launch). The game side polling the file is not part of this task.
- **Folder validation** (`set_dev_folder` and again per instance at launch): absolute, no `..`, an existing folder, no `mods` component anywhere in the path, not inside / equal to / containing the launcher data folder, not inside any instance's `home` or `work` (at launch: this instance's home and its mods folder).
- Locking: `apply` takes `CONTROL_LOCK` for the whole install/approve/remove/rewrite step.

## Tests

`launcher/src/yoake_mods/plan/tests/full_access_dev.rs` (7 tests, `DevSync::tick` with a fake clock unless noted):

1. `a_dev_change_installs_approves_and_rewrites_the_list_with_the_next_generation`: install (origin dev, approved, accepted, no pending diff, row `dev`/Approved/passed), generation 1, no temp file, idempotent look, replacement by new bytes (generation 2, old record and approval gone), same id under another file name.
2. `a_removed_dev_file_removes_its_copy_and_approval`: removal needs the file gone for 500 ms, generation + 1, control files emptied; a leftover dev copy is removed at the next run.
3. `the_same_id_from_the_local_origin_is_refused_and_never_overwritten`: local file, origin and approval untouched, no generation bump, error reported, later removal of the dev file does not touch the local mod.
4. `a_file_still_being_written_is_not_taken_and_a_non_full_access_package_is_ignored`: half file, restart of the clock on change, taken after 500 ms; non-full-access, broken zip and `.txt` ignored.
5. `forbidden_folder_choices_are_refused`: instance, its mods folder and subfolders, data folder and subfolder, data folder's parent, `Mods` anywhere, `..`, relative, a file, a missing folder; a launch with a forbidden folder gets no flag and no watcher.
6. `without_a_dev_folder_there_is_no_flag_and_no_watcher`: no flag/no thread without a folder; setting inactive while developer mode is off; absent from the settings file; with a folder the flag follows the approved arguments and the watcher stops with the run folder.
7. `the_worker_thread_picks_a_file_up_by_itself_and_stops_with_the_run`: real thread, real clock.

`crates/horizon-mods/src/yak/control.rs`: `the_dev_origin_round_trips_can_be_full_access_and_old_files_still_read` (Origin parse/as_str/can_be_full_access, list with dev + generation round trip, generation 0 not written and absent reads as 0, negative or non-integer generation refused, origin file with dev record, older origin file).

Results: `cd launcher && cargo test -j 4` (lib 604 passed, bin 170 passed, integration tests all ok, 0 failed); `cargo test -j 4 -p horizon-mods --features yak` (74 + 41 passed; one run showed a flaky `config_writes_retry_a_brief_in_process_lock` under load, unrelated, green on rerun); `cargo check -p horizon-client` clean.

## 要確認 (not guessed)

1. **`generation` on the other agent's branch.** `vps/yoake2/hot-reload` did not exist on the remote when I checked, so the field is mine: `generation: u64` on `ApprovedList` (private field, `generation()` / `with_generation()`), written as `generation = N` only when N > 0, parsed as an optional non-negative integer. If the other agent's version differs (type, always written, name), the merge touches `ApprovedList` in control.rs: `new()`, `to_text()`, `parse()`, the struct literal in `approved_list()`.
2. **`horizon-client` `decide`**: I added `Origin::Dev` as "like Local when approved". The game side may want a different rule (for example refuse `Dev` unless `--full-access-dev` was passed). One-line change in `decide` and in the origin match in the loader (`Some(origin) => origin` already lets it through).
3. **Which switch is "developer mode".** There is no single developer-mode flag; the section is revealed by clicks and the options are `dev_offline`, `dev_unsigned_dll`, `dev_unsigned_wasm`. I gated on `dev_unsigned_wasm` ("Test your own WASM mods"), as the dev folder holds WASM mods. If the owner wants another switch, change `active_dev_folder`.
4. **`set_dev_folder` and settings in memory.** It loads `settings.json`, changes only the folder and saves. The UI keeps its own `Settings` copy: reload it after the call or the next UI save overwrites the folder.
5. **`--full-access-dev` is added only when a run list exists.** If `prepare` has a problem (control file failed its check, no list) nothing is passed, no flag, no watcher.
6. **Rename over a file the game has open.** `write_atomic` renames over `approved.toml`. Rust opens files with delete sharing, so a reader that also uses std is fine on Windows; a reader that opens it without `FILE_SHARE_DELETE` makes the rename fail (the error shows as `last_error`, retried every second). Not tested against the real game.
7. **Mods page labels.** `origin_label` / `entry_origin` in `yoake_mods_page/full_access_rows.rs` show "Unknown origin" for `dev`; I did not touch the UI (yours). Row text for the new origin ("dev folder") and the dev status line are for the UI.
8. **Replace is remove-then-install.** If the install fails after the old dev copy was removed (disk full), the old copy stays gone; the next change retries. A temp-name install would need a new `install_package` mode.
9. **A dev file whose id equals a hand-dropped, record-less file** in `mods/` is refused as "unknown origin" like any other non-dev origin.
10. Only top-level files of the dev folder are watched; a build tool that writes to a subfolder is not picked up. Easy to widen if wanted.

## Update: merged vps/yoake2/hot-reload (MX3HR1)

- Took the game side's `ApprovedList.generation` (identical API, 3 trivial conflicts in control.rs resolved to theirs) and its `control::DEV_ARG`; the launcher re-exports it (`full_access::DEV_ARG`). 要確認 1 is resolved.
- Rewrites of the run list are at least 1.1 s apart (the game checks mtime+size once a second); the watcher waits before taking the lock.
- After the merge: launcher `cargo test -j 4` lib 604, bin 170, integration ok; horizon-mods --features yak 74+41 ok; `cargo check -p horizon-client` clean.
