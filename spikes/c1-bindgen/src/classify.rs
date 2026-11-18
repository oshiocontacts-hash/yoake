use std::collections::HashSet;

use serde_json::Value;

use crate::model::{type_display, Category, Function};

pub fn classify(function: &mut Function, document: &Value) {
    let mut mapper = Mapper {
        document,
        self_type: function.self_type.as_ref(),
        visiting: HashSet::new(),
        reasons: Vec::new(),
        category: Category::A,
    };
    let header = function.function.get("header").unwrap_or(&Value::Null);
    for (key, reason) in [
        ("is_const", "const functions are excluded from generated bindings"),
        ("is_unsafe", "unsafe functions require an explicit safety contract"),
        ("is_async", "async functions are not supported by the synchronous ABI"),
    ] {
        if header.get(key).and_then(Value::as_bool) == Some(true) {
            mapper.fail(reason);
        }
    }
    if function
        .function
        .pointer("/sig/is_c_variadic")
        .and_then(Value::as_bool)
        == Some(true)
    {
        mapper.fail("C-variadic functions are not mappable");
    }
    let generics = function
        .function
        .get("generics")
        .and_then(|generics| generics.get("params"))
        .and_then(Value::as_array);
    if let Some(params) = generics {
        for param in params {
            let name = param.get("name").and_then(Value::as_str).unwrap_or("generic");
            if param.get("kind").and_then(Value::as_object).and_then(|kind| kind.get("lifetime")).is_some() {
                mapper.fail("functions with generic lifetimes are not mappable");
            } else if name.starts_with("impl ") {
                mapper.fail(format!("impl Trait parameter `{name}` requires a listed instantiation"));
            } else {
                mapper.fail(format!("generic parameter `{name}` requires a listed instantiation"));
            }
        }
    }

    match function.receiver.as_str() {
        "-" => {}
        "&self" | "&mut self" => {
            if function.self_type.as_ref().map(type_leaf).as_deref() != Some("Game") {
                mapper.at_least(
                    Category::B,
                    "non-Game receiver requires a generated path resolver".to_string(),
                );
                if let Some(self_type) = function.self_type.clone() {
                    mapper.map_type(&self_type, false);
                }
            }
        }
        "self" => {
            mapper.at_least(
                Category::B,
                "by-value receiver requires a copy or consume rule".to_string(),
            );
            if let Some(self_type) = function.self_type.clone() {
                mapper.map_type(&self_type, false);
            }
        }
        _ => mapper.fail("explicit or exotic receiver types are not mappable"),
    }
    for argument in &function.arguments {
        mapper.map_type(&argument.ty, false);
    }
    if let Some(output) = function.output.as_ref().filter(|output| !output.is_null()) {
        mapper.map_type(output, true);
    }
    mapper.reasons.sort();
    mapper.reasons.dedup();
    function.category = mapper.category;
    function.reasons = mapper.reasons;
}

struct Mapper<'a> {
    document: &'a Value,
    self_type: Option<&'a Value>,
    visiting: HashSet<u64>,
    reasons: Vec<String>,
    category: Category,
}

impl Mapper<'_> {
    fn at_least(&mut self, category: Category, reason: String) {
        self.category = self.category.max(category);
        self.reasons.push(reason);
    }

    fn fail(&mut self, reason: impl Into<String>) {
        self.at_least(Category::C, reason.into());
    }

    fn map_type(&mut self, ty: &Value, output: bool) {
        if ty.is_null() {
            return;
        }
        if let Some(primitive) = ty.get("primitive").and_then(Value::as_str) {
            match primitive {
                "bool" | "u8" | "u16" | "u32" | "u64" | "u128" | "i8" | "i16"
                | "i32" | "i64" | "i128" | "f32" | "f64" => {}
                "usize" | "isize" => self.at_least(
                    Category::B,
                    format!("`{primitive}` requires an explicit fixed-width ABI rule"),
                ),
                "char" => self.at_least(Category::B, "`char` requires a scalar-value codec rule".into()),
                "str" if !output => self.at_least(Category::B, "unsized `str` requires a slice rule".into()),
                _ => self.fail(format!("primitive `{primitive}` is not mappable in this position")),
            }
            return;
        }
        if let Some(generic) = ty.get("generic").and_then(Value::as_str) {
            if generic == "Self" {
                if let Some(self_type) = self.self_type.cloned() {
                    self.map_type(&self_type, output);
                } else {
                    self.fail("`Self` outside an impl has no concrete type");
                }
            } else {
                self.fail(format!("generic type `{generic}` requires a listed instantiation"));
            }
            return;
        }
        if let Some(reference) = ty.get("borrowed_ref") {
            let inner = reference.get("type").unwrap_or(&Value::Null);
            if output {
                self.fail("functions returning references must return child handles instead");
                return;
            }
            if inner.get("primitive").and_then(Value::as_str) == Some("str")
                && reference.get("is_mutable").and_then(Value::as_bool) != Some(true)
            {
                return;
            }
            self.at_least(
                Category::B,
                "borrowed non-string arguments require copy/write-back or handle rules".into(),
            );
            self.map_type(inner, false);
            return;
        }
        if let Some(array) = ty.get("array") {
            self.map_type(array.get("type").unwrap_or(&Value::Null), output);
            return;
        }
        if let Some(slice) = ty.get("slice") {
            self.at_least(Category::B, "slices require length-prefixed element rules".into());
            self.map_type(slice, output);
            return;
        }
        if let Some(items) = ty.get("tuple").and_then(Value::as_array) {
            if !items.is_empty() {
                self.at_least(Category::B, "tuples require a field-order codec rule".into());
            }
            for item in items {
                self.map_type(item, output);
            }
            return;
        }
        if let Some(path) = ty.get("resolved_path") {
            self.map_path(path, output);
            return;
        }
        if ty.get("impl_trait").is_some() {
            self.fail("impl Trait requires a listed instantiation");
        } else if ty.get("dyn_trait").is_some() {
            self.fail("dyn Trait values cannot cross this data ABI");
        } else if ty.get("fn_pointer").is_some() {
            self.fail("function pointers and closures cannot cross this data ABI");
        } else {
            self.fail(format!("unsupported rustdoc type `{}`", type_display(ty)));
        }
    }

    fn map_path(&mut self, path: &Value, output: bool) {
        let name = path.get("path").and_then(Value::as_str).unwrap_or("?");
        let leaf = name.rsplit("::").next().unwrap_or(name);
        let arguments = path
            .pointer("/args/angle_bracketed/args")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if arguments.iter().any(|argument| argument.get("lifetime").is_some()) {
            self.fail(format!("lifetime-carrying type `{name}` is not mappable"));
            return;
        }
        match leaf {
            "String" => return,
            "Vec" | "Option" | "Result" => {
                self.at_least(
                    Category::B,
                    format!("`{leaf}` requires its standard container codec rule"),
                );
                for argument in arguments.iter().filter_map(|argument| argument.get("type")) {
                    self.map_type(argument, output);
                }
                return;
            }
            _ => {}
        }
        for argument in arguments.iter().filter_map(|argument| argument.get("type")) {
            if argument.get("generic").is_some() {
                self.fail(format!("generic type `{name}` requires a listed instantiation"));
            }
        }
        let Some(id) = path.get("id").and_then(Value::as_u64) else {
            self.at_least(Category::B, format!("named type `{name}` needs a data or handle rule"));
            return;
        };
        if !self.visiting.insert(id) {
            return;
        }
        let item = self.document.pointer(&format!("/index/{id}"));
        match item.and_then(|item| item.get("inner")).and_then(Value::as_object) {
            Some(inner) if inner.contains_key("struct") => self.map_struct(name, &inner["struct"]),
            Some(inner) if inner.contains_key("enum") => self.map_enum(name, &inner["enum"]),
            Some(inner) if inner.contains_key("type_alias") => {
                if let Some(target) = inner["type_alias"].get("type") {
                    self.map_type(target, output);
                }
            }
            _ => self.at_least(Category::B, format!("named type `{name}` needs a data or handle rule")),
        }
        self.visiting.remove(&id);
    }

    fn map_struct(&mut self, name: &str, definition: &Value) {
        let params = definition
            .get("generics")
            .and_then(|generics| generics.get("params"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if !params.is_empty() {
            self.fail(format!("generic or lifetime-carrying struct `{name}` is not mappable"));
            return;
        }
        self.at_least(Category::B, format!("plain-data struct `{name}` requires a record codec"));
        let Some(fields) = definition.pointer("/kind/plain/fields").and_then(Value::as_array) else {
            self.fail(format!("struct `{name}` is not a fully visible plain-data record"));
            return;
        };
        for field in fields.iter().filter_map(Value::as_u64) {
            if let Some(ty) = self.document.pointer(&format!("/index/{field}/inner/struct_field")) {
                self.map_type(ty, false);
            }
        }
    }

    fn map_enum(&mut self, name: &str, definition: &Value) {
        self.at_least(Category::B, format!("simple enum `{name}` requires a tag codec"));
        let variants = definition.get("variants").and_then(Value::as_array).cloned().unwrap_or_default();
        for variant in variants.iter().filter_map(Value::as_u64) {
            let kind = self.document.pointer(&format!("/index/{variant}/inner/variant/kind"));
            if kind.and_then(Value::as_str) != Some("plain") {
                self.fail(format!("enum `{name}` has fields and is not a simple enum"));
            }
        }
    }
}

fn type_leaf(ty: &Value) -> String {
    ty.get("resolved_path")
        .and_then(|path| path.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .rsplit("::")
        .next()
        .unwrap_or("")
        .to_string()
}
