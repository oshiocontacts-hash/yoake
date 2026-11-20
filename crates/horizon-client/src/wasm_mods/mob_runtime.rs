//! The guest side of fully custom mobs (F4M-10): `mobs::custom::script::Runtime` over the server
//! modules of the installed packages. Each call goes to every module that subscribed to the hook
//! (the module's own subscription list is the filter); a module that is busy (re-entered from its
//! own service call), disabled, or that traps or overruns its limits answers nothing, and the engine
//! uses the fallback of the call.
use super::server_services::{ServerSlot, endpoints};
use super::*;
use crate::mobs::custom::script::{HurtReply, InteractReply, MobScriptValue, Runtime, ScriptVec3};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use yoake_wasm::ServerFullMod;
use yoake_wasm::full_bindings::client::yoake::mods::engine_types as et;

pub(crate) struct WasmRuntime;

/// Installs the runtime (once; the first server module that registers does it, and `entry::run` when native mods
/// exist): the WASM modules first, then the native mods (`mob_native`, through `mod_api::mods()`).
pub(crate) fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        crate::mobs::custom::script::set_runtime(Some(Arc::new(crate::mobs::custom::script::Chain {
            first: Arc::new(WasmRuntime),
            second: Arc::new(crate::mob_native::NativeRuntime::process()),
        })));
        crate::mobs::custom::ai::interact::set_hook(Some(Arc::new(WasmRuntime)));
    });
}

/// Installs the client runtime (once): the WASM client modules first, then the native mods.
pub(crate) fn install_client() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        crate::mobs::custom::script::set_client_runtime(Some(Arc::new(crate::mobs::custom::script::ClientChain {
            first: Arc::new(WasmRuntime),
            second: Arc::new(crate::mob_native::NativeClientRuntime::process()),
        })));
    });
}

fn et_vec(v: &ScriptVec3) -> et::ScriptVec3 {
    et::ScriptVec3 { x: v.x, y: v.y, z: v.z }
}

pub(super) fn et_value(v: &MobScriptValue) -> et::MobScriptValue {
    match v {
        MobScriptValue::Bool(b) => et::MobScriptValue::Bool(*b),
        MobScriptValue::Int(n) => et::MobScriptValue::Int(*n),
        MobScriptValue::Float(n) => et::MobScriptValue::Float(*n),
        MobScriptValue::Text(s) => et::MobScriptValue::Text(s.clone()),
        MobScriptValue::Vec3(p) => et::MobScriptValue::Vec3(et_vec(p)),
        MobScriptValue::Choice(s) => et::MobScriptValue::Choice(s.clone()),
    }
}

/// Runs `f` on every server module that has a running instance, in registration order, each inside
/// its own invocation scope (no game access: a guest's `spawn` answers an error there). `f` returns
/// `None` when the module did not answer. The results of the modules that did, in order.
fn each<R>(mut f: impl FnMut(&mut ServerFullMod) -> Option<R>) -> Vec<R> {
    let mut out = Vec::new();
    for slot in endpoints() {
        if let Some(r) = one(&slot, &mut f) {
            out.push(r);
        }
    }
    out
}

fn one<R>(slot: &Arc<ServerSlot>, f: &mut impl FnMut(&mut ServerFullMod) -> Option<R>) -> Option<R> {
    let mut state = slot.state.try_lock().ok()?;
    let module = state.module.as_mut()?;
    if module.is_disabled() {
        return None;
    }
    slot.dirty.store(true, Ordering::Relaxed);
    let mut invocation = Invocation { host: HostAccess::None, batch: None, batch_start: 0, client: ClientExtras::default() };
    let r = {
        let _scope = Scope::enter(&mut invocation);
        f(module)
    };
    if module.is_disabled() {
        log::error!("wasm mod (server) disabled while scripting a custom mob: trapped or exceeded its limits");
    }
    r
}

/// `Some(true)` when any module said yes, `Some(false)` when some answered and none said yes,
/// `None` when nobody answered.
fn any_true(answers: Vec<bool>) -> Option<bool> {
    if answers.is_empty() { None } else { Some(answers.contains(&true)) }
}

impl Runtime for WasmRuntime {
    fn live(&self) -> bool {
        endpoints().iter().any(|slot| {
            slot.state.try_lock().map(|s| s.module.as_ref().is_some_and(|m| !m.is_disabled())).unwrap_or(true)
        })
    }
    fn goal_can_use(&self, mob: u32, goal: &str) -> Option<bool> {
        any_true(each(|m| m.dispatch("mob_goal_can_use", |e, s| e.yoake_mods_server_hooks().call_mob_goal_can_use(s, mob, goal))))
    }
    fn goal_can_continue(&self, mob: u32, goal: &str) -> Option<bool> {
        any_true(each(|m| m.dispatch("mob_goal_can_continue", |e, s| e.yoake_mods_server_hooks().call_mob_goal_can_continue(s, mob, goal))))
    }
    fn goal_start(&self, mob: u32, goal: &str) {
        each(|m| m.dispatch("mob_goal_start", |e, s| e.yoake_mods_server_hooks().call_mob_goal_start(s, mob, goal)));
    }
    fn goal_tick(&self, mob: u32, goal: &str) {
        each(|m| m.dispatch("mob_goal_tick", |e, s| e.yoake_mods_server_hooks().call_mob_goal_tick(s, mob, goal)));
    }
    fn goal_stop(&self, mob: u32, goal: &str) {
        each(|m| m.dispatch("mob_goal_stop", |e, s| e.yoake_mods_server_hooks().call_mob_goal_stop(s, mob, goal)));
    }
    fn mob_tick(&self, mob: u32) {
        each(|m| m.dispatch("mob_tick", |e, s| e.yoake_mods_server_hooks().call_mob_tick(s, mob)));
    }
    fn mob_spawn_check(&self, type_id: &str, dimension: &str, pos: [i32; 3]) -> Option<bool> {
        let pos = pos.to_vec();
        any_true(each(|m| m.dispatch("mob_spawn_check", |e, s| e.yoake_mods_server_hooks().call_mob_spawn_check(s, type_id, dimension, &pos))))
    }
    fn mob_spawned(&self, mob: u32, type_id: &str, reason: &str) {
        each(|m| m.dispatch("custom_mob_spawned", |e, s| e.yoake_mods_server_hooks().call_custom_mob_spawned(s, mob, type_id, reason)));
    }
    fn mob_hurt(&self, mob: u32, part: &str, source: &str, amount: f32) -> Option<HurtReply> {
        let mut reply = HurtReply { amount, cancel: false };
        let mut answered = false;
        let (mut a, mut c) = (amount, false);
        for r in each(|m| m.dispatch("mob_hurt", |e, s| e.yoake_mods_server_hooks().call_mob_hurt(s, mob, part, source, a, c))) {
            answered = true;
            if let Some(x) = r.amount.filter(|x| x.is_finite() && *x >= 0.0) {
                a = x;
            }
            c |= r.cancel.unwrap_or(false);
        }
        reply.amount = a;
        reply.cancel = c;
        answered.then_some(reply)
    }
    fn mob_died(&self, mob: u32, type_id: &str, attacker: Option<u32>) {
        each(|m| m.dispatch("mob_died", |e, s| e.yoake_mods_server_hooks().call_mob_died(s, mob, type_id, attacker.unwrap_or(0))));
    }
    fn mob_interact(&self, mob: u32, player: &str, hand: &str, item: &str) -> InteractReply {
        for r in each(|m| m.dispatch("mob_interact", |e, s| e.yoake_mods_server_hooks().call_mob_interact(s, mob, player, hand, item, ""))) {
            match r.reply.as_deref() {
                Some("consume") => return InteractReply::Consume,
                Some("success") => return InteractReply::Success,
                _ => {}
            }
        }
        InteractReply::Pass
    }
    fn mob_anim_event(&self, mob: u32, name: &str) {
        each(|m| m.dispatch("mob_anim_event", |e, s| e.yoake_mods_server_hooks().call_mob_anim_event(s, mob, name)));
    }
    fn mob_load(&self, mob: u32, data: &[u8]) {
        each(|m| m.dispatch("mob_load", |e, s| e.yoake_mods_server_hooks().call_mob_load(s, mob, data)));
    }
    fn mob_save(&self, mob: u32, data: &[u8]) -> Option<Vec<u8>> {
        let mut cur = data.to_vec();
        let mut changed = false;
        for r in each(|m| m.dispatch("mob_save", |e, s| e.yoake_mods_server_hooks().call_mob_save(s, mob, &cur))) {
            if let Some(d) = r.data.filter(|d| d.len() <= crate::mobs::custom::script::MAX_DATA_BYTES) {
                cur = d;
                changed = true;
            }
        }
        changed.then_some(cur)
    }
    fn mob_tamed(&self, mob: u32, owner: &str) {
        each(|m| m.dispatch("mob_tamed", |e, s| e.yoake_mods_server_hooks().call_mob_tamed(s, mob, owner)));
    }
    fn mob_bred(&self, mob: u32, partner: u32, child: u32) {
        each(|m| m.dispatch("mob_bred", |e, s| e.yoake_mods_server_hooks().call_mob_bred(s, mob, partner, child)));
    }
    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue) {
        let (o, n) = (et_value(old), et_value(new));
        each(|m| m.dispatch("synced_changed_server", |e, s| e.yoake_mods_server_hooks().call_synced_changed_server(s, mob, key, &o, &n)));
    }
}

/// `mob-interact` for the data dispatch of `mobs::custom::ai::interact`: the guests answer first.
impl crate::mobs::custom::ai::interact::Hook for WasmRuntime {
    fn mob_interact(&self, mob: &mut crate::mobs::Mob, ctx: &crate::mobs::custom::ai::interact::Context) -> crate::mobs::custom::ai::interact::Reply {
        use crate::mobs::custom::ai::interact::{ClickHand, Reply};
        let hand = if ctx.hand == ClickHand::Off { "off" } else { "main" };
        let player = crate::mobs::custom::script::uuid_text(ctx.player);
        match crate::mobs::custom::script::interact(mob, &player, hand, ctx.item.unwrap_or("")) {
            InteractReply::Pass => Reply::Pass,
            InteractReply::Consume => Reply::Consume,
            InteractReply::Success => Reply::Success,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The client modules
// ---------------------------------------------------------------------------------------------

type ClientSlot = Arc<Mutex<super::full::SideState<yoake_wasm::ClientFullMod>>>;

static CLIENTS: Mutex<Vec<std::sync::Weak<Mutex<super::full::SideState<yoake_wasm::ClientFullMod>>>>> = Mutex::new(Vec::new());

/// Makes a package's client module reachable by the script hooks of custom mobs.
pub(super) fn register_client(slot: &ClientSlot) {
    let mut g = CLIENTS.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|w| w.strong_count() > 0);
    g.push(Arc::downgrade(slot));
    drop(g);
    install_client();
}

fn client_each<R>(mut f: impl FnMut(&mut yoake_wasm::ClientFullMod) -> Option<R>) -> Vec<R> {
    let slots: Vec<ClientSlot> = CLIENTS.lock().unwrap_or_else(|e| e.into_inner()).iter().filter_map(|w| w.upgrade()).collect();
    let mut out = Vec::new();
    for slot in slots {
        let Ok(mut state) = slot.try_lock() else { continue };
        let Some(module) = state.module.as_mut() else { continue };
        if module.is_disabled() {
            continue;
        }
        let mut invocation = Invocation { host: HostAccess::None, batch: None, batch_start: 0, client: ClientExtras::default() };
        let r = {
            let _scope = Scope::enter(&mut invocation);
            f(module)
        };
        if let Some(r) = r {
            out.push(r);
        }
    }
    out
}

impl crate::mobs::custom::script::ClientRuntime for WasmRuntime {
    fn live(&self) -> bool {
        CLIENTS.lock().unwrap_or_else(|e| e.into_inner()).iter().filter_map(|w| w.upgrade()).any(|slot| {
            slot.try_lock().map(|s| s.module.as_ref().is_some_and(|m| !m.is_disabled())).unwrap_or(true)
        })
    }
    fn mob_pose(&self, mob: u32, kind: &str, frame: &crate::mobs::custom::script::MobPoseFrame) -> Vec<crate::mobs::custom::script::MobBoneWrite> {
        let f = et::MobPoseFrame {
            time: frame.time,
            partial: frame.partial,
            ground_speed: frame.ground_speed,
            head_yaw: frame.head_yaw,
            head_pitch: frame.head_pitch,
            on_ground: frame.on_ground,
            target: frame.target.as_ref().map(et_vec),
        };
        let mut out = Vec::new();
        for r in client_each(|m| m.dispatch("mob_pose", |e, s| e.yoake_mods_client_hooks().call_mob_pose(s, mob, kind, f, &[]))) {
            for w in r.writes.unwrap_or_default() {
                let v = |p: et::ScriptVec3| ScriptVec3 { x: p.x, y: p.y, z: p.z };
                out.push(crate::mobs::custom::script::MobBoneWrite {
                    bone: w.bone,
                    additive: w.additive,
                    translation: w.translation.map(v),
                    rotation_degrees: w.rotation_degrees.map(v),
                    scale: w.scale.map(v),
                    visible: w.visible,
                });
            }
        }
        out.truncate(crate::mobs::custom::script::MAX_POSE_WRITES);
        out
    }
    fn mob_anim_event(&self, mob: u32, name: &str) {
        client_each(|m| m.dispatch("mob_anim_event_client", |e, s| e.yoake_mods_client_hooks().call_mob_anim_event_client(s, mob, name)));
    }
    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue) {
        let (o, n) = (et_value(old), et_value(new));
        client_each(|m| m.dispatch("synced_changed_client", |e, s| e.yoake_mods_client_hooks().call_synced_changed_client(s, mob, key, &o, &n)));
    }
}

// These tests need the resource-free world of `mobs::mob_hook_bench`:
// `cargo test -p horizon-client --lib --features yoake-hook-bench wasm_mods::mob_runtime`.
#[cfg(all(test, feature = "yoake-hook-bench"))]
mod tests {
    //! The fox_ai guest (`crates/yoake-wasm-sdk/examples/fox_ai`) scripting the fixture Cinder Fox
    //! through the real sandbox, services and engine goal selector; a guest that traps or never
    //! returns; the permission gate. Guests that cannot be built here skip their test (see
    //! `yoake_wasm::test_guests`).
    use super::super::full::{FullMod, GameServerHost, ModuleSource, load_server};
    use super::super::mob_services;
    use crate::mobs::{
        Mob, MobEvent, Who,
        custom::{CustomType, ai::attacks, build, script},
        goals::Cx,
        mob_hook_bench::BenchWorld,
    };
    use horizon_core::{geometry::Vec3, random::LegacyRandom};
    use horizon_mods::custom_mob::CustomMobFile;
    use std::sync::Arc;
    use yoake_wasm::test_guests::{FOX_AI, FOX_AI_SPIN, FOX_AI_TRAP, Guest, load};

    const FOX_JSON: &str = include_str!("../../../../tools/fixtures/cinder_fox/content/data/sample_mobs/horizon/entity_type/cinder_fox.json");

    fn fox_type() -> Arc<CustomType> {
        let file = CustomMobFile::parse("sample_mobs:cinder_fox", FOX_JSON.as_bytes()).unwrap();
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixtures/cinder_fox/content");
        let model = file.load_model(&mut |p| std::fs::read(root.join(p)).map_err(|e| e.to_string())).unwrap();
        Arc::new(CustomType::new(file, Some(&model)).unwrap())
    }

    fn fox(id: u32, at: Vec3) -> Mob {
        let t = fox_type();
        let kind = Arc::new(build::mob_type(&t));
        let mut m = Mob::new(id, kind, at, 0.0);
        build::register(&mut m);
        m.on_ground = true;
        m
    }

    /// Loads `guest` as package `id` with (or without) `service.mobs`; keeps it registered while alive.
    fn package(guest: &Guest, id: &str, grant: bool) -> Option<Arc<FullMod>> {
        let bytes = load(guest)?;
        let server = load_server(&ModuleSource { id, bytes: &bytes, commands: false, gpu: false }).expect("the guest loads");
        if grant {
            mob_services::grant(id);
        }
        Some(Arc::new(FullMod::new(id.into(), None, Some(server))))
    }

    /// `tools/fixtures/cinder_fox` (the package: `yak.toml`, `content/`) copied to a scratch folder as an installed
    /// package (`yoake-mods/sample_mobs/0.1.0/`), with the built guest written to the `bin/wasm/fox_ai.wasm` its
    /// `[wasm]` section names. The scratch folder goes away with the value; the repository is never written.
    struct DevMod {
        game: std::path::PathBuf,
        package: std::path::PathBuf,
    }

    impl DevMod {
        fn stage(guest: &[u8]) -> Self {
            fn copy(from: &std::path::Path, to: &std::path::Path) {
                std::fs::create_dir_all(to).unwrap();
                for entry in std::fs::read_dir(from).unwrap() {
                    let entry = entry.unwrap();
                    let target = to.join(entry.file_name());
                    if entry.file_type().unwrap().is_dir() {
                        copy(&entry.path(), &target);
                    } else {
                        std::fs::copy(entry.path(), target).unwrap();
                    }
                }
            }
            static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            let game = std::env::temp_dir().join(format!(
                "cinder-fox-dev-mod-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let package = game.join("yoake-mods/sample_mobs/0.1.0");
            let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/fixtures/cinder_fox");
            std::fs::create_dir_all(&package).unwrap();
            std::fs::copy(fixture.join("yak.toml"), package.join("yak.toml")).unwrap();
            copy(&fixture.join("content"), &package.join("content"));
            std::fs::create_dir_all(package.join("bin/wasm")).unwrap();
            std::fs::write(package.join("bin/wasm/fox_ai.wasm"), guest).unwrap();
            std::fs::write(game.join("yoake-mods.txt"), "sample_mobs 0.1.0\n").unwrap();
            Self { game, package }
        }
    }

    impl Drop for DevMod {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.game);
        }
    }

    /// The Cinder Fox dev mod as the game loads it: its `yak.toml` is read, its content must be the same package
    /// (id and version), the package yields one server module with the `service.mobs` permission and the guest's
    /// bytes, and that module is loaded and granted as `wasm_mods::extend` does. `None` when the guest cannot be built.
    fn dev_mod_package() -> Option<(DevMod, Arc<FullMod>)> {
        use crate::yak_mods::{Installed, WasmSide, wasm_mods_from};
        let bytes = load(&FOX_AI)?;
        let dev = DevMod::stage(&bytes);
        let manifest = horizon_mods::yak::Manifest::read(&dev.package.join("yak.toml")).expect("yak.toml is valid");
        assert_eq!(manifest.wasm.as_ref().and_then(|w| w.server.as_deref()), Some("bin/wasm/fox_ai.wasm"));
        let content = horizon_mods::manifest::Package::read(&dev.package.join("content")).expect("content/ is a content mod");
        assert_eq!(content.manifest.id, manifest.id, "content and package are the same mod");
        assert_eq!(content.manifest.version, manifest.version.to_string());
        let entry = Installed::new_for_test(dev.package.clone(), manifest);
        let found = wasm_mods_from(std::slice::from_ref(&entry), WasmSide::Server, &[], true);
        assert_eq!(found.len(), 1, "the package declares one server module");
        let module = &found[0];
        assert_eq!(module.mod_id, "sample_mobs");
        assert_eq!(module.bytes, bytes, "the module is the built guest");
        assert!(module.permissions.iter().any(|p| p == "service.mobs"));
        if module.permissions.iter().any(|p| p == "service.mobs") {
            mob_services::grant(&module.mod_id);
        }
        let source = super::super::full::ServerSource::new(&module.mod_id, module.bytes.clone(), module.commands);
        let server = load_server(&source.module()).expect("the guest loads");
        Some((dev, Arc::new(FullMod::new(module.mod_id.clone(), None, Some(server)).with_server_source(source))))
    }

    /// One evaluation of the goal selector at game time `t`.
    fn select(m: &mut Mob, fx: &BenchWorld, others: &[crate::mobs::Seen], lr: &mut LegacyRandom, events: &mut Vec<MobEvent>, t: i64) {
        let mut w = fx.world();
        w.game_time = t;
        w.others = others;
        let mut goals = std::mem::take(&mut m.ai.goals);
        {
            let mut cx = Cx { w: &w, lr, events };
            goals.tick(m, &mut cx);
        }
        m.ai.goals = goals;
    }

    /// Whether package `id` still has a running (not disabled) server module.
    fn module_live(id: &str) -> bool {
        super::super::server_services::endpoint(id)
            .is_some_and(|slot| slot.state.lock().unwrap().module.as_ref().is_some_and(|m| !m.is_disabled()))
    }

    fn pounce_running(m: &Mob) -> bool {
        m.ai.goals.is_running("ScriptGoal")
    }

    #[test]
    fn the_fox_ai_guest_pounces_when_the_target_is_three_to_six_blocks_away_in_sight() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        script::reset();
        // The real dev mod: the fixture's `yak.toml` and content with the built fox_ai guest.
        let Some((_dev, _pkg)) = dev_mod_package() else { return };
        let fx = BenchWorld::new();
        let mut lr = LegacyRandom::new(1);
        let mut events = Vec::new();
        for (distance, expect) in [(2.0, false), (8.0, false), (4.0, true)] {
            let mut m = fox(1, Vec3::new(0.5, 1.0, 0.5));
            let prey = fox(2, Vec3::new(0.5 + distance, 1.0, 0.5)).seen(None);
            m.ai.target = Some(Who::Mob(2));
            select(&mut m, &fx, std::slice::from_ref(&prey), &mut lr, &mut events, 1000);
            assert_eq!(pounce_running(&m), expect, "distance {distance}");
            if !expect {
                continue;
            }
            // `start`: the animation was asked for.
            assert!(m.extra.custom.as_ref().unwrap().anim.is_some(), "the pounce animation was asked for");
            // The server timeline event `hit` makes the leap and the bite.
            attacks::deliver_frame(&mut m, "hit");
            let w = {
                let mut w = fx.world();
                w.game_time = 1001;
                w.others = std::slice::from_ref(&prey);
                w
            };
            let v0 = m.velocity;
            events.clear();
            assert!(script::pre_tick(&mut m, &w, &mut events));
            assert!(m.velocity.y > v0.y + 0.3, "the leap: {:?}", m.velocity);
            assert!(events.iter().any(|e| matches!(e, MobEvent::AttackMob { target: 2, .. })), "the bite: {events:?}");
            // The goal ends once the animation is over.
            // (The prey has moved off, or the fox would pounce again at once.)
            let gone = fox(2, Vec3::new(30.5, 1.0, 0.5)).seen(None);
            let mut t = 1001;
            while pounce_running(&m) && t < 1100 {
                t += 1;
                select(&mut m, &fx, std::slice::from_ref(&gone), &mut lr, &mut events, t);
            }
            assert!(t - 1001 <= 20, "stopped after {} ticks", t - 1001);
        }
        script::reset();
    }

    #[test]
    fn without_service_mobs_the_guest_cannot_read_the_mob_and_the_goal_never_starts() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        script::reset();
        let Some(_pkg) = package(&FOX_AI, "fox_ai_ungranted", false) else { return };
        let fx = BenchWorld::new();
        let mut lr = LegacyRandom::new(1);
        let mut events = Vec::new();
        let mut m = fox(1, Vec3::new(0.5, 1.0, 0.5));
        let prey = fox(2, Vec3::new(4.5, 1.0, 0.5)).seen(None);
        m.ai.target = Some(Who::Mob(2));
        select(&mut m, &fx, std::slice::from_ref(&prey), &mut lr, &mut events, 1000);
        assert!(!pounce_running(&m));
        use yoake_wasm::ServerHost;
        let mut host = GameServerHost { id: "fox_ai_ungranted".into(), commands: false };
        let e = host.mob_info_call(1).unwrap_err();
        assert!(e.contains("service.mobs"), "{e}");
        script::reset();
    }

    #[test]
    fn a_guest_that_traps_is_disabled_and_the_fox_carries_on() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        script::reset();
        let Some(_pkg) = package(&FOX_AI_TRAP, "fox_ai_trap", true) else { return };
        let fx = BenchWorld::new();
        let mut lr = LegacyRandom::new(1);
        let mut events = Vec::new();
        let mut m = fox(1, Vec3::new(0.5, 1.0, 0.5));
        let prey = fox(2, Vec3::new(4.5, 1.0, 0.5)).seen(None);
        m.ai.target = Some(Who::Mob(2));
        for t in 1000..1030 {
            select(&mut m, &fx, std::slice::from_ref(&prey), &mut lr, &mut events, t);
            assert!(!pounce_running(&m));
        }
        assert!(!module_live("fox_ai_trap"), "the module is disabled after its trap");
        script::reset();
    }

    #[test]
    fn a_guest_that_never_returns_is_stopped_by_its_limits() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        script::reset();
        let Some(_pkg) = package(&FOX_AI_SPIN, "fox_ai_spin", true) else { return };
        let fx = BenchWorld::new();
        let mut lr = LegacyRandom::new(1);
        let mut events = Vec::new();
        let mut m = fox(1, Vec3::new(0.5, 1.0, 0.5));
        let prey = fox(2, Vec3::new(4.5, 1.0, 0.5)).seen(None);
        m.ai.target = Some(Who::Mob(2));
        let began = std::time::Instant::now();
        select(&mut m, &fx, std::slice::from_ref(&prey), &mut lr, &mut events, 1000);
        assert!(began.elapsed() < std::time::Duration::from_secs(5), "the call was cut off: {:?}", began.elapsed());
        assert!(!pounce_running(&m));
        assert!(!module_live("fox_ai_spin"));
        script::reset();
    }

    #[test]
    fn the_hold_services_queue_their_commands_and_are_gated_by_service_mobs() {
        use yoake_wasm::ServerHost;
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        script::reset();
        let m = fox(1, Vec3::new(0.5, 1.0, 0.5));
        script::publish(script::MobView::of(&m).unwrap());
        mob_services::grant("hold_granted");
        let mut host = GameServerHost { id: "hold_granted".into(), commands: false };
        assert_eq!(host.mob_set_controller_state_hold("locomotion".into(), 1, Some(10), "run".into()), Ok(true));
        assert_eq!(host.mob_set_controller_state_hold("locomotion".into(), 1, None, "run".into()), Ok(true));
        assert_eq!(host.mob_release_controller_state("locomotion".into(), 1), Ok(true));
        assert_eq!(
            script::take_for(1),
            vec![
                script::Command::HoldControllerState { controller: "locomotion".into(), state: "run".into(), hold_ticks: Some(10) },
                script::Command::HoldControllerState { controller: "locomotion".into(), state: "run".into(), hold_ticks: None },
                script::Command::ReleaseControllerState { controller: "locomotion".into() },
            ]
        );
        // Not for a mob nobody published, and not without the permission.
        assert!(host.mob_set_controller_state_hold("locomotion".into(), 99, None, "run".into()).is_err());
        let mut ungranted = GameServerHost { id: "hold_ungranted".into(), commands: false };
        let e = ungranted.mob_release_controller_state("locomotion".into(), 1).unwrap_err();
        assert!(e.contains("service.mobs"), "{e}");
        script::reset();
    }
}

#[cfg(test)]
mod hook_ids {
    /// The 20 script hooks are one block of the catalog: `FullMod::wants` keeps the module adapters of exactly
    /// that block silent (the runtime reaches the module itself).
    #[test]
    fn the_script_hooks_are_the_block_the_module_adapters_skip() {
        use super::super::full::{H_MOB_GOAL_CAN_USE, H_SYNCED_CHANGED_CLIENT};
        let names: Vec<&str> = yoake_wasm::FULL_HOOK_LIST[H_MOB_GOAL_CAN_USE..=H_SYNCED_CHANGED_CLIENT].iter().map(|(n, _)| *n).collect();
        let fragment: serde_json::Value = serde_json::from_str(include_str!("../../../../tools/native-hook-fragments/f4m_mobs.json")).unwrap();
        let expected: Vec<&str> = fragment["hooks"].as_array().unwrap().iter().map(|h| h["name"].as_str().unwrap()).collect();
        assert_eq!(names, expected);
        assert_eq!(names.len(), 20);
    }
}
