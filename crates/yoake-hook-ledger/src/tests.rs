use super::{Kind, Ledger, Status};

const CANONICAL: &str = r#"{
  "version": 1,
  "next_id": 3,
  "entries": [
    {
      "id": 1,
      "key": "demo::first",
      "sig": "0123456789abcdef",
      "status": "live",
      "kind": "fn"
    },
    {
      "id": 2,
      "key": "demo::second",
      "sig": "fedcba9876543210",
      "status": "tombstone",
      "kind": "fn",
      "renamed_from": [
        "demo::old_second"
      ]
    }
  ],
  "excluded": [
    {
      "key": "demo::unsupported",
      "reason": "async",
      "kind": "fn"
    }
  ]
}
"#;

#[test]
fn assigning_twice_and_adding_keys_preserves_ids() {
    let mut ledger = Ledger::new();
    let first = ledger.assign(Kind::Fn, "demo::first", "one");
    assert_eq!(first.id, 1);
    assert!(first.new);
    assert_eq!(ledger.assign(Kind::Fn, "demo::first", "one").id, first.id);
    let second = ledger.assign(Kind::Fn, "demo::second", "two");
    assert_eq!(second.id, 2);
    assert_eq!(ledger.assign(Kind::Fn, "demo::first", "one").id, first.id);
    assert_eq!(ledger.max_slots(), 3);
}

#[test]
fn rename_keeps_id_and_records_history() {
    let mut ledger = Ledger::new();
    ledger.assign(Kind::Fn, "demo::old", "signature");
    let id = ledger.rename("demo::old", "demo::new").unwrap();
    assert_eq!(id, 1);
    assert_eq!(ledger.live_id("demo::old"), None);
    assert_eq!(ledger.live_id("demo::new"), Some(id));
    let entry = ledger.live().next().unwrap();
    assert_eq!(entry.renamed_from, ["demo::old"]);
}

#[test]
fn tombstoned_ids_are_never_reused() {
    let mut ledger = Ledger::new();
    ledger.assign(Kind::Fn, "demo::old", "signature");
    assert_eq!(ledger.tombstone("demo::old").unwrap(), 1);
    assert_eq!(ledger.live_id("demo::old"), None);
    assert_eq!(ledger.entries[0].status, Status::Tombstone);
    let assigned = ledger.assign(Kind::Fn, "demo::old", "signature");
    assert_eq!(assigned.id, 2);
    assert!(assigned.new);
}

#[test]
fn signature_changes_keep_the_id_and_are_reported() {
    let mut ledger = Ledger::new();
    ledger.assign(Kind::Fn, "demo::function", "old");
    let assigned = ledger.assign(Kind::Fn, "demo::function", "new");
    assert_eq!(assigned.id, 1);
    assert!(!assigned.new);
    assert!(assigned.sig_changed);
    assert!(!ledger.assign(Kind::Fn, "demo::function", "new").sig_changed);
}

fn invalid(json: &str, expected: &str) {
    let error = Ledger::from_json(json).unwrap_err().to_string();
    assert!(error.contains(expected), "{error}");
}

#[test]
fn validation_rejects_bad_ids_and_duplicate_live_keys() {
    invalid(
        r#"{"version":1,"next_id":3,"entries":[{"id":1,"key":"a","sig":"","status":"live"},{"id":1,"key":"b","sig":"","status":"live"}],"excluded":[]}"#,
        "duplicate id",
    );
    invalid(
        r#"{"version":1,"next_id":2,"entries":[{"id":0,"key":"a","sig":"","status":"live"}],"excluded":[]}"#,
        "outside",
    );
    invalid(
        r#"{"version":1,"next_id":2,"entries":[{"id":2,"key":"a","sig":"","status":"live"}],"excluded":[]}"#,
        "outside",
    );
    invalid(
        r#"{"version":1,"next_id":3,"entries":[{"id":1,"key":"a","sig":"","status":"live"},{"id":2,"key":"a","sig":"","status":"live"}],"excluded":[]}"#,
        "live key",
    );
}

#[test]
fn validation_rejects_live_and_excluded_key() {
    invalid(
        r#"{"version":1,"next_id":2,"entries":[{"id":1,"key":"a","sig":"","status":"live"}],"excluded":[{"key":"a","reason":"no"}]}"#,
        "both live",
    );
}

#[test]
fn canonical_json_round_trips_exactly() {
    let ledger = Ledger::from_json(CANONICAL).unwrap();
    assert_eq!(ledger.to_json(), CANONICAL);
    assert_eq!(Ledger::from_json(&ledger.to_json()).unwrap().to_json(), CANONICAL);
}

#[test]
fn assigning_a_previously_excluded_key_makes_it_live() {
    let mut ledger = Ledger::new();
    ledger.exclude("demo::function", "old reason").unwrap();
    ledger.assign(Kind::Fn, "demo::function", "signature");
    assert!(ledger.excluded().is_empty());
    ledger.validate().unwrap();
}

#[test]
fn missing_kind_defaults_to_fn_and_is_serialized() {
    let legacy = r#"{"version":1,"next_id":2,"entries":[{"id":1,"key":"demo::f","sig":"x","status":"live"}],"excluded":[{"key":"demo::x","reason":"async"}]}"#;
    let ledger = Ledger::from_json(legacy).unwrap();
    assert_eq!(ledger.live().next().unwrap().kind, Kind::Fn);
    assert_eq!(ledger.excluded()[0].kind, Kind::Fn);
    let json = ledger.to_json();
    assert!(json.contains("\"kind\": \"fn\""));
    assert_eq!(Ledger::from_json(&json).unwrap().to_json(), json);
}

#[test]
fn callsite_and_store_kinds_round_trip() {
    let mut ledger = Ledger::new();
    ledger.assign(Kind::Callsite, "demo::f@call:demo::g#0", "call");
    ledger.assign(Kind::Store, "demo::f@store:self.value#0", "store");
    ledger.exclude_kind(Kind::Store, "demo::f@store:self.other#0", "later")
        .unwrap();
    let json = ledger.to_json();
    assert!(json.contains("\"kind\": \"callsite\""));
    assert!(json.contains("\"kind\": \"store\""));
    let parsed = Ledger::from_json(&json).unwrap();
    let kinds: Vec<_> = parsed.live().map(|entry| entry.kind).collect();
    assert_eq!(kinds, [Kind::Callsite, Kind::Store]);
    assert_eq!(parsed.excluded()[0].kind, Kind::Store);
}

#[test]
fn callsite_site_round_trips_and_is_omitted_for_other_entries() {
    let mut ledger = Ledger::new();
    ledger.assign(Kind::Callsite, "fn:103@call:fn:2207", "call");
    let site = super::CallSite {
        caller: 103,
        callee_fn: Some(2207),
        callee_path: "horizon_client::game::Game::held".into(),
        anchor: "0123456789abcdef".into(),
        stmt_path: vec!["9c1e".into(), "02ab".into()],
        context: ["77d0".into(), "0000".into()],
        dup_rank: 0,
        shim: "__yoake_cs_1".into(),
    };
    ledger.entries[0].site = Some(site.clone());
    let json = ledger.to_json();
    assert!(json.contains("\"callee_fn\": 2207"));
    let parsed = Ledger::from_json(&json).unwrap();
    assert_eq!(parsed.entries[0].site, Some(site));
    assert_eq!(parsed.to_json(), json);

    let mut function = Ledger::new();
    function.assign(Kind::Fn, "demo::f", "sig");
    assert!(!function.to_json().contains("\"site\""));
    let without_callee = json.replace("\"callee_fn\": 2207", "\"callee_fn\": null");
    let parsed = Ledger::from_json(&without_callee).unwrap();
    assert_eq!(parsed.entries[0].site.as_ref().unwrap().callee_fn, None);
}

#[test]
fn live_keys_are_unique_even_across_kinds() {
    invalid(
        r#"{"version":1,"next_id":3,"entries":[{"id":1,"key":"a","sig":"","status":"live","kind":"fn"},{"id":2,"key":"a","sig":"","status":"live","kind":"callsite"}],"excluded":[]}"#,
        "live key",
    );
}

#[test]
fn signature_hash_collapses_whitespace() {
    assert_eq!(Ledger::sig_hash(""), "cbf29ce484222325");
    assert_eq!(Ledger::sig_hash("a"), "af63dc4c8601ec8c");
    let expected = Ledger::sig_hash("fn example ( value : u32 )");
    assert_eq!(Ledger::sig_hash("fn  example ( value : u32 )"), expected);
    assert_eq!(Ledger::sig_hash("\n fn\texample (\n value : u32 ) \t"), expected);
}
