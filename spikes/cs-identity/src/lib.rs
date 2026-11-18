mod extract;
mod lcs;
mod reidentify;

pub use extract::{extract, extract_caller, fnv1a_64};
pub use reidentify::{records_from_sites, reidentify, reidentify_with_snapshots};

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
    pub kept: Vec<(u32, usize)>,
    pub new: Vec<(u32, usize)>,
    pub lost: Vec<u32>,
    pub ambiguous: Vec<(u32, Vec<usize>)>,
    pub fragile: Vec<u32>,
}
