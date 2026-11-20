//! W9b: the generated hook bridges of a stamped crate, produced inside the overlay (never committed as source).
//!
//! After the stamping of a crate (`overlay::build_overlay`), for every **file module** of the crate:
//! 1. the committed field-table glue of that module (`<src>/yoake_glue/tables/<crate>/<module>.rs`, bindgen `--tables`)
//!    is copied into the overlay and `include!`d into the module (the tables read private fields, so they live inside);
//! 2. `bridge --nary --shallow` writes one `hook_bridge_<module>.rs` per module and it is `include!`d into **its own
//!    module**, so a type private to that module is nameable there (W4's private-type stamps);
//! 3. every module gets `__yoake_bridge_install` / `__yoake_bridge_mode`, which call its own bridge and its child
//!    modules' (a private child is nameable from its parent), and the crate root exports
//!    `__yoake_install_bridges` / `__yoake_bridge_mode`. The loader calls the root functions when built with
//!    `--cfg yoake_stamped` (set by the wrapper).
//!
//! Modules that already include a committed bridge (`hook_bridge_*.rs` in the source) keep it. Hooks inside inline
//! `mod x { .. }` blocks belong to no file module and are not bridged.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::bridge::BridgeConfig;
use crate::modules::{self, ScannedFile};

/// One file module of the crate, as the overlay sees it.
struct FileModule {
    path: Vec<String>,
    file: PathBuf,
    /// Child file modules declared directly in this file: (name, cfg attributes as source text).
    children: Vec<(String, Vec<String>)>,
    has_bridge: bool,
    committed_bridge: bool,
}

fn rel_from(file: &Path, target: &Path) -> String {
    // target relative to the directory of `file`, with forward slashes
    let from: Vec<_> = file.parent().unwrap_or(Path::new("")).components().collect();
    let to: Vec<_> = target.components().collect();
    let mut i = 0;
    while i < from.len() && i < to.len() && from[i] == to[i] {
        i += 1;
    }
    let mut parts: Vec<String> = from[i..].iter().map(|_| "..".to_string()).collect();
    parts.extend(to[i..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    parts.join("/")
}

fn bridge_file_name(path: &[String]) -> String {
    format!("hook_bridge_{}.rs", path.join("__"))
}

fn module_cfgs(item: &syn::ItemMod) -> Vec<String> {
    item.attrs
        .iter()
        .filter(|a| a.path().is_ident("cfg"))
        .map(|a| quote::ToTokens::to_token_stream(a).to_string())
        .collect()
}

/// Appends the bridges, glue includes and chain functions to the overlay copy of the crate.
/// `overlay_crate_root`: the crate directory inside the overlay; `workspace_src`: the crate's real source directory
/// (where `yoake_glue/tables/` is committed).
pub fn add_bridges(
    overlay_crate_root: &Path,
    crate_name: &str,
    ledger: &Path,
    workspace_src: &Path,
    exclude_top: &[String],
) -> Result<BridgeStats, String> {
    let ident = crate::key::normalize_crate_name(crate_name);
    let root_file = modules::find_crate_root_file(overlay_crate_root, crate_name)?;
    let root_dir = root_file.parent().ok_or("crate root has no parent")?.to_path_buf();
    let tops = modules::expand_star_modules(overlay_crate_root, crate_name, &["*".to_string()], exclude_top)?;
    let targets = tops.iter().map(|m| modules::normalize_module_arg(m)).collect::<Result<Vec<_>, _>>()?;
    let scanned: Vec<ScannedFile> = modules::collect_module_files(overlay_crate_root, crate_name, &targets, false)?;

    // 1. the module list (root included) with children
    let mut mods: BTreeMap<Vec<String>, FileModule> = BTreeMap::new();
    let mut files: Vec<(&ScannedFile, Vec<String>)> = vec![];
    for f in &scanned {
        files.push((f, f.root_mod_path.clone()));
    }
    let root_parsed = syn::parse_file(&std::fs::read_to_string(&root_file).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let mut all: Vec<(Vec<String>, PathBuf, syn::File, bool)> = vec![(Vec::new(), root_file.clone(), root_parsed, false)];
    for (f, path) in files {
        let text = std::fs::read_to_string(&f.path).unwrap_or_else(|_| f.source.clone());
        let parsed = syn::parse_file(&text).map_err(|e| format!("{}: {e}", f.path.display()))?;
        let committed = text.contains("hook_bridge_");
        all.push((path, f.path.clone(), parsed, committed));
    }
    for (path, file, parsed, committed) in &all {
        let mut children = Vec::new();
        for item in &parsed.items {
            if let syn::Item::Mod(m) = item
                && m.content.is_none()
                && !modules::has_cfg_test(&m.attrs)
            {
                let name = m.ident.to_string();
                let mut child_path = path.clone();
                child_path.push(name.clone());
                if all.iter().any(|(p, ..)| *p == child_path) {
                    children.push((name, module_cfgs(m)));
                }
            }
        }
        mods.insert(path.clone(), FileModule { path: path.clone(), file: file.clone(), children, has_bridge: false, committed_bridge: *committed });
    }

    // 2. the committed table glue of each module is copied into the overlay and included
    let tables_src = workspace_src.join("yoake_glue").join("tables").join(&ident);
    let tables_dst = root_dir.join("yoake_glue").join("tables").join(&ident);
    let mut glues: Vec<PathBuf> = Vec::new();
    // glue files of the crate that are committed next to the sources (the hand-wired modules' tables)
    if let Ok(entries) = std::fs::read_dir(workspace_src.join("yoake_glue")) {
        for e in entries.flatten() {
            let p = e.path();
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.extension().is_some_and(|x| x == "rs") && !name.starts_with("hook_bridge_") {
                let dst = root_dir.join("yoake_glue").join(name);
                std::fs::create_dir_all(dst.parent().unwrap()).map_err(|e| e.to_string())?;
                std::fs::copy(&p, &dst).map_err(|e| e.to_string())?;
                glues.push(dst);
            }
        }
    }
    let mut table_includes: BTreeMap<Vec<String>, PathBuf> = BTreeMap::new();
    for m in mods.values() {
        if m.path.is_empty() {
            continue;
        }
        let src = tables_src.join(format!("{}.rs", m.path.join("__")));
        if src.is_file() {
            let dst = tables_dst.join(format!("{}.rs", m.path.join("__")));
            std::fs::create_dir_all(&tables_dst).map_err(|e| e.to_string())?;
            std::fs::copy(&src, &dst).map_err(|e| e.to_string())?;
            glues.push(dst.clone());
            table_includes.insert(m.path.clone(), dst);
        }
    }
    let mut appended: BTreeMap<PathBuf, String> = BTreeMap::new();
    for (path, dst) in &table_includes {
        let m = &mods[path];
        appended
            .entry(m.file.clone())
            .or_default()
            .push_str(&format!("\n#[cfg(feature = \"yoake2-slice\")]\ninclude!(\"{}\");\n", rel_from(&m.file, dst)));
    }
    // the includes must be in the sources before the bridge generator scans for them
    for (file, text) in &appended {
        let mut s = std::fs::read_to_string(file).map_err(|e| e.to_string())?;
        s.push_str(text);
        std::fs::write(file, s).map_err(|e| e.to_string())?;
    }
    appended.clear();

    // 3. the bridges, one per module
    let module_args: Vec<String> = mods.keys().filter(|p| !p.is_empty() && !mods[*p].committed_bridge).map(|p| p.join("::")).collect();
    let bridge_dir = root_dir.join("yoake_glue");
    let mut stats = BridgeStats::default();
    if !module_args.is_empty() {
        crate::bridge::execute(&BridgeConfig {
            nary: true,
            check: false,
            crate_root: overlay_crate_root.to_path_buf(),
            crate_name: crate_name.to_string(),
            ledger: ledger.to_path_buf(),
            modules: module_args,
            shallow: true,
            glues,
            exclude_file: None,
        })?;
    }
    for m in mods.values_mut() {
        if m.path.is_empty() || m.committed_bridge {
            continue;
        }
        let file = bridge_dir.join(bridge_file_name(&m.path));
        if file.is_file() {
            let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
            // a module with no bridged hook still has an (empty) `hook_bridge`; keep it only when it bridges something
            if text.contains("pub const BRIDGED: &[u32] = &[\n    ];") {
                std::fs::remove_file(&file).ok();
                stats.empty_modules += 1;
                continue;
            }
            m.has_bridge = true;
            stats.bridged_modules += 1;
            appended
                .entry(m.file.clone())
                .or_default()
                .push_str(&format!("\n#[cfg(feature = \"yoake2-slice\")]\ninclude!(\"{}\");\n", rel_from(&m.file, &file)));
        }
    }

    // 4. the chain functions: a module gets them when it bridges something itself or a child has them (bottom-up)
    let mut chain: BTreeSet<Vec<String>> = BTreeSet::new();
    let mut by_depth: Vec<Vec<String>> = mods.keys().cloned().collect();
    by_depth.sort_by_key(|p| std::cmp::Reverse(p.len()));
    for path in by_depth {
        let m = &mods[&path];
        let child_has = m.children.iter().any(|(name, _)| {
            let mut c = path.clone();
            c.push(name.clone());
            chain.contains(&c)
        });
        if m.has_bridge || child_has {
            chain.insert(path.clone());
        }
    }
    for (path, m) in mods.iter_mut() {
        let keep: Vec<(String, Vec<String>)> = m
            .children
            .iter()
            .filter(|(name, _)| {
                let mut c = path.clone();
                c.push(name.clone());
                chain.contains(&c)
            })
            .cloned()
            .collect();
        m.children = keep;
    }
    let bridge_fns = |m: &FileModule, own: bool| -> String {
        let mut s = String::new();
        s.push_str("\n#[cfg(feature = \"yoake2-slice\")]\n#[doc(hidden)]\n#[allow(dead_code)]\n");
        s.push_str("pub(crate) fn __yoake_bridge_install(registered: &[(u32, i32)], out: &mut yoake_slice::bridge::InstallReport) {\n");
        if own {
            s.push_str("    out.merge(hook_bridge::install(registered));\n");
        }
        for (name, cfgs) in &m.children {
            for c in cfgs {
                s.push_str(&format!("    {c}\n"));
            }
            s.push_str(&format!("    {name}::__yoake_bridge_install(registered, out);\n"));
        }
        s.push_str("}\n\n#[cfg(feature = \"yoake2-slice\")]\n#[doc(hidden)]\n#[allow(dead_code)]\n");
        s.push_str("pub(crate) fn __yoake_bridge_mode(id: u32) -> Option<yoake_slice::bridge::Mode> {\n    let mut found = None;\n");
        if own {
            s.push_str("    if let Some(mode) = hook_bridge::mode(id) { found = Some(mode); }\n");
        }
        for (name, cfgs) in &m.children {
            for c in cfgs {
                s.push_str(&format!("    {c}\n"));
            }
            s.push_str(&format!("    if found.is_none() {{ found = {name}::__yoake_bridge_mode(id); }}\n"));
        }
        s.push_str("    found\n}\n");
        s
    };
    for m in mods.values() {
        if m.path.is_empty() {
            continue;
        }
        // a module that keeps a committed bridge is wired by hand and also chains its children
        let own = m.has_bridge;
        if !own && m.children.is_empty() {
            continue;
        }
        let mut text = bridge_fns(m, own);
        if m.committed_bridge {
            text = text.replace("    out.merge(hook_bridge::install(registered));\n", "");
        }
        appended.entry(m.file.clone()).or_default().push_str(&text);
    }
    // the crate root exports the whole chain
    let root = &mods[&Vec::<String>::new()];
    let mut root_text = bridge_fns(root, false);
    root_text = root_text.replace("pub(crate) fn __yoake_bridge_install", "pub fn __yoake_install_bridges_into").replace("pub(crate) fn __yoake_bridge_mode", "pub fn __yoake_bridge_mode");
    root_text.push_str(
        "\n#[cfg(feature = \"yoake2-slice\")]\n#[doc(hidden)]\npub fn __yoake_install_bridges(registered: &[(u32, i32)]) -> yoake_slice::bridge::InstallReport {\n    let mut report = yoake_slice::bridge::InstallReport::default();\n    __yoake_install_bridges_into(registered, &mut report);\n    report\n}\n",
    );
    appended.entry(root.file.clone()).or_default().push_str(&root_text);
    for (file, text) in appended {
        let mut s = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        s.push_str(&text);
        std::fs::write(&file, s).map_err(|e| e.to_string())?;
    }
    let _ = BTreeSet::<()>::new();
    Ok(stats)
}

#[derive(Default, Debug)]
pub struct BridgeStats {
    pub bridged_modules: usize,
    pub empty_modules: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn include_paths_are_relative_to_the_including_file() {
        let file = Path::new("/w/crates/c/src/game/chat.rs");
        let glue = Path::new("/w/crates/c/src/yoake_glue/hook_bridge_game__chat.rs");
        assert_eq!(rel_from(file, glue), "../yoake_glue/hook_bridge_game__chat.rs");
        let sibling = Path::new("/w/crates/c/src/game/yoake_glue/x.rs");
        assert_eq!(rel_from(file, sibling), "yoake_glue/x.rs");
        assert_eq!(bridge_file_name(&["game".into(), "chat".into()]), "hook_bridge_game__chat.rs");
    }
}
