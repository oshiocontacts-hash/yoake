use std::fmt::Write as _;
use std::path::PathBuf;

use yoake_hook_ledger::Ledger;

fn main() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let ledger_path = manifest_dir.join("hook-ids.lock.json");
    println!("cargo:rerun-if-changed={}", ledger_path.display());
    let ledger = Ledger::load(&ledger_path)
        .unwrap_or_else(|error| panic!("invalid hook ledger {}: {error}", ledger_path.display()));

    let mut keys = vec![String::new(); ledger.max_slots() as usize];
    for entry in ledger.live() {
        keys[entry.id as usize] = entry.key.clone();
    }
    let mut generated = format!(
        "pub const MAX_SLOTS: usize = {};\n\
         pub const TEST_ID_BASE: usize = {};\n\
         pub const TEST_SLOTS: usize = 64;\n\
         pub const LEDGER_KEYS: [&str; MAX_SLOTS] = [\n",
        ledger.max_slots(),
        ledger.max_slots(),
    );
    for key in keys {
        writeln!(generated, "    {key:?},").unwrap();
    }
    generated.push_str("];\n");

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out_dir.join("ledger.rs"), generated).unwrap();
}
