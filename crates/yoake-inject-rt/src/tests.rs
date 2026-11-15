use super::*;
use std::cell::Cell;
use std::sync::atomic::{AtomicUsize, Ordering};

#[yoake_inject_macro::yoake_inject(id = "tests::no_mod_gate", side = "server", allow = "head")]
fn no_mod_gate(value: i32) -> i32 {
    value + 1
}

#[test]
fn zero_mask_never_enters_dispatch() {
    let site = __yoake_inject_site_no_mod_gate();
    assert_eq!(site.mask(), 0);
    {
        let _exclusive = site.handlers.write().unwrap();
        assert_eq!(no_mod_gate(7), 8);
    }
    assert_eq!(site.invocations.load(Ordering::Relaxed), 0);

    assert_eq!(
        site.invoke(vec![Value::Signed(7)], |_| Value::Signed(8)),
        Value::Signed(8)
    );
    assert_eq!(site.invocations.load(Ordering::Relaxed), 1);
}

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OTHER_HASH: &str = "1123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[derive(Clone, Copy)]
enum Behavior {
    Modify,
    Increment,
    Cancel,
    ReplaceReturn,
    Overwrite,
    TrapHead,
    TrapReturn,
    TrapOverwrite,
    InvalidArguments,
}

struct TestGuest {
    behavior: Behavior,
    calls: Arc<AtomicUsize>,
}

impl Guest for TestGuest {
    fn head(&self, _site: &str, arguments: &[Value]) -> Result<HeadOutcome, CallbackError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.behavior {
            Behavior::Modify => Ok(HeadOutcome::ReplaceArguments(vec![Value::Signed(9)])),
            Behavior::Increment => match arguments {
                [Value::Signed(value)] => Ok(HeadOutcome::ReplaceArguments(vec![Value::Signed(
                    *value + 2,
                )])),
                _ => Err(CallbackError::Trap),
            },
            Behavior::Cancel => Ok(HeadOutcome::Cancel(Value::Signed(42))),
            Behavior::TrapHead => Err(CallbackError::Trap),
            Behavior::InvalidArguments => {
                Ok(HeadOutcome::ReplaceArguments(vec![Value::Boolean(true)]))
            }
            _ => Ok(HeadOutcome::Continue),
        }
    }

    fn returned(&self, _site: &str, _result: &Value) -> Result<Option<Value>, CallbackError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.behavior {
            Behavior::ReplaceReturn => Ok(Some(Value::Signed(77))),
            Behavior::TrapReturn => Err(CallbackError::Trap),
            _ => Ok(None),
        }
    }

    fn overwrite(&self, _site: &str, _arguments: &[Value]) -> Result<Value, CallbackError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.behavior {
            Behavior::Overwrite => Ok(Value::Signed(55)),
            _ => Err(CallbackError::Trap),
        }
    }
}

fn site() -> &'static Site {
    ACTIVE_SIDE.set(Side::Server);
    Box::leak(Box::new(Site::new(
        "game::damage",
        HASH,
        Side::Server,
        &[ValueType::S32],
        ValueType::S32,
        HEAD | RETURN | OVERWRITE,
    )))
}

fn module(id: &str, behavior: Behavior) -> (Arc<Mod>, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let guest = TestGuest {
        behavior,
        calls: Arc::clone(&calls),
    };
    (Arc::new(Mod::new(id, Arc::new(guest))), calls)
}

fn subscription(site: &Site, module: &Arc<Mod>, points: u8) -> Subscription {
    Subscription {
        site_id: site.id(),
        side: site.side(),
        points,
        module: Arc::clone(module),
    }
}

fn harness(
    behavior: Behavior,
    points: u8,
) -> (&'static Site, Registry, Arc<Mod>, Arc<AtomicUsize>) {
    let site = site();
    let mut registry = Registry::new([site]).unwrap();
    let (module, calls) = module("test", behavior);
    let prepared = registry
        .prepare([subscription(site, &module, points)], |_, _, _| true)
        .unwrap();
    registry.publish(prepared).unwrap();
    (site, registry, module, calls)
}

fn original(arguments: Vec<Value>) -> Value {
    match arguments.as_slice() {
        [Value::Signed(value)] => Value::Signed(*value + 1),
        _ => panic!("unexpected arguments"),
    }
}

#[test]
fn head_can_replace_arguments_or_cancel() {
    let (site, registry, _, _) = harness(Behavior::Modify, HEAD);
    assert_eq!(
        registry.invoke(site, vec![Value::Signed(1)], original),
        Value::Signed(10)
    );
    let (site, registry, _, _) = harness(Behavior::Cancel, HEAD);
    let native_calls = Cell::new(0);
    let result = registry.invoke(site, vec![Value::Signed(1)], |arguments| {
        native_calls.set(native_calls.get() + 1);
        original(arguments)
    });
    assert_eq!(result, Value::Signed(42));
    assert_eq!(native_calls.get(), 0);
}

#[test]
fn return_replaces_native_result_without_reexecuting_original() {
    let (site, registry, _, _) = harness(Behavior::ReplaceReturn, RETURN);
    let native_calls = Cell::new(0);
    let result = registry.invoke(site, vec![Value::Signed(1)], |arguments| {
        native_calls.set(native_calls.get() + 1);
        original(arguments)
    });
    assert_eq!(result, Value::Signed(77));
    assert_eq!(native_calls.get(), 1);
}

#[test]
fn overwrite_replaces_original() {
    let (site, registry, _, _) = harness(Behavior::Overwrite, OVERWRITE);
    let native_calls = Cell::new(0);
    let result = registry.invoke(site, vec![Value::Signed(1)], |arguments| {
        native_calls.set(native_calls.get() + 1);
        original(arguments)
    });
    assert_eq!(result, Value::Signed(55));
    assert_eq!(native_calls.get(), 0);
}

#[test]
fn head_callbacks_apply_in_mod_id_order() {
    let site = site();
    let mut registry = Registry::new([site]).unwrap();
    let (first, _) = module("a", Behavior::Modify);
    let (second, _) = module("z", Behavior::Increment);
    let prepared = registry
        .prepare(
            [
                subscription(site, &second, HEAD),
                subscription(site, &first, HEAD),
            ],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    assert_eq!(
        registry.invoke(site, vec![Value::Signed(1)], original),
        Value::Signed(12)
    );
}

#[test]
fn trap_or_invalid_head_disables_only_failing_module_and_restores_arguments() {
    for behavior in [Behavior::TrapHead, Behavior::InvalidArguments] {
        let site = site();
        let mut registry = Registry::new([site]).unwrap();
        let (first, _) = module("a", Behavior::Modify);
        let (failing, calls) = module("b", behavior);
        let prepared = registry
            .prepare(
                [
                    subscription(site, &first, HEAD),
                    subscription(site, &failing, HEAD),
                ],
                |_, _, _| true,
            )
            .unwrap();
        registry.publish(prepared).unwrap();
        assert_eq!(
            registry.invoke(site, vec![Value::Signed(1)], original),
            Value::Signed(2)
        );
        assert!(!first.has_disabled_handlers());
        assert!(failing.has_disabled_handlers());
        assert_eq!(
            registry.invoke(site, vec![Value::Signed(1)], original),
            Value::Signed(10)
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }
}

#[test]
fn overwrite_trap_runs_original_and_disables_module() {
    let (site, registry, module, _) = harness(Behavior::TrapOverwrite, OVERWRITE);
    assert_eq!(
        registry.invoke(site, vec![Value::Signed(1)], original),
        Value::Signed(2)
    );
    assert!(module.has_disabled_handlers());
}

#[test]
fn return_trap_keeps_native_result_without_repeating_original() {
    let (site, registry, module, _) = harness(Behavior::TrapReturn, RETURN);
    let native_calls = Cell::new(0);
    let result = registry.invoke(site, vec![Value::Signed(1)], |arguments| {
        native_calls.set(native_calls.get() + 1);
        original(arguments)
    });
    assert_eq!(result, Value::Signed(2));
    assert_eq!(native_calls.get(), 1);
    assert!(module.has_disabled_handlers());
}

#[test]
fn return_trap_after_head_modification_preserves_the_single_native_call() {
    let site = site();
    let mut registry = Registry::new([site]).unwrap();
    let (modifying, _) = module("a", Behavior::Modify);
    let (failing, _) = module("b", Behavior::TrapReturn);
    let prepared = registry
        .prepare(
            [
                subscription(site, &modifying, HEAD),
                subscription(site, &failing, RETURN),
            ],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    let native_calls = Cell::new(0);
    let result = registry.invoke(site, vec![Value::Signed(1)], |arguments| {
        native_calls.set(native_calls.get() + 1);
        original(arguments)
    });
    assert_eq!(result, Value::Signed(10));
    assert_eq!(native_calls.get(), 1);
    assert!(failing.has_disabled_handlers());
}

#[test]
fn return_trap_after_cancellation_falls_back_to_original() {
    let site = site();
    let mut registry = Registry::new([site]).unwrap();
    let (cancelling, _) = module("a", Behavior::Cancel);
    let (failing, _) = module("b", Behavior::TrapReturn);
    let prepared = registry
        .prepare(
            [
                subscription(site, &cancelling, HEAD),
                subscription(site, &failing, RETURN),
            ],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    assert_eq!(
        registry.invoke(site, vec![Value::Signed(1)], original),
        Value::Signed(2)
    );
    assert!(failing.has_disabled_handlers());
}

#[test]
fn return_trap_after_overwrite_runs_original_once() {
    let site = site();
    let mut registry = Registry::new([site]).unwrap();
    let (overwriting, overwrite_calls) = module("a", Behavior::Overwrite);
    let (failing, _) = module("b", Behavior::TrapReturn);
    let prepared = registry
        .prepare(
            [
                subscription(site, &overwriting, OVERWRITE),
                subscription(site, &failing, RETURN),
            ],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    let native_calls = Cell::new(0);
    let result = registry.invoke(site, vec![Value::Signed(1)], |arguments| {
        native_calls.set(native_calls.get() + 1);
        original(arguments)
    });
    assert_eq!(result, Value::Signed(2));
    assert_eq!(native_calls.get(), 1);
    assert_eq!(overwrite_calls.load(Ordering::Relaxed), 1);
    assert!(failing.has_disabled_handlers());
}

#[test]
fn conflicting_overwrites_and_changed_signatures_are_rejected() {
    let site = site();
    let registry = Registry::new([site]).unwrap();
    let (first, _) = module("a", Behavior::Overwrite);
    let (second, _) = module("b", Behavior::Overwrite);
    let conflict = registry.prepare(
        [
            subscription(site, &first, OVERWRITE),
            subscription(site, &second, OVERWRITE),
        ],
        |_, _, _| true,
    );
    assert!(matches!(
        conflict,
        Err(RegistryError::ConflictingOverwrite(_))
    ));
    let mut changed = subscription(site, &first, OVERWRITE);
    changed.site_id = format!("{}#{OTHER_HASH}", site.path());
    let mismatch = registry.prepare([changed], |_, _, _| true);
    assert!(matches!(
        mismatch,
        Err(RegistryError::SignatureMismatch { .. })
    ));
    assert_eq!(site.mask(), 0);
}

#[test]
fn policy_and_side_rejections_do_not_publish_handlers() {
    let site = site();
    let registry = Registry::new([site]).unwrap();
    let (module, _) = module("test", Behavior::Modify);
    let denied = registry.prepare([subscription(site, &module, HEAD)], |_, _, _| false);
    assert!(matches!(
        denied,
        Err(RegistryError::PermissionDenied { .. })
    ));
    let mut wrong_side = subscription(site, &module, HEAD);
    wrong_side.side = Side::Client;
    let rejected = registry.prepare([wrong_side], |_, _, _| true);
    assert!(matches!(rejected, Err(RegistryError::WrongSide(_))));
    assert_eq!(site.mask(), 0);
}

#[test]
fn registry_claims_are_released_when_the_owner_is_dropped() {
    let site = site();
    let first = Registry::new([site]).unwrap();
    assert!(matches!(
        Registry::new([site]),
        Err(RegistryError::SiteAlreadyClaimed(_))
    ));
    drop(first);
    assert!(Registry::new([site]).is_ok());
}

#[test]
fn publication_controls_masks_and_zero_mask_skips_snapshots_and_dispatch() {
    let site = site();
    let mut registry = Registry::new([site]).unwrap();
    let (module, calls) = module("test", Behavior::Modify);
    let snapshots = Cell::new(0);
    let run = |registry: &Registry| {
        if site.mask() == 0 {
            Value::Signed(2)
        } else {
            snapshots.set(snapshots.get() + 1);
            registry.invoke(site, vec![Value::Signed(1)], original)
        }
    };
    assert_eq!(run(&registry), Value::Signed(2));
    assert_eq!(snapshots.get(), 0);
    assert_eq!(calls.load(Ordering::Relaxed), 0);
    let prepared = registry
        .prepare([subscription(site, &module, HEAD)], |_, _, _| true)
        .unwrap();
    registry.publish(prepared).unwrap();
    assert_eq!(site.mask(), HEAD);
    assert_eq!(run(&registry), Value::Signed(10));
    let prepared = registry.prepare([], |_, _, _| true).unwrap();
    registry.publish(prepared).unwrap();
    assert_eq!(site.mask(), 0);
    assert_eq!(run(&registry), Value::Signed(2));
    assert_eq!(snapshots.get(), 1);
    assert_eq!(calls.load(Ordering::Relaxed), 1);
}

#[test]
fn rejected_and_unpublished_snapshots_leave_active_dispatch_unchanged() {
    let (site, mut registry, _, _) = harness(Behavior::Modify, HEAD);
    let (first, _) = module("first", Behavior::Overwrite);
    let (second, _) = module("second", Behavior::Overwrite);
    let conflict = registry.prepare(
        [
            subscription(site, &first, OVERWRITE),
            subscription(site, &second, OVERWRITE),
        ],
        |_, _, _| true,
    );
    assert!(matches!(
        conflict,
        Err(RegistryError::ConflictingOverwrite(_))
    ));
    assert_eq!(site.mask(), HEAD);
    assert_eq!(
        registry.invoke(site, vec![Value::Signed(1)], original),
        Value::Signed(10)
    );

    let (replacement, _) = module("replacement", Behavior::Cancel);
    let prepared = registry
        .prepare([subscription(site, &replacement, HEAD)], |_, _, _| true)
        .unwrap();
    assert_eq!(
        registry.invoke(site, vec![Value::Signed(1)], original),
        Value::Signed(10)
    );
    registry.publish(prepared).unwrap();
    assert_eq!(
        registry.invoke(site, vec![Value::Signed(1)], original),
        Value::Signed(42)
    );
}

#[test]
fn a_snapshot_cannot_be_published_by_another_registry() {
    let first_site = site();
    let second_site = site();
    let first_registry = Registry::new([first_site]).unwrap();
    let mut second_registry = Registry::new([second_site]).unwrap();
    let (module, _) = module("test", Behavior::Modify);
    let prepared = first_registry
        .prepare([subscription(first_site, &module, HEAD)], |_, _, _| true)
        .unwrap();

    assert_eq!(
        second_registry.publish(prepared),
        Err(RegistryError::WrongRegistry)
    );
    assert_eq!(first_site.mask(), 0);
    assert_eq!(second_site.mask(), 0);
}
