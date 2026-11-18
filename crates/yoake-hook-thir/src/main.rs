use std::collections::BTreeSet;
use std::path::PathBuf;

use yoake_hook_thir::dump::{DumpOptions, dump};
use yoake_hook_thir::{Error, ParseOptions, Result, parse_file, write_json};

fn main() {
    if let Err(error) = run() {
        eprintln!("yoake-hook-thir: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        Some("dump") => {
            let mut options = DumpOptions {
                crate_pkg: String::new(),
                crate_name: String::new(),
                workspace: PathBuf::new(),
                out: PathBuf::new(),
                files: Vec::new(),
            };
            while let Some(name) = arguments.next() {
                let value = arguments.next().ok_or_else(|| Error::new(format!("{name} needs a value")))?;
                match name.as_str() {
                    "--crate-pkg" => options.crate_pkg = value,
                    "--crate-name" => options.crate_name = value,
                    "--workspace" => options.workspace = value.into(),
                    "--out" => options.out = value.into(),
                    "--file" => options.files.push(value),
                    _ => return Err(Error::new(format!("unknown dump option {name}"))),
                }
            }
            if options.crate_pkg.is_empty()
                || options.crate_name.is_empty()
                || options.workspace.as_os_str().is_empty()
                || options.out.as_os_str().is_empty()
                || options.files.is_empty()
            {
                return Err(Error::new(usage()));
            }
            dump(&options)
        }
        Some("parse") => {
            let mut thir = None;
            let mut out = None;
            let mut options = ParseOptions {
                toolchain: "unknown (parse subcommand)".into(),
                ..ParseOptions::default()
            };
            while let Some(name) = arguments.next() {
                let value = arguments.next().ok_or_else(|| Error::new(format!("{name} needs a value")))?;
                match name.as_str() {
                    "--thir" => thir = Some(PathBuf::from(value)),
                    "--out" => out = Some(PathBuf::from(value)),
                    "--file" => options.files.push(value),
                    "--extern" => {
                        options.externs.insert(value);
                    }
                    _ => return Err(Error::new(format!("unknown parse option {name}"))),
                }
            }
            if options.files.is_empty() {
                return Err(Error::new(usage()));
            }
            let thir = thir.ok_or_else(|| Error::new("parse requires --thir"))?;
            let out = out.ok_or_else(|| Error::new("parse requires --out"))?;
            let parsed = parse_file(&thir, &options)?;
            write_json(&parsed, &out)
        }
        _ => Err(Error::new(usage())),
    }
}

fn usage() -> &'static str {
    "usage:\n  yoake-hook-thir dump --crate-pkg PKG --crate-name CRATE --workspace DIR --out JSON --file PATH...\n  yoake-hook-thir parse --thir FILE --out JSON --file PATH... [--extern NAME]..."
}

#[allow(dead_code)]
fn _assert_send_sync(_: BTreeSet<String>) {}
