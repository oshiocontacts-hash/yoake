use std::collections::BTreeSet;
use std::fmt;
use std::io::BufReader;
use std::path::Path;

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

#[derive(Clone, Debug, Default)]
pub struct ParseOptions {
    pub files: Vec<String>,
    pub externs: BTreeSet<String>,
    pub crate_name: Option<String>,
    pub toolchain: String,
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
