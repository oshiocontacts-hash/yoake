use super::*;
use std::sync::Mutex;

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

    fn overwrite(&self, _: &str, _: &[Value]) -> Result<Value, CallbackError> {
        Ok(self.result.clone())
    }
}

fn site(path: &'static str, arguments: &[ValueType], result: ValueType) -> &'static Site {
    Box::leak(Box::new(Site::new(
        path,
        HASH,
        Side::Both,
        Box::leak(arguments.to_vec().into_boxed_slice()),
        result,
        ALL_POINTS,
    )))
}

fn install(site: &'static Site, module: Arc<Mod>, side: Side, points: u8) -> Registry {
    let mut registry = Registry::new([site]).unwrap();
    let prepared = registry
        .prepare(
            [Subscription {
                site_id: site.id(),
                side,
                points,
                module,
            }],
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    registry
}

fn response(head: HeadOutcome, result: Value) -> Arc<Mod> {
    Arc::new(Mod::new("response", Arc::new(Response { head, result })))
}

fn command_response(head: HeadOutcome, result: Value) -> Arc<Mod> {
    Arc::new(Mod::with_permissions(
        "command",
        Arc::new(Response { head, result }),
        Side::Server,
        ["service.commands".into()],
    ))
}

#[test]
fn live_block_state_bounds_prevent_native_index_panics() {
    let _side = enter_side(Side::Server);
    for path in [
        "world::level::on_place",
        "world::level::tick::crop_random_tick",
        "world::level::bonemeal::crop_bonemeal_increase",
        "game::drop_falling_block",
    ] {
        for index in if path.ends_with("on_place") {
            vec![1, 3]
        } else {
            vec![1]
        } {
            for count in [None, Some(2), Some(4_000_000_001)] {
                let arguments = vec![
                    Value::Unsigned(0),
                    Value::Unsigned(0),
                    Value::List(vec![Value::Signed(0); 3]),
                    Value::Unsigned(0),
                ];
                let mut replacement = arguments.clone();
                replacement[index] = Value::Unsigned(4_000_000_000);
                let site = site(
                    path,
                    &[
                        ValueType::U32,
                        ValueType::U32,
                        ValueType::BlockPos,
                        ValueType::U32,
                    ],
                    ValueType::U32,
                );
                let module = response(HeadOutcome::ReplaceArguments(replacement), Value::Unit);
                let _registry = install(site, Arc::clone(&module), Side::Server, HEAD);
                let result = site.invoke_validated(arguments, count, |arguments| {
                    let Value::Unsigned(state) = arguments[index] else {
                        panic!()
                    };
                    if count == Some(4_000_000_001) {
                        Value::Unsigned(state)
                    } else {
                        Value::Unsigned([10, 20][state as usize])
                    }
                });
                if count == Some(4_000_000_001) {
                    assert_eq!(result, Value::Unsigned(4_000_000_000));
                    assert!(!module.has_disabled_handlers());
                } else {
                    assert_eq!(result, Value::Unsigned(10));
                    assert_eq!(site.mask(), 0);
                    assert!(module.has_disabled_handlers());
                }
            }
        }
    }
}

#[test]
fn sensitive_sites_require_capabilities_at_every_point() {
    for path in [
        "game::commands::execute_command",
        "game::commands::execute::execute_line",
        "game::chat::handle_chat_input",
    ] {
        for side in [Side::Client, Side::Server] {
            let site = site(
                path,
                &[ValueType::Handle, ValueType::String],
                ValueType::S32,
            );
            let registry = Registry::new([site]).unwrap();
            for points in [HEAD, RETURN, OVERWRITE] {
                for permissions in [
                    vec![],
                    vec!["service.commands"],
                    vec!["service.commands", "service.chat"],
                ] {
                    let module = Arc::new(Mod::with_permissions(
                        "command",
                        Arc::new(Response {
                            head: HeadOutcome::Continue,
                            result: Value::Signed(0),
                        }),
                        side,
                        permissions
                            .iter()
                            .map(|permission| (*permission).to_owned()),
                    ));
                    let accepted = registry
                        .prepare(
                            [Subscription {
                                site_id: site.id(),
                                side,
                                points,
                                module,
                            }],
                            |module, site, _| module.permits(site),
                        )
                        .is_ok();
                    assert_eq!(
                        accepted,
                        permissions.contains(&"service.commands")
                            && (path != "game::chat::handle_chat_input"
                                || side != Side::Client
                                || permissions.contains(&"service.chat"))
                    );
                }
            }
        }
    }
}

#[test]
fn command_rewrites_use_the_shared_denylist() {
    for (path, lines) in [
        (
            "game::commands::execute_command",
            &["whitelist.off", "test.run", "pregen.start", "say"][..],
        ),
        (
            "game::commands::execute::execute_line",
            &[
                "/stop",
                "/minecraft:STOP",
                "/execute as @a RUN execute at @s run minecraft:op test",
                "/say safe",
            ][..],
        ),
        (
            "game::chat::handle_chat_input",
            &["/stop", "/minecraft:STOP", "/say safe"][..],
        ),
    ] {
        for line in lines {
            let _side = enter_side(Side::Server);
            let site = site(
                path,
                &[ValueType::Handle, ValueType::String],
                ValueType::S32,
            );
            let handle = Scope::default().handle("game", None);
            let original = vec![handle.clone(), Value::Text("/id".into())];
            let module = Arc::new(Mod::with_permissions(
                "command",
                Arc::new(Response {
                    head: HeadOutcome::ReplaceArguments(vec![handle, Value::Text((*line).into())]),
                    result: Value::Signed(0),
                }),
                Side::Server,
                ["service.commands".into()],
            ));
            let _registry = install(site, Arc::clone(&module), Side::Server, HEAD);
            let permitted = line.starts_with("say") || line.starts_with("/say");
            assert_eq!(
                site.invoke(original, |arguments| {
                    assert!(in_guest_command());
                    assert_eq!(arguments[1], Value::Text((*line).into()));
                    let gate = if path.ends_with("execute_command") {
                        guest_command_leaf_gate(line)
                    } else {
                        guest_command_gate(line)
                    };
                    Value::Signed(if gate.is_ok() { 3 } else { 0 })
                }),
                Value::Signed(if permitted { 3 } else { 0 })
            );
            assert!(!in_guest_command());
            assert!(!module.has_disabled_handlers());
        }
    }
}

#[test]
fn rewritten_chat_rejects_control_characters_before_the_native_body() {
    let _side = enter_side(Side::Server);
    let site = site(
        "game::chat::handle_chat_input",
        &[ValueType::String, ValueType::String],
        ValueType::Unit,
    );
    let original = vec![Value::Text("game".into()), Value::Text("/say safe".into())];
    let module = command_response(
        HeadOutcome::ReplaceArguments(vec![original[0].clone(), Value::Text("\u{1}/stop".into())]),
        Value::Unit,
    );
    let _registry = install(site, Arc::clone(&module), Side::Server, HEAD);
    site.invoke(original.clone(), |arguments| {
        assert_eq!(arguments, original);
        assert!(!in_guest_command());
        Value::Unit
    });
    assert!(module.has_disabled_handlers());
}

#[test]
fn any_changed_command_argument_scopes_the_native_body() {
    let _side = enter_side(Side::Server);
    for changed in [false, true] {
        let site = site(
            "game::commands::execute_command",
            &[ValueType::String, ValueType::String, ValueType::U32],
            ValueType::Unit,
        );
        let original = vec![
            Value::Text("game".into()),
            Value::Text("whitelist.off".into()),
            Value::Unsigned(1),
        ];
        let mut replacement = original.clone();
        replacement[2] = Value::Unsigned(if changed { 2 } else { 1 });
        let _registry = install(
            site,
            command_response(HeadOutcome::ReplaceArguments(replacement), Value::Unit),
            Side::Server,
            HEAD,
        );
        site.invoke(original, |_| {
            assert_eq!(in_guest_command(), changed);
            assert_eq!(guest_command_leaf_gate("whitelist.off").is_err(), changed);
            Value::Unit
        });
        assert!(!in_guest_command());
    }
}

#[test]
fn echoed_engine_values_do_not_disable_argument_replacements() {
    let _side = enter_side(Side::Server);
    for (path, engine_value) in [
        ("game::creatures::hurt_player_by", f64::from(f32::MAX)),
        ("game::explosion::server_explode", 256.0),
    ] {
        let site = site(
            path,
            &[ValueType::U32, ValueType::Vec3, ValueType::F32],
            ValueType::Unit,
        );
        let original = vec![
            Value::Unsigned(1),
            Value::List(vec![Value::Float(0.0); 3]),
            Value::Float(engine_value),
        ];
        let mut replacement = original.clone();
        replacement[0] = Value::Unsigned(2);
        let module = response(
            HeadOutcome::ReplaceArguments(replacement.clone()),
            Value::Unit,
        );
        let _registry = install(site, Arc::clone(&module), Side::Server, HEAD);
        site.invoke(original, |arguments| {
            assert_eq!(arguments, replacement);
            Value::Unit
        });
        assert!(!module.has_disabled_handlers());
        assert_eq!(site.mask(), HEAD);
    }
}

#[test]
fn guest_command_scope_is_nested_thread_local_and_unwind_safe() {
    assert!(!in_guest_command());
    assert!(guest_command_leaf_gate("whitelist.off").is_ok());
    {
        let _scope = guest_command_scope();
        assert!(guest_command_leaf_gate("whitelist.off").is_err());
        assert!(
            std::panic::catch_unwind(|| {
                let _nested = guest_command_scope();
                panic!("unwind the nested command scope");
            })
            .is_err()
        );
        assert!(in_guest_command());
        std::thread::spawn(|| assert!(!in_guest_command()))
            .join()
            .unwrap();
    }
    assert!(!in_guest_command());
}

#[test]
fn invalid_numbers_fall_back_at_all_injection_points() {
    let _side = enter_side(Side::Server);
    for path in [
        "world::level::tick::crop_growth_speed",
        "game::hand::attack_damage",
        "game::hand::main_hand_attack_speed",
        "world::living_motion::jump_power",
    ] {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 1.0e20] {
            for points in [HEAD, RETURN, OVERWRITE] {
                let site = site(path, &[], ValueType::F32);
                let module = response(
                    HeadOutcome::Cancel(Value::Float(value)),
                    Value::Float(value),
                );
                let _registry = install(site, Arc::clone(&module), Side::Server, points);
                assert_eq!(
                    site.invoke(vec![], |_| Value::Float(1.0)),
                    Value::Float(1.0)
                );
                assert!(module.has_disabled_handlers());
                assert_eq!(site.mask(), 0);
            }
        }
    }
    for (path, argument, maximum) in [
        ("game::explosion::server_explode", 2, 128.0),
        ("game::creatures::hurt_player_by", 2, 1_000_000.0),
        ("game::creatures::mob_hits_mob", 2, 1_000_000.0),
        ("game::creatures::mob_hits_player", 1, 1_000_000.0),
        ("world::food::add_exhaustion", 1, 40.0),
        ("world::food::eat_properties", 2, 40.0),
    ] {
        for value in [f64::NAN, f64::INFINITY, -1.0, maximum + 1.0] {
            let site = site(path, &[ValueType::F32; 3], ValueType::Unit);
            let mut replacement = vec![Value::Float(1.0); 3];
            replacement[argument] = Value::Float(value);
            let _registry = install(
                site,
                response(HeadOutcome::ReplaceArguments(replacement), Value::Unit),
                Side::Server,
                HEAD,
            );
            site.invoke(vec![Value::Float(1.0); 3], |arguments| {
                assert_eq!(arguments, vec![Value::Float(1.0); 3]);
                Value::Unit
            });
            assert_eq!(site.mask(), 0);
        }
    }
}

struct Ordered {
    name: &'static str,
    order: Arc<Mutex<Vec<&'static str>>>,
}

impl Guest for Ordered {
    fn head(&self, _: &str, _: &[Value]) -> Result<HeadOutcome, CallbackError> {
        self.order.lock().unwrap().push(self.name);
        Ok(HeadOutcome::Continue)
    }
}

#[test]
fn handler_order_uses_bare_mod_id() {
    let _side = enter_side(Side::Server);
    let site = site("test::order", &[], ValueType::Unit);
    let mut registry = Registry::new([site]).unwrap();
    let order = Arc::new(Mutex::new(Vec::new()));
    let prepared = registry
        .prepare(
            ["foo-extras", "foo"].map(|name| Subscription {
                site_id: site.id(),
                side: Side::Server,
                points: HEAD,
                module: Arc::new(Mod::new(
                    name,
                    Arc::new(Ordered {
                        name,
                        order: Arc::clone(&order),
                    }),
                )),
            }),
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    site.invoke(vec![], |_| Value::Unit);
    assert_eq!(*order.lock().unwrap(), ["foo", "foo-extras"]);
}

#[test]
fn unentered_threads_do_not_dispatch_and_masks_are_side_specific() {
    let site = site("test::thread_side", &[], ValueType::S32);
    let _registry = install(
        site,
        response(HeadOutcome::Cancel(Value::Signed(7)), Value::Signed(7)),
        Side::Client,
        HEAD,
    );
    std::thread::spawn(move || {
        assert!(!site.has_handlers());
        assert_eq!(site.mask(), 0);
        assert_eq!(site.invoke(vec![], |_| Value::Signed(2)), Value::Signed(2));
        let _side = enter_side(Side::Client);
        assert!(site.has_handlers());
        assert_eq!(site.mask(), HEAD);
        assert_eq!(site.invoke(vec![], |_| Value::Signed(2)), Value::Signed(7));
        {
            let _side = enter_side(Side::Server);
            assert!(!site.has_handlers());
            assert_eq!(site.mask(), 0);
        }
        assert!(site.has_handlers());
        assert_eq!(site.mask(), HEAD);
    })
    .join()
    .unwrap();
}

struct DisablingGuest(AtomicBool);

impl Guest for DisablingGuest {
    fn is_disabled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }

    fn head(&self, _: &str, _: &[Value]) -> Result<HeadOutcome, CallbackError> {
        self.0.store(true, Ordering::Relaxed);
        Err(CallbackError::Trap)
    }
}

#[test]
fn instance_disable_clears_every_registered_site_mask() {
    let _side = enter_side(Side::Server);
    let first = site("test::disable_first", &[], ValueType::Unit);
    let second = site("test::disable_second", &[], ValueType::Unit);
    let module = Arc::new(Mod::new(
        "disabled",
        Arc::new(DisablingGuest(AtomicBool::new(false))),
    ));
    let mut registry = Registry::new([first, second]).unwrap();
    let prepared = registry
        .prepare(
            [first, second].map(|site| Subscription {
                site_id: site.id(),
                side: Side::Server,
                points: HEAD | RETURN,
                module: Arc::clone(&module),
            }),
            |_, _, _| true,
        )
        .unwrap();
    registry.publish(prepared).unwrap();
    assert_eq!(second.mask(), HEAD | RETURN);
    first.invoke(vec![], |_| Value::Unit);
    assert_eq!(first.mask(), 0);
    assert_eq!(second.mask(), 0);
}

struct SnapshotDisabledGuest {
    disabled: AtomicBool,
    pause_next: AtomicBool,
    snapshot_taken: std::sync::mpsc::SyncSender<()>,
    resume: Mutex<std::sync::mpsc::Receiver<()>>,
}

impl Guest for SnapshotDisabledGuest {
    fn is_disabled(&self) -> bool {
        let disabled = self.disabled.load(Ordering::Acquire);
        if self.pause_next.swap(false, Ordering::AcqRel) {
            self.snapshot_taken.send(()).unwrap();
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        disabled
    }
}

#[test]
fn concurrent_mask_refresh_cannot_restore_disabled_handlers() {
    let _side = enter_side(Side::Server);
    let site = site("test::concurrent_mask_refresh", &[], ValueType::Unit);
    let (snapshot_sender, snapshot_receiver) = std::sync::mpsc::sync_channel(1);
    let (resume_sender, resume_receiver) = std::sync::mpsc::sync_channel(1);
    let guest = Arc::new(SnapshotDisabledGuest {
        disabled: AtomicBool::new(false),
        pause_next: AtomicBool::new(false),
        snapshot_taken: snapshot_sender,
        resume: Mutex::new(resume_receiver),
    });
    let _registry = install(
        site,
        Arc::new(Mod::new("snapshot", guest.clone())),
        Side::Server,
        HEAD,
    );
    guest.pause_next.store(true, Ordering::Release);
    std::thread::scope(|threads| {
        let first = threads.spawn(|| site.refresh_masks());
        snapshot_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        guest.disabled.store(true, Ordering::Release);
        let (done_sender, done_receiver) = std::sync::mpsc::sync_channel(1);
        let second = threads.spawn(move || {
            site.refresh_masks();
            done_sender.send(()).unwrap();
        });
        let _ = done_receiver.recv_timeout(std::time::Duration::from_millis(250));
        resume_sender.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
    });
    assert_eq!(site.mask(), 0);
}
