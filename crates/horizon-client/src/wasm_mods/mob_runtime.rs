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

/// Installs the runtime (once; the first server module that registers does it).
pub(super) fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        crate::mobs::custom::script::set_runtime(Some(Arc::new(WasmRuntime)));
        crate::mobs::custom::ai::interact::set_hook(Some(Arc::new(WasmRuntime)));
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
