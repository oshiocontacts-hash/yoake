use std::{env, fmt::Write, fs, path::Path};

fn collect(directory: &Path, prefix: &str, files: &mut Vec<(String, String)>) -> std::io::Result<()> {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().expect("SDK bundle paths must be UTF-8");
        let path = entry.path();
        let bundled = format!("{prefix}/{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if name != "target" && name != "examples" {
                collect(&path, &bundled, files)?;
            }
        } else {
            assert!(kind.is_file(), "SDK bundle cannot contain links: {}", path.display());
            println!("cargo:rerun-if-changed={}", path.display());
            files.push((bundled, path.to_str().expect("SDK bundle paths must be UTF-8").to_owned()));
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let manifest = env::var_os("CARGO_MANIFEST_DIR").expect("Cargo provides the manifest directory");
    let crates = Path::new(&manifest).parent().expect("CLI lives in crates");
    let mut files = Vec::new();
    collect(&crates.join("yoake-wasm-sdk"), "yoake-wasm-sdk", &mut files)?;
    collect(&crates.join("yoake-wasm/wit"), "yoake-wasm/wit", &mut files)?;
    files.sort();
    let mut generated = String::from("&[\n");
    for (bundled, source) in files {
        writeln!(generated, "    ({bundled:?}, include_bytes!({source:?})),").unwrap();
    }
    generated.push_str("]\n");
    let output = env::var_os("OUT_DIR").expect("Cargo provides the build output directory");
    fs::write(Path::new(&output).join("bundled_sdk.rs"), generated)
}
