use horizon_mods::yak::{Archive, Manifest, Summary, Target, pack_directory, parse_keys};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub fn main(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("verify") => verify(&args[1..]),
        _ => Err("usage: cargo horizon-mod yak verify <file> [--key <public key>]".into()),
    }
}

pub fn pack(args: &[String]) -> Result<(), String> {
    let mut folder = None;
    let mut binaries = BTreeMap::new();
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        if argument == "--bin" {
            let value = args.next().ok_or("--bin needs <target>=<path>")?;
            let (target, path) = value.split_once('=').ok_or("--bin needs <target>=<path>")?;
            let target = target.parse::<Target>()?;
            if path.is_empty() || binaries.insert(target, PathBuf::from(path)).is_some() {
                return Err(format!("empty path or duplicate --bin for {target}"));
            }
        } else if argument.starts_with('-') || folder.replace(argument).is_some() {
            return Err("usage: cargo horizon-mod pack [folder] [--bin <target>=<path>]".into());
        }
    }
    let current = std::env::current_dir().map_err(|error| error.to_string())?;
    let root = folder.map_or(current.clone(), |folder| current.join(folder));
    let manifest = Manifest::read(&root.join("yak.toml"))?;
    for path in binaries.values_mut() {
        if path.is_relative() {
            *path = root.join(&*path);
        }
    }
    if binaries.is_empty() && manifest.bin.is_some() {
        let target = Target::host()?;
        let bin = manifest.bin.as_ref().ok_or("missing [bin]")?;
        if !bin.targets.contains(&target) {
            return Err(format!("[bin].targets does not include this host ({target}); supply --bin"));
        }
        binaries.insert(target, build_host_library(&root, target, &bin.library)?);
    }
    let output = root.join(format!("{}-{}.yak", manifest.id, manifest.version));
    let summary = pack_directory(&root, &output, &binaries)?;
    print_summary(&summary);
    println!("Wrote unsigned {}", output.display());
    Ok(())
}

fn verify(args: &[String]) -> Result<(), String> {
    let file = args.first().filter(|argument| !argument.starts_with('-'))
        .ok_or("usage: cargo horizon-mod yak verify <file> [--key <public key>]")?;
    let mut keys = Vec::new();
    let mut args = args[1..].iter();
    while let Some(argument) = args.next() {
        if argument != "--key" {
            return Err(format!("unexpected argument: {argument}"));
        }
        let value = args.next().ok_or("--key needs a base64 Ed25519 public key")?;
        keys.extend(parse_keys(value)?);
    }
    let archive = Archive::new(File::open(file).map_err(|error| error.to_string())?)?;
    let signer = archive.verify_signature(&keys)?;
    print_summary(&archive.summary());
    for (part, entries) in archive.parts() {
        println!("  {part:?}: {} files", entries.len());
    }
    println!("Paths, sizes, CRCs and SHA-256 hashes checked");
    println!("Signature: {}", if signer.is_some() { "verified with a pinned key" } else { "absent (unsigned)" });
    Ok(())
}

fn print_summary(summary: &Summary) {
    println!("{} {}: {} files, {} uncompressed bytes, {}", summary.manifest.id,
        summary.manifest.version, summary.files, summary.uncompressed_size,
        if summary.manifest.bin.is_some() { "program package" } else { "data-only package" });
}

fn build_host_library(root: &Path, target: Target, library: &str) -> Result<PathBuf, String> {
    let host = Command::new("rustc").arg("-vV").output()
        .map_err(|error| format!("cannot discover the Rust host: {error}; supply --bin instead"))?;
    if !host.status.success() {
        return Err("rustc -vV failed; supply --bin instead".into());
    }
    let host = String::from_utf8(host.stdout).map_err(|_| "rustc host is not UTF-8")?;
    let triple = host.lines().find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc did not report its host triple")?;
    let matches = match target {
        Target::WindowsX64 => triple.starts_with("x86_64-") && triple.contains("-windows-"),
        Target::LinuxX64 => triple.starts_with("x86_64-") && triple.contains("-linux-"),
        Target::MacosArm64 => triple == "aarch64-apple-darwin",
        Target::MacosX64 => triple == "x86_64-apple-darwin",
    };
    if !matches {
        return Err("rustc host differs from this tool's host; supply --bin explicitly".into());
    }
    eprintln!("Building the mod's host library with cargo build --release --lib --target {triple}");
    let mut child = Command::new("cargo").current_dir(root)
        .args(["build", "--release", "--lib", "--target", triple, "--message-format=json-render-diagnostics"])
        .stdout(Stdio::piped()).stderr(Stdio::inherit()).spawn()
        .map_err(|error| format!("cargo build: {error}"))?;
    let stdout = child.stdout.take().ok_or("cargo stdout is unavailable")?;
    let expected = target.library_filename(library);
    let mut artifacts = BTreeSet::new();
    for line in BufReader::new(stdout).lines() {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("cargo output: {error}"));
            }
        };
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
            eprintln!("{line}");
            continue;
        };
        if message["reason"] == "compiler-message" {
            if let Some(rendered) = message["message"]["rendered"].as_str() {
                eprint!("{rendered}");
            }
        } else if message["reason"] == "compiler-artifact"
            && message["target"]["crate_types"].as_array()
                .is_some_and(|types| types.iter().any(|kind| kind == "cdylib"))
        {
            if let Some(filenames) = message["filenames"].as_array() {
                for filename in filenames.iter().filter_map(serde_json::Value::as_str) {
                    let path = PathBuf::from(filename);
                    if path.file_name().and_then(|name| name.to_str()) == Some(expected.as_str()) {
                        artifacts.insert(path);
                    }
                }
            }
        }
    }
    if !child.wait().map_err(|error| error.to_string())?.success() {
        return Err("cargo build failed; no .yak was written".into());
    }
    if artifacts.len() != 1 {
        return Err(format!("expected one cdylib artifact named {expected}; configure crate-type = [\"cdylib\"] or supply --bin"));
    }
    artifacts.into_iter().next().ok_or("missing library artifact".into())
}
