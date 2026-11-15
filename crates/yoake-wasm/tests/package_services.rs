use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use horizon_mods::yak::{ConfigField, ConfigRuntimeSide, PermissionNode};
use yoake_wasm::package_services::{PackageServices, ServiceHandle};
use yoake_wasm::server_types::{Player, StorageScope, Vec3};
use yoake_wasm::{ClientFullMod, ClientHost, Sandbox, ServerFullMod, ServerHost};

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "yoake-services-guest-{}-{time}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed),
            ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn services(&self, side: ConfigRuntimeSide, allowed: bool) -> ServiceHandle {
        let fields: BTreeMap<String, ConfigField> = toml::from_str(
            "[local]\ntype='bool'\ndefault=false\nside='client'\n[shared]\ntype='bool'\ndefault=false\nside='synced'\n",
        ).unwrap();
        let permissions: BTreeMap<String, PermissionNode> =
            toml::from_str("['example.edit']\ndefault_op_level=2\n").unwrap();
        let capabilities = if allowed {
            vec![
                "service.config".into(),
                "service.permissions".into(),
                "service.storage".into(),
            ]
        } else {
            Vec::new()
        };
        Arc::new(Mutex::new(
            PackageServices::open(
                "example",
                &capabilities,
                &fields,
                &permissions,
                side,
                &self.0,
                Some(&self.0),
            )
            .unwrap(),
        ))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Server(ServiceHandle);

impl ServerHost for Server {
    fn services(&mut self) -> Result<ServiceHandle, String> {
        Ok(self.0.clone())
    }
    fn server_players(&mut self) -> Result<Vec<Player>, String> {
        Ok(vec![Player {
            dimension: "minecraft:overworld".into(),
            game_mode: 1,
            health: 20.0,
            name: "Guest".into(),
            permission_level: 2,
            pitch: 0.0,
            position: Vec3 {
                x: 0.0,
                y: 64.0,
                z: 0.0,
            },
            sneaking: false,
            uuid: "0123456789abcdef0123456789abcdef".into(),
            yaw: 0.0,
        }])
    }
}

struct Client(ServiceHandle, Arc<Mutex<Vec<String>>>);

impl ClientHost for Client {
    fn services(&mut self) -> Result<ServiceHandle, String> {
        Ok(self.0.clone())
    }
    fn log(
        &mut self,
        target: &str,
        message: &str,
    ) -> Result<(), yoake_wasm::bindings::yoake::mods::types::HostError> {
        self.1.lock().unwrap().push(format!("{target}:{message}"));
        Ok(())
    }
}

fn call(guest: &mut ServerFullMod, method: &str) -> Vec<u8> {
    guest
        .handle_call("test", method, &[])
        .expect("guest must remain enabled")
        .expect("guest service call must succeed")
}

#[test]
fn server_guest_round_trips_config_storage_permissions_and_change_hooks() {
    let scratch = Scratch::new();
    let services = scratch.services(ConfigRuntimeSide::Server, true);
    let bytes = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::FULL_SERVICES_SERVER)
        .expect("services guest is required");
    let mut guest = ServerFullMod::instantiate(
        &mut Sandbox::new(),
        "example",
        &bytes,
        Box::new(Server(services.clone())),
        &[],
    )
    .unwrap();
    assert_eq!(call(&mut guest, "exercise"), b"ok");
    assert_eq!(call(&mut guest, "changes"), 1_u32.to_le_bytes());
    assert_eq!(call(&mut guest, "permission"), [1]);
    std::fs::write(
        scratch.0.join("permissions.toml"),
        "[mods.example.players.'0123456789abcdef0123456789abcdef']\n'example.edit'=false\n",
    )
    .unwrap();
    assert_eq!(call(&mut guest, "permission"), [0]);
    drop(guest);
    drop(services);
    let restored = scratch.services(ConfigRuntimeSide::Server, true);
    let mut restored = restored.lock().unwrap();
    assert!(restored.config_get("shared").unwrap().bool_value);
    assert_eq!(
        restored.storage_get("value", StorageScope::World).unwrap(),
        Some(b"guest bytes".to_vec())
    );
    assert_eq!(
        restored.storage_get("blob", StorageScope::World).unwrap(),
        None
    );
}

#[test]
fn server_guest_without_capabilities_is_denied_without_creating_files() {
    let scratch = Scratch::new();
    let services = scratch.services(ConfigRuntimeSide::Server, false);
    let bytes = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::FULL_SERVICES_SERVER)
        .expect("services guest is required");
    let mut guest = ServerFullMod::instantiate(
        &mut Sandbox::new(),
        "example",
        &bytes,
        Box::new(Server(services)),
        &[],
    )
    .unwrap();
    assert_eq!(call(&mut guest, "denied"), b"denied");
    assert!(std::fs::read_dir(&scratch.0).unwrap().next().is_none());
}

#[test]
fn client_guest_round_trips_client_data_and_observes_synced_changes() {
    let scratch = Scratch::new();
    let services = scratch.services(ConfigRuntimeSide::Client, true);
    let log = Arc::new(Mutex::new(Vec::new()));
    let bytes = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::FULL_SERVICES_CLIENT)
        .expect("services guest is required");
    let mut guest = ClientFullMod::instantiate(
        &mut Sandbox::new(),
        "example",
        &bytes,
        Box::new(Client(services.clone(), log.clone())),
        &[],
    )
    .unwrap();
    {
        let mut state = services.lock().unwrap();
        assert!(state.config_get("local").unwrap().bool_value);
        assert_eq!(
            state.storage_get("value", StorageScope::Client).unwrap(),
            Some(b"guest bytes".to_vec())
        );
        state.apply_snapshot(b"shared = true\n").unwrap();
    }
    guest.notify_config_changes();
    services.lock().unwrap().clear_snapshot();
    guest.notify_config_changes();
    assert!(
        !services
            .lock()
            .unwrap()
            .config_get("shared")
            .unwrap()
            .bool_value
    );
    let lines = log.lock().unwrap();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("client exercise: Ok(())")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("client changed local: true")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("client changed shared: true")),
        "{lines:?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("client changed shared: false")),
        "{lines:?}"
    );
}

#[test]
fn client_guest_without_capabilities_is_denied_without_creating_files() {
    let scratch = Scratch::new();
    let services = scratch.services(ConfigRuntimeSide::Client, false);
    let log = Arc::new(Mutex::new(Vec::new()));
    let bytes = yoake_wasm::test_guests::load(&yoake_wasm::test_guests::FULL_SERVICES_CLIENT)
        .expect("services guest is required");
    let _guest = ClientFullMod::instantiate(
        &mut Sandbox::new(),
        "example",
        &bytes,
        Box::new(Client(services, log.clone())),
        &[],
    )
    .unwrap();
    let lines = log.lock().unwrap();
    assert!(
        lines
            .iter()
            .any(|line| line.contains("client denied: Ok(())")),
        "{lines:?}"
    );
    assert!(std::fs::read_dir(&scratch.0).unwrap().next().is_none());
}
