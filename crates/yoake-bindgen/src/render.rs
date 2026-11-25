//! Writing the glue: one file per module, `include!`d inside it so private items are reachable.
//!
//! Generated code contains no `unsafe` and no raw pointers. A receiver comes from
//! `yoake_slice::activation::with_arg` (the slice runtime's lent, type-checked argument); data goes
//! through `yoake_slice::codec`; every failure is a `CallError`.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::analyze::{ArgMode, Cat, FieldReport, FnReport, ModuleScan, Plan, RecvPlan, RetPlan, reachable_defs};
use crate::shape::{Analyzer, DefBody, Form, NamedDef, Shape};

const KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut",
    "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract",
    "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen",
];

thread_local! {
    /// Set while the guest stubs are written: the same codec text, over the guest's types (`Box` for the
    /// shared pointers, `Vec` for maps and sets, `types::X` for the data types).
    static GUEST: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
}

pub(crate) fn set_guest_names(names: Option<Vec<String>>) {
    GUEST.with(|g| *g.borrow_mut() = names);
}

fn guest_on() -> bool {
    GUEST.with(|g| g.borrow().is_some())
}

/// How generated code names the type of definition `index`.
fn def_path(index: usize, def: &NamedDef) -> String {
    GUEST.with(|g| match g.borrow().as_ref() {
        Some(names) => format!("types::{}", names[index]),
        None => def.rust_path.clone(),
    })
}

pub fn ident(name: &str) -> String {
    if KEYWORDS.contains(&name) { format!("r#{name}") } else { name.to_string() }
}

pub(crate) fn field_access(name: &str, tuple: bool) -> String {
    if tuple { name.to_string() } else { ident(name) }
}

/// What a lock key means to the renderer.
pub struct Bound<'a> {
    pub id: u32,
    pub path: String,
    pub what: BoundKind<'a>,
}

pub enum BoundKind<'a> {
    Fn(&'a FnReport, &'a Plan),
    Getter(&'a FieldReport, &'a Shape),
    Setter(&'a FieldReport, &'a Shape),
    /// A setter that exists only as a reserved id: the call returns `CallError::ReadOnly`.
    ReadOnlySetter(&'a FieldReport),
    /// `Game.<name>#handle`: makes a path handle (index into the target's `game_paths`).
    PathHandle(&'a crate::GamePathCfg, usize),
}

pub fn path_key(label: &str, path: &crate::GamePathCfg) -> String {
    format!("{label}::Game.{}#handle", path.name)
}

pub fn getter_key(field: &FieldReport) -> String {
    format!("{}.{}#get", field.owner_label, field.field)
}

pub fn setter_key(field: &FieldReport) -> String {
    format!("{}.{}#set", field.owner_label, field.field)
}

/// The lock keys and signatures of everything this module binds.
pub fn wanted(scan: &ModuleScan, label: &str, game_paths: &[crate::GamePathCfg]) -> BTreeMap<String, (String, &'static str)> {
    let mut out = BTreeMap::new();
    for path in game_paths {
        let sig = match &path.by_id {
            Some(by_id) => format!("handle {} by {}: {}", path.ty, by_id.field, by_id.ty),
            None => format!("handle {}", path.ty),
        };
        out.insert(path_key(label, path), (format!("{sig} at Game.{}", path.path.join(".")), "bound"));
    }
    for report in scan.fns.iter().filter(|r| matches!(r.cat, Cat::BoundA | Cat::BoundMech)) {
        out.insert(report.path.clone(), (report.sig.clone(), "bound"));
    }
    for field in scan.fields.iter().filter(|f| f.shape.is_some()) {
        let sig = format!("{}: {}", field.field, field.ty_text);
        out.insert(getter_key(field), (format!("get {sig}"), "bound"));
        out.insert(setter_key(field), (format!("set {sig}"), if field.writable { "bound" } else { "ReadOnly" }));
    }
    out
}

pub fn render(
    an: &Analyzer<'_>,
    module_label: &str,
    crate_name: &str,
    scan: &ModuleScan,
    ids: &BTreeMap<String, u32>,
    game_paths: &[crate::GamePathCfg],
    root_name: Option<&str>,
    table_readonly: &[String],
) -> String {
    let mut bound: Vec<Bound<'_>> = Vec::new();
    for (index, path) in game_paths.iter().enumerate() {
        bound.push(Bound { id: ids[&path_key(module_label, path)], path: path_key(module_label, path), what: BoundKind::PathHandle(path, index) });
    }
    for report in scan.fns.iter().filter(|r| matches!(r.cat, Cat::BoundA | Cat::BoundMech)) {
        let plan = report.plan.as_ref().expect("a bound function has a plan");
        bound.push(Bound { id: ids[&report.path], path: report.path.clone(), what: BoundKind::Fn(report, plan) });
    }
    for field in scan.fields.iter().filter(|f| f.shape.is_some()) {
        let shape = field.shape.as_ref().unwrap();
        bound.push(Bound { id: ids[&getter_key(field)], path: getter_key(field), what: BoundKind::Getter(field, shape) });
        let what = if field.writable { BoundKind::Setter(field, shape) } else { BoundKind::ReadOnlySetter(field) };
        bound.push(Bound { id: ids[&setter_key(field)], path: setter_key(field), what });
    }
    bound.sort_by_key(|b| b.id);

    // data codecs, only those a binding reaches
    let mut roots: Vec<&Shape> = Vec::new();
    for b in &bound {
        match &b.what {
            BoundKind::Fn(_, plan) => {
                for arg in &plan.args {
                    roots.push(&arg.shape);
                }
                match &plan.ret {
                    RetPlan::Value(s) | RetPlan::Copy(s) => roots.push(s),
                    RetPlan::Unit => {}
                }
                if let RecvPlan::Value(s) = &plan.recv {
                    roots.push(s);
                }
            }
            BoundKind::Getter(_, s) | BoundKind::Setter(_, s) => roots.push(s),
            BoundKind::ReadOnlySetter(_) | BoundKind::PathHandle(..) => {}
        }
    }
    let defs = reachable_defs(an, &roots);

    let mut out = String::new();
    let _ = writeln!(out, "// @generated by yoake-bindgen (MX3G1). DO NOT EDIT.");
    let _ = writeln!(out, "// Regenerate: cargo run -p yoake-bindgen -- --config crates/yoake-bindgen/bindgen.json");
    let _ = writeln!(out, "// Module `{module_label}` of crate `{crate_name}`; ids come from crates/yoake-bindgen/ids.lock.json.");
    let _ = writeln!(out, "// Included inside the module so private items are reachable. `#![forbid(unsafe_code)]` below.");
    let _ = writeln!(out, "#[allow(dead_code, unused_mut, unused_variables, unused_parens, unused_imports, non_snake_case, non_upper_case_globals, private_interfaces, private_bounds, clippy::all)]");
    let _ = writeln!(out, "pub mod yoake_glue {{");
    let _ = writeln!(out, "    // The compiler, not a grep, guarantees the glue has no unsafe code.");
    let _ = writeln!(out, "    #![forbid(unsafe_code)]");
    let _ = writeln!(out, "    use yoake_slice::activation::{{self, Accessor, FieldTable, PathRoot, PathRootRef, RefFieldTable, Status}};");
    let _ = writeln!(out, "    use yoake_slice::codec::{{self, CallError, Decode, Decoder, Encode}};");
    let _ = writeln!(out);

    for index in &defs {
        render_def(&an.defs[*index], &an.defs, &mut out);
    }
    for b in &bound {
        render_binding(b, &an.defs, &mut out);
    }
    // REM-11 access-transformer parity: a target's field tables get the same setters as the stamped tables
    // (`table_setter_allowed`), besides the fields its `writable` list opens to guest bindings.
    render_tables(an, scan, Some(table_readonly), &mut out);
    if let (false, Some(root)) = (game_paths.is_empty(), root_name) {
        render_path_root(an, root, game_paths, &mut out);
    }
    render_dispatch(&bound, &mut out);
    out.push_str("}\n");
    out
}

fn indent(text: &str, spaces: usize) -> String {
    let pad = " ".repeat(spaces);
    text.lines().map(|l| if l.is_empty() { String::new() } else { format!("{pad}{l}") }).collect::<Vec<_>>().join("\n")
}

// ---- codec expressions ----

pub(crate) fn enc(shape: &Shape, place: &str, depth: usize, defs: &[NamedDef]) -> String {
    match shape {
        Shape::Unit => format!("let _ = {place};"),
        Shape::Prim(_) | Shape::Pointer(_) | Shape::Char | Shape::Str => format!("Encode::encode({place}, out);"),
        Shape::Array(inner, _) => {
            format!("for e{depth} in ({place}).iter() {{\n{}\n}}", indent(&enc(inner, &format!("e{depth}"), depth + 1, defs), 4))
        }
        Shape::Vec(inner) => format!(
            "{{\n    let s{depth} = {place};\n    codec::enc_len(s{depth}.len(), out);\n    for e{depth} in s{depth}.iter() {{\n{}\n    }}\n}}",
            indent(&enc(inner, &format!("e{depth}"), depth + 1, defs), 8)
        ),
        Shape::Option(inner) => format!(
            "match {place} {{\n    None => out.push(0),\n    Some(v{depth}) => {{\n        out.push(1);\n{}\n    }}\n}}",
            indent(&enc(inner, &format!("v{depth}"), depth + 1, defs), 8)
        ),
        Shape::Result(ok, err) => format!(
            "match {place} {{\n    Ok(v{depth}) => {{\n        out.push(0);\n{}\n    }}\n    Err(e{depth}) => {{\n        out.push(1);\n{}\n    }}\n}}",
            indent(&enc(ok, &format!("v{depth}"), depth + 1, defs), 8),
            indent(&enc(err, &format!("e{depth}"), depth + 1, defs), 8)
        ),
        Shape::Boxed(inner) | Shape::Shared(_, inner) => enc(inner, &format!("&**({place})"), depth + 1, defs),
        Shape::SharedStr(_) => format!("Encode::encode(&**({place}), out);"),
        Shape::SharedSlice(_, inner) => enc(&Shape::Vec(inner.clone()), &format!("&**({place})"), depth + 1, defs),
        Shape::VecDeque(inner) => format!(
            "{{
    let s{depth} = {place};
    codec::enc_len(s{depth}.len(), out);
    for e{depth} in s{depth}.iter() {{
{}
    }}
}}",
            indent(&enc(inner, &format!("e{depth}"), depth + 1, defs), 8)
        ),
        Shape::Set(_, inner) => format!(
            "{{
    let s{depth} = {place};
    codec::enc_len(s{depth}.len(), out);
    for e{depth} in s{depth}.iter() {{
{}
    }}
}}",
            indent(&enc(inner, &format!("e{depth}"), depth + 1, defs), 8)
        ),
        Shape::Map(_, key, value) => format!(
            "{{
    let s{depth} = {place};
    codec::enc_len(s{depth}.len(), out);
    for (k{depth}, e{depth}) in s{depth}.iter() {{
{}
{}
    }}
}}",
            indent(&enc(key, &format!("k{depth}"), depth + 1, defs), 8),
            indent(&enc(value, &format!("e{depth}"), depth + 1, defs), 8)
        ),
        Shape::Tuple(items) => {
            let mut body = format!("{{\n    let t{depth} = {place};\n");
            for (index, item) in items.iter().enumerate() {
                body.push_str(&indent(&enc(item, &format!("&t{depth}.{index}"), depth + 1, defs), 4));
                body.push('\n');
            }
            body.push('}');
            body
        }
        Shape::Named(index) => format!("enc_{}({place}, out);", defs[*index].mangle),
    }
}

pub(crate) fn dec(shape: &Shape, depth: usize, defs: &[NamedDef]) -> String {
    if guest_on() {
        // the guest has no Arc, maps or sets: the same bytes decode into Box and Vec
        match shape {
            Shape::Shared(_, inner) => return format!("Box::new({})", dec(inner, depth + 1, defs)),
            Shape::SharedStr(_) => return "<String as Decode>::decode(d)?".to_string(),
            Shape::SharedSlice(_, inner) | Shape::VecDeque(inner) | Shape::Set(_, inner) => return dec(&Shape::Vec(inner.clone()), depth, defs),
            Shape::Map(_, key, value) => return dec(&Shape::Vec(Box::new(Shape::Tuple(vec![(**key).clone(), (**value).clone()]))), depth, defs),
            _ => {}
        }
    }
    match shape {
        Shape::Unit => "()".to_string(),
        Shape::Prim(p) | Shape::Pointer(p) => format!("<{p} as Decode>::decode(d)?"),
        Shape::Char => "<char as Decode>::decode(d)?".to_string(),
        Shape::Str => "<String as Decode>::decode(d)?".to_string(),
        Shape::Array(inner, len) => format!(
            "{{\n    let mut v{depth} = Vec::new();\n    for _ in 0..({len}) {{\n        v{depth}.push({});\n    }}\n    match <[_; {len}]>::try_from(v{depth}) {{\n        Ok(a) => a,\n        Err(_) => return Err(CallError::UnexpectedEnd),\n    }}\n}}",
            dec(inner, depth + 1, defs)
        ),
        Shape::Vec(inner) => format!(
            "{{\n    let n{depth} = d.vec_len()?;\n    let mut v{depth} = Decoder::vec_with(n{depth});\n    for _ in 0..n{depth} {{\n        v{depth}.push({});\n    }}\n    v{depth}\n}}",
            dec(inner, depth + 1, defs)
        ),
        Shape::Option(inner) => format!(
            "match d.tag()? {{\n    0 => None,\n    1 => Some({}),\n    t => return Err(CallError::InvalidTag(\"Option\", u32::from(t))),\n}}",
            dec(inner, depth + 1, defs)
        ),
        Shape::Result(ok, err) => format!(
            "match d.tag()? {{\n    0 => Ok({}),\n    1 => Err({}),\n    t => return Err(CallError::InvalidTag(\"Result\", u32::from(t))),\n}}",
            dec(ok, depth + 1, defs),
            dec(err, depth + 1, defs)
        ),
        Shape::Boxed(inner) => format!("Box::new({})", dec(inner, depth + 1, defs)),
        Shape::Shared(kind, inner) => format!("{kind}::new({})", dec(inner, depth + 1, defs)),
        Shape::SharedStr(kind) => format!("{kind}::<str>::from(<String as Decode>::decode(d)?)"),
        Shape::SharedSlice(kind, inner) => format!("{kind}::from({})", dec(&Shape::Vec(inner.clone()), depth + 1, defs)),
        Shape::VecDeque(inner) => format!(
            "{{
    let n{depth} = d.vec_len()?;
    let mut v{depth} = std::collections::VecDeque::new();
    for _ in 0..n{depth} {{
        v{depth}.push_back({});
    }}
    v{depth}
}}",
            dec(inner, depth + 1, defs)
        ),
        Shape::Set(kind, inner) => format!(
            "{{
    let n{depth} = d.vec_len()?;
    let mut v{depth} = {kind};
    for _ in 0..n{depth} {{
        v{depth}.insert({});
    }}
    v{depth}
}}",
            dec(inner, depth + 1, defs)
        ),
        Shape::Map(kind, key, value) => format!(
            "{{
    let n{depth} = d.vec_len()?;
    let mut v{depth} = {kind};
    for _ in 0..n{depth} {{
        let k = {};
        let e = {};
        v{depth}.insert(k, e);
    }}
    v{depth}
}}",
            dec(key, depth + 1, defs),
            dec(value, depth + 1, defs)
        ),
        Shape::Tuple(items) => format!("({},)", items.iter().map(|i| dec(i, depth + 1, defs)).collect::<Vec<_>>().join(", ")),
        Shape::Named(index) => format!("dec_{}(d)?", defs[*index].mangle),
    }
}

pub(crate) fn render_def(def: &NamedDef, defs: &[NamedDef], out: &mut String) {
    let index = defs.iter().position(|d| d.mangle == def.mangle).unwrap_or(0);
    let path = &def_path(index, def);
    let m = &def.mangle;
    let _ = writeln!(out, "    fn enc_{m}(v: &{path}, out: &mut Vec<u8>) {{");
    match &def.body {
        DefBody::Struct { form, fields } => {
            if fields.is_empty() {
                let _ = writeln!(out, "        let _ = v;");
            }
            for field in fields {
                let access = field_access(&field.name, *form == Form::Tuple);
                let _ = writeln!(out, "{}", indent(&enc(&field.shape, &format!("&v.{access}"), 0, defs), 8));
            }
        }
        DefBody::Enum { variants } => {
            let _ = writeln!(out, "        match v {{");
            for (tag, variant) in variants.iter().enumerate() {
                let vname = ident(&variant.name);
                let pattern = match variant.form {
                    Form::Unit => format!("{path}::{vname}"),
                    Form::Tuple => format!("{path}::{vname}({})", (0..variant.fields.len()).map(|i| format!("f{i}")).collect::<Vec<_>>().join(", ")),
                    Form::Plain => format!(
                        "{path}::{vname} {{ {} }}",
                        variant.fields.iter().enumerate().map(|(i, f)| format!("{}: f{i}", ident(&f.name))).collect::<Vec<_>>().join(", ")
                    ),
                };
                let _ = writeln!(out, "            {pattern} => {{");
                let _ = writeln!(out, "                Encode::encode(&{tag}u32, out);");
                for (i, field) in variant.fields.iter().enumerate() {
                    let _ = writeln!(out, "{}", indent(&enc(&field.shape, &format!("f{i}"), 0, defs), 16));
                }
                let _ = writeln!(out, "            }}");
            }
            let _ = writeln!(out, "        }}");
        }
        DefBody::Pending => {}
    }
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out, "    fn dec_{m}(d: &mut Decoder<'_>) -> Result<{path}, CallError> {{");
    let _ = writeln!(out, "        d.enter()?;\n        let r = dec_{m}_body(d);\n        d.leave();\n        r\n    }}");
    let _ = writeln!(out, "    fn dec_{m}_body(d: &mut Decoder<'_>) -> Result<{path}, CallError> {{");
    match &def.body {
        DefBody::Struct { form, fields } => match form {
            Form::Unit => {
                let _ = writeln!(out, "        Ok({path})");
            }
            Form::Tuple => {
                let items = fields.iter().map(|f| dec(&f.shape, 0, defs)).collect::<Vec<_>>().join(", ");
                let _ = writeln!(out, "        Ok({path}({items}))");
            }
            Form::Plain => {
                let _ = writeln!(out, "        Ok({path} {{");
                for field in fields {
                    let _ = writeln!(out, "            {}: {},", ident(&field.name), indent(&dec(&field.shape, 0, defs), 12).trim_start());
                }
                let _ = writeln!(out, "        }})");
            }
        },
        DefBody::Enum { variants } => {
            let _ = writeln!(out, "        match <u32 as Decode>::decode(d)? {{");
            for (tag, variant) in variants.iter().enumerate() {
                let vname = ident(&variant.name);
                let value = match variant.form {
                    Form::Unit => format!("{path}::{vname}"),
                    Form::Tuple => format!("{path}::{vname}({})", variant.fields.iter().map(|f| dec(&f.shape, 0, defs)).collect::<Vec<_>>().join(", ")),
                    Form::Plain => format!(
                        "{path}::{vname} {{ {} }}",
                        variant.fields.iter().map(|f| format!("{}: {}", ident(&f.name), dec(&f.shape, 0, defs))).collect::<Vec<_>>().join(", ")
                    ),
                };
                let _ = writeln!(out, "            {tag} => Ok({value}),");
            }
            let _ = writeln!(out, "            n => Err(CallError::InvalidTag(\"{}\", n)),", path.rsplit("::").next().unwrap_or("enum"));
            let _ = writeln!(out, "        }}");
        }
        DefBody::Pending => {
            let _ = writeln!(out, "        Err(CallError::UnexpectedEnd)");
        }
    }
    let _ = writeln!(out, "    }}");
    let _ = writeln!(out);
}

// ---- bindings ----

fn render_binding(b: &Bound<'_>, defs: &[NamedDef], out: &mut String) {
    let id = b.id;
    let _ = writeln!(out, "    /// `{}`", b.path.replace('\n', " "));
    match &b.what {
        BoundKind::Fn(report, plan) => render_fn(id, report, plan, defs, out),
        BoundKind::Getter(field, shape) => {
            let _ = writeln!(out, "    fn call_{id}(recv: &{}, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {{", field.owner_path);
            let _ = writeln!(out, "        Decoder::new(args).finish()?;");
            let access = field_access(&field.field, field.tuple);
            let _ = writeln!(out, "{}", indent(&enc(shape, &format!("&recv.{access}"), 0, defs), 8));
            let _ = writeln!(out, "        Ok(())\n    }}\n");
        }
        BoundKind::ReadOnlySetter(_) => {
            let _ = writeln!(out, "    // read-only: the id is reserved, the dispatcher refuses the call\n");
        }
        BoundKind::PathHandle(path, index) => {
            let _ = writeln!(out, "    fn call_{id}(root: u64, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {{");
            let _ = writeln!(out, "        let mut dec = Decoder::new(args);\n        let d = &mut dec;");
            let steps = match &path.by_id {
                Some(by_id) => {
                    let _ = writeln!(out, "        let key = <{} as Decode>::decode(d)?;", by_id.ty);
                    format!("vec![Accessor::Field({index}), Accessor::Id(u64::from(key))]")
                }
                None => format!("vec![Accessor::Field({index})]"),
            };
            let _ = writeln!(out, "        d.finish()?;");
            let _ = writeln!(out, "        let handle = activation::open_path(root, {steps}).map_err(CallError::from)?;");
            let _ = writeln!(out, "        Encode::encode(&handle, out);\n        Ok(())\n    }}\n");
        }
        BoundKind::Setter(field, shape) => {
            let _ = writeln!(out, "    fn call_{id}(recv: &mut {}, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {{", field.owner_path);
            let _ = writeln!(out, "        let mut dec = Decoder::new(args);
        let d = &mut dec;");
            let _ = writeln!(out, "        let value = {};", indent(&dec(shape, 0, defs), 8).trim_start());
            let _ = writeln!(out, "        d.finish()?;");
            let access = field_access(&field.field, field.tuple);
            let _ = writeln!(out, "        recv.{access} = value;");
            let _ = writeln!(out, "        Ok(())\n    }}\n");
        }
    }
}

fn render_fn(id: u32, report: &FnReport, plan: &Plan, defs: &[NamedDef], out: &mut String) {
    let recv_param = match &plan.recv {
        RecvPlan::Handle { type_path, mutable, .. } => format!("recv: &{}{type_path}, ", if *mutable { "mut " } else { "" }),
        RecvPlan::Game { type_path, .. } => format!("game: &mut {type_path}<'_>, "),
        RecvPlan::None | RecvPlan::Value(_) => String::new(),
    };
    let _ = writeln!(out, "    fn call_{id}({recv_param}args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {{");
    let _ = writeln!(out, "        let mut dec = Decoder::new(args);
        let d = &mut dec;");
    let mut call_args: Vec<String> = Vec::new();
    match &plan.recv {
        RecvPlan::Handle { .. } => call_args.push("recv".into()),
        RecvPlan::Game { mutable, .. } => call_args.push(if *mutable { "game".into() } else { "&*game".into() }),
        RecvPlan::Value(shape) => {
            let _ = writeln!(out, "        let this = {};", indent(&dec(shape, 0, defs), 8).trim_start());
            call_args.push("this".into());
        }
        RecvPlan::None => {}
    }
    for (index, arg) in plan.args.iter().enumerate() {
        let _ = writeln!(out, "        let mut a{index} = {};", indent(&dec(&arg.shape, 0, defs), 8).trim_start());
        call_args.push(match arg.mode {
            ArgMode::Value => format!("a{index}"),
            ArgMode::Ref => format!("&a{index}"),
            ArgMode::RefMut => format!("&mut a{index}"),
            ArgMode::OptRef => format!("a{index}.as_ref()"),
            ArgMode::OptDeref => format!("a{index}.as_deref()"),
        });
    }
    let _ = writeln!(out, "        d.finish()?;");
    let call = format!("{}({})", plan.call, call_args.join(", "));
    match &plan.ret {
        RetPlan::Unit => {
            let _ = writeln!(out, "        {call};");
        }
        RetPlan::Value(shape) => {
            let _ = writeln!(out, "        let r = {call};");
            let _ = writeln!(out, "{}", indent(&enc(shape, "&r", 0, defs), 8));
        }
        RetPlan::Copy(shape) => {
            let _ = writeln!(out, "        let r = {call};");
            let _ = writeln!(out, "{}", indent(&enc(shape, "r", 0, defs), 8));
        }
    }
    for (index, arg) in plan.args.iter().enumerate() {
        if arg.mode == ArgMode::RefMut {
            let _ = writeln!(out, "{}", indent(&enc(&arg.shape, &format!("&a{index}"), 0, defs), 8));
        }
    }
    let _ = report;
    let _ = writeln!(out, "        Ok(())\n    }}\n");
}

// ---- field tables and dispatch ----

fn get_expr(p: &str, access: &str) -> String {
    match p {
        "bool" | "u8" | "u16" | "u32" | "i8" | "i16" | "i32" => format!("i64::from(t.{access})"),
        "i64" => format!("t.{access}"),
        "u64" | "usize" | "isize" => format!("t.{access} as i64"),
        "f32" => format!("i64::from(t.{access}.to_bits())"),
        "f64" => format!("t.{access}.to_bits() as i64"),
        other => unreachable!("not a scalar: {other}"),
    }
}

fn set_stmt(p: &str, access: &str) -> String {
    match p {
        "bool" => format!("t.{access} = v != 0;"),
        "f32" => format!("t.{access} = f32::from_bits(v as u32);"),
        "f64" => format!("t.{access} = f64::from_bits(v as u64);"),
        other => format!("t.{access} = v as {other};"),
    }
}

fn scalar_of(shape: &Shape) -> Option<&'static str> {
    match shape {
        Shape::Pointer(p) => Some(p),
        other => other.scalar(),
    }
}

/// W9b "all fields" mode: only the field tables of one module (`FIELDS_*` and `REF_FIELDS_*`, getters of every
/// scalar field and, since REM-11, setters of every scalar field the policy allows (`table_setter_allowed`); no bindings), in the glue layout the bridge generator reads. `None` when the module
/// has no struct with a scalar field.
pub fn render_tables_only(an: &Analyzer<'_>, module_label: &str, crate_name: &str, scan: &ModuleScan) -> Option<String> {
    if !scan.fields.iter().any(|f| f.shape.as_ref().and_then(scalar_of).is_some()) {
        return None;
    }
    let mut out = String::new();
    let _ = writeln!(out, "// @generated by yoake-bindgen (W9b tables). DO NOT EDIT.");
    let _ = writeln!(out, "// Regenerate: cargo run -p yoake-bindgen -- --config crates/yoake-bindgen/bindgen.json --tables CRATE");
    let _ = writeln!(out, "// Module `{module_label}` of crate `{crate_name}`; getters of every scalar field, setters of the ones the policy allows, no bindings.");
    let _ = writeln!(out, "// Included inside the module (by the stamping overlay) so private fields are readable.");
    let _ = writeln!(out, "#[allow(dead_code, unused_mut, unused_variables, unused_parens, unused_imports, non_snake_case, non_upper_case_globals, private_interfaces, private_bounds, clippy::all)]");
    let _ = writeln!(out, "pub mod yoake_glue_tables {{");
    let _ = writeln!(out, "    #![forbid(unsafe_code)]");
    let _ = writeln!(out, "    use yoake_slice::activation::{{FieldTable, RefFieldTable}};");
    let _ = writeln!(out);
    // `tables` already applied the policy to `writable`
    render_tables(an, scan, None, &mut out);
    out.push_str("}
");
    Some(out)
}

/// `table_policy`: `Some(table_readonly)` adds the setters [`crate::table_setter_allowed`] allows to the `writable` ones.
fn render_tables(an: &Analyzer<'_>, scan: &ModuleScan, table_policy: Option<&[String]>, out: &mut String) {
    let mut by_type: BTreeMap<&str, Vec<&FieldReport>> = BTreeMap::new();
    for field in scan.fields.iter().filter(|f| f.shape.as_ref().and_then(scalar_of).is_some()) {
        by_type.entry(field.owner_path.as_str()).or_default().push(field);
    }
    let _ = an;
    for (owner, fields) in by_type {
        let mangle: String = owner.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let _ = writeln!(out, "    /// Scalar fields of `{owner}` as `i64` bit patterns, by declaration index (the slice runtime's `FieldTable`).");
        let _ = writeln!(out, "    pub static FIELDS_{mangle}: FieldTable<{owner}> = FieldTable {{");
        let _ = writeln!(out, "        get: |t, f| match f {{");
        for field in &fields {
            let p = scalar_of(field.shape.as_ref().unwrap()).unwrap();
            let _ = writeln!(out, "            {} => Some({}),", field.index, get_expr(p, &field_access(&field.field, field.tuple)));
        }
        let _ = writeln!(out, "            _ => None,\n        }},");
        let _ = writeln!(out, "        set: |t, f, v| match f {{");
        let settable = |f: &&&FieldReport| {
            f.writable || table_policy.is_some_and(|ro| crate::table_setter_allowed(&f.owner_path, &f.field, ro))
        };
        for field in fields.iter().filter(settable) {
            let p = scalar_of(field.shape.as_ref().unwrap()).unwrap();
            let _ = writeln!(out, "            {} => {{\n                {}\n                true\n            }}", field.index, set_stmt(p, &field_access(&field.field, field.tuple)));
        }
        let _ = writeln!(out, "            _ => false,\n        }},\n    }};\n");
        // the same getters over `&T`, for hooks whose argument is shared (`runtime::dispatch_id_ref`): the getters
        // only read fields, so they need no `&mut`
        let _ = writeln!(out, "    /// The getters of `FIELDS_{mangle}` over a shared `&{owner}` (read-only hook arguments, `RefFieldTable`).");
        let _ = writeln!(out, "    pub static REF_FIELDS_{mangle}: RefFieldTable<{owner}> = RefFieldTable {{");
        let _ = writeln!(out, "        get: |t, f| match f {{");
        for field in &fields {
            let p = scalar_of(field.shape.as_ref().unwrap()).unwrap();
            let _ = writeln!(out, "            {} => Some({}),", field.index, get_expr(p, &field_access(&field.field, field.tuple)));
        }
        let _ = writeln!(out, "            _ => None,\n        }},\n    }};\n");
    }
}

fn render_dispatch(bound: &[Bound<'_>], out: &mut String) {
    let _ = writeln!(out, "    /// Every binding of this module: id and lock key.");
    let _ = writeln!(out, "    pub const BINDINGS: &[(u32, &str)] = &[");
    for b in bound {
        let _ = writeln!(out, "        ({}, {:?}),", b.id, b.path);
    }
    let _ = writeln!(out, "    ];\n");

    let _ = writeln!(out, "    /// Runs binding `id`. `handle` is the hook-argument handle for a method (its receiver is checked to be");
    let _ = writeln!(out, "    /// the right type), ignored for functions without a receiver. `out` is left as it was on an error.");
    let _ = writeln!(out, "    pub fn call(handle: u64, id: u32, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {{");
    let _ = writeln!(out, "        let start = out.len();");
    let _ = writeln!(out, "        let result = match id {{");
    for b in bound {
        let id = b.id;
        match &b.what {
            BoundKind::Fn(_, plan) => match &plan.recv {
                RecvPlan::None | RecvPlan::Value(_) => {
                    let _ = writeln!(out, "            {id} => call_{id}(args, out),");
                }
                RecvPlan::Handle { type_path, mutable, .. } => {
                    let with = if *mutable { "with_arg" } else { "with_arg_shared" };
                    let _ = writeln!(out, "            {id} => activation::{with}::<{type_path}, _>(handle, |recv| call_{id}(recv, args, out)).map_err(CallError::from).and_then(|r| r),");
                }
                RecvPlan::Game { .. } => {}
            },
            BoundKind::ReadOnlySetter(_) => {
                let _ = writeln!(out, "            {id} => Err(CallError::ReadOnly(id)),");
            }
            BoundKind::PathHandle(..) => {
                let _ = writeln!(out, "            {id} => call_{id}(handle, args, out),");
            }
            BoundKind::Getter(field, _) => {
                let _ = writeln!(out, "            {id} => activation::with_arg_shared::<{}, _>(handle, |recv| call_{id}(recv, args, out)).map_err(CallError::from).and_then(|r| r),", field.owner_path);
            }
            BoundKind::Setter(field, _) => {
                let _ = writeln!(out, "            {id} => activation::with_arg::<{}, _>(handle, |recv| call_{id}(recv, args, out)).map_err(CallError::from).and_then(|r| r),", field.owner_path);
            }
        }
    }
    let _ = writeln!(out, "            _ => Err(CallError::UnknownBinding(id)),");
    let _ = writeln!(out, "        }};");
    let _ = writeln!(out, "        if result.is_err() {{\n            out.truncate(start);\n        }}\n        result\n    }}");

    let games: Vec<&Bound<'_>> = bound.iter().filter(|b| matches!(&b.what, BoundKind::Fn(_, Plan { recv: RecvPlan::Game { .. }, .. }))).collect();
    if let Some(Bound { what: BoundKind::Fn(_, Plan { recv: RecvPlan::Game { type_path, .. }, .. }), .. }) = games.first() {
        let _ = writeln!(out);
        let _ = writeln!(out, "    /// Runs a binding whose receiver is the `Game` root.");
        let _ = writeln!(out, "    pub fn call_game(game: &mut {type_path}<'_>, id: u32, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {{");
        let _ = writeln!(out, "        let start = out.len();");
        let _ = writeln!(out, "        let result = match id {{");
        for b in &games {
            let _ = writeln!(out, "            {0} => call_{0}(game, args, out),", b.id);
        }
        let _ = writeln!(out, "            _ => Err(CallError::UnknownBinding(id)),");
        let _ = writeln!(out, "        }};");
        let _ = writeln!(out, "        if result.is_err() {{\n            out.truncate(start);\n        }}\n        result\n    }}");
    }
}

/// `impl PathRoot for Game`: one `match` arm per configured path. The glue sits inside a module below
/// `game`, so the private fields of `Game` are reachable.
fn render_path_root(an: &Analyzer<'_>, root_name: &str, game_paths: &[crate::GamePathCfg], out: &mut String) {
    let root = root_path(an, root_name);
    let _ = writeln!(out, "    /// Paths a guest may reach from the `Game` root (see `game_paths` in bindgen.json).");
    let _ = writeln!(out, "    impl PathRoot for {root}<'_> {{");
    let _ = writeln!(out, "        fn resolve(&mut self, path: &[Accessor], f: &mut dyn FnMut(&mut dyn std::any::Any)) -> Result<(), Status> {{");
    let _ = writeln!(out, "            match path {{");
    for (index, path) in game_paths.iter().enumerate() {
        let place = format!("self.{}", path.path.iter().map(|p| ident(p)).collect::<Vec<_>>().join("."));
        match &path.by_id {
            Some(by_id) => {
                let _ = writeln!(out, "                [Accessor::Field({index}), Accessor::Id(id)] => match {place}.iter_mut().find(|e| u64::from(e.{}) == *id) {{", ident(&by_id.field));
                let _ = writeln!(out, "                    Some(e) => {{\n                        f(e);\n                        Ok(())\n                    }}");
                let _ = writeln!(out, "                    None => Err(Status::Stale),\n                }},");
            }
            None if path.optional => {
                let _ = writeln!(out, "                [Accessor::Field({index})] => match {place}.as_mut() {{");
                let _ = writeln!(out, "                    Some(e) => {{\n                        f(e);\n                        Ok(())\n                    }}");
                let _ = writeln!(out, "                    None => Err(Status::Stale),\n                }},");
            }
            None => {
                let _ = writeln!(out, "                [Accessor::Field({index})] => {{\n                    f(&mut {place});\n                    Ok(())\n                }}");
            }
        }
    }
    let _ = writeln!(out, "                _ => Err(Status::BadField),\n            }}\n        }}\n    }}\n");

    // the same paths read-only (W9b), for hooks whose argument is `&Game`
    let _ = writeln!(out, "    /// The same paths over a shared `&{root}` (`&Game` hooks: read-only traversal).");
    let _ = writeln!(out, "    impl PathRootRef for {root}<'_> {{");
    let _ = writeln!(out, "        fn resolve_ref(&self, path: &[Accessor], f: &mut dyn FnMut(&dyn std::any::Any)) -> Result<(), Status> {{");
    let _ = writeln!(out, "            match path {{");
    for (index, path) in game_paths.iter().enumerate() {
        let place = format!("self.{}", path.path.iter().map(|p| ident(p)).collect::<Vec<_>>().join("."));
        match &path.by_id {
            Some(by_id) => {
                let _ = writeln!(out, "                [Accessor::Field({index}), Accessor::Id(id)] => match {place}.iter().find(|e| u64::from(e.{}) == *id) {{", ident(&by_id.field));
                let _ = writeln!(out, "                    Some(e) => {{\n                        f(e);\n                        Ok(())\n                    }}");
                let _ = writeln!(out, "                    None => Err(Status::Stale),\n                }},");
            }
            None if path.optional => {
                let _ = writeln!(out, "                [Accessor::Field({index})] => match {place}.as_ref() {{");
                let _ = writeln!(out, "                    Some(e) => {{\n                        f(e);\n                        Ok(())\n                    }}");
                let _ = writeln!(out, "                    None => Err(Status::Stale),\n                }},");
            }
            None => {
                let _ = writeln!(out, "                [Accessor::Field({index})] => {{\n                    f(&{place});\n                    Ok(())\n                }}");
            }
        }
    }
    let _ = writeln!(out, "                _ => Err(Status::BadField),\n            }}\n        }}\n    }}\n");
}

/// The path of the root struct (`crate::game::Game`) in the target crate.
fn root_path(an: &Analyzer<'_>, name: &str) -> String {
    for (doc_index, doc) in an.uni.docs.iter().enumerate() {
        if doc.name != an.target.crate_name {
            continue;
        }
        if let Some(index) = doc.v.get("index").and_then(serde_json::Value::as_object) {
            let mut found: Vec<u64> = index
                .iter()
                .filter(|(_, item)| item.get("name").and_then(serde_json::Value::as_str) == Some(name) && item.pointer("/inner/struct").is_some())
                .filter_map(|(id, _)| id.parse().ok())
                .collect();
            found.sort_unstable();
            if let Some(id) = found.first() {
                return an.rust_path(crate::doc::ItemRef { doc: doc_index, id: *id });
            }
        }
    }
    format!("crate::{name}")
}
