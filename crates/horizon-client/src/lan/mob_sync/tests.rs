//! Tests of the server half of `horizon:mob_sync`: what is collected, merged and rate limited, the
//! snapshot, frames and who gets what.
use super::*;
use crate::mobs::{
    Mob,
    custom::{
        SyncedValue,
        ai::test_support::{flat, view},
        anim_sync::{
            request_trigger,
            tests::{fox_mob, fox_type_with},
        },
    },
};
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use horizon_protocol::mob_sync::{self as wire, Value};
use serde_json::json;

fn idle_ai() -> serde_json::Value {
    json!({"navigator": "ground", "goals": []})
}

struct World {
    fx: crate::mobs::mob_hook_bench::BenchWorld,
    lr: LegacyRandom,
    events: Vec<crate::mobs::MobEvent>,
}
impl World {
    fn new() -> Self {
        Self {
            fx: flat(),
            lr: LegacyRandom::new(1),
            events: Vec::new(),
        }
    }
    fn tick(&mut self, m: &mut Mob, now: i64) {
        let others = [];
        let mut w = view(&self.fx.world(), &others, None);
        w.game_time = now;
        m.tick(&w, &mut self.lr, &mut self.events);
    }
}

fn fox(edit: impl FnOnce(&mut serde_json::Value)) -> Mob {
    fox_mob(&fox_type_with(idle_ai(), edit), Vec3::new(0.5, 1.0, 0.5))
}

fn set(m: &mut Mob, name: &str, v: SyncedValue) {
    let t = m.kind.custom.clone().unwrap();
    assert!(m.extra.custom.as_mut().unwrap().set_synced(&t, name, v));
}

fn value_of(d: &Delta) -> Vec<(usize, Value)> {
    d.values
        .as_ref()
        .map_or(Vec::new(), |v| v.iter().map(|(s, v)| (s, v.clone())).collect())
}

#[test]
fn the_first_collection_announces_every_layer_and_then_a_still_mob_costs_nothing() {
    let mut w = World::new();
    let mut m = fox(|_| {});
    w.tick(&mut m, 100);
    let first = collect(&mut m, 100).expect("anchors on the first look");
    let delta = first.delta.expect("a delta");
    assert_eq!(delta.anchors.len(), 3);
    assert!(delta.values.is_none() && delta.hitbox_mask.is_none());
    assert_eq!(first.entity, 1);
    let mut bytes = 0;
    for now in 101..400 {
        w.tick(&mut m, now);
        if let Some(c) = collect(&mut m, now) {
            bytes += frames(now, c.messages()).iter().map(Vec::len).sum::<usize>();
        }
    }
    assert_eq!(bytes, 0, "an idle mob sends nothing after its first delta");
}

/// Several changes of a value inside one interval go out once, as the latest value.
#[test]
fn changes_merge_into_one_delta_with_the_latest_value() {
    let mut w = World::new();
    let mut m = fox(|_| {});
    w.tick(&mut m, 100);
    collect(&mut m, 100);
    set(&mut m, "warmth", SyncedValue::I32(3));
    set(&mut m, "state", SyncedValue::Enum(2));
    set(&mut m, "warmth", SyncedValue::I32(5));
    let c = collect(&mut m, 110).unwrap();
    let d = c.delta.unwrap();
    // The fox values in slot order: state (0), warmth (1).
    assert_eq!(value_of(&d), [(0, Value::Enum(2)), (1, Value::I32(5))]);
    assert_eq!(d.values.as_ref().unwrap().mask, 0b11);
    assert!(collect(&mut m, 111).is_none(), "nothing changed since");
    // Changed and changed back before it was sent: what is sent is the value as it is.
    set(&mut m, "warmth", SyncedValue::I32(9));
    set(&mut m, "warmth", SyncedValue::I32(5));
    let c = collect(&mut m, 120).map(|c| c.delta.unwrap());
    assert!(c.is_none_or(|d| value_of(&d) == [(1, Value::I32(5))]));
}

#[test]
fn a_value_is_sent_at_most_as_often_as_its_sync_rate() {
    let count = |rate: Option<u32>| {
        let mut w = World::new();
        let mut m = fox(|v| {
            if let Some(r) = rate {
                v["synced"]["warmth"]["sync_rate"] = json!(r);
            }
        });
        w.tick(&mut m, 0);
        collect(&mut m, 0);
        let mut sent = Vec::new();
        for now in 1..=200 {
            w.tick(&mut m, now);
            // The value changes every single tick.
            set(&mut m, "warmth", SyncedValue::I32((now % 9) as i32 + 1));
            if let Some(c) = collect(&mut m, now)
                && let Some(d) = c.delta
                && d.values.is_some()
            {
                sent.push(now);
            }
        }
        sent
    };
    let default = count(None);
    assert_eq!(default.len(), 100, "10 a second by default: one in two ticks, {default:?}");
    assert!(default.windows(2).all(|p| p[1] - p[0] >= 2));
    assert_eq!(count(Some(20)).len(), 200, "20 a second is every tick");
    let slow = count(Some(2));
    assert!(slow.windows(2).all(|p| p[1] - p[0] >= 10), "{slow:?}");
    assert!(slow.len() <= 20);
    let one = count(Some(1));
    assert!(one.windows(2).all(|p| p[1] - p[0] >= 20) && one.len() <= 10);
    assert_eq!(interval(10), 2);
    assert_eq!(interval(1), 20);
    assert_eq!(interval(20), 1);
    assert_eq!(interval(0), 20, "clamped");
    assert_eq!(interval(99), 1, "clamped");
}

/// The mob's deltas are no closer than the fastest rate among what they carry, whatever mix of
/// values, anchors and the hitbox mask changes.
#[test]
fn a_mob_sends_at_most_ten_deltas_a_second_by_default() {
    let mut w = World::new();
    let mut m = fox(|_| {});
    w.tick(&mut m, 0);
    collect(&mut m, 0);
    let t = m.kind.custom.clone().unwrap();
    let mut deltas = Vec::new();
    for now in 1..=100 {
        m.position.x += if now % 7 < 3 { 0.12 } else { 0.0 };
        w.tick(&mut m, now);
        set(&mut m, "warmth", SyncedValue::I32((now % 9) as i32 + 1));
        m.extra
            .custom
            .as_mut()
            .unwrap()
            .set_hitbox_enabled(&t, (now % 2) as usize, now % 3 == 0);
        if let Some(c) = collect(&mut m, now)
            && c.delta.is_some()
        {
            deltas.push(now);
        }
    }
    assert!(deltas.len() <= 50, "{} deltas in 5 seconds", deltas.len());
    assert!(deltas.windows(2).all(|p| p[1] - p[0] >= 2), "{deltas:?}");
    assert!(deltas.len() >= 40, "but it does keep up: {}", deltas.len());
}

#[test]
fn triggers_go_at_once_and_unmerged() {
    let mut w = World::new();
    let mut m = fox(|_| {});
    w.tick(&mut m, 100);
    collect(&mut m, 100);
    // Something else sent a delta a tick ago: the trigger does not wait.
    set(&mut m, "warmth", SyncedValue::I32(2));
    assert!(collect(&mut m, 102).unwrap().delta.is_some());
    assert!(request_trigger(&mut m, "pounce", 1.0));
    set(&mut m, "warmth", SyncedValue::I32(4));
    w.tick(&mut m, 103);
    let c = collect(&mut m, 103).expect("the trigger");
    assert_eq!(c.triggers.len(), 1);
    assert!(c.delta.is_none(), "the value waits for its turn");
    let ta = m.kind.custom.as_ref().unwrap().anim.as_ref().unwrap();
    assert_eq!(c.triggers[0].trigger.id, ta.trigger_id("pounce").unwrap());
    assert_eq!(c.triggers[0].trigger.start_tick, 102);
    assert!(matches!(c.messages().next(), Some(Message::Trigger(_))));
    // Two triggers in a tick are two messages.
    request_trigger(&mut m, "pounce", 2.0);
    w.tick(&mut m, 104);
    let c = collect(&mut m, 104).unwrap();
    assert_eq!(c.triggers.len(), 1);
    assert_eq!(c.triggers[0].trigger.speed, 32);
}

#[test]
fn values_nobody_is_told_are_not_sent_or_kept_pending() {
    let mut w = World::new();
    let mut m = fox(|v| {
        v["synced"]["warmth"]["sync"] = json!("none");
    });
    w.tick(&mut m, 1);
    collect(&mut m, 1);
    set(&mut m, "warmth", SyncedValue::I32(4));
    assert!(collect(&mut m, 50).is_none());
    assert_eq!(m.extra.custom.as_ref().unwrap().synced_dirty, 0);
    let t = m.kind.custom.clone().unwrap();
    assert_eq!(announced_mask(&t), 0b01, "only `state`");
    let s = snapshot(&m).unwrap();
    assert_eq!(s.values.mask, 0b01);
    assert_eq!(s.values.values, [Value::Enum(0)]);
    // `owner` is not told to everyone either (the owner is F4M-4c).
    let owner = fox(|v| {
        v["synced"]["warmth"]["sync"] = json!("owner");
    });
    assert_eq!(announced_mask(owner.kind.custom.as_ref().unwrap()), 0b01);
}

#[test]
fn a_snapshot_is_the_whole_state() {
    let mut w = World::new();
    let mut m = fox(|_| {});
    w.tick(&mut m, 100);
    set(&mut m, "state", SyncedValue::Enum(1));
    set(&mut m, "warmth", SyncedValue::I32(7));
    request_trigger(&mut m, "pounce", 1.0);
    w.tick(&mut m, 101);
    let t = m.kind.custom.clone().unwrap();
    let s = snapshot(&m).unwrap();
    assert_eq!(s.entity, 1);
    assert_eq!(s.type_version, t.sync_version);
    assert_eq!(s.values.mask, 0b11);
    assert_eq!(s.values.values, [Value::Enum(1), Value::I32(7)]);
    assert_eq!(s.anchors.len(), 3);
    assert!(s.anchors.iter().all(|a| a.tick == 101));
    assert_eq!(s.triggers.len(), 1);
    assert_eq!(s.hitbox_mask, 0b11, "the fox has two hitbox parts, both on");
    // It does not use up what is pending for the deltas.
    assert_eq!(m.extra.custom.as_ref().unwrap().synced_dirty, 0b11);
    assert!(collect(&mut m, 102).unwrap().delta.is_some());
    // It round trips through the wire.
    let frame = frames(101, [Message::Snapshot(s.clone())]);
    assert_eq!(frame.len(), 1);
    let back = wire::decode(&frame[0]).unwrap();
    assert_eq!(back.game_time, 101);
    assert_eq!(back.messages, vec![Message::Snapshot(s)]);
    // A mob that is not a custom one has none.
    let cow = Mob::new(2, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
    assert!(snapshot(&cow).is_none());
}

#[test]
fn hitbox_changes_are_sent() {
    let mut w = World::new();
    let mut m = fox(|_| {});
    w.tick(&mut m, 1);
    collect(&mut m, 1);
    let t = m.kind.custom.clone().unwrap();
    let state = m.extra.custom.as_mut().unwrap();
    assert!(!state.set_hitbox_enabled(&t, 5, false), "no such part");
    assert!(state.set_hitbox_enabled(&t, 1, false));
    assert!(state.set_hitbox_enabled(&t, 1, false), "again: not a change");
    let d = collect(&mut m, 10).unwrap().delta.unwrap();
    assert_eq!(d.hitbox_mask, Some(0b01));
    assert!(collect(&mut m, 12).is_none());
}

#[test]
fn frames_split_at_the_byte_limit_and_keep_their_order() {
    let msgs: Vec<Message> = (0..2000)
        .map(|i| {
            Message::Trigger(TriggerStart {
                entity: i,
                trigger: wire::Trigger {
                    id: 1,
                    start_tick: i64::from(i),
                    speed: 16,
                },
            })
        })
        .collect();
    let out = frames(77, msgs.clone());
    assert!(out.len() > 1);
    assert!(
        out.iter().all(|f| f.len() <= MAX_FRAME + 64),
        "{:?}",
        out.iter().map(Vec::len).collect::<Vec<_>>()
    );
    let mut back = Vec::new();
    for f in &out {
        let d = wire::decode(f).unwrap();
        assert_eq!(d.game_time, 77);
        back.extend(d.messages);
    }
    assert_eq!(back, msgs);
    assert!(frames(1, []).is_empty(), "no messages, no frame");
}

#[test]
fn a_player_gets_snapshots_for_new_mobs_and_deltas_for_those_in_range_only() {
    let delta = |entity, mask| Collected {
        entity,
        triggers: vec![],
        delta: Some(Delta {
            entity,
            hitbox_mask: Some(mask),
            ..Default::default()
        }),
        owner_delta: None,
        owner: None,
        seats: None,
        players: vec![],
    };
    let snap = |e: i32| {
        Some(Snapshot {
            entity: e,
            type_version: 0,
            values: Default::default(),
            anchors: vec![],
            triggers: vec![],
            hitbox_mask: 0,
        })
    };
    let (a, b, c) = (delta(1, 1), delta(2, 2), delta(3, 3));
    let tracking = |e: i32| e == 1 || e == 2;
    let out = for_viewer(tracking, &[2], &[a.clone(), b, c], snap);
    let kinds: Vec<(i32, &str)> = out
        .iter()
        .map(|m| {
            (
                m.entity(),
                match m {
                    Message::Snapshot(_) => "snapshot",
                    Message::Delta(_) => "delta",
                    Message::Trigger(_) => "trigger",
                    Message::Seats(_) => "seats",
                },
            )
        })
        .collect();
    assert_eq!(
        kinds,
        [(2, "snapshot"), (1, "delta")],
        "3 is out of range; 2 is new: its snapshot is all it gets"
    );
    // A start of a mob that is no longer tracked (it left within the tick) is nothing.
    assert!(for_viewer(|_| false, &[1, 2], &[a], snap).is_empty());
    assert!(for_viewer(|_| true, &[], &[], snap).is_empty());
}

#[test]
fn a_vanilla_mob_has_nothing_to_send() {
    let mut cow = Mob::new(2, crate::mobs::sensor_scan_tests::kind(), Vec3::ZERO, 0.0);
    assert!(collect(&mut cow, 5).is_none());
    assert!(snapshot(&cow).is_none());
    assert!(frames(5, []).is_empty());
}

#[test]
fn values_convert_to_the_wire_and_back_cut_to_their_type() {
    let t = fox_type_with(idle_ai(), |_| {});
    let warmth = &t.synced[1];
    assert_eq!(from_wire(warmth, &Value::I32(99)), Some(SyncedValue::I32(10)), "cut to the range");
    assert_eq!(from_wire(warmth, &Value::Bool(true)), None, "another kind");
    assert_eq!(from_wire(&t.synced[0], &Value::Enum(9)), None, "no such value");
    assert_eq!(from_wire(&t.synced[0], &Value::Enum(2)), Some(SyncedValue::Enum(2)));
    let f = crate::mobs::custom::SyncedDef {
        kind: "f32".into(),
        ..t.synced[1].clone()
    };
    assert_eq!(from_wire(&f, &Value::F32(f32::NAN)), None);
    for v in [
        SyncedValue::Bool(true),
        SyncedValue::I32(-4),
        SyncedValue::F32(0.5),
        SyncedValue::Str("hé".into()),
        SyncedValue::Vec3([1.0, 2.0, 3.0]),
        SyncedValue::Enum(1),
    ] {
        let back = match to_wire(&v) {
            Value::Bool(b) => SyncedValue::Bool(b),
            Value::I32(n) => SyncedValue::I32(n),
            Value::F32(f) => SyncedValue::F32(f),
            Value::Str(s) => SyncedValue::Str(s),
            Value::Vec3(v) => SyncedValue::Vec3(v),
            Value::Enum(i) => SyncedValue::Enum(i),
        };
        assert_eq!(back, v);
    }
}

// ---- `sync: owner` (F4M-4c) -----------------------------------------------------------------------

/// The slots of the values a viewer's messages carry (deltas and the snapshot).
fn slots_sent(msgs: &[Message]) -> Vec<usize> {
    let mut out = Vec::new();
    for m in msgs {
        let set = match m {
            Message::Delta(d) => d.values.as_ref(),
            Message::Snapshot(s) => Some(&s.values),
            _ => None,
        };
        if let Some(set) = set {
            out.extend(set.iter().map(|(s, _)| s));
        }
    }
    out.sort_unstable();
    out
}

fn snap_of(m: &Mob) -> impl Fn(i32, Option<[i32; 4]>) -> Option<wire::Snapshot> + '_ {
    move |e, viewer| (e == 1).then(|| snapshot_for(m, viewer).unwrap())
}

#[test]
fn an_owner_value_is_sent_to_the_owner_only_and_to_nobody_before_there_is_one() {
    use crate::mobs::custom::ai::tame;
    const OWNER: [i32; 4] = [7, 8, 9, 10];
    const OTHER: [i32; 4] = [1, 2, 3, 4];
    let mut w = World::new();
    // The values in slot order: mood (owner only), state, warmth.
    let mut m = fox(|v| {
        v["synced"]["mood"] = json!({"type": "i32", "min": 0, "max": 100, "default": 5, "sync": "owner"});
        v["tameable"]["sits_on_command"] = json!(false);
    });
    w.tick(&mut m, 100);
    collect(&mut m, 100);
    // Nobody owns it: the owner value goes to nobody, the public one to everybody.
    set(&mut m, "mood", SyncedValue::I32(40));
    set(&mut m, "warmth", SyncedValue::I32(3));
    let c = collect(&mut m, 110).expect("the public value");
    assert!(c.owner_delta.is_none() && c.owner.is_none());
    assert_eq!(value_of(c.delta.as_ref().unwrap()), [(2, Value::I32(3))]);
    assert!(collect(&mut m, 120).is_none(), "and the owner value waits quietly");
    // Taming marks the owner's values: the owner is told what it holds.
    tame::tame(&mut m, OWNER);
    let c = collect(&mut m, 130).expect("the owner value");
    assert_eq!(c.owner, Some(OWNER));
    assert_eq!(value_of(c.owner_delta.as_ref().unwrap()), [(0, Value::I32(40))]);
    assert!(c.delta.is_none(), "the public delta has none of it");
    let all = [c];
    let owner_msgs = for_viewer_as(Some(OWNER), |_| true, &[], &all, snap_of(&m));
    let other_msgs = for_viewer_as(Some(OTHER), |_| true, &[], &all, snap_of(&m));
    let nobody_msgs = for_viewer(|_| true, &[], &all, |e| snap_of(&m)(e, None));
    assert_eq!(slots_sent(&owner_msgs), [0]);
    assert!(other_msgs.is_empty() && nobody_msgs.is_empty(), "no one else is told");
    // The distance gate holds for the owner too.
    assert!(for_viewer_as(Some(OWNER), |_| false, &[], &all, snap_of(&m)).is_empty());
    // A new snapshot: the owner's has the value (with the public ones), the others' only the public.
    let starts = [1];
    let for_owner = for_viewer_as(Some(OWNER), |_| true, &starts, &[], snap_of(&m));
    let for_other = for_viewer_as(Some(OTHER), |_| true, &starts, &[], snap_of(&m));
    let for_none = for_viewer(|_| true, &starts, &[], |e| snap_of(&m)(e, None));
    assert_eq!(slots_sent(&for_owner), [0, 1, 2]);
    assert_eq!(slots_sent(&for_other), [1, 2]);
    assert_eq!(slots_sent(&for_none), [1, 2]);
    // Later changes go straight to the owner.
    set(&mut m, "mood", SyncedValue::I32(50));
    let c = collect(&mut m, 140).unwrap();
    assert_eq!(value_of(c.owner_delta.as_ref().unwrap()), [(0, Value::I32(50))]);
    // A new owner is told everything again, the old one is not told any more.
    tame::tame(&mut m, OTHER);
    let c = collect(&mut m, 150).unwrap();
    assert_eq!(c.owner, Some(OTHER));
    assert!(for_viewer_as(Some(OWNER), |_| true, &[], &[c.clone()], snap_of(&m)).is_empty());
    assert_eq!(slots_sent(&for_viewer_as(Some(OTHER), |_| true, &[], &[c], snap_of(&m))), [0]);
}

#[test]
fn the_hosts_own_view_hides_owner_values_of_mobs_it_does_not_own() {
    use crate::mobs::custom::ai::tame;
    const HOST: [i32; 4] = [7, 8, 9, 10];
    const OTHER: [i32; 4] = [1, 2, 3, 4];
    let mut w = World::new();
    let mut m = fox(|v| {
        v["synced"]["mood"] = json!({"type": "i32", "min": 0, "max": 100, "default": 5, "sync": "owner"});
        v["tameable"]["sits_on_command"] = json!(false);
    });
    w.tick(&mut m, 100);
    set(&mut m, "mood", SyncedValue::I32(40));
    set(&mut m, "warmth", SyncedValue::I32(3));
    // No owner: the owner value is hidden from the host, the public one is not.
    assert_eq!(host_value(&m, Some(HOST), "mood"), None);
    assert_eq!(host_value(&m, Some(HOST), "warmth"), Some(&SyncedValue::I32(3)));
    // Owned by another player: still hidden.
    tame::tame(&mut m, OTHER);
    assert_eq!(host_value(&m, Some(HOST), "mood"), None);
    assert_eq!(host_value(&m, Some(OTHER), "mood"), Some(&SyncedValue::I32(40)));
    // Owned by the host's player: visible.
    tame::tame(&mut m, HOST);
    assert_eq!(host_value(&m, Some(HOST), "mood"), Some(&SyncedValue::I32(40)));
    assert_eq!(host_value(&m, None, "mood"), None);
    assert_eq!(host_value(&m, Some(HOST), "nope"), None);
    // The host's view is exactly the slots a remote client of the same uuid is sent.
    let slots = m.kind.custom.as_ref().unwrap().synced.len();
    for viewer in [HOST, OTHER] {
        let sent = slots_sent(&for_viewer_as(Some(viewer), |_| true, &[1], &[], snap_of(&m)));
        for slot in 0..slots {
            assert_eq!(value_visible_to(&m, slot, Some(viewer)), sent.contains(&slot), "{viewer:?} slot {slot}");
        }
    }
}

/// F4M-4d: who sits where on a custom vehicle is sent once when it changes, at once, to everyone who tracks
/// it, and to a player that starts tracking it after the snapshot.
#[test]
fn the_seats_of_a_vehicle_are_sent_when_they_change_and_to_a_new_viewer() {
    use crate::mobs::custom::SeatClaim;
    let mut w = World::new();
    let mut m = fox(|_| {});
    m.id = 5;
    w.tick(&mut m, 0);
    let _ = collect(&mut m, 0);
    assert!(collect(&mut m, 1).is_none(), "a vehicle nobody sits on says nothing");
    assert!(seats_message(&m).is_none());
    // Two riders: one message at once (no rate limit), carrying the seats in seat order.
    m.extra
        .custom
        .as_mut()
        .unwrap()
        .set_seat_claims(vec![SeatClaim { rider: 9, seat: 1 }, SeatClaim { rider: 8, seat: 0 }]);
    let c = collect(&mut m, 2).expect("the seats changed");
    let sent: Vec<Message> = c.messages().collect();
    let expected = wire::Seats {
        entity: 5,
        seats: vec![wire::SeatEntry { rider: 8, seat: 0 }, wire::SeatEntry { rider: 9, seat: 1 }],
    };
    assert_eq!(sent, [Message::Seats(expected.clone())]);
    // Said once: the same list set again changes nothing; a different one is said again.
    m.extra.custom.as_mut().unwrap().set_seat_claims(vec![SeatClaim { rider: 8, seat: 0 }, SeatClaim { rider: 9, seat: 1 }]);
    assert!(collect(&mut m, 3).is_none());
    m.extra.custom.as_mut().unwrap().set_seat_claims(vec![]);
    let c = collect(&mut m, 4).unwrap();
    assert_eq!(c.messages().collect::<Vec<_>>(), [Message::Seats(wire::Seats { entity: 5, seats: vec![] })]);
    // A player that starts tracking gets the current seats (its snapshot has no room for them).
    m.extra.custom.as_mut().unwrap().set_seat_claims(vec![SeatClaim { rider: 8, seat: 0 }, SeatClaim { rider: 9, seat: 1 }]);
    assert_eq!(seats_message(&m), Some(Message::Seats(expected)));
    // Frames carry it and a client decodes it.
    let frames = frames(7, seats_message(&m));
    assert_eq!(frames.len(), 1);
    let back = wire::decode(&frames[0]).unwrap();
    assert!(matches!(back.messages[..], [Message::Seats(_)]));
}
