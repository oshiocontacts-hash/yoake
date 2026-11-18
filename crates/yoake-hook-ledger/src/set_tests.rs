use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::{Kind, LedgerSet, check_append_only};

static TEMP_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

fn temp_dir() -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "yoake-hook-ledger-test-{}-{sequence}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

#[test]
fn two_shards_cannot_share_an_id() {
    let dir = temp_dir();
    std::fs::write(dir.join("meta.json"), r#"{"version":2,"next_id":2}"#).unwrap();
    for crate_name in ["a", "b"] {
        let shard = format!(
            r#"{{"crate":"{crate_name}","entries":[{{"id":1,"key":"{crate_name}::f","sig":"x","status":"live"}}],"excluded":[]}}"#
        );
        std::fs::write(dir.join(format!("{crate_name}.json")), shard).unwrap();
    }
    let error = LedgerSet::load(&dir).unwrap_err().to_string();
    assert!(error.contains("shared by crates"), "{error}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn set_save_and_load_preserves_global_ids() {
    let dir = temp_dir();
    let mut set = LedgerSet::new();
    let first = set.assign("a", Kind::Fn, "a::f", "one");
    let second = set.assign("b", Kind::Fn, "b::f", "two");
    assert_eq!((first.id, second.id), (1, 2));
    set.save(&dir).unwrap();
    let loaded = LedgerSet::load(&dir).unwrap();
    assert_eq!(loaded.next_id(), 3);
    assert_eq!(loaded.shard("a").unwrap().live_id("a::f"), Some(1));
    assert_eq!(loaded.shard("b").unwrap().live_id("b::f"), Some(2));
    std::fs::remove_dir_all(dir).unwrap();
}

fn history_base() -> LedgerSet {
    let mut base = LedgerSet::new();
    base.assign("a", Kind::Fn, "a::first", "one");
    base.assign("a", Kind::Fn, "a::dead", "two");
    base.shard_mut("a").tombstone("a::dead").unwrap();
    base
}

fn history_errors(base: &LedgerSet, current: &LedgerSet) -> String {
    check_append_only(base, current).unwrap_err().join("\n")
}

#[test]
fn append_only_rejects_a_changed_key() {
    let base = history_base();
    let mut current = base.clone();
    current.shard_mut("a").entries[0].key = "a::changed".into();
    assert!(history_errors(&base, &current).contains("changed key"));
}

#[test]
fn append_only_rejects_a_removed_tombstone() {
    let base = history_base();
    let mut current = base.clone();
    current.shard_mut("a").entries.retain(|entry| entry.id != 2);
    assert!(history_errors(&base, &current).contains("id 2"));
}

#[test]
fn append_only_rejects_decreased_next_id() {
    let base = history_base();
    let mut current = base.clone();
    current.meta.next_id -= 1;
    assert!(history_errors(&base, &current).contains("next_id decreased"));
}

#[test]
fn append_only_rejects_an_id_moved_between_crates() {
    let base = history_base();
    let mut current = base.clone();
    let entry = current.shard_mut("a").entries.remove(0);
    current.shard_mut("b").entries.push(entry);
    assert!(history_errors(&base, &current).contains("moved from a to b"));
}

#[test]
fn append_only_accepts_a_recorded_rename() {
    let base = history_base();
    let mut current = base.clone();
    current
        .shard_mut("a")
        .rename("a::first", "a::renamed")
        .unwrap();
    check_append_only(&base, &current).unwrap();
    assert_eq!(
        current
            .shard("a")
            .unwrap()
            .live_id_including_renames("a::first"),
        Some(1)
    );
}

#[test]
fn append_only_rejects_reusing_a_gap_id() {
    let mut base = LedgerSet::new();
    base.assign("a", Kind::Fn, "a::first", "one");
    base.assign("a", Kind::Fn, "a::gap", "two");
    base.shard_mut("a").entries.retain(|entry| entry.id != 2);
    let mut current = base.clone();
    let mut gap = current.shard("a").unwrap().entries[0].clone();
    gap.id = 2;
    gap.key = "a::new_gap_user".into();
    current.shard_mut("a").entries.push(gap);
    assert!(history_errors(&base, &current).contains("reuses a gap"));
}

#[test]
fn append_only_rejects_losing_rename_history() {
    let mut base = LedgerSet::new();
    base.assign("a", Kind::Fn, "a::old", "one");
    base.shard_mut("a").rename("a::old", "a::middle").unwrap();
    base.shard_mut("a").rename("a::middle", "a::new").unwrap();
    let mut current = base.clone();
    current.shard_mut("a").entries[0].renamed_from.pop();
    assert!(history_errors(&base, &current).contains("lost renamed_from"));
}

#[test]
fn append_only_rejects_changing_entry_kind() {
    let base = history_base();
    let mut current = base.clone();
    current.shard_mut("a").entries[0].kind = Kind::Store;
    assert!(history_errors(&base, &current).contains("changed kind"));
}

#[test]
fn append_only_rejects_changing_excluded_kind() {
    let mut base = LedgerSet::new();
    base.shard_mut("a").exclude("a::f", "manual").unwrap();
    let mut current = base.clone();
    current.shard_mut("a").excluded[0].kind = Kind::Store;
    assert!(history_errors(&base, &current).contains("excluded key"));
}

#[test]
fn automatic_exclusion_does_not_overwrite_manual_reason() {
    let mut set = LedgerSet::new();
    let shard = set.shard_mut("a");
    shard.exclude_manual(Kind::Fn, "a::f", "keep this").unwrap();
    shard.exclude("a::f", "automatic replacement").unwrap();
    assert_eq!(shard.excluded()[0].reason, "keep this");
    assert!(shard.excluded()[0].manual);
}
