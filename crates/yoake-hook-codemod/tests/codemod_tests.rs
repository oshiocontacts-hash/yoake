use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

static NEXT_DIR_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let id = NEXT_DIR_ID.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "yoake_hook_codemod_test_{label}_{}_{}",
            std::process::id(),
            id
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_fixture(root: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::fs::write(&full, content).expect("write fixture file");
    }
}

fn run_codemod(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yoake-hook-codemod"))
        .args(args)
        .output()
        .expect("run yoake-hook-codemod")
}

fn read_ledger(path: &Path) -> Value {
    let text = std::fs::read_to_string(path).expect("read ledger");
    serde_json::from_str(&text).expect("parse ledger json")
}

fn entry_id(ledger: &Value, key: &str) -> Option<u64> {
    ledger["entries"]
        .as_array()?
        .iter()
        .find(|e| e["key"].as_str() == Some(key))
        .and_then(|e| e["id"].as_u64())
}

fn entry_status<'a>(ledger: &'a Value, key: &str) -> Option<&'a str> {
    ledger["entries"]
        .as_array()?
        .iter()
        .find(|e| e["key"].as_str() == Some(key))
        .and_then(|e| e["status"].as_str())
}

#[test]
fn fresh_apply_assigns_in_source_order_and_is_idempotent() {
    let tmp = TempDir::new("fresh");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo-crate\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod alpha;\n"),
            (
                "src/alpha.rs",
                "use yoake_hook_macro::yoake_hook;\n\n\
                 pub fn first(x: u32) -> u32 {\n    x + 1\n}\n\n\
                 pub struct Boxed(u32);\n\n\
                 impl Boxed {\n    pub fn second(&self) -> u32 {\n        self.0\n    }\n}\n",
            ),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    let out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo-crate",
        "--ledger",
        &ledger_s,
        "--module",
        "alpha",
    ]);
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let ledger = read_ledger(&ledger_path);
    assert_eq!(entry_id(&ledger, "demo_crate::alpha::first"), Some(1));
    assert_eq!(
        entry_id(&ledger, "demo_crate::alpha::Boxed::second"),
        Some(2)
    );
    assert_eq!(ledger["next_id"].as_u64(), Some(3));

    let after_first = std::fs::read_to_string(root.join("src/alpha.rs")).unwrap();
    assert!(after_first.contains("#[yoake_hook(id = 1, key = \"demo_crate::alpha::first\")]"));
    assert!(
        after_first
            .contains("#[yoake_hook(method, id = 2, key = \"demo_crate::alpha::Boxed::second\")]")
    );

    let out2 = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo-crate",
        "--ledger",
        &ledger_s,
        "--module",
        "alpha",
    ]);
    assert!(out2.status.success());
    assert_eq!(
        std::fs::read_to_string(root.join("src/alpha.rs")).unwrap(),
        after_first
    );

    let chk = run_codemod(&[
        "check",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo-crate",
        "--ledger",
        &ledger_s,
        "--module",
        "alpha",
    ]);
    assert!(
        chk.status.success(),
        "check should pass: {}",
        String::from_utf8_lossy(&chk.stdout)
    );
}

#[test]
fn inserting_in_middle_assigns_new_id_without_moving_existing_ids() {
    let tmp = TempDir::new("insert_middle");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod m;\n"),
            (
                "src/m.rs",
                "use yoake_hook_macro::yoake_hook;\n\npub fn a() {}\n\npub fn c() {}\n",
            ),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    assert!(
        run_codemod(&[
            "apply",
            "--crate-root",
            &root_s,
            "--crate-name",
            "demo",
            "--ledger",
            &ledger_s,
            "--module",
            "m",
        ])
        .status
        .success()
    );

    let updated = std::fs::read_to_string(root.join("src/m.rs"))
        .unwrap()
        .replace(
            "pub fn c()",
            "pub fn b() {}\n\npub fn c()",
        );
    std::fs::write(root.join("src/m.rs"), updated).unwrap();

    let out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    assert!(out.status.success());

    let ledger = read_ledger(&ledger_path);
    assert_eq!(entry_id(&ledger, "demo::m::a"), Some(1));
    assert_eq!(entry_id(&ledger, "demo::m::c"), Some(2));
    assert_eq!(entry_id(&ledger, "demo::m::b"), Some(3));
}

#[test]
fn rename_keeps_existing_id_and_records_renamed_from() {
    let tmp = TempDir::new("rename");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod m;\n"),
            (
                "src/m.rs",
                "use yoake_hook_macro::yoake_hook;\n\npub fn old_fn() {}\n",
            ),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    assert!(
        run_codemod(&[
            "apply",
            "--crate-root",
            &root_s,
            "--crate-name",
            "demo",
            "--ledger",
            &ledger_s,
            "--module",
            "m",
        ])
        .status
        .success()
    );

    let src = std::fs::read_to_string(root.join("src/m.rs"))
        .unwrap()
        .replace("pub fn old_fn()", "pub fn new_fn()");
    std::fs::write(root.join("src/m.rs"), src).unwrap();

    let out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
        "--rename",
        "demo::m::old_fn=demo::m::new_fn",
    ]);
    assert!(
        out.status.success(),
        "rename apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let ledger = read_ledger(&ledger_path);
    assert_eq!(entry_id(&ledger, "demo::m::new_fn"), Some(1));
    assert_eq!(ledger["next_id"].as_u64(), Some(2));
    let new_entry = ledger["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"].as_str() == Some("demo::m::new_fn"))
        .unwrap();
    let renamed_from = new_entry["renamed_from"].as_array().unwrap();
    assert_eq!(renamed_from[0].as_str(), Some("demo::m::old_fn"));
}

#[test]
fn removing_fn_requires_tombstone_missing_and_never_reuses_id() {
    let tmp = TempDir::new("tombstone");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod m;\n"),
            (
                "src/m.rs",
                "use yoake_hook_macro::yoake_hook;\n\npub fn keep() {}\npub fn gone() {}\n",
            ),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    assert!(
        run_codemod(&[
            "apply",
            "--crate-root",
            &root_s,
            "--crate-name",
            "demo",
            "--ledger",
            &ledger_s,
            "--module",
            "m",
        ])
        .status
        .success()
    );

    std::fs::write(
        root.join("src/m.rs"),
        "use yoake_hook_macro::yoake_hook;\n\n#[yoake_hook(id = 1, key = \"demo::m::keep\")]\npub fn keep() {}\n",
    )
    .unwrap();

    let err_out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    assert!(!err_out.status.success());
    let stderr = String::from_utf8_lossy(&err_out.stderr);
    assert!(stderr.contains("demo::m::gone"), "stderr: {stderr}");

    let ok_out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
        "--tombstone-missing",
    ]);
    assert!(ok_out.status.success());

    let ledger = read_ledger(&ledger_path);
    assert_eq!(entry_status(&ledger, "demo::m::gone"), Some("tombstone"));
    assert_eq!(entry_id(&ledger, "demo::m::gone"), Some(2));

    let with_added = format!(
        "{}\npub fn brand_new() {{}}\n",
        std::fs::read_to_string(root.join("src/m.rs")).unwrap()
    );
    std::fs::write(root.join("src/m.rs"), with_added).unwrap();

    assert!(
        run_codemod(&[
            "apply",
            "--crate-root",
            &root_s,
            "--crate-name",
            "demo",
            "--ledger",
            &ledger_s,
            "--module",
            "m",
        ])
        .status
        .success()
    );

    let ledger2 = read_ledger(&ledger_path);
    assert_eq!(entry_id(&ledger2, "demo::m::brand_new"), Some(3));
}

#[test]
fn trait_impl_method_is_converted_and_idempotent() {
    let tmp = TempDir::new("trait_conv");
    let root = tmp.path();
    let initial = "\
use yoake_hook_macro::yoake_hook;

pub trait Rules {
    fn tick(&mut self, mut step: u32) -> u32;
}

pub struct Stand {
    pub val: u32,
}

impl Rules for Stand {
    /// Doc kept on trait method.
    #[cfg_attr(not(feature = \"yoake-hook-baseline\"), yoake_hook(skip = \"old\"))]
    fn tick(&mut self, mut step: u32) -> u32 {
        step += 1;
        self.val + step
    }
}
";
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod t;\n"),
            ("src/t.rs", initial),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    let out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "t",
    ]);
    assert!(
        out.status.success(),
        "trait apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let converted = std::fs::read_to_string(root.join("src/t.rs")).unwrap();
    assert!(converted.contains(
        "fn tick(&mut self, step: u32) -> u32 {\n        self.__yoake_trait_rules_tick(step)\n    }"
    ));
    assert!(converted.contains("/// Doc kept on trait method."));
    assert!(!converted.contains("skip = \"old\""));
    assert!(converted.contains("impl Stand {"));
    assert!(converted.contains("#[inline(always)]"));
    assert!(converted.contains("key = \"demo::t::<Stand as Rules>::tick\""));
    assert!(converted.contains("fn __yoake_trait_rules_tick(&mut self, mut step: u32) -> u32 {"));

    let ledger = read_ledger(&ledger_path);
    assert_eq!(
        entry_id(&ledger, "demo::t::<Stand as Rules>::tick"),
        Some(1)
    );
    assert_eq!(ledger["next_id"].as_u64(), Some(2));

    let out2 = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "t",
    ]);
    assert!(out2.status.success());
    assert_eq!(
        std::fs::read_to_string(root.join("src/t.rs")).unwrap(),
        converted
    );

    let chk = run_codemod(&[
        "check",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "t",
    ]);
    assert!(
        chk.status.success(),
        "check failed: {}",
        String::from_utf8_lossy(&chk.stdout)
    );
}

#[test]
fn exclusions_cfg_test_and_formatting_outside_edits_are_preserved() {
    let tmp = TempDir::new("exclusions");
    let root = tmp.path();
    let header_comment = "// 日本語コメント preserved byte-for-byte   with weird spacing\r\n";
    let footer_tests = "\r\n#[cfg(test)]\r\nmod tests {\r\n    #[test]\r\n    fn test_helper() {}\r\n}\r\n";
    let src = format!(
        "{header_comment}\
use yoake_hook_macro::yoake_hook;\r\n\r\n\
pub fn ok_fn(a:   u32) -> u32 {{\r\n    a +   2\r\n}}\r\n\r\n\
#[yoake_hook(skip = \"stale\")]\r\npub fn gen_fn<T>(x: T) -> T {{ x }}\r\n\r\n\
pub fn impl_ret() -> impl Copy {{ 1u32 }}\r\n\r\n\
pub const fn const_fn() -> u32 {{ 7 }}\r\n\r\n\
pub struct Game<'a>(&'a mut u32);\r\n\
impl Game<'_> {{\r\n    pub fn step(&mut self) {{ *self.0 += 1; }}\r\n}}\r\n\
{footer_tests}"
    );
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "#[path = \"custom/sub.rs\"]\npub mod sub;\n"),
            ("src/custom/sub.rs", &src),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    let out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "sub",
    ]);
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let edited = std::fs::read_to_string(root.join("src/custom/sub.rs")).unwrap();
    assert!(edited.starts_with(header_comment));
    assert!(edited.ends_with(footer_tests));
    assert!(edited.contains("pub fn ok_fn(a:   u32) -> u32 {\r\n    a +   2\r\n}"));
    assert!(!edited.contains("skip = \"stale\""));
    assert!(edited.contains("#[yoake_hook(method, self_ty = Game<'_>, id = 2, key = \"demo::sub::Game::step\")]"));

    let ledger = read_ledger(&ledger_path);
    assert_eq!(entry_id(&ledger, "demo::sub::ok_fn"), Some(1));
    assert_eq!(entry_id(&ledger, "demo::sub::Game::step"), Some(2));
    assert_eq!(entry_id(&ledger, "demo::sub::tests::test_helper"), None);

    let excluded = ledger["excluded"].as_array().unwrap();
    let reasons: Vec<(&str, &str)> = excluded
        .iter()
        .map(|e| (e["key"].as_str().unwrap(), e["reason"].as_str().unwrap()))
        .collect();
    assert!(reasons.contains(&("demo::sub::gen_fn", "generic type params")));
    assert!(reasons.contains(&("demo::sub::impl_ret", "impl Trait return")));
    assert!(reasons.contains(&("demo::sub::const_fn", "const fn")));

    let chk = run_codemod(&[
        "check",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "sub",
    ]);
    assert!(chk.status.success());
}

#[test]
fn check_exits_nonzero_on_drift_and_excludes_trait_self_assoc_and_reports_duplicates() {
    let tmp = TempDir::new("edge_cases");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod m;\n"),
            (
                "src/m.rs",
                "use yoake_hook_macro::yoake_hook;\n\n\
                 pub trait Climbing {\n    type Error;\n    fn climb(&mut self) -> Result<bool, Self::Error>;\n}\n\
                 pub struct Stand;\n\
                 impl Climbing for Stand {\n    type Error = ();\n    fn climb(&mut self) -> Result<bool, Self::Error> { Ok(true) }\n}\n\n\
                 pub mod inner {\n    pub fn nested() {}\n}\n",
            ),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    let chk_before = run_codemod(&[
        "check",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    assert!(!chk_before.status.success(), "check before apply must fail");

    let apply = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    assert!(
        apply.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&apply.stderr)
    );

    let ledger = read_ledger(&ledger_path);
    assert_eq!(entry_id(&ledger, "demo::m::inner::nested"), Some(1));
    let excluded = ledger["excluded"].as_array().unwrap();
    assert!(excluded.iter().any(|e| {
        e["key"].as_str() == Some("demo::m::<Stand as Climbing>::climb")
            && e["reason"].as_str() == Some("uses trait Self:: item")
    }));

    let dup_src = format!(
        "{}\n#[cfg(unix)]\npub fn dup() {{}}\n#[cfg(windows)]\npub fn dup() {{}}\n",
        std::fs::read_to_string(root.join("src/m.rs")).unwrap()
    );
    std::fs::write(root.join("src/m.rs"), dup_src).unwrap();

    let dup_out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    assert!(!dup_out.status.success());
    let stderr = String::from_utf8_lossy(&dup_out.stderr);
    assert!(stderr.contains("duplicate hook key 'demo::m::dup'"), "{stderr}");
}

#[test]
fn signature_hash_survives_reformatting_with_trailing_commas_and_comments() {
    let tmp = TempDir::new("sig_invariant");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod m;\n"),
            (
                "src/m.rs",
                "use yoake_hook_macro::yoake_hook;\n\n\
                 pub fn compute<'a>(a: (u32, u64), b: Vec<u8>, c: &'a str) -> u32 {\n\
                     let _ = (a, b, c);\n\
                     0\n\
                 }\n",
            ),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    let apply = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    assert!(
        apply.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&apply.stderr)
    );

    let ledger_before = read_ledger(&ledger_path);
    let sig_before = ledger_before["entries"][0]["sig"].as_str().unwrap().to_string();

    let reformatted = "\
use yoake_hook_macro::yoake_hook;

#[yoake_hook(id = 1, key = \"demo::m::compute\")]
pub fn compute<
    'a,
    // comment in generics
>(
    /* inline comment */
    a: (
        u32,
        u64,
    ),
    b: Vec<
        u8,
    >,
    c: &'a str,
) -> u32 {
    let _ = (a, b, c);
    0
}
";
    std::fs::write(root.join("src/m.rs"), reformatted).unwrap();

    let chk = run_codemod(&[
        "check",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    let stdout = String::from_utf8_lossy(&chk.stdout);
    assert!(chk.status.success(), "check should be clean: {stdout}");
    assert!(stdout.contains("sig_changes=0"), "stdout: {stdout}");
    assert!(stdout.contains("check: clean"), "stdout: {stdout}");

    let ledger_after = read_ledger(&ledger_path);
    assert_eq!(ledger_after["entries"][0]["sig"].as_str(), Some(sig_before.as_str()));
}

#[test]
fn apply_with_rustfmt_then_check_is_clean() {
    let has_rustfmt = Command::new("rustfmt")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !has_rustfmt {
        return;
    }

    let tmp = TempDir::new("rustfmt");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod m;\n"),
            (
                "src/m.rs",
                "pub trait VeryLongTraitNameForWrapping {\n\
                     fn compute_some_very_long_method_name(&self, first_argument: u32, second_argument: u64) -> u64;\n\
                 }\n\
                 pub struct Holder { pub val: u64 }\n\
                 impl VeryLongTraitNameForWrapping for Holder {\n\
                     fn compute_some_very_long_method_name(&self, first_argument: u32, second_argument: u64) -> u64 { self.val + u64::from(first_argument) + second_argument }\n\
                 }\n",
            ),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    let apply = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
        "--rustfmt",
    ]);
    assert!(
        apply.status.success(),
        "apply --rustfmt failed: {}",
        String::from_utf8_lossy(&apply.stderr)
    );

    let fmt_check = Command::new("rustfmt")
        .args(["--check", "--edition", "2024"])
        .arg(root.join("src/m.rs"))
        .output()
        .expect("run rustfmt --check");
    assert!(
        fmt_check.status.success(),
        "rustfmt --check failed:\n{}",
        String::from_utf8_lossy(&fmt_check.stdout)
    );

    let chk = run_codemod(&[
        "check",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    assert!(
        chk.status.success(),
        "check after apply --rustfmt failed:\n{}",
        String::from_utf8_lossy(&chk.stdout)
    );
}

#[test]
fn counts_callsite_and_store_candidates_without_modifying_bodies_or_ledger() {
    let tmp = TempDir::new("candidates");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod m;\n"),
            (
                "src/m.rs",
                "use yoake_hook_macro::yoake_hook;\n\n\
                 pub struct Vec2 { pub x: f64 }\n\
                 pub struct Stand { pub velocity: f64, pub pos: Vec2 }\n\
                 impl Stand {\n\
                     pub fn first(&mut self, other: &Self) {\n\
                         let a = Helper::step(1);\n\
                         let b = self.compute(a).compute(2);\n\
                         let _ = Helper::step(b);\n\
                         self.velocity = 1.0;\n\
                         self.velocity = 2.0;\n\
                         let mut local = 0;\n\
                         local = 3;\n\
                         let _ = (other, local);\n\
                     }\n\
                     pub fn second(&mut self, other: &Stand) {\n\
                         let _ = Helper::step(0);\n\
                         let _ = other.compute(1);\n\
                         self.pos.x = 4.0;\n\
                     }\n\
                 }\n",
            ),
        ],
    );
    let ledger_path = root.join("hook-ids.lock.json");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_path.to_string_lossy().into_owned();

    let apply = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "m",
    ]);
    assert!(
        apply.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&apply.stderr)
    );

    let stdout = String::from_utf8_lossy(&apply.stdout);
    assert!(
        stdout.contains("callsite_candidates=6 (ambiguous=3), store_candidates=3"),
        "unexpected summary:\n{stdout}"
    );

    let edited = std::fs::read_to_string(root.join("src/m.rs")).unwrap();
    assert!(!edited.contains("callsite!"));
    assert!(edited.contains("let b = self.compute(a).compute(2);"));
    assert!(edited.contains("self.pos.x = 4.0;"));

    let ledger = read_ledger(&ledger_path);
    let entries = ledger["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e["kind"].as_str() == Some("fn")));
    assert!(ledger["excluded"].as_array().unwrap().is_empty());
}
