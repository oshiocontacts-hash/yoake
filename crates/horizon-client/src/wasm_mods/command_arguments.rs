//! Custom Brigadier-style command argument types registered by `server-full` WASM mods (REM-17d).
//!
//! A server mod registers argument types during `init` via `register-command-argument-types`
//! (`server-services`), namespaced automatically as `<modid>:<name>`, and uses them in command
//! usages as `<arg:modid:name>` or `[arg:modid:name]`. When parsing a command input or requesting
//! tab completions, the host invokes the owning mod's `handle-call` export with caller
//! `"yoake:engine"` and method [`METHOD_PARSE`] or [`METHOD_SUGGEST`].

use super::server_services::endpoint;
use super::{ClientExtras, HostAccess, Invocation, Scope};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};

/// Method name passed to `handle-call` when parsing and normalizing a custom argument value.
pub(crate) const METHOD_PARSE: &str = "yoake:command-argument/parse";
/// Method name passed to `handle-call` when fetching tab-completion candidates.
pub(crate) const METHOD_SUGGEST: &str = "yoake:command-argument/suggest";
const CALLER: &str = "yoake:engine";
const WIRE_VERSION: u8 = 1;

pub(crate) const MAX_TYPES_PER_MOD: usize = 64;
pub(crate) const MAX_TOTAL_TYPES: usize = 512;
pub(crate) const MAX_NAME_LEN: usize = 64;
pub(crate) const MAX_SUGGESTIONS: usize = 64;
pub(crate) const MAX_SUGGESTION_LEN: usize = 128;
pub(crate) const MAX_PARSED_VALUE_LEN: usize = 4096;
pub(crate) const MAX_ERROR_LEN: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegisteredArgType {
    pub(crate) owner: String,
    pub(crate) greedy: bool,
}

#[derive(Default)]
struct Registry {
    types: BTreeMap<String, RegisteredArgType>,
    counts: BTreeMap<String, usize>,
}

static TABLE: Mutex<Registry> = Mutex::new(Registry {
    types: BTreeMap::new(),
    counts: BTreeMap::new(),
});
static ANY: AtomicBool = AtomicBool::new(false);
static BUSY_WARNED: AtomicBool = AtomicBool::new(false);
static FAILED_WARNED: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = Mutex::new(());

fn table() -> MutexGuard<'static, Registry> {
    TABLE.lock().unwrap_or_else(|error| error.into_inner())
}

/// Whether any server mod has registered a custom command argument type (single atomic load).
pub(crate) fn any() -> bool {
    ANY.load(Ordering::Relaxed)
}

/// Clears all registered custom argument types (tests).
#[cfg(test)]
pub(crate) fn reset() {
    let mut guard = table();
    guard.types.clear();
    guard.counts.clear();
    ANY.store(false, Ordering::Release);
}

fn valid_mod_id(owner: &str) -> bool {
    !owner.is_empty()
        && owner.len() <= MAX_NAME_LEN
        && owner
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-' | b'.'))
}

fn valid_type_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'))
}

/// Registers custom command argument types for `owner`, namespacing each as `<owner>:<name>`.
pub(crate) fn register_types(owner: &str, entries: &[(String, bool)]) -> Result<(), String> {
    if !valid_mod_id(owner) {
        return Err(format!("invalid mod id `{owner:.64}`"));
    }
    if entries.len() > MAX_TYPES_PER_MOD {
        return Err(format!(
            "a package registers at most {MAX_TYPES_PER_MOD} command argument types"
        ));
    }
    let mut staged: Vec<(String, RegisteredArgType)> = Vec::with_capacity(entries.len());
    for (name, greedy) in entries {
        if !valid_type_name(name) {
            return Err(format!(
                "invalid command argument type name `{name:.64}` (expected 1..={MAX_NAME_LEN} lowercase ASCII letters, digits, `_` or `-`)"
            ));
        }
        let id = format!("{owner}:{name}");
        if let Some((_, prev)) = staged.iter().find(|(existing, _)| existing == &id) {
            if prev.greedy != *greedy {
                return Err(format!(
                    "command argument type `{id}` declared with conflicting greedy flags"
                ));
            }
            continue;
        }
        staged.push((
            id,
            RegisteredArgType {
                owner: owner.to_owned(),
                greedy: *greedy,
            },
        ));
    }

    let mut guard = table();
    let existing_count = guard.counts.get(owner).copied().unwrap_or(0);
    let mut new_for_owner = 0usize;
    for (id, entry) in &staged {
        if let Some(prev) = guard.types.get(id) {
            if prev.owner != owner {
                return Err(format!(
                    "command argument type `{id}` belongs to another package"
                ));
            }
        } else {
            new_for_owner += 1;
        }
        let _ = entry;
    }
    if existing_count + new_for_owner > MAX_TYPES_PER_MOD {
        return Err(format!(
            "a package registers at most {MAX_TYPES_PER_MOD} command argument types"
        ));
    }
    if guard.types.len() + new_for_owner > MAX_TOTAL_TYPES {
        return Err(format!(
            "too many command argument types registered (limit {MAX_TOTAL_TYPES})"
        ));
    }
    for (id, entry) in staged {
        guard.types.insert(id, entry);
    }
    *guard.counts.entry(owner.to_owned()).or_insert(0) += new_for_owner;
    ANY.store(!guard.types.is_empty(), Ordering::Release);
    Ok(())
}

/// Looks up a registered `<modid>:<name>` custom argument type.
pub(crate) fn lookup_type(type_id: &str) -> Option<RegisteredArgType> {
    if !any() {
        return None;
    }
    table().types.get(type_id).cloned()
}

fn push_str(buf: &mut Vec<u8>, text: &str) {
    let bytes = &text.as_bytes()[..text.len().min(usize::from(u16::MAX))];
    buf.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn read_str<'a>(slice: &mut &'a [u8]) -> Option<String> {
    if slice.len() < 2 {
        return None;
    }
    let len = usize::from(u16::from_le_bytes([slice[0], slice[1]]));
    *slice = &slice[2..];
    if slice.len() < len {
        return None;
    }
    let (head, tail) = slice.split_at(len);
    *slice = tail;
    std::str::from_utf8(head).ok().map(str::to_owned)
}

fn read_u8(slice: &mut &[u8]) -> Option<u8> {
    let (&first, rest) = slice.split_first()?;
    *slice = rest;
    Some(first)
}

fn read_u16(slice: &mut &[u8]) -> Option<u16> {
    if slice.len() < 2 {
        return None;
    }
    let v = u16::from_le_bytes([slice[0], slice[1]]);
    *slice = &slice[2..];
    Some(v)
}

pub(crate) fn encode_request(type_id: &str, text: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(1 + 4 + type_id.len() + text.len());
    out.push(WIRE_VERSION);
    push_str(&mut out, type_id);
    push_str(&mut out, text);
    out
}

fn sanitize_single_line(text: &str, max_bytes: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        let mapped = if ch.is_control() { ' ' } else { ch };
        if out.len() + mapped.len_utf8() > max_bytes {
            break;
        }
        out.push(mapped);
    }
    out
}

pub(crate) fn decode_parse_response(mut bytes: &[u8]) -> Option<Result<String, String>> {
    if read_u8(&mut bytes)? != WIRE_VERSION {
        return None;
    }
    let status = read_u8(&mut bytes)?;
    let text = read_str(&mut bytes)?;
    if !bytes.is_empty() {
        return None;
    }
    match status {
        0 => {
            if text.len() > MAX_PARSED_VALUE_LEN || text.chars().any(char::is_control) {
                return None;
            }
            Some(Ok(text))
        }
        1 => {
            let clean = sanitize_single_line(text.trim(), MAX_ERROR_LEN);
            let message = if clean.is_empty() {
                "invalid command argument".to_owned()
            } else {
                clean
            };
            Some(Err(message))
        }
        _ => None,
    }
}

pub(crate) fn decode_suggest_response(mut bytes: &[u8]) -> Option<Vec<String>> {
    if read_u8(&mut bytes)? != WIRE_VERSION {
        return None;
    }
    let count = usize::from(read_u16(&mut bytes)?);
    let mut out = Vec::with_capacity(count.min(MAX_SUGGESTIONS));
    for _ in 0..count {
        let raw = read_str(&mut bytes)?;
        if out.len() < MAX_SUGGESTIONS {
            let clean = sanitize_single_line(&raw, MAX_SUGGESTION_LEN);
            if !clean.is_empty() && !out.contains(&clean) {
                out.push(clean);
            }
        }
    }
    if !bytes.is_empty() {
        return None;
    }
    Some(out)
}

fn invoke_owner(
    owner: &str,
    method: &str,
    payload: &[u8],
    host: HostAccess<'_>,
) -> Option<Result<Vec<u8>, String>> {
    let slot = endpoint(owner)?;
    let mut state = match slot.state.try_lock() {
        Ok(state) => state,
        Err(std::sync::TryLockError::WouldBlock) => {
            if !BUSY_WARNED.swap(true, Ordering::Relaxed) {
                log::warn!(
                    "wasm mod {owner}: {method} dropped, the module was busy (further drops logged at debug level)"
                );
            } else {
                log::debug!("wasm mod {owner}: {method} dropped, the module was busy");
            }
            return None;
        }
        Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
    };
    let module = state.module.as_mut()?;
    slot.dirty.store(true, Ordering::Relaxed);
    let mut invocation = Invocation {
        host,
        batch: None,
        batch_start: 0,
        client: ClientExtras::default(),
    };
    let _scope = Scope::enter(&mut invocation);
    module.handle_call(CALLER, method, payload)
}

fn log_failure(owner: &str, method: &str, detail: &str) {
    if !FAILED_WARNED.swap(true, Ordering::Relaxed) {
        log::warn!(
            "wasm mod {owner}: {method} failed: {detail:.200} (further failures logged at debug level)"
        );
    } else {
        log::debug!("wasm mod {owner}: {method} failed: {detail:.200}");
    }
}

/// Calls the owning mod to parse and normalize a custom command argument value.
pub(crate) fn parse_argument(
    type_id: &str,
    input: &str,
    host: HostAccess<'_>,
) -> Result<String, String> {
    let Some(entry) = lookup_type(type_id) else {
        return Err(format!("unknown custom argument type `{type_id}`"));
    };
    let payload = encode_request(type_id, input);
    let Some(answer) = invoke_owner(&entry.owner, METHOD_PARSE, &payload, host) else {
        return Err(format!("command argument parser `{type_id}` is unavailable"));
    };
    match answer {
        Ok(bytes) if bytes.len() <= super::server_services::MAX_BLOB => {
            decode_parse_response(&bytes).unwrap_or_else(|| {
                log_failure(&entry.owner, METHOD_PARSE, "malformed parse response");
                Err(format!("command argument parser `{type_id}` failed"))
            })
        }
        Ok(_) => {
            log_failure(&entry.owner, METHOD_PARSE, "response exceeded size limit");
            Err(format!("command argument parser `{type_id}` failed"))
        }
        Err(error) => {
            let clean = sanitize_single_line(error.trim(), MAX_ERROR_LEN);
            Err(if clean.is_empty() {
                format!("invalid `{type_id}` argument")
            } else {
                clean
            })
        }
    }
}

/// Calls the owning mod to collect bounded tab-completion suggestions for a custom argument.
pub(crate) fn suggest_argument(
    type_id: &str,
    partial: &str,
    host: HostAccess<'_>,
) -> Vec<String> {
    let Some(entry) = lookup_type(type_id) else {
        return Vec::new();
    };
    let payload = encode_request(type_id, partial);
    let Some(answer) = invoke_owner(&entry.owner, METHOD_SUGGEST, &payload, host) else {
        return Vec::new();
    };
    match answer {
        Ok(bytes) if bytes.len() <= super::server_services::MAX_BLOB => {
            decode_suggest_response(&bytes).unwrap_or_else(|| {
                log_failure(&entry.owner, METHOD_SUGGEST, "malformed suggest response");
                Vec::new()
            })
        }
        Ok(_) => {
            log_failure(&entry.owner, METHOD_SUGGEST, "response exceeded size limit");
            Vec::new()
        }
        Err(error) => {
            log_failure(&entry.owner, METHOD_SUGGEST, &error);
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_namespaces_validates_and_enforces_limits() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        assert!(!any());

        // Rejects bad names (empty, colon, uppercase, whitespace, overlong).
        assert!(register_types("cmdmod", &[("".into(), false)]).is_err());
        assert!(register_types("cmdmod", &[("a:b".into(), false)]).is_err());
        assert!(register_types("cmdmod", &[("Color".into(), false)]).is_err());
        assert!(register_types("cmdmod", &[("two words".into(), false)]).is_err());
        assert!(register_types("cmdmod", &[("x".repeat(MAX_NAME_LEN + 1), false)]).is_err());
        assert!(register_types("bad mod", &[("ok".into(), false)]).is_err());
        assert!(!any());

        // Valid registration namespaces as `<modid>:<name>`.
        register_types("cmdmod", &[("color".into(), false), ("note".into(), true)]).unwrap();
        assert!(any());
        assert_eq!(
            lookup_type("cmdmod:color"),
            Some(RegisteredArgType {
                owner: "cmdmod".into(),
                greedy: false,
            })
        );
        assert_eq!(
            lookup_type("cmdmod:note"),
            Some(RegisteredArgType {
                owner: "cmdmod".into(),
                greedy: true,
            })
        );
        assert!(lookup_type("other:color").is_none());

        // Idempotent repeat is accepted; conflicting greedy flag inside batch is rejected.
        register_types("cmdmod", &[("color".into(), false)]).unwrap();
        assert!(
            register_types(
                "cmdmod",
                &[("dup".into(), false), ("dup".into(), true)]
            )
            .is_err()
        );

        // Enforces per-mod cap.
        let too_many: Vec<(String, bool)> = (0..=MAX_TYPES_PER_MOD)
            .map(|i| (format!("t{i}"), false))
            .collect();
        assert!(register_types("capmod", &too_many).is_err());
        reset();
    }

    #[test]
    fn wire_parse_and_suggest_responses_validate_and_bound() {
        let mut ok = vec![WIRE_VERSION, 0];
        push_str(&mut ok, "normalized_value");
        assert_eq!(decode_parse_response(&ok), Some(Ok("normalized_value".into())));

        let mut err = vec![WIRE_VERSION, 1];
        push_str(&mut err, "bad\ninput");
        assert_eq!(decode_parse_response(&err), Some(Err("bad input".into())));

        // Control chars in normalized output or trailing garbage are rejected.
        let mut bad_ok = vec![WIRE_VERSION, 0];
        push_str(&mut bad_ok, "a\nb");
        assert_eq!(decode_parse_response(&bad_ok), None);

        let mut too_long = vec![WIRE_VERSION, 0];
        push_str(&mut too_long, &"x".repeat(MAX_PARSED_VALUE_LEN + 1));
        assert_eq!(decode_parse_response(&too_long), None);

        // Suggestions count, length, empty filtering and deduplication.
        let total = MAX_SUGGESTIONS + 10;
        let mut sug = vec![WIRE_VERSION];
        sug.extend_from_slice(&(total as u16).to_le_bytes());
        for i in 0..total {
            if i == 0 {
                push_str(&mut sug, "");
            } else if i == 1 {
                push_str(&mut sug, &"a".repeat(MAX_SUGGESTION_LEN + 20));
            } else if i == 2 {
                push_str(&mut sug, &"a".repeat(MAX_SUGGESTION_LEN));
            } else {
                push_str(&mut sug, &format!("opt_{i}"));
            }
        }
        let decoded = decode_suggest_response(&sug).unwrap();
        assert!(decoded.len() <= MAX_SUGGESTIONS);
        assert_eq!(decoded[0], "a".repeat(MAX_SUGGESTION_LEN));
        assert_eq!(decoded.iter().filter(|s| *s == &"a".repeat(MAX_SUGGESTION_LEN)).count(), 1);
    }

    #[test]
    fn parse_argument_fails_cleanly_for_unknown_or_missing_owner() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        reset();
        let err = parse_argument("ghostmod:arg", "x", HostAccess::None).unwrap_err();
        assert!(err.contains("unknown"), "{err}");

        register_types("ghostmod", &[("arg".into(), false)]).unwrap();
        let err = parse_argument("ghostmod:arg", "x", HostAccess::None).unwrap_err();
        assert!(err.contains("unavailable"), "{err}");
        assert!(suggest_argument("ghostmod:arg", "x", HostAccess::None).is_empty());
        reset();
    }
}
