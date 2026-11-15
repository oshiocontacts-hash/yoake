use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use horizon_mods::yak::ConfigRuntimeSide;
use yoake_wasm::config_sync::Snapshot;
use yoake_wasm::package_services::{PackageServices, ServiceHandle};

#[derive(Default)]
struct RegistryState {
    handles: BTreeMap<(String, bool), ServiceHandle>,
}

#[derive(Default)]
struct Registry(Mutex<RegistryState>);

static SERVICES: OnceLock<Registry> = OnceLock::new();

impl Registry {
    fn load(
        &self,
        id: &str,
        side: ConfigRuntimeSide,
        open: impl FnOnce() -> Result<PackageServices, String>,
    ) -> Result<ServiceHandle, String> {
        let key = (id.to_owned(), side == ConfigRuntimeSide::Server);
        if let Some(handle) = self
            .0
            .lock()
            .map_err(|_| "package services unavailable")?
            .handles
            .get(&key)
            .cloned()
        {
            return Ok(handle);
        }
        let handle = Arc::new(Mutex::new(open()?));
        let mut cache = self.0.lock().map_err(|_| "package services unavailable")?;
        Ok(cache.handles.entry(key).or_insert(handle).clone())
    }

    fn bind_world(&self, id: &str, world: Option<&Path>) {
        let handle = {
            let Ok(cache) = self.0.lock() else { return };
            cache.handles.get(&(id.into(), true)).cloned()
        };
        if let Some(handle) = handle
            && let Ok(mut service) = handle.lock()
        {
            service.bind_world(world);
        }
    }
}

pub(super) fn bind_world(id: &str, world: Option<&Path>) {
    if world.is_some() {
        let _ = services(id, ConfigRuntimeSide::Server);
    }
    if let Some(registry) = SERVICES.get() {
        registry.bind_world(id, world);
    }
}

pub(super) fn abort_tickets(id: &str, side: ConfigRuntimeSide) {
    let handle = SERVICES.get().and_then(|registry| {
        registry
            .0
            .lock()
            .ok()?
            .handles
            .get(&(id.into(), side == ConfigRuntimeSide::Server))
            .cloned()
    });
    if let Some(handle) = handle
        && let Ok(mut service) = handle.lock()
    {
        service.abort_all_tickets();
    }
}

pub(crate) fn services(id: &str, side: ConfigRuntimeSide) -> Result<ServiceHandle, String> {
    if !crate::yak_mods::entries()
        .iter()
        .any(|entry| entry.manifest.id == id && entry.manifest.wasm.is_some())
    {
        return Err("not a WASM package".into());
    }
    load(id, side)
}

pub(crate) fn settings_services(
    id: &str,
    side: ConfigRuntimeSide,
) -> Result<ServiceHandle, String> {
    load(id, side)
}

fn load(id: &str, side: ConfigRuntimeSide) -> Result<ServiceHandle, String> {
    let entries = crate::yak_mods::entries();
    let entry = entries
        .iter()
        .find(|entry| entry.manifest.id == id)
        .ok_or("unknown package")?;
    let capabilities = entry
        .manifest
        .wasm
        .as_ref()
        .map(|wasm| wasm.permissions.as_slice())
        .unwrap_or_default();
    let root = crate::yak_mods::game_root().ok_or("package root unavailable")?;
    SERVICES.get_or_init(Default::default).load(id, side, || {
        PackageServices::open(
            id,
            capabilities,
            &entry.manifest.config,
            &entry.manifest.permissions,
            side,
            root,
            None,
        )
    })
}

pub(crate) fn snapshots() -> Result<Vec<Snapshot>, String> {
    collect_snapshots(
        crate::yak_mods::entries()
            .iter()
            .filter(|entry| entry.has_synced_config())
            .map(|entry| {
                let handle = services(&entry.manifest.id, ConfigRuntimeSide::Server)?;
                let (revision, data) = handle
                    .lock()
                    .map_err(|_| "package services unavailable")?
                    .snapshot()?;
                Ok(Snapshot {
                    id: entry.manifest.id.clone(),
                    revision,
                    data,
                })
            }),
    )
}

fn collect_snapshots(
    candidates: impl Iterator<Item = Result<Snapshot, String>>,
) -> Result<Vec<Snapshot>, String> {
    let mut snapshots = Vec::new();
    let mut total = 0usize;
    for candidate in candidates {
        let Ok(snapshot) = candidate else { continue };
        total = total
            .checked_add(snapshot.data.len())
            .ok_or("config sync aggregate limit exceeded")?;
        if total > yoake_wasm::config_sync::MAX_TOTAL_BYTES
            || snapshots.len() >= yoake_wasm::config_sync::MAX_MODS
        {
            return Err("config sync aggregate limit exceeded".into());
        }
        snapshots.push(snapshot);
    }
    Ok(snapshots)
}

pub(crate) fn apply_snapshot(id: &str, data: &[u8]) -> Result<(), String> {
    let handle = services(id, ConfigRuntimeSide::Client)?;
    let mut service = handle.lock().map_err(|_| "package services unavailable")?;
    service.require(yoake_wasm::package_services::CONFIG_CAPABILITY)?;
    service.apply_snapshot(data)
}

pub(crate) fn clear_snapshots() {
    let Some(cache) = SERVICES.get() else {
        return;
    };
    let Ok(cache) = cache.0.lock() else {
        return;
    };
    let handles: Vec<_> = cache
        .handles
        .iter()
        .filter(|((_, server), _)| !server)
        .map(|(_, handle)| handle.clone())
        .collect();
    drop(cache);
    for handle in handles {
        if let Ok(mut service) = handle.lock() {
            service.clear_snapshot();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use yoake_wasm::server_types::StorageScope;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "yoake-registry-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root.canonicalize().unwrap())
        }

        fn open(&self) -> Result<PackageServices, String> {
            PackageServices::open(
                "sample-mod",
                &["service.storage".into()],
                &BTreeMap::new(),
                &BTreeMap::new(),
                ConfigRuntimeSide::Server,
                &self.0,
                None,
            )
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn world_lifecycle_binds_storage_until_close_and_never_reuses_a_folder() {
        use crate::mod_api::{
            Hooks, Host, HudBatch, NativeAuthority, NativeLifecycleContext, NativeTickId,
            NativeWorldEpoch, ServerAboutToStartEvent, ServerStoppingEvent, WorldLifecycleEvent,
            WorldPhase,
        };
        use yoake_wasm::host_services::storage::DiskStorage;

        struct WorldHost(Option<PathBuf>);
        impl Host for WorldHost {
            fn window(&self) -> bool {
                false
            }
            fn text(&self, _: &mut HudBatch, _: &str, _: [f32; 2], _: u32, _: bool) {}
            fn fill(&self, _: &mut HudBatch, _: [f32; 4], _: u32) {}
            fn world_dir(&self) -> Option<PathBuf> {
                self.0.clone()
            }
        }

        let root = Scratch::new();
        let first = root.0.join("first-world");
        let second = root.0.join("second-world");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let id = "lifecycle-storage-test";
        let registry = SERVICES.get_or_init(Default::default);
        let handle = registry
            .load(id, ConfigRuntimeSide::Server, || {
                PackageServices::open(
                    id,
                    &["service.storage".into()],
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                    ConfigRuntimeSide::Server,
                    &root.0,
                    None,
                )
            })
            .unwrap();
        let module = super::super::full::FullMod::new(id.into(), None, None);
        let mut host = WorldHost(None);
        let context = NativeLifecycleContext {
            authority: NativeAuthority::DedicatedServer,
            epoch: NativeWorldEpoch::new(1),
            tick: NativeTickId::new(0),
        };
        module.server_about_to_start(&mut host, &mut ServerAboutToStartEvent { dedicated: true });
        host.0 = Some(first.clone());
        module.world_lifecycle(
            &mut host,
            &mut WorldLifecycleEvent {
                context: &context,
                phase: WorldPhase::WorldOpenReady,
                error_text: "",
            },
        );
        handle
            .lock()
            .unwrap()
            .storage_put("value", StorageScope::World, b"first")
            .unwrap();
        let client_context = NativeLifecycleContext {
            authority: NativeAuthority::RemoteClient,
            ..context
        };
        module.world_lifecycle(
            &mut host,
            &mut WorldLifecycleEvent {
                context: &client_context,
                phase: WorldPhase::WorldCloseFinal,
                error_text: "",
            },
        );
        assert_eq!(
            handle
                .lock()
                .unwrap()
                .storage_get("value", StorageScope::World)
                .unwrap(),
            Some(b"first".to_vec())
        );
        module.server_stopping(&mut host, &mut ServerStoppingEvent { dedicated: true });
        handle
            .lock()
            .unwrap()
            .storage_put("final", StorageScope::World, b"saved")
            .unwrap();
        module.world_lifecycle(
            &mut host,
            &mut WorldLifecycleEvent {
                context: &context,
                phase: WorldPhase::WorldCloseFinal,
                error_text: "",
            },
        );
        assert!(
            handle
                .lock()
                .unwrap()
                .storage_get("value", StorageScope::World)
                .is_err()
        );
        module.server_about_to_start(&mut host, &mut ServerAboutToStartEvent { dedicated: true });
        assert!(
            handle
                .lock()
                .unwrap()
                .storage_get("value", StorageScope::World)
                .is_err()
        );
        host.0 = Some(second.clone());
        module.world_lifecycle(
            &mut host,
            &mut WorldLifecycleEvent {
                context: &context,
                phase: WorldPhase::WorldOpenReady,
                error_text: "",
            },
        );
        handle
            .lock()
            .unwrap()
            .storage_put("value", StorageScope::World, b"second")
            .unwrap();
        module.world_lifecycle(
            &mut host,
            &mut WorldLifecycleEvent {
                context: &context,
                phase: WorldPhase::WorldCloseFailed,
                error_text: "failed save",
            },
        );
        assert!(
            handle
                .lock()
                .unwrap()
                .storage_get("value", StorageScope::World)
                .is_err()
        );
        module.world_lifecycle(
            &mut host,
            &mut WorldLifecycleEvent {
                context: &context,
                phase: WorldPhase::WorldOpenReady,
                error_text: "",
            },
        );
        module.world_lifecycle(
            &mut host,
            &mut WorldLifecycleEvent {
                context: &context,
                phase: WorldPhase::WorldOpenFailed,
                error_text: "failed open",
            },
        );
        assert!(
            handle
                .lock()
                .unwrap()
                .storage_get("value", StorageScope::World)
                .is_err()
        );
        let first_store = DiskStorage::open(&first, id).unwrap();
        let second_store = DiskStorage::open(&second, id).unwrap();
        assert_eq!(first_store.get("value").unwrap(), Some(b"first".to_vec()));
        assert_eq!(first_store.get("final").unwrap(), Some(b"saved".to_vec()));
        assert_eq!(second_store.get("value").unwrap(), Some(b"second".to_vec()));
        assert_eq!(second_store.get("final").unwrap(), None);
        registry
            .0
            .lock()
            .unwrap()
            .handles
            .remove(&(id.into(), true));
    }

    #[test]
    fn registry_does_not_hold_map_lock_during_open_or_handle_use() {
        let root = Scratch::new();
        let registry = Registry::default();
        let handle = registry
            .load("sample-mod", ConfigRuntimeSide::Server, || {
                assert!(registry.0.try_lock().is_ok());
                root.open()
            })
            .unwrap();
        let _service = handle.lock().unwrap();
        let cached = registry
            .load("sample-mod", ConfigRuntimeSide::Server, || {
                panic!("a cached service must not reopen its config");
            })
            .unwrap();
        assert!(Arc::ptr_eq(&handle, &cached));
        assert!(registry.0.try_lock().is_ok());
    }

    #[test]
    fn registry_keeps_the_winner_of_a_double_checked_insert() {
        let root = Scratch::new();
        let registry = Registry::default();
        let mut inserted = None;
        let handle = registry
            .load("sample-mod", ConfigRuntimeSide::Server, || {
                inserted =
                    Some(registry.load("sample-mod", ConfigRuntimeSide::Server, || root.open())?);
                root.open()
            })
            .unwrap();
        assert!(Arc::ptr_eq(&handle, inserted.as_ref().unwrap()));
        assert_eq!(registry.0.lock().unwrap().handles.len(), 1);
    }

    #[test]
    fn nonbinding_access_preserves_world_store_and_pending_blob() {
        let root = Scratch::new();
        let registry = Registry::default();
        let handle = registry
            .load("sample-mod", ConfigRuntimeSide::Server, || root.open())
            .unwrap();
        registry.bind_world("sample-mod", Some(&root.0));
        let ticket = handle
            .lock()
            .unwrap()
            .storage_blob_begin("blob", StorageScope::World, 4)
            .unwrap();
        for _ in 0..3 {
            let service = registry
                .load("sample-mod", ConfigRuntimeSide::Server, || root.open())
                .unwrap();
            assert!(Arc::ptr_eq(&handle, &service));
        }
        {
            let mut service = handle.lock().unwrap();
            service.storage_blob_write(b"kept", ticket, 0).unwrap();
            service.storage_blob_commit(ticket).unwrap();
            assert_eq!(
                service.storage_get("blob", StorageScope::World).unwrap(),
                Some(b"kept".to_vec())
            );
        }
        registry.bind_world("sample-mod", None);
        assert!(
            handle
                .lock()
                .unwrap()
                .storage_get("blob", StorageScope::World)
                .is_err()
        );
        registry.bind_world("sample-mod", Some(&root.0));
        assert_eq!(
            handle
                .lock()
                .unwrap()
                .storage_get("blob", StorageScope::World)
                .unwrap(),
            Some(b"kept".to_vec())
        );
    }

    #[test]
    fn a_failing_snapshot_does_not_hide_healthy_mods() {
        let snapshots = collect_snapshots(
            [
                Err("invalid config".to_owned()),
                Ok(Snapshot {
                    id: "healthy-mod".into(),
                    revision: 7,
                    data: Arc::new(b"healthy".to_vec()),
                }),
            ]
            .into_iter(),
        )
        .unwrap();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].id, "healthy-mod");
        assert_eq!(snapshots[0].revision, 7);
    }
}
