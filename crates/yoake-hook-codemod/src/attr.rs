use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream, TokenTree};
use quote::quote;
use syn::parse::ParseStream;
use syn::{Attribute, Item, LitInt, LitStr, Meta, Path, Token, Type, UseTree};

use crate::edit::LineIndex;

#[derive(Clone, Debug)]
pub struct FileStyle {
    pub imports_yoake_hook: bool,
    pub uses_baseline_cfg_attr: bool,
    pub imported_names: BTreeSet<String>,
    pub has_glob_import: bool,
}

#[derive(Clone, Debug, Default)]
pub struct HookAttrArgs {
    pub method: bool,
    pub self_ty: Option<String>,
    pub id: Option<u32>,
    pub key: Option<String>,
    pub skip: Option<String>,
    pub test_id: Option<u32>,
    pub valid_syntax: bool,
}

#[derive(Clone, Debug)]
pub struct FoundHookAttr {
    pub attr_index: usize,
    pub wrapper_cfg: Option<String>,
    pub attr_path: String,
    pub parsed: HookAttrArgs,
}

pub struct ExpectedHookAttr<'a> {
    pub method: bool,
    pub self_ty_src: Option<&'a str>,
    pub self_ty_norm: Option<&'a str>,
    pub id: u32,
    pub key: &'a str,
}

pub fn detect_file_style(items: &[Item], source: &str) -> FileStyle {
    let mut style = FileStyle {
        imports_yoake_hook: false,
        uses_baseline_cfg_attr: source.contains("yoake-hook-baseline"),
        imported_names: BTreeSet::new(),
        has_glob_import: false,
    };

    for item in items {
        if let Item::Use(item_use) = item {
            collect_use_tree(&item_use.tree, &mut style);
        }
    }
    style
}

pub fn merge_module_style(base: &FileStyle, items: &[Item]) -> FileStyle {
    let mut merged = base.clone();
    for item in items {
        if let Item::Use(item_use) = item {
            collect_use_tree(&item_use.tree, &mut merged);
        }
    }
    merged
}

fn collect_use_tree(tree: &UseTree, style: &mut FileStyle) {
    match tree {
        UseTree::Path(path) => {
            if path.ident == "yoake_hook_macro"
                && matches!(&*path.tree, UseTree::Name(n) if n.ident == "yoake_hook")
            {
                style.imports_yoake_hook = true;
            }
            if path.ident == "yoake_hook_macro" && matches!(&*path.tree, UseTree::Glob(_)) {
                style.imports_yoake_hook = true;
            }
            collect_use_tree(&path.tree, style);
        }
        UseTree::Name(name) => {
            let ident = name.ident.to_string();
            if ident == "yoake_hook" {
                style.imports_yoake_hook = true;
            }
            style.imported_names.insert(ident);
        }
        UseTree::Rename(rename) => {
            let ident = rename.rename.to_string();
            if ident == "yoake_hook" {
                style.imports_yoake_hook = true;
            }
            style.imported_names.insert(ident);
        }
        UseTree::Glob(_) => {
            style.has_glob_import = true;
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_tree(item, style);
            }
        }
    }
}

pub fn trait_is_in_scope(trait_path: &Path, style: &FileStyle) -> bool {
    if trait_path.segments.len() == 1 && trait_path.leading_colon.is_none() {
        return true;
    }
    let Some(last) = trait_path.segments.last() else {
        return false;
    };
    style.has_glob_import || style.imported_names.contains(&last.ident.to_string())
}

pub fn find_hook_attrs(attrs: &[Attribute], source: &str, idx: &LineIndex) -> Vec<FoundHookAttr> {
    let mut found = Vec::new();
    for (i, attr) in attrs.iter().enumerate() {
        if let Some(item) = parse_single_hook_attr(i, attr, source, idx) {
            found.push(item);
        }
    }
    found
}

fn is_yoake_hook_path(path: &Path) -> bool {
    path.segments
        .last()
        .is_some_and(|seg| seg.ident == "yoake_hook")
}

fn path_to_compact_string(path: &Path) -> String {
    let mut out = String::new();
    if path.leading_colon.is_some() {
        out.push_str("::");
    }
    for (i, seg) in path.segments.iter().enumerate() {
        if i > 0 {
            out.push_str("::");
        }
        out.push_str(&seg.ident.to_string());
    }
    out
}

fn parse_single_hook_attr(
    attr_index: usize,
    attr: &Attribute,
    source: &str,
    idx: &LineIndex,
) -> Option<FoundHookAttr> {
    if is_yoake_hook_path(attr.path()) {
        let parsed = match &attr.meta {
            Meta::List(list) => parse_hook_args(list.tokens.clone()),
            Meta::Path(_) => HookAttrArgs::default(),
            Meta::NameValue(_) => HookAttrArgs::default(),
        };
        return Some(FoundHookAttr {
            attr_index,
            wrapper_cfg: None,
            attr_path: path_to_compact_string(attr.path()),
            parsed,
        });
    }

    if attr.path().is_ident("cfg_attr")
        && let Meta::List(list) = &attr.meta
        && let Some((cond_tokens, inner_tokens)) = split_first_comma(list.tokens.clone())
        && let Ok((inner_path, inner_args)) = parse_inner_attr(inner_tokens)
        && is_yoake_hook_path(&inner_path)
    {
        let cond_str = idx
            .slice_token_stream(source, &cond_tokens)
            .unwrap_or_else(|| quote!(#cond_tokens).to_string());
        return Some(FoundHookAttr {
            attr_index,
            wrapper_cfg: Some(cond_str),
            attr_path: path_to_compact_string(&inner_path),
            parsed: parse_hook_args(inner_args),
        });
    }

    None
}

fn split_first_comma(tokens: TokenStream) -> Option<(TokenStream, TokenStream)> {
    let mut cond = Vec::new();
    let mut rest = Vec::new();
    let mut seen_comma = false;
    for tt in tokens {
        if !seen_comma && matches!(&tt, TokenTree::Punct(p) if p.as_char() == ',') {
            seen_comma = true;
            continue;
        }
        if seen_comma {
            rest.push(tt);
        } else {
            cond.push(tt);
        }
    }
    if !seen_comma || cond.is_empty() || rest.is_empty() {
        return None;
    }
    Some((cond.into_iter().collect(), rest.into_iter().collect()))
}

fn parse_inner_attr(tokens: TokenStream) -> syn::Result<(Path, TokenStream)> {
    syn::parse::Parser::parse2(
        |input: ParseStream<'_>| {
            let path: Path = input.parse()?;
            let content;
            let _paren = syn::parenthesized!(content in input);
            let args: TokenStream = content.parse()?;
            if input.peek(Token![,]) {
                let _ = input.parse::<Token![,]>()?;
            }
            if !input.is_empty() {
                return Err(input.error("unexpected trailing tokens in cfg_attr"));
            }
            Ok((path, args))
        },
        tokens,
    )
}

fn parse_hook_args(tokens: TokenStream) -> HookAttrArgs {
    let mut args = HookAttrArgs {
        valid_syntax: true,
        ..HookAttrArgs::default()
    };

    let res = syn::parse::Parser::parse2(
        |input: ParseStream<'_>| {
            while !input.is_empty() {
                let ident: syn::Ident = input.parse()?;
                if ident == "method" {
                    args.method = true;
                } else if ident == "id" {
                    input.parse::<Token![=]>()?;
                    let lit: LitInt = input.parse()?;
                    args.id = Some(lit.base10_parse::<u32>()?);
                } else if ident == "test_id" {
                    input.parse::<Token![=]>()?;
                    let lit: LitInt = input.parse()?;
                    args.test_id = Some(lit.base10_parse::<u32>()?);
                } else if ident == "key" {
                    input.parse::<Token![=]>()?;
                    let lit: LitStr = input.parse()?;
                    args.key = Some(lit.value());
                } else if ident == "skip" {
                    input.parse::<Token![=]>()?;
                    let lit: LitStr = input.parse()?;
                    args.skip = Some(lit.value());
                } else if ident == "self_ty" {
                    input.parse::<Token![=]>()?;
                    let ty: Type = input.parse()?;
                    args.self_ty = Some(quote!(#ty).to_string());
                } else {
                    return Err(syn::Error::new(Span::call_site(), "unknown option"));
                }

                if !input.is_empty() {
                    input.parse::<Token![,]>()?;
                }
            }
            Ok(())
        },
        tokens,
    );

    if res.is_err() {
        args.valid_syntax = false;
    }
    args
}

pub fn matches_expected(found: &FoundHookAttr, expected: &ExpectedHookAttr<'_>) -> bool {
    let p = &found.parsed;
    p.valid_syntax
        && p.skip.is_none()
        && p.test_id.is_none()
        && p.method == expected.method
        && p.id == Some(expected.id)
        && p.key.as_deref() == Some(expected.key)
        && p.self_ty.as_deref() == expected.self_ty_norm
}

pub fn format_hook_attr(
    expected: &ExpectedHookAttr<'_>,
    wrapper_cfg: Option<&str>,
    attr_path: Option<&str>,
    style: &FileStyle,
) -> String {
    let mut parts = Vec::new();
    if expected.method {
        parts.push("method".to_string());
    }
    if let Some(self_ty_src) = expected.self_ty_src {
        parts.push(format!("self_ty = {self_ty_src}"));
    }
    parts.push(format!("id = {}", expected.id));
    let escaped_key = serde_json::to_string(expected.key).unwrap_or_else(|_| format!("\"{}\"", expected.key));
    parts.push(format!("key = {escaped_key}"));

    let path = attr_path.unwrap_or_else(|| {
        if style.imports_yoake_hook {
            "yoake_hook"
        } else {
            "yoake_hook_macro::yoake_hook"
        }
    });
    let inner = format!("{path}({})", parts.join(", "));

    if let Some(cfg) = wrapper_cfg {
        format!("#[cfg_attr({cfg}, {inner})]")
    } else if attr_path.is_none() && style.uses_baseline_cfg_attr {
        format!("#[cfg_attr(not(feature = \"yoake-hook-baseline\"), {inner})]")
    } else {
        format!("#[{inner}]")
    }
}

pub fn normalized_type_tokens(ty: &Type) -> String {
    quote!(#ty).to_string()
}
