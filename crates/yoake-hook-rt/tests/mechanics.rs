use std::ops::ControlFlow;
use std::sync::atomic::{AtomicUsize, Ordering};

use yoake_hook_macro::yoake_hook;
use yoake_hook_rt::{Order, Site, Slot};

#[test]
fn head_replaces_arguments_and_return_replaces_normal_result() {
    #[yoake_hook]
    fn add(left: u32, right: u32) -> u32 {
        left + right
    }
    assert_eq!(add(1, 2), 3);
    let site = __yoake_hook_add();
    site.register_head(
        |left, right| {
            *left = 10;
            *right = 20;
            ControlFlow::Continue(())
        },
        Order::default(),
    )
    .unwrap();
    site.register_return(|result| *result *= 2, Order::default())
        .unwrap();
    assert_eq!(add(1, 2), 60);
}

#[test]
fn head_early_return_still_runs_return_hooks() {
    #[yoake_hook]
    fn never_called() -> u32 {
        panic!("HEAD should bypass the body")
    }
    __yoake_hook_never_called()
        .register_head(|| ControlFlow::Break(7), Order::default())
        .unwrap();
    __yoake_hook_never_called()
        .register_return(|result| *result += 1, Order::default())
        .unwrap();
    assert_eq!(never_called(), 8);
}

#[test]
fn return_hooks_see_question_mark_and_explicit_returns() {
    #[yoake_hook]
    fn checked(input: Result<u32, u32>) -> Result<u32, u32> {
        let value = input?;
        if value == 0 {
            return Ok(10);
        }
        Ok(value)
    }
    __yoake_hook_checked()
        .register_return(
            |result| *result = result.map(|value| value + 1).map_err(|error| error + 2),
            Order::default(),
        )
        .unwrap();
    assert_eq!(checked(Err(3)), Err(5));
    assert_eq!(checked(Ok(0)), Ok(11));
    assert_eq!(checked(Ok(4)), Ok(5));
}

#[test]
fn order_is_priority_then_load_then_insertion_and_break_stops_head() {
    #[yoake_hook]
    fn digits(value: u32) -> u32 {
        value
    }
    let site = __yoake_hook_digits();
    site.register_head(
        |value| {
            *value = *value * 10 + 4;
            ControlFlow::Continue(())
        },
        Order {
            priority: -1,
            mod_load_order: 0,
        },
    )
    .unwrap();
    site.register_head(
        |value| {
            *value = *value * 10 + 2;
            ControlFlow::Continue(())
        },
        Order {
            priority: 10,
            mod_load_order: 2,
        },
    )
    .unwrap();
    site.register_head(
        |value| {
            *value = *value * 10 + 1;
            ControlFlow::Continue(())
        },
        Order {
            priority: 10,
            mod_load_order: 1,
        },
    )
    .unwrap();
    site.register_head(
        |value| {
            *value = *value * 10 + 3;
            ControlFlow::Break(*value)
        },
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
    #[yoake_hook(method)]
    fn increment(&mut self, amount: u32) -> u32 {
        self.0 += amount;
        self.0
    }

    #[yoake_hook(method)]
    fn read(&self) -> u32 {
        self.0
    }
}

#[test]
fn borrowed_receivers_are_typed_and_reborrowed() {
    Counter::__yoake_hook_increment()
        .register_head(
            |receiver, amount| {
                receiver.0 = 40;
                *amount = 2;
                ControlFlow::Continue(())
            },
            Order::default(),
        )
        .unwrap();
    Counter::__yoake_hook_read()
        .register_head(
            |receiver| ControlFlow::Break(receiver.0 + 1),
            Order::default(),
        )
        .unwrap();
    let mut counter = Counter(0);
    assert_eq!(counter.increment(1), 42);
    assert_eq!(counter.read(), 43);
}

#[test]
fn higher_ranked_arguments_can_rebind_and_call_borrowed_closures() {
    #[yoake_hook]
    fn length<'input>(value: &'input str, run: &mut dyn FnMut(usize)) -> usize {
        run(value.len());
        value.len()
    }
    let callback: for<'input, 'run> fn(
        &mut &'input str,
        &mut &'run mut (dyn FnMut(usize) + 'run),
    ) -> ControlFlow<usize> = |value, _run| {
        *value = "replacement";
        ControlFlow::Continue(())
    };
    __yoake_hook_length()
        .register_head(callback, Order::default())
        .unwrap();
    let input = String::from("short");
    let mut observed = 0;
    assert_eq!(length(&input, &mut |value| observed = value), 11);
    assert_eq!(observed, 11);
}

#[test]
fn generic_slot_is_shared_but_dispatch_is_typed() {
    #[yoake_hook]
    fn identity<Value: 'static>(value: Value) -> Value {
        value
    }
    __yoake_hook_identity::<u32>()
        .register_head(
            |value| {
                *value += 10;
                ControlFlow::Continue(())
            },
            Order::default(),
        )
        .unwrap();
    __yoake_hook_identity::<String>()
        .register_return(|result| result.push('!'), Order::default())
        .unwrap();
    assert_eq!(identity(1_u32), 11);
    assert_eq!(identity(2_u64), 2);
    assert_eq!(identity(String::from("typed")), "typed!");

    #[yoake_hook]
    fn constant<const VALUE: usize>() -> usize {
        VALUE
    }
    __yoake_hook_constant::<1>()
        .register_return(|value| *value += 100, Order::default())
        .unwrap();
    assert_eq!(constant::<1>(), 101);
    assert_eq!(constant::<2>(), 102);
}

static INLINE_FIRES: AtomicUsize = AtomicUsize::new(0);

#[yoake_hook]
#[inline(always)]
fn inline_leaf(value: u64) -> u64 {
    value + 1
}

#[test]
fn inline_always_body_keeps_its_guard() {
    assert_eq!(inline_leaf(4), 5);
    __yoake_hook_inline_leaf()
        .register_head(
            |_value| {
                INLINE_FIRES.fetch_add(1, Ordering::Relaxed);
                ControlFlow::Continue(())
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
fn concurrent_publication_keeps_old_snapshots_alive_and_bounds_registrations() {
    static SLOT: Slot = Slot::new();
    type Callback = fn() -> ControlFlow<()>;
    let site = Site::<Callback, fn(&mut ())>::new(&SLOT);
    site.register_head(|| ControlFlow::Continue(()), Order::default())
        .unwrap();
    let old = site.load().unwrap();
    std::thread::scope(|scope| {
        for _index in 0..7 {
            scope.spawn(|| {
                for _registration in 0..9 {
                    site.register_head(|| ControlFlow::Continue(()), Order::default())
                        .unwrap();
                }
            });
        }
    });
    let mut old_count = 0;
    let outcome = site.head(&old, |callback| {
        old_count += 1;
        callback()
    });
    assert!(outcome.is_continue());
    assert_eq!(old_count, 1);
    let mut current_count = 0;
    let outcome = site.head(&site.load().unwrap(), |callback| {
        current_count += 1;
        callback()
    });
    assert!(outcome.is_continue());
    assert_eq!(current_count, 64);
    assert!(
        site.register_head(|| ControlFlow::Continue(()), Order::default())
            .is_err()
    );
}
