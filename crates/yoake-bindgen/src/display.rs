//! A readable text form of a rustdoc type (signatures in the lock file and the reports).

use serde_json::Value;

pub fn type_display(ty: &Value) -> String {
    if let Some(value) = ty.get("primitive").and_then(Value::as_str) {
        return value.to_string();
    }
    if let Some(value) = ty.get("generic").and_then(Value::as_str) {
        return value.to_string();
    }
    if let Some(path) = ty.get("resolved_path") {
        let name = path.get("path").and_then(Value::as_str).unwrap_or("?");
        let mut rendered = Vec::new();
        for argument in path.pointer("/args/angle_bracketed/args").and_then(Value::as_array).into_iter().flatten() {
            if let Some(inner) = argument.get("type") {
                rendered.push(type_display(inner));
            } else if let Some(lifetime) = argument.get("lifetime").and_then(Value::as_str) {
                rendered.push(lifetime.to_string());
            }
        }
        if rendered.is_empty() {
            return name.to_string();
        }
        return format!("{name}<{}>", rendered.join(", "));
    }
    if let Some(reference) = ty.get("borrowed_ref") {
        let mutable = reference.get("is_mutable").and_then(Value::as_bool) == Some(true);
        return format!("&{}{}", if mutable { "mut " } else { "" }, type_display(reference.get("type").unwrap_or(&Value::Null)));
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
    if ty.get("function_pointer").is_some() {
        return "fn(..)".to_string();
    }
    "?".to_string()
}
