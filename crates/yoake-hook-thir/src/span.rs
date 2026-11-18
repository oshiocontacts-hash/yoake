use crate::model::{ParsedSpan, Span};

pub fn indentation(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

pub fn parse_span(text: &str) -> Option<(String, ParsedSpan)> {
    let start = text.find("span: ")? + 6;
    let text = &text[start..];
    let context_start = text.rfind(" (#")?;
    let context_end = text[context_start + 3..].find(')')? + context_start + 3;
    let ctxt = text[context_start + 3..context_end].parse().ok()?;
    let location = text[..context_start].trim();
    let (file, numbers) = split_file_numbers(location)?;
    let values: Vec<u32> = numbers
        .split([':', ' '])
        .filter(|part| !part.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if values.len() != 4 {
        return None;
    }
    Some((
        file.replace('\\', "/"),
        ParsedSpan {
            position: Span(values[0], values[1], values[2], values[3]),
            ctxt,
        },
    ))
}

/// `file:L1:C1: L2:C2` -> (`file`, `L1:C1: L2:C2`). The file may itself contain `:` (`C:\...`).
fn split_file_numbers(location: &str) -> Option<(String, String)> {
    let (left, right) = location.rsplit_once(": ")?;
    let (end_line, end_col) = right.split_once(':')?;
    let (rest, start_col) = left.rsplit_once(':')?;
    let (file, start_line) = rest.rsplit_once(':')?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
    if file.is_empty() || ![start_line, start_col, end_line, end_col].into_iter().all(digits) {
        return None;
    }
    Some((file.to_owned(), format!("{start_line}:{start_col}: {end_line}:{end_col}")))
}

pub fn strip_defid_hashes(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut result = String::with_capacity(text.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'['
            && index + 5 < bytes.len()
            && bytes[index + 1..index + 5]
                .iter()
                .all(|byte| byte.is_ascii_hexdigit())
            && bytes[index + 5] == b']'
        {
            index += 6;
        } else {
            result.push(bytes[index] as char);
            index += 1;
        }
    }
    result
}

pub fn defid_path(text: &str) -> Option<String> {
    let path = text.split(" ~ ").nth(1)?.strip_suffix("):")?;
    Some(strip_defid_hashes(path))
}

pub fn callee_path(ty_line: &str) -> Option<String> {
    let rest = ty_line.strip_prefix("ty: FnDef(DefId(")?;
    let path = rest.split(" ~ ").nth(1)?.split("), ").next()?;
    Some(strip_defid_hashes(path))
}
