use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_directory(name: &str) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "horizon-cleanup-{name}-{}-{timestamp}",
        std::process::id()
    ));
    fs::create_dir_all(&directory).unwrap();
    directory
}

#[test]
fn removes_a_populated_directory_and_accepts_an_absent_one() {
    let directory = temporary_directory("remove");
    fs::write(directory.join("file"), b"content").unwrap();

    crate::test_support::remove_dir_all(&directory);
    assert!(!directory.exists());
    crate::test_support::remove_dir_all(&directory);
}

#[test]
fn leaves_an_unremovable_path_without_panicking() {
    let directory = temporary_directory("failure");
    let file = directory.join("file");
    fs::write(&file, b"content").unwrap();

    crate::test_support::remove_dir_all(&file);
    assert!(file.is_file());
    crate::test_support::remove_dir_all(&directory);
}

#[cfg(windows)]
fn lock_file(directory: &std::path::Path) -> fs::File {
    use std::os::windows::fs::OpenOptionsExt;

    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .share_mode(0)
        .open(directory.join("locked"))
        .unwrap()
}

#[cfg(windows)]
#[test]
fn retries_until_a_sharing_violation_clears() {
    let directory = temporary_directory("transient-lock");
    let file = lock_file(&directory);
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(250));
        drop(file);
    });

    crate::test_support::remove_dir_all(&directory);
    release.join().unwrap();
    assert!(!directory.exists());
}

#[cfg(windows)]
#[test]
fn leaves_a_directory_when_a_sharing_violation_persists() {
    let directory = temporary_directory("persistent-lock");
    let file = lock_file(&directory);

    crate::test_support::remove_dir_all(&directory);
    assert!(directory.exists());
    drop(file);
    crate::test_support::remove_dir_all(&directory);
}
