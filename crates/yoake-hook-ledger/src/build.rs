use std::fmt::Write as _;
use std::path::PathBuf;

use crate::Ledger;

pub fn emit_crate_table(crate_name: &str) {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let mut ancestor = manifest_dir.as_path();
    let ledger_dir = loop {
        let candidate = ancestor.join("hook-ledger");
        if candidate.is_dir() {
            break candidate;
        }
        ancestor = ancestor
            .parent()
            .unwrap_or_else(|| panic!("could not find hook-ledger above {}", manifest_dir.display()));
    };
    let shard_path = ledger_dir.join(format!("{crate_name}.json"));
    println!("cargo:rerun-if-changed={}", shard_path.display());
    let shard = Ledger::load(&shard_path)
        .unwrap_or_else(|error| panic!("invalid hook shard {}: {error}", shard_path.display()));

    let mut generated = String::from("pub const KEYS: &[(usize, &str)] = &[\n");
    let mut live: Vec<_> = shard.live().collect();
    live.sort_by_key(|entry| entry.id);
    for entry in live {
        writeln!(generated, "    ({}, {:?}),", entry.id, entry.key).unwrap();
    }
    generated.push_str("];\n");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("yoake_hook_keys.rs"), generated).unwrap();
}
