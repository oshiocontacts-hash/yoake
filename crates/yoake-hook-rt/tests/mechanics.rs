use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use yoake_hook_macro::yoake_hook;
use yoake_hook_rt::{MAX_SLOTS, Order, SLOTS, register};

#[test]
fn zero_calls_and_early_return_skip_the_original_body() {
    #[yoake_hook(id = 1)]
    fn never_called() -> u32 {
        panic!("around hook should bypass the body")
    }
    type Hook = fn(&mut dyn FnMut() -> u32) -> u32;
    register::<Hook>(__YOAKE_HOOK_ID_never_called, |_original| 7, Order::default()).unwrap();
    assert_eq!(never_called(), 7);
}

#[test]
fn one_call_replaces_arguments_and_transforms_result() {
    #[yoake_hook(id = 2)]
    fn add(left: u32, right: u32) -> u32 {
        left + right
    }
    assert_eq!(add(1, 2), 3);
    type Hook = fn(&mut dyn FnMut(u32, u32) -> u32, u32, u32) -> u32;
    register::<Hook>(
        __YOAKE_HOOK_ID_add,
        |original, _left, _right| original(10, 20) * 2,
        Order::default(),
    )
    .unwrap();
    assert_eq!(add(1, 2), 60);
}

#[test]
fn two_calls_of_original_run_the_rest_of_the_chain_twice() {
    static BODY_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[yoake_hook(id = 3)]
    fn compute(value: u32) -> u32 {
        BODY_CALLS.fetch_add(1, Ordering::Relaxed);
        value + 1
    }
    type Hook = fn(&mut dyn FnMut(u32) -> u32, u32) -> u32;
    register::<Hook>(
        __YOAKE_HOOK_ID_compute,
        |original, value| original(value) + original(value + 10),
        Order {
            priority: 10,
            mod_load_order: 0,
        },
    )
    .unwrap();
    register::<Hook>(
        __YOAKE_HOOK_ID_compute,
        |original, value| original(value * 2),
        Order {
            priority: 0,
            mod_load_order: 0,
        },
    )
    .unwrap();
    assert_eq!(compute(5), 42);
    assert_eq!(BODY_CALLS.load(Ordering::Relaxed), 2);
}

#[test]
fn around_hooks_observe_question_mark_and_explicit_returns() {
    #[yoake_hook(id = 4)]
    fn checked(input: Result<u32, u32>) -> Result<u32, u32> {
        let value = input?;
        if value == 0 {
            return Ok(10);
        }
        Ok(value)
    }
    type Hook = fn(
        &mut dyn FnMut(Result<u32, u32>) -> Result<u32, u32>,
        Result<u32, u32>,
    ) -> Result<u32, u32>;
    register::<Hook>(
        __YOAKE_HOOK_ID_checked,
        |original, input| original(input).map(|v| v + 1).map_err(|e| e + 2),
        Order::default(),
    )
    .unwrap();
    assert_eq!(checked(Err(3)), Err(5));
    assert_eq!(checked(Ok(0)), Ok(11));
    assert_eq!(checked(Ok(4)), Ok(5));
}

#[test]
fn chain_order_is_priority_then_load_then_insertion_and_early_return_stops() {
    #[yoake_hook(id = 5)]
    fn digits(value: u32) -> u32 {
        value * 10 + 9
    }
    type Hook = fn(&mut dyn FnMut(u32) -> u32, u32) -> u32;
    register::<Hook>(
        __YOAKE_HOOK_ID_digits,
        |original, value| original(value * 10 + 4),
        Order {
            priority: -1,
            mod_load_order: 0,
        },
    )
    .unwrap();
    register::<Hook>(
        __YOAKE_HOOK_ID_digits,
        |original, value| original(value * 10 + 2),
        Order {
            priority: 10,
            mod_load_order: 2,
        },
    )
    .unwrap();
    register::<Hook>(
        __YOAKE_HOOK_ID_digits,
        |original, value| original(value * 10 + 1),
        Order {
            priority: 10,
            mod_load_order: 1,
        },
    )
    .unwrap();
    register::<Hook>(
        __YOAKE_HOOK_ID_digits,
        |_original, value| value * 10 + 3,
        Order {
            priority: 10,
            mod_load_order: 2,
        },
    )
    .unwrap();
    assert_eq!(digits(0), 123);
}

struct Counter(u32);

impl Counter {
    #[yoake_hook(method, id = 6)]
    fn increment(&mut self, amount: u32) -> u32 {
        self.0 += amount;
        self.0
    }

    #[yoake_hook(method, id = 7)]
    fn read(&self) -> u32 {
        self.0
    }
}

#[test]
fn borrowed_mut_self_method_can_be_hooked_and_reborrowed() {
    type IncrementHook = fn(&mut dyn FnMut(&mut Counter, u32) -> u32, &mut Counter, u32) -> u32;
    type ReadHook = fn(&mut dyn FnMut(&Counter) -> u32, &Counter) -> u32;
    let inc: IncrementHook = |original, receiver, _amount| {
        receiver.0 = 40;
        original(receiver, 2)
    };
    let read: ReadHook = |original, receiver| original(receiver) + 1;
    register(Counter::__YOAKE_HOOK_ID_increment, inc, Order::default()).unwrap();
    register(Counter::__YOAKE_HOOK_ID_read, read, Order::default()).unwrap();
    let mut counter = Counter(0);
    assert_eq!(counter.increment(1), 42);
    assert_eq!(counter.read(), 43);
}

#[test]
fn borrowed_str_arg_and_shared_lifetimes_work_without_static_data() {
    #[yoake_hook(id = 8)]
    fn length<'input>(value: &'input str, run: &mut dyn FnMut(usize)) -> usize {
        run(value.len());
        value.len()
    }
    type LengthHook = fn(
        &mut dyn FnMut(&str, &mut dyn FnMut(usize)) -> usize,
        &str,
        &mut dyn FnMut(usize),
    ) -> usize;
    let callback: LengthHook = |original, _value, run| original("replacement", run);
    register(__YOAKE_HOOK_ID_length, callback, Order::default()).unwrap();
    let input = String::from("short");
    let mut observed = 0;
    assert_eq!(length(&input, &mut |value| observed = value), 11);
    assert_eq!(observed, 11);

    #[yoake_hook(id = 9)]
    fn pair_len<'a>(left: &'a str, right: &'a str) -> usize {
        left.len() + right.len()
    }
    type PairHook = for<'a> fn(
        &mut (dyn for<'x> FnMut(&'x str, &'x str) -> usize),
        &'a str,
        &'a str,
    ) -> usize;
    let pair_cb: PairHook = |original, left, _right| original(left, "abc");
    register(__YOAKE_HOOK_ID_pair_len, pair_cb, Order::default()).unwrap();
    let a = String::from("hi");
    let b = String::from("longer");
    assert_eq!(pair_len(&a, &b), 5);
}

#[test]
fn reentry_from_inside_a_hook_runs_the_chain_independently() {
    static HOOK_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[yoake_hook(id = 10)]
    fn countdown(n: u32) -> u32 {
        n
    }
    type Hook = fn(&mut dyn FnMut(u32) -> u32, u32) -> u32;
    let hook: Hook = |original, n| {
        HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
        if n == 0 {
            original(0)
        } else {
            original(n) + countdown(n - 1)
        }
    };
    register(__YOAKE_HOOK_ID_countdown, hook, Order::default()).unwrap();
    assert_eq!(countdown(3), 6);
    assert_eq!(HOOK_CALLS.load(Ordering::Relaxed), 4);
}

static INLINE_FIRES: AtomicUsize = AtomicUsize::new(0);

#[yoake_hook(id = 11)]
#[inline(always)]
fn inline_leaf(value: u64) -> u64 {
    value + 1
}

#[test]
fn inline_always_body_keeps_its_guard() {
    assert_eq!(inline_leaf(4), 5);
    type Hook = fn(&mut dyn FnMut(u64) -> u64, u64) -> u64;
    register::<Hook>(
        __YOAKE_HOOK_ID_inline_leaf,
        |original, value| {
            INLINE_FIRES.fetch_add(1, Ordering::Relaxed);
            original(value)
        },
        Order::default(),
    )
    .unwrap();
    for value in 0..100 {
        assert_eq!(inline_leaf(value), value + 1);
    }
    assert_eq!(INLINE_FIRES.load(Ordering::Relaxed), 100);
}

#[test]
fn generic_slot_is_shared_but_dispatch_is_typed() {
    #[yoake_hook(id = 12)]
    fn identity<Value: 'static>(value: Value) -> Value {
        value
    }
    type U32Hook = fn(&mut dyn FnMut(u32) -> u32, u32) -> u32;
    type StringHook = fn(&mut dyn FnMut(String) -> String, String) -> String;
    register::<U32Hook>(
        __YOAKE_HOOK_ID_identity,
        |original, value| original(value + 10),
        Order::default(),
    )
    .unwrap();
    register::<StringHook>(
        __YOAKE_HOOK_ID_identity,
        |original, value| {
            let mut out = original(value);
            out.push('!');
            out
        },
        Order::default(),
    )
    .unwrap();
    assert_eq!(identity(1_u32), 11);
    assert_eq!(identity(2_u64), 2);
    assert_eq!(identity(String::from("typed")), "typed!");

    #[yoake_hook(id = 13)]
    fn constant<const VALUE: usize>() -> usize {
        VALUE
    }
    type ConstHook = fn(&mut dyn FnMut() -> usize) -> usize;
    register::<ConstHook>(
        __YOAKE_HOOK_ID_constant,
        |original| original() + 100,
        Order::default(),
    )
    .unwrap();
    assert_eq!(constant::<1>(), 101);
    assert_eq!(constant::<2>(), 102);
}

#[test]
fn concurrent_publication_keeps_old_snapshots_alive_and_bounds_registrations() {
    const SLOT_ID: usize = 14;
    type Callback = fn(&mut dyn FnMut());
    let noop: Callback = |original| original();
    register(SLOT_ID, noop, Order::default()).unwrap();
    let old = SLOTS[SLOT_ID].load().unwrap();
    std::thread::scope(|scope| {
        for _index in 0..7 {
            scope.spawn(|| {
                for _registration in 0..9 {
                    register(SLOT_ID, noop, Order::default()).unwrap();
                }
            });
        }
    });
    let mut old_count = 0;
    let mut rest = old;
    while let Some((_cb, next)) = rest.next::<Callback>() {
        old_count += 1;
        rest = next;
    }
    assert_eq!(old_count, 1);

    let mut current_count = 0;
    let mut current = SLOTS[SLOT_ID].load().unwrap();
    while let Some((_cb, next)) = current.next::<Callback>() {
        current_count += 1;
        current = next;
    }
    assert_eq!(current_count, 64);
    assert!(register(SLOT_ID, noop, Order::default()).is_err());
    assert!(register(MAX_SLOTS, noop, Order::default()).is_err());

    fn zst_fn(_: &mut dyn FnMut()) {}
    assert!(register(15, zst_fn, Order::default()).is_err());
}

fn find_matching_paren(src: &str, open_idx: usize) -> usize {
    let bytes = src.as_bytes();
    let mut depth = 0usize;
    let mut i = open_idx;
    let mut in_string = false;
    let mut escape = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    panic!("unclosed yoake_hook(");
}

fn extract_next_fn_name(src: &str, mut pos: usize) -> String {
    let bytes = src.as_bytes();
    while pos < bytes.len() {
        if bytes[pos..].starts_with(b"//") {
            while pos < bytes.len() && bytes[pos] != b'\n' {
                pos += 1;
            }
            continue;
        }
        if bytes[pos..].starts_with(b"fn")
            && pos + 2 < bytes.len()
            && bytes[pos + 2].is_ascii_whitespace()
            && (pos == 0 || !(bytes[pos - 1].is_ascii_alphanumeric() || bytes[pos - 1] == b'_'))
        {
            let mut start = pos + 2;
            while start < bytes.len() && bytes[start].is_ascii_whitespace() {
                start += 1;
            }
            let mut end = start;
            while end < bytes.len()
                && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_')
            {
                end += 1;
            }
            return src[start..end].to_string();
        }
        pos += 1;
    }
    panic!("no fn following yoake_hook attribute");
}

#[test]
fn entities_hook_ids_match_lock_file() {
    let entities = include_str!("../../horizon-client/src/entities.rs");
    let lock_json = include_str!("../hook-ids.lock.json");

    let mut from_entities = BTreeMap::new();
    let mut search = 0usize;
    while let Some(offset) = entities[search..].find("yoake_hook(") {
        let open_paren = search + offset + "yoake_hook".len();
        let close_paren = find_matching_paren(entities, open_paren);
        let args = &entities[open_paren + 1..close_paren];
        search = close_paren + 1;
        if args.contains("skip") {
            continue;
        }
        let mut found_id = None;
        for part in args.split(',') {
            let trimmed = part.trim();
            if let Some(rest) = trimmed.strip_prefix("id") {
                let rest = rest.trim_start();
                if let Some(num) = rest.strip_prefix('=') {
                    let id: usize = num.trim().parse().expect("valid hook id");
                    assert!(found_id.replace(id).is_none(), "duplicate id in attr");
                }
            }
        }
        let id = found_id.expect("hook attribute missing id");
        assert!(id != 0, "slot 0 must stay unused");
        assert!(id < MAX_SLOTS, "hook id {id} exceeds MAX_SLOTS");
        let fn_name = extract_next_fn_name(entities, close_paren + 1);
        assert!(
            from_entities.insert(id, fn_name).is_none(),
            "duplicate hook id {id} in entities.rs"
        );
    }

    let mut from_lock = BTreeMap::new();
    let mut rest = lock_json.trim();
    rest = rest.strip_prefix('[').unwrap().strip_suffix(']').unwrap();
    for entry in rest.split('}') {
        let entry = entry.trim().trim_start_matches(',').trim();
        if entry.is_empty() {
            continue;
        }
        let body = entry.strip_prefix('{').unwrap().trim();
        let mut id = None;
        let mut path = None;
        let mut fn_name = None;
        for field in body.split(',') {
            let (key, value) = field.trim().split_once(':').unwrap();
            let key = key.trim().trim_matches('"');
            let value = value.trim();
            match key {
                "id" => id = Some(value.parse::<usize>().unwrap()),
                "path" => path = Some(value.trim_matches('"').to_string()),
                "fn" => fn_name = Some(value.trim_matches('"').to_string()),
                other => panic!("unexpected lock field {other}"),
            }
        }
        let id = id.unwrap();
        let path = path.unwrap();
        let fn_name = fn_name.unwrap();
        assert!(id != 0 && id < MAX_SLOTS);
        assert!(path.starts_with("horizon-client::entities"));
        assert!(!fn_name.is_empty());
        assert!(
            from_lock.insert(id, fn_name).is_none(),
            "duplicate hook id {id} in lock file"
        );
    }

    assert!(!from_entities.is_empty());
    assert_eq!(from_entities, from_lock);
}
