use std::collections::BTreeSet;

use yoake_hook_thir::{Borrow, CallKind, ParseOptions, Resolution, Span, parse_str};

const SAMPLE: &str = include_str!("fixtures/thir-fixture.txt");

fn parse_sample() -> yoake_hook_thir::Dump {
    parse_str(
        SAMPLE,
        &ParseOptions {
            files: vec!["lib.rs".into()],
            externs: BTreeSet::new(),
            crate_name: Some("thir_fixture".into()),
            toolchain: "fixture".into(),
            workspace: None,
            file_digests: Default::default(),
        },
    )
    .unwrap()
}

#[test]
fn extracts_held_call_and_box_receiver_adjustment() {
    let dump = parse_sample();
    assert_eq!(dump.crate_name, "thir_fixture");
    assert_eq!(dump.files["lib.rs"].written_calls, 17);
    let call = dump
        .calls
        .iter()
        .find(|call| {
            call.caller_def == "thir_fixture::{impl#3}::held"
                && call.callee == "thir_fixture::Stack::first"
        })
        .unwrap();
    assert_eq!(call.kind, CallKind::Method);
    assert_eq!(call.resolution, Resolution::Concrete);
    assert_eq!(call.span, Span(46, 9, 46, 27));
    assert_eq!(call.fun_span, Span(46, 20, 46, 25));
    assert_eq!(call.args.len(), 1);
    assert_eq!(call.args[0].ty.as_deref(), Some("&'_ crate::Stack"));
    assert_eq!(call.args[0].span, Span(46, 9, 46, 19));
    assert_eq!(call.ret_ty.as_deref(), Some("std::option::Option<&'_ u32>"));
    let adjustment = call.recv_adjust.unwrap();
    assert_eq!(adjustment.borrow, Borrow::Shared);
    assert_eq!(adjustment.derefs, 1);
}

#[test]
fn classifies_operators_macro_expansions_and_closure_arguments() {
    let dump = parse_sample();
    let methods = dump.calls.iter().filter(|call| call.kind == CallKind::Method).count();
    let paths = dump.calls.iter().filter(|call| call.kind == CallKind::Path).count();
    let overloaded = dump
        .calls
        .iter()
        .filter(|call| call.kind == CallKind::Overloaded)
        .count();
    assert_eq!((methods, paths, overloaded), (17, 1, 5));
    let add = dump
        .calls
        .iter()
        .find(|call| call.callee_def == "core::ops::arith::Add::add")
        .unwrap();
    assert_eq!(add.kind, CallKind::Overloaded);
    assert_eq!(add.callee, "<Self as core::ops::arith::Add>::add");
    let panic = dump
        .calls
        .iter()
        .find(|call| call.callee_def == "core::panicking::panic")
        .unwrap();
    assert_ne!(panic.ctxt, 0);
    let map = dump
        .calls
        .iter()
        .find(|call| call.callee_def == "core::iter::traits::iterator::Iterator::map")
        .unwrap();
    assert_eq!(map.args.len(), 2);
    assert_eq!(map.args[1].ty, None);
    assert_eq!(map.ret_ty, None);
    let json = serde_json::to_value(map).unwrap();
    assert!(json.get("recv_adjust").is_some());
    assert!(json["args"][1]["ty"].is_null());
}

#[test]
fn rejects_a_dump_without_call_spans() {
    let error = parse_str(
        "DefId(0:1 ~ empty[abcd]::f):\nparams: []\nbody:\n",
        &ParseOptions {
            files: vec!["lib.rs".into()],
            ..ParseOptions::default()
        },
    )
    .unwrap_err();
    assert!(error.to_string().contains("no THIR Call node"));
}

#[test]
fn computes_file_sha_len_and_source_snapshot_for_listed_file() {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "yoake_thir_snapshot_{}_{}",
        std::process::id(),
        nonce
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let source = b"pub fn alpha() {}\npub fn beta() { alpha(); }\n";
    std::fs::write(dir.join("src/mini.rs"), source).unwrap();

    let tiny_thir = "\
DefId(0:1 ~ mini[abcd]::beta):
body:
    Expr {
        ty: ()
        span: src/mini.rs:2:33: 2:42 (#0)
        kind:
            Call {
                ty: FnDef(DefId(0:2 ~ mini[abcd]::alpha), [])
                from_hir_call: true
                fn_span: src/mini.rs:2:33: 2:42 (#0)
                fun:
                    Expr {
                        ty: FnDef(DefId(0:2 ~ mini[abcd]::alpha), [])
                        span: src/mini.rs:2:33: 2:38 (#0)
                    }
                args: [
                ]
            }
    }
";
    let dump = parse_str(
        tiny_thir,
        &ParseOptions {
            files: vec!["src/mini.rs".into()],
            externs: BTreeSet::new(),
            crate_name: Some("mini".into()),
            toolchain: "fixture".into(),
            workspace: Some(dir.clone()),
            ..ParseOptions::default()
        },
    )
    .unwrap();
    let summary = &dump.files["src/mini.rs"];
    assert_eq!(summary.written_calls, 1);
    assert_eq!(summary.len, source.len() as u64);
    assert_eq!(
        summary.sha,
        yoake_hook_thir::format_hash(yoake_hook_thir::fnv1a_64(source))
    );
    assert_eq!(summary.sha.len(), 16);
    assert_eq!(
        dump.source_snapshot,
        yoake_hook_thir::compute_source_snapshot(&dump.files)
    );
    assert_eq!(dump.source_snapshot.len(), 16);

    let json = serde_json::to_value(&dump).unwrap();
    assert_eq!(json["files"]["src/mini.rs"]["sha"], summary.sha);
    assert_eq!(json["files"]["src/mini.rs"]["len"], source.len() as u64);
    assert_eq!(json["source_snapshot"], dump.source_snapshot);

    let _ = std::fs::remove_dir_all(&dir);
}
