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
