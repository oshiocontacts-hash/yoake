//! MX3U1: full-access packages go to `<instance>/mods/` (origin local or catalog), are approved separately, and reach
//! the game only through the launch list.
use super::fixtures::Package;
use super::*;
use crate::full_access as fa;
use horizon_mods::yak::control::{self, APPROVED_ARG};
use std::process::Command;

fn data(fixture: &Fixture) -> PathBuf {
    fixture.client.data.clone()
}

fn mods(fixture: &Fixture) -> PathBuf {
    fixture.home.join("mods")
}

fn install_local(fixture: &Fixture, id: &str, version: &str) -> (Package, String) {
    let package = fixture.package(fixture.full_access(id, version), false);
    let plan = fixture.local(&package.path).unwrap();
    assert_eq!(plan.full_access.len(), 1);
    fixture.apply(plan, true).unwrap();
    let sha = horizon_release_sign::sha256_hex(&package.bytes);
    (package, sha)
}

fn control_of(fixture: &Fixture) -> control::Control {
    control::Control::create(&control::control_dir_in(&data(fixture)), &fixture.home).unwrap()
}

fn command_args(prepared: &fa::Prepared) -> Vec<String> {
    let mut cmd = Command::new("game");
    prepared.apply(&mut cmd);
    cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
}

fn listed(prepared: &fa::Prepared) -> Vec<String> {
    let args = command_args(prepared);
    assert_eq!(args.len(), 2, "{args:?}");
    assert_eq!(args[0], APPROVED_ARG);
    let list = control::ApprovedList::read(Path::new(&args[1])).unwrap();
    list.entries().iter().map(|e| e.id.clone()).collect()
}

#[test]
fn a_local_full_access_package_lands_in_mods_with_origin_local_and_is_not_approved() {
    let fixture = Fixture::new();
    let (package, sha) = install_local(&fixture, "fa-one", "1.0.0");
    // a top-level .yak in mods/, byte for byte the chosen file
    let name = package.path.file_name().unwrap();
    assert_eq!(std::fs::read(mods(&fixture).join(name)).unwrap(), package.bytes);
    // not extracted into the sandboxed store, not listed there
    assert!(!fixture.home.join("yoake-mods.txt").exists());
    assert!(fixture.client.installed(&fixture.home).unwrap().is_empty());
    // origin recorded as local, approval absent
    let ctl = control_of(&fixture);
    assert_eq!(ctl.origins().unwrap().origin_of(&sha), Some(control::Origin::Local));
    assert!(ctl.allow().unwrap().entries().is_empty());
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].id.as_str(), rows[0].origin.as_str()), ("fa-one", "local"));
    assert_eq!(rows[0].state, fa::RowState::NotApproved);
    assert!(!rows[0].approved && !rows[0].passed_at_launch);
    assert_eq!(rows[0].sha256_short, sha[..12]);
    // installing is not approving: a launch passes an empty list
    let prepared = fa::prepare_in(&data(&fixture), &fixture.home);
    assert!(prepared.problem.is_none());
    assert!(listed(&prepared).is_empty());
    // a second install under another version is refused until the first is removed
    let other = fixture.package(fixture.full_access("fa-one", "1.1.0"), false);
    assert!(fixture.apply(fixture.local(&other.path).unwrap(), true).is_err());
}

#[test]
fn approving_puts_the_entry_into_the_launch_list_and_the_argument() {
    let fixture = Fixture::new();
    let (_, sha) = install_local(&fixture, "fa-two", "1.0.0");
    assert!(fa::approve_in(&data(&fixture), &fixture.home, "fa-two", "abc").is_err(), "a short prefix");
    assert!(fa::approve_in(&data(&fixture), &fixture.home, "fa-two", &"0".repeat(64)).is_err(), "other bytes");
    fa::approve_in(&data(&fixture), &fixture.home, "fa-two", &sha[..16]).unwrap();
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    assert_eq!(rows[0].state, fa::RowState::Approved);
    assert!(rows[0].approved && rows[0].passed_at_launch);
    let prepared = fa::prepare_in(&data(&fixture), &fixture.home);
    assert_eq!(prepared.passed, 1);
    assert!(prepared.pending.is_none());
    let args = command_args(&prepared);
    assert_eq!(args[0], "--full-access-approved");
    assert!(Path::new(&args[1]).is_absolute());
    let text = std::fs::read_to_string(&args[1]).unwrap();
    assert!(text.contains(&sha) && text.contains("fa-two") && text.contains("origin = \"local\""), "{text}");
    assert_eq!(fa::pending_changes_in(&data(&fixture), &fixture.home), None);
    drop(prepared);
    // revoking takes it out again
    fa::revoke_in(&data(&fixture), &fixture.home, "fa-two").unwrap();
    assert_eq!(fa::list_in(&data(&fixture), &fixture.home)[0].state, fa::RowState::NotApproved);
    assert!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)).is_empty());
    // removing deletes the file and forgets it
    fa::approve_in(&data(&fixture), &fixture.home, "fa-two", &sha).unwrap();
    fa::remove_in(&data(&fixture), &fixture.home, "fa-two").unwrap();
    assert!(fa::list_in(&data(&fixture), &fixture.home).is_empty());
    assert_eq!(std::fs::read_dir(mods(&fixture)).unwrap().count(), 0);
    assert!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)).is_empty());
}

#[test]
fn a_changed_file_is_dropped_and_shown_and_never_upgraded_silently() {
    let fixture = Fixture::new();
    let (package, sha) = install_local(&fixture, "fa-three", "1.0.0");
    fa::approve_in(&data(&fixture), &fixture.home, "fa-three", &sha).unwrap();
    assert_eq!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)), ["fa-three"]);
    // the bytes change behind the player's back (same file name, other package)
    let other = fixture.package(fixture.full_access("fa-three", "1.0.1"), false);
    std::fs::write(mods(&fixture).join(package.path.file_name().unwrap()), &other.bytes).unwrap();
    let prepared = fa::prepare_in(&data(&fixture), &fixture.home);
    assert!(listed(&prepared).is_empty(), "the changed file is not passed");
    let diff = prepared.pending.clone().expect("shown as pending");
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(diff.removed[0].sha256, sha);
    assert_eq!(fa::pending_changes_in(&data(&fixture), &fixture.home), Some(diff));
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    assert!(matches!(rows[0].state, fa::RowState::Refused(_)), "{:?}", rows[0].state);
    assert!(!rows[0].approved && !rows[0].passed_at_launch);
    // accepting acknowledges the change; the new bytes are still not approved
    fa::accept_changes_in(&data(&fixture), &fixture.home, None).unwrap();
    assert_eq!(fa::pending_changes_in(&data(&fixture), &fixture.home), None);
    assert!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)).is_empty());
}

#[test]
fn an_updated_package_needs_approving_again() {
    let fixture = Fixture::new();
    let (_, sha) = install_local(&fixture, "fa-up", "1.0.0");
    fa::approve_in(&data(&fixture), &fixture.home, "fa-up", &sha).unwrap();
    fa::remove_in(&data(&fixture), &fixture.home, "fa-up").unwrap();
    let (_, sha2) = install_local(&fixture, "fa-up", "1.1.0");
    assert_ne!(sha, sha2);
    assert_eq!(fa::list_in(&data(&fixture), &fixture.home)[0].state, fa::RowState::NotApproved);
    assert!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)).is_empty());
    // an approval of the same id for other bytes shows as changed content
    let ctl = control_of(&fixture);
    let mut allow = ctl.allow().unwrap();
    allow.allow("fa-up", "1.0.0", &sha);
    ctl.save_allow(&allow).unwrap();
    assert_eq!(fa::list_in(&data(&fixture), &fixture.home)[0].state, fa::RowState::ContentChanged);
}

#[test]
fn control_files_changed_behind_the_launchers_back_are_held_back_until_accepted() {
    let fixture = Fixture::new();
    let (_, sha_a) = install_local(&fixture, "fa-a", "1.0.0");
    let (_, sha_b) = install_local(&fixture, "fa-b", "1.0.0");
    fa::approve_in(&data(&fixture), &fixture.home, "fa-a", &sha_a).unwrap();
    // 1. a mod approves fa-b through the control files and keeps the checksum honest (the launcher's API is bypassed)
    let ctl = control_of(&fixture);
    let mut allow = ctl.allow().unwrap();
    allow.allow("fa-b", "1.0.0", &sha_b);
    ctl.save_allow(&allow).unwrap();
    let prepared = fa::prepare_in(&data(&fixture), &fixture.home);
    assert_eq!(listed(&prepared), ["fa-a"], "the new approval is not passed");
    assert_eq!(prepared.pending.as_ref().unwrap().added.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["fa-b"]);
    drop(prepared);
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    let b = rows.iter().find(|r| r.id == "fa-b").unwrap();
    assert!(b.approved && !b.passed_at_launch);
    // the player confirmed the diff they were shown
    let shown = fa::pending_changes_in(&data(&fixture), &fixture.home).unwrap();
    fa::accept_changes_in(&data(&fixture), &fixture.home, Some(&shown)).unwrap();
    assert_eq!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)).len(), 2);
    assert_eq!(fa::pending_changes_in(&data(&fixture), &fixture.home), None);
    // 2. the file is edited by hand without the checksum: no list at all, the reason is reported, the diff is shown
    let path = ctl.dir().join(control::AllowFile::FILE_NAME);
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str(&format!("\n[[allow]]\nid = \"evil\"\nversion = \"1\"\nsha256 = \"{}\"\n", "c".repeat(64)));
    std::fs::write(&path, text).unwrap();
    let prepared = fa::prepare_in(&data(&fixture), &fixture.home);
    assert!(prepared.run.is_none() && command_args(&prepared).is_empty(), "nothing is passed");
    assert!(prepared.problem.as_deref().unwrap().contains("checksum"));
    assert_eq!(fa::last_launch_problem(&fixture.home), prepared.problem);
    assert!(fa::control_problem_in(&data(&fixture), &fixture.home).is_some());
    assert!(fa::list_in(&data(&fixture), &fixture.home).iter().all(|r| matches!(r.state, fa::RowState::Refused(_))));
    // accepting records the files as they are; the launch works again
    fa::accept_changes_in(&data(&fixture), &fixture.home, None).unwrap();
    assert!(fa::control_problem_in(&data(&fixture), &fixture.home).is_none());
    let prepared = fa::prepare_in(&data(&fixture), &fixture.home);
    assert!(prepared.problem.is_none());
    assert_eq!(fa::last_launch_problem(&fixture.home), None);
    // an unreadable file cannot be accepted
    std::fs::write(&path, "not [ toml").unwrap();
    assert!(fa::accept_changes_in(&data(&fixture), &fixture.home, None).is_err());
}

#[test]
fn an_accept_for_a_stale_diff_is_refused() {
    let fixture = Fixture::new();
    let (_, sha) = install_local(&fixture, "fa-stale", "1.0.0");
    let ctl = control_of(&fixture);
    let mut allow = ctl.allow().unwrap();
    allow.allow("fa-stale", "1.0.0", &sha);
    ctl.save_allow(&allow).unwrap();
    let shown = fa::pending_changes_in(&data(&fixture), &fixture.home).unwrap();
    fa::revoke_in(&data(&fixture), &fixture.home, "fa-stale").unwrap();
    assert!(fa::accept_changes_in(&data(&fixture), &fixture.home, Some(&shown)).is_err());
}

#[test]
fn run_folders_are_deleted_on_exit_and_stale_ones_at_startup() {
    let fixture = Fixture::new();
    let data = data(&fixture);
    let prepared = fa::prepare_in(&data, &fixture.home);
    let run = prepared.run.as_ref().unwrap();
    let (dir, file) = (run.dir().to_path_buf(), run.approved_path().to_path_buf());
    assert!(file.is_file() && dir.starts_with(data.join(fa::RUN_SUBDIR)));
    // a launcher that crashed left a folder behind (its lock is not held)
    let dead = data.join(fa::RUN_SUBDIR).join("999999-1-0");
    std::fs::create_dir_all(&dead).unwrap();
    std::fs::write(dead.join("approved.toml"), "format = 1\n").unwrap();
    std::fs::write(dead.join("owner.lock"), "").unwrap();
    // a fresh folder that has no lock yet is being created: left alone
    let unlocked = data.join(fa::RUN_SUBDIR).join("999998-1-0");
    std::fs::create_dir_all(&unlocked).unwrap();
    assert_eq!(fa::remove_stale_runs_in(&data), 1, "the dead one only");
    assert!(!dead.exists() && dir.exists() && unlocked.exists());
    std::fs::remove_dir_all(&unlocked).unwrap();
    // the game exits: the run folder goes
    drop(prepared);
    assert!(!dir.exists() && !file.exists());
    assert_eq!(fa::remove_stale_runs_in(&data), 0);
}

#[test]
fn a_catalog_full_access_package_installs_with_origin_catalog_and_its_signer() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.full_access("fa-cat", "2.0.0"), true);
    let catalog = fixture.publish(&[package.version.clone()]);
    let plan = fixture.plan(&catalog, "fa-cat").unwrap();
    assert_eq!(plan.full_access.len(), 1);
    assert_eq!(plan.changes.len(), 1);
    fixture.apply(plan, true).unwrap();
    assert_eq!(std::fs::read(mods(&fixture).join("fa-cat-2.0.0.yak")).unwrap(), package.bytes);
    assert!(!fixture.home.join("yoake-mods.txt").exists());
    let sha = horizon_release_sign::sha256_hex(&package.bytes);
    let origins = control_of(&fixture).origins().unwrap();
    let record = origins.get(&sha).unwrap();
    assert_eq!(record.origin, control::Origin::Catalog);
    assert!(record.signer.as_deref().is_some_and(|s| s.len() == 64));
    // still not approved
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    assert_eq!((rows[0].origin.as_str(), &rows[0].state), ("catalog", &fa::RowState::NotApproved));
    fa::approve_in(&data(&fixture), &fixture.home, "fa-cat", &sha).unwrap();
    assert_eq!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)), ["fa-cat"]);
}

#[test]
fn a_catalog_download_that_differs_from_the_catalog_is_refused() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.full_access("fa-bad", "1.0.0"), true);
    let catalog = fixture.publish(&[package.version.clone()]);
    fixture.http.respond(&package.version.url, Ok(b"not the package".to_vec()));
    let plan = fixture.plan(&catalog, "fa-bad").unwrap();
    assert!(fixture.apply(plan, true).is_err());
    assert!(!mods(&fixture).exists() || std::fs::read_dir(mods(&fixture)).unwrap().count() == 0);
}

#[test]
fn a_hand_dropped_or_duplicated_file_is_refused_and_never_approvable() {
    let fixture = Fixture::new();
    let (package, sha) = install_local(&fixture, "fa-dup", "1.0.0");
    // a copy under another name: same id, same bytes
    std::fs::copy(mods(&fixture).join(package.path.file_name().unwrap()), mods(&fixture).join("copy.yak")).unwrap();
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.state == fa::RowState::DuplicateId), "{rows:?}");
    assert!(fa::approve_in(&data(&fixture), &fixture.home, "fa-dup", &sha).is_err());
    std::fs::remove_file(mods(&fixture).join("copy.yak")).unwrap();
    // another package dropped in by hand has no install record
    let hand = fixture.package(fixture.full_access("fa-hand", "1.0.0"), false);
    std::fs::copy(&hand.path, mods(&fixture).join("hand.yak")).unwrap();
    let hand_sha = horizon_release_sign::sha256_hex(&hand.bytes);
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    let row = rows.iter().find(|r| r.id == "fa-hand").unwrap();
    assert!(matches!(&row.state, fa::RowState::Refused(why) if why.contains("install record")), "{:?}", row.state);
    assert_eq!(row.origin, "unknown");
    assert!(fa::approve_in(&data(&fixture), &fixture.home, "fa-hand", &hand_sha).is_err());
}

#[test]
fn a_sandboxed_yak_still_goes_through_the_old_path() {
    let fixture = Fixture::new();
    let package = fixture.package(fixture.manifest("plain-one", "1.0.0"), false);
    let plan = fixture.local(&package.path).unwrap();
    assert!(plan.full_access.is_empty());
    assert_eq!(plan.changes.len(), 1);
    fixture.apply(plan, true).unwrap();
    assert_eq!(fixture.list(), "plain-one\t1.0.0\n");
    assert_eq!(fixture.client.installed(&fixture.home).unwrap().len(), 1);
    // nothing in mods/, nothing for the full-access list
    assert!(!mods(&fixture).exists() || std::fs::read_dir(mods(&fixture)).unwrap().count() == 0);
    assert!(fa::list_in(&data(&fixture), &fixture.home).is_empty());
    // and a full-access install next to it leaves the sandboxed store as it was
    install_local(&fixture, "fa-next", "1.0.0");
    assert_eq!(fixture.list(), "plain-one\t1.0.0\n");
    assert_eq!(fixture.client.installed(&fixture.home).unwrap().len(), 1);
}

#[test]
fn removing_one_file_of_a_duplicate_keeps_the_other_and_its_record() {
    let fixture = Fixture::new();
    let (package, sha) = install_local(&fixture, "fa-two-copies", "1.0.0");
    let original = package.path.file_name().unwrap().to_string_lossy().into_owned();
    std::fs::copy(mods(&fixture).join(&original), mods(&fixture).join("copy.yak")).unwrap();
    assert!(fa::remove_file_in(&data(&fixture), &fixture.home, "missing.yak").is_err());
    fa::remove_file_in(&data(&fixture), &fixture.home, "copy.yak").unwrap();
    assert!(!mods(&fixture).join("copy.yak").exists());
    assert!(mods(&fixture).join(&original).exists(), "the other copy stays");
    // same bytes: the install record is still there, so it can be approved and launched
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].state, fa::RowState::NotApproved);
    fa::approve_in(&data(&fixture), &fixture.home, "fa-two-copies", &sha).unwrap();
    assert_eq!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)), ["fa-two-copies"]);
    // removing the last file withdraws the approval and the launch entry
    fa::remove_file_in(&data(&fixture), &fixture.home, &original).unwrap();
    assert!(fa::list_in(&data(&fixture), &fixture.home).is_empty());
    assert!(control_of(&fixture).allow().unwrap().entries().is_empty());
    assert!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)).is_empty());
}

#[test]
fn a_lost_checksum_with_no_mod_changes_can_be_accepted_from_an_empty_diff() {
    let fixture = Fixture::new();
    let (_, sha) = install_local(&fixture, "fa-seal", "1.0.0");
    fa::approve_in(&data(&fixture), &fixture.home, "fa-seal", &sha).unwrap();
    let ctl = control_of(&fixture);
    std::fs::remove_file(ctl.checksum_path()).unwrap();
    assert!(fa::control_problem_in(&data(&fixture), &fixture.home).is_some());
    // nothing changed in the list itself: the page shows an empty diff and the accept button
    let shown = fa::pending_changes_in(&data(&fixture), &fixture.home).unwrap_or_default();
    assert!(shown.is_empty());
    fa::accept_changes_in(&data(&fixture), &fixture.home, Some(&shown)).unwrap();
    assert!(fa::control_problem_in(&data(&fixture), &fixture.home).is_none());
    assert_eq!(listed(&fa::prepare_in(&data(&fixture), &fixture.home)), ["fa-seal"]);
}
