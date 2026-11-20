fn main() {
    // The hook-ID key table this crate's `#[yoake_hook]` attributes are checked against (hook-ledger/).
    yoake_hook_ledger::build::emit_crate_table("horizon_protocol");
}
