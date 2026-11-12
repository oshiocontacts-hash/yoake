use horizon_mods::yak::{Archive, Target};
use std::fs::{self, File};
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MANIFEST: &str = "format = 1\nid = 'sample-mod'\nversion = '0.3.1'\nname = 'Sample'\nauthors = ['Test author']\ndescription = 'Test package'\n[yoake]\nversion = '^0.3'\n[side]\nclient = true\nserver = false\nrequired_on_join = false\n";

struct Temp(PathBuf);

impl Temp {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let root = std::env::temp_dir().join(format!("cargo-horizon-mod-yak-{}-{stamp}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("yak.toml"), MANIFEST).unwrap();
        Self(root)
    }

    fn tool(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod")).current_dir(&self.0).args(args).output().unwrap()
    }

    fn archive(&self) -> PathBuf {
        self.0.join("sample-mod-0.3.1.yak")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn success(output: Output) -> String {
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn data_pack_and_verify_work_with_cargo_argument_prefix() {
    let temp = Temp::new();
    fs::create_dir(temp.0.join("content")).unwrap();
    fs::write(temp.0.join("content/example.json"), b"{}").unwrap();
    assert!(success(temp.tool(&["horizon-mod", "pack"])).contains("unsigned"));
    let archive = temp.archive();
    let summary = success(temp.tool(&["horizon-mod", "yak", "verify", archive.to_str().unwrap()]));
    assert!(summary.contains("sample-mod"));
    assert!(summary.contains("unsigned"));
    let mut archive = Archive::new(File::open(archive).unwrap()).unwrap();
    archive.extract_target(&temp.0.join("view"), Target::WindowsX64, &[]).unwrap();
    assert_eq!(fs::read(temp.0.join("view/content/example.json")).unwrap(), b"{}");
    assert!(!temp.tool(&["pack"]).status.success());
}

#[test]
fn explicit_fake_binaries_do_not_invoke_cargo_or_rustc() {
    let temp = Temp::new();
    let manifest = format!("{MANIFEST}\n[bin]\nlibrary = 'sample'\ntargets = ['windows-x64', 'linux-x64']\nengine = '0.3.1+test'\nabi = 2\n");
    fs::write(temp.0.join("yak.toml"), manifest).unwrap();
    fs::write(temp.0.join("fake.dll"), b"fake dll").unwrap();
    fs::write(temp.0.join("fake.so"), b"fake so").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_cargo-horizon-mod"))
        .current_dir(&temp.0).env("PATH", "").args(["pack", "--bin", "windows-x64=fake.dll", "--bin", "linux-x64=fake.so"]).output().unwrap();
    success(output);
    let archive = Archive::new(File::open(temp.archive()).unwrap()).unwrap();
    assert!(archive.hashes().contains_key("bin/windows-x64/sample.dll"));
    assert!(archive.hashes().contains_key("bin/linux-x64/libsample.so"));
}

#[test]
fn bad_options_and_unknown_targets_fail_without_publishing() {
    let temp = Temp::new();
    for args in [vec!["pack", "--bin"], vec!["pack", "--bin", "unknown=fake"], vec!["pack", "--bin", "windows-x64=a", "--bin", "windows-x64=b"], vec!["yak", "verify"], vec!["yak", "verify", "missing.yak", "--unexpected"]] {
        assert!(!temp.tool(&args).status.success());
        assert!(!temp.archive().exists());
    }
}
