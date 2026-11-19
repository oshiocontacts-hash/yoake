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
            "yoake_codemod_shallow_test_{label}_{}_{}",
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

fn byte_to_line_col(src: &str, offset: usize) -> (usize, usize) {
    let prefix = &src[..offset];
    let line = prefix.bytes().filter(|&b| b == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |p| p + 1);
    let col = prefix[line_start..].chars().count() + 1;
    (line, col)
}

fn span_of(src: &str, needle: &str) -> [usize; 4] {
    let start_byte = src
        .find(needle)
        .unwrap_or_else(|| panic!("missing {needle:?} in:\n{src}"));
    let end_byte = start_byte + needle.len();
    let (sl, sc) = byte_to_line_col(src, start_byte);
    let (el, ec) = byte_to_line_col(src, end_byte);
    [sl, sc, el, ec]
}

const M_RS: &str = r#"pub mod child;

pub struct Actor {
    pub hp: i32,
}

pub fn own_fn(a: &mut Actor) {
    a.hp += 1;
}

pub mod inner {
    pub fn inner_fn(a: &mut super::Actor) {
        a.hp += 2;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ignored() {}
}

include!("yoake_glue/m.rs");
"#;

const CHILD_RS: &str = r#"pub fn child_fn(a: &mut super::Actor) {
    a.hp += 3;
}
"#;

fn setup_fixture(root: &Path) {
    fs::create_dir_all(root.join("src/m")).unwrap();
    fs::create_dir_all(root.join("src/yoake_glue")).unwrap();
    fs::create_dir_all(root.join("ledger")).unwrap();

    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub mod m;\n").unwrap();
    fs::write(root.join("src/m.rs"), M_RS).unwrap();
    fs::write(root.join("src/m/child.rs"), CHILD_RS).unwrap();
    fs::write(
        root.join("src/yoake_glue/m.rs"),
        r#"pub mod yoake_glue {
    use yoake_slice::activation::FieldTable;
    pub static FIELDS_crate__m__Actor: FieldTable<crate::m::Actor> = FieldTable {
        get: |_, _| None,
        set: |_, _, _| false,
    };
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("ledger/meta.json"),
        "{\n  \"version\": 2,\n  \"next_id\": 1\n}\n",
    )
    .unwrap();
}

fn live_keys(ledger_dir: &Path) -> Vec<String> {
    let text = fs::read_to_string(ledger_dir.join("demo.json")).unwrap();
    let val: serde_json::Value = serde_json::from_str(&text).unwrap();
    val["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["status"] == "live")
        .map(|e| e["key"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn shallow_apply_check_and_bridge_instrument_only_target_own_items() {
    let dir = TestDir::new("only_own");
    setup_fixture(dir.path());
    let ledger_dir = dir.path().join("ledger");
    let glue_path = dir.path().join("src/yoake_glue/m.rs");
    let bridge_path = dir.path().join("src/yoake_glue/hook_bridge_m.rs");

    let apply_out = Command::new(codemod_bin())
        .args(["apply", "--shallow", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m"])
        .output()
        .unwrap();
    assert!(
        apply_out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&apply_out.stdout),
        String::from_utf8_lossy(&apply_out.stderr)
    );

    let m_after = fs::read_to_string(dir.path().join("src/m.rs")).unwrap();
    assert!(
        m_after.contains("#[yoake_hook_macro::yoake_hook(id = 1, key = \"demo::m::own_fn\")]"),
        "{m_after}"
    );
    assert_eq!(m_after.matches("yoake_hook(").count(), 1, "{m_after}");
    assert_eq!(
        fs::read_to_string(dir.path().join("src/m/child.rs")).unwrap(),
        CHILD_RS
    );
    assert_eq!(live_keys(&ledger_dir), vec!["demo::m::own_fn"]);

    // `check --shallow` passes; non-shallow `check` sees uninstrumented inner/child fns.
    let chk_shallow = Command::new(codemod_bin())
        .args(["check", "--shallow", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m"])
        .output()
        .unwrap();
    assert!(
        chk_shallow.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&chk_shallow.stdout),
        String::from_utf8_lossy(&chk_shallow.stderr)
    );

    let chk_deep = Command::new(codemod_bin())
        .args(["check", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m"])
        .output()
        .unwrap();
    assert_eq!(chk_deep.status.code(), Some(1));

    // `bridge --shallow` bridges only id 1 and `bridge --check --shallow` agrees.
    let bridge_out = Command::new(codemod_bin())
        .args(["bridge", "--shallow", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(
        bridge_out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&bridge_out.stdout),
        String::from_utf8_lossy(&bridge_out.stderr)
    );
    let bridge_src = fs::read_to_string(&bridge_path).unwrap();
    assert!(
        bridge_src.contains("pub const BRIDGED: &[u32] = &[\n        1,\n    ];"),
        "{bridge_src}"
    );
    assert!(bridge_src.contains("fn __bridge_1("), "{bridge_src}");
    assert!(
        bridge_src.contains("super::__yoake_register_own_fn(cb, order)"),
        "{bridge_src}"
    );

    let bridge_chk = Command::new(codemod_bin())
        .args(["bridge", "--check", "--shallow", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(
        bridge_chk.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&bridge_chk.stdout),
        String::from_utf8_lossy(&bridge_chk.stderr)
    );
}

#[test]
fn non_shallow_instruments_all_three_and_shallow_bridge_and_check_still_scope_to_m() {
    let dir = TestDir::new("deep_vs_shallow");
    setup_fixture(dir.path());
    let ledger_dir = dir.path().join("ledger");
    let glue_path = dir.path().join("src/yoake_glue/m.rs");
    let bridge_path = dir.path().join("src/yoake_glue/hook_bridge_m.rs");

    let apply_out = Command::new(codemod_bin())
        .args(["apply", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m"])
        .output()
        .unwrap();
    assert!(
        apply_out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&apply_out.stdout),
        String::from_utf8_lossy(&apply_out.stderr)
    );

    let m_after = fs::read_to_string(dir.path().join("src/m.rs")).unwrap();
    let child_after = fs::read_to_string(dir.path().join("src/m/child.rs")).unwrap();
    assert!(m_after.contains("key = \"demo::m::own_fn\""), "{m_after}");
    assert!(
        m_after.contains("key = \"demo::m::inner::inner_fn\""),
        "{m_after}"
    );
    assert_eq!(m_after.matches("yoake_hook(").count(), 2, "{m_after}");
    assert!(
        child_after.contains("key = \"demo::m::child::child_fn\""),
        "{child_after}"
    );
    assert_eq!(
        live_keys(&ledger_dir),
        vec![
            "demo::m::own_fn",
            "demo::m::inner::inner_fn",
            "demo::m::child::child_fn",
        ]
    );

    // Both `check` and `check --shallow` succeed: `--shallow` must not report child/inner keys as missing.
    for extra in [&[][..], &["--shallow"][..]] {
        let chk = Command::new(codemod_bin())
            .arg("check")
            .args(extra)
            .arg("--crate-root")
            .arg(dir.path())
            .args(["--crate-name", "demo", "--ledger"])
            .arg(&ledger_dir)
            .args(["--module", "m"])
            .output()
            .unwrap();
        assert!(
            chk.status.success(),
            "extra={extra:?}\n{}\n{}",
            String::from_utf8_lossy(&chk.stdout),
            String::from_utf8_lossy(&chk.stderr)
        );
    }

    // With all 3 entries live in the ledger, `bridge --shallow` emits only id 1, while `bridge` emits all 3.
    let bridge_shallow = Command::new(codemod_bin())
        .args(["bridge", "--shallow", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(
        bridge_shallow.status.success(),
        "{}",
        String::from_utf8_lossy(&bridge_shallow.stderr)
    );
    let shallow_src = fs::read_to_string(&bridge_path).unwrap();
    assert!(
        shallow_src.contains("pub const BRIDGED: &[u32] = &[\n        1,\n    ];"),
        "{shallow_src}"
    );
    assert!(!shallow_src.contains("fn __bridge_2("), "{shallow_src}");
    assert!(!shallow_src.contains("fn __bridge_3("), "{shallow_src}");

    let bridge_deep = Command::new(codemod_bin())
        .args(["bridge", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(
        bridge_deep.status.success(),
        "{}",
        String::from_utf8_lossy(&bridge_deep.stderr)
    );
    let deep_src = fs::read_to_string(&bridge_path).unwrap();
    assert!(
        deep_src.contains("pub const BRIDGED: &[u32] = &[\n        1,\n        2,\n        3,\n    ];"),
        "{deep_src}"
    );

    let bridge_chk_deep = Command::new(codemod_bin())
        .args(["bridge", "--check", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(bridge_chk_deep.status.success());

    let bridge_chk_shallow = Command::new(codemod_bin())
        .args(["bridge", "--check", "--shallow", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "m", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert_eq!(bridge_chk_shallow.status.code(), Some(1));
}

#[test]
fn shallow_skips_loading_child_files_and_supports_inline_target_module() {
    let dir = TestDir::new("missing_child_and_inline");
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::create_dir_all(dir.path().join("ledger")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("src/lib.rs"),
        r#"pub mod m;
pub mod inline_m {
    pub fn own_inline() {}
    pub mod inner {
        pub fn inner_inline() {}
    }
}
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("src/m.rs"),
        "pub mod missing_child;\npub fn own_fn() {}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("ledger/meta.json"),
        "{\n  \"version\": 2,\n  \"next_id\": 1\n}\n",
    )
    .unwrap();

    // Without `--shallow`, `m` fails trying to load `m/missing_child.rs`.
    let deep_fail = Command::new(codemod_bin())
        .args(["check", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "m"])
        .output()
        .unwrap();
    assert_eq!(deep_fail.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&deep_fail.stderr).contains("missing_child"),
        "{}",
        String::from_utf8_lossy(&deep_fail.stderr)
    );

    // With `--shallow`, `missing_child` is never loaded and inline `inline_m` skips `inner`.
    let apply_ok = Command::new(codemod_bin())
        .args(["apply", "--shallow", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "m", "--module", "inline_m"])
        .output()
        .unwrap();
    assert!(
        apply_ok.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&apply_ok.stdout),
        String::from_utf8_lossy(&apply_ok.stderr)
    );
    assert_eq!(
        live_keys(&dir.path().join("ledger")),
        vec!["demo::inline_m::own_inline", "demo::m::own_fn"]
    );
}

#[test]
fn shallow_callsites_rewrites_only_target_module_own_fns() {
    let dir = TestDir::new("callsites");
    fs::create_dir_all(dir.path().join("src/m")).unwrap();
    fs::create_dir_all(dir.path().join("ledger")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub mod m;\n").unwrap();
    fs::write(
        dir.path().join("src/m.rs"),
        r#"pub mod child;

pub struct Actor {
    pub hp: i32,
}

impl Actor {
    pub fn bump(&mut self, delta: i32) {
        self.hp += delta;
    }
}

pub fn own_fn(a: &mut Actor) {
    a.bump(1);
}

pub mod inner {
    pub fn inner_fn(a: &mut super::Actor) {
        a.bump(2);
    }
}
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("src/m/child.rs"),
        r#"pub fn child_fn(a: &mut super::Actor) {
    a.bump(3);
}
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("ledger/meta.json"),
        "{\n  \"version\": 2,\n  \"next_id\": 1\n}\n",
    )
    .unwrap();

    let fn_apply = Command::new(codemod_bin())
        .args(["apply", "--shallow", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "m"])
        .output()
        .unwrap();
    assert!(
        fn_apply.status.success(),
        "{}",
        String::from_utf8_lossy(&fn_apply.stderr)
    );

    let m_src = fs::read_to_string(dir.path().join("src/m.rs")).unwrap();
    let own_call_span = span_of(&m_src, "a.bump(1)");
    let inner_call_span = span_of(&m_src, "a.bump(2)");
    let thir = serde_json::json!({
        "crate": "demo",
        "files": {
            "src/m.rs": {
                "written_calls": 2,
                "sha": fnv1a_hex(m_src.as_bytes()),
                "len": m_src.len()
            }
        },
        "calls": [
            {
                "file": "src/m.rs",
                "span": own_call_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::m::Actor::bump",
                "callee_def": "m::{impl#0}::bump",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ mut crate::m::Actor" }, { "ty": "i32" } ],
                "recv_adjust": { "borrow": "mut", "derefs": 1 },
                "resolution": "concrete"
            },
            {
                "file": "src/m.rs",
                "span": inner_call_span,
                "ctxt": 0,
                "kind": "method",
                "callee": "demo::m::Actor::bump",
                "callee_def": "m::{impl#0}::bump",
                "ret_ty": "()",
                "args": [ { "ty": "&'_ mut crate::m::Actor" }, { "ty": "i32" } ],
                "recv_adjust": { "borrow": "mut", "derefs": 1 },
                "resolution": "concrete"
            }
        ]
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string(&thir).unwrap()).unwrap();

    let cs_apply = Command::new(codemod_bin())
        .args(["apply", "--shallow", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "m", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(
        cs_apply.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&cs_apply.stdout),
        String::from_utf8_lossy(&cs_apply.stderr)
    );

    let m_after_cs = fs::read_to_string(dir.path().join("src/m.rs")).unwrap();
    assert!(m_after_cs.contains("__yoake_cs_3(&mut *a, 1);"), "{m_after_cs}");
    assert!(m_after_cs.contains("a.bump(2);"), "{m_after_cs}");
    assert!(
        fs::read_to_string(dir.path().join("src/m/child.rs"))
            .unwrap()
            .contains("a.bump(3);")
    );
    assert_eq!(
        live_keys(&dir.path().join("ledger")),
        vec![
            "demo::m::Actor::bump",
            "demo::m::own_fn",
            "fn:2@call:fn:1",
        ]
    );

    let cs_check = Command::new(codemod_bin())
        .args(["check", "--shallow", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "m"])
        .output()
        .unwrap();
    assert!(
        cs_check.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&cs_check.stdout),
        String::from_utf8_lossy(&cs_check.stderr)
    );
}
