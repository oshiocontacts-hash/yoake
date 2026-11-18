use yoake_hook_rt::assert_ledger_key;

const KEY: &str = "crate::module::hook";
const KEYS: [(usize, &str); 10_000] = {
    let mut keys = [(0, KEY); 10_000];
    let mut index = 0;
    while index < keys.len() {
        keys[index].0 = index + 1;
        index += 1;
    }
    keys
};

const _: () = {
    assert_ledger_key(&KEYS, 1, KEY);
    assert_ledger_key(&KEYS, 5_000, KEY);
    assert_ledger_key(&KEYS, 10_000, KEY);
};

#[test]
fn finds_both_ends_and_the_middle_of_a_large_sorted_table() {
    assert_ledger_key(&KEYS, 1, KEY);
    assert_ledger_key(&KEYS, 5_000, KEY);
    assert_ledger_key(&KEYS, 10_000, KEY);
}

#[test]
fn rejects_the_reserved_legacy_id() {
    let message = panic_message(|| assert_ledger_key(&KEYS, 0, KEY));
    assert_eq!(
        message,
        "hook id 0 is reserved for the legacy single-export guest API"
    );
}

#[test]
fn rejects_ids_outside_the_table() {
    for id in [10_001, usize::MAX] {
        let message = panic_message(|| assert_ledger_key(&KEYS, id, KEY));
        assert_eq!(message, "hook id/key pair is absent from the crate ledger");
    }
}

#[test]
fn rejects_a_tombstoned_id_absent_from_a_key_gap() {
    const KEYS_WITH_TOMBSTONE: &[(usize, &str)] = &[(1, "first"), (3, "third")];
    let message = panic_message(|| assert_ledger_key(KEYS_WITH_TOMBSTONE, 2, "second"));
    assert_eq!(message, "hook id/key pair is absent from the crate ledger");
}

#[test]
fn still_rejects_a_mismatched_key_after_binary_search() {
    let message = panic_message(|| assert_ledger_key(&KEYS, 5_000, "crate::module::other"));
    assert_eq!(message, "hook id/key pair does not match the crate ledger");
}

fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe) -> String {
    let payload = std::panic::catch_unwind(f).expect_err("ledger key assertion should panic");
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        panic!("ledger key assertion panicked with a non-string payload")
    }
}
