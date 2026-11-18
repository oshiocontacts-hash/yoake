use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, FileDigest, ParseOptions, Result, collect_file_digests, parse_file, write_json};

const MIN_START_FREE: u64 = 10 * 1024 * 1024 * 1024;
#[cfg(windows)]
const MIN_RUNNING_FREE: u64 = 5 * 1024 * 1024 * 1024;
const WORKSPACE_MARKER_NAME: &str = ".yoake-workspace-root";

#[derive(Clone, Debug)]
pub struct DumpOptions {
    pub crate_pkg: String,
    pub crate_name: String,
    pub workspace: PathBuf,
    pub out: PathBuf,
    pub files: Vec<String>,
    pub allow_shared_target: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TargetDirDecision {
    Skip,
    WriteMarker,
    AllowExisting,
}

pub fn dump(options: &DumpOptions) -> Result<()> {
    check_target_dir_guard(&options.workspace, options.allow_shared_target)?;
    let file_digests = collect_file_digests(&options.workspace, &options.files)?;
    let rustc = rustc_version(&options.workspace)?;
    // THIR output is toolchain-specific: only the pinned compiler (rust-toolchain.toml) may produce a dump
    check_pinned_rustc(&options.workspace, &rustc)?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let probe_cfg = format!("yoake_thir_{}_{}", std::process::id(), nonce);
    let cargo = Command::new("cargo")
        .current_dir(&options.workspace)
        .args(["rustc", "-vv", "-p"])
        .arg(&options.crate_pkg)
        .args(["--lib", "--", "--cfg"])
        .arg(&probe_cfg)
        .output()?;
    let cargo_log = format!(
        "{}{}",
        String::from_utf8_lossy(&cargo.stdout),
        String::from_utf8_lossy(&cargo.stderr)
    );
    let running = cargo_log
        .lines()
        .filter(|line| line.contains("Running `") && line.contains("--crate-name"))
        .filter(|line| line.contains(&format!("--crate-name {}", options.crate_name)))
        .next_back()
        .ok_or_else(|| Error::new(format!("cargo did not print a rustc Running line: {cargo_log}")))?;
    let (cargo_env, command) = parse_running(running)?;
    let mut argv = tokenize_windows(&command);
    if argv.is_empty() {
        return Err(Error::new("empty rustc command"));
    }
    argv = clean_rustc_args(argv);
    let externs = extern_names(&argv);
    argv.push("-Zunpretty=thir-tree".into());
    check_disk_space(&options.out, MIN_START_FREE)?;

    let thir_path = sidecar(&options.out, "thir");
    let arg_path = sidecar(&options.out, "args");
    let stderr_path = sidecar(&options.out, "stderr");
    // rustc's @argfile takes one argument per line, verbatim: no quoting (an argument such as
    // `cfg(feature, values(...))` contains spaces and must not be wrapped in quotes).
    let args = argv[1..].join("\n");
    std::fs::write(&arg_path, args + "\n")?;
    let absolute_args = absolute(&arg_path)?;
    let stdout = std::fs::File::create(&thir_path)?;
    let stderr = std::fs::File::create(&stderr_path)?;
    let mut command = Command::new(&argv[0]);
    command
        .arg(format!("@{}", absolute_args.display()))
        .current_dir(&options.workspace)
        .envs(cargo_env)
        .env("RUSTC_BOOTSTRAP", "1")
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    let result = run_and_parse(
        command,
        options,
        &thir_path,
        &externs,
        rustc,
        &file_digests,
    );
    // YOAKE_THIR_KEEP=1 keeps the dump so `parse` can be re-run without rustc (debugging only).
    if std::env::var_os("YOAKE_THIR_KEEP").is_none() {
        let _ = std::fs::remove_file(&thir_path);
    }
    if result.is_ok() {
        let _ = std::fs::remove_file(&arg_path);
        let _ = std::fs::remove_file(&stderr_path);
        return result;
    }
    // Keep the argfile and rustc's stderr for diagnosis, and show the end of stderr.
    let tail = std::fs::read_to_string(&stderr_path)
        .map(|text| {
            let lines: Vec<&str> = text.lines().filter(|line| line.starts_with("error")).collect();
            lines[lines.len().saturating_sub(5)..].join("\n")
        })
        .unwrap_or_default();
    result.map_err(|error| {
        Error::new(format!(
            "{error}\n{tail}\n(rustc stderr kept at {}, args at {})",
            stderr_path.display(),
            arg_path.display()
        ))
    })
}

fn run_and_parse(
    mut command: Command,
    options: &DumpOptions,
    thir_path: &Path,
    externs: &std::collections::BTreeSet<String>,
    rustc: String,
    file_digests: &BTreeMap<String, FileDigest>,
) -> Result<()> {
    let mut child = command.spawn()?;
    let status = wait_for_rustc(&mut child, &options.out)?;
    if !status.success() {
        return Err(Error::new(format!("THIR rustc failed with {status}")));
    }
    let parsed = parse_file(
        thir_path,
        &ParseOptions {
            files: options.files.clone(),
            externs: externs.clone(),
            crate_name: Some(options.crate_name.clone()),
            rustc,
            cargo_lock_sha: crate::cargo_lock_sha(&options.workspace)?,
            workspace: Some(options.workspace.clone()),
            file_digests: file_digests.clone(),
        },
    )?;
    write_json(&parsed, &options.out)
}

fn rustc_version(workspace: &Path) -> Result<String> {
    // run in the workspace, so rustup picks the toolchain the way cargo will
    let output = Command::new("rustc").arg("-vV").current_dir(workspace).output()?;
    if !output.status.success() {
        return Err(Error::new("rustc -vV failed"));
    }
    String::from_utf8(output.stdout)
        .ok()
        .and_then(|text| text.lines().next().map(str::to_owned))
        .ok_or_else(|| Error::new("rustc -vV produced no version line"))
}

fn parse_running(line: &str) -> Result<(BTreeMap<String, String>, String)> {
    let start = line.find('`').ok_or_else(|| Error::new("invalid Running line"))? + 1;
    let end = line.rfind('`').ok_or_else(|| Error::new("invalid Running line"))?;
    let mut body = &line[start..end];
    let mut env = BTreeMap::new();
    while let Some(rest) = body.strip_prefix("set ") {
        let (assignment, remaining) = rest
            .split_once("&&")
            .ok_or_else(|| Error::new("unterminated cargo environment prefix"))?;
        let (key, value) = assignment
            .split_once('=')
            .ok_or_else(|| Error::new("invalid cargo environment assignment"))?;
        env.insert(key.trim().to_owned(), value.trim().trim_matches('\'').to_owned());
        body = remaining.trim_start();
    }
    Ok((env, body.trim().to_owned()))
}

pub fn tokenize_windows(command: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    for character in command.chars() {
        match (character, quote) {
            ('\'' | '"', None) => quote = Some(character),
            (value, Some(active)) if value == active => quote = None,
            (value, None) if value.is_whitespace() => {
                if !token.is_empty() {
                    result.push(std::mem::take(&mut token));
                }
            }
            _ => token.push(character),
        }
    }
    if !token.is_empty() {
        result.push(token);
    }
    result
}

fn clean_rustc_args(argv: Vec<String>) -> Vec<String> {
    let mut result = Vec::new();
    let mut index = 0;
    while index < argv.len() {
        let argument = &argv[index];
        if argument == "--cfg"
            && argv.get(index + 1).is_some_and(|value| value.starts_with("yoake_thir_"))
            || argument == "--error-format"
        {
            index += 2;
            continue;
        }
        if argument.starts_with("--emit=")
            || argument.starts_with("--error-format=")
            || argument.starts_with("--json=")
        {
            index += 1;
            continue;
        }
        result.push(argument.clone());
        index += 1;
    }
    result
}

fn extern_names(argv: &[String]) -> std::collections::BTreeSet<String> {
    let mut result = std::collections::BTreeSet::new();
    let mut index = 1;
    while index < argv.len() {
        let value = if argv[index] == "--extern" {
            index += 1;
            argv.get(index)
        } else {
            argv[index].strip_prefix("--extern=").map(|_| &argv[index])
        };
        if let Some(value) = value {
            let value = value.strip_prefix("--extern=").unwrap_or(value);
            result.insert(value.split('=').next().unwrap_or(value).to_owned());
        }
        index += 1;
    }
    result
}

fn sidecar(output: &Path, extension: &str) -> PathBuf {
    let name = output.file_name().unwrap_or_default().to_string_lossy();
    output.with_file_name(format!("{name}.{extension}"))
}

fn absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_owned())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

#[cfg(not(windows))]
fn check_disk_space(_: &Path, _: u64) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn check_disk_space(path: &Path, minimum: u64) -> Result<()> {
    let free = free_disk_space(path).ok_or_else(|| Error::new("GetDiskFreeSpaceExW failed"))?;
    if free < minimum {
        Err(Error::new(format!(
            "insufficient free disk space: {free} bytes available, {minimum} required"
        )))
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn wait_for_rustc(child: &mut std::process::Child, _: &Path) -> Result<ExitStatus> {
    Ok(child.wait()?)
}

#[cfg(windows)]
fn wait_for_rustc(child: &mut std::process::Child, output: &Path) -> Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if free_disk_space(output).is_some_and(|free| free < MIN_RUNNING_FREE) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::new("killed rustc: free disk space fell below 5 GiB"));
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
    }
}

#[cfg(windows)]
fn free_disk_space(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;

    #[link(name = "Kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            directory: *const u16,
            free_available: *mut u64,
            total: *mut u64,
            total_free: *mut u64,
        ) -> i32;
    }

    let directory = if path.is_dir() { path } else { path.parent()? };
    let wide: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut free = 0;
    let result = unsafe {
        GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut())
    };
    (result != 0).then_some(free)
}

pub fn resolve_target_dir(workspace: &Path, cargo_target_dir: Option<&OsStr>) -> PathBuf {
    match cargo_target_dir.filter(|value| !value.is_empty()) {
        Some(raw) => {
            let path = PathBuf::from(raw);
            if path.is_relative() {
                workspace.join(path)
            } else {
                path
            }
        }
        None => workspace.join("target"),
    }
}

pub fn canonical_workspace_root(workspace: &Path) -> Result<String> {
    let canonical = std::fs::canonicalize(workspace).map_err(|error| {
        Error::new(format!(
            "failed to canonicalize workspace {}: {error}",
            workspace.display()
        ))
    })?;
    Ok(strip_verbatim_prefix(&canonical.to_string_lossy()))
}

fn strip_verbatim_prefix(path: &str) -> String {
    if let Some(rest) = path.strip_prefix(r"\\?\")
        && !rest.starts_with(r"UNC\")
    {
        rest.to_owned()
    } else {
        path.to_owned()
    }
}

fn normalize_root_for_cmp(root: &str) -> String {
    let trimmed = strip_verbatim_prefix(root.trim());
    let slashes = trimmed.replace('\\', "/");
    let without_trailing = slashes.trim_end_matches('/');
    #[cfg(windows)]
    {
        without_trailing.to_ascii_lowercase()
    }
    #[cfg(not(windows))]
    {
        without_trailing.to_owned()
    }
}

pub fn decide_target_dir_guard(
    target_dir: &Path,
    this_root: &str,
    existing_marker: Option<&str>,
    allow_shared_target: bool,
) -> Result<TargetDirDecision> {
    if allow_shared_target {
        return Ok(TargetDirDecision::Skip);
    }
    let marker = existing_marker.map(str::trim).filter(|value| !value.is_empty());
    match marker {
        None => Ok(TargetDirDecision::WriteMarker),
        Some(marker_root)
            if normalize_root_for_cmp(marker_root) == normalize_root_for_cmp(this_root) =>
        {
            Ok(TargetDirDecision::AllowExisting)
        }
        Some(marker_root) => Err(Error::new(format!(
            "refusing to share CARGO_TARGET_DIR {} between worktrees ({} vs {}): cargo reuses artifacts across worktrees with stale sources; use a separate target dir or pass --allow-shared-target",
            target_dir.display(),
            marker_root,
            this_root,
        ))),
    }
}

pub fn check_target_dir_guard(workspace: &Path, allow_shared_target: bool) -> Result<()> {
    let cargo_target_dir = std::env::var_os("CARGO_TARGET_DIR");
    let target_dir = resolve_target_dir(workspace, cargo_target_dir.as_deref());
    check_target_dir_guard_at(workspace, &target_dir, allow_shared_target)
}

pub fn check_target_dir_guard_at(
    workspace: &Path,
    target_dir: &Path,
    allow_shared_target: bool,
) -> Result<()> {
    if allow_shared_target {
        return Ok(());
    }
    let this_root = canonical_workspace_root(workspace)?;
    let marker_path = target_dir.join(WORKSPACE_MARKER_NAME);
    let existing_marker = match std::fs::read_to_string(&marker_path) {
        Ok(contents) => Some(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(Error::new(format!(
                "failed to read {}: {error}",
                marker_path.display()
            )));
        }
    };
    if decide_target_dir_guard(
        target_dir,
        &this_root,
        existing_marker.as_deref(),
        allow_shared_target,
    )? == TargetDirDecision::WriteMarker
    {
        std::fs::create_dir_all(target_dir)?;
        std::fs::write(&marker_path, format!("{this_root}\n"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_dir_guard_decisions() {
        let target = Path::new("/tmp/shared-target");
        assert_eq!(
            decide_target_dir_guard(target, "/repo/wt-a", None, false).unwrap(),
            TargetDirDecision::WriteMarker
        );
        assert_eq!(
            decide_target_dir_guard(target, "/repo/wt-a", Some("/repo/wt-a\n"), false).unwrap(),
            TargetDirDecision::AllowExisting
        );
        let err = decide_target_dir_guard(target, "/repo/wt-b", Some("/repo/wt-a\n"), false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("refusing to share CARGO_TARGET_DIR"), "{err}");
        assert!(err.contains("/repo/wt-a vs /repo/wt-b"), "{err}");
        assert!(err.contains("--allow-shared-target"), "{err}");
        assert_eq!(
            decide_target_dir_guard(target, "/repo/wt-b", Some("/repo/wt-a\n"), true).unwrap(),
            TargetDirDecision::Skip
        );
    }

    #[test]
    fn target_dir_guard_writes_reads_and_skips_marker_on_disk() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let base = std::env::temp_dir().join(format!("yoake_thir_guard_{}_{}", std::process::id(), nonce));
        let ws_a = base.join("wt-a");
        let ws_b = base.join("wt-b");
        let target = base.join("shared-target");
        std::fs::create_dir_all(&ws_a).unwrap();
        std::fs::create_dir_all(&ws_b).unwrap();

        check_target_dir_guard_at(&ws_a, &target, false).unwrap();
        let marker_path = target.join(WORKSPACE_MARKER_NAME);
        let written = std::fs::read_to_string(&marker_path).unwrap();
        assert_eq!(written.trim(), canonical_workspace_root(&ws_a).unwrap());

        check_target_dir_guard_at(&ws_a, &target, false).unwrap();

        let err = check_target_dir_guard_at(&ws_b, &target, false)
            .unwrap_err()
            .to_string();
        assert!(err.contains("refusing to share CARGO_TARGET_DIR"), "{err}");

        check_target_dir_guard_at(&ws_b, &target, true).unwrap();
        assert_eq!(std::fs::read_to_string(&marker_path).unwrap(), written);

        let _ = std::fs::remove_dir_all(&base);
    }
}

/// The `channel = "..."` of a `rust-toolchain.toml`.
pub fn parse_toolchain_channel(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("channel")?.trim_start().strip_prefix('=')?.trim();
        Some(rest.trim_matches('"').to_string())
    })
}

/// Stops unless `rustc_version` (the first line of `rustc -V`) is the exact version `<workspace>/rust-toolchain.toml`
/// pins.
pub fn check_pinned_rustc(workspace: &Path, rustc_version: &str) -> Result<()> {
    let path = workspace.join("rust-toolchain.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| Error::new(format!("cannot read {}: {e}", path.display())))?;
    let channel = parse_toolchain_channel(&text).ok_or_else(|| Error::new(format!("{} has no channel line", path.display())))?;
    if !channel.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return Err(Error::new(format!("rust-toolchain.toml pins `{channel}`: THIR needs an exact version")));
    }
    let line = rustc_version.trim();
    if line == format!("rustc {channel}") || line.starts_with(&format!("rustc {channel} ")) {
        Ok(())
    } else {
        Err(Error::new(format!("the workspace pins rustc {channel} (rust-toolchain.toml), but rustc is `{line}`; THIR output is toolchain-specific")))
    }
}

#[cfg(test)]
mod toolchain_tests {
    use super::*;

    #[test]
    fn a_dump_needs_the_pinned_rustc() {
        let dir = std::env::temp_dir().join(format!("yoake_thir_pin_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("rust-toolchain.toml"), "[toolchain]
channel = \"1.100.0\"
").unwrap();
        assert!(check_pinned_rustc(&dir, "rustc 1.100.0 (0ef1bc386 2026-11-09)").is_ok());
        assert!(check_pinned_rustc(&dir, "rustc 1.99.1 (x)").is_err());
        assert!(check_pinned_rustc(&dir, "rustc 1.100.01 (x)").is_err());
        std::fs::write(dir.join("rust-toolchain.toml"), "[toolchain]
channel = \"stable\"
").unwrap();
        assert!(check_pinned_rustc(&dir, "rustc 1.100.0 (x)").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
