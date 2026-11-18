use std::collections::HashMap;
use std::error::Error;

use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Category {
    A,
    B,
    C,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Self::A => "a",
            Self::B => "b",
            Self::C => "c",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Argument {
    pub name: String,
    pub ty: Value,
}

#[derive(Clone, Debug)]
pub struct Function {
    pub rustdoc_id: u64,
    pub name: String,
    pub full_path: String,
    pub module_path: String,
    pub span: String,
    pub visibility: String,
    pub receiver: String,
    pub self_type: Option<Value>,
    pub arguments: Vec<Argument>,
    pub output: Option<Value>,
    pub function: Value,
    pub category: Category,
    pub reasons: Vec<String>,
    pub binding_id: Option<u32>,
}

struct ImplInfo {
    self_type: Value,
    trait_path: Option<String>,
}

pub fn extract_functions(document: &Value, suffixes: &[String]) -> Result<Vec<Function>, Box<dyn Error>> {
    let index = document
        .get("index")
        .and_then(Value::as_object)
        .ok_or("rustdoc JSON has no index object")?;
    let mut impls: HashMap<u64, ImplInfo> = HashMap::new();
    for item in index.values() {
        let Some(implementation) = item.pointer("/inner/impl") else {
            continue;
        };
        let Some(self_type) = implementation.get("for") else {
            continue;
        };
        for child in implementation
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = child.as_u64() {
                impls.insert(
                    id,
                    ImplInfo {
                        self_type: self_type.clone(),
                        trait_path: implementation
                            .get("trait")
                            .and_then(Value::as_object)
                            .and_then(|path| path.get("path"))
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    },
                );
            }
        }
    }

    let normalized: Vec<String> = suffixes
        .iter()
        .map(|suffix| suffix.replace('\\', "/"))
        .collect();
    let mut functions = Vec::new();
    for (id, item) in index {
        let Some(function) = item.pointer("/inner/function") else {
            continue;
        };
        let Some(span) = item
            .get("span")
            .and_then(|span| span.get("filename"))
            .and_then(Value::as_str)
        else {
            continue;
        };
        let span = span.replace('\\', "/");
        if !normalized.iter().any(|suffix| span.ends_with(suffix)) {
            continue;
        }
        let rustdoc_id = id.parse()?;
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .ok_or("function without a name")?
            .to_string();
        let module_path = module_path(&span);
        let implementation = impls.get(&rustdoc_id);
        let self_type = implementation.map(|info| info.self_type.clone());
        let type_path = self_type.as_ref().map(type_display);
        let full_path = match (type_path, implementation.and_then(|info| info.trait_path.as_ref())) {
            (Some(self_type), Some(trait_path)) => {
                format!("{module_path}::<{self_type} as {trait_path}>::{name}")
            }
            (Some(self_type), None) => format!("{module_path}::{self_type}::{name}"),
            (None, _) => format!("{module_path}::{name}"),
        };
        let inputs = function
            .pointer("/sig/inputs")
            .and_then(Value::as_array)
            .ok_or("function without sig.inputs")?;
        let receiver = inputs
            .first()
            .filter(|input| input.get(0).and_then(Value::as_str) == Some("self"))
            .map(|input| receiver(input.get(1).unwrap_or(&Value::Null)));
        let arguments = inputs
            .iter()
            .skip(usize::from(receiver.is_some()))
            .map(|input| {
                Ok(Argument {
                    name: input
                        .get(0)
                        .and_then(Value::as_str)
                        .unwrap_or("arg")
                        .to_string(),
                    ty: input.get(1).cloned().unwrap_or(Value::Null),
                })
            })
            .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
        functions.push(Function {
            rustdoc_id,
            name,
            full_path,
            module_path,
            span: span.clone(),
            visibility: visibility(item.get("visibility").unwrap_or(&Value::Null)),
            receiver: receiver.unwrap_or_else(|| "-".to_string()),
            self_type,
            arguments,
            output: function.pointer("/sig/output").cloned(),
            function: function.clone(),
            category: Category::A,
            reasons: Vec::new(),
            binding_id: None,
        });
    }
    Ok(functions)
}

fn module_path(span: &str) -> String {
    let source = span.split("/src/").nth(1).unwrap_or(span);
    let mut parts: Vec<&str> = source.trim_end_matches(".rs").split('/').collect();
    if matches!(parts.last(), Some(&"mod" | &"lib" | &"main")) {
        parts.pop();
    }
    parts.join("::")
}

fn receiver(ty: &Value) -> String {
    if let Some(reference) = ty.get("borrowed_ref") {
        if reference.get("is_mutable").and_then(Value::as_bool) == Some(true) {
            "&mut self".to_string()
        } else {
            "&self".to_string()
        }
    } else if ty.get("generic").and_then(Value::as_str) == Some("Self") {
        "self".to_string()
    } else {
        "other self".to_string()
    }
}

fn visibility(value: &Value) -> String {
    match value {
        Value::String(visibility) => visibility.clone(),
        Value::Object(object) => object
            .get("restricted")
            .and_then(|restricted| restricted.get("path"))
            .and_then(Value::as_str)
            .map(|path| format!("restricted({path})"))
            .unwrap_or_else(|| "restricted".to_string()),
        _ => "unknown".to_string(),
    }
}

pub fn type_display(ty: &Value) -> String {
    if let Some(value) = ty.get("primitive").and_then(Value::as_str) {
        return value.to_string();
    }
    if let Some(value) = ty.get("generic").and_then(Value::as_str) {
        return value.to_string();
    }
    if let Some(path) = ty.get("resolved_path") {
        let name = path.get("path").and_then(Value::as_str).unwrap_or("?");
        if let Some(arguments) = path
            .pointer("/args/angle_bracketed/args")
            .and_then(Value::as_array)
        {
            let rendered: Vec<String> = arguments
                .iter()
                .filter_map(|argument| argument.get("type").map(type_display))
                .collect();
            if !rendered.is_empty() {
                return format!("{name}<{}>", rendered.join(", "));
            }
        }
        return name.to_string();
    }
    if let Some(reference) = ty.get("borrowed_ref") {
        let mutable = reference.get("is_mutable").and_then(Value::as_bool) == Some(true);
        return format!(
            "&{}{}",
            if mutable { "mut " } else { "" },
            type_display(reference.get("type").unwrap_or(&Value::Null))
        );
    }
    if let Some(array) = ty.get("array") {
        return format!(
            "[{}; {}]",
            type_display(array.get("type").unwrap_or(&Value::Null)),
            array.get("len").and_then(Value::as_str).unwrap_or("?")
        );
    }
    if let Some(items) = ty.get("tuple").and_then(Value::as_array) {
        return format!("({})", items.iter().map(type_display).collect::<Vec<_>>().join(", "));
    }
    if let Some(inner) = ty.get("slice") {
        return format!("[{}]", type_display(inner));
    }
    if ty.get("impl_trait").is_some() {
        return "impl Trait".to_string();
    }
    if ty.get("dyn_trait").is_some() {
        return "dyn Trait".to_string();
    }
    "unknown".to_string()
}
