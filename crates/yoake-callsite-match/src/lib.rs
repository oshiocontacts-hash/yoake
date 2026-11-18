mod extract;
mod lcs;
mod reidentify;

pub use extract::{
    ExtractedCall, SpanRange, assign_dup_ranks, extract, extract_block_calls, extract_caller,
    fnv1a_64, hash_tokens,
};
pub use reidentify::{records_from_sites, reidentify, reidentify_with_snapshots};

/// Formats a 64-bit hash as a 16-character lowercase hex string.
pub fn format_hash(hash: u64) -> String {
    format!("{hash:016x}")
}

/// Parses a 16-character hex hash string into a `u64`.
pub fn parse_hash(hex: &str) -> Option<u64> {
    if hex.len() != 16 {
        return None;
    }
    u64::from_str_radix(hex, 16).ok()
}

/// Extracted call-site metadata for one call expression inside a caller body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub caller: String,
    pub callee: String,
    pub anchor: u64,
    pub stmt_index: usize,
    pub stmt_hash: u64,
    pub stmt_path: Vec<usize>,
    pub stmt_path_hashes: Vec<u64>,
    pub context: (u64, u64),
    pub dup_rank: u32,
    pub in_closure: bool,
}

/// Persisted identity record for a known call site.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub id: u32,
    pub caller: String,
    pub callee: String,
    pub anchor: u64,
    pub stmt_path_hashes: Vec<u64>,
    pub context: (u64, u64),
    pub dup_rank: u32,
}

/// Top-level statement hashes of a caller body for diff alignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallerSnapshot {
    pub caller: String,
    pub body_stmt_hashes: Vec<u64>,
}

/// Result of matching old call-site records against newly extracted sites.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub confirmed: Vec<(u32, usize)>,
    pub moved: Vec<(u32, usize)>,
    pub changed: Vec<(u32, usize)>,
    pub fragile: Vec<(u32, usize)>,
    pub ambiguous: Vec<(u32, Vec<usize>)>,
    pub lost: Vec<u32>,
    pub new: Vec<(u32, usize)>,
}
