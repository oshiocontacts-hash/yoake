use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use syn::{Attribute, Expr, ExprLit, Item, Lit, Meta, Token, Visibility, punctuated::Punctuated};

use crate::key;

pub struct ScannedFile {
    pub path: PathBuf,
    pub source: String,
    pub parsed: syn::File,
    pub root_mod_path: Vec<String>,
    pub root_cfg_tags: Vec<String>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleSpec {
    pub crate_root: PathBuf,
    pub crate_name: String,
    pub module: String,
    pub callsites: bool,
    pub shallow: bool,
}

#[allow(dead_code)]
pub fn parse_modules_line(line: &str) -> Result<Option<ModuleSpec>, String> {
    let trimmed = line.split('#').next().unwrap_or("").trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let mut parts = trimmed.split_whitespace();
    let Some(root) = parts.next() else {
        return Ok(None);
    };
    let crate_name = parts
        .next()
        .ok_or_else(|| format!("missing crate name in modules line: '{trimmed}'"))?;
    let module = parts
        .next()
        .ok_or_else(|| format!("missing module path in modules line: '{trimmed}'"))?;
    let mut callsites = false;
    let mut shallow = false;
    for flag in parts {
        if let Some((key, val)) = flag.split_once('=') {
            if key == "callsites" {
                callsites = match val {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(format!(
                            "invalid callsites flag value '{other}' in modules line: '{trimmed}'"
                        ));
                    }
                };
            } else if key == "shallow" {
                shallow = match val {
                    "true" => true,
                    "false" => false,
                    other => {
                        return Err(format!(
                            "invalid shallow flag value '{other}' in modules line: '{trimmed}'"
                        ));
                    }
                };
            }
        }
    }
    Ok(Some(ModuleSpec {
        crate_root: PathBuf::from(root),
        crate_name: crate_name.to_string(),
        module: module.to_string(),
        callsites,
        shallow,
    }))
}

#[allow(dead_code)]
pub fn parse_modules_txt(text: &str) -> Result<Vec<ModuleSpec>, String> {
    let mut out = Vec::new();
    for line in text.lines() {
        if let Some(spec) = parse_modules_line(line)? {
            out.push(spec);
        }
    }
    Ok(out)
}

/// W5: `--module '*'` stands for every top-level `mod` of the crate root (not `cfg(test)`, not in `exclude`).
/// Other arguments pass through. An exclude entry matches a top-level module name exactly.
pub fn expand_star_modules(
    crate_root: &Path,
    crate_name: &str,
    modules: &[String],
    exclude: &[String],
) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    for module in modules {
        if module != "*" {
            if !exclude.iter().any(|e| e == module) {
                out.push(module.clone());
            }
            continue;
        }
        let root_file = find_crate_root_file(crate_root, crate_name)?;
        let source = std::fs::read_to_string(&root_file)
            .map_err(|e| format!("failed to read {}: {e}", root_file.display()))?;
        let parsed = syn::parse_file(&source)
            .map_err(|e| format!("failed to parse {}: {e}", root_file.display()))?;
        for item in &parsed.items {
            if let Item::Mod(item_mod) = item {
                let name = item_mod.ident.to_string();
                if has_cfg_test(&item_mod.attrs)
                    || name.starts_with("__yoake")
                    || exclude.iter().any(|e| *e == name)
                {
                    continue;
                }
                out.push(name);
            }
        }
    }
    Ok(out)
}

pub fn normalize_module_arg(raw: &str) -> Result<Vec<String>, String> {
    let trimmed = raw.trim().strip_prefix("::").unwrap_or(raw.trim());
    let trimmed = trimmed.strip_prefix("crate::").unwrap_or(trimmed);
    if trimmed.is_empty() {
        return Err("module path cannot be empty".to_string());
    }
    let segments: Vec<String> = trimmed.split("::").map(|s| s.trim().to_string()).collect();
    if segments.iter().any(|s| s.is_empty()) {
        return Err(format!("invalid module path '{raw}'"));
    }
    Ok(segments)
}

pub fn is_target_or_descendant(
    mod_path: &[String],
    targets: &[Vec<String>],
    shallow: bool,
) -> bool {
    if shallow {
        targets.iter().any(|target| mod_path == target.as_slice())
    } else {
        targets.iter().any(|target| mod_path.starts_with(target))
    }
}

fn is_ancestor_of_target(mod_path: &[String], targets: &[Vec<String>]) -> bool {
    targets.iter().any(|target| target.starts_with(mod_path))
}

pub fn has_cfg_test(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        let Meta::List(list) = &attr.meta else {
            return false;
        };
        syn::parse2::<Meta>(list.tokens.clone())
            .map(|meta| meta_is_cfg_test(&meta))
            .unwrap_or(false)
    })
}

fn meta_is_cfg_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) => {
            if list.path.is_ident("not") {
                return false;
            }
            let Ok(inner) = syn::parse::Parser::parse2(
                Punctuated::<Meta, Token![,]>::parse_terminated,
                list.tokens.clone(),
            ) else {
                return false;
            };
            if list.path.is_ident("all") {
                inner.iter().any(meta_is_cfg_test)
            } else if list.path.is_ident("any") {
                !inner.is_empty() && inner.iter().all(meta_is_cfg_test)
            } else {
                false
            }
        }
        Meta::NameValue(_) => false,
    }
}

pub(crate) fn attr_path_value(attrs: &[Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("path") {
            continue;
        }
        if let Meta::NameValue(nv) = &attr.meta
            && let Expr::Lit(ExprLit {
                lit: Lit::Str(lit), ..
            }) = &nv.value
        {
            return Some(lit.value());
        }
    }
    None
}

pub fn find_crate_root_file(crate_root: &Path, crate_name: &str) -> Result<PathBuf, String> {
    let manifest_path = crate_root.join("Cargo.toml");
    if manifest_path.is_file() {
        let manifest = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("failed to read {}: {e}", manifest_path.display()))?;
        if let Some(lib_path) = parse_toml_target_path(&manifest, "lib", None) {
            let resolved = crate_root.join(lib_path);
            if resolved.is_file() {
                return Ok(resolved);
            }
            return Err(format!(
                "Cargo.toml [lib] path '{}' does not exist",
                resolved.display()
            ));
        }
        let src_lib = crate_root.join("src/lib.rs");
        if src_lib.is_file() {
            return Ok(src_lib);
        }
        if let Some(bin_path) = parse_toml_target_path(&manifest, "[bin]", Some(crate_name)) {
            let resolved = crate_root.join(bin_path);
            if resolved.is_file() {
                return Ok(resolved);
            }
        }
    }

    for candidate in ["src/lib.rs", "src/main.rs"] {
        let path = crate_root.join(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    Err(format!(
        "could not find crate root file (src/lib.rs, src/main.rs, or Cargo.toml [lib] path) under {}",
        crate_root.display()
    ))
}

fn parse_toml_target_path(manifest: &str, section: &str, expected_name: Option<&str>) -> Option<String> {
    let mut in_section = false;
    let mut current_name: Option<String> = None;
    let mut current_path: Option<String> = None;
    let header = format!("[{section}]");

    for raw_line in manifest.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if in_section
                && let Some(path) = current_path.take()
                && expected_name.is_none_or(|n| current_name.as_deref() == Some(n))
            {
                return Some(path);
            }
            in_section = line == header;
            current_name = None;
            current_path = None;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((key, val)) = line.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            if let Some(unquoted) = unquote_toml(val) {
                match key {
                    "path" => current_path = Some(unquoted),
                    "name" => current_name = Some(unquoted),
                    _ => {}
                }
            }
        }
    }

    if in_section
        && let Some(path) = current_path
        && expected_name.is_none_or(|n| current_name.as_deref() == Some(n))
    {
        return Some(path);
    }
    None
}

pub fn detect_crate_edition(crate_root: &Path) -> String {
    let manifest_path = crate_root.join("Cargo.toml");
    let Ok(manifest) = std::fs::read_to_string(&manifest_path) else {
        return "2024".to_string();
    };
    parse_package_edition(&manifest).unwrap_or_else(|| "2024".to_string())
}

fn parse_package_edition(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for raw_line in manifest.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        if let Some((key, val)) = line.split_once('=')
            && key.trim() == "edition"
        {
            return unquote_toml(val.trim());
        }
    }
    None
}

fn unquote_toml(val: &str) -> Option<String> {
    if val.len() >= 2
        && ((val.starts_with('"') && val.ends_with('"'))
            || (val.starts_with('\'') && val.ends_with('\'')))
    {
        Some(val[1..val.len() - 1].to_string())
    } else {
        None
    }
}

pub fn collect_module_files(
    crate_root: &Path,
    crate_name: &str,
    targets: &[Vec<String>],
    shallow: bool,
) -> Result<Vec<ScannedFile>, String> {
    let root_file = find_crate_root_file(crate_root, crate_name)?;
    let root_dir = root_file
        .parent()
        .ok_or_else(|| format!("crate root {} has no parent directory", root_file.display()))?
        .to_path_buf();

    let mut files = Vec::new();
    let mut found_targets = BTreeSet::new();
    let mut visited_files = BTreeSet::new();

    load_module_file(
        &root_file,
        &root_dir,
        Vec::new(),
        Vec::new(),
        targets,
        shallow,
        &mut files,
        &mut found_targets,
        &mut visited_files,
    )?;

    for target in targets {
        if !found_targets.contains(target) {
            return Err(format!(
                "module '{}' not found in crate '{}'",
                target.join("::"),
                crate_name
            ));
        }
    }

    Ok(files)
}

#[allow(clippy::too_many_arguments)]
fn load_module_file(
    file_path: &Path,
    dir_for_children: &Path,
    mod_path: Vec<String>,
    cfg_tags: Vec<String>,
    targets: &[Vec<String>],
    shallow: bool,
    files: &mut Vec<ScannedFile>,
    found_targets: &mut BTreeSet<Vec<String>>,
    visited_files: &mut BTreeSet<(PathBuf, Vec<String>, Vec<String>)>,
) -> Result<(), String> {
    let canonical = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    if !visited_files.insert((canonical, mod_path.clone(), cfg_tags.clone())) {
        return Ok(());
    }

    let source = std::fs::read_to_string(file_path)
        .map_err(|e| format!("failed to read {}: {e}", file_path.display()))?;
    let parsed = syn::parse_file(&source)
        .map_err(|e| format!("failed to parse {}: {e}", file_path.display()))?;

    if targets.contains(&mod_path) {
        found_targets.insert(mod_path.clone());
    }

    let mut child_files = Vec::new();
    collect_child_mods(
        &parsed.items,
        dir_for_children,
        &mod_path,
        &cfg_tags,
        targets,
        shallow,
        found_targets,
        &mut child_files,
    )?;

    let include_this_file = is_target_or_descendant(&mod_path, targets, shallow)
        || has_inline_target(&parsed.items, &mod_path, targets, shallow);
    if include_this_file {
        files.push(ScannedFile {
            path: file_path.to_path_buf(),
            source,
            parsed,
            root_mod_path: mod_path,
            root_cfg_tags: cfg_tags,
        });
    }

    for (child_file, child_dir, child_mod, child_cfgs) in child_files {
        load_module_file(
            &child_file,
            &child_dir,
            child_mod,
            child_cfgs,
            targets,
            shallow,
            files,
            found_targets,
            visited_files,
        )?;
    }

    Ok(())
}

fn has_inline_target(
    items: &[Item],
    mod_path: &[String],
    targets: &[Vec<String>],
    shallow: bool,
) -> bool {
    for item in items {
        let Item::Mod(item_mod) = item else {
            continue;
        };
        if has_cfg_test(&item_mod.attrs) {
            continue;
        }
        let Some((_, inner)) = &item_mod.content else {
            continue;
        };
        let mut child_path = mod_path.to_vec();
        child_path.push(item_mod.ident.to_string());
        if is_target_or_descendant(&child_path, targets, shallow)
            || (is_ancestor_of_target(&child_path, targets)
                && has_inline_target(inner, &child_path, targets, shallow))
        {
            return true;
        }
    }
    false
}

#[allow(clippy::too_many_arguments)]
fn collect_child_mods(
    items: &[Item],
    dir_for_children: &Path,
    mod_path: &[String],
    cfg_tags: &[String],
    targets: &[Vec<String>],
    shallow: bool,
    found_targets: &mut BTreeSet<Vec<String>>,
    out: &mut Vec<(PathBuf, PathBuf, Vec<String>, Vec<String>)>,
) -> Result<(), String> {
    for item in items {
        let Item::Mod(item_mod) = item else {
            continue;
        };
        if has_cfg_test(&item_mod.attrs) {
            continue;
        }
        let name = item_mod.ident.to_string();
        let mut child_mod = mod_path.to_vec();
        child_mod.push(name.clone());

        if !is_target_or_descendant(&child_mod, targets, shallow)
            && !is_ancestor_of_target(&child_mod, targets)
        {
            continue;
        }

        let mut child_cfgs = cfg_tags.to_vec();
        child_cfgs.extend(key::extract_cfg_tags(&item_mod.attrs));

        if let Some((_, inner)) = &item_mod.content {
            if targets.contains(&child_mod) {
                found_targets.insert(child_mod.clone());
            }
            let child_dir = match attr_path_value(&item_mod.attrs) {
                Some(rel) => dir_for_children.join(rel),
                None => dir_for_children.join(&name),
            };
            collect_child_mods(
                inner,
                &child_dir,
                &child_mod,
                &child_cfgs,
                targets,
                shallow,
                found_targets,
                out,
            )?;
        } else {
            let child_file = resolve_external_mod(dir_for_children, &name, &item_mod.attrs)?;
            let child_dir = child_mod_dir(dir_for_children, &child_file, &name);
            out.push((child_file, child_dir, child_mod, child_cfgs));
        }
    }
    Ok(())
}

fn child_mod_dir(dir_for_children: &Path, child_file: &Path, name: &str) -> PathBuf {
    if child_file.file_name().and_then(|s| s.to_str()) == Some("mod.rs") {
        child_file
            .parent()
            .unwrap_or(dir_for_children)
            .to_path_buf()
    } else {
        let stem = child_file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(name);
        child_file
            .parent()
            .unwrap_or(dir_for_children)
            .join(stem)
    }
}

fn resolve_external_mod(
    dir_for_children: &Path,
    name: &str,
    attrs: &[Attribute],
) -> Result<PathBuf, String> {
    if let Some(rel) = attr_path_value(attrs) {
        let path = dir_for_children.join(&rel);
        if path.is_file() {
            return Ok(path);
        }
        // F1 spike: for a `#[path]` module declared in a non-mod-rs file (`foo.rs`), rustc resolves the
        // path relative to the directory of that file, not `foo/`.
        if let Some(parent) = dir_for_children.parent() {
            let alt = parent.join(&rel);
            if alt.is_file() {
                return Ok(alt);
            }
        }
        return Err(format!(
            "module '{name}' #[path = \"{rel}\"] resolved to missing file {}",
            path.display()
        ));
    }

    let direct = dir_for_children.join(format!("{name}.rs"));
    if direct.is_file() {
        return Ok(direct);
    }
    let mod_rs = dir_for_children.join(name).join("mod.rs");
    if mod_rs.is_file() {
        return Ok(mod_rs);
    }
    Err(format!(
        "could not find module file for '{name}' (tried {} and {})",
        direct.display(),
        mod_rs.display()
    ))
}

pub fn walk_crate_tree<F>(
    crate_root: &Path,
    crate_name: &str,
    cached_files: &[ScannedFile],
    mut on_module: F,
) -> Result<(), String>
where
    F: FnMut(&[String], Option<&Visibility>, &[Item]),
{
    let root_file = find_crate_root_file(crate_root, crate_name)?;
    let root_dir = root_file
        .parent()
        .ok_or_else(|| format!("crate root {} has no parent directory", root_file.display()))?
        .to_path_buf();

    let mut cache: HashMap<PathBuf, &syn::File> = HashMap::new();
    for sf in cached_files {
        let canonical = sf.path.canonicalize().unwrap_or_else(|_| sf.path.clone());
        cache.insert(canonical, &sf.parsed);
    }

    let mut visited = BTreeSet::new();
    walk_module_file(
        &root_file,
        &root_dir,
        Vec::new(),
        None,
        &cache,
        &mut visited,
        &mut on_module,
    )
}

fn walk_module_file<F>(
    file_path: &Path,
    dir_for_children: &Path,
    mod_path: Vec<String>,
    mod_vis: Option<&Visibility>,
    cache: &HashMap<PathBuf, &syn::File>,
    visited: &mut BTreeSet<(PathBuf, Vec<String>)>,
    on_module: &mut F,
) -> Result<(), String>
where
    F: FnMut(&[String], Option<&Visibility>, &[Item]),
{
    let canonical = file_path
        .canonicalize()
        .unwrap_or_else(|_| file_path.to_path_buf());
    if !visited.insert((canonical.clone(), mod_path.clone())) {
        return Ok(());
    }

    let owned_file;
    let parsed: &syn::File = if let Some(cached) = cache.get(&canonical) {
        cached
    } else {
        let Ok(source) = std::fs::read_to_string(file_path) else {
            return Ok(());
        };
        let Ok(file) = syn::parse_file(&source) else {
            return Ok(());
        };
        owned_file = file;
        &owned_file
    };

    on_module(&mod_path, mod_vis, &parsed.items);
    walk_child_items(
        &parsed.items,
        dir_for_children,
        &mod_path,
        cache,
        visited,
        on_module,
    )
}

fn walk_child_items<F>(
    items: &[Item],
    dir_for_children: &Path,
    mod_path: &[String],
    cache: &HashMap<PathBuf, &syn::File>,
    visited: &mut BTreeSet<(PathBuf, Vec<String>)>,
    on_module: &mut F,
) -> Result<(), String>
where
    F: FnMut(&[String], Option<&Visibility>, &[Item]),
{
    for item in items {
        let Item::Mod(item_mod) = item else {
            continue;
        };
        if has_cfg_test(&item_mod.attrs) {
            continue;
        }
        let name = item_mod.ident.to_string();
        let mut child_mod = mod_path.to_vec();
        child_mod.push(name.clone());

        if let Some((_, inner)) = &item_mod.content {
            on_module(&child_mod, Some(&item_mod.vis), inner);
            let child_dir = match attr_path_value(&item_mod.attrs) {
                Some(rel) => dir_for_children.join(rel),
                None => dir_for_children.join(&name),
            };
            walk_child_items(inner, &child_dir, &child_mod, cache, visited, on_module)?;
        } else if let Ok(child_file) = resolve_external_mod(dir_for_children, &name, &item_mod.attrs) {
            let child_dir = child_mod_dir(dir_for_children, &child_file, &name);
            walk_module_file(
                &child_file,
                &child_dir,
                child_mod,
                Some(&item_mod.vis),
                cache,
                visited,
                on_module,
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_attribute_in_a_non_mod_rs_file_is_relative_to_that_files_directory() {
        let dir = std::env::temp_dir().join(format!("yoake-codemod-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "pub mod outer;
").unwrap();
        std::fs::write(
            dir.join("src/outer.rs"),
            "#[path = \"outer_inner.rs\"]
mod inner;
pub fn a() {}
",
        )
        .unwrap();
        std::fs::write(dir.join("src/outer_inner.rs"), "pub fn b() {}
").unwrap();
        let files = collect_module_files(&dir, "c", &[vec!["outer".to_string()]], false).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"outer_inner.rs".to_string()), "{names:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_package_edition_handles_explicit_and_workspace_inherited() {
        let explicit = "[package]\nname = \"a\"\nedition = \"2021\"\n";
        assert_eq!(parse_package_edition(explicit).as_deref(), Some("2021"));

        let inherited = "[package]\nname = \"a\"\nedition.workspace = true\n";
        assert_eq!(parse_package_edition(inherited), None);

        let table = "[package]\nname = \"a\"\nedition = { workspace = true }\n";
        assert_eq!(parse_package_edition(table), None);
    }

    #[test]
    fn parse_modules_txt_supports_callsites_shallow_and_ignores_unknown_flags() {
        let input = "\
# comment
crates/horizon-client-lib horizon_client_lib entities
crates/horizon-client horizon_client game::item_use callsites=true foo=bar
crates/horizon-client-lib horizon_client_lib mobs shallow=true
crates/demo demo mod_a callsites=false shallow=false other=123
";
        let specs = parse_modules_txt(input).unwrap();
        assert_eq!(
            specs,
            vec![
                ModuleSpec {
                    crate_root: PathBuf::from("crates/horizon-client-lib"),
                    crate_name: "horizon_client_lib".to_string(),
                    module: "entities".to_string(),
                    callsites: false,
                    shallow: false,
                },
                ModuleSpec {
                    crate_root: PathBuf::from("crates/horizon-client"),
                    crate_name: "horizon_client".to_string(),
                    module: "game::item_use".to_string(),
                    callsites: true,
                    shallow: false,
                },
                ModuleSpec {
                    crate_root: PathBuf::from("crates/horizon-client-lib"),
                    crate_name: "horizon_client_lib".to_string(),
                    module: "mobs".to_string(),
                    callsites: false,
                    shallow: true,
                },
                ModuleSpec {
                    crate_root: PathBuf::from("crates/demo"),
                    crate_name: "demo".to_string(),
                    module: "mod_a".to_string(),
                    callsites: false,
                    shallow: false,
                },
            ]
        );
    }
}
