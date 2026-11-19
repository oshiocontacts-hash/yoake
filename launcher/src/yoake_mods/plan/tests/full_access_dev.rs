//! MX3DF1: the developer folder. A dev change is installed (origin dev), approved and written into the run's approved
//! list with generation + 1; a removed file takes its copy away; nothing from another origin is ever overwritten.
use super::fixtures::Package;
use super::*;
use crate::full_access::{self as fa, DevSync, DEV_ARG};
use horizon_mods::yak::control::{self, ApprovedList, Origin};
use std::process::Command;
use std::time::{Duration, Instant};

fn data(fixture: &Fixture) -> PathBuf {
    fixture.client.data.clone()
}

fn mods(fixture: &Fixture) -> PathBuf {
    fixture.home.join("mods")
}

fn dev_folder(fixture: &Fixture) -> PathBuf {
    fixture.home.parent().unwrap().join("dev-folder")
}

fn status(fixture: &Fixture) -> fa::DevStatus {
    fa::status_for(&fixture.home, Some(dev_folder(fixture))).unwrap()
}

/// A dev folder beside the instance, and a run (what a launch with `prepare_in` made) with an engine we drive by hand.
struct Run {
    folder: PathBuf,
    prepared: fa::Prepared,
    sync: DevSync,
    now: Instant,
}

impl Run {
    fn new(fixture: &Fixture) -> Self {
        let folder = dev_folder(fixture);
        std::fs::create_dir_all(&folder).unwrap();
        let prepared = fa::prepare_in(&data(fixture), &fixture.home);
        let approved = prepared.run.as_ref().unwrap().approved_path().to_path_buf();
        let sync = DevSync::new(&data(fixture), &fixture.home, &folder, &approved);
        Self { folder, prepared, sync, now: Instant::now() }
    }

    fn approved(&self) -> PathBuf {
        self.prepared.run.as_ref().unwrap().approved_path().to_path_buf()
    }

    fn list(&self) -> ApprovedList {
        ApprovedList::read(&self.approved()).unwrap()
    }

    /// A look now, and another one after the file was quiet for longer than the stability time.
    fn settle(&mut self) {
        self.sync.tick(self.now);
        self.now += Duration::from_millis(700);
        self.sync.tick(self.now);
    }

    fn put(&self, name: &str, package: &Package) {
        std::fs::write(self.folder.join(name), &package.bytes).unwrap();
    }
}

fn sha(package: &Package) -> String {
    horizon_release_sign::sha256_hex(&package.bytes)
}

fn control_of(fixture: &Fixture) -> control::Control {
    control::Control::create(&control::control_dir_in(&data(fixture)), &fixture.home).unwrap()
}

fn args(prepared: &fa::Prepared) -> Vec<String> {
    prepared.args().iter().map(|a| a.to_string_lossy().into_owned()).collect()
}

#[test]
fn a_dev_change_installs_approves_and_rewrites_the_list_with_the_next_generation() {
    let fixture = Fixture::new();
    let mut run = Run::new(&fixture);
    assert_eq!(run.list().generation(), 0);
    let one = fixture.package(fixture.full_access("dev-mod", "1.0.0"), false);
    run.put("dev-mod.yak", &one);
    run.settle();
    // installed top-level with origin dev, approved, accepted
    assert_eq!(std::fs::read(mods(&fixture).join("dev-mod.yak")).unwrap(), one.bytes);
    let ctl = control_of(&fixture);
    assert_eq!(ctl.origins().unwrap().origin_of(&sha(&one)), Some(Origin::Dev));
    assert!(ctl.allow().unwrap().is_allowed("dev-mod", &sha(&one)));
    let rows = fa::list_in(&data(&fixture), &fixture.home);
    assert_eq!((rows[0].origin.as_str(), rows[0].state.clone(), rows[0].passed_at_launch), ("dev", fa::RowState::Approved, true));
    assert_eq!(fa::pending_changes_in(&data(&fixture), &fixture.home), None);
    // the run's list: the entry, origin dev, generation 1
    let list = run.list();
    assert_eq!(list.generation(), 1);
    assert_eq!(list.entries().len(), 1);
    assert_eq!((list.entries()[0].id.as_str(), list.entries()[0].origin, list.entries()[0].sha256.as_str()), ("dev-mod", Origin::Dev, sha(&one).as_str()));
    let state = status(&fixture);
    assert_eq!(state.generation, 1);
    assert!(state.last_event.unwrap().contains("installed dev-mod 1.0.0"));
    // no temp file is left next to the list
    let names: Vec<_> = std::fs::read_dir(run.approved().parent().unwrap()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    assert_eq!(names.iter().filter(|n| n.contains("tmp")).count(), 0, "{names:?}");
    // an unchanged look changes nothing
    run.settle();
    assert_eq!(run.list().generation(), 1);
    // the file is replaced by a newer version (same name): the old dev copy goes, the new one is approved
    let two = fixture.package(fixture.full_access("dev-mod", "1.1.0"), false);
    run.put("dev-mod.yak", &two);
    run.settle();
    let list = run.list();
    assert_eq!(list.generation(), 2);
    assert_eq!(list.entries().len(), 1);
    assert_eq!(list.entries()[0].sha256, sha(&two));
    assert_eq!(std::fs::read_dir(mods(&fixture)).unwrap().flatten().filter(|e| e.path().extension().is_some_and(|x| x == "yak")).count(), 1);
    let ctl = control_of(&fixture);
    assert!(ctl.origins().unwrap().origin_of(&sha(&one)).is_none(), "the old record is forgotten");
    assert!(!ctl.allow().unwrap().is_allowed("dev-mod", &sha(&one)) && ctl.allow().unwrap().is_allowed("dev-mod", &sha(&two)));
    // a copy under another file name with the same id replaces it too
    let three = fixture.package(fixture.full_access("dev-mod", "1.2.0"), false);
    run.put("renamed.yak", &three);
    std::fs::remove_file(run.folder.join("dev-mod.yak")).unwrap();
    run.settle();
    let list = run.list();
    assert_eq!(list.entries().len(), 1);
    assert_eq!(list.entries()[0].sha256, sha(&three));
    assert!(mods(&fixture).join("renamed.yak").exists() && !mods(&fixture).join("dev-mod.yak").exists());
}

#[test]
fn a_removed_dev_file_removes_its_copy_and_approval() {
    let fixture = Fixture::new();
    let mut run = Run::new(&fixture);
    let one = fixture.package(fixture.full_access("gone-mod", "1.0.0"), false);
    run.put("gone.yak", &one);
    run.settle();
    assert_eq!(run.list().entries().len(), 1);
    let generation = run.list().generation();
    std::fs::remove_file(run.folder.join("gone.yak")).unwrap();
    // not at once: a file that is gone must stay gone for the stability time
    run.sync.tick(run.now);
    assert_eq!(run.list().entries().len(), 1, "still there before the file has been gone long enough");
    run.settle();
    assert!(!mods(&fixture).join("gone.yak").exists());
    assert!(run.list().entries().is_empty());
    assert_eq!(run.list().generation(), generation + 1);
    let ctl = control_of(&fixture);
    assert!(ctl.allow().unwrap().entries().is_empty());
    assert!(ctl.origins().unwrap().is_empty());
    assert!(fa::list_in(&data(&fixture), &fixture.home).is_empty());
    // a dev copy left from an earlier run whose file is gone is removed when the next run starts looking
    run.put("again.yak", &one);
    run.settle();
    assert!(mods(&fixture).join("again.yak").exists());
    drop(run);
    std::fs::remove_file(dev_folder(&fixture).join("again.yak")).unwrap();
    let mut next = Run::new(&fixture);
    next.settle();
    assert!(!mods(&fixture).join("again.yak").exists());
    assert!(next.list().entries().is_empty());
}

#[test]
fn the_same_id_from_the_local_origin_is_refused_and_never_overwritten() {
    let fixture = Fixture::new();
    let local = fixture.package(fixture.full_access("shared-id", "1.0.0"), false);
    fixture.apply(fixture.local(&local.path).unwrap(), true).unwrap();
    let local_name = local.path.file_name().unwrap().to_owned();
    fa::approve_in(&data(&fixture), &fixture.home, "shared-id", &sha(&local)).unwrap();
    let mut run = Run::new(&fixture);
    assert_eq!(run.list().entries().len(), 1, "the local mod is in the run's list");
    let dev = fixture.package(fixture.full_access("shared-id", "9.9.9"), false);
    run.put("dev.yak", &dev);
    run.settle();
    // nothing changed: the local file and its approval stay, no dev copy, no generation bump
    assert_eq!(std::fs::read(mods(&fixture).join(&local_name)).unwrap(), local.bytes);
    assert!(!mods(&fixture).join("dev.yak").exists());
    let ctl = control_of(&fixture);
    assert_eq!(ctl.origins().unwrap().origin_of(&sha(&local)), Some(Origin::Local));
    assert!(ctl.origins().unwrap().origin_of(&sha(&dev)).is_none());
    assert!(ctl.allow().unwrap().is_allowed("shared-id", &sha(&local)) && !ctl.allow().unwrap().is_allowed("shared-id", &sha(&dev)));
    assert_eq!(run.list().generation(), 0);
    assert_eq!(run.list().entries()[0].origin, Origin::Local);
    let error = status(&fixture).last_error.expect("reported");
    assert!(error.contains("already installed from origin local"), "{error}");
    // removing the dev file later never touches the local mod
    std::fs::remove_file(run.folder.join("dev.yak")).unwrap();
    run.settle();
    assert!(mods(&fixture).join(&local_name).exists());
    assert_eq!(run.list().entries().len(), 1);
}

#[test]
fn a_file_still_being_written_is_not_taken_and_a_non_full_access_package_is_ignored() {
    let fixture = Fixture::new();
    let mut run = Run::new(&fixture);
    let one = fixture.package(fixture.full_access("slow-mod", "1.0.0"), false);
    // half of a file: not taken; the file changes, the clock restarts
    std::fs::write(run.folder.join("slow.yak"), &one.bytes[..one.bytes.len() / 2]).unwrap();
    run.sync.tick(run.now);
    run.now += Duration::from_millis(300);
    run.sync.tick(run.now);
    assert!(!mods(&fixture).join("slow.yak").exists(), "300 ms is not stable");
    run.put("slow.yak", &one);
    run.now += Duration::from_millis(300);
    run.sync.tick(run.now);
    run.now += Duration::from_millis(300);
    run.sync.tick(run.now);
    assert!(!mods(&fixture).join("slow.yak").exists(), "the file changed 300 ms ago");
    assert_eq!(run.list().generation(), 0);
    run.now += Duration::from_millis(300);
    run.sync.tick(run.now);
    assert!(mods(&fixture).join("slow.yak").exists(), "unchanged for 600 ms: taken");
    assert_eq!(run.list().generation(), 1);
    // a package that is not full-access is not taken, a broken one is reported, neither is installed
    let plain = fixture.package(fixture.manifest("plain", "1.0.0"), false);
    run.put("plain.yak", &plain);
    std::fs::write(run.folder.join("broken.yak"), b"not a zip").unwrap();
    std::fs::write(run.folder.join("notes.txt"), b"ignored").unwrap();
    run.settle();
    assert!(!mods(&fixture).join("plain.yak").exists() && !mods(&fixture).join("broken.yak").exists());
    assert_eq!(run.list().generation(), 1);
}

#[test]
fn forbidden_folder_choices_are_refused() {
    let fixture = Fixture::new();
    let base = fixture.home.parent().unwrap().to_path_buf();
    let data_dir = data(&fixture);
    std::fs::create_dir_all(&data_dir).unwrap();
    let homes = vec![fixture.home.clone()];
    let ok = base.join("my-builds");
    std::fs::create_dir_all(&ok).unwrap();
    assert!(fa::validate_dev_folder(&ok, &data_dir, &homes).is_ok());
    // the instance, its mods folder, a subfolder of either, the data folder, a subfolder of it, its parent
    std::fs::create_dir_all(mods(&fixture).join("sub")).unwrap();
    std::fs::create_dir_all(fixture.home.join("saves")).unwrap();
    std::fs::create_dir_all(data_dir.join("run")).unwrap();
    for bad in [fixture.home.clone(), mods(&fixture), mods(&fixture).join("sub"), fixture.home.join("saves"), data_dir.clone(), data_dir.join("run"), base.clone()] {
        assert!(fa::validate_dev_folder(&bad, &data_dir, &homes).is_err(), "{bad:?}");
    }
    // a mods folder anywhere, `..`, a relative path, a file, a missing folder
    let elsewhere_mods = base.join("elsewhere").join("Mods");
    std::fs::create_dir_all(&elsewhere_mods).unwrap();
    assert!(fa::validate_dev_folder(&elsewhere_mods, &data_dir, &homes).is_err());
    // (built as text: `PathBuf::join` resolves a `..` on a verbatim path)
    let dotdot = PathBuf::from(format!("{}/../my-builds", ok.parent().unwrap().to_string_lossy().trim_start_matches(r"\?\")));
    assert!(fa::validate_dev_folder(&dotdot, &data_dir, &homes).is_err(), "{dotdot:?}");
    assert!(fa::validate_dev_folder(Path::new("relative"), &data_dir, &homes).is_err());
    std::fs::write(base.join("a-file"), b"x").unwrap();
    assert!(fa::validate_dev_folder(&base.join("a-file"), &data_dir, &homes).is_err());
    assert!(fa::validate_dev_folder(&base.join("missing"), &data_dir, &homes).is_err());
    // a launch with a forbidden folder gets no flag and no watcher, and the status says why
    let prepared = fa::prepare_with(&data_dir, &fixture.home, Some(&mods(&fixture).join("sub")));
    assert!(!prepared.dev);
    assert!(!args(&prepared).iter().any(|a| a == DEV_ARG));
    let state = status(&fixture);
    assert!(!state.watching && state.last_error.is_some());
}

#[test]
fn without_a_dev_folder_there_is_no_flag_and_no_watcher() {
    let fixture = Fixture::new();
    let prepared = fa::prepare_in(&data(&fixture), &fixture.home);
    assert!(!prepared.dev && prepared.run.is_some());
    let list = args(&prepared);
    assert_eq!(list.len(), 2, "{list:?}");
    assert!(!list.iter().any(|a| a == DEV_ARG));
    let mut cmd = Command::new("game");
    prepared.apply(&mut cmd);
    assert_eq!(cmd.get_args().count(), 2);
    drop(prepared);
    // the setting is active only with developer mode on
    let folder = dev_folder(&fixture);
    let mut settings = crate::backend::Settings { full_access_dev_folder: Some(folder.clone()), ..Default::default() };
    assert_eq!(fa::active_dev_folder(&settings), None, "developer mode off");
    settings.dev_unsigned_wasm = true;
    assert_eq!(fa::active_dev_folder(&settings), Some(folder.clone()));
    settings.full_access_dev_folder = None;
    assert_eq!(fa::active_dev_folder(&settings), None);
    assert!(fa::status_for(&fixture.home, None).is_none());
    // the setting is absent from the file while unset, and an older file still reads
    assert!(!serde_json::to_string(&crate::backend::Settings::default()).unwrap().contains("full_access_dev_folder"));
    assert_eq!(crate::backend::Settings::parse("{}").full_access_dev_folder, None);
    // with a folder: the flag follows the approved arguments, and the watcher stops with the run
    std::fs::create_dir_all(&folder).unwrap();
    let prepared = fa::prepare_with(&data(&fixture), &fixture.home, Some(&folder));
    assert!(prepared.dev);
    let list = args(&prepared);
    assert_eq!((list.len(), list[0].as_str(), list[2].as_str()), (3, "--full-access-approved", DEV_ARG));
    assert!(status(&fixture).watching);
    let run_dir = prepared.run.as_ref().unwrap().dir().to_path_buf();
    drop(prepared);
    assert!(!run_dir.exists());
    assert!(!status(&fixture).watching);
}

#[test]
fn the_worker_thread_picks_a_file_up_by_itself_and_stops_with_the_run() {
    let fixture = Fixture::new();
    let folder = dev_folder(&fixture);
    std::fs::create_dir_all(&folder).unwrap();
    let prepared = fa::prepare_with(&data(&fixture), &fixture.home, Some(&folder));
    assert!(prepared.dev);
    let approved = prepared.run.as_ref().unwrap().approved_path().to_path_buf();
    let package = fixture.package(fixture.full_access("thread-mod", "1.0.0"), false);
    std::fs::write(folder.join("thread.yak"), &package.bytes).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while ApprovedList::read(&approved).unwrap().generation() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let list = ApprovedList::read(&approved).unwrap();
    assert_eq!(list.generation(), 1);
    assert_eq!(list.entries()[0].id, "thread-mod");
    let state = status(&fixture);
    assert!(state.watching && state.generation == 1 && state.last_error.is_none(), "{state:?}");
    drop(prepared);
    assert!(!status(&fixture).watching);
}
