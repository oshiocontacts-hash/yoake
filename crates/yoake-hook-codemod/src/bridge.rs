use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use quote::quote;
use syn::spanned::Spanned;
use syn::visit::Visit;
use syn::visit_mut::{self, VisitMut};
use syn::{
    Attribute, FnArg, GenericArgument, ImplItem, Item, Lifetime, LitStr, PathArguments,
    ReturnType, Signature, Type,
};
use yoake_hook_ledger::{Kind, LedgerSet};

use crate::attr;
use crate::edit::LineIndex;
use crate::types::CrateIndex;
use crate::{eligible, key, ledger_io, modules, run};

pub struct BridgeConfig {
    pub check: bool,
    pub crate_root: PathBuf,
    pub crate_name: String,
    pub ledger: PathBuf,
    pub modules: Vec<String>,
    pub shallow: bool,
    pub glues: Vec<PathBuf>,
    pub exclude_file: Option<PathBuf>,
}

struct ModuleOutput {
    path: PathBuf,
    content: String,
    bridged: usize,
    unbridged: usize,
}

pub fn execute(config: &BridgeConfig) -> Result<bool, String> {
    let crate_ident = key::normalize_crate_name(&config.crate_name);
    let targets = config
        .modules
        .iter()
        .map(|m| modules::normalize_module_arg(m))
        .collect::<Result<Vec<_>, _>>()?;

    let root_file = modules::find_crate_root_file(&config.crate_root, &config.crate_name)?;
    let root_dir = root_file
        .parent()
        .ok_or_else(|| format!("crate root {} has no parent directory", root_file.display()))?;
    let glue_out_dir = root_dir.join("yoake_glue");

    let exclude_path = config
        .exclude_file
        .clone()
        .unwrap_or_else(|| config.ledger.join("bridge-exclude.txt"));
    let excludes = load_bridge_excludes(&exclude_path)?;
    let ledger_set = ledger_io::load_ledger_set(&config.ledger)?;
    let crate_index = CrateIndex::build(&config.crate_root, &config.crate_name, &crate_ident, &[])?;
    let includes = collect_glue_includes(&config.crate_root, &config.crate_name)?;
    let field_tables = load_field_tables(&config.glues, &crate_ident, &includes)?;

    let mut outputs = Vec::with_capacity(targets.len());
    for target_mod in &targets {
        let scanned_files = modules::collect_module_files(
            &config.crate_root,
            &config.crate_name,
            std::slice::from_ref(target_mod),
            config.shallow,
        )?;
        let colliding_short_keys = run::collect_colliding_short_keys(
            &crate_ident,
            std::slice::from_ref(target_mod),
            config.shallow,
            &scanned_files,
        );
        let hooks = collect_module_hooks(
            &crate_ident,
            target_mod,
            config.shallow,
            &scanned_files,
            &colliding_short_keys,
            &ledger_set,
        )?;
        let copy_types = collect_copy_types(&scanned_files);
        COPY_TYPES.with(|c| *c.borrow_mut() = copy_types);
        let (content, bridged, unbridged) = render_bridge_module(
            &crate_ident,
            target_mod,
            &hooks,
            &crate_index,
            &field_tables,
            &excludes,
        )?;
        let file_name = format!("hook_bridge_{}.rs", target_mod.join("__"));
        outputs.push(ModuleOutput {
            path: glue_out_dir.join(file_name),
            content,
            bridged,
            unbridged,
        });
    }

    if config.check {
        let mut dirty = false;
        for out in &outputs {
            match std::fs::read_to_string(&out.path) {
                Ok(existing) if existing == out.content => {}
                Ok(_) => {
                    println!("check: stale bridge file {}", out.path.display());
                    dirty = true;
                }
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    println!("check: missing bridge file {}", out.path.display());
                    dirty = true;
                }
                Err(err) => {
                    return Err(format!("failed to read {}: {err}", out.path.display()));
                }
            }
        }
        if dirty {
            return Ok(false);
        }
        println!("check: clean ({} bridge file(s))", outputs.len());
        return Ok(true);
    }

    std::fs::create_dir_all(&glue_out_dir)
        .map_err(|e| format!("failed to create {}: {e}", glue_out_dir.display()))?;
    for out in &outputs {
        std::fs::write(&out.path, &out.content)
            .map_err(|e| format!("failed to write {}: {e}", out.path.display()))?;
        println!(
            "bridge: {} (bridged={}, unbridged={})",
            out.path.display(),
            out.bridged,
            out.unbridged
        );
    }
    Ok(true)
}

fn load_bridge_excludes(path: &Path) -> Result<BTreeMap<u32, String>, String> {
    if !path.is_file() {
        return Ok(BTreeMap::new());
    }
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let mut out = BTreeMap::new();
    for (line_idx, raw_line) in text.lines().enumerate() {
        let trimmed = raw_line.split('#').next().unwrap_or("").trim();
        if trimmed.is_empty() {
            continue;
        }
        let Some((id_str, reason_raw)) = trimmed.split_once(char::is_whitespace) else {
            return Err(format!(
                "{}:{}: expected '<id> <reason>', got '{trimmed}'",
                path.display(),
                line_idx + 1
            ));
        };
        let reason = reason_raw.trim();
        if reason.is_empty() {
            return Err(format!(
                "{}:{}: expected '<id> <reason>', got '{trimmed}'",
                path.display(),
                line_idx + 1
            ));
        }
        let id: u32 = id_str.parse().map_err(|_| {
            format!(
                "{}:{}: invalid hook id '{id_str}'",
                path.display(),
                line_idx + 1
            )
        })?;
        out.insert(id, reason.to_string());
    }
    Ok(out)
}

fn collect_glue_includes(
    crate_root: &Path,
    crate_name: &str,
) -> Result<Vec<(Vec<String>, String)>, String> {
    let mut includes = Vec::new();
    modules::walk_crate_tree(crate_root, crate_name, &[], |mod_path, _, items| {
        for item in items {
            if let Item::Macro(im) = item
                && im.mac.path.is_ident("include")
                && let Ok(lit) = syn::parse2::<LitStr>(im.mac.tokens.clone())
            {
                includes.push((mod_path.to_vec(), lit.value().replace('\\', "/")));
            }
        }
    })?;
    Ok(includes)
}

#[derive(Default)]
struct FieldTables {
    mut_tables: BTreeMap<String, String>,
    ref_tables: BTreeMap<String, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldTableKind {
    Mut,
    Ref,
}

fn load_field_tables(
    glues: &[PathBuf],
    crate_ident: &str,
    includes: &[(Vec<String>, String)],
) -> Result<FieldTables, String> {
    let mut tables = FieldTables::default();
    for glue_path in glues {
        let source = std::fs::read_to_string(glue_path)
            .map_err(|e| format!("failed to read glue {}: {e}", glue_path.display()))?;
        let parsed = syn::parse_file(&source)
            .map_err(|e| format!("failed to parse glue {}: {e}", glue_path.display()))?;
        let (header_crate, header_mod) = parse_glue_header(&source);
        let glue_crate = header_crate
            .or_else(|| find_crate_name_for_file(glue_path))
            .unwrap_or_else(|| crate_ident.to_string());

        let norm_glue = glue_path.to_string_lossy().replace('\\', "/");
        let file_name = glue_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let include_mod = if glue_crate == crate_ident {
            includes
                .iter()
                .find(|(_, inc)| {
                    norm_glue.ends_with(inc.as_str())
                        || (!file_name.is_empty() && inc.ends_with(&format!("/{file_name}")))
                        || inc == file_name
                })
                .map(|(m, _)| m.clone())
        } else {
            None
        };

        let mut submod = Vec::new();
        collect_glue_statics(
            &parsed.items,
            &glue_crate,
            crate_ident,
            header_mod.as_deref(),
            include_mod.as_deref(),
            glue_path,
            &mut submod,
            &mut tables,
        );
    }
    Ok(tables)
}

fn parse_glue_header(source: &str) -> (Option<String>, Option<String>) {
    for line in source.lines().take(10) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("// Module `")
            && let Some((mod_name, after_mod)) = rest.split_once("` of crate `")
            && let Some((crate_name, _)) = after_mod.split_once('`')
        {
            return (
                Some(key::normalize_crate_name(crate_name)),
                Some(mod_name.to_string()),
            );
        }
    }
    (None, None)
}

fn find_crate_name_for_file(file_path: &Path) -> Option<String> {
    let mut dir = file_path.parent()?;
    loop {
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file()
            && let Ok(text) = std::fs::read_to_string(&manifest)
        {
            let mut section = String::new();
            let mut pkg_name = None;
            let mut lib_name = None;
            for raw in text.lines() {
                let line = raw.split('#').next().unwrap_or("").trim();
                if line.starts_with('[') && line.ends_with(']') {
                    section = line.to_string();
                    continue;
                }
                if let Some((k, v)) = line.split_once('=') {
                    let k = k.trim();
                    let v = v.trim();
                    if v.len() >= 2
                        && ((v.starts_with('"') && v.ends_with('"'))
                            || (v.starts_with('\'') && v.ends_with('\'')))
                    {
                        let val = &v[1..v.len() - 1];
                        if section == "[package]" && k == "name" {
                            pkg_name = Some(val.to_string());
                        } else if section == "[lib]" && k == "name" {
                            lib_name = Some(val.to_string());
                        }
                    }
                }
            }
            if let Some(name) = lib_name.or(pkg_name) {
                return Some(key::normalize_crate_name(&name));
            }
        }
        dir = dir.parent()?;
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_glue_statics(
    items: &[Item],
    glue_crate: &str,
    target_crate: &str,
    header_mod: Option<&str>,
    include_mod: Option<&[String]>,
    glue_path: &Path,
    submod: &mut Vec<String>,
    tables: &mut FieldTables,
) {
    for item in items {
        match item {
            Item::Mod(item_mod) => {
                if let Some((_, inner)) = &item_mod.content {
                    submod.push(item_mod.ident.to_string());
                    collect_glue_statics(
                        inner,
                        glue_crate,
                        target_crate,
                        header_mod,
                        include_mod,
                        glue_path,
                        submod,
                        tables,
                    );
                    submod.pop();
                }
            }
            Item::Static(item_static) => {
                let Some((kind, inner_ty)) = extract_field_table_inner_ty(&item_static.ty) else {
                    continue;
                };
                let Some((canon_ty, type_mod)) = resolve_glue_inner_ty(
                    inner_ty,
                    glue_crate,
                    header_mod,
                    include_mod,
                    glue_path,
                ) else {
                    continue;
                };
                let host_mod: Vec<String> = if let Some(im) = include_mod {
                    im.to_vec()
                } else if let Some(hm) = header_mod {
                    hm.split("::").map(str::to_string).collect()
                } else if !type_mod.is_empty() {
                    type_mod
                } else if let Some(stem) = glue_path.file_stem().and_then(|s| s.to_str()) {
                    vec![stem.to_string()]
                } else {
                    Vec::new()
                };
                let mut parts = Vec::new();
                if glue_crate == target_crate {
                    parts.push("crate".to_string());
                } else {
                    parts.push(glue_crate.to_string());
                }
                parts.extend(host_mod);
                parts.extend(submod.iter().cloned());
                parts.push(item_static.ident.to_string());
                let target = match kind {
                    FieldTableKind::Mut => &mut tables.mut_tables,
                    FieldTableKind::Ref => &mut tables.ref_tables,
                };
                target.insert(canon_ty, parts.join("::"));
            }
            _ => {}
        }
    }
}

fn extract_field_table_inner_ty(ty: &Type) -> Option<(FieldTableKind, &Type)> {
    let mut cur = ty;
    loop {
        match cur {
            Type::Group(g) => cur = &g.elem,
            Type::Paren(p) => cur = &p.elem,
            _ => break,
        }
    }
    let Type::Path(tp) = cur else {
        return None;
    };
    let last = tp.path.segments.last()?;
    let kind = if last.ident == "FieldTable" {
        FieldTableKind::Mut
    } else if last.ident == "RefFieldTable" {
        FieldTableKind::Ref
    } else {
        return None;
    };
    let PathArguments::AngleBracketed(ab) = &last.arguments else {
        return None;
    };
    if ab.args.len() != 1 {
        return None;
    }
    match ab.args.first()? {
        GenericArgument::Type(inner) => Some((kind, inner)),
        _ => None,
    }
}

fn resolve_glue_inner_ty(
    mut ty: &Type,
    glue_crate: &str,
    header_mod: Option<&str>,
    include_mod: Option<&[String]>,
    glue_path: &Path,
) -> Option<(String, Vec<String>)> {
    loop {
        match ty {
            Type::Group(g) => ty = &g.elem,
            Type::Paren(p) => ty = &p.elem,
            _ => break,
        }
    }
    let Type::Path(tp) = ty else {
        return None;
    };
    if tp.qself.is_some() {
        return None;
    }
    let segs: Vec<String> = tp.path.segments.iter().map(|s| s.ident.to_string()).collect();
    if segs.is_empty() {
        return None;
    }
    if segs[0] == "crate" && segs.len() >= 2 {
        let type_mod = segs[1..segs.len() - 1].to_vec();
        let canon = format!("{glue_crate}::{}", segs[1..].join("::"));
        return Some((canon, type_mod));
    }
    if segs.len() == 1 {
        let mod_segs: Vec<String> = if let Some(im) = include_mod {
            im.to_vec()
        } else if let Some(hm) = header_mod {
            hm.split("::").map(str::to_string).collect()
        } else if let Some(stem) = glue_path.file_stem().and_then(|s| s.to_str()) {
            vec![stem.to_string()]
        } else {
            Vec::new()
        };
        let canon = if mod_segs.is_empty() {
            format!("{glue_crate}::{}", segs[0])
        } else {
            format!("{glue_crate}::{}::{}", mod_segs.join("::"), segs[0])
        };
        return Some((canon, mod_segs));
    }
    Some((segs.join("::"), Vec::new()))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HookKind {
    Method,
    FreeFn,
    Callsite,
}

struct CollectedHook {
    id: u32,
    kind: HookKind,
    fn_ident: syn::Ident,
    sig: Signature,
    self_ty: Option<Type>,
    hook_self_ty: Option<Type>,
    mod_path: Vec<String>,
    rel_mod: Vec<String>,
    cfg_attrs: Vec<String>,
}

fn collect_cfg_attrs_from_slice(
    attrs: &[Attribute],
    source: &str,
    idx: &LineIndex,
) -> Vec<String> {
    let mut out = Vec::new();
    for attr in attrs {
        if attr.path().is_ident("cfg") {
            let (s, e) = idx.span_range(source, attr.span());
            out.push(source[s..e].to_string());
        }
    }
    out
}

fn collect_module_hooks(
    crate_ident: &str,
    target_mod: &[String],
    shallow: bool,
    scanned_files: &[modules::ScannedFile],
    colliding_short_keys: &BTreeSet<String>,
    ledger_set: &LedgerSet,
) -> Result<Vec<CollectedHook>, String> {
    let mut live_fn_by_key: BTreeMap<String, u32> = BTreeMap::new();
    let mut live_cs_by_id: BTreeSet<u32> = BTreeSet::new();
    if let Some(shard) = ledger_set.shard(crate_ident) {
        for entry in shard.live() {
            match entry.kind {
                Kind::Fn => {
                    live_fn_by_key.insert(entry.key.clone(), entry.id);
                }
                Kind::Callsite => {
                    live_cs_by_id.insert(entry.id);
                }
                Kind::Store => {}
            }
        }
    }

    let mut hooks = Vec::new();
    for file in scanned_files {
        let idx = LineIndex::new(&file.source);
        let root_cfg_attrs: Vec<String> = file
            .root_cfg_tags
            .iter()
            .filter_map(|tag| tag.strip_prefix('[').and_then(|s| s.strip_suffix(']')))
            .map(|inner| format!("#[{inner}]"))
            .collect();
        let mut collector = HookCollector {
            crate_ident,
            target_mod,
            shallow,
            source: &file.source,
            idx: &idx,
            colliding_short_keys,
            live_fn_by_key: &live_fn_by_key,
            live_cs_by_id: &live_cs_by_id,
            mod_path: file.root_mod_path.clone(),
            mod_cfg_tags: file.root_cfg_tags.clone(),
            mod_cfg_attrs: root_cfg_attrs,
            hooks: &mut hooks,
        };
        collector.visit_items(&file.parsed.items);
    }

    hooks.sort_by_key(|h| h.id);
    Ok(hooks)
}

struct HookCollector<'a> {
    crate_ident: &'a str,
    target_mod: &'a [String],
    shallow: bool,
    source: &'a str,
    idx: &'a LineIndex,
    colliding_short_keys: &'a BTreeSet<String>,
    live_fn_by_key: &'a BTreeMap<String, u32>,
    live_cs_by_id: &'a BTreeSet<u32>,
    mod_path: Vec<String>,
    mod_cfg_tags: Vec<String>,
    mod_cfg_attrs: Vec<String>,
    hooks: &'a mut Vec<CollectedHook>,
}

impl HookCollector<'_> {
    fn visit_items(&mut self, items: &[Item]) {
        let in_target = if self.shallow {
            self.mod_path.as_slice() == self.target_mod
        } else {
            self.mod_path.starts_with(self.target_mod)
        };
        let prefix = key::module_prefix(self.crate_ident, &self.mod_path);
        let (matched_helpers, _) = if in_target {
            run::pair_converted_trait_helpers(
                self.source,
                self.idx,
                &prefix,
                &self.mod_cfg_tags,
                self.colliding_short_keys,
                items,
            )
        } else {
            (BTreeMap::new(), BTreeMap::new())
        };

        for item in items {
            match item {
                Item::Mod(item_mod) => {
                    if modules::has_cfg_test(&item_mod.attrs) {
                        continue;
                    }
                    if let Some((_, inner)) = &item_mod.content {
                        let prev_path = self.mod_path.clone();
                        let prev_tags = self.mod_cfg_tags.clone();
                        let prev_attrs = self.mod_cfg_attrs.clone();
                        self.mod_path.push(item_mod.ident.to_string());
                        self.mod_cfg_tags.extend(key::extract_cfg_tags(&item_mod.attrs));
                        self.mod_cfg_attrs.extend(collect_cfg_attrs_from_slice(
                            &item_mod.attrs,
                            self.source,
                            self.idx,
                        ));
                        self.visit_items(inner);
                        self.mod_path = prev_path;
                        self.mod_cfg_tags = prev_tags;
                        self.mod_cfg_attrs = prev_attrs;
                    }
                }
                Item::Fn(item_fn) if in_target && !modules::has_cfg_test(&item_fn.attrs) => {
                    let found_attrs = attr::find_hook_attrs(&item_fn.attrs, self.source, self.idx);
                    let Some(active_attr) = found_attrs.iter().find(|a| a.parsed.skip.is_none())
                    else {
                        continue;
                    };
                    let ident_str = item_fn.sig.ident.to_string();
                    let is_cs = ident_str.starts_with("__yoake_cs_")
                        || active_attr.parsed.kind.as_deref() == Some("callsite");
                    let rel_mod = self.mod_path[self.target_mod.len()..].to_vec();
                    let mut cfg_attrs = self.mod_cfg_attrs.clone();
                    cfg_attrs.extend(collect_cfg_attrs_from_slice(
                        &item_fn.attrs,
                        self.source,
                        self.idx,
                    ));
                    if is_cs {
                        let cs_id = ident_str
                            .strip_prefix("__yoake_cs_")
                            .and_then(|s| s.parse::<u32>().ok())
                            .or(active_attr.parsed.id);
                        if let Some(id) = cs_id
                            && self.live_cs_by_id.contains(&id)
                        {
                            self.hooks.push(CollectedHook {
                                id,
                                kind: HookKind::Callsite,
                                fn_ident: item_fn.sig.ident.clone(),
                                sig: item_fn.sig.clone(),
                                self_ty: None,
                                hook_self_ty: None,
                                mod_path: self.mod_path.clone(),
                                rel_mod,
                                cfg_attrs,
                            });
                        }
                    } else {
                        let mut fn_cfg_tags = self.mod_cfg_tags.clone();
                        fn_cfg_tags.extend(key::extract_cfg_tags(&item_fn.attrs));
                        let fn_key =
                            key::free_fn_key(&prefix, &item_fn.sig.ident, &fn_cfg_tags);
                        if let Some(&id) = self.live_fn_by_key.get(&fn_key) {
                            self.hooks.push(CollectedHook {
                                id,
                                kind: HookKind::FreeFn,
                                fn_ident: item_fn.sig.ident.clone(),
                                sig: item_fn.sig.clone(),
                                self_ty: None,
                                hook_self_ty: None,
                                mod_path: self.mod_path.clone(),
                                rel_mod,
                                cfg_attrs,
                            });
                        }
                    }
                }
                Item::Impl(item_impl)
                    if in_target
                        && item_impl.trait_.is_none()
                        && !modules::has_cfg_test(&item_impl.attrs) =>
                {
                    let mut impl_cfg_tags = self.mod_cfg_tags.clone();
                    impl_cfg_tags.extend(key::extract_cfg_tags(&item_impl.attrs));
                    let mut impl_cfg_attrs = self.mod_cfg_attrs.clone();
                    impl_cfg_attrs.extend(collect_cfg_attrs_from_slice(
                        &item_impl.attrs,
                        self.source,
                        self.idx,
                    ));
                    let rel_mod = self.mod_path[self.target_mod.len()..].to_vec();
                    for impl_item in &item_impl.items {
                        let ImplItem::Fn(method) = impl_item else {
                            continue;
                        };
                        if modules::has_cfg_test(&method.attrs) {
                            continue;
                        }
                        let found_attrs =
                            attr::find_hook_attrs(&method.attrs, self.source, self.idx);
                        let Some(active_attr) =
                            found_attrs.iter().find(|a| a.parsed.skip.is_none())
                        else {
                            continue;
                        };
                        let method_span = self.idx.span_range(self.source, method.span());
                        let fn_key = if let Some(trait_key) = matched_helpers.get(&method_span) {
                            trait_key.clone()
                        } else {
                            let mut fn_cfg_tags = impl_cfg_tags.clone();
                            fn_cfg_tags.extend(key::extract_cfg_tags(&method.attrs));
                            key::inherent_method_key(
                                &prefix,
                                &item_impl.self_ty,
                                &method.sig.ident,
                                &fn_cfg_tags,
                            )
                        };
                        if let Some(&id) = self.live_fn_by_key.get(&fn_key) {
                            let mut cfg_attrs = impl_cfg_attrs.clone();
                            cfg_attrs.extend(collect_cfg_attrs_from_slice(
                                &method.attrs,
                                self.source,
                                self.idx,
                            ));
                            let hook_self_ty = active_attr
                                .parsed
                                .self_ty
                                .as_deref()
                                .and_then(|s| syn::parse_str::<Type>(s).ok());
                            self.hooks.push(CollectedHook {
                                id,
                                kind: HookKind::Method,
                                fn_ident: method.sig.ident.clone(),
                                sig: method.sig.clone(),
                                self_ty: Some((*item_impl.self_ty).clone()),
                                hook_self_ty,
                                mod_path: self.mod_path.clone(),
                                rel_mod: rel_mod.clone(),
                                cfg_attrs,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[derive(Clone)]
enum DispatchTarget {
    FieldTable(String),
    RefFieldTable(String),
    Root,
}

#[derive(Clone)]
enum RetKind {
    Unit,
    Bool,
    Int(String),
    F32,
    F64,
    ObserveOnly,
}

#[derive(Clone, Copy)]
enum ParamPass {
    ReborrowMut,
    /// Known to be `Copy` (primitives, shared references, types the bridged module derives `Copy` for, and tuples,
    /// arrays and `Option`s of those): passed to every `original` call.
    Copy,
    /// By value and not known to be `Copy`: moved into the first `original` call (the dispatcher keeps it in an
    /// `Option` and takes it), so the real original runs at most once; a later `original` returns `0`.
    ByValue,
}

thread_local! {
    /// The idents of the types with `#[derive(.., Copy, ..)]` in the module being bridged (set per target in `run`).
    static COPY_TYPES: std::cell::RefCell<BTreeSet<String>> = const { std::cell::RefCell::new(BTreeSet::new()) };
}

/// The types the scanned files derive `Copy` for (structs and enums, inline modules included).
fn collect_copy_types(files: &[modules::ScannedFile]) -> BTreeSet<String> {
    fn visit(items: &[syn::Item], out: &mut BTreeSet<String>) {
        for item in items {
            let (attrs, ident) = match item {
                syn::Item::Struct(s) => (&s.attrs, &s.ident),
                syn::Item::Enum(e) => (&e.attrs, &e.ident),
                syn::Item::Mod(m) => {
                    if let Some((_, inner)) = &m.content {
                        visit(inner, out);
                    }
                    continue;
                }
                _ => continue,
            };
            let derives_copy = attrs.iter().any(|a| {
                a.path().is_ident("derive")
                    && quote::ToTokens::to_token_stream(a)
                        .to_string()
                        .split(|c: char| !c.is_alphanumeric() && c != '_')
                        .any(|word| word == "Copy")
            });
            if derives_copy {
                out.insert(ident.to_string());
            }
        }
    }
    let mut out = BTreeSet::new();
    for file in files {
        visit(&file.parsed.items, &mut out);
    }
    out
}

/// Whether a by-value parameter type is certainly `Copy` (see [`ParamPass::Copy`]).
fn is_known_copy(ty: &Type) -> bool {
    const PRIMITIVES: &[&str] = &[
        "bool", "char", "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f32",
        "f64",
    ];
    match ty {
        Type::Paren(p) => is_known_copy(&p.elem),
        Type::Group(g) => is_known_copy(&g.elem),
        Type::Reference(r) => r.mutability.is_none(),
        Type::Tuple(t) => t.elems.iter().all(is_known_copy),
        Type::Array(a) => is_known_copy(&a.elem),
        Type::Path(p) if p.qself.is_none() => {
            let Some(last) = p.path.segments.last() else { return false };
            let name = last.ident.to_string();
            match &last.arguments {
                syn::PathArguments::None => {
                    PRIMITIVES.contains(&name.as_str()) || COPY_TYPES.with(|c| c.borrow().contains(&name))
                }
                syn::PathArguments::AngleBracketed(args) if name == "Option" => args
                    .args
                    .iter()
                    .all(|a| matches!(a, syn::GenericArgument::Type(t) if is_known_copy(t))),
                _ => false,
            }
        }
        _ => false,
    }
}

struct BridgedHook {
    id: u32,
    kind: HookKind,
    fn_ident: syn::Ident,
    rel_mod: Vec<String>,
    cfg_attrs: Vec<String>,
    shared_lifetimes: Vec<Lifetime>,
    dyn_lifetimes: Vec<Lifetime>,
    dyn_arg_types: Vec<Type>,
    dyn_ret_type: Option<Type>,
    param_types: Vec<Type>,
    param_passes: Vec<ParamPass>,
    ret_type: Option<Type>,
    ret_kind: RetKind,
    dispatch: DispatchTarget,
    reg_self_ty: Option<Type>,
}

struct ReplaceSelf<'a>(&'a Type);

impl VisitMut for ReplaceSelf<'_> {
    fn visit_type_mut(&mut self, ty: &mut Type) {
        if matches!(ty, Type::Path(p) if p.qself.is_none() && p.path.is_ident("Self")) {
            *ty = self.0.clone();
        } else {
            visit_mut::visit_type_mut(self, ty);
        }
    }
}

#[derive(Default)]
struct NamedLifetimeCounts {
    counts: BTreeMap<String, usize>,
    order: Vec<String>,
}

impl<'ast> Visit<'ast> for NamedLifetimeCounts {
    fn visit_lifetime(&mut self, lifetime: &'ast Lifetime) {
        let name = lifetime.ident.to_string();
        if name != "static" && name != "_" {
            let entry = self.counts.entry(name.clone()).or_insert(0);
            if *entry == 0 {
                self.order.push(name);
            }
            *entry += 1;
        }
    }
}

struct BridgeLifetimeNormalizer {
    shared: BTreeSet<String>,
    rename: BTreeMap<String, Lifetime>,
    elide_all: bool,
}

impl VisitMut for BridgeLifetimeNormalizer {
    fn visit_lifetime_mut(&mut self, lifetime: &mut Lifetime) {
        let name = lifetime.ident.to_string();
        if name == "static" || name == "_" {
            return;
        }
        if !self.elide_all && self.shared.contains(&name) {
            if let Some(mapped) = self.rename.get(&name) {
                *lifetime = mapped.clone();
            }
        } else {
            *lifetime = Lifetime::new("'_", lifetime.span());
        }
    }

    fn visit_type_reference_mut(&mut self, reference: &mut syn::TypeReference) {
        if let Some(lt) = &reference.lifetime {
            let name = lt.ident.to_string();
            if name != "static" {
                if !self.elide_all && self.shared.contains(&name) {
                    if let Some(mapped) = self.rename.get(&name) {
                        reference.lifetime = Some(mapped.clone());
                    }
                } else {
                    reference.lifetime = None;
                }
            }
        }
        visit_mut::visit_type_mut(self, &mut reference.elem);
        if let Type::TraitObject(object) = &mut *reference.elem {
            object.bounds = std::mem::take(&mut object.bounds)
                .into_iter()
                .filter(|bound| {
                    !matches!(bound, syn::TypeParamBound::Lifetime(lt) if lt.ident != "static")
                })
                .collect();
        }
    }
}

fn unwrap_type_parens(mut ty: &Type) -> &Type {
    loop {
        match ty {
            Type::Group(g) => ty = &g.elem,
            Type::Paren(p) => ty = &p.elem,
            _ => return ty,
        }
    }
}

fn format_type_tokens(ty: &Type) -> String {
    let raw = quote!(#ty).to_string();
    clean_rust_tokens(&raw)
}

fn clean_rust_tokens(raw: &str) -> String {
    let mut s = raw
        .replace(" :: ", "::")
        .replace(" ,", ",")
        .replace(" ;", ";")
        .replace("& mut ", "&mut ")
        .replace("& '", "&'")
        .replace("' _", "'_")
        .replace("< ", "<")
        .replace(" <", "<")
        .replace("( ", "(")
        .replace(" )", ")")
        .replace("[ ", "[")
        .replace(" ]", "]");
    while s.contains(" >") {
        s = s.replace(" >", ">");
    }
    s.replace("& ", "&")
}

#[derive(Default)]
struct OpaqueOrNeverFinder {
    found: bool,
}

impl<'ast> Visit<'ast> for OpaqueOrNeverFinder {
    fn visit_type_impl_trait(&mut self, _: &'ast syn::TypeImplTrait) {
        self.found = true;
    }

    fn visit_type_never(&mut self, _: &'ast syn::TypeNever) {
        self.found = true;
    }
}

fn type_has_impl_trait_or_never(ty: &Type) -> bool {
    let mut finder = OpaqueOrNeverFinder::default();
    finder.visit_type(ty);
    finder.found
}

fn classify_ret(output: &ReturnType) -> (Option<Type>, RetKind) {
    let ty = match output {
        ReturnType::Default => return (None, RetKind::Unit),
        ReturnType::Type(_, ty) => ty.as_ref(),
    };
    let inner = unwrap_type_parens(ty);
    if let Type::Tuple(t) = inner
        && t.elems.is_empty()
    {
        return (None, RetKind::Unit);
    }
    if let Type::Path(tp) = inner
        && tp.qself.is_none()
        && tp.path.leading_colon.is_none()
        && tp.path.segments.len() == 1
        && tp.path.segments[0].arguments.is_empty()
    {
        let name = tp.path.segments[0].ident.to_string();
        let kind = match name.as_str() {
            "bool" => RetKind::Bool,
            "i8" | "i16" | "i32" | "i64" | "isize" | "u8" | "u16" | "u32" | "u64" | "usize" => {
                RetKind::Int(name)
            }
            "f32" => RetKind::F32,
            "f64" => RetKind::F64,
            _ => RetKind::ObserveOnly,
        };
        return (Some(ty.clone()), kind);
    }
    (Some(ty.clone()), RetKind::ObserveOnly)
}

fn lookup_field_table(canon: &str, field_tables: &BTreeMap<String, String>) -> Option<String> {
    if let Some(path) = field_tables.get(canon) {
        return Some(path.clone());
    }
    for prefix in ["horizon_client::", "horizon_client_lib::", "horizon_client_render::"] {
        if let Some(rest) = canon.strip_prefix(prefix) {
            if let Some(path) = field_tables.get(&format!("horizon_items::{rest}")) {
                return Some(path.clone());
            }
            if let Some(path) = field_tables.get(&format!("horizon_client_lib::{rest}")) {
                return Some(path.clone());
            }
        }
    }
    None
}

fn classify_hook(
    crate_ident: &str,
    target_mod: &[String],
    hook: &CollectedHook,
    crate_index: &CrateIndex,
    field_tables: &FieldTables,
) -> Result<BridgedHook, String> {
    if hook.sig.generics.type_params().next().is_some()
        || hook.sig.generics.const_params().next().is_some()
    {
        return Err("generic parameters".to_string());
    }
    if hook.sig.inputs.is_empty() {
        return Err("no parameters".to_string());
    }

    let effective_self_ty = hook.hook_self_ty.as_ref().or(hook.self_ty.as_ref());
    let replace_self = |ty: &Type| -> Type {
        let mut out = ty.clone();
        if let Some(self_ty) = effective_self_ty {
            ReplaceSelf(self_ty).visit_type_mut(&mut out);
        }
        out
    };

    let mut raw_param_types = Vec::with_capacity(hook.sig.inputs.len());
    let (arg0_inner, arg0_is_mut, arg0_is_receiver) = match &hook.sig.inputs[0] {
        FnArg::Receiver(r) => {
            if r.reference.is_none() {
                return Err("by-value self".to_string());
            }
            let self_ty = effective_self_ty.ok_or_else(|| "missing impl self type".to_string())?;
            raw_param_types.push(replace_self(&r.ty));
            (self_ty.clone(), r.mutability.is_some(), true)
        }
        FnArg::Typed(pt) => {
            let replaced = replace_self(&pt.ty);
            let inner = unwrap_type_parens(&replaced);
            let Type::Reference(r) = inner else {
                return Err("arg0 is not &T or &mut T".to_string());
            };
            let elem = r.elem.as_ref().clone();
            let mutable = r.mutability.is_some();
            raw_param_types.push(replaced);
            (elem, mutable, false)
        }
    };

    let canon = crate_index.resolve_bridge_type_canon(&hook.mod_path, &arg0_inner);
    let root_canon = format!("{crate_ident}::game::Game");
    let dispatch = if canon.as_deref() == Some(root_canon.as_str()) {
        if arg0_is_mut {
            DispatchTarget::Root
        } else {
            return Err("arg0 is &Game: no read-only root dispatch".to_string());
        }
    } else if eligible::type_has_non_static_lifetime(&arg0_inner) {
        return Err("arg0 type has non-static lifetime".to_string());
    } else if arg0_is_mut {
        if let Some(c) = &canon
            && let Some(ft_path) = lookup_field_table(c, &field_tables.mut_tables)
        {
            DispatchTarget::FieldTable(ft_path)
        } else {
            let label = canon.unwrap_or_else(|| format_type_tokens(&arg0_inner));
            return Err(format!("no FieldTable for `{label}`"));
        }
    } else if let Some(c) = &canon
        && let Some(ft_path) = lookup_field_table(c, &field_tables.ref_tables)
    {
        DispatchTarget::RefFieldTable(ft_path)
    } else {
        let label = canon.unwrap_or_else(|| format_type_tokens(&arg0_inner));
        if arg0_is_receiver {
            return Err(format!("arg0 is &self, no RefFieldTable for `{label}`"));
        } else {
            return Err(format!("arg0 is &T, no RefFieldTable for `{label}`"));
        }
    };

    let (raw_ret_type, ret_kind) = classify_ret(&hook.sig.output);
    let replaced_ret_type = raw_ret_type.map(|rt| replace_self(&rt));
    if let Some(rt) = &replaced_ret_type {
        if type_has_impl_trait_or_never(rt) {
            return Err(format!("unsupported return type `{}`", format_type_tokens(rt)));
        }
        if matches!(ret_kind, RetKind::ObserveOnly) && eligible::type_has_non_static_lifetime(rt) {
            return Err(format!(
                "return type `{}` has non-static lifetime",
                format_type_tokens(rt)
            ));
        }
    }

    let mut param_passes = Vec::with_capacity(hook.sig.inputs.len().saturating_sub(1));
    for arg in hook.sig.inputs.iter().skip(1) {
        let FnArg::Typed(pt) = arg else {
            return Err("unexpected receiver argument".to_string());
        };
        let replaced = replace_self(&pt.ty);
        let pass = match unwrap_type_parens(&replaced) {
            Type::Reference(r) if r.mutability.is_some() => ParamPass::ReborrowMut,
            Type::Reference(_) => ParamPass::Copy,
            other if is_known_copy(other) => ParamPass::Copy,
            _ => ParamPass::ByValue,
        };
        param_passes.push(pass);
        raw_param_types.push(replaced);
    }

    let mut counts = NamedLifetimeCounts::default();
    for ty in &raw_param_types {
        counts.visit_type(ty);
    }
    if let Some(rt) = &replaced_ret_type {
        counts.visit_type(rt);
    }
    let shared_names: BTreeSet<String> = counts
        .counts
        .iter()
        .filter_map(|(k, &v)| (v >= 2).then_some(k.clone()))
        .collect();
    let shared_lifetimes: Vec<Lifetime> = counts
        .order
        .iter()
        .filter(|name| shared_names.contains(*name))
        .map(|name| Lifetime::new(&format!("'{name}"), proc_macro2::Span::call_site()))
        .collect();
    let dyn_lifetimes: Vec<Lifetime> = (0..shared_lifetimes.len())
        .map(|i| Lifetime::new(&format!("'__yoake_dyn_{i}"), proc_macro2::Span::call_site()))
        .collect();
    let dyn_rename: BTreeMap<String, Lifetime> = counts
        .order
        .iter()
        .filter(|name| shared_names.contains(*name))
        .zip(dyn_lifetimes.iter().cloned())
        .map(|(n, lt)| (n.clone(), lt))
        .collect();

    let mut param_norm = BridgeLifetimeNormalizer {
        shared: shared_names.clone(),
        rename: BTreeMap::new(),
        elide_all: false,
    };
    let mut dyn_norm = BridgeLifetimeNormalizer {
        shared: shared_names,
        rename: dyn_rename,
        elide_all: false,
    };
    let mut param_types = Vec::with_capacity(raw_param_types.len());
    let mut dyn_arg_types = Vec::with_capacity(raw_param_types.len());
    for ty in &raw_param_types {
        let mut pty = ty.clone();
        param_norm.visit_type_mut(&mut pty);
        param_types.push(crate_index.rewrite_type_for_bridge(&hook.mod_path, target_mod, &pty));

        let mut dty = ty.clone();
        dyn_norm.visit_type_mut(&mut dty);
        dyn_arg_types.push(crate_index.rewrite_type_for_bridge(&hook.mod_path, target_mod, &dty));
    }

    let (ret_type, dyn_ret_type) = if let Some(rt) = &replaced_ret_type {
        let mut prt = rt.clone();
        param_norm.visit_type_mut(&mut prt);
        let rewritten_p = crate_index.rewrite_type_for_bridge(&hook.mod_path, target_mod, &prt);

        let mut drt = rt.clone();
        dyn_norm.visit_type_mut(&mut drt);
        let rewritten_d = crate_index.rewrite_type_for_bridge(&hook.mod_path, target_mod, &drt);

        (Some(rewritten_p), Some(rewritten_d))
    } else {
        (None, None)
    };

    let reg_self_ty = if hook.kind == HookKind::Method {
        effective_self_ty.map(|st| {
            let mut sty = st.clone();
            let mut elide = BridgeLifetimeNormalizer {
                shared: BTreeSet::new(),
                rename: BTreeMap::new(),
                elide_all: true,
            };
            elide.visit_type_mut(&mut sty);
            crate_index.rewrite_type_for_bridge(&hook.mod_path, target_mod, &sty)
        })
    } else {
        None
    };

    Ok(BridgedHook {
        id: hook.id,
        kind: hook.kind,
        fn_ident: hook.fn_ident.clone(),
        rel_mod: hook.rel_mod.clone(),
        cfg_attrs: hook.cfg_attrs.clone(),
        shared_lifetimes,
        dyn_lifetimes,
        dyn_arg_types,
        dyn_ret_type,
        param_types,
        param_passes,
        ret_type,
        ret_kind,
        dispatch,
        reg_self_ty,
    })
}

fn render_bridge_module(
    crate_ident: &str,
    target_mod: &[String],
    hooks: &[CollectedHook],
    crate_index: &CrateIndex,
    field_tables: &FieldTables,
    excludes: &BTreeMap<u32, String>,
) -> Result<(String, usize, usize), String> {
    let mut bridged = Vec::new();
    let mut unbridged: Vec<(u32, String)> = Vec::new();

    for hook in hooks {
        if let Some(reason) = excludes.get(&hook.id) {
            unbridged.push((hook.id, reason.clone()));
            continue;
        }
        match classify_hook(crate_ident, target_mod, hook, crate_index, field_tables) {
            Ok(bh) => bridged.push(bh),
            Err(reason) => unbridged.push((hook.id, reason)),
        }
    }

    let mut out = String::new();
    out.push_str("// @generated by yoake-hook-codemod bridge. DO NOT EDIT.\n\n");
    out.push_str(
        "/// Generated S2 -> `yoake-slice` bridge dispatchers and installer.\n\
         ///\n\
         /// - `BRIDGED`: sorted list of every hook id this module can install.\n\
         /// - `install`: installs dispatchers for `(id, priority)` pairs at most once per\n\
         ///   process (`yoake_slice::bridge::install_once`) and returns a\n\
         ///   `yoake_slice::bridge::InstallReport` separating `installed`, `failed`,\n\
         ///   `not_bridgeable` (`UNBRIDGED`), and `not_mine` (ids of other modules,\n\
         ///   unknown ids, or `>= yoake_slice::runtime::SYNTHETIC_HOOK_BASE`).\n\
         /// - `mode`: returns `Some(Mode::Full)` or `Some(Mode::ObserveOnly)` for ids in\n\
         ///   `BRIDGED`, and `None` otherwise.\n\
         ///\n\
         /// Hooks whose `arg0` is `&T` or `&self` (when `T` has a `RefFieldTable`) use\n\
         /// `yoake_slice::runtime::dispatch_id_ref` with `REF_FIELDS_*`. On that handle the\n\
         /// guest can call `get` and `original`, while `set` and `engine_call` (which\n\
         /// require `&mut T`) are refused with `ReadOnly` (see\n\
         /// `docs/yoake2/ENGINE_CALL_ABI.md`, \"Read-only hook arguments\").\n\
         ///\n\
         /// Hooks returning `()`, `bool`, primitive integers, `f32`, or `f64` round-trip their\n\
         /// result through `i64`. When a guest returns an out-of-range value for `bool`, an\n\
         /// integer other than `i64`/`u64`, or `f32`, `yoake_slice::bridge::note_out_of_range`\n\
         /// records it and the original's result is used (running `original` if needed).\n\
         ///\n\
         /// Hooks listed in `OBSERVE_ONLY` have a result type `R` that does not fit an `i64`:\n\
         /// the guest sees the hook and its `arg0` handle and may call `original`, but cannot\n\
         /// replace the result. The real original runs **exactly once** per call: the first\n\
         /// `original` that reaches it runs it, later ones do not run it again; if no guest's\n\
         /// `original` reaches it, it runs once after the guests return, so the hooked fn cannot\n\
         /// be cancelled in this mode.\n\
         ///\n\
         /// Hooks whose shape cannot be bridged are listed in `UNBRIDGED`.\n",
    );
    out.push_str(
        "#[allow(\n\
         \x20   dead_code,\n\
         \x20   unused_mut,\n\
         \x20   unused_variables,\n\
         \x20   unused_parens,\n\
         \x20   unused_imports,\n\
         \x20   non_snake_case,\n\
         \x20   clippy::all\n\
         )]\n",
    );
    out.push_str("pub mod hook_bridge {\n");
    out.push_str("    use super::*;\n");

    for bh in &bridged {
        out.push('\n');
        out.push_str(&render_bridged_fn(bh));
    }

    out.push_str("\n    pub const BRIDGED: &[u32] = &[\n");
    for bh in &bridged {
        out.push_str(&format!("        {},\n", bh.id));
    }
    out.push_str("    ];\n\n");

    out.push_str("    pub fn mode(id: u32) -> Option<yoake_slice::bridge::Mode> {\n");
    out.push_str("        if BRIDGED.binary_search(&id).is_err() {\n");
    out.push_str("            return None;\n");
    out.push_str("        }\n");
    out.push_str("        if OBSERVE_ONLY.binary_search(&id).is_ok() {\n");
    out.push_str("            Some(yoake_slice::bridge::Mode::ObserveOnly)\n");
    out.push_str("        } else {\n");
    out.push_str("            Some(yoake_slice::bridge::Mode::Full)\n");
    out.push_str("        }\n");
    out.push_str("    }\n\n");

    out.push_str(
        "    pub fn install(registered: &[(u32, i32)]) -> yoake_slice::bridge::InstallReport {\n",
    );
    out.push_str("        let mut report = yoake_slice::bridge::InstallReport::default();\n");
    out.push_str("        for &(id, priority) in registered {\n");
    out.push_str("            if id >= yoake_slice::runtime::SYNTHETIC_HOOK_BASE {\n");
    out.push_str("                report.not_mine.push(id);\n");
    out.push_str("                continue;\n");
    out.push_str("            }\n");
    out.push_str("            let order = yoake_hook_rt::Order {\n");
    out.push_str("                priority,\n");
    out.push_str("                mod_load_order: 0,\n");
    out.push_str("            };\n");
    out.push_str("            let outcome: ::core::option::Option<::core::result::Result<(), &'static str>> = match id {\n");
    for bh in &bridged {
        out.push_str(&render_install_arm(bh)?);
    }
    out.push_str("                _ => None,\n");
    out.push_str("            };\n");
    out.push_str("            match outcome {\n");
    out.push_str("                Some(Ok(())) => report.installed.push(id),\n");
    out.push_str("                Some(Err(err)) => report.failed.push((id, err)),\n");
    out.push_str("                None => {\n");
    out.push_str(
        "                    if let Some(&entry) = UNBRIDGED.iter().find(|&&(u_id, _)| u_id == id) {\n",
    );
    out.push_str("                        report.not_bridgeable.push(entry);\n");
    out.push_str("                    } else {\n");
    out.push_str("                        report.not_mine.push(id);\n");
    out.push_str("                    }\n");
    out.push_str("                }\n");
    out.push_str("            }\n");
    out.push_str("        }\n");
    out.push_str("        report\n");
    out.push_str("    }\n\n");

    let observe_only_ids: Vec<u32> = bridged
        .iter()
        .filter(|bh| matches!(bh.ret_kind, RetKind::ObserveOnly))
        .map(|bh| bh.id)
        .collect();
    out.push_str("    pub const OBSERVE_ONLY: &[u32] = &[\n");
    for id in &observe_only_ids {
        out.push_str(&format!("        {id},\n"));
    }
    out.push_str("    ];\n\n");

    out.push_str("    pub const UNBRIDGED: &[(u32, &str)] = &[\n");
    for (id, reason) in &unbridged {
        out.push_str(&format!("        ({id}, {reason:?}),\n"));
    }
    out.push_str("    ];\n");
    out.push_str("}\n");

    Ok((out, bridged.len(), unbridged.len()))
}

fn render_bridged_fn(bh: &BridgedHook) -> String {
    let mut out = String::new();
    for cfg in &bh.cfg_attrs {
        out.push_str(&format!("    {cfg}\n"));
    }

    let generics = if bh.shared_lifetimes.is_empty() {
        String::new()
    } else {
        let lts: Vec<String> = bh
            .shared_lifetimes
            .iter()
            .map(|lt| format!("'{ident}", ident = lt.ident))
            .collect();
        format!("<{}>", lts.join(", "))
    };
    let dyn_for = if bh.dyn_lifetimes.is_empty() {
        String::new()
    } else {
        let lts: Vec<String> = bh
            .dyn_lifetimes
            .iter()
            .map(|lt| format!("'{ident}", ident = lt.ident))
            .collect();
        format!("for<{}> ", lts.join(", "))
    };
    let dyn_params: Vec<String> = bh.dyn_arg_types.iter().map(format_type_tokens).collect();
    let dyn_ret_sig = bh
        .dyn_ret_type
        .as_ref()
        .map(|ty| format!(" -> {}", format_type_tokens(ty)))
        .unwrap_or_default();
    let ret_sig = bh
        .ret_type
        .as_ref()
        .map(|ty| format!(" -> {}", format_type_tokens(ty)))
        .unwrap_or_default();

    out.push_str(&format!("    fn __bridge_{}{generics}(\n", bh.id));
    out.push_str(&format!(
        "        original: &mut dyn {dyn_for}FnMut({}){dyn_ret_sig},\n",
        dyn_params.join(", ")
    ));
    for (idx, pty) in bh.param_types.iter().enumerate() {
        out.push_str(&format!("        p{idx}: {},\n", format_type_tokens(pty)));
    }
    out.push_str(&format!("    ){ret_sig} {{\n"));

    let mut call_args = vec!["p0".to_string()];
    let mut fallback_args = vec!["p0".to_string()];
    let mut moved = Vec::new();
    for (idx, pass) in bh.param_passes.iter().enumerate() {
        let p = idx + 1;
        match pass {
            ParamPass::ReborrowMut => {
                call_args.push(format!("&mut *p{p}"));
                fallback_args.push(format!("&mut *p{p}"));
            }
            ParamPass::Copy => {
                call_args.push(format!("p{p}"));
                fallback_args.push(format!("p{p}"));
            }
            ParamPass::ByValue => {
                call_args.push(format!("p{p}_v"));
                fallback_args.push(format!("p{p}.take().expect(\"only an original call moves it, and none ran\")"));
                moved.push(p);
            }
        }
    }
    let orig_call = format!("original({})", call_args.join(", "));
    // the direct run when no guest's `original` reached the real original (nothing was moved yet)
    let fallback_call = format!("original({})", fallback_args.join(", "));
    // a by-value argument that may not be `Copy` goes into the first `original` call; a later one returns 0
    let takes: String = moved
        .iter()
        .map(|p| format!("let ::core::option::Option::Some(p{p}_v) = p{p}.take() else {{ return 0; }}; "))
        .collect();
    let range_checked: Option<(&str, &str, String)> = match &bh.ret_kind {
        RetKind::Bool => Some((
            "bool",
            "yoake_slice::bridge::encode::bool(r)",
            "yoake_slice::bridge::decode::bool(value)".to_string(),
        )),
        RetKind::Int(name) if name == "usize" => Some((
            "usize",
            "yoake_slice::bridge::encode::usize(r)",
            "yoake_slice::bridge::decode::int::<usize>(value)".to_string(),
        )),
        RetKind::Int(name) if name == "isize" => Some((
            "isize",
            "r as i64",
            "yoake_slice::bridge::decode::int::<isize>(value)".to_string(),
        )),
        RetKind::Int(name) if name != "i64" && name != "u64" => Some((
            name.as_str(),
            "yoake_slice::bridge::encode::int(r)",
            format!("yoake_slice::bridge::decode::int::<{name}>(value)"),
        )),
        RetKind::F32 => Some((
            "f32",
            "yoake_slice::bridge::encode::f32(r)",
            "yoake_slice::bridge::decode::f32(value)".to_string(),
        )),
        _ => None,
    };
    let closure_body = if let Some((_, encode_expr, _)) = &range_checked {
        format!("{{ let r = {orig_call}; last = Some(r); {encode_expr} }}")
    } else {
        match &bh.ret_kind {
            RetKind::Unit => format!("{{ {orig_call}; 0 }}"),
            RetKind::Int(name) if name == "i64" => orig_call.clone(),
            RetKind::Int(name) if name == "u64" => {
                format!("yoake_slice::bridge::encode::u64({orig_call})")
            }
            RetKind::F64 => format!("yoake_slice::bridge::encode::f64({orig_call})"),
            // observe-only: the real original runs at most once per call; a second `original` from the guest
            // gets nothing new (the result is kept for the engine, the guest cannot see it anyway)
            RetKind::ObserveOnly => {
                format!("{{ if result.is_none() {{ result = Some({orig_call}); }} 0 }}")
            }
            _ => unreachable!("range-checked kinds handled above"),
        }
    };
    let closure_body = if takes.is_empty() { closure_body } else { format!("{{ {takes}{closure_body} }}") };
    let dispatch_call = match &bh.dispatch {
        DispatchTarget::FieldTable(ft_path) => format!(
            "yoake_slice::runtime::dispatch_id({}, p0, &{ft_path}, &mut |p0| {closure_body})",
            bh.id
        ),
        DispatchTarget::RefFieldTable(ft_path) => format!(
            "yoake_slice::runtime::dispatch_id_ref({}, p0, &{ft_path}, &mut |p0| {closure_body})",
            bh.id
        ),
        DispatchTarget::Root => format!(
            "yoake_slice::runtime::dispatch_root_id({}, p0, &mut |p0| {closure_body})",
            bh.id
        ),
    };
    for p in &moved {
        out.push_str(&format!("        let mut p{p} = ::core::option::Option::Some(p{p});\n"));
    }
    if let Some((ret_ty_str, encode_expr, decode_expr)) = range_checked {
        out.push_str(&format!("        let mut last: Option<{ret_ty_str}> = None;\n"));
        out.push_str(&format!("        let value = {dispatch_call};\n"));
        out.push_str(&format!("        match {decode_expr} {{\n"));
        out.push_str("            Some(r) => r,\n");
        out.push_str("            None => match last {\n");
        out.push_str(
            "                // the guest passed the original's own result through (e.g. a usize above i64::MAX): not out of range\n",
        );
        out.push_str(&format!("                Some(r) if {encode_expr} == value => r,\n"));
        out.push_str("                _ => {\n");
        out.push_str(&format!(
            "                    yoake_slice::bridge::note_out_of_range({}, value);\n",
            bh.id
        ));
        out.push_str(
            "                    // the last original result if the original ran, else run it now (the rest of the chain is skipped)\n",
        );
        out.push_str(&format!(
            "                    match last {{ Some(r) => r, None => {fallback_call} }}\n"
        ));
        out.push_str("                }\n");
        out.push_str("            },\n");
        out.push_str("        }\n");
    } else {
        match &bh.ret_kind {
            RetKind::Unit => out.push_str(&format!("        let _ = {dispatch_call};\n")),
            RetKind::Int(name) if name == "i64" => {
                out.push_str(&format!("        {dispatch_call}\n"))
            }
            RetKind::Int(name) if name == "u64" => {
                out.push_str(&format!(
                    "        yoake_slice::bridge::decode::u64({dispatch_call})\n"
                ));
            }
            RetKind::F64 => {
                out.push_str(&format!(
                    "        yoake_slice::bridge::decode::f64({dispatch_call})\n"
                ));
            }
            RetKind::ObserveOnly => {
                let ret_ty = bh
                    .ret_type
                    .as_ref()
                    .map(format_type_tokens)
                    .unwrap_or_else(|| "()".to_string());
                out.push_str(&format!("        let mut result: Option<{ret_ty}> = None;\n"));
                out.push_str(&format!("        let _ = {dispatch_call};\n"));
                out.push_str("        match result {\n");
                out.push_str("            Some(value) => value,\n");
                out.push_str(
                    "            // The guest did not call `original`: the result cannot be produced by the guest, so the original runs.\n",
                );
                out.push_str(&format!("            None => {fallback_call},\n"));
                out.push_str("        }\n");
            }
            _ => unreachable!("range-checked kinds handled above"),
        }
    }
    out.push_str("    }\n");
    out
}

fn render_install_arm(bh: &BridgedHook) -> Result<String, String> {
    let mut out = String::new();
    for cfg in &bh.cfg_attrs {
        out.push_str(&format!("                {cfg}\n"));
    }
    match bh.kind {
        HookKind::Method => {
            let self_ty = bh
                .reg_self_ty
                .as_ref()
                .ok_or_else(|| format!("missing self_ty for method hook {}", bh.id))?;
            let reg_path = format_method_register_path(self_ty, &bh.fn_ident);
            out.push_str(&format!(
                "                {} => Some(yoake_slice::bridge::install_once(id, || {reg_path}(__bridge_{}, order))),\n",
                bh.id, bh.id
            ));
        }
        HookKind::FreeFn | HookKind::Callsite => {
            let mod_prefix = if bh.rel_mod.is_empty() {
                "super".to_string()
            } else {
                format!("super::{}", bh.rel_mod.join("::"))
            };
            let alias = format!("{mod_prefix}::__YoakeHook_{}", bh.fn_ident);
            let reg_fn = format!("{mod_prefix}::__yoake_register_{}", bh.fn_ident);
            out.push_str(&format!(
                "                {} => Some(yoake_slice::bridge::install_once(id, || {{\n",
                bh.id
            ));
            out.push_str(&format!(
                "                    let cb: {alias} = __bridge_{};\n",
                bh.id
            ));
            out.push_str(&format!("                    {reg_fn}(cb, order)\n"));
            out.push_str("                })),\n");
        }
    }
    Ok(out)
}

fn format_method_register_path(ty: &Type, fn_ident: &syn::Ident) -> String {
    let inner = unwrap_type_parens(ty);
    if let Type::Path(tp) = inner
        && tp.qself.is_none()
    {
        let only_lifetimes = tp.path.segments.iter().all(|seg| match &seg.arguments {
            PathArguments::None => true,
            PathArguments::AngleBracketed(ab) => ab
                .args
                .iter()
                .all(|a| matches!(a, GenericArgument::Lifetime(_))),
            PathArguments::Parenthesized(_) => false,
        });
        if only_lifetimes {
            let segs: Vec<String> = tp
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            let prefix = if tp.path.leading_colon.is_some() {
                "::"
            } else {
                ""
            };
            return format!("{prefix}{}::__yoake_register_{fn_ident}", segs.join("::"));
        }
    }
    format!(
        "<{}>::__yoake_register_{fn_ident}",
        format_type_tokens(inner)
    )
}
