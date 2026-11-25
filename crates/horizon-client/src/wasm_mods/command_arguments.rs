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
use std::sync::{Arc, Mutex};

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

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct CommandArgRegistry {
    types: BTreeMap<String, RegisteredArgType>,
    counts: BTreeMap<String, usize>,
}

static BUSY_WARNED: AtomicBool = AtomicBool::new(false);
static FAILED_WARNED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static ACTIVE_ARG_TYPES: std::cell::RefCell<Option<CustomArgTypes>> =
        const { std::cell::RefCell::new(None) };
}

/// Server-owned custom command argument type registry shared only within one server session.
#[derive(Clone, Default)]
pub(crate) struct CustomArgTypes {
    inner: Arc<CustomArgTypesInner>,
}

#[derive(Default)]
struct CustomArgTypesInner {
    registry: Mutex<CommandArgRegistry>,
    any: AtomicBool,
}

/// Binds a server's [`CustomArgTypes`] onto the current thread while server modules initialize
/// or hot-reload.
pub(crate) struct ActiveArgTypesScope(Option<CustomArgTypes>);

impl ActiveArgTypesScope {
    pub(crate) fn enter(target: &CustomArgTypes) -> Self {
        Self(ACTIVE_ARG_TYPES.with(|active| active.replace(Some(target.clone()))))
    }
}

impl Drop for ActiveArgTypesScope {
    fn drop(&mut self) {
        ACTIVE_ARG_TYPES.with(|active| active.replace(self.0.take()));
    }
}

/// Commits types staged during module initialization into the active server's registry, if any.
///
/// Outside a live server open/reload (such as startup probing or isolated guest tests), staged
/// registrations were already validated and are discarded.
pub(crate) fn commit_active_owner(
    owner: &str,
    staged: &CommandArgRegistry,
) -> Result<(), String> {
    let target = ACTIVE_ARG_TYPES.with(|active| active.borrow().clone());
    if let Some(target) = target {
        target.replace_owner(owner, staged)?;
    }
    Ok(())
}

impl CustomArgTypes {
    /// Whether no custom argument type is registered for this server session (single atomic load).
    pub(crate) fn is_empty(&self) -> bool {
        !self.inner.any.load(Ordering::Relaxed)
    }

    /// Clears all custom argument types owned by this server session.
    pub(crate) fn clear(&self) {
        let mut guard = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.clear();
        self.inner.any.store(false, Ordering::Release);
    }

    /// Replaces `owner`'s registered types atomically when that server module initializes or reloads.
    pub(crate) fn replace_owner(
        &self,
        owner: &str,
        staged: &CommandArgRegistry,
    ) -> Result<(), String> {
        let mut guard = self
            .inner
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        guard.replace_owner_from(owner, staged)?;
        self.inner.any.store(!guard.is_empty(), Ordering::Release);
        Ok(())
    }

    /// Looks up a registered `<modid>:<name>` custom argument type on this server session.
    pub(crate) fn lookup_type(&self, type_id: &str) -> Option<RegisteredArgType> {
        if self.is_empty() {
            return None;
        }
        self.inner
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .lookup_type(type_id)
    }

    /// Calls the owning mod to parse and normalize a custom command argument value.
    pub(crate) fn parse_argument(
        &self,
        type_id: &str,
        input: &str,
        host: HostAccess<'_>,
    ) -> Result<String, String> {
        let Some(entry) = self.lookup_type(type_id) else {
            return Err(format!("unknown custom argument type `{type_id}`"));
        };
        parse_with_entry(&entry, type_id, input, host)
    }

    /// Calls the owning mod to collect bounded tab-completion suggestions for a custom argument.
    pub(crate) fn suggest_argument(
        &self,
        type_id: &str,
        partial: &str,
        host: HostAccess<'_>,
    ) -> Vec<String> {
        let Some(entry) = self.lookup_type(type_id) else {
            return Vec::new();
        };
        suggest_with_entry(&entry, type_id, partial, host)
    }
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

impl CommandArgRegistry {
    pub(crate) fn is_empty(&self) -> bool {
        self.types.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.types.clear();
        self.counts.clear();
    }

    fn clear_owner(&mut self, owner: &str) {
        self.counts.remove(owner);
        self.types.retain(|_, entry| entry.owner != owner);
    }

    /// Registers custom command argument types for `owner`, namespacing each as `<owner>:<name>`.
    pub(crate) fn register_types(
        &mut self,
        owner: &str,
        entries: &[(String, bool)],
    ) -> Result<(), String> {
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

        let existing_count = self.counts.get(owner).copied().unwrap_or(0);
        let mut new_for_owner = 0usize;
        for (id, entry) in &staged {
            if let Some(prev) = self.types.get(id) {
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
        if self.types.len() + new_for_owner > MAX_TOTAL_TYPES {
            return Err(format!(
                "too many command argument types registered (limit {MAX_TOTAL_TYPES})"
            ));
        }
        for (id, entry) in staged {
            self.types.insert(id, entry);
        }
        *self.counts.entry(owner.to_owned()).or_insert(0) += new_for_owner;
        Ok(())
    }

    pub(crate) fn replace_owner_from(
        &mut self,
        owner: &str,
        staged: &Self,
    ) -> Result<(), String> {
        let old_count = self.counts.get(owner).copied().unwrap_or(0);
        let new_count = staged.counts.get(owner).copied().unwrap_or(0);
        if self.types.len().saturating_sub(old_count) + new_count > MAX_TOTAL_TYPES {
            return Err(format!(
                "too many command argument types registered (limit {MAX_TOTAL_TYPES})"
            ));
        }
        self.clear_owner(owner);
        for (id, entry) in &staged.types {
            if entry.owner == owner {
                self.types.insert(id.clone(), entry.clone());
            }
        }
        if new_count > 0 {
            self.counts.insert(owner.to_owned(), new_count);
        }
        Ok(())
    }

    /// Looks up a registered `<modid>:<name>` custom argument type.
    pub(crate) fn lookup_type(&self, type_id: &str) -> Option<RegisteredArgType> {
        if self.is_empty() {
            return None;
        }
        self.types.get(type_id).cloned()
    }

    #[cfg(test)]
    pub(crate) fn parse_argument(
        &self,
        type_id: &str,
        input: &str,
        host: HostAccess<'_>,
    ) -> Result<String, String> {
        let Some(entry) = self.lookup_type(type_id) else {
            return Err(format!("unknown custom argument type `{type_id}`"));
        };
        parse_with_entry(&entry, type_id, input, host)
    }

    #[cfg(test)]
    pub(crate) fn suggest_argument(
        &self,
        type_id: &str,
        partial: &str,
        host: HostAccess<'_>,
    ) -> Vec<String> {
        let Some(entry) = self.lookup_type(type_id) else {
            return Vec::new();
        };
        suggest_with_entry(&entry, type_id, partial, host)
    }
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

fn parse_with_entry(
    entry: &RegisteredArgType,
    type_id: &str,
    input: &str,
    host: HostAccess<'_>,
) -> Result<String, String> {
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

fn suggest_with_entry(
    entry: &RegisteredArgType,
    type_id: &str,
    partial: &str,
    host: HostAccess<'_>,
) -> Vec<String> {
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
        let mut registry = CommandArgRegistry::default();
        assert!(registry.is_empty());

        // Rejects bad names (empty, colon, uppercase, whitespace, overlong).
        assert!(registry.register_types("cmdmod", &[("".into(), false)]).is_err());
        assert!(registry.register_types("cmdmod", &[("a:b".into(), false)]).is_err());
        assert!(registry.register_types("cmdmod", &[("Color".into(), false)]).is_err());
        assert!(registry.register_types("cmdmod", &[("two words".into(), false)]).is_err());
        assert!(registry.register_types("cmdmod", &[("x".repeat(MAX_NAME_LEN + 1), false)]).is_err());
        assert!(registry.register_types("bad mod", &[("ok".into(), false)]).is_err());
        assert!(registry.is_empty());

        // Valid registration namespaces as `<modid>:<name>`.
        registry
            .register_types("cmdmod", &[("color".into(), false), ("note".into(), true)])
            .unwrap();
        assert!(!registry.is_empty());
        assert_eq!(
            registry.lookup_type("cmdmod:color"),
            Some(RegisteredArgType {
                owner: "cmdmod".into(),
                greedy: false,
            })
        );
        assert_eq!(
            registry.lookup_type("cmdmod:note"),
            Some(RegisteredArgType {
                owner: "cmdmod".into(),
                greedy: true,
            })
        );
        assert!(registry.lookup_type("other:color").is_none());

        // Idempotent repeat is accepted; conflicting greedy flag inside batch is rejected.
        registry.register_types("cmdmod", &[("color".into(), false)]).unwrap();
        assert!(
            registry
                .register_types("cmdmod", &[("dup".into(), false), ("dup".into(), true)])
                .is_err()
        );

        // Enforces per-mod cap.
        let too_many: Vec<(String, bool)> = (0..=MAX_TYPES_PER_MOD)
            .map(|i| (format!("t{i}"), false))
            .collect();
        assert!(registry.register_types("capmod", &too_many).is_err());
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
        let mut registry = CommandArgRegistry::default();
        let err = registry
            .parse_argument("ghostmod:arg", "x", HostAccess::None)
            .unwrap_err();
        assert!(err.contains("unknown"), "{err}");

        registry
            .register_types("ghostmod", &[("arg".into(), false)])
            .unwrap();
        let err = registry
            .parse_argument("ghostmod:arg", "x", HostAccess::None)
            .unwrap_err();
        assert!(err.contains("unavailable"), "{err}");
        assert!(
            registry
                .suggest_argument("ghostmod:arg", "x", HostAccess::None)
                .is_empty()
        );
    }

    #[test]
    fn separate_server_registries_stay_isolated_and_clear_on_stop_or_reload() {
        let server_a = CustomArgTypes::default();
        let server_b = CustomArgTypes::default();
        assert!(server_a.is_empty());
        assert!(server_b.is_empty());

        let mut staged_a = CommandArgRegistry::default();
        staged_a
            .register_types("mod_a", &[("zone".into(), false), ("note".into(), true)])
            .unwrap();
        let mut staged_b = CommandArgRegistry::default();
        staged_b
            .register_types("mod_b", &[("region".into(), true)])
            .unwrap();

        {
            let _scope_a = ActiveArgTypesScope::enter(&server_a);
            commit_active_owner("mod_a", &staged_a).unwrap();
            {
                let _scope_b = ActiveArgTypesScope::enter(&server_b);
                commit_active_owner("mod_b", &staged_b).unwrap();
            }
        }

        assert!(!server_a.is_empty());
        assert!(!server_b.is_empty());
        assert!(server_a.lookup_type("mod_a:zone").is_some());
        assert!(server_a.lookup_type("mod_b:region").is_none());
        assert!(server_b.lookup_type("mod_b:region").is_some());
        assert!(server_b.lookup_type("mod_a:zone").is_none());

        // Reloading mod_a replaces only mod_a's types on server_a.
        let mut staged_other = CommandArgRegistry::default();
        staged_other
            .register_types("other", &[("keep".into(), false)])
            .unwrap();
        server_a.replace_owner("other", &staged_other).unwrap();
        let mut reloaded_a = CommandArgRegistry::default();
        reloaded_a
            .register_types("mod_a", &[("zone".into(), true)])
            .unwrap();
        server_a.replace_owner("mod_a", &reloaded_a).unwrap();
        assert_eq!(server_a.lookup_type("mod_a:zone").map(|t| t.greedy), Some(true));
        assert!(server_a.lookup_type("mod_a:note").is_none());
        assert!(server_a.lookup_type("other:keep").is_some());

        // Stopping server_a clears its registry without affecting server_b.
        server_a.clear();
        assert!(server_a.is_empty());
        assert!(server_a.lookup_type("mod_a:zone").is_none());
        assert!(server_a.lookup_type("other:keep").is_none());
        assert!(server_b.lookup_type("mod_b:region").is_some());
    }
}
