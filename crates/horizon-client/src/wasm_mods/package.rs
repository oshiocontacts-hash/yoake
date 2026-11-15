use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use horizon_mods::yak::ConfigRuntimeSide;
use yoake_wasm::config_sync::Snapshot;
use yoake_wasm::package_services::{PackageServices, ServiceHandle};

static SERVICES: OnceLock<Mutex<BTreeMap<(String, bool), ServiceHandle>>> = OnceLock::new();

pub(crate) fn services(
    id: &str,
    side: ConfigRuntimeSide,
    world: Option<&Path>,
) -> Result<ServiceHandle, String> {
    if !crate::yak_mods::entries()
        .iter()
        .any(|entry| entry.manifest.id == id && entry.manifest.wasm.is_some())
    {
        return Err("not a WASM package".into());
    }
    load(id, side, world, true)
}

pub(crate) fn settings_services(
    id: &str,
    side: ConfigRuntimeSide,
) -> Result<ServiceHandle, String> {
    load(id, side, None, false)
}

fn load(
    id: &str,
    side: ConfigRuntimeSide,
    world: Option<&Path>,
    bind: bool,
) -> Result<ServiceHandle, String> {
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
    let mut cache = SERVICES
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "package services unavailable")?;
    let handle = cache.entry((id.into(), side == ConfigRuntimeSide::Server));
    let handle = match handle {
        std::collections::btree_map::Entry::Occupied(entry) => entry.into_mut(),
        std::collections::btree_map::Entry::Vacant(slot) => {
            slot.insert(Arc::new(Mutex::new(PackageServices::open(
                id,
                capabilities,
                &entry.manifest.config,
                &entry.manifest.permissions,
                side,
                root,
                world,
            )?)))
        }
    };
    if bind && side == ConfigRuntimeSide::Server {
        handle
            .lock()
            .map_err(|_| "package services unavailable")?
            .bind_world(world);
    }
    Ok(handle.clone())
}

pub(crate) fn snapshots(world: Option<&Path>) -> Result<Vec<Snapshot>, String> {
    let mut snapshots = Vec::new();
    let mut total = 0usize;
    for entry in crate::yak_mods::entries() {
        let Some(wasm) = &entry.manifest.wasm else {
            continue;
        };
        if wasm.server.is_none()
            || !wasm
                .permissions
                .iter()
                .any(|permission| permission == yoake_wasm::package_services::CONFIG_CAPABILITY)
            || !entry
                .manifest
                .config
                .values()
                .any(|field| field.side == horizon_mods::yak::ConfigSide::Synced)
        {
            continue;
        }
        let handle = services(&entry.manifest.id, ConfigRuntimeSide::Server, world)?;
        let (revision, data) = handle
            .lock()
            .map_err(|_| "package services unavailable")?
            .snapshot()?;
        total = total
            .checked_add(data.len())
            .ok_or("config sync aggregate limit exceeded")?;
        if total > yoake_wasm::config_sync::MAX_TOTAL_BYTES
            || snapshots.len() >= yoake_wasm::config_sync::MAX_MODS
        {
            return Err("config sync aggregate limit exceeded".into());
        }
        snapshots.push(Snapshot {
            id: entry.manifest.id.clone(),
            revision,
            data: Arc::new(data),
        });
    }
    Ok(snapshots)
}

pub(crate) fn apply_snapshot(id: &str, data: &[u8]) -> Result<(), String> {
    let handle = services(id, ConfigRuntimeSide::Client, None)?;
    let mut service = handle.lock().map_err(|_| "package services unavailable")?;
    service.require(yoake_wasm::package_services::CONFIG_CAPABILITY)?;
    service.apply_snapshot(data)
}

pub(crate) fn clear_snapshots() {
    let Some(cache) = SERVICES.get() else {
        return;
    };
    let Ok(cache) = cache.lock() else {
        return;
    };
    for ((_, server), handle) in cache.iter() {
        if !server && let Ok(mut service) = handle.lock() {
            service.clear_snapshot();
        }
    }
}
