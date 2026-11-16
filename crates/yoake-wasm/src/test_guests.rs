//! Test support (feature `test-guests`): finds, and when needed builds, the example guest modules
//! the integration tests run (`crates/yoake-wasm-sdk/examples`).
//!
//! The guests honor `CARGO_TARGET_DIR`, then `YOAKE_WASM_TARGET_DIR`; otherwise a `target-wasm` folder next to
//! the repository folder when one exists, else into `target/wasm-guests` of the workspace. A build
//! that cannot run (no `wasm32-wasip2` target, offline) falls back to an artifact already there. A
//! guest that is not available fails the test when `YOAKE_CI` or `CI` is set (and when
//! `YOAKE_REQUIRE_GUESTS=1`), and is reported as skipped otherwise.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

/// An example guest: its folder, the cargo features it is built with and the file it produces.
pub struct Guest {
    pub name: &'static str,
    example: &'static str,
    features: &'static str,
    artifact: &'static str,
}

pub const HELLO: Guest = Guest {
    name: "hello",
    example: "hello",
    features: "",
    artifact: "yoake_wasm_hello.wasm",
};
pub const HELLO_STD: Guest = Guest {
    name: "hello-std",
    example: "hello-std",
    features: "",
    artifact: "yoake_wasm_hello_std.wasm",
};
pub const TEST_DEMO: Guest = Guest {
    name: "test-demo",
    example: "test-demo",
    features: "",
    artifact: "yoake_wasm_test_demo.wasm",
};
pub const FULL_DEMO_CLIENT: Guest = Guest {
    name: "full-demo-client",
    example: "full-demo",
    features: "client",
    artifact: "yoake_wasm_full_demo.wasm",
};
pub const FULL_DEMO_SERVER: Guest = Guest {
    name: "full-demo-server",
    example: "full-demo",
    features: "server",
    artifact: "yoake_wasm_full_demo.wasm",
};

pub const FULL_SERVICES_CLIENT: Guest = Guest {
    name: "full-services-client",
    example: "full-services",
    features: "client",
    artifact: "yoake_wasm_full_services.wasm",
};
pub const FULL_SERVICES_SERVER: Guest = Guest {
    name: "full-services-server",
    example: "full-services",
    features: "server",
    artifact: "yoake_wasm_full_services.wasm",
};
pub const CLIENT_SERVICES_DEMO: Guest = Guest {
    name: "client-services-demo",
    example: "client-services-demo",
    features: "",
    artifact: "yoake_wasm_client_services_demo.wasm",
};

pub const GPU_DEMO: Guest = Guest {
    name: "gpu-demo",
    example: "gpu-demo",
    features: "",
    artifact: "yoake_wasm_gpu_demo.wasm",
};
pub const GPU_VERIFY: Guest = Guest {
    name: "gpu-verify",
    example: "gpu-demo",
    features: "verify",
    artifact: "yoake_wasm_gpu_demo.wasm",
};
pub const GPU_RELOAD: Guest = Guest {
    name: "gpu-reload",
    example: "gpu-demo",
    features: "reload",
    artifact: "yoake_wasm_gpu_demo.wasm",
};
pub const GPU_TRAP: Guest = Guest {
    name: "gpu-trap",
    example: "gpu-demo",
    features: "trap",
    artifact: "yoake_wasm_gpu_demo.wasm",
};

/// The second package of the `mod-call` tests: only answers `handle-call`.
pub const FULL_DEMO_PEER: Guest = Guest {
    name: "full-demo-peer",
    example: "full-demo",
    features: "peer",
    artifact: "yoake_wasm_full_demo.wasm",
};

pub const INVALID_HOOK: Guest = Guest {
    name: "invalid-hook", example: "validation-fixture", features: "typo", artifact: "yoake_wasm_validation_fixture.wasm",
};
pub const TRAPPING_INIT: Guest = Guest {
    name: "trapping-init", example: "validation-fixture", features: "trap", artifact: "yoake_wasm_validation_fixture.wasm",
};
pub const UNWIRED_CLIENT_HOOK: Guest = Guest {
    name: "unwired-client-hook", example: "validation-fixture", features: "unwired-client", artifact: "yoake_wasm_validation_fixture.wasm",
};
pub const WRONG_CLIENT_HOOK: Guest = Guest {
    name: "wrong-client-hook", example: "validation-fixture", features: "wrong-client", artifact: "yoake_wasm_validation_fixture.wasm",
};
pub const WRONG_SERVER_HOOK: Guest = Guest {
    name: "wrong-server-hook", example: "validation-fixture", features: "wrong-server", artifact: "yoake_wasm_validation_fixture.wasm",
};

static BUILD: Mutex<()> = Mutex::new(());

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn target_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("CARGO_TARGET_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(dir) = std::env::var_os("YOAKE_WASM_TARGET_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    let sibling = workspace().join("../target-wasm");
    if sibling.is_dir() {
        return sibling;
    }
    workspace().join("target/wasm-guests")
}

fn strict() -> bool {
    ["YOAKE_CI", "CI", "YOAKE_REQUIRE_GUESTS"]
        .iter()
        .any(|name| std::env::var_os(name).is_some_and(|value| !value.is_empty() && value != "0"))
}

/// The component bytes of `guest`, built when out of date. `None` only outside CI when the guest
/// can neither be built nor found (the caller should return, which the test log shows as skipped).
pub fn load(guest: &Guest) -> Option<Vec<u8>> {
    let _serial = BUILD.lock().unwrap_or_else(|error| error.into_inner());
    let target = target_dir();
    let built = target.join("wasm32-wasip2/release").join(guest.artifact);
    let copy = target.join("wasm32-wasip2/release").join(format!("{}.wasm", guest.name.replace('-', "_")));
    let example = if guest.example == "validation-fixture" {
        workspace().join("crates/yoake-wasm/tests/validation-fixture")
    } else {
        workspace().join("crates/yoake-wasm-sdk/examples").join(guest.example)
    };
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command
        .args([
            "build",
            "--offline",
            "--release",
            "--target",
            "wasm32-wasip2",
            "-j",
            "4",
        ])
        .current_dir(&example)
        .env("CARGO_TARGET_DIR", &target)
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS");
    if !guest.features.is_empty() {
        command.args(["--features", guest.features]);
    }
    let build = command.output();
    let bytes = match &build {
        Ok(output) if output.status.success() => std::fs::copy(&built, &copy)
            .ok()
            .and_then(|_| std::fs::read(&copy).ok()),
        _ => None,
    };
    let bytes = bytes.or_else(|| (!strict()).then(|| std::fs::read(&copy).ok()).flatten());
    if bytes.is_none() {
        let why = match &build {
            Ok(output) => String::from_utf8_lossy(&output.stderr).into_owned(),
            Err(error) => error.to_string(),
        };
        let message = format!(
            "guest {} is not available (building it in {} failed: {why})",
            guest.name,
            example.display()
        );
        assert!(!strict(), "{message}");
        eprintln!("skipped: {message}");
    }
    bytes
}
