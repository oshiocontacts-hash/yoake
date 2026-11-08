use super::*;
use serde_json::{Value, json};
use std::fs;

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let root = fs::canonicalize(std::env::temp_dir()).unwrap();
        let path = root.join(format!("yoake-install-test-{}", transaction_id().unwrap()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct DirectorySource {
    root: PathBuf,
    calls: Mutex<Vec<String>>,
}

impl DirectorySource {
    fn new(root: &Path, objects: &[&[u8]]) -> Self {
        fs::create_dir_all(root).unwrap();
        for bytes in objects {
            fs::write(root.join(hash(bytes)), bytes).unwrap();
        }
        Self {
            root: root.to_owned(),
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl ObjectSource for DirectorySource {
    fn open(&self, sha256: &str) -> Result<Box<dyn Read + Send>, InstallError> {
        self.calls.lock().unwrap().push(sha256.to_owned());
        Ok(Box::new(File::open(self.root.join(sha256))?))
    }
}

struct LoginRequiredSource;

impl ObjectSource for LoginRequiredSource {
    fn open(&self, _sha256: &str) -> Result<Box<dyn Read + Send>, InstallError> {
        Err(InstallError::NeedsLogin)
    }
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn entry(destination: &str, policy: &str, bytes: &[u8]) -> Value {
    json!({
        "scope": "instance",
        "policy": policy,
        "destination": destination,
        "sha256": hash(bytes),
        "size": bytes.len(),
    })
}

fn feed(version: &str, files: Vec<Value>) -> VerifiedFeed {
    let mut index: Value =
        serde_json::from_slice(include_bytes!("../tests/fixtures/yoake-schema2/index.json"))
            .unwrap();
    let base = &mut index["bases"][0]["targets"]["windows-x86_64"]["files"][0];
    base["sha256"] = json!(hash(b"base"));
    base["size"] = json!(4);
    index["packs"][0]["versions"][0]["version"] = json!(version);
    index["packs"][0]["versions"][0]["targets"]["windows-x86_64"]["files"] = json!(files);
    VerifiedFeed {
        index: ParsedFeedIndex::V2(serde_json::from_value(index).unwrap()),
        index_sha256: "0".repeat(64),
        proof: None,
    }
}

fn plan(
    feed: &VerifiedFeed,
    version: &str,
    snapshot: &InstallSnapshot,
) -> Result<InstallPlan, InstallError> {
    let root = std::env::temp_dir();
    plan_install_with_snapshot(
        feed,
        "example",
        version,
        &root.join("instance"),
        &root.join("data"),
        "windows-x86_64",
        "1.0.0",
        snapshot,
    )
}

fn install(
    feed: VerifiedFeed,
    version: &str,
    instance: &Path,
    data: &Path,
    source: Arc<dyn ObjectSource>,
) -> Result<InstalledPack, InstallError> {
    let task = start_install_with_source(
        feed,
        "example".to_owned(),
        version.to_owned(),
        instance.to_owned(),
        data.to_owned(),
        Ok("windows-x86_64".to_owned()),
        "1.0.0".to_owned(),
        source,
    );
    loop {
        if let InstallEvent::Finished(result) = task.events.recv().unwrap() {
            return result;
        }
    }
}

#[test]
fn plans_fresh_files_and_policies() {
    let index = feed(
        "1",
        vec![
            entry("horizon-mods/old.hmod", "managed", b"old mod"),
            entry("config/test.toml", "default-on-create", b"default"),
        ],
    );
    let plan = plan(&index, "1", &InstallSnapshot::default()).unwrap();
    assert_eq!(plan.base_version, "1");
    assert_eq!(plan.sequence, 42);
    assert_eq!(plan.files.len(), 3);
    assert_eq!(plan.files[0].scope, FileScope::Base);
    assert_eq!(plan.files[0].action, FileAction::Install);
    assert_eq!(plan.files[1].policy, InstallPolicy::Managed);
    assert_eq!(plan.files[2].policy, InstallPolicy::DefaultOnCreate);
    assert!(
        plan.files
            .iter()
            .all(|file| file.action == FileAction::Install)
    );
    assert!(plan.removals.is_empty());
}

#[test]
fn plans_skips_defaults_and_obsolete_managed_files() {
    let index = feed(
        "2",
        vec![
            entry("horizon-mods/new.hmod", "managed", b"new mod"),
            entry("config/test.toml", "default-on-create", b"new default"),
        ],
    );
    let mut snapshot = InstallSnapshot::default();
    snapshot.previous = Some(InstalledPack {
        pack_id: "example".to_owned(),
        pack_version: "1".to_owned(),
        base_version: "1".to_owned(),
        sequence: 41,
        target: "windows-x86_64".to_owned(),
        managed_files: vec![ManagedFile {
            destination: "horizon-mods/old.hmod".to_owned(),
            sha256: hash(b"old mod"),
        }],
        transaction: "a".repeat(32),
        rollback_index: None,
        rollback_proof: None,
    });
    snapshot
        .base_files
        .insert("horizon-client.exe".to_owned(), hash(b"base"));
    snapshot
        .instance_files
        .insert("horizon-mods/old.hmod".to_owned(), hash(b"old mod"));
    snapshot
        .instance_files
        .insert("config/test.toml".to_owned(), hash(b"user edit"));
    let result = plan(&index, "2", &snapshot).unwrap();
    assert_eq!(result.files[0].action, FileAction::Skip);
    assert_eq!(result.files[1].action, FileAction::Install);
    assert_eq!(result.files[2].action, FileAction::PreserveDefault);
    assert_eq!(
        result.removals,
        vec![Removal {
            destination: "horizon-mods/old.hmod".to_owned(),
            sha256: hash(b"old mod"),
        }]
    );

    snapshot
        .instance_files
        .insert("horizon-mods/old.hmod".to_owned(), hash(b"modified"));
    let modified = plan(&index, "2", &snapshot).unwrap();
    assert_eq!(modified.removals[0].sha256, hash(b"modified"));

    let replacement = feed(
        "2",
        vec![entry("horizon-mods/old.hmod", "managed", b"replacement")],
    );
    let modified = plan(&replacement, "2", &snapshot).unwrap();
    assert_eq!(modified.files[1].action, FileAction::Install);
}

#[test]
fn matches_base_versions_case_insensitively() {
    let mut index = feed("1", Vec::new());
    let ParsedFeedIndex::V2(ref mut schema) = index.index else {
        panic!("expected schema 2");
    };
    schema.bases[0].version = "BaseV1".to_owned();
    schema.packs[0].versions[0].base_version = "basev1".to_owned();
    let result = plan(&index, "1", &InstallSnapshot::default()).unwrap();
    assert_eq!(result.base_version, "BaseV1");
}

#[test]
fn rejects_untrusted_destinations_missing_targets_and_old_launcher() {
    let bad = feed("1", vec![entry("../escape.toml", "managed", b"bad")]);
    assert_eq!(
        plan(&bad, "1", &InstallSnapshot::default()).unwrap_err(),
        InstallError::InvalidFeed
    );
    let index = feed("1", Vec::new());
    let root = std::env::temp_dir();
    let missing = plan_install_with_snapshot(
        &index,
        "example",
        "1",
        &root.join("i"),
        &root.join("d"),
        "linux-x86_64",
        "1.0.0",
        &InstallSnapshot::default(),
    );
    assert_eq!(missing.unwrap_err(), InstallError::UnsupportedTarget);
    let old = plan_install_with_snapshot(
        &index,
        "example",
        "1",
        &root.join("i"),
        &root.join("d"),
        "windows-x86_64",
        "0.9.0",
        &InstallSnapshot::default(),
    );
    assert_eq!(old.unwrap_err(), InstallError::LauncherTooOld);
}

#[test]
fn installs_upgrades_preserves_defaults_and_reuses_shared_cache() {
    let temp = TestDirectory::new();
    let source = Arc::new(DirectorySource::new(
        &temp.0.join("source"),
        &[b"base", b"old mod", b"new mod", b"default", b"new default"],
    ));
    let data = temp.0.join("data");
    let instance = temp.0.join("instance");
    let first = feed(
        "1",
        vec![
            entry("horizon-mods/old.hmod", "managed", b"old mod"),
            entry("config/test.toml", "default-on-create", b"default"),
        ],
    );
    install(first, "1", &instance, &data, source.clone()).unwrap();
    assert_eq!(
        fs::read(data.join("yoake-bases/1/horizon-client.exe")).unwrap(),
        b"base"
    );
    assert_eq!(
        fs::read(instance.join("horizon-mods/old.hmod")).unwrap(),
        b"old mod"
    );
    fs::write(instance.join("config/test.toml"), b"user edit").unwrap();
    let second = feed(
        "2",
        vec![
            entry("horizon-mods/new.hmod", "managed", b"new mod"),
            entry("config/test.toml", "default-on-create", b"new default"),
        ],
    );
    let manifest = install(second.clone(), "2", &instance, &data, source.clone()).unwrap();
    assert_eq!(manifest.pack_version, "2");
    assert_eq!(manifest.base_version, "1");
    assert_eq!(manifest.managed_files.len(), 1);
    assert!(!instance.join("horizon-mods/old.hmod").exists());
    assert_eq!(
        fs::read(instance.join("horizon-mods/new.hmod")).unwrap(),
        b"new mod"
    );
    assert_eq!(
        fs::read(instance.join("config/test.toml")).unwrap(),
        b"user edit"
    );
    assert_eq!(read_manifest(&instance).unwrap(), Some(manifest));
    let before = source.calls.lock().unwrap().len();
    install(
        second,
        "2",
        &temp.0.join("other-instance"),
        &data,
        source.clone(),
    )
    .unwrap();
    let calls = source.calls.lock().unwrap();
    assert_eq!(calls.len(), before + 1);
    assert_eq!(calls.last().unwrap(), &hash(b"new default"));
}

#[test]
fn rollback_restores_cached_objects_without_opening_source() {
    let temp = TestDirectory::new();
    let source = Arc::new(DirectorySource::new(
        &temp.0.join("source"),
        &[b"base", b"old", b"new"],
    ));
    let data = temp.0.join("data");
    let instance = temp.0.join("instance");
    let first = feed(
        "1",
        vec![entry("horizon-mods/test.hmod", "managed", b"old")],
    );
    install(first.clone(), "1", &instance, &data, source.clone()).unwrap();
    let second = feed(
        "2",
        vec![entry("horizon-mods/test.hmod", "managed", b"new")],
    );
    install(second.clone(), "2", &instance, &data, source.clone()).unwrap();
    install(second, "2", &instance, &data, source).unwrap();

    let task = start_rollback_with_source(
        Some(first),
        instance.clone(),
        data,
        Ok("windows-x86_64".to_owned()),
        "1.0.0".to_owned(),
        Arc::new(LoginRequiredSource),
    );
    let restored = loop {
        if let InstallEvent::Finished(result) = task.events.recv().unwrap() {
            break result.unwrap();
        }
    };
    assert_eq!(restored.pack_version, "1");
    assert_eq!(
        fs::read(instance.join("horizon-mods/test.hmod")).unwrap(),
        b"old"
    );
    let previous: InstalledPack =
        serde_json::from_slice(&fs::read(instance.join(PREVIOUS_MANIFEST)).unwrap()).unwrap();
    assert_eq!(previous.pack_version, "2");
}

#[test]
fn update_download_bytes_counts_unique_uncached_objects() {
    let temp = TestDirectory::new();
    let source = Arc::new(DirectorySource::new(
        &temp.0.join("source"),
        &[b"base", b"old"],
    ));
    let data = temp.0.join("data");
    let instance = temp.0.join("instance");
    install(
        feed(
            "1",
            vec![entry("horizon-mods/test.hmod", "managed", b"old")],
        ),
        "1",
        &instance,
        &data,
        source,
    )
    .unwrap();
    let next = feed(
        "2",
        vec![
            entry("horizon-mods/test.hmod", "managed", b"new"),
            entry("horizon-mods/other.hmod", "managed", b"new"),
        ],
    );
    let plan = plan_install_at(
        &next,
        "example",
        "2",
        &instance,
        &data,
        "windows-x86_64",
        "1.0.0",
    )
    .unwrap();
    assert_eq!(download_bytes(&plan).unwrap(), 3);
}

struct FixedUpdateFeed(VerifiedFeed);

impl crate::yoake_update::UpdateFeedSource for FixedUpdateFeed {
    fn fetch(&self) -> Result<VerifiedFeed, crate::yoake_feed::FeedError> {
        Ok(self.0.clone())
    }
}

#[test]
fn update_check_reports_installed_version_without_network() {
    let temp = TestDirectory::new();
    let source = Arc::new(DirectorySource::new(
        &temp.0.join("source"),
        &[b"base", b"old"],
    ));
    let data = temp.0.join("data");
    let instance = temp.0.join("instance");
    let original = feed(
        "1",
        vec![entry("horizon-mods/test.hmod", "managed", b"old")],
    );
    install(original.clone(), "1", &instance, &data, source).unwrap();
    let task = crate::yoake_update::start_update_check_with_source(
        Arc::new(FixedUpdateFeed(original)),
        vec![instance.clone()],
        data,
        "1.0.0".to_owned(),
    );
    let result = task.events.recv().unwrap();
    assert_eq!(result.instance_dir, instance);
    assert_eq!(result.state, crate::yoake_update::UpdateState::UpToDate);
}

#[test]
fn login_required_preserves_previous_installation() {
    let temp = TestDirectory::new();
    let source = Arc::new(DirectorySource::new(
        &temp.0.join("source"),
        &[b"base", b"old"],
    ));
    let data = temp.0.join("data");
    let instance = temp.0.join("instance");
    let first = feed("1", vec![entry("horizon-mods/mod.hmod", "managed", b"old")]);
    install(first, "1", &instance, &data, source).unwrap();
    let previous_manifest = fs::read(instance.join(MANIFEST)).unwrap();
    let second = feed("2", vec![entry("horizon-mods/mod.hmod", "managed", b"new")]);
    let result = install(second, "2", &instance, &data, Arc::new(LoginRequiredSource));
    assert_eq!(result.unwrap_err(), InstallError::NeedsLogin);
    assert_eq!(
        fs::read(instance.join(MANIFEST)).unwrap(),
        previous_manifest
    );
    assert_eq!(
        fs::read(instance.join("horizon-mods/mod.hmod")).unwrap(),
        b"old"
    );
}

#[test]
fn rejects_corrupt_download_without_changing_previous_install() {
    let temp = TestDirectory::new();
    let source = Arc::new(DirectorySource::new(
        &temp.0.join("source"),
        &[b"base", b"old", b"new"],
    ));
    let data = temp.0.join("data");
    let instance = temp.0.join("instance");
    let first = feed("1", vec![entry("horizon-mods/mod.hmod", "managed", b"old")]);
    install(first, "1", &instance, &data, source.clone()).unwrap();
    let before = fs::read(instance.join(MANIFEST)).unwrap();
    fs::write(source.root.join(hash(b"new")), b"bad").unwrap();
    let next = feed("2", vec![entry("horizon-mods/mod.hmod", "managed", b"new")]);
    assert_eq!(
        install(next, "2", &instance, &data, source).unwrap_err(),
        InstallError::InvalidObject
    );
    assert_eq!(
        fs::read(instance.join("horizon-mods/mod.hmod")).unwrap(),
        b"old"
    );
    assert_eq!(fs::read(instance.join(MANIFEST)).unwrap(), before);
    assert!(
        !data
            .join("yoake-cache/objects")
            .join(format!("{}.partial", hash(b"new")))
            .exists()
    );
}

#[test]
fn discards_partial_downloads_and_honors_cancellation() {
    let temp = TestDirectory::new();
    let source = DirectorySource::new(&temp.0.join("source"), &[b"object"]);
    let cache = temp.0.join("cache");
    fs::create_dir_all(&cache).unwrap();
    let file = PlannedFile {
        scope: FileScope::Instance,
        destination: "config/test.toml".to_owned(),
        sha256: hash(b"object"),
        size: 6,
        policy: InstallPolicy::Managed,
        action: FileAction::Install,
        expected_before: None,
    };
    let partial = cache.join(format!("{}.partial", file.sha256));
    fs::write(&partial, b"stale").unwrap();
    let (events, _) = mpsc::channel();
    let done = AtomicU64::new(0);
    assert_eq!(
        cache_object(
            &cache,
            &file,
            &source,
            &AtomicBool::new(true),
            &done,
            6,
            &events
        )
        .unwrap_err(),
        InstallError::Cancelled,
    );
    cache_object(
        &cache,
        &file,
        &source,
        &AtomicBool::new(false),
        &done,
        6,
        &events,
    )
    .unwrap();
    assert!(!partial.exists());
    assert_eq!(fs::read(cache.join(file.sha256)).unwrap(), b"object");
}

#[test]
fn recovers_interrupted_replacement_before_and_after_old_file_unlink() {
    for remove_original in [false, true] {
        let temp = TestDirectory::new();
        let instance = temp.0.join("instance");
        let stage = instance.join(STAGING);
        fs::create_dir_all(instance.join("horizon-mods")).unwrap();
        fs::create_dir_all(&stage).unwrap();
        let transaction = "a".repeat(32);
        let destination = instance.join("horizon-mods/mod.hmod");
        let staged = format!("new-{transaction}-0");
        let backup = format!("old-{transaction}-0");
        fs::write(&destination, b"old").unwrap();
        fs::write(stage.join(&staged), b"new").unwrap();
        fs::write(stage.join(format!("new-{transaction}-1")), b"manifest").unwrap();
        let journal = Journal {
            transaction: transaction.clone(),
            operations: vec![
                JournalOperation {
                    destination: "horizon-mods/mod.hmod".to_owned(),
                    before: Some(hash(b"old")),
                    after: Some(hash(b"new")),
                    staged: Some(staged.clone()),
                    backup: backup.clone(),
                    default_on_create: false,
                },
                JournalOperation {
                    destination: MANIFEST.to_owned(),
                    before: None,
                    after: Some(hash(b"manifest")),
                    staged: Some(format!("new-{transaction}-1")),
                    backup: format!("old-{transaction}-1"),
                    default_on_create: false,
                },
            ],
        };
        fs::write(stage.join(JOURNAL), serde_json::to_vec(&journal).unwrap()).unwrap();
        fs::hard_link(&destination, stage.join(&backup)).unwrap();
        if remove_original {
            fs::remove_file(&destination).unwrap();
            fs::hard_link(stage.join(staged), &destination).unwrap();
        }
        recover_install(&instance).unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"old");
        assert!(!stage.join(JOURNAL).exists());
    }
}

#[test]
fn rollback_after_manifest_replacement_restores_previous_manifest() {
    let temp = TestDirectory::new();
    let instance = temp.0.join("instance");
    let stage = instance.join(STAGING);
    fs::create_dir_all(&stage).unwrap();
    let transaction = "d".repeat(32);
    let previous = InstalledPack {
        pack_id: "example".to_owned(),
        pack_version: "1".to_owned(),
        base_version: "1".to_owned(),
        sequence: 1,
        target: "windows-x86_64".to_owned(),
        managed_files: Vec::new(),
        transaction: "e".repeat(32),
        rollback_index: None,
        rollback_proof: None,
    };
    let mut installed = previous.clone();
    installed.pack_version = "2".to_owned();
    installed.transaction = transaction.clone();
    let previous_bytes = serde_json::to_vec(&previous).unwrap();
    let installed_bytes = serde_json::to_vec(&installed).unwrap();
    let staged = format!("new-{transaction}-0");
    let backup = format!("old-{transaction}-0");
    fs::write(stage.join(&backup), &previous_bytes).unwrap();
    fs::write(stage.join(&staged), &installed_bytes).unwrap();
    fs::hard_link(stage.join(&staged), instance.join(MANIFEST)).unwrap();
    let journal = Journal {
        transaction,
        operations: vec![JournalOperation {
            destination: MANIFEST.to_owned(),
            before: Some(hash(&previous_bytes)),
            after: Some(hash(&installed_bytes)),
            staged: Some(staged),
            backup,
            default_on_create: false,
        }],
    };
    fs::write(stage.join(JOURNAL), serde_json::to_vec(&journal).unwrap()).unwrap();
    let staging = Dir::open(&stage, false).unwrap();

    recover_locked(&instance, &staging, true).unwrap();
    assert_eq!(read_manifest(&instance).unwrap(), Some(previous));
    assert!(!stage.join(JOURNAL).exists());
}

#[test]
fn launch_checks_staging_without_a_manifest() {
    let temp = TestDirectory::new();
    let instance = temp.0.join("instance");
    fs::create_dir(&instance).unwrap();
    assert!(!has_manifest_or_pending(&instance));
    assert_eq!(prepare_launch(&instance, &temp.0).unwrap(), None);

    fs::create_dir(instance.join(STAGING)).unwrap();
    assert!(!has_manifest_or_pending(&instance));
    assert_eq!(prepare_launch(&instance, &temp.0).unwrap(), None);
    assert!(!has_manifest_or_pending(&instance));
}

#[test]
fn missing_recovery_backup_keeps_the_journal() {
    let temp = TestDirectory::new();
    let instance = temp.0.join("instance");
    let stage = instance.join(STAGING);
    fs::create_dir_all(instance.join("horizon-mods")).unwrap();
    fs::create_dir_all(&stage).unwrap();
    let transaction = "b".repeat(32);
    let journal = Journal {
        transaction: transaction.clone(),
        operations: vec![
            JournalOperation {
                destination: "horizon-mods/mod.hmod".to_owned(),
                before: Some(hash(b"old")),
                after: Some(hash(b"new")),
                staged: Some(format!("new-{transaction}-0")),
                backup: format!("old-{transaction}-0"),
                default_on_create: false,
            },
            JournalOperation {
                destination: MANIFEST.to_owned(),
                before: None,
                after: Some(hash(b"manifest")),
                staged: Some(format!("new-{transaction}-1")),
                backup: format!("old-{transaction}-1"),
                default_on_create: false,
            },
        ],
    };
    fs::write(stage.join(JOURNAL), serde_json::to_vec(&journal).unwrap()).unwrap();
    assert_eq!(
        recover_install(&instance).unwrap_err(),
        InstallError::RecoveryRequired
    );
    assert_eq!(
        prepare_launch(&instance, &temp.0).unwrap_err(),
        InstallError::RecoveryRequired
    );
    assert!(stage.join(JOURNAL).exists());
}

#[test]
fn modified_recovery_backup_is_not_restored() {
    let temp = TestDirectory::new();
    let instance = temp.0.join("instance");
    let stage = instance.join(STAGING);
    fs::create_dir_all(instance.join("horizon-mods")).unwrap();
    fs::create_dir_all(&stage).unwrap();
    let transaction = "c".repeat(32);
    let staged = format!("new-{transaction}-0");
    let backup = format!("old-{transaction}-0");
    let destination = instance.join("horizon-mods/mod.hmod");
    let journal = Journal {
        transaction: transaction.clone(),
        operations: vec![
            JournalOperation {
                destination: "horizon-mods/mod.hmod".to_owned(),
                before: Some(hash(b"old")),
                after: Some(hash(b"new")),
                staged: Some(staged.clone()),
                backup: backup.clone(),
                default_on_create: false,
            },
            JournalOperation {
                destination: MANIFEST.to_owned(),
                before: None,
                after: Some(hash(b"manifest")),
                staged: Some(format!("new-{transaction}-1")),
                backup: format!("old-{transaction}-1"),
                default_on_create: false,
            },
        ],
    };
    fs::write(stage.join(JOURNAL), serde_json::to_vec(&journal).unwrap()).unwrap();
    fs::write(stage.join(&staged), b"new").unwrap();
    fs::hard_link(stage.join(&staged), &destination).unwrap();
    fs::write(stage.join(&backup), b"tampered").unwrap();

    assert_eq!(
        recover_install(&instance).unwrap_err(),
        InstallError::RecoveryRequired
    );
    assert_eq!(fs::read(destination).unwrap(), b"new");
    assert!(stage.join(JOURNAL).exists());
}

#[cfg(unix)]
#[test]
fn refuses_symlinked_instance_parent() {
    use std::os::unix::fs::symlink;

    let temp = TestDirectory::new();
    let instance = temp.0.join("instance");
    let outside = temp.0.join("outside");
    fs::create_dir_all(&instance).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, instance.join("config")).unwrap();
    let source = Arc::new(DirectorySource::new(
        &temp.0.join("source"),
        &[b"base", b"data"],
    ));
    let index = feed("1", vec![entry("config/test.toml", "managed", b"data")]);
    assert!(install(index, "1", &instance, &temp.0.join("data"), source).is_err());
    assert!(!outside.join("test.toml").exists());
}
