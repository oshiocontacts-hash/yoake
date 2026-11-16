use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::{SystemTime, UNIX_EPOCH};

struct Project(PathBuf);

impl Project {
    fn new() -> Self {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("yoake-output-test-{}-{stamp}", process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for Project {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn invoke(root: &Path, arguments: &[&str], success: bool) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_yoake"))
        .current_dir(root)
        .env_remove("YOAKE_SDK_PATH")
        .args(arguments)
        .output()
        .unwrap();
    assert_eq!(output.status.success(), success, "{:?}", output);
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "stdout: {stdout:?}");
    let result: Value = serde_json::from_str(stdout).unwrap();
    assert_eq!(result["ok"], success);
    if !success {
        assert!(!output.stderr.is_empty());
        assert!(!result["errors"].as_array().unwrap().is_empty());
    }
    result
}

#[test]
fn commands_emit_one_json_line_on_success_and_failure() {
    let project = Project::new();
    for command in ["build", "check", "pack", "dev", "unknown"] {
        let result = invoke(&project.0, &[command], false);
        if command == "dev" {
            assert_eq!(result["built"], false);
            assert_eq!(result["checked"], false);
        }
    }
    invoke(&project.0, &["new", "hud-only", "con"], false);
    invoke(&project.0, &["new", "hud-only", "fresh"], true);
    assert_eq!(fs::read_to_string(project.0.join("fresh/.gitignore")).unwrap(), "target/\ndist/\n.yoake/\n");

    fs::write(project.0.join("yak.toml"), "format = 1\nid = \"data-only\"\nversion = \"0.1.0\"\nname = \"Data Only\"\nauthors = [\"Test\"]\ndescription = \"Fixture\"\n\n[yoake]\nversion = \"*\"\n\n[side]\nclient = true\nserver = false\nrequired_on_join = false\n").unwrap();
    fs::create_dir(project.0.join("content")).unwrap();
    fs::write(project.0.join("content/example.json"), "{}\n").unwrap();
    invoke(&project.0, &["check"], true);
    invoke(&project.0, &["pack"], true);
}

#[test]
#[cfg(any(unix, windows))]
fn dev_reports_progress_for_non_utf8_arguments() {
    #[cfg(unix)]
    let invalid = {
        use std::os::unix::ffi::OsStringExt;
        std::ffi::OsString::from_vec(vec![0xff])
    };
    #[cfg(windows)]
    let invalid = {
        use std::os::windows::ffi::OsStringExt;
        std::ffi::OsString::from_wide(&[0xd800])
    };
    let project = Project::new();
    let output = Command::new(env!("CARGO_BIN_EXE_yoake"))
        .current_dir(&project.0)
        .arg("dev")
        .arg(invalid)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1);
    let result: Value = serde_json::from_str(stdout).unwrap();
    assert_eq!(result["ok"], false);
    assert_eq!(result["built"], false);
    assert_eq!(result["checked"], false);
    assert!(!output.stderr.is_empty());
}
