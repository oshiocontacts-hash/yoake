use proc_macro2::{LineColumn, Span, TokenStream};
use syn::spanned::Spanned;
use syn::{Attribute, Expr, FnArg, ImplItemFn, ItemImpl, Pat, Signature, Stmt, Visibility};

use crate::attr::{
    self, ExpectedHookAttr, FileStyle, format_hook_attr,
};

#[derive(Clone, Debug)]
pub struct TextEdit {
    pub start: usize,
    pub end: usize,
    pub replacement: String,
}

pub struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    pub fn new(source: &str) -> Self {
        let mut line_starts = vec![0];
        for (i, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(i + 1);
            }
        }
        Self { line_starts }
    }

    pub fn offset(&self, source: &str, lc: LineColumn) -> usize {
        let line_idx = lc.line.saturating_sub(1);
        let line_start = self
            .line_starts
            .get(line_idx)
            .copied()
            .unwrap_or(source.len());
        let line_end = self
            .line_starts
            .get(line_idx + 1)
            .copied()
            .unwrap_or(source.len());
        let line_slice = &source[line_start..line_end];
        if lc.column == 0 {
            return line_start;
        }
        match line_slice.char_indices().nth(lc.column) {
            Some((rel, _)) => line_start + rel,
            None => line_end,
        }
    }

    pub fn span_range(&self, source: &str, span: Span) -> (usize, usize) {
        (
            self.offset(source, span.start()),
            self.offset(source, span.end()),
        )
    }

    pub fn slice_token_stream(&self, source: &str, tokens: &TokenStream) -> Option<String> {
        let mut iter = tokens.clone().into_iter();
        let first = iter.next()?;
        let last = iter.last().unwrap_or_else(|| first.clone());
        let start = self.offset(source, first.span().start());
        let end = self.offset(source, last.span().end());
        source.get(start..end).map(str::to_string)
    }

    pub fn line_start_of(&self, byte_offset: usize) -> usize {
        let idx = self
            .line_starts
            .partition_point(|&start| start <= byte_offset)
            .saturating_sub(1);
        self.line_starts[idx]
    }

    pub fn line_indent_at<'a>(&self, source: &'a str, byte_offset: usize) -> &'a str {
        let ls = self.line_start_of(byte_offset);
        let rest = &source[ls..];
        let len = rest
            .bytes()
            .take_while(|&b| b == b' ' || b == b'\t')
            .count();
        &rest[..len]
    }
}

pub fn line_ending(source: &str) -> &'static str {
    if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

pub fn apply_edits(source: &str, mut edits: Vec<TextEdit>) -> Result<String, String> {
    edits.sort_by(|a, b| b.start.cmp(&a.start).then_with(|| b.end.cmp(&a.end)));
    let mut prev_start = source.len();
    for edit in &edits {
        if edit.start > edit.end || edit.end > source.len() {
            return Err(format!(
                "invalid edit range {}..{} for len {}",
                edit.start,
                edit.end,
                source.len()
            ));
        }
        if edit.end > prev_start {
            return Err(format!(
                "overlapping edits: {}..{} overlaps with start {}",
                edit.start, edit.end, prev_start
            ));
        }
        prev_start = edit.start;
    }

    let mut out = source.to_string();
    for edit in edits {
        out.replace_range(edit.start..edit.end, &edit.replacement);
    }
    Ok(out)
}

pub fn fn_insert_anchor(
    source: &str,
    idx: &LineIndex,
    attrs: &[Attribute],
    vis: &Visibility,
    sig: &Signature,
) -> usize {
    for attr in attrs {
        if !attr.path().is_ident("doc") && !attr.path().is_ident("cfg") {
            return idx.span_range(source, attr.span()).0;
        }
    }
    if !matches!(vis, Visibility::Inherited) {
        idx.span_range(source, vis.span()).0
    } else {
        idx.span_range(source, sig.span()).0
    }
}

pub fn plan_ensure_attr(
    source: &str,
    idx: &LineIndex,
    attrs: &[Attribute],
    insert_anchor: usize,
    expected: &ExpectedHookAttr<'_>,
    style: &FileStyle,
    edits: &mut Vec<TextEdit>,
) {
    let found = attr::find_hook_attrs(attrs, source, idx);
    if let Some(first) = found.first() {
        if !attr::matches_expected(first, expected) {
            let (start, end) = idx.span_range(source, attrs[first.attr_index].span());
            let replacement = format_hook_attr(
                expected,
                first.wrapper_cfg.as_deref(),
                Some(&first.attr_path),
                style,
            );
            edits.push(TextEdit {
                start,
                end,
                replacement,
            });
        }
        for extra in found.iter().skip(1) {
            plan_remove_attr(source, idx, &attrs[extra.attr_index], edits);
        }
    } else {
        let attr_str = format_hook_attr(expected, None, None, style);
        let line_start = idx.line_start_of(insert_anchor);
        let nl = line_ending(source);
        if source[line_start..insert_anchor]
            .bytes()
            .all(|b| b == b' ' || b == b'\t')
        {
            let indent = &source[line_start..insert_anchor];
            edits.push(TextEdit {
                start: line_start,
                end: line_start,
                replacement: format!("{indent}{attr_str}{nl}"),
            });
        } else {
            edits.push(TextEdit {
                start: insert_anchor,
                end: insert_anchor,
                replacement: format!("{attr_str} "),
            });
        }
    }
}

pub fn plan_remove_hook_attrs(
    source: &str,
    idx: &LineIndex,
    attrs: &[Attribute],
    edits: &mut Vec<TextEdit>,
) {
    for found in attr::find_hook_attrs(attrs, source, idx) {
        if found.parsed.skip.is_none() {
            plan_remove_attr(source, idx, &attrs[found.attr_index], edits);
        }
    }
}

pub fn plan_remove_attr(
    source: &str,
    idx: &LineIndex,
    attr: &Attribute,
    edits: &mut Vec<TextEdit>,
) {
    let (start, end) = idx.span_range(source, attr.span());
    let line_start = idx.line_start_of(start);
    let leading_ok = source[line_start..start]
        .bytes()
        .all(|b| b == b' ' || b == b'\t');
    let after = &source[end..];
    let trailing_ws = after
        .bytes()
        .take_while(|&b| b == b' ' || b == b'\t')
        .count();
    let after_ws = &after[trailing_ws..];
    let nl_len = if after_ws.starts_with("\r\n") {
        Some(2)
    } else if after_ws.starts_with('\n') {
        Some(1)
    } else {
        None
    };

    if leading_ok && let Some(nl) = nl_len {
        edits.push(TextEdit {
            start: line_start,
            end: end + trailing_ws + nl,
            replacement: String::new(),
        });
    } else {
        let extra_space = usize::from(after.starts_with(' '));
        edits.push(TextEdit {
            start,
            end: end + extra_space,
            replacement: String::new(),
        });
    }
}

pub fn is_already_converted_trait_method(method: &ImplItemFn, expected_helper: &str) -> bool {
    if method.block.stmts.len() != 1 {
        return false;
    }
    let Stmt::Expr(expr, _) = &method.block.stmts[0] else {
        return false;
    };
    let has_receiver = matches!(method.sig.inputs.first(), Some(FnArg::Receiver(_)));
    let mut param_names = Vec::new();
    for arg in &method.sig.inputs {
        if let FnArg::Typed(pt) = arg {
            let Pat::Ident(pat) = &*pt.pat else {
                return false;
            };
            param_names.push(pat.ident.clone());
        }
    }

    let args_match = |args: &syn::punctuated::Punctuated<Expr, syn::Token![,]>| {
        if args.len() != param_names.len() {
            return false;
        }
        args.iter().zip(&param_names).all(|(expr, ident)| {
            matches!(expr, Expr::Path(ep) if ep.qself.is_none() && ep.path.is_ident(ident))
        })
    };

    if has_receiver {
        let Expr::MethodCall(mc) = expr else {
            return false;
        };
        matches!(&*mc.receiver, Expr::Path(ep) if ep.qself.is_none() && ep.path.is_ident("self"))
            && mc.turbofish.is_none()
            && mc.method == expected_helper
            && args_match(&mc.args)
    } else {
        let Expr::Call(call) = expr else {
            return false;
        };
        let Expr::Path(ep) = &*call.func else {
            return false;
        };
        ep.qself.is_none()
            && ep.path.leading_colon.is_none()
            && ep.path.segments.len() == 2
            && ep.path.segments[0].ident == "Self"
            && ep.path.segments[1].ident == expected_helper
            && ep.path.segments[1].arguments.is_empty()
            && args_match(&call.args)
    }
}

pub struct TraitMethodConversion {
    pub method: ImplItemFn,
    pub helper_name: String,
    pub id: u32,
    pub key: String,
    pub self_ty_src: Option<String>,
    pub self_ty_norm: Option<String>,
}

fn format_single_line_block(
    raw_body: &str,
    nl: &str,
    body_indent: &str,
    method_indent: &str,
) -> String {
    if !raw_body.contains('\n') && raw_body.starts_with('{') && raw_body.ends_with('}') {
        let inner = raw_body[1..raw_body.len() - 1].trim();
        if inner.is_empty() {
            return "{}".to_string();
        }
        if !inner.contains("//") {
            return format!("{{{nl}{body_indent}{inner}{nl}{method_indent}}}");
        }
    }
    raw_body.to_string()
}

pub fn plan_trait_impl_conversions(
    source: &str,
    idx: &LineIndex,
    trait_impl: &ItemImpl,
    existing_helper_close_brace: Option<usize>,
    conversions: &[TraitMethodConversion],
    style: &FileStyle,
    edits: &mut Vec<TextEdit>,
) {
    if conversions.is_empty() {
        return;
    }

    let nl = line_ending(source);
    let impl_start = idx.span_range(source, trait_impl.span()).0;
    let impl_indent = idx.line_indent_at(source, impl_start);
    let indent_unit = if impl_indent.contains('\t') { "\t" } else { "    " };
    let method_indent = format!("{impl_indent}{indent_unit}");
    let body_indent = format!("{method_indent}{indent_unit}");

    let mut helper_methods = Vec::new();

    for conv in conversions {
        let method = &conv.method;
        plan_remove_hook_attrs(source, idx, &method.attrs, edits);

        let mut has_receiver = false;
        let mut arg_names = Vec::new();
        for arg in &method.sig.inputs {
            match arg {
                FnArg::Receiver(_) => has_receiver = true,
                FnArg::Typed(pt) => {
                    if let Pat::Ident(pat) = &*pt.pat {
                        if let Some(mut_tok) = pat.mutability {
                            let mut_start = idx.span_range(source, mut_tok.span()).0;
                            let ident_start = idx.span_range(source, pat.ident.span()).0;
                            edits.push(TextEdit {
                                start: mut_start,
                                end: ident_start,
                                replacement: String::new(),
                            });
                        }
                        arg_names.push(pat.ident.to_string());
                    }
                }
            }
        }

        let call = if has_receiver {
            format!("self.{}({})", conv.helper_name, arg_names.join(", "))
        } else {
            format!("Self::{}({})", conv.helper_name, arg_names.join(", "))
        };
        let (sig_start, sig_end) = idx.span_range(source, method.sig.span());
        let sig_line_start = idx.line_start_of(sig_start);
        let delegating_method_indent = if source[sig_line_start..sig_start]
            .bytes()
            .all(|b| b == b' ' || b == b'\t')
        {
            &source[sig_line_start..sig_start]
        } else {
            method_indent.as_str()
        };
        let delegating_body_indent = format!("{delegating_method_indent}{indent_unit}");

        let (block_start, block_end) = idx.span_range(source, method.block.span());
        edits.push(TextEdit {
            start: block_start,
            end: block_end,
            replacement: format!(
                "{{{nl}{delegating_body_indent}{call}{nl}{delegating_method_indent}}}"
            ),
        });

        let expected = ExpectedHookAttr {
            method: true,
            self_ty_src: conv.self_ty_src.as_deref(),
            self_ty_norm: conv.self_ty_norm.as_deref(),
            id: conv.id,
            key: &conv.key,
        };
        let hook_attr = format_hook_attr(&expected, None, None, style);

        let mut helper_lines = Vec::new();
        helper_lines.push(format!("{method_indent}{hook_attr}"));
        let mut has_inline_always = false;
        for attr in &method.attrs {
            if attr.path().is_ident("cfg")
                || attr.path().is_ident("allow")
                || attr.path().is_ident("expect")
                || attr.path().is_ident("warn")
                || attr.path().is_ident("deny")
            {
                let (as_, ae) = idx.span_range(source, attr.span());
                helper_lines.push(format!("{method_indent}{}", &source[as_..ae]));
            } else if attr.path().is_ident("inline")
                && let syn::Meta::List(list) = &attr.meta
                && list.tokens.to_string().trim() == "always"
            {
                has_inline_always = true;
            }
        }
        if !has_inline_always {
            helper_lines.push(format!("{method_indent}#[inline(always)]"));
        }

        let (ident_start, ident_end) = idx.span_range(source, method.sig.ident.span());
        let helper_sig = format!(
            "{}{}{}",
            &source[sig_start..ident_start],
            conv.helper_name,
            &source[ident_end..sig_end]
        );
        let helper_body =
            format_single_line_block(&source[block_start..block_end], nl, &body_indent, &method_indent);
        helper_lines.push(format!("{method_indent}{helper_sig} {helper_body}"));
        helper_methods.push(helper_lines.join(nl));
    }

    let joined_helpers = helper_methods.join(nl);
    if let Some(close_brace_start) = existing_helper_close_brace {
        let line_start = idx.line_start_of(close_brace_start);
        if source[line_start..close_brace_start]
            .bytes()
            .all(|b| b == b' ' || b == b'\t')
        {
            edits.push(TextEdit {
                start: line_start,
                end: line_start,
                replacement: format!("{joined_helpers}{nl}"),
            });
        } else {
            edits.push(TextEdit {
                start: close_brace_start,
                end: close_brace_start,
                replacement: format!("{nl}{joined_helpers}{nl}{impl_indent}"),
            });
        }
    } else {
        let mut impl_lines = Vec::new();
        for attr in &trait_impl.attrs {
            if attr.path().is_ident("cfg") || attr.path().is_ident("allow") {
                let (as_, ae) = idx.span_range(source, attr.span());
                impl_lines.push(format!("{impl_indent}{}", &source[as_..ae]));
            }
        }
        let generics_slice = match (
            trait_impl.generics.lt_token,
            trait_impl.generics.gt_token,
        ) {
            (Some(lt), Some(gt)) if !trait_impl.generics.params.is_empty() => {
                let gs = idx.span_range(source, lt.span()).0;
                let ge = idx.span_range(source, gt.span()).1;
                &source[gs..ge]
            }
            _ => "",
        };
        let (sty_s, sty_e) = idx.span_range(source, trait_impl.self_ty.span());
        let self_ty_slice = &source[sty_s..sty_e];
        let where_slice = trait_impl
            .generics
            .where_clause
            .as_ref()
            .map(|wc| {
                let (ws, we) = idx.span_range(source, wc.span());
                format!(" {}", &source[ws..we])
            })
            .unwrap_or_default();
        impl_lines.push(format!(
            "{impl_indent}impl{generics_slice} {self_ty_slice}{where_slice} {{"
        ));
        impl_lines.push(joined_helpers);
        impl_lines.push(format!("{impl_indent}}}"));

        let trait_impl_end = idx.span_range(source, trait_impl.span()).1;
        edits.push(TextEdit {
            start: trait_impl_end,
            end: trait_impl_end,
            replacement: format!("{nl}{}", impl_lines.join(nl)),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_index_handles_utf8_and_crlf() {
        let src = "// 日本語コメント\r\nfn foo(x: u32) -> u32 {\r\n    x + 1\r\n}\r\n";
        let parsed = syn::parse_file(src).unwrap();
        let idx = LineIndex::new(src);
        let syn::Item::Fn(item_fn) = &parsed.items[0] else {
            panic!("expected fn");
        };
        let (s, e) = idx.span_range(src, item_fn.sig.ident.span());
        assert_eq!(&src[s..e], "foo");
    }
}
