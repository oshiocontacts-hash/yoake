use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::io::BufReader;
use std::path::{Path, PathBuf};

mod model;
mod normalize;
mod parser;
mod receiver;
mod span;

pub mod dump;

pub use model::*;

#[derive(Debug)]
pub struct Error(String);

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x100000001b3;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileDigest {
    pub sha: String,
    pub len: u64,
}

pub fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = FNV_OFFSET_BASIS;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

pub fn format_hash(hash: u64) -> String {
    format!("{hash:016x}")
}

pub fn hash_file_bytes(bytes: &[u8]) -> FileDigest {
    FileDigest {
        sha: format_hash(fnv1a_64(bytes)),
        len: bytes.len() as u64,
    }
}

pub fn cargo_lock_sha(workspace: &Path) -> Result<String> {
    let path = workspace.join("Cargo.lock");
    let bytes = std::fs::read(&path)
        .map_err(|error| Error::new(format!("failed to read {}: {error}", path.display())))?;
    Ok(format_hash(fnv1a_64(&bytes)))
}

pub fn collect_file_digests(workspace: &Path, files: &[String]) -> Result<BTreeMap<String, FileDigest>> {
    let mut digests = BTreeMap::new();
    for file in files {
        let key = file.replace('\\', "/");
        let path = workspace.join(file);
        let bytes = std::fs::read(&path).map_err(|error| {
            Error::new(format!("failed to read source file {}: {error}", path.display()))
        })?;
        digests.insert(key, hash_file_bytes(&bytes));
    }
    Ok(digests)
}

pub fn compute_source_snapshot(files: &BTreeMap<String, FileSummary>) -> String {
    let mut hash = FNV_OFFSET_BASIS;
    for (path, summary) in files {
        for &byte in path.as_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash ^= 0;
        hash = hash.wrapping_mul(FNV_PRIME);
        for &byte in summary.sha.as_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        hash ^= u64::from(b'\n');
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format_hash(hash)
}

#[derive(Clone, Debug, Default)]
pub struct ParseOptions {
    pub files: Vec<String>,
    pub externs: BTreeSet<String>,
    pub crate_name: Option<String>,
    pub rustc: String,
    pub cargo_lock_sha: String,
    pub workspace: Option<PathBuf>,
    pub file_digests: BTreeMap<String, FileDigest>,
}

pub fn parse_file(thir: &Path, options: &ParseOptions) -> Result<Dump> {
    let file = std::fs::File::open(thir)?;
    parser::parse_reader(BufReader::with_capacity(1 << 20, file), options)
}

pub fn parse_str(thir: &str, options: &ParseOptions) -> Result<Dump> {
    parser::parse_reader(std::io::Cursor::new(thir), options)
}

pub fn write_json(dump: &Dump, output: &Path) -> Result<()> {
    let mut json = serde_json::to_string_pretty(dump)?;
    json.push('\n');
    std::fs::write(output, json)?;
    Ok(())
}
