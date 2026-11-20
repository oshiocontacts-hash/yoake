//! `yoake-bindgen --config bindgen.json [--root DIR] [--check]`
//!
//! Writes the glue, the id lock and the coverage reports, or with `--check` compares what it would
//! write with what is on disk and exits 1 on any difference (the CI mode).
//!
//! `yoake-bindgen --config bindgen.json --survey CRATE` classifies every file module of a crate and
//! writes `survey-CRATE.md` next to the coverage report (the "whole crate" estimate; binds nothing).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use yoake_bindgen::{Config, generate, load_universe};

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("yoake-bindgen: {error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, Box<dyn std::error::Error>> {
    let mut config_path = None;
    let mut root = PathBuf::from(".");
    let mut check = false;
    let mut survey_crate: Option<String> = None;
    let mut lock_base: Option<PathBuf> = None;
    let mut tables_crate: Option<String> = None;
    let mut tables_dir: Option<PathBuf> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" => config_path = args.next(),
            "--root" => root = PathBuf::from(args.next().ok_or("--root needs a directory")?),
            "--check" => check = true,
            "--lock-base" => lock_base = Some(PathBuf::from(args.next().ok_or("--lock-base needs a file")?)),
            "--tables" => tables_crate = Some(args.next().ok_or("--tables needs a crate name")?),
            "--tables-dir" => tables_dir = Some(PathBuf::from(args.next().ok_or("--tables-dir needs a directory")?)),
            "--survey" => survey_crate = Some(args.next().ok_or("--survey needs a crate name")?),
            other => return Err(format!("unknown argument `{other}`").into()),
        }
    }
    let config_path = config_path.ok_or("usage: yoake-bindgen --config bindgen.json [--root DIR] [--check] [--survey CRATE]")?;
    // the rustdoc JSON was made by the pinned compiler; another rustc may print another JSON (rust-toolchain.toml)
    yoake_bindgen::toolchain::check_rustc(&root)?;
    let config: Config = serde_json::from_str(&std::fs::read_to_string(&config_path)?)?;
    let uni = load_universe(&config, &root)?;
    if let Some(krate) = tables_crate {
        // W9b: one small glue file per module with scalar fields, in `<tables-dir>/<module path with __>.rs`
        let dir = root.join(tables_dir.ok_or("--tables needs --tables-dir")?);
        std::fs::create_dir_all(&dir)?;
        let all = yoake_bindgen::tables(&uni, &krate, &config.frozen_files)?;
        let mut bytes = 0usize;
        for (label, _file, text) in &all {
            bytes += text.len();
            std::fs::write(dir.join(format!("{}.rs", label.replace("::", "__"))), text)?;
        }
        println!("{krate}: {} module table files, {} bytes into {}", all.len(), bytes, dir.display());
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(krate) = survey_crate {
        return survey(&config, &uni, &krate, &root);
    }
    let generated = generate(&config, &root, &uni)?;
    if let Some(base_path) = lock_base {
        // The append-only gate: the lock on this branch against the lock on the base branch.
        let base = yoake_bindgen::lock::Lock::load(&base_path)?;
        let now: yoake_bindgen::lock::Lock = serde_json::from_str(&generated.files[&PathBuf::from(&config.lock)])?;
        let violations = now.append_only_violations(&base);
        for v in &violations {
            eprintln!("LOCK: {v}");
        }
        if !violations.is_empty() {
            return Ok(ExitCode::from(1));
        }
        println!("ids.lock.json is append-only against {} ({} base ids)", base_path.display(), base.bindings.len());
    }
    let stale = yoake_bindgen::stale_glue_files(&config, &root);
    for path in &stale {
        eprintln!("STALE: {} belongs to no configured target", path.display());
    }
    if check && !stale.is_empty() {
        return Ok(ExitCode::from(1));
    }

    for module in &generated.modules {
        let c = yoake_bindgen::report::count(&module.scan);
        println!(
            "{}: {} functions, bound {} (a {} + b {}), handle-required {}, excluded {}, derived {} ({} bound); fields bound {}/{}",
            module.label,
            c.total,
            c.bound(),
            c.bound_a,
            c.bound_mech,
            c.handle_required,
            c.excluded,
            c.derived,
            c.derived_bound,
            c.fields_bound,
            c.fields_total
        );
    }
    for (path, old, new) in &generated.lock_delta.changed {
        println!("signature changed (id kept): {path}: {old} -> {new}");
    }
    if !generated.unresolved.is_empty() {
        println!("external types without loaded JSON: {}", generated.unresolved.len());
    }

    let mut differences = 0;
    for (path, text) in &generated.files {
        let full = root.join(path);
        let current = std::fs::read_to_string(&full).ok();
        let same = current.as_deref() == Some(text.as_str());
        if check {
            if !same {
                differences += 1;
                eprintln!("DIFF: {} {}", full.display(), if current.is_none() { "(missing)" } else { "(differs)" });
            }
        } else if !same {
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full, text)?;
            println!("wrote {}", full.display());
        }
    }
    if check && differences > 0 {
        eprintln!("{differences} generated file(s) are out of date; run yoake-bindgen without --check");
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn survey(config: &Config, uni: &yoake_bindgen::doc::Universe, krate: &str, root: &Path) -> Result<ExitCode, Box<dyn std::error::Error>> {
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use yoake_bindgen::analyze::Cat;
    use yoake_bindgen::report::Counts;

    let roots: Vec<String> = config.targets.iter().flat_map(|t| t.roots.clone()).collect();
    let modules = yoake_bindgen::survey(uni, krate, &roots)?;
    let mut total = Counts::default();
    let mut reasons: BTreeMap<(bool, String), usize> = BTreeMap::new();
    for (_, c, fns) in &modules {
        total.total += c.total;
        total.bound_a += c.bound_a;
        total.bound_mech += c.bound_mech;
        total.handle_required += c.handle_required;
        total.excluded += c.excluded;
        total.derived += c.derived;
        total.derived_bound += c.derived_bound;
        total.fields_total += c.fields_total;
        total.fields_bound += c.fields_bound;
        for f in fns.iter().filter(|f| !f.derived) {
            let hard = match f.cat {
                Cat::Excluded => true,
                Cat::HandleRequired => false,
                _ => continue,
            };
            for reason in &f.reasons {
                // names in backticks are generalised so that "type `Foo`" and "type `Bar`" count together
                let generic = reason.split('`').enumerate().map(|(i, part)| if i % 2 == 1 { "_" } else { part }).collect::<Vec<_>>().join("`");
                *reasons.entry((hard, generic)).or_insert(0) += 1;
            }
        }
    }
    let pct = |n: usize| if total.total == 0 { 0.0 } else { n as f64 * 100.0 / total.total as f64 };
    let mut md = String::new();
    let _ = writeln!(md, "# Survey of crate `{krate}`: the classifier over every file module\n");
    let _ = writeln!(md, "Generated; nothing is bound by this. Each module is classified as if its own glue were included in it (private items of other modules are not reachable). Derived trait impls are left out of the denominator.\n");
    let _ = writeln!(md, "- modules: {}", modules.len());
    let _ = writeln!(md, "- functions: {}", total.total);
    let _ = writeln!(md, "- bound now: {} ({:.1}%): {} basic + {} mechanical", total.bound(), pct(total.bound()), total.bound_a, total.bound_mech);
    let _ = writeln!(md, "- handle-required: {} ({:.1}%)", total.handle_required, pct(total.handle_required));
    let _ = writeln!(md, "- excluded: {} ({:.1}%)", total.excluded, pct(total.excluded));
    let _ = writeln!(md, "- fields with accessors: {} of {}\n", total.fields_bound, total.fields_total);
    for (title, hard) in [("Handle-required: most frequent reasons (names generalised)", false), ("Excluded: most frequent reasons (names generalised)", true)] {
        let mut rows: Vec<(&String, &usize)> = reasons.iter().filter(|((h, _), _)| *h == hard).map(|((_, r), n)| (r, n)).collect();
        rows.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let _ = writeln!(md, "## {title}\n\n| reason | occurrences |\n|---|---:|");
        for (reason, n) in rows.iter().take(25) {
            let _ = writeln!(md, "| {} | {} |", reason.replace('|', "\\|"), n);
        }
        let _ = writeln!(md);
    }
    let _ = writeln!(md, "## Per module\n\n| module | functions | bound | handle-required | excluded |\n|---|---:|---:|---:|---:|");
    let mut rows: Vec<_> = modules.iter().filter(|(_, c, _)| c.total > 0).collect();
    rows.sort_by(|a, b| b.1.total.cmp(&a.1.total).then(a.0.cmp(&b.0)));
    for (label, c, _) in rows {
        let _ = writeln!(md, "| `{}` | {} | {} | {} | {} |", label, c.total, c.bound(), c.handle_required, c.excluded);
    }
    let path = root.join(&config.report_dir).join(format!("survey-{krate}.md"));
    std::fs::write(&path, md)?;
    println!(
        "{krate}: {} modules, {} functions, bound {} ({:.1}%), handle-required {} ({:.1}%), excluded {} ({:.1}%); fields {}/{}",
        modules.len(),
        total.total,
        total.bound(),
        pct(total.bound()),
        total.handle_required,
        pct(total.handle_required),
        total.excluded,
        pct(total.excluded),
        total.fields_bound,
        total.fields_total
    );
    println!("wrote {}", path.display());
    Ok(ExitCode::SUCCESS)
}
