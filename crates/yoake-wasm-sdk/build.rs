use std::{env, fs, path::PathBuf};
use wit_bindgen_core::{Files, WorldGenerator, wit_parser::Resolve};

fn main() {
    let wit = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("../yoake-wasm/wit");
    println!("cargo:rerun-if-changed={}", wit.display());
    println!("cargo:rerun-if-changed=build.rs");

    let mut resolve = Resolve::default();
    let (pkg, _) = resolve.push_dir(&wit).expect("parse wit");
    let world = resolve.select_world(&[pkg], Some("client-mod")).expect("world");

    let opts = wit_bindgen_rust::Opts {
        generate_all: true,
        pub_export_macro: true,
        default_bindings_module: Some("yoake_wasm_sdk::bindings".to_string()),
        ..Default::default()
    };
    let mut files = Files::default();
    opts.build().generate(&mut resolve, world, &mut files).expect("generate");
    let (_, src) = files.iter().next().expect("one file");
    let src = String::from_utf8(src.to_vec()).unwrap();
    let src = src.replace("pub mod mod {", "pub mod r#mod {").replace("::mod::", "::r#mod::");
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("client_mod.rs");
    fs::write(out, src).unwrap();
}
