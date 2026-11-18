//! The Yoake 2.0 binding generator (MX3G1).
//!
//! Reads rustdoc JSON (format 61: stable toolchain with `RUSTC_BOOTSTRAP=1`, `-Z unstable-options
//! --output-format json --document-private-items`) of the engine crates and writes, for each
//! configured module, one glue file that the engine `include!`s inside that module, so private
//! items are reachable. See `bindgen.json` for the configuration and `docs/yoake2/reports/MX3G1.md`.
//!
//! Pipeline: [`doc`] loads the JSON and resolves ids across crates; [`shape`] decides which types are
//! plain data (the wire codec reads and writes them field by field), which need a handle, which cannot
//! cross at all; [`analyze`] classifies every function and field of a module; [`render`] writes the glue;
//! [`lock`] keeps ids stable; [`report`] writes the coverage numbers.

pub mod analyze;
pub mod display;
pub mod doc;
pub mod lock;
pub mod reach;
pub mod render;
pub mod report;
pub mod shape;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::analyze::{ModuleScan, find_module, scan_module};
use crate::doc::{Doc, R, Universe};
use crate::lock::Lock;
use crate::shape::{Analyzer, TargetInfo};

/// The rustdoc JSON format this generator reads; anything else is refused.
pub const FORMAT_VERSION: u64 = 61;

#[derive(Deserialize, Debug, Clone)]
pub struct Config {
    /// rustdoc JSON files, relative to the root. The first crate of each target must be among them.
    pub docs: Vec<String>,
    /// The id lock file.
    pub lock: String,
    /// Where `coverage.md`, `summary.json` and the per-module function lists go.
    pub report_dir: String,
    pub targets: Vec<TargetCfg>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct TargetCfg {
    /// A short label used in lock keys (`entities`).
    pub name: String,
    /// The crate the module is in (`horizon_client_lib`).
    pub crate_name: String,
    /// A path suffix of the module's file, as rustdoc reports it (`horizon-client/src/entities.rs`).
    pub source: String,
    /// The glue file to write, relative to the root.
    pub glue: String,
    /// Struct names that are the `Game` root.
    #[serde(default)]
    pub roots: Vec<String>,
}

pub struct ModuleResult {
    pub label: String,
    pub source: String,
    pub scan: ModuleScan,
}

pub struct Generated {
    /// Every file the run writes, by path relative to the root.
    pub files: BTreeMap<PathBuf, String>,
    pub modules: Vec<ModuleResult>,
    pub lock_delta: lock::LockDelta,
    pub unresolved: Vec<String>,
}

pub fn load_universe(config: &Config, root: &Path) -> R<Universe> {
    let mut docs = Vec::new();
    for path in &config.docs {
        let full = root.join(path);
        let text = std::fs::read_to_string(&full).map_err(|e| format!("{}: {e}", full.display()))?;
        docs.push(Doc::parse(&text)?);
    }
    Ok(Universe { docs })
}

pub fn generate(config: &Config, root: &Path, uni: &Universe) -> R<Generated> {
    struct Prepared<'u> {
        an: Analyzer<'u>,
        scan: ModuleScan,
        target: TargetCfg,
    }
    let mut prepared: Vec<Prepared<'_>> = Vec::new();
    for target in &config.targets {
        let doc = uni.doc_index(&target.crate_name).ok_or_else(|| format!("no rustdoc JSON loaded for crate `{}`", target.crate_name))?;
        let mut an = Analyzer::new(
            uni,
            TargetInfo { crate_name: target.crate_name.clone(), module: Vec::new(), roots: target.roots.clone() },
        );
        let module = find_module(&an, doc, &target.source).ok_or_else(|| format!("no module for `{}` in crate `{}`", target.source, target.crate_name))?;
        an.target.module = uni.docs[doc].module_path(module);
        let scan = scan_module(&mut an, doc, module, &target.name);
        prepared.push(Prepared { an, scan, target: target.clone() });
    }

    let lock_path = root.join(&config.lock);
    let mut lock = Lock::load(&lock_path)?;
    let mut wanted = BTreeMap::new();
    for p in &prepared {
        wanted.extend(render::wanted(&p.scan));
    }
    let (ids, delta) = lock.assign(&wanted);

    let mut files = BTreeMap::new();
    files.insert(PathBuf::from(&config.lock), lock.to_text()?);
    let mut unresolved = std::collections::BTreeSet::new();
    for p in &prepared {
        let text = render::render(&p.an, &p.target.name, &p.target.crate_name, &p.scan, &ids);
        files.insert(PathBuf::from(&p.target.glue), text);
        unresolved.extend(p.an.unresolved.iter().cloned());
    }
    let unresolved: Vec<String> = unresolved.into_iter().collect();

    let mut delta_note = String::new();
    for (title, items) in [("added", &delta.added), ("no longer bound", &delta.unbound), ("bound again", &delta.rebound)] {
        if !items.is_empty() {
            delta_note.push_str(&format!("- {title}: {}\n", items.len()));
        }
    }
    if !delta.changed.is_empty() {
        delta_note.push_str(&format!("- signature changed (id kept): {}\n", delta.changed.len()));
        for (path, old, new) in &delta.changed {
            delta_note.push_str(&format!("  - `{path}`: `{old}` -> `{new}`\n"));
        }
    }
    let _ = delta_note;

    // how each handle type is reached from the `Game` root
    let mut handle_set = std::collections::BTreeSet::new();
    for p in &prepared {
        handle_set.extend(p.scan.handle_refs.iter().map(|(r, _)| *r));
    }
    let mut game_paths = BTreeMap::new();
    for p in &prepared {
        for root in &p.target.roots {
            if let Some(doc) = uni.doc_index(&p.target.crate_name) {
                for (item, path) in reach::game_paths(uni, doc, root, &handle_set, 4) {
                    game_paths.entry(item).or_insert(path);
                }
            }
        }
    }
    let reports: Vec<report::ModuleReport<'_>> = prepared
        .iter()
        .map(|p| report::ModuleReport { label: &p.target.name, source: &p.target.source, scan: &p.scan, game_paths: &game_paths })
        .collect();
    let report_dir = PathBuf::from(&config.report_dir);
    files.insert(report_dir.join("coverage.md"), report::coverage_md(&reports, &unresolved, ""));
    files.insert(report_dir.join("summary.json"), report::summary_json(&reports));
    for m in &reports {
        files.insert(report_dir.join(format!("functions-{}.md", m.label)), report::functions_md(m));
    }
    drop(reports);

    let modules = prepared.into_iter().map(|p| ModuleResult { label: p.target.name.clone(), source: p.target.source.clone(), scan: p.scan }).collect();
    Ok(Generated { files, modules, lock_delta: delta, unresolved })
}

/// The counts of every file module of one crate, for the "whole crate" estimate: the same classifier,
/// run once per module as if its glue were included there. Nothing is written.
pub fn survey(uni: &Universe, crate_name: &str, roots: &[String]) -> R<Vec<(String, report::Counts, Vec<analyze::FnReport>)>> {
    let doc = uni.doc_index(crate_name).ok_or_else(|| format!("no rustdoc JSON loaded for crate `{crate_name}`"))?;
    let index = uni.docs[doc].v.get("index").and_then(serde_json::Value::as_object).ok_or("no index")?;
    let mut modules: Vec<u64> = Vec::new();
    for (id, item) in index {
        let begin = item.pointer("/span/begin").and_then(serde_json::Value::as_array);
        let at_start = begin.is_some_and(|b| b.first().and_then(serde_json::Value::as_u64) == Some(1) && b.get(1).and_then(serde_json::Value::as_u64) == Some(1));
        if item.pointer("/inner/module").is_some() && at_start && item.pointer("/span/filename").is_some() {
            if let Ok(id) = id.parse() {
                modules.push(id);
            }
        }
    }
    modules.sort_unstable();
    let mut out = Vec::new();
    for module in modules {
        let path = uni.docs[doc].module_path(module);
        let label = path[1..].join("::");
        let label = if label.is_empty() { "(crate root)".to_string() } else { label };
        let mut an = Analyzer::new(uni, TargetInfo { crate_name: crate_name.to_string(), module: path, roots: roots.to_vec() });
        let scan = scan_module(&mut an, doc, module, &label);
        let counts = report::count(&scan);
        out.push((label, counts, scan.fns));
    }
    Ok(out)
}
