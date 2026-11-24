//! W1: `apply --verify`: stamp, build, withdraw the stamps the compiler rejects, repeat.
//!
//! One round = `run::execute` (stamps, honouring `hook-ledger/stamp-exclude.txt`), then `cargo <verify-args>
//! --message-format=json`. Every error is mapped to the hook attribute it belongs to (the one just above the error
//! line, or just below it for the wrapper of a converted trait method). The keys go into `stamp-exclude.txt` with
//! the error code as reason; the next `execute` records them as manual exclusions in the ledger (retiring the id of
//! a live hook), so the result is deterministic and a later plain `apply` keeps them out. The file is removed when
//! the build is clean. Rounds stop when the build is clean, when a round maps no new key (error outside the
//! scanned sources or not attributable), or at `--verify-rounds`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use crate::Config;

pub const EXCLUDE_FILE: &str = "stamp-exclude.txt";

/// One compiler error reduced to what the loop needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    pub code: String,
    pub locations: Vec<(PathBuf, usize)>,
    pub message: String,
}

pub struct BuildOutcome {
    pub success: bool,
    pub errors: Vec<BuildError>,
    /// stderr tail for failures without a compiler message (linker, disk, rustc crash).
    pub tail: String,
}

/// Parses `cargo --message-format=json` stdout. Primary spans and their macro-expansion call sites count.
pub fn parse_build_output(stdout: &str) -> Vec<BuildError> {
    let mut errors = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if !line.starts_with('{') {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(line) else { continue };
        if value.get("reason").and_then(Value::as_str) != Some("compiler-message") {
            continue;
        }
        let message = &value["message"];
        if message.get("level").and_then(Value::as_str) != Some("error") {
            continue;
        }
        let mut locations = Vec::new();
        for span in message.get("spans").and_then(Value::as_array).into_iter().flatten() {
            if span.get("is_primary").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            collect_span(span, &mut locations);
        }
        errors.push(BuildError {
            code: message
                .pointer("/code/code")
                .and_then(Value::as_str)
                .unwrap_or("error")
                .to_string(),
            locations,
            message: message.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
        });
    }
    errors
}

fn collect_span(span: &Value, out: &mut Vec<(PathBuf, usize)>) {
    if let (Some(file), Some(line)) = (
        span.get("file_name").and_then(Value::as_str),
        span.get("line_start").and_then(Value::as_u64),
    ) {
        out.push((PathBuf::from(file), line as usize));
    }
    // the call site of the attribute macro that produced the span
    if let Some(expansion) = span.get("expansion").filter(|e| !e.is_null())
        && let Some(inner) = expansion.get("span")
    {
        collect_span(inner, out);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookAttr {
    pub line: usize,
    pub key: String,
}

/// Hook attributes of one file in source order (`#[...yoake_hook(... key = "K" ...)]`, also inside `cfg_attr`).
pub fn find_hook_attrs(text: &str) -> Vec<HookAttr> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut search_from = 0;
    while let Some(rel) = text[search_from..].find("yoake_hook(") {
        let at = search_from + rel;
        search_from = at + "yoake_hook(".len();
        let Some(start) = text[..at].rfind("#[") else { continue };
        // the attribute must close after `at`
        let mut depth = 0i32;
        let mut end = None;
        for (i, b) in bytes.iter().enumerate().skip(start + 1) {
            match b {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { continue };
        if end < at {
            continue;
        }
        let body = &text[start..=end];
        if body.contains("kind = callsite") {
            continue;
        }
        let Some(k) = body.find("key = \"") else { continue };
        let rest = &body[k + 7..];
        let Some(close) = rest.find('"') else { continue };
        out.push(HookAttr {
            line: text[..start].matches('\n').count() + 1,
            key: rest[..close].to_string(),
        });
        search_from = end;
    }
    out
}

fn is_fn_header(line: &str) -> bool {
    let t = line.trim_start();
    let t = t.strip_prefix("pub").map_or(t, |r| {
        let r = r.trim_start();
        if r.starts_with('(') {
            r.split_once(')').map_or(r, |(_, after)| after.trim_start())
        } else {
            r
        }
    });
    let mut t = t;
    for word in ["default ", "const ", "async ", "unsafe "] {
        t = t.strip_prefix(word).unwrap_or(t);
    }
    t.starts_with("fn ")
}

/// The hook an error at `line` belongs to, or `None` when it cannot be told.
pub fn attribute_error(text: &str, attrs: &[HookAttr], line: usize) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let headers_between = |from: usize, to: usize| {
        (from..=to.min(lines.len()))
            .filter(|n| *n >= 1 && is_fn_header(lines[n - 1]))
            .count()
    };
    if let Some(prev) = attrs.iter().rev().find(|a| a.line <= line)
        && headers_between(prev.line, line) <= 1
    {
        return Some(prev.key.clone());
    }
    // wrapper of a converted trait method: the error is above its helper's attribute
    if let Some(next) = attrs.iter().find(|a| a.line > line)
        && headers_between(line, next.line) <= 1
    {
        return Some(next.key.clone());
    }
    None
}

/// Keys to withdraw for these errors. `files` resolves a compiler path to the source text.
pub fn keys_for_errors(
    errors: &[BuildError],
    base: &Path,
    scanned: &BTreeSet<PathBuf>,
) -> BTreeMap<String, String> {
    let mut cache: BTreeMap<PathBuf, (String, Vec<HookAttr>)> = BTreeMap::new();
    let mut out = BTreeMap::new();
    for error in errors {
        for (file, line) in &error.locations {
            let full = if file.is_absolute() { file.clone() } else { base.join(file) };
            let Ok(canon) = full.canonicalize() else { continue };
            let canon = normalize(&canon);
            if !scanned.contains(&canon) {
                continue;
            }
            let entry = cache.entry(canon.clone()).or_insert_with(|| {
                let text = std::fs::read_to_string(&canon).unwrap_or_default();
                let attrs = find_hook_attrs(&text);
                (text, attrs)
            });
            if let Some(key) = attribute_error(&entry.0, &entry.1, *line) {
                out.entry(key).or_insert_with(|| format!("compile error {}: {}", error.code, short(&error.message)));
                break;
            }
        }
    }
    out
}

fn short(message: &str) -> String {
    let one: String = message.chars().take(80).collect();
    one.replace(['\n', '\r', '#'], " ")
}

fn normalize(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\', "/").to_lowercase())
}

/// W7: verify through the overlay: the tree stays unstamped, ids and exclusions go to the real ledger.
#[derive(Clone)]
pub struct OverlayVerify {
    pub workspace_root: PathBuf,
    /// `<target>/yoake-stamp`; `<target>` is also the cargo target dir of the build.
    pub stamp_dir: PathBuf,
    pub stamp_config: PathBuf,
}

pub struct VerifyOptions {
    pub overlay: Option<OverlayVerify>,
    pub cargo_args: Vec<String>,
    pub dir: PathBuf,
    pub rounds: usize,
    /// Stop when one round would withdraw more stamps than this: the cause is systematic (a missing dependency, a
    /// broken macro), not individual functions.
    pub max_withdraw_per_round: usize,
}

fn run_cargo(options: &VerifyOptions) -> Result<BuildOutcome, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut args: Vec<String> = if options.cargo_args.is_empty() {
        vec!["check".into(), "--offline".into()]
    } else {
        options.cargo_args.clone()
    };
    args.push("--message-format=json".into());
    let mut command = Command::new(cargo);
    command.args(&args).current_dir(&options.dir);
    if let Some(overlay) = &options.overlay {
        command
            .env("RUSTC_WORKSPACE_WRAPPER", std::env::current_exe().map_err(|e| e.to_string())?)
            .env("YOAKE_STAMP", "1")
            .env("YOAKE_STAMP_DIR", &overlay.stamp_dir)
            .env("YOAKE_WORKSPACE", &overlay.workspace_root)
            .env("CARGO_TARGET_DIR", overlay.stamp_dir.parent().unwrap_or(&overlay.stamp_dir));
    }
    let output = command
        .output()
        .map_err(|e| format!("failed to run cargo: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let tail: String = stderr.lines().rev().take(25).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join("\n");
    Ok(BuildOutcome { success: output.status.success(), errors: parse_build_output(&stdout), tail })
}

/// The files of the scanned modules, as the compiler will name them (normalized for comparison).
fn scanned_files(config: &Config, options: &VerifyOptions) -> Result<BTreeSet<PathBuf>, String> {
    if let Some(overlay) = &options.overlay {
        let (_, files) = crate::overlay::source_closure(&config.crate_root, &config.crate_name)?;
        let workspace = crate::overlay::canon(&overlay.workspace_root)?;
        let mut out = BTreeSet::new();
        for f in files {
            let f = crate::overlay::canon(&f)?;
            if let Ok(rel) = f.strip_prefix(&workspace)
                && let Ok(c) = overlay.stamp_dir.join("root").join(rel).canonicalize()
            {
                out.insert(normalize(&c));
            }
        }
        return Ok(out);
    }
    let module_args = crate::modules::expand_star_modules(
        &config.crate_root,
        &config.crate_name,
        &config.modules,
        &config.exclude_modules,
    )?;
    let targets = module_args
        .iter()
        .map(|m| crate::modules::normalize_module_arg(m))
        .collect::<Result<Vec<_>, _>>()?;
    let files = crate::modules::collect_module_files(&config.crate_root, &config.crate_name, &targets, config.shallow)?;
    Ok(files
        .iter()
        .filter_map(|f| f.path.canonicalize().ok())
        .map(|p| normalize(&p))
        .collect())
}

pub fn append_excludes(ledger: &Path, keys: &BTreeMap<String, String>) -> Result<(), String> {
    let path = ledger.join(EXCLUDE_FILE);
    let mut text = std::fs::read_to_string(&path).unwrap_or_default();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for (key, reason) in keys {
        text.push_str(&format!("{key} # {reason}\n"));
    }
    std::fs::write(&path, text).map_err(|e| format!("failed to write {}: {e}", path.display()))
}

/// `key # reason` lines of `stamp-exclude.txt`.
pub fn read_excludes(ledger: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Ok(text) = std::fs::read_to_string(ledger.join(EXCLUDE_FILE)) {
        for line in text.lines() {
            let (key, reason) = line.split_once(" # ").unwrap_or((line, "compile error"));
            let key = key.trim();
            if !key.is_empty() {
                out.insert(key.to_string(), reason.trim().to_string());
            }
        }
    }
    out
}

/// One stamping step: in place (`execute`) or into the ledger only (overlay mode: ids and exclusions are
/// written, the sources are not).
fn stamp_round(config: &Config, options: &VerifyOptions) -> Result<(), String> {
    match &options.overlay {
        None => crate::run::execute(config).map(|_| ()),
        Some(overlay) => crate::overlay::build_overlay(&crate::overlay::OverlayArgs {
            workspace_root: overlay.workspace_root.clone(),
            overlay_root: overlay.stamp_dir.join("root"),
            crate_root: config.crate_root.clone(),
            crate_name: config.crate_name.clone(),
            ledger: config.ledger.clone(),
            stamp_config: overlay.stamp_config.clone(),
            frozen: false,
            renames: config.renames.clone(),
            tombstones: config.tombstones.clone(),
        })
        .map(|_| ()),
    }
}

/// The loop, with the build injected (tests pass a closure).
pub fn run_with(
    config: &Config,
    options: &VerifyOptions,
    mut build: impl FnMut() -> Result<BuildOutcome, String>,
) -> Result<bool, String> {
    let mut withdrawn_total = 0usize;
    for round in 1..=options.rounds.max(1) {
        stamp_round(config, options)?;
        let outcome = build()?;
        if outcome.success {
            let _ = std::fs::remove_file(config.ledger.join(EXCLUDE_FILE));
            println!("verify: build clean after {round} round(s); {withdrawn_total} stamp(s) withdrawn");
            return Ok(true);
        }
        let scanned = scanned_files(config, options)?;
        let base = options
            .overlay
            .as_ref()
            .map_or_else(|| options.dir.clone(), |o| o.stamp_dir.join("root"));
        let keys = keys_for_errors(&outcome.errors, &base, &scanned);
        let known = read_excludes(&config.ledger);
        let new: BTreeMap<String, String> =
            keys.into_iter().filter(|(k, _)| !known.contains_key(k)).collect();
        println!(
            "verify: round {round}: {} error(s), {} new stamp(s) to withdraw",
            outcome.errors.len(),
            new.len()
        );
        if new.is_empty() {
            for error in outcome.errors.iter().take(8) {
                println!("  unattributed {} {}: {:?}", error.code, error.message, error.locations.first());
            }
            if !outcome.tail.is_empty() {
                println!("  cargo tail:\n{}", outcome.tail);
            }
            return Err("verify made no progress: the remaining errors are not in a stamped item of the scanned modules".into());
        }
        if new.len() > options.max_withdraw_per_round {
            let mut by_code: BTreeMap<&str, usize> = BTreeMap::new();
            for reason in new.values() {
                *by_code.entry(reason.split(':').next().unwrap_or("")).or_default() += 1;
            }
            let sample = outcome.errors.first().map(|e| format!("{} {}", e.code, e.message)).unwrap_or_default();
            return Err(format!(
                "verify: round {round} would withdraw {} stamps (limit {}): {by_code:?}; first error: {sample}. Nothing was withdrawn this round; fix the cause or raise --verify-max-withdraw",
                new.len(),
                options.max_withdraw_per_round
            ));
        }
        withdrawn_total += new.len();
        append_excludes(&config.ledger, &new)?;
    }
    Err(format!("verify: still failing after {} rounds", options.rounds))
}

pub fn run(config: &Config, options: &VerifyOptions) -> Result<bool, String> {
    run_with(config, options, || run_cargo(options))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Mode;

    const SRC: &str = "\
use x;

#[yoake_hook_macro::yoake_hook(id = 1, key = \"c::a\")]
pub fn a() -> u32 {
    1
}

#[cfg_attr(
    not(feature = \"b\"),
    yoake_hook(id = 2, key = \"c::b\")
)]
pub fn b() -> u32 {
    2
}

impl T {
    fn fmt(&self) -> u32 {
        self.__yoake_trait_x()
    }

    #[yoake_hook_macro::yoake_hook(method, id = 3, key = \"c::<T as X>::fmt\")]
    fn __yoake_trait_x(&self) -> u32 {
        3
    }
}

pub fn unstamped() {
    let x = 1;
}
";

    #[test]
    fn hook_attributes_are_found_with_lines_and_keys() {
        let attrs = find_hook_attrs(SRC);
        assert_eq!(
            attrs,
            vec![
                HookAttr { line: 3, key: "c::a".into() },
                HookAttr { line: 8, key: "c::b".into() },
                HookAttr { line: 21, key: "c::<T as X>::fmt".into() },
            ]
        );
    }

    #[test]
    fn errors_map_to_their_own_item_or_the_helper_below_a_trait_wrapper() {
        let attrs = find_hook_attrs(SRC);
        assert_eq!(attribute_error(SRC, &attrs, 3).as_deref(), Some("c::a"), "error on the attribute line");
        assert_eq!(attribute_error(SRC, &attrs, 5).as_deref(), Some("c::a"));
        assert_eq!(attribute_error(SRC, &attrs, 12).as_deref(), Some("c::b"));
        assert_eq!(attribute_error(SRC, &attrs, 18).as_deref(), Some("c::<T as X>::fmt"), "wrapper above helper");
        assert_eq!(attribute_error(SRC, &attrs, 23).as_deref(), Some("c::<T as X>::fmt"));
        assert_eq!(attribute_error(SRC, &attrs, 29), None, "an unstamped fn after the last hook is not attributed");
    }

    #[test]
    fn cargo_json_errors_keep_primary_spans_and_expansion_sites() {
        let json = r#"{"reason":"compiler-message","message":{"level":"error","message":"boom","code":{"code":"E0581"},"spans":[{"is_primary":true,"file_name":"src/a.rs","line_start":9,"expansion":{"span":{"file_name":"src/a.rs","line_start":3}}},{"is_primary":false,"file_name":"src/z.rs","line_start":1}]}}
{"reason":"compiler-message","message":{"level":"warning","message":"w","spans":[]}}
{"reason":"build-finished","success":false}"#;
        let errors = parse_build_output(json);
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].code, "E0581");
        assert_eq!(errors[0].locations, vec![(PathBuf::from("src/a.rs"), 9), (PathBuf::from("src/a.rs"), 3)]);
    }

    #[test]
    fn exclude_file_round_trips_keys_with_reasons() {
        let dir = std::env::temp_dir().join(format!("yoake-verify-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut keys = BTreeMap::new();
        keys.insert("c::a".to_string(), "compile error E0581: boom".to_string());
        append_excludes(&dir, &keys).unwrap();
        append_excludes(&dir, &BTreeMap::from([("c::b".to_string(), "r".to_string())])).unwrap();
        let read = read_excludes(&dir);
        assert_eq!(read.len(), 2);
        assert_eq!(read["c::a"], "compile error E0581: boom");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_loop_withdraws_a_rejected_stamp_retires_its_id_and_ends_clean() {
        let dir = std::env::temp_dir().join(format!("yoake-verify-loop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]
name = \"demo-crate\"
version = \"0.1.0\"
edition = \"2024\"
",
        )
        .unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub mod alpha;
").unwrap();
        std::fs::write(
            dir.join("src/alpha.rs"),
            "pub fn good(x: u32) -> u32 {
    x + 1
}

pub fn bad(x: u32) -> u32 {
    x + 2
}
",
        )
        .unwrap();
        let config = Config {
            mode: Mode::Apply,
            crate_root: dir.clone(),
            crate_name: "demo-crate".into(),
            ledger: dir.join("hook-ledger"),
            modules: vec!["alpha".into()],
            shallow: false,
            renames: vec![],
            tombstones: vec![],
            callsites: false,
            thir: None,
            accept_sites: vec![],
            rename_sites: vec![],
            tombstone_sites: vec![],
            engine_crates: None,
            retire_ineligible: false,
            rustfmt: false,
            exclude_modules: vec![],
            min_stmts: 0,
            verify: true,
            verify_args: vec![],
            verify_dir: dir.clone(),
            verify_rounds: 4,
            verify_max_withdraw: 300,
            frozen_ledger: false,
            private_types: false,
            overlay: false,
        };
        let options = VerifyOptions { overlay: None, cargo_args: vec![], dir: dir.clone(), rounds: 4, max_withdraw_per_round: 300 };
        let alpha = dir.join("src/alpha.rs");
        let mut round = 0;
        let result = run_with(&config, &options, || {
            round += 1;
            if round == 1 {
                // the compiler rejects the body of `bad` (its attribute is on the line above the fn)
                let text = std::fs::read_to_string(&alpha).unwrap();
                let line = text.lines().position(|l| l.contains("x + 2")).unwrap() + 1;
                Ok(BuildOutcome {
                    success: false,
                    errors: vec![BuildError { code: "E0581".into(), locations: vec![(alpha.clone(), line)], message: "boom".into() }],
                    tail: String::new(),
                })
            } else {
                Ok(BuildOutcome { success: true, errors: vec![], tail: String::new() })
            }
        });
        assert_eq!(result, Ok(true));
        assert_eq!(round, 2);
        let text = std::fs::read_to_string(&alpha).unwrap();
        assert!(text.contains("key = \"demo_crate::alpha::good\""), "{text}");
        assert!(!text.contains("demo_crate::alpha::bad"), "the rejected stamp is gone: {text}");
        let ledger = std::fs::read_to_string(dir.join("hook-ledger/demo_crate.json")).unwrap();
        let ledger: Value = serde_json::from_str(&ledger).unwrap();
        let status = |key: &str| {
            ledger["entries"].as_array().unwrap().iter().find(|e| e["key"] == key).map(|e| e["status"].as_str().unwrap().to_string())
        };
        assert_eq!(status("demo_crate::alpha::good").as_deref(), Some("live"));
        assert_eq!(status("demo_crate::alpha::bad").as_deref(), Some("tombstone"), "id retired, never reused");
        assert!(ledger["excluded"].as_array().unwrap().iter().any(|e| e["key"] == "demo_crate::alpha::bad" && e["manual"] == true));
        assert!(!dir.join("hook-ledger").join(EXCLUDE_FILE).exists(), "request file removed after a clean build");
        // a plain re-apply keeps `bad` out and changes nothing
        crate::run::execute(&Config { verify: false, ..config_clone(&config) }).unwrap();
        let again = std::fs::read_to_string(&alpha).unwrap();
        assert_eq!(again, text);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn config_clone(c: &Config) -> Config {
        Config {
            mode: Mode::Apply,
            crate_root: c.crate_root.clone(),
            crate_name: c.crate_name.clone(),
            ledger: c.ledger.clone(),
            modules: c.modules.clone(),
            shallow: c.shallow,
            renames: vec![],
            tombstones: vec![],
            callsites: false,
            thir: None,
            accept_sites: vec![],
            rename_sites: vec![],
            tombstone_sites: vec![],
            engine_crates: None,
            retire_ineligible: false,
            rustfmt: false,
            exclude_modules: vec![],
            min_stmts: 0,
            verify: false,
            verify_args: vec![],
            verify_dir: c.verify_dir.clone(),
            verify_rounds: 1,
            verify_max_withdraw: 300,
            frozen_ledger: false,
            private_types: false,
            overlay: false,
        }
    }

    #[test]
    fn a_round_that_would_withdraw_too_many_stamps_aborts_without_withdrawing() {
        let dir = std::env::temp_dir().join(format!("yoake-verify-cap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Cargo.toml"), "[package]
name = \"demo-crate\"
version = \"0.1.0\"
edition = \"2024\"
").unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub mod alpha;
").unwrap();
        std::fs::write(dir.join("src/alpha.rs"), "pub fn a(x: u32) -> u32 {
    x + 1
}

pub fn b(x: u32) -> u32 {
    x + 2
}
").unwrap();
        let mut config = config_clone_for(&dir);
        config.verify = true;
        let options = VerifyOptions { overlay: None, cargo_args: vec![], dir: dir.clone(), rounds: 4, max_withdraw_per_round: 1 };
        let alpha = dir.join("src/alpha.rs");
        let result = run_with(&config, &options, || {
            Ok(BuildOutcome {
                success: false,
                errors: vec![
                    BuildError { code: "E0433".into(), locations: vec![(alpha.clone(), 2)], message: "no crate".into() },
                    BuildError { code: "E0433".into(), locations: vec![(alpha.clone(), 6)], message: "no crate".into() },
                ],
                tail: String::new(),
            })
        });
        let err = result.unwrap_err();
        assert!(err.contains("would withdraw 2 stamps"), "{err}");
        assert!(!dir.join("hook-ledger").join(EXCLUDE_FILE).exists(), "nothing written");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn config_clone_for(dir: &Path) -> Config {
        Config {
            mode: Mode::Apply,
            crate_root: dir.to_path_buf(),
            crate_name: "demo-crate".into(),
            ledger: dir.join("hook-ledger"),
            modules: vec!["alpha".into()],
            shallow: false,
            renames: vec![],
            tombstones: vec![],
            callsites: false,
            thir: None,
            accept_sites: vec![],
            rename_sites: vec![],
            tombstone_sites: vec![],
            engine_crates: None,
            retire_ineligible: false,
            rustfmt: false,
            exclude_modules: vec![],
            min_stmts: 0,
            verify: false,
            verify_args: vec![],
            verify_dir: dir.to_path_buf(),
            verify_rounds: 4,
            verify_max_withdraw: 300,
            frozen_ledger: false,
            private_types: false,
            overlay: false,
        }
    }
}
