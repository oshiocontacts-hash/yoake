use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::model::{CallKind, Resolution};
use crate::span::strip_defid_hashes;

const UNWRITABLE: &[&str] = &[
    "{closure", "Closure(", "{async", "Async(", "{coroutine", "Coroutine(", "impl ",
    "Alias(", "Opaque", "FnDef", "{type error}", "Param(",
];

/// Workspace-derived path normalization tables for a THIR dump.
#[derive(Clone, Debug, Default)]
pub struct NormalizeContext {
    pub externs: BTreeSet<String>,
    /// Pairs `"wrapper::target" -> "target"` where `rustc` pretty-prints a re-exported extern
    /// crate (`pub use target as alias;`) as `wrapper::target::...`, while `target` is directly
    /// available via `--extern` and `wrapper` does not re-export it as `wrapper::target`.
    pub aliased_externs: BTreeMap<String, String>,
    /// Normalized crate names from `Cargo.lock` that are not direct `--extern`s of the target
    /// crate and not top-level items/modules declared in the target crate's root.
    pub transitive_crates: BTreeSet<String>,
}

impl NormalizeContext {
    pub fn from_workspace(workspace: Option<&Path>, crate_name: Option<&str>, externs: &BTreeSet<String>) -> Self {
        let mut ctx = Self {
            externs: externs.clone(),
            aliased_externs: BTreeMap::new(),
            transitive_crates: BTreeSet::new(),
        };
        let Some(ws) = workspace else {
            return ctx;
        };
        let ws_crates = discover_workspace_crate_roots(ws);
        for (ext_ident, root_file) in &ws_crates {
            if !externs.contains(ext_ident) {
                continue;
            }
            if let Ok(source) = std::fs::read_to_string(root_file) {
                for (target, _) in collect_aliased_extern_reexports(&source, externs) {
                    ctx.aliased_externs
                        .insert(format!("{ext_ident}::{target}"), target);
                }
            }
        }
        if let Some(local_crate) = crate_name {
            let mut transitive = read_cargo_lock_crates(ws);
            transitive.remove("std");
            transitive.remove("core");
            transitive.remove("alloc");
            transitive.remove(local_crate);
            for ext in externs {
                transitive.remove(ext);
            }
            if let Some(root_file) = ws_crates.get(local_crate)
                && let Ok(source) = std::fs::read_to_string(root_file)
            {
                for ident in collect_top_level_decls(&source) {
                    transitive.remove(&ident);
                }
            }
            ctx.transitive_crates = transitive;
        }
        ctx
    }
}

pub fn normalize_type(raw: &str, local_crate: &str, ctx: &NormalizeContext) -> Option<String> {
    if UNWRITABLE.iter().any(|needle| raw.contains(needle)) || mentions_never(raw) {
        return None;
    }
    // The default allocator argument (`Vec<T, std::alloc::Global>`) can only be written with the
    // unstable `allocator_api`; dropping it gives the same type.
    let raw = raw
        .replace(", std::alloc::Global>", ">")
        .replace(", alloc::alloc::Global>", ">");
    let raw = replace_regions(&strip_defid_hashes(&raw));
    let raw = rewrite_aliased_externs(&raw, &ctx.aliased_externs);
    prefix_local_paths(&raw, local_crate, &ctx.externs, &ctx.transitive_crates)
}

pub fn normalize_callee(
    raw: &str,
    receiver_ty: Option<&str>,
    kind: CallKind,
    local_crate: &str,
    ctx: &NormalizeContext,
) -> String {
    let raw = rewrite_aliased_externs(&strip_defid_hashes(raw), &ctx.aliased_externs);
    let method = raw.rsplit("::").next().unwrap_or(&raw);
    if kind == CallKind::Path && !raw.contains("{impl#") {
        return canonical_path(&raw, local_crate, ctx);
    }
    // A path call to an inherent method (`Type::m(recv, ..)`, the fully qualified shim body) names
    // `{impl#k}`; its owner is taken from the first argument's type, as for method calls. This is
    // wrong for associated fns without a receiver (`Vec3::new(..)`): Phase 2 (path call sites)
    // must take the owner from the written path instead.
    if raw.contains("{impl#") {
        let recv = replace_regions(&strip_defid_hashes(receiver_ty.unwrap_or("Self")));
        let owner = strip_owner_generics(strip_references(&recv));
        let owner = rewrite_aliased_externs(&owner, &ctx.aliased_externs);
        format!("{}::{method}", canonical_path(&owner, local_crate, ctx))
    } else {
        let trait_path = raw.rsplit_once("::").map_or(raw.as_str(), |(path, _)| path);
        format!("<Self as {}>::{method}", canonical_path(trait_path, local_crate, ctx))
    }
}

pub fn resolution(receiver_ty: Option<&str>) -> Resolution {
    let owner = strip_references(receiver_ty.unwrap_or(""));
    if owner.contains("dyn ") {
        Resolution::Dyn
    } else if owner.contains("Param(")
        || (owner.len() == 1 && owner.as_bytes()[0].is_ascii_uppercase())
    {
        Resolution::Generic
    } else {
        Resolution::Concrete
    }
}

fn strip_references(mut ty: &str) -> &str {
    loop {
        ty = ty.trim();
        let Some(rest) = ty.strip_prefix('&') else { return ty };
        ty = rest.trim_start();
        if ty.starts_with('\'') {
            let end = ty.find(char::is_whitespace).unwrap_or(ty.len());
            ty = ty[end..].trim_start();
        }
        ty = ty.strip_prefix("mut ").unwrap_or(ty).trim_start();
    }
}

fn strip_owner_generics(ty: &str) -> String {
    let mut depth = 0_usize;
    for (index, character) in ty.char_indices() {
        match character {
            '<' if depth == 0 => return ty[..index].trim().to_owned(),
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    ty.trim().to_owned()
}

fn canonical_path(path: &str, local_crate: &str, ctx: &NormalizeContext) -> String {
    let first = path.split("::").next().unwrap_or(path);
    if first == "crate" {
        return format!("{local_crate}{}", &path[5..]);
    }
    if first == local_crate
        || is_external(first, &ctx.externs)
        || is_keyword_or_primitive(first)
        || ctx.transitive_crates.contains(first)
        || path.starts_with(['<', '[', '(', '*'])
    {
        path.to_owned()
    } else {
        format!("{local_crate}::{path}")
    }
}

fn replace_regions(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("'^") {
        result.push_str(&rest[..index]);
        let tail = &rest[index..];
        if let Some(named) = tail.find(".Named(") {
            let mut depth = 0;
            let mut end = named + 7;
            for (offset, character) in tail[named + 6..].char_indices() {
                match character {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            end = named + 6 + offset + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            result.push_str("'_");
            rest = &tail[end..];
        } else {
            result.push_str("'_");
            rest = &tail[2..];
        }
    }
    result.push_str(rest);
    result.replace("'{erased}", "'_")
}

fn rewrite_aliased_externs(text: &str, aliased_externs: &BTreeMap<String, String>) -> String {
    if aliased_externs.is_empty() {
        return text.to_owned();
    }
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    let mut path_start = true;
    while index < text.len() {
        let character = text[index..].chars().next().unwrap();
        if character == '\'' {
            let end = text[index + 1..]
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .map_or(text.len(), |offset| index + 1 + offset);
            output.push_str(&text[index..end]);
            index = end;
            continue;
        }
        if character.is_alphabetic() || character == '_' {
            if path_start
                && let Some((from, to)) = aliased_externs.iter().find(|(from, _)| {
                    let rest = &text[index..];
                    rest.starts_with(from.as_str())
                        && rest[from.len()..]
                            .chars()
                            .next()
                            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
                })
            {
                output.push_str(to);
                index += from.len();
                path_start = false;
                continue;
            }
            let end = text[index..]
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .map_or(text.len(), |offset| index + offset);
            let word = &text[index..end];
            output.push_str(word);
            path_start = matches!(word, "mut" | "fn" | "for" | "dyn" | "impl" | "const");
            index = end;
            continue;
        }
        output.push(character);
        index += character.len_utf8();
        if !character.is_whitespace() {
            path_start = matches!(character, '&' | '<' | '(' | '[' | ',' | ';' | '=' | '|' | '+');
        }
    }
    output
}

fn prefix_local_paths(
    text: &str,
    local_crate: &str,
    externs: &BTreeSet<String>,
    transitive_crates: &BTreeSet<String>,
) -> Option<String> {
    let mut output = String::with_capacity(text.len() + 8);
    let mut index = 0;
    let mut path_start = true;
    while index < text.len() {
        let character = text[index..].chars().next().unwrap();
        if character == '\'' {
            let end = text[index + 1..]
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .map_or(text.len(), |offset| index + 1 + offset);
            output.push_str(&text[index..end]);
            index = end;
            continue;
        }
        if character.is_alphabetic() || character == '_' {
            let end = text[index..]
                .find(|c: char| !c.is_alphanumeric() && c != '_')
                .map_or(text.len(), |offset| index + offset);
            let word = &text[index..end];
            if path_start && word == local_crate {
                output.push_str("crate");
            } else if path_start && transitive_crates.contains(word) {
                return None;
            } else if path_start && !is_external(word, externs) && !is_keyword_or_primitive(word) {
                output.push_str("crate::");
                output.push_str(word);
            } else {
                output.push_str(word);
            }
            path_start = matches!(word, "mut" | "fn" | "for" | "dyn" | "impl" | "const");
            index = end;
            continue;
        }
        output.push(character);
        index += character.len_utf8();
        // Whitespace keeps the state: in `&'_ Stack` or `dyn Trait` the path starts after the space.
        if !character.is_whitespace() {
            path_start = matches!(character, '&' | '<' | '(' | '[' | ',' | ';' | '=' | '|' | '+');
        }
    }
    Some(output)
}

fn discover_workspace_crate_roots(ws: &Path) -> BTreeMap<String, PathBuf> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(ws.join("crates")) else {
        return out;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        let mut section = String::new();
        let mut pkg_name: Option<String> = None;
        let mut lib_path: Option<String> = None;
        for raw in manifest.lines() {
            let line = raw.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                section = line.to_owned();
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                let key = k.trim();
                let val = v.trim().trim_matches('"');
                if section == "[package]" && key == "name" && !val.is_empty() {
                    pkg_name = Some(val.to_owned());
                } else if section == "[lib]" && key == "path" && !val.is_empty() {
                    lib_path = Some(val.to_owned());
                }
            }
        }
        let Some(pkg) = pkg_name else {
            continue;
        };
        let ident = pkg.replace('-', "_");
        let root = if let Some(rel) = lib_path {
            dir.join(rel)
        } else if dir.join("src/lib.rs").is_file() {
            dir.join("src/lib.rs")
        } else {
            dir.join("src/main.rs")
        };
        if root.is_file() {
            out.insert(ident, root);
        }
    }
    out
}

fn read_cargo_lock_crates(ws: &Path) -> BTreeSet<String> {
    let mut crates = BTreeSet::new();
    let Ok(text) = std::fs::read_to_string(ws.join("Cargo.lock")) else {
        return crates;
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("name = ") {
            let name = rest.trim().trim_matches('"');
            if !name.is_empty() {
                crates.insert(name.replace('-', "_"));
            }
        }
    }
    crates
}

fn collect_aliased_extern_reexports(source: &str, externs: &BTreeSet<String>) -> Vec<(String, String)> {
    let mut direct_public = BTreeSet::new();
    let mut aliased = Vec::new();
    for raw in source.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if let Some(rest) = line.strip_prefix("pub mod ") {
            let ident: String = rest
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !ident.is_empty() {
                direct_public.insert(ident);
            }
            continue;
        }
        let Some(rest) = line.strip_prefix("pub use ") else {
            continue;
        };
        let stmt = rest.trim_end_matches(';').trim();
        if let Some((target, alias)) = stmt.split_once(" as ") {
            let target = target.trim();
            let alias = alias.trim();
            if !target.contains("::")
                && target != alias
                && externs.contains(target)
                && !alias.is_empty()
            {
                aliased.push((target.to_owned(), alias.to_owned()));
            }
        } else if !stmt.contains("::") && !stmt.contains('{') && externs.contains(stmt) {
            direct_public.insert(stmt.to_owned());
        }
    }
    aliased
        .into_iter()
        .filter(|(target, _)| !direct_public.contains(target))
        .collect()
}

fn collect_top_level_decls(source: &str) -> BTreeSet<String> {
    let mut decls = BTreeSet::new();
    let mut brace_depth = 0usize;
    for raw in source.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        let opens = line.bytes().filter(|&b| b == b'{').count();
        let closes = line.bytes().filter(|&b| b == b'}').count();
        if brace_depth == 0 {
            let words: Vec<&str> = line
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .filter(|w| !w.is_empty())
                .collect();
            for pair in words.windows(2) {
                if matches!(
                    pair[0],
                    "mod" | "struct" | "enum" | "union" | "trait" | "type" | "fn" | "const" | "static"
                ) {
                    decls.insert(pair[1].to_owned());
                    break;
                }
            }
        }
        brace_depth = (brace_depth + opens).saturating_sub(closes);
    }
    decls
}

fn is_external(first: &str, externs: &BTreeSet<String>) -> bool {
    matches!(first, "std" | "core" | "alloc") || externs.contains(first)
}

fn is_keyword_or_primitive(word: &str) -> bool {
    matches!(
        word,
        "bool" | "char" | "str" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize"
            | "i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "f32" | "f64"
            | "Self" | "self" | "super" | "crate" | "dyn" | "impl" | "fn" | "for"
            | "mut" | "ref" | "where" | "const" | "unsafe" | "extern" | "move" | "type"
    )
}

/// The never type `!` as a type (for example `Result<T, E<!>>`) is unstable to write.
fn mentions_never(ty: &str) -> bool {
    let bytes = ty.as_bytes();
    bytes.iter().enumerate().any(|(index, &byte)| {
        byte == b'!'
            && bytes.get(index + 1) != Some(&b'=')
            && index
                .checked_sub(1)
                .and_then(|before| ty[..=before].trim_end().chars().last())
                .is_none_or(|c| matches!(c, '<' | ',' | '(' | '>' | '&' | ' '))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_aliased_extern_prefixes_at_path_roots() {
        let mut ctx = NormalizeContext::default();
        ctx.externs.insert("horizon_client_lib".into());
        ctx.externs.insert("horizon_command".into());
        ctx.aliased_externs
            .insert("horizon_client_lib::horizon_command".into(), "horizon_command".into());

        let ty = normalize_type(
            "&'_ horizon_client_lib::horizon_command::args::GameMode",
            "horizon_client",
            &ctx,
        );
        assert_eq!(ty.as_deref(), Some("&'_ horizon_command::args::GameMode"));

        let res = normalize_type(
            "std::result::Result<(), horizon_client_lib::horizon_command::CommandError>",
            "horizon_client",
            &ctx,
        );
        assert_eq!(
            res.as_deref(),
            Some("std::result::Result<(), horizon_command::CommandError>")
        );

        let callee = normalize_callee(
            "horizon_client_lib::horizon_command::Reader::{impl#1}::can_read",
            Some("&'^0_0.Named(DefId(0:1), \"'r\") horizon_client_lib::horizon_command::Reader<'{erased}>"),
            CallKind::Method,
            "horizon_client",
            &ctx,
        );
        assert_eq!(callee, "horizon_command::Reader::can_read");
    }

    #[test]
    fn rejects_transitive_crate_types_and_keeps_raw_pointer_callees_unprefixed() {
        let mut ctx = NormalizeContext::default();
        ctx.externs.insert("yoake_wasm".into());
        ctx.externs.insert("horizon_mod_abi".into());
        ctx.transitive_crates.insert("wasmtime".into());
        ctx.transitive_crates.insert("digest".into());
        ctx.transitive_crates.insert("horizon_mobmodel".into());

        assert_eq!(
            normalize_type(
                "&'_ mut wasmtime::runtime::store::Store<yoake_wasm::HostState>",
                "horizon_client",
                &ctx
            ),
            None
        );
        assert_eq!(
            normalize_callee("digest::digest::Digest::new", None, CallKind::Path, "horizon_client", &ctx),
            "digest::digest::Digest::new"
        );
        assert_eq!(
            normalize_callee(
                "core::ptr::const_ptr::{impl#0}::is_null",
                Some("*const horizon_mod_abi::ModDescriptor"),
                CallKind::Method,
                "horizon_client",
                &ctx
            ),
            "*const horizon_mod_abi::ModDescriptor::is_null"
        );
    }

    #[test]
    fn collect_aliased_extern_reexports_ignores_direct_reexports() {
        let externs: BTreeSet<String> = ["horizon_command", "sha2", "digest"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let src = "pub use horizon_command as command;\npub use digest;\npub use digest as dig;\n";
        let aliased = collect_aliased_extern_reexports(src, &externs);
        assert_eq!(aliased, vec![("horizon_command".to_owned(), "command".to_owned())]);
    }
}
