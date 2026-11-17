use std::sync::Arc;

use yoake_inject_rt::{
    CallbackError, Guest, HEAD, HeadOutcome, Mod, OVERWRITE, RETURN, Registry, Side, Site,
    Subscription, Value, ValueType, enter_side,
};

const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

struct Response {
    head: HeadOutcome,
    result: Value,
}

impl Guest for Response {
    fn head(&self, _: &str, _: &[Value]) -> Result<HeadOutcome, CallbackError> {
        Ok(self.head.clone())
    }

    fn returned(&self, _: &str, _: &Value) -> Result<Option<Value>, CallbackError> {
        Ok(Some(self.result.clone()))
    }
}

struct Trap;

impl Guest for Trap {
    fn head(&self, _: &str, _: &[Value]) -> Result<HeadOutcome, CallbackError> {
        Err(CallbackError::Trap)
    }
}

fn site(path: &'static str, arguments: &[ValueType], result: ValueType) -> &'static Site {
    Box::leak(Box::new(Site::new(
        path,
        HASH,
        Side::Both,
        Box::leak(arguments.to_vec().into_boxed_slice()),
        result,
        HEAD | RETURN | OVERWRITE,
    )))
}

fn install(site: &'static Site, guest: Arc<dyn Guest>, side: Side, points: u8) -> Registry {
    let mut registry = Registry::new([site]).unwrap();
    let prepared = registry
        .prepare(
            [Subscription {
                site_id: site.id(),
                side,
                points,
                module: Arc::new(Mod::new("review", guest)),
            }],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    registry
}

#[test]
fn replacement_block_states_cannot_panic_native_indexing() {
    let _side = enter_side(Side::Server);
    let site = site(
        "world::level::on_place",
        &[
            ValueType::U32,
            ValueType::U32,
            ValueType::BlockPos,
            ValueType::U32,
        ],
        ValueType::U32,
    );
    let arguments = vec![
        Value::Unsigned(0),
        Value::Unsigned(0),
        Value::List(vec![Value::Signed(0); 3]),
        Value::Unsigned(0),
    ];
    let mut replacement = arguments.clone();
    replacement[1] = Value::Unsigned(4_000_000_000);
    let _registry = install(
        site,
        Arc::new(Response {
            head: HeadOutcome::ReplaceArguments(replacement),
            result: Value::Unsigned(10),
        }),
        Side::Server,
        HEAD | RETURN,
    );
    assert_eq!(
        site.invoke(arguments, |arguments| {
            let Value::Unsigned(state) = arguments[1] else {
                panic!("expected a block state")
            };
            Value::Unsigned([10, 20][state as usize])
        }),
        Value::Unsigned(10)
    );
}

#[test]
fn command_sites_cannot_grant_their_own_capability() {
    for path in [
        "game::commands::execute_command",
        "game::commands::execute::execute_line",
        "game::chat::handle_chat_input",
    ] {
        for points in [HEAD, RETURN, OVERWRITE] {
            let site = site(
                path,
                &[ValueType::Handle, ValueType::String],
                ValueType::Unit,
            );
            let registry = Registry::new([site]).unwrap();
            let prepared = registry.prepare(
                [Subscription {
                    site_id: site.id(),
                    side: Side::Server,
                    points,
                    module: Arc::new(Mod::new("unprivileged", Arc::new(Trap))),
                }],
                |_, _, _| true,
            );
            assert!(prepared.is_err(), "unprivileged subscription to {path}");
        }
    }
}

#[test]
fn replacement_growth_speed_is_finite_and_positive() {
    let _side = enter_side(Side::Server);
    for replacement in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0] {
        let site = site("world::level::tick::crop_growth_speed", &[], ValueType::F32);
        let _registry = install(
            site,
            Arc::new(Response {
                head: HeadOutcome::Continue,
                result: Value::Float(replacement),
            }),
            Side::Server,
            RETURN,
        );
        assert_eq!(
            site.invoke(vec![], |_| Value::Float(1.0)),
            Value::Float(1.0)
        );
    }
}

#[test]
fn unentered_threads_do_not_dispatch_client_handlers() {
    let site = site("tests::unentered_review", &[ValueType::U32], ValueType::U32);
    let _registry = install(
        site,
        Arc::new(Response {
            head: HeadOutcome::ReplaceArguments(vec![Value::Unsigned(1)]),
            result: Value::Unsigned(1),
        }),
        Side::Client,
        HEAD | RETURN,
    );
    let result = std::thread::spawn(move || {
        site.invoke(vec![Value::Unsigned(0)], |arguments| arguments[0].clone())
    })
    .join()
    .unwrap();
    assert_eq!(result, Value::Unsigned(0));
}

#[test]
fn trapping_handlers_clear_the_fast_path_mask() {
    let _side = enter_side(Side::Server);
    let site = site("tests::disabled_review", &[], ValueType::Unit);
    let _registry = install(site, Arc::new(Trap), Side::Server, HEAD);
    assert_ne!(site.mask(), 0);
    assert_eq!(site.invoke(vec![], |_| Value::Unit), Value::Unit);
    assert_eq!(site.mask(), 0);
}

#[test]
fn client_handlers_do_not_set_the_server_mask() {
    let _side = enter_side(Side::Server);
    let site = site("tests::side_mask_review", &[], ValueType::Unit);
    let _registry = install(site, Arc::new(Trap), Side::Client, HEAD);
    assert_eq!(site.mask(), 0);
}
