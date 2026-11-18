//! The hook-ID key table this crate's `#[yoake_hook]` attributes are checked against (hook-ledger/).
fn main() {
    yoake_hook_ledger::build::emit_crate_table("horizon_client_lib");
}
