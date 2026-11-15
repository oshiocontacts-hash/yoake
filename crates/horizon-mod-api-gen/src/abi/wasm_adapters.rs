//! The WASM adapters of the full catalog: for every hook, Rust that converts the native event to
//! the generated WIT values (`yoake-full.wit`), calls the guest when it subscribed, and applies
//! the answer per contract. The output goes to the consuming crate's `OUT_DIR`; nothing is
//! committed. The conversions come from the same `Shape`s that produced the WIT text, so the two
//! cannot drift.
//!
//! A hook whose conversion cannot be generated is not wired (see [`WasmAdapters::unsupported`]):
//! a guest that subscribes to it is refused at load, so nothing is silently dropped.
use super::wit_full::shape::{Field, ListKind, MapKind, NamedDef, RustPath, Shape, StrKind, VCase, WrapKind};
use super::wit_full::{EffectPlan, ExtraPlan, FullWit, HookPlan, Main, ResultForm, Side, generate_full_wit};
use quote::ToTokens;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

/// Newtypes with a private field and the accessors that read and build them.
const NEWTYPE_ACCESS: &[(&str, &str, &str)] = &[("NativeTickId", "get", "new"), ("NativeWorldEpoch", "get", "new")];

const DENYLIST: &str = include_str!("../../wasm-adapter-denylist.json");

/// The generated adapters and what they cover.
pub struct WasmAdapters {
    pub code: String,
    /// Hooks wired to WASM, in catalog order.
    pub wired: Vec<String>,
    /// Hooks that are not: name and why.
    pub unsupported: Vec<(String, String)>,
    /// Wired hooks whose answer cannot (all) be applied: name and what is left out.
    pub partial: Vec<(String, String)>,
    /// Every catalog hook with its sides, in catalog order (the ordinals of the generated code).
    pub hooks: Vec<(String, Side)>,
}

// ---------------------------------------------------------------------------------------------
// The generated Hooks trait

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum HostKind {
    None,
    Read,
    Write,
}

struct TraitHook {
    host: HostKind,
    /// The host parameter's name and type as written.
    host_param: Option<(String, String)>,
    event: String,
    /// The event parameter's type as written (`& mut XEvent < '_ >`).
    event_sig: String,
    ret: Option<String>,
    /// `Hooks` or `RenderHooks`.
    in_render: bool,
}

struct EventDef {
    fields: Vec<(String, syn::Type)>,
}

fn type_text(ty: &syn::Type) -> String {
    ty.to_token_stream().to_string()
}

fn parse_trait(source: &str, trait_name: &str, render: bool, out: &mut HashMap<String, TraitHook>, events: &mut HashMap<String, EventDef>) -> Result<(), String> {
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    for item in &file.items {
        match item {
            syn::Item::Trait(item) if item.ident == trait_name => {
                for member in &item.items {
                    let syn::TraitItem::Fn(method) = member else { continue };
                    let name = method.sig.ident.to_string();
                    if name == "render" {
                        continue;
                    }
                    let mut host = HostKind::None;
                    let mut host_param = None;
                    let mut event = None;
                    let mut event_sig = String::new();
                    for arg in &method.sig.inputs {
                        let syn::FnArg::Typed(arg) = arg else { continue };
                        let syn::Pat::Ident(pat) = &*arg.pat else { continue };
                        let text = type_text(&arg.ty);
                        let event_type = match &*arg.ty {
                            syn::Type::Reference(reference) if reference.mutability.is_some() => match &*reference.elem {
                                syn::Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()).filter(|n| n.ends_with("Event")),
                                _ => None,
                            },
                            _ => None,
                        };
                        if let Some(event_type) = event_type {
                            event_sig = text.clone();
                            event = Some(event_type);
                        } else {
                            host = if text.contains("dyn Host") {
                                if text.starts_with("& mut") { HostKind::Write } else { HostKind::Read }
                            } else {
                                HostKind::None
                            };
                            host_param = Some((pat.ident.to_string(), text));
                        }
                    }
                    let event = event.ok_or_else(|| format!("hook {name} has no event"))?;
                    let ret = match &method.sig.output {
                        syn::ReturnType::Default => None,
                        syn::ReturnType::Type(_, ty) => Some(type_text(ty)),
                    };
                    out.insert(name, TraitHook { host, host_param, event, event_sig, ret, in_render: render });
                }
            }
            syn::Item::Struct(item) if item.ident.to_string().ends_with("Event") => {
                if let syn::Fields::Named(fields) = &item.fields {
                    let fields = fields
                        .named
                        .iter()
                        .map(|f| (f.ident.as_ref().unwrap().to_string(), f.ty.clone()))
                        .collect();
                    events.insert(item.ident.to_string(), EventDef { fields });
                }
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Names

fn camel(wit: &str) -> String {
    use heck::ToUpperCamelCase;
    wit.trim_start_matches('%').to_upper_camel_case()
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "break", "const", "continue", "crate", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in",
    "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super",
    "trait", "true", "type", "unsafe", "use", "where", "while", "async", "await", "dyn", "abstract", "become", "box",
    "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try",
];

fn snake(wit: &str) -> String {
    let name = wit.trim_start_matches('%').replace('-', "_");
    if RUST_KEYWORDS.contains(&name.as_str()) {
        if matches!(name.as_str(), "crate" | "self" | "super" | "Self") { format!("{name}_") } else { format!("r#{name}") }
    } else {
        name
    }
}

fn ident(name: &str) -> String {
    name.replace('-', "_")
}

fn helper_name(prefix: &str, wit: &str) -> String {
    format!("{prefix}_{}", ident(wit.trim_start_matches('%')))
}

/// The path a native type is named by in generated code: through `crate::mod_api` when the mod API
/// re-exports it (those paths are public), else where it is defined.
fn rust_path(path: &RustPath, exports: &BTreeSet<String>) -> String {
    for name in [&path.alias, &path.name] {
        if exports.contains(name.as_str()) {
            return format!("crate::mod_api::{name}");
        }
    }
    let mut text = if path.krate == "horizon-client" { "crate".to_owned() } else { path.krate.replace('-', "_") };
    for segment in path.module.split("::").filter(|s| !s.is_empty()) {
        text.push_str("::");
        text.push_str(segment);
    }
    text.push_str("::");
    text.push_str(&path.name);
    text
}

// ---------------------------------------------------------------------------------------------
// Conversion expressions

struct Emitter<'a> {
    defs: &'a BTreeMap<String, NamedDef>,
    /// The names `crate::mod_api` re-exports.
    exports: &'a BTreeSet<String>,
    /// The generated conversion functions of named types.
    helpers: String,
    forward: BTreeMap<String, Result<(), String>>,
    reverse: BTreeMap<String, Result<(), String>>,
    update: BTreeMap<String, Result<(), String>>,
    depth: usize,
    /// Notes on the hook being built: parts of its answer that cannot be applied.
    notes: Vec<String>,
}

type Res<T> = Result<T, String>;

impl<'a> Emitter<'a> {
    fn fresh(&mut self, base: &str) -> String {
        self.depth += 1;
        format!("{base}{}", self.depth)
    }

    fn need_forward(&mut self, wit: &str) -> Res<()> {
        if let Some(done) = self.forward.get(wit) {
            return done.clone();
        }
        self.forward.insert(wit.to_owned(), Err("recursive type".into()));
        let made = forward_helper(self, wit);
        let result = match made {
            Ok(code) => {
                self.helpers.push_str(&code);
                Ok(())
            }
            Err(error) => Err(format!("{wit}: {error}")),
        };
        self.forward.insert(wit.to_owned(), result.clone());
        result
    }

    /// The record `shape` is a foreign non-exhaustive struct that can only be updated field by field.
    fn inplace(&self, shape: &Shape) -> Option<String> {
        if let Shape::Named(wit) = shape
            && let Some(NamedDef::Record { rust, tuple: false, .. }) = self.defs.get(wit)
            && rust.krate != "horizon-client"
            && rust.non_exhaustive
        {
            return Some(wit.clone());
        }
        None
    }

    fn need_update(&mut self, wit: &str) -> Res<()> {
        if let Some(done) = self.update.get(wit) {
            return done.clone();
        }
        self.update.insert(wit.to_owned(), Err("recursive type".into()));
        let made = (|| -> Res<String> {
            let Some(NamedDef::Record { rust, fields, .. }) = self.defs.get(wit).cloned() else { return Err("not a record".into()) };
            let mut body = String::new();
            for field in &fields {
                let conv = self.from_wit(&field.shape, &format!("v.{}", snake(&field.wit)))?;
                let _ = write!(body, "t.{} = ({conv})?; ", field.rust);
            }
            Ok(format!("fn {}(v: {}, t: &mut {}) -> Option<()> {{ {body} Some(()) }}\n", helper_name("fu", wit), wit_rust(wit), rust_path(&rust, self.exports)))
        })();
        let result = match made {
            Ok(code) => {
                self.helpers.push_str(&code);
                Ok(())
            }
            Err(error) => Err(format!("{wit}: {error}")),
        };
        self.update.insert(wit.to_owned(), result.clone());
        result
    }

    fn need_reverse(&mut self, wit: &str) -> Res<()> {
        if let Some(done) = self.reverse.get(wit) {
            return done.clone();
        }
        self.reverse.insert(wit.to_owned(), Err("recursive type".into()));
        let made = reverse_helper(self, wit);
        let result = match made {
            Ok(code) => {
                self.helpers.push_str(&code);
                Ok(())
            }
            Err(error) => Err(format!("{wit}: {error}")),
        };
        self.reverse.insert(wit.to_owned(), result.clone());
        result
    }

    /// An expression of the WIT value of the native value `v`, which is a `T`, `&T` or `&&T`.
    fn to_wit(&mut self, shape: &Shape, v: &str) -> Res<String> {
        Ok(match shape {
            Shape::Unsupported => return Err("a type with no WIT mapping".into()),
            Shape::Scalar { rust } => match *rust {
                "usize" => format!("({v}.val() as u64)"),
                "isize" => format!("({v}.val() as i64)"),
                _ => format!("{v}.val()"),
            },
            Shape::Str(StrKind::Text) => format!("{v}.to_string()"),
            Shape::Str(StrKind::Path) => format!("{v}.to_string_lossy().into_owned()"),
            Shape::Wide { signed } => format!("({v}.val() as {}).to_be_bytes().to_vec()", if *signed { "i128" } else { "u128" }),
            Shape::Glam { count, .. } => {
                let parts = (0..*count).map(|i| format!("a[{i}]")).collect::<Vec<_>>().join(", ");
                format!("{{ let a = {v}.to_array(); ({parts}) }}")
            }
            Shape::Handle => "next_handle()".into(),
            Shape::Tag => format!("nbt_tag(&{v})"),
            Shape::Compound => format!("nbt_compound(&{v})"),
            Shape::Text => format!("text_json(&{v})"),
            Shape::Json => format!("json_text(&{v})"),
            Shape::JsonMap => format!("json_map_text(&{v})"),
            Shape::Duration => format!("({v}.as_nanos().min(u128::from(u64::MAX)) as u64)"),
            Shape::Item => format!("item_w(&{v})"),
            Shape::Option(inner) => {
                let x = self.fresh("x");
                let conv = self.to_wit(inner, &x)?;
                format!("{v}.as_ref().map(|{x}| {conv})")
            }
            Shape::List(inner, _) => {
                let x = self.fresh("x");
                let conv = self.to_wit(inner, &x)?;
                format!("{v}.iter().map(|{x}| {conv}).collect::<Vec<_>>()")
            }
            Shape::Tuple(items) => {
                let mut parts = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    parts.push(self.to_wit(item, &format!("{v}.{index}"))?);
                }
                format!("({},)", parts.join(", "))
            }
            Shape::Result(ok, err) => {
                let (a, b) = (self.fresh("a"), self.fresh("b"));
                let ok = self.to_wit(ok, &a)?;
                let err = self.to_wit(err, &b)?;
                format!("match {v}.as_ref() {{ Ok({a}) => Ok({ok}), Err({b}) => Err({err}) }}")
            }
            Shape::Map(key, value, _) => {
                let (k, x) = (self.fresh("k"), self.fresh("x"));
                let key = self.to_wit(key, &k)?;
                let value = self.to_wit(value, &x)?;
                format!("{v}.iter().map(|({k}, {x})| ({key}, {value})).collect::<Vec<_>>()")
            }
            Shape::Wrap(inner, _) => self.to_wit(inner, v)?,
            Shape::Named(wit) => {
                if !self.defs.contains_key(wit) {
                    return Err(format!("unknown generated type {wit}"));
                }
                self.need_forward(wit)?;
                format!("{}(&{v})", helper_name("tw", wit))
            }
            Shape::Newtype { rust, inner, public } => {
                if *public {
                    self.to_wit(inner, &format!("{v}.0"))?
                } else if let Some((_, get, _)) = NEWTYPE_ACCESS.iter().find(|(name, _, _)| *name == rust.name) {
                    self.to_wit(inner, &format!("{v}.{get}()"))?
                } else {
                    return Err(format!("newtype {} with a private field", rust.name));
                }
            }
        })
    }

    /// An expression of type `Option<Native>` for the WIT value `v` (owned).
    fn from_wit(&mut self, shape: &Shape, v: &str) -> Res<String> {
        Ok(match shape {
            Shape::Unsupported => return Err("a type with no WIT mapping".into()),
            // The answer's own validator: a guest's float must be finite, a string and a list bounded.
            Shape::Scalar { rust } => match *rust {
                "usize" | "isize" => format!("Some({v} as {rust})"),
                "f32" => format!("finite_f32({v})"),
                "f64" => format!("finite_f64({v})"),
                _ => format!("Some({v})"),
            },
            Shape::Str(StrKind::Text) => format!("bounded_string({v})"),
            Shape::Str(StrKind::Path) => format!("Some(std::path::PathBuf::from({v}))"),
            Shape::Wide { signed } => {
                let ty = if *signed { "i128" } else { "u128" };
                format!("<[u8; 16]>::try_from({v}).ok().map({ty}::from_be_bytes)")
            }
            Shape::Glam { rust, count } => {
                let parts = (0..*count).map(|i| format!("t.{i}")).collect::<Vec<_>>().join(", ");
                let name = rust.strip_prefix("glam::").unwrap_or(rust);
                if ["Vec", "DVec", "Quat", "DQuat"].iter().any(|prefix| name.starts_with(prefix)) {
                    format!("{{ let t = {v}; Some({rust}::from_array([{parts}])).filter(|g| g.is_finite()) }}")
                } else {
                    format!("{{ let t = {v}; Some({rust}::from_array([{parts}])) }}")
                }
            }
            Shape::Handle => return Err("an opaque handle cannot be answered".into()),
            Shape::Tag => format!("nbt_tag_from(&{v})"),
            Shape::Compound => format!("nbt_compound_from(&{v})"),
            Shape::Text => format!("text_from(&{v})"),
            Shape::Json => format!("json_from(&{v})"),
            Shape::JsonMap => format!("json_map_from(&{v})"),
            Shape::Duration => format!("Some(std::time::Duration::from_nanos({v}))"),
            Shape::Item => format!("item_from(&{v})"),
            Shape::Option(inner) => {
                let x = self.fresh("x");
                let conv = self.from_wit(inner, &x)?;
                format!("match {v} {{ Some({x}) => {conv}.map(Some), None => Some(None) }}")
            }
            Shape::List(inner, kind) => {
                let x = self.fresh("x");
                let conv = self.from_wit(inner, &x)?;
                let collect = match kind {
                    ListKind::Vec => "Vec<_>".to_owned(),
                    ListKind::VecDeque => "std::collections::VecDeque<_>".to_owned(),
                    ListKind::HashSet => "std::collections::HashSet<_>".to_owned(),
                    ListKind::BTreeSet => "std::collections::BTreeSet<_>".to_owned(),
                    ListKind::BinaryHeap => "std::collections::BinaryHeap<_>".to_owned(),
                    ListKind::Array(_) => "Vec<_>".to_owned(),
                    ListKind::Slice => return Err("a slice cannot be answered".into()),
                };
                let base = format!("{v}.into_iter().map(|{x}| {conv}).collect::<Option<{collect}>>().filter(|list| list.len() <= MAX_ANSWER_LIST)");
                match kind {
                    ListKind::Array(_) => format!("{base}.and_then(|l| l.try_into().ok())"),
                    _ => base,
                }
            }
            Shape::Tuple(items) => {
                let t = self.fresh("t");
                let mut parts = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    parts.push(format!("({})?", self.from_wit(item, &format!("{t}.{index}"))?));
                }
                format!("{{ let {t} = {v}; (|| Some(({},)))() }}", parts.join(", "))
            }
            Shape::Result(ok, err) => {
                let (a, b) = (self.fresh("a"), self.fresh("b"));
                let ok = self.from_wit(ok, &a)?;
                let err = self.from_wit(err, &b)?;
                format!("match {v} {{ Ok({a}) => {ok}.map(Ok), Err({b}) => {err}.map(Err) }}")
            }
            Shape::Map(key, value, kind) => {
                let (k, x) = (self.fresh("k"), self.fresh("x"));
                let key = self.from_wit(key, &k)?;
                let value = self.from_wit(value, &x)?;
                let collect = match kind {
                    MapKind::HashMap => "std::collections::HashMap<_, _>",
                    MapKind::BTreeMap => "std::collections::BTreeMap<_, _>",
                    MapKind::Other => return Err("an unsupported map type cannot be answered".into()),
                };
                format!("{v}.into_iter().map(|({k}, {x})| Some((({key})?, ({value})?))).collect::<Option<{collect}>>().filter(|map| map.len() <= MAX_ANSWER_LIST)")
            }
            Shape::Wrap(inner, kind) => {
                let inner = self.from_wit(inner, v)?;
                match kind {
                    WrapKind::Box => format!("({inner}).map(Box::new)"),
                    WrapKind::Rc => format!("({inner}).map(std::rc::Rc::new)"),
                    WrapKind::Arc => format!("({inner}).map(std::sync::Arc::new)"),
                    WrapKind::Cow => format!("({inner}).map(std::borrow::Cow::Owned)"),
                    WrapKind::StaticRef => format!("({inner}).and_then(leak_static)"),
                    WrapKind::Ref => return Err("a borrowed value cannot be answered".into()),
                }
            }
            Shape::Named(wit) => {
                if !self.defs.contains_key(wit) {
                    return Err(format!("unknown generated type {wit}"));
                }
                self.need_reverse(wit)?;
                format!("{}({v})", helper_name("fw", wit))
            }
            Shape::Newtype { rust, inner, public } => {
                if rust.lifetimes > 0 {
                    return Err("a type with lifetimes cannot be answered".into());
                }
                let inner = self.from_wit(inner, v)?;
                if *public && rust.name == "BlockStateId" {
                    // A block state the registry does not have must never reach the world.
                    format!("({inner}).map({}).filter(|state| valid_block_state(state.0))", rust_path(rust, self.exports))
                } else if *public {
                    format!("({inner}).map({})", rust_path(rust, self.exports))
                } else if let Some((_, _, new)) = NEWTYPE_ACCESS.iter().find(|(name, _, _)| *name == rust.name) {
                    format!("({inner}).map({}::{new})", rust_path(rust, self.exports))
                } else {
                    return Err(format!("newtype {} with a private field", rust.name));
                }
            }
        })
    }
}

fn peel(shape: &Shape) -> &Shape {
    match shape {
        Shape::Wrap(inner, _) | Shape::Newtype { inner, .. } => peel(inner),
        other => other,
    }
}

/// Whether the generated Rust value holds no string or list (so wasmtime's bindings pass it by
/// value; anything else by reference).
fn plain(shape: &Shape, defs: &BTreeMap<String, NamedDef>, seen: &mut Vec<String>) -> bool {
    match shape {
        Shape::Unsupported | Shape::Scalar { .. } | Shape::Glam { .. } | Shape::Handle | Shape::Duration => true,
        Shape::Str(_) | Shape::Wide { .. } | Shape::Tag | Shape::Compound | Shape::Text | Shape::Json | Shape::JsonMap | Shape::Item => false,
        Shape::Option(inner) | Shape::Wrap(inner, _) => plain(inner, defs, seen),
        Shape::List(..) | Shape::Map(..) => false,
        Shape::Tuple(items) => items.iter().all(|s| plain(s, defs, seen)),
        Shape::Result(a, b) => plain(a, defs, seen) && plain(b, defs, seen),
        Shape::Newtype { inner, .. } => plain(inner, defs, seen),
        Shape::Named(wit) => {
            if seen.contains(wit) {
                return true;
            }
            seen.push(wit.clone());
            let answer = match defs.get(wit) {
                Some(NamedDef::Record { fields, .. }) => fields.iter().all(|f| plain(&f.shape, defs, seen)),
                Some(NamedDef::Unit { .. } | NamedDef::UnitEnum { .. }) => true,
                Some(NamedDef::Variant { cases, .. }) => cases.iter().all(|(_, _, case)| match case {
                    VCase::Unit => true,
                    VCase::Single(s) => plain(s, defs, seen),
                    VCase::Tuple(items) => items.iter().all(|s| plain(s, defs, seen)),
                    VCase::Struct { fields, .. } => fields.iter().all(|f| plain(&f.shape, defs, seen)),
                }),
                None => true,
            };
            seen.pop();
            answer
        }
    }
}

fn by_reference(shape: &Shape, defs: &BTreeMap<String, NamedDef>) -> bool {
    matches!(peel(shape), Shape::Named(_) | Shape::Item) && !plain(shape, defs, &mut Vec::new())
}

/// The expression that passes the owned WIT value `var` to a wasmtime `call_*` method: strings and
/// lists borrowed, records and variants by reference, the aliases `nbt`, `text-json` and `json` by
/// reference to the alias, scalars and enums by value.
fn borrowed(shape: &Shape, var: &str, defs: &BTreeMap<String, NamedDef>) -> Res<String> {
    Ok(match peel(shape) {
        Shape::Unsupported => return Err("a type with no WIT mapping".into()),
        Shape::Scalar { .. } | Shape::Glam { .. } | Shape::Handle | Shape::Duration => var.to_owned(),
        Shape::Str(_) => format!("{var}.as_str()"),
        Shape::Wide { .. } | Shape::List(..) | Shape::Map(..) => format!("{var}.as_slice()"),
        Shape::Tag | Shape::Compound | Shape::Text | Shape::Json | Shape::JsonMap | Shape::Item => format!("&{var}"),
        Shape::Named(_) => {
            if by_reference(shape, defs) { format!("&{var}") } else { var.to_owned() }
        }
        Shape::Option(inner) => match peel(inner) {
            Shape::Scalar { .. } | Shape::Glam { .. } | Shape::Handle | Shape::Duration => var.to_owned(),
            Shape::Str(_) | Shape::Wide { .. } | Shape::List(..) | Shape::Map(..) => format!("{var}.as_deref()"),
            Shape::Tag | Shape::Compound | Shape::Text | Shape::Json | Shape::JsonMap | Shape::Item => format!("{var}.as_ref()"),
            Shape::Named(_) => {
                if by_reference(inner, defs) {
                    format!("{var}.as_ref()")
                } else {
                    var.to_owned()
                }
            }
            Shape::Tuple(items)
                if items.iter().all(|item| {
                    matches!(
                        peel(item),
                        Shape::Scalar { .. } | Shape::Glam { .. } | Shape::Duration | Shape::Handle
                    )
                }) =>
            {
                var.to_owned()
            }
            _ => return Err("an option of a compound value is not passed yet".into()),
        },
        Shape::Tuple(items) => {
            let mut parts = Vec::new();
            for (index, item) in items.iter().enumerate() {
                parts.push(borrowed(item, &format!("{var}.{index}"), defs)?);
            }
            format!("({},)", parts.join(", "))
        }
        _ => return Err("a result value is not passed yet".into()),
    })
}

// ---------------------------------------------------------------------------------------------
// Named type helpers

fn wit_rust(wit: &str) -> String {
    format!("et::{}", camel(wit))
}

fn field_conv_forward(emitter: &mut Emitter<'_>, fields: &[Field], base: &str, tuple: bool) -> Res<String> {
    let mut text = String::new();
    for field in fields {
        let access = if tuple { format!("{base}.{}", field.rust) } else { format!("{base}.{}", field.rust) };
        let conv = emitter.to_wit(&field.shape, &access)?;
        let _ = write!(text, "{}: {conv}, ", snake(&field.wit));
    }
    Ok(text)
}

fn forward_helper(emitter: &mut Emitter<'_>, wit: &str) -> Res<String> {
    let def = emitter.defs.get(wit).ok_or("unknown type")?.clone();
    let name = helper_name("tw", wit);
    let target = wit_rust(wit);
    Ok(match def {
        NamedDef::Record { rust, fields, tuple, .. } => {
            let body = field_conv_forward(emitter, &fields, "v", tuple)?;
            format!("fn {name}(v: &{}) -> {target} {{ {target} {{ {body} }} }}\n", rust_path(&rust, emitter.exports))
        }
        NamedDef::Unit { rust, .. } => format!("fn {name}(_: &{}) -> {target} {{ {target}::Unit }}\n", rust_path(&rust, emitter.exports)),
        NamedDef::UnitEnum { rust, cases, .. } => {
            let path = rust_path(&rust, emitter.exports);
            let mut arms = String::new();
            for (rust_case, wit_case) in &cases {
                let _ = write!(arms, "{path}::{rust_case} => {target}::{}, ", camel(wit_case));
            }
            let fallback = cases.first().map(|(_, wit_case)| camel(wit_case)).unwrap_or_default();
            format!("#[allow(unreachable_patterns)] fn {name}(v: &{path}) -> {target} {{ match v {{ {arms} _ => {target}::{fallback} }} }}\n")
        }
        NamedDef::Variant { rust, cases, .. } => {
            let path = rust_path(&rust, emitter.exports);
            let mut arms = String::new();
            for (rust_case, wit_case, case) in &cases {
                let wit_variant = camel(wit_case);
                match case {
                    VCase::Unit => {
                        let _ = write!(arms, "{path}::{rust_case} {{ .. }} => {target}::{wit_variant}, ");
                    }
                    VCase::Single(shape) => {
                        let conv = emitter.to_wit(shape, "p")?;
                        let _ = write!(arms, "{path}::{rust_case}(p) => {target}::{wit_variant}({conv}), ");
                    }
                    VCase::Tuple(items) => {
                        let binds = (0..items.len()).map(|i| format!("p{i}")).collect::<Vec<_>>().join(", ");
                        let mut parts = Vec::new();
                        for (index, item) in items.iter().enumerate() {
                            parts.push(emitter.to_wit(item, &format!("p{index}"))?);
                        }
                        let _ = write!(arms, "{path}::{rust_case}({binds}) => {target}::{wit_variant}(({},)), ", parts.join(", "));
                    }
                    VCase::Struct { record, fields } => {
                        let binds = fields.iter().enumerate().map(|(i, f)| format!("{}: p{i}", f.rust)).collect::<Vec<_>>().join(", ");
                        let mut parts = String::new();
                        for (index, field) in fields.iter().enumerate() {
                            let conv = emitter.to_wit(&field.shape, &format!("p{index}"))?;
                            let _ = write!(parts, "{}: {conv}, ", snake(&field.wit));
                        }
                        let _ = write!(arms, "{path}::{rust_case} {{ {binds} }} => {target}::{wit_variant}({} {{ {parts} }}), ", wit_rust(record));
                    }
                }
            }
            format!("fn {name}(v: &{path}) -> {target} {{ match v {{ {arms} }} }}\n")
        }
    })
}

fn reverse_helper(emitter: &mut Emitter<'_>, wit: &str) -> Res<String> {
    let def = emitter.defs.get(wit).ok_or("unknown type")?.clone();
    let name = helper_name("fw", wit);
    let target = wit_rust(wit);
    Ok(match def {
        NamedDef::Record { rust, fields, tuple, .. } => {
            if rust.lifetimes > 0 {
                return Err("a type with lifetimes cannot be answered".into());
            }
            if rust.krate != "horizon-client" && rust.non_exhaustive {
                return Err("a non-exhaustive foreign struct cannot be built".into());
            }
            let mut body = String::new();
            for field in &fields {
                let conv = emitter.from_wit(&field.shape, &format!("v.{}", snake(&field.wit)))?;
                let _ = write!(body, "{}: ({conv})?, ", field.rust);
            }
            let path = rust_path(&rust, emitter.exports);
            if tuple {
                let mut parts = String::new();
                for field in &fields {
                    let conv = emitter.from_wit(&field.shape, &format!("v.{}", snake(&field.wit)))?;
                    let _ = write!(parts, "({conv})?, ");
                }
                format!("fn {name}(v: {target}) -> Option<{path}> {{ Some({path}({parts})) }}\n")
            } else {
                format!("fn {name}(v: {target}) -> Option<{path}> {{ Some({path} {{ {body} }}) }}\n")
            }
        }
        NamedDef::Unit { rust, braced, .. } => {
            if rust.lifetimes > 0 {
                return Err("a type with lifetimes cannot be answered".into());
            }
            let path = rust_path(&rust, emitter.exports);
            let value = if braced { format!("{path} {{}}") } else { path.clone() };
            format!("fn {name}(_: {target}) -> Option<{path}> {{ Some({value}) }}\n")
        }
        NamedDef::UnitEnum { rust, cases, .. } => {
            if rust.lifetimes > 0 {
                return Err("a type with lifetimes cannot be answered".into());
            }
            let path = rust_path(&rust, emitter.exports);
            let mut arms = String::new();
            for (rust_case, wit_case) in &cases {
                let _ = write!(arms, "{target}::{} => {path}::{rust_case}, ", camel(wit_case));
            }
            format!("fn {name}(v: {target}) -> Option<{path}> {{ Some(match v {{ {arms} }}) }}\n")
        }
        NamedDef::Variant { rust, cases, .. } => {
            if rust.lifetimes > 0 {
                return Err("a type with lifetimes cannot be answered".into());
            }
            let path = rust_path(&rust, emitter.exports);
            let mut arms = String::new();
            for (rust_case, wit_case, case) in &cases {
                let wit_variant = camel(wit_case);
                match case {
                    VCase::Unit => {
                        let _ = write!(arms, "{target}::{wit_variant} => {path}::{rust_case}, ");
                    }
                    VCase::Single(shape) => {
                        let conv = emitter.from_wit(shape, "p")?;
                        let _ = write!(arms, "{target}::{wit_variant}(p) => {path}::{rust_case}(({conv})?), ");
                    }
                    VCase::Tuple(items) => {
                        let mut parts = Vec::new();
                        for (index, item) in items.iter().enumerate() {
                            parts.push(format!("({})?", emitter.from_wit(item, &format!("p.{index}"))?));
                        }
                        let _ = write!(arms, "{target}::{wit_variant}(p) => {path}::{rust_case}({}), ", parts.join(", "));
                    }
                    VCase::Struct { fields, .. } => {
                        let mut parts = String::new();
                        for field in fields {
                            let conv = emitter.from_wit(&field.shape, &format!("p.{}", snake(&field.wit)))?;
                            let _ = write!(parts, "{}: ({conv})?, ", field.rust);
                        }
                        let _ = write!(arms, "{target}::{wit_variant}(p) => {path}::{rust_case} {{ {parts} }}, ");
                    }
                }
            }
            format!("fn {name}(v: {target}) -> Option<{path}> {{ Some(match v {{ {arms} }}) }}\n")
        }
    })
}

// ---------------------------------------------------------------------------------------------
// Hooks

fn const_name(hook: &str) -> String {
    format!("H_{}", hook.to_uppercase())
}

fn side_label(side: Side) -> &'static str {
    match side {
        Side::Client => "Client",
        Side::Server => "Server",
        Side::Both => "Both",
    }
}

struct Built {
    method: String,
}

/// The client mod's guest call, skipped (never made with an empty stand-in) when an NBT argument
/// failed to encode.

/// An event field is `&mut` of a non-scalar (a live object or an aggregate) rather than a value.
fn field_is_mut_ref(ty: &syn::Type) -> bool {
    matches!(ty, syn::Type::Reference(reference) if reference.mutability.is_some())
}

fn elem_name(ty: &syn::Type) -> Option<String> {
    let syn::Type::Reference(reference) = ty else { return None };
    let syn::Type::Path(path) = &*reference.elem else { return None };
    path.path.segments.last().map(|s| s.ident.to_string())
}

fn build_hook(
    emitter: &mut Emitter<'_>,
    plan: &HookPlan,
    method: &TraitHook,
    event: &EventDef,
    ordinal_const: &str,
) -> Res<Built> {
    let fields: HashMap<&str, &syn::Type> = event.fields.iter().map(|(n, t)| (n.as_str(), t)).collect();
    let name = &plan.name;
    let sig_host = match &method.host_param {
        Some((param, ty)) => format!("{param}: {ty}, "),
        None => String::new(),
    };
    let event_ty = method.event_sig.clone();
    let ret = method.ret.clone();
    // The answer's default and how a guest's answer is folded in.
    let mut body = String::new();
    if plan.name == "server_about_to_start" {
        // The server runs one world at a time and a guest keeps its state in its instance: each
        // world's server starts with a fresh one (nothing happens before the first hook ran).
        body.push_str("self.reset_server(); ");
    }
    let (default, out_kind) = match (plan.contract.as_str(), ret.as_deref()) {
        ("cancel", Some(r)) if r.trim() == "Flow" => ("Flow::Continue".to_owned(), "cancel"),
        ("replace", Some(_)) => ("None".to_owned(), "replace"),
        (_, None) => (String::new(), "none"),
        (c, Some(r)) => return Err(format!("unexpected {c} return type {r}")),
    };
    if ret.is_some() {
        let _ = write!(body, "let mut out = {default}; ");
    }
    let sides: &[&str] = match plan.side {
        Side::Client => &["client"],
        Side::Server => &["server"],
        Side::Both => &["client", "server"],
    };
    for side in sides {
        let side_enum = if *side == "client" { "Side::Client" } else { "Side::Server" };
        let mut block = String::new();
        let guard = match out_kind {
            "cancel" => " && matches!(out, Flow::Continue)",
            "replace" => " && out.is_none()",
            _ => "",
        };
        // A hook of the game's shared code runs on the client thread or the (integrated or
        // dedicated) server thread: the thread decides which module gets it.
        let routing = match (plan.side, *side) {
            (Side::Both, "server") => " && on_server_thread()",
            (Side::Both, _) => " && !on_server_thread()",
            _ => "",
        };
        let _ = write!(block, "if self.wants({side_enum}, {ordinal_const}){routing}{guard} {{ ");
        // Arguments.
        let mut call_args = Vec::new();
        for param in &plan.params {
            let Some(field_ty) = fields.get(param.field.as_str()) else {
                return Err(format!("event field {} not found", param.field));
            };
            let _ = field_ty;
            let conv = emitter.to_wit(&param.shape, &format!("event.{}", param.field))?;
            let var = format!("a_{}", ident(&param.field));
            let _ = write!(block, "let {var} = {conv}; ");
            call_args.push(borrowed(&param.shape, &var, emitter.defs)?);
        }
        let batch = plan
            .effects
            .iter()
            .find(|effect| fields.get(effect.field.as_str()).and_then(|ty| elem_name(ty)).as_deref() == Some("HudBatch"))
            .map(|effect| format!("Some(&mut *event.{})", effect.field))
            .or_else(|| {
                fields
                    .iter()
                    .find(|(_, ty)| elem_name(ty).as_deref() == Some("HudBatch") && field_is_mut_ref(ty))
                    .map(|(field, _)| format!("Some(&mut *event.{field})"))
            })
            .unwrap_or_else(|| "None".into());
        let access = match method.host {
            HostKind::Write => {
                let host = &method.host_param.as_ref().unwrap().0;
                format!("HostAccess::Write(&mut *{host})")
            }
            HostKind::Read => {
                let host = &method.host_param.as_ref().unwrap().0;
                format!("HostAccess::Read(&*{host})")
            }
            HostKind::None => "HostAccess::None".into(),
        };
        let args = if call_args.is_empty() { String::new() } else { format!(", {}", call_args.join(", ")) };
        let call = format!("call_{}", ident(&super::wit_full::kebab(name)));
        let _ = write!(
            block,
            "let r = if take_encode_failure() {{ note_encode_skip({ordinal_const}); None }} else {{ self.with_{side}({ordinal_const}, {access}, {batch}, |g| g.dispatch({name:?}, |e, s| e.yoake_mods_{side}_hooks().{call}(s{args}))) }}; "
        );
        // The answer.
        let host_ref = match method.host {
            HostKind::Write | HostKind::Read => {
                let host = &method.host_param.as_ref().unwrap().0;
                format!("Some(&*{host})")
            }
            HostKind::None => "None::<&dyn Host>".to_owned(),
        };
        let apply = apply_result(emitter, plan, &fields, out_kind, &host_ref)?;
        if !apply.is_empty() {
            let _ = write!(block, "if let Some(r) = r {{ {apply} }} ");
        } else {
            let _ = write!(block, "let _ = r; ");
        }
        block.push_str("} ");
        body.push_str(&block);
    }
    if ret.is_some() {
        body.push_str("out");
    }
    let ret_text = ret.map(|r| format!(" -> {r}")).unwrap_or_default();
    // A host the hook receives but does not pass is still used (silences unused parameters).
    let unused = method.host_param.as_ref().map(|(param, _)| format!("let _ = &{param}; ")).unwrap_or_default();
    let method_text = format!("fn {name}(&self, {sig_host}event: {event_ty}){ret_text} {{ {unused}let _ = &event; {body} }}\n");
    Ok(Built { method: method_text })
}

fn apply_result(emitter: &mut Emitter<'_>, plan: &HookPlan, fields: &HashMap<&str, &syn::Type>, out_kind: &str, host_ref: &str) -> Res<String> {
    let mut apply = String::new();
    let assign = |field: &str, value: &str, scalar: bool| -> String {
        if scalar { format!("event.{field} = {value};") } else { format!("*event.{field} = {value};") }
    };
    let apply_effect = |effect: &EffectPlan, commands: &str| -> Res<String> {
        let ty = fields.get(effect.field.as_str()).ok_or("effect field missing")?;
        let element = elem_name(ty).ok_or("effect field is not a reference")?;
        Ok(match element.as_str() {
            "HudBatch" => format!("apply_hud({host_ref}, &mut *event.{}, &{commands});", effect.field),
            "BlockEntityDraw" | "NativeItemDraw" => format!("apply_model({host_ref}, &mut *event.{}, &{commands});", effect.field),
            "Corners" => format!("apply_corners(&mut *event.{}, &{commands});", effect.field),
            "WorldDrawn" | "HandDrawn" | "WorldPrePost" | "OverlayDrawn" => format!("apply_frame_stage(&{commands});"),
            other => return Err(format!("no way to apply commands to {other}")),
        })
    };
    match &plan.result {
        ResultForm::None => {}
        ResultForm::Commands => {
            let effect = plan.effects.first().ok_or("commands without an effect")?;
            apply.push_str(&apply_effect(effect, "r")?);
        }
        ResultForm::Direct => match &plan.main {
            Main::Cancel => apply.push_str("if r { out = Flow::Cancel; }"),
            Main::Replace(shape) => {
                if !matches!(shape, Shape::Option(_)) {
                    return Err("a replace hook whose answer is not an Option".into());
                }
                let conv = emitter.from_wit(shape, "r")?;
                apply.push_str(&format!("if let Some(v) = {conv} {{ out = v; }}"));
            }
            Main::Collect { field, shape, list, scalar } => {
                if *list {
                    match emitter.from_wit(shape, "r") {
                        Ok(conv) => apply.push_str(&format!("if let Some(v) = {conv} {{ {} }}", assign(field, "v", *scalar))),
                        Err(error) if error.contains("opaque handle") || error.contains("cannot be answered") => {
                            emitter.notes.push(format!("the guest's list for `{field}` is not applied: it holds live engine objects"));
                            apply.push_str("let _ = r;");
                        }
                        Err(error) => return Err(error),
                    }
                } else if let Some(wit) = emitter.inplace(shape) {
                    emitter.need_update(&wit)?;
                    apply.push_str(&format!("if let Some(v) = r {{ let _ = {}(v, &mut *event.{field}); }}", helper_name("fu", &wit)));
                } else {
                    let inner = Shape::Option(Box::new(shape.clone()));
                    let conv = emitter.from_wit(&inner, "r")?;
                    apply.push_str(&format!("if let Some(Some(v)) = {conv} {{ {} }}", assign(field, "v", *scalar)));
                }
            }
            Main::None => return Err("a direct result without an answer".into()),
        },
        ResultForm::Outcome(_) => {
            match &plan.main {
                Main::None => {}
                Main::Cancel => apply.push_str("if r.cancel { out = Flow::Cancel; }"),
                Main::Replace(shape) => {
                    if !matches!(shape, Shape::Option(_)) {
                        return Err("a replace hook whose answer is not an Option".into());
                    }
                    let conv = emitter.from_wit(shape, "r.value")?;
                    apply.push_str(&format!("if let Some(v) = {conv} {{ out = v; }}"));
                }
                Main::Collect { field, shape, list, scalar } => {
                    if *list {
                        let conv = emitter.from_wit(shape, "r.value")?;
                        apply.push_str(&format!("if let Some(v) = {conv} {{ {} }}", assign(field, "v", *scalar)));
                    } else if let Some(wit) = emitter.inplace(shape) {
                        emitter.need_update(&wit)?;
                        apply.push_str(&format!("if let Some(v) = r.value {{ let _ = {}(v, &mut *event.{field}); }}", helper_name("fu", &wit)));
                    } else {
                        let inner = Shape::Option(Box::new(shape.clone()));
                        let conv = emitter.from_wit(&inner, "r.value")?;
                        apply.push_str(&format!("if let Some(Some(v)) = {conv} {{ {} }}", assign(field, "v", *scalar)));
                    }
                }
            }
            let single = plan.effects.len() == 1;
            let _ = single;
            for effect in &plan.effects {
                apply.push_str(&apply_effect(effect, &format!("r.{}", snake(&effect.wit_field)))?);
            }
            for extra in &plan.extras {
                match apply_extra(emitter, &mut apply, extra, &assign) {
                    Ok(()) => {}
                    Err(error) if error.contains("cannot be answered") => {
                        emitter.notes.push(format!("the guest's new `{}` is not applied: {error}", extra.field));
                    }
                    Err(error) => return Err(error),
                }
            }
        }
    }
    let _ = out_kind;
    Ok(apply)
}

fn apply_extra(emitter: &mut Emitter<'_>, apply: &mut String, extra: &ExtraPlan, assign: &dyn Fn(&str, &str, bool) -> String) -> Res<()> {
    if let Some(wit) = emitter.inplace(&extra.shape) {
        emitter.need_update(&wit)?;
        apply.push_str(&format!("if let Some(v) = r.{} {{ let _ = {}(v, &mut *event.{}); }}", snake(&extra.wit_field), helper_name("fu", &wit), extra.field));
        return Ok(());
    }
    let option = Shape::Option(Box::new(extra.shape.clone()));
    let conv = emitter.from_wit(&option, &format!("r.{}", snake(&extra.wit_field)))?;
    apply.push_str(&format!("if let Some(Some(v)) = {conv} {{ {} }}", assign(&extra.field, "v", extra.scalar)));
    Ok(())
}

/// `full_hook_names.rs` of the `yoake-wasm` crate: every catalog hook in catalog order with its
/// side, which is also the ordinal numbering of the generated adapters.
pub fn render_full_hook_names(hooks: &[(String, Side)]) -> String {
    let mut text = String::from("// @generated by horizon-mod-api-gen; do not edit.\n/// The side(s) of a catalog hook.\n#[derive(Clone, Copy, Debug, Eq, PartialEq)]\npub enum HookSide {\n    Client,\n    Server,\n    Both,\n}\n\n/// Every catalog hook in catalog order, with the sides it runs on.\npub const FULL_HOOKS: &[(&str, HookSide)] = &[\n");
    for (name, side) in hooks {
        let _ = writeln!(text, "    ({name:?}, HookSide::{}),", side_label(*side));
    }
    text.push_str("];\n");
    text
}

/// The hook names list the runtime validates subscriptions against.
pub fn generate_full_hook_names(root: &Path) -> Result<String, String> {
    let full = generate_full_wit(root)?;
    let mut hooks = full
        .plans
        .iter()
        .map(|plan| (plan.name.clone(), plan.side))
        .collect::<Vec<_>>();
    hooks.push(("config_changed".into(), Side::Both));
    Ok(render_full_hook_names(&hooks))
}

#[derive(serde::Deserialize)]
struct Deny(BTreeMap<String, String>);

fn denylist() -> Result<BTreeMap<String, String>, String> {
    let deny: Deny = serde_json::from_str(DENYLIST).map_err(|e| format!("wasm-adapter-denylist.json: {e}"))?;
    Ok(deny.0)
}

/// The names a source file re-exports with `pub use` (globs excluded).
fn mod_api_exports(source: &str) -> Result<BTreeSet<String>, String> {
    fn walk(tree: &syn::UseTree, out: &mut BTreeSet<String>) {
        match tree {
            syn::UseTree::Path(path) => walk(&path.tree, out),
            syn::UseTree::Name(name) => {
                out.insert(name.ident.to_string());
            }
            syn::UseTree::Rename(rename) => {
                out.insert(rename.rename.to_string());
            }
            syn::UseTree::Group(group) => group.items.iter().for_each(|item| walk(item, out)),
            syn::UseTree::Glob(_) => {}
        }
    }
    let file = syn::parse_file(source).map_err(|e| e.to_string())?;
    let mut out = BTreeSet::new();
    for item in &file.items {
        if let syn::Item::Use(item) = item
            && matches!(item.vis, syn::Visibility::Public(_))
        {
            walk(&item.tree, &mut out);
        }
    }
    Ok(out)
}

/// The adapters for the hooks of `plans`, whose Rust traits are `hooks_source` (`Hooks`) and
/// `render_source` (`RenderHooks`).
pub fn generate_wasm_adapters(root: &Path, hooks_source: &str, render_source: &str) -> Result<WasmAdapters, String> {
    let full = generate_full_wit(root)?;
    let api = std::fs::read_to_string(root.join("crates/horizon-client/src/mod_api.rs")).map_err(|e| e.to_string())?;
    adapters_for(&full, hooks_source, render_source, &mod_api_exports(&api)?)
}

pub(crate) fn adapters_for(full: &FullWit, hooks_source: &str, render_source: &str, exports: &BTreeSet<String>) -> Result<WasmAdapters, String> {
    let mut traits = HashMap::new();
    let mut events = HashMap::new();
    parse_trait(hooks_source, "Hooks", false, &mut traits, &mut events)?;
    parse_trait(render_source, "RenderHooks", true, &mut traits, &mut events)?;
    let deny = denylist()?;
    let mut emitter = Emitter { defs: &full.defs, exports, helpers: String::new(), forward: BTreeMap::new(), reverse: BTreeMap::new(), update: BTreeMap::new(), depth: 0, notes: Vec::new() };
    let mut hooks_impl = String::new();
    let mut render_impl = String::new();
    let mut render_ordinals: Vec<usize> = Vec::new();
    let mut wired = Vec::new();
    let mut partial = Vec::new();
    let mut unsupported = Vec::new();
    let mut ordinals = String::new();
    let mut names = Vec::new();
    for (ordinal, plan) in full.plans.iter().enumerate() {
        let constant = const_name(&plan.name);
        let _ = writeln!(ordinals, "pub(crate) const {constant}: usize = {ordinal};");
        names.push((plan.name.clone(), plan.side));
        if let Some(reason) = deny.get(&plan.name) {
            unsupported.push((plan.name.clone(), reason.clone()));
            continue;
        }
        let Some(method) = traits.get(&plan.name) else {
            unsupported.push((plan.name.clone(), "not in the generated Hooks trait".into()));
            continue;
        };
        let Some(event) = events.get(&method.event) else {
            unsupported.push((plan.name.clone(), format!("event {} not found", method.event)));
            continue;
        };
        emitter.notes.clear();
        match build_hook(&mut emitter, plan, method, event, &constant) {
            Ok(built) => {
                wired.push(plan.name.clone());
                if !emitter.notes.is_empty() {
                    partial.push((plan.name.clone(), emitter.notes.join("; ")));
                }
                if method.in_render {
                    render_ordinals.push(ordinal);
                    render_impl.push_str(&built.method);
                } else {
                    hooks_impl.push_str(&built.method);
                }
            }
            Err(reason) => unsupported.push((plan.name.clone(), reason)),
        }
    }
    let helpers = std::mem::take(&mut emitter.helpers);
    let mut code = String::from("// @generated by horizon-mod-api-gen; do not edit.\n");
    code.push_str(&ordinals);
    let _ = writeln!(code, "pub(crate) const HOOK_COUNT: usize = {};", full.plans.len());
    let unwired = unsupported.iter().map(|(name, _)| format!("{name:?}")).collect::<Vec<_>>().join(", ");
    let _ = writeln!(code, "/// Hooks with no adapter: a guest that subscribes to one is refused at load.\npub(crate) const UNWIRED: &[&str] = &[{unwired}];");
    let render_list = render_ordinals.iter().map(|ordinal| ordinal.to_string()).collect::<Vec<_>>().join(", ");
    let _ = writeln!(code, "/// The ordinals of the hooks of the render pass: a package takes part in it only when its client module subscribed to one.\npub(crate) const RENDER_HOOKS: &[usize] = &[{render_list}];");
    code.push_str(&helpers);
    let _ = writeln!(code, "impl Hooks for FullMod {{");
    code.push_str("fn render(&self) -> Option<&dyn RenderHooks> { if self.render_wanted() { Some(self) } else { None } }\n");
    code.push_str(&hooks_impl);
    code.push_str("}\n");
    let _ = writeln!(code, "impl RenderHooks for FullMod {{");
    code.push_str(&render_impl);
    code.push_str("}\n");
    syn::parse_file(&code).map_err(|e| format!("generated WASM adapters: {e}"))?;
    Ok(WasmAdapters { code, wired, unsupported, partial, hooks: names })
}

/// The adapters of the workspace as the generator's tests and the committed file see them.
#[cfg(test)]
pub(super) fn generated_for_tests() -> WasmAdapters {
    tests::generated()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    pub(super) fn generated() -> WasmAdapters {
        let sources = crate::sources(&root(), crate::FillMode::Require).expect("catalog sources");
        generate_wasm_adapters(&root(), &sources.hooks, &sources.render).expect("generate WASM adapters")
    }

    #[test]
    fn adapters_parse_and_report() {
        let adapters = generated();
        if let Some(dir) = std::env::var_os("YOAKE_ADAPTER_DUMP") {
            let dir = std::path::PathBuf::from(dir);
            std::fs::write(dir.join("wasm_full_adapters.rs"), &adapters.code).unwrap();
            let mut report = String::new();
            for (name, reason) in &adapters.unsupported {
                report.push_str(&format!("{name}: {reason}\n"));
            }
            std::fs::write(dir.join("unsupported.txt"), report).unwrap();
        }
        eprintln!("wired {} of {}", adapters.wired.len(), adapters.hooks.len());
        eprintln!("partial {:?}", adapters.partial);
        assert!(syn::parse_file(&adapters.code).is_ok());
        assert_eq!(adapters.wired.len() + adapters.unsupported.len(), adapters.hooks.len());
    }
}
