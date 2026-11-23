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

/// Installs what is process-wide (once; the first server module that registers does it): the `mob-interact`
/// hook of the data dispatch. The script runtime is not: a server opens it with its world (see [`world_runtime`]).
pub(crate) fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        crate::mobs::custom::ai::interact::set_hook(Some(Arc::new(WasmRuntime)));
    });
}

/// The script runtime a server opens its session with (`script::open_session`): the WASM server modules first,
/// then the native mods (`mob_native`, through `mod_api::mods()`). `None` when there is neither (the custom
/// mobs then run without scripts and publish no views).
pub(crate) fn world_runtime() -> Option<Arc<dyn crate::mobs::custom::script::Runtime>> {
    if endpoints().is_empty() && crate::mod_api::mods().is_empty() {
        return None;
    }
    Some(Arc::new(crate::mobs::custom::script::Chain {
        first: Arc::new(WasmRuntime),
        second: Arc::new(crate::mob_native::NativeRuntime::process()),
    }))
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
        let answer = any_true(each(|m| m.dispatch("mob_goal_can_use", |e, s| e.yoake_mods_server_hooks().call_mob_goal_can_use(s, mob, goal))));
        // One info line when the gate's inputs for this mob and goal change (for the end-to-end run): the script goal's gate.
        static LAST: std::sync::Mutex<Option<(u32, String, String)>> = std::sync::Mutex::new(None);
        if let Ok(mut last) = LAST.lock() {
            let v = crate::mobs::custom::script::view(mob);
            let state = format!(
                "can_use {answer:?} (on_ground {:?}, target {:?}, visible {:?}, has target_position {:?})",
                v.as_ref().map(|v| v.on_ground),
                v.as_ref().map(|v| (v.target_mob, v.target_player)),
                v.as_ref().map(|v| v.target_visible),
                v.as_ref().map(|v| v.target_position.is_some())
            );
            let key = (mob, goal.to_owned(), state.clone());
            if last.as_ref() != Some(&key) {
                let distance = v.as_ref().and_then(|v| v.target_position.map(|t| ((t[0] - v.position[0]).powi(2) + (t[1] - v.position[1]).powi(2) + (t[2] - v.position[2]).powi(2)).sqrt()));
                log::info!("Horizon custom-mob script goal: mob {mob} {goal} {state} target distance {distance:?}");
                *last = Some(key);
            }
        }
        answer
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
    use yoake_wasm::server_types as st;
    use yoake_wasm::test_guests::{FOX_AI, FOX_AI_SPIN, FOX_AI_TRAP, FOX_TEXTURE, Guest, load};

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

    /// The server's session over the registered modules (what the game opens with its world); the tests hold it.
    fn session() -> script::Session {
        script::open_session(super::world_runtime().expect("a server module is registered")).expect("no other session is open")
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
        let _session = session();
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
        let _session = session();
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
        let _session = session();
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
        let _session = session();
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

    /// F4M-4d: `mob-seat-set` queues its command for a vehicle that has the seat, refuses a seat the type
    /// does not have, a mob that rides itself and an unknown vehicle, and needs `service.mobs`.
    #[test]
    fn the_seat_set_service_queues_its_command_and_is_gated_by_service_mobs() {
        use yoake_wasm::ServerHost;
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        script::reset();
        let m = fox(1, Vec3::new(0.5, 1.0, 0.5));
        let mut view = script::MobView::of(&m).unwrap();
        view.seats = 3;
        script::publish(view);
        mob_services::grant("seat_granted");
        let mut host = GameServerHost { id: "seat_granted".into(), commands: false };
        assert_eq!(host.mob_seat_set(1, Some(7), 2), Ok(true));
        assert_eq!(host.mob_seat_set(1, None, 0), Ok(true));
        assert_eq!(
            script::take_for(1),
            vec![
                script::Command::SeatSet { seat: 2, rider: Some(7) },
                script::Command::SeatSet { seat: 0, rider: None },
            ]
        );
        let e = host.mob_seat_set(1, Some(7), 3).unwrap_err();
        assert!(e.contains("3 seats"), "{e}");
        assert!(host.mob_seat_set(1, Some(1), 0).is_err(), "a mob does not ride itself");
        assert!(host.mob_seat_set(99, Some(7), 0).is_err(), "no such vehicle");
        assert!(script::take_for(1).is_empty(), "nothing was queued by the refusals");
        let mut ungranted = GameServerHost { id: "seat_ungranted".into(), commands: false };
        let e = ungranted.mob_seat_set(1, Some(7), 0).unwrap_err();
        assert!(e.contains("service.mobs"), "{e}");
        script::reset();
    }

    #[test]
    fn the_memory_services_read_write_expire_and_check_types_and_permissions() {
        use yoake_wasm::ServerHost;
        struct NoopRuntime;
        impl script::Runtime for NoopRuntime {
            fn live(&self) -> bool { true }
            fn goal_can_use(&self, _: u32, _: &str) -> Option<bool> { None }
            fn goal_can_continue(&self, _: u32, _: &str) -> Option<bool> { None }
            fn goal_start(&self, _: u32, _: &str) {}
            fn goal_tick(&self, _: u32, _: &str) {}
            fn goal_stop(&self, _: u32, _: &str) {}
            fn mob_tick(&self, _: u32) {}
            fn mob_spawn_check(&self, _: &str, _: &str, _: [i32; 3]) -> Option<bool> { None }
            fn mob_spawned(&self, _: u32, _: &str, _: &str) {}
            fn mob_hurt(&self, _: u32, _: &str, _: &str, _: f32) -> Option<script::HurtReply> { None }
            fn mob_died(&self, _: u32, _: &str, _: Option<u32>) {}
            fn mob_interact(&self, _: u32, _: &str, _: &str, _: &str) -> script::InteractReply { script::InteractReply::Pass }
            fn mob_anim_event(&self, _: u32, _: &str) {}
            fn mob_load(&self, _: u32, _: &[u8]) {}
            fn mob_save(&self, _: u32, _: &[u8]) -> Option<Vec<u8>> { None }
            fn mob_tamed(&self, _: u32, _: &str) {}
            fn mob_bred(&self, _: u32, _: u32, _: u32) {}
            fn synced_changed(&self, _: u32, _: &str, _: &script::MobScriptValue, _: &script::MobScriptValue) {}
        }
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                script::set_runtime(None);
                script::reset();
            }
        }
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let _reset = Reset;
        let json = r#"{
            "template": "yoake:mob",
            "category": "creature",
            "size": {"width": 0.6, "height": 1.0, "eye_height": 0.9},
            "attributes": {"max_health": 20, "movement_speed": 0.25},
            "ai": {
                "memories": {
                    "alert": {"type": "bool", "default": false},
                    "mood": {"type": "string"},
                    "seen": {"type": "entity", "ttl": 5},
                    "counter": {"type": "i32"}
                },
                "goals": []
            }
        }"#;
        let file = CustomMobFile::parse("demo:memfox", json.as_bytes()).unwrap();
        let t = Arc::new(CustomType::new(file, None).unwrap());
        let kind = Arc::new(build::mob_type(&t));
        let mut mob = Mob::new(11, kind, Vec3::new(0.5, 1.0, 0.5), 0.0);
        mob.on_ground = true;

        let world = BenchWorld::new();
        let w = world.world();
        let mut events = Vec::new();

        script::reset();
        script::set_runtime(Some(Arc::new(NoopRuntime)));
        super::mob_services::grant("mem_granted");
        script::publish(script::MobView::of_in(&mob, Some(&w), false).unwrap());
        let mut host = GameServerHost { id: "mem_granted".into(), commands: false };

        let got = host.mob_memory_get(11, "alert".into()).unwrap().unwrap();
        assert_eq!(got.kind, st::MobMemoryKind::Bool);
        assert!(!got.bool_value);
        assert!(host.mob_memory_get(11, "mood".into()).unwrap().is_none());

        let base_val = st::MobMemoryValue {
            kind: st::MobMemoryKind::Bool,
            bool_value: true,
            int_value: 0,
            float_value: 0.0,
            string_value: String::new(),
            vec3_value: st::Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            block_pos_value: st::BlockPos { x: 0, y: 0, z: 0 },
            entity_mob: None,
            entity_player: None,
        };
        host.mob_memory_set(11, "alert".into(), None, Some(base_val.clone())).unwrap();
        host.mob_memory_set(
            11,
            "mood".into(),
            Some(3),
            Some(st::MobMemoryValue {
                kind: st::MobMemoryKind::String,
                string_value: "curious".into(),
                ..base_val.clone()
            }),
        )
        .unwrap();
        host.mob_memory_set(
            11,
            "seen".into(),
            None,
            Some(st::MobMemoryValue {
                kind: st::MobMemoryKind::Entity,
                entity_mob: Some(42),
                ..base_val.clone()
            }),
        )
        .unwrap();

        let type_err = host.mob_memory_set(11, "counter".into(), None, Some(base_val.clone())).unwrap_err();
        assert!(type_err.contains("i32"), "{type_err}");
        let key_err = host.mob_memory_set(11, "unknown".into(), None, Some(base_val.clone())).unwrap_err();
        assert!(key_err.contains("no memory"), "{key_err}");
        let ttl_err = host.mob_memory_set(11, "counter".into(), Some(72_001), None).unwrap_err();
        assert!(ttl_err.contains("72000"), "{ttl_err}");

        let mut ungranted = GameServerHost { id: "mem_ungranted".into(), commands: false };
        let perm_err = ungranted.mob_memory_get(11, "alert".into()).unwrap_err();
        assert!(perm_err.contains("service.mobs"), "{perm_err}");

        // Apply queued commands cleanly via script::pre_tick + Mob::tick
        let mut lr = LegacyRandom::new(7);
        mob.tick(&w, &mut lr, &mut events);

        script::publish(script::MobView::of_in(&mob, Some(&w), false).unwrap());
        assert!(host.mob_memory_get(11, "alert".into()).unwrap().unwrap().bool_value);
        assert_eq!(host.mob_memory_get(11, "mood".into()).unwrap().unwrap().string_value, "curious");
        assert_eq!(host.mob_memory_get(11, "seen".into()).unwrap().unwrap().entity_mob, Some(42));

        // After 3 ticks, "mood" (ttl 3) expires (tick 1 set -> remaining 2 -> 1 -> 0 removed)
        for _ in 0..3 {
            mob.tick(&w, &mut lr, &mut events);
        }
        script::publish(script::MobView::of_in(&mob, Some(&w), false).unwrap());
        assert!(host.mob_memory_get(11, "mood".into()).unwrap().is_none(), "mood TTL expired");
        assert!(host.mob_memory_get(11, "seen".into()).unwrap().is_some(), "seen uses default ttl=5");

        // Explicit erase with None
        host.mob_memory_set(11, "alert".into(), None, None).unwrap();
        mob.tick(&w, &mut lr, &mut events);
        script::publish(script::MobView::of_in(&mob, Some(&w), false).unwrap());
        assert!(host.mob_memory_get(11, "alert".into()).unwrap().is_none());
        script::reset();
    }

    #[test]
    fn the_fox_ai_guest_writes_and_reads_pounced_memory_with_ttl_and_brain_switches_activities() {
        use crate::mobs::custom::ai::memory::{EntityMemory, MemoryValue};
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        script::reset();
        let Some((_dev, _pkg)) = dev_mod_package() else { return };
        let _session = session();
        let fx = BenchWorld::new();
        let mut lr = LegacyRandom::new(1);
        let mut events = Vec::new();
        let mut m = fox(1, Vec3::new(0.5, 1.0, 0.5));
        assert_eq!(m.extra.custom.as_ref().unwrap().brain.active_activity.as_deref(), Some("idle"));

        let mut chicken = fox(2, Vec3::new(4.5, 1.0, 0.5));
        Arc::make_mut(&mut chicken.kind).species = crate::mobs::Species::Chicken;
        Arc::make_mut(&mut chicken.kind).custom = None;
        let prey = chicken.seen(None);
        let others = [prey];
        m.ai.target = Some(Who::Mob(2));

        // Tick 0: nearest_living sensor sees chicken -> prey_nearby -> hunt activity, and fox_ai pounces
        // and writes `pounced = true` with TTL 40 via `mob-memory-set`.
        let mut w = fx.world();
        w.game_time = 1000;
        w.others = &others;
        m.tick(&w, &mut lr, &mut events);
        {
            let brain = &m.extra.custom.as_ref().unwrap().brain;
            assert_eq!(
                brain.memories.get("prey_nearby"),
                Some(&MemoryValue::Entity(EntityMemory::Mob(2)))
            );
            assert_eq!(brain.active_activity.as_deref(), Some("hunt"));
            assert_eq!(brain.memories.get("pounced"), Some(&MemoryValue::Bool(true)));
            assert_eq!(brain.memories.entry("pounced").unwrap().ttl, Some(40));
            assert!(pounce_running(&m));
        }

        // Let pounce finish; while `pounced` is still present, `fox_ai`'s `mob-memory-get` prevents re-pouncing.
        for t in 1001..1025 {
            w.game_time = t;
            m.tick(&w, &mut lr, &mut events);
        }
        assert!(!pounce_running(&m), "pounced memory cooldown blocks immediate re-pounce");
        assert!(m.extra.custom.as_ref().unwrap().brain.memories.get("pounced").is_some());

        // Remove chicken and advance past sensor interval (10) and `pounced` TTL (40).
        w.others = &[];
        for t in 1025..1045 {
            w.game_time = t;
            m.tick(&w, &mut lr, &mut events);
        }
        let brain = &m.extra.custom.as_ref().unwrap().brain;
        assert_eq!(brain.memories.get("prey_nearby"), None);
        assert_eq!(brain.active_activity.as_deref(), Some("idle"));
        assert_eq!(brain.memories.get("pounced"), None, "pounced TTL=40 expired");

        // Now with the target back 4 blocks away, `fox_ai` pounces again once its interval allows.
        let mut chicken2 = fox(2, Vec3::new(m.position.x + 4.0, m.position.y, m.position.z));
        Arc::make_mut(&mut chicken2.kind).species = crate::mobs::Species::Chicken;
        Arc::make_mut(&mut chicken2.kind).custom = None;
        let others2 = [chicken2.seen(None)];
        w.others = &others2;
        m.ai.target = Some(Who::Mob(2));
        for t in 1045..=1050 {
            w.game_time = t;
            m.tick(&w, &mut lr, &mut events);
            if pounce_running(&m) {
                break;
            }
        }
        assert!(pounce_running(&m), "fox_ai pounces again once pounced memory expires");
        script::reset();
    }

    #[test]
    fn the_fox_ai_guest_sets_warmth_when_the_fox_is_hurt() {
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        script::reset();
        let Some((_dev, _pkg)) = dev_mod_package() else { return };
        let _session = session();
        let fx = BenchWorld::new();
        let mut lr = LegacyRandom::new(1);
        let mut events = Vec::new();
        let mut m = fox(1, Vec3::new(0.5, 1.0, 0.5));
        assert!(script::hurt(&mut m, "", "Generic", 4.0).is_some(), "fox_ai answers mob-hurt");
        let mut w = fx.world();
        w.game_time = 1000;
        m.tick(&w, &mut lr, &mut events);
        let t = m.kind.custom.clone().unwrap();
        let v = m.extra.custom.as_ref().unwrap().synced_value(&t, "warmth").cloned();
        assert!(matches!(v, Some(crate::mobs::custom::SyncedValue::I32(x)) if x > 0), "{v:?}");
        script::reset();
    }

    #[test]
    fn the_fox_texture_guest_creates_writes_and_binds_on_warmth_change_and_cleans_up_on_unload() {
        use crate::mobs::custom::{script::MobScriptValue, textures};
        use yoake_wasm::ClientServices;
        let _serial = super::super::GUEST_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        textures::store().lock().unwrap_or_else(|e| e.into_inner()).clear();

        // Ungranted client mod cannot call mob_texture_create.
        let mut ungranted = super::super::GameHost::new("fox_tex_ungranted");
        let err = ClientServices::mob_texture_create(&mut ungranted, 32, 64).unwrap_err();
        assert!(err.contains("service.mobs"), "{err}");

        let Some(bytes) = load(&FOX_TEXTURE) else { return };
        let mod_id = "fox_tex_granted";
        mob_services::grant(mod_id);
        let client = super::super::full::load_client(&ModuleSource {
            id: mod_id,
            bytes: &bytes,
            commands: false,
            gpu: false,
        })
        .expect("fox_texture guest loads");
        let pkg = Arc::new(FullMod::new(mod_id.into(), Some(client), None));

        let rt = script::client_runtime().expect("client runtime installed");
        rt.synced_changed(77, "warmth", &MobScriptValue::Int(0), &MobScriptValue::Int(4));

        let region = [2560, 0, 1024, 1024];
        let atlas_size = [4096, 4096];
        let ov = textures::overrides_for(77, region, atlas_size).expect("default slot bound");
        assert!(ov.get("default").is_some());

        let mut patches = Vec::new();
        textures::flush_patches(region, atlas_size, &mut patches);
        assert_eq!(patches.len(), 1);
        assert_eq!((patches[0].width, patches[0].height), (64, 32));
        // Top-left 32x16 sub-region was overwritten with [255, 220, 40, 255]; rest has [120, 220, 255, 255].
        assert_eq!(&patches[0].rgba[0..4], &[255, 220, 40, 255]);
        let row20_col40 = (20 * 64 + 40) * 4;
        assert_eq!(&patches[0].rgba[row20_col40..row20_col40 + 4], &[120, 220, 255, 255]);

        drop(pkg);
        assert!(textures::overrides_for(77, region, atlas_size).is_none());
        assert!(textures::is_empty());
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
