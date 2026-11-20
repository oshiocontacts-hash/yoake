//! W6: with feature `count`, every entry of a hooked function bumps its slot's counter.
#![cfg(feature = "count")]

use yoake_hook_macro::yoake_hook;
use yoake_hook_rt::{counts, reset_counts, TEST_ID_BASE};

#[yoake_hook(test_id = 52)]
fn counted(x: u32) -> u32 {
    x + 1
}

#[test]
fn entries_are_counted_and_reset() {
    reset_counts();
    for i in 0..5 {
        assert_eq!(counted(i), i + 1);
    }
    let slot = (TEST_ID_BASE + 52) as u32;
    assert_eq!(counts().into_iter().find(|&(id, _)| id == slot).map(|(_, n)| n), Some(5));
    reset_counts();
    assert!(counts().iter().all(|&(id, _)| id != slot));
}
