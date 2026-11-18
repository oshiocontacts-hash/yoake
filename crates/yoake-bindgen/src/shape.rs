//! Types as the wire sees them: which engine types are plain data, which are handles, which cannot
//! cross the boundary at all.

use std::collections::{BTreeSet, HashMap};

use serde_json::Value;

use crate::doc::{ItemRef, Res, Universe};

/// A type that crosses the ABI by value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Shape {
    Unit,
    /// `bool`, fixed-width integers and floats.
    Prim(&'static str),
    /// `usize` / `isize`: 64 bits on the wire.
    Pointer(&'static str),
    Char,
    /// `String` (and `&str`, `&String`, decoded to a `String`).
    Str,
    Array(Box<Shape>, String),
    Vec(Box<Shape>),
    Option(Box<Shape>),
    Result(Box<Shape>, Box<Shape>),
    Boxed(Box<Shape>),
    /// `Arc<T>` / `Rc<T>` (decoded into a fresh one: a copy, not shared state).
    Shared(&'static str, Box<Shape>),
    /// `Arc<str>` / `Rc<str>`.
    SharedStr(&'static str),
    /// `Arc<[T]>` / `Rc<[T]>`.
    SharedSlice(&'static str, Box<Shape>),
    /// `HashMap` / `BTreeMap`: the expression that builds an empty one (`std::collections::HashMap::new()`,
    /// or `<crate::fast_hash::FxHashMap<_, _>>::default()` for an alias with another hasher).
    Map(String, Box<Shape>, Box<Shape>),
    /// `HashSet` / `BTreeSet`, same.
    Set(String, Box<Shape>),
    VecDeque(Box<Shape>),
    Tuple(Vec<Shape>),
    /// Index into [`Analyzer::defs`]: a plain-data struct or enum.
    Named(usize),
}

impl Shape {
    /// "Mappable now" without any container, record or width rule (the old category (a)).
    pub fn is_basic(&self) -> bool {
        match self {
            Shape::Unit | Shape::Prim(_) | Shape::Str => true,
            Shape::Array(inner, _) => inner.is_basic(),
            _ => false,
        }
    }

    /// A single scalar that fits an `i64` bit pattern (the `FieldTable` contract).
    pub fn scalar(&self) -> Option<&'static str> {
        match self {
            Shape::Prim(p) if !matches!(*p, "u128" | "i128") => Some(p),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Fail {
    /// Hard: the function can never be bound with this design (category (c)). Soft: it needs a
    /// handle the runtime does not have yet.
    pub hard: bool,
    pub reason: String,
}

pub fn hard(reason: impl Into<String>) -> Fail {
    Fail { hard: true, reason: reason.into() }
}

pub fn soft(reason: impl Into<String>) -> Fail {
    Fail { hard: false, reason: reason.into() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Form {
    Plain,
    Tuple,
    Unit,
}

#[derive(Clone, Debug)]
pub struct FieldDef {
    /// The field name, or the tuple index.
    pub name: String,
    pub shape: Shape,
}

#[derive(Clone, Debug)]
pub struct VariantDef {
    pub name: String,
    pub form: Form,
    pub fields: Vec<FieldDef>,
}

#[derive(Clone, Debug)]
pub enum DefBody {
    Pending,
    Struct { form: Form, fields: Vec<FieldDef> },
    Enum { variants: Vec<VariantDef> },
}

/// A plain-data struct or enum the codec can read and write field by field.
#[derive(Clone, Debug)]
pub struct NamedDef {
    pub item: ItemRef,
    /// How generated code names the type (`crate::entities::ItemEntity`).
    pub rust_path: String,
    /// A unique identifier-safe name for the codec functions.
    pub mangle: String,
    pub body: DefBody,
}

#[derive(Clone)]
enum Memo {
    InProgress(usize),
    Done(Result<usize, Fail>),
}

/// Where the glue will be included.
#[derive(Clone, Debug)]
pub struct TargetInfo {
    /// The crate the glue is compiled in.
    pub crate_name: String,
    /// `["horizon_client_lib", "entities"]`.
    pub module: Vec<String>,
    /// Struct names that are the `Game` root (`&self` / `&mut self` on them take the root).
    pub roots: Vec<String>,
    /// Setters are opt-in: type name -> the fields a guest may write. Everything else is read-only.
    pub writable: std::collections::BTreeMap<String, Vec<String>>,
    /// Types with no field accessors and no bound methods at all.
    pub frozen_types: Vec<String>,
    /// Source files that contain `unsafe`: every type defined in one is frozen.
    pub frozen_files: Vec<String>,
}

impl TargetInfo {
    pub fn plain(crate_name: &str, module: Vec<String>, roots: Vec<String>) -> Self {
        Self { crate_name: crate_name.to_string(), module, roots, writable: Default::default(), frozen_types: Vec::new(), frozen_files: Vec::new() }
    }
}

#[derive(Clone, Copy)]
pub struct SelfTy<'u> {
    pub doc: usize,
    pub ty: &'u Value,
}

pub struct Analyzer<'u> {
    pub uni: &'u Universe,
    pub target: TargetInfo,
    pub defs: Vec<NamedDef>,
    memo: HashMap<ItemRef, Memo>,
    /// External types whose crate was not loaded: they are handles until their JSON is added.
    pub unresolved: BTreeSet<String>,
    used_mangles: BTreeSet<String>,
}

impl<'u> Analyzer<'u> {
    pub fn new(uni: &'u Universe, target: TargetInfo) -> Self {
        Self { uni, target, defs: Vec::new(), memo: HashMap::new(), unresolved: BTreeSet::new(), used_mangles: BTreeSet::new() }
    }

    /// The path generated code uses for an item: the target crate is `crate`.
    pub fn rust_path(&self, r: ItemRef) -> String {
        let mut path = self.uni.doc(r).item_path(r.id);
        if path.first().map(String::as_str) == Some(self.target.crate_name.as_str()) {
            path[0] = "crate".to_string();
        }
        path.join("::")
    }

    pub fn shape(&mut self, doc: usize, ty: &'u Value, sty: Option<SelfTy<'u>>) -> Result<Shape, Fail> {
        if let Some(primitive) = ty.get("primitive").and_then(Value::as_str) {
            return match primitive {
                "bool" => Ok(Shape::Prim("bool")),
                "u8" => Ok(Shape::Prim("u8")),
                "u16" => Ok(Shape::Prim("u16")),
                "u32" => Ok(Shape::Prim("u32")),
                "u64" => Ok(Shape::Prim("u64")),
                "u128" => Ok(Shape::Prim("u128")),
                "i8" => Ok(Shape::Prim("i8")),
                "i16" => Ok(Shape::Prim("i16")),
                "i32" => Ok(Shape::Prim("i32")),
                "i64" => Ok(Shape::Prim("i64")),
                "i128" => Ok(Shape::Prim("i128")),
                "f32" => Ok(Shape::Prim("f32")),
                "f64" => Ok(Shape::Prim("f64")),
                "usize" => Ok(Shape::Pointer("usize")),
                "isize" => Ok(Shape::Pointer("isize")),
                "char" => Ok(Shape::Char),
                "str" => Err(soft("unsized `str` outside a reference")),
                other => Err(hard(format!("primitive `{other}` is not mappable"))),
            };
        }
        if let Some(generic) = ty.get("generic").and_then(Value::as_str) {
            if generic == "Self" {
                return match sty {
                    Some(s) => self.shape(s.doc, s.ty, None),
                    None => Err(hard("`Self` outside an impl has no concrete type")),
                };
            }
            return Err(hard(format!("generic type `{generic}` requires a listed instantiation")));
        }
        if ty.get("borrowed_ref").is_some() {
            return Err(soft("a reference nested inside another type needs a handle"));
        }
        if let Some(array) = ty.get("array") {
            let inner = self.shape(doc, array.get("type").unwrap_or(&Value::Null), sty)?;
            let len = array.get("len").and_then(Value::as_str).unwrap_or("0").to_string();
            return Ok(Shape::Array(Box::new(inner), len));
        }
        if ty.get("slice").is_some() {
            return Err(soft("an unsized slice outside a reference"));
        }
        if let Some(items) = ty.get("tuple").and_then(Value::as_array) {
            if items.is_empty() {
                return Ok(Shape::Unit);
            }
            let mut shapes = Vec::new();
            for item in items {
                shapes.push(self.shape(doc, item, sty)?);
            }
            return Ok(Shape::Tuple(shapes));
        }
        if let Some(path) = ty.get("resolved_path") {
            return self.path_shape(doc, path, sty);
        }
        if ty.get("impl_trait").is_some() {
            return Err(hard("`impl Trait` requires a listed instantiation"));
        }
        if ty.get("dyn_trait").is_some() {
            return Err(hard("`dyn Trait` values cannot cross the data ABI"));
        }
        if ty.get("function_pointer").is_some() {
            return Err(hard("function pointers and closures cannot cross the data ABI (guest callbacks are not built)"));
        }
        if ty.get("raw_pointer").is_some() {
            return Err(hard("raw pointers cannot cross the ABI"));
        }
        if ty.get("qualified_path").is_some() {
            return Err(hard("associated-type projection requires a listed instantiation"));
        }
        Err(hard("unsupported rustdoc type"))
    }

    fn path_shape(&mut self, doc: usize, path: &'u Value, sty: Option<SelfTy<'u>>) -> Result<Shape, Fail> {
        let name = path.get("path").and_then(Value::as_str).unwrap_or("?").to_string();
        let id = path.get("id").and_then(Value::as_u64).ok_or_else(|| soft(format!("named type `{name}` has no id")))?;
        let args = path.get("args").filter(|args| !args.is_null());
        let mut type_args: Vec<&'u Value> = Vec::new();
        if let Some(args) = args {
            if args.get("parenthesized").is_some() {
                return Err(hard(format!("`{name}` is a closure-style type")));
            }
            for arg in args.pointer("/angle_bracketed/args").and_then(Value::as_array).into_iter().flatten() {
                if arg.get("lifetime").is_some() {
                    return Err(hard(format!("lifetime-carrying type `{name}`")));
                }
                if let Some(ty) = arg.get("type") {
                    type_args.push(ty);
                } else if arg.get("const").is_some() {
                    return Err(hard(format!("const-generic type `{name}`")));
                }
            }
        }
        let uni = self.uni;
        match uni.resolve(doc, id) {
            Res::Std(std_path) => {
                let leaf = std_path.last().map(String::as_str).unwrap_or("");
                match (leaf, type_args.len()) {
                    ("String", 0) => Ok(Shape::Str),
                    ("Vec", 1) => Ok(Shape::Vec(Box::new(self.shape(doc, type_args[0], sty)?))),
                    ("Option", 1) => Ok(Shape::Option(Box::new(self.shape(doc, type_args[0], sty)?))),
                    ("Box", 1) => Ok(Shape::Boxed(Box::new(self.shape(doc, type_args[0], sty)?))),
                    ("Arc" | "Rc", 1) => {
                        let kind = if leaf == "Arc" { "std::sync::Arc" } else { "std::rc::Rc" };
                        let arg = type_args[0];
                        if arg.get("primitive").and_then(Value::as_str) == Some("str") {
                            Ok(Shape::SharedStr(kind))
                        } else if let Some(slice) = arg.get("slice") {
                            Ok(Shape::SharedSlice(kind, Box::new(self.shape(doc, slice, sty)?)))
                        } else {
                            Ok(Shape::Shared(kind, Box::new(self.shape(doc, arg, sty)?)))
                        }
                    }
                    ("HashMap" | "BTreeMap", 2) => {
                        let kind = if leaf == "HashMap" { "std::collections::HashMap::new()" } else { "std::collections::BTreeMap::new()" }.to_string();
                        let key = self.shape(doc, type_args[0], sty)?;
                        let value = self.shape(doc, type_args[1], sty)?;
                        Ok(Shape::Map(kind, Box::new(key), Box::new(value)))
                    }
                    ("HashSet" | "BTreeSet", 1) => {
                        let kind = if leaf == "HashSet" { "std::collections::HashSet::new()" } else { "std::collections::BTreeSet::new()" }.to_string();
                        Ok(Shape::Set(kind, Box::new(self.shape(doc, type_args[0], sty)?)))
                    }
                    ("VecDeque", 1) => Ok(Shape::VecDeque(Box::new(self.shape(doc, type_args[0], sty)?))),
                    ("Result", 2) => {
                        let ok = self.shape(doc, type_args[0], sty)?;
                        let err = self.shape(doc, type_args[1], sty)?;
                        Ok(Shape::Result(Box::new(ok), Box::new(err)))
                    }
                    _ => {
                        // Inspect the type arguments too, so a hard failure inside (a `dyn`, a
                        // closure) is reported as hard rather than hidden behind the container.
                        for arg in &type_args {
                            if let Err(fail) = self.shape(doc, arg, sty) {
                                if fail.hard {
                                    return Err(fail);
                                }
                            }
                        }
                        Err(soft(format!("std type `{}` has no codec yet", std_path.join("::"))))
                    }
                }
            }
            Res::Missing(missing) => {
                let joined = missing.join("::");
                self.unresolved.insert(joined.clone());
                for arg in &type_args {
                    if let Err(fail) = self.shape(doc, arg, sty) {
                        if fail.hard {
                            return Err(fail);
                        }
                    }
                }
                Err(soft(format!("external type `{joined}` (crate not loaded)")))
            }
            Res::Local(item) => {
                let kind = uni.doc(item).kind(item.id).unwrap_or("").to_string();
                match kind.as_str() {
                    "type_alias" => {
                        let alias = uni.item(item).and_then(|it| it.pointer("/inner/type_alias")).ok_or_else(|| soft("alias without a body"))?;
                        let params = alias.pointer("/generics/params").and_then(Value::as_array).map_or(0, Vec::len);
                        if params > 0 {
                            return self.generic_alias(doc, item, alias, &type_args, &name, sty);
                        }
                        let target = alias.get("type").ok_or_else(|| soft("alias without a type"))?;
                        self.shape(item.doc, target, None)
                    }
                    "struct" | "enum" => {
                        if !type_args.is_empty() {
                            return Err(hard(format!("generic type `{name}` requires a listed instantiation")));
                        }
                        self.named(item).map(Shape::Named)
                    }
                    "union" => Err(soft(format!("union `{name}` has no codec"))),
                    other => Err(soft(format!("`{name}` is a {other}, not a data type"))),
                }
            }
        }
    }

    /// `type FxHashMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>`: a map or set whose only
    /// difference from the std one is the hasher. Anything else generic stays excluded.
    fn generic_alias(&mut self, doc: usize, item: ItemRef, alias: &'u Value, use_args: &[&'u Value], name: &str, sty: Option<SelfTy<'u>>) -> Result<Shape, Fail> {
        let uni = self.uni;
        let fail = || hard(format!("generic type alias `{name}`"));
        let params: Vec<&str> = alias
            .pointer("/generics/params")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|p| p.pointer("/kind/type").is_some())
            .filter_map(|p| p.get("name").and_then(Value::as_str))
            .collect();
        let all = alias.pointer("/generics/params").and_then(Value::as_array).map_or(0, Vec::len);
        let target = alias.get("type").ok_or_else(fail)?;
        let path = target.get("resolved_path").ok_or_else(fail)?;
        let id = path.get("id").and_then(Value::as_u64).ok_or_else(fail)?;
        let Res::Std(std_path) = uni.resolve(item.doc, id) else { return Err(fail()) };
        let leaf = std_path.last().map(String::as_str).unwrap_or("");
        let wanted = match leaf {
            "HashMap" => 2,
            "HashSet" => 1,
            _ => return Err(fail()),
        };
        let target_args: Vec<&Value> = path
            .pointer("/args/angle_bracketed/args")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|a| a.get("type"))
            .collect();
        let in_order = params.len() == wanted
            && all == wanted
            && use_args.len() == wanted
            && target_args.len() > wanted
            && (0..wanted).all(|i| target_args[i].get("generic").and_then(Value::as_str) == Some(params[i]));
        if !in_order {
            return Err(fail());
        }
        let alias_path = self.rust_path(item);
        let placeholders = vec!["_"; wanted].join(", ");
        let ctor = format!("<{alias_path}<{placeholders}>>::default()");
        let key = self.shape(doc, use_args[0], sty)?;
        if wanted == 2 {
            let value = self.shape(doc, use_args[1], sty)?;
            Ok(Shape::Map(ctor, Box::new(key), Box::new(value)))
        } else {
            Ok(Shape::Set(ctor, Box::new(key)))
        }
    }

    /// The plain-data definition of a struct or enum, or why it is not data.
    pub fn named(&mut self, item: ItemRef) -> Result<usize, Fail> {
        match self.memo.get(&item) {
            Some(Memo::InProgress(index)) => return Ok(*index),
            Some(Memo::Done(result)) => return result.clone(),
            None => {}
        }
        let index = self.defs.len();
        let rust_path = self.rust_path(item);
        let mut mangle: String = rust_path.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        while !self.used_mangles.insert(mangle.clone()) {
            mangle.push('_');
        }
        self.defs.push(NamedDef { item, rust_path, mangle, body: DefBody::Pending });
        self.memo.insert(item, Memo::InProgress(index));
        let result = self.build_def(item);
        match result {
            Ok(body) => {
                self.defs[index].body = body;
                self.memo.insert(item, Memo::Done(Ok(index)));
                Ok(index)
            }
            Err(fail) => {
                self.memo.insert(item, Memo::Done(Err(fail.clone())));
                Err(fail)
            }
        }
    }

    fn build_def(&mut self, r: ItemRef) -> Result<DefBody, Fail> {
        let uni = self.uni;
        let item = uni.item(r).ok_or_else(|| soft("type without an item"))?;
        let name = item.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
        let foreign = uni.doc(r).name != self.target.crate_name;
        if foreign && item.get("visibility").and_then(Value::as_str) != Some("public") {
            return Err(soft(format!("type `{name}` is not public in its crate")));
        }
        let exhaustive_ok = !foreign || !has_attr(item, "non_exhaustive");
        if !exhaustive_ok {
            return Err(soft(format!("`{name}` is #[non_exhaustive] in another crate")));
        }
        let inner = item.get("inner").ok_or_else(|| soft("type without an inner"))?;
        if let Some(definition) = inner.get("struct") {
            self.generics_ok(&name, definition.get("generics"))?;
            return self.build_struct(r, &name, definition);
        }
        if let Some(definition) = inner.get("enum") {
            self.generics_ok(&name, definition.get("generics"))?;
            return self.build_enum(r, &name, definition);
        }
        Err(soft(format!("`{name}` is not a struct or enum")))
    }

    fn generics_ok(&self, name: &str, generics: Option<&Value>) -> Result<(), Fail> {
        let params = generics.and_then(|g| g.get("params")).and_then(Value::as_array).cloned().unwrap_or_default();
        if params.is_empty() {
            return Ok(());
        }
        if params.iter().any(|param| param.pointer("/kind/lifetime").is_some()) {
            return Err(hard(format!("lifetime-carrying type `{name}`")));
        }
        Err(hard(format!("generic type `{name}` requires a listed instantiation")))
    }

    fn field_visible(&self, owner: ItemRef, field: &Value) -> bool {
        let uni = self.uni;
        let same_crate = uni.doc(owner).name == self.target.crate_name;
        match field.get("visibility") {
            Some(Value::String(vis)) => match vis.as_str() {
                "public" => true,
                "crate" => same_crate,
                // private: visible in the defining module and below it
                _ => same_crate && self.target.module.starts_with(&uni.doc(owner).defining_module(owner.id)),
            },
            Some(Value::Object(object)) => {
                let path = object.get("restricted").and_then(|r| r.get("path")).and_then(Value::as_str).unwrap_or("");
                let mut wanted = vec![self.target.crate_name.clone()];
                wanted.extend(path.split("::").filter(|s| !s.is_empty()).map(str::to_string));
                same_crate && self.target.module.starts_with(&wanted)
            }
            _ => false,
        }
    }

    fn field_shape(&mut self, owner: ItemRef, owner_name: &str, field_id: u64, tuple_index: Option<usize>) -> Result<FieldDef, Fail> {
        let uni = self.uni;
        let field = uni.doc(owner).item(field_id).ok_or_else(|| soft(format!("field of `{owner_name}` is hidden")))?;
        let name = match tuple_index {
            Some(index) => index.to_string(),
            None => field.get("name").and_then(Value::as_str).unwrap_or("?").to_string(),
        };
        if !self.field_visible(owner, field) {
            return Err(soft(format!("field `{name}` of `{owner_name}` is private to another module")));
        }
        let ty = field.pointer("/inner/struct_field").ok_or_else(|| soft("field without a type"))?;
        let shape = self.shape(owner.doc, ty, None)?;
        Ok(FieldDef { name, shape })
    }

    fn fields_of(&mut self, owner: ItemRef, owner_name: &str, kind: &'u Value) -> Result<(Form, Vec<FieldDef>), Fail> {
        if kind.as_str() == Some("unit") {
            return Ok((Form::Unit, Vec::new()));
        }
        if let Some(tuple) = kind.get("tuple").and_then(Value::as_array) {
            let mut fields = Vec::new();
            for (index, id) in tuple.iter().enumerate() {
                let Some(id) = id.as_u64() else {
                    return Err(soft(format!("tuple struct `{owner_name}` has hidden fields")));
                };
                fields.push(self.field_shape(owner, owner_name, id, Some(index))?);
            }
            return Ok((Form::Tuple, fields));
        }
        let plain = kind.get("plain").or_else(|| kind.get("struct"));
        if let Some(plain) = plain {
            if plain.get("has_stripped_fields").and_then(Value::as_bool) == Some(true) {
                return Err(soft(format!("`{owner_name}` has private fields from another crate")));
            }
            let mut fields = Vec::new();
            for id in plain.get("fields").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64) {
                fields.push(self.field_shape(owner, owner_name, id, None)?);
            }
            return Ok((Form::Plain, fields));
        }
        Err(soft(format!("`{owner_name}` has an unknown layout")))
    }

    fn build_struct(&mut self, r: ItemRef, name: &str, definition: &'u Value) -> Result<DefBody, Fail> {
        let kind = definition.get("kind").ok_or_else(|| soft("struct without a kind"))?;
        let (form, fields) = self.fields_of(r, name, kind)?;
        Ok(DefBody::Struct { form, fields })
    }

    fn build_enum(&mut self, r: ItemRef, name: &str, definition: &'u Value) -> Result<DefBody, Fail> {
        let uni = self.uni;
        let ids: Vec<u64> = definition.get("variants").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64).collect();
        if ids.is_empty() {
            return Err(soft(format!("enum `{name}` has no variants")));
        }
        let foreign = uni.doc(r).name != self.target.crate_name;
        let mut variants = Vec::new();
        for id in ids {
            let variant = uni.doc(r).item(id).ok_or_else(|| soft("variant without an item"))?;
            let variant_name = variant.get("name").and_then(Value::as_str).unwrap_or("?").to_string();
            if foreign && has_attr(variant, "non_exhaustive") {
                return Err(soft(format!("variant `{variant_name}` is #[non_exhaustive] in another crate")));
            }
            let kind = variant.pointer("/inner/variant/kind").ok_or_else(|| soft("variant without a kind"))?;
            let owner_name = format!("{name}::{variant_name}");
            // Variant fields are as public as the enum.
            let (form, fields) = self.variant_fields(r, &owner_name, kind)?;
            variants.push(VariantDef { name: variant_name, form, fields });
        }
        Ok(DefBody::Enum { variants })
    }

    fn variant_fields(&mut self, owner: ItemRef, owner_name: &str, kind: &'u Value) -> Result<(Form, Vec<FieldDef>), Fail> {
        let uni = self.uni;
        if kind.as_str() == Some("plain") {
            return Ok((Form::Unit, Vec::new()));
        }
        let (form, ids): (Form, Vec<Option<u64>>) = if let Some(tuple) = kind.get("tuple").and_then(Value::as_array) {
            (Form::Tuple, tuple.iter().map(Value::as_u64).collect())
        } else if let Some(plain) = kind.get("struct") {
            if plain.get("has_stripped_fields").and_then(Value::as_bool) == Some(true) {
                return Err(soft(format!("`{owner_name}` has hidden fields")));
            }
            (Form::Plain, plain.get("fields").and_then(Value::as_array).into_iter().flatten().map(Value::as_u64).collect())
        } else {
            return Err(soft(format!("`{owner_name}` has an unknown layout")));
        };
        let mut fields = Vec::new();
        for (index, id) in ids.into_iter().enumerate() {
            let id = id.ok_or_else(|| soft(format!("`{owner_name}` has hidden fields")))?;
            let field = uni.doc(owner).item(id).ok_or_else(|| soft("hidden variant field"))?;
            let name = if form == Form::Tuple { index.to_string() } else { field.get("name").and_then(Value::as_str).unwrap_or("?").to_string() };
            let ty = field.pointer("/inner/struct_field").ok_or_else(|| soft("field without a type"))?;
            fields.push(FieldDef { name, shape: self.shape(owner.doc, ty, None)? });
        }
        Ok((form, fields))
    }
}

fn has_attr(item: &Value, wanted: &str) -> bool {
    item.get("attrs")
        .and_then(Value::as_array)
        .is_some_and(|attrs| attrs.iter().any(|attr| attr.as_str().is_some_and(|text| text.contains(wanted))))
}
