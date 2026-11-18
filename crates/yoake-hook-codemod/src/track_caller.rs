use std::collections::BTreeSet;
use std::path::Path;

use syn::{Attribute, ImplItem, Item, TraitItem};

use crate::types::CrateIndex;
use crate::{key, modules};

/// Built-in non-exhaustive list of `std`/`core`/`alloc` methods that carry `#[track_caller]`.
pub const TRACK_CALLER_STD_LEAVES: &[&str] = &[
    // Option
    "core::option::Option::unwrap",
    "std::option::Option::unwrap",
    "core::option::Option::expect",
    "std::option::Option::expect",
    "core::option::Option::unwrap_unchecked",
    "std::option::Option::unwrap_unchecked",
    // Result
    "core::result::Result::unwrap",
    "std::result::Result::unwrap",
    "core::result::Result::expect",
    "std::result::Result::expect",
    "core::result::Result::unwrap_err",
    "std::result::Result::unwrap_err",
    "core::result::Result::expect_err",
    "std::result::Result::expect_err",
    "core::result::Result::unwrap_unchecked",
    "std::result::Result::unwrap_unchecked",
    "core::result::Result::unwrap_err_unchecked",
    "std::result::Result::unwrap_err_unchecked",
    // Index / IndexMut / SliceIndex
    "<Self as core::ops::index::Index>::index",
    "<Self as core::ops::Index>::index",
    "<Self as std::ops::index::Index>::index",
    "<Self as std::ops::Index>::index",
    "<Self as core::ops::index::IndexMut>::index_mut",
    "<Self as core::ops::IndexMut>::index_mut",
    "<Self as std::ops::index::IndexMut>::index_mut",
    "<Self as std::ops::IndexMut>::index_mut",
    "<Self as core::slice::index::SliceIndex>::index",
    "<Self as core::slice::index::SliceIndex>::index_mut",
    "<Self as std::slice::SliceIndex>::index",
    "<Self as std::slice::SliceIndex>::index_mut",
    // str
    "str::split_at",
    "str::split_at_mut",
    // RefCell
    "core::cell::RefCell::borrow",
    "std::cell::RefCell::borrow",
    "core::cell::RefCell::borrow_mut",
    "std::cell::RefCell::borrow_mut",
    "core::cell::RefCell::replace",
    "std::cell::RefCell::replace",
    "core::cell::RefCell::replace_with",
    "std::cell::RefCell::replace_with",
    "core::cell::RefCell::swap",
    "std::cell::RefCell::swap",
    // Vec
    "alloc::vec::Vec::remove",
    "std::vec::Vec::remove",
    "alloc::vec::Vec::insert",
    "std::vec::Vec::insert",
    "alloc::vec::Vec::swap_remove",
    "std::vec::Vec::swap_remove",
    "alloc::vec::Vec::drain",
    "std::vec::Vec::drain",
    "alloc::vec::Vec::split_off",
    "std::vec::Vec::split_off",
    "alloc::vec::Vec::splice",
    "std::vec::Vec::splice",
    "alloc::vec::Vec::extend_from_within",
    "std::vec::Vec::extend_from_within",
    // String
    "alloc::string::String::remove",
    "std::string::String::remove",
    "alloc::string::String::insert",
    "std::string::String::insert",
    "alloc::string::String::insert_str",
    "std::string::String::insert_str",
    "alloc::string::String::drain",
    "std::string::String::drain",
    "alloc::string::String::split_off",
    "std::string::String::split_off",
    "alloc::string::String::replace_range",
    "std::string::String::replace_range",
    // VecDeque
    "alloc::collections::vec_deque::VecDeque::remove",
    "std::collections::vec_deque::VecDeque::remove",
    "std::collections::VecDeque::remove",
    "alloc::collections::vec_deque::VecDeque::insert",
    "std::collections::vec_deque::VecDeque::insert",
    "std::collections::VecDeque::insert",
    "alloc::collections::vec_deque::VecDeque::swap_remove_front",
    "std::collections::vec_deque::VecDeque::swap_remove_front",
    "std::collections::VecDeque::swap_remove_front",
    "alloc::collections::vec_deque::VecDeque::swap_remove_back",
    "std::collections::vec_deque::VecDeque::swap_remove_back",
    "std::collections::VecDeque::swap_remove_back",
    "alloc::collections::vec_deque::VecDeque::drain",
    "std::collections::vec_deque::VecDeque::drain",
    "std::collections::VecDeque::drain",
    "alloc::collections::vec_deque::VecDeque::split_off",
    "std::collections::vec_deque::VecDeque::split_off",
    "std::collections::VecDeque::split_off",
    "alloc::collections::vec_deque::VecDeque::swap",
    "std::collections::vec_deque::VecDeque::swap",
    "std::collections::VecDeque::swap",
    // Iterator
    "<Self as core::iter::traits::iterator::Iterator>::step_by",
    "<Self as core::iter::Iterator>::step_by",
    "<Self as std::iter::Iterator>::step_by",
];

const TRACK_CALLER_SLICE_METHODS: &[&str] = &[
    "split_at",
    "split_at_mut",
    "swap",
    "copy_from_slice",
    "clone_from_slice",
    "copy_within",
    "chunks",
    "chunks_mut",
    "chunks_exact",
    "chunks_exact_mut",
    "rchunks",
    "rchunks_mut",
    "rchunks_exact",
    "rchunks_exact_mut",
    "windows",
    "rotate_left",
    "rotate_right",
];

pub fn is_track_caller(callee_path: &str, workspace_set: &BTreeSet<String>) -> bool {
    if TRACK_CALLER_STD_LEAVES.contains(&callee_path) || workspace_set.contains(callee_path) {
        return true;
    }
    if let Some((owner, method)) = callee_path.rsplit_once("::")
        && ((owner.starts_with('[') && owner.ends_with(']'))
            || owner == "core::slice"
            || owner == "std::slice")
        && TRACK_CALLER_SLICE_METHODS.contains(&method)
    {
        return true;
    }
    false
}

pub fn load_track_caller_file(ledger_dir: &Path) -> Result<BTreeSet<String>, String> {
    let path = ledger_dir.join("track-caller.txt");
    if !path.is_file() {
        return Ok(BTreeSet::new());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut out = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.split('#').next().unwrap_or("").trim();
        if !trimmed.is_empty() {
            out.insert(trimmed.to_string());
        }
    }
    Ok(out)
}

fn has_track_caller(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("track_caller"))
}

pub fn run_scan(workspace: &Path, out: &Path) -> Result<bool, String> {
    let manifest_path = workspace.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("failed to read {}: {e}", manifest_path.display()))?;
    let (raw_members, exclude) = parse_workspace_manifest(&manifest)?;
    if raw_members.is_empty() {
        return Err(format!("no [workspace] members found in {}", manifest_path.display()));
    }
    let members = resolve_workspace_members(workspace, &raw_members, &exclude)?;
    if members.is_empty() {
        return Err(format!(
            "no workspace members resolved in {}",
            manifest_path.display()
        ));
    }

    let mut found = BTreeSet::new();
    for member in &members {
        let member_dir = workspace.join(member);
        let member_manifest_path = member_dir.join("Cargo.toml");
        let member_manifest = std::fs::read_to_string(&member_manifest_path)
            .map_err(|e| format!("failed to read {}: {e}", member_manifest_path.display()))?;
        let Some((pkg_name, lib_name, first_bin_path)) = parse_crate_manifest(&member_manifest) else {
            continue;
        };
        let crate_name = lib_name.as_deref().unwrap_or(&pkg_name);
        let crate_ident = key::normalize_crate_name(crate_name);

        if modules::find_crate_root_file(&member_dir, crate_name).is_err() {
            if let Some(bin_rel) = first_bin_path
                && member_dir.join(&bin_rel).is_file()
            {
                scan_single_root_file(&member_dir, &member_dir.join(&bin_rel), &crate_ident, &mut found)?;
                continue;
            }
            return Err(format!(
                "could not find crate root for workspace member '{}' under {}",
                member,
                member_dir.display()
            ));
        }

        let crate_index = CrateIndex::build(&member_dir, crate_name, &crate_ident, &[])?;
        modules::walk_crate_tree(&member_dir, crate_name, &[], |mod_path, _, items| {
            collect_items_track_caller(&crate_ident, Some(&crate_index), mod_path, items, &mut found);
        })?;
    }

    if let Some(parent) = out.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create {}: {e}", parent.display()))?;
    }
    let mut contents = String::new();
    for path in &found {
        contents.push_str(path);
        contents.push('\n');
    }
    std::fs::write(out, contents)
        .map_err(|e| format!("failed to write {}: {e}", out.display()))?;
    println!(
        "track-caller-scan: wrote {} paths from {} workspace members to {}",
        found.len(),
        members.len(),
        out.display()
    );
    Ok(true)
}

fn scan_single_root_file(
    crate_root: &Path,
    root_file: &Path,
    crate_ident: &str,
    found: &mut BTreeSet<String>,
) -> Result<(), String> {
    let source = std::fs::read_to_string(root_file)
        .map_err(|e| format!("failed to read {}: {e}", root_file.display()))?;
    let parsed = syn::parse_file(&source)
        .map_err(|e| format!("failed to parse {}: {e}", root_file.display()))?;
    let idx = CrateIndex::build(crate_root, crate_ident, crate_ident, &[]).ok();
    collect_items_track_caller(crate_ident, idx.as_ref(), &[], &parsed.items, found);
    Ok(())
}

fn fallback_type_name(ty: &syn::Type) -> String {
    let seg = key::type_key_segment(ty);
    seg.split('<').next().unwrap_or(&seg).trim().to_string()
}

fn collect_items_track_caller(
    crate_ident: &str,
    crate_index: Option<&CrateIndex>,
    mod_path: &[String],
    items: &[Item],
    found: &mut BTreeSet<String>,
) {
    let prefix = key::module_prefix(crate_ident, mod_path);
    for item in items {
        match item {
            Item::Fn(item_fn) if !modules::has_cfg_test(&item_fn.attrs) => {
                if has_track_caller(&item_fn.attrs) {
                    found.insert(format!("{prefix}::{}", item_fn.sig.ident));
                }
            }
            Item::Trait(item_trait) if !modules::has_cfg_test(&item_trait.attrs) => {
                for trait_item in &item_trait.items {
                    if let TraitItem::Fn(method) = trait_item
                        && !modules::has_cfg_test(&method.attrs)
                        && has_track_caller(&method.attrs)
                    {
                        found.insert(format!(
                            "<Self as {prefix}::{}>::{}",
                            item_trait.ident, method.sig.ident
                        ));
                    }
                }
            }
            Item::Impl(item_impl) if !modules::has_cfg_test(&item_impl.attrs) => {
                if let Some((_, trait_path, _)) = &item_impl.trait_ {
                    let trait_canon = crate_index
                        .and_then(|ci| ci.resolve_trait_canon(mod_path, trait_path))
                        .unwrap_or_else(|| key::trait_key_segment(trait_path, true));
                    for impl_item in &item_impl.items {
                        if let ImplItem::Fn(method) = impl_item
                            && !modules::has_cfg_test(&method.attrs)
                            && has_track_caller(&method.attrs)
                        {
                            found.insert(format!(
                                "<Self as {trait_canon}>::{}",
                                method.sig.ident
                            ));
                        }
                    }
                } else {
                    let owner_canon = crate_index
                        .and_then(|ci| ci.resolve_local_type_canon(mod_path, &item_impl.self_ty))
                        .unwrap_or_else(|| {
                            format!("{prefix}::{}", fallback_type_name(&item_impl.self_ty))
                        });
                    for impl_item in &item_impl.items {
                        if let ImplItem::Fn(method) = impl_item
                            && !modules::has_cfg_test(&method.attrs)
                            && has_track_caller(&method.attrs)
                        {
                            found.insert(format!("{owner_canon}::{}", method.sig.ident));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WorkspaceField {
    Members,
    Exclude,
}

fn parse_workspace_manifest(manifest: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut in_workspace = false;
    let mut collecting: Option<WorkspaceField> = None;
    let mut members = Vec::new();
    let mut exclude = Vec::new();

    for raw_line in manifest.lines() {
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if collecting.is_none() && line.starts_with('[') && line.ends_with(']') {
            in_workspace = line == "[workspace]";
            continue;
        }
        if !in_workspace {
            continue;
        }
        let (field, slice) = if let Some(field) = collecting {
            (field, line)
        } else if let Some((k, v)) = line.split_once('=') {
            let field = match k.trim() {
                "members" => WorkspaceField::Members,
                "exclude" => WorkspaceField::Exclude,
                _ => continue,
            };
            collecting = Some(field);
            (field, v.trim())
        } else {
            continue;
        };
        let target = match field {
            WorkspaceField::Members => &mut members,
            WorkspaceField::Exclude => &mut exclude,
        };
        extract_quoted_strings(slice, target);
        if has_closing_bracket(slice) {
            collecting = None;
        }
    }
    Ok((members, exclude))
}

#[cfg(test)]
fn parse_workspace_members(manifest: &str) -> Result<Vec<String>, String> {
    parse_workspace_manifest(manifest).map(|(members, _)| members)
}

fn has_closing_bracket(slice: &str) -> bool {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for ch in slice.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_double && ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            ']' if !in_double && !in_single => return true,
            _ => {}
        }
    }
    false
}

fn normalize_rel_segments(raw: &str) -> Vec<String> {
    raw.replace('\\', "/")
        .split('/')
        .filter(|seg| !seg.is_empty() && *seg != ".")
        .map(ToString::to_string)
        .collect()
}

fn join_rel_segments(segs: &[String]) -> String {
    if segs.is_empty() {
        ".".to_string()
    } else {
        segs.join("/")
    }
}

fn has_glob_meta(seg: &str) -> bool {
    seg.contains('*') || seg.contains('?') || seg.contains('[')
}

fn match_char_class(class: &[char], ch: char) -> bool {
    let (negated, rest) = match class.first() {
        Some('!') | Some('^') => (true, &class[1..]),
        _ => (false, class),
    };
    let mut matched = false;
    let mut i = 0;
    while i < rest.len() {
        if i + 2 < rest.len() && rest[i + 1] == '-' {
            if (rest[i]..=rest[i + 2]).contains(&ch) {
                matched = true;
            }
            i += 3;
        } else {
            if ch == rest[i] {
                matched = true;
            }
            i += 1;
        }
    }
    if negated { !matched } else { matched }
}

fn match_segment(pat: &str, text: &str) -> bool {
    let pat: Vec<char> = pat.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_pi, mut star_ti) = (None, 0usize);
    while ti < text.len() {
        if pi < pat.len() && pat[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if pi < pat.len() && pat[pi] == '?' {
            pi += 1;
            ti += 1;
        } else if pi < pat.len()
            && pat[pi] == '['
            && let Some(rel_end) = pat[pi + 1..].iter().position(|&c| c == ']')
        {
            let class = &pat[pi + 1..pi + 1 + rel_end];
            if match_char_class(class, text[ti]) {
                pi += rel_end + 2;
                ti += 1;
            } else if let Some(sp) = star_pi {
                star_ti += 1;
                ti = star_ti;
                pi = sp + 1;
            } else {
                return false;
            }
        } else if pi < pat.len() && pat[pi] == text[ti] {
            pi += 1;
            ti += 1;
        } else if let Some(sp) = star_pi {
            star_ti += 1;
            ti = star_ti;
            pi = sp + 1;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == '*' {
        pi += 1;
    }
    pi == pat.len()
}

fn match_glob_segments(pat: &[String], path: &[String]) -> bool {
    let Some((first, rest)) = pat.split_first() else {
        return path.is_empty();
    };
    if first == "**" {
        match_glob_segments(rest, path)
            || (!path.is_empty() && match_glob_segments(pat, &path[1..]))
    } else {
        !path.is_empty()
            && match_segment(first, &path[0])
            && match_glob_segments(rest, &path[1..])
    }
}

fn is_excluded(rel_segs: &[String], exclude: &[Vec<String>]) -> bool {
    exclude.iter().any(|ex| {
        if ex.is_empty() {
            return false;
        }
        (1..=rel_segs.len()).any(|k| match_glob_segments(ex, &rel_segs[..k]))
    })
}

fn expand_glob_members(
    base_dir: &Path,
    rel_segs: &[String],
    pat_segs: &[String],
    exclude: &[Vec<String>],
    out: &mut BTreeSet<String>,
) -> Result<(), String> {
    let Some((first, rest)) = pat_segs.split_first() else {
        if !is_excluded(rel_segs, exclude) && base_dir.join("Cargo.toml").is_file() {
            out.insert(join_rel_segments(rel_segs));
        }
        return Ok(());
    };

    if first == "**" {
        expand_glob_members(base_dir, rel_segs, rest, exclude, out)?;
        if base_dir.is_dir() {
            let entries = std::fs::read_dir(base_dir)
                .map_err(|e| format!("failed to read {}: {e}", base_dir.display()))?;
            for entry in entries {
                let entry =
                    entry.map_err(|e| format!("failed to read {}: {e}", base_dir.display()))?;
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == ".git" || name == "target" {
                    continue;
                }
                let mut next_rel = rel_segs.to_vec();
                next_rel.push(name);
                if !is_excluded(&next_rel, exclude) {
                    expand_glob_members(&path, &next_rel, pat_segs, exclude, out)?;
                }
            }
        }
    } else if !has_glob_meta(first) {
        let child = base_dir.join(first);
        if child.is_dir() {
            let mut next_rel = rel_segs.to_vec();
            next_rel.push(first.clone());
            if !is_excluded(&next_rel, exclude) {
                expand_glob_members(&child, &next_rel, rest, exclude, out)?;
            }
        }
    } else if base_dir.is_dir() {
        let entries = std::fs::read_dir(base_dir)
            .map_err(|e| format!("failed to read {}: {e}", base_dir.display()))?;
        for entry in entries {
            let entry =
                entry.map_err(|e| format!("failed to read {}: {e}", base_dir.display()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if match_segment(first, &name) {
                let mut next_rel = rel_segs.to_vec();
                next_rel.push(name);
                if !is_excluded(&next_rel, exclude) {
                    expand_glob_members(&path, &next_rel, rest, exclude, out)?;
                }
            }
        }
    }
    Ok(())
}

fn resolve_workspace_members(
    workspace: &Path,
    raw_members: &[String],
    raw_exclude: &[String],
) -> Result<Vec<String>, String> {
    let exclude: Vec<Vec<String>> = raw_exclude
        .iter()
        .map(String::as_str)
        .map(normalize_rel_segments)
        .collect();
    let mut resolved = BTreeSet::new();
    for raw in raw_members {
        let segs = normalize_rel_segments(raw);
        if segs.iter().any(|s| has_glob_meta(s)) {
            expand_glob_members(workspace, &[], &segs, &exclude, &mut resolved)?;
        } else if !is_excluded(&segs, &exclude) {
            resolved.insert(join_rel_segments(&segs));
        }
    }
    Ok(resolved.into_iter().collect())
}

fn parse_crate_manifest(manifest: &str) -> Option<(String, Option<String>, Option<String>)> {
    let mut section = String::new();
    let mut pkg_name = None;
    let mut lib_name = None;
    let mut first_bin_path = None;

    for raw_line in manifest.lines() {
        let line = strip_toml_comment(raw_line).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line.to_string();
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let Some(val) = unquote_toml_str(v.trim()) else {
            continue;
        };
        match section.as_str() {
            "[package]" if key == "name" => pkg_name = Some(val),
            "[lib]" if key == "name" => lib_name = Some(val),
            "[[bin]]" if key == "path" && first_bin_path.is_none() => first_bin_path = Some(val),
            _ => {}
        }
    }
    pkg_name.map(|p| (p, lib_name, first_bin_path))
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if in_double && ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '"' if !in_single => in_double = !in_double,
            '\'' if !in_double => in_single = !in_single,
            '#' if !in_double && !in_single => return &line[..idx],
            _ => {}
        }
    }
    line
}

fn extract_quoted_strings(slice: &str, out: &mut Vec<String>) {
    let bytes = slice.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != quote {
                if quote == b'"' && bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i <= bytes.len() {
                out.push(slice[start..i].to_string());
            }
            i += 1;
        } else {
            i += 1;
        }
    }
}

fn unquote_toml_str(val: &str) -> Option<String> {
    if val.len() >= 2
        && ((val.starts_with('"') && val.ends_with('"'))
            || (val.starts_with('\'') && val.ends_with('\'')))
    {
        Some(val[1..val.len() - 1].to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_workspace_members_and_std_leaves() {
        let toml = "\
[workspace]
exclude = [\"launcher\", \"crates/skip\"]
members = [
    \"crates/a\", # inline comment
    \"crates/*\",
]
";
        assert_eq!(parse_workspace_members(toml).unwrap(), vec!["crates/a", "crates/*"]);
        let (members, exclude) = parse_workspace_manifest(toml).unwrap();
        assert_eq!(members, vec!["crates/a", "crates/*"]);
        assert_eq!(exclude, vec!["launcher", "crates/skip"]);

        let empty = BTreeSet::new();
        assert!(is_track_caller("std::option::Option::unwrap", &empty));
        assert!(is_track_caller("<Self as core::ops::index::Index>::index", &empty));
        assert!(is_track_caller("[u32]::split_at", &empty));
        assert!(!is_track_caller("std::option::Option::is_some", &empty));
    }

    #[test]
    fn expands_member_globs_and_skips_excludes() {
        let dir = std::env::temp_dir().join(format!(
            "yoake_track_caller_glob_test_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("crates/alpha")).unwrap();
        std::fs::create_dir_all(dir.join("crates/beta")).unwrap();
        std::fs::create_dir_all(dir.join("crates/excluded_crate")).unwrap();
        std::fs::create_dir_all(dir.join("crates/not_a_crate")).unwrap();
        std::fs::create_dir_all(dir.join("tools/explicit_excluded")).unwrap();
        std::fs::write(dir.join("crates/alpha/Cargo.toml"), "[package]\nname = \"alpha\"\n").unwrap();
        std::fs::write(dir.join("crates/beta/Cargo.toml"), "[package]\nname = \"beta\"\n").unwrap();
        std::fs::write(
            dir.join("crates/excluded_crate/Cargo.toml"),
            "[package]\nname = \"excluded-crate\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("tools/explicit_excluded/Cargo.toml"),
            "[package]\nname = \"explicit-excluded\"\n",
        )
        .unwrap();

        let members = vec![
            "crates/*".to_string(),
            "tools/explicit_excluded".to_string(),
        ];
        let exclude = vec![
            "crates/excluded_crate".to_string(),
            "tools/*".to_string(),
        ];
        let resolved = resolve_workspace_members(&dir, &members, &exclude).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(resolved, vec!["crates/alpha", "crates/beta"]);
    }
}
