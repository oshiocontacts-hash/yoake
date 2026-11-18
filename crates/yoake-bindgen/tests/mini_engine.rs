//! The generator on a small stand-in engine module (`fixtures/mini_engine.rs`):
//!
//! 1. rustdoc JSON of the fixture (stable toolchain, `RUSTC_BOOTSTRAP=1`, private items);
//! 2. the generator writes the glue;
//! 3. the glue is `include!`d back into the fixture (`--cfg glue`) and compiled with `rustc` against the
//!    real `yoake_slice` rlib, and the fixture's `main` calls every kind of binding and asserts on the result.
//!
//! So a generator change that emits uncompilable or wrong glue fails here, fast, without the engine.

use std::path::{Path, PathBuf};
use std::process::Command;

use yoake_bindgen::analyze::Cat;
use yoake_bindgen::{Config, Generated, TargetCfg, generate, load_universe};

fn tmp(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("reports")).unwrap();
    std::fs::create_dir_all(dir.join("glue")).unwrap();
    dir
}

fn fixture_source() -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mini_engine.rs")).unwrap()
}

/// rustdoc JSON of `dir/mini_engine.rs`; returns the JSON path.
fn rustdoc_json(dir: &Path) -> PathBuf {
    let out = dir.join("doc");
    let status = Command::new("rustdoc")
        .env("RUSTC_BOOTSTRAP", "1")
        .args(["--edition", "2021", "--crate-name", "mini_engine", "--crate-type", "lib"])
        .args(["-Z", "unstable-options", "--output-format", "json", "--document-private-items"])
        .arg("-o")
        .arg(&out)
        .arg(dir.join("mini_engine.rs"))
        .output()
        .expect("run rustdoc");
    assert!(status.status.success(), "rustdoc failed:\n{}", String::from_utf8_lossy(&status.stderr));
    out.join("mini_engine.json")
}

fn config(dir: &Path, json: &Path) -> Config {
    let writable = |list: &[&str]| list.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
    Config {
        frozen_files: Vec::new(),
        docs: vec![json.to_string_lossy().into_owned()],
        lock: dir.join("ids.lock.json").to_string_lossy().into_owned(),
        report_dir: dir.join("reports").to_string_lossy().into_owned(),
        targets: vec![TargetCfg {
            name: "mini".into(),
            crate_name: "mini_engine".into(),
            source: "mini_engine.rs".into(),
            glue: dir.join("glue").join("glue.rs").to_string_lossy().into_owned(),
            roots: vec!["Game".into()],
            writable: [("Thing".to_string(), writable(&["pos", "name", "scores", "locked", "opt", "big", "table", "ratio", "flag"]))].into_iter().collect(),
            frozen_types: vec!["Sealed".into()],
            guest_stubs: None,
            game_paths: vec![
                yoake_bindgen::GamePathCfg {
                    name: "thing".into(),
                    ty: "Thing".into(),
                    path: vec!["things".into()],
                    by_id: Some(yoake_bindgen::ByIdCfg { field: "serial".into(), ty: "u32".into() }),
                },
                yoake_bindgen::GamePathCfg { name: "lone".into(), ty: "Thing".into(), path: vec!["lone".into()], by_id: None },
            ],
        }],
    }
}

fn run_generator(dir: &Path) -> Generated {
    let json = rustdoc_json(dir);
    let config = config(dir, &json);
    let uni = load_universe(&config, Path::new(".")).unwrap();
    let generated = generate(&config, Path::new("."), &uni).unwrap();
    for (path, text) in &generated.files {
        std::fs::write(path, text).unwrap();
    }
    generated
}

fn prepared(name: &str) -> PathBuf {
    let dir = tmp(name);
    std::fs::write(dir.join("mini_engine.rs"), fixture_source()).unwrap();
    dir
}

/// `libyoake_slice-*.rlib` next to this test binary (cargo builds the dev-dependency there).
fn yoake_slice_rlib() -> (PathBuf, PathBuf) {
    let deps = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&deps).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("libyoake_slice-") && name.ends_with(".rlib") {
            let modified = entry.metadata().unwrap().modified().unwrap();
            if best.as_ref().is_none_or(|(t, _)| modified > *t) {
                best = Some((modified, entry.path()));
            }
        }
    }
    (deps, best.expect("libyoake_slice rlib next to the test binary").1)
}

#[test]
fn generated_glue_compiles_and_every_binding_works() {
    let dir = prepared("mini_a");
    run_generator(&dir);
    let (deps, rlib) = yoake_slice_rlib();
    let exe = dir.join(if cfg!(windows) { "mini_prog.exe" } else { "mini_prog" });
    let compile = Command::new("rustc")
        .args(["--edition", "2021", "--cfg", "glue", "--crate-name", "mini_prog", "--crate-type", "bin"])
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("yoake_slice={}", rlib.display()))
        .arg("-o")
        .arg(&exe)
        .arg(dir.join("mini_engine.rs"))
        .output()
        .expect("run rustc");
    assert!(compile.status.success(), "the generated glue did not compile:\n{}", String::from_utf8_lossy(&compile.stderr));
    let run = Command::new(&exe).output().expect("run the compiled fixture");
    assert!(
        run.status.success(),
        "fixture assertions failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(String::from_utf8_lossy(&run.stdout).contains("all glue checks passed"));
}

fn compile_glue(dir: &Path, glue: &str) -> std::process::Output {
    std::fs::write(dir.join("glue").join("glue.rs"), glue).unwrap();
    let (deps, rlib) = yoake_slice_rlib();
    Command::new("rustc")
        .args(["--edition", "2021", "--cfg", "glue", "--crate-name", "mini_prog", "--crate-type", "bin"])
        .arg("-L")
        .arg(format!("dependency={}", deps.display()))
        .arg("--extern")
        .arg(format!("yoake_slice={}", rlib.display()))
        .arg("-o")
        .arg(dir.join("forbid_probe.exe"))
        .arg(dir.join("mini_engine.rs"))
        .output()
        .expect("run rustc")
}

/// The compiler, not a text search, keeps the glue free of `unsafe`: the module carries
/// `#![forbid(unsafe_code)]`, and a hand-inserted `unsafe` block fails to compile.
#[test]
fn glue_module_forbids_unsafe_code() {
    let dir = prepared("mini_b");
    let generated = run_generator(&dir);
    let glue = generated.files[&dir.join("glue").join("glue.rs")].clone();
    assert!(glue.contains("#![forbid(unsafe_code)]"));
    let ok = compile_glue(&dir, &glue);
    assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
    let probe = glue.replacen("    use yoake_slice::activation", "    fn smuggled() -> u8 {\n        unsafe { std::mem::zeroed() }\n    }\n    use yoake_slice::activation", 1);
    assert_ne!(probe, glue);
    let bad = compile_glue(&dir, &probe);
    assert!(!bad.status.success(), "an unsafe block inside the glue module must not compile");
    assert!(String::from_utf8_lossy(&bad.stderr).contains("unsafe"), "{}", String::from_utf8_lossy(&bad.stderr));
}

#[test]
fn writable_is_opt_in_and_never_opens_identities() {
    let dir = prepared("mini_f");
    let json = rustdoc_json(&dir);
    let uni_config = config(&dir, &json);
    let uni = load_universe(&uni_config, Path::new(".")).unwrap();
    for bad in ["id", "player_id", "uuid", "len", "index", "seq", "join", "0", "entry_index"] {
        let mut cfg = uni_config.clone();
        cfg.targets[0].writable.insert("Thing".into(), vec![bad.to_string()]);
        let err = generate(&cfg, Path::new("."), &uni).err().unwrap_or_else(|| panic!("{bad} must be refused")).to_string();
        assert!(err.contains("cannot be writable"), "{bad}: {err}");
    }
    let mut cfg = uni_config.clone();
    cfg.targets[0].writable.insert("Thing".into(), vec!["no_such_field".into()]);
    assert!(generate(&cfg, Path::new("."), &uni).err().unwrap().to_string().contains("not a data field"));
    // the lock reserves the frozen setters
    let generated = generate(&uni_config, Path::new("."), &uni).unwrap();
    let lock: yoake_bindgen::lock::Lock = serde_json::from_str(&generated.files[&PathBuf::from(&uni_config.lock)]).unwrap();
    let status = |path: &str| lock.bindings.iter().find(|e| e.path == path).unwrap_or_else(|| panic!("{path}")).status.clone();
    assert_eq!(status("mini::Thing.count#set"), "ReadOnly");
    assert_eq!(status("mini::Thing.count#get"), "bound");
    assert_eq!(status("mini::Thing.name#set"), "bound");
    // and the report lists the writable fields
    let coverage = &generated.files[&PathBuf::from(&uni_config.report_dir).join("coverage.md")];
    assert!(coverage.contains("Writable fields") && coverage.contains("`mini::Thing`: `pos`, `name`"));
}

#[test]
fn lock_is_append_only_against_a_base() {
    use yoake_bindgen::lock::{Lock, LockEntry};
    let entry = |id: u32, path: &str| LockEntry { id, path: path.into(), sig: "s".into(), status: "bound".into() };
    let base = Lock { format: 1, rustdoc_format_version: 61, bindings: vec![entry(1, "a"), entry(2, "b")] };
    let mut now = base.clone();
    now.bindings.push(entry(3, "c"));
    now.bindings[1].status = "unbound".into();
    assert!(now.append_only_violations(&base).is_empty(), "adding and unbinding are fine");
    let mut removed = base.clone();
    removed.bindings.remove(0);
    assert!(!removed.append_only_violations(&base).is_empty());
    let mut renumbered = base.clone();
    renumbered.bindings[1].id = 9;
    assert!(renumbered.append_only_violations(&base).iter().any(|v| v.contains("removed") || v.contains("renumbered")));
    let mut swapped = base.clone();
    swapped.bindings[0].path = "x".into();
    assert!(swapped.append_only_violations(&base).iter().any(|v| v.contains("renumbered")));
}

#[test]
fn glue_files_of_targets_that_are_gone_are_stale() {
    let dir = prepared("mini_g");
    let generated = run_generator(&dir);
    let json = dir.join("doc").join("mini_engine.json");
    let cfg = config(&dir, &json);
    assert!(yoake_bindgen::stale_glue_files(&cfg, Path::new(".")).is_empty(), "{:?}", generated.files.keys().collect::<Vec<_>>());
    std::fs::write(dir.join("glue").join("old_module.rs"), "// glue of a module nobody generates any more\n").unwrap();
    let stale = yoake_bindgen::stale_glue_files(&cfg, Path::new("."));
    assert_eq!(stale, vec![dir.join("glue").join("old_module.rs")]);
}

#[test]
fn generation_is_deterministic() {
    let dir = prepared("mini_c");
    let first = run_generator(&dir);
    let second = run_generator(&dir);
    assert_eq!(first.files, second.files, "a second run over the same input must write the same bytes");
}

fn cat_of(generated: &Generated, path: &str) -> (Cat, Vec<String>) {
    let f = generated.modules[0].scan.fns.iter().find(|f| f.path == path).unwrap_or_else(|| panic!("no function {path}"));
    (f.cat, f.reasons.clone())
}

#[test]
fn classifier_decisions() {
    let dir = prepared("mini_d");
    let g = run_generator(&dir);
    let is_bound = |c: Cat| matches!(c, Cat::BoundA | Cat::BoundMech);

    // (a): scalars and strings only
    assert_eq!(cat_of(&g, "mini::Game::value").0, Cat::BoundA);
    assert_eq!(cat_of(&g, "mini::Game::string_is").0, Cat::BoundA);
    assert_eq!(cat_of(&g, "mini::midpoint").0, Cat::BoundA, "arrays of scalars");
    // mechanical (b): usize, Option, Vec, tuples, plain structs and enums, &mut write-back
    for path in [
        "mini::Game::name_count",
        "mini::Game::scale",
        "mini::takes_char",
        "mini::private_free",
        "mini::pass_marker",
        "mini::node_sum",
        "mini::opt_args",
        "mini::fast",
        "mini::Thing::translate",
        "mini::Thing::set_mode",
        "mini::Thing::sum_into",
        "mini::Thing::first_tag",
        "mini::Thing::lookup",
        "mini::Thing::private_helper",
    ] {
        assert_eq!(cat_of(&g, path).0, Cat::BoundMech, "{path}");
    }
    // Fable's list: a non-Game receiver of a type that is not data is a handle, not (c)
    let (cat, _) = cat_of(&g, "mini::Opaque::peek");
    assert!(is_bound(cat));
    let (cat, _) = cat_of(&g, "mini::Thing::bump");
    assert!(is_bound(cat), "Thing holds a Cell, so it is not data, but `&mut self` on it is a handle call");
    // tuple, unit and data-carrying enums are plain data (Mode has all three variant shapes)
    assert_eq!(cat_of(&g, "mini::Thing::set_mode").0, Cat::BoundMech);
    // handle-required, not (c)
    for path in ["mini::Thing::new", "mini::Thing::opaque", "mini::Thing::with_opaque", "mini::Thing::name_mut", "mini::Thing::into_count", "mini::plain_cell"] {
        assert_eq!(cat_of(&g, path).0, Cat::HandleRequired, "{path}: {:?}", cat_of(&g, path).1);
    }
    // excluded
    for path in [
        "mini::Thing::apply",
        "mini::Thing::each",
        "mini::Thing::raw",
        "mini::Thing::dynamic",
        "mini::generic_free",
        "mini::Borrowing::len",
        "mini::cb_cell",
    ] {
        assert_eq!(cat_of(&g, path).0, Cat::Excluded, "{path}");
    }
    let (_, reasons) = cat_of(&g, "mini::Borrowing::len");
    assert!(reasons.iter().any(|r| r.contains("lifetime")), "{reasons:?}");
    let (_, reasons) = cat_of(&g, "mini::Thing::each");
    assert!(reasons.iter().any(|r| r.contains("closure")), "{reasons:?}");
    // impl-level generics: `impl<T: Clone> Tick for Vec<T>` is excluded, the concrete impl is bound
    let generic = g.modules[0].scan.fns.iter().find(|f| f.sig.contains("fn tick") && f.cat == Cat::Excluded);
    assert!(generic.is_some(), "the generic Tick impl must be excluded");
    assert!(is_bound(cat_of(&g, "mini::<Thing as Tick>::tick").0));
    // fields: every field of Thing that is data has a getter and a setter, private ones included
    let fields = &g.modules[0].scan.fields;
    let bound: Vec<&str> = fields.iter().filter(|f| f.owner_label == "mini::Thing" && f.shape.is_some()).map(|f| f.field.as_str()).collect();
    for name in ["count", "pos", "name", "mode", "tags", "scores", "opt", "ratio", "flag", "big", "locked", "table"] {
        assert!(bound.contains(&name), "field {name} must have accessors, have {bound:?}");
    }
    assert!(!bound.contains(&"cell"));
}

#[test]
fn ids_are_stable_when_the_module_changes() {
    let dir = prepared("mini_e");
    let first = run_generator(&dir);
    let ids_of = |lock: &str| -> std::collections::BTreeMap<String, u32> {
        let lock: yoake_bindgen::lock::Lock = serde_json::from_str(lock).unwrap();
        lock.bindings.iter().map(|e| (e.path.clone(), e.id)).collect()
    };
    let lock_path = dir.join("ids.lock.json");
    let before = ids_of(&first.files[&lock_path]);
    assert!(before.len() > 40);

    // add a function that sorts before everything else, and change the signature of another
    let changed = fixture_source().replace(
        "pub fn midpoint(bounding_box: [[f64; 3]; 2]) -> [f64; 3] {",
        "pub fn aaa_new(x: u8) -> u8 {\n    x\n}\n\npub fn midpoint(bounding_box: [[f64; 3]; 2], extra: u8) -> [f64; 3] {\n    let _ = extra;",
    );
    std::fs::write(dir.join("mini_engine.rs"), changed).unwrap();
    let second = run_generator(&dir);
    let after = ids_of(&second.files[&lock_path]);
    for (path, id) in &before {
        assert_eq!(after.get(path), Some(id), "{path} must keep its id");
    }
    let new_id = after["mini::aaa_new"];
    assert_eq!(new_id, before.values().max().unwrap() + 1, "a new binding takes the next free id, not the sorted position");
    assert_eq!(second.lock_delta.changed.len(), 1, "the changed signature is reported: {:?}", second.lock_delta.changed);
    assert_eq!(second.lock_delta.changed[0].0, "mini::midpoint");

    // remove a function: its id stays reserved and is marked unbound
    let removed = fixture_source().replace("pub fn takes_char(c: char, s: &str, big: isize) -> String {", "pub fn takes_char_renamed(c: char, s: &str, big: isize) -> String {");
    std::fs::write(dir.join("mini_engine.rs"), removed).unwrap();
    let third = run_generator(&dir);
    let lock: yoake_bindgen::lock::Lock = serde_json::from_str(&third.files[&lock_path]).unwrap();
    let old = lock.bindings.iter().find(|e| e.path == "mini::takes_char").unwrap();
    assert_eq!(old.status, "unbound");
    assert_eq!(old.id, before["mini::takes_char"]);
    let renamed = lock.bindings.iter().find(|e| e.path == "mini::takes_char_renamed").unwrap();
    assert!(renamed.id > before.values().copied().max().unwrap(), "a renamed function is a new binding with a new id");
}
