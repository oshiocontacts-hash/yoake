fn main() {
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    horizon_mod_api_gen::generate_test_hooks(&root, &out_dir)
        .expect("generating dispatch test fixture");
    let crash_path = root.join("crates/horizon-client-render/src/crash.rs");
    println!("cargo:rerun-if-changed={}", crash_path.display());
    let crash = std::fs::read_to_string(crash_path)
        .expect("reading the production mod guard")
        .replace("\r\n", "\n");
    let start = crash.find("pub mod mod_guard {").expect("mod guard module");
    let end = crash[start..]
        .find("\n#[cfg(test)]\nmod tests {")
        .map(|offset| start + offset)
        .expect("outer crash tests after the mod guard");
    std::fs::write(out_dir.join("mod_guard_runtime.rs"), &crash[start..end])
        .expect("copying the production mod guard into the fixture");
}
