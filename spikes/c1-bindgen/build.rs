#[path = "src/classify.rs"]
mod classify;
#[path = "src/model.rs"]
mod model;
#[path = "src/render.rs"]
mod render;

use std::error::Error;
use std::fs;

use model::Category;

fn main() -> Result<(), Box<dyn Error>> {
    const FIXTURE: &str = "tests/fixtures/sample-items.json";
    println!("cargo:rerun-if-changed={FIXTURE}");
    println!("cargo:rerun-if-changed=src/model.rs");
    println!("cargo:rerun-if-changed=src/classify.rs");
    println!("cargo:rerun-if-changed=src/render.rs");
    println!("cargo:rerun-if-changed=src/render/host.rs");

    let input = fs::read_to_string(FIXTURE)?;
    let document: serde_json::Value = serde_json::from_str(&input)?;
    let suffixes = vec![
        "native_entity_query.rs".to_string(),
        "native_client_items.rs".to_string(),
    ];
    let mut functions = model::extract_functions(&document, &suffixes)?;
    for function in &mut functions {
        classify::classify(function, &document);
    }
    functions.sort_by(|left, right| {
        left.full_path
            .cmp(&right.full_path)
            .then(left.rustdoc_id.cmp(&right.rustdoc_id))
    });
    let mut next_id = 1u32;
    for function in functions
        .iter_mut()
        .filter(|function| function.category == Category::A)
    {
        function.binding_id = Some(next_id);
        next_id = next_id.checked_add(1).ok_or("binding id overflow")?;
    }
    let bound: Vec<&model::Function> = functions
        .iter()
        .filter(|function| function.category == Category::A)
        .collect();
    let output = std::path::PathBuf::from(std::env::var_os("OUT_DIR").ok_or("missing OUT_DIR")?);
    fs::write(output.join("host_glue.rs"), render::host_glue(&bound))?;
    Ok(())
}
