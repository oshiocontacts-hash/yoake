//! The guest side: typed stubs over `yoake_guest_sdk::call` for the bindings of a module, in Rust, for the
//! wasm32 guest. Same ids, same wire bytes as the host glue; the data types are generated here as guest
//! structs and enums. Bindings whose receiver is the `Game` root itself (`call_game`) have no stub: a
//! guest reaches `Game` only through path handles.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::analyze::{ArgMode, Cat, FnReport, ModuleScan, Plan, RecvPlan, RetPlan, reachable_defs};
use crate::render::{dec, enc, ident, render_def, set_guest_names};
use crate::shape::{Analyzer, DefBody, Form, NamedDef, Shape};

fn sanitize(path: &str) -> String {
    let mut out = String::new();
    let mut last_underscore = true;
    for c in path.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_end_matches('_').to_string()
}

/// The guest Rust type of a shape; `None` when the guest cannot hold it (an array length that is not a number).
fn ty(shape: &Shape, defs: &[NamedDef], names: &[String]) -> Option<String> {
    Some(match shape {
        Shape::Unit => "()".into(),
        Shape::Prim(p) | Shape::Pointer(p) => (*p).to_string(),
        Shape::Char => "char".into(),
        Shape::Str | Shape::SharedStr(_) => "String".into(),
        Shape::Array(inner, len) => {
            len.trim().parse::<usize>().ok()?;
            format!("[{}; {}]", ty(inner, defs, names)?, len.trim())
        }
        Shape::Vec(inner) | Shape::SharedSlice(_, inner) | Shape::VecDeque(inner) | Shape::Set(_, inner) => format!("Vec<{}>", ty(inner, defs, names)?),
        Shape::Option(inner) => format!("Option<{}>", ty(inner, defs, names)?),
        Shape::Result(ok, err) => format!("Result<{}, {}>", ty(ok, defs, names)?, ty(err, defs, names)?),
        Shape::Boxed(inner) | Shape::Shared(_, inner) => format!("Box<{}>", ty(inner, defs, names)?),
        Shape::Map(_, k, v) => format!("Vec<({}, {})>", ty(k, defs, names)?, ty(v, defs, names)?),
        Shape::Tuple(items) => {
            let parts: Option<Vec<String>> = items.iter().map(|i| ty(i, defs, names)).collect();
            format!("({},)", parts?.join(", "))
        }
        Shape::Named(index) => format!("types::{}", names[*index]),
    })
}

fn def_supported(def: &NamedDef, defs: &[NamedDef], names: &[String]) -> bool {
    match &def.body {
        DefBody::Struct { fields, .. } => fields.iter().all(|f| ty(&f.shape, defs, names).is_some()),
        DefBody::Enum { variants } => variants.iter().all(|v| v.fields.iter().all(|f| ty(&f.shape, defs, names).is_some())),
        DefBody::Pending => false,
    }
}

fn uses_unsupported(shape: &Shape, defs: &[NamedDef], names: &[String], bad: &[bool]) -> bool {
    if ty(shape, defs, names).is_none() {
        return true;
    }
    let mut stack = vec![shape];
    while let Some(s) = stack.pop() {
        match s {
            Shape::Named(i) => {
                if bad[*i] {
                    return true;
                }
            }
            Shape::Array(i, _) | Shape::Vec(i) | Shape::Option(i) | Shape::Boxed(i) | Shape::Shared(_, i) | Shape::SharedSlice(_, i) | Shape::Set(_, i) | Shape::VecDeque(i) => stack.push(i),
            Shape::Result(a, b) | Shape::Map(_, a, b) => {
                stack.push(a);
                stack.push(b);
            }
            Shape::Tuple(items) => stack.extend(items.iter()),
            _ => {}
        }
    }
    false
}

/// Parameter text for an argument.
fn param_ty(mode: ArgMode, shape: &Shape, defs: &[NamedDef], names: &[String]) -> Option<String> {
    let t = ty(shape, defs, names)?;
    Some(match mode {
        ArgMode::Value => t,
        ArgMode::Ref => match shape {
            Shape::Str => "&str".into(),
            Shape::Vec(inner) => format!("&[{}]", ty(inner, defs, names)?),
            _ => format!("&{t}"),
        },
        ArgMode::RefMut => format!("&mut {t}"),
        ArgMode::OptRef => match shape {
            Shape::Option(inner) => format!("Option<&{}>", ty(inner, defs, names)?),
            _ => return None,
        },
        ArgMode::OptDeref => match shape {
            Shape::Option(inner) => match &**inner {
                Shape::Str => "Option<&str>".into(),
                Shape::Vec(e) => format!("Option<&[{}]>", ty(e, defs, names)?),
                _ => return None,
            },
            _ => return None,
        },
    })
}

fn arg_place(mode: ArgMode, i: usize) -> String {
    match mode {
        ArgMode::Value | ArgMode::OptRef | ArgMode::OptDeref => format!("&a{i}"),
        ArgMode::Ref => format!("a{i}"),
        ArgMode::RefMut => format!("&*a{i}"),
    }
}

struct Stub {
    #[allow(dead_code)]
    name: String,
    id: u32,
    text: String,
    uses: Vec<Shape>,
}

pub fn render_guest(an: &Analyzer<'_>, module_label: &str, scan: &ModuleScan, ids: &BTreeMap<String, u32>, game_paths: &[crate::GamePathCfg]) -> String {
    // 1. which data definitions can the guest hold, and what are they called
    let defs = &an.defs;
    let mut names: Vec<String> = Vec::with_capacity(defs.len());
    let mut used = std::collections::BTreeSet::new();
    for def in defs {
        let leaf = def.rust_path.rsplit("::").next().unwrap_or("T").to_string();
        let name = if used.insert(leaf.clone()) { leaf } else { def.mangle.clone() };
        used.insert(name.clone());
        names.push(name);
    }
    let mut bad: Vec<bool> = defs.iter().map(|d| !def_supported(d, defs, &names)).collect();
    // a definition that holds an unsupported one is unsupported too
    loop {
        let mut changed = false;
        for (i, def) in defs.iter().enumerate() {
            if bad[i] {
                continue;
            }
            let fields: Vec<&Shape> = match &def.body {
                DefBody::Struct { fields, .. } => fields.iter().map(|f| &f.shape).collect(),
                DefBody::Enum { variants } => variants.iter().flat_map(|v| v.fields.iter().map(|f| &f.shape)).collect(),
                DefBody::Pending => Vec::new(),
            };
            if fields.iter().any(|s| uses_unsupported(s, defs, &names, &bad)) {
                bad[i] = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    set_guest_names(Some(names.clone()));
    let mut stubs: Vec<Stub> = Vec::new();
    let mut taken_names = std::collections::BTreeSet::new();
    let mut fn_name = |path: &str, id: u32| {
        let mut n = sanitize(path);
        if n.is_empty() || !taken_names.insert(n.clone()) {
            n = format!("{n}_{id}");
            taken_names.insert(n.clone());
        }
        n
    };

    // 2. functions
    for report in scan.fns.iter().filter(|r| matches!(r.cat, Cat::BoundA | Cat::BoundMech)) {
        let Some(plan) = report.plan.as_ref() else { continue };
        if matches!(plan.recv, RecvPlan::Game { .. }) {
            continue;
        }
        let id = ids[&report.path];
        if let Some(stub) = fn_stub(report, plan, id, defs, &names, &bad, &mut fn_name) {
            stubs.push(stub);
        }
    }
    // 3. fields
    for field in scan.fields.iter().filter(|f| f.shape.is_some()) {
        let shape = field.shape.as_ref().unwrap();
        if uses_unsupported(shape, defs, &names, &bad) {
            continue;
        }
        let t = ty(shape, defs, &names).unwrap();
        let key = crate::render::getter_key(field);
        let id = ids[&key];
        let name = fn_name(&key, id);
        let mut text = String::new();
        let _ = writeln!(text, "/// `{key}`\npub fn {name}(h: Handle) -> Result<{t}, Error> {{");
        let _ = writeln!(text, "    let result = call(h, {id}, &[])?;\n    let mut dec = Decoder::new(&result);");
        let _ = writeln!(text, "    let value = (|| -> Result<{t}, CallError> {{\n        let d = &mut dec;\n        let value = {};\n        d.finish()?;\n        Ok(value)\n    }})()?;\n    Ok(value)\n}}\n", indent(&dec(shape, 0, defs), 8).trim_start());
        stubs.push(Stub { name, id, text, uses: vec![shape.clone()] });
        if field.writable {
            let key = crate::render::setter_key(field);
            let id = ids[&key];
            let name = fn_name(&key, id);
            let mut text = String::new();
            let _ = writeln!(text, "/// `{key}`\npub fn {name}(h: Handle, value: {t}) -> Result<(), Error> {{\n    let mut args: Vec<u8> = Vec::new();\n    {{\n        let out = &mut args;");
            let _ = writeln!(text, "{}", indent(&enc(shape, "&value", 0, defs), 8));
            let _ = writeln!(text, "    }}\n    call(h, {id}, &args)?;\n    Ok(())\n}}\n");
            stubs.push(Stub { name, id, text, uses: vec![shape.clone()] });
        }
    }
    // 4. path handles
    for path in game_paths {
        let key = crate::render::path_key(module_label, path);
        let id = ids[&key];
        let name = fn_name(&key, id);
        let mut text = String::new();
        match &path.by_id {
            Some(by_id) => {
                let _ = writeln!(text, "/// `{key}`: a path handle to the `{}` with this id, valid inside this activation.\npub fn {name}(root: Handle, id: {}) -> Result<Handle, Error> {{", path.ty, by_id.ty);
                let _ = writeln!(text, "    let mut args: Vec<u8> = Vec::new();\n    Encode::encode(&id, &mut args);");
            }
            None => {
                let _ = writeln!(text, "/// `{key}`: a path handle to the `{}`, valid inside this activation.\npub fn {name}(root: Handle) -> Result<Handle, Error> {{", path.ty);
                let _ = writeln!(text, "    let args: Vec<u8> = Vec::new();");
            }
        }
        let _ = writeln!(text, "    let result = call(root, {id}, &args)?;\n    Ok(Handle(crate::decode::<u64>(&result)?))\n}}\n");
        stubs.push(Stub { name, id, text, uses: Vec::new() });
    }
    stubs.sort_by_key(|s| s.id);

    // 5. only the data types the emitted stubs reach
    let roots: Vec<&Shape> = stubs.iter().flat_map(|s| s.uses.iter()).collect();
    let reach = reachable_defs(an, &roots);

    let mut out = String::new();
    let _ = writeln!(out, "// @generated by yoake-bindgen (MX3K1), guest stubs of module `{module_label}`. DO NOT EDIT.");
    let _ = writeln!(out, "// Same binding ids and wire bytes as the host glue; see docs/yoake2/ENGINE_CALL_ABI.md.");
    let _ = writeln!(out, "#![allow(dead_code, unused_mut, unused_variables, unused_parens, unused_imports, non_snake_case, non_camel_case_types, clippy::all)]");
    let _ = writeln!(out, "use alloc::boxed::Box;\nuse alloc::string::String;\nuse alloc::vec::Vec;\nuse crate::wire::{{self as codec, CallError, Decode, Decoder, Encode}};\nuse crate::{{Error, Handle, call}};\n");
    let _ = writeln!(out, "/// Engine data types as the guest holds them.\npub mod types {{\n    use super::types;\n    use alloc::boxed::Box;\n    use alloc::string::String;\n    use alloc::vec::Vec;");
    for &index in &reach {
        let def = &defs[index];
        let name = &names[index];
        match &def.body {
            DefBody::Struct { form, fields } => {
                let _ = writeln!(out, "    #[derive(Clone, Debug, PartialEq)]");
                match form {
                    Form::Unit => {
                        let _ = writeln!(out, "    pub struct {name};");
                    }
                    Form::Tuple => {
                        let items = fields.iter().map(|f| format!("pub {}", ty(&f.shape, defs, &names).unwrap())).collect::<Vec<_>>().join(", ");
                        let _ = writeln!(out, "    pub struct {name}({items});");
                    }
                    Form::Plain => {
                        let _ = writeln!(out, "    pub struct {name} {{");
                        for f in fields {
                            let _ = writeln!(out, "        pub {}: {},", ident(&f.name), ty(&f.shape, defs, &names).unwrap());
                        }
                        let _ = writeln!(out, "    }}");
                    }
                }
            }
            DefBody::Enum { variants } => {
                let _ = writeln!(out, "    #[derive(Clone, Debug, PartialEq)]\n    pub enum {name} {{");
                for v in variants {
                    let vn = ident(&v.name);
                    match v.form {
                        Form::Unit => {
                            let _ = writeln!(out, "        {vn},");
                        }
                        Form::Tuple => {
                            let items = v.fields.iter().map(|f| ty(&f.shape, defs, &names).unwrap()).collect::<Vec<_>>().join(", ");
                            let _ = writeln!(out, "        {vn}({items}),");
                        }
                        Form::Plain => {
                            let items = v.fields.iter().map(|f| format!("{}: {}", ident(&f.name), ty(&f.shape, defs, &names).unwrap())).collect::<Vec<_>>().join(", ");
                            let _ = writeln!(out, "        {vn} {{ {items} }},");
                        }
                    }
                }
                let _ = writeln!(out, "    }}");
            }
            DefBody::Pending => {}
        }
    }
    let _ = writeln!(out, "}}\n");
    for &index in &reach {
        render_def(&defs[index], defs, &mut out);
    }
    for stub in &stubs {
        out.push_str(&stub.text);
    }
    set_guest_names(None);
    out
}

fn indent(text: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    text.lines().map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}") }).collect::<Vec<_>>().join("\n")
}

fn fn_stub(
    report: &FnReport,
    plan: &Plan,
    id: u32,
    defs: &[NamedDef],
    names: &[String],
    bad: &[bool],
    fn_name: &mut dyn FnMut(&str, u32) -> String,
) -> Option<Stub> {
    let mut uses: Vec<Shape> = Vec::new();
    let mut params: Vec<String> = Vec::new();
    let mut body = String::new();
    let free = matches!(plan.recv, RecvPlan::None);
    if !free && !matches!(plan.recv, RecvPlan::Value(_)) {
        params.push("h: Handle".to_string());
    }
    let mut call_handle = if free { "Handle(0)".to_string() } else { "h".to_string() };
    if let RecvPlan::Value(shape) = &plan.recv {
        if uses_unsupported(shape, defs, names, bad) {
            return None;
        }
        // the receiver value is the first encoded value; there is no handle to route by
        call_handle = "Handle(0)".to_string();
        params.push(format!("this: {}", ty(shape, defs, names)?));
        let _ = writeln!(body, "{}", indent(&enc(shape, "&this", 0, defs), 8));
        uses.push(shape.clone());
    }
    for (i, arg) in plan.args.iter().enumerate() {
        if uses_unsupported(&arg.shape, defs, names, bad) {
            return None;
        }
        params.push(format!("a{i}: {}", param_ty(arg.mode, &arg.shape, defs, names)?));
        let _ = writeln!(body, "{}", indent(&enc(&arg.shape, &arg_place(arg.mode, i), 0, defs), 8));
        uses.push(arg.shape.clone());
    }
    let (ret_shape, ret_ty) = match &plan.ret {
        RetPlan::Unit => (Shape::Unit, "()".to_string()),
        RetPlan::Value(s) | RetPlan::Copy(s) => {
            if uses_unsupported(s, defs, names, bad) {
                return None;
            }
            (s.clone(), ty(s, defs, names)?)
        }
    };
    uses.push(ret_shape.clone());
    let name = fn_name(&report.path, id);
    let mut text = String::new();
    let _ = writeln!(text, "/// `{}`", report.path.replace('\n', " "));
    let _ = writeln!(text, "pub fn {name}({}) -> Result<{ret_ty}, Error> {{", params.join(", "));
    let _ = writeln!(text, "    let mut args: Vec<u8> = Vec::new();\n    {{\n        let out = &mut args;");
    text.push_str(&body);
    let _ = writeln!(text, "    }}\n    let result = call({call_handle}, {id}, &args)?;\n    let mut dec = Decoder::new(&result);");
    let _ = writeln!(text, "    let value = (|| -> Result<{ret_ty}, CallError> {{\n        let d = &mut dec;\n        let value = {};", indent(&dec(&ret_shape, 0, defs), 8).trim_start());
    for (i, arg) in plan.args.iter().enumerate() {
        if arg.mode == ArgMode::RefMut {
            let _ = writeln!(text, "        *a{i} = {};", indent(&dec(&arg.shape, 0, defs), 8).trim_start());
        }
    }
    let _ = writeln!(text, "        d.finish()?;\n        Ok(value)\n    }})()?;\n    Ok(value)\n}}\n");
    Some(Stub { name, id, text, uses })
}
