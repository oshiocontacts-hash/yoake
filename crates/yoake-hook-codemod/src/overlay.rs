//! W7: build-time stamping through a rustc wrapper. The repository stays unstamped; `YOAKE_STAMP=1` builds
//! the stamped overlay from the committed ledger and `hook-ledger/stamp-config.txt`.
//!
//! `RUSTC_WORKSPACE_WRAPPER=<this exe>`: cargo runs `<exe> <rustc> <args>` for every workspace crate. In wrapper mode
//! (`run_wrapper`) a crate listed in the stamp config is compiled from an **overlay**: the crate's source closure (module
//! files, `include*!` files, `Cargo.toml`) copied to `<overlay>/<same path relative to the workspace>`, stamped there
//! with a frozen ledger (no new ids: a fn without a ledger id is simply not hooked), and compiled with
//! `--remap-path-prefix` so `file!()`, panic locations and debug info name the original files. Afterwards the
//! dependency file is rewritten to the original paths (cargo's change detection sees the real sources) plus the
//! ledger shard and the stamp config (a ledger change rebuilds the crate).
//!
//! Everything that is not a stamped crate's library root (build scripts, bins, tests of other crates, dependencies
//! that are not workspace members) is passed to rustc unchanged.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Duration, SystemTime};

use crate::{Config, Mode};

pub struct OverlayArgs {
    pub workspace_root: PathBuf,
    pub overlay_root: PathBuf,
    /// Crate directory (holds `Cargo.toml`), absolute or relative to the workspace root.
    pub crate_root: PathBuf,
    pub crate_name: String,
    pub ledger: PathBuf,
    pub stamp_config: PathBuf,
    /// `false`: assign ids and record exclusions in the real ledger (`ledger sync`, `--verify`); `true`: read-only.
    pub frozen: bool,
}

pub struct OverlayResult {
    /// The crate root file inside the overlay.
    pub root_file: PathBuf,
    /// The same file in the workspace.
    pub original_root_file: PathBuf,
    /// Overlay copies of every file of the closure (for the verify loop).
    pub overlay_files: Vec<PathBuf>,
}

pub(crate) fn canon(path: &Path) -> Result<PathBuf, String> {
    let c = path.canonicalize().map_err(|e| format!("cannot resolve {}: {e}", path.display()))?;
    Ok(PathBuf::from(c.to_string_lossy().trim_start_matches("\\\\?\\").to_string()))
}

/// Literal paths of `include_str!("..")`, `include_bytes!("..")` and `include!("..")`.
#[cfg(test)]
pub fn included_files(text: &str, dir: &Path) -> Vec<PathBuf> {
    scan_includes(text, dir).0
}

fn scan_includes(text: &str, dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let mut all = Vec::new();
    let mut code = Vec::new();
    for needle in ["include_str!(", "include_bytes!(", "include!("] {
        let mut from = 0;
        while let Some(rel) = text[from..].find(needle) {
            let at = from + rel + needle.len();
            from = at;
            let rest = text[at..].trim_start();
            if let Some(body) = rest.strip_prefix('"')
                && let Some(end) = body.find('"')
            {
                let path = dir.join(&body[..end]);
                if needle == "include!(" {
                    code.push(path.clone());
                }
                all.push(path);
            }
        }
    }
    (all, code)
}

fn collect_mod_items(
    items: &[syn::Item],
    path_base: &Path,
    dir_for_children: &Path,
    files: &mut BTreeSet<PathBuf>,
    mod_queue: &mut Vec<(PathBuf, bool)>,
) {
    for item in items {
        let syn::Item::Mod(item_mod) = item else { continue };
        let explicit_path = crate::modules::attr_path_value(&item_mod.attrs);
        if let Some((_, inner)) = &item_mod.content {
            let inner_dir = match explicit_path {
                Some(rel) => path_base.join(rel),
                None => dir_for_children.join(item_mod.ident.to_string()),
            };
            collect_mod_items(inner, &inner_dir, &inner_dir, files, mod_queue);
        } else if let Some(rel) = explicit_path {
            let candidate = path_base.join(&rel);
            if candidate.is_file() {
                files.insert(candidate.clone());
                // rustc resolves child mods of a `#[path = "..."]` module relative to that file's directory.
                mod_queue.push((candidate, true));
            }
        } else {
            let name = item_mod.ident.to_string();
            let direct = dir_for_children.join(format!("{name}.rs"));
            let mod_rs = dir_for_children.join(&name).join("mod.rs");
            if direct.is_file() {
                files.insert(direct.clone());
                mod_queue.push((direct, false));
            } else if mod_rs.is_file() {
                files.insert(mod_rs.clone());
                mod_queue.push((mod_rs, true));
            }
        }
    }
}

/// Files the compiler reads for this crate: root file, module files (including `#[cfg(test)]` and `#[path]`),
/// included files, `Cargo.toml`.
pub fn source_closure(crate_root: &Path, crate_name: &str) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let root_file = crate::modules::find_crate_root_file(crate_root, crate_name)?;
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    files.insert(root_file.clone());
    files.insert(crate_root.join("Cargo.toml"));

    let mut visited: BTreeSet<(PathBuf, PathBuf)> = BTreeSet::new();
    let mut mod_queue: Vec<(PathBuf, bool)> = vec![(root_file.clone(), true)];
    let mut include_queue: Vec<(PathBuf, PathBuf)> = Vec::new();
    let mut visited_includes: BTreeSet<PathBuf> = BTreeSet::new();

    while !mod_queue.is_empty() || !include_queue.is_empty() {
        while let Some((file_path, flat_dir)) = mod_queue.pop() {
            let file_dir = file_path.parent().unwrap_or(Path::new("."));
            let is_mod_rs = file_path.file_name().and_then(|s| s.to_str()) == Some("mod.rs");
            let dir_for_children = if flat_dir || is_mod_rs {
                file_dir.to_path_buf()
            } else {
                let stem = file_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                file_dir.join(stem)
            };
            let canonical = file_path.canonicalize().unwrap_or_else(|_| file_path.clone());
            if !visited.insert((canonical, dir_for_children.clone())) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file_path) else { continue };
            let (all_inc, code_inc) = scan_includes(&text, file_dir);
            for p in all_inc {
                if p.is_file() {
                    files.insert(p);
                }
            }
            for p in code_inc {
                if p.is_file() {
                    include_queue.push((p, dir_for_children.clone()));
                }
            }
            if let Ok(parsed) = syn::parse_file(&text) {
                collect_mod_items(&parsed.items, file_dir, &dir_for_children, &mut files, &mut mod_queue);
            }
        }
        while let Some((inc_path, parent_dir_for_children)) = include_queue.pop() {
            let canonical = inc_path.canonicalize().unwrap_or_else(|_| inc_path.clone());
            if !visited_includes.insert(canonical) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&inc_path) else { continue };
            let inc_dir = inc_path.parent().unwrap_or(Path::new("."));
            let (all_inc, code_inc) = scan_includes(&text, inc_dir);
            for p in all_inc {
                if p.is_file() {
                    files.insert(p);
                }
            }
            for p in code_inc {
                if p.is_file() {
                    include_queue.push((p, parent_dir_for_children.clone()));
                }
            }
            if let Ok(parsed) = syn::parse_file(&text) {
                collect_mod_items(&parsed.items, inc_dir, &parent_dir_for_children, &mut files, &mut mod_queue);
            }
        }
    }

    Ok((root_file, files.into_iter().collect()))
}

fn stat_line(path: &Path) -> String {
    match std::fs::metadata(path) {
        Ok(m) => {
            let t = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d: Duration| d.as_nanos());
            format!("{}\t{}\t{t}", path.display(), m.len())
        }
        Err(_) => format!("{}\tmissing", path.display()),
    }
}

/// Size and time of every overlay file of a crate, as it was when the crate's overlay was written (F1 gate).
fn overlay_snapshot(files: &[PathBuf]) -> String {
    let mut text = String::new();
    for f in files {
        text.push_str(&stat_line(f));
        text.push(char::from(10));
    }
    text
}

/// Builds (or reuses) the overlay of one crate.
pub fn build_overlay(args: &OverlayArgs) -> Result<OverlayResult, String> {
    let workspace = canon(&args.workspace_root)?;
    let crate_root = if args.crate_root.is_absolute() { args.crate_root.clone() } else { workspace.join(&args.crate_root) };
    let crate_root = canon(&crate_root)?;
    let crate_rel = crate_root
        .strip_prefix(&workspace)
        .map_err(|_| format!("crate {} is outside the workspace", crate_root.display()))?
        .to_path_buf();
    let (root_file, files) = source_closure(&crate_root, &args.crate_name)?;
    let root_file = canon(&root_file)?;
    let mut rels = Vec::new();
    let mut seen = BTreeSet::new();
    for f in &files {
        let f = canon(f)?;
        if !seen.insert(f.clone()) {
            continue;
        }
        let rel = f
            .strip_prefix(&workspace)
            .map_err(|_| format!("{} is outside the workspace; the overlay cannot hold it", f.display()))?
            .to_path_buf();
        rels.push((f, rel));
    }
    let overlay_files: Vec<PathBuf> = rels.iter().map(|(_, rel)| args.overlay_root.join(rel)).collect();
    let root_rel = root_file.strip_prefix(&workspace).map_err(|_| "root file outside workspace".to_string())?;
    let result = OverlayResult {
        root_file: args.overlay_root.join(root_rel),
        original_root_file: root_file.clone(),
        overlay_files,
    };

    // Fast path: nothing the stamping reads has changed since the last run.
    let ident = crate::key::normalize_crate_name(&args.crate_name);
    let state_dir = args.overlay_root.join(".state");
    let state_file = state_dir.join(format!("{ident}.{}.txt", if args.frozen { "frozen" } else { "sync" }));
    let snapshot_file = state_dir.join(format!("{ident}.overlay.txt"));
    let mut state = String::new();
    for (f, _) in &rels {
        state.push_str(&stat_line(f));
        state.push('\n');
    }
    // the committed field-table glue the bridges are built from
    let tables_dir = root_file.parent().map(|p| p.join("yoake_glue").join("tables").join(&ident));
    if let Some(dir) = &tables_dir
        && let Ok(entries) = std::fs::read_dir(dir)
    {
        let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        names.sort();
        for p in names {
            state.push_str(&stat_line(&p));
            state.push(char::from(10));
        }
    }
    for extra in [
        args.ledger.join(format!("{ident}.json")),
        args.ledger.join("meta.json"),
        args.ledger.join("stamp-allow.txt"),
        args.ledger.join("stamp-exclude.txt"),
        args.ledger.join("hot-exclusions.txt"),
        args.stamp_config.clone(),
        std::env::current_exe().unwrap_or_default(),
    ] {
        state.push_str(&stat_line(&extra));
        state.push('\n');
    }
    if !args.frozen {
        // a sync run always runs: it is how ids are assigned
    } else if std::fs::read_to_string(&state_file).is_ok_and(|old| old == state)
        && result.root_file.is_file()
        && std::fs::read_to_string(&snapshot_file).is_ok_and(|old| old == overlay_snapshot(&result.overlay_files))
    {
        // the inputs are as they were AND the overlay files are as this crate left them: another crate's overlay
        // that shares a source file (stamped differently, or an include_str! of it) may have replaced one since
        return Ok(result);
    }

    for (src, rel) in &rels {
        let dest = args.overlay_root.join(rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
        }
        // A file of ANOTHER crate that this crate only reads (`include_str!("../../../horizon-world/src/loot.rs")` in a
        // test of horizon-client) is a plain copy: it must never replace the stamped copy its own crate's overlay made
        // (the lost-bridge-include bug of the FR-F1 gate: horizon-world's loot.rs came back unstamped while its bridge
        // file and the chain call stayed, so the next build of horizon-world failed). It is copied when absent only.
        let own_tree = src.starts_with(&crate_root) || root_file.parent().is_some_and(|dir| src.starts_with(dir));
        if dest.exists() && !own_tree {
            continue;
        }
        std::fs::copy(src, &dest).map_err(|e| format!("cannot copy {} to {}: {e}", src.display(), dest.display()))?;
    }

    let mut modules = Vec::new();
    let mut exclude = Vec::new();
    let mut min_stmts = 0usize;
    let mut private_types = false;
    let mut bridges = false;
    crate::apply_stamp_config(&args.stamp_config, &args.crate_name, &mut modules, &mut exclude, &mut min_stmts, &mut private_types, &mut bridges)?;
    let config = Config {
        mode: Mode::Apply,
        crate_root: args.overlay_root.join(&crate_rel),
        crate_name: args.crate_name.clone(),
        ledger: args.ledger.clone(),
        modules,
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
        exclude_modules: exclude.clone(),
        min_stmts,
        verify: false,
        verify_args: vec![],
        verify_dir: PathBuf::from("."),
        verify_rounds: 1,
        verify_max_withdraw: 0,
        frozen_ledger: args.frozen,
        private_types,
        overlay: false,
    };
    crate::run::execute(&config)?;
    if bridges {
        let workspace_src = root_file.parent().ok_or("crate root has no parent")?.to_path_buf();
        crate::overlay_bridge::add_bridges(&args.overlay_root.join(&crate_rel), &args.crate_name, &args.ledger, &workspace_src, &exclude)?;
    }

    std::fs::create_dir_all(&state_dir).map_err(|e| format!("cannot create {}: {e}", state_dir.display()))?;
    if args.frozen {
        // the state describes inputs as they were when the overlay was written
        std::fs::write(&state_file, state).map_err(|e| format!("cannot write {}: {e}", state_file.display()))?;
        std::fs::write(&snapshot_file, overlay_snapshot(&result.overlay_files)).map_err(|e| format!("cannot write {}: {e}", snapshot_file.display()))?;
    }
    Ok(result)
}

struct Lock(PathBuf);
impl Lock {
    fn take(path: PathBuf) -> Lock {
        for _ in 0..600 {
            if std::fs::create_dir(&path).is_ok() {
                return Lock(path);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = std::fs::remove_dir(&path);
        let _ = std::fs::create_dir(&path);
        Lock(path)
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir(&self.0);
    }
}

pub fn is_wrapper_invocation(args: &[String]) -> bool {
    args.get(1).is_some_and(|a| {
        Path::new(a).file_stem().is_some_and(|s| s.eq_ignore_ascii_case("rustc"))
    })
}

fn passthrough(rustc: &str, rest: &[String]) -> ExitCode {
    match Command::new(rustc).args(rest).status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(e) => {
            eprintln!("yoake-stamp: cannot run {rustc}: {e}");
            ExitCode::from(1)
        }
    }
}

fn arg_value(rest: &[String], flag: &str) -> Option<String> {
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        if a == flag {
            return it.next().cloned();
        }
        if let Some(v) = a.strip_prefix(&format!("{flag}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn find_input(rest: &[String]) -> Option<usize> {
    let valued = ["--crate-name", "--crate-type", "--emit", "--out-dir", "-C", "-L", "--extern", "--cfg", "--check-cfg", "--error-format", "--json", "--target", "--edition", "-A", "-W", "-D", "-F", "--cap-lints", "--remap-path-prefix", "-l", "--cap-lints", "--diagnostic-width", "--color", "-Z"];
    let mut i = 0;
    while i < rest.len() {
        let a = &rest[i];
        if valued.contains(&a.as_str()) {
            i += 2;
            continue;
        }
        if !a.starts_with('-') && a.ends_with(".rs") {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn find_workspace(start: &Path) -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("YOAKE_WORKSPACE") {
        return Some(PathBuf::from(root));
    }
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join("hook-ledger").join("stamp-config.txt").is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// `@file` arguments replaced by the lines of the file (cargo's argfiles: one argument per line).
pub fn expand_argfiles(args: &[String]) -> Vec<String> {
    args.iter()
        .flat_map(|a| match a.strip_prefix('@') {
            Some(file) => match std::fs::read_to_string(file) {
                Ok(text) => text.lines().map(|l| l.trim_end_matches(char::from(13)).to_string()).collect::<Vec<_>>(),
                Err(_) => vec![a.clone()],
            },
            None => vec![a.clone()],
        })
        .collect()
}

/// `<exe> <rustc> <args...>`: see the module comment.
pub fn run_wrapper(args: Vec<String>) -> ExitCode {
    crate::run::set_quiet(true);
    let debug = std::env::var("YOAKE_STAMP_DEBUG").as_deref() == Ok("1");
    if debug {
        eprintln!("yoake-stamp: wrapper args: {}", args[2..].join(" "));
    }
    let rustc = args[1].clone();
    let original_rest: Vec<String> = args[2..].to_vec();
    // cargo passes long command lines (Windows: the libraries with hundreds of `--extern`/`-L`) as `@argfile`, one
    // argument per line; the crate name and the input file are inside it
    let rest: Vec<String> = expand_argfiles(&original_rest);
    if std::env::var("YOAKE_STAMP").as_deref() != Ok("1") {
        return passthrough(&rustc, &original_rest);
    }
    let Some(crate_name) = arg_value(&rest, "--crate-name") else { return passthrough(&rustc, &original_rest) };
    if crate_name.starts_with("build_script_") {
        return passthrough(&rustc, &original_rest);
    }
    let Some(input_idx) = find_input(&rest) else {
        if debug { eprintln!("yoake-stamp: no input file"); }
        return passthrough(&rustc, &original_rest);
    };
    let Ok(cwd) = std::env::current_dir() else { return passthrough(&rustc, &original_rest) };
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from).unwrap_or_else(|| cwd.clone());
    let Some(workspace) = find_workspace(&manifest_dir) else {
        if debug { eprintln!("yoake-stamp: no workspace from {}", manifest_dir.display()); }
        return passthrough(&rustc, &original_rest);
    };
    let config_path = workspace.join("hook-ledger").join("stamp-config.txt");
    let Ok(config_text) = std::fs::read_to_string(&config_path) else { return passthrough(&rustc, &original_rest) };
    let wanted = crate::key::normalize_crate_name(&crate_name);
    let Some(crate_dir) = config_text.lines().find_map(|raw| {
        let line = raw.split('#').next().unwrap_or("").trim();
        let parts: Vec<&str> = line.split_whitespace().collect();
        (parts.len() >= 3 && crate::key::normalize_crate_name(parts[1]) == wanted).then(|| workspace.join(parts[0]))
    }) else {
        return passthrough(&rustc, &original_rest);
    };
    // only the crate's library root is stamped (bins, examples and tests of the crate keep their own roots)
    let input = cwd.join(&rest[input_idx]);
    let Ok(root_file) = crate::modules::find_crate_root_file(&crate_dir, &crate_name) else { return passthrough(&rustc, &original_rest) };
    match (canon(&input), canon(&root_file)) {
        (Ok(a), Ok(b)) if a == b => {}
        (a, b) => {
            if debug { eprintln!("yoake-stamp: {crate_name}: input {a:?} is not the library root {b:?}"); }
            return passthrough(&rustc, &original_rest);
        }
    }

    let stamp_dir = std::env::var_os("YOAKE_STAMP_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace.join("target").join("yoake-stamp"));
    let _ = std::fs::create_dir_all(&stamp_dir);
    let stamp_dir = canon(&stamp_dir).unwrap_or(stamp_dir);
    let overlay_root = stamp_dir.join("root");
    let _lock = Lock::take(stamp_dir.join(format!(".lock-{wanted}")));
    let overlay_args = OverlayArgs {
        workspace_root: workspace.clone(),
        overlay_root: overlay_root.clone(),
        crate_root: crate_dir.clone(),
        crate_name: crate_name.clone(),
        ledger: workspace.join("hook-ledger"),
        stamp_config: config_path.clone(),
        frozen: true,
    };
    let result = match build_overlay(&overlay_args) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("yoake-stamp: {crate_name} is built UNSTAMPED: {e}");
            return passthrough(&rustc, &original_rest);
        }
    };

    // The overlay path is passed absolute and its prefix is remapped to nothing: rustc then prints
    // `crates\x\src\lib.rs` for `file!()`, panic locations, debug info and diagnostics, exactly like the
    // unstamped build (measured with rustc 1.100.0: a relative input would print forward slashes, and a
    // remap target of "." a leading `.\`).
    let cwd_c = canon(&cwd).unwrap_or(cwd.clone());
    let mut new_args = rest.clone();
    new_args.push("--cfg".into());
    new_args.push("yoake_stamped".into());
    new_args[input_idx] = result.root_file.to_string_lossy().into_owned();
    new_args.push("--remap-path-prefix".into());
    new_args.push(format!("{}=", overlay_root.display()));

    let total: usize = new_args.iter().map(|a| a.len() + 1).sum();
    let status = if total > 16000 {
        // too long for a Windows command line: hand rustc an argfile of its own
        let argfile = stamp_dir.join(format!(".args-{wanted}-{}.txt", std::process::id()));
        if let Err(e) = std::fs::write(&argfile, new_args.join(&char::from(10).to_string())) {
            eprintln!("yoake-stamp: cannot write {}: {e}", argfile.display());
            return passthrough(&rustc, &original_rest);
        }
        let status = Command::new(&rustc).arg(format!("@{}", argfile.display())).status();
        let _ = std::fs::remove_file(&argfile);
        status
    } else {
        Command::new(&rustc).args(&new_args).status()
    };
    let code = match status {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("yoake-stamp: cannot run {rustc}: {e}");
            return ExitCode::from(1);
        }
    };
    if code == 0 {
        let ident = crate::key::normalize_crate_name(&crate_name);
        let out_dir = arg_value(&rest, "--out-dir");
        let extra = rest.windows(2).find_map(|w| (w[0] == "-C").then(|| w[1].strip_prefix("extra-filename=").map(str::to_string)).flatten())
            .or_else(|| rest.iter().find_map(|a| a.strip_prefix("-Cextra-filename=").map(str::to_string)))
            .unwrap_or_default();
        if let Some(out_dir) = out_dir {
            let dep = Path::new(&out_dir).join(format!("{crate_name}{extra}.d"));
            let _ = rewrite_dep_info(&dep, &overlay_root, &cwd_c, &workspace, &ident);
        }
    }
    ExitCode::from(code as u8)
}

/// Rewrites a rustc dependency file: overlay paths become the original source paths and the ledger shard,
/// the stamp config and the allow list are added as dependencies.
pub fn rewrite_dep_info(dep: &Path, overlay_root: &Path, cwd: &Path, workspace: &Path, ident: &str) -> Result<(), String> {
    let Ok(text) = std::fs::read_to_string(dep) else { return Ok(()) };
    let forms = |p: &Path| -> Vec<(String, String)> {
        let parts: Vec<String> = p
            .components()
            .filter(|c| !matches!(c, std::path::Component::RootDir))
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let prefix_root = p.has_root();
        let mut out = Vec::new();
        for sep in ["\\", "/"] {
            let mut joined = parts.join(sep);
            if prefix_root && parts.first().is_some_and(|f| !f.ends_with(':')) {
                joined = format!("{sep}{joined}");
            }
            out.push((format!("{joined}{sep}"), sep.to_string()));
        }
        out
    };
    let mut text = text;
    let workspace_forms = forms(workspace);
    if let Ok(rel) = overlay_root.strip_prefix(cwd) {
        for (form, _) in forms(rel) {
            text = text.replace(&form, "");
        }
    }
    for ((from, _), (to, _)) in forms(overlay_root).into_iter().zip(workspace_forms) {
        text = text.replace(&from, &to);
    }
    // The generated bridge files exist only in the overlay: after the paths are mapped to the workspace they name files
    // that are not there, and cargo then calls the crate dirty ("the file is missing") on every build. They are
    // functions of the sources and the ledger, which are dependencies already.
    let is_generated = |token: &str| token.contains("yoake_glue/hook_bridge_") || token.contains("yoake_glue\\hook_bridge_");
    let text = text
        .lines()
        .filter(|line| !(line.ends_with(':') && is_generated(line)))
        .map(|line| {
            if line.contains(": ") {
                line.split(' ').filter(|t| !is_generated(t)).collect::<Vec<_>>().join(" ")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(char::from(10).to_string().as_str())
        + char::from(10).to_string().as_str();
    let ledger = workspace.join("hook-ledger");
    let mut extra_deps: Vec<String> = Vec::new();
    for name in [format!("{ident}.json"), "stamp-config.txt".to_string(), "stamp-allow.txt".to_string()] {
        let path = ledger.join(&name);
        if path.is_file() {
            let shown = path.strip_prefix(cwd).map_or_else(|_| path.to_string_lossy().into_owned(), |p| p.to_string_lossy().into_owned());
            extra_deps.push(shown);
        }
    }
    if extra_deps.is_empty() {
        return std::fs::write(dep, text).map_err(|e| e.to_string());
    }
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if let Some(first) = lines.iter_mut().find(|l| l.contains(": ")) {
        for d in &extra_deps {
            first.push(' ');
            first.push_str(d);
        }
    }
    for d in &extra_deps {
        lines.push(format!("{d}:"));
    }
    std::fs::write(dep, lines.join("\n") + "\n").map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn included_files_finds_literal_paths_only() {
        let text = "const A: &str = include_str!(\"../data/a.txt\");\nstatic B: &[u8] = include_bytes!(\"b.bin\");\ninclude!(concat!(env!(\"OUT_DIR\"), \"/x.rs\"));\ninclude!(\"gen/c.rs\");\n";
        let found = included_files(text, Path::new("/w/src"));
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found.contains(&PathBuf::from("/w/src/../data/a.txt")));
        assert!(found.contains(&PathBuf::from("/w/src/gen/c.rs")));
    }

    #[test]
    fn wrapper_invocations_are_recognised_by_the_rustc_argument() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(is_wrapper_invocation(&a(&["yoake-hook-codemod", "rustc", "--crate-name", "x"])));
        assert!(is_wrapper_invocation(&a(&["w", "C:\\tc\\bin\\rustc.exe", "-vV"])));
        assert!(!is_wrapper_invocation(&a(&["w", "apply", "--crate-root", "x"])));
        assert!(!is_wrapper_invocation(&a(&["w"])));
    }

    #[test]
    fn the_input_file_is_the_first_positional_rs_argument() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let rest = a(&["--crate-name", "x", "--edition=2024", "crates\\x\\src\\lib.rs", "--crate-type", "lib", "-C", "extra-filename=-1", "--out-dir", "t"]);
        assert_eq!(find_input(&rest), Some(3));
        assert_eq!(arg_value(&rest, "--crate-name").as_deref(), Some("x"));
        assert_eq!(arg_value(&rest, "--edition").as_deref(), Some("2024"));
        assert_eq!(find_input(&a(&["-vV"])), None);
    }

    #[test]
    fn dep_info_gets_original_paths_and_the_ledger_as_dependencies() {
        let dir = std::env::temp_dir().join(format!("yoake-depinfo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("hook-ledger")).unwrap();
        std::fs::write(dir.join("hook-ledger/demo.json"), "{}").unwrap();
        std::fs::write(dir.join("hook-ledger/stamp-config.txt"), "").unwrap();
        let overlay = dir.join("target/yoake-stamp/root");
        let sep = std::path::MAIN_SEPARATOR;
        let rel = format!("target{sep}yoake-stamp{sep}root");
        let dep = dir.join("x.d");
        std::fs::write(&dep, format!("out.rlib: {rel}{sep}crates{sep}demo{sep}src{sep}lib.rs\n\n{rel}{sep}crates{sep}demo{sep}src{sep}lib.rs:\n")).unwrap();
        rewrite_dep_info(&dep, &overlay, &dir, &dir, "demo").unwrap();
        let out = std::fs::read_to_string(&dep).unwrap();
        assert!(!out.contains("yoake-stamp"), "{out}");
        assert!(out.contains(&format!("crates{sep}demo{sep}src{sep}lib.rs")), "{out}");
        assert!(out.lines().next().unwrap().contains("demo.json"), "{out}");
        assert!(out.contains("stamp-config.txt:"), "{out}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn source_closure_resolves_cfg_test_and_path_modules_like_rustc() {
        let dir = std::env::temp_dir().join(format!("yoake-closure-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let crate_dir = dir.join("crates/demo");
        let src = crate_dir.join("src");
        std::fs::create_dir_all(src.join("wasm_mods/full")).unwrap();
        std::fs::create_dir_all(src.join("nested")).unwrap();
        std::fs::create_dir_all(src.join("plain/inline_sub")).unwrap();
        std::fs::create_dir_all(dir.join("tests/support")).unwrap();

        std::fs::write(crate_dir.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "mod entry;\nmod nested;\nmod plain;\n#[cfg(test)]\nmod unit_support;\n#[cfg(test)]\n#[path = \"../../../tests/support/temp_dir.rs\"]\nmod test_support;\n",
        )
        .unwrap();
        std::fs::write(src.join("entry.rs"), "#[path = \"wasm_mods.rs\"]\nmod wasm_mods;\n").unwrap();
        std::fs::write(src.join("wasm_mods.rs"), "#[path = \"wasm_mods/full.rs\"]\nmod full;\n").unwrap();
        std::fs::write(src.join("wasm_mods/full.rs"), "#[cfg(test)]\n#[path = \"full/gpu_tests.rs\"]\nmod gpu_tests;\n").unwrap();
        std::fs::write(src.join("wasm_mods/full/gpu_tests.rs"), "#[test]\nfn t() {}\n").unwrap();
        std::fs::write(src.join("nested/mod.rs"), "#[cfg(test)]\n#[path = \"nested_tests.rs\"]\nmod tests;\n").unwrap();
        std::fs::write(src.join("nested/nested_tests.rs"), "#[test]\nfn t() {}\n").unwrap();
        std::fs::write(
            src.join("plain.rs"),
            "#[cfg(test)]\n#[path = \"plain/sub_tests.rs\"]\nmod sub_tests;\nmod inline_sub {\n    #[cfg(test)]\n    #[path = \"in_tests.rs\"]\n    mod t;\n}\n",
        )
        .unwrap();
        std::fs::write(src.join("plain/sub_tests.rs"), "#[test]\nfn t() {}\n").unwrap();
        std::fs::write(src.join("plain/inline_sub/in_tests.rs"), "#[test]\nfn t() {}\n").unwrap();
        std::fs::write(src.join("unit_support.rs"), "pub fn ok() {}\n").unwrap();
        std::fs::write(dir.join("tests/support/temp_dir.rs"), "pub fn tmp() {}\n").unwrap();

        let (_, files) = source_closure(&crate_dir, "demo").unwrap();
        let has = |p: PathBuf| {
            let c = canon(&p).unwrap();
            files.iter().any(|f| canon(f).ok().as_ref() == Some(&c))
        };
        assert!(has(src.join("wasm_mods/full/gpu_tests.rs")), "full/gpu_tests.rs missing: {files:?}");
        assert!(has(src.join("nested/nested_tests.rs")), "nested_tests.rs missing: {files:?}");
        assert!(has(src.join("plain/sub_tests.rs")), "plain/sub_tests.rs missing: {files:?}");
        assert!(has(src.join("plain/inline_sub/in_tests.rs")), "in_tests.rs missing: {files:?}");
        assert!(has(src.join("unit_support.rs")), "unit_support.rs missing: {files:?}");
        assert!(has(dir.join("tests/support/temp_dir.rs")), "foreign temp_dir.rs missing: {files:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_overlay_copies_path_modules_keeps_foreign_when_present_and_snapshots_them() {
        let dir = std::env::temp_dir().join(format!("yoake-overlay-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let crate_dir = dir.join("crates/demo");
        let src = crate_dir.join("src");
        let ledger = dir.join("hook-ledger");
        let overlay = dir.join("overlay");
        std::fs::create_dir_all(src.join("wasm_mods/full")).unwrap();
        std::fs::create_dir_all(dir.join("tests/support")).unwrap();
        std::fs::create_dir_all(&ledger).unwrap();
        std::fs::create_dir_all(overlay.join("tests/support")).unwrap();

        std::fs::write(crate_dir.join("Cargo.toml"), "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n").unwrap();
        std::fs::write(
            src.join("lib.rs"),
            "#[path = \"wasm_mods.rs\"]\nmod wasm_mods;\n#[cfg(test)]\n#[path = \"../../../tests/support/temp_dir.rs\"]\nmod test_support;\n",
        )
        .unwrap();
        std::fs::write(src.join("wasm_mods.rs"), "#[path = \"wasm_mods/full.rs\"]\nmod full;\n").unwrap();
        std::fs::write(src.join("wasm_mods/full.rs"), "#[cfg(test)]\n#[path = \"full/gpu_tests.rs\"]\nmod gpu_tests;\n").unwrap();
        std::fs::write(src.join("wasm_mods/full/gpu_tests.rs"), "pub fn original_test() {}\n").unwrap();
        std::fs::write(dir.join("tests/support/temp_dir.rs"), "pub fn plain_foreign() {}\n").unwrap();
        std::fs::write(overlay.join("tests/support/temp_dir.rs"), "pub fn stamped_foreign() {}\n").unwrap();

        std::fs::write(ledger.join("meta.json"), "{\"version\":2,\"next_id\":1,\"reserved\":[]}\n").unwrap();
        std::fs::write(ledger.join("demo.json"), "{\"crate\":\"demo\",\"entries\":[]}\n").unwrap();
        let stamp_config = ledger.join("stamp-config.txt");
        std::fs::write(&stamp_config, "crates/demo demo * min_stmts=0\n").unwrap();

        let args = OverlayArgs {
            workspace_root: dir.clone(),
            overlay_root: overlay.clone(),
            crate_root: PathBuf::from("crates/demo"),
            crate_name: "demo".to_string(),
            ledger: ledger.clone(),
            stamp_config,
            frozen: true,
        };
        build_overlay(&args).unwrap();

        let overlay_gpu_tests = overlay.join("crates/demo/src/wasm_mods/full/gpu_tests.rs");
        assert_eq!(std::fs::read_to_string(&overlay_gpu_tests).unwrap(), "pub fn original_test() {}\n");
        assert_eq!(
            std::fs::read_to_string(overlay.join("tests/support/temp_dir.rs")).unwrap(),
            "pub fn stamped_foreign() {}\n",
            "foreign #[path] file already in overlay must follow copy-only-when-absent"
        );

        // Tampering with the overlay copy invalidates the snapshot check and restores it on rebuild.
        std::fs::write(&overlay_gpu_tests, "pub fn tampered() {}\n").unwrap();
        build_overlay(&args).unwrap();
        assert_eq!(std::fs::read_to_string(&overlay_gpu_tests).unwrap(), "pub fn original_test() {}\n");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
