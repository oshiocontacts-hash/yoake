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

fn read_meta(ledger_dir: &Path) -> Value {
    let text = std::fs::read_to_string(ledger_dir.join("meta.json")).expect("read meta.json");
    serde_json::from_str(&text).expect("parse meta.json")
}

fn read_shard(ledger_dir: &Path, crate_ident: &str) -> Value {
    let text = std::fs::read_to_string(ledger_dir.join(format!("{crate_ident}.json")))
        .expect("read shard json");
    serde_json::from_str(&text).expect("parse shard json")
}

fn entry_id(shard: &Value, key: &str) -> Option<u64> {
    shard["entries"]
        .as_array()?
        .iter()
        .find(|e| e["key"].as_str() == Some(key))
        .and_then(|e| e["id"].as_u64())
}

fn entry_status<'a>(shard: &'a Value, key: &str) -> Option<&'a str> {
    shard["entries"]
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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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

    let meta = read_meta(&ledger_dir);
    let shard = read_shard(&ledger_dir, "demo_crate");
    assert_eq!(entry_id(&shard, "demo_crate::alpha::first"), Some(1));
    assert_eq!(
        entry_id(&shard, "demo_crate::alpha::Boxed::second"),
        Some(2)
    );
    assert_eq!(meta["next_id"].as_u64(), Some(3));

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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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

    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard, "demo::m::a"), Some(1));
    assert_eq!(entry_id(&shard, "demo::m::c"), Some(2));
    assert_eq!(entry_id(&shard, "demo::m::b"), Some(3));
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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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

    let meta = read_meta(&ledger_dir);
    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard, "demo::m::new_fn"), Some(1));
    assert_eq!(meta["next_id"].as_u64(), Some(2));
    let new_entry = shard["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["key"].as_str() == Some("demo::m::new_fn"))
        .unwrap();
    let renamed_from = new_entry["renamed_from"].as_array().unwrap();
    assert_eq!(renamed_from[0].as_str(), Some("demo::m::old_fn"));
}

#[test]
fn removing_fn_requires_tombstone_flag_and_never_reuses_id() {
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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
        "--tombstone",
        "demo::m::gone",
    ]);
    assert!(
        ok_out.status.success(),
        "tombstone apply failed: {}",
        String::from_utf8_lossy(&ok_out.stderr)
    );

    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_status(&shard, "demo::m::gone"), Some("tombstone"));
    assert_eq!(entry_id(&shard, "demo::m::gone"), Some(2));

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

    let shard2 = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard2, "demo::m::brand_new"), Some(3));
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
    #[cfg_attr(not(feature = \"yoake-hook-baseline\"), yoake_hook(id = 99, key = \"wrong::key\"))]
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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
    assert!(!converted.contains("wrong::key"));
    assert!(converted.contains("impl Stand {"));
    assert!(converted.contains("#[inline(always)]"));
    assert!(converted.contains("key = \"demo::t::<Stand as Rules>::tick\""));
    assert!(converted.contains("fn __yoake_trait_rules_tick(&mut self, mut step: u32) -> u32 {"));

    let meta = read_meta(&ledger_dir);
    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(
        entry_id(&shard, "demo::t::<Stand as Rules>::tick"),
        Some(1)
    );
    assert_eq!(meta["next_id"].as_u64(), Some(2));

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
#[yoake_hook(id = 99, key = \"stale\")]\r\npub fn gen_fn<T>(x: T) -> T {{ x }}\r\n\r\n\
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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
    assert!(!edited.contains("key = \"stale\""));
    assert!(edited.contains("#[yoake_hook(method, self_ty = Game<'_>, id = 2, key = \"demo::sub::Game::step\")]"));

    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard, "demo::sub::ok_fn"), Some(1));
    assert_eq!(entry_id(&shard, "demo::sub::Game::step"), Some(2));
    assert_eq!(entry_id(&shard, "demo::sub::tests::test_helper"), None);

    let excluded = shard["excluded"].as_array().unwrap();
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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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

    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard, "demo::m::inner::nested"), Some(1));
    let excluded = shard["excluded"].as_array().unwrap();
    assert!(excluded.iter().any(|e| {
        e["key"].as_str() == Some("demo::m::<Stand as Climbing>::climb")
            && e["reason"].as_str() == Some("uses trait Self:: item")
    }));

    let dup_src = format!(
        "{}\n#[cfg(unix)]\npub fn dup() {{}}\n#[cfg(unix)]\npub fn dup() {{}}\n",
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
    assert!(stderr.contains("duplicate hook key 'demo::m::dup[cfg(unix)]'"), "{stderr}");
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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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

    let shard_before = read_shard(&ledger_dir, "demo");
    let sig_before = shard_before["entries"][0]["sig"].as_str().unwrap().to_string();

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

    let shard_after = read_shard(&ledger_dir, "demo");
    assert_eq!(shard_after["entries"][0]["sig"].as_str(), Some(sig_before.as_str()));
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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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

    let shard = read_shard(&ledger_dir, "demo");
    let entries = shard["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    assert!(entries.iter().all(|e| e["kind"].as_str() == Some("fn")));
    assert!(shard["excluded"].as_array().unwrap().is_empty());
}

#[test]
fn manual_skip_attribute_and_ledger_manual_exclusion_survive_apply() {
    let tmp = TempDir::new("manual_skip");
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
                 #[yoake_hook(skip = \"keep out\")]\n\
                 pub fn skipped_attr() {}\n\n\
                 pub fn skipped_in_ledger() {}\n\n\
                 pub fn normal_fn() {}\n",
            ),
            (
                "hook-ledger/meta.json",
                "{\n  \"version\": 2,\n  \"next_id\": 1\n}\n",
            ),
            (
                "hook-ledger/demo.json",
                "{\n  \"crate\": \"demo\",\n  \"entries\": [],\n  \"excluded\": [\n    {\n      \"key\": \"demo::m::skipped_in_ledger\",\n      \"reason\": \"hand excluded\",\n      \"kind\": \"fn\",\n      \"manual\": true\n    }\n  ]\n}\n",
            ),
        ],
    );
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let src = std::fs::read_to_string(root.join("src/m.rs")).unwrap();
    assert!(src.contains("#[yoake_hook(skip = \"keep out\")]\npub fn skipped_attr() {}"));
    assert!(src.contains("\npub fn skipped_in_ledger() {}"));
    assert!(!src.contains("demo::m::skipped_in_ledger\""));
    assert!(src.contains("#[yoake_hook(id = 1, key = \"demo::m::normal_fn\")]"));

    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard, "demo::m::normal_fn"), Some(1));
    let excluded = shard["excluded"].as_array().unwrap();
    assert!(excluded.iter().any(|e| {
        e["key"].as_str() == Some("demo::m::skipped_attr")
            && e["reason"].as_str() == Some("keep out")
            && e["manual"].as_bool() == Some(true)
    }));
    assert!(excluded.iter().any(|e| {
        e["key"].as_str() == Some("demo::m::skipped_in_ledger")
            && e["reason"].as_str() == Some("hand excluded")
            && e["manual"].as_bool() == Some(true)
    }));

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
        "check should be clean: {}",
        String::from_utf8_lossy(&chk.stdout)
    );
}

#[test]
fn cfg_and_generic_impl_and_trait_path_disambiguators_are_distinct() {
    let tmp = TempDir::new("disambiguators");
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
                "#[cfg(unix)]\npub fn open() {}\n\
                 #[cfg(windows)]\npub fn open() {}\n\n\
                 pub struct W<T>(pub T);\n\
                 impl W<u32> {\n    pub fn get(&self) -> u32 { self.0 }\n}\n\
                 impl W<u64> {\n    pub fn get(&self) -> u64 { self.0 }\n}\n\n\
                 pub mod a { pub trait Tr { fn ping(&self); } }\n\
                 pub mod b { pub trait Tr { fn ping(&self); } }\n\
                 pub struct Holder;\n\
                 impl a::Tr for Holder {\n    fn ping(&self) {}\n}\n\
                 impl b::Tr for Holder {\n    fn ping(&self) {}\n}\n",
            ),
        ],
    );
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard, "demo::m::open[cfg(unix)]"), Some(1));
    assert_eq!(entry_id(&shard, "demo::m::open[cfg(windows)]"), Some(2));
    assert_eq!(entry_id(&shard, "demo::m::W<u32>::get"), Some(3));
    assert_eq!(entry_id(&shard, "demo::m::W<u64>::get"), Some(4));
    assert_eq!(entry_id(&shard, "demo::m::<Holder as a::Tr>::ping"), Some(5));
    assert_eq!(entry_id(&shard, "demo::m::<Holder as b::Tr>::ping"), Some(6));

    let src = std::fs::read_to_string(root.join("src/m.rs")).unwrap();
    assert!(src.contains("__yoake_trait_a_tr_ping"));
    assert!(src.contains("__yoake_trait_b_tr_ping"));

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
        "check failed: {}",
        String::from_utf8_lossy(&chk.stdout)
    );
}

#[test]
fn non_inherent_trait_self_types_and_body_changing_attrs_are_excluded() {
    let tmp = TempDir::new("sf5_exclusions");
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
                "pub trait Tr { fn act(&self); }\n\
                 pub struct Foo;\n\n\
                 impl Tr for &Foo {\n    fn act(&self) {}\n}\n\
                 impl Tr for (Foo, Foo) {\n    fn act(&self) {}\n}\n\
                 impl Tr for u32 {\n    fn act(&self) {}\n}\n\
                 impl Tr for Foo {\n    #[custom_body_attr]\n    fn act(&self) {}\n}\n",
            ),
        ],
    );
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let shard = read_shard(&ledger_dir, "demo");
    assert!(shard["entries"].as_array().unwrap().is_empty());
    let excluded = shard["excluded"].as_array().unwrap();
    let reasons: Vec<(&str, &str)> = excluded
        .iter()
        .map(|e| (e["key"].as_str().unwrap(), e["reason"].as_str().unwrap()))
        .collect();
    assert!(reasons.contains(&(
        "demo::m::<&Foo as Tr>::act",
        "trait impl on a type that cannot take an inherent impl"
    )));
    assert!(reasons.contains(&(
        "demo::m::<(Foo,Foo) as Tr>::act",
        "trait impl on a type that cannot take an inherent impl"
    )));
    assert!(reasons.contains(&(
        "demo::m::<u32 as Tr>::act",
        "trait impl on a type that cannot take an inherent impl"
    )));
    assert!(reasons.contains(&("demo::m::<Foo as Tr>::act", "attribute may change the body")));
}

#[test]
fn private_self_type_is_excluded_while_pub_crate_is_hooked() {
    let tmp = TempDir::new("sf6_visibility");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub(crate) mod m;\n"),
            (
                "src/m.rs",
                "pub trait Tr { fn work(&self); }\n\
                 struct Secret;\n\
                 impl Secret {\n    pub fn hidden(&self) {}\n}\n\
                 impl Tr for Secret {\n    fn work(&self) {}\n}\n\
                 pub(crate) struct Visible;\n\
                 impl Visible {\n    pub fn shown(&self) {}\n}\n",
            ),
        ],
    );
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard, "demo::m::Visible::shown"), Some(1));
    assert_eq!(entry_id(&shard, "demo::m::Secret::hidden"), None);
    let excluded = shard["excluded"].as_array().unwrap();
    let reasons: Vec<(&str, &str)> = excluded
        .iter()
        .map(|e| (e["key"].as_str().unwrap(), e["reason"].as_str().unwrap()))
        .collect();
    assert!(reasons.contains(&("demo::m::Secret::hidden", "self type has no crate-level path")));
    assert!(reasons.contains(&("demo::m::<Secret as Tr>::work", "self type has no crate-level path")));
}

#[test]
fn check_history_accepts_rename_and_rejects_untracked_key_change() {
    let tmp = TempDir::new("check_history");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "base/meta.json",
                "{\n  \"version\": 2,\n  \"next_id\": 2\n}\n",
            ),
            (
                "base/demo.json",
                "{\n  \"crate\": \"demo\",\n  \"entries\": [\n    {\n      \"id\": 1,\n      \"key\": \"demo::m::old_name\",\n      \"sig\": \"0123456789abcdef\",\n      \"status\": \"live\",\n      \"kind\": \"fn\"\n    }\n  ]\n}\n",
            ),
            (
                "renamed/meta.json",
                "{\n  \"version\": 2,\n  \"next_id\": 2\n}\n",
            ),
            (
                "renamed/demo.json",
                "{\n  \"crate\": \"demo\",\n  \"entries\": [\n    {\n      \"id\": 1,\n      \"key\": \"demo::m::new_name\",\n      \"sig\": \"0123456789abcdef\",\n      \"status\": \"live\",\n      \"kind\": \"fn\",\n      \"renamed_from\": [\"demo::m::old_name\"]\n    }\n  ]\n}\n",
            ),
            (
                "bad/meta.json",
                "{\n  \"version\": 2,\n  \"next_id\": 2\n}\n",
            ),
            (
                "bad/demo.json",
                "{\n  \"crate\": \"demo\",\n  \"entries\": [\n    {\n      \"id\": 1,\n      \"key\": \"demo::m::new_name\",\n      \"sig\": \"0123456789abcdef\",\n      \"status\": \"live\",\n      \"kind\": \"fn\"\n    }\n  ]\n}\n",
            ),
        ],
    );

    let base_s = root.join("base").to_string_lossy().into_owned();
    let renamed_s = root.join("renamed").to_string_lossy().into_owned();
    let bad_s = root.join("bad").to_string_lossy().into_owned();

    let ok_out = run_codemod(&[
        "check-history",
        "--base",
        &base_s,
        "--current",
        &renamed_s,
    ]);
    assert!(
        ok_out.status.success(),
        "check-history should accept rename: {}",
        String::from_utf8_lossy(&ok_out.stderr)
    );

    let bad_out = run_codemod(&[
        "check-history",
        "--base",
        &base_s,
        "--current",
        &bad_s,
    ]);
    assert!(!bad_out.status.success(), "check-history should reject untracked key change");
}

#[test]
fn atomic_apply_rolls_back_and_leaves_no_tmp_files_on_failure() {
    let tmp = TempDir::new("atomic_apply");
    let root = tmp.path();
    let orig_a = "pub fn a() {}\n";
    let orig_b = "pub fn b() {}\n";
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod a;\npub mod b;\n"),
            ("src/a.rs", orig_a),
            ("src/b.rs", orig_b),
        ],
    );

    let b_path = root.join("src/b.rs");
    let mut perms = std::fs::metadata(&b_path).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&b_path, perms).unwrap();

    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

    let out = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "a",
        "--module",
        "b",
    ]);

    let mut restore_perms = std::fs::metadata(&b_path).unwrap().permissions();
    restore_perms.set_readonly(false);
    let _ = std::fs::set_permissions(&b_path, restore_perms);

    assert!(!out.status.success(), "apply with read-only target should fail");
    assert_eq!(std::fs::read_to_string(root.join("src/a.rs")).unwrap(), orig_a);
    assert_eq!(std::fs::read_to_string(&b_path).unwrap(), orig_b);
    assert!(!ledger_dir.exists(), "ledger dir should not be created on failure");
    for entry in std::fs::read_dir(root.join("src")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.ends_with(".yoake-tmp"), "leftover tmp file: {name}");
    }

    // Invalid second file fails before touching first file or ledger
    std::fs::write(&b_path, "pub fn broken( {}").unwrap();
    let out2 = run_codemod(&[
        "apply",
        "--crate-root",
        &root_s,
        "--crate-name",
        "demo",
        "--ledger",
        &ledger_s,
        "--module",
        "a",
        "--module",
        "b",
    ]);
    assert!(!out2.status.success(), "apply with invalid second file should fail");
    assert_eq!(std::fs::read_to_string(root.join("src/a.rs")).unwrap(), orig_a);
    assert!(!ledger_dir.exists());
    for entry in std::fs::read_dir(root.join("src")).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        assert!(!name.ends_with(".yoake-tmp"), "leftover tmp file: {name}");
    }
}

#[test]
fn cfg_test_only_skips_test_code_and_test_id_in_engine_module_is_error() {
    let tmp = TempDir::new("n2_cfg_test");
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
                "pub mod tests {\n    pub fn non_cfg_test_fn() {}\n}\n\n\
                 #[cfg(test)]\nmod real_tests {\n    #[yoake_hook(test_id = 0)]\n    fn ok_in_cfg_test() {}\n}\n",
            ),
        ],
    );
    let ledger_dir = root.join("hook-ledger");
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = ledger_dir.to_string_lossy().into_owned();

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
    assert!(
        out.status.success(),
        "apply failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let shard = read_shard(&ledger_dir, "demo");
    assert_eq!(entry_id(&shard, "demo::m::tests::non_cfg_test_fn"), Some(1));
    assert_eq!(entry_id(&shard, "demo::m::real_tests::ok_in_cfg_test"), None);

    let with_bad_test_id = format!(
        "{}\n#[yoake_hook(test_id = 1)]\npub fn bad_engine_test_id() {{}}\n",
        std::fs::read_to_string(root.join("src/m.rs")).unwrap()
    );
    std::fs::write(root.join("src/m.rs"), with_bad_test_id).unwrap();

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
    assert!(!chk.status.success(), "check must fail on test_id in engine module");
    let stderr = String::from_utf8_lossy(&chk.stderr);
    assert!(stderr.contains("test_id"), "stderr: {stderr}");
}

fn w5_fixture(tmp: &TempDir) -> (String, String) {
    let root = tmp.path();
    write_fixture(
        root,
        &[
            (
                "Cargo.toml",
                "[package]\nname = \"demo-crate\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
            ),
            ("src/lib.rs", "pub mod alpha;\npub mod beta;\n#[cfg(test)]\nmod tests;\n"),
            (
                "src/alpha.rs",
                "pub fn tiny(x: u32) -> u32 {\n    x + 1\n}\n\n\
                 pub fn big(x: u32) -> u32 {\n    let a = x + 1;\n    let b = a * 2;\n    let c = b - 1;\n    c + a\n}\n",
            ),
            ("src/beta.rs", "pub fn other(x: u32) -> u32 {\n    let a = x;\n    let b = a;\n    let c = b;\n    c + 1\n}\n"),
        ],
    );
    (root.to_string_lossy().into_owned(), root.join("hook-ledger").to_string_lossy().into_owned())
}

#[test]
fn star_module_covers_every_top_level_module_and_exclude_module_skips_one() {
    let tmp = TempDir::new("w5star");
    let (root_s, ledger_s) = w5_fixture(&tmp);
    let out = run_codemod(&[
        "apply", "--crate-root", &root_s, "--crate-name", "demo-crate", "--ledger", &ledger_s,
        "--module", "*", "--exclude-module", "beta",
    ]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shard = read_shard(tmp.path().join("hook-ledger").as_path(), "demo_crate");
    assert!(entry_id(&shard, "demo_crate::alpha::tiny").is_some());
    assert!(entry_id(&shard, "demo_crate::alpha::big").is_some());
    assert!(entry_id(&shard, "demo_crate::beta::other").is_none(), "beta is excluded");
}

#[test]
fn min_stmts_excludes_small_bodies_except_allow_listed_and_already_live_keys() {
    let tmp = TempDir::new("w5size");
    let (root_s, ledger_s) = w5_fixture(&tmp);
    let ledger_dir = tmp.path().join("hook-ledger");
    let args = |extra: &[&'static str]| {
        let mut v: Vec<String> = ["apply", "--crate-root", &root_s, "--crate-name", "demo-crate", "--ledger", &ledger_s, "--module", "alpha", "--min-stmts", "3"]
            .iter().map(|s| s.to_string()).collect();
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    };
    let run = |a: Vec<String>| {
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        run_codemod(&refs)
    };
    let out = run(args(&[]));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shard = read_shard(&ledger_dir, "demo_crate");
    assert!(entry_id(&shard, "demo_crate::alpha::tiny").is_none(), "1 statement is under the size rule");
    assert!(entry_id(&shard, "demo_crate::alpha::big").is_some(), "4 statements are stamped");

    // allow list: the small fn is stamped on the next run and stays stamped without the list entry
    std::fs::write(ledger_dir.join("stamp-allow.txt"), "demo_crate::alpha::tiny # a mod needs it\n").unwrap();
    let out = run(args(&[]));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shard = read_shard(&ledger_dir, "demo_crate");
    assert!(entry_id(&shard, "demo_crate::alpha::tiny").is_some());
    std::fs::remove_file(ledger_dir.join("stamp-allow.txt")).unwrap();
    let out = run(args(&[]));
    assert!(out.status.success(), "a live hook is never retired by the size rule: {}", String::from_utf8_lossy(&out.stderr));
    let shard = read_shard(&ledger_dir, "demo_crate");
    assert_eq!(entry_status(&shard, "demo_crate::alpha::tiny"), Some("live"));
}

#[test]
fn stamp_config_supplies_modules_exclusions_and_size_rule() {
    let tmp = TempDir::new("w5cfg");
    let (root_s, ledger_s) = w5_fixture(&tmp);
    let cfg = tmp.path().join("stamp-config.txt");
    std::fs::write(&cfg, format!("# comment\n{root_s} demo-crate * exclude=beta min_stmts=3\n")).unwrap();
    let cfg_s = cfg.to_string_lossy().into_owned();
    let out = run_codemod(&[
        "apply", "--crate-root", &root_s, "--crate-name", "demo-crate", "--ledger", &ledger_s,
        "--stamp-config", &cfg_s,
    ]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shard = read_shard(tmp.path().join("hook-ledger").as_path(), "demo_crate");
    assert!(entry_id(&shard, "demo_crate::alpha::big").is_some());
    assert!(entry_id(&shard, "demo_crate::alpha::tiny").is_none());
    assert!(entry_id(&shard, "demo_crate::beta::other").is_none());
}

#[test]
fn private_types_flag_hooks_private_self_types_and_destructuring_params() {
    let tmp = TempDir::new("w4");
    let root = tmp.path();
    write_fixture(
        root,
        &[
            ("Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
            ("src/lib.rs", "pub(crate) mod m;\n"),
            (
                "src/m.rs",
                "pub trait Tr { fn work(&self, (a, b): (u32, u32), _: u8) -> u32; }\n\
                 struct Secret(u32);\n\
                 impl Secret {\n    pub fn hidden(&self, (a, b): (u32, u32), _: u8) -> u32 {\n        self.0 + a + b\n    }\n}\n\
                 impl Tr for Secret {\n    fn work(&self, (a, b): (u32, u32), _: u8) -> u32 {\n        self.0 + a + b\n    }\n}\n",
            ),
        ],
    );
    let root_s = root.to_string_lossy().into_owned();
    let ledger_s = root.join("hook-ledger").to_string_lossy().into_owned();
    let args = |extra: &[&str]| {
        let mut v: Vec<String> = ["apply", "--crate-root", &root_s, "--crate-name", "demo", "--ledger", &ledger_s, "--module", "m"]
            .iter().map(|s| s.to_string()).collect();
        v.extend(extra.iter().map(|s| s.to_string()));
        v
    };
    let run = |a: Vec<String>| {
        let refs: Vec<&str> = a.iter().map(String::as_str).collect();
        run_codemod(&refs)
    };
    let out = run(args(&[]));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shard = read_shard(&root.join("hook-ledger"), "demo");
    assert!(entry_id(&shard, "demo::m::Secret::hidden").is_none(), "default: no crate-level path, excluded");

    let out = run(args(&["--private-types"]));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let shard = read_shard(&root.join("hook-ledger"), "demo");
    assert!(entry_id(&shard, "demo::m::Secret::hidden").is_some(), "private type hooked; destructuring param no longer excludes it");
    assert!(entry_id(&shard, "demo::m::<Secret as Tr>::work").is_some(), "trait impl on a private type converted");
    let converted = std::fs::read_to_string(root.join("src/m.rs")).unwrap();
    assert!(converted.contains("__yoake_a0") && converted.contains("__yoake_a1"), "wrapper names its patterns: {converted}");
    assert!(converted.contains("(a, b): (u32, u32)"), "the helper keeps the pattern: {converted}");
    // idempotent: the converted wrapper is recognised on the next run
    let out = run(args(&["--private-types"]));
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(std::fs::read_to_string(root.join("src/m.rs")).unwrap(), converted);
}
