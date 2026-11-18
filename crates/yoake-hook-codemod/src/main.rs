mod attr;
mod edit;
mod eligible;
mod key;
mod ledger_io;
mod modules;
mod run;
mod types;

use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Apply,
    Check,
}

pub struct Config {
    pub mode: Mode,
    pub crate_root: PathBuf,
    pub crate_name: String,
    pub ledger: PathBuf,
    pub modules: Vec<String>,
    pub renames: Vec<(String, String)>,
    pub tombstones: Vec<String>,
    pub rustfmt: bool,
}

pub struct HistoryConfig {
    pub base: PathBuf,
    pub current: PathBuf,
}

enum Command {
    Run(Config),
    CheckHistory(HistoryConfig),
}

fn usage() -> &'static str {
    "usage:\n  \
     yoake-hook-codemod <apply|check> --crate-root <dir> --crate-name <name> \
     --ledger <dir> --module <mod> [--module <mod>...] [--rename OLD=NEW]... [--tombstone KEY]... [--rustfmt]\n  \
     yoake-hook-codemod check-history --base <dir> --current <dir>"
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

    if first == "check-history" {
        return parse_check_history_args(&mut args);
    }

    let mode = match first.as_str() {
        "apply" => Mode::Apply,
        "check" => Mode::Check,
        other => {
            return Err(format!(
                "expected 'apply', 'check', or 'check-history', got '{other}'\n{}",
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

    Ok(Some(Command::Run(Config {
        mode,
        crate_root,
        crate_name,
        ledger,
        modules,
        renames,
        tombstones,
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
        Command::CheckHistory(history) => ledger_io::run_check_history(&history.base, &history.current),
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
