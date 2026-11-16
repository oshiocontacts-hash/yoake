use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use yoake_inject_macro::yoake_inject;
use yoake_inject_rt::{
    CallbackError, Guest, HEAD, HeadOutcome, Mod, OVERWRITE, RETURN, Registry, Side, Subscription,
    Value,
};

#[yoake_inject(id = "tests::calculate", side = "server", allow = "head")]
fn calculate(value: i32) -> i32 {
    if value < 0 {
        return -1;
    }
    value + 1
}

struct ReplaceArguments {
    calls: Arc<AtomicUsize>,
}

impl Guest for ReplaceArguments {
    fn head(&self, _site: &str, arguments: &[Value]) -> Result<HeadOutcome, CallbackError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match arguments {
            [Value::Signed(99)] => Ok(HeadOutcome::ReplaceArguments(vec![Value::Signed(-3)])),
            _ => Ok(HeadOutcome::ReplaceArguments(vec![Value::Signed(9)])),
        }
    }
}

#[test]
fn generated_gate_dispatches_and_preserves_early_return() {
    let _side = yoake_inject_rt::enter_side(Side::Server);
    let site = __yoake_inject_site_calculate();
    let mut registry = Registry::new([site]).unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let guest = Arc::new(ReplaceArguments {
        calls: Arc::clone(&calls),
    });
    let module = Arc::new(Mod::new("test", guest));

    assert_eq!(site.mask(), 0);
    assert_eq!(calculate(-1), -1);
    assert_eq!(calls.load(Ordering::Relaxed), 0);

    let prepared = registry
        .prepare(
            [Subscription {
                site_id: site.id(),
                side: Side::Server,
                points: HEAD,
                module,
            }],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();

    assert_eq!(calculate(1), 10);
    assert_eq!(calculate(99), -1);
    assert_eq!(calls.load(Ordering::Relaxed), 2);
}

#[yoake_inject(id = "tests::combine", side = "server", allow = "head")]
fn combine(mut text: String, bytes: Vec<u8>) -> String {
    text.push(':');
    text.push_str(std::str::from_utf8(&bytes).unwrap());
    text
}

struct ReplaceOwnedArguments;

impl Guest for ReplaceOwnedArguments {
    fn head(&self, _site: &str, arguments: &[Value]) -> Result<HeadOutcome, CallbackError> {
        assert_eq!(
            arguments,
            [
                Value::Text("original".into()),
                Value::Bytes(b"input".to_vec())
            ]
        );
        Ok(HeadOutcome::ReplaceArguments(vec![
            Value::Text("changed".into()),
            Value::Bytes(b"output".to_vec()),
        ]))
    }
}

#[test]
fn generated_gate_moves_and_replaces_owned_values() {
    let _side = yoake_inject_rt::enter_side(Side::Server);
    let site = __yoake_inject_site_combine();
    let mut registry = Registry::new([site]).unwrap();
    assert_eq!(
        combine("original".into(), b"input".to_vec()),
        "original:input"
    );

    let module = Arc::new(Mod::new("owned-values", Arc::new(ReplaceOwnedArguments)));
    let prepared = registry
        .prepare(
            [Subscription {
                site_id: site.id(),
                side: Side::Server,
                points: HEAD,
                module,
            }],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();

    assert_eq!(
        combine("original".into(), b"input".to_vec()),
        "changed:output"
    );
}

#[yoake_inject(id = "tests::colliding_arguments", side = "server", allow = "head")]
fn colliding_arguments(__yoake_inject_iter: i32, __yoake_inject_native: i32) -> i32 {
    __yoake_inject_iter * 10 + __yoake_inject_native
}

struct KeepArguments;

impl Guest for KeepArguments {
    fn head(&self, _site: &str, _arguments: &[Value]) -> Result<HeadOutcome, CallbackError> {
        Ok(HeadOutcome::Continue)
    }
}

#[test]
fn generated_bindings_do_not_shadow_parameters() {
    let _side = yoake_inject_rt::enter_side(Side::Server);
    let site = __yoake_inject_site_colliding_arguments();
    let mut registry = Registry::new([site]).unwrap();
    assert_eq!(colliding_arguments(2, 3), 23);

    let module = Arc::new(Mod::new("hygiene", Arc::new(KeepArguments)));
    let prepared = registry
        .prepare(
            [Subscription {
                site_id: site.id(),
                side: Side::Server,
                points: HEAD,
                module,
            }],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();

    assert_eq!(colliding_arguments(2, 3), 23);
}

static NATIVE_CALLS: AtomicUsize = AtomicUsize::new(0);

#[yoake_inject(id = "tests::pipeline", side = "server")]
fn pipeline(value: i32) -> i32 {
    NATIVE_CALLS.fetch_add(1, Ordering::Relaxed);
    value + 1
}

enum Behavior {
    Cancel,
    ReplaceReturn,
    Overwrite,
    Trap,
}

struct PipelineGuest {
    behavior: Behavior,
    calls: Arc<AtomicUsize>,
}

impl Guest for PipelineGuest {
    fn head(&self, _site: &str, _arguments: &[Value]) -> Result<HeadOutcome, CallbackError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        match self.behavior {
            Behavior::Cancel => Ok(HeadOutcome::Cancel(Value::Signed(42))),
            Behavior::Trap => Err(CallbackError::Trap),
            _ => Ok(HeadOutcome::Continue),
        }
    }

    fn returned(&self, _site: &str, _result: &Value) -> Result<Option<Value>, CallbackError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Some(Value::Signed(77)))
    }

    fn overwrite(&self, _site: &str, _arguments: &[Value]) -> Result<Value, CallbackError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        Ok(Value::Signed(55))
    }
}

#[test]
fn generated_gate_handles_cancel_return_overwrite_and_trap() {
    let _side = yoake_inject_rt::enter_side(Side::Server);
    let site = __yoake_inject_site_pipeline();
    let mut registry = Registry::new([site]).unwrap();
    NATIVE_CALLS.store(0, Ordering::Relaxed);
    assert_eq!(site.mask(), 0);
    assert_eq!(pipeline(1), 2);
    assert_eq!(NATIVE_CALLS.load(Ordering::Relaxed), 1);

    let calls = Arc::new(AtomicUsize::new(0));
    for (behavior, points, expected, native_calls) in [
        (Behavior::Cancel, HEAD, 42, 1),
        (Behavior::ReplaceReturn, RETURN, 77, 2),
        (Behavior::Overwrite, OVERWRITE, 55, 2),
    ] {
        let module = Arc::new(Mod::new(
            "pipeline",
            Arc::new(PipelineGuest {
                behavior,
                calls: Arc::clone(&calls),
            }),
        ));
        let prepared = registry
            .prepare(
                [Subscription {
                    site_id: site.id(),
                    side: Side::Server,
                    points,
                    module,
                }],
                |_, _, _| true,
            )
            .unwrap();
        registry.publish(prepared).unwrap();
        assert_eq!(pipeline(1), expected);
        assert_eq!(NATIVE_CALLS.load(Ordering::Relaxed), native_calls);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 3);

    let module = Arc::new(Mod::new(
        "trap",
        Arc::new(PipelineGuest {
            behavior: Behavior::Trap,
            calls: Arc::clone(&calls),
        }),
    ));
    let prepared = registry
        .prepare(
            [Subscription {
                site_id: site.id(),
                side: Side::Server,
                points: HEAD,
                module: Arc::clone(&module),
            }],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    assert_eq!(pipeline(8), 9);
    assert!(module.has_disabled_handlers());
    assert_eq!(pipeline(9), 10);
    assert_eq!(NATIVE_CALLS.load(Ordering::Relaxed), 4);
    assert_eq!(calls.load(Ordering::Relaxed), 4);
}
