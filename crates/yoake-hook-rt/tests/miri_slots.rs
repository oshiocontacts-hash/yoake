//! Miri-only tests of the hook slots (MX3MIRI): publication of chains, snapshots that outlive a republish,
//! typed registration, concurrent registration. Compiled only under Miri.
#![cfg(miri)]

use yoake_hook_rt::{CallsiteHook, Order, Slot, SLOTS, TEST_ID_BASE, __register_typed};

type Hook = CallsiteHook<u32, u32>;

fn register(slot: usize, hook: Hook, order: Order) -> Result<(), &'static str> {
    __register_typed(slot, std::any::TypeId::of::<Hook>(), hook, order)
}

fn run(slot: &'static Slot, value: u32) -> u32 {
    match slot.load() {
        Some(chain) => yoake_hook_rt::callsite_dispatch(chain, value, &mut |v| v + 1),
        None => value + 1,
    }
}

#[test]
fn miri_an_old_snapshot_stays_readable_after_a_republish() {
    let id = TEST_ID_BASE + 40;
    assert!(SLOTS[id].load().is_none());
    register(id, |original, v| original(v) + 100, Order::default()).unwrap();
    let old = SLOTS[id].load().unwrap();
    // republish twice: the chain `old` points into is superseded (and intentionally leaked)
    register(id, |original, v| original(v) * 2, Order { priority: 5, mod_load_order: 0 }).unwrap();
    register(id, |_, _| 7, Order { priority: -5, mod_load_order: 1 }).unwrap();
    let (first, rest) = old.next::<Hook>().unwrap();
    assert!(rest.next::<Hook>().is_none());
    let mut orig = |v: u32| v + 1;
    assert_eq!(first(&mut orig, 1), 102);
    // the new chain: x2 (priority 5) around +100 (priority 0); the priority -5 hook ignores `original` and returns 7
    assert_eq!(run(&SLOTS[id], 1), 214);
}

#[test]
fn miri_registration_is_typed_and_bounded() {
    let id = TEST_ID_BASE + 41;
    register(id, |o, v| o(v), Order::default()).unwrap();
    type Other = fn(u8) -> u8;
    let other: Other = |v| v;
    assert!(__register_typed(id, std::any::TypeId::of::<Other>(), other, Order::default()).is_err());
    assert!(__register_typed(1 << 20, std::any::TypeId::of::<Other>(), other, Order::default()).is_err());
    for _ in 1..64 {
        register(id, |o, v| o(v), Order::default()).unwrap();
    }
    assert!(register(id, |o, v| o(v), Order::default()).is_err());
}

#[test]
fn miri_concurrent_registration_and_reads_do_not_race() {
    let id = TEST_ID_BASE + 42;
    let writers: Vec<_> = (0..2)
        .map(|n| {
            std::thread::spawn(move || {
                for k in 0..4 {
                    register(id, |o, v| o(v) + 1, Order { priority: n, mod_load_order: k }).unwrap();
                }
            })
        })
        .collect();
    let reader = std::thread::spawn(move || {
        let mut last = 0;
        for _ in 0..8 {
            last = run(&SLOTS[id], 0);
        }
        last
    });
    for w in writers {
        w.join().unwrap();
    }
    reader.join().unwrap();
    assert_eq!(run(&SLOTS[id], 0), 8 + 1);
}
