use std::{fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let catalog = horizon_mod_api_gen::generate_inject_catalog(&root)?;
    fs::write(root.join("crates/yoake-wasm/wit/inject-sites.json"), catalog)?;
    Ok(())
}
