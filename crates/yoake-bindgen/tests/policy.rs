//! The unsafe policy of the engine crates (MX3R1): the compiler is told `forbid`/`deny`, the only
//! `#[allow(unsafe_code)]` modules are files the generator freezes, and a violation is caught.

use std::path::{Path, PathBuf};

use yoake_bindgen::Config;
use yoake_bindgen::policy::check_unsafe_policy;

fn workspace() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn the_engine_tree_follows_the_unsafe_policy() {
    let root = workspace();
    let config: Config = serde_json::from_str(&std::fs::read_to_string(root.join("crates/yoake-bindgen/bindgen.json")).unwrap()).unwrap();
    let violations = check_unsafe_policy(&root, &config.frozen_files);
    assert!(violations.is_empty(), "{violations:#?}");
    // the two files that hold unsafe are frozen, so no glue type from them is ever writable
    assert!(config.frozen_files.iter().any(|f| f.ends_with("dll_mods.rs")));
    assert!(config.frozen_files.iter().any(|f| f.ends_with("wasm_mods.rs")));
}

fn fake_tree(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    for krate in yoake_bindgen::policy::FORBID_CRATES {
        let src = dir.join("crates").join(krate).join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
    }
    let src = dir.join("crates/horizon-client/src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("lib.rs"), "#![deny(unsafe_code)]\n").unwrap();
    std::fs::write(src.join("client_lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
    std::fs::write(src.join("entry.rs"), "#[allow(unsafe_code)]\n#[path = \"native.rs\"]\nmod native;\n").unwrap();
    std::fs::write(src.join("native.rs"), "pub fn f() { unsafe { std::hint::unreachable_unchecked() } }\n").unwrap();
    dir
}

#[test]
fn an_allowed_module_must_be_frozen() {
    let dir = fake_tree("policy_a");
    assert!(check_unsafe_policy(&dir, &["horizon-client/src/native.rs".to_string()]).is_empty());
    let violations = check_unsafe_policy(&dir, &[]);
    assert!(violations.iter().any(|v| v.contains("native.rs") && v.contains("frozen_files")), "{violations:?}");
}

#[test]
fn a_missing_forbid_or_a_stray_unsafe_is_caught() {
    let dir = fake_tree("policy_b");
    let frozen = ["horizon-client/src/native.rs".to_string()];
    std::fs::write(dir.join("crates/horizon-items/src/lib.rs"), "// no attribute\n").unwrap();
    let violations = check_unsafe_policy(&dir, &frozen);
    assert!(violations.iter().any(|v| v.contains("horizon-items") && v.contains("forbid")), "{violations:?}");
    std::fs::write(dir.join("crates/horizon-items/src/lib.rs"), "#![forbid(unsafe_code)]\n").unwrap();
    std::fs::write(dir.join("crates/horizon-client/src/stray.rs"), "pub fn g() { unsafe { } }\n").unwrap();
    let violations = check_unsafe_policy(&dir, &frozen);
    assert!(violations.iter().any(|v| v.contains("stray.rs")), "{violations:?}");
    // comments and strings that only say "unsafe" are not unsafe code
    std::fs::write(dir.join("crates/horizon-client/src/stray.rs"), "// unsafe { }\nconst S: &str = \"unsafe id\";\n").unwrap();
    assert!(check_unsafe_policy(&dir, &frozen).is_empty());
}
