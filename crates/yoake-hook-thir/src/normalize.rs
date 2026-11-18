use std::collections::BTreeSet;

use crate::model::{CallKind, Resolution};
use crate::span::strip_defid_hashes;

const UNWRITABLE: &[&str] = &[
    "{closure", "Closure(", "{async", "Async(", "{coroutine", "Coroutine(", "impl ",
    "Alias(", "Opaque", "FnDef", "{type error}", "Param(",
];

pub fn normalize_type(raw: &str, local_crate: &str, externs: &BTreeSet<String>) -> Option<String> {
    if UNWRITABLE.iter().any(|needle| raw.contains(needle)) {
        return None;
    }
    let raw = replace_regions(&strip_defid_hashes(raw));
    Some(prefix_local_paths(&raw, local_crate, externs))
}

pub fn normalize_callee(
    raw: &str,
    receiver_ty: Option<&str>,
    kind: CallKind,
    local_crate: &str,
    externs: &BTreeSet<String>,
) -> String {
    let raw = strip_defid_hashes(raw);
    let method = raw.rsplit("::").next().unwrap_or(&raw);
    if kind == CallKind::Path {
        return canonical_path(&raw, local_crate, externs);
    }
    if raw.contains("{impl#") {
        let owner = strip_owner_generics(strip_references(receiver_ty.unwrap_or("Self")));
        format!("{}::{method}", canonical_path(&owner, local_crate, externs))
    } else {
        let trait_path = raw.rsplit_once("::").map_or(raw.as_str(), |(path, _)| path);
        format!("<Self as {}>::{method}", canonical_path(trait_path, local_crate, externs))
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

fn canonical_path(path: &str, local_crate: &str, externs: &BTreeSet<String>) -> String {
    let first = path.split("::").next().unwrap_or(path);
    if first == "crate" {
        return format!("{local_crate}{}", &path[5..]);
    }
    if first == local_crate
        || is_external(first, externs)
        || is_keyword_or_primitive(first)
        || path.starts_with(['<', '[', '('])
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

fn prefix_local_paths(text: &str, local_crate: &str, externs: &BTreeSet<String>) -> String {
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
    output
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
