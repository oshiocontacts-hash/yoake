use std::collections::BTreeMap;

use proc_macro2::{LineColumn, Span, TokenStream};
use quote::quote;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Block, Expr, ExprCall, ExprClosure, ExprMethodCall, ImplItemFn, ItemFn, TraitItemFn,
};

use crate::{CallerSnapshot, Site};

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

/// 1-based `[start_line, start_col, end_line, end_col]` (end_col exclusive), matching rustc spans.
pub type SpanRange = [usize; 4];

/// Extracted call-site metadata paired with source spans for joining against THIR.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedCall {
    pub site: Site,
    pub span: SpanRange,
    pub fun_span: SpanRange,
    pub is_method: bool,
}

/// Computes the 64-bit FNV-1a hash of the given byte slice.
pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn compact_tokens(tokens: TokenStream) -> String {
    tokens.to_string().chars().filter(|c| !c.is_whitespace()).collect()
}

/// Computes the FNV-1a 64-bit hash of whitespace-stripped tokens.
pub fn hash_tokens(tokens: TokenStream) -> u64 {
    fnv1a_64(compact_tokens(tokens).as_bytes())
}

fn span_tuple(pos: LineColumn) -> (usize, usize) {
    (pos.line, pos.column)
}

fn to_span_range(span: Span) -> SpanRange {
    let start = span.start();
    let end = span.end();
    [start.line, start.column + 1, end.line, end.column + 1]
}

fn callee_text(expr: &Expr) -> String {
    match expr {
        Expr::Path(ep) if ep.qself.is_none() => {
            let mut out = if ep.path.leading_colon.is_some() {
                String::from("::")
            } else {
                String::new()
            };
            for (idx, seg) in ep.path.segments.iter().enumerate() {
                if idx > 0 {
                    out.push_str("::");
                }
                out.push_str(&seg.ident.to_string());
            }
            out
        }
        other => compact_tokens(quote!(#other)),
    }
}

struct FindFn<'a> {
    target_name: &'a str,
    found: Option<&'a Block>,
}

impl<'ast> Visit<'ast> for FindFn<'ast> {
    fn visit_item_fn(&mut self, node: &'ast ItemFn) {
        if self.found.is_some() {
            return;
        }
        if node.sig.ident == self.target_name {
            self.found = Some(&node.block);
            return;
        }
        visit::visit_item_fn(self, node);
    }

    fn visit_impl_item_fn(&mut self, node: &'ast ImplItemFn) {
        if self.found.is_some() {
            return;
        }
        if node.sig.ident == self.target_name {
            self.found = Some(&node.block);
            return;
        }
        visit::visit_impl_item_fn(self, node);
    }

    fn visit_trait_item_fn(&mut self, node: &'ast TraitItemFn) {
        if self.found.is_some() {
            return;
        }
        if node.sig.ident == self.target_name
            && let Some(block) = &node.default
        {
            self.found = Some(block);
            return;
        }
        visit::visit_trait_item_fn(self, node);
    }
}

#[derive(Clone)]
struct StmtFrame {
    index: usize,
    hash: u64,
    context: (u64, u64),
}

struct RawSite {
    sort_key: ((usize, usize), (usize, usize), (usize, usize)),
    call: ExtractedCall,
}

struct BodyVisitor<'a> {
    caller: &'a str,
    stmt_stack: Vec<StmtFrame>,
    closure_depth: usize,
    raw_sites: Vec<RawSite>,
}

impl BodyVisitor<'_> {
    fn record_call(
        &mut self,
        callee: String,
        anchor: u64,
        callee_span: Span,
        expr_span: Span,
        is_method: bool,
    ) {
        let stmt_index = self.stmt_stack.first().map_or(0, |frame| frame.index);
        let stmt_hash = self.stmt_stack.first().map_or(0, |frame| frame.hash);
        let stmt_path = self.stmt_stack.iter().map(|frame| frame.index).collect();
        let stmt_path_hashes = self.stmt_stack.iter().map(|frame| frame.hash).collect();
        let context = self.stmt_stack.last().map_or((0, 0), |frame| frame.context);
        self.raw_sites.push(RawSite {
            sort_key: (
                span_tuple(callee_span.start()),
                span_tuple(expr_span.start()),
                span_tuple(expr_span.end()),
            ),
            call: ExtractedCall {
                site: Site {
                    caller: self.caller.to_owned(),
                    callee,
                    anchor,
                    stmt_index,
                    stmt_hash,
                    stmt_path,
                    stmt_path_hashes,
                    context,
                    dup_rank: 0,
                    in_closure: self.closure_depth > 0,
                },
                span: to_span_range(expr_span),
                fun_span: to_span_range(callee_span),
                is_method,
            },
        });
    }
}

impl<'ast> Visit<'ast> for BodyVisitor<'_> {
    fn visit_block(&mut self, node: &'ast Block) {
        let hashes: Vec<u64> = node.stmts.iter().map(|s| hash_tokens(quote!(#s))).collect();
        for (idx, stmt) in node.stmts.iter().enumerate() {
            let prev = if idx > 0 { hashes[idx - 1] } else { 0 };
            let next = if idx + 1 < hashes.len() { hashes[idx + 1] } else { 0 };
            self.stmt_stack.push(StmtFrame {
                index: idx,
                hash: hashes[idx],
                context: (prev, next),
            });
            visit::visit_stmt(self, stmt);
            self.stmt_stack.pop();
        }
    }

    fn visit_expr_call(&mut self, node: &'ast ExprCall) {
        self.record_call(
            callee_text(&node.func),
            hash_tokens(quote!(#node)),
            node.func.span(),
            node.span(),
            false,
        );
        visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast ExprMethodCall) {
        self.record_call(
            format!(".{}", node.method),
            hash_tokens(quote!(#node)),
            node.method.span(),
            node.span(),
            true,
        );
        visit::visit_expr_method_call(self, node);
    }

    fn visit_expr_closure(&mut self, node: &'ast ExprClosure) {
        self.closure_depth += 1;
        visit::visit_expr_closure(self, node);
        self.closure_depth -= 1;
    }

    fn visit_item_fn(&mut self, _node: &'ast ItemFn) {}
    fn visit_impl_item_fn(&mut self, _node: &'ast ImplItemFn) {}
    fn visit_trait_item_fn(&mut self, _node: &'ast TraitItemFn) {}
}

/// Assigns `dup_rank` per `(caller, callee, anchor)` in slice order.
pub fn assign_dup_ranks(sites: &mut [Site]) {
    let mut dup_counts: BTreeMap<(String, String, u64), u32> = BTreeMap::new();
    for site in sites {
        let count = dup_counts
            .entry((site.caller.clone(), site.callee.clone(), site.anchor))
            .or_insert(0);
        site.dup_rank = *count;
        *count += 1;
    }
}

/// Extracts all call expressions with source spans and statement snapshot from a function block.
pub fn extract_block_calls(block: &Block, caller: &str) -> (Vec<ExtractedCall>, CallerSnapshot) {
    let body_stmt_hashes = block.stmts.iter().map(|s| hash_tokens(quote!(#s))).collect();
    let mut visitor = BodyVisitor {
        caller,
        stmt_stack: Vec::new(),
        closure_depth: 0,
        raw_sites: Vec::new(),
    };
    visitor.visit_block(block);
    visitor.raw_sites.sort_by_key(|raw| raw.sort_key);
    let mut dup_counts: BTreeMap<(String, u64), u32> = BTreeMap::new();
    let calls = visitor
        .raw_sites
        .into_iter()
        .map(|mut raw| {
            let count = dup_counts
                .entry((raw.call.site.callee.clone(), raw.call.site.anchor))
                .or_insert(0);
            raw.call.site.dup_rank = *count;
            *count += 1;
            raw.call
        })
        .collect();
    (
        calls,
        CallerSnapshot {
            caller: caller.to_owned(),
            body_stmt_hashes,
        },
    )
}

/// Extracts all call sites in the first function/method named `caller`.
pub fn extract(source: &str, caller: &str) -> Vec<Site> {
    extract_caller(source, caller).0
}

/// Extracts all call sites and the top-level statement hash snapshot for `caller`.
pub fn extract_caller(source: &str, caller: &str) -> (Vec<Site>, CallerSnapshot) {
    let file = syn::parse_file(source).expect("source must parse as valid Rust");
    let target_name = caller.rsplit("::").next().unwrap_or(caller);
    let mut finder = FindFn {
        target_name,
        found: None,
    };
    finder.visit_file(&file);
    let Some(body) = finder.found else {
        return (
            Vec::new(),
            CallerSnapshot {
                caller: caller.to_owned(),
                body_stmt_hashes: Vec::new(),
            },
        );
    };
    let (calls, snapshot) = extract_block_calls(body, caller);
    (calls.into_iter().map(|c| c.site).collect(), snapshot)
}
