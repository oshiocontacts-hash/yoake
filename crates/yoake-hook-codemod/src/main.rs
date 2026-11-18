mod attr;
mod bridge;
mod callsite;
mod edit;
mod eligible;
mod key;
mod ledger_io;
mod modules;
mod run;
mod track_caller;
mod types;

use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Apply,
    Check,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcceptSite {
    pub key: String,
    pub file: String,
    pub line: usize,
    pub col: usize,
}

pub struct Config {
    pub mode: Mode,
    pub crate_root: PathBuf,
    pub crate_name: String,
    pub ledger: PathBuf,
    pub modules: Vec<String>,
    pub renames: Vec<(String, String)>,
    pub tombstones: Vec<String>,
    pub callsites: bool,
    pub thir: Option<PathBuf>,
    pub accept_sites: Vec<AcceptSite>,
    pub rename_sites: Vec<(String, String)>,
    pub tombstone_sites: Vec<String>,
    pub engine_crates: Option<Vec<String>>,
    pub retire_ineligible: bool,
    pub rustfmt: bool,
}

pub struct HistoryConfig {
    pub base: PathBuf,
    pub current: PathBuf,
}

pub struct TrackCallerScanConfig {
    pub workspace: PathBuf,
    pub out: PathBuf,
}

pub struct VerifyCallsitesConfig {
    pub crate_name: String,
    pub ledger: PathBuf,
    pub thir: PathBuf,
}

enum Command {
    Run(Config),
    Bridge(bridge::BridgeConfig),
    CheckHistory(HistoryConfig),
    TrackCallerScan(TrackCallerScanConfig),
    VerifyCallsites(VerifyCallsitesConfig),
}

fn usage() -> &'static str {
    "usage:\n  \
     yoake-hook-codemod <apply|check> --crate-root <dir> --crate-name <name> \
     --ledger <dir> --module <mod> [--module <mod>...] [--rename OLD=NEW]... [--tombstone KEY]... \
     [--callsites] [--thir <calls.json>] [--accept-site KEY@FILE:LINE:COL]... [--rename-site OLD=NEW]... [--tombstone-site KEY]... \
     [--engine-crates LIST]... [--retire-ineligible] [--rustfmt]\n  \
     yoake-hook-codemod bridge --crate-root <dir> --crate-name <name> \
     --ledger <dir> --module <mod> [--module <mod>...] [--glue <file>...] [--exclude-file <file>] [--check]\n  \
     yoake-hook-codemod check-history --base <dir> --current <dir>\n  \
     yoake-hook-codemod track-caller-scan --workspace <dir> --out <file>\n  \
     yoake-hook-codemod verify-callsites --crate-name <name> --ledger <dir> --thir <calls.json>"
}

fn parse_accept_site(raw: &str) -> Result<AcceptSite, String> {
    let err = || format!("--accept-site expects KEY@FILE:LINE:COL, got '{raw}'");
    let (rest, col_str) = raw.rsplit_once(':').ok_or_else(err)?;
    let (rest, line_str) = rest.rsplit_once(':').ok_or_else(err)?;
    let (key, file) = rest.rsplit_once('@').ok_or_else(err)?;
    let line: usize = line_str.parse().map_err(|_| err())?;
    let col: usize = col_str.parse().map_err(|_| err())?;
    if key.is_empty() || file.is_empty() || line == 0 || col == 0 {
        return Err(err());
    }
    Ok(AcceptSite {
        key: key.to_string(),
        file: file.to_string(),
        line,
        col,
    })
}

fn next_arg(args: &mut std::env::Args, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn parse_args() -> Result<Option<Command>, String> {
    let mut args = std::env::args();
    let _bin = args.next();
    let Some(first) = args.next() else {
        return Err(usage().to_string());
    };
    if first == "-h" || first == "--help" {
        println!("{}", usage());
        return Ok(None);
    }

    if first == "bridge" {
        return parse_bridge_args(args);
    }
    if first == "check-history" {
        return parse_check_history_args(&mut args);
    }
    if first == "track-caller-scan" {
        return parse_track_caller_scan_args(&mut args);
    }
    if first == "verify-callsites" {
        return parse_verify_callsites_args(&mut args);
    }

    let mode = match first.as_str() {
        "apply" => Mode::Apply,
        "check" => Mode::Check,
        other => {
            return Err(format!(
                "expected 'apply', 'check', 'bridge', 'check-history', 'track-caller-scan', or 'verify-callsites', got '{other}'\n{}",
                usage()
            ));
        }
    };

    let mut crate_root = None;
    let mut crate_name = None;
    let mut ledger = None;
    let mut modules = Vec::new();
    let mut renames = Vec::new();
    let mut tombstones = Vec::new();
    let mut callsites = false;
    let mut thir = None;
    let mut accept_sites = Vec::new();
    let mut rename_sites = Vec::new();
    let mut tombstone_sites = Vec::new();
    let mut engine_crates: Option<Vec<String>> = None;
    let mut retire_ineligible = false;
    let mut rustfmt = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(None);
            }
            "--crate-root" => crate_root = Some(PathBuf::from(next_arg(&mut args, "--crate-root")?)),
            "--crate-name" => crate_name = Some(next_arg(&mut args, "--crate-name")?),
            "--ledger" => ledger = Some(PathBuf::from(next_arg(&mut args, "--ledger")?)),
            "--module" => modules.push(next_arg(&mut args, "--module")?),
            "--rename" => {
                let raw = next_arg(&mut args, "--rename")?;
                let Some((old_key, new_key)) = raw.split_once('=') else {
                    return Err(format!("--rename expects OLD_KEY=NEW_KEY, got '{raw}'"));
                };
                if old_key.is_empty() || new_key.is_empty() {
                    return Err(format!("--rename keys must be non-empty, got '{raw}'"));
                }
                renames.push((old_key.to_string(), new_key.to_string()));
            }
            "--tombstone" => {
                let key = next_arg(&mut args, "--tombstone")?;
                if key.is_empty() {
                    return Err("--tombstone key must be non-empty".to_string());
                }
                tombstones.push(key);
            }
            "--callsites" => callsites = true,
            "--thir" => thir = Some(PathBuf::from(next_arg(&mut args, "--thir")?)),
            "--accept-site" => {
                let raw = next_arg(&mut args, "--accept-site")?;
                accept_sites.push(parse_accept_site(&raw)?);
            }
            "--rename-site" => {
                let raw = next_arg(&mut args, "--rename-site")?;
                let Some((old_key, new_key)) = raw.split_once('=') else {
                    return Err(format!("--rename-site expects OLD=NEW, got '{raw}'"));
                };
                if old_key.is_empty() || new_key.is_empty() {
                    return Err(format!("--rename-site keys must be non-empty, got '{raw}'"));
                }
                rename_sites.push((old_key.to_string(), new_key.to_string()));
            }
            "--tombstone-site" => {
                let key = next_arg(&mut args, "--tombstone-site")?;
                if key.is_empty() {
                    return Err("--tombstone-site key must be non-empty".to_string());
                }
                tombstone_sites.push(key);
            }
            "--engine-crates" => {
                let raw = next_arg(&mut args, "--engine-crates")?;
                let list = engine_crates.get_or_insert_with(Vec::new);
                for part in raw.split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        list.push(key::normalize_crate_name(trimmed));
                    }
                }
                if list.is_empty() {
                    return Err("--engine-crates requires at least one crate name".to_string());
                }
            }
            "--retire-ineligible" => retire_ineligible = true,
            "--rustfmt" => rustfmt = true,
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }

    let crate_root = crate_root.ok_or_else(|| format!("missing --crate-root\n{}", usage()))?;
    let crate_name = crate_name.ok_or_else(|| format!("missing --crate-name\n{}", usage()))?;
    let ledger = ledger.ok_or_else(|| format!("missing --ledger\n{}", usage()))?;
    if modules.is_empty() {
        return Err(format!("at least one --module is required\n{}", usage()));
    }
    if thir.is_some() && !callsites {
        return Err("--thir requires --callsites".to_string());
    }
    if (!accept_sites.is_empty()
        || !rename_sites.is_empty()
        || !tombstone_sites.is_empty()
        || engine_crates.is_some()
        || retire_ineligible)
        && !callsites
    {
        return Err(
            "--accept-site, --rename-site, --tombstone-site, --engine-crates, and --retire-ineligible require --callsites"
                .to_string(),
        );
    }
    if (!accept_sites.is_empty() || !rename_sites.is_empty() || retire_ineligible) && thir.is_none()
    {
        return Err(
            "--accept-site, --rename-site, and --retire-ineligible require --thir <calls.json>"
                .to_string(),
        );
    }
    if callsites && mode == Mode::Apply && thir.is_none() {
        return Err("apply --callsites requires --thir <calls.json>".to_string());
    }

    Ok(Some(Command::Run(Config {
        mode,
        crate_root,
        crate_name,
        ledger,
        modules,
        renames,
        tombstones,
        callsites,
        thir,
        accept_sites,
        rename_sites,
        tombstone_sites,
        engine_crates,
        retire_ineligible,
        rustfmt,
    })))
}

fn parse_check_history_args(args: &mut std::env::Args) -> Result<Option<Command>, String> {
    let mut base = None;
    let mut current = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(None);
            }
            "--base" => base = Some(PathBuf::from(next_arg(args, "--base")?)),
            "--current" => current = Some(PathBuf::from(next_arg(args, "--current")?)),
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }
    let base = base.ok_or_else(|| format!("missing --base\n{}", usage()))?;
    let current = current.ok_or_else(|| format!("missing --current\n{}", usage()))?;
    Ok(Some(Command::CheckHistory(HistoryConfig { base, current })))
}

fn parse_track_caller_scan_args(args: &mut std::env::Args) -> Result<Option<Command>, String> {
    let mut workspace = None;
    let mut out = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(None);
            }
            "--workspace" => workspace = Some(PathBuf::from(next_arg(args, "--workspace")?)),
            "--out" => out = Some(PathBuf::from(next_arg(args, "--out")?)),
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }
    let workspace = workspace.ok_or_else(|| format!("missing --workspace\n{}", usage()))?;
    let out = out.ok_or_else(|| format!("missing --out\n{}", usage()))?;
    Ok(Some(Command::TrackCallerScan(TrackCallerScanConfig { workspace, out })))
}

fn parse_verify_callsites_args(args: &mut std::env::Args) -> Result<Option<Command>, String> {
    let mut crate_name = None;
    let mut ledger = None;
    let mut thir = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(None);
            }
            "--crate-name" => crate_name = Some(next_arg(args, "--crate-name")?),
            "--ledger" => ledger = Some(PathBuf::from(next_arg(args, "--ledger")?)),
            "--thir" => thir = Some(PathBuf::from(next_arg(args, "--thir")?)),
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }
    let crate_name = crate_name.ok_or_else(|| format!("missing --crate-name\n{}", usage()))?;
    let ledger = ledger.ok_or_else(|| format!("missing --ledger\n{}", usage()))?;
    let thir = thir.ok_or_else(|| format!("missing --thir\n{}", usage()))?;
    Ok(Some(Command::VerifyCallsites(VerifyCallsitesConfig {
        crate_name,
        ledger,
        thir,
    })))
}

fn parse_bridge_args(args: std::env::Args) -> Result<Option<Command>, String> {
    let mut args = args.peekable();
    let mut check = false;
    let mut crate_root = None;
    let mut crate_name = None;
    let mut ledger = None;
    let mut modules = Vec::new();
    let mut glues = Vec::new();
    let mut exclude_file = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(None);
            }
            "--check" => check = true,
            "--crate-root" => {
                let val = args
                    .next()
                    .ok_or_else(|| "missing value for --crate-root".to_string())?;
                crate_root = Some(PathBuf::from(val));
            }
            "--crate-name" => {
                crate_name = Some(
                    args.next()
                        .ok_or_else(|| "missing value for --crate-name".to_string())?,
                );
            }
            "--ledger" => {
                let val = args
                    .next()
                    .ok_or_else(|| "missing value for --ledger".to_string())?;
                ledger = Some(PathBuf::from(val));
            }
            "--module" => {
                let val = args
                    .next()
                    .ok_or_else(|| "missing value for --module".to_string())?;
                modules.push(val);
            }
            "--glue" => {
                let first = args
                    .next()
                    .ok_or_else(|| "missing value for --glue".to_string())?;
                glues.push(PathBuf::from(first));
                while args.peek().is_some_and(|next| !next.starts_with('-')) {
                    glues.push(PathBuf::from(args.next().unwrap()));
                }
            }
            "--exclude-file" => {
                let val = args
                    .next()
                    .ok_or_else(|| "missing value for --exclude-file".to_string())?;
                exclude_file = Some(PathBuf::from(val));
            }
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }

    let crate_root = crate_root.ok_or_else(|| format!("missing --crate-root\n{}", usage()))?;
    let crate_name = crate_name.ok_or_else(|| format!("missing --crate-name\n{}", usage()))?;
    let ledger = ledger.ok_or_else(|| format!("missing --ledger\n{}", usage()))?;
    if modules.is_empty() {
        return Err(format!("at least one --module is required\n{}", usage()));
    }
    Ok(Some(Command::Bridge(bridge::BridgeConfig {
        check,
        crate_root,
        crate_name,
        ledger,
        modules,
        glues,
        exclude_file,
    })))
}

fn main() -> ExitCode {
    let command = match parse_args() {
        Ok(Some(cmd)) => cmd,
        Ok(None) => return ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            return ExitCode::from(2);
        }
    };

    let result = match command {
        Command::Run(config) => run::execute(&config),
        Command::Bridge(config) => bridge::execute(&config),
        Command::CheckHistory(history) => ledger_io::run_check_history(&history.base, &history.current),
        Command::TrackCallerScan(scan) => track_caller::run_scan(&scan.workspace, &scan.out),
        Command::VerifyCallsites(verify) => {
            callsite::verify_callsites(&verify.crate_name, &verify.ledger, &verify.thir)
        }
    };

    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}
