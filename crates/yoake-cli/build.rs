use std::{env, fmt::Write, fs, path::Path};

fn collect(
    directory: &Path,
    prefix: &str,
    files: &mut Vec<(String, String)>,
) -> std::io::Result<()> {
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().expect("SDK bundle paths must be UTF-8");
        let path = entry.path();
        let bundled = format!("{prefix}/{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if name != "target" && name != "examples" && name != "tests" {
                collect(&path, &bundled, files)?;
            }
        } else {
            assert!(
                kind.is_file(),
                "SDK bundle cannot contain links: {}",
                path.display()
            );
            println!("cargo:rerun-if-changed={}", path.display());
            files.push((
                bundled,
                path.to_str()
                    .expect("SDK bundle paths must be UTF-8")
                    .to_owned(),
            ));
        }
    }
    Ok(())
}

fn main() -> std::io::Result<()> {
    let manifest =
        env::var_os("CARGO_MANIFEST_DIR").expect("Cargo provides the manifest directory");
    let crates = Path::new(&manifest).parent().expect("CLI lives in crates");
    let mut files = Vec::new();
    collect(&crates.join("yoake-wasm-sdk"), "yoake-wasm-sdk", &mut files)?;
    collect(&crates.join("yoake-wasm/wit"), "yoake-wasm/wit", &mut files)?;
    collect(
        &crates.join("yoake-wasm/wit-worker"),
        "yoake-wasm/wit-worker",
        &mut files,
    )?;
    collect(
        &crates.join("yoake-guest-sdk/src"),
        "yoake-guest-sdk/src",
        &mut files,
    )?;
    let guest_cargo = crates.join("yoake-guest-sdk/Cargo.toml");
    println!("cargo:rerun-if-changed={}", guest_cargo.display());
    let output = env::var_os("OUT_DIR").expect("Cargo provides the build output directory");
    let output_dir = Path::new(&output);
    let standalone_guest_cargo = output_dir.join("guest_sdk_Cargo.toml");
    fs::write(
        &standalone_guest_cargo,
        standalone_cargo_toml(&fs::read_to_string(&guest_cargo)?),
    )?;
    files.push((
        "yoake-guest-sdk/Cargo.toml".to_owned(),
        standalone_guest_cargo
            .to_str()
            .expect("SDK bundle paths must be UTF-8")
            .to_owned(),
    ));
    files.sort();
    let mut generated = String::from("&[\n");
    for (bundled, source) in files {
        writeln!(generated, "    ({bundled:?}, include_bytes!({source:?})),").unwrap();
    }
    generated.push_str("]\n");
    fs::write(output_dir.join("bundled_sdk.rs"), generated)
}

fn standalone_cargo_toml(source: &str) -> String {
    let mut out = String::new();
    let mut skip_section = false;
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
            skip_section = trimmed == "[dev-dependencies]" || trimmed == "[lints]";
            if skip_section {
                continue;
            }
        }
        if skip_section {
            continue;
        }
        match trimmed {
            "edition.workspace = true" => out.push_str("edition = \"2024\"\n"),
            "license.workspace = true" => out.push_str("license = \"MIT\"\n"),
            "rust-version.workspace = true" => out.push_str("rust-version = \"1.85\"\n"),
            _ => {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    out.push_str("\n[workspace]\n");
    out
}
