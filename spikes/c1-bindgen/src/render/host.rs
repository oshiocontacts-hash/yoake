use serde_json::Value;

use crate::model::Function;

pub fn host_glue(functions: &[&Function]) -> String {
    let mut output = String::from(
        "use c1_bindgen::rt::codec::{Decode, Decoder, Encode};\n\
         use c1_bindgen::rt::{CallError, PathHandle, ResolvePath, Root};\n\n",
    );
    if functions.iter().any(|function| is_game(function)) {
        output.push_str(
            "impl<'g> ResolvePath<Game<'g>> for Game<'g> {\n\
             \x20   fn resolve_path<'a>(&'a mut self, path: &PathHandle) -> Result<&'a mut Game<'g>, CallError> {\n\
             \x20       if path.root == Root::Game && path.accessors.is_empty() {\n\
             \x20           Ok(self)\n\
             \x20       } else {\n\
             \x20           Err(CallError::InvalidPath(format!(\"Game cannot resolve {:?}\", path.accessors)))\n\
             \x20       }\n\
             \x20   }\n\
             }\n\n",
        );
    }
    for function in functions {
        render_function(function, &mut output);
    }
    output
}

fn render_function(function: &Function, output: &mut String) {
    let id = function.binding_id.expect("category-a function without an id");
    output.push_str(&format!(
        "pub fn call_{id}(root: &mut Game<'_>, path: &PathHandle, args: &[u8], out: &mut Vec<u8>) -> Result<(), CallError> {{\n"
    ));
    if !is_game(function) {
        output.push_str("    let _ = (root, path);\n");
    }
    if function.arguments.is_empty() {
        output.push_str("    Decoder::new(args).finish()?;\n");
    } else {
        output.push_str("    let mut decoder = Decoder::new(args);\n");
        for argument in &function.arguments {
            output.push_str(&format!(
                "    let {} = <{} as Decode>::decode(&mut decoder)?;\n",
                argument.name,
                owned_type(&argument.ty)
            ));
        }
        output.push_str("    decoder.finish()?;\n");
    }
    if is_game(function) {
        output.push_str("    let target: &mut Game = root.resolve_path(path)?;\n");
    }
    let arguments = function
        .arguments
        .iter()
        .map(|argument| pass_argument(&argument.name, &argument.ty))
        .collect::<Vec<_>>()
        .join(", ");
    let call = if is_game(function) {
        format!("target.{}({arguments})", function.name)
    } else if function.self_type.is_some() {
        format!("{}::{}({arguments})", self_type(function), function.name)
    } else {
        format!("{}({arguments})", function.name)
    };
    if function.output.as_ref().is_some_and(|output| !output.is_null()) {
        output.push_str(&format!("    let result = {call};\n    result.encode(out);\n"));
    } else {
        output.push_str(&format!("    {call};\n"));
    }
    output.push_str("    Ok(())\n}\n\n");
}

fn is_game(function: &Function) -> bool {
    function.receiver != "-"
        && function
            .self_type
            .as_ref()
            .and_then(|ty| ty.get("resolved_path"))
            .and_then(|path| path.get("path"))
            .and_then(Value::as_str)
            .is_some_and(|path| path.rsplit("::").next() == Some("Game"))
}

fn self_type(function: &Function) -> String {
    function
        .self_type
        .as_ref()
        .and_then(|ty| ty.get("resolved_path"))
        .and_then(|path| path.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("Self")
        .rsplit("::")
        .next()
        .unwrap_or("Self")
        .to_string()
}

fn pass_argument(name: &str, ty: &Value) -> String {
    if is_str_ref(ty) {
        format!("{name}.as_str()")
    } else {
        name.to_string()
    }
}

fn owned_type(ty: &Value) -> String {
    if is_str_ref(ty) {
        return "String".to_string();
    }
    if let Some(primitive) = ty.get("primitive").and_then(Value::as_str) {
        return primitive.to_string();
    }
    if let Some(path) = ty.get("resolved_path") {
        return path.get("path").and_then(Value::as_str).unwrap_or("String").to_string();
    }
    if let Some(array) = ty.get("array") {
        return format!(
            "[{}; {}]",
            owned_type(array.get("type").unwrap_or(&Value::Null)),
            array.get("len").and_then(Value::as_str).unwrap_or("0")
        );
    }
    if ty.get("tuple").and_then(Value::as_array).is_some_and(Vec::is_empty) {
        return "()".to_string();
    }
    panic!("category-a type is not renderable: {ty}")
}

fn is_str_ref(ty: &Value) -> bool {
    ty.get("borrowed_ref")
        .and_then(|reference| reference.get("type"))
        .and_then(|inner| inner.get("primitive"))
        .and_then(Value::as_str)
        == Some("str")
}
