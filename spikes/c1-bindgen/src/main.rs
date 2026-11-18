use std::collections::BTreeMap;
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

use c1_bindgen::classify::classify;
use c1_bindgen::model::{extract_functions, Category, Function};
use c1_bindgen::render::{classification_md, guest_stubs, host_glue};
use serde::Serialize;

#[derive(Serialize)]
struct Summary {
    format_version: u64,
    total: usize,
    counts: BTreeMap<String, usize>,
    percentages: BTreeMap<String, f64>,
    reason_histogram: BTreeMap<String, usize>,
    span_suffixes: Vec<String>,
}

#[derive(Serialize)]
struct IdEntry {
    id: u32,
    path: String,
    rustdoc_id: u64,
    span: String,
}

#[derive(Serialize)]
struct IdLock {
    format: u32,
    bindings: Vec<IdEntry>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("c1-bindgen: {error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = env::args().collect();
    if arguments.len() < 4 {
        return Err("usage: c1-bindgen <rustdoc.json> <out-dir> <span-suffix>...".into());
    }
    let input = fs::read_to_string(&arguments[1])?;
    let document: serde_json::Value = serde_json::from_str(&input)?;
    let format_version = document
        .get("format_version")
        .and_then(serde_json::Value::as_u64)
        .ok_or("rustdoc JSON has no integer format_version")?;
    let suffixes = arguments[3..].to_vec();
    let mut functions = extract_functions(&document, &suffixes)?;
    for function in &mut functions {
        classify(function, &document);
    }
    functions.sort_by(|left, right| {
        left.full_path
            .cmp(&right.full_path)
            .then(left.rustdoc_id.cmp(&right.rustdoc_id))
    });

    let mut next_id = 1u32;
    let mut ids = Vec::new();
    for function in functions.iter_mut().filter(|function| function.category == Category::A) {
        function.binding_id = Some(next_id);
        ids.push(IdEntry {
            id: next_id,
            path: function.full_path.clone(),
            rustdoc_id: function.rustdoc_id,
            span: function.span.clone(),
        });
        next_id = next_id.checked_add(1).ok_or("binding id overflow")?;
    }

    let output = Path::new(&arguments[2]);
    fs::create_dir_all(output)?;
    fs::write(output.join("classification.md"), classification_md(&functions))?;
    fs::write(
        output.join("summary.json"),
        serde_json::to_vec_pretty(&summarize(&functions, format_version, suffixes))?,
    )?;
    fs::write(
        output.join("ids.lock.json"),
        serde_json::to_vec_pretty(&IdLock { format: 1, bindings: ids })?,
    )?;
    let bound: Vec<&Function> = functions
        .iter()
        .filter(|function| function.category == Category::A)
        .collect();
    fs::write(output.join("host_glue.rs"), host_glue(&bound))?;
    fs::write(output.join("guest_stubs.rs"), guest_stubs(&bound))?;
    println!(
        "c1-bindgen: {} functions, {} category-a bindings",
        functions.len(),
        bound.len()
    );
    Ok(())
}

fn summarize(functions: &[Function], format_version: u64, suffixes: Vec<String>) -> Summary {
    let mut counts = BTreeMap::new();
    let mut percentages = BTreeMap::new();
    let mut reason_histogram = BTreeMap::new();
    for category in [Category::A, Category::B, Category::C] {
        let count = functions
            .iter()
            .filter(|function| function.category == category)
            .count();
        counts.insert(category.label().to_string(), count);
        let percentage = if functions.is_empty() {
            0.0
        } else {
            count as f64 * 100.0 / functions.len() as f64
        };
        percentages.insert(category.label().to_string(), percentage);
    }
    for function in functions {
        for reason in &function.reasons {
            *reason_histogram.entry(reason.clone()).or_insert(0) += 1;
        }
    }
    Summary {
        format_version,
        total: functions.len(),
        counts,
        percentages,
        reason_histogram,
        span_suffixes: suffixes,
    }
}
