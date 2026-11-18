use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static TEST_SEQ: AtomicU64 = AtomicU64::new(0);

struct TestDir(PathBuf);

impl TestDir {
    fn new(label: &str) -> Self {
        let seq = TEST_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "yoake_codemod_callsite_test_{label}_{}_{}",
            std::process::id(),
            seq
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn span_of(src: &str, needle: &str) -> [usize; 4] {
    let start_byte = src.find(needle).unwrap_or_else(|| panic!("missing {needle:?} in:\n{src}"));
    let end_byte = start_byte + needle.len();
    let (sl, sc) = byte_to_line_col(src, start_byte);
    let (el, ec) = byte_to_line_col(src, end_byte);
    [sl, sc, el, ec]
}

fn byte_to_line_col(src: &str, offset: usize) -> (usize, usize) {
    let prefix = &src[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |p| p + 1);
    let col = prefix[line_start..].chars().count() + 1;
    (line, col)
}

fn codemod_bin() -> &'static str {
    env!("CARGO_BIN_EXE_yoake-hook-codemod")
}

fn write_fixture_crate(root: &Path, bag_rs: &str, ledger_json: &str, next_id: u32) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("ledger")).unwrap();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub mod bag;\n").unwrap();
    fs::write(root.join("src/bag.rs"), bag_rs).unwrap();
    fs::write(
        root.join("ledger/meta.json"),
        format!("{{\n  \"version\": 2,\n  \"next_id\": {next_id}\n}}\n"),
    )
    .unwrap();
    fs::write(root.join("ledger/demo.json"), ledger_json).unwrap();
}

#[test]
fn apply_rewrites_with_adjust_prefix_generates_shim_block_once_and_is_idempotent() {
    let dir = TestDir::new("basic");
    let bag_rs = "\
pub struct Bag {
    pub items: Vec<u32>,
}

impl Bag {
    pub fn first_item(&self) -> Option<&u32> {
        self.items.first()
    }

    pub fn touch(&mut self, other: &Self, opaque: impl Fn()) -> Option<&u32> {
        assert!(self.ok());
        self.ignore(opaque);
        let _ = self.pick(other);
        if self.ok() {
            self.items.first()
        } else {
            None
        }
    }

    pub fn ok(&self) -> bool {
        !self.items.is_empty()
    }

    pub fn ignore(&self, _f: impl Fn()) {}

    pub fn pick(&self, _other: &Self) -> &u32 {
        &self.items[0]
    }
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::first_item", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::bag::Bag::touch", "sig": "0000000000000002", "status": "live", "kind": "fn" },
    { "id": 3, "key": "demo::bag::Bag::ok", "sig": "0000000000000003", "status": "live", "kind": "fn" },
    { "id": 4, "key": "demo::bag::Bag::ignore", "sig": "0000000000000004", "status": "live", "kind": "fn" },
    { "id": 5, "key": "demo::bag::Bag::pick", "sig": "0000000000000005", "status": "live", "kind": "fn" }
  ],
  "excluded": []
}
"#;
    write_fixture_crate(dir.path(), bag_rs, ledger_json, 10);

    let first_call_span = span_of(bag_rs, "self.items.first()");
    let assert_ok_span = span_of(bag_rs, "self.ok()");
    let ignore_span = span_of(bag_rs, "self.ignore(opaque)");
    let pick_span = span_of(bag_rs, "self.pick(other)");
    let if_ok_span = {
        let first = bag_rs.find("self.ok()").unwrap();
        let second = bag_rs[first + 1..].find("self.ok()").unwrap() + first + 1;
        let (sl, sc) = byte_to_line_col(bag_rs, second);
        let (el, ec) = byte_to_line_col(bag_rs, second + "self.ok()".len());
        [sl, sc, el, ec]
    };
    let else_first_span = {
        let first = bag_rs.find("self.items.first()").unwrap();
        let second = bag_rs[first + 1..].find("self.items.first()").unwrap() + first + 1;
        let (sl, sc) = byte_to_line_col(bag_rs, second);
        let (el, ec) = byte_to_line_col(bag_rs, second + "self.items.first()".len());
        [sl, sc, el, ec]
    };
    let is_empty_span = span_of(bag_rs, "self.items.is_empty()");

    let calls_json = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": first_call_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "core::slice::first",
                "callee_def": "core::slice::{impl#0}::first",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": assert_ok_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ok",
                "callee_def": "bag::{impl#0}::ok",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 1 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": ignore_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ignore",
                "callee_def": "bag::{impl#0}::ignore",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ crate::bag::Bag" }, { "ty": null } ],
                "recv_adjust": { "borrow": "shared", "derefs": 1 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": pick_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::pick",
                "callee_def": "bag::{impl#0}::pick",
                "ret_ty": "&'_ u32",
                "args": [ { "ty": "&'_ crate::bag::Bag" }, { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 1 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": if_ok_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ok",
                "callee_def": "bag::{impl#0}::ok",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 1 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": else_first_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "core::slice::first",
                "callee_def": "core::slice::{impl#0}::first",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": is_empty_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "alloc::vec::Vec::is_empty",
                "callee_def": "alloc::vec::Vec::is_empty",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::vec::Vec<u32>" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&calls_json).unwrap()).unwrap();

    let out = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "apply failed:\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(
        stdout.contains("callsites: joined=6 eligible=5 rewritten=5 excluded=2"),
        "unexpected summary:\n{stdout}"
    );
    assert!(stdout.contains("inside macro invocation=1"), "{stdout}");
    assert!(stdout.contains("unwritable arg type=1"), "{stdout}");
    // A borrowed return on a borrowed receiver now uses the method elision rule (named `'r`).
    assert!(!stdout.contains("borrowed return with several borrowed inputs"), "{stdout}");

    let rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(rewritten.contains("__yoake_cs_10(&self.items)"), "{rewritten}");
    assert!(rewritten.contains("__yoake_cs_11(&*self, other)"), "{rewritten}");
    assert!(rewritten.contains("__yoake_cs_12(&*self)"), "{rewritten}");
    assert!(rewritten.contains("__yoake_cs_13(&self.items)"), "{rewritten}");
    assert!(rewritten.contains("__yoake_cs_14(&self.items)"), "{rewritten}");
    assert!(rewritten.contains("fn __yoake_cs_11<'r>(recv: &'r crate::bag::Bag, a0: &crate::bag::Bag) -> &'r u32"), "{rewritten}");
    assert!(rewritten.contains("assert!(self.ok());"), "{rewritten}");
    assert!(rewritten.contains("self.ignore(opaque);"), "{rewritten}");
    assert_eq!(
        rewritten
            .matches("// Generated by yoake-hook-codemod: call-site shims (hook-ledger). Do not edit by hand.")
            .count(),
        1,
        "{rewritten}"
    );
    assert!(
        rewritten.contains("fn __yoake_cs_10<'r>(recv: &'r [u32]) -> std::option::Option<&'r u32>"),
        "{rewritten}"
    );
    assert!(
        rewritten.contains("key = \"fn:2@call:fn:3\""),
        "{rewritten}"
    );
    assert!(
        rewritten.contains("key = \"fn:2@call:core::slice::first\""),
        "{rewritten}"
    );

    let ledger_after = fs::read_to_string(dir.path().join("ledger/demo.json")).unwrap();
    assert!(ledger_after.contains("\"site\": {"), "{ledger_after}");
    assert!(ledger_after.contains("\"shim\": \"__yoake_cs_11\""), "{ledger_after}");

    let chk = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag"])
        .output()
        .unwrap();
    let chk_out = String::from_utf8_lossy(&chk.stdout);
    let chk_err = String::from_utf8_lossy(&chk.stderr);
    assert!(chk.status.success(), "syn-only check failed:\n{chk_out}\n{chk_err}");
    assert!(chk_out.contains("check: clean"), "{chk_out}");

    let calls2_json = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "__yoake_cs_10(&self.items)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_10",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "self.ok()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ok",
                "callee_def": "bag::{impl#0}::ok",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 1 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "self.ignore(opaque)"),
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ignore",
                "callee_def": "bag::{impl#0}::ignore",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ crate::bag::Bag" }, { "ty": null } ],
                "recv_adjust": { "borrow": "shared", "derefs": 1 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "__yoake_cs_11(&*self, other)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_11",
                "ret_ty": "&'_ u32",
                "args": [ { "ty": "&'_ crate::bag::Bag" }, { "ty": "&'_ crate::bag::Bag" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "__yoake_cs_12(&*self)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_12",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "__yoake_cs_13(&self.items)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_13",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "__yoake_cs_14(&self.items)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_14",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::vec::Vec<u32>" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "recv.first()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "core::slice::first",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    let thir2_path = dir.path().join("calls2.json");
    fs::write(&thir2_path, serde_json::to_string_pretty(&calls2_json).unwrap()).unwrap();

    let out2 = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir2_path)
        .output()
        .unwrap();
    assert!(out2.status.success(), "second apply failed: {}", String::from_utf8_lossy(&out2.stderr));
    let rewritten2 = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    let ledger_after2 = fs::read_to_string(dir.path().join("ledger/demo.json")).unwrap();
    assert_eq!(rewritten, rewritten2);
    assert_eq!(ledger_after, ledger_after2);
}

#[test]
fn existing_rewritten_site_keeps_id_when_new_site_added() {
    let dir = TestDir::new("keep_id");
    let bag_rs = "\
pub struct Bag {
    pub items: Vec<u32>,
}

impl Bag {
    pub fn check_both(&self) -> bool {
        let has_first = __yoake_cs_10(&self.items).is_some();
        has_first && self.ok()
    }

    pub fn ok(&self) -> bool {
        true
    }
}

// Generated by yoake-hook-codemod: call-site shims (hook-ledger). Do not edit by hand.
#[yoake_hook_macro::yoake_hook(id = 10, key = \"fn:1@call:core::slice::first\", kind = callsite)]
#[inline(always)]
fn __yoake_cs_10(recv: &[u32]) -> std::option::Option<&u32> {
    recv.first()
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::check_both", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::bag::Bag::ok", "sig": "0000000000000002", "status": "live", "kind": "fn" },
    {
      "id": 10,
      "key": "fn:1@call:core::slice::first",
      "sig": "0000000000000010",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": null,
        "callee_path": "core::slice::first",
        "anchor": "1111111111111111",
        "stmt_path": ["2222222222222222"],
        "context": ["0000000000000000", "3333333333333333"],
        "dup_rank": 0,
        "shim": "__yoake_cs_10"
      }
    }
  ],
  "excluded": []
}
"#;
    write_fixture_crate(dir.path(), bag_rs, ledger_json, 11);

    let calls_json = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "__yoake_cs_10(&self.items)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_10",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "__yoake_cs_10(&self.items).is_some()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "std::option::Option::is_some",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::option::Option<&'_ u32>" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "self.ok()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ok",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "recv.first()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "core::slice::first",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&calls_json).unwrap()).unwrap();

    let out = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "apply failed: {}", String::from_utf8_lossy(&out.stderr));

    let rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(rewritten.contains("__yoake_cs_11(&__yoake_cs_10(&self.items))"), "{rewritten}");
    assert!(rewritten.contains("__yoake_cs_12(self)"), "{rewritten}");
    assert!(rewritten.contains("fn __yoake_cs_10("), "{rewritten}");
    assert!(rewritten.contains("fn __yoake_cs_11("), "{rewritten}");
    assert!(rewritten.contains("fn __yoake_cs_12("), "{rewritten}");
}

#[test]
fn chained_method_calls_rewrite_without_overlap() {
    let dir = TestDir::new("chained");
    let bag_rs = "\
pub struct Bag {
    pub items: Vec<u32>,
}

impl Bag {
    pub fn has_first(&self) -> bool {
        self.items.first().is_some()
    }
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::has_first", "sig": "0000000000000001", "status": "live", "kind": "fn" }
  ],
  "excluded": []
}
"#;
    write_fixture_crate(dir.path(), bag_rs, ledger_json, 10);

    let calls_json = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "self.items.first()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "core::slice::first",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "self.items.first().is_some()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "std::option::Option::is_some",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::option::Option<&'_ u32>" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&calls_json).unwrap()).unwrap();

    let out = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "apply failed: {}", String::from_utf8_lossy(&out.stderr));
    let rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(rewritten.contains("__yoake_cs_11(&__yoake_cs_10(&self.items))"), "{rewritten}");
}

#[test]
fn moved_site_requires_accept_site_and_lost_site_requires_tombstone_site() {
    let dir = TestDir::new("accept_tombstone");
    let before_rs = "\
pub struct Bag;

impl Bag {
    pub fn run(&self, cond: bool) {
        if cond {
            self.ping();
        }
    }

    pub fn ping(&self) {}
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::run", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::bag::Bag::ping", "sig": "0000000000000002", "status": "live", "kind": "fn" }
  ],
  "excluded": []
}
"#;
    write_fixture_crate(dir.path(), before_rs, ledger_json, 10);

    let before_calls = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": span_of(before_rs, "self.ping()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ping",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&before_calls).unwrap()).unwrap();

    // First run check (leaves file unrewritten) or apply then revert source to simulate unrewritten move.
    let apply1 = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(apply1.status.success(), "{}", String::from_utf8_lossy(&apply1.stderr));

    // Move unrewritten self.ping() to else branch while ledger still has id=10 unrewritten.
    let after_rs = "\
pub struct Bag;

impl Bag {
    pub fn run(&self, cond: bool) {
        if cond {
        } else {
            self.ping();
        }
    }

    pub fn ping(&self) {}
}
";
    fs::write(dir.path().join("src/bag.rs"), after_rs).unwrap();
    let ping_span = span_of(after_rs, "self.ping()");
    let after_calls = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": ping_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ping",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    fs::write(&thir_path, serde_json::to_string_pretty(&after_calls).unwrap()).unwrap();

    let expected_moved = format!(
        "callsite moved: fn:1@call:fn:2 -> candidate src/bag.rs:{}:{} `self.ping()`",
        ping_span[0], ping_span[1]
    );
    let check_move = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!check_move.status.success());
    let check_move_err = String::from_utf8_lossy(&check_move.stderr);
    assert!(check_move_err.contains(&expected_moved), "{check_move_err}");

    let fail_move = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!fail_move.status.success());
    let move_err = String::from_utf8_lossy(&fail_move.stderr);
    assert!(move_err.contains(&expected_moved), "{move_err}");

    let accept_flag = format!(
        "fn:1@call:fn:2@src/bag.rs:{}:{}",
        ping_span[0], ping_span[1]
    );
    let pass_accept = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .args(["--accept-site", &accept_flag])
        .output()
        .unwrap();
    assert!(pass_accept.status.success(), "{}", String::from_utf8_lossy(&pass_accept.stderr));
    let moved_rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(moved_rewritten.contains("__yoake_cs_10(self)"), "{moved_rewritten}");
    assert!(moved_rewritten.contains("key = \"fn:1@call:fn:2\""), "{moved_rewritten}");

    // Delete the rewritten call and shim -> lost until --tombstone-site.
    fs::write(
        dir.path().join("src/bag.rs"),
        "pub struct Bag;\nimpl Bag {\n    pub fn run(&self, _cond: bool) {}\n    pub fn ping(&self) {}\n}\n",
    )
    .unwrap();
    let empty_calls = serde_json::json!({ "crate": "demo", "calls": [] });
    fs::write(&thir_path, serde_json::to_string_pretty(&empty_calls).unwrap()).unwrap();

    let fail_lost = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag"])
        .output()
        .unwrap();
    assert!(!fail_lost.status.success());
    let lost_err = String::from_utf8_lossy(&fail_lost.stderr);
    assert!(lost_err.contains("missing from source"), "{lost_err}");

    let pass_tombstone = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .args(["--tombstone-site", "fn:1@call:fn:2"])
        .output()
        .unwrap();
    assert!(pass_tombstone.status.success(), "{}", String::from_utf8_lossy(&pass_tombstone.stderr));
    let ledger_tomb = fs::read_to_string(dir.path().join("ledger/demo.json")).unwrap();
    assert!(ledger_tomb.contains("\"status\": \"tombstone\""), "{ledger_tomb}");
}

#[test]
fn rename_site_updates_key_and_keeps_id_on_real_key_change() {
    let dir = TestDir::new("rename_real_key");
    let bag_rs = "\
pub struct Bag;

impl Bag {
    pub fn run(&self) {
        self.pong();
    }

    pub fn pong(&self) {}
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::run", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::bag::Bag::ping", "sig": "0000000000000002", "status": "tombstone", "kind": "fn" },
    { "id": 3, "key": "demo::bag::Bag::pong", "sig": "0000000000000003", "status": "live", "kind": "fn" },
    {
      "id": 10,
      "key": "fn:1@call:fn:2",
      "sig": "0000000000000010",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": 2,
        "callee_path": "demo::bag::Bag::ping",
        "anchor": "1111111111111111",
        "stmt_path": ["2222222222222222"],
        "context": ["0000000000000000", "0000000000000000"],
        "dup_rank": 0,
        "shim": "__yoake_cs_10"
      }
    }
  ],
  "excluded": []
}
"#;
    write_fixture_crate(dir.path(), bag_rs, ledger_json, 11);

    let calls_json = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "self.pong()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::pong",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&calls_json).unwrap()).unwrap();

    let fail_lost = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!fail_lost.status.success());
    let lost_err = String::from_utf8_lossy(&fail_lost.stderr);
    assert!(lost_err.contains("callsite lost: fn:1@call:fn:2"), "{lost_err}");

    let pass_rename = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .args(["--rename-site", "fn:1@call:fn:2=fn:1@call:fn:3"])
        .output()
        .unwrap();
    assert!(pass_rename.status.success(), "{}", String::from_utf8_lossy(&pass_rename.stderr));
    let rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(rewritten.contains("__yoake_cs_10(self)"), "{rewritten}");
    assert!(rewritten.contains("key = \"fn:1@call:fn:3\""), "{rewritten}");
    assert!(rewritten.contains("recv.pong()"), "{rewritten}");

    let ledger_after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("ledger/demo.json")).unwrap()).unwrap();
    let entry10 = ledger_after["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == 10)
        .unwrap();
    assert_eq!(entry10["key"], "fn:1@call:fn:3");
    assert_eq!(entry10["renamed_from"], serde_json::json!(["fn:1@call:fn:2"]));
    assert_eq!(entry10["site"]["callee_fn"], 3);
    assert_eq!(entry10["site"]["callee_path"], "demo::bag::Bag::pong");

    // Now rename caller fn (fn:1 -> fn:4) while site __yoake_cs_10 is already rewritten.
    let exec_rs = rewritten.replace("pub fn run(&self)", "pub fn execute(&self)");
    fs::write(dir.path().join("src/bag.rs"), &exec_rs).unwrap();
    let mut updated_ledger = ledger_after.clone();
    let entries = updated_ledger["entries"].as_array_mut().unwrap();
    for entry in entries.iter_mut() {
        if entry["id"] == 1 {
            entry["status"] = serde_json::json!("tombstone");
        }
    }
    entries.push(serde_json::json!({
        "id": 4,
        "key": "demo::bag::Bag::execute",
        "sig": "0000000000000004",
        "status": "live",
        "kind": "fn"
    }));
    fs::write(
        dir.path().join("ledger/demo.json"),
        serde_json::to_string_pretty(&updated_ledger).unwrap(),
    )
    .unwrap();
    let exec_calls = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": span_of(&exec_rs, "__yoake_cs_10(self)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_10",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&exec_rs, "recv.pong()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::pong",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    fs::write(&thir_path, serde_json::to_string_pretty(&exec_calls).unwrap()).unwrap();

    let caller_rename = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .args(["--rename-site", "fn:1@call:fn:3=fn:4@call:fn:3"])
        .output()
        .unwrap();
    assert!(caller_rename.status.success(), "{}", String::from_utf8_lossy(&caller_rename.stderr));
    let exec_rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(exec_rewritten.contains("key = \"fn:4@call:fn:3\""), "{exec_rewritten}");
    assert!(!exec_rewritten.contains("key = \"fn:1@call:fn:3\""), "{exec_rewritten}");

    let chk = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag"])
        .output()
        .unwrap();
    assert!(chk.status.success(), "{}", String::from_utf8_lossy(&chk.stderr));
}

#[test]
fn changed_site_requires_accept_site_and_rejects_same_key_rename() {
    let dir = TestDir::new("changed_accept");
    let before_rs = "\
pub struct Bag;

impl Bag {
    pub fn run(&self) -> usize {
        let n = self.len_a();
        n
    }

    pub fn len_a(&self) -> usize {
        1
    }
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::run", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::bag::Bag::len_a", "sig": "0000000000000002", "status": "live", "kind": "fn" }
  ],
  "excluded": []
}
"#;
    write_fixture_crate(dir.path(), before_rs, ledger_json, 10);

    let before_calls = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": span_of(before_rs, "self.len_a()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::len_a",
                "ret_ty": "usize",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&before_calls).unwrap()).unwrap();

    let apply1 = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(apply1.status.success(), "{}", String::from_utf8_lossy(&apply1.stderr));

    let after_rs = "\
pub struct Bag;

impl Bag {
    pub fn run(&self) -> usize {
        let n = (self).len_a() + 1;
        n
    }

    pub fn len_a(&self) -> usize {
        1
    }
}
";
    fs::write(dir.path().join("src/bag.rs"), after_rs).unwrap();
    let len_span = span_of(after_rs, "(self).len_a()");
    let after_calls = serde_json::json!({
        "crate": "demo",
        "calls": [
            {
                "file": "src/bag.rs",
                "span": len_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::len_a",
                "ret_ty": "usize",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]
    });
    fs::write(&thir_path, serde_json::to_string_pretty(&after_calls).unwrap()).unwrap();

    let check_changed = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!check_changed.status.success());
    let changed_err = String::from_utf8_lossy(&check_changed.stderr);
    let expected_changed = format!(
        "callsite changed: fn:1@call:fn:2 -> candidate src/bag.rs:{}:{} `(self).len_a()`",
        len_span[0], len_span[1]
    );
    assert!(changed_err.contains(&expected_changed), "{changed_err}");

    let same_key_rename = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .args(["--rename-site", "fn:1@call:fn:2=fn:1@call:fn:2"])
        .output()
        .unwrap();
    assert!(!same_key_rename.status.success());
    let same_err = String::from_utf8_lossy(&same_key_rename.stderr);
    assert!(same_err.contains("old and new keys are identical"), "{same_err}");

    let accept_flag = format!(
        "fn:1@call:fn:2@src/bag.rs:{}:{}",
        len_span[0], len_span[1]
    );
    let pass_accept = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .args(["--accept-site", &accept_flag])
        .output()
        .unwrap();
    assert!(pass_accept.status.success(), "{}", String::from_utf8_lossy(&pass_accept.stderr));
    let rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(rewritten.contains("__yoake_cs_10((self)) + 1"), "{rewritten}");
    assert!(rewritten.contains("key = \"fn:1@call:fn:2\""), "{rewritten}");
}
