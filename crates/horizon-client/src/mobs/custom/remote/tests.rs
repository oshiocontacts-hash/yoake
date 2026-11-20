//! The client store against a real server mob: the Cinder Fox runs on the flat world, what it
//! collects goes through the wire codec, and the store applies it (the integrated path reads the mob
//! directly; this is the same apply code the network path uses).
use super::*;
use crate::lan::mob_sync::{collect, frames, snapshot};
use crate::mobs::{
    Mob,
    custom::{
        ai::test_support::{flat, view},
        anim_sync::{
            request_trigger,
            tests::{fox_mob, fox_type_with},
        },
    },
};
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use horizon_mobmodel::events::{EventBuf, MobEventKind};
use horizon_protocol::mob_sync::Message;
use serde_json::json;

const ENTITY: i32 = 1;

fn idle_ai() -> serde_json::Value {
    json!({"navigator": "ground", "goals": []})
}

struct Server {
    fx: crate::mobs::mob_hook_bench::BenchWorld,
    lr: LegacyRandom,
    events: Vec<crate::mobs::MobEvent>,
    mob: Mob,
    kind: Arc<CustomType>,
}
impl Server {
    fn new() -> Self {
        let kind = fox_type_with(idle_ai(), |_| {});
        Self {
            fx: flat(),
            lr: LegacyRandom::new(1),
            events: Vec::new(),
            mob: fox_mob(&kind, Vec3::new(0.5, 1.0, 0.5)),
            kind,
        }
    }
    fn tick(&mut self, now: i64) {
        let others = [];
        let mut w = view(&self.fx.world(), &others, None);
        w.game_time = now;
        self.mob.tick(&w, &mut self.lr, &mut self.events);
    }
    fn set(&mut self, name: &str, v: SyncedValue) {
        let t = self.kind.clone();
        assert!(self.mob.extra.custom.as_mut().unwrap().set_synced(&t, name, v));
    }
    /// The payloads of this tick's collection.
    fn payloads(&mut self, now: i64) -> Vec<Vec<u8>> {
        match collect(&mut self.mob, now) {
            Some(c) => frames(now, c.messages()),
            None => Vec::new(),
        }
    }
    fn snapshot_payload(&self, now: i64) -> Vec<u8> {
        frames(now, [Message::Snapshot(snapshot(&self.mob).unwrap())]).remove(0)
    }
    fn anim(&mut self) -> &mut crate::mobs::custom::anim_sync::MobAnim {
        self.mob.extra.custom.as_mut().unwrap().anim.as_mut().unwrap()
    }
}

fn kinds(server: &Server) -> impl Fn(i32) -> Option<Arc<CustomType>> + use<> {
    let k = server.kind.clone();
    move |e| (e == ENTITY).then(|| k.clone())
}

#[test]
fn a_late_join_gets_the_whole_state_from_the_snapshot() {
    let mut s = Server::new();
    s.tick(100);
    s.set("state", SyncedValue::Enum(1));
    s.set("warmth", SyncedValue::I32(7));
    assert!(request_trigger(&mut s.mob, "pounce", 1.0));
    s.tick(101);
    let mut c = RemoteCustomMobs::default();
    let done = c.apply_payload(&s.snapshot_payload(101), &kinds(&s)).unwrap();
    assert_eq!(done, Applied { snapshots: 1, ..Default::default() });
    let mob = c.get(ENTITY).unwrap();
    assert_eq!(mob.value("state"), Some(&SyncedValue::Enum(1)));
    assert_eq!(mob.value("warmth"), Some(&SyncedValue::I32(7)));
    assert_eq!(mob.hitbox_mask, 0b11);
    // The clocks are the server's, at the snapshot's tick and after it.
    let server_anchors = s.anim().anchors().to_vec();
    for a in &server_anchors {
        for (slot, (time, _)) in a.slots.iter().enumerate() {
            let t = mob.time_at(usize::from(a.layer), slot, 101, 0.0).unwrap();
            assert!((t - f64::from(*time)).abs() < 1e-6, "layer {} slot {slot}", a.layer);
        }
    }
    assert_eq!(mob.triggers.len(), 1);
    assert_eq!(mob.triggers[0].trigger.start_tick, 100);
    // Both changed values were announced.
    let changes = c.take_changes();
    let names: Vec<_> = changes.iter().map(|c| (c.name.as_str(), c.entity)).collect();
    assert_eq!(names, [("state", 1), ("warmth", 1)]);
    assert_eq!(changes[1].old, SyncedValue::I32(0));
    assert_eq!(changes[1].new, SyncedValue::I32(7));
    assert!(c.take_changes().is_empty());
}

#[test]
fn deltas_keep_the_client_with_the_server() {
    let mut s = Server::new();
    s.tick(0);
    let mut c = RemoteCustomMobs::default();
    c.apply_payload(&s.snapshot_payload(0), &kinds(&s)).unwrap();
    c.take_changes();
    let mut worst = 0.0f64;
    let mut walking = false;
    for now in 1..=240 {
        // Walk for a while, stand, now and then change the values.
        if (now / 40) % 2 == 1 {
            s.mob.position.x += 0.12;
            walking = true;
        }
        s.tick(now);
        if now % 30 == 0 {
            s.set("warmth", SyncedValue::I32((now / 30) as i32));
        }
        if now == 100 {
            s.set("state", SyncedValue::Enum(2));
        }
        if now == 150 {
            request_trigger(&mut s.mob, "pounce", 1.0);
        }
        for p in s.payloads(now) {
            c.apply_payload(&p, &kinds(&s)).unwrap();
        }
        // Where the client's clocks are against the server's.
        let mob = c.get(ENTITY).unwrap();
        for a in s.anim().anchors().to_vec() {
            for (slot, (time, _)) in a.slots.iter().enumerate() {
                if let Some(t) = mob.time_at(usize::from(a.layer), slot, now, 0.0) {
                    worst = worst.max((t - f64::from(*time)).abs());
                }
            }
        }
    }
    assert!(walking);
    // The tolerance of the anchors is 20 ms (a layer re-anchors at a state change exactly).
    assert!(worst <= anim_tolerance() + 0.011, "the client was {worst} s from the server's clock at the worst");
    let mob = c.get(ENTITY).unwrap();
    let server_state = s.mob.extra.custom.as_ref().unwrap();
    assert_eq!(mob.values, server_state.synced);
    assert!(mob.triggers.iter().any(|t| t.trigger.start_tick == 150));
    // The notifications: warmth 8 times and state once.
    let n = c.take_changes();
    assert_eq!(n.iter().filter(|c| c.name == "warmth").count(), 8);
    assert_eq!(n.iter().filter(|c| c.name == "state").count(), 1);
}

fn anim_tolerance() -> f64 {
    crate::mobs::custom::anim_sync::ANCHOR_TOLERANCE
}

/// A delta applied twice, late, or after a newer one changes nothing: last writer by frame time.
#[test]
fn reordered_and_repeated_deltas_converge_on_the_newest() {
    let mut s = Server::new();
    s.tick(0);
    let mut c = RemoteCustomMobs::default();
    c.apply_payload(&s.snapshot_payload(0), &kinds(&s)).unwrap();
    // Three values of warmth at 10, 20 and 30, collected as the server would.
    let mut payloads = Vec::new();
    for (now, v) in [(10, 1), (20, 2), (30, 3)] {
        s.tick(now);
        s.set("warmth", SyncedValue::I32(v));
        let mut p = s.payloads(now);
        payloads.append(&mut p);
    }
    assert_eq!(payloads.len(), 3);
    c.take_changes();
    for i in [2, 0, 1, 2, 0] {
        c.apply_payload(&payloads[i], &kinds(&s)).unwrap();
    }
    assert_eq!(c.get(ENTITY).unwrap().value("warmth"), Some(&SyncedValue::I32(3)));
    let n = c.take_changes();
    assert_eq!(n.len(), 1, "only the newest was a change: {n:?}");
    // The same in the order they were sent.
    let mut d = RemoteCustomMobs::default();
    d.apply_payload(&s.snapshot_payload(0), &kinds(&s)).unwrap();
    for p in &payloads {
        d.apply_payload(p, &kinds(&s)).unwrap();
    }
    assert_eq!(d.get(ENTITY).unwrap().value("warmth"), Some(&SyncedValue::I32(3)));
    assert_eq!(d.take_changes().len(), 4, "the snapshot's own (defaults are unchanged) is none; three deltas");
}

#[test]
fn a_lost_delta_is_repaired_by_the_next_one_or_a_snapshot() {
    let mut s = Server::new();
    s.tick(0);
    let mut c = RemoteCustomMobs::default();
    c.apply_payload(&s.snapshot_payload(0), &kinds(&s)).unwrap();
    s.tick(10);
    s.set("warmth", SyncedValue::I32(4));
    s.set("state", SyncedValue::Enum(1));
    let lost = s.payloads(10);
    assert!(!lost.is_empty());
    // Not applied. The next change of the slot arrives and fixes that slot only.
    s.tick(20);
    s.set("warmth", SyncedValue::I32(6));
    for p in s.payloads(20) {
        c.apply_payload(&p, &kinds(&s)).unwrap();
    }
    let mob = c.get(ENTITY).unwrap();
    assert_eq!(mob.value("warmth"), Some(&SyncedValue::I32(6)));
    assert_eq!(mob.value("state"), Some(&SyncedValue::Enum(0)), "still the stale one");
    // A snapshot (a resync, or the player coming back into range) converges everything.
    c.apply_payload(&s.snapshot_payload(21), &kinds(&s)).unwrap();
    assert_eq!(c.get(ENTITY).unwrap().value("state"), Some(&SyncedValue::Enum(1)));
    // And an old snapshot arriving late does not roll anything back.
    s.tick(30);
    s.set("warmth", SyncedValue::I32(9));
    for p in s.payloads(30) {
        c.apply_payload(&p, &kinds(&s)).unwrap();
    }
    c.apply_payload(&s.snapshot_payload(21), &kinds(&s)).unwrap_or_default();
    assert_eq!(c.get(ENTITY).unwrap().value("warmth"), Some(&SyncedValue::I32(9)));
}

#[test]
fn a_stale_anchor_does_not_replace_a_newer_one() {
    let mut s = Server::new();
    s.tick(0);
    let first = s.payloads(0);
    s.tick(1);
    let mut c = RemoteCustomMobs::default();
    c.apply_payload(&s.snapshot_payload(1), &kinds(&s)).unwrap();
    let t = c.get(ENTITY).unwrap().layers[0].as_ref().unwrap().clocks[0].anchor_tick;
    assert_eq!(t, 1);
    // The older first-look delta (tick 0) arrives after the snapshot of tick 1.
    for p in &first {
        c.apply_payload(p, &kinds(&s)).unwrap();
    }
    assert_eq!(c.get(ENTITY).unwrap().layers[0].as_ref().unwrap().clocks[0].anchor_tick, 1);
}

#[test]
fn what_cannot_be_applied_is_dropped_and_counted() {
    let mut s = Server::new();
    s.tick(5);
    let snap = s.snapshot_payload(5);
    s.set("warmth", SyncedValue::I32(3));
    let delta = s.payloads(5);
    let mut c = RemoteCustomMobs::default();
    // A delta before any snapshot, and a snapshot of an entity the client does not know.
    let done = c.apply_payload(&delta[0], &kinds(&s)).unwrap();
    assert_eq!(done.dropped, 1);
    let done = c.apply_payload(&snap, &|_| None).unwrap();
    assert_eq!(done.dropped, 1);
    assert!(c.is_empty());
    // A type whose layout differs from the server's.
    let other = fox_type_with(idle_ai(), |v| v["synced"]["warmth"]["max"] = json!(20));
    let done = c.apply_payload(&snap, &move |_| Some(other.clone())).unwrap();
    assert_eq!(done.incompatible, 1);
    assert!(c.get(ENTITY).unwrap().incompatible);
    let done = c.apply_payload(&delta[0], &kinds(&s)).unwrap();
    assert_eq!(done.incompatible, 1, "nothing of a mob whose type differs is read");
    assert_eq!(c.get(ENTITY).unwrap().value("warmth"), Some(&SyncedValue::I32(0)));
    // Garbage is an error and changes nothing; removal and clear forget.
    assert!(c.apply_payload(&[9, 9, 9], &kinds(&s)).is_err());
    c.remove(ENTITY);
    assert!(c.is_empty());
    c.apply_payload(&snap, &kinds(&s)).unwrap();
    c.clear();
    assert!(c.is_empty() && c.take_changes().is_empty());
}

/// Whether polling the mob at `now` gives the `hit` custom event.
fn hit_now(c: &mut RemoteCustomMobs, now: i64, buf: &mut EventBuf) -> bool {
    buf.clear();
    c.get_mut(ENTITY).unwrap().poll_events(now, 0.0, buf);
    let ta = c.get(ENTITY).unwrap().kind.anim.as_ref().unwrap();
    buf.as_slice()
        .iter()
        .any(|e| matches!(e.kind, MobEventKind::Custom { name, .. } if ta.events.string(name) == "hit"))
}

/// A server that has a pounce start in tick 2 (its virtual start is tick 1), and the payloads of
/// every tick up to 30.
fn pounce_run() -> (Server, Vec<(i64, Vec<Vec<u8>>)>, Vec<(i64, Vec<u8>)>) {
    let mut s = Server::new();
    s.tick(0);
    s.tick(1);
    let (mut sent, mut snaps) = (Vec::new(), Vec::new());
    snaps.push((1, s.snapshot_payload(1)));
    request_trigger(&mut s.mob, "pounce", 1.0);
    for now in 2..=30 {
        s.tick(now);
        sent.push((now, s.payloads(now)));
        snaps.push((now, s.snapshot_payload(now)));
    }
    (s, sent, snaps)
}

/// The server's keyframe and the client's event are in the same tick: the one the blow lands in.
#[test]
fn the_clients_hit_event_comes_with_the_servers_frame() {
    let (s, sent, snaps) = pounce_run();
    let k = kinds(&s);
    let mut c = RemoteCustomMobs::default();
    // It heard of the mob before the pounce (a snapshot of tick 1).
    c.apply_payload(&snaps[0].1, &k).unwrap();
    let mut buf = EventBuf::with_capacity(32);
    let mut hits = Vec::new();
    for (now, payloads) in &sent {
        for p in payloads {
            c.apply_payload(p, &k).unwrap();
        }
        if hit_now(&mut c, *now, &mut buf) {
            hits.push(*now);
        }
    }
    // Started in tick 2 (virtual start 1): the keyframe at 0.35 s is in the 8th window, tick 9, which
    // is the tick of the server's `hit` (see `anim_sync_tests`: 8 ticks after the virtual start).
    assert_eq!(hits, [9], "once, in the tick of the server's frame");
    assert_eq!(c.get(ENTITY).unwrap().triggers[0].trigger.start_tick, 1);
}

/// A joined client never replays what happened before it joined and still gets what is to come.
#[test]
fn a_late_joiner_gets_only_the_events_after_it_joined() {
    let (s, _sent, snaps) = pounce_run();
    let k = kinds(&s);
    let mut buf = EventBuf::with_capacity(32);
    // The keyframe is in tick 9 (its window [0.35, 0.40)): join at 5 or 7 (before) and the event is still to
    // come; join at 8, exactly at 0.35 s, and it is already behind (a joined cursor is exclusive of the
    // moment it joined); at 9 and later it was in the past.
    for (join, expected) in [(5i64, vec![9i64]), (7, vec![9]), (8, vec![]), (9, vec![]), (12, vec![])] {
        let mut c = RemoteCustomMobs::default();
        let snap = snaps.iter().find(|(t, _)| *t == join).unwrap();
        c.apply_payload(&snap.1, &k).unwrap();
        let mut hits = Vec::new();
        for now in join..=30 {
            if hit_now(&mut c, now, &mut buf) {
                hits.push(now);
            }
        }
        assert_eq!(hits, expected, "joined at tick {join}");
    }
}

/// A trigger heard of within a quarter second of its start is fresh (its first events fire), one that
/// comes in a snapshot is joined.
#[test]
fn a_trigger_message_heard_in_time_is_fresh() {
    let (s, sent, snaps) = pounce_run();
    let k = kinds(&s);
    let mut buf = EventBuf::with_capacity(32);
    let mut c = RemoteCustomMobs::default();
    c.apply_payload(&snaps[0].1, &k).unwrap();
    // The trigger message of tick 2 arrives two ticks late, at tick 4.
    let trigger_payload = sent[0].1.iter().find(|p| {
        let f = wire::decode(p).unwrap();
        f.messages.iter().any(|m| matches!(m, Message::Trigger(_)))
    });
    let trigger_payload = trigger_payload.expect("the trigger message of tick 2");
    assert!(!hit_now(&mut c, 3, &mut buf));
    let frame = wire::decode(trigger_payload).unwrap();
    assert_eq!(frame.game_time, 2);
    c.apply_at(&frame, 4, &k);
    assert!(!c.get(ENTITY).unwrap().triggers.is_empty());
    let mut hits = Vec::new();
    for now in 4..=30 {
        if hit_now(&mut c, now, &mut buf) {
            hits.push(now);
        }
    }
    assert_eq!(hits, [9]);
}

/// The fox with a time-0 `roar` event added to its `pounce` animation (the fixture has none: its only
/// event is the `hit` keyframe at 0.35 s).
fn roaring_fox() -> Arc<CustomType> {
    use crate::mobs::custom::anim_sync::tests::root;
    use horizon_mods::custom_mob::CustomMobFile;
    let text = include_str!(
        "../../../../../../tools/fixtures/cinder_fox/data/sample_mobs/horizon/entity_type/cinder_fox.json"
    );
    let mut v: serde_json::Value = serde_json::from_str(text).unwrap();
    v["ai"] = idle_ai();
    let file = CustomMobFile::parse("sample_mobs:cinder_fox", v.to_string().as_bytes()).unwrap();
    let model = file
        .load_model(&mut |p| {
            let bytes = std::fs::read(root().join(p)).map_err(|e| e.to_string())?;
            if !p.ends_with("animations/cinder_fox.animation.json") {
                return Ok(bytes);
            }
            let mut a: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            a["animations"]["animation.cinder_fox.pounce"]["timeline"]["0.0"] = json!(["roar"]);
            Ok(a.to_string().into_bytes())
        })
        .unwrap();
    Arc::new(CustomType::new(file, Some(&model)).unwrap())
}

/// F4M-5b reviewer decision: a trigger heard of 0.1 s after its time-0 event (inside the 0.25 s fresh
/// window), then a re-anchor (a repeated trigger message and a new layer anchor), fires the time-0
/// event exactly once: the cursors' high-water marks leave no way to fire it twice.
#[test]
fn a_late_trigger_then_a_re_anchor_fires_its_time_zero_event_once() {
    assert!((FRESH_TRIGGER_SECONDS - 0.25).abs() < 1e-9, "the documented window");
    let kind = roaring_fox();
    let mut s = Server::new();
    s.kind = kind.clone();
    s.mob = fox_mob(&kind, Vec3::new(0.5, 1.0, 0.5));
    s.tick(0);
    s.tick(1);
    let k = kinds(&s);
    let mut c = RemoteCustomMobs::default();
    c.apply_payload(&s.snapshot_payload(1), &k).unwrap();
    request_trigger(&mut s.mob, "pounce", 1.0);
    let mut trigger_frame = None;
    for now in 2..=30 {
        s.tick(now);
        for p in s.payloads(now) {
            let f = wire::decode(&p).unwrap();
            if f.messages.iter().any(|m| matches!(m, Message::Trigger(_))) {
                trigger_frame = Some(f);
            }
        }
    }
    let trigger_frame = trigger_frame.expect("the trigger message");
    assert_eq!(trigger_frame.game_time, 2, "started in tick 2, virtual start 1");

    let ta = kind.clone();
    let mut buf = EventBuf::with_capacity(32);
    let mut roars = Vec::new();
    let mut poll = |c: &mut RemoteCustomMobs, now: i64, roars: &mut Vec<i64>| {
        buf.clear();
        c.get_mut(ENTITY).unwrap().poll_events(now, 0.0, &mut buf);
        roars.extend(buf.as_slice().iter().filter_map(|e| match e.kind {
            MobEventKind::Custom { name, .. } if ta.anim.as_ref().unwrap().events.string(name) == "roar" => Some(now),
            _ => None,
        }));
    };
    // Its start is tick 1, so tick 3 is 0.1 s after the time-0 event.
    poll(&mut c, 2, &mut roars);
    c.apply_at(&trigger_frame, 3, &k);
    poll(&mut c, 3, &mut roars);
    assert_eq!(roars, [3], "the time-0 event fires when the late trigger is heard");
    // The re-anchor: the same trigger message repeated, and a new anchor of the layer (same state,
    // restarted at tick 4 with the clock at 0 as a server that restarted the state would send).
    c.apply_at(&trigger_frame, 4, &k);
    let state = c.get(ENTITY).unwrap().layers[0].as_ref().unwrap().state;
    let mut f = wire::Frame::new(4);
    f.messages.push(Message::Delta(wire::Delta {
        entity: ENTITY,
        values: None,
        anchors: vec![wire::LayerAnchor { layer: 0, state, tick: 4, slots: vec![(0.0, 1.0)] }],
        hitbox_mask: None,
    }));
    c.apply_at(&f, 4, &k);
    for now in 4..=30 {
        poll(&mut c, now, &mut roars);
    }
    assert_eq!(roars, [3], "never again after the re-anchor");
    assert_eq!(c.get(ENTITY).unwrap().triggers.len(), 1, "one trigger run, not two");
}
