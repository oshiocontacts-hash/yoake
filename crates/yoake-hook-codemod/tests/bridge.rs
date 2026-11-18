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
            "yoake_codemod_bridge_test_{label}_{}_{}",
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

fn setup_bridge_fixture(root: &Path) {
    fs::create_dir_all(root.join("src/game")).unwrap();
    fs::create_dir_all(root.join("src/yoake_glue")).unwrap();
    fs::create_dir_all(root.join("ledger")).unwrap();

    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(root.join("src/lib.rs"), "pub mod entities;\npub mod game;\n").unwrap();
    fs::write(
        root.join("src/yoake_glue/entities.rs"),
        r#"pub mod yoake_glue {
    use yoake_slice::activation::FieldTable;
    pub static FIELDS_crate__entities__Thing: FieldTable<crate::entities::Thing> = FieldTable {
        get: |_, _| None,
        set: |_, _, _| false,
    };
}
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/entities.rs"),
        r#"pub struct Thing {
    pub count: i32,
}

pub struct Other {
    pub count: i32,
}

pub trait Rules {
    fn poke(&mut self) -> f32;
}

impl Rules for Thing {
    fn poke(&mut self) -> f32 {
        self.__yoake_trait_rules_poke()
    }
}

impl Thing {
    #[yoake_hook_macro::yoake_hook(method, id = 1, key = "demo::entities::Thing::bump")]
    pub fn bump(&mut self, delta: &mut i32, count: i32) -> i32 {
        self.count += *delta + count;
        *delta = 0;
        self.count
    }

    #[yoake_hook_macro::yoake_hook(method, id = 2, key = "demo::entities::<Thing as Rules>::poke")]
    fn __yoake_trait_rules_poke(&mut self) -> f32 {
        1.5
    }

    #[yoake_hook_macro::yoake_hook(method, id = 3, key = "demo::entities::Thing::read_count")]
    pub fn read_count(&self) -> i32 {
        self.count
    }

    #[yoake_hook_macro::yoake_hook(method, id = 9, key = "demo::entities::Thing::take_pair")]
    pub fn take_pair(&mut self, delta: &mut i32) -> Option<Self> {
        self.count += *delta;
        *delta = 0;
        None
    }
}

#[yoake_hook_macro::yoake_hook(id = 4, key = "demo::entities::reset_thing")]
pub fn reset_thing(t: &mut Thing) {
    t.count = 0;
}

#[yoake_hook_macro::yoake_hook(id = 5, key = "demo::entities::bad_ret")]
pub fn bad_ret(t: &mut Thing) -> (i32, i32) {
    (t.count, 0)
}

#[yoake_hook_macro::yoake_hook(id = 6, key = "demo::entities::no_table")]
pub fn no_table(_o: &mut Other) {}

#[yoake_hook_macro::yoake_hook(id = 7, key = "fn:1@call:fn:4", kind = callsite)]
#[inline(always)]
fn __yoake_cs_7(recv: &mut crate::entities::Thing) {
    crate::entities::reset_thing(recv)
}

#[yoake_hook_macro::yoake_hook(id = 10, key = "fn:1@call:fn:5", kind = callsite)]
#[inline(always)]
fn __yoake_cs_10(recv: &mut crate::entities::Thing, a0: i32) -> std::option::Option<std::vec::Vec<i32>> {
    let _ = (recv, a0);
    None
}

#[yoake_hook_macro::yoake_hook(id = 11, key = "fn:1@call:fn:3", kind = callsite)]
#[inline(always)]
fn __yoake_cs_11<'r>(recv: &'r mut crate::entities::Thing) -> &'r mut i32 {
    &mut recv.count
}

include!("yoake_glue/entities.rs");
"#,
    )
    .unwrap();
    fs::write(
        root.join("src/game.rs"),
        "pub mod item_use;\npub struct Game<'a> {\n    pub marker: &'a str,\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("src/game/item_use.rs"),
        r#"use super::Game;

impl Game<'_> {
    #[yoake_hook_macro::yoake_hook(method, self_ty = Game<'_>, id = 8, key = "demo::game::item_use::Game::tick_item")]
    pub fn tick_item(&mut self) -> bool {
        true
    }
}
"#,
    )
    .unwrap();

    fs::write(
        root.join("ledger/meta.json"),
        "{\n  \"version\": 2,\n  \"next_id\": 20\n}\n",
    )
    .unwrap();
    fs::write(
        root.join("ledger/demo.json"),
        r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::entities::Thing::bump", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::entities::<Thing as Rules>::poke", "sig": "0000000000000002", "status": "live", "kind": "fn" },
    { "id": 3, "key": "demo::entities::Thing::read_count", "sig": "0000000000000003", "status": "live", "kind": "fn" },
    { "id": 4, "key": "demo::entities::reset_thing", "sig": "0000000000000004", "status": "live", "kind": "fn" },
    { "id": 5, "key": "demo::entities::bad_ret", "sig": "0000000000000005", "status": "live", "kind": "fn" },
    { "id": 6, "key": "demo::entities::no_table", "sig": "0000000000000006", "status": "live", "kind": "fn" },
    { "id": 7, "key": "fn:1@call:fn:4", "sig": "0000000000000007", "status": "live", "kind": "callsite" },
    { "id": 8, "key": "demo::game::item_use::Game::tick_item", "sig": "0000000000000008", "status": "live", "kind": "fn" },
    { "id": 9, "key": "demo::entities::Thing::take_pair", "sig": "0000000000000009", "status": "live", "kind": "fn" },
    { "id": 10, "key": "fn:1@call:fn:5", "sig": "0000000000000010", "status": "live", "kind": "callsite" },
    { "id": 11, "key": "fn:1@call:fn:3", "sig": "0000000000000011", "status": "live", "kind": "callsite" }
  ],
  "excluded": []
}
"#,
    )
    .unwrap();
}

#[test]
fn bridge_generates_dispatchers_checks_staleness_and_applies_excludes() {
    let dir = TestDir::new("main");
    setup_bridge_fixture(dir.path());
    let entities_bridge = dir.path().join("src/yoake_glue/hook_bridge_entities.rs");
    let game_bridge = dir
        .path()
        .join("src/yoake_glue/hook_bridge_game__item_use.rs");
    let glue_path = dir.path().join("src/yoake_glue/entities.rs");
    let ledger_dir = dir.path().join("ledger");

    // Missing bridge files fail `--check` without creating files.
    let chk_missing = Command::new(codemod_bin())
        .args(["bridge", "--check", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "entities", "--module", "game::item_use", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert_eq!(chk_missing.status.code(), Some(1));
    assert!(!entities_bridge.exists());
    assert!(!game_bridge.exists());

    // Generate both bridge files.
    let gen_out = Command::new(codemod_bin())
        .args(["bridge", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "entities", "--module", "game::item_use", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(
        gen_out.status.success(),
        "bridge failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&gen_out.stdout),
        String::from_utf8_lossy(&gen_out.stderr)
    );

    let entities_src = fs::read_to_string(&entities_bridge).unwrap();
    assert!(entities_src.contains("pub mod hook_bridge {"), "{entities_src}");
    assert!(entities_src.contains("fn __bridge_1("), "{entities_src}");
    assert!(
        entities_src.contains("original(p0, &mut *p1, p2)"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("let mut last: Option<i32> = None;"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains(
            "yoake_slice::runtime::dispatch_id(1, p0, &crate::entities::yoake_glue::FIELDS_crate__entities__Thing, &mut |p0| { let r = original(p0, &mut *p1, p2); last = Some(r); yoake_slice::bridge::encode::int(r) })"
        ),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("match yoake_slice::bridge::decode::int::<i32>(value)"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("yoake_slice::bridge::note_out_of_range(1, value);"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("match last { Some(r) => r, None => original(p0, &mut *p1, p2) }"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains(
            "yoake_slice::bridge::install_once(id, || Thing::__yoake_register_bump(__bridge_1, order))"
        ),
        "{entities_src}"
    );
    assert!(entities_src.contains("fn __bridge_2("), "{entities_src}");
    assert!(
        entities_src.contains("let mut last: Option<f32> = None;"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains(
            "yoake_slice::runtime::dispatch_id(2, p0, &crate::entities::yoake_glue::FIELDS_crate__entities__Thing, &mut |p0| { let r = original(p0); last = Some(r); yoake_slice::bridge::encode::f32(r) })"
        ),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("match yoake_slice::bridge::decode::f32(value)"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("yoake_slice::bridge::note_out_of_range(2, value);"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("match last { Some(r) => r, None => original(p0) }"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains(
            "yoake_slice::bridge::install_once(id, || Thing::__yoake_register___yoake_trait_rules_poke(__bridge_2, order))"
        ),
        "{entities_src}"
    );
    assert!(entities_src.contains("fn __bridge_4("), "{entities_src}");
    assert!(
        entities_src.contains("let cb: super::__YoakeHook_reset_thing = __bridge_4;"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("super::__yoake_register_reset_thing(cb, order)\n"),
        "{entities_src}"
    );
    assert!(entities_src.contains("fn __bridge_7("), "{entities_src}");
    assert!(
        entities_src.contains("let cb: super::__YoakeHook___yoake_cs_7 = __bridge_7;"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("super::__yoake_register___yoake_cs_7(cb, order)\n"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("pub const BRIDGED: &[u32] = &[\n        1,\n        2,\n        4,\n        5,\n        7,\n        9,\n        10,\n    ];"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("pub fn mode(id: u32) -> Option<yoake_slice::bridge::Mode>"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("Some(yoake_slice::bridge::Mode::ObserveOnly)"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("Some(yoake_slice::bridge::Mode::Full)"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("pub fn install(registered: &[(u32, i32)]) -> yoake_slice::bridge::InstallReport"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("if id >= yoake_slice::runtime::SYNTHETIC_HOOK_BASE"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("Some(Ok(())) => report.installed.push(id),"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("Some(Err(err)) => report.failed.push((id, err)),"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("report.not_bridgeable.push(entry);"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("report.not_mine.push(id);"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("/// - `BRIDGED`:"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("/// - `install`:"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("/// - `mode`:"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("/// Hooks listed in `OBSERVE_ONLY`"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("cannot be cancelled in this mode"),
        "{entities_src}"
    );
    // Observe-only non-i64 return types: id 5 (free fn), id 9 (method returning Option<Self>), id 10 (callsite).
    assert!(entities_src.contains("fn __bridge_5("), "{entities_src}");
    assert!(
        entities_src.contains("let mut result: Option<(i32, i32)> = None;"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("result = Some(original(p0)); 0"),
        "{entities_src}"
    );
    assert!(entities_src.contains("None => original(p0),"), "{entities_src}");
    assert!(entities_src.contains("fn __bridge_9("), "{entities_src}");
    assert!(
        entities_src.contains("let mut result: Option<Option<Thing>> = None;"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("result = Some(original(p0, &mut *p1)); 0"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("None => original(p0, &mut *p1),"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains(
            "yoake_slice::bridge::install_once(id, || Thing::__yoake_register_take_pair(__bridge_9, order))"
        ),
        "{entities_src}"
    );
    assert!(entities_src.contains("fn __bridge_10("), "{entities_src}");
    assert!(
        entities_src.contains("let mut result: Option<std::option::Option<std::vec::Vec<i32>>> = None;"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("super::__yoake_register___yoake_cs_10(cb, order)\n"),
        "{entities_src}"
    );
    assert!(
        entities_src.contains("pub const OBSERVE_ONLY: &[u32] = &[\n        5,\n        9,\n        10,\n    ];"),
        "{entities_src}"
    );
    assert!(!entities_src.contains("fn __bridge_3("), "{entities_src}");
    assert!(!entities_src.contains("fn __bridge_6("), "{entities_src}");
    assert!(!entities_src.contains("fn __bridge_11("), "{entities_src}");
    assert!(entities_src.contains("(3, \""), "{entities_src}");
    assert!(!entities_src.contains("(5, \""), "{entities_src}");
    assert!(entities_src.contains("(6, \""), "{entities_src}");
    assert!(!entities_src.contains("(9, \""), "{entities_src}");
    assert!(!entities_src.contains("(10, \""), "{entities_src}");
    assert!(
        entities_src.contains("(11, \"return type `&'r mut i32` has non-static lifetime\")"),
        "{entities_src}"
    );

    let game_src = fs::read_to_string(&game_bridge).unwrap();
    assert!(game_src.contains("fn __bridge_8("), "{game_src}");
    assert!(
        game_src.contains("let mut last: Option<bool> = None;"),
        "{game_src}"
    );
    assert!(
        game_src.contains(
            "yoake_slice::runtime::dispatch_root_id(8, p0, &mut |p0| { let r = original(p0); last = Some(r); yoake_slice::bridge::encode::bool(r) })"
        ),
        "{game_src}"
    );
    assert!(
        game_src.contains("match yoake_slice::bridge::decode::bool(value)"),
        "{game_src}"
    );
    assert!(
        game_src.contains("yoake_slice::bridge::note_out_of_range(8, value);"),
        "{game_src}"
    );
    assert!(
        game_src.contains("match last { Some(r) => r, None => original(p0) }"),
        "{game_src}"
    );
    assert!(
        game_src.contains(
            "pub const BRIDGED: &[u32] = &[\n        8,\n    ];"
        ),
        "{game_src}"
    );
    assert!(
        game_src.contains(
            "yoake_slice::bridge::install_once(id, || Game::__yoake_register_tick_item(__bridge_8, order))"
        ),
        "{game_src}"
    );
    assert!(
        game_src.contains("pub const OBSERVE_ONLY: &[u32] = &[\n    ];"),
        "{game_src}"
    );

    // Fresh `--check` succeeds.
    let chk_clean = Command::new(codemod_bin())
        .args(["bridge", "--check", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "entities", "--module", "game::item_use", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert!(
        chk_clean.status.success(),
        "{}",
        String::from_utf8_lossy(&chk_clean.stdout)
    );

    // Stale file fails `--check` without overwriting.
    let stale_content = format!("{entities_src}\n// stale\n");
    fs::write(&entities_bridge, &stale_content).unwrap();
    let chk_stale = Command::new(codemod_bin())
        .args(["bridge", "--check", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "entities", "--module", "game::item_use", "--glue"])
        .arg(&glue_path)
        .output()
        .unwrap();
    assert_eq!(chk_stale.status.code(), Some(1));
    assert_eq!(fs::read_to_string(&entities_bridge).unwrap(), stale_content);

    // Exclude file moves listed ids into UNBRIDGED with their reasons (including observe-only ids).
    let exclude_path = dir.path().join("custom-exclude.txt");
    fs::write(
        &exclude_path,
        "# ignore free fn and observe-only fn\n4 excluded by test\n5 excluded observe-only\n",
    )
    .unwrap();
    let gen_excl = Command::new(codemod_bin())
        .args(["bridge", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(&ledger_dir)
        .args(["--module", "entities", "--glue"])
        .arg(&glue_path)
        .args(["--exclude-file"])
        .arg(&exclude_path)
        .output()
        .unwrap();
    assert!(gen_excl.status.success());
    let entities_excl = fs::read_to_string(&entities_bridge).unwrap();
    assert!(!entities_excl.contains("fn __bridge_4("), "{entities_excl}");
    assert!(!entities_excl.contains("fn __bridge_5("), "{entities_excl}");
    assert!(
        entities_excl.contains("(4, \"excluded by test\")"),
        "{entities_excl}"
    );
    assert!(
        entities_excl.contains("(5, \"excluded observe-only\")"),
        "{entities_excl}"
    );
    assert!(
        entities_excl.contains("pub const BRIDGED: &[u32] = &[\n        1,\n        2,\n        7,\n        9,\n        10,\n    ];"),
        "{entities_excl}"
    );
    assert!(
        entities_excl.contains("pub const OBSERVE_ONLY: &[u32] = &[\n        9,\n        10,\n    ];"),
        "{entities_excl}"
    );
}

#[test]
fn bridge_scalar_result_kinds() {
    let dir = TestDir::new("scalars");
    fs::create_dir_all(dir.path().join("src/yoake_glue")).unwrap();
    fs::create_dir_all(dir.path().join("ledger")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub mod scalars;\n").unwrap();
    fs::write(
        dir.path().join("src/yoake_glue/scalars.rs"),
        r#"pub mod yoake_glue {
    use yoake_slice::activation::FieldTable;
    pub static FIELDS_crate__scalars__Counter: FieldTable<crate::scalars::Counter> = FieldTable {
        get: |_, _| None,
        set: |_, _, _| false,
    };
}
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("src/scalars.rs"),
        r#"pub struct Counter {
    pub v: i64,
}

#[yoake_hook_macro::yoake_hook(id = 1, key = "demo::scalars::ret_i64")]
pub fn ret_i64(c: &mut Counter) -> i64 {
    c.v
}

#[yoake_hook_macro::yoake_hook(id = 2, key = "demo::scalars::ret_u64")]
pub fn ret_u64(c: &mut Counter) -> u64 {
    c.v as u64
}

#[yoake_hook_macro::yoake_hook(id = 3, key = "demo::scalars::ret_usize")]
pub fn ret_usize(c: &mut Counter) -> usize {
    c.v as usize
}

#[yoake_hook_macro::yoake_hook(id = 4, key = "demo::scalars::ret_isize")]
pub fn ret_isize(c: &mut Counter) -> isize {
    c.v as isize
}

#[yoake_hook_macro::yoake_hook(id = 5, key = "demo::scalars::ret_u8")]
pub fn ret_u8(c: &mut Counter) -> u8 {
    c.v as u8
}

#[yoake_hook_macro::yoake_hook(id = 6, key = "demo::scalars::ret_f64")]
pub fn ret_f64(c: &mut Counter) -> f64 {
    c.v as f64
}

include!("yoake_glue/scalars.rs");
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("ledger/meta.json"),
        "{\n  \"version\": 2,\n  \"next_id\": 10\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("ledger/demo.json"),
        r#"{
  "crate": "demo",
  "entries": [
    { "id": 1, "key": "demo::scalars::ret_i64", "sig": "0000000000000001", "status": "live", "kind": "fn" },
    { "id": 2, "key": "demo::scalars::ret_u64", "sig": "0000000000000002", "status": "live", "kind": "fn" },
    { "id": 3, "key": "demo::scalars::ret_usize", "sig": "0000000000000003", "status": "live", "kind": "fn" },
    { "id": 4, "key": "demo::scalars::ret_isize", "sig": "0000000000000004", "status": "live", "kind": "fn" },
    { "id": 5, "key": "demo::scalars::ret_u8", "sig": "0000000000000005", "status": "live", "kind": "fn" },
    { "id": 6, "key": "demo::scalars::ret_f64", "sig": "0000000000000006", "status": "live", "kind": "fn" }
  ],
  "excluded": []
}
"#,
    )
    .unwrap();

    let out = Command::new(codemod_bin())
        .args(["bridge", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "scalars", "--glue"])
        .arg(dir.path().join("src/yoake_glue/scalars.rs"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let src = fs::read_to_string(dir.path().join("src/yoake_glue/hook_bridge_scalars.rs")).unwrap();
    assert!(
        src.contains("pub const BRIDGED: &[u32] = &[\n        1,\n        2,\n        3,\n        4,\n        5,\n        6,\n    ];"),
        "{src}"
    );
    // i64: identity, no last
    assert!(
        src.contains(
            "yoake_slice::runtime::dispatch_id(1, p0, &crate::scalars::yoake_glue::FIELDS_crate__scalars__Counter, &mut |p0| original(p0))"
        ),
        "{src}"
    );
    // u64: bit pattern, never out of range, no last
    assert!(
        src.contains(
            "yoake_slice::bridge::decode::u64(yoake_slice::runtime::dispatch_id(2, p0, &crate::scalars::yoake_glue::FIELDS_crate__scalars__Counter, &mut |p0| yoake_slice::bridge::encode::u64(original(p0))))"
        ),
        "{src}"
    );
    // usize: range-checked with encode::usize / decode::int::<usize>
    assert!(src.contains("let mut last: Option<usize> = None;"), "{src}");
    assert!(src.contains("yoake_slice::bridge::encode::usize(r)"), "{src}");
    assert!(
        src.contains("match yoake_slice::bridge::decode::int::<usize>(value)"),
        "{src}"
    );
    assert!(
        src.contains("yoake_slice::bridge::note_out_of_range(3, value);"),
        "{src}"
    );
    // isize: range-checked with `r as i64` / decode::int::<isize>
    assert!(src.contains("let mut last: Option<isize> = None;"), "{src}");
    assert!(src.contains("last = Some(r); r as i64"), "{src}");
    assert!(
        src.contains("match yoake_slice::bridge::decode::int::<isize>(value)"),
        "{src}"
    );
    assert!(
        src.contains("yoake_slice::bridge::note_out_of_range(4, value);"),
        "{src}"
    );
    // u8: range-checked with encode::int / decode::int::<u8>
    assert!(src.contains("let mut last: Option<u8> = None;"), "{src}");
    assert!(src.contains("yoake_slice::bridge::encode::int(r)"), "{src}");
    assert!(
        src.contains("match yoake_slice::bridge::decode::int::<u8>(value)"),
        "{src}"
    );
    assert!(
        src.contains("yoake_slice::bridge::note_out_of_range(5, value);"),
        "{src}"
    );
    // f64: bit pattern, never out of range, no last
    assert!(
        src.contains(
            "yoake_slice::bridge::decode::f64(yoake_slice::runtime::dispatch_id(6, p0, &crate::scalars::yoake_glue::FIELDS_crate__scalars__Counter, &mut |p0| yoake_slice::bridge::encode::f64(original(p0))))"
        ),
        "{src}"
    );
    // Only usize (3), isize (4), u8 (5) emit `last` and `note_out_of_range`.
    assert_eq!(src.matches("let mut last:").count(), 3, "{src}");
    assert_eq!(src.matches("note_out_of_range(").count(), 3, "{src}");
    assert_eq!(
        src.matches("match last { Some(r) => r, None => original(p0) }")
            .count(),
        3,
        "{src}"
    );
}

#[test]
fn hot_callers_ignores_keys_from_other_crates_silently() {
    let dir = TestDir::new("hot_callers");
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::create_dir_all(dir.path().join("ledger")).unwrap();
    fs::write(
        dir.path().join("Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    fs::write(dir.path().join("src/lib.rs"), "pub mod bag;\n").unwrap();
    let bag_rs = "pub fn ping() {}\n";
    fs::write(dir.path().join("src/bag.rs"), bag_rs).unwrap();
    fs::write(
        dir.path().join("ledger/meta.json"),
        "{\n  \"version\": 2,\n  \"next_id\": 10\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("ledger/demo.json"),
        "{\n  \"crate\": \"demo\",\n  \"entries\": [],\n  \"excluded\": []\n}\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("ledger/callsite-hot-callers.txt"),
        "other_crate::entities::ArmorStand::tick_settle hot-path\ndemo::bag::missing_fn hot-path\n",
    )
    .unwrap();

    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bag_rs.as_bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let thir = serde_json::json!({
        "crate": "demo",
        "files": {
            "src/bag.rs": {
                "written_calls": 0,
                "sha": format!("{hash:016x}"),
                "len": bag_rs.len()
            }
        },
        "calls": []
    });
    let thir_path = dir.path().join("calls.json");
    fs::write(&thir_path, serde_json::to_string(&thir).unwrap()).unwrap();

    let out = Command::new(codemod_bin())
        .args(["check", "--callsites", "--crate-root"])
        .arg(dir.path())
        .args(["--crate-name", "demo", "--ledger"])
        .arg(dir.path().join("ledger"))
        .args(["--module", "bag", "--thir"])
        .arg(&thir_path)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stderr.contains("other_crate::entities::ArmorStand::tick_settle"), "{stderr}");
    assert!(
        stderr.contains("warning: callsite-hot-callers.txt key 'demo::bag::missing_fn' matched no fn in scanned modules"),
        "{stderr}"
    );
}
