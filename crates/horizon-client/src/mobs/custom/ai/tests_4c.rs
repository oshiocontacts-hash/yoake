//! F4M-4c: taming, the owner goals, breeding, seats and the `mob-interact` plumbing, on the Cinder
//! Fox fixture (the real loaders over its model) and the flat resource-free world.
use super::{
    breed::{self, BABY_AGE, Fed},
    interact::{self, Action, Context, Hand, Hook, Outcome, Reply},
    riding::{self, Controls, RiderKind},
    tame,
    test_support::*,
};
use crate::mobs::{
    Mob, MobEvent, Species, Who,
    custom::{CustomType, Notice, SyncedValue, anim_sync::tests::{fox_mob, fox_type_with}},
    equine, riders,
};
use horizon_core::{geometry::Vec3, random::LegacyRandom};
use serde_json::{Value, json};
use std::sync::Arc;

const OWNER: [i32; 4] = [7, 8, 9, 10];
const STRANGER: [i32; 4] = [1, 1, 1, 1];

fn at(x: f64, z: f64) -> Vec3 {
    Vec3::new(x, 1.0, z)
}

/// The fox with `ai` goals, its own `tameable` (sweet berries, `chance`), and `edit` over the file.
fn ty(goals: Value, edit: impl FnOnce(&mut Value)) -> Arc<CustomType> {
    fox_type_with(json!({"navigator": "ground", "goals": goals}), edit)
}

fn plain(t: &Arc<CustomType>, x: f64, z: f64) -> Mob {
    fox_mob(t, at(x, z))
}

/// A player view at `pos` with the given uuid.
fn player_view<'a>(
    fx: &'a crate::mobs::mob_hook_bench::BenchWorld,
    others: &'a [crate::mobs::Seen],
    pos: Vec3,
    uuid: [i32; 4],
) -> crate::mobs::MobWorld<'a> {
    let mut w = view(&fx.world(), others, Some(pos));
    w.player.as_mut().unwrap().uuid = uuid;
    w
}

fn eq(p: &str, i: &str) -> bool {
    p == i
}

fn click<'a>(item: Option<&'a str>, player: [i32; 4]) -> Context<'a> {
    Context { player, hand: Hand::Main, item, sneaking: false, matches: &eq, ride_seat: None }
}

const BERRIES: &str = "minecraft:sweet_berries";

// ---- taming ------------------------------------------------------------------------------------

/// The chance is the mob's own random: the same seed gives the same answer every time, and over many
/// seeds the share of successes is the type's chance.
#[test]
fn the_taming_chance_is_deterministic_per_seed_and_matches_its_rate() {
    let t = ty(json!([]), |v| v["tameable"]["chance"] = json!(0.33));
    let tries = |seed_from: i64, n: i64| -> Vec<bool> {
        (seed_from..seed_from + n)
            .map(|n| n.wrapping_mul(0x9E37_79B9_7F4A_7C15_u64 as i64))
            .map(|seed| {
                let mut m = plain(&t, 0.5, 0.5);
                m.random = LegacyRandom::new(seed);
                tame::try_tame(&mut m, OWNER)
            })
            .collect()
    };
    let first = tries(1, 600);
    assert_eq!(first, tries(1, 600), "same seeds, same results");
    let tamed = first.iter().filter(|b| **b).count();
    assert!((160..=236).contains(&tamed), "{tamed} of 600 at 0.33");
    // The bounds of the schema: 1 always tames, 0 never does.
    let always = ty(json!([]), |v| v["tameable"]["chance"] = json!(1.0));
    let never = ty(json!([]), |v| v["tameable"]["chance"] = json!(0.0));
    for seed in 0..50 {
        let mut a = plain(&always, 0.5, 0.5);
        a.random = LegacyRandom::new(seed);
        assert!(tame::try_tame(&mut a, OWNER));
        let mut n = plain(&never, 0.5, 0.5);
        n.random = LegacyRandom::new(seed);
        assert!(!tame::try_tame(&mut n, OWNER));
        assert!(!tame::is_tamed(&n));
    }
    // A tamed mob is not tamed again, and a type without `tameable` cannot be.
    let mut m = plain(&always, 0.5, 0.5);
    assert!(tame::try_tame(&mut m, OWNER));
    assert!(!tame::try_tame(&mut m, STRANGER));
    assert_eq!(m.extra.owner, Some(OWNER));
    let wild = ty(json!([]), |v| {
        v.as_object_mut().unwrap().remove("tameable");
    });
    let mut w = plain(&wild, 0.5, 0.5);
    assert!(!tame::try_tame(&mut w, OWNER));
}

/// A right click with the tame item: the item is used up whatever the roll says, a success sets the
/// owner, keeps the mob, sits it (the fox `sits_on_command`), and queues the `mob-tamed` notice.
#[test]
fn berries_tame_a_fox_and_the_notice_is_queued() {
    let t = ty(json!([]), |v| v["tameable"]["chance"] = json!(1.0));
    let mut m = plain(&t, 0.5, 0.5);
    m.persistent = false;
    let o = interact::dispatch(&mut m, &click(Some(BERRIES), OWNER), None);
    assert_eq!(o.reply, Reply::Consume);
    assert_eq!(o.actions, [Action::Event(7)], "hearts");
    assert_eq!(m.extra.owner, Some(OWNER));
    assert!(m.extra.tame && m.persistent && m.extra.sitting, "tame, kept, sitting");
    let state = m.extra.custom.as_mut().unwrap();
    assert_eq!(state.take_notices(), [Notice::Tamed { owner: OWNER }]);
    assert!(state.take_notices().is_empty(), "taken once");
    // The failed roll still uses the berry, shows smoke and tames nothing.
    let never = ty(json!([]), |v| v["tameable"]["chance"] = json!(0.0));
    let mut n = plain(&never, 0.5, 0.5);
    let o = interact::dispatch(&mut n, &click(Some(BERRIES), OWNER), None);
    assert_eq!((o.reply, o.actions), (Reply::Consume, vec![Action::Event(6)]));
    assert!(!n.extra.tame && n.extra.owner.is_none());
    // Another item, or no item, does nothing to a wild fox.
    assert_eq!(interact::dispatch(&mut n, &click(Some("minecraft:stick"), OWNER), None), Outcome::pass());
    assert_eq!(interact::dispatch(&mut n, &click(None, OWNER), None), Outcome::pass());
}

/// The owner's click makes it sit and stand (the pose value follows); another player's does not.
#[test]
fn the_owner_orders_a_tamed_fox_to_sit_and_stand() {
    let t = ty(json!([{"type": "sit", "priority": 1}]), |v| v["tameable"]["chance"] = json!(1.0));
    let mut m = plain(&t, 0.5, 0.5);
    tame::tame(&mut m, OWNER);
    assert!(m.extra.sitting, "a freshly tamed fox sits");
    let state = |m: &Mob| m.extra.custom.as_ref().unwrap().synced_value(&t, "state").cloned().unwrap();
    assert_eq!(state(&m), SyncedValue::Enum(1), "the `sit` value");
    let stranger = interact::dispatch(&mut m, &click(None, STRANGER), None);
    assert_eq!(stranger, Outcome::pass());
    assert!(m.extra.sitting, "a stranger's click changes nothing");
    let o = interact::dispatch(&mut m, &click(None, OWNER), None);
    assert_eq!(o.reply, Reply::Success);
    assert!(!m.extra.sitting);
    assert_eq!(state(&m), SyncedValue::Enum(0), "back to idle");
    interact::dispatch(&mut m, &click(Some("minecraft:stick"), OWNER), None);
    assert!(m.extra.sitting, "any non-food item orders it too");
    // A fox that does not sit on command has no such order.
    let free = ty(json!([]), |v| {
        v["tameable"]["chance"] = json!(1.0);
        v["tameable"]["sits_on_command"] = json!(false);
    });
    let mut f = plain(&free, 0.5, 0.5);
    tame::tame(&mut f, OWNER);
    assert!(!f.extra.sitting);
    assert_eq!(interact::dispatch(&mut f, &click(None, OWNER), None), Outcome::pass());
}

/// While ordered to sit the mob stays put whatever other goals want; standing, it wanders again.
#[test]
fn a_sitting_fox_stays_put_and_a_standing_one_wanders() {
    let goals = json!([{"type": "sit", "priority": 1}, {"type": "wander", "priority": 5, "interval": 1}]);
    let t = ty(goals, |v| v["tameable"]["chance"] = json!(1.0));
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut m = plain(&t, 0.5, 0.5);
    tame::tame(&mut m, OWNER);
    let others = [];
    let w = player_view(&fx, &others, at(0.5, 3.5), OWNER);
    let start = m.position;
    run(&mut m, &w, &mut ctx, 200);
    assert!(m.ai.goals.is_running("Sit"));
    assert_eq!((m.position.x, m.position.z), (start.x, start.z), "sitting still");
    tame::set_sitting(&mut m, false);
    // (The despawn check that resets this counter when a player is near is not part of the tick.)
    m.ai.no_action_time = 0;
    run(&mut m, &w, &mut ctx, 100);
    assert!(!m.ai.goals.is_running("Sit"));
    assert!(m.position.distance(start) > 0.5, "it walks off once it may");
}

// ---- following the owner -----------------------------------------------------------------------

#[test]
fn a_tamed_fox_follows_its_owner_and_a_sitting_one_does_not() {
    let t = ty(json!([{"type": "follow_owner", "priority": 4, "speed": 1.2}]), |v| {
        v["tameable"]["chance"] = json!(1.0)
    });
    let fx = flat();
    let others = [];
    let owner_at = at(0.5, 12.0);
    let w = player_view(&fx, &others, owner_at, OWNER);
    // Eleven blocks away is further than the 10 it starts at; it walks and ends within a few blocks.
    let mut m = plain(&t, 0.5, 1.0);
    tame::tame(&mut m, OWNER);
    tame::set_sitting(&mut m, false);
    let mut ctx = Ctx::new();
    let before = m.position.distance(owner_at);
    assert!(before > 10.0 && before < 12.0);
    run(&mut m, &w, &mut ctx, 20);
    assert!(m.ai.goals.is_running("FollowOwner"));
    run(&mut m, &w, &mut ctx, 200);
    assert!(m.position.distance(owner_at) < 4.0, "arrived: {}", m.position.distance(owner_at));
    // A wild fox, or one that someone else owns, ignores the player.
    let mut wild = plain(&t, 0.5, 1.0);
    let mut c2 = Ctx::new();
    run(&mut wild, &w, &mut c2, 100);
    assert!(wild.position.distance(owner_at) > 10.0);
    let mut theirs = plain(&t, 0.5, 1.0);
    tame::tame(&mut theirs, STRANGER);
    tame::set_sitting(&mut theirs, false);
    run(&mut theirs, &w, &mut c2, 100);
    assert!(theirs.position.distance(owner_at) > 10.0);
    // Ordered to sit, the owner can walk away.
    let mut sitter = plain(&t, 0.5, 1.0);
    tame::tame(&mut sitter, OWNER);
    assert!(sitter.extra.sitting);
    run(&mut sitter, &w, &mut c2, 100);
    assert!(sitter.position.distance(owner_at) > 10.0);
}

/// An owner 12 blocks away or more is a teleport, not a walk.
#[test]
fn a_far_owner_is_a_teleport() {
    let t = ty(json!([{"type": "follow_owner", "priority": 4}]), |v| v["tameable"]["chance"] = json!(1.0));
    let fx = flat();
    let others = [];
    let owner_at = at(0.5, 30.0);
    let w = player_view(&fx, &others, owner_at, OWNER);
    let mut m = plain(&t, 0.5, 1.0);
    tame::tame(&mut m, OWNER);
    tame::set_sitting(&mut m, false);
    let mut ctx = Ctx::new();
    run(&mut m, &w, &mut ctx, 60);
    assert!(m.position.distance(owner_at) < 6.0, "{:?}", m.position);
}

/// `tameable.follow_owner` and `sits_on_command` give the goals a type does not list.
#[test]
fn tameable_flags_add_the_default_goals() {
    let t = ty(json!([]), |v| v["tameable"]["chance"] = json!(1.0));
    let m = plain(&t, 0.5, 0.5);
    assert!(m.ai.goals.has("FollowOwner") && m.ai.goals.has("Sit"));
    let none = ty(json!([]), |v| {
        v["tameable"] = json!({"items": [BERRIES]});
    });
    let m = plain(&none, 0.5, 0.5);
    assert!(!m.ai.goals.has("FollowOwner") && !m.ai.goals.has("Sit"));
    // A listed row is not doubled.
    let listed = ty(json!([{"type": "follow_owner", "priority": 2}, {"type": "sit", "priority": 1}]), |_| {});
    let m = plain(&listed, 0.5, 0.5);
    assert!(m.ai.goals.has("FollowOwner") && m.ai.goals.has("Sit"));
}

// ---- owner targeting ---------------------------------------------------------------------------

fn with_targets(rows: Value) -> Arc<CustomType> {
    fox_type_with(
        json!({"navigator": "ground", "goals": [{"type": "melee_attack", "priority": 3}], "targets": rows}),
        |v| v["tameable"]["chance"] = json!(1.0),
    )
}

fn hurt_world<'a>(
    fx: &'a crate::mobs::mob_hook_bench::BenchWorld,
    others: &'a [crate::mobs::Seen],
    hurt_by: Option<(Who, i64)>,
    last_hurt: Option<(Who, i64)>,
) -> crate::mobs::MobWorld<'a> {
    let mut w = player_view(fx, others, at(0.5, 3.5), OWNER);
    let p = w.player.as_mut().unwrap();
    p.hurt_by = hurt_by;
    p.last_hurt = last_hurt;
    w
}

#[test]
fn owner_hurt_by_attacks_what_hurt_the_owner() {
    let t = with_targets(json!([{"type": "owner_hurt_by", "priority": 1}]));
    let mut m = plain(&t, 0.5, 0.5);
    tame::tame(&mut m, OWNER);
    tame::set_sitting(&mut m, false);
    let fx = flat();
    let zombie = seen(5, Species::Zombie, at(3.5, 3.5));
    let others = [zombie];
    let mut ctx = Ctx::new();
    // Nothing happened to the owner yet.
    let w = hurt_world(&fx, &others, None, None);
    select(&mut m, &w, &mut ctx);
    assert!(m.ai.target.is_none());
    // The zombie hurt the owner (this is what the owner hit does not trigger).
    let w = hurt_world(&fx, &others, Some((Who::Mob(5), 40)), Some((Who::Mob(6), 41)));
    select(&mut m, &w, &mut ctx);
    assert_eq!(m.ai.target, Some(Who::Mob(5)));
}

#[test]
fn owner_hurt_target_attacks_what_the_owner_attacked() {
    let t = with_targets(json!([{"type": "owner_hurt_target", "priority": 1}]));
    let mut m = plain(&t, 0.5, 0.5);
    tame::tame(&mut m, OWNER);
    tame::set_sitting(&mut m, false);
    let fx = flat();
    let others = [seen(5, Species::Zombie, at(3.5, 3.5)), seen(6, Species::Skeleton, at(4.5, 3.5))];
    let mut ctx = Ctx::new();
    // The attacker of the owner is not what this one follows; what the owner hit is.
    let w = hurt_world(&fx, &others, Some((Who::Mob(5), 40)), Some((Who::Mob(6), 41)));
    select(&mut m, &w, &mut ctx);
    assert_eq!(m.ai.target, Some(Who::Mob(6)));
}

#[test]
fn owner_target_takes_either_the_more_recent() {
    let t = with_targets(json!([{"type": "owner_target", "priority": 1}]));
    let fx = flat();
    let others = [seen(5, Species::Zombie, at(3.5, 3.5)), seen(6, Species::Skeleton, at(4.5, 3.5))];
    let mut ctx = Ctx::new();
    for (by, hit, want) in [(40, 41, 6), (50, 41, 5)] {
        let mut m = plain(&t, 0.5, 0.5);
        tame::tame(&mut m, OWNER);
        tame::set_sitting(&mut m, false);
        let w = hurt_world(&fx, &others, Some((Who::Mob(5), by)), Some((Who::Mob(6), hit)));
        select(&mut m, &w, &mut ctx);
        assert_eq!(m.ai.target, Some(Who::Mob(want)), "hurt_by at {by}, last_hurt at {hit}");
    }
}

/// Not for a wild mob, a sitting one, another owner's mob, a stranger's quarrel, a creeper, a pet of
/// the same owner or itself; and the same event does not retrigger after the goal ended.
#[test]
fn the_owner_goals_leave_out_what_they_should() {
    let t = with_targets(json!([{"type": "owner_hurt_by", "priority": 1}]));
    let fx = flat();
    let mut ctx = Ctx::new();
    let ev = Some((Who::Mob(5), 40));
    let go = |m: &mut Mob, others: &[crate::mobs::Seen], uuid: [i32; 4], ctx: &mut Ctx| {
        let mut w = player_view(&fx, others, at(0.5, 3.5), uuid);
        w.player.as_mut().unwrap().hurt_by = ev;
        select(m, &w, ctx);
    };
    let zombie = [seen(5, Species::Zombie, at(3.5, 3.5))];
    let mut wild = plain(&t, 0.5, 0.5);
    go(&mut wild, &zombie, OWNER, &mut ctx);
    assert!(wild.ai.target.is_none(), "a wild mob has no owner");
    let mut theirs = plain(&t, 0.5, 0.5);
    tame::tame(&mut theirs, STRANGER);
    tame::set_sitting(&mut theirs, false);
    go(&mut theirs, &zombie, OWNER, &mut ctx);
    assert!(theirs.ai.target.is_none(), "another player's pet");
    let mut sitting = plain(&t, 0.5, 0.5);
    tame::tame(&mut sitting, OWNER);
    assert!(sitting.extra.sitting);
    go(&mut sitting, &zombie, OWNER, &mut ctx);
    assert!(sitting.ai.target.is_none(), "sitting");
    let creeper = [seen(5, Species::Creeper, at(3.5, 3.5))];
    let mut m = plain(&t, 0.5, 0.5);
    tame::tame(&mut m, OWNER);
    tame::set_sitting(&mut m, false);
    go(&mut m, &creeper, OWNER, &mut ctx);
    assert!(m.ai.target.is_none(), "creepers are left alone");
    // A pet of the same owner.
    let mut pet = seen(5, Species::Wolf, at(3.5, 3.5));
    pet.tame = true;
    pet.owner = Some(OWNER);
    go(&mut m, &[pet], OWNER, &mut ctx);
    assert!(m.ai.target.is_none(), "the same owner's pet");
    // The same event twice: after the target is lost the old event does not call it back.
    go(&mut m, &zombie, OWNER, &mut ctx);
    assert_eq!(m.ai.target, Some(Who::Mob(5)));
    m.ai.target = None;
    select(&mut m, &player_view(&fx, &[], at(0.5, 3.5), OWNER), &mut ctx);
    go(&mut m, &zombie, OWNER, &mut ctx);
    assert!(m.ai.target.is_none(), "answered once");
}

// ---- breeding ----------------------------------------------------------------------------------

fn breeder(cooldown: Option<u32>, edit: impl FnOnce(&mut Value)) -> Arc<CustomType> {
    let mut b = json!({"food": ["minecraft:wheat"], "baby_scale": 0.4});
    if let Some(c) = cooldown {
        b["cooldown"] = json!(c);
    }
    ty(
        json!([{"type": "breed", "priority": 2, "speed": 1.2}, {"type": "follow_parent", "priority": 3}]),
        |v| {
            v.as_object_mut().unwrap().remove("tameable");
            v["breedable"] = b;
            edit(v);
        },
    )
}

const WHEAT: &str = "minecraft:wheat";

#[test]
fn feeding_an_adult_starts_love_once_and_feeding_a_baby_brings_growth_forward() {
    let t = breeder(Some(600), |_| {});
    let mut m = plain(&t, 0.5, 0.5);
    let o = interact::dispatch(&mut m, &click(Some(WHEAT), OWNER), None);
    assert_eq!(o.reply, Reply::Consume);
    assert_eq!(o.actions, [Action::Eat, Action::Event(18)]);
    assert_eq!((m.extra.in_love, m.extra.love_cause), (600, Some(OWNER)));
    assert_eq!(breed::feed(&mut m, Some(OWNER)), Fed::No, "already in love");
    assert_eq!(interact::dispatch(&mut m, &click(Some(WHEAT), OWNER), None), Outcome::pass());
    assert_eq!(interact::dispatch(&mut m, &click(Some("minecraft:stick"), OWNER), None), Outcome::pass());
    // A baby grows a tenth of what it has left per feeding.
    let mut baby = plain(&t, 0.5, 0.5);
    baby.extra.age = BABY_AGE;
    assert_eq!(breed::feed(&mut baby, Some(OWNER)), Fed::Grew);
    assert_eq!(baby.extra.age, BABY_AGE + 2400);
    assert_eq!(baby.extra.in_love, 0, "a baby is not in love");
    // A pause: no love.
    let mut rest = plain(&t, 0.5, 0.5);
    rest.extra.age = 300;
    assert_eq!(breed::feed(&mut rest, None), Fed::No);
    // A type that must be tamed first is not hungry wild.
    let wild = fox_type_with(json!({"navigator": "ground", "goals": []}), |v| {
        v["breedable"] = json!({"food": [WHEAT]});
    });
    let mut f = plain(&wild, 0.5, 0.5);
    assert_eq!(breed::feed(&mut f, None), Fed::No, "a tameable type falls in love tame only");
    tame::tame(&mut f, OWNER);
    assert_eq!(breed::feed(&mut f, None), Fed::Love);
}

/// Two adults in love close together breed: both get the pause, the baby comes (the game's part,
/// `complete`, is called here as the game calls it), with the type's baby age and scale.
#[test]
fn two_foxes_in_love_breed_and_rest_and_the_baby_is_small() {
    let t = breeder(Some(600), |_| {});
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut a = plain(&t, 0.5, 0.5);
    let mut b = fox_mob(&t, at(2.5, 0.5));
    b.id = 2;
    a.extra.in_love = 600;
    b.extra.in_love = 600;
    let others = [b.seen(None)];
    let w = view(&fx.world(), &others, None);
    let mut breeds = None;
    for _ in 0..200 {
        select(&mut a, &w, &mut ctx);
        if let Some(e) = ctx.events.iter().find_map(|e| match e {
            MobEvent::Breed { parent, partner, .. } => Some((*parent, *partner)),
            _ => None,
        }) {
            breeds = Some(e);
            break;
        }
    }
    assert_eq!(breeds, Some((1, 2)), "the breed event of the pair");
    assert_eq!((a.extra.in_love, a.extra.age), (0, 600), "the parent rests for the cooldown");
    // The game's part.
    let mut child = fox_mob(&t, at(1.5, 0.5));
    child.id = 3;
    breed::complete(&mut a, Some(&mut b), &mut child);
    assert_eq!((b.extra.in_love, b.extra.age), (0, 600), "the partner rests too");
    assert_eq!(child.extra.age, BABY_AGE);
    assert!(child.is_baby());
    assert!((child.model_scale() - 0.4).abs() < 1e-6, "baby scale {}", child.model_scale());
    assert_eq!(
        a.extra.custom.as_mut().unwrap().take_notices(),
        [Notice::Bred { partner: Some(2), child: Some(3) }]
    );
    assert_eq!(breed::offspring_kind(&a).as_deref(), Some("sample_mobs:cinder_fox"));
}

#[test]
fn the_pause_counts_down_and_a_baby_grows_up_and_loses_its_small_scale() {
    let t = breeder(Some(100), |_| {});
    let mut m = plain(&t, 0.5, 0.5);
    m.extra.age = 100;
    for _ in 0..99 {
        breed::tick(&mut m, None);
    }
    assert_eq!(m.extra.age, 1);
    assert_eq!(breed::feed(&mut m, None), Fed::No, "still resting");
    breed::tick(&mut m, None);
    assert_eq!(m.extra.age, 0);
    assert_eq!(breed::feed(&mut m, None), Fed::Love, "ready again");
    // A baby: small, then grown after its 24000 ticks.
    let mut baby = plain(&t, 0.5, 0.5);
    baby.extra.age = BABY_AGE;
    breed::tick(&mut baby, None);
    assert!((baby.model_scale() - 0.4).abs() < 1e-6);
    for _ in 0..23_998 {
        breed::tick(&mut baby, None);
    }
    assert!(baby.is_baby());
    breed::tick(&mut baby, None);
    breed::tick(&mut baby, None);
    assert!(!baby.is_baby() && baby.extra.age == 0);
    assert!((baby.model_scale() - 1.0).abs() < 1e-6, "full size again: {}", baby.model_scale());
    // An age lock holds a baby back.
    let mut locked = plain(&t, 0.5, 0.5);
    locked.extra.age = BABY_AGE;
    locked.extra.age_locked = true;
    breed::tick(&mut locked, None);
    assert_eq!(locked.extra.age, BABY_AGE);
}

#[test]
fn offspring_and_partner_can_be_other_types() {
    let t = breeder(None, |v| {
        v["breedable"]["offspring"] = json!("sample_mobs:kit");
        v["breedable"]["partner"] = json!("sample_mobs:other");
    });
    let m = plain(&t, 0.5, 0.5);
    assert_eq!(breed::offspring_kind(&m).as_deref(), Some("sample_mobs:kit"));
    assert_eq!(t.breed.as_ref().unwrap().cooldown, breed::DEFAULT_COOLDOWN);
    // Its own kind is no partner: the goal looks for the other type.
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut a = plain(&t, 0.5, 0.5);
    a.extra.in_love = 600;
    let mine = {
        let mut b = fox_mob(&t, at(2.5, 0.5));
        b.id = 2;
        b.extra.in_love = 600;
        b.seen(None)
    };
    let mut theirs = seen(3, Species::Fox, at(2.5, 1.5));
    theirs.custom = Some(Arc::from("sample_mobs:other"));
    theirs.in_love = true;
    let others = [mine];
    select(&mut a, &view(&fx.world(), &others, None), &mut ctx);
    assert!(!a.ai.goals.is_running("Breed"), "its own type is not its partner");
    let others = [theirs];
    select(&mut a, &view(&fx.world(), &others, None), &mut ctx);
    assert!(a.ai.goals.is_running("Breed"));
}

#[test]
fn a_tamed_parent_has_a_tamed_baby_of_the_same_owner() {
    let t = ty(json!([]), |v| {
        v["tameable"]["chance"] = json!(1.0);
        v["breedable"] = json!({"food": [WHEAT]});
    });
    let mut a = plain(&t, 0.5, 0.5);
    tame::tame(&mut a, OWNER);
    let mut child = plain(&t, 0.5, 0.5);
    child.id = 9;
    breed::complete(&mut a, None, &mut child);
    assert_eq!(child.extra.owner, Some(OWNER));
    assert!(child.extra.tame && !child.extra.sitting);
}

#[test]
fn a_baby_follows_an_adult_of_its_type() {
    let t = breeder(None, |_| {});
    let fx = flat();
    let mut ctx = Ctx::new();
    let mut baby = plain(&t, 0.5, 0.5);
    baby.extra.age = BABY_AGE;
    let mut adult = fox_mob(&t, at(0.5, 6.5));
    adult.id = 2;
    let others = [adult.seen(None)];
    let w = view(&fx.world(), &others, None);
    run(&mut baby, &w, &mut ctx, 100);
    assert!(baby.ai.goals.is_running("FollowParent") || baby.position.z > 2.0);
    assert!(baby.position.z > 2.0, "it walked toward the adult: {:?}", baby.position);
    // An adult follows no one.
    let mut grown = plain(&t, 0.5, 0.5);
    run(&mut grown, &w, &mut ctx, 100);
    assert!(!grown.ai.goals.is_running("FollowParent"));
}

// ---- seats and riding ----------------------------------------------------------------------------

fn rideable_type(edit: impl FnOnce(&mut Value)) -> Arc<CustomType> {
    ty(json!([]), |v| {
        v.as_object_mut().unwrap().remove("tameable");
        v["seats"] = json!([
            {"offset": [0.0, 0.9, -0.2], "controls": "player", "yaw_lock": true, "allowed_types": ["minecraft:player"]},
            {"locator": "mouth", "allowed_types": ["minecraft:chicken"]}
        ]);
        edit(v);
    })
}

/// The passenger sits at the seat's static offset turned with the mob, less the player's own vehicle
/// attachment, on the server.
#[test]
fn the_passenger_is_placed_by_the_servers_static_offset() {
    let t = rideable_type(|_| {});
    let mut m = plain(&t, 10.0, 20.0);
    let d = &t.ride;
    assert_eq!(d.seats.len(), 2);
    assert_eq!(d.seats[0].offset, [0.0, 0.9, -0.2]);
    assert_eq!(d.seats[0].controls, Controls::Player);
    assert!(d.seats[0].yaw_lock && !d.seats[1].yaw_lock);
    // A locator seat is where the model puts the locator (the mouth is ahead of and above the feet).
    let mouth = d.seats[1].offset;
    assert!(mouth[1] > 0.0 && mouth != [0.0; 3], "the mouth: {mouth:?}");
    assert_eq!(d.seats[1].locator.as_deref(), Some("mouth"));
    // Facing +z at yaw 0: behind is -z.
    let at0 = riding::seat_position(d, 0, m.position, 0.0, None);
    assert!((at0.x - 10.0).abs() < 1e-9 && (at0.y - 1.9).abs() < 1e-9 && (at0.z - 19.8).abs() < 1e-9, "{at0:?}");
    // Turned a quarter: the offset turns with the mob.
    let at90 = riding::seat_position(d, 0, m.position, 90.0, None);
    assert!((at90.y - 1.9).abs() < 1e-9);
    assert!(((at90.x - 10.0).abs() - 0.2).abs() < 1e-6 && (at90.z - 20.0).abs() < 1e-6, "{at90:?}");
    // The client's smooth seat (F4M-6) replaces it.
    let smooth = riding::seat_position(d, 0, m.position, 0.0, Some([0.0, 1.0, 0.5]));
    assert!((smooth.y - 2.0).abs() < 1e-9 && (smooth.z - 20.5).abs() < 1e-9);
    // What the engine's rider placement reads: the first seat as the attachment.
    assert_eq!(riders::passenger_attachment(&m), [0.0, 0.9, -0.2]);
    let p = riders::seat(riders::passenger_attachment(&m), 0.6, m.position, m.yaw);
    assert!((p.y - 1.3).abs() < 1e-9 && (p.z - 19.8).abs() < 1e-9, "feet of a player: {p:?}");
    // A baby scales its seat as it scales the mob.
    m.extra.model_scale = 0.5;
    assert!((riders::passenger_attachment(&m)[1] - 0.45).abs() < 1e-9);
    // The yaw lock turns the rider with the mob.
    m.yaw = 33.0;
    m.rider = Some(rider());
    assert_eq!(riding::locked_yaw(&m), Some(33.0));
}

fn rider() -> equine::Rider {
    equine::Rider {
        strafe: 0.0,
        forward: 1.0,
        yaw: 0.0,
        pitch: 0.0,
        jump: 0.0,
        steer: false,
        sprint: false,
        jumping: false,
        client: false,
    }
}

/// Who takes which seat: the player the first one that allows it, mobs the next that allow them.
#[test]
fn seats_are_taken_by_allowed_types_in_order() {
    let t = rideable_type(|_| {});
    let m = plain(&t, 0.5, 0.5);
    let chicken = RiderKind::Mob { species: Species::Chicken, custom: None };
    let cow = RiderKind::Mob { species: Species::Cow, custom: None };
    assert_eq!(riding::free_seat(&m, RiderKind::Player, false, &[]), Some(0));
    assert_eq!(riding::free_seat(&m, chicken, false, &[]), Some(1));
    assert_eq!(riding::free_seat(&m, cow, false, &[]), None, "a cow is allowed nowhere");
    assert_eq!(riding::free_seat(&m, chicken, true, &[]), Some(1));
    assert_eq!(riding::free_seat(&m, RiderKind::Player, true, &[]), Some(0), "the player's own seat");
    assert_eq!(riding::free_seat(&m, chicken, false, &[chicken]), None, "the one chicken seat is taken");
    // A type with no allowed list lets anything sit, and a custom type is matched by its id.
    let open = ty(json!([]), |v| {
        v.as_object_mut().unwrap().remove("tameable");
        v["seats"] = json!([{"offset": [0, 1, 0]}, {"offset": [0, 1, -1], "allowed_types": ["sample_mobs:cinder_fox"]}]);
    });
    let m = plain(&open, 0.5, 0.5);
    let fox = RiderKind::Mob { species: Species::Custom, custom: Some("sample_mobs:cinder_fox") };
    assert_eq!(riding::free_seat(&m, cow, false, &[]), Some(0));
    assert_eq!(riding::free_seat(&m, fox, false, &[cow]), Some(1));
    let none = plain(&ty(json!([]), |_| {}), 0.5, 0.5);
    assert_eq!(riding::free_seat(&none, RiderKind::Player, false, &[]), None, "no seats");
}

/// The saddle, the owner of a tameable type and `rideable_when` decide whether a mob may be ridden,
/// and a controlling seat steers once all hold.
#[test]
fn the_saddle_the_owner_and_rideable_when_decide() {
    let fx = flat();
    let others = [];
    let w = view(&fx.world(), &others, None);
    // A saddled type.
    let t = rideable_type(|v| v["saddle_item"] = json!("minecraft:saddle"));
    let mut m = plain(&t, 0.5, 0.5);
    assert!(!riding::rideable(&m, &w, Some(OWNER)), "no saddle yet");
    assert!(riding::takes_saddle(&m, Some("minecraft:saddle")));
    assert!(!riding::takes_saddle(&m, Some("minecraft:stick")) && !riding::takes_saddle(&m, None));
    let o = interact::dispatch(&mut m, &click(Some("minecraft:saddle"), OWNER), None);
    assert_eq!((o.reply, o.actions), (Reply::Consume, vec![Action::Saddle]));
    m.set_saddle(Some(crate::items::ItemStack::new("minecraft:saddle", 1)));
    assert!(riding::rideable(&m, &w, Some(OWNER)));
    assert!(!riding::takes_saddle(&m, Some("minecraft:saddle")), "already on");
    // The controlling seat steers a ridden, saddled mob (`equine::controlled` reads it).
    assert!(!equine::controlled(&m), "nobody on it");
    m.rider = Some(rider());
    assert!(equine::controlled(&m));
    let plain_seats = rideable_type(|v| v["seats"][0]["controls"] = json!("none"));
    let mut carried = plain(&plain_seats, 0.5, 0.5);
    carried.rider = Some(rider());
    assert!(!equine::controlled(&carried), "a seat that does not control only carries");
    let mut unsaddled = plain(&t, 0.5, 0.5);
    unsaddled.rider = Some(rider());
    assert!(!equine::controlled(&unsaddled), "no saddle, no steering");
    // A tameable type is ridden by its owner, tamed.
    let tt = ty(json!([]), |v| {
        v["tameable"]["chance"] = json!(1.0);
        v["seats"] = json!([{"offset": [0, 1, 0], "controls": "player"}]);
    });
    let mut f = plain(&tt, 0.5, 0.5);
    assert!(!riding::rideable(&f, &w, Some(OWNER)), "wild");
    tame::tame(&mut f, OWNER);
    assert!(riding::rideable(&f, &w, Some(OWNER)));
    assert!(!riding::rideable(&f, &w, Some(STRANGER)), "not the owner");
    // `rideable_when` is the goal rows' expression language.
    let e = rideable_type(|v| v["rideable_when"] = json!("query.health > 10"));
    let mut hurt = plain(&e, 0.5, 0.5);
    assert!(riding::rideable(&hurt, &w, Some(OWNER)));
    hurt.health = 5.0;
    assert!(!riding::rideable(&hurt, &w, Some(OWNER)));
    // A bad expression is the file's error.
    let bad = std::panic::catch_unwind(|| rideable_type(|v| v["rideable_when"] = json!("query.nothing_like_this +")));
    assert!(bad.is_err(), "the type does not load with a bad rideable_when");
}

/// The click that gets on: the dispatch gives the seat it was told the player may take.
#[test]
fn a_click_on_a_rideable_mob_boards_it_unless_sneaking() {
    let t = rideable_type(|_| {});
    let mut m = plain(&t, 0.5, 0.5);
    let mut ctx = click(None, OWNER);
    ctx.ride_seat = Some(0);
    let o = interact::dispatch(&mut m, &ctx, None);
    assert_eq!((o.reply, o.actions), (Reply::Success, vec![Action::Ride { seat: 0 }]));
    ctx.sneaking = true;
    assert_eq!(interact::dispatch(&mut m, &ctx, None), Outcome::pass(), "sneaking does not board");
    ctx.sneaking = false;
    ctx.ride_seat = None;
    assert_eq!(interact::dispatch(&mut m, &ctx, None), Outcome::pass(), "no seat to take");
    // An explicit `ride` row boards even while sneaking when it says so.
    let row = rideable_type(|v| v["interact"] = json!([{"action": "ride", "sneaking": true}]));
    let mut r = plain(&row, 0.5, 0.5);
    let mut c = click(None, OWNER);
    c.sneaking = true;
    c.ride_seat = Some(0);
    assert_eq!(interact::dispatch(&mut r, &c, None).actions, [Action::Ride { seat: 0 }]);
}

// ---- the mob-interact plumbing -----------------------------------------------------------------

struct Double(Reply, std::sync::Mutex<Vec<(Option<String>, bool)>>);
impl Hook for Double {
    fn mob_interact(&self, _: &mut Mob, ctx: &Context) -> Reply {
        self.1.lock().unwrap().push((ctx.item.map(str::to_owned), ctx.hand == Hand::Off));
        self.0
    }
}

#[test]
fn the_hook_decides_first_and_pass_goes_on_to_the_data_shortcuts() {
    let t = ty(json!([]), |v| v["tameable"]["chance"] = json!(1.0));
    // Consume and Success end it: nothing of the data runs.
    for reply in [Reply::Consume, Reply::Success] {
        let hook = Double(reply, Default::default());
        let mut m = plain(&t, 0.5, 0.5);
        let mut c = click(Some(BERRIES), OWNER);
        c.hand = Hand::Off;
        let o = interact::dispatch(&mut m, &c, Some(&hook));
        assert_eq!((o.reply, o.actions.len()), (reply, 0));
        assert!(!m.extra.tame, "the berry did not tame it: the hook handled the click");
        assert_eq!(*hook.1.lock().unwrap(), [(Some(BERRIES.to_owned()), true)], "the hook saw the item and hand");
    }
    // Pass: the shortcuts run.
    let hook = Double(Reply::Pass, Default::default());
    let mut m = plain(&t, 0.5, 0.5);
    let o = interact::dispatch(&mut m, &click(Some(BERRIES), OWNER), Some(&hook));
    assert_eq!(o.reply, Reply::Consume);
    assert!(m.extra.tame);
    assert_eq!(hook.1.lock().unwrap().len(), 1);
    // A dead mob and a vanilla mob are not interacted with.
    let mut dead = plain(&t, 0.5, 0.5);
    dead.health = 0.0;
    assert_eq!(interact::dispatch(&mut dead, &click(Some(BERRIES), OWNER), Some(&hook)), Outcome::pass());
    let mut cow = Mob::new(1, crate::mobs::sensor_scan_tests::kind(), at(0.5, 0.5), 0.0);
    assert_eq!(interact::dispatch(&mut cow, &click(Some(BERRIES), OWNER), Some(&hook)), Outcome::pass());
    assert_eq!(hook.1.lock().unwrap().len(), 1, "the hook never saw them");
}

/// The installed hook is what `dispatch_global` (the game's entry) calls. The only test that touches
/// the global slot.
#[test]
fn the_installed_hook_is_what_the_game_reaches() {
    let t = ty(json!([]), |_| {});
    let mut m = plain(&t, 0.5, 0.5);
    assert_eq!(interact::dispatch_global(&mut m, &click(None, OWNER)), Outcome::pass());
    let hook = Arc::new(Double(Reply::Success, Default::default()));
    let old = interact::set_hook(Some(hook.clone()));
    assert!(old.is_none());
    let o = interact::dispatch_global(&mut m, &click(Some("minecraft:stick"), OWNER));
    assert_eq!(o.reply, Reply::Success);
    assert_eq!(hook.1.lock().unwrap().len(), 1);
    assert!(interact::set_hook(None).is_some());
    assert_eq!(interact::dispatch_global(&mut m, &click(None, OWNER)), Outcome::pass());
}

#[test]
fn rows_feed_tame_give_and_the_implicit_shortcuts_follow() {
    let t = ty(json!([]), |v| {
        v["tameable"]["chance"] = json!(1.0);
        v["breedable"] = json!({"food": [WHEAT]});
        v["interact"] = json!([
            {"action": "give_item", "item": "minecraft:bowl", "give": "minecraft:mushroom_stew", "count": 2},
            {"action": "give_item", "item": "minecraft:glass_bottle", "give": "minecraft:potion", "consume": false}
        ]);
    });
    let mut m = plain(&t, 0.5, 0.5);
    let o = interact::dispatch(&mut m, &click(Some("minecraft:bowl"), OWNER), None);
    assert_eq!(o.reply, Reply::Consume);
    assert_eq!(o.actions, [Action::Give { item: "minecraft:mushroom_stew".into(), count: 2 }]);
    let o = interact::dispatch(&mut m, &click(Some("minecraft:glass_bottle"), OWNER), None);
    assert_eq!(o.reply, Reply::Success, "`consume: false`");
    assert!(!m.extra.tame, "the rows came first and took the click");
    // The implicit shortcuts: berries tame, and once tamed wheat feeds (tamed foxes fall in love).
    interact::dispatch(&mut m, &click(Some(BERRIES), OWNER), None);
    assert!(m.extra.tame);
    let o = interact::dispatch(&mut m, &click(Some(WHEAT), OWNER), None);
    assert_eq!(o.actions, [Action::Eat, Action::Event(18)]);
    // Tags go through the matcher.
    let tagged = ty(json!([]), |v| {
        v["tameable"]["items"] = json!(["#minecraft:fishes"]);
        v["tameable"]["chance"] = json!(1.0);
    });
    let mut f = plain(&tagged, 0.5, 0.5);
    let in_tag = |p: &str, i: &str| p == "#minecraft:fishes" && i == "minecraft:cod";
    let mut c = click(Some("minecraft:cod"), OWNER);
    c.matches = &in_tag;
    assert_eq!(interact::dispatch(&mut f, &c, None).reply, Reply::Consume);
    assert!(f.extra.tame);
}

// ---- the real tick -------------------------------------------------------------------------------

/// `Mob::tick` runs the native growth for a breedable type and not for others.
#[test]
fn the_mob_tick_runs_the_breeding_clock_of_breedable_types_only() {
    let t = breeder(Some(600), |_| {});
    let fx = flat();
    let others = [];
    let w = view(&fx.world(), &others, None);
    let mut lr = LegacyRandom::new(1);
    let mut events = Vec::new();
    let mut m = plain(&t, 0.5, 0.5);
    m.extra.age = 5;
    for _ in 0..3 {
        m.tick(&w, &mut lr, &mut events);
    }
    assert_eq!(m.extra.age, 2);
    let other = ty(json!([]), |_| {});
    let mut n = plain(&other, 0.5, 0.5);
    n.extra.age = 5;
    for _ in 0..3 {
        n.tick(&w, &mut lr, &mut events);
    }
    assert_eq!(n.extra.age, 5, "a type that is not breedable keeps its age");
}

/// A `feed` row with `heal` is data: the allowed items, the amount, tamed and hurt only; the item is
/// used up and the mob's health is clamped to its maximum.
#[test]
fn a_feed_row_heals_a_hurt_tamed_mob_with_the_items_it_allows() {
    const COD: &str = "minecraft:cod";
    let t = ty(json!([]), |v| {
        v["breedable"] = json!({"food": [WHEAT]});
        v["interact"] = json!([{"action": "feed", "heal": 4, "items": [COD, "#minecraft:fishes"]}]);
    });
    let tags = |p: &str, i: &str| p == i || (p == "#minecraft:fishes" && i == "minecraft:salmon");
    let click_with = |item: &'static str| {
        let mut c = click(Some(item), STRANGER);
        c.matches = &tags;
        c
    };
    let mut m = plain(&t, 0.5, 0.5);
    let max = m.attrs.max_health;
    m.health = max - 6.0;
    // Not tamed: nothing (the fish is not food of the type either).
    let o = interact::dispatch(&mut m, &click_with(COD), None);
    assert_eq!((o.reply, m.health), (Reply::Pass, max - 6.0), "wild");
    tame::tame(&mut m, OWNER);
    // Tamed and hurt: the item heals by the row's amount and is used up; tags go through the matcher.
    let o = interact::dispatch(&mut m, &click_with(COD), None);
    assert_eq!((o.reply, o.actions), (Reply::Consume, vec![Action::Eat]));
    assert_eq!(m.health, max - 2.0);
    let o = interact::dispatch(&mut m, &click_with("minecraft:salmon"), None);
    assert_eq!(o.reply, Reply::Consume, "a tag");
    assert_eq!(m.health, max, "clamped to the maximum");
    // Full health: the item is not used.
    let o = interact::dispatch(&mut m, &click_with(COD), None);
    assert_eq!(o.reply, Reply::Pass, "not hurt");
    // An item the row does not allow does not heal, even when it is the type's food (it breeds).
    m.health = max - 6.0;
    let o = interact::dispatch(&mut m, &click_with(WHEAT), None);
    assert_eq!(o.actions, [Action::Eat, Action::Event(18)]);
    assert_eq!(m.health, max - 6.0);
    assert_eq!(interact::dispatch(&mut m, &click_with("minecraft:stick"), None).reply, Reply::Pass);
    // Without `items`, the row's own item heals; `consume: false` keeps it.
    let t = ty(json!([]), |v| {
        v["interact"] = json!([{"action": "feed", "item": COD, "heal": 3, "consume": false}]);
    });
    let mut m = plain(&t, 0.5, 0.5);
    tame::tame(&mut m, OWNER);
    m.health = 5.0;
    let o = interact::dispatch(&mut m, &click_with(COD), None);
    assert_eq!((o.reply, m.health), (Reply::Success, 8.0));
    // Without `items` or `item`, the type's food heals.
    let t = ty(json!([]), |v| {
        v["breedable"] = json!({"food": [WHEAT]});
        v["interact"] = json!([{"action": "feed", "heal": 2}]);
    });
    let mut m = plain(&t, 0.5, 0.5);
    tame::tame(&mut m, OWNER);
    m.health = 5.0;
    let o = interact::dispatch(&mut m, &click_with(WHEAT), None);
    assert_eq!((o.reply, m.health), (Reply::Consume, 7.0));
    assert_eq!(m.extra.in_love, 0, "healing is not love");
}

// ---- one effective scale (the `scale` attribute times the baby scale) ------------------------------

/// The baby scale multiplies the type's `scale` attribute (one effective scale) instead of replacing
/// it, and the attribute alone scales an adult and a type that is not breedable.
#[test]
fn the_baby_scale_is_multiplied_with_the_scale_attribute() {
    use crate::mobs::custom::attrs;
    let t = breeder(Some(100), |v| v["attributes"]["scale"] = json!(2.0));
    // The attribute alone: the adult is twice the size, its box follows.
    let mut adult = plain(&t, 0.5, 0.5);
    let plain_h = adult.dims().1;
    breed::tick(&mut adult, None);
    assert!((adult.model_scale() - 2.0).abs() < 1e-6);
    assert!((adult.dims().1 - plain_h * 2.0).abs() < 1e-6, "the box follows");
    // A baby: 2 times 0.4, whether it is born or ticked into being one.
    let mut baby = plain(&t, 0.5, 0.5);
    baby.extra.age = BABY_AGE;
    breed::tick(&mut baby, None);
    assert!((baby.model_scale() - 0.8).abs() < 1e-6, "{}", baby.model_scale());
    assert!((attrs::attribute_scale(&baby) - 2.0).abs() < 1e-6, "the attribute itself is untouched");
    let mut parent = plain(&t, 0.5, 0.5);
    let mut child = plain(&t, 0.5, 0.5);
    breed::complete(&mut parent, None, &mut child);
    assert!((child.model_scale() - 0.8).abs() < 1e-6, "born: {}", child.model_scale());
    // Grown up (an age lock-free baby on its last tick): back to the attribute, not to what it was.
    baby.extra.age = -1;
    breed::tick(&mut baby, None);
    assert!(!baby.is_baby());
    assert!((baby.model_scale() - 2.0).abs() < 1e-6, "{}", baby.model_scale());
    // A type that is not breedable still gets its attribute; one with none stays at the default.
    let wild = ty(json!([]), |v| v["attributes"]["scale"] = json!(0.5));
    let mut w = plain(&wild, 0.5, 0.5);
    breed::tick(&mut w, None);
    assert!((w.model_scale() - 0.5).abs() < 1e-6);
    let none = ty(json!([]), |_| {});
    let mut n = plain(&none, 0.5, 0.5);
    breed::tick(&mut n, None);
    assert_eq!(n.extra.model_scale, 0.0, "no scale attribute: the engine's default is untouched");
    assert_eq!(n.model_scale(), 1.0);
}

/// A baby that grows up beside a wall is moved to where its bigger box fits
/// (`fudgePositionAfterSizeChange`), as the vanilla ageable mobs are; without a world nothing moves.
#[test]
fn a_baby_that_grows_up_is_moved_out_of_the_wall_its_bigger_box_would_overlap() {
    let t = breeder(Some(100), |_| {});
    let mut fx = flat();
    fx.set_stone(1, 1, 0);
    let others = [];
    let w = view(&fx.world(), &others, None);
    let grown = |world: Option<&crate::mobs::MobWorld>| {
        let mut m = plain(&t, 0.85, 0.5);
        m.age = 10;
        m.extra.age = BABY_AGE;
        breed::tick(&mut m, None);
        assert!(m.is_baby() && m.extra.model_scale > 0.0);
        m.extra.age = -1;
        breed::tick(&mut m, world);
        assert!(!m.is_baby());
        m.position
    };
    assert_eq!(grown(None).x, 0.85, "no world, no move");
    let moved = grown(Some(&w));
    // The grown box is 0.6 wide: its east side is at the wall (x = 1) at the farthest.
    assert!(moved.x <= 0.7 + 1e-4 && moved.x > 0.55, "moved off the wall: {moved:?}");
}
