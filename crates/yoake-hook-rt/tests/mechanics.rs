use std::sync::atomic::{AtomicUsize, Ordering};

use yoake_hook_macro::yoake_hook;
use yoake_hook_rt::{__register_typed, Order, SLOTS, SLOT_CAPACITY, TEST_ID_BASE};

const _: () = yoake_hook_rt::assert_hook_name("crate::m::W<u32>::get", "get");
const _: () = yoake_hook_rt::assert_hook_name("crate::m::W<[u8;4]>::get", "get");
const _: () = yoake_hook_rt::assert_hook_name("crate::m::W<[u8;4]>::get[cfg(unix)]", "get");
const _: () = yoake_hook_rt::assert_hook_name("crate::m::<W<[u8;4]> as Tr>::f", "f");
const _: () = yoake_hook_rt::assert_hook_name("crate::m::open[cfg(unix)]", "open");
const _: () = yoake_hook_rt::assert_hook_name(
    "crate::m::open[cfg(feature=\"a::b\")]",
    "open",
);
const _: () = yoake_hook_rt::assert_hook_name(
    "horizon_client::entities::<ArmorStand as EntityRules>::tick",
    "__yoake_trait_entity_rules_tick",
);

#[test]
fn zero_calls_and_early_return_skip_the_original_body() {
    #[yoake_hook(test_id = 1)]
    fn never_called() -> u32 {
        panic!("around hook should bypass the body")
    }
    __yoake_register_never_called(|_original| 7, Order::default()).unwrap();
    assert_eq!(never_called(), 7);
}

#[test]
fn one_call_replaces_arguments_and_transforms_result() {
    #[yoake_hook(test_id = 2)]
    fn add(left: u32, right: u32) -> u32 {
        left + right
    }
    assert_eq!(add(1, 2), 3);
    __yoake_register_add(
        |original, _left, _right| original(10, 20) * 2,
        Order::default(),
    )
    .unwrap();
    assert_eq!(add(1, 2), 60);
}

#[test]
fn two_calls_of_original_run_the_rest_of_the_chain_twice() {
    static BODY_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[yoake_hook(test_id = 3)]
    fn compute(value: u32) -> u32 {
        BODY_CALLS.fetch_add(1, Ordering::Relaxed);
        value + 1
    }
    __yoake_register_compute(
        |original, value| original(value) + original(value + 10),
        Order {
            priority: 10,
            mod_load_order: 0,
        },
    )
    .unwrap();
    __yoake_register_compute(
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
    #[yoake_hook(test_id = 4)]
    fn checked(input: Result<u32, u32>) -> Result<u32, u32> {
        let value = input?;
        if value == 0 {
            return Ok(10);
        }
        Ok(value)
    }
    __yoake_register_checked(
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
    #[yoake_hook(test_id = 5)]
    fn digits(value: u32) -> u32 {
        value * 10 + 9
    }
    __yoake_register_digits(
        |original, value| original(value * 10 + 4),
        Order {
            priority: -1,
            mod_load_order: 0,
        },
    )
    .unwrap();
    __yoake_register_digits(
        |original, value| original(value * 10 + 2),
        Order {
            priority: 10,
            mod_load_order: 2,
        },
    )
    .unwrap();
    __yoake_register_digits(
        |original, value| original(value * 10 + 1),
        Order {
            priority: 10,
            mod_load_order: 1,
        },
    )
    .unwrap();
    __yoake_register_digits(
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
    #[yoake_hook(method, test_id = 6)]
    fn increment(&mut self, amount: u32) -> u32 {
        self.0 += amount;
        self.0
    }

    #[yoake_hook(method, test_id = 7)]
    fn read(&self) -> u32 {
        self.0
    }
}

#[test]
fn borrowed_mut_self_method_can_be_hooked_and_reborrowed() {
    Counter::__yoake_register_increment(
        |original, receiver, _amount| {
            receiver.0 = 40;
            original(receiver, 2)
        },
        Order::default(),
    )
    .unwrap();
    Counter::__yoake_register_read(
        |original, receiver| original(receiver) + 1,
        Order::default(),
    )
    .unwrap();
    let mut counter = Counter(0);
    assert_eq!(counter.increment(1), 42);
    assert_eq!(counter.read(), 43);
}

#[test]
fn borrowed_str_arg_and_shared_lifetimes_work_without_static_data() {
    #[yoake_hook(test_id = 8)]
    fn length<'input>(value: &'input str, run: &mut dyn FnMut(usize)) -> usize {
        run(value.len());
        value.len()
    }
    __yoake_register_length(
        |original, _value, run| original("replacement", run),
        Order::default(),
    )
    .unwrap();
    let input = String::from("short");
    let mut observed = 0;
    assert_eq!(length(&input, &mut |value| observed = value), 11);
    assert_eq!(observed, 11);

    #[yoake_hook(test_id = 9)]
    fn pair_len<'a>(left: &'a str, right: &'a str) -> usize {
        left.len() + right.len()
    }
    __yoake_register_pair_len(
        |original, left, _right| original(left, "abc"),
        Order::default(),
    )
    .unwrap();
    let a = String::from("hi");
    let b = String::from("longer");
    assert_eq!(pair_len(&a, &b), 5);
}

#[test]
fn reentry_from_inside_a_hook_runs_the_chain_independently() {
    static HOOK_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[yoake_hook(test_id = 10)]
    fn countdown(n: u32) -> u32 {
        n
    }
    __yoake_register_countdown(
        |original, n| {
            HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
            if n == 0 {
                original(0)
            } else {
                original(n) + countdown(n - 1)
            }
        },
        Order::default(),
    )
    .unwrap();
    assert_eq!(countdown(3), 6);
    assert_eq!(HOOK_CALLS.load(Ordering::Relaxed), 4);
}

static INLINE_FIRES: AtomicUsize = AtomicUsize::new(0);

#[yoake_hook(test_id = 11)]
#[inline(always)]
fn inline_leaf(value: u64) -> u64 {
    value + 1
}

#[test]
fn inline_always_body_keeps_its_guard() {
    assert_eq!(inline_leaf(4), 5);
    __yoake_register_inline_leaf(
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
fn generic_slots_use_typed_registration_and_reject_mismatches() {
    #[yoake_hook(test_id = 12)]
    fn identity<Value: 'static>(value: Value) -> Value {
        value
    }
    __yoake_register_identity::<u32>(
        |original, value| original(value + 10),
        Order::default(),
    )
    .unwrap();
    type StringHook = fn(&mut dyn FnMut(String) -> String, String) -> String;
    let mismatched = __register_typed::<StringHook>(
        __YOAKE_HOOK_ID_identity,
        std::any::TypeId::of::<StringHook>(),
        |original, value| original(value),
        Order::default(),
    );
    assert!(mismatched.is_err());
    assert_eq!(identity(1_u32), 11);
    assert_eq!(identity(2_u64), 2);
    assert_eq!(identity(String::from("typed")), "typed");

    #[yoake_hook(test_id = 13)]
    fn constant<const VALUE: usize>() -> usize {
        VALUE
    }
    __yoake_register_constant::<1>(|original| original() + 100, Order::default()).unwrap();
    assert_eq!(constant::<1>(), 101);
    assert_eq!(constant::<2>(), 102);
}

#[test]
fn concurrent_publication_keeps_old_snapshots_alive_and_bounds_registrations() {
    const SLOT_ID: usize = TEST_ID_BASE + 14;
    type Callback = fn(&mut dyn FnMut());
    let noop: Callback = |original| original();
    __register_typed(
        SLOT_ID,
        std::any::TypeId::of::<Callback>(),
        noop,
        Order::default(),
    ).unwrap();
    let old = SLOTS[SLOT_ID].load().unwrap();
    std::thread::scope(|scope| {
        for _index in 0..7 {
            scope.spawn(|| {
                for _registration in 0..9 {
                    __register_typed(
        SLOT_ID,
        std::any::TypeId::of::<Callback>(),
        noop,
        Order::default(),
    ).unwrap();
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
    assert!(__register_typed(
        SLOT_ID,
        std::any::TypeId::of::<Callback>(),
        noop,
        Order::default(),
    ).is_err());
    assert!(__register_typed(
        SLOT_CAPACITY,
        std::any::TypeId::of::<Callback>(),
        noop,
        Order::default(),
    )
    .is_err());

    fn zst_fn(_: &mut dyn FnMut()) {}
    assert!(__register_typed(
        TEST_ID_BASE + 15,
        std::any::TypeId::of::<fn(&mut dyn FnMut())>(),
        zst_fn,
        Order::default(),
    )
    .is_err());
}

struct Holder<'a>(&'a mut u32);

impl Holder<'_> {
    #[yoake_hook(method, test_id = 15, self_ty = Holder<'_>)]
    fn add(&mut self, amount: u32) -> u32 {
        *self.0 += amount;
        *self.0
    }
}

#[test]
fn borrowed_impl_self_type_uses_a_static_callback_pointer() {
    Holder::__yoake_register_add(
        |original, receiver, amount| {
            *receiver.0 += 10;
            original(receiver, amount) + 1
        },
        Order::default(),
    )
    .unwrap();
    let mut value = 1;
    {
        let mut holder = Holder(&mut value);
        // 1 + 10 (hook) + 2 (original) = 13 stored; the hook returns 13 + 1.
        assert_eq!(holder.add(2), 14);
    }
    assert_eq!(value, 13);
}

// F1 W2: a constructor of `impl<'r> Reg<'r>` whose argument and result (`io::Result<Self>`) share `'r`.
struct Reg<'r> {
    source: &'r u32,
}

impl<'r> Reg<'r> {
    #[yoake_hook(method, test_id = 40, self_ty = Reg<'r>)]
    fn new(source: &'r u32) -> std::io::Result<Self> {
        Ok(Self { source })
    }

    #[yoake_hook(method, test_id = 41, self_ty = Reg<'r>)]
    fn pick(first: &'r u32, second: &'r u32, take_first: bool) -> Option<Self> {
        Some(Self { source: if take_first { first } else { second } })
    }
}

#[test]
fn constructor_with_a_named_impl_lifetime_in_argument_and_result_is_hookable() {
    let value = 5u32;
    assert_eq!(*Reg::new(&value).unwrap().source, 5);
    Reg::__yoake_register_new(|original, source| original(source), Order::default()).unwrap();
    assert_eq!(*Reg::new(&value).unwrap().source, 5);
    let (a, b) = (1u32, 2u32);
    Reg::__yoake_register_pick(
        |original, first, second, _take_first| original(first, second, false),
        Order::default(),
    )
    .unwrap();
    assert_eq!(*Reg::pick(&a, &b, true).unwrap().source, 2, "the hook replaced an argument");
}

// F1 W2: `-> &'static dyn Trait` (horizon-world `worldgen::blending::shift_noise`) and `&dyn Trait` arguments.
trait Shape {
    fn sides(&self) -> u32;
}
struct Tri;
impl Shape for Tri {
    fn sides(&self) -> u32 {
        3
    }
}

#[yoake_hook(test_id = 42)]
fn static_shape() -> &'static dyn Shape {
    static TRI: Tri = Tri;
    &TRI
}

#[yoake_hook(test_id = 43)]
fn sides_of(shape: &dyn Shape, extra: u32) -> u32 {
    shape.sides() + extra
}

#[test]
fn dyn_trait_objects_in_results_and_arguments_are_hookable() {
    assert_eq!(static_shape().sides(), 3);
    assert_eq!(sides_of(&Tri, 1), 4);
    __yoake_register_static_shape(|original| original(), Order::default()).unwrap();
    __yoake_register_sides_of(|original, shape, extra| original(shape, extra + 10), Order::default()).unwrap();
    assert_eq!(static_shape().sides(), 3);
    assert_eq!(sides_of(&Tri, 1), 14);
}

// F1 W4: destructuring, `_` and `ref` parameters are hookable (the outer fn names them, the original keeps them).
#[yoake_hook(test_id = 44)]
fn destructured((first, second): (u32, u32), _unused: u8, Wrapper { inner }: Wrapper) -> u32 {
    first * 100 + second * 10 + inner
}

#[yoake_hook(test_id = 45)]
fn wildcard(_: u32, kept: u32) -> u32 {
    kept
}

struct Wrapper {
    inner: u32,
}

#[test]
fn destructuring_and_wildcard_parameters_are_hookable() {
    assert_eq!(destructured((1, 2), 0, Wrapper { inner: 3 }), 123);
    assert_eq!(wildcard(9, 4), 4);
    __yoake_register_destructured(
        |original, pair, byte, wrapper| original((pair.1, pair.0), byte, wrapper),
        Order::default(),
    )
    .unwrap();
    __yoake_register_wildcard(|original, first, second| original(first, second + 1), Order::default()).unwrap();
    assert_eq!(destructured((1, 2), 0, Wrapper { inner: 3 }), 213, "the hook swapped the tuple");
    assert_eq!(wildcard(9, 4), 5);
}

// F1 W12: a by-value `self` of a Copy type, `impl Fn` / `impl FnMut` arguments (dyn shim), `'static` generics.
#[derive(Clone, Copy)]
struct Meters(f32);

impl Meters {
    #[yoake_hook(method, test_id = 46)]
    fn doubled(self, extra: f32) -> f32 {
        self.0 * 2.0 + extra
    }

    #[yoake_hook(method, test_id = 47)]
    fn consume(mut self, add: f32) -> f32 {
        self.0 += add;
        self.0
    }
}

#[test]
fn a_by_value_self_of_a_copy_type_is_hookable() {
    assert_eq!(Meters(2.0).doubled(1.0), 5.0);
    Meters::__yoake_register_doubled(|original, this, extra| original(Meters(this.0 + 10.0), extra), Order::default()).unwrap();
    assert_eq!(Meters(2.0).doubled(1.0), 25.0);
    Meters::__yoake_register_consume(|original, this, add| original(this, add) + 100.0, Order::default()).unwrap();
    assert_eq!(Meters(1.0).consume(2.0), 103.0);
}

#[yoake_hook(test_id = 48)]
fn apply(value: u32, f: impl Fn(u32) -> u32) -> u32 {
    f(value) + 1
}

#[yoake_hook(test_id = 49)]
fn apply_mut(value: u32, mut f: impl FnMut(u32) -> u32) -> u32 {
    f(value) + f(value)
}

#[test]
fn impl_fn_arguments_go_through_a_dyn_shim() {
    assert_eq!(apply(3, |v| v * 2), 7);
    __yoake_register_apply(|original, value, f| original(value + f(1), f), Order::default()).unwrap();
    // the hook calls the real closure through `&dyn Fn` and forwards it on
    assert_eq!(apply(3, |v| v * 2), 11);

    let mut calls = 0;
    assert_eq!(apply_mut(1, |v| { calls += 1; v + calls }), 2 + 3);
    __yoake_register_apply_mut(|original, value, f| { let first = f(0); original(value + first, f) }, Order::default()).unwrap();
    let mut calls = 0;
    // f(0) = 1 (calls = 1), then original(2, f): f(2) = 4, f(2) = 5 -> 9
    assert_eq!(apply_mut(1, |v| { calls += 1; v + calls }), 9);
}

#[yoake_hook(test_id = 50)]
fn pick<Value: 'static + Copy>(first: Value, _second: Value) -> Value {
    first
}

#[test]
fn static_generics_hook_per_instantiation() {
    __yoake_register_pick::<u8>(|original, a, b| original(b, a), Order::default()).unwrap();
    assert_eq!(pick(1_u8, 2_u8), 2);
    assert_eq!(pick(1_u16, 2_u16), 1, "another instantiation is untouched");
}
