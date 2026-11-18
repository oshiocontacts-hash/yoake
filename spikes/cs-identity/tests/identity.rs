use cs_identity::{extract, extract_caller, records_from_sites, reidentify, reidentify_with_snapshots};

#[test]
fn insert_earlier_call_keeps_old_ids() {
    let before = "fn demo() { prep(); work(1); done(); }";
    let after = "fn demo() { work(0); prep(); work(1); done(); }";

    let mut next_id = 1;
    let old_records = records_from_sites(&extract(before, "demo"), &mut next_id);
    let new_sites = extract(after, "demo");
    let out = reidentify(&old_records, &new_sites, &mut next_id);

    assert_eq!(out.kept, vec![(1, 1), (2, 2), (3, 3)]);
    assert_eq!(out.new, vec![(4, 0)]);
    assert!(out.lost.is_empty());
    assert!(out.ambiguous.is_empty());
    assert!(out.fragile.is_empty());
}

#[test]
fn swap_statements_follows_calls() {
    let before = "fn demo() { prep(); f(1); f(2); done(); }";
    let after = "fn demo() { prep(); f(2); f(1); done(); }";

    let mut next_id = 1;
    let old_records = records_from_sites(&extract(before, "demo"), &mut next_id);
    let new_sites = extract(after, "demo");
    let out = reidentify(&old_records, &new_sites, &mut next_id);

    assert_eq!(out.kept, vec![(1, 0), (2, 2), (3, 1), (4, 3)]);
    assert!(out.new.is_empty());
    assert!(out.lost.is_empty());
    assert!(out.ambiguous.is_empty());
    assert!(out.fragile.is_empty());
}

#[test]
fn changed_args_matched_by_alignment() {
    let before = "fn demo() { let a = 1; f(10, 20); let b = 2; }";
    let after = "fn demo() { let a = 1; f(99, 100); let b = 2; }";

    let mut next_id = 1;
    let (old_sites, old_snap) = extract_caller(before, "demo");
    let old_records = records_from_sites(&old_sites, &mut next_id);
    let (new_sites, new_snap) = extract_caller(after, "demo");

    let mut next_snap = next_id;
    let out_snap =
        reidentify_with_snapshots(&old_records, &[old_snap], &new_sites, &[new_snap], &mut next_snap);
    assert_eq!(out_snap.kept, vec![(1, 0)]);
    assert!(out_snap.new.is_empty());
    assert!(out_snap.lost.is_empty());
    assert!(out_snap.ambiguous.is_empty());

    let out_plain = reidentify(&old_records, &new_sites, &mut next_id);
    assert_eq!(out_plain.kept, vec![(1, 0)]);
    assert!(out_plain.new.is_empty());
    assert!(out_plain.lost.is_empty());
}

#[test]
fn deleted_call_reported_lost() {
    let before = "fn demo() { prep(); f(1); done(); }";
    let after = "fn demo() { prep(); done(); }";

    let mut next_id = 1;
    let old_records = records_from_sites(&extract(before, "demo"), &mut next_id);
    let new_sites = extract(after, "demo");
    let out = reidentify(&old_records, &new_sites, &mut next_id);

    assert_eq!(out.kept, vec![(1, 0), (3, 1)]);
    assert_eq!(out.lost, vec![2]);
    assert!(out.new.is_empty());
    assert!(out.ambiguous.is_empty());
}

#[test]
fn identical_duplicates_insert_at_top_is_fragile() {
    let before = "fn demo() { f(1); f(1); }";
    let after = "fn demo() { f(1); f(1); f(1); }";

    let mut next_id = 1;
    let old_records = records_from_sites(&extract(before, "demo"), &mut next_id);
    let new_sites = extract(after, "demo");
    let out = reidentify(&old_records, &new_sites, &mut next_id);

    assert_eq!(out.kept, vec![(1, 0), (2, 1)]);
    assert_eq!(out.new, vec![(3, 2)]);
    assert_eq!(out.fragile, vec![1, 2]);
    assert!(out.lost.is_empty());
    assert!(out.ambiguous.is_empty());
}

#[test]
fn moved_to_another_fn_is_lost_in_old_caller() {
    let before = "fn first() { prep(); f(1); } fn second() {}";
    let after = "fn first() { prep(); } fn second() { f(1); }";

    let mut next_id = 1;
    let old_records = records_from_sites(&extract(before, "first"), &mut next_id);
    let new_sites = extract(after, "first");
    let out = reidentify(&old_records, &new_sites, &mut next_id);

    assert_eq!(out.kept, vec![(1, 0)]);
    assert_eq!(out.lost, vec![2]);
    assert!(out.new.is_empty());
    assert!(out.ambiguous.is_empty());
}

#[test]
fn comments_and_whitespace_change_nothing() {
    let before = "fn demo() { prep(); f(1, 2); if cond { g(3); } }";
    let after = r#"
        fn demo() {
            // leading comment
            prep();
            f(
                1, /* inline comment */
                2
            );
            if cond {
                // nested comment
                g(3);
            }
        }
    "#;

    let old_sites = extract(before, "demo");
    let new_sites = extract(after, "demo");
    assert_eq!(old_sites, new_sites);

    let mut next_id = 1;
    let old_records = records_from_sites(&old_sites, &mut next_id);
    let out = reidentify(&old_records, &new_sites, &mut next_id);
    assert_eq!(out.kept, vec![(1, 0), (2, 1), (3, 2)]);
    assert!(out.new.is_empty());
    assert!(out.lost.is_empty());
    assert!(out.ambiguous.is_empty());
    assert!(out.fragile.is_empty());
}

#[test]
fn nested_blocks_closures_and_chained_method_order() {
    let src = r#"
        struct S;
        impl S {
            fn demo(&self, cond: bool) {
                let items = [1, 2];
                if cond {
                    let _ = items.iter().map(|x| helper(*x));
                }
            }
        }
    "#;

    let sites = extract(src, "demo");
    let callees: Vec<&str> = sites.iter().map(|s| s.callee.as_str()).collect();
    assert_eq!(callees, vec![".iter", ".map", "helper"]);
    assert_eq!(sites[0].stmt_path, vec![1, 0]);
    assert_eq!(sites[0].stmt_path_hashes.len(), 2);
    assert!(!sites[0].in_closure);
    assert!(!sites[1].in_closure);
    assert!(sites[2].in_closure);
}

#[test]
fn one_old_two_new_same_anchor_is_ambiguous() {
    let before = "fn demo(c: bool) { if c { f(1); } }";
    let after = "fn demo(c: bool) { if c { f(1); } else { f(1); } }";

    let mut next_id = 1;
    let old_records = records_from_sites(&extract(before, "demo"), &mut next_id);
    let new_sites = extract(after, "demo");
    let out = reidentify(&old_records, &new_sites, &mut next_id);

    assert!(out.kept.is_empty());
    assert!(out.lost.is_empty());
    assert!(out.new.is_empty());
    assert_eq!(out.ambiguous, vec![(1, vec![0, 1])]);
}
