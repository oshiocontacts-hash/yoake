use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use toml::Value;

const MAX_FIELDS: usize = 256;
const MAX_NAME_BYTES: usize = 128;
const MAX_TEXT_BYTES: usize = 65_536;
const MAX_LIST_ITEMS: usize = 4_096;
const MAX_CHOICES: usize = 256;
const MAX_CONFIG_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigKind {
    Bool,
    Int,
    Float,
    String,
    Enum,
    List,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSide {
    Client,
    Server,
    Synced,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigField {
    #[serde(rename = "type")]
    pub kind: ConfigKind,
    pub default: Value,
    pub side: ConfigSide,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element: Option<ConfigKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub label: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub restart: bool,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionNode {
    pub default_op_level: u8,
}

pub fn has_permission(
    declarations: &BTreeMap<String, PermissionNode>,
    node: &str,
    op_level: u8,
    explicit: Option<bool>,
) -> bool {
    if op_level > 4 {
        return false;
    }
    declarations
        .get(node)
        .is_some_and(|permission| permission.default_op_level <= 4 && explicit.unwrap_or(op_level >= permission.default_op_level))
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_BYTES
        && name.split('.').all(|part| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        })
}

pub(super) fn validate_declarations(
    fields: &BTreeMap<String, ConfigField>,
    permissions: &BTreeMap<String, PermissionNode>,
) -> Result<(), String> {
    if fields.len() > MAX_FIELDS || permissions.len() > MAX_FIELDS {
        return Err("too many config fields or permission nodes".into());
    }
    let mut default_bytes = 0usize;
    for (name, field) in fields {
        if field.label.len() > 16
            || field
                .label
                .iter()
                .any(|(language, label)| language.len() > 32 || label.len() > 1024)
        {
            return Err("config label exceeds limits".into());
        }
        if !valid_name(name) {
            return Err(format!("invalid config key: {name}"));
        }
        field.validate_schema().map_err(|error| format!("config {name}: {error}"))?;
        default_bytes = default_bytes.checked_add(name.len())
            .and_then(|total| total.checked_add(config_value_size(&field.default)?))
            .ok_or("invalid or excessive config defaults")?;
        if default_bytes > MAX_CONFIG_BYTES {
            return Err("config defaults exceed aggregate size limit".into());
        }
    }
    for (name, node) in permissions {
        if !valid_name(name) || node.default_op_level > 4 {
            return Err(format!("invalid permission declaration: {name}"));
        }
    }
    Ok(())
}

impl ConfigField {
    fn scalar_kind(&self) -> Result<ConfigKind, String> {
        match (self.kind, self.element) {
            (ConfigKind::List, Some(kind)) if kind != ConfigKind::List => Ok(kind),
            (ConfigKind::List, _) => Err("list needs a non-list element type".into()),
            (_, Some(_)) => Err("element type is only valid for a list".into()),
            (kind, None) => Ok(kind),
        }
    }

    fn validate_schema(&self) -> Result<(), String> {
        let kind = self.scalar_kind()?;
        if self.comment.as_ref().is_some_and(|text| text.len() > MAX_TEXT_BYTES) {
            return Err("comment is too long".into());
        }
        if kind == ConfigKind::Enum {
            let mut unique = BTreeSet::new();
            if self.values.is_empty() || self.values.len() > MAX_CHOICES
                || self.values.iter().any(|value| {
                    value.is_empty() || value.len() > MAX_TEXT_BYTES || !unique.insert(value)
                })
            {
                return Err("enum values must be nonempty, bounded, and unique".into());
            }
        } else if !self.values.is_empty() {
            return Err("values are only valid for enum fields".into());
        }
        match kind {
            ConfigKind::Int => {
                let minimum = self.min.as_ref().map(|value| value.as_integer().ok_or("integer minimum required")).transpose()?;
                let maximum = self.max.as_ref().map(|value| value.as_integer().ok_or("integer maximum required")).transpose()?;
                if minimum.zip(maximum).is_some_and(|(minimum, maximum)| minimum > maximum) {
                    return Err("minimum exceeds maximum".into());
                }
            }
            ConfigKind::Float => {
                let minimum = self.min.as_ref().map(|value| finite_float(value, "float minimum required")).transpose()?;
                let maximum = self.max.as_ref().map(|value| finite_float(value, "float maximum required")).transpose()?;
                if minimum.zip(maximum).is_some_and(|(minimum, maximum)| minimum > maximum) {
                    return Err("minimum exceeds maximum".into());
                }
            }
            _ if self.min.is_some() || self.max.is_some() => return Err("ranges require numeric fields".into()),
            _ => {}
        }
        self.validate_value(&self.default)
    }

    pub fn validate_value(&self, value: &Value) -> Result<(), String> {
        let kind = self.scalar_kind()?;
        if self.kind == ConfigKind::List {
            let items = value.as_array().ok_or("expected a list")?;
            if items.len() > MAX_LIST_ITEMS {
                return Err("list is too long".into());
            }
            for item in items {
                self.validate_scalar(kind, item)?;
            }
        } else {
            self.validate_scalar(kind, value)?;
        }
        if config_value_size(value).is_none_or(|size| size > MAX_CONFIG_BYTES) {
            return Err("config value is too large".into());
        }
        Ok(())
    }

    fn validate_scalar(&self, kind: ConfigKind, value: &Value) -> Result<(), String> {
        match kind {
            ConfigKind::Bool => { value.as_bool().ok_or("expected a bool")?; }
            ConfigKind::Int => {
                let actual = value.as_integer().ok_or("expected an integer")?;
                let minimum = self.min.as_ref().map(|bound| bound.as_integer().ok_or("integer minimum required")).transpose()?;
                let maximum = self.max.as_ref().map(|bound| bound.as_integer().ok_or("integer maximum required")).transpose()?;
                if minimum.is_some_and(|bound| actual < bound) || maximum.is_some_and(|bound| actual > bound) {
                    return Err("integer outside declared range".into());
                }
            }
            ConfigKind::Float => {
                let actual = finite_float(value, "expected a finite float")?;
                let minimum = self.min.as_ref().map(|bound| finite_float(bound, "float minimum required")).transpose()?;
                let maximum = self.max.as_ref().map(|bound| finite_float(bound, "float maximum required")).transpose()?;
                if minimum.is_some_and(|bound| actual < bound) || maximum.is_some_and(|bound| actual > bound) {
                    return Err("float outside declared range".into());
                }
            }
            ConfigKind::String | ConfigKind::Enum => {
                let actual = value.as_str().ok_or("expected a string")?;
                if actual.len() > MAX_TEXT_BYTES {
                    return Err("string is too long".into());
                }
                if kind == ConfigKind::Enum && !self.values.iter().any(|choice| choice == actual) {
                    return Err("value is not a declared enum choice".into());
                }
            }
            ConfigKind::List => return Err("nested lists are not supported".into()),
        }
        Ok(())
    }
}

fn finite_float(value: &Value, error: &str) -> Result<f64, String> {
    value.as_float().filter(|number| number.is_finite()).ok_or_else(|| error.to_owned())
}

fn config_value_size(value: &Value) -> Option<usize> {
    match value {
        Value::String(text) => Some(text.len()),
        Value::Integer(_) | Value::Float(_) => Some(8),
        Value::Boolean(_) => Some(1),
        Value::Array(items) => items.iter().try_fold(0usize, |total, item| {
            total.checked_add(config_value_size(item)?)
        }),
        _ => None,
    }
}

pub fn resolve_config(
    fields: &BTreeMap<String, ConfigField>,
    supplied: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, String> {
    validate_declarations(fields, &BTreeMap::new())?;
    if let Some(name) = supplied.keys().find(|name| !fields.contains_key(*name)) {
        return Err(format!("unknown config key: {name}"));
    }
    let mut total = 0usize;
    for (name, field) in fields {
        let value = supplied.get(name).unwrap_or(&field.default);
        field.validate_value(value).map_err(|error| format!("config {name}: {error}"))?;
        total = total.checked_add(name.len())
            .and_then(|size| size.checked_add(config_value_size(value)?))
            .ok_or("config is too large")?;
        if total > MAX_CONFIG_BYTES {
            return Err("config is too large".into());
        }
    }
    Ok(fields.iter().map(|(name, field)| {
        (name.clone(), supplied.get(name).unwrap_or(&field.default).clone())
    }).collect())
}

pub fn parse_config(
    fields: &BTreeMap<String, ConfigField>,
    document: &str,
) -> Result<BTreeMap<String, Value>, String> {
    if document.len() > MAX_CONFIG_BYTES {
        return Err("config document is too large".into());
    }
    let supplied = toml::from_str(document)
        .map_err(|error| format!("invalid config TOML: {error}"))?;
    resolve_config(fields, &supplied)
}

pub fn serialize_config(
    fields: &BTreeMap<String, ConfigField>,
    supplied: &BTreeMap<String, Value>,
) -> Result<String, String> {
    let resolved = resolve_config(fields, supplied)?;
    let document = toml::to_string(&resolved)
        .map_err(|error| format!("cannot encode config TOML: {error}"))?;
    if document.len() > MAX_CONFIG_BYTES {
        return Err("config document is too large".into());
    }
    Ok(document)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigRuntimeSide {
    Client,
    Server,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConfigChange {
    pub name: String,
    pub old: Value,
    pub new: Value,
}

#[derive(Clone)]
pub struct ConfigState {
    side: ConfigRuntimeSide,
    fields: BTreeMap<String, ConfigField>,
    values: BTreeMap<String, Value>,
    sync_active: bool,
}

impl ConfigState {
    pub fn new(
        fields: &BTreeMap<String, ConfigField>,
        side: ConfigRuntimeSide,
        persisted: &BTreeMap<String, Value>,
    ) -> Result<Self, String> {
        for name in persisted.keys() {
            match fields.get(name) {
                Some(field) if Self::is_local(side, field.side) => {}
                Some(_) => return Err(format!("config key is not local to this side: {name}")),
                None => return Err(format!("unknown config key: {name}")),
            }
        }
        let state = Self {
            side,
            fields: fields.clone(),
            values: resolve_config(fields, persisted)?,
            sync_active: false,
        };
        state.validate_candidate(&state.values)?;
        Ok(state)
    }

    pub fn from_local_document(
        fields: &BTreeMap<String, ConfigField>,
        side: ConfigRuntimeSide,
        document: &str,
    ) -> Result<Self, String> {
        if document.len() > MAX_CONFIG_BYTES {
            return Err("config document is too large".into());
        }
        let persisted = toml::from_str(document)
            .map_err(|error| format!("invalid config TOML: {error}"))?;
        Self::new(fields, side, &persisted)
    }

    pub fn get(&self, name: &str) -> Option<&Value> {
        let field = self.fields.get(name)?;
        if Self::is_local(self.side, field.side)
            || (self.side == ConfigRuntimeSide::Client && field.side == ConfigSide::Synced)
        {
            self.values.get(name)
        } else {
            None
        }
    }

    pub fn set_local(&mut self, name: &str, value: Value) -> Result<Option<ConfigChange>, String> {
        let field = self.fields.get(name).ok_or_else(|| format!("unknown config key: {name}"))?;
        if !Self::is_local(self.side, field.side) {
            return Err(format!("config key is not local to this side: {name}"));
        }
        let old = self.values[name].clone();
        let mut candidate = self.values.clone();
        candidate.insert(name.to_owned(), value.clone());
        self.validate_candidate(&candidate)?;
        if old == value {
            return Ok(None);
        }
        self.values = candidate;
        Ok(Some(ConfigChange { name: name.to_owned(), old, new: value }))
    }

    pub fn synced_snapshot(&self) -> Result<BTreeMap<String, Value>, String> {
        if self.side != ConfigRuntimeSide::Server {
            return Err("only the server can produce a synced config snapshot".into());
        }
        Ok(self.fields.iter()
            .filter(|(_, field)| field.side == ConfigSide::Synced)
            .map(|(name, _)| (name.clone(), self.values[name].clone()))
            .collect())
    }

    pub fn apply_synced_snapshot(
        &mut self,
        snapshot: &BTreeMap<String, Value>,
    ) -> Result<Vec<ConfigChange>, String> {
        if self.side != ConfigRuntimeSide::Client {
            return Err("only a client can apply a synced config snapshot".into());
        }
        let expected = self.fields.values().filter(|field| field.side == ConfigSide::Synced).count();
        if snapshot.len() != expected || snapshot.keys().any(|name| {
            !self.fields.get(name).is_some_and(|field| field.side == ConfigSide::Synced)
        }) {
            return Err("synced config snapshot must contain exactly the synced keys".into());
        }
        let mut candidate = self.values.clone();
        candidate.extend(snapshot.iter().map(|(name, value)| (name.clone(), value.clone())));
        self.validate_candidate(&candidate)?;
        let changes = snapshot.iter().filter_map(|(name, value)| {
            let old = &self.values[name];
            (old != value).then(|| ConfigChange { name: name.clone(), old: old.clone(), new: value.clone() })
        }).collect();
        self.values = candidate;
        self.sync_active = true;
        Ok(changes)
    }

    pub fn clear_synced(&mut self) -> Vec<ConfigChange> {
        if self.side != ConfigRuntimeSide::Client || !self.sync_active {
            return Vec::new();
        }
        let mut changes = Vec::new();
        for (name, field) in &self.fields {
            if field.side == ConfigSide::Synced {
                let old = &self.values[name];
                if old != &field.default {
                    changes.push(ConfigChange {
                        name: name.clone(),
                        old: old.clone(),
                        new: field.default.clone(),
                    });
                    self.values.insert(name.clone(), field.default.clone());
                }
            }
        }
        self.sync_active = false;
        changes
    }

    pub fn local_document(&self) -> Result<String, String> {
        let fields: BTreeMap<_, _> = self.fields.iter()
            .filter(|(_, field)| Self::is_local(self.side, field.side))
            .map(|(name, field)| (name.clone(), field.clone()))
            .collect();
        let values = fields.keys().map(|name| (name.clone(), self.values[name].clone())).collect();
        serialize_config(&fields, &values)
    }

    fn is_local(side: ConfigRuntimeSide, field: ConfigSide) -> bool {
        matches!((side, field),
            (ConfigRuntimeSide::Client, ConfigSide::Client)
                | (ConfigRuntimeSide::Server, ConfigSide::Server | ConfigSide::Synced))
    }

    fn validate_candidate(&self, candidate: &BTreeMap<String, Value>) -> Result<(), String> {
        resolve_config(&self.fields, candidate)?;
        if self.side == ConfigRuntimeSide::Client {
            let mut disconnected = candidate.clone();
            for (name, field) in &self.fields {
                if field.side == ConfigSide::Synced {
                    disconnected.insert(name.clone(), field.default.clone());
                }
            }
            resolve_config(&self.fields, &disconnected)?;
        }
        Ok(())
    }
}

pub struct ConfigFile {
    base: super::ServiceDirectory,
    filename: String,
    mod_id: String,
    fields: BTreeMap<String, ConfigField>,
    state: ConfigState,
}

impl ConfigFile {
    pub fn load(
        game_dir: &Path,
        mod_id: &str,
        fields: &BTreeMap<String, ConfigField>,
        side: ConfigRuntimeSide,
    ) -> Result<Self, String> {
        super::manifest::identifier(mod_id)?;
        if reserved_file_name(mod_id) {
            return Err("reserved config file name".into());
        }
        let canonical = std::fs::canonicalize(game_dir).map_err(|error| error.to_string())?;
        let base =
            super::ServiceDirectory::open(&canonical, false).map_err(|error| error.to_string())?;
        let filename = format!("{mod_id}.toml");
        let supplied = match base.child(OsStr::new("config"), false) {
            Ok(directory) => read_config_file(&directory, &filename, fields)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error.to_string()),
        };
        let local = supplied
            .into_iter()
            .filter(|(name, _)| {
                fields
                    .get(name)
                    .is_some_and(|field| ConfigState::is_local(side, field.side))
            })
            .collect();
        let state = ConfigState::new(fields, side, &local)?;
        Ok(Self {
            base,
            filename,
            mod_id: mod_id.into(),
            fields: fields.clone(),
            state,
        })
    }

    pub fn state(&self) -> &ConfigState {
        &self.state
    }

    pub fn set_local(&mut self, name: &str, value: Value) -> Result<Option<ConfigChange>, String> {
        Ok(self
            .set_many(&BTreeMap::from([(name.into(), value)]))?
            .into_iter()
            .find(|change| change.name == name))
    }

    pub fn set_many(
        &mut self,
        values: &BTreeMap<String, Value>,
    ) -> Result<Vec<ConfigChange>, String> {
        self.set_many_with_budget(values, |_| Ok(()))
    }

    pub fn set_many_with_budget(
        &mut self,
        values: &BTreeMap<String, Value>,
        charge: impl FnOnce(usize) -> Result<(), String>,
    ) -> Result<Vec<ConfigChange>, String> {
        let mut candidate = self.state.clone();
        let mut requested_change = false;
        for (name, value) in values {
            requested_change |= candidate.set_local(name, value.clone())?.is_some();
        }
        let directory = match self.base.child(OsStr::new("config"), false) {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && !requested_change => {
                return Ok(Vec::new());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => self
                .base
                .child(OsStr::new("config"), true)
                .map_err(|error| error.to_string())?,
            Err(error) => return Err(error.to_string()),
        };
        let (candidate, changes, encoded) = self.prepare_update(&directory, values)?;
        if encoded.is_none() {
            self.state = candidate;
            return Ok(changes);
        }
        let lock_name = format!("{}.lock", self.mod_id);
        let began = Instant::now();
        let _lock = loop {
            match directory.lock(OsStr::new(&lock_name)) {
                Ok(lock) => break lock,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && began.elapsed() < Duration::from_millis(40) =>
                {
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    return Err(format!("store is already open in another process: {error}"));
                }
                Err(error) => return Err(error.to_string()),
            }
        };
        let (candidate, changes, encoded) = self.prepare_update(&directory, values)?;
        let Some(encoded) = encoded else {
            self.state = candidate;
            return Ok(changes);
        };
        charge(encoded.len())?;
        let prefix = format!(".config-{}.", self.mod_id);
        for entry in directory.entries().map_err(|error| error.to_string())? {
            if entry
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix) && name.ends_with(".tmp"))
            {
                directory
                    .remove(&entry)
                    .map_err(|error| error.to_string())?;
            }
        }
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        let temporary = format!(
            "{prefix}{}-{}.tmp",
            std::process::id(),
            NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
        );
        let result = (|| {
            let mut file = directory.create(OsStr::new(&temporary))?;
            file.write_all(encoded.as_bytes())?;
            file.sync_all()?;
            drop(file);
            directory.replace(OsStr::new(&temporary), OsStr::new(&self.filename))
        })();
        if let Err(error) = result {
            let _ = directory.remove(OsStr::new(&temporary));
            return Err(error.to_string());
        }
        self.state = candidate;
        Ok(changes)
    }

    fn prepare_update(
        &self,
        directory: &super::ServiceDirectory,
        values: &BTreeMap<String, Value>,
    ) -> Result<(ConfigState, Vec<ConfigChange>, Option<String>), String> {
        let mut document = read_config_file(directory, &self.filename, &self.fields)?;
        let persisted = resolve_config(&self.fields, &document)?;
        let mut candidate = self.state.clone();
        for (name, field) in &self.fields {
            if ConfigState::is_local(candidate.side, field.side) {
                candidate
                    .values
                    .insert(name.clone(), persisted[name].clone());
            }
        }
        let mut write_required = false;
        for (name, value) in values {
            write_required |= candidate.set_local(name, value.clone())?.is_some();
            document.insert(name.clone(), value.clone());
        }
        let changes = self
            .fields
            .iter()
            .filter(|(_, field)| ConfigState::is_local(candidate.side, field.side))
            .filter(|(name, _)| self.state.values[*name] != candidate.values[*name])
            .map(|(name, _)| ConfigChange {
                name: name.clone(),
                old: self.state.values[name].clone(),
                new: candidate.values[name].clone(),
            })
            .collect();
        if !write_required {
            return Ok((candidate, changes, None));
        }
        let encoded = toml::to_string(&document).map_err(|error| error.to_string())?;
        parse_config(&self.fields, &encoded)?;
        Ok((candidate, changes, Some(encoded)))
    }

    pub fn apply_synced_snapshot(
        &mut self,
        snapshot: &BTreeMap<String, Value>,
    ) -> Result<Vec<ConfigChange>, String> {
        self.state.apply_synced_snapshot(snapshot)
    }

    pub fn clear_synced(&mut self) -> Vec<ConfigChange> {
        self.state.clear_synced()
    }
}

pub(super) fn reserved_file_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

fn read_config_file(
    directory: &super::ServiceDirectory,
    filename: &str,
    fields: &BTreeMap<String, ConfigField>,
) -> Result<BTreeMap<String, Value>, String> {
    let Some(file) = directory
        .read(OsStr::new(filename))
        .map_err(|error| error.to_string())?
    else {
        return Ok(BTreeMap::new());
    };
    let mut bytes = Vec::new();
    file.take((MAX_CONFIG_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read {filename}: {error}"))?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(format!("{filename}: config exceeds size limit"));
    }
    let document = std::str::from_utf8(&bytes)
        .map_err(|error| format!("invalid UTF-8 in {filename}: {error}"))?;
    let supplied: BTreeMap<String, Value> =
        toml::from_str(document).map_err(|error| format!("{filename}: {error}"))?;
    let supplied = supplied
        .into_iter()
        .filter(|(name, value)| match fields.get(name) {
            Some(field) if field.validate_value(value).is_ok() => true,
            _ => {
                eprintln!("[yoake] {filename}: ignoring unknown or invalid config field {name}");
                false
            }
        })
        .collect();
    resolve_config(fields, &supplied).map_err(|error| format!("{filename}: {error}"))?;
    Ok(supplied)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yak::Manifest;

    const BASE: &str = r#"format = 1
id = "sample-mod"
version = "0.3.1"
name = "Sample"
authors = ["Test author"]
description = "Test package"
[yoake]
version = ">=0.3, <0.4"
[side]
client = true
server = true
required_on_join = false
"#;

    #[test]
    fn manifest_defaults_and_validates_typed_values() {
        let manifest = Manifest::parse(&format!("{BASE}{}", r#"
[config.enabled]
type = "bool"
side = "client"
default = true
[config.radius]
type = "int"
side = "synced"
default = 3
min = 1
max = 5
[config.choices]
type = "list"
side = "server"
element = "enum"
values = ["north", "south"]
default = ["north"]
[permissions."example.use"]
default_op_level = 2
"#)).unwrap();
        let values = resolve_config(&manifest.config, &BTreeMap::new()).unwrap();
        assert_eq!(values["radius"], Value::Integer(3));
        assert_eq!(values["choices"], Value::Array(vec![Value::String("north".into())]));
        let supplied = BTreeMap::from([("radius".into(), Value::Integer(6))]);
        assert!(resolve_config(&manifest.config, &supplied).is_err());
        let supplied = BTreeMap::from([("choices".into(), Value::Array(vec![Value::String("east".into())]))]);
        assert!(resolve_config(&manifest.config, &supplied).is_err());
        let supplied = BTreeMap::from([("unknown".into(), Value::Boolean(true))]);
        assert!(resolve_config(&manifest.config, &supplied).is_err());
        assert!(!has_permission(&manifest.permissions, "example.use", 1, None));
        assert!(has_permission(&manifest.permissions, "example.use", 2, None));
        assert!(!has_permission(&manifest.permissions, "example.use", 4, Some(false)));
        assert!(has_permission(&manifest.permissions, "example.use", 0, Some(true)));
        assert!(!has_permission(&manifest.permissions, "unknown", 4, Some(true)));
    }

    #[test]
    fn config_documents_validate_and_round_trip() {
        let manifest = Manifest::parse(&format!("{BASE}{}", r#"
[config."options.zoom"]
type = "int"
side = "synced"
default = 3
min = 1
max = 5
[config.enabled]
type = "bool"
side = "client"
default = true
"#)).unwrap();
        let values = parse_config(&manifest.config, "'options.zoom' = 4\n").unwrap();
        assert_eq!(values["options.zoom"], Value::Integer(4));
        assert_eq!(values["enabled"], Value::Boolean(true));
        let document = serialize_config(&manifest.config, &values).unwrap();
        assert_eq!(parse_config(&manifest.config, &document).unwrap(), values);
        assert!(parse_config(&manifest.config, "'options.zoom' = 6\n").is_err());
        assert!(parse_config(&manifest.config, "unknown = true\n").is_err());
        assert!(parse_config(&manifest.config, "enabled = true\nenabled = false\n").is_err());
        assert!(parse_config(&manifest.config, &" ".repeat(MAX_CONFIG_BYTES + 1)).is_err());
    }

    fn side_manifest() -> Manifest {
        Manifest::parse(&format!("{BASE}{}", r#"
[config.client_setting]
type = "bool"
side = "client"
default = false
[config.server_setting]
type = "bool"
side = "server"
default = true
[config.shared]
type = "int"
side = "synced"
default = 3
min = 1
max = 5
[config.shared_flag]
type = "bool"
side = "synced"
default = false
"#)).unwrap()
    }

    fn temporary_config_root() -> std::path::PathBuf {
        let root = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "yoake-config-regression-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
            ));
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn invalid_and_removed_fields_fall_back_without_hiding_valid_fields() {
        let root = temporary_config_root();
        let manifest = side_manifest();
        std::fs::create_dir(root.join("config")).unwrap();
        let path = root.join("config").join("sample-mod.toml");
        std::fs::write(
            &path,
            "shared = 99\nclient_setting = 'wrong'\nserver_setting = false\nremoved = true\n",
        )
        .unwrap();
        let mut server = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        let client = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Client,
        )
        .unwrap();
        assert_eq!(server.state().get("shared"), Some(&Value::Integer(3)));
        assert_eq!(
            server.state().get("server_setting"),
            Some(&Value::Boolean(false))
        );
        assert_eq!(
            client.state().get("client_setting"),
            Some(&Value::Boolean(false))
        );
        server.set_local("shared", Value::Integer(4)).unwrap();
        let saved: BTreeMap<String, Value> =
            toml::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(saved["shared"], Value::Integer(4));
        assert_eq!(saved["server_setting"], Value::Boolean(false));
        assert!(!saved.contains_key("removed"));
        assert!(!saved.contains_key("client_setting"));
        drop((client, server));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stale_handles_merge_the_latest_config_before_writing() {
        let root = temporary_config_root();
        let manifest = side_manifest();
        let mut first = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        let mut second = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        first.set_local("shared", Value::Integer(5)).unwrap();
        second
            .set_local("shared_flag", Value::Boolean(true))
            .unwrap();
        assert_eq!(second.state().get("shared"), Some(&Value::Integer(5)));
        first.set_local("shared", Value::Integer(4)).unwrap();
        let reloaded = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        assert_eq!(reloaded.state().get("shared"), Some(&Value::Integer(4)));
        assert_eq!(
            reloaded.state().get("shared_flag"),
            Some(&Value::Boolean(true))
        );
        drop((first, second, reloaded));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_writes_exclude_other_processes() {
        const ROOT_ENV: &str = "YOAKE_CONFIG_PROCESS_TEST";
        let manifest = side_manifest();
        if let Some(root) = std::env::var_os(ROOT_ENV) {
            let mut config = ConfigFile::load(
                Path::new(&root),
                "sample-mod",
                &manifest.config,
                ConfigRuntimeSide::Server,
            )
            .unwrap();
            let result = config.set_local("shared", Value::Integer(5));
            if std::env::var_os("YOAKE_CONFIG_EXPECT_LOCKED").is_some() {
                assert!(result.unwrap_err().contains("already open"));
            } else {
                assert!(result.is_ok());
            }
            return;
        }
        let root = temporary_config_root();
        let base = crate::yak::ServiceDirectory::open(&root, false).unwrap();
        let directory = base.child(OsStr::new("config"), true).unwrap();
        let lock = directory.lock(OsStr::new("sample-mod.lock")).unwrap();
        let child = |locked| {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args([
                    "--exact",
                    "yak::config::tests::config_writes_exclude_other_processes",
                ])
                .env(ROOT_ENV, &root);
            if locked {
                command.env("YOAKE_CONFIG_EXPECT_LOCKED", "1");
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stdout)
            );
        };
        child(true);
        drop(lock);
        child(false);
        drop((directory, base));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_write_removes_stale_temporary_files() {
        let root = temporary_config_root();
        let manifest = side_manifest();
        std::fs::create_dir(root.join("config")).unwrap();
        let temporary = root
            .join("config")
            .join(".config-sample-mod.interrupted.tmp");
        let other_mod = root
            .join("config")
            .join(".config-sample-mod-other.interrupted.tmp");
        std::fs::write(&temporary, "partial").unwrap();
        std::fs::write(&other_mod, "other mod").unwrap();
        let mut config = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        config.set_local("shared", Value::Integer(5)).unwrap();
        assert!(!temporary.exists());
        assert!(other_mod.exists());
        assert_eq!(config.state().get("shared"), Some(&Value::Integer(5)));
        drop(config);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unchanged_config_does_not_create_a_lock_file() {
        let root = temporary_config_root();
        let manifest = side_manifest();
        let mut config = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        assert!(
            config
                .set_local("shared", Value::Integer(3))
                .unwrap()
                .is_none()
        );
        assert!(!root.join("config").exists());
        std::fs::create_dir(root.join("config")).unwrap();
        std::fs::write(root.join("config/sample-mod.toml"), "shared = 3\n").unwrap();
        assert!(
            config
                .set_local("shared", Value::Integer(3))
                .unwrap()
                .is_none()
        );
        assert!(!root.join("config/sample-mod.lock").exists());
        drop(config);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_writes_retry_a_brief_in_process_lock() {
        let root = temporary_config_root();
        let manifest = side_manifest();
        let mut config = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        let base = crate::yak::ServiceDirectory::open(&root, false).unwrap();
        let directory = base.child(OsStr::new("config"), true).unwrap();
        let lock = directory.lock(OsStr::new("sample-mod.lock")).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(5));
            drop(lock);
        });
        config.set_local("shared", Value::Integer(5)).unwrap();
        release.join().unwrap();
        let saved: BTreeMap<String, Value> =
            toml::from_str(&std::fs::read_to_string(root.join("config/sample-mod.toml")).unwrap())
                .unwrap();
        assert_eq!(saved["shared"], Value::Integer(5));
        drop((config, directory, base));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejected_config_write_budget_preserves_disk_and_state() {
        let root = temporary_config_root();
        let manifest = side_manifest();
        let mut config = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        config.set_local("shared", Value::Integer(4)).unwrap();
        let path = root.join("config/sample-mod.toml");
        let saved = std::fs::read(&path).unwrap();
        let error = config
            .set_many_with_budget(
                &BTreeMap::from([("shared".into(), Value::Integer(5))]),
                |bytes| {
                    assert!(bytes > 0);
                    Err("write budget exceeded".into())
                },
            )
            .unwrap_err();
        assert_eq!(error, "write budget exceeded");
        assert_eq!(config.state().get("shared"), Some(&Value::Integer(4)));
        assert_eq!(std::fs::read(path).unwrap(), saved);
        drop(config);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_file_preserves_other_sides_and_rejects_malformed_files() {
        let root = std::fs::canonicalize(std::env::temp_dir()).unwrap().join(format!(
            "yoake-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
        ));
        let manifest = side_manifest();
        std::fs::create_dir_all(&root).unwrap();
        let mut client = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Client,
        )
        .unwrap();
        assert!(!root.join("config").exists());
        client
            .set_local("client_setting", Value::Boolean(true))
            .unwrap();

        let mut server = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        server.set_local("shared", Value::Integer(5)).unwrap();
        let reloaded_client = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Client,
        )
        .unwrap();
        let reloaded_server = ConfigFile::load(
            &root,
            "sample-mod",
            &manifest.config,
            ConfigRuntimeSide::Server,
        )
        .unwrap();
        assert_eq!(
            reloaded_client.state().get("client_setting"),
            Some(&Value::Boolean(true))
        );
        assert_eq!(
            reloaded_server.state().get("shared"),
            Some(&Value::Integer(5))
        );
        assert!(client.set_local("shared", Value::Integer(4)).is_err());
        assert!(server.set_local("shared", Value::Integer(6)).is_err());

        let path = root.join("config").join("sample-mod.toml");
        let malformed = "broken = [";
        std::fs::write(&path, malformed).unwrap();
        assert!(server.set_local("shared", Value::Integer(5)).is_err());
        assert!(server.set_local("shared", Value::Integer(4)).is_err());
        assert_eq!(server.state().get("shared"), Some(&Value::Integer(5)));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), malformed);
        assert!(
            ConfigFile::load(
                &root,
                "sample-mod",
                &manifest.config,
                ConfigRuntimeSide::Server
            )
            .is_err()
        );
        assert!(
            ConfigFile::load(&root, "con", &manifest.config, ConfigRuntimeSide::Client).is_err()
        );
        drop((client, server, reloaded_client, reloaded_server));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_state_separates_local_and_synced_values() {
        let manifest = side_manifest();
        let mut server = ConfigState::new(
            &manifest.config, ConfigRuntimeSide::Server, &BTreeMap::new(),
        ).unwrap();
        let mut client = ConfigState::from_local_document(
            &manifest.config, ConfigRuntimeSide::Client, "client_setting = true",
        ).unwrap();
        assert_eq!(client.get("client_setting"), Some(&Value::Boolean(true)));
        assert_eq!(client.get("shared"), Some(&Value::Integer(3)));
        assert_eq!(client.get("server_setting"), None);
        assert_eq!(server.get("client_setting"), None);
        assert!(client.set_local("shared", Value::Integer(4)).is_err());
        assert!(server.set_local("client_setting", Value::Boolean(false)).is_err());
        assert!(ConfigState::from_local_document(
            &manifest.config, ConfigRuntimeSide::Client, "shared = 4",
        ).is_err());
        assert!(server.set_local("shared", Value::Integer(4)).unwrap().is_some());
        assert!(server.set_local("shared", Value::Integer(4)).unwrap().is_none());
        assert!(server.set_local("shared_flag", Value::Boolean(true)).unwrap().is_some());
        assert_eq!(client.apply_synced_snapshot(&server.synced_snapshot().unwrap()).unwrap().len(), 2);
        assert_eq!(client.get("shared"), Some(&Value::Integer(4)));
        let client_document: BTreeMap<String, Value> = toml::from_str(&client.local_document().unwrap()).unwrap();
        let server_document: BTreeMap<String, Value> = toml::from_str(&server.local_document().unwrap()).unwrap();
        assert_eq!(client_document.len(), 1);
        assert_eq!(client_document["client_setting"], Value::Boolean(true));
        assert_eq!(server_document.len(), 3);
        assert_eq!(server_document["shared"], Value::Integer(4));
        let changes = client.clear_synced();
        assert_eq!(changes.len(), 2);
        assert_eq!(client.get("shared"), Some(&Value::Integer(3)));
        assert_eq!(client.get("shared_flag"), Some(&Value::Boolean(false)));
        assert!(client.clear_synced().is_empty());
        assert_eq!(client.get("client_setting"), Some(&Value::Boolean(true)));
    }

    #[test]
    fn config_state_rejects_incomplete_or_invalid_snapshots_atomically() {
        let manifest = side_manifest();
        let mut client = ConfigState::new(
            &manifest.config, ConfigRuntimeSide::Client, &BTreeMap::new(),
        ).unwrap();
        let invalid = BTreeMap::from([
            ("shared".into(), Value::Integer(4)),
            ("shared_flag".into(), Value::Integer(1)),
        ]);
        assert!(client.apply_synced_snapshot(&invalid).is_err());
        assert_eq!(client.get("shared"), Some(&Value::Integer(3)));
        let incomplete = BTreeMap::from([("shared".into(), Value::Integer(4))]);
        assert!(client.apply_synced_snapshot(&incomplete).is_err());
        let extra = BTreeMap::from([
            ("shared".into(), Value::Integer(4)),
            ("client_setting".into(), Value::Boolean(true)),
        ]);
        assert!(client.apply_synced_snapshot(&extra).is_err());
        assert_eq!(client.get("shared"), Some(&Value::Integer(3)));
        assert!(client.synced_snapshot().is_err());
        assert!(client.clear_synced().is_empty());
    }

    #[test]
    fn floating_point_and_string_values_respect_type_and_bounds() {
        let manifest = Manifest::parse(&format!("{BASE}{}", r#"
[config.ratio]
type = "float"
side = "synced"
default = 0.5
min = 0.0
max = 1.0
[config.message]
type = "string"
side = "client"
default = "hello"
"#)).unwrap();
        let ratio = &manifest.config["ratio"];
        for value in [Value::Float(0.0), Value::Float(1.0)] {
            assert!(ratio.validate_value(&value).is_ok());
        }
        for value in [
            Value::Float(f64::NAN),
            Value::Float(f64::INFINITY),
            Value::Float(-0.1),
            Value::Float(1.1),
            Value::Integer(1),
        ] {
            assert!(ratio.validate_value(&value).is_err());
        }
        let message = &manifest.config["message"];
        assert!(message.validate_value(&Value::String("x".repeat(MAX_TEXT_BYTES))).is_ok());
        assert!(message.validate_value(&Value::String("x".repeat(MAX_TEXT_BYTES + 1))).is_err());
    }

    #[test]
    fn config_rejects_excessive_aggregate_values() {
        let manifest = Manifest::parse(&format!("{BASE}{}", r#"
[config.message]
type = "string"
side = "client"
default = ""
[config.items]
type = "list"
side = "client"
element = "string"
default = []
"#)).unwrap();
        let strings = vec![Value::String("x".repeat(MAX_TEXT_BYTES)); 17];
        assert!(manifest.config["items"].validate_value(&Value::Array(strings)).is_err());
        let fields = (0..17).map(|index| {
            (format!("message{index}"), manifest.config["message"].clone())
        }).collect();
        let supplied = (0..17).map(|index| {
            (format!("message{index}"), Value::String("x".repeat(MAX_TEXT_BYTES)))
        }).collect();
        assert!(resolve_config(&fields, &supplied).is_err());
        let oversized_defaults = (0..17).map(|index| {
            let mut field = manifest.config["message"].clone();
            field.default = Value::String("x".repeat(MAX_TEXT_BYTES));
            (format!("message{index}"), field)
        }).collect();
        assert!(validate_declarations(&oversized_defaults, &BTreeMap::new()).is_err());
    }

    #[test]
    fn legacy_manifest_serialization_does_not_add_declarations() {
        let manifest = Manifest::parse(BASE).unwrap();
        let serialized = toml::to_string(&manifest).unwrap();
        let parsed: Value = toml::from_str(&serialized).unwrap();
        assert!(parsed.get("config").is_none());
        assert!(parsed.get("permissions").is_none());
    }

    #[test]
    fn rejects_invalid_schema_and_keeps_old_manifest_valid() {
        assert!(Manifest::parse(BASE).unwrap().config.is_empty());
        for declaration in [
            "[config.a]\ntype = 'int'\nside = 'server'\ndefault = 1\nmin = 2\n",
            "[config.a]\ntype = 'enum'\nside = 'server'\ndefault = ''\nvalues = ['']\n",
            "[config.a]\ntype = 'list'\nside = 'client'\ndefault = []\nelement = 'list'\n",
            "[config.a]\ntype = 'enum'\nside = 'server'\ndefault = 'a'\nvalues = ['a', 'a']\n",
            "[config.a]\ntype = 'float'\nside = 'server'\ndefault = nan\n",
            "[permissions.'example.use']\ndefault_op_level = 5\n",
        ] {
            assert!(Manifest::parse(&format!("{BASE}{declaration}")).is_err());
        }
    }
}
