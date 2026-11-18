//! The unsafe policy of the engine crates the glue is generated from, checked from the sources:
//! the compiler enforces it (`forbid`/`deny` plus a few `#[allow(unsafe_code)] mod`), and this module
//! checks that the compiler is told the right thing and that the files allowed to hold `unsafe` are
//! exactly the ones the generator freezes (`frozen_files`).

use std::path::{Path, PathBuf};

/// Crates that must carry `#![forbid(unsafe_code)]` in their `lib.rs`.
pub const FORBID_CRATES: [&str; 4] = ["horizon-core", "horizon-world", "horizon-client-core", "horizon-items"];

/// The crate whose `lib.rs` is `#![deny(unsafe_code)]` with `#[allow(unsafe_code)]` on single modules.
pub const DENY_CRATE: &str = "horizon-client";

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Whether a line of code (not a comment) opens unsafe code.
fn opens_unsafe(line: &str) -> bool {
    let line = line.trim_start();
    if line.starts_with("//") || line.starts_with("#!") || line.starts_with("#[") {
        return false;
    }
    let mut rest = line;
    while let Some(at) = rest.find("unsafe") {
        let before_ok = at == 0 || !rest[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_' || c == '"');
        let after = rest[at + "unsafe".len()..].trim_start();
        let after_ok = ["{", "fn ", "impl", "trait ", "extern"].iter().any(|k| after.starts_with(k));
        if before_ok && after_ok {
            return true;
        }
        rest = &rest[at + "unsafe".len()..];
    }
    false
}

fn ends_with_any(path: &Path, files: &[String]) -> bool {
    let path = path.to_string_lossy().replace('\\', "/");
    files.iter().any(|f| path.ends_with(&f.replace('\\', "/")))
}

/// Violations of the policy (empty = fine) under the workspace `root`.
pub fn check_unsafe_policy(root: &Path, frozen_files: &[String]) -> Vec<String> {
    let mut violations = Vec::new();
    for krate in FORBID_CRATES {
        let lib = root.join("crates").join(krate).join("src/lib.rs");
        match std::fs::read_to_string(&lib) {
            Ok(text) if text.contains("#![forbid(unsafe_code)]") => {}
            Ok(_) => violations.push(format!("{krate}: src/lib.rs lacks #![forbid(unsafe_code)]")),
            Err(e) => violations.push(format!("{krate}: cannot read src/lib.rs: {e}")),
        }
    }
    let src = root.join("crates").join(DENY_CRATE).join("src");
    for (file, attr) in [("lib.rs", "#![deny(unsafe_code)]"), ("client_lib.rs", "#![forbid(unsafe_code)]")] {
        match std::fs::read_to_string(src.join(file)) {
            Ok(text) if text.contains(attr) => {}
            _ => violations.push(format!("{DENY_CRATE}: src/{file} lacks {attr}")),
        }
    }

    // modules allowed to hold unsafe: `#[allow(unsafe_code)]` before `#[path = ".."] mod x;` / `mod x;`
    let mut files = Vec::new();
    for krate in FORBID_CRATES.iter().copied().chain([DENY_CRATE]) {
        rust_files(&root.join("crates").join(krate).join("src"), &mut files);
    }
    files.sort();
    let mut allowed: Vec<PathBuf> = Vec::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        let lines: Vec<&str> = text.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.starts_with("#![allow(unsafe_code)]") {
                allowed.push(file.clone());
            } else if trimmed.starts_with("#[allow(unsafe_code)]") {
                let dir = file.parent().unwrap_or(Path::new("."));
                for next in lines.iter().skip(index + 1).take(4) {
                    let next = next.trim();
                    if let Some(rest) = next.strip_prefix("#[path = \"") {
                        if let Some(name) = rest.split('"').next() {
                            allowed.push(dir.join(name));
                        }
                        break;
                    }
                    if let Some(rest) = next.strip_prefix("mod ").or_else(|| next.strip_prefix("pub mod ")).or_else(|| next.strip_prefix("pub(crate) mod ")) {
                        let name = rest.trim_end_matches(';').trim();
                        let flat = dir.join(format!("{name}.rs"));
                        allowed.push(if flat.exists() { flat } else { dir.join(name).join("mod.rs") });
                        break;
                    }
                }
            }
        }
    }
    for file in &allowed {
        if !ends_with_any(file, frozen_files) {
            violations.push(format!("{} is allowed to hold unsafe code but is not in frozen_files (its types must be frozen)", file.display()));
        }
    }
    // and every file that really opens unsafe code must be one of them (the compiler enforces this too)
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else { continue };
        if text.lines().any(opens_unsafe) && !allowed.iter().any(|a| a == file) {
            violations.push(format!("{} contains unsafe code outside the allowed modules", file.display()));
        }
    }
    violations
}
