use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Dump {
    pub toolchain: String,
    #[serde(rename = "crate")]
    pub crate_name: String,
    pub files: BTreeMap<String, FileSummary>,
    pub calls: Vec<Call>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct FileSummary {
    pub written_calls: usize,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Span(pub u32, pub u32, pub u32, pub u32);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParsedSpan {
    pub position: Span,
    pub ctxt: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Argument {
    pub ty: Option<String>,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    Method,
    Path,
    Overloaded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Borrow {
    None,
    Shared,
    Mut,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiverAdjustment {
    pub borrow: Borrow,
    pub derefs: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Resolution {
    Concrete,
    Dyn,
    Generic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Call {
    pub file: String,
    pub span: Span,
    pub fun_span: Span,
    pub ctxt: u32,
    pub kind: CallKind,
    pub caller_def: String,
    pub callee: String,
    pub callee_def: String,
    pub ret_ty: Option<String>,
    pub args: Vec<Argument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recv_adjust: Option<ReceiverAdjustment>,
    pub resolution: Resolution,
}
