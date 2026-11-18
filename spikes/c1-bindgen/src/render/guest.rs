use serde_json::Value;

use crate::model::Function;

pub fn guest_stubs(functions: &[&Function]) -> String {
    let mut output = String::from(GUEST_PRELUDE);
    for function in functions {
        render_stub(function, &mut output);
    }
    output
}

fn render_stub(function: &Function, output: &mut String) {
    let id = function.binding_id.expect("category-a function without an id");
    let mut parameters = vec!["path: u64".to_string()];
    for argument in &function.arguments {
        parameters.push(format!("{}: {}", argument.name, guest_type(&argument.ty, true)));
    }
    let return_type = function
        .output
        .as_ref()
        .filter(|output| !output.is_null())
        .map(|output| guest_type(output, false))
        .unwrap_or_else(|| "()".to_string());
    output.push_str(&format!(
        "pub fn {}({}) -> Result<{}, String> {{\n    let mut args = Vec::new();\n",
        function.name,
        parameters.join(", "),
        return_type
    ));
    for argument in &function.arguments {
        output.push_str(&format!("    {}.guest_encode(&mut args);\n", argument.name));
    }
    output.push_str(&format!("    let bytes = invoke({id}, path, args)?;\n    let mut decoder = GuestDecoder::new(&bytes);\n"));
    if function.output.as_ref().is_some_and(|output| !output.is_null()) {
        output.push_str("    let result = GuestDecode::guest_decode(&mut decoder)?;\n");
    } else {
        output.push_str("    let result = ();\n");
    }
    output.push_str("    decoder.finish()?;\n    Ok(result)\n}\n\n");
}

fn guest_type(ty: &Value, argument: bool) -> String {
    if let Some(primitive) = ty.get("primitive").and_then(Value::as_str) {
        return primitive.to_string();
    }
    if let Some(reference) = ty.get("borrowed_ref") {
        let inner = reference.get("type").unwrap_or(&Value::Null);
        if inner.get("primitive").and_then(Value::as_str) == Some("str") {
            return if argument { "&str" } else { "String" }.to_string();
        }
        return format!("&{}", guest_type(inner, argument));
    }
    if let Some(path) = ty.get("resolved_path") {
        return path.get("path").and_then(Value::as_str).unwrap_or("String").to_string();
    }
    if let Some(array) = ty.get("array") {
        return format!(
            "[{}; {}]",
            guest_type(array.get("type").unwrap_or(&Value::Null), false),
            array.get("len").and_then(Value::as_str).unwrap_or("0")
        );
    }
    if ty.get("tuple").and_then(Value::as_array).is_some_and(Vec::is_empty) {
        return "()".to_string();
    }
    panic!("category-a type is not renderable: {ty}")
}

const GUEST_PRELUDE: &str = r#"#[link(wasm_import_module = "env")]
extern "C" {
    fn engine_call(id: u32, ptr: *mut u8, len: usize) -> i32;
}

const OUTPUT_CAPACITY: usize = 4096;

fn invoke(id: u32, path: u64, args: Vec<u8>) -> Result<Vec<u8>, String> {
    let args_len = u32::try_from(args.len()).map_err(|_| "arguments are too large".to_string())?;
    let mut frame = Vec::new();
    frame.extend_from_slice(&path.to_le_bytes());
    frame.extend_from_slice(&args_len.to_le_bytes());
    frame.extend_from_slice(&args);
    frame.resize(frame.len() + OUTPUT_CAPACITY, 0);
    let status = unsafe { engine_call(id, frame.as_mut_ptr(), frame.len()) };
    if status != 0 {
        return Err(format!("engine call {id} failed with status {status}"));
    }
    if frame.len() < 4 {
        return Err("engine returned a short frame".to_string());
    }
    let out_len = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
    let end = 4usize.checked_add(out_len).ok_or("engine result length overflow")?;
    if end > frame.len() {
        return Err("engine result exceeds its frame".to_string());
    }
    Ok(frame[4..end].to_vec())
}

trait GuestEncode {
    fn guest_encode(&self, out: &mut Vec<u8>);
}

trait GuestDecode: Sized {
    fn guest_decode(input: &mut GuestDecoder<'_>) -> Result<Self, String>;
}

struct GuestDecoder<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> GuestDecoder<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn take(&mut self, len: usize) -> Result<&'a [u8], String> {
        let end = self.position.checked_add(len).ok_or("decode length overflow")?;
        let result = self.bytes.get(self.position..end).ok_or("short engine result")?;
        self.position = end;
        Ok(result)
    }

    fn finish(&self) -> Result<(), String> {
        if self.position == self.bytes.len() {
            Ok(())
        } else {
            Err("engine result has trailing bytes".to_string())
        }
    }
}

macro_rules! guest_numbers {
    ($($ty:ty),* $(,)?) => {$ (
        impl GuestEncode for $ty {
            fn guest_encode(&self, out: &mut Vec<u8>) {
                out.extend_from_slice(&self.to_le_bytes());
            }
        }
        impl GuestDecode for $ty {
            fn guest_decode(input: &mut GuestDecoder<'_>) -> Result<Self, String> {
                let bytes = input.take(std::mem::size_of::<$ty>())?;
                Ok(<$ty>::from_le_bytes(bytes.try_into().unwrap()))
            }
        }
    )* };
}

guest_numbers!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);

impl GuestEncode for bool {
    fn guest_encode(&self, out: &mut Vec<u8>) {
        out.push(u8::from(*self));
    }
}

impl GuestDecode for bool {
    fn guest_decode(input: &mut GuestDecoder<'_>) -> Result<Self, String> {
        match u8::guest_decode(input)? {
            0 => Ok(false),
            1 => Ok(true),
            tag => Err(format!("invalid bool tag {tag}")),
        }
    }
}

impl GuestEncode for str {
    fn guest_encode(&self, out: &mut Vec<u8>) {
        let len = u32::try_from(self.len()).expect("string is too large");
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(self.as_bytes());
    }
}

impl GuestEncode for String {
    fn guest_encode(&self, out: &mut Vec<u8>) {
        self.as_str().guest_encode(out);
    }
}

impl GuestDecode for String {
    fn guest_decode(input: &mut GuestDecoder<'_>) -> Result<Self, String> {
        let len = u32::guest_decode(input)? as usize;
        let bytes = input.take(len)?;
        Ok(std::str::from_utf8(bytes).map_err(|_| "engine returned invalid UTF-8")?.to_owned())
    }
}

impl<T: GuestEncode + ?Sized> GuestEncode for &T {
    fn guest_encode(&self, out: &mut Vec<u8>) {
        (*self).guest_encode(out);
    }
}

impl<T: GuestEncode, const N: usize> GuestEncode for [T; N] {
    fn guest_encode(&self, out: &mut Vec<u8>) {
        for item in self {
            item.guest_encode(out);
        }
    }
}

impl<T: GuestDecode, const N: usize> GuestDecode for [T; N] {
    fn guest_decode(input: &mut GuestDecoder<'_>) -> Result<Self, String> {
        let items = (0..N)
            .map(|_| T::guest_decode(input))
            .collect::<Result<Vec<_>, _>>()?;
        items.try_into().map_err(|_| "wrong array length".to_string())
    }
}

impl GuestEncode for () {
    fn guest_encode(&self, _out: &mut Vec<u8>) {}
}

impl GuestDecode for () {
    fn guest_decode(_input: &mut GuestDecoder<'_>) -> Result<Self, String> {
        Ok(())
    }
}

"#;
