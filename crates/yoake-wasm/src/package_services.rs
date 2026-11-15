use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use horizon_mods::yak::{
    ConfigField, ConfigFile, ConfigKind, ConfigRuntimeSide, PermissionNode, PermissionOverrides,
};
use toml::Value;

use crate::host_services::storage::DiskStorage;
use crate::server_types::{ConfigValue, ConfigValueKind, StorageScope};

pub type ServiceHandle = Arc<Mutex<PackageServices>>;
pub const CONFIG_CAPABILITY: &str = "service.config";
pub const PERMISSIONS_CAPABILITY: &str = "service.permissions";
pub const STORAGE_CAPABILITY: &str = "service.storage";
pub const MAX_SYNC_BYTES: usize = 1024 * 1024;
pub const MAX_WRITE_BYTES_PER_SECOND: usize = 64 * 1024 * 1024;

struct PermissionCache {
    checked: Instant,
    stamp: Option<(SystemTime, u64)>,
    overrides: Result<PermissionOverrides, String>,
}

pub struct PackageServices {
    id: String,
    capabilities: BTreeSet<String>,
    fields: BTreeMap<String, ConfigField>,
    permissions: BTreeMap<String, PermissionNode>,
    config: Result<ConfigFile, String>,
    side: ConfigRuntimeSide,
    global: PathBuf,
    world: Option<PathBuf>,
    stores: BTreeMap<u8, DiskStorage>,
    tickets: BTreeMap<u64, (u8, u64)>,
    next_ticket: u64,
    changes: BTreeMap<String, Value>,
    revision: u64,
    snapshot_cache: Option<(u64, Arc<Vec<u8>>)>,
    permission_cache: Option<PermissionCache>,
    write_window: Instant,
    write_bytes: usize,
}

impl PackageServices {
    pub fn open(
        id: &str,
        capabilities: &[String],
        fields: &BTreeMap<String, ConfigField>,
        permissions: &BTreeMap<String, PermissionNode>,
        side: ConfigRuntimeSide,
        global: &Path,
        world: Option<&Path>,
    ) -> Result<Self, String> {
        Ok(Self {
            id: id.into(),
            capabilities: capabilities.iter().cloned().collect(),
            fields: fields.clone(),
            permissions: permissions.clone(),
            config: ConfigFile::load(global, id, fields, side).inspect_err(|error| {
                eprintln!("[yoake] config {id}: {error}");
            }),
            side,
            global: global.into(),
            world: world.map(Path::to_path_buf),
            stores: BTreeMap::new(),
            tickets: BTreeMap::new(),
            next_ticket: 0,
            changes: BTreeMap::new(),
            revision: 0,
            snapshot_cache: None,
            permission_cache: None,
            write_window: Instant::now(),
            write_bytes: 0,
        })
    }

    pub fn require(&self, capability: &str) -> Result<(), String> {
        if self.capabilities.contains(capability) {
            Ok(())
        } else {
            Err(format!("permission denied: {capability}"))
        }
    }

    pub fn bind_world(&mut self, world: Option<&Path>) {
        if self.world.as_deref() != world {
            self.world = world.map(Path::to_path_buf);
            self.stores.remove(&0);
            self.tickets.retain(|_, (scope, _)| *scope != 0);
            self.permission_cache = None;
        }
    }

    pub fn config_get(&self, key: &str) -> Result<ConfigValue, String> {
        self.require(CONFIG_CAPABILITY)?;
        let field = self.fields.get(key).ok_or("unknown config key")?;
        encode_value(
            field,
            self.config
                .as_ref()
                .map_err(Clone::clone)?
                .state()
                .get(key)
                .ok_or("unknown config key")?,
        )
    }

    pub fn settings_value(&self, key: &str) -> Option<&Value> {
        self.config.as_ref().ok()?.state().get(key)
    }

    pub fn config_set(&mut self, key: &str, value: ConfigValue) -> Result<(), String> {
        self.require(CONFIG_CAPABILITY)?;
        let field = self.fields.get(key).ok_or("unknown config key")?;
        let value = decode_value(field, value)?;
        self.save_config(&BTreeMap::from([(key.into(), value)]))
    }

    pub fn save_config(&mut self, values: &BTreeMap<String, Value>) -> Result<(), String> {
        let changes = self
            .config
            .as_mut()
            .map_err(|error| error.clone())?
            .set_many(values)?;
        if !changes.is_empty() {
            self.revision = self.revision.wrapping_add(1);
            for change in changes {
                self.changes.insert(change.name, change.new);
            }
        }
        Ok(())
    }

    pub fn take_changes(&mut self) -> Vec<(String, ConfigValue)> {
        std::mem::take(&mut self.changes)
            .into_iter()
            .filter_map(|(key, value)| {
                encode_value(self.fields.get(&key)?, &value)
                    .ok()
                    .map(|value| (key, value))
            })
            .collect()
    }

    pub fn snapshot(&mut self) -> Result<(u64, Arc<Vec<u8>>), String> {
        if let Some((revision, data)) = &self.snapshot_cache
            && *revision == self.revision
        {
            return Ok((*revision, data.clone()));
        }
        let values = self
            .config
            .as_ref()
            .map_err(Clone::clone)?
            .state()
            .synced_snapshot()?;
        let data = toml::to_string(&values)
            .map_err(|error| error.to_string())?
            .into_bytes();
        if data.len() > MAX_SYNC_BYTES {
            return Err("config snapshot too large".into());
        }
        let data = Arc::new(data);
        self.snapshot_cache = Some((self.revision, data.clone()));
        Ok((self.revision, data))
    }

    pub fn apply_snapshot(&mut self, data: &[u8]) -> Result<(), String> {
        if data.len() > MAX_SYNC_BYTES {
            return Err("config snapshot too large".into());
        }
        let text = std::str::from_utf8(data).map_err(|error| error.to_string())?;
        let values: BTreeMap<String, Value> =
            toml::from_str(text).map_err(|error| error.to_string())?;
        for change in self
            .config
            .as_mut()
            .map_err(|error| error.clone())?
            .apply_synced_snapshot(&values)?
        {
            self.changes.insert(change.name, change.new);
        }
        Ok(())
    }

    pub fn clear_snapshot(&mut self) {
        let Ok(config) = &mut self.config else {
            return;
        };
        for change in config.clear_synced() {
            self.changes.insert(change.name, change.new);
        }
    }

    pub fn has_permission(
        &mut self,
        node: &str,
        player: &str,
        op_level: u8,
    ) -> Result<bool, String> {
        self.require(PERMISSIONS_CAPABILITY)?;
        if self.side != ConfigRuntimeSide::Server {
            return Err("server service only".into());
        }
        let world = self.world.as_deref().ok_or("no world open")?;
        if self
            .permission_cache
            .as_ref()
            .is_none_or(|cache| cache.checked.elapsed() >= Duration::from_secs(1))
        {
            let stamp = std::fs::metadata(world.join("permissions.toml"))
                .ok()
                .and_then(|metadata| Some((metadata.modified().ok()?, metadata.len())));
            if let Some(cache) = &mut self.permission_cache
                && cache.stamp == stamp
            {
                cache.checked = Instant::now();
            } else {
                self.permission_cache = Some(PermissionCache {
                    checked: Instant::now(),
                    stamp,
                    overrides: PermissionOverrides::load_world(world),
                });
            }
        }
        let cache = self
            .permission_cache
            .as_ref()
            .ok_or("permissions unavailable")?;
        Ok(cache.overrides.as_ref().map_err(Clone::clone)?.check(
            &self.id,
            player,
            &self.permissions,
            node,
            op_level,
        ))
    }

    fn store(&mut self, scope: u8) -> Result<&mut DiskStorage, String> {
        self.require(STORAGE_CAPABILITY)?;
        let base = match (self.side, scope) {
            (ConfigRuntimeSide::Server, 0) => self.world.as_ref().ok_or("no world open")?,
            (ConfigRuntimeSide::Server, 1) | (ConfigRuntimeSide::Client, 2) => &self.global,
            _ => return Err("storage scope is unavailable on this side".into()),
        };
        if !self.stores.contains_key(&scope) {
            let store = match scope {
                0 => DiskStorage::open(base, &self.id)?,
                1 => DiskStorage::open_scoped(base, &self.id, "global")?,
                _ => DiskStorage::open_scoped(base, &self.id, "client")?,
            };
            self.stores.insert(scope, store);
        }
        self.stores
            .get_mut(&scope)
            .ok_or_else(|| "storage unavailable".into())
    }

    fn has_store(&self, scope: u8, key: &str) -> Result<bool, String> {
        self.require(STORAGE_CAPABILITY)?;
        crate::host_services::storage::validate_key(key)?;
        let base = match scope {
            0 if self.side == ConfigRuntimeSide::Server => {
                self.world.as_deref().ok_or("world storage unavailable")?
            }
            1 if self.side == ConfigRuntimeSide::Server => self.global.as_path(),
            2 if self.side == ConfigRuntimeSide::Client => self.global.as_path(),
            _ => return Err("storage scope unavailable".into()),
        };
        Ok(self.stores.contains_key(&scope)
            || DiskStorage::exists(
                base,
                &self.id,
                match scope {
                    1 => Some("global"),
                    2 => Some("client"),
                    _ => None,
                },
            )?)
    }

    fn charge_write(&mut self, bytes: usize) -> Result<(), String> {
        self.require(STORAGE_CAPABILITY)?;
        if self.write_window.elapsed() >= Duration::from_secs(1) {
            self.write_window = Instant::now();
            self.write_bytes = 0;
        }
        let total = self
            .write_bytes
            .checked_add(bytes)
            .ok_or("storage write budget exceeded")?;
        if total > MAX_WRITE_BYTES_PER_SECOND {
            return Err("storage write budget exceeded".into());
        }
        self.write_bytes = total;
        Ok(())
    }

    pub fn abort_all_tickets(&mut self) {
        self.tickets.clear();
        for store in self.stores.values_mut() {
            store.abort_all();
        }
    }

    pub fn storage_get(
        &mut self,
        key: &str,
        scope: StorageScope,
    ) -> Result<Option<Vec<u8>>, String> {
        if !self.has_store(scope_id(scope), key)? {
            return Ok(None);
        }
        let value = self.store(scope_id(scope))?.get(key)?;
        if value
            .as_ref()
            .is_some_and(|value| value.len() > crate::host_services::MAX_MESSAGE_BYTES)
        {
            return Err("value requires storage-blob-read".into());
        }
        Ok(value)
    }

    pub fn storage_put(
        &mut self,
        key: &str,
        scope: StorageScope,
        value: &[u8],
    ) -> Result<(), String> {
        self.require(STORAGE_CAPABILITY)?;
        if value.len() > crate::host_services::MAX_MESSAGE_BYTES {
            return Err("value requires blob upload".into());
        }
        self.charge_write(value.len() + key.len() + 6)?;
        self.store(scope_id(scope))?.put(key, value)
    }

    pub fn storage_delete(&mut self, key: &str, scope: StorageScope) -> Result<bool, String> {
        if !self.has_store(scope_id(scope), key)? {
            return Ok(false);
        }
        self.charge_write(key.len() + 6)?;
        self.store(scope_id(scope))?.delete(key)
    }

    pub fn storage_blob_begin(
        &mut self,
        key: &str,
        scope: StorageScope,
        total: u64,
    ) -> Result<u64, String> {
        self.require(STORAGE_CAPABILITY)?;
        if self.tickets.len() >= 64 {
            return Err("too many blob tickets".into());
        }
        let next = self
            .next_ticket
            .checked_add(1)
            .ok_or("blob ticket exhausted")?;
        let scope = scope_id(scope);
        let total = usize::try_from(total).map_err(|_| "blob too large")?;
        let pending: usize = self.stores.values().map(DiskStorage::pending_bytes).sum();
        if total > crate::host_services::storage::MAX_PENDING_BYTES.saturating_sub(pending) {
            return Err("pending blob byte limit exceeded".into());
        }
        let ticket = self
            .store(scope)?
            .begin_blob(key, total)
            .map_err(str::to_owned)?;
        self.next_ticket = next;
        self.tickets.insert(next, (scope, ticket));
        Ok(next)
    }

    fn ticket(&self, id: u64) -> Result<(u8, u64), String> {
        self.require(STORAGE_CAPABILITY)?;
        self.tickets
            .get(&id)
            .copied()
            .ok_or_else(|| "unknown blob ticket".into())
    }

    pub fn storage_blob_write(&mut self, data: &[u8], id: u64, offset: u64) -> Result<(), String> {
        let (scope, ticket) = self.ticket(id)?;
        let offset = usize::try_from(offset).map_err(|_| "invalid blob offset")?;
        self.store(scope)?
            .write_blob(ticket, offset, data)
            .map_err(str::to_owned)
    }

    pub fn storage_blob_commit(&mut self, id: u64) -> Result<(), String> {
        let (scope, ticket) = self.ticket(id)?;
        let bytes = self
            .store(scope)?
            .blob_size(ticket)
            .ok_or("unknown blob ticket")?;
        self.charge_write(bytes + 262)?;
        self.store(scope)?.commit_blob(ticket)?;
        self.tickets.remove(&id);
        Ok(())
    }

    pub fn storage_blob_abort(&mut self, id: u64) -> Result<bool, String> {
        self.require(STORAGE_CAPABILITY)?;
        let Some((scope, ticket)) = self.tickets.remove(&id) else {
            return Ok(false);
        };
        Ok(self.store(scope)?.abort_blob(ticket))
    }

    pub fn storage_blob_read(
        &mut self,
        key: &str,
        length: u32,
        offset: u64,
        scope: StorageScope,
    ) -> Result<Vec<u8>, String> {
        self.require(STORAGE_CAPABILITY)?;
        if !self.has_store(scope_id(scope), key)? {
            return Ok(Vec::new());
        }
        let offset = usize::try_from(offset).map_err(|_| "invalid blob offset")?;
        Ok(self
            .store(scope_id(scope))?
            .read_blob(key, offset, length as usize)?
            .unwrap_or_default())
    }
}

fn scope_id(scope: StorageScope) -> u8 {
    match scope {
        StorageScope::World => 0,
        StorageScope::Global => 1,
        StorageScope::Client => 2,
    }
}

pub fn encode_value(field: &ConfigField, value: &Value) -> Result<ConfigValue, String> {
    let mut result = ConfigValue {
        kind: ConfigValueKind::Bool,
        bool_value: false,
        int_value: 0,
        float_value: 0.0,
        string_value: String::new(),
        bool_list: Vec::new(),
        int_list: Vec::new(),
        float_list: Vec::new(),
        string_list: Vec::new(),
    };
    result.kind = match (field.kind, field.element) {
        (ConfigKind::Bool, _) => {
            result.bool_value = value.as_bool().ok_or("expected bool")?;
            ConfigValueKind::Bool
        }
        (ConfigKind::Int, _) => {
            result.int_value = value.as_integer().ok_or("expected int")?;
            ConfigValueKind::Int
        }
        (ConfigKind::Float, _) => {
            result.float_value = value.as_float().ok_or("expected float")?;
            ConfigValueKind::Float
        }
        (ConfigKind::String | ConfigKind::Enum, _) => {
            result.string_value = value.as_str().ok_or("expected string")?.into();
            if field.kind == ConfigKind::Enum {
                ConfigValueKind::Enum
            } else {
                ConfigValueKind::String
            }
        }
        (ConfigKind::List, Some(element)) => {
            let values = value.as_array().ok_or("expected list")?;
            match element {
                ConfigKind::Bool => {
                    result.bool_list = values
                        .iter()
                        .map(|value| value.as_bool().ok_or("expected bool"))
                        .collect::<Result<_, _>>()?;
                    ConfigValueKind::BoolList
                }
                ConfigKind::Int => {
                    result.int_list = values
                        .iter()
                        .map(|value| value.as_integer().ok_or("expected int"))
                        .collect::<Result<_, _>>()?;
                    ConfigValueKind::IntList
                }
                ConfigKind::Float => {
                    result.float_list = values
                        .iter()
                        .map(|value| value.as_float().ok_or("expected float"))
                        .collect::<Result<_, _>>()?;
                    ConfigValueKind::FloatList
                }
                ConfigKind::String | ConfigKind::Enum => {
                    result.string_list = values
                        .iter()
                        .map(|value| value.as_str().map(str::to_owned).ok_or("expected string"))
                        .collect::<Result<_, _>>()?;
                    if element == ConfigKind::Enum {
                        ConfigValueKind::EnumList
                    } else {
                        ConfigValueKind::StringList
                    }
                }
                ConfigKind::List => return Err("nested config list".into()),
            }
        }
        _ => return Err("missing list element type".into()),
    };
    Ok(result)
}

fn decode_value(field: &ConfigField, value: ConfigValue) -> Result<Value, String> {
    let expected = encode_value(field, &field.default)?.kind;
    if value.kind != expected {
        return Err("config type mismatch".into());
    }
    let result = match value.kind {
        ConfigValueKind::Bool => Value::Boolean(value.bool_value),
        ConfigValueKind::Int => Value::Integer(value.int_value),
        ConfigValueKind::Float => Value::Float(value.float_value),
        ConfigValueKind::String | ConfigValueKind::Enum => Value::String(value.string_value),
        ConfigValueKind::BoolList => {
            Value::Array(value.bool_list.into_iter().map(Value::Boolean).collect())
        }
        ConfigValueKind::IntList => {
            Value::Array(value.int_list.into_iter().map(Value::Integer).collect())
        }
        ConfigValueKind::FloatList => {
            Value::Array(value.float_list.into_iter().map(Value::Float).collect())
        }
        ConfigValueKind::StringList | ConfigValueKind::EnumList => {
            Value::Array(value.string_list.into_iter().map(Value::String).collect())
        }
    };
    field.validate_value(&result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(0);

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "yoake-package-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT_ROOT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn fields() -> BTreeMap<String, ConfigField> {
        toml::from_str(
            r#"
[count]
type = "int"
default = 2
min = 1
max = 4
side = "server"
[local]
type = "bool"
default = false
side = "client"
[shared]
type = "bool"
default = false
side = "synced"
"#,
        )
        .unwrap()
    }

    fn open(root: &Path, side: ConfigRuntimeSide, capabilities: &[&str]) -> PackageServices {
        PackageServices::open(
            "example",
            &capabilities
                .iter()
                .map(|value| (*value).into())
                .collect::<Vec<_>>(),
            &fields(),
            &BTreeMap::from([(
                "example.edit".into(),
                PermissionNode {
                    default_op_level: 2,
                },
            )]),
            side,
            root,
            Some(root),
        )
        .unwrap()
    }

    #[test]
    fn denied_services_do_not_create_files() {
        let root = Scratch::new();
        let mut service = open(&root.0, ConfigRuntimeSide::Server, &[]);
        let value = encode_value(&fields()["shared"], &Value::Boolean(true)).unwrap();
        for error in [
            service.config_get("shared").unwrap_err(),
            service.config_set("shared", value).unwrap_err(),
            service
                .has_permission("example.edit", "0123456789abcdef0123456789abcdef", 4)
                .unwrap_err(),
            service
                .storage_get("value", StorageScope::World)
                .unwrap_err(),
            service
                .storage_put("value", StorageScope::World, b"secret")
                .unwrap_err(),
            service
                .storage_delete("value", StorageScope::World)
                .unwrap_err(),
            service
                .storage_blob_begin("value", StorageScope::World, 4)
                .unwrap_err(),
            service.storage_blob_write(b"data", 1, 0).unwrap_err(),
            service.storage_blob_commit(1).unwrap_err(),
            service.storage_blob_abort(1).unwrap_err(),
            service
                .storage_blob_read("value", 4, 0, StorageScope::World)
                .unwrap_err(),
        ] {
            assert!(error.starts_with("permission denied:"), "{error}");
        }
        assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
    }

    #[test]
    fn config_validates_persists_and_coalesces_changes() {
        let root = Scratch::new();
        let mut service = open(&root.0, ConfigRuntimeSide::Server, &[CONFIG_CAPABILITY]);
        let mut value = service.config_get("count").unwrap();
        service.config_set("count", value.clone()).unwrap();
        assert!(!root.0.join("config").exists());
        value.int_value = 5;
        assert!(service.config_set("count", value.clone()).is_err());
        value.int_value = 3;
        service.config_set("count", value.clone()).unwrap();
        value.int_value = 4;
        service.config_set("count", value).unwrap();
        assert_eq!(
            service
                .take_changes()
                .iter()
                .map(|(key, value)| (key.as_str(), value.int_value))
                .collect::<Vec<_>>(),
            [("count", 4)]
        );
        assert!(service.take_changes().is_empty());
        assert_eq!(
            open(&root.0, ConfigRuntimeSide::Server, &[CONFIG_CAPABILITY])
                .config_get("count")
                .unwrap()
                .int_value,
            4
        );
        let wrong = encode_value(&fields()["shared"], &Value::Boolean(true)).unwrap();
        assert!(service.config_set("count", wrong.clone()).is_err());
        assert!(service.config_set("local", wrong).is_err());
    }

    #[test]
    fn synced_config_is_client_read_only_and_resets_without_disk_writes() {
        let root = Scratch::new();
        let mut server = open(&root.0, ConfigRuntimeSide::Server, &[CONFIG_CAPABILITY]);
        let client_root = Scratch::new();
        let mut client = open(
            &client_root.0,
            ConfigRuntimeSide::Client,
            &[CONFIG_CAPABILITY],
        );
        let value = encode_value(&fields()["shared"], &Value::Boolean(true)).unwrap();
        assert!(client.config_set("shared", value.clone()).is_err());
        server.config_set("shared", value).unwrap();
        let (revision, snapshot) = server.snapshot().unwrap();
        assert_eq!(revision, 1);
        assert_eq!(
            std::str::from_utf8(&snapshot).unwrap().trim(),
            "shared = true"
        );
        client.apply_snapshot(&snapshot).unwrap();
        assert!(client.config_get("shared").unwrap().bool_value);
        assert_eq!(client.take_changes().len(), 1);
        client.apply_snapshot(&snapshot).unwrap();
        assert!(client.take_changes().is_empty());
        assert!(client.apply_snapshot(b"shared = 1").is_err());
        assert!(client.apply_snapshot(b"local = true").is_err());
        assert!(
            client
                .apply_snapshot(&vec![b' '; MAX_SYNC_BYTES + 1])
                .is_err()
        );
        assert!(client.config_get("shared").unwrap().bool_value);
        client.clear_snapshot();
        assert!(!client.config_get("shared").unwrap().bool_value);
        assert_eq!(client.take_changes().len(), 1);
        assert_eq!(std::fs::read_dir(&client_root.0).unwrap().count(), 0);
    }

    #[test]
    fn storage_scopes_blobs_and_world_switches_are_isolated() {
        let root = Scratch::new();
        let other = Scratch::new();
        let mut server = open(&root.0, ConfigRuntimeSide::Server, &[STORAGE_CAPABILITY]);
        assert_eq!(
            server.storage_get("value", StorageScope::World).unwrap(),
            None
        );
        assert_eq!(std::fs::read_dir(&root.0).unwrap().count(), 0);
        server
            .storage_put("value", StorageScope::World, b"world")
            .unwrap();
        server
            .storage_put("value", StorageScope::Global, b"global")
            .unwrap();
        assert!(
            server
                .storage_put("value", StorageScope::Client, b"client")
                .is_err()
        );
        let ticket = server
            .storage_blob_begin("blob", StorageScope::World, 6)
            .unwrap();
        server.storage_blob_write(b"abc", ticket, 0).unwrap();
        assert!(server.storage_blob_commit(ticket).is_err());
        server.storage_blob_write(b"def", ticket, 3).unwrap();
        server.storage_blob_commit(ticket).unwrap();
        assert_eq!(
            server
                .storage_blob_read("blob", 3, 2, StorageScope::World)
                .unwrap(),
            b"cde"
        );
        assert!(!server.storage_blob_abort(ticket).unwrap());
        let stale = server
            .storage_blob_begin("pending", StorageScope::World, 4)
            .unwrap();
        server.bind_world(Some(&other.0));
        assert!(server.storage_blob_write(b"data", stale, 0).is_err());
        assert_eq!(
            server.storage_get("value", StorageScope::World).unwrap(),
            None
        );
        assert_eq!(
            server
                .storage_get("value", StorageScope::Global)
                .unwrap()
                .unwrap(),
            b"global"
        );
        server.bind_world(Some(&root.0));
        assert_eq!(
            server
                .storage_get("value", StorageScope::World)
                .unwrap()
                .unwrap(),
            b"world"
        );
        let mut client = open(&root.0, ConfigRuntimeSide::Client, &[STORAGE_CAPABILITY]);
        client
            .storage_put("value", StorageScope::Client, b"client")
            .unwrap();
        assert!(client.storage_get("value", StorageScope::World).is_err());
        assert_eq!(
            client
                .storage_get("value", StorageScope::Client)
                .unwrap()
                .unwrap(),
            b"client"
        );
        assert!(
            server
                .storage_blob_begin("huge", StorageScope::World, 256 * 1024 * 1024)
                .is_err()
        );
        assert!(
            server
                .storage_put("../escape", StorageScope::World, b"bad")
                .is_err()
        );
    }

    #[test]
    fn broken_config_is_cached_without_disabling_other_services() {
        let root = Scratch::new();
        std::fs::create_dir(root.0.join("config")).unwrap();
        let config = root.0.join("config/example.toml");
        std::fs::write(&config, "count = [").unwrap();
        let mut service = open(
            &root.0,
            ConfigRuntimeSide::Server,
            &[
                CONFIG_CAPABILITY,
                STORAGE_CAPABILITY,
                PERMISSIONS_CAPABILITY,
            ],
        );
        assert!(service.config_get("count").is_err());
        std::fs::write(config, "count = 3\n").unwrap();
        assert!(service.config_get("count").is_err());
        assert!(service.snapshot().is_err());
        service
            .storage_put("value", StorageScope::World, b"kept")
            .unwrap();
        assert_eq!(
            service
                .storage_get("value", StorageScope::World)
                .unwrap()
                .unwrap(),
            b"kept"
        );
        assert!(
            service
                .has_permission("example.edit", "0123456789abcdef0123456789abcdef", 2)
                .unwrap()
        );
    }

    #[test]
    fn snapshots_reuse_bytes_until_the_revision_changes() {
        let root = Scratch::new();
        let mut service = open(&root.0, ConfigRuntimeSide::Server, &[CONFIG_CAPABILITY]);
        let (revision, first) = service.snapshot().unwrap();
        let (same_revision, second) = service.snapshot().unwrap();
        assert_eq!(revision, same_revision);
        assert!(Arc::ptr_eq(&first, &second));
        let value = encode_value(&fields()["shared"], &Value::Boolean(true)).unwrap();
        service.config_set("shared", value).unwrap();
        let (new_revision, third) = service.snapshot().unwrap();
        assert!(new_revision > revision);
        assert!(!Arc::ptr_eq(&first, &third));
        assert_ne!(first, third);
    }

    #[test]
    fn write_budget_rejects_a_mutation_without_losing_its_ticket() {
        let root = Scratch::new();
        let mut service = open(&root.0, ConfigRuntimeSide::Server, &[STORAGE_CAPABILITY]);
        let ticket = service
            .storage_blob_begin("blob", StorageScope::World, 4)
            .unwrap();
        service.storage_blob_write(b"data", ticket, 0).unwrap();
        service.write_bytes = MAX_WRITE_BYTES_PER_SECOND;
        assert!(
            service
                .storage_put("value", StorageScope::World, b"denied")
                .unwrap_err()
                .contains("write budget")
        );
        assert!(
            service
                .storage_delete("value", StorageScope::World)
                .unwrap_err()
                .contains("write budget")
        );
        assert!(
            service
                .storage_blob_commit(ticket)
                .unwrap_err()
                .contains("write budget")
        );
        assert_eq!(
            service.storage_get("blob", StorageScope::World).unwrap(),
            None
        );
        service.write_window = Instant::now() - Duration::from_secs(2);
        service.storage_blob_commit(ticket).unwrap();
        assert_eq!(
            service
                .storage_get("blob", StorageScope::World)
                .unwrap()
                .unwrap(),
            b"data"
        );
    }

    #[test]
    fn pending_bytes_are_shared_between_scopes_and_released_on_abort() {
        let root = Scratch::new();
        let mut service = open(&root.0, ConfigRuntimeSide::Server, &[STORAGE_CAPABILITY]);
        let size = 17 * 1024 * 1024;
        let ticket = service
            .storage_blob_begin("world", StorageScope::World, size)
            .unwrap();
        assert!(
            service
                .storage_blob_begin("global", StorageScope::Global, size)
                .is_err()
        );
        service.abort_all_tickets();
        assert!(service.storage_blob_write(b"x", ticket, 0).is_err());
        assert!(
            service
                .storage_blob_begin("global", StorageScope::Global, size)
                .is_ok()
        );
    }

    #[test]
    fn permission_defaults_and_world_overrides_reach_the_host() {
        let root = Scratch::new();
        let player = "0123456789abcdef0123456789abcdef";
        let mut service = open(
            &root.0,
            ConfigRuntimeSide::Server,
            &[PERMISSIONS_CAPABILITY],
        );
        assert!(!service.has_permission("example.edit", player, 1).unwrap());
        assert!(service.has_permission("example.edit", player, 2).unwrap());
        assert!(
            !service
                .has_permission("example.unknown", player, 4)
                .unwrap()
        );
        std::fs::write(
            root.0.join("permissions.toml"),
            format!("[mods.example.players.'{player}']\n'example.edit' = false\n"),
        )
        .unwrap();
        assert!(service.has_permission("example.edit", player, 4).unwrap());
        service.permission_cache.as_mut().unwrap().checked =
            Instant::now() - Duration::from_secs(2);
        assert!(!service.has_permission("example.edit", player, 4).unwrap());
        std::fs::write(root.0.join("permissions.toml"), format!("[mods.example.groups.builders]\n'example.edit' = true\n[mods.example.members]\n'{player}' = ['builders']\n")).unwrap();
        service.permission_cache.as_mut().unwrap().checked =
            Instant::now() - Duration::from_secs(2);
        assert!(service.has_permission("example.edit", player, 0).unwrap());
        assert!(
            open(
                &root.0,
                ConfigRuntimeSide::Client,
                &[PERMISSIONS_CAPABILITY]
            )
            .has_permission("example.edit", player, 4)
            .is_err()
        );
    }
}
