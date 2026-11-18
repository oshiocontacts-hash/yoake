mod attr;
mod edit;
mod eligible;
mod key;
mod modules;
mod run;

use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Apply,
    Check,
}

struct Config {
    mode: Mode,
    crate_root: PathBuf,
    crate_name: String,
    ledger: PathBuf,
    modules: Vec<String>,
    renames: Vec<(String, String)>,
    tombstone_missing: bool,
    rustfmt: bool,
}

fn usage() -> &'static str {
    "usage: yoake-hook-codemod <apply|check> --crate-root <dir> --crate-name <name> \
     --ledger <path> --module <mod> [--module <mod>...] [--rename OLD=NEW]... [--tombstone-missing] [--rustfmt]"
}

fn next_arg(args: &mut std::env::Args, flag: &str) -> Result<String, String> {
    args.next()
        .ok_or_else(|| format!("missing value for {flag}"))
}

fn parse_args() -> Result<Option<Config>, String> {
    let mut args = std::env::args();
    let _bin = args.next();
    let Some(first) = args.next() else {
        return Err(usage().to_string());
    };
    if first == "-h" || first == "--help" {
        println!("{}", usage());
        return Ok(None);
    }
    let mode = match first.as_str() {
        "apply" => Mode::Apply,
        "check" => Mode::Check,
        other => return Err(format!("expected 'apply' or 'check', got '{other}'\n{}", usage())),
    };

    let mut crate_root = None;
    let mut crate_name = None;
    let mut ledger = None;
    let mut modules = Vec::new();
    let mut renames = Vec::new();
    let mut tombstone_missing = false;
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
            "--tombstone-missing" => tombstone_missing = true,
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

    Ok(Some(Config {
        mode,
        crate_root,
        crate_name,
        ledger,
        modules,
        renames,
        tombstone_missing,
        rustfmt,
    }))
}

fn main() -> ExitCode {
    let config = match parse_args() {
        Ok(Some(cfg)) => cfg,
        Ok(None) => return ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            return ExitCode::from(2);
        }
    };

    match run::execute(&config) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(1)
        }
    }
}
