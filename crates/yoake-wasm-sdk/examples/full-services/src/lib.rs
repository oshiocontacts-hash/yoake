#[cfg(all(feature = "client", feature = "server"))]
compile_error!("build each world separately");

#[cfg(feature = "client")]
use yoake_wasm_sdk::full::client as api;
#[cfg(feature = "server")]
use yoake_wasm_sdk::full::server as api;

#[cfg(any(feature = "client", feature = "server"))]
fn exercise() -> Result<(), String> {
    use api::{services, types};
    let key = if cfg!(feature = "server") {
        "shared"
    } else {
        "local"
    };
    let scope = if cfg!(feature = "server") {
        types::StorageScope::World
    } else {
        types::StorageScope::Client
    };
    let mut value = services::config_get(key)?;
    value.bool_value = true;
    services::config_set(key, &value)?;
    if !services::config_get(key)?.bool_value {
        return Err("config round-trip failed".into());
    }
    services::storage_put("value", scope, b"guest bytes")?;
    if services::storage_get("value", scope)?.as_deref() != Some(b"guest bytes".as_slice()) {
        return Err("storage round-trip failed".into());
    }
    let ticket = services::storage_blob_begin("blob", scope, 6)?;
    services::storage_blob_write(b"abc", ticket, 0)?;
    if services::storage_blob_commit(ticket).is_ok() {
        return Err("incomplete blob accepted".into());
    }
    services::storage_blob_write(b"def", ticket, 3)?;
    services::storage_blob_commit(ticket)?;
    if services::storage_blob_read("blob", 3, 2, scope)? != b"cde" {
        return Err("blob read failed".into());
    }
    let ticket = services::storage_blob_begin("abort", scope, 1)?;
    if !services::storage_blob_abort(ticket)? {
        return Err("blob abort failed".into());
    }
    if !services::storage_delete("blob", scope)? {
        return Err("storage delete failed".into());
    }
    Ok(())
}

#[cfg(any(feature = "client", feature = "server"))]
fn denied() -> Result<(), String> {
    use api::{services, types};
    let scope = if cfg!(feature = "server") {
        types::StorageScope::World
    } else {
        types::StorageScope::Client
    };
    let value = types::ConfigValue {
        kind: types::ConfigValueKind::Bool,
        bool_value: true,
        int_value: 0,
        float_value: 0.0,
        string_value: String::new(),
        bool_list: Vec::new(),
        int_list: Vec::new(),
        float_list: Vec::new(),
        string_list: Vec::new(),
    };
    let errors = [
        services::config_get("shared").err(),
        services::config_set("shared", &value).err(),
        services::storage_get("value", scope).err(),
        services::storage_put("value", scope, b"data").err(),
        services::storage_delete("value", scope).err(),
        services::storage_blob_begin("blob", scope, 4).err(),
        services::storage_blob_write(b"data", 1, 0).err(),
        services::storage_blob_commit(1).err(),
        services::storage_blob_abort(1).err(),
        services::storage_blob_read("blob", 4, 0, scope).err(),
    ];
    if errors.iter().any(|error| {
        !error
            .as_ref()
            .is_some_and(|error| error.starts_with("permission denied:"))
    }) {
        return Err(format!(
            "missing capability did not deny every import: {errors:?}"
        ));
    }
    Ok(())
}

#[cfg(feature = "server")]
mod server {
    use super::*;
    use api::{ServerMod, services, types};
    use yoake_wasm_sdk::full::hook;

    #[derive(Default)]
    struct Guest {
        changes: u32,
    }

    impl ServerMod for Guest {
        fn subscriptions() -> &'static [&'static str] {
            &[hook::CONFIG_CHANGED]
        }
        fn config_changed(&mut self, _key: String, _value: types::ConfigValue) {
            self.changes += 1;
        }
        fn handle_call(
            &mut self,
            _caller: String,
            method: String,
            _payload: Vec<u8>,
        ) -> Result<Vec<u8>, String> {
            match method.as_str() {
                "exercise" => {
                    exercise()?;
                    Ok(b"ok".to_vec())
                }
                "denied" => {
                    denied()?;
                    let error = services::has_permission(
                        "example.edit",
                        "0123456789abcdef0123456789abcdef",
                    )
                    .unwrap_err();
                    if !error.starts_with("permission denied:") {
                        return Err(error);
                    }
                    Ok(b"denied".to_vec())
                }
                "permission" => Ok(vec![u8::from(services::has_permission(
                    "example.edit",
                    "0123456789abcdef0123456789abcdef",
                )?)]),
                "changes" => Ok(self.changes.to_le_bytes().to_vec()),
                _ => Err("unknown method".into()),
            }
        }
    }

    yoake_wasm_sdk::export_server_full!(Guest);
}

#[cfg(feature = "client")]
mod client {
    use super::*;
    use api::{ClientMod, services, types};
    use yoake_wasm_sdk::full::hook;

    #[derive(Default)]
    struct Guest;

    impl ClientMod for Guest {
        fn subscriptions() -> &'static [&'static str] {
            &[hook::CONFIG_CHANGED]
        }
        fn init(&mut self) {
            if services::config_get("local").is_err() {
                println!("client denied: {:?}", denied());
            } else {
                println!("client exercise: {:?}", exercise());
            }
        }
        fn config_changed(&mut self, key: String, value: types::ConfigValue) {
            println!("client changed {key}: {}", value.bool_value);
        }
    }

    yoake_wasm_sdk::export_client_full!(Guest);
}
