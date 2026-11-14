//! How a Rust value maps onto a WIT value, kept next to the WIT text so the generated WASM
//! adapters convert exactly what the WIT declares.
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StrKind {
    /// `String`, `str`, `Uuid` (anything with `Display`/`ToString` that reads back from a string).
    Text,
    /// `PathBuf`, `Path`, `OsString`.
    Path,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ListKind {
    Vec,
    VecDeque,
    HashSet,
    BTreeSet,
    BinaryHeap,
    /// `[T; N]`: the length expression as written.
    Array(String),
    /// `&[T]` and `[T]`: read only.
    Slice,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MapKind {
    HashMap,
    BTreeMap,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WrapKind {
    Box,
    Rc,
    Arc,
    Cow,
    /// `&T` and `&mut T`: read only.
    Ref,
    /// `&'static T`: an answer is made static by interning (strings only).
    StaticRef,
}

/// Where a plain Rust type is defined, for naming it in generated code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RustPath {
    pub file: PathBuf,
    pub krate: String,
    pub module: String,
    /// The definition's own name.
    pub name: String,
    /// The name it was written with where it was met (a `use X as Y` alias), if different.
    pub alias: String,
    /// Lifetime parameters (the generated functions elide them).
    pub lifetimes: usize,
    /// `#[non_exhaustive]`: it cannot be built from another crate.
    pub non_exhaustive: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum Shape {
    /// Not expressible; a hook that needs it is not wired.
    #[default]
    Unsupported,
    Scalar { rust: &'static str },
    Str(StrKind),
    /// `u128` / `i128` as 16 big-endian bytes.
    Wide { signed: bool },
    /// glam vectors as tuples.
    Glam { rust: String, count: usize },
    /// An opaque engine handle (`u64`), only valid during the call.
    Handle,
    Tag,
    Compound,
    Text,
    Json,
    /// `std::time::Duration` as nanoseconds.
    Duration,
    /// `serde_json::Map<String, Value>`: a JSON object as `json` text.
    JsonMap,
    Item,
    Option(Box<Shape>),
    List(Box<Shape>, ListKind),
    Tuple(Vec<Shape>),
    Result(Box<Shape>, Box<Shape>),
    Map(Box<Shape>, Box<Shape>, MapKind),
    Wrap(Box<Shape>, WrapKind),
    /// A generated record, enum or variant, by WIT name (see `NamedDef`).
    Named(String),
    /// A newtype `Name(inner)`: the WIT value is the inner value.
    Newtype { rust: RustPath, inner: Box<Shape>, public: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Field {
    /// The Rust field: a name, or the tuple index.
    pub rust: String,
    /// The WIT field name (kebab).
    pub wit: String,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum VCase {
    Unit,
    Single(Shape),
    Tuple(Vec<Shape>),
    /// A struct variant: its own WIT record and the fields.
    Struct { record: String, fields: Vec<Field> },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NamedDef {
    /// A plain struct with fields: `Name { a, b }` or `Name(a, b)`.
    Record { wit: String, rust: RustPath, fields: Vec<Field>, tuple: bool },
    /// A unit or field-less struct, as `enum { unit }`.
    Unit { wit: String, rust: RustPath, braced: bool },
    /// An enum whose variants are all unit: (Rust variant, WIT case).
    UnitEnum { wit: String, rust: RustPath, cases: Vec<(String, String)> },
    /// An enum with payloads: (Rust variant, WIT case, payload).
    Variant { wit: String, rust: RustPath, cases: Vec<(String, String, VCase)> },
}
