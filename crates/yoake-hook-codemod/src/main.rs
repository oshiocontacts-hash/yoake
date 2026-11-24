mod attr;
mod bridge;
mod callsite;
mod callsite_patch;
mod edit;
mod eligible;
mod key;
mod ledger_io;
mod modules;
mod overlay;
mod overlay_bridge;
mod run;
mod track_caller;
mod types;
mod verify;

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
    pub shallow: bool,
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
    /// W5: `--exclude-module NAME` (with `--module '*'`), `--min-stmts N`.
    pub exclude_modules: Vec<String>,
    pub min_stmts: usize,
    /// W1: `apply --verify [--verify-arg A]... [--verify-dir D] [--verify-rounds N]`.
    pub verify: bool,
    pub verify_args: Vec<String>,
    pub verify_dir: PathBuf,
    pub verify_rounds: usize,
    pub verify_max_withdraw: usize,
    /// W7: `--frozen-ledger`: read-only ledger (see `run::ScanCtx::frozen`).
    pub frozen_ledger: bool,
    /// W4: `--private-types` (stamp-config `private_types=true`).
    pub private_types: bool,
    /// W7: `--overlay`: `--verify` stamps and builds through the overlay wrapper, the tree stays unstamped.
    pub overlay: bool,
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
    SyncLedger(SyncLedgerConfig),
    CallsitePatch(callsite_patch::GenerateArgs),
}

/// W7: `sync-ledger --workspace-root W [--stamp-config F] [--ledger D] [--overlay-root O] [--crate NAME]...`:
/// stamps every configured crate in a scratch overlay with a writable ledger, so ids and exclusions land in
/// `hook-ledger/` while the sources stay untouched.
pub struct SyncLedgerConfig {
    pub workspace_root: PathBuf,
    pub stamp_config: PathBuf,
    pub ledger: PathBuf,
    pub overlay_root: PathBuf,
    pub only: Vec<String>,
    /// `--frozen`: build the overlays exactly as the compiler wrapper does (read-only ledger), without a build.
    pub frozen: bool,
    /// `--rename OLD=NEW` / `--tombstone KEY`: drift fixes applied while the overlay is stamped (sources untouched).
    pub renames: Vec<(String, String)>,
    pub tombstones: Vec<String>,
}

fn usage() -> &'static str {
    "usage:\n  \
     yoake-hook-codemod <apply|check> --crate-root <dir> --crate-name <name> \
     --ledger <dir> --module <mod> [--module <mod>...] [--shallow] [--rename OLD=NEW]... [--tombstone KEY]... \
     [--callsites] [--thir <calls.json>] [--accept-site KEY@FILE:LINE:COL]... [--rename-site OLD=NEW]... [--tombstone-site KEY]... \
     [--engine-crates LIST]... [--retire-ineligible] [--rustfmt] \
     [--exclude-module NAME]... [--min-stmts N] [--stamp-config FILE] (--module '*' = every top-level module)\n  \
     yoake-hook-codemod bridge --crate-root <dir> --crate-name <name> \
     --ledger <dir> --module <mod> [--module <mod>...] [--shallow] [--glue <file>...] [--exclude-file <file>] [--check]\n  \
     yoake-hook-codemod sync-ledger --workspace-root <dir> [--crate NAME]... [--rename OLD=NEW]... [--tombstone KEY]... [--frozen]\n  \
     yoake-hook-codemod callsite-patch --workspace-root <dir> --crate NAME --thir <calls.json> [--stamp-config F] [--ledger D] [--scratch DIR]\n  \
     yoake-hook-codemod check-history --base <dir> --current <dir>\n  \
     yoake-hook-codemod track-caller-scan --workspace <dir> --out <file>\n  \
     yoake-hook-codemod verify-callsites --crate-name <name> --ledger <dir> --thir <calls.json>\n  \
     --shallow: only include each --module's own items (exclude inline mod blocks and child modules)"
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
    if first == "sync-ledger" {
        return parse_sync_ledger_args(&mut args);
    }
    if first == "callsite-patch" {
        return parse_callsite_patch_args(&mut args);
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
    let mut shallow = false;
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
    let mut exclude_modules: Vec<String> = Vec::new();
    let mut min_stmts: usize = 0;
    let mut verify = false;
    let mut verify_args: Vec<String> = Vec::new();
    let mut verify_dir = PathBuf::from(".");
    let mut verify_rounds: usize = 8;
    let mut verify_max_withdraw: usize = 300;
    let mut frozen_ledger = false;
    let mut private_types = false;
    let mut bridges_unused = false;
    let mut overlay = false;
    let mut stamp_config: Option<PathBuf> = None;

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
            "--shallow" => shallow = true,
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
            "--exclude-module" => exclude_modules.push(next_arg(&mut args, "--exclude-module")?),
            "--min-stmts" => {
                let raw = next_arg(&mut args, "--min-stmts")?;
                min_stmts = raw.parse().map_err(|_| format!("--min-stmts expects a number, got '{raw}'"))?;
            }
            "--verify" => verify = true,
            "--frozen-ledger" => frozen_ledger = true,
            "--private-types" => private_types = true,
            "--overlay" => overlay = true,
            "--verify-arg" => verify_args.push(next_arg(&mut args, "--verify-arg")?),
            "--verify-dir" => verify_dir = PathBuf::from(next_arg(&mut args, "--verify-dir")?),
            "--verify-rounds" => {
                let raw = next_arg(&mut args, "--verify-rounds")?;
                verify_rounds = raw.parse().map_err(|_| format!("--verify-rounds expects a number, got '{raw}'"))?;
            }
            "--verify-max-withdraw" => {
                let raw = next_arg(&mut args, "--verify-max-withdraw")?;
                verify_max_withdraw = raw.parse().map_err(|_| format!("--verify-max-withdraw expects a number, got '{raw}'"))?;
            }
            "--stamp-config" => stamp_config = Some(PathBuf::from(next_arg(&mut args, "--stamp-config")?)),
            other => return Err(format!("unknown argument '{other}'\n{}", usage())),
        }
    }

    let crate_root = crate_root.ok_or_else(|| format!("missing --crate-root\n{}", usage()))?;
    let crate_name = crate_name.ok_or_else(|| format!("missing --crate-name\n{}", usage()))?;
    let ledger = ledger.ok_or_else(|| format!("missing --ledger\n{}", usage()))?;
    if let Some(path) = &stamp_config {
        apply_stamp_config(path, &crate_name, &mut modules, &mut exclude_modules, &mut min_stmts, &mut private_types, &mut bridges_unused)?;
    }
    if modules.is_empty() {
        return Err(format!("at least one --module is required\n{}", usage()));
    }
    if overlay && !verify {
        return Err("--overlay applies to --verify".to_string());
    }
    if verify && (mode != Mode::Apply || callsites) {
        return Err("--verify applies to `apply` without --callsites".to_string());
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
        shallow,
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
        exclude_modules,
        min_stmts,
        verify,
        verify_args,
        verify_dir,
        verify_rounds,
        verify_max_withdraw,
        frozen_ledger,
        private_types,
        overlay,
    })))
}

/// W5: `hook-ledger/stamp-config.txt`, one line per crate:
/// `<crate root> <crate name> <module|*> [exclude=a,b] [min_stmts=N]` (`#` comments). Command-line values win:
/// `--module` arguments replace the file's module, `--min-stmts` is kept when the file has none.
pub(crate) fn apply_stamp_config(
    path: &std::path::Path,
    crate_name: &str,
    modules: &mut Vec<String>,
    exclude: &mut Vec<String>,
    min_stmts: &mut usize,
    private_types: &mut bool,
    bridges: &mut bool,
) -> Result<(), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let wanted = key::normalize_crate_name(crate_name);
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 || key::normalize_crate_name(parts[1]) != wanted {
            continue;
        }
        if modules.is_empty() {
            modules.push(parts[2].to_string());
        }
        for flag in &parts[3..] {
            if let Some(list) = flag.strip_prefix("exclude=") {
                exclude.extend(list.split(',').filter(|s| !s.is_empty()).map(str::to_string));
            } else if *flag == "bridges=true" {
                *bridges = true;
            } else if *flag == "private_types=true" {
                *private_types = true;
            } else if let Some(n) = flag.strip_prefix("min_stmts=") {
                if *min_stmts == 0 {
                    *min_stmts = n.parse().map_err(|_| format!("bad min_stmts in '{line}'"))?;
                }
            } else {
                return Err(format!("unknown stamp-config flag '{flag}' in '{line}'"));
            }
        }
        return Ok(());
    }
    Err(format!("no line for crate '{crate_name}' in {}", path.display()))
}

/// REM-11: `callsite-patch --workspace-root W --crate NAME --thir calls.json [--stamp-config F] [--ledger D] [--scratch DIR]`.
fn parse_callsite_patch_args(args: &mut std::env::Args) -> Result<Option<Command>, String> {
    let mut workspace_root = PathBuf::from(".");
    let (mut stamp_config, mut ledger, mut scratch, mut crate_name, mut thir) = (None, None, None, None, None);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(None);
            }
            "--workspace-root" => workspace_root = PathBuf::from(next_arg(args, "--workspace-root")?),
            "--stamp-config" => stamp_config = Some(PathBuf::from(next_arg(args, "--stamp-config")?)),
            "--ledger" => ledger = Some(PathBuf::from(next_arg(args, "--ledger")?)),
            "--scratch" => scratch = Some(PathBuf::from(next_arg(args, "--scratch")?)),
            "--crate" => crate_name = Some(next_arg(args, "--crate")?),
            "--thir" => thir = Some(PathBuf::from(next_arg(args, "--thir")?)),
            other => return Err(format!("unknown argument '{other}'
{}", usage())),
        }
    }
    let crate_name = crate_name.ok_or_else(|| format!("missing --crate
{}", usage()))?;
    let thir = thir.ok_or_else(|| format!("missing --thir
{}", usage()))?;
    Ok(Some(Command::CallsitePatch(callsite_patch::GenerateArgs {
        stamp_config: stamp_config.unwrap_or_else(|| workspace_root.join("hook-ledger").join("stamp-config.txt")),
        ledger: ledger.unwrap_or_else(|| workspace_root.join("hook-ledger")),
        scratch: scratch.unwrap_or_else(|| workspace_root.join("target").join("yoake-callsite").join(key::normalize_crate_name(&crate_name))),
        workspace_root,
        crate_name,
        thir,
    })))
}

fn parse_sync_ledger_args(args: &mut std::env::Args) -> Result<Option<Command>, String> {
    let mut workspace_root = PathBuf::from(".");
    let mut stamp_config = None;
    let mut ledger = None;
    let mut overlay_root = None;
    let mut only = Vec::new();
    let mut frozen = false;
    let mut renames = Vec::new();
    let mut tombstones = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{}", usage());
                return Ok(None);
            }
            "--rename" => {
                let raw = next_arg(args, "--rename")?;
                match raw.split_once('=') {
                    Some((o, n)) if !o.is_empty() && !n.is_empty() => renames.push((o.to_string(), n.to_string())),
                    _ => return Err(format!("--rename expects OLD_KEY=NEW_KEY, got '{raw}'")),
                }
            }
            "--tombstone" => tombstones.push(next_arg(args, "--tombstone")?),
            "--workspace-root" => workspace_root = PathBuf::from(next_arg(args, "--workspace-root")?),
            "--stamp-config" => stamp_config = Some(PathBuf::from(next_arg(args, "--stamp-config")?)),
            "--ledger" => ledger = Some(PathBuf::from(next_arg(args, "--ledger")?)),
            "--overlay-root" => overlay_root = Some(PathBuf::from(next_arg(args, "--overlay-root")?)),
            "--crate" => only.push(next_arg(args, "--crate")?),
            "--frozen" => frozen = true,
            other => return Err(format!("unknown argument '{other}'
{}", usage())),
        }
    }
    Ok(Some(Command::SyncLedger(SyncLedgerConfig {
        stamp_config: stamp_config.unwrap_or_else(|| workspace_root.join("hook-ledger").join("stamp-config.txt")),
        ledger: ledger.unwrap_or_else(|| workspace_root.join("hook-ledger")),
        overlay_root: overlay_root.unwrap_or_else(|| workspace_root.join("target").join("yoake-sync").join("root")),
        workspace_root,
        only,
        frozen,
        renames,
        tombstones,
    })))
}

fn sync_ledger(config: &SyncLedgerConfig) -> Result<bool, String> {
    let text = std::fs::read_to_string(&config.stamp_config)
        .map_err(|e| format!("failed to read {}: {e}", config.stamp_config.display()))?;
    for raw in text.lines() {
        let line = raw.split('#').next().unwrap_or("").trim();
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        if !config.only.is_empty() && !config.only.iter().any(|c| key::normalize_crate_name(c) == key::normalize_crate_name(parts[1])) {
            continue;
        }
        println!("sync-ledger: {}", parts[1]);
        overlay::build_overlay(&overlay::OverlayArgs {
            workspace_root: config.workspace_root.clone(),
            overlay_root: config.overlay_root.clone(),
            crate_root: PathBuf::from(parts[0]),
            crate_name: parts[1].to_string(),
            ledger: config.ledger.clone(),
            stamp_config: config.stamp_config.clone(),
            frozen: config.frozen,
            renames: config.renames.clone(),
            tombstones: config.tombstones.clone(),
        })?;
    }
    Ok(true)
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
    let mut shallow = false;
    let mut glues = Vec::new();
    let mut exclude_file = None;
    let mut nary = false;

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
            "--shallow" => shallow = true,
            "--glue" => {
                let first = args
                    .next()
                    .ok_or_else(|| "missing value for --glue".to_string())?;
                glues.push(PathBuf::from(first));
                while args.peek().is_some_and(|next| !next.starts_with('-')) {
                    glues.push(PathBuf::from(args.next().unwrap()));
                }
            }
            "--nary" => nary = true,
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
        nary,
        check,
        crate_root,
        crate_name,
        ledger,
        modules,
        shallow,
        glues,
        exclude_file,
    })))
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().collect();
    if overlay::is_wrapper_invocation(&raw) {
        return overlay::run_wrapper(raw);
    }
    let command = match parse_args() {
        Ok(Some(cmd)) => cmd,
        Ok(None) => return ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            return ExitCode::from(2);
        }
    };

    let result = match command {
        Command::Run(config) if config.verify && config.mode == Mode::Apply => {
            let overlay = config.overlay.then(|| {
                let target = std::env::var_os("YOAKE_STAMP_TARGET")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| config.verify_dir.join("target-stamped"));
                verify::OverlayVerify {
                    workspace_root: config.verify_dir.clone(),
                    stamp_dir: target.join("yoake-stamp"),
                    stamp_config: config.verify_dir.join("hook-ledger").join("stamp-config.txt"),
                }
            });
            let options = verify::VerifyOptions {
                overlay,
                cargo_args: config.verify_args.clone(),
                dir: config.verify_dir.clone(),
                rounds: config.verify_rounds,
                max_withdraw_per_round: config.verify_max_withdraw,
            };
            verify::run(&config, &options)
        }
        Command::SyncLedger(config) => sync_ledger(&config),
        Command::CallsitePatch(args) => callsite_patch::generate(&args).map(|(files, hunks)| {
            println!("callsite-patch: {files} file(s), {hunks} hunk(s) in {}", callsite_patch::patch_path(&args.ledger, &key::normalize_crate_name(&args.crate_name)).display());
            true
        }),
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
