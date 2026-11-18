//! Builds Rust guests to wasm32 core modules with the installed `wasm32-wasip2` target (nothing is
//! installed): the SDK as an rlib, then the guest as a cdylib linked with `rust-lld` in `wasm-ld` flavor,
//! which skips the component wrapper. Shared by this crate's tests and the engine's end-to-end test
//! (`#[path]`-included), so it takes its directories as arguments.

use std::path::{Path, PathBuf};
use std::process::Command;

fn rustc(args: &[&str], extra: &[String]) -> std::process::Output {
    Command::new("rustc")
        .args(["--edition", "2024", "--target", "wasm32-wasip2", "-C", "panic=abort", "-C", "opt-level=1"])
        .args(args)
        .args(extra)
        .output()
        .expect("run rustc")
}

/// The SDK (`sdk_dir/src/lib.rs`) as a wasm32 rlib in `scratch/sdk`; returns the rlib path.
pub fn build_sdk(sdk_dir: &Path, scratch: &Path) -> PathBuf {
    let out = scratch.join("sdk");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let lib = sdk_dir.join("src/lib.rs");
    let result = rustc(&["--crate-type", "rlib", "--crate-name", "yoake_guest_sdk", "--out-dir", out.to_str().unwrap(), lib.to_str().unwrap()], &[]);
    assert!(result.status.success(), "the SDK did not compile for wasm32:\n{}", String::from_utf8_lossy(&result.stderr));
    out.join("libyoake_guest_sdk.rlib")
}

/// Compiles one guest source to a core module and returns its bytes.
pub fn build_guest(rlib: &Path, source: &Path, scratch: &Path, name: &str) -> Vec<u8> {
    std::fs::create_dir_all(scratch).unwrap();
    let wasm = scratch.join(format!("{name}.wasm"));
    let result = rustc(
        &[
            "--crate-type",
            "cdylib",
            "--crate-name",
            name,
            "-C",
            "linker=rust-lld",
            "-C",
            "linker-flavor=wasm-ld",
            "-C",
            "strip=symbols",
            "-C",
            "link-arg=--no-entry",
            "-C",
            "link-arg=--export-memory",
            "-C",
            "link-arg=--export=yoake_hook",
            "-C",
            "link-arg=--export=yoake_init",
        ],
        &[
            "-L".into(),
            format!("dependency={}", rlib.parent().unwrap().display()),
            "--extern".into(),
            format!("yoake_guest_sdk={}", rlib.display()),
            "-o".into(),
            wasm.display().to_string(),
            source.display().to_string(),
        ],
    );
    assert!(result.status.success(), "guest {name} did not compile:\n{}", String::from_utf8_lossy(&result.stderr));
    std::fs::read(&wasm).unwrap()
}
