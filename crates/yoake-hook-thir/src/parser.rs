use std::collections::{BTreeMap, BTreeSet};
use std::io::BufRead;

use crate::model::*;
use crate::normalize::{normalize_callee, normalize_type, resolution};
use crate::receiver::{NodeKind, ReceiverTree};
use crate::span::{callee_path, defid_path, indentation, parse_span};
use crate::{Error, ParseOptions, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CallState {
    Head,
    Fun,
    Args,
    Done,
}

#[derive(Default)]
struct RawArgument {
    ty: Option<String>,
    span: Option<ParsedSpan>,
    file: Option<String>,
}

struct RawCall {
    indent: usize,
    caller: String,
    whole_span: Option<ParsedSpan>,
    ret_ty: Option<String>,
    callee: Option<String>,
    from_hir_call: bool,
    fn_span: Option<(String, ParsedSpan)>,
    fun_span: Option<ParsedSpan>,
    args: Vec<RawArgument>,
    state: CallState,
    file: Option<String>,
    receiver_root: Option<usize>,
    receiver_node: Option<usize>,
}

struct ExprFrame {
    indent: usize,
    ty: Option<String>,
    span: Option<ParsedSpan>,
    receiver_node: Option<usize>,
}

pub fn parse_reader<R: BufRead>(reader: R, options: &ParseOptions) -> Result<Dump> {
    let targets: Vec<String> = options
        .files
        .iter()
        .map(|file| file.replace('\\', "/"))
        .collect();
    let digests = if !options.file_digests.is_empty() {
        options.file_digests.clone()
    } else if let Some(workspace) = &options.workspace {
        crate::collect_file_digests(workspace, &options.files)?
    } else {
        BTreeMap::new()
    };
    let mut files = targets
        .iter()
        .map(|file| {
            let mut summary = FileSummary::default();
            if let Some(digest) = digests.get(file) {
                summary.sha = digest.sha.clone();
                summary.len = digest.len;
            }
            (file.clone(), summary)
        })
        .collect::<BTreeMap<_, _>>();
    let mut calls = Vec::new();
    let mut stack: Vec<RawCall> = Vec::new();
    let mut exprs: Vec<ExprFrame> = Vec::new();
    let mut tree = ReceiverTree::default();
    let mut caller = String::new();
    let mut saw_fn_span = false;

    for line in reader.lines() {
        let line = line?;
        let text = line.trim();
        if text.is_empty() {
            continue;
        }
        let indent = indentation(&line);
        if indent == 0 && text.starts_with("DefId(") {
            finish_calls(&mut stack, &tree, &options.externs, &mut files, &mut calls)?;
            stack.clear();
            exprs.clear();
            tree = ReceiverTree::default();
            caller = defid_path(text).unwrap_or_default();
            continue;
        }
        while exprs.last().is_some_and(|expr| indent <= expr.indent) {
            exprs.pop();
        }
        finish_ready_calls(
            &mut stack,
            indent,
            &tree,
            &options.externs,
            &mut files,
            &mut calls,
        )?;
        if let Some(expr) = exprs.last_mut() {
            update_expr(expr, indent, text, &mut tree);
        }
        if text == "Expr {" {
            open_expr(indent, &mut stack, &mut exprs, &mut tree);
            continue;
        }
        if text == "Call {" {
            let (ret_ty, whole_span, receiver_node) = exprs.last().map_or((None, None, None), |expr| {
                (expr.ty.clone(), expr.span, expr.receiver_node)
            });
            if let Some(node) = receiver_node {
                tree.nodes[node].kind = NodeKind::Call;
            }
            stack.push(RawCall {
                indent,
                caller: caller.clone(),
                whole_span,
                ret_ty,
                callee: None,
                from_hir_call: true,
                fn_span: None,
                fun_span: None,
                args: Vec::new(),
                state: CallState::Head,
                file: None,
                receiver_root: None,
                receiver_node,
            });
            continue;
        }
        let Some(call) = stack.last_mut() else { continue };
        let relative = indent.saturating_sub(call.indent);
        if relative == 4 {
            match text {
                "fun:" => call.state = CallState::Fun,
                "args: [" => call.state = CallState::Args,
                "]" if call.state == CallState::Args => call.state = CallState::Done,
                _ => {}
            }
            if let Some(path) = callee_path(text) {
                if let Some(node) = call.receiver_node {
                    tree.nodes[node].callee = Some(path.clone());
                }
                call.callee = Some(path);
            } else if let Some(value) = text.strip_prefix("from_hir_call:") {
                call.from_hir_call = value.trim() == "true";
                if let Some(node) = call.receiver_node {
                    tree.nodes[node].from_hir_call = call.from_hir_call;
                }
            } else if text.starts_with("fn_span:") {
                if let Some((file, span)) = parse_span(text) {
                    saw_fn_span = true;
                    call.file = match_target(&file, &targets);
                    call.fn_span = Some((file, span));
                }
            }
        } else if relative == 12 && call.state == CallState::Fun && call.fun_span.is_none() {
            if let Some((_, span)) = parse_span(text) {
                call.fun_span = Some(span);
            }
        } else if relative == 12 && call.state == CallState::Args {
            if let Some(argument) = call.args.last_mut() {
                if let Some(raw) = text.strip_prefix("ty:") {
                    argument.ty = Some(raw.trim().to_owned());
                } else if let Some((file, span)) = parse_span(text) {
                    argument.file = Some(file);
                    argument.span = Some(span);
                }
            }
        }
    }
    finish_calls(&mut stack, &tree, &options.externs, &mut files, &mut calls)?;
    if !saw_fn_span {
        return Err(Error::new("no THIR Call node with fn_span was seen"));
    }
    calls.sort_by_key(|call: &Call| (call.file.clone(), call.span, call.fun_span));
    let crate_name = options.crate_name.clone().unwrap_or_else(|| {
        calls
            .first()
            .and_then(|call| call.caller_def.split("::").next().map(str::to_owned))
            .unwrap_or_default()
    });
    let source_snapshot = crate::compute_source_snapshot(&files);
    Ok(Dump {
        rustc: options.rustc.clone(),
        cargo_lock_sha: options.cargo_lock_sha.clone(),
        crate_name,
        source_snapshot,
        files,
        calls,
    })
}

fn open_expr(
    indent: usize,
    calls: &mut [RawCall],
    exprs: &mut Vec<ExprFrame>,
    tree: &mut ReceiverTree,
) {
    let direct_arg = calls.last().is_some_and(|call| {
        call.state == CallState::Args && indent == call.indent + 8
    });
    let parent = exprs.last().and_then(|expr| expr.receiver_node);
    let first_receiver = direct_arg && calls.last().is_some_and(|call| call.args.is_empty());
    let node = (parent.is_some() || first_receiver).then(|| tree.node());
    if let (Some(parent), Some(node)) = (parent, node) {
        tree.child(parent, node);
    }
    if direct_arg {
        let call = calls.last_mut().unwrap();
        call.args.push(RawArgument::default());
        if call.args.len() == 1 {
            call.receiver_root = node;
        }
        if let (Some(call_node), Some(node)) = (call.receiver_node, node) {
            tree.nodes[call_node].call_args.push(node);
        }
    }
    exprs.push(ExprFrame {
        indent,
        ty: None,
        span: None,
        receiver_node: node,
    });
}

fn update_expr(expr: &mut ExprFrame, indent: usize, text: &str, tree: &mut ReceiverTree) {
    if indent == expr.indent + 4 {
        if let Some(raw) = text.strip_prefix("ty:") {
            expr.ty = Some(raw.trim().to_owned());
        } else if let Some((_, span)) = parse_span(text) {
            expr.span = Some(span);
        }
    }
    let Some(node) = expr.receiver_node else { return };
    if indent == expr.indent + 8 {
        let kind = match text {
            "Scope {" => Some(NodeKind::Scope),
            // An unsizing or other pointer coercion (`&[T; N]` -> `&[T]`): transparent for the
            // receiver adjustment, the call site coerces the same way.
            "Pointer {" => Some(NodeKind::Pointer),
            "Borrow (" => Some(NodeKind::Borrow),
            "Deref {" => Some(NodeKind::Deref),
            "Call {" => Some(NodeKind::Call),
            _ => None,
        };
        if let Some(kind) = kind {
            tree.nodes[node].kind = kind;
        }
    } else if indent == expr.indent + 12
        && tree.nodes[node].kind == NodeKind::Borrow
        && text.starts_with("borrow_kind:")
    {
        // Only the `borrow_kind:` line; the `arg:` line at the same depth must not reset it.
        tree.nodes[node].borrow = match text {
            "borrow_kind: Shared" => Borrow::Shared,
            value if value.starts_with("borrow_kind: Mut") => Borrow::Mut,
            _ => Borrow::None,
        };
    }
}

fn finish_ready_calls(
    stack: &mut Vec<RawCall>,
    indent: usize,
    tree: &ReceiverTree,
    externs: &BTreeSet<String>,
    files: &mut BTreeMap<String, FileSummary>,
    calls: &mut Vec<Call>,
) -> Result<()> {
    while stack.last().is_some_and(|call| indent <= call.indent) {
        finish_one(stack.pop().unwrap(), tree, externs, files, calls)?;
    }
    Ok(())
}

fn finish_calls(
    stack: &mut Vec<RawCall>,
    tree: &ReceiverTree,
    externs: &BTreeSet<String>,
    files: &mut BTreeMap<String, FileSummary>,
    calls: &mut Vec<Call>,
) -> Result<()> {
    while let Some(call) = stack.pop() {
        finish_one(call, tree, externs, files, calls)?;
    }
    Ok(())
}

fn finish_one(
    raw: RawCall,
    tree: &ReceiverTree,
    externs: &BTreeSet<String>,
    files: &mut BTreeMap<String, FileSummary>,
    calls: &mut Vec<Call>,
) -> Result<()> {
    let Some(file) = raw.file else { return Ok(()) };
    let (fn_file, fn_span) = raw.fn_span.ok_or_else(|| Error::new("target Call lacks fn_span"))?;
    let whole_span = raw.whole_span.ok_or_else(|| Error::new("target Call lacks enclosing span"))?;
    let fun_span = raw.fun_span.ok_or_else(|| Error::new("target Call lacks fun span"))?;
    // A Call whose `ty` is not an FnDef calls a fn pointer or closure value: an indirect call.
    // It stays in the list (reported, never rewritten) with resolution `dyn`.
    let indirect = raw.callee.is_none();
    let callee_def = raw.callee.unwrap_or_else(|| "<indirect>".to_owned());
    let is_method_receiver = raw.args.first().is_some_and(|argument| {
        let Some(receiver) = argument.span else { return false };
        argument.file.as_deref() == Some(fn_file.as_str())
            && (receiver.position.0, receiver.position.1) >= (whole_span.position.0, whole_span.position.1)
            && (receiver.position.0, receiver.position.1) < (fun_span.position.0, fun_span.position.1)
    });
    let kind = if !raw.from_hir_call {
        CallKind::Overloaded
    } else if is_method_receiver {
        CallKind::Method
    } else {
        CallKind::Path
    };
    let receiver_ty = raw
        .args
        .first()
        .and_then(|argument| argument.ty.clone());
    let local_crate = raw.caller.split("::").next().unwrap_or_default().to_string();
    // Calls expanded from macros (a non-zero syntax context, e.g. inside `assert!`) can have
    // arguments without their own span. They are never rewritten but stay in the list (so the
    // join can give them a reason); such an argument gets the whole call's span.
    let macro_expanded = fn_span.ctxt != 0;
    let mut normalized_args = Vec::with_capacity(raw.args.len());
    for argument in raw.args {
        normalized_args.push(Argument {
            ty: argument
                .ty
                .as_deref()
                .and_then(|ty| normalize_type(ty, &local_crate, externs)),
            span: argument
                .span
                .or(macro_expanded.then_some(whole_span))
                .ok_or_else(|| {
                    Error::new(format!(
                        "target Call argument lacks span (call at {:?})",
                        whole_span.position
                    ))
                })?
                .position,
        });
    }
    let recv_adjust = (kind == CallKind::Method).then(|| tree.adjustment(raw.receiver_root));
    if fn_span.ctxt == 0 && matches!(kind, CallKind::Method | CallKind::Path) {
        files.entry(file.clone()).or_default().written_calls += 1;
    }
    calls.push(Call {
        file,
        span: whole_span.position,
        fun_span: fun_span.position,
        ctxt: fn_span.ctxt,
        kind,
        caller_def: raw.caller,
        callee: if indirect {
            callee_def.clone()
        } else {
            normalize_callee(&callee_def, receiver_ty.as_deref(), kind, &local_crate, externs)
        },
        callee_def,
        ret_ty: raw
            .ret_ty
            .as_deref()
            .and_then(|ty| normalize_type(ty, &local_crate, externs)),
        args: normalized_args,
        recv_adjust,
        resolution: if indirect { Resolution::Dyn } else { resolution(receiver_ty.as_deref()) },
    });
    Ok(())
}

fn match_target(file: &str, targets: &[String]) -> Option<String> {
    targets.iter().find_map(|target| {
        (file == target || file.ends_with(&format!("/{target}"))).then(|| target.clone())
    })
}
