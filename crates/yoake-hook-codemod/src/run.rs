use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use syn::spanned::Spanned;
use syn::visit_mut::{self, VisitMut};
use syn::{
    Attribute, Block, ExprAssign, ExprCall, ExprMethodCall, ImplItem, ImplItemFn, Item, ItemFn,
    ItemImpl, ItemMod, ItemTrait, Path as SynPath, Signature, Type,
};
use yoake_hook_ledger::{Kind, Ledger};

use crate::attr::{self, ExpectedHookAttr, FileStyle};
use crate::edit::{self, LineIndex, TextEdit, TraitMethodConversion};
use crate::{Config, Mode, eligible, key, modules};

#[derive(Default)]
struct SummaryStats {
    hooked: usize,
    newly_assigned: usize,
    sig_changes: usize,
    trait_methods_converted: usize,
    tombstoned: usize,
    excluded_by_reason: BTreeMap<String, usize>,
    callsite_keys: Vec<String>,
    ambiguous_callsite_candidates: usize,
    store_keys: Vec<String>,
}

struct HelperTarget {
    method_span: (usize, usize),
    method: ImplItemFn,
    impl_close_brace_start: usize,
}

#[derive(Clone)]
struct PairedHelper {
    impl_close_brace_start: usize,
    method: ImplItemFn,
}

struct CurrentImpl {
    is_trait: bool,
    prefix: String,
    self_ty: Type,
    trait_path: Option<SynPath>,
    self_ty_src: Option<String>,
    self_ty_norm: Option<String>,
    impl_generics_check: Result<(), &'static str>,
    trait_meta: Option<eligible::TraitImplMeta>,
    existing_helper_close: Option<usize>,
    conversions: Vec<TraitMethodConversion>,
}

struct CallerState {
    key: String,
    callsite_ordinals: BTreeMap<String, usize>,
    store_ordinals: BTreeMap<String, usize>,
}

struct ScanCtx<'a> {
    crate_ident: &'a str,
    targets: &'a [Vec<String>],
    tombstone_missing: bool,
    ledger: &'a mut Ledger,
    seen_locations: &'a mut BTreeMap<String, String>,
    seen_keys: &'a mut BTreeSet<String>,
    stats: &'a mut SummaryStats,
}

struct FileVisitor<'a, 'b> {
    file_path: &'a Path,
    source: &'a str,
    idx: &'a LineIndex,
    mod_path: Vec<String>,
    style: FileStyle,
    in_target_mod: bool,
    matched_helpers: BTreeMap<(usize, usize), String>,
    trait_pairs: BTreeMap<(usize, usize), PairedHelper>,
    current_impl: Option<CurrentImpl>,
    current_caller: Option<CallerState>,
    ctx: &'a mut ScanCtx<'b>,
    edits: &'a mut Vec<TextEdit>,
    error: Option<String>,
}

impl FileVisitor<'_, '_> {
    fn visit_fn_body(&mut self, caller_key: String, block: &mut Block) {
        let prev = self.current_caller.replace(CallerState {
            key: caller_key,
            callsite_ordinals: BTreeMap::new(),
            store_ordinals: BTreeMap::new(),
        });
        visit_mut::visit_block_mut(self, block);
        self.current_caller = prev;
    }
}

impl VisitMut for FileVisitor<'_, '_> {
    fn visit_file_mut(&mut self, file: &mut syn::File) {
        self.in_target_mod = modules::is_target_or_descendant(&self.mod_path, self.ctx.targets);
        if self.in_target_mod {
            let prefix = key::module_prefix(self.ctx.crate_ident, &self.mod_path);
            let (matched, pairs) =
                pair_converted_trait_helpers(self.source, self.idx, &prefix, &file.items);
            self.matched_helpers = matched;
            self.trait_pairs = pairs;
        }
        for item in &mut file.items {
            self.visit_item_mut(item);
            if self.error.is_some() {
                break;
            }
        }
    }

    fn visit_item_mod_mut(&mut self, node: &mut ItemMod) {
        if self.error.is_some() || self.current_caller.is_some() {
            return;
        }
        if node.ident == "tests" || modules::has_cfg_test(&node.attrs) {
            return;
        }
        let Some((_, inner)) = &mut node.content else {
            return;
        };

        let prev_mod_path = self.mod_path.clone();
        let prev_style = self.style.clone();
        let prev_in_target = self.in_target_mod;
        let prev_matched = std::mem::take(&mut self.matched_helpers);
        let prev_pairs = std::mem::take(&mut self.trait_pairs);

        self.mod_path.push(node.ident.to_string());
        self.style = attr::merge_module_style(&prev_style, inner);
        self.in_target_mod = modules::is_target_or_descendant(&self.mod_path, self.ctx.targets);
        if self.in_target_mod {
            let prefix = key::module_prefix(self.ctx.crate_ident, &self.mod_path);
            let (matched, pairs) =
                pair_converted_trait_helpers(self.source, self.idx, &prefix, inner);
            self.matched_helpers = matched;
            self.trait_pairs = pairs;
        }

        for item in inner {
            self.visit_item_mut(item);
            if self.error.is_some() {
                break;
            }
        }

        self.mod_path = prev_mod_path;
        self.style = prev_style;
        self.in_target_mod = prev_in_target;
        self.matched_helpers = prev_matched;
        self.trait_pairs = prev_pairs;
    }

    fn visit_item_trait_mut(&mut self, _node: &mut ItemTrait) {}

    fn visit_item_fn_mut(&mut self, item_fn: &mut ItemFn) {
        if self.error.is_some()
            || self.current_caller.is_some()
            || !self.in_target_mod
            || modules::has_cfg_test(&item_fn.attrs)
        {
            return;
        }

        let prefix = key::module_prefix(self.ctx.crate_ident, &self.mod_path);
        let fn_key = key::free_fn_key(&prefix, &item_fn.sig.ident);
        let line = item_fn.sig.ident.span().start().line;
        if let Err(err) = register_seen(self.ctx, &fn_key, self.file_path, line) {
            self.error = Some(err);
            return;
        }

        let sig = key::signature_hash(&item_fn.sig);
        let anchor = edit::fn_insert_anchor(
            self.source,
            self.idx,
            &item_fn.attrs,
            &item_fn.vis,
            &item_fn.sig,
        );
        match eligible::check_fn(&item_fn.attrs, &item_fn.sig) {
            Ok(()) => {
                let id = assign_hook(self.ctx, &fn_key, &sig);
                let expected = ExpectedHookAttr {
                    method: false,
                    self_ty_src: None,
                    self_ty_norm: None,
                    id,
                    key: &fn_key,
                };
                edit::plan_ensure_attr(
                    self.source,
                    self.idx,
                    &item_fn.attrs,
                    anchor,
                    &expected,
                    &self.style,
                    self.edits,
                );
            }
            Err(reason) => {
                if let Err(err) = exclude_hook(self.ctx, &fn_key, reason) {
                    self.error = Some(err);
                    return;
                }
                edit::plan_remove_hook_attrs(self.source, self.idx, &item_fn.attrs, self.edits);
            }
        }

        self.visit_fn_body(fn_key, &mut item_fn.block);
    }

    fn visit_item_impl_mut(&mut self, item_impl: &mut ItemImpl) {
        if self.error.is_some()
            || self.current_caller.is_some()
            || !self.in_target_mod
            || modules::has_cfg_test(&item_impl.attrs)
        {
            return;
        }

        let prefix = key::module_prefix(self.ctx.crate_ident, &self.mod_path);
        let (self_ty_src, self_ty_norm) = impl_self_ty_spec(self.source, self.idx, item_impl);
        let trait_path = item_impl.trait_.as_ref().map(|(_, p, _)| p.clone());
        let is_trait = trait_path.is_some();
        let trait_meta = trait_path.as_ref().map(|tp| {
            eligible::TraitImplMeta::from_impl(
                item_impl,
                attr::trait_is_in_scope(tp, &self.style),
            )
        });
        let impl_generics_check = eligible::check_impl_generics(&item_impl.generics);

        let prev_impl = self.current_impl.replace(CurrentImpl {
            is_trait,
            prefix,
            self_ty: (*item_impl.self_ty).clone(),
            trait_path,
            self_ty_src,
            self_ty_norm,
            impl_generics_check,
            trait_meta,
            existing_helper_close: None,
            conversions: Vec::new(),
        });

        for item in &mut item_impl.items {
            self.visit_impl_item_mut(item);
            if self.error.is_some() {
                break;
            }
        }

        let finished = self.current_impl.take();
        self.current_impl = prev_impl;

        if self.error.is_none()
            && let Some(finished) = finished
            && finished.is_trait
        {
            edit::plan_trait_impl_conversions(
                self.source,
                self.idx,
                item_impl,
                finished.existing_helper_close,
                &finished.conversions,
                &self.style,
                self.edits,
            );
        }
    }

    fn visit_impl_item_fn_mut(&mut self, method: &mut ImplItemFn) {
        if self.error.is_some()
            || self.current_caller.is_some()
            || !self.in_target_mod
            || modules::has_cfg_test(&method.attrs)
        {
            return;
        }

        let method_span = self.idx.span_range(self.source, method.span());
        let Some(cur) = self.current_impl.as_ref() else {
            return;
        };

        if !cur.is_trait {
            if let Some(trait_caller_key) = self.matched_helpers.get(&method_span).cloned() {
                self.visit_fn_body(trait_caller_key, &mut method.block);
                return;
            }

            let fn_key = key::inherent_method_key(&cur.prefix, &cur.self_ty, &method.sig.ident);
            let self_ty_src = cur.self_ty_src.clone();
            let self_ty_norm = cur.self_ty_norm.clone();
            let impl_check = cur.impl_generics_check;

            let line = method.sig.ident.span().start().line;
            if let Err(err) = register_seen(self.ctx, &fn_key, self.file_path, line) {
                self.error = Some(err);
                return;
            }

            let sig = key::signature_hash(&method.sig);
            let anchor = edit::fn_insert_anchor(
                self.source,
                self.idx,
                &method.attrs,
                &method.vis,
                &method.sig,
            );
            let eligibility =
                impl_check.and_then(|()| eligible::check_fn(&method.attrs, &method.sig));
            match eligibility {
                Ok(()) => {
                    let id = assign_hook(self.ctx, &fn_key, &sig);
                    let expected = ExpectedHookAttr {
                        method: true,
                        self_ty_src: self_ty_src.as_deref(),
                        self_ty_norm: self_ty_norm.as_deref(),
                        id,
                        key: &fn_key,
                    };
                    edit::plan_ensure_attr(
                        self.source,
                        self.idx,
                        &method.attrs,
                        anchor,
                        &expected,
                        &self.style,
                        self.edits,
                    );
                }
                Err(reason) => {
                    if let Err(err) = exclude_hook(self.ctx, &fn_key, reason) {
                        self.error = Some(err);
                        return;
                    }
                    edit::plan_remove_hook_attrs(self.source, self.idx, &method.attrs, self.edits);
                }
            }

            self.visit_fn_body(fn_key, &mut method.block);
            return;
        }

        let trait_path = cur.trait_path.clone().expect("trait impl has trait_path");
        let fn_key =
            key::trait_method_key(&cur.prefix, &cur.self_ty, &trait_path, &method.sig.ident);
        let self_ty_src = cur.self_ty_src.clone();
        let self_ty_norm = cur.self_ty_norm.clone();

        let line = method.sig.ident.span().start().line;
        if let Err(err) = register_seen(self.ctx, &fn_key, self.file_path, line) {
            self.error = Some(err);
            return;
        }

        if let Some(paired) = self.trait_pairs.get(&method_span).cloned() {
            if let Some(cur_mut) = self.current_impl.as_mut() {
                cur_mut.existing_helper_close = Some(paired.impl_close_brace_start);
            }
            let helper = &paired.method;
            let mut restored_sig = helper.sig.clone();
            restored_sig.ident = method.sig.ident.clone();
            let sig = key::signature_hash(&restored_sig);

            let trait_meta = self
                .current_impl
                .as_ref()
                .and_then(|c| c.trait_meta.as_ref())
                .expect("trait impl has trait_meta");
            match check_trait_eligibility(trait_meta, &method.attrs, &method.sig, &helper.block) {
                Ok(()) => {
                    let id = assign_hook(self.ctx, &fn_key, &sig);
                    let expected = ExpectedHookAttr {
                        method: true,
                        self_ty_src: self_ty_src.as_deref(),
                        self_ty_norm: self_ty_norm.as_deref(),
                        id,
                        key: &fn_key,
                    };
                    let anchor = edit::fn_insert_anchor(
                        self.source,
                        self.idx,
                        &helper.attrs,
                        &helper.vis,
                        &helper.sig,
                    );
                    edit::plan_ensure_attr(
                        self.source,
                        self.idx,
                        &helper.attrs,
                        anchor,
                        &expected,
                        &self.style,
                        self.edits,
                    );
                    edit::plan_remove_hook_attrs(self.source, self.idx, &method.attrs, self.edits);
                }
                Err(reason) => {
                    if let Err(err) = exclude_hook(self.ctx, &fn_key, reason) {
                        self.error = Some(err);
                        return;
                    }
                    edit::plan_remove_hook_attrs(self.source, self.idx, &helper.attrs, self.edits);
                    edit::plan_remove_hook_attrs(self.source, self.idx, &method.attrs, self.edits);
                }
            }
            return;
        }

        let sig = key::signature_hash(&method.sig);
        let trait_meta = self
            .current_impl
            .as_ref()
            .and_then(|c| c.trait_meta.as_ref())
            .expect("trait impl has trait_meta");
        match check_trait_eligibility(trait_meta, &method.attrs, &method.sig, &method.block) {
            Ok(()) => {
                let id = assign_hook(self.ctx, &fn_key, &sig);
                self.ctx.stats.trait_methods_converted += 1;
                let helper_name = key::trait_helper_name(&trait_path, &method.sig.ident);
                let method_clone = method.clone();
                if let Some(cur_mut) = self.current_impl.as_mut() {
                    cur_mut.conversions.push(TraitMethodConversion {
                        method: method_clone,
                        helper_name,
                        id,
                        key: fn_key.clone(),
                        self_ty_src,
                        self_ty_norm,
                    });
                }
            }
            Err(reason) => {
                if let Err(err) = exclude_hook(self.ctx, &fn_key, reason) {
                    self.error = Some(err);
                    return;
                }
                edit::plan_remove_hook_attrs(self.source, self.idx, &method.attrs, self.edits);
            }
        }

        self.visit_fn_body(fn_key, &mut method.block);
    }

    fn visit_expr_call_mut(&mut self, node: &mut ExprCall) {
        if self.error.is_some() {
            return;
        }
        self.visit_expr_mut(&mut node.func);
        if let Some(caller) = self.current_caller.as_mut() {
            let (callee_path, ambiguous) = key::call_expr_callee(&node.func);
            let entry = caller
                .callsite_ordinals
                .entry(callee_path.clone())
                .or_insert(0);
            let ordinal = *entry;
            *entry += 1;
            let cs_key = key::callsite_key(&caller.key, &callee_path, ordinal);
            self.ctx.stats.callsite_keys.push(cs_key);
            if ambiguous {
                self.ctx.stats.ambiguous_callsite_candidates += 1;
            }
        }
        for arg in &mut node.args {
            self.visit_expr_mut(arg);
        }
    }

    fn visit_expr_method_call_mut(&mut self, node: &mut ExprMethodCall) {
        if self.error.is_some() {
            return;
        }
        self.visit_expr_mut(&mut node.receiver);
        if let Some(caller) = self.current_caller.as_mut() {
            let callee_path = format!("?::{}", node.method);
            let entry = caller
                .callsite_ordinals
                .entry(callee_path.clone())
                .or_insert(0);
            let ordinal = *entry;
            *entry += 1;
            let cs_key = key::callsite_key(&caller.key, &callee_path, ordinal);
            self.ctx.stats.callsite_keys.push(cs_key);
            self.ctx.stats.ambiguous_callsite_candidates += 1;
        }
        if let Some(turbofish) = &mut node.turbofish {
            self.visit_angle_bracketed_generic_arguments_mut(turbofish);
        }
        for arg in &mut node.args {
            self.visit_expr_mut(arg);
        }
    }

    fn visit_expr_assign_mut(&mut self, node: &mut ExprAssign) {
        if self.error.is_some() {
            return;
        }
        self.visit_expr_mut(&mut node.left);
        if let Some(caller) = self.current_caller.as_mut()
            && let Some(field_path) = key::assign_field_path(&node.left)
        {
            let entry = caller
                .store_ordinals
                .entry(field_path.clone())
                .or_insert(0);
            let ordinal = *entry;
            *entry += 1;
            let st_key = key::store_key(&caller.key, &field_path, ordinal);
            self.ctx.stats.store_keys.push(st_key);
        }
        self.visit_expr_mut(&mut node.right);
    }
}

pub fn execute(config: &Config) -> Result<bool, String> {
    let crate_ident = key::normalize_crate_name(&config.crate_name);
    let targets = config
        .modules
        .iter()
        .map(|m| modules::normalize_module_arg(m))
        .collect::<Result<Vec<_>, _>>()?;

    let mut scanned_files =
        modules::collect_module_files(&config.crate_root, &config.crate_name, &targets)?;

    let original_ledger_text = if config.ledger.exists() {
        Some(
            std::fs::read_to_string(&config.ledger)
                .map_err(|e| format!("failed to read ledger {}: {e}", config.ledger.display()))?,
        )
    } else {
        None
    };

    let mut ledger = match &original_ledger_text {
        Some(text) => Ledger::from_json(text).map_err(|e| format!("invalid ledger: {e}"))?,
        None => Ledger::new(),
    };

    for (old_key, new_key) in &config.renames {
        ledger
            .rename(old_key, new_key)
            .map_err(|e| format!("rename '{old_key}' -> '{new_key}' failed: {e}"))?;
    }

    let mut seen_locations = BTreeMap::new();
    let mut seen_keys = BTreeSet::new();
    let mut stats = SummaryStats::default();
    let mut changed_files: Vec<(PathBuf, String)> = Vec::new();

    for file in &mut scanned_files {
        let idx = LineIndex::new(&file.source);
        let style = attr::detect_file_style(&file.parsed.items, &file.source);
        let mut edits = Vec::new();
        let mut ctx = ScanCtx {
            crate_ident: &crate_ident,
            targets: &targets,
            tombstone_missing: config.tombstone_missing,
            ledger: &mut ledger,
            seen_locations: &mut seen_locations,
            seen_keys: &mut seen_keys,
            stats: &mut stats,
        };

        let mut visitor = FileVisitor {
            file_path: &file.path,
            source: &file.source,
            idx: &idx,
            mod_path: file.root_mod_path.clone(),
            style,
            in_target_mod: false,
            matched_helpers: BTreeMap::new(),
            trait_pairs: BTreeMap::new(),
            current_impl: None,
            current_caller: None,
            ctx: &mut ctx,
            edits: &mut edits,
            error: None,
        };
        visitor.visit_file_mut(&mut file.parsed);
        if let Some(err) = visitor.error {
            return Err(err);
        }

        let new_source = edit::apply_edits(&file.source, edits)?;
        if new_source != file.source {
            changed_files.push((file.path.clone(), new_source));
        }
    }

    reconcile_missing_keys(
        &crate_ident,
        &targets,
        &seen_keys,
        config.tombstone_missing,
        &mut ledger,
        &mut stats,
    )?;

    let new_ledger_text = ledger.to_json();
    let ledger_changed = original_ledger_text.as_deref() != Some(new_ledger_text.as_str());
    print_summary(&stats);

    match config.mode {
        Mode::Check => {
            if changed_files.is_empty() && !ledger_changed {
                println!("check: clean (no changes)");
                Ok(true)
            } else {
                println!("check: changes needed:");
                for (path, _) in &changed_files {
                    println!("  file: {}", path.display());
                }
                if ledger_changed {
                    println!("  ledger: {}", config.ledger.display());
                }
                Ok(false)
            }
        }
        Mode::Apply => {
            let mut written_paths = Vec::with_capacity(changed_files.len());
            for (path, content) in changed_files {
                std::fs::write(&path, content)
                    .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
                written_paths.push(path);
            }
            if config.rustfmt && !written_paths.is_empty() {
                run_rustfmt(&config.crate_root, &written_paths)?;
            }
            if ledger_changed {
                if let Some(parent) = config.ledger.parent()
                    && !parent.as_os_str().is_empty()
                {
                    std::fs::create_dir_all(parent).map_err(|e| {
                        format!("failed to create directory {}: {e}", parent.display())
                    })?;
                }
                ledger
                    .save(&config.ledger)
                    .map_err(|e| format!("failed to save ledger {}: {e}", config.ledger.display()))?;
            }
            Ok(true)
        }
    }
}

fn run_rustfmt(crate_root: &Path, paths: &[PathBuf]) -> Result<(), String> {
    if paths.is_empty() {
        return Ok(());
    }
    let edition = modules::detect_crate_edition(crate_root);
    let mut cmd = Command::new("rustfmt");
    cmd.arg("--edition").arg(&edition);
    for path in paths {
        cmd.arg(path);
    }
    match cmd.output() {
        Ok(output) if output.status.success() => Ok(()),
        Ok(output) => Err(format!(
            "rustfmt failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("warning: rustfmt not found on PATH; skipping formatting");
            Ok(())
        }
        Err(err) => Err(format!("failed to execute rustfmt: {err}")),
    }
}

fn reconcile_missing_keys(
    crate_ident: &str,
    targets: &[Vec<String>],
    seen_keys: &BTreeSet<String>,
    tombstone_missing: bool,
    ledger: &mut Ledger,
    stats: &mut SummaryStats,
) -> Result<(), String> {
    let prefixes: Vec<String> = targets
        .iter()
        .map(|t| format!("{}::", key::module_prefix(crate_ident, t)))
        .collect();

    let missing_live: Vec<String> = ledger
        .live()
        .filter(|e| {
            e.kind == Kind::Fn
                && prefixes.iter().any(|p| e.key.starts_with(p))
                && !seen_keys.contains(&e.key)
        })
        .map(|e| e.key.clone())
        .collect();

    if !missing_live.is_empty() {
        if tombstone_missing {
            for missing_key in missing_live {
                ledger
                    .tombstone(&missing_key)
                    .map_err(|e| format!("failed to tombstone '{missing_key}': {e}"))?;
                stats.tombstoned += 1;
            }
        } else {
            return Err(format!(
                "missing ledger keys in scanned modules (pass --rename OLD=NEW or --tombstone-missing):\n  {}",
                missing_live.join("\n  ")
            ));
        }
    }

    let stale_excluded: Vec<String> = ledger
        .excluded()
        .iter()
        .filter(|ex| prefixes.iter().any(|p| ex.key.starts_with(p)) && !seen_keys.contains(&ex.key))
        .map(|ex| ex.key.clone())
        .collect();
    for stale_key in stale_excluded {
        ledger.unexclude(&stale_key);
    }
    Ok(())
}

fn pair_converted_trait_helpers(
    source: &str,
    idx: &LineIndex,
    prefix: &str,
    items: &[Item],
) -> (
    BTreeMap<(usize, usize), String>,
    BTreeMap<(usize, usize), PairedHelper>,
) {
    let mut helpers: BTreeMap<(String, String), HelperTarget> = BTreeMap::new();
    for item in items {
        let Item::Impl(imp) = item else {
            continue;
        };
        if imp.trait_.is_some() || modules::has_cfg_test(&imp.attrs) {
            continue;
        }
        let self_name = key::type_last_segment_name(&imp.self_ty);
        let close_start = idx.span_range(source, imp.brace_token.span.close()).0;
        for impl_item in &imp.items {
            let ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if modules::has_cfg_test(&method.attrs) {
                continue;
            }
            let name = method.sig.ident.to_string();
            if name.starts_with("__yoake_trait_") {
                let method_span = idx.span_range(source, method.span());
                helpers.entry((self_name.clone(), name)).or_insert(HelperTarget {
                    method_span,
                    method: method.clone(),
                    impl_close_brace_start: close_start,
                });
            }
        }
    }

    let mut matched = BTreeMap::new();
    let mut pairs = BTreeMap::new();
    for item in items {
        let Item::Impl(imp) = item else {
            continue;
        };
        let Some((_, trait_path, _)) = &imp.trait_ else {
            continue;
        };
        if modules::has_cfg_test(&imp.attrs) {
            continue;
        }
        let self_name = key::type_last_segment_name(&imp.self_ty);
        for impl_item in &imp.items {
            let ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if modules::has_cfg_test(&method.attrs) {
                continue;
            }
            let expected_helper = key::trait_helper_name(trait_path, &method.sig.ident);
            if edit::is_already_converted_trait_method(method, &expected_helper)
                && let Some(target) = helpers.get(&(self_name.clone(), expected_helper))
            {
                let trait_method_span = idx.span_range(source, method.span());
                let trait_key =
                    key::trait_method_key(prefix, &imp.self_ty, trait_path, &method.sig.ident);
                matched.insert(target.method_span, trait_key);
                pairs.insert(
                    trait_method_span,
                    PairedHelper {
                        impl_close_brace_start: target.impl_close_brace_start,
                        method: target.method.clone(),
                    },
                );
            }
        }
    }

    (matched, pairs)
}

fn check_trait_eligibility(
    trait_meta: &eligible::TraitImplMeta,
    attrs: &[Attribute],
    sig: &Signature,
    block: &Block,
) -> Result<(), &'static str> {
    eligible::check_trait_method_movable(trait_meta, sig, block)?;
    eligible::check_fn(attrs, sig)
}

fn impl_self_ty_spec(
    source: &str,
    idx: &LineIndex,
    item_impl: &ItemImpl,
) -> (Option<String>, Option<String>) {
    if eligible::type_has_non_static_lifetime(&item_impl.self_ty) {
        let (s, e) = idx.span_range(source, item_impl.self_ty.span());
        (
            Some(source[s..e].to_string()),
            Some(attr::normalized_type_tokens(&item_impl.self_ty)),
        )
    } else {
        (None, None)
    }
}

fn register_seen(
    ctx: &mut ScanCtx<'_>,
    fn_key: &str,
    file_path: &Path,
    line: usize,
) -> Result<(), String> {
    let loc = format!("{}:{line}", file_path.display());
    if let Some(prev) = ctx.seen_locations.insert(fn_key.to_string(), loc.clone()) {
        return Err(format!(
            "duplicate hook key '{fn_key}' in crate: {prev} and {loc}"
        ));
    }
    ctx.seen_keys.insert(fn_key.to_string());
    Ok(())
}

fn assign_hook(ctx: &mut ScanCtx<'_>, fn_key: &str, sig: &str) -> u32 {
    ctx.ledger.unexclude(fn_key);
    let assigned = ctx.ledger.assign(Kind::Fn, fn_key, sig);
    ctx.stats.hooked += 1;
    if assigned.new {
        ctx.stats.newly_assigned += 1;
    }
    if assigned.sig_changed {
        ctx.stats.sig_changes += 1;
    }
    assigned.id
}

fn exclude_hook(ctx: &mut ScanCtx<'_>, fn_key: &str, reason: &str) -> Result<(), String> {
    if ctx.ledger.live_id(fn_key).is_some() {
        if ctx.tombstone_missing {
            ctx.ledger
                .tombstone(fn_key)
                .map_err(|e| format!("failed to tombstone '{fn_key}': {e}"))?;
            ctx.stats.tombstoned += 1;
        } else {
            return Err(format!(
                "'{fn_key}' is live in ledger but now ineligible ({reason}); pass --tombstone-missing to tombstone it"
            ));
        }
    }
    ctx.ledger
        .exclude(fn_key, reason)
        .map_err(|e| format!("failed to exclude '{fn_key}': {e}"))?;
    *ctx.stats
        .excluded_by_reason
        .entry(reason.to_string())
        .or_default() += 1;
    Ok(())
}

fn print_summary(stats: &SummaryStats) {
    let excluded_total: usize = stats.excluded_by_reason.values().sum();
    println!(
        "summary: hooked={}, newly_assigned={}, excluded={}, trait_methods_converted={}, sig_changes={}, tombstoned={}",
        stats.hooked,
        stats.newly_assigned,
        excluded_total,
        stats.trait_methods_converted,
        stats.sig_changes,
        stats.tombstoned
    );
    println!(
        "  callsite_candidates={} (ambiguous={}), store_candidates={}",
        stats.callsite_keys.len(),
        stats.ambiguous_callsite_candidates,
        stats.store_keys.len()
    );
    for (reason, count) in &stats.excluded_by_reason {
        println!("  excluded [{reason}]: {count}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visitor_counts_callsite_and_store_candidates_and_ordinals() {
        let source = "\
pub struct Stand {
    pub velocity: f64,
}

impl Stand {
    pub fn tick_settle(&mut self, other: &Self) {
        let a = Vec3::add_vector(1.0);
        let b = self.add_vector(a).add_vector(2.0);
        let c = Vec3::add_vector(b);
        self.velocity = c;
        self.velocity = self.velocity + 1.0;
        let local = 0.0;
        let _ = local;
    }

    pub fn second_caller(&mut self) {
        let _ = Vec3::add_vector(0.0);
        self.velocity = 0.0;
    }
}
";
        let mut parsed = syn::parse_file(source).unwrap();
        let idx = LineIndex::new(source);
        let style = attr::detect_file_style(&parsed.items, source);
        let targets = vec![vec!["entities".to_string()]];
        let mut ledger = Ledger::new();
        let mut seen_locations = BTreeMap::new();
        let mut seen_keys = BTreeSet::new();
        let mut stats = SummaryStats::default();
        let mut edits = Vec::new();
        let mut ctx = ScanCtx {
            crate_ident: "demo",
            targets: &targets,
            tombstone_missing: false,
            ledger: &mut ledger,
            seen_locations: &mut seen_locations,
            seen_keys: &mut seen_keys,
            stats: &mut stats,
        };
        let mut visitor = FileVisitor {
            file_path: Path::new("src/entities.rs"),
            source,
            idx: &idx,
            mod_path: vec!["entities".to_string()],
            style,
            in_target_mod: false,
            matched_helpers: BTreeMap::new(),
            trait_pairs: BTreeMap::new(),
            current_impl: None,
            current_caller: None,
            ctx: &mut ctx,
            edits: &mut edits,
            error: None,
        };
        visitor.visit_file_mut(&mut parsed);
        assert!(visitor.error.is_none());

        assert_eq!(
            stats.callsite_keys,
            vec![
                "demo::entities::Stand::tick_settle@call:Vec3::add_vector#0",
                "demo::entities::Stand::tick_settle@call:?::add_vector#0",
                "demo::entities::Stand::tick_settle@call:?::add_vector#1",
                "demo::entities::Stand::tick_settle@call:Vec3::add_vector#1",
                "demo::entities::Stand::second_caller@call:Vec3::add_vector#0",
            ]
        );
        assert_eq!(stats.ambiguous_callsite_candidates, 2);
        assert_eq!(
            stats.store_keys,
            vec![
                "demo::entities::Stand::tick_settle@store:self.velocity#0",
                "demo::entities::Stand::tick_settle@store:self.velocity#1",
                "demo::entities::Stand::second_caller@store:self.velocity#0",
            ]
        );
    }
}
