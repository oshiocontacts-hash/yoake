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

fn fnv1a_hex(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn thir_with_file(src: &str, calls: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "crate": "demo",
        "files": {
            "src/bag.rs": {
                "written_calls": 0,
                "sha": fnv1a_hex(src.as_bytes()),
                "len": src.len()
            }
        },
        "calls": calls
    })
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
    fs::write(
        dir.path().join("ledger/callsite-allow.txt"),
        "core::slice::first\nstd::option::Option::is_some\n",
    )
    .unwrap();

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

    let calls_json = thir_with_file(
        bag_rs,
        serde_json::json!([
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
        ]),
    );
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
        stdout.contains("callsites: joined=6 eligible=4 rewritten=4 excluded=3"),
        "unexpected summary:\n{stdout}"
    );
    assert!(stdout.contains("inside macro invocation=1"), "{stdout}");
    assert!(stdout.contains("non-engine callee=1"), "{stdout}");
    assert!(stdout.contains("unwritable arg type=1"), "{stdout}");
    // A borrowed return on a borrowed receiver now uses the method elision rule (named `'r`).
    assert!(!stdout.contains("borrowed return with several borrowed inputs"), "{stdout}");

    let rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(rewritten.contains("__yoake_cs_10(&self.items)"), "{rewritten}");
    assert!(rewritten.contains("__yoake_cs_11(&*self, other)"), "{rewritten}");
    assert!(rewritten.contains("__yoake_cs_12(&*self)"), "{rewritten}");
    assert!(rewritten.contains("__yoake_cs_13(&self.items)"), "{rewritten}");
    assert!(!rewritten.contains("__yoake_cs_14"), "{rewritten}");
    assert!(rewritten.contains("!self.items.is_empty()"), "{rewritten}");
    assert!(rewritten.contains("fn __yoake_cs_11<'r>(recv: &'r crate::bag::Bag, a0: &crate::bag::Bag) -> &'r u32"), "{rewritten}");
    assert!(rewritten.contains("<[u32]>::first(recv)"), "{rewritten}");
    assert!(rewritten.contains("crate::bag::Bag::pick(recv, a0)"), "{rewritten}");
    assert!(rewritten.contains("crate::bag::Bag::ok(recv)"), "{rewritten}");
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
    assert!(ledger_after.contains("\"orig\": \"self.pick(other)\""), "{ledger_after}");

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

    let calls2_json = thir_with_file(
        &rewritten,
        serde_json::json!([
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
                "span": span_of(&rewritten, "self.items.is_empty()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "alloc::vec::Vec::is_empty",
                "callee_def": "alloc::vec::Vec::is_empty",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::vec::Vec<u32>" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(&rewritten, "<[u32]>::first(recv)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "core::slice::first",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ [u32]" } ],
                "resolution": "concrete"
            }
        ]),
    );
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
    fs::write(
        dir.path().join("ledger/callsite-allow.txt"),
        "core::slice::first\nstd::option::Option::is_some\n",
    )
    .unwrap();

    let calls_json = thir_with_file(
        bag_rs,
        serde_json::json!([
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
        ]),
    );
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
    assert!(rewritten.contains("<[u32]>::first(recv)"), "{rewritten}");
    assert!(rewritten.contains("fn __yoake_cs_11("), "{rewritten}");
    assert!(rewritten.contains("<std::option::Option<&u32>>::is_some(recv)"), "{rewritten}");
    assert!(rewritten.contains("fn __yoake_cs_12("), "{rewritten}");
    assert!(rewritten.contains("crate::bag::Bag::ok(recv)"), "{rewritten}");
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
    fs::write(
        dir.path().join("ledger/callsite-allow.txt"),
        "core::slice::first\nstd::option::Option::is_some\n",
    )
    .unwrap();

    let calls_json = thir_with_file(
        bag_rs,
        serde_json::json!([
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
        ]),
    );
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

    let before_calls = thir_with_file(
        before_rs,
        serde_json::json!([
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
        ]),
    );
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
    let after_calls = thir_with_file(
        after_rs,
        serde_json::json!([
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
        ]),
    );
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
    let empty_rs =
        "pub struct Bag;\nimpl Bag {\n    pub fn run(&self, _cond: bool) {}\n    pub fn ping(&self) {}\n}\n";
    fs::write(dir.path().join("src/bag.rs"), empty_rs).unwrap();
    let empty_calls = thir_with_file(empty_rs, serde_json::json!([]));
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
    fs::write(
        dir.path().join("ledger/callsite-allow.txt"),
        "core::slice::first\nstd::option::Option::is_some\n",
    )
    .unwrap();

    let calls_json = thir_with_file(
        bag_rs,
        serde_json::json!([
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
        ]),
    );
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
    assert!(rewritten.contains("crate::bag::Bag::pong(recv)"), "{rewritten}");

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
    let exec_calls = thir_with_file(
        &exec_rs,
        serde_json::json!([
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
                "span": span_of(&exec_rs, "crate::bag::Bag::pong(recv)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::Bag::pong",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "resolution": "concrete"
            }
        ]),
    );
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

    let before_calls = thir_with_file(
        before_rs,
        serde_json::json!([
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
        ]),
    );
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
    let after_calls = thir_with_file(
        after_rs,
        serde_json::json!([
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
        ]),
    );
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

#[test]
fn stale_or_missing_sha_thir_fails_before_reporting_lost() {
    let dir = TestDir::new("stale_thir");
    let before_rs = "\
pub struct Bag;

impl Bag {
    pub fn run(&self) {
        self.ping();
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

    let calls_json = thir_with_file(
        before_rs,
        serde_json::json!([
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
        ]),
    );
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&calls_json).unwrap()).unwrap();

    // Matching sha passes.
    let apply_ok = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(
        apply_ok.status.success(),
        "matching sha should pass:\n{}",
        String::from_utf8_lossy(&apply_ok.stderr)
    );

    // After apply, `src/bag.rs` was rewritten while `calls.json` still holds the pre-rewrite sha.
    // Even though `self.ping()` was rewritten to `__yoake_cs_10(self)` (which would look `lost`
    // against the old THIR), codemod must fail with `THIR is stale` and never report `lost`.
    for mode in ["check", "apply"] {
        let stale_out = Command::new(codemod_bin())
            .args([mode, "--callsites", "--crate-root"])
            .arg(dir.path())
            .args(["--crate-name", "demo", "--ledger"])
            .arg(dir.path().join("ledger"))
            .args(["--module", "bag", "--thir"])
            .arg(&thir_path)
            .output()
            .unwrap();
        assert!(!stale_out.status.success(), "{mode} should fail on stale THIR");
        let err = String::from_utf8_lossy(&stale_out.stderr);
        assert!(err.contains("THIR is stale"), "{mode} stderr:\n{err}");
        assert!(
            err.contains("re-run yoake-hook-thir dump"),
            "{mode} stderr:\n{err}"
        );
        assert!(!err.contains("callsite lost"), "{mode} stderr:\n{err}");
    }

    // Missing `sha` in the dump is also a hard error before matching/lost reporting.
    let no_sha_json = serde_json::json!({
        "crate": "demo",
        "files": {
            "src/bag.rs": { "written_calls": 1 }
        },
        "calls": []
    });
    fs::write(&thir_path, serde_json::to_string_pretty(&no_sha_json).unwrap()).unwrap();
    let missing_out = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!missing_out.status.success());
    let missing_err = String::from_utf8_lossy(&missing_out.stderr);
    assert!(missing_err.contains("THIR is stale"), "{missing_err}");
    assert!(!missing_err.contains("callsite lost"), "{missing_err}");
}

#[test]
fn s3_pin_callee_verifies_and_upgrades_old_shims_in_place() {
    let dir = TestDir::new("s3_pin");
    let bag_rs = "\
pub struct Bag {
    pub maybe: Option<u32>,
}

impl Clone for Bag {
    fn clone(&self) -> Self {
        Self { maybe: self.maybe }
    }
}

impl Bag {
    pub fn run(&self) -> bool {
        let _ = __yoake_cs_10(self);
        let _ = self.maybe.is_none();
        let _ = self.clone();
        true
    }

    pub fn ok(&self) -> bool {
        true
    }
}

// Generated by yoake-hook-codemod: call-site shims (hook-ledger). Do not edit by hand.
#[yoake_hook_macro::yoake_hook(id = 10, key = \"fn:1@call:fn:2\", kind = callsite)]
#[inline(always)]
fn __yoake_cs_10(recv: &crate::bag::Bag) -> bool {
    recv.ok()
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::run", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::bag::Bag::ok", "sig": "0000000000000002", "status": "live", "kind": "fn" },
    {
      "id": 10,
      "key": "fn:1@call:fn:2",
      "sig": "0000000000000010",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": 2,
        "callee_path": "demo::bag::Bag::ok",
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
    fs::write(
        dir.path().join("ledger/callsite-allow.txt"),
        "std::option::Option::is_none\n<Self as core::clone::Clone>::clone\n",
    )
    .unwrap();

    let syn_chk = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag"])
        .output()
        .unwrap();
    assert!(!syn_chk.status.success());
    let syn_out = String::from_utf8_lossy(&syn_chk.stdout);
    assert!(syn_out.contains("shim body not fully qualified"), "{syn_out}");

    let calls_json = thir_with_file(
        bag_rs,
        serde_json::json!([
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "__yoake_cs_10(self)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_10",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "self.maybe.is_none()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "std::option::Option::is_none",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::option::Option<u32>" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "self.clone()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "<Self as core::clone::Clone>::clone",
                "ret_ty": "crate::bag::Bag",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "recv.ok()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::bag::Bag::ok",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 },
                "resolution": "concrete"
            }
        ]),
    );
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&calls_json).unwrap()).unwrap();

    let thir_chk = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!thir_chk.status.success());
    let thir_out = String::from_utf8_lossy(&thir_chk.stdout);
    assert!(thir_out.contains("shim body not fully qualified"), "{thir_out}");

    let apply = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(apply.status.success(), "{}", String::from_utf8_lossy(&apply.stderr));

    let rewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(rewritten.contains("crate::bag::Bag::ok(recv)"), "{rewritten}");
    assert!(rewritten.contains("<std::option::Option<u32>>::is_none(recv)"), "{rewritten}");
    assert!(rewritten.contains("<crate::bag::Bag as core::clone::Clone>::clone(recv)"), "{rewritten}");
    assert!(!rewritten.contains("recv.ok()"), "{rewritten}");

    let clean_chk = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag"])
        .output()
        .unwrap();
    assert!(clean_chk.status.success(), "{}", String::from_utf8_lossy(&clean_chk.stdout));
}

#[test]
fn s5_narrow_default_allowlist_and_retire_ineligible() {
    let dir = TestDir::new("s5_retire");
    let bag_rs = "\
pub struct Bag {
    pub maybe: Option<u32>,
}

impl Bag {
    pub fn run(&self) -> bool {
        let a = self.maybe.is_none();
        let b = __yoake_cs_10(&*self);
        let c = __yoake_cs_11(&__yoake_cs_12(&self.maybe));
        a && b && c
    }

    pub fn ok(&self) -> bool {
        true
    }
}

// Generated by yoake-hook-codemod: call-site shims (hook-ledger). Do not edit by hand.
#[yoake_hook_macro::yoake_hook(id = 10, key = \"fn:1@call:horizon_world::Item::check\", kind = callsite)]
#[inline(always)]
fn __yoake_cs_10(recv: &crate::bag::Bag) -> bool {
    horizon_world::Item::check(recv)
}
#[yoake_hook_macro::yoake_hook(id = 11, key = \"fn:1@call:std::option::Option::is_some\", kind = callsite)]
#[inline(always)]
fn __yoake_cs_11(recv: &std::option::Option<u32>) -> bool {
    <std::option::Option<u32>>::is_some(recv)
}
#[yoake_hook_macro::yoake_hook(id = 12, key = \"fn:1@call:std::option::Option::as_ref\", kind = callsite)]
#[inline(always)]
fn __yoake_cs_12(recv: &std::option::Option<u32>) -> std::option::Option<&u32> {
    <std::option::Option<u32>>::as_ref(recv)
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::run", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    {
      "id": 10,
      "key": "fn:1@call:horizon_world::Item::check",
      "sig": "0000000000000010",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": null,
        "callee_path": "horizon_world::Item::check",
        "anchor": "1111111111111111",
        "stmt_path": ["2222222222222222"],
        "context": ["0000000000000000", "3333333333333333"],
        "dup_rank": 0,
        "shim": "__yoake_cs_10",
        "orig": "self.ok()"
      }
    },
    {
      "id": 11,
      "key": "fn:1@call:std::option::Option::is_some",
      "sig": "0000000000000011",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": null,
        "callee_path": "std::option::Option::is_some",
        "anchor": "4444444444444444",
        "stmt_path": ["5555555555555555"],
        "context": ["0000000000000000", "6666666666666666"],
        "dup_rank": 0,
        "shim": "__yoake_cs_11"
      }
    },
    {
      "id": 12,
      "key": "fn:1@call:std::option::Option::as_ref",
      "sig": "0000000000000012",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": null,
        "callee_path": "std::option::Option::as_ref",
        "anchor": "7777777777777777",
        "stmt_path": ["5555555555555555"],
        "context": ["0000000000000000", "6666666666666666"],
        "dup_rank": 0,
        "shim": "__yoake_cs_12"
      }
    }
  ],
  "excluded": []
}
"#;
    write_fixture_crate(dir.path(), bag_rs, ledger_json, 13);

    let calls_json = thir_with_file(
        bag_rs,
        serde_json::json!([
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "self.maybe.is_none()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "std::option::Option::is_none",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::option::Option<u32>" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "__yoake_cs_10(&*self)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_10",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ crate::bag::Bag" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "__yoake_cs_11(&__yoake_cs_12(&self.maybe))"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_11",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::option::Option<&'_ u32>" } ],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": span_of(bag_rs, "__yoake_cs_12(&self.maybe)"),
                "ctxt": 0,
                "kind": "path",
                "callee": "demo::bag::__yoake_cs_12",
                "ret_ty": "std::option::Option<&'_ u32>",
                "args": [ { "ty": "&'_ std::option::Option<u32>" } ],
                "resolution": "concrete"
            }
        ]),
    );
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&calls_json).unwrap()).unwrap();

    // Without --retire-ineligible: __yoake_cs_11 and __yoake_cs_12 are non-engine callees -> error.
    let fail = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!fail.status.success());
    let fail_err = String::from_utf8_lossy(&fail.stderr);
    assert!(fail_err.contains("__yoake_cs_11") && fail_err.contains("non-engine callee"), "{fail_err}");
    assert!(fail_err.contains("__yoake_cs_12"), "{fail_err}");

    // With --engine-crates demo (excludes horizon_world!) and --retire-ineligible: retires 10, 11, 12.
    let retire = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .args(["--engine-crates", "demo", "--retire-ineligible"])
        .output()
        .unwrap();
    assert!(retire.status.success(), "{}", String::from_utf8_lossy(&retire.stderr));
    let retire_out = String::from_utf8_lossy(&retire.stdout);
    assert!(retire_out.contains("non-engine callee=4"), "{retire_out}");

    let unrewritten = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(unrewritten.contains("let b = self.ok();"), "{unrewritten}");
    assert!(unrewritten.contains("let c = self.maybe.as_ref().is_some();"), "{unrewritten}");
    assert!(!unrewritten.contains("__yoake_cs_"), "{unrewritten}");

    let ledger_after: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join("ledger/demo.json")).unwrap()).unwrap();
    for id in [10, 11, 12] {
        let entry = ledger_after["entries"].as_array().unwrap().iter().find(|e| e["id"] == id).unwrap();
        assert_eq!(entry["status"], "tombstone", "{entry}");
    }

    // Allowlisting Option::is_none via <owner>::method with a comment makes self.maybe.is_none() eligible.
    fs::write(
        dir.path().join("ledger/callsite-allow.txt"),
        "# allow Option::is_none\nOption::is_none\n",
    )
    .unwrap();
    let calls2 = thir_with_file(
        &unrewritten,
        serde_json::json!([
            {
                "file": "src/bag.rs",
                "span": span_of(&unrewritten, "self.maybe.is_none()"),
                "ctxt": 0,
                "kind": "method",
                "callee": "std::option::Option::is_none",
                "ret_ty": "bool",
                "args": [ { "ty": "&'_ std::option::Option<u32>" } ],
                "recv_adjust": { "borrow": "shared", "derefs": 0 },
                "resolution": "concrete"
            }
        ]),
    );
    fs::write(&thir_path, serde_json::to_string_pretty(&calls2).unwrap()).unwrap();
    let allow_ok = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(allow_ok.status.success(), "{}", String::from_utf8_lossy(&allow_ok.stderr));
    let final_rs = fs::read_to_string(dir.path().join("src/bag.rs")).unwrap();
    assert!(final_rs.contains("__yoake_cs_13(&self.maybe)"), "{final_rs}");
}

#[test]
fn s1_track_caller_excludes_std_leaves_and_workspace_scan() {
    let dir = TestDir::new("s1_tc");
    let ws = dir.path();
    fs::create_dir_all(ws.join("crates/horizon_b/src")).unwrap();
    fs::create_dir_all(ws.join("crates/horizon_a/src")).unwrap();
    fs::create_dir_all(ws.join("crates/excluded_pkg/src")).unwrap();
    fs::create_dir_all(ws.join("ledger")).unwrap();

    fs::write(
        ws.join("Cargo.toml"),
        "[workspace]\nexclude = [\"crates/excluded_pkg\"]\nmembers = [\n  \"crates/*\",\n]\n",
    )
    .unwrap();
    fs::write(
        ws.join("crates/horizon_b/Cargo.toml"),
        "[package]\nname = \"horizon-b\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        ws.join("crates/horizon_b/src/lib.rs"),
        "\
pub mod inner;
use inner::Widget;

pub trait Tracked {
    #[track_caller]
    fn fail_trait(&self);
}

#[track_caller]
pub fn free_tc() {}

impl Widget {
    #[track_caller]
    pub fn check_tc(&self) {}

    pub fn ok(&self) {}
}

#[cfg(test)]
#[track_caller]
fn test_only_tc() {}
",
    )
    .unwrap();
    fs::write(
        ws.join("crates/horizon_b/src/inner.rs"),
        "pub struct Widget;\n",
    )
    .unwrap();

    fs::write(
        ws.join("crates/excluded_pkg/Cargo.toml"),
        "[package]\nname = \"excluded-pkg\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        ws.join("crates/excluded_pkg/src/lib.rs"),
        "#[track_caller]\npub fn excluded_tc() {}\n",
    )
    .unwrap();

    fs::write(
        ws.join("crates/horizon_a/Cargo.toml"),
        "[package]\nname = \"horizon-a\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(ws.join("crates/horizon_a/src/lib.rs"), "pub mod caller;\n").unwrap();
    let caller_rs = "\
pub struct Holder;
impl Holder {
    pub fn run(&self, opt: Option<u32>, rc: &std::cell::RefCell<u32>, v: &mut Vec<u32>, w: &horizon_b::inner::Widget) {
        let _ = opt.unwrap();
        let _ = rc.borrow();
        let _ = v.remove(0);
        w.check_tc();
        w.ok();
    }
}
";
    fs::write(ws.join("crates/horizon_a/src/caller.rs"), caller_rs).unwrap();

    let tc_out = ws.join("ledger/track-caller.txt");
    let scan = Command::new(codemod_bin())
        .args(["track-caller-scan", "--workspace"])
        .arg(ws)
        .args(["--out"])
        .arg(&tc_out)
        .output()
        .unwrap();
    assert!(scan.status.success(), "{}", String::from_utf8_lossy(&scan.stderr));
    let tc_text = fs::read_to_string(&tc_out).unwrap();
    assert!(tc_text.contains("horizon_b::free_tc\n"), "{tc_text}");
    assert!(tc_text.contains("horizon_b::inner::Widget::check_tc\n"), "{tc_text}");
    assert!(tc_text.contains("<Self as horizon_b::Tracked>::fail_trait\n"), "{tc_text}");
    assert!(!tc_text.contains("excluded_tc"), "{tc_text}");
    assert!(!tc_text.contains("test_only_tc"), "{tc_text}");
    assert!(!tc_text.contains("ok"), "{tc_text}");
    fs::write(ws.join("ledger/meta.json"), "{\n  \"version\": 2,\n  \"next_id\": 10\n}\n").unwrap();
    fs::write(
        ws.join("ledger/horizon_a.json"),
        r#"{
  "crate": "horizon_a",
  "entries": [
    { "id": 1, "key": "horizon_a::caller::Holder::run", "sig": "0000000000000001", "status": "live", "kind": "fn" }
  ],
  "excluded": []
}"#,
    )
    .unwrap();
    // Even if std leaves are listed in callsite-allow.txt, #[track_caller] still excludes them.
    fs::write(
        ws.join("ledger/callsite-allow.txt"),
        "Option::unwrap\nRefCell::borrow\nVec::remove\n",
    )
    .unwrap();

    let thir_json = serde_json::json!({
        "crate": "horizon_a",
        "files": {
            "src/caller.rs": {
                "written_calls": 5,
                "sha": fnv1a_hex(caller_rs.as_bytes()),
                "len": caller_rs.len()
            }
        },
        "calls": [
            {
                "file": "src/caller.rs",
                "span": span_of(caller_rs, "opt.unwrap()"),
                "ctxt": 0, "kind": "method", "callee": "std::option::Option::unwrap",
                "ret_ty": "u32", "args": [ { "ty": "std::option::Option<u32>" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 }, "resolution": "concrete"
            },
            {
                "file": "src/caller.rs",
                "span": span_of(caller_rs, "rc.borrow()"),
                "ctxt": 0, "kind": "method", "callee": "core::cell::RefCell::borrow",
                "ret_ty": "std::cell::Ref<'_, u32>", "args": [ { "ty": "&'_ std::cell::RefCell<u32>" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 }, "resolution": "concrete"
            },
            {
                "file": "src/caller.rs",
                "span": span_of(caller_rs, "v.remove(0)"),
                "ctxt": 0, "kind": "method", "callee": "alloc::vec::Vec::remove",
                "ret_ty": "u32", "args": [ { "ty": "&'_ mut std::vec::Vec<u32>" }, { "ty": "usize" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 }, "resolution": "concrete"
            },
            {
                "file": "src/caller.rs",
                "span": span_of(caller_rs, "w.check_tc()"),
                "ctxt": 0, "kind": "method", "callee": "horizon_b::inner::Widget::check_tc",
                "ret_ty": "()", "args": [ { "ty": "&'_ horizon_b::inner::Widget" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 }, "resolution": "concrete"
            },
            {
                "file": "src/caller.rs",
                "span": span_of(caller_rs, "w.ok()"),
                "ctxt": 0, "kind": "method", "callee": "horizon_b::inner::Widget::ok",
                "ret_ty": "()", "args": [ { "ty": "&'_ horizon_b::inner::Widget" } ],
                "recv_adjust": { "borrow": "none", "derefs": 0 }, "resolution": "concrete"
            }
        ]
    });
    let thir_path = ws.join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&thir_json).unwrap()).unwrap();

    let apply = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(ws.join("crates/horizon_a"))
        .args(["--crate-name", "horizon_a", "--ledger"])
        .arg(ws.join("ledger"))
        .args(["--module", "caller", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(apply.status.success(), "{}", String::from_utf8_lossy(&apply.stderr));
    let out = String::from_utf8_lossy(&apply.stdout);
    assert!(out.contains("track-caller=4"), "{out}");
    assert!(out.contains("eligible=1 rewritten=1"), "{out}");

    let rewritten = fs::read_to_string(ws.join("crates/horizon_a/src/caller.rs")).unwrap();
    assert!(rewritten.contains("__yoake_cs_10(w)"), "{rewritten}");
    assert!(rewritten.contains("opt.unwrap()"), "{rewritten}");
    assert!(rewritten.contains("w.check_tc()"), "{rewritten}");
}

#[test]
fn s2_one_call_per_shim_errors_on_zero_or_multiple_refs() {
    let dir = TestDir::new("s2_one_call");
    let zero_rs = "\
pub struct Bag;
impl Bag {
    pub fn run(&self) {}
    pub fn ok(&self) -> bool { true }
}
// Generated by yoake-hook-codemod: call-site shims (hook-ledger). Do not edit by hand.
#[yoake_hook_macro::yoake_hook(id = 10, key = \"fn:1@call:fn:2\", kind = callsite)]
#[inline(always)]
fn __yoake_cs_10(recv: &crate::bag::Bag) -> bool {
    crate::bag::Bag::ok(recv)
}
";
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::run", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::bag::Bag::ok", "sig": "0000000000000002", "status": "live", "kind": "fn" },
    {
      "id": 10,
      "key": "fn:1@call:fn:2",
      "sig": "0000000000000010",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": 2,
        "callee_path": "demo::bag::Bag::ok",
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
    write_fixture_crate(dir.path(), zero_rs, ledger_json, 11);

    let zero_chk = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag"])
        .output()
        .unwrap();
    assert!(!zero_chk.status.success());
    let zero_err = String::from_utf8_lossy(&zero_chk.stderr);
    assert!(zero_err.contains("0 references") && zero_err.contains("src/bag.rs:9:"), "{zero_err}");

    let dup_rs = "\
pub struct Bag;
impl Bag {
    pub fn run(&self) -> bool {
        let a = __yoake_cs_10(self);
        let b = __yoake_cs_10(self);
        a && b
    }
    pub fn ok(&self) -> bool { true }
}
// Generated by yoake-hook-codemod: call-site shims (hook-ledger). Do not edit by hand.
#[yoake_hook_macro::yoake_hook(id = 10, key = \"fn:1@call:fn:2\", kind = callsite)]
#[inline(always)]
fn __yoake_cs_10(recv: &crate::bag::Bag) -> bool {
    crate::bag::Bag::ok(recv)
}
";
    fs::write(dir.path().join("src/bag.rs"), dup_rs).unwrap();
    let dup_chk = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag"])
        .output()
        .unwrap();
    assert!(!dup_chk.status.success());
    let dup_err = String::from_utf8_lossy(&dup_chk.stderr);
    assert!(dup_err.contains("referenced 2 times"), "{dup_err}");
    assert!(dup_err.contains("src/bag.rs:4:") && dup_err.contains("src/bag.rs:5:"), "{dup_err}");

    let thir_json = thir_with_file(dup_rs, serde_json::json!([]));
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&thir_json).unwrap()).unwrap();
    let dup_apply = Command::new(codemod_bin())
        .args(["apply", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!dup_apply.status.success());
    let dup_apply_err = String::from_utf8_lossy(&dup_apply.stderr);
    assert!(dup_apply_err.contains("referenced 2 times"), "{dup_apply_err}");
}

#[test]
fn verify_callsites_reports_verified_or_mismatches_and_missing() {
    let dir = TestDir::new("verify_cs");
    let ledger_json = r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::bag::Bag::run", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::bag::Bag::ok", "sig": "0000000000000002", "status": "live", "kind": "fn" },
    {
      "id": 10,
      "key": "fn:1@call:fn:2",
      "sig": "0000000000000010",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": 2,
        "callee_path": "demo::bag::Bag::ok",
        "anchor": "1111111111111111",
        "stmt_path": ["2222222222222222"],
        "context": ["0000000000000000", "3333333333333333"],
        "dup_rank": 0,
        "shim": "__yoake_cs_10"
      }
    },
    {
      "id": 11,
      "key": "fn:1@call:std::option::Option::is_some",
      "sig": "0000000000000011",
      "status": "live",
      "kind": "callsite",
      "site": {
        "caller": 1,
        "callee_fn": null,
        "callee_path": "std::option::Option::is_some",
        "anchor": "4444444444444444",
        "stmt_path": ["5555555555555555"],
        "context": ["0000000000000000", "6666666666666666"],
        "dup_rank": 0,
        "shim": "__yoake_cs_11"
      }
    }
  ],
  "excluded": []
}
"#;
    write_fixture_crate(dir.path(), "", ledger_json, 12);

    let ok_thir = serde_json::json!({
        "crate": "demo",
        "files": {},
        "calls": [
            {
                "file": "src/bag.rs",
                "span": [10, 5, 10, 29],
                "ctxt": 0,
                "kind": "path",
                "caller_def": "demo::bag::__yoake_cs_10",
                "callee": "demo::bag::Bag::ok",
                "ret_ty": "bool",
                "args": [],
                "resolution": "concrete"
            },
            {
                "file": "src/bag.rs",
                "span": [15, 5, 15, 45],
                "ctxt": 0,
                "kind": "path",
                "caller_def": "demo::bag::__yoake_cs_11",
                "callee": "std::option::Option::is_some",
                "ret_ty": "bool",
                "args": [],
                "resolution": "concrete"
            }
        ]
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string_pretty(&ok_thir).unwrap()).unwrap();

    let ok_out = Command::new(codemod_bin())
        .args(["verify-callsites", "--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(ok_out.status.success(), "{}", String::from_utf8_lossy(&ok_out.stderr));
    let stdout = String::from_utf8_lossy(&ok_out.stdout);
    assert!(stdout.contains("verified 2 of 2"), "{stdout}");

    let bad_thir = serde_json::json!({
        "crate": "demo",
        "files": {},
        "calls": [
            {
                "file": "src/bag.rs",
                "span": [10, 5, 10, 29],
                "ctxt": 0,
                "kind": "path",
                "caller_def": "demo::bag::__yoake_cs_10",
                "callee": "demo::bag::Bag::wrong",
                "ret_ty": "bool",
                "args": [],
                "resolution": "concrete"
            }
        ]
    });
    fs::write(&thir_path, serde_json::to_string_pretty(&bad_thir).unwrap()).unwrap();
    let bad_out = Command::new(codemod_bin())
        .args(["verify-callsites", "--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(!bad_out.status.success());
    let stderr = String::from_utf8_lossy(&bad_out.stderr);
    assert!(stderr.contains("mismatch: id=10"), "{stderr}");
    assert!(stderr.contains("missing: id=11"), "{stderr}");
}
