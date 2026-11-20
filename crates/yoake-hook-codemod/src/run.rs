use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use syn::spanned::Spanned;
use syn::visit_mut::{self, VisitMut};
use syn::{
    Attribute, Block, ExprAssign, ExprCall, ExprMethodCall, ImplItem, ImplItemFn, Item, ItemFn,
    ItemImpl, ItemMod, ItemTrait, Path as SynPath, Signature, Type,
};
use yoake_hook_ledger::{Kind, LedgerSet};

use crate::attr::{self, ExpectedHookAttr, FileStyle};
use crate::edit::{self, LineIndex, TextEdit, TraitMethodConversion};
use crate::ledger_io::{self, FileUpdate};
use crate::types::{CrateIndex, SelfTyStatus};
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
pub(crate) struct PairedHelper {
    impl_close_brace_start: usize,
    method: ImplItemFn,
}

struct CurrentImpl {
    is_trait: bool,
    prefix: String,
    self_ty: Type,
    trait_path: Option<SynPath>,
    impl_cfg_tags: Vec<String>,
    self_ty_src: Option<String>,
    self_ty_norm: Option<String>,
    impl_generics_check: Result<(), &'static str>,
    self_ty_check: Result<(), &'static str>,
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
    shallow: bool,
    crate_index: &'a CrateIndex,
    colliding_short_keys: &'a BTreeSet<String>,
    tombstones: &'a mut BTreeSet<String>,
    manual_keys: &'a mut BTreeMap<String, String>,
    ledger_set: &'a mut LedgerSet,
    seen_locations: &'a mut BTreeMap<String, String>,
    seen_keys: &'a mut BTreeSet<String>,
    stats: &'a mut SummaryStats,
    /// W5: functions with at most this many statements are not stamped (0 = no rule).
    min_stmts: usize,
    /// W5: keys stamped regardless of the size rule (`hook-ledger/stamp-allow.txt`).
    allow_small: &'a BTreeSet<String>,
}

/// W5 size rule: a small body is excluded unless it is allow-listed or already live in the ledger.
fn size_gate(ctx: &mut ScanCtx<'_>, fn_key: &str, block: &Block) -> Result<(), &'static str> {
    if ctx.min_stmts == 0
        || block.stmts.len() > ctx.min_stmts
        || ctx.allow_small.contains(fn_key)
        || ctx.ledger_set.shard_mut(ctx.crate_ident).live_id(fn_key).is_some()
    {
        return Ok(());
    }
    Err("small body")
}

struct FileVisitor<'a, 'b> {
    file_path: &'a Path,
    source: &'a str,
    idx: &'a LineIndex,
    mod_path: Vec<String>,
    mod_cfg_tags: Vec<String>,
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
        self.in_target_mod =
            modules::is_target_or_descendant(&self.mod_path, self.ctx.targets, self.ctx.shallow);
        if self.in_target_mod {
            let prefix = key::module_prefix(self.ctx.crate_ident, &self.mod_path);
            let (matched, pairs) = pair_converted_trait_helpers(
                self.source,
                self.idx,
                &prefix,
                &self.mod_cfg_tags,
                self.ctx.colliding_short_keys,
                &file.items,
            );
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
        if modules::has_cfg_test(&node.attrs) {
            return;
        }
        let Some((_, inner)) = &mut node.content else {
            return;
        };

        let prev_mod_path = self.mod_path.clone();
        let prev_mod_cfgs = self.mod_cfg_tags.clone();
        let prev_style = self.style.clone();
        let prev_in_target = self.in_target_mod;
        let prev_matched = std::mem::take(&mut self.matched_helpers);
        let prev_pairs = std::mem::take(&mut self.trait_pairs);

        self.mod_path.push(node.ident.to_string());
        self.mod_cfg_tags.extend(key::extract_cfg_tags(&node.attrs));
        self.style = attr::merge_module_style(&prev_style, inner);
        self.in_target_mod =
            modules::is_target_or_descendant(&self.mod_path, self.ctx.targets, self.ctx.shallow);
        if self.in_target_mod {
            let prefix = key::module_prefix(self.ctx.crate_ident, &self.mod_path);
            let (matched, pairs) = pair_converted_trait_helpers(
                self.source,
                self.idx,
                &prefix,
                &self.mod_cfg_tags,
                self.ctx.colliding_short_keys,
                inner,
            );
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
        self.mod_cfg_tags = prev_mod_cfgs;
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
            || is_callsite_shim_fn(item_fn, self.source, self.idx)
        {
            return;
        }

        let prefix = key::module_prefix(self.ctx.crate_ident, &self.mod_path);
        let mut fn_cfg_tags = self.mod_cfg_tags.clone();
        fn_cfg_tags.extend(key::extract_cfg_tags(&item_fn.attrs));
        let fn_key = key::free_fn_key(&prefix, &item_fn.sig.ident, &fn_cfg_tags);
        let line = item_fn.sig.ident.span().start().line;
        if let Err(err) = register_seen(self.ctx, &fn_key, self.file_path, line) {
            self.error = Some(err);
            return;
        }

        let found_attrs = attr::find_hook_attrs(&item_fn.attrs, self.source, self.idx);
        if let Err(err) = check_no_test_id(&found_attrs, self.file_path, line) {
            self.error = Some(err);
            return;
        }

        let manual_attr_reason = attr::find_manual_skip(&found_attrs);
        let is_ledger_manual = self.ctx.manual_keys.contains_key(&fn_key);
        if manual_attr_reason.is_some() || is_ledger_manual {
            if let Err(err) = exclude_manual_hook(self.ctx, &fn_key, manual_attr_reason) {
                self.error = Some(err);
                return;
            }
            edit::plan_remove_hook_attrs(self.source, self.idx, &item_fn.attrs, self.edits);
            self.visit_fn_body(fn_key, &mut item_fn.block);
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
        let gate = size_gate(self.ctx, &fn_key, &item_fn.block);
        match eligible::check_fn(&item_fn.attrs, &item_fn.sig).and_then(|()| gate) {
            Ok(()) => {
                let id = match assign_hook(self.ctx, &fn_key, &sig) {
                    Ok(id) => id,
                    Err(err) => {
                        self.error = Some(err);
                        return;
                    }
                };
                let expected = ExpectedHookAttr {
                    method: false,
                    self_ty_src: None,
                    self_ty_norm: None,
                    id,
                    key: &fn_key,
                    kind: None,
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
        let mut impl_cfg_tags = self.mod_cfg_tags.clone();
        impl_cfg_tags.extend(key::extract_cfg_tags(&item_impl.attrs));
        let (self_ty_src, self_ty_norm) = impl_self_ty_spec(self.source, self.idx, item_impl);
        let trait_path = item_impl.trait_.as_ref().map(|(_, p, _)| p.clone());
        let is_trait = trait_path.is_some();
        let self_ty_status = self
            .ctx
            .crate_index
            .check_self_ty(&self.mod_path, &item_impl.self_ty);
        let self_ty_check = if is_trait {
            match self_ty_status {
                SelfTyStatus::Local {
                    is_struct_enum_union: true,
                    crate_level_visible: true,
                } => Ok(()),
                SelfTyStatus::Local {
                    is_struct_enum_union: true,
                    crate_level_visible: false,
                } => Err("self type has no crate-level path"),
                SelfTyStatus::Local {
                    is_struct_enum_union: false,
                    ..
                }
                | SelfTyStatus::ForeignOrPrimitive
                | SelfTyStatus::NonPath => {
                    Err("trait impl on a type that cannot take an inherent impl")
                }
            }
        } else {
            match self_ty_status {
                SelfTyStatus::Local {
                    crate_level_visible: false,
                    ..
                } => Err("self type has no crate-level path"),
                _ => Ok(()),
            }
        };

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
            impl_cfg_tags,
            self_ty_src,
            self_ty_norm,
            impl_generics_check,
            self_ty_check,
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
        let line = method.sig.ident.span().start().line;

        if !cur.is_trait {
            if let Some(trait_caller_key) = self.matched_helpers.get(&method_span).cloned() {
                let found_attrs = attr::find_hook_attrs(&method.attrs, self.source, self.idx);
                if let Err(err) = check_no_test_id(&found_attrs, self.file_path, line) {
                    self.error = Some(err);
                    return;
                }
                self.visit_fn_body(trait_caller_key, &mut method.block);
                return;
            }

            let mut fn_cfg_tags = cur.impl_cfg_tags.clone();
            fn_cfg_tags.extend(key::extract_cfg_tags(&method.attrs));
            let fn_key =
                key::inherent_method_key(&cur.prefix, &cur.self_ty, &method.sig.ident, &fn_cfg_tags);
            let self_ty_src = cur.self_ty_src.clone();
            let self_ty_norm = cur.self_ty_norm.clone();
            let impl_check = cur.self_ty_check.and_then(|()| cur.impl_generics_check);

            if let Err(err) = register_seen(self.ctx, &fn_key, self.file_path, line) {
                self.error = Some(err);
                return;
            }

            let found_attrs = attr::find_hook_attrs(&method.attrs, self.source, self.idx);
            if let Err(err) = check_no_test_id(&found_attrs, self.file_path, line) {
                self.error = Some(err);
                return;
            }

            let manual_attr_reason = attr::find_manual_skip(&found_attrs);
            let is_ledger_manual = self.ctx.manual_keys.contains_key(&fn_key);
            if manual_attr_reason.is_some() || is_ledger_manual {
                if let Err(err) = exclude_manual_hook(self.ctx, &fn_key, manual_attr_reason) {
                    self.error = Some(err);
                    return;
                }
                edit::plan_remove_hook_attrs(self.source, self.idx, &method.attrs, self.edits);
                self.visit_fn_body(fn_key, &mut method.block);
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
            let gate = size_gate(self.ctx, &fn_key, &method.block);
            let eligibility = impl_check
                .and_then(|()| eligible::check_fn(&method.attrs, &method.sig))
                .and_then(|()| gate);
            match eligibility {
                Ok(()) => {
                    let id = match assign_hook(self.ctx, &fn_key, &sig) {
                        Ok(id) => id,
                        Err(err) => {
                            self.error = Some(err);
                            return;
                        }
                    };
                    let expected = ExpectedHookAttr {
                        method: true,
                        self_ty_src: self_ty_src.as_deref(),
                        self_ty_norm: self_ty_norm.as_deref(),
                        id,
                        key: &fn_key,
                        kind: None,
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
        let mut fn_cfg_tags = cur.impl_cfg_tags.clone();
        fn_cfg_tags.extend(key::extract_cfg_tags(&method.attrs));
        let short_key = key::trait_method_key(
            &cur.prefix,
            &cur.self_ty,
            &trait_path,
            &method.sig.ident,
            &fn_cfg_tags,
            false,
        );
        let use_full_trait = self.ctx.colliding_short_keys.contains(&short_key);
        let fn_key = if use_full_trait {
            key::trait_method_key(
                &cur.prefix,
                &cur.self_ty,
                &trait_path,
                &method.sig.ident,
                &fn_cfg_tags,
                true,
            )
        } else {
            short_key
        };
        let self_ty_src = cur.self_ty_src.clone();
        let self_ty_norm = cur.self_ty_norm.clone();
        let self_ty_check = cur.self_ty_check;

        if let Err(err) = register_seen(self.ctx, &fn_key, self.file_path, line) {
            self.error = Some(err);
            return;
        }

        let found_attrs = attr::find_hook_attrs(&method.attrs, self.source, self.idx);
        if let Err(err) = check_no_test_id(&found_attrs, self.file_path, line) {
            self.error = Some(err);
            return;
        }

        if let Some(paired) = self.trait_pairs.get(&method_span).cloned() {
            if let Some(cur_mut) = self.current_impl.as_mut() {
                cur_mut.existing_helper_close = Some(paired.impl_close_brace_start);
            }
            let helper = &paired.method;
            let helper_line = helper.sig.ident.span().start().line;
            let found_helper_attrs = attr::find_hook_attrs(&helper.attrs, self.source, self.idx);
            if let Err(err) = check_no_test_id(&found_helper_attrs, self.file_path, helper_line) {
                self.error = Some(err);
                return;
            }

            let manual_attr_reason =
                attr::find_manual_skip(&found_attrs).or_else(|| attr::find_manual_skip(&found_helper_attrs));
            let is_ledger_manual = self.ctx.manual_keys.contains_key(&fn_key);
            if manual_attr_reason.is_some() || is_ledger_manual {
                if let Err(err) = exclude_manual_hook(self.ctx, &fn_key, manual_attr_reason) {
                    self.error = Some(err);
                    return;
                }
                edit::plan_remove_hook_attrs(self.source, self.idx, &helper.attrs, self.edits);
                edit::plan_remove_hook_attrs(self.source, self.idx, &method.attrs, self.edits);
                return;
            }

            let mut restored_sig = helper.sig.clone();
            restored_sig.ident = method.sig.ident.clone();
            let sig = key::signature_hash(&restored_sig);

            let trait_meta = self
                .current_impl
                .as_ref()
                .and_then(|c| c.trait_meta.as_ref())
                .expect("trait impl has trait_meta");
            let eligibility = self_ty_check
                .and_then(|()| check_trait_eligibility(trait_meta, &method.attrs, &method.sig, &helper.block));
            match eligibility {
                Ok(()) => {
                    let id = match assign_hook(self.ctx, &fn_key, &sig) {
                        Ok(id) => id,
                        Err(err) => {
                            self.error = Some(err);
                            return;
                        }
                    };
                    let expected = ExpectedHookAttr {
                        method: true,
                        self_ty_src: self_ty_src.as_deref(),
                        self_ty_norm: self_ty_norm.as_deref(),
                        id,
                        key: &fn_key,
                        kind: None,
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

        let manual_attr_reason = attr::find_manual_skip(&found_attrs);
        let is_ledger_manual = self.ctx.manual_keys.contains_key(&fn_key);
        if manual_attr_reason.is_some() || is_ledger_manual {
            if let Err(err) = exclude_manual_hook(self.ctx, &fn_key, manual_attr_reason) {
                self.error = Some(err);
                return;
            }
            edit::plan_remove_hook_attrs(self.source, self.idx, &method.attrs, self.edits);
            self.visit_fn_body(fn_key, &mut method.block);
            return;
        }

        let sig = key::signature_hash(&method.sig);
        let trait_meta = self
            .current_impl
            .as_ref()
            .and_then(|c| c.trait_meta.as_ref())
            .expect("trait impl has trait_meta");
        let eligibility = self_ty_check
            .and_then(|()| check_trait_eligibility(trait_meta, &method.attrs, &method.sig, &method.block));
        match eligibility {
            Ok(()) => {
                let id = match assign_hook(self.ctx, &fn_key, &sig) {
                    Ok(id) => id,
                    Err(err) => {
                        self.error = Some(err);
                        return;
                    }
                };
                self.ctx.stats.trait_methods_converted += 1;
                let helper_name = if use_full_trait {
                    key::trait_helper_name_full(&trait_path, &method.sig.ident)
                } else {
                    key::trait_helper_name(&trait_path, &method.sig.ident)
                };
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
    if config.callsites {
        return crate::callsite::execute(config);
    }
    let crate_ident = key::normalize_crate_name(&config.crate_name);
    let module_args = modules::expand_star_modules(
        &config.crate_root,
        &config.crate_name,
        &config.modules,
        &config.exclude_modules,
    )?;
    let allow_small = read_allow_small(&config.ledger);
    let targets = module_args
        .iter()
        .map(|m| modules::normalize_module_arg(m))
        .collect::<Result<Vec<_>, _>>()?;

    let mut scanned_files = modules::collect_module_files(
        &config.crate_root,
        &config.crate_name,
        &targets,
        config.shallow,
    )?;
    let crate_index =
        CrateIndex::build(&config.crate_root, &config.crate_name, &crate_ident, &scanned_files)?;

    let mut ledger_set = ledger_io::load_ledger_set(&config.ledger)?;
    let mut manual_keys = ledger_io::read_manual_exclusions(&config.ledger, &crate_ident)?;
    if let Some(shard) = ledger_set.shard(&crate_ident) {
        for ex in shard.excluded() {
            if ex.manual {
                manual_keys
                    .entry(ex.key.clone())
                    .or_insert_with(|| ex.reason.clone());
            }
        }
    }

    // W1: keys the verify loop withdrew (`hook-ledger/stamp-exclude.txt`): retire a live id, then record the
    // manual exclusion in the ledger (the request file is only the transport between rounds).
    for (key, reason) in crate::verify::read_excludes(&config.ledger) {
        let shard = ledger_set.shard_mut(&crate_ident);
        if shard.live_id(&key).is_some() {
            shard
                .tombstone(&key)
                .map_err(|e| format!("failed to tombstone '{key}': {e}"))?;
        }
        shard
            .exclude_manual(Kind::Fn, &key, &reason)
            .map_err(|e| format!("failed to record manual exclusion '{key}': {e}"))?;
        manual_keys.entry(key).or_insert(reason);
    }

    for (old_key, new_key) in &config.renames {
        ledger_set
            .shard_mut(&crate_ident)
            .rename(old_key, new_key)
            .map_err(|e| format!("rename '{old_key}' -> '{new_key}' failed: {e}"))?;
    }
    ledger_set
        .validate()
        .map_err(|e| format!("invalid ledger after renames: {e}"))?;

    let colliding_short_keys = collect_colliding_short_keys(
        &crate_ident,
        &targets,
        config.shallow,
        &scanned_files,
    );

    let mut tombstones: BTreeSet<String> = config.tombstones.iter().cloned().collect();
    let mut seen_locations = BTreeMap::new();
    let mut seen_keys = BTreeSet::new();
    let mut stats = SummaryStats::default();
    let mut file_updates: Vec<FileUpdate> = Vec::new();

    for file in &mut scanned_files {
        let idx = LineIndex::new(&file.source);
        let style = attr::detect_file_style(&file.parsed.items, &file.source);
        let mut edits = Vec::new();
        let mut ctx = ScanCtx {
            crate_ident: &crate_ident,
            targets: &targets,
            shallow: config.shallow,
            crate_index: &crate_index,
            colliding_short_keys: &colliding_short_keys,
            tombstones: &mut tombstones,
            manual_keys: &mut manual_keys,
            ledger_set: &mut ledger_set,
            seen_locations: &mut seen_locations,
            seen_keys: &mut seen_keys,
            stats: &mut stats,
            min_stmts: config.min_stmts,
            allow_small: &allow_small,
        };

        let mut visitor = FileVisitor {
            file_path: &file.path,
            source: &file.source,
            idx: &idx,
            mod_path: file.root_mod_path.clone(),
            mod_cfg_tags: file.root_cfg_tags.clone(),
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
            file_updates.push(FileUpdate {
                path: file.path.clone(),
                original: file.source.clone(),
                updated: new_source,
            });
        }
    }

    reconcile_missing_keys(
        &crate_ident,
        &targets,
        config.shallow,
        &crate_index,
        &seen_keys,
        &mut tombstones,
        &mut ledger_set,
        &mut stats,
    )?;

    let manual_set: BTreeSet<String> = manual_keys.keys().cloned().collect();
    let staging = ledger_io::stage_ledger_set(&ledger_set, &crate_ident, &manual_set)?;
    let ledger_changed = ledger_io::diff_ledger_dirs(staging.path(), &config.ledger)?;

    print_summary(&stats);

    match config.mode {
        Mode::Check => {
            if file_updates.is_empty() && ledger_changed.is_empty() {
                println!("check: clean (no changes)");
                Ok(true)
            } else {
                println!("check: changes needed:");
                for update in &file_updates {
                    println!("  file: {}", update.path.display());
                }
                for path in &ledger_changed {
                    println!("  ledger: {}", path.display());
                }
                Ok(false)
            }
        }
        Mode::Apply => {
            if !file_updates.is_empty() || !ledger_changed.is_empty() {
                let rustfmt_edition = config
                    .rustfmt
                    .then(|| modules::detect_crate_edition(&config.crate_root));
                ledger_io::atomic_apply(
                    &file_updates,
                    staging.path(),
                    &ledger_changed,
                    &config.ledger,
                    rustfmt_edition.as_deref(),
                )?;
            }
            Ok(true)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn reconcile_missing_keys(
    crate_ident: &str,
    targets: &[Vec<String>],
    shallow: bool,
    crate_index: &CrateIndex,
    seen_keys: &BTreeSet<String>,
    tombstones: &mut BTreeSet<String>,
    ledger_set: &mut LedgerSet,
    stats: &mut SummaryStats,
) -> Result<(), String> {
    let target_scopes: Vec<(String, Vec<String>)> = targets
        .iter()
        .map(|t| {
            let prefix = format!("{}::", key::module_prefix(crate_ident, t));
            let child_prefixes = if shallow {
                crate_index
                    .child_modules(t)
                    .map(|children| children.iter().map(|c| format!("{prefix}{c}::")).collect())
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            (prefix, child_prefixes)
        })
        .collect();
    let in_scan = |key_str: &str| -> bool {
        target_scopes.iter().any(|(prefix, child_prefixes)| {
            key_str.starts_with(prefix)
                && !child_prefixes.iter().any(|cp| key_str.starts_with(cp))
        })
    };

    let shard = ledger_set.shard_mut(crate_ident);
    let missing_live: Vec<String> = shard
        .live()
        .filter(|e| e.kind == Kind::Fn && in_scan(&e.key) && !seen_keys.contains(&e.key))
        .map(|e| e.key.clone())
        .collect();

    let mut unhandled_missing = Vec::new();
    for missing_key in missing_live {
        if tombstones.remove(&missing_key) {
            shard
                .tombstone(&missing_key)
                .map_err(|e| format!("failed to tombstone '{missing_key}': {e}"))?;
            stats.tombstoned += 1;
        } else {
            unhandled_missing.push(missing_key);
        }
    }

    if !unhandled_missing.is_empty() {
        let lines: Vec<String> = unhandled_missing
            .iter()
            .map(|k| format!("  {k} (pass --rename {k}=<NEW_KEY> or --tombstone {k})"))
            .collect();
        return Err(format!(
            "missing live ledger keys in scanned modules:\n{}",
            lines.join("\n")
        ));
    }

    let remaining_tombstones: Vec<String> = tombstones.iter().cloned().collect();
    for key in remaining_tombstones {
        shard
            .tombstone(&key)
            .map_err(|e| format!("failed to tombstone '{key}': {e}"))?;
        stats.tombstoned += 1;
    }

    let stale_excluded: Vec<String> = shard
        .excluded()
        .iter()
        .filter(|ex| !ex.manual && in_scan(&ex.key) && !seen_keys.contains(&ex.key))
        .map(|ex| ex.key.clone())
        .collect();
    for stale_key in stale_excluded {
        shard.unexclude(&stale_key);
    }
    Ok(())
}

fn is_callsite_shim_fn(item_fn: &ItemFn, source: &str, idx: &LineIndex) -> bool {
    if item_fn.sig.ident.to_string().starts_with("__yoake_cs_") {
        return true;
    }
    attr::find_hook_attrs(&item_fn.attrs, source, idx)
        .iter()
        .any(|a| a.parsed.kind.as_deref() == Some("callsite"))
}

pub(crate) fn collect_colliding_short_keys(
    crate_ident: &str,
    targets: &[Vec<String>],
    shallow: bool,
    scanned_files: &[modules::ScannedFile],
) -> BTreeSet<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let empty_colliding = BTreeSet::new();
    for file in scanned_files {
        let idx = LineIndex::new(&file.source);
        collect_short_keys_in_items(
            crate_ident,
            targets,
            shallow,
            &file.source,
            &idx,
            &file.root_mod_path,
            &file.root_cfg_tags,
            &file.parsed.items,
            &empty_colliding,
            &mut counts,
        );
    }
    counts
        .into_iter()
        .filter_map(|(k, count)| (count > 1).then_some(k))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn collect_short_keys_in_items(
    crate_ident: &str,
    targets: &[Vec<String>],
    shallow: bool,
    source: &str,
    idx: &LineIndex,
    mod_path: &[String],
    mod_cfg_tags: &[String],
    items: &[Item],
    empty_colliding: &BTreeSet<String>,
    counts: &mut BTreeMap<String, usize>,
) {
    let in_target = modules::is_target_or_descendant(mod_path, targets, shallow);
    if in_target {
        let prefix = key::module_prefix(crate_ident, mod_path);
        let (matched_helpers, _) = pair_converted_trait_helpers(
            source,
            idx,
            &prefix,
            mod_cfg_tags,
            empty_colliding,
            items,
        );
        for item in items {
            match item {
                Item::Fn(item_fn)
                    if !modules::has_cfg_test(&item_fn.attrs)
                        && !is_callsite_shim_fn(item_fn, source, idx) =>
                {
                    let mut cfgs = mod_cfg_tags.to_vec();
                    cfgs.extend(key::extract_cfg_tags(&item_fn.attrs));
                    let k = key::free_fn_key(&prefix, &item_fn.sig.ident, &cfgs);
                    *counts.entry(k).or_default() += 1;
                }
                Item::Impl(imp) if !modules::has_cfg_test(&imp.attrs) => {
                    let mut impl_cfgs = mod_cfg_tags.to_vec();
                    impl_cfgs.extend(key::extract_cfg_tags(&imp.attrs));
                    let trait_path = imp.trait_.as_ref().map(|(_, p, _)| p);
                    for impl_item in &imp.items {
                        let ImplItem::Fn(method) = impl_item else {
                            continue;
                        };
                        if modules::has_cfg_test(&method.attrs) {
                            continue;
                        }
                        if trait_path.is_none() {
                            let span = idx.span_range(source, method.span());
                            if matched_helpers.contains_key(&span) {
                                continue;
                            }
                        }
                        let mut fn_cfgs = impl_cfgs.clone();
                        fn_cfgs.extend(key::extract_cfg_tags(&method.attrs));
                        let k = match trait_path {
                            Some(tp) => key::trait_method_key(
                                &prefix,
                                &imp.self_ty,
                                tp,
                                &method.sig.ident,
                                &fn_cfgs,
                                false,
                            ),
                            None => key::inherent_method_key(
                                &prefix,
                                &imp.self_ty,
                                &method.sig.ident,
                                &fn_cfgs,
                            ),
                        };
                        *counts.entry(k).or_default() += 1;
                    }
                }
                _ => {}
            }
        }
    }

    for item in items {
        if let Item::Mod(item_mod) = item
            && !modules::has_cfg_test(&item_mod.attrs)
            && let Some((_, inner)) = &item_mod.content
        {
            let mut child_mod = mod_path.to_vec();
            child_mod.push(item_mod.ident.to_string());
            let mut child_cfgs = mod_cfg_tags.to_vec();
            child_cfgs.extend(key::extract_cfg_tags(&item_mod.attrs));
            collect_short_keys_in_items(
                crate_ident,
                targets,
                shallow,
                source,
                idx,
                &child_mod,
                &child_cfgs,
                inner,
                empty_colliding,
                counts,
            );
        }
    }
}

pub(crate) fn pair_converted_trait_helpers(
    source: &str,
    idx: &LineIndex,
    prefix: &str,
    mod_cfg_tags: &[String],
    colliding_short_keys: &BTreeSet<String>,
    items: &[Item],
) -> (
    BTreeMap<(usize, usize), String>,
    BTreeMap<(usize, usize), PairedHelper>,
) {
    type HelperKey = (String, Vec<String>, Vec<String>, String);
    let mut helpers: BTreeMap<HelperKey, HelperTarget> = BTreeMap::new();
    for item in items {
        let Item::Impl(imp) = item else {
            continue;
        };
        if imp.trait_.is_some() || modules::has_cfg_test(&imp.attrs) {
            continue;
        }
        let self_name = key::type_key_segment(&imp.self_ty);
        let impl_cfgs = key::extract_cfg_tags(&imp.attrs);
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
                let fn_cfgs = key::extract_cfg_tags(&method.attrs);
                let method_span = idx.span_range(source, method.span());
                helpers
                    .entry((self_name.clone(), impl_cfgs.clone(), fn_cfgs, name))
                    .or_insert(HelperTarget {
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
        let self_name = key::type_key_segment(&imp.self_ty);
        let impl_cfgs = key::extract_cfg_tags(&imp.attrs);
        for impl_item in &imp.items {
            let ImplItem::Fn(method) = impl_item else {
                continue;
            };
            if modules::has_cfg_test(&method.attrs) {
                continue;
            }
            let fn_cfgs = key::extract_cfg_tags(&method.attrs);
            let full_helper = key::trait_helper_name_full(trait_path, &method.sig.ident);
            let short_helper = key::trait_helper_name(trait_path, &method.sig.ident);
            let target = [full_helper, short_helper].into_iter().find_map(|cand| {
                edit::is_already_converted_trait_method(method, &cand)
                    .then(|| helpers.get(&(self_name.clone(), impl_cfgs.clone(), fn_cfgs.clone(), cand)))
                    .flatten()
            });
            if let Some(target) = target {
                let mut all_cfgs = mod_cfg_tags.to_vec();
                all_cfgs.extend(impl_cfgs.iter().cloned());
                all_cfgs.extend(fn_cfgs);
                let short_key = key::trait_method_key(
                    prefix,
                    &imp.self_ty,
                    trait_path,
                    &method.sig.ident,
                    &all_cfgs,
                    false,
                );
                let trait_key = if colliding_short_keys.contains(&short_key) {
                    key::trait_method_key(
                        prefix,
                        &imp.self_ty,
                        trait_path,
                        &method.sig.ident,
                        &all_cfgs,
                        true,
                    )
                } else {
                    short_key
                };
                let trait_method_span = idx.span_range(source, method.span());
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

fn check_no_test_id(
    found_attrs: &[attr::FoundHookAttr],
    file_path: &Path,
    line: usize,
) -> Result<(), String> {
    if let Some(tid) = attr::find_test_id(found_attrs) {
        return Err(format!(
            "{}:{line}: forbidden #[yoake_hook(test_id = {tid})] in scanned engine module (test_id belongs only to yoake-hook-* tests or #[cfg(test)])",
            file_path.display()
        ));
    }
    Ok(())
}

fn check_trait_eligibility(
    trait_meta: &eligible::TraitImplMeta,
    attrs: &[Attribute],
    sig: &Signature,
    block: &Block,
) -> Result<(), &'static str> {
    eligible::check_trait_method_movable(trait_meta, sig, block)?;
    eligible::check_fn(attrs, sig)?;
    eligible::check_trait_method_body_attrs(attrs)
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

/// `hook-ledger/stamp-allow.txt`: one fn key per line (`#` comments), stamped even when the body is small.
fn read_allow_small(ledger: &Path) -> BTreeSet<String> {
    std::fs::read_to_string(ledger.join("stamp-allow.txt"))
        .map(|text| {
            text.lines()
                .map(|l| l.split('#').next().unwrap_or("").trim())
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn assign_hook(ctx: &mut ScanCtx<'_>, fn_key: &str, sig: &str) -> Result<u32, String> {
    if ctx.tombstones.contains(fn_key) {
        return Err(format!(
            "cannot tombstone '{fn_key}': function is still present and eligible in source"
        ));
    }
    ctx.ledger_set.shard_mut(ctx.crate_ident).unexclude(fn_key);
    let assigned = ctx.ledger_set.assign(ctx.crate_ident, Kind::Fn, fn_key, sig);
    ctx.stats.hooked += 1;
    if assigned.new {
        ctx.stats.newly_assigned += 1;
    }
    if assigned.sig_changed {
        ctx.stats.sig_changes += 1;
    }
    Ok(assigned.id)
}

fn tombstone_live_if_requested(
    ctx: &mut ScanCtx<'_>,
    fn_key: &str,
    reason: &str,
) -> Result<(), String> {
    let shard = ctx.ledger_set.shard_mut(ctx.crate_ident);
    if shard.live_id(fn_key).is_some() {
        if ctx.tombstones.remove(fn_key) {
            shard
                .tombstone(fn_key)
                .map_err(|e| format!("failed to tombstone '{fn_key}': {e}"))?;
            ctx.stats.tombstoned += 1;
        } else {
            return Err(format!(
                "'{fn_key}' is live in ledger but now ineligible ({reason}); pass --tombstone {fn_key} to tombstone it"
            ));
        }
    }
    Ok(())
}

fn exclude_hook(ctx: &mut ScanCtx<'_>, fn_key: &str, reason: &str) -> Result<(), String> {
    tombstone_live_if_requested(ctx, fn_key, reason)?;
    ctx.ledger_set
        .shard_mut(ctx.crate_ident)
        .exclude_kind(Kind::Fn, fn_key, reason)
        .map_err(|e| format!("failed to exclude '{fn_key}': {e}"))?;
    *ctx.stats
        .excluded_by_reason
        .entry(reason.to_string())
        .or_default() += 1;
    Ok(())
}

fn exclude_manual_hook(
    ctx: &mut ScanCtx<'_>,
    fn_key: &str,
    attr_reason: Option<&str>,
) -> Result<(), String> {
    let existing_manual = ctx.manual_keys.get(fn_key).cloned().or_else(|| {
        ctx.ledger_set
            .shard(ctx.crate_ident)
            .and_then(|s| s.excluded().iter().find(|e| e.key == fn_key && e.manual))
            .map(|e| e.reason.clone())
    });
    let display_reason = existing_manual
        .as_deref()
        .or(attr_reason)
        .filter(|r| !r.trim().is_empty())
        .unwrap_or("manual skip")
        .to_string();

    tombstone_live_if_requested(ctx, fn_key, &display_reason)?;

    if existing_manual.is_none() {
        ctx.ledger_set
            .shard_mut(ctx.crate_ident)
            .exclude_manual(Kind::Fn, fn_key, &display_reason)
            .map_err(|e| format!("failed to record manual exclusion '{fn_key}': {e}"))?;
        ctx.manual_keys
            .insert(fn_key.to_string(), display_reason.clone());
    }

    *ctx.stats
        .excluded_by_reason
        .entry(display_reason)
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
        let tmp = ledger_io::TempDirGuard::new("unit_visitor").unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(tmp.path().join("src/lib.rs"), "pub mod entities;\n").unwrap();
        std::fs::write(tmp.path().join("src/entities.rs"), source).unwrap();

        let targets = vec![vec!["entities".to_string()]];
        let mut scanned_files =
            modules::collect_module_files(tmp.path(), "demo", &targets, false).unwrap();
        let crate_index =
            CrateIndex::build(tmp.path(), "demo", "demo", &scanned_files).unwrap();
        let file = &mut scanned_files[0];

        let idx = LineIndex::new(&file.source);
        let style = attr::detect_file_style(&file.parsed.items, &file.source);
        let colliding_short_keys = BTreeSet::new();
        let mut tombstones = BTreeSet::new();
        let mut manual_keys = BTreeMap::new();
        let mut ledger_set = LedgerSet::new();
        let mut seen_locations = BTreeMap::new();
        let mut seen_keys = BTreeSet::new();
        let mut stats = SummaryStats::default();
        let mut edits = Vec::new();
        let mut ctx = ScanCtx {
            crate_ident: "demo",
            targets: &targets,
            shallow: false,
            crate_index: &crate_index,
            colliding_short_keys: &colliding_short_keys,
            tombstones: &mut tombstones,
            manual_keys: &mut manual_keys,
            ledger_set: &mut ledger_set,
            seen_locations: &mut seen_locations,
            seen_keys: &mut seen_keys,
            stats: &mut stats,
            min_stmts: 0,
            allow_small: &BTreeSet::new(),
        };
        let mut visitor = FileVisitor {
            file_path: &file.path,
            source: &file.source,
            idx: &idx,
            mod_path: file.root_mod_path.clone(),
            mod_cfg_tags: file.root_cfg_tags.clone(),
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
        assert!(visitor.error.is_none(), "error: {:?}", visitor.error);

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
