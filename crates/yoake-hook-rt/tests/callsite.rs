use yoake_hook_rt::{CallsiteHook, Order, TEST_ID_BASE, register};

fn callee(left: u32, right: u32) -> u32 {
    left * 10 + right
}

fn call_twice(value: u32) -> u32 {
    let first = yoake_hook_rt::callsite!(test_id = 20, callee, (value, 2));
    first + callee(value, 3)
}

#[test]
fn one_callsite_wraps_replaces_and_skips_without_affecting_the_other_call() {
    type Hook = CallsiteHook<(u32, u32), u32>;
    const SLOT: usize = TEST_ID_BASE + 20;

    assert_eq!(call_twice(1), 25);

    let wrap: Hook = |original, args| original(args) + 100;
    register(SLOT, wrap, Order::default()).unwrap();
    assert_eq!(call_twice(1), 125);

    let replace: Hook = |original, _args| original((5, 6));
    register(SLOT, replace, Order::default()).unwrap();
    assert_eq!(call_twice(1), 169);

    let skip: Hook = |_original, _args| 999;
    register(
        SLOT,
        skip,
        Order {
            priority: 10,
            mod_load_order: 0,
        },
    )
    .unwrap();
    assert_eq!(call_twice(1), 1012);
}
