fn main() {
    // The hook-ID key table this crate's `#[yoake_hook]` attributes are checked against (hook-ledger/).
    yoake_hook_ledger::build::emit_crate_table("horizon_client_render");
    let root =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../..");
    let out_dir = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    horizon_mod_api_gen::generate(&root, &out_dir, horizon_mod_api_gen::Target::Render)
        .expect("generating native render mod API");
}
