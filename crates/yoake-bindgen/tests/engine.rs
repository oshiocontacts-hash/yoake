//! The CI gate for the checked-in glue (MX3G1): regenerate it from the real engine and fail on any
//! difference, then compile the engine with the feature on and fail on any compile error.
//!
//! Slow (rustdoc JSON of six crates, then a `cargo check` of the engine with wasmtime), so it is
//! `#[ignore]`; CI runs it with
//!
//!   cargo test --offline -j 4 -p yoake-bindgen --test engine -- --ignored --nocapture
//!
//! To accept a change instead of failing on it, run crates/yoake-bindgen/regen.sh
//! (it is crates/yoake-bindgen/regen.sh) and commit the result.

use std::path::{Path, PathBuf};
use std::process::Command;

use yoake_bindgen::{Config, generate, load_universe};

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn cargo(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO"))
        .current_dir(root)
        .env("RUSTC_BOOTSTRAP", "1")
        // the cfg of the JSON is pinned: default features, no extra `--cfg`, and never the glue's own feature
        .env("RUSTDOCFLAGS", "")
        .env("CARGO_INCREMENTAL", "0")
        .args(args)
        .output()
        .expect("run cargo")
}

#[test]
#[ignore = "slow: rustdoc JSON of six engine crates plus a cargo check of the engine with the feature on"]
fn checked_in_glue_is_current_and_compiles_in_the_engine() {
    let root = workspace();

    // 0. the toolchain the rustdoc JSON comes from is pinned (rust-toolchain.toml): another rustc may print another JSON
    if let Err(e) = yoake_bindgen::toolchain::check_rustc(&root) {
        panic!("the gate needs the pinned toolchain: {e}");
    }

    // 1. rustdoc JSON with the feature off (the glue must not see itself)
    for krate in ["horizon-client-lib", "horizon-client", "horizon-items", "horizon-core", "horizon-world", "horizon-client-core"] {
        let out = cargo(
            &root,
            &["rustdoc", "--offline", "-j", "4", "-p", krate, "--lib", "--", "-Z", "unstable-options", "--output-format", "json", "--document-private-items"],
        );
        assert!(out.status.success(), "rustdoc json of {krate} failed:\n{}", String::from_utf8_lossy(&out.stderr));
    }

    // 2. regenerate in memory and compare with what is checked in
    let config: Config = serde_json::from_str(&std::fs::read_to_string(root.join("crates/yoake-bindgen/bindgen.json")).unwrap()).unwrap();
    let uni = load_universe(&config, &root).unwrap();
    let generated = generate(&config, &root, &uni).unwrap();
    let mut stale = Vec::new();
    for (path, text) in &generated.files {
        let on_disk = std::fs::read_to_string(root.join(path)).ok();
        if on_disk.as_deref() != Some(text.as_str()) {
            stale.push(path.display().to_string());
        }
    }
    assert!(
        stale.is_empty(),
        "the checked-in glue is out of date; run crates/yoake-bindgen/regen.sh and commit:\n  {}",
        stale.join("\n  ")
    );

    // 2b. append-only ids against the base branch's lock (CI sets the variable from `git show`)
    if let Ok(base) = std::env::var("YOAKE_BINDGEN_BASE_LOCK") {
        let base = yoake_bindgen::lock::Lock::load(Path::new(&base)).unwrap();
        let now: yoake_bindgen::lock::Lock = serde_json::from_str(&generated.files[&PathBuf::from(&config.lock)]).unwrap();
        let violations = now.append_only_violations(&base);
        assert!(violations.is_empty(), "ids.lock.json is append-only:
  {}", violations.join("
  "));
    }
    // 2b'. the unsafe policy: forbid/deny in the engine crates, the allowed modules are frozen
    let violations = yoake_bindgen::policy::check_unsafe_policy(&root, &config.frozen_files);
    assert!(violations.is_empty(), "unsafe policy: {violations:#?}");
    // 2c. glue files of modules that are no longer targeted
    let stale = yoake_bindgen::stale_glue_files(&config, &root);
    assert!(stale.is_empty(), "stale glue files (no configured target): {stale:?}");

    // 3. the generated glue compiles inside the real engine
    let out = cargo(&root, &["check", "--offline", "-j", "4", "-p", "horizon-client", "--features", "yoake2-slice"]);
    assert!(out.status.success(), "the engine does not compile with the generated glue:\n{}", String::from_utf8_lossy(&out.stderr));
}
