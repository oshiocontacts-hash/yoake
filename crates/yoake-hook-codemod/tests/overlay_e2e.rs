//! W7 end to end: a throwaway workspace built plain and through the rustc wrapper overlay.
//! Slow (compiles the hook crates): `cargo test -p yoake-hook-codemod --test overlay_e2e -- --ignored --nocapture`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn fwd(p: &Path) -> String {
    p.to_string_lossy().trim_start_matches("\\\\?\\").replace('\\', "/")
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn fixture(dir: &Path, with_conflict: bool) {
    let repo = fwd(&repo_root());
    write(
        &dir.join("Cargo.toml"),
        "[workspace]\nresolver = \"3\"\nmembers = [\"crates/demo\", \"crates/other\", \"crates/app\"]\n",
    );
    write(&dir.join("hook-ledger/stamp-config.txt"), "crates/demo demo * min_stmts=1\n");
    write(&dir.join("hook-ledger/meta.json"), "{\n  \"version\": 2,\n  \"next_id\": 1\n}\n");
    write(&dir.join("hook-ledger/demo.json"), "{\n  \"crate\": \"demo\",\n  \"entries\": [],\n  \"excluded\": []\n}\n");
    write(
        &dir.join("crates/demo/Cargo.toml"),
        &format!(
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[build-dependencies]\nyoake-hook-ledger = {{ path = \"{repo}/crates/yoake-hook-ledger\" }}\n\n[dependencies]\nyoake-hook-macro = {{ path = \"{repo}/crates/yoake-hook-macro\" }}\nyoake-hook-rt = {{ path = \"{repo}/crates/yoake-hook-rt\" }}\n"
        ),
    );
    write(&dir.join("crates/demo/build.rs"), "fn main() { yoake_hook_ledger::build::emit_crate_table(\"demo\"); }\n");
    write(
        &dir.join("crates/demo/src/lib.rs"),
        "pub mod a;\n#[doc(hidden)]\npub mod __yoake_hook_keys {\n    include!(concat!(env!(\"OUT_DIR\"), \"/yoake_hook_keys.rs\"));\n}\n",
    );
    write(
        &dir.join("crates/demo/src/a.rs"),
        "pub fn where_am_i() -> &'static str {\n    let a = file!();\n    let b = a;\n    b\n}\n\npub fn boom_line() -> u32 {\n    let r = std::panic::catch_unwind(|| {\n        panic!(\"boom\");\n    });\n    let _ = r;\n    line!()\n}\n\npub struct Counter(pub u32);\n\nimpl Counter {\n    pub fn bump(&mut self, by: u32) -> u32 {\n        let before = self.0;\n        self.0 += by;\n        before + by\n    }\n}\n",
    );
    if with_conflict {
        // compiles unstamped; stamped, the generated `__yoake_orig_foo` is a second definition of this one
        let a = std::fs::read_to_string(dir.join("crates/demo/src/a.rs")).unwrap();
        write(
            &dir.join("crates/demo/src/a.rs"),
            &format!("{a}
impl Counter {{
    #[allow(dead_code)]
    fn __yoake_orig_foo(&self) -> u32 {{
        0
    }}

    pub fn foo(&self) -> u32 {{
        let a = self.0;
        let b = a;
        a + b
    }}
}}
"),
        );
    }
    write(&dir.join("crates/other/Cargo.toml"), "[package]\nname = \"other\"\nversion = \"0.1.0\"\nedition = \"2024\"\n");
    write(&dir.join("crates/other/src/lib.rs"), "pub fn n() -> u32 {\n    1\n}\n");
    write(
        &dir.join("crates/app/Cargo.toml"),
        "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\ndemo = { path = \"../demo\" }\nother = { path = \"../other\" }\n",
    );
    write(
        &dir.join("crates/app/src/main.rs"),
        "use std::sync::Mutex;\nstatic LOC: Mutex<String> = Mutex::new(String::new());\nfn main() {\n    std::panic::set_hook(Box::new(|info| {\n        *LOC.lock().unwrap() = info.location().map(|l| l.file().to_string()).unwrap_or_default();\n    }));\n    let _ = demo::a::boom_line();\n    println!(\"file={}\", demo::a::where_am_i());\n    println!(\"panic_file={}\", LOC.lock().unwrap());\n    println!(\"bump={}\", demo::a::Counter(1).bump(2));\n    println!(\"other={}\", other::n());\n}\n",
    );
}

fn cargo(dir: &Path, target: &Path, stamped: bool, args: &[&str]) -> Output {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(dir)
        .args(args)
        .env("CARGO_TARGET_DIR", target)
        .env("CARGO_INCREMENTAL", "0")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("YOAKE_STAMP");
    if stamped {
        cmd.env("RUSTC_WORKSPACE_WRAPPER", env!("CARGO_BIN_EXE_yoake-hook-codemod"))
            .env("YOAKE_STAMP", "1")
            .env("YOAKE_STAMP_DIR", target.join("yoake-stamp"));
    }
    cmd.output().expect("run cargo")
}

fn ok(out: &Output) -> String {
    let text = format!("{}\n{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(out.status.success(), "{text}");
    text
}

fn run_app(target: &Path) -> BTreeMap<String, String> {
    let exe = target.join("debug").join(if cfg!(windows) { "app.exe" } else { "app" });
    let out = Command::new(exe).output().expect("run app");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('=').map(|(k, v)| (k.to_string(), v.trim().to_string())))
        .collect()
}

fn tree_hash(dir: &Path) -> BTreeMap<String, u64> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, u64>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == ".state") {
                    continue;
                }
                walk(base, &p, out);
            } else {
                let bytes = std::fs::read(&p).unwrap();
                let mut h: u64 = 0xcbf29ce484222325;
                for b in bytes {
                    h ^= u64::from(b);
                    h = h.wrapping_mul(0x100000001b3);
                }
                out.insert(p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/"), h);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

fn compiled(text: &str, name: &str) -> bool {
    text.contains(&format!("Compiling {name} v"))
}

#[test]
#[ignore = "slow: compiles the hook crates twice"]
fn the_overlay_stamps_without_touching_the_tree_and_keeps_paths_ids_and_fingerprints() {
    let tmp = std::env::temp_dir().join(format!("yoake-overlay-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ws = tmp.join("ws");
    fixture(&ws, false);
    let (t_plain, t_a, t_b) = (tmp.join("t-plain"), tmp.join("t-a"), tmp.join("t-b"));
    let exe = env!("CARGO_BIN_EXE_yoake-hook-codemod");
    let source_before = std::fs::read_to_string(ws.join("crates/demo/src/a.rs")).unwrap();

    // 1. plain build: unstamped, no overlay
    ok(&cargo(&ws, &t_plain, false, &["build", "--offline"]));
    let plain = run_app(&t_plain);
    assert!(!t_plain.join("yoake-stamp").exists());
    assert!(plain["file"].ends_with("a.rs"), "{plain:?}");

    // 2. ledger sync: ids land in hook-ledger, the sources do not change
    let sync = Command::new(exe)
        .current_dir(&ws)
        .args(["sync-ledger", "--workspace-root", ".", "--overlay-root"])
        .arg(tmp.join("sync-root"))
        .output()
        .unwrap();
    ok(&sync);
    assert_eq!(std::fs::read_to_string(ws.join("crates/demo/src/a.rs")).unwrap(), source_before);
    let ledger = std::fs::read_to_string(ws.join("hook-ledger/demo.json")).unwrap();
    assert!(ledger.contains("demo::a::Counter::bump"), "{ledger}");

    // 3. stamped build from the committed ledger
    ok(&cargo(&ws, &t_a, true, &["build", "--offline"]));
    let stamped = run_app(&t_a);
    let overlay_a = std::fs::read_to_string(t_a.join("yoake-stamp/root/crates/demo/src/a.rs")).unwrap();
    assert!(overlay_a.contains("yoake_hook(") && overlay_a.contains("demo::a::Counter::bump"), "{overlay_a}");
    assert_eq!(std::fs::read_to_string(ws.join("crates/demo/src/a.rs")).unwrap(), source_before, "tree untouched");
    assert_eq!(stamped["file"], plain["file"], "file!() names the original file");
    assert_eq!(stamped["panic_file"], plain["panic_file"], "panic locations name the original file");
    assert_eq!(stamped["bump"], plain["bump"], "same behaviour with no mod");
    let size = |t: &Path| std::fs::metadata(t.join("debug").join(if cfg!(windows) { "app.exe" } else { "app" })).unwrap().len();
    println!("exe sizes: plain {} stamped {}", size(&t_plain), size(&t_a));

    // 4. identical ids on a second clean machine
    ok(&cargo(&ws, &t_b, true, &["build", "--offline"]));
    assert_eq!(tree_hash(&t_a.join("yoake-stamp/root")), tree_hash(&t_b.join("yoake-stamp/root")), "overlay trees equal");
    let _ = std::fs::remove_dir_all(&t_b);

    // 5. fingerprints
    let nothing = ok(&cargo(&ws, &t_a, true, &["build", "--offline", "-v"]));
    assert!(!compiled(&nothing, "demo") && !compiled(&nothing, "app"), "nothing changed, nothing rebuilt:\n{nothing}");

    write(&ws.join("crates/other/src/lib.rs"), "pub fn n() -> u32 {\n    2\n}\n");
    let other = ok(&cargo(&ws, &t_a, true, &["build", "--offline", "-v"]));
    assert!(compiled(&other, "other") && compiled(&other, "app"), "{other}");
    assert!(!compiled(&other, "demo"), "an unrelated edit does not rebuild the stamped crate:\n{other}");

    let mut edited = source_before.clone();
    edited.push_str("\npub fn extra(x: u32) -> u32 {\n    let y = x;\n    y + 1\n}\n");
    write(&ws.join("crates/demo/src/a.rs"), &edited);
    let demo = ok(&cargo(&ws, &t_a, true, &["build", "--offline", "-v"]));
    assert!(compiled(&demo, "demo"), "an edit of the original source rebuilds the stamped crate:\n{demo}");
    let overlay_after = std::fs::read_to_string(t_a.join("yoake-stamp/root/crates/demo/src/a.rs")).unwrap();
    assert!(overlay_after.contains("pub fn extra"), "overlay follows the source");
    assert!(!overlay_after.contains("::extra\""), "a fn without a ledger id is not hooked (frozen ledger)");

    let mut ledger_edit = std::fs::read_to_string(ws.join("hook-ledger/demo.json")).unwrap();
    ledger_edit.push('\n');
    write(&ws.join("hook-ledger/demo.json"), &ledger_edit);
    let after_ledger = ok(&cargo(&ws, &t_a, true, &["build", "--offline", "-v"]));
    assert!(compiled(&after_ledger, "demo"), "a ledger change rebuilds the stamped crate:\n{after_ledger}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
#[ignore = "slow: compiles the hook crates"]
fn verify_through_the_overlay_withdraws_a_rejected_stamp_and_leaves_the_tree_alone() {
    let tmp = std::env::temp_dir().join(format!("yoake-overlay-verify-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let ws = tmp.join("ws");
    fixture(&ws, true);
    let target = tmp.join("t");
    let source_before = std::fs::read_to_string(ws.join("crates/demo/src/a.rs")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_yoake-hook-codemod"))
        .current_dir(&ws)
        .env("YOAKE_STAMP_TARGET", &target)
        .env("CARGO_INCREMENTAL", "0")
        .args([
            "apply", "--stamp-config", "hook-ledger/stamp-config.txt", "--crate-root", "crates/demo", "--crate-name", "demo",
            "--ledger", "hook-ledger", "--verify", "--overlay", "--verify-dir", ".", "--verify-arg", "build", "--verify-arg", "--offline",
        ])
        .output()
        .unwrap();
    let text = ok(&out);
    assert!(text.contains("build clean after 2 round"), "{text}");
    assert_eq!(std::fs::read_to_string(ws.join("crates/demo/src/a.rs")).unwrap(), source_before, "tree untouched");
    let ledger: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(ws.join("hook-ledger/demo.json")).unwrap()).unwrap();
    let status = |key: &str| {
        ledger["entries"].as_array().unwrap().iter().find(|e| e["key"] == key).map(|e| e["status"].as_str().unwrap().to_string())
    };
    assert_eq!(status("demo::a::Counter::foo").as_deref(), Some("tombstone"), "{ledger}");
    assert_eq!(status("demo::a::Counter::bump").as_deref(), Some("live"), "{ledger}");
    let overlay = std::fs::read_to_string(target.join("yoake-stamp/root/crates/demo/src/a.rs")).unwrap();
    assert!(overlay.contains("Counter::bump") && !overlay.contains("Counter::foo\""), "{overlay}");
    let _ = std::fs::remove_dir_all(&tmp);
}
