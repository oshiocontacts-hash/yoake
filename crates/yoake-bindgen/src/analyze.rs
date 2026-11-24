//! Classifying the functions and fields of one module.

use serde_json::Value;

use crate::doc::{ItemRef, Res};
use crate::shape::{Analyzer, DefBody, Fail, SelfTy, Shape, hard, soft};

/// What a function turned out to be.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cat {
    /// Bound now; every type is a scalar, string or fixed array (the old category (a)).
    BoundA,
    /// Bound now with a mechanical rule: `usize`, `Option`, `Vec`, tuples, plain structs and enums.
    BoundMech,
    /// Needs a handle the runtime does not have yet (a non-data argument, a returned reference).
    HandleRequired,
    /// Cannot be bound with this design: generics, closures, `dyn`, `unsafe`, `async`, lifetimes.
    Excluded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArgMode {
    /// Passed by value.
    Value,
    /// Decoded, passed as `&v`.
    Ref,
    /// Decoded, passed as `&mut v`, encoded back after the call.
    RefMut,
    /// `Option<&T>`: decoded as `Option<T>`, passed as `a.as_ref()`.
    OptRef,
    /// `Option<&str>` / `Option<&[T]>`: decoded as `Option<String>` / `Option<Vec<T>>`, passed as `a.as_deref()`.
    OptDeref,
}

#[derive(Clone, Debug)]
pub struct ArgPlan {
    pub mode: ArgMode,
    pub shape: Shape,
}

#[derive(Clone, Debug)]
pub enum RetPlan {
    Unit,
    Value(Shape),
    /// The function returns `&T`; the glue encodes the referent.
    Copy(Shape),
}

#[derive(Clone, Debug)]
pub enum RecvPlan {
    None,
    /// `&self` / `&mut self` on a handle type (reached as `Root::HookArg`, or by an accessor path).
    Handle { type_path: String, mutable: bool },
    /// `&self` / `&mut self` on the `Game` root.
    Game { type_path: String, mutable: bool },
    /// `self` by value on a data type: the receiver is the first decoded value.
    Value(Shape),
}

#[derive(Clone, Debug)]
pub struct Plan {
    pub recv: RecvPlan,
    pub args: Vec<ArgPlan>,
    pub ret: RetPlan,
    /// `<crate::entities::ItemEntity>::bounds`, `super::free_fn`.
    pub call: String,
}

#[derive(Clone, Debug)]
pub struct FnReport {
    /// Stable lock key, `entities::ItemEntity::bounds`.
    pub path: String,
    pub name: String,
    /// A text form of the signature; a change here is a changed binding.
    pub sig: String,
    pub cat: Cat,
    pub reasons: Vec<String>,
    pub plan: Option<Plan>,
    pub derived: bool,
    pub receiver: String,
    pub line: u64,
    /// The type of a handle receiver (`&self` / `&mut self` on a struct or enum).
    pub recv_ref: Option<ItemRef>,
}

#[derive(Clone, Debug)]
pub struct FieldReport {
    pub owner_path: String,
    pub owner_label: String,
    pub field: String,
    pub index: usize,
    pub tuple: bool,
    pub shape: Option<Shape>,
    pub reason: Option<String>,
    pub owner_ref: ItemRef,
    /// A guest may write it (listed `writable`); otherwise the setter id is reserved as `ReadOnly`.
    pub writable: bool,
    /// The field's type as written in the source (stable text for the lock).
    pub ty_text: String,
    /// The field has a `#[cfg(..)]` (rustdoc keeps it as `CfgTrace`): it exists only in some builds, so the field tables
    /// (which are generated once, from one configuration) leave it out.
    pub cfg_gated: bool,
}

pub struct ModuleScan {
    pub module: u64,
    pub fns: Vec<FnReport>,
    pub fields: Vec<FieldReport>,
    /// Receiver types of bound handle methods (rust paths), sorted.
    pub handle_types: Vec<String>,
    /// The same handle types as items, with their rust paths.
    pub handle_refs: Vec<(ItemRef, String)>,
}

/// The module whose file is `source` (a path suffix) in document `doc`.
pub fn find_module(an: &Analyzer<'_>, doc: usize, source: &str) -> Option<u64> {
    let wanted = source.replace('\\', "/");
    let d = &an.uni.docs[doc];
    let index = d.v.get("index")?.as_object()?;
    let mut found = None;
    for (id, item) in index {
        if item.pointer("/inner/module").is_none() {
            continue;
        }
        let Some(file) = item.pointer("/span/filename").and_then(Value::as_str) else { continue };
        let begin = item.pointer("/span/begin").and_then(Value::as_array);
        let at_start = begin.is_some_and(|b| b.first().and_then(Value::as_u64) == Some(1));
        if at_start && file.replace('\\', "/").ends_with(&wanted) {
            found = id.parse().ok();
            break;
        }
    }
    found
}

pub fn scan_module(an: &mut Analyzer<'_>, doc: usize, module: u64, label: &str) -> ModuleScan {
    let uni = an.uni;
    let d = &uni.docs[doc];
    let mut scan = ModuleScan { module, fns: Vec::new(), fields: Vec::new(), handle_types: Vec::new(), handle_refs: Vec::new() };
    let items: Vec<u64> = d
        .item(module)
        .and_then(|m| m.pointer("/inner/module/items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_u64)
        .collect();
    for id in items {
        match d.kind(id) {
            Some("function") => {
                let report = analyze_fn(an, doc, id, None, label);
                scan.fns.push(report);
            }
            Some("struct") => scan_fields(an, doc, id, label, &mut scan.fields),
            _ => {}
        }
    }
    // Impl blocks are not listed in a module's `items`: take the ones written in the module's file
    // and not inside an inline child module.
    let file = d.item(module).and_then(|m| m.pointer("/span/filename")).and_then(Value::as_str).unwrap_or("").to_string();
    let mut inline_modules: Vec<(u64, u64)> = Vec::new();
    let mut impls: Vec<(u64, u64)> = Vec::new();
    if let Some(index) = d.v.get("index").and_then(Value::as_object) {
        for (key, item) in index {
            if item.pointer("/span/filename").and_then(Value::as_str) != Some(file.as_str()) {
                continue;
            }
            let begin = item.pointer("/span/begin/0").and_then(Value::as_u64).unwrap_or(0);
            let end = item.pointer("/span/end/0").and_then(Value::as_u64).unwrap_or(0);
            if item.pointer("/inner/module").is_some() && !(begin == 1 && item.pointer("/span/begin/1").and_then(Value::as_u64) == Some(1)) {
                inline_modules.push((begin, end));
            } else if item.pointer("/inner/impl").is_some() {
                if let Ok(id) = key.parse() {
                    impls.push((begin, id));
                }
            }
        }
    }
    impls.sort();
    for (begin, id) in impls {
        if inline_modules.iter().any(|(b, e)| begin >= *b && begin <= *e) {
            continue;
        }
        let item = d.item(id).unwrap();
        let Some(imp) = item.pointer("/inner/impl") else { continue };
        let synthetic = imp.get("is_synthetic").and_then(Value::as_bool) == Some(true) || imp.get("blanket_impl").is_some_and(|b| !b.is_null());
        if synthetic {
            continue;
        }
        let derived = has_attr(item, "automatically_derived");
        let children: Vec<u64> = imp.get("items").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
        for child in children {
            if d.kind(child) != Some("function") {
                continue;
            }
            let mut report = analyze_fn(an, doc, child, Some(id), label);
            report.derived = derived;
            scan.fns.push(report);
        }
    }
    // Duplicate lock keys (overloads through different impl blocks) get a counter.
    let mut seen = std::collections::HashMap::<String, usize>::new();
    for report in &mut scan.fns {
        let n = seen.entry(report.path.clone()).or_insert(0);
        *n += 1;
        if *n > 1 {
            report.path = format!("{}#{}", report.path, n);
        }
    }
    let mut handles: Vec<String> = scan
        .fns
        .iter()
        .filter(|r| matches!(r.cat, Cat::BoundA | Cat::BoundMech))
        .filter_map(|r| match r.plan.as_ref().map(|p| &p.recv) {
            Some(RecvPlan::Handle { type_path, .. }) => Some(type_path.clone()),
            _ => None,
        })
        .collect();
    for field in &scan.fields {
        if field.shape.is_some() {
            handles.push(field.owner_path.clone());
        }
    }
    handles.sort();
    handles.dedup();
    let mut refs: Vec<(ItemRef, String)> = Vec::new();
    for report in scan.fns.iter().filter(|r| matches!(r.cat, Cat::BoundA | Cat::BoundMech)) {
        if let (Some(r), Some(RecvPlan::Handle { type_path, .. })) = (report.recv_ref, report.plan.as_ref().map(|p| &p.recv)) {
            refs.push((r, type_path.clone()));
        }
    }
    for field in scan.fields.iter().filter(|f| f.shape.is_some()) {
        refs.push((field.owner_ref, field.owner_path.clone()));
    }
    refs.sort_by(|a, b| a.1.cmp(&b.1));
    refs.dedup();
    scan.handle_types = handles;
    scan.handle_refs = refs;
    scan
}

fn scan_fields(an: &mut Analyzer<'_>, doc: usize, id: u64, label: &str, out: &mut Vec<FieldReport>) {
    let uni = an.uni;
    let d = &uni.docs[doc];
    let item = d.item(id).unwrap();
    let name = item.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
    let Some(definition) = item.pointer("/inner/struct") else { return };
    let owner_path = an.rust_path(ItemRef { doc, id });
    let owner_label = format!("{label}::{name}");
    let params = definition.pointer("/generics/params").and_then(Value::as_array).cloned().unwrap_or_default();
    let ids: Vec<(String, Option<u64>)> = match definition.get("kind") {
        Some(kind) if kind.get("plain").is_some() => kind
            .pointer("/plain/fields")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .map(|f| (d.item_name(f).unwrap_or("?").to_string(), Some(f)))
            .collect(),
        Some(kind) if kind.get("tuple").is_some() => kind
            .get("tuple")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
            .map(|(i, f)| (i.to_string(), f.as_u64()))
            .collect(),
        _ => Vec::new(),
    };
    let tuple = definition.pointer("/kind/tuple").is_some();
    for (index, (field_name, field_id)) in ids.into_iter().enumerate() {
        let mut report = FieldReport {
            owner_path: owner_path.clone(),
            owner_label: owner_label.clone(),
            field: field_name,
            index,
            tuple,
            shape: None,
            reason: None,
            owner_ref: ItemRef { doc, id },
            writable: false,
            ty_text: String::new(),
            cfg_gated: false,
        };
        report.writable = an.target.writable.get(&name).is_some_and(|list| list.contains(&report.field));
        if is_frozen(an, doc, id, &name) {
            report.reason = Some(format!("`{name}` is frozen (no accessors, no bound methods)"));
        } else if !params.is_empty() {
            let lifetime = params.iter().any(|p| p.pointer("/kind/lifetime").is_some());
            report.reason = Some(if lifetime { format!("`{name}` carries a lifetime (cannot be a handle type)") } else { format!("`{name}` is generic") });
        } else if let Some(field) = field_id.and_then(|f| d.item(f)) {
            report.cfg_gated = field.get("attrs").is_some_and(|a| a.to_string().contains("CfgTrace"));
            match field.pointer("/inner/struct_field") {
                Some(ty) => match {
                    report.ty_text = crate::display::type_display(ty);
                    an.shape(doc, ty, None)
                } {
                    Ok(shape) => report.shape = Some(shape),
                    Err(fail) => report.reason = Some(fail.reason),
                },
                None => report.reason = Some("field without a type".into()),
            }
        } else {
            report.reason = Some("hidden field".into());
        }
        out.push(report);
    }
}

/// A type is frozen when it is listed or is defined in a file that contains `unsafe`.
fn is_frozen(an: &Analyzer<'_>, doc: usize, id: u64, name: &str) -> bool {
    if an.target.frozen_types.iter().any(|t| t == name) {
        return true;
    }
    let file = an.uni.docs[doc].item(id).and_then(|i| i.pointer("/span/filename")).and_then(Value::as_str).unwrap_or("").replace('\\', "/");
    an.target.frozen_files.iter().any(|f| file.ends_with(&f.replace('\\', "/")))
}

fn has_attr(item: &Value, wanted: &str) -> bool {
    item.get("attrs")
        .and_then(Value::as_array)
        .is_some_and(|attrs| attrs.iter().any(|attr| attr.as_str().is_some_and(|text| text.contains(wanted))))
}

fn leaf(path: &str) -> &str {
    path.rsplit("::").next().unwrap_or(path)
}

fn type_label(ty: &Value) -> String {
    if let Some(path) = ty.get("resolved_path") {
        return leaf(path.get("path").and_then(Value::as_str).unwrap_or("?")).to_string();
    }
    crate::display::type_display(ty)
}

fn analyze_fn(an: &mut Analyzer<'_>, doc: usize, id: u64, impl_id: Option<u64>, label: &str) -> FnReport {
    let uni = an.uni;
    let d = &uni.docs[doc];
    let item = d.item(id).unwrap();
    let name = item.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
    let function = item.pointer("/inner/function").unwrap();
    let line = item.pointer("/span/begin/0").and_then(Value::as_u64).unwrap_or(0);
    let imp = impl_id.and_then(|i| d.item(i)).and_then(|i| i.pointer("/inner/impl"));
    let mut fails: Vec<Fail> = Vec::new();

    // the hook machinery (yoake-hook-macro registrations and originals, codemod call-site shims, trait-method hook
    // wrappers) is never a binding: a guest calling it would bypass or re-enter the hooks
    if name.starts_with("__yoake_") {
        fails.push(hard("hook machinery (`__yoake_*`): not an engine function"));
    }

    // header
    let header = function.get("header");
    if header.and_then(|h| h.get("is_unsafe")).and_then(Value::as_bool) == Some(true) {
        fails.push(hard("unsafe functions require an explicit safety contract"));
    }
    if header.and_then(|h| h.get("is_async")).and_then(Value::as_bool) == Some(true) {
        fails.push(hard("async functions are not supported by the synchronous ABI"));
    }
    if function.pointer("/sig/is_c_variadic").and_then(Value::as_bool) == Some(true) {
        fails.push(hard("C-variadic functions are not mappable"));
    }
    // generics of the function and of its impl: only lifetimes are harmless
    for (where_, generics) in [("function", function.get("generics")), ("impl", imp.and_then(|i| i.get("generics")))] {
        for param in generics.and_then(|g| g.get("params")).and_then(Value::as_array).into_iter().flatten() {
            let pname = param.get("name").and_then(Value::as_str).unwrap_or("?");
            if param.pointer("/kind/lifetime").is_some() {
                continue;
            }
            if param.pointer("/kind/type/is_synthetic").and_then(Value::as_bool) == Some(true) {
                let fnlike = param
                    .pointer("/kind/type/bounds")
                    .and_then(Value::as_array)
                    .is_some_and(|bounds| bounds.iter().any(|b| b.pointer("/trait_bound/trait/path").and_then(Value::as_str).is_some_and(|p| p.starts_with("Fn"))));
                fails.push(hard(if fnlike {
                    "closure argument (`impl Fn*`): guest callback handles are not built".to_string()
                } else {
                    "`impl Trait` argument requires a listed instantiation".to_string()
                }));
            } else if param.pointer("/kind/const").is_some() {
                fails.push(hard(format!("{where_}-level const parameter `{pname}`")));
            } else {
                fails.push(hard(format!("{where_}-level type parameter `{pname}` requires a listed instantiation")));
            }
        }
    }

    // the impl's Self type
    let for_ty = imp.and_then(|i| i.get("for"));
    let mut self_def: Option<ItemRef> = None;
    let mut self_label = String::new();
    let mut is_root = false;
    if let Some(for_ty) = for_ty {
        self_label = type_label(for_ty);
        match for_ty.pointer("/resolved_path/id").and_then(Value::as_u64).map(|i| uni.resolve(doc, i)) {
            Some(Res::Local(r)) if matches!(uni.doc(r).kind(r.id), Some("struct" | "enum")) => {
                self_def = Some(r);
                if is_frozen(an, r.doc, r.id, uni.doc(r).item_name(r.id).unwrap_or("")) {
                    fails.push(hard("receiver type is frozen (listed, or defined in a file that contains unsafe)"));
                }
                let local_name = uni.doc(r).item_name(r.id).unwrap_or("");
                is_root = uni.doc(r).name == an.target.crate_name && an.target.roots.iter().any(|root| root == local_name);
            }
            Some(Res::Local(_)) => fails.push(soft("impl on a type that is not a struct or enum")),
            _ => fails.push(hard("impl on a type that is not defined in a loaded engine crate")),
        }
    }
    let mut trait_path: Option<String> = None;
    let mut trait_label = String::new();
    if let Some(trait_) = imp.and_then(|i| i.get("trait")).filter(|t| !t.is_null()) {
        let has_args = trait_.pointer("/args/angle_bracketed/args").and_then(Value::as_array).is_some_and(|a| !a.is_empty());
        trait_label = leaf(trait_.get("path").and_then(Value::as_str).unwrap_or("?")).to_string();
        if has_args {
            fails.push(hard(format!("generic trait impl `{trait_label}<..>`")));
        }
        match trait_.get("id").and_then(Value::as_u64).map(|i| uni.resolve(doc, i)) {
            Some(Res::Local(r)) => trait_path = Some(an.rust_path(r)),
            Some(Res::Std(path) | Res::Missing(path)) => trait_path = Some(path.join("::")),
            None => fails.push(hard("trait impl without a trait id")),
        }
    }
    let sty: Option<SelfTy<'_>> = for_ty.map(|ty| SelfTy { doc, ty });

    // lock key
    let key = match (&for_ty, &trait_path) {
        (None, _) => format!("{label}::{name}"),
        (Some(_), None) => format!("{label}::{self_label}::{name}"),
        (Some(_), Some(_)) => format!("{label}::<{self_label} as {trait_label}>::{name}"),
    };

    // receiver
    let inputs: Vec<&Value> = function.pointer("/sig/inputs").and_then(Value::as_array).map(|a| a.iter().collect()).unwrap_or_default();
    let has_self = inputs.first().is_some_and(|input| input.get(0).and_then(Value::as_str) == Some("self"));
    let mut receiver = "-".to_string();
    let mut recv_plan = RecvPlan::None;
    let mut skip = 0;
    if has_self {
        skip = 1;
        let ty = inputs[0].get(1).unwrap_or(&Value::Null);
        let by_ref = ty.get("borrowed_ref");
        let self_generic = |t: &Value| t.get("generic").and_then(Value::as_str) == Some("Self");
        if let Some(r) = by_ref.filter(|r| r.get("type").is_some_and(self_generic)) {
            let mutable = r.get("is_mutable").and_then(Value::as_bool) == Some(true);
            receiver = if mutable { "&mut self" } else { "&self" }.to_string();
            match self_def {
                Some(def) => {
                    let type_path = an.rust_path(def);
                    if is_root {
                        recv_plan = RecvPlan::Game { type_path, mutable };
                    } else {
                        let generics = uni.item(def).and_then(|i| i.pointer("/inner/struct/generics").or_else(|| i.pointer("/inner/enum/generics")));
                        let params = generics.and_then(|g| g.get("params")).and_then(Value::as_array).cloned().unwrap_or_default();
                        if params.iter().any(|p| p.pointer("/kind/lifetime").is_some()) {
                            fails.push(hard("receiver type carries a lifetime: it cannot be a hook-argument handle"));
                        } else if !params.is_empty() {
                            fails.push(hard("receiver type is generic: bind a concrete instantiation first"));
                        } else {
                            recv_plan = RecvPlan::Handle { type_path, mutable };
                        }
                    }
                }
                None => {
                    if for_ty.is_none() {
                        fails.push(hard("`self` receiver outside an impl"));
                    }
                }
            }
        } else if self_generic(ty) {
            receiver = "self".to_string();
            match (for_ty, self_def) {
                (Some(for_ty), Some(_)) if !is_root => match an.shape(doc, for_ty, None) {
                    Ok(shape) => recv_plan = RecvPlan::Value(shape),
                    Err(fail) => fails.push(soft(format!("{} [by-value self]", fail.reason))),
                },
                _ => fails.push(soft("by-value `self` needs a consume rule")),
            }
        } else {
            receiver = "other self".to_string();
            fails.push(hard("explicit receiver type (`self: Box<Self>` and similar) is not mappable"));
        }
    }

    // arguments and result
    let mut args = Vec::new();
    for input in inputs.iter().skip(skip) {
        let arg_ty = input.get(1).unwrap_or(&Value::Null);
        match arg_plan(an, doc, arg_ty, sty) {
            Ok(plan) => args.push(plan),
            Err(fail) => fails.push(fail),
        }
    }
    let output = function.pointer("/sig/output").filter(|o| !o.is_null());
    let ret = match output {
        None => Some(RetPlan::Unit),
        Some(out) => match ret_plan(an, doc, out, sty) {
            Ok(plan) => Some(plan),
            Err(fail) => {
                fails.push(fail);
                None
            }
        },
    };

    let sig = format!(
        "fn {name}({}) -> {}",
        inputs.iter().map(|i| crate::display::type_display(i.get(1).unwrap_or(&Value::Null))).collect::<Vec<_>>().join(", "),
        output.map(crate::display::type_display).unwrap_or_else(|| "()".to_string())
    );

    let mut reasons: Vec<String> = fails.iter().map(|f| f.reason.clone()).collect();
    reasons.sort();
    reasons.dedup();
    let (cat, plan) = if fails.iter().any(|f| f.hard) {
        (Cat::Excluded, None)
    } else if !fails.is_empty() {
        (Cat::HandleRequired, None)
    } else {
        let call = match (&for_ty, &trait_path, self_def) {
            (None, _, _) => format!("super::{name}"),
            (Some(_), tp, Some(def)) => {
                let mut ty = an.rust_path(def);
                if is_root {
                    ty.push_str("<'_>");
                }
                match tp {
                    Some(tp) => format!("<{ty} as {tp}>::{name}"),
                    None => format!("<{ty}>::{name}"),
                }
            }
            _ => unreachable!("an impl function without a Self definition is a failure"),
        };
        let ret = ret.unwrap();
        let recv = recv_plan;
        let basic = args.iter().all(|a| matches!(a.mode, ArgMode::Value | ArgMode::Ref) && a.shape.is_basic())
            && match &ret {
                RetPlan::Unit => true,
                RetPlan::Value(s) => s.is_basic(),
                RetPlan::Copy(_) => false,
            }
            && !matches!(recv, RecvPlan::Value(_));
        (if basic { Cat::BoundA } else { Cat::BoundMech }, Some(Plan { recv, args, ret, call }))
    };
    let recv_ref = match plan.as_ref().map(|p| &p.recv) {
        Some(RecvPlan::Handle { .. }) => self_def,
        _ => None,
    };
    FnReport { path: key, name, sig, cat, reasons, plan, derived: false, receiver, line, recv_ref }
}

fn arg_plan<'u>(an: &mut Analyzer<'u>, doc: usize, ty: &'u Value, sty: Option<SelfTy<'u>>) -> Result<ArgPlan, Fail> {
    if let Some(reference) = ty.get("borrowed_ref") {
        let mutable = reference.get("is_mutable").and_then(Value::as_bool) == Some(true);
        let inner = reference.get("type").unwrap_or(&Value::Null);
        let mode = if mutable { ArgMode::RefMut } else { ArgMode::Ref };
        // a decoded argument lives only for the call: a `&'static` parameter (an interned key the engine keeps) cannot
        // be fed from guest bytes
        if reference.get("lifetime").and_then(Value::as_str) == Some("'static") {
            return Err(hard("`&'static` argument: the engine keeps it, a value decoded for the call cannot be"));
        }
        if inner.get("primitive").and_then(Value::as_str) == Some("str") {
            if mutable {
                return Err(hard("`&mut str` cannot be written back"));
            }
            return Ok(ArgPlan { mode, shape: Shape::Str });
        }
        if let Some(slice) = inner.get("slice") {
            let element = an.shape(doc, slice, sty)?;
            return Ok(ArgPlan { mode, shape: Shape::Vec(Box::new(element)) });
        }
        let shape = an.shape(doc, inner, sty).map_err(|fail| {
            if fail.hard { fail } else { soft(format!("{} [{}argument]", fail.reason, if mutable { "&mut " } else { "&" })) }
        })?;
        return Ok(ArgPlan { mode, shape });
    }
    if let Some(inner) = option_of_shared_ref(an, doc, ty) {
        if inner.get("primitive").and_then(Value::as_str) == Some("str") {
            return Ok(ArgPlan { mode: ArgMode::OptDeref, shape: Shape::Option(Box::new(Shape::Str)) });
        }
        if let Some(slice) = inner.get("slice") {
            let element = an.shape(doc, slice, sty)?;
            return Ok(ArgPlan { mode: ArgMode::OptDeref, shape: Shape::Option(Box::new(Shape::Vec(Box::new(element)))) });
        }
        let shape = an.shape(doc, inner, sty).map_err(|fail| {
            if fail.hard { fail } else { soft(format!("{} [Option<&> argument]", fail.reason)) }
        })?;
        return Ok(ArgPlan { mode: ArgMode::OptRef, shape: Shape::Option(Box::new(shape)) });
    }
    Ok(ArgPlan { mode: ArgMode::Value, shape: an.shape(doc, ty, sty)? })
}

/// `Option<&T>` (std `Option`, shared reference): the referent's type.
fn option_of_shared_ref<'u>(an: &Analyzer<'u>, doc: usize, ty: &'u Value) -> Option<&'u Value> {
    let path = ty.get("resolved_path")?;
    let Res::Std(std_path) = an.uni.resolve(doc, path.get("id")?.as_u64()?) else { return None };
    if std_path.last().map(String::as_str) != Some("Option") {
        return None;
    }
    let args = path.pointer("/args/angle_bracketed/args")?.as_array()?;
    let [only] = args.as_slice() else { return None };
    let reference = only.get("type")?.get("borrowed_ref")?;
    if reference.get("is_mutable").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    reference.get("type")
}

fn ret_plan<'u>(an: &mut Analyzer<'u>, doc: usize, ty: &'u Value, sty: Option<SelfTy<'u>>) -> Result<RetPlan, Fail> {
    if let Some(reference) = ty.get("borrowed_ref") {
        let mutable = reference.get("is_mutable").and_then(Value::as_bool) == Some(true);
        let inner = reference.get("type").unwrap_or(&Value::Null);
        if mutable {
            // Look inside first so a hard failure is reported as hard.
            if let Err(fail) = an.shape(doc, inner, sty) {
                if fail.hard {
                    return Err(fail);
                }
            }
            return Err(soft("returns `&mut`: needs a child handle that locks its parent"));
        }
        if inner.get("primitive").and_then(Value::as_str) == Some("str") {
            return Ok(RetPlan::Copy(Shape::Str));
        }
        if let Some(slice) = inner.get("slice") {
            return Ok(RetPlan::Copy(Shape::Vec(Box::new(an.shape(doc, slice, sty)?))));
        }
        return match an.shape(doc, inner, sty) {
            Ok(shape) => Ok(RetPlan::Copy(shape)),
            Err(fail) if fail.hard => Err(fail),
            Err(fail) => Err(soft(format!("{} [returned reference]", fail.reason))),
        };
    }
    let shape = an.shape(doc, ty, sty).map_err(|fail| {
        if fail.hard { fail } else { soft(format!("{} [return value]", fail.reason)) }
    })?;
    Ok(if shape == Shape::Unit { RetPlan::Unit } else { RetPlan::Value(shape) })
}

/// The data definitions reachable from bound bindings, in a stable order (dependencies first).
pub fn reachable_defs(an: &Analyzer<'_>, roots: &[&Shape]) -> Vec<usize> {
    fn visit_shape(an: &Analyzer<'_>, shape: &Shape, seen: &mut Vec<usize>, state: &mut Vec<bool>) {
        match shape {
            Shape::Array(inner, _)
            | Shape::Vec(inner)
            | Shape::Option(inner)
            | Shape::Boxed(inner)
            | Shape::Shared(_, inner)
            | Shape::SharedSlice(_, inner)
            | Shape::Set(_, inner)
            | Shape::VecDeque(inner) => visit_shape(an, inner, seen, state),
            Shape::Result(a, b) | Shape::Map(_, a, b) => {
                visit_shape(an, a, seen, state);
                visit_shape(an, b, seen, state);
            }
            Shape::Tuple(items) => {
                for item in items {
                    visit_shape(an, item, seen, state);
                }
            }
            Shape::Named(index) => {
                if state[*index] {
                    return;
                }
                state[*index] = true;
                match &an.defs[*index].body {
                    DefBody::Struct { fields, .. } => {
                        for field in fields {
                            visit_shape(an, &field.shape, seen, state);
                        }
                    }
                    DefBody::Enum { variants } => {
                        for variant in variants {
                            for field in &variant.fields {
                                visit_shape(an, &field.shape, seen, state);
                            }
                        }
                    }
                    DefBody::Pending => {}
                }
                seen.push(*index);
            }
            _ => {}
        }
    }
    let mut order = Vec::new();
    let mut state = vec![false; an.defs.len()];
    for shape in roots {
        visit_shape(an, shape, &mut order, &mut state);
    }
    order.sort_by(|a, b| an.defs[*a].rust_path.cmp(&an.defs[*b].rust_path));
    order
}
