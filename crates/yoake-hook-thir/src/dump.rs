use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{Error, ParseOptions, Result, parse_file, write_json};

const MIN_START_FREE: u64 = 10 * 1024 * 1024 * 1024;
#[cfg(windows)]
const MIN_RUNNING_FREE: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct DumpOptions {
    pub crate_pkg: String,
    pub crate_name: String,
    pub workspace: PathBuf,
    pub out: PathBuf,
    pub files: Vec<String>,
}

pub fn dump(options: &DumpOptions) -> Result<()> {
    let toolchain = rustc_version()?;
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
    let result = run_and_parse(command, options, &thir_path, &externs, toolchain);
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
    toolchain: String,
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
            toolchain,
        },
    )?;
    write_json(&parsed, &options.out)
}

fn rustc_version() -> Result<String> {
    let output = Command::new("rustc").arg("-vV").output()?;
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
