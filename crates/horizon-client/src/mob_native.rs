//! Native (DLL / trait) mods hooking the script hooks of fully custom mobs (F4M-10).
//!
//! The 20 hooks of `tools/native-hook-fragments/f4m_mobs.json` are exports of a WASM guest, and a mob is ticked
//! as `&mut Mob` with no `Game` in reach, so the engine reaches them through `mobs::custom::script::Runtime`.
//! [`NativeRuntime`] is the runtime of the native side: every call is the generated `mod_api::dispatch_*` over
//! the process-wide [`mod_api::mods`] (or a list given to [`NativeRuntime::over`], for tests), so a native mod
//! is called in `mods()` order with the same panic guard as any other hook.
//!
//! * The WASM modules are in `mods()` too, but their adapters for these hooks stay silent (`FullMod::wants`):
//!   the WASM runtime path (`wasm_mods::mob_runtime`) calls them, and a module is never called twice.
//! * `script::Chain` runs the WASM runtime first and this one second for each hook.
//! * The host a native mod gets is empty (no `Game` is in reach inside a mob tick): it is told, it may answer
//!   (claim `can-use` / `can-continue` / `spawn-check`, change a hurt, reply to an interaction, replace the save
//!   data, append bone writes) but cannot call the engine back from these hooks.
//! * A claim hook cannot say "no": the contract is "any mod may claim", so a native mod that does not claim is
//!   the same as one that did not answer.
use crate::mod_api::{self, Hooks, Host};
use crate::mobs::custom::script::{
    ClientRuntime, HurtReply, InteractReply, MAX_DATA_BYTES, MAX_POSE_WRITES, MobBoneWrite, MobPoseFrame, MobScriptValue, Runtime,
};

/// The empty host of a native script hook.
struct ScriptHost;
impl Host for ScriptHost {}

/// Where the native mods come from.
#[derive(Clone, Copy)]
enum Source {
    /// The process-wide list.
    Process,
    /// A list of its own (tests).
    List(&'static [Box<dyn Hooks>]),
}

/// The server side: the native mods as a script runtime.
#[derive(Clone, Copy)]
pub(crate) struct NativeRuntime(Source);

impl NativeRuntime {
    /// Over `mod_api::mods()`.
    pub(crate) const fn process() -> Self {
        Self(Source::Process)
    }

    /// Over a list of its own.
    #[cfg(test)]
    pub(crate) fn over(list: &'static [Box<dyn Hooks>]) -> Self {
        Self(Source::List(list))
    }

    fn mods(&self) -> &'static [Box<dyn Hooks>] {
        match self.0 {
            Source::Process => mod_api::mods(),
            Source::List(list) => list,
        }
    }
}

impl Runtime for NativeRuntime {
    fn live(&self) -> bool {
        !self.mods().is_empty()
    }
    fn goal_can_use(&self, mob: u32, goal: &str) -> Option<bool> {
        let mut claimed = false;
        mod_api::dispatch_mob_goal_can_use(&mut ScriptHost, self.mods(), mob, goal, &mut claimed);
        claimed.then_some(true)
    }
    fn goal_can_continue(&self, mob: u32, goal: &str) -> Option<bool> {
        let mut claimed = false;
        mod_api::dispatch_mob_goal_can_continue(&mut ScriptHost, self.mods(), mob, goal, &mut claimed);
        claimed.then_some(true)
    }
    fn goal_start(&self, mob: u32, goal: &str) {
        mod_api::dispatch_mob_goal_start(&mut ScriptHost, self.mods(), mob, goal);
    }
    fn goal_tick(&self, mob: u32, goal: &str) {
        mod_api::dispatch_mob_goal_tick(&mut ScriptHost, self.mods(), mob, goal);
    }
    fn goal_stop(&self, mob: u32, goal: &str) {
        mod_api::dispatch_mob_goal_stop(&mut ScriptHost, self.mods(), mob, goal);
    }
    fn mob_tick(&self, mob: u32) {
        mod_api::dispatch_mob_tick(&mut ScriptHost, self.mods(), mob);
    }
    fn mob_spawn_check(&self, type_id: &str, dimension: &str, pos: [i32; 3]) -> Option<bool> {
        let mut allowed = false;
        mod_api::dispatch_mob_spawn_check(&mut ScriptHost, self.mods(), type_id, dimension, pos, &mut allowed);
        allowed.then_some(true)
    }
    fn mob_spawned(&self, mob: u32, type_id: &str, reason: &str) {
        mod_api::dispatch_custom_mob_spawned(&mut ScriptHost, self.mods(), mob, type_id, reason);
    }
    fn mob_hurt(&self, mob: u32, part: &str, source: &str, amount: f32) -> Option<HurtReply> {
        let (mut a, mut cancel) = (amount, false);
        mod_api::dispatch_mob_hurt(&mut ScriptHost, self.mods(), mob, part, source, &mut a, &mut cancel);
        // As for a WASM answer: a non-finite or negative amount is ignored.
        let a = if a.is_finite() && a >= 0.0 { a } else { amount };
        (a != amount || cancel).then_some(HurtReply { amount: a, cancel })
    }
    fn mob_died(&self, mob: u32, type_id: &str, attacker: Option<u32>) {
        mod_api::dispatch_mob_died(&mut ScriptHost, self.mods(), mob, type_id, attacker.unwrap_or(0));
    }
    fn mob_interact(&self, mob: u32, player: &str, hand: &str, item: &str) -> InteractReply {
        let mut reply = String::new();
        mod_api::dispatch_mob_interact(&mut ScriptHost, self.mods(), mob, player, hand, item, &mut reply);
        match reply.as_str() {
            "consume" => InteractReply::Consume,
            "success" => InteractReply::Success,
            _ => InteractReply::Pass,
        }
    }
    fn mob_anim_event(&self, mob: u32, name: &str) {
        mod_api::dispatch_mob_anim_event(&mut ScriptHost, self.mods(), mob, name);
    }
    fn mob_load(&self, mob: u32, data: &[u8]) {
        mod_api::dispatch_mob_load(&mut ScriptHost, self.mods(), mob, &data.to_vec());
    }
    fn mob_save(&self, mob: u32, data: &[u8]) -> Option<Vec<u8>> {
        let mut out = data.to_vec();
        mod_api::dispatch_mob_save(&mut ScriptHost, self.mods(), mob, &mut out);
        (out != data && out.len() <= MAX_DATA_BYTES).then_some(out)
    }
    fn mob_tamed(&self, mob: u32, owner: &str) {
        mod_api::dispatch_mob_tamed(&mut ScriptHost, self.mods(), mob, owner);
    }
    fn mob_bred(&self, mob: u32, partner: u32, child: u32) {
        mod_api::dispatch_mob_bred(&mut ScriptHost, self.mods(), mob, partner, child);
    }
    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue) {
        mod_api::dispatch_synced_changed_server(&mut ScriptHost, self.mods(), mob, key, old, new);
    }
}

/// The client side: the native mods as a client script runtime.
#[derive(Clone, Copy)]
pub(crate) struct NativeClientRuntime(Source);

impl NativeClientRuntime {
    pub(crate) const fn process() -> Self {
        Self(Source::Process)
    }

    #[cfg(test)]
    pub(crate) fn over(list: &'static [Box<dyn Hooks>]) -> Self {
        Self(Source::List(list))
    }

    fn mods(&self) -> &'static [Box<dyn Hooks>] {
        match self.0 {
            Source::Process => mod_api::mods(),
            Source::List(list) => list,
        }
    }
}

impl ClientRuntime for NativeClientRuntime {
    fn live(&self) -> bool {
        !self.mods().is_empty()
    }
    fn mob_pose(&self, mob: u32, kind: &str, frame: &MobPoseFrame) -> Vec<MobBoneWrite> {
        let mut writes = Vec::new();
        mod_api::dispatch_mob_pose(&mut ScriptHost, self.mods(), mob, kind, frame, &mut writes);
        writes.truncate(MAX_POSE_WRITES);
        writes
    }
    fn mob_anim_event(&self, mob: u32, name: &str) {
        mod_api::dispatch_mob_anim_event_client(&mut ScriptHost, self.mods(), mob, name);
    }
    fn synced_changed(&self, mob: u32, key: &str, old: &MobScriptValue, new: &MobScriptValue) {
        mod_api::dispatch_synced_changed_client(&mut ScriptHost, self.mods(), mob, key, old, new);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mod_api::*;
    use crate::mobs::custom::script::{Chain, ClientChain, ScriptVec3};
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<String>>>;

    /// A native mod that records every script hook it is given and answers some of them.
    struct Spy {
        name: &'static str,
        log: Log,
    }

    impl Spy {
        fn say(&self, line: String) {
            self.log.lock().unwrap().push(format!("{}:{line}", self.name));
        }
    }

    impl Hooks for Spy {
        fn mob_goal_can_use(&self, _host: &mut dyn Host, e: &mut MobGoalCanUseEvent<'_>) -> Flow {
            self.say(format!("can_use {} {}", e.mob, e.goal));
            if e.goal == "yes" { Flow::Cancel } else { Flow::Continue }
        }
        fn mob_goal_can_continue(&self, _host: &mut dyn Host, e: &mut MobGoalCanContinueEvent<'_>) -> Flow {
            self.say(format!("can_continue {} {}", e.mob, e.goal));
            if e.goal == "yes" { Flow::Cancel } else { Flow::Continue }
        }
        fn mob_goal_start(&self, _host: &mut dyn Host, e: &mut MobGoalStartEvent<'_>) {
            self.say(format!("start {} {}", e.mob, e.goal));
        }
        fn mob_goal_tick(&self, _host: &mut dyn Host, e: &mut MobGoalTickEvent<'_>) {
            self.say(format!("tick {} {}", e.mob, e.goal));
        }
        fn mob_goal_stop(&self, _host: &mut dyn Host, e: &mut MobGoalStopEvent<'_>) {
            self.say(format!("stop {} {}", e.mob, e.goal));
        }
        fn mob_tick(&self, _host: &mut dyn Host, e: &mut MobTickEvent) {
            self.say(format!("mob_tick {}", e.mob));
        }
        fn mob_spawn_check(&self, _host: &mut dyn Host, e: &mut MobSpawnCheckEvent<'_>) -> Flow {
            self.say(format!("spawn_check {} {} {:?}", e.kind, e.dimension, e.pos));
            if e.kind == "t:ok" { Flow::Cancel } else { Flow::Continue }
        }
        fn custom_mob_spawned(&self, _host: &mut dyn Host, e: &mut CustomMobSpawnedEvent<'_>) {
            self.say(format!("spawned {} {} {}", e.mob, e.kind, e.reason));
        }
        fn mob_hurt(&self, _host: &mut dyn Host, e: &mut MobHurtEvent<'_>) {
            self.say(format!("hurt {} {} {} {}", e.mob, e.part, e.source, e.amount));
            e.amount *= 2.0;
            e.cancel |= e.part == "head";
        }
        fn mob_died(&self, _host: &mut dyn Host, e: &mut MobDiedEvent<'_>) {
            self.say(format!("died {} {} {}", e.mob, e.kind, e.attacker));
        }
        fn mob_interact(&self, _host: &mut dyn Host, e: &mut MobInteractEvent<'_>) {
            self.say(format!("interact {} {} {} {}", e.mob, e.player, e.hand, e.item));
            if e.item == "t:bone" {
                *e.reply = "consume".into();
            }
        }
        fn mob_anim_event(&self, _host: &mut dyn Host, e: &mut MobAnimEventEvent<'_>) {
            self.say(format!("anim {} {}", e.mob, e.name));
        }
        fn mob_load(&self, _host: &mut dyn Host, e: &mut MobLoadEvent<'_>) {
            self.say(format!("load {} {:?}", e.mob, e.data));
        }
        fn mob_save(&self, _host: &mut dyn Host, e: &mut MobSaveEvent<'_>) {
            self.say(format!("save {} {:?}", e.mob, e.data));
            e.data.push(7);
        }
        fn mob_tamed(&self, _host: &mut dyn Host, e: &mut MobTamedEvent<'_>) {
            self.say(format!("tamed {} {}", e.mob, e.owner));
        }
        fn mob_bred(&self, _host: &mut dyn Host, e: &mut MobBredEvent) {
            self.say(format!("bred {} {} {}", e.mob, e.partner, e.child));
        }
        fn synced_changed_server(&self, _host: &mut dyn Host, e: &mut SyncedChangedServerEvent<'_>) {
            self.say(format!("synced {} {} {:?} {:?}", e.mob, e.key, e.old, e.new));
        }
        fn mob_pose(&self, _host: &mut dyn Host, e: &mut MobPoseEvent<'_>) {
            self.say(format!("pose {} {} {}", e.mob, e.kind, e.frame.time));
            e.writes.push(MobBoneWrite { bone: 3, additive: true, ..Default::default() });
        }
        fn mob_anim_event_client(&self, _host: &mut dyn Host, e: &mut MobAnimEventClientEvent<'_>) {
            self.say(format!("anim_client {} {}", e.mob, e.name));
        }
        fn synced_changed_client(&self, _host: &mut dyn Host, e: &mut SyncedChangedClientEvent<'_>) {
            self.say(format!("synced_client {} {}", e.mob, e.key));
        }
    }

    fn list(names: &[&'static str], log: &Log) -> &'static [Box<dyn Hooks>] {
        let mods: Vec<Box<dyn Hooks>> =
            names.iter().map(|name| Box::new(Spy { name, log: log.clone() }) as Box<dyn Hooks>).collect();
        Box::leak(mods.into_boxed_slice())
    }

    fn lines(log: &Log) -> Vec<String> {
        std::mem::take(&mut *log.lock().unwrap())
    }

    #[test]
    fn a_native_mod_is_told_every_server_script_hook_and_may_answer() {
        let log = Log::default();
        let rt = NativeRuntime::over(list(&["a"], &log));
        assert!(rt.live());
        assert!(!NativeRuntime::over(&[]).live(), "no native mods: not live");
        // Claims: yes is Some(true), anything else is no answer.
        assert_eq!(rt.goal_can_use(1, "yes"), Some(true));
        assert_eq!(rt.goal_can_use(1, "no"), None);
        assert_eq!(rt.goal_can_continue(2, "yes"), Some(true));
        assert_eq!(rt.goal_can_continue(2, "no"), None);
        assert_eq!(rt.mob_spawn_check("t:ok", "minecraft:overworld", [1, 2, 3]), Some(true));
        assert_eq!(rt.mob_spawn_check("t:no", "minecraft:overworld", [1, 2, 3]), None);
        rt.goal_start(3, "g");
        rt.goal_tick(3, "g");
        rt.goal_stop(3, "g");
        rt.mob_tick(4);
        rt.mob_spawned(5, "t:fox", "natural");
        rt.mob_died(6, "t:fox", Some(9));
        rt.mob_died(6, "t:fox", None);
        rt.mob_anim_event(7, "hit");
        rt.mob_load(8, &[1, 2]);
        rt.mob_tamed(9, "uuid");
        rt.mob_bred(10, 11, 12);
        rt.synced_changed(13, "warmth", &MobScriptValue::Int(1), &MobScriptValue::Int(2));
        assert_eq!(
            lines(&log),
            [
                "a:can_use 1 yes", "a:can_use 1 no", "a:can_continue 2 yes", "a:can_continue 2 no",
                "a:spawn_check t:ok minecraft:overworld [1, 2, 3]", "a:spawn_check t:no minecraft:overworld [1, 2, 3]",
                "a:start 3 g", "a:tick 3 g", "a:stop 3 g", "a:mob_tick 4", "a:spawned 5 t:fox natural",
                "a:died 6 t:fox 9", "a:died 6 t:fox 0", "a:anim 7 hit", "a:load 8 [1, 2]", "a:tamed 9 uuid",
                "a:bred 10 11 12", "a:synced 13 warmth Int(1) Int(2)",
            ]
        );
        // Answers: a changed hurt, a reply to an interaction, replaced save data.
        assert_eq!(rt.mob_hurt(1, "body", "PlayerAttack", 3.0), Some(HurtReply { amount: 6.0, cancel: false }));
        assert_eq!(rt.mob_hurt(1, "head", "PlayerAttack", 0.0), Some(HurtReply { amount: 0.0, cancel: true }));
        assert_eq!(rt.mob_interact(1, "u", "main", "t:bone"), InteractReply::Consume);
        assert_eq!(rt.mob_interact(1, "u", "main", "t:stick"), InteractReply::Pass);
        assert_eq!(rt.mob_save(1, &[5]), Some(vec![5, 7]));
        lines(&log);
    }

    #[test]
    fn several_native_mods_are_called_in_mods_order_and_a_hurt_goes_through_them_all() {
        let log = Log::default();
        let rt = NativeRuntime::over(list(&["first", "second"], &log));
        rt.mob_tick(1);
        assert_eq!(lines(&log), ["first:mob_tick 1", "second:mob_tick 1"]);
        // Each doubles the amount: 3 -> 6 -> 12.
        assert_eq!(rt.mob_hurt(1, "body", "x", 3.0), Some(HurtReply { amount: 12.0, cancel: false }));
        assert_eq!(rt.mob_save(1, &[]), Some(vec![7, 7]));
        // A claim of the first stops the later mods (the hook contract), so the second never sees it.
        lines(&log);
        assert_eq!(rt.goal_can_use(1, "yes"), Some(true));
        let seen = lines(&log);
        assert_eq!(seen.iter().filter(|l| l.contains("can_use")).count(), 1, "{seen:?}");
    }

    #[test]
    fn the_client_hooks_reach_native_mods_and_pose_writes_come_back() {
        let log = Log::default();
        let rt = NativeClientRuntime::over(list(&["a", "b"], &log));
        assert!(rt.live());
        let frame = MobPoseFrame { time: 1.5, ..Default::default() };
        let writes = rt.mob_pose(4, "t:fox", &frame);
        assert_eq!(writes.len(), 2, "one write from each native mod");
        assert!(writes.iter().all(|w| w.bone == 3 && w.additive));
        rt.mob_anim_event(4, "step");
        rt.synced_changed(4, "warmth", &MobScriptValue::Int(0), &MobScriptValue::Int(1));
        assert_eq!(
            lines(&log),
            ["a:pose 4 t:fox 1.5", "b:pose 4 t:fox 1.5", "a:anim_client 4 step", "b:anim_client 4 step", "a:synced_client 4 warmth", "b:synced_client 4 warmth"]
        );
    }

    /// A runtime that records what it is asked, as the WASM side.
    struct Wasm {
        log: Log,
        claim: Option<bool>,
        interact: InteractReply,
    }

    impl Runtime for Wasm {
        fn live(&self) -> bool {
            true
        }
        fn goal_can_use(&self, mob: u32, goal: &str) -> Option<bool> {
            self.log.lock().unwrap().push(format!("wasm:can_use {mob} {goal}"));
            self.claim
        }
        fn goal_can_continue(&self, _: u32, _: &str) -> Option<bool> {
            self.claim
        }
        fn goal_start(&self, mob: u32, goal: &str) {
            self.log.lock().unwrap().push(format!("wasm:start {mob} {goal}"));
        }
        fn goal_tick(&self, _: u32, _: &str) {}
        fn goal_stop(&self, _: u32, _: &str) {}
        fn mob_tick(&self, _: u32) {}
        fn mob_spawn_check(&self, _: &str, _: &str, _: [i32; 3]) -> Option<bool> {
            self.claim
        }
        fn mob_spawned(&self, _: u32, _: &str, _: &str) {}
        fn mob_hurt(&self, _: u32, _: &str, _: &str, amount: f32) -> Option<HurtReply> {
            Some(HurtReply { amount: amount + 1.0, cancel: false })
        }
        fn mob_died(&self, _: u32, _: &str, _: Option<u32>) {}
        fn mob_interact(&self, _: u32, _: &str, _: &str, _: &str) -> InteractReply {
            self.interact
        }
        fn mob_anim_event(&self, _: u32, _: &str) {}
        fn mob_load(&self, _: u32, _: &[u8]) {}
        fn mob_save(&self, _: u32, data: &[u8]) -> Option<Vec<u8>> {
            let mut d = data.to_vec();
            d.push(1);
            Some(d)
        }
        fn mob_tamed(&self, _: u32, _: &str) {}
        fn mob_bred(&self, _: u32, _: u32, _: u32) {}
        fn synced_changed(&self, _: u32, _: &str, _: &MobScriptValue, _: &MobScriptValue) {}
    }

    fn chain(log: &Log, claim: Option<bool>, interact: InteractReply) -> Chain {
        Chain {
            first: Arc::new(Wasm { log: log.clone(), claim, interact }),
            second: Arc::new(NativeRuntime::over(list(&["native"], log))),
        }
    }

    #[test]
    fn the_wasm_runtime_is_asked_first_and_the_native_mods_second() {
        let log = Log::default();
        let c = chain(&log, None, InteractReply::Pass);
        assert!(c.live());
        c.goal_start(1, "g");
        assert_eq!(lines(&log), ["wasm:start 1 g", "native:start 1 g"]);
        // A yes of either side is a yes; a no of the WASM side with no native claim stays no.
        assert_eq!(c.goal_can_use(1, "yes"), Some(true), "native claim, wasm no answer");
        assert_eq!(chain(&log, Some(false), InteractReply::Pass).goal_can_use(1, "no"), Some(false));
        assert_eq!(chain(&log, Some(false), InteractReply::Pass).goal_can_use(1, "yes"), Some(true));
        assert_eq!(chain(&log, Some(true), InteractReply::Pass).goal_can_use(1, "no"), Some(true));
        assert_eq!(chain(&log, None, InteractReply::Pass).goal_can_use(1, "no"), None);
        lines(&log);
        // A hurt passes through the WASM side (3 -> 4) and then the native mod (4 -> 8).
        assert_eq!(c.mob_hurt(1, "body", "x", 3.0), Some(HurtReply { amount: 8.0, cancel: false }));
        assert_eq!(c.mob_hurt(1, "head", "x", 3.0), Some(HurtReply { amount: 8.0, cancel: true }));
        // The first interaction that is not a pass wins.
        assert_eq!(c.mob_interact(1, "u", "main", "t:bone"), InteractReply::Consume, "native");
        assert_eq!(chain(&log, None, InteractReply::Success).mob_interact(1, "u", "main", "t:bone"), InteractReply::Success);
        // The save data goes through both.
        assert_eq!(c.mob_save(1, &[0]), Some(vec![0, 1, 7]));
        lines(&log);
    }

    #[test]
    fn the_client_chain_joins_the_bone_writes_of_both_sides() {
        struct Pose;
        impl ClientRuntime for Pose {
            fn live(&self) -> bool {
                true
            }
            fn mob_pose(&self, _: u32, _: &str, _: &MobPoseFrame) -> Vec<MobBoneWrite> {
                vec![MobBoneWrite { bone: 1, translation: Some(ScriptVec3 { x: 1.0, y: 0.0, z: 0.0 }), ..Default::default() }]
            }
            fn mob_anim_event(&self, _: u32, _: &str) {}
            fn synced_changed(&self, _: u32, _: &str, _: &MobScriptValue, _: &MobScriptValue) {}
        }
        let log = Log::default();
        let c = ClientChain { first: Arc::new(Pose), second: Arc::new(NativeClientRuntime::over(list(&["n"], &log))) };
        let writes = c.mob_pose(1, "t:fox", &MobPoseFrame::default());
        assert_eq!(writes.iter().map(|w| w.bone).collect::<Vec<_>>(), [1, 3], "WASM first, native second");
    }
}
