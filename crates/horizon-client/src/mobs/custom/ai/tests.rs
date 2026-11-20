//! Tests of the custom AI on the flat resource-free world of `test_support` (a stone floor at
//! `y = 0`; mobs stand on `y = 1`). One test per goal, then the control flags, the navigators,
//! the attacks' damage frames, and the path budget.
use super::{
    Support, attacks,
    attacks::{ChaseStrike, Strike, Timing},
    nav_select, species_list, support,
    test_support::*,
    unsupported,
};
use crate::mobs::{
    Damage, MobEvent, Species, Who,
    control::MoveKind,
    goals::{Goal, MeleeAttack},
    nav::NavKind,
};
use horizon_core::geometry::Vec3;
use horizon_mods::custom_mob::GOAL_TYPES;

fn at(x: f64, z: f64) -> Vec3 {
    Vec3::new(x, 1.0, z)
}

fn ai(goals: &str) -> String {
    format!(r#"{{"goals":[{goals}]}}"#)
}
fn ai_targets(targets: &str) -> String {
    format!(r#"{{"targets":[{targets}]}}"#)
}

fn hits(events: &[MobEvent]) -> usize {
    count(events, |e| matches!(e, MobEvent::AttackPlayer(..)))
}

// ---- the catalogue -------------------------------------------------------------------------

#[test]
fn every_schema_goal_type_has_a_status() {
    let mut running = 0;
    let mut deferred = Vec::new();
    for kind in GOAL_TYPES {
        match support(kind) {
            Support::Goal | Support::Target => running += 1,
            Support::Deferred(why) => {
                assert_ne!(why, "unknown goal type", "{kind} has no status");
                deferred.push(kind);
            }
        }
    }
    assert_eq!(running, 30);
    deferred.sort();
    assert_eq!(
        deferred,
        [
            "breed",
            "eat_block",
            "follow_owner",
            "follow_parent",
            "open_door",
            "owner_hurt_by",
            "owner_hurt_target",
            "owner_target",
            "script",
            "sit",
            "sleep_in_bed_like"
        ]
    );
    assert_eq!(support("owner_target"), Support::Deferred("F4M-4c: needs the tameable owner state"));
}

#[test]
fn the_fixture_fox_lists_the_goals_it_does_not_get() {
    let t = super::super::tests::fox_type();
    let rows = unsupported(&t.file);
    let list: Vec<&str> = rows.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(list, ["script", "follow_owner", "sit"]);
}

#[test]
fn entity_type_lists_split_into_species_player_and_the_rest() {
    let ids = ["minecraft:chicken", "minecraft:player", "demo:other", "minecraft:nothing"]
        .map(String::from);
    let (species, player, unknown) = species_list(&ids);
    assert_eq!(species, [Species::Chicken]);
    assert!(player);
    assert_eq!(unknown, ["demo:other", "minecraft:nothing"]);
}

// ---- movement and idle goals, one each ------------------------------------------------------

#[test]
fn swim_floats_a_mob_that_is_in_lava_or_deep_water() {
    let fx = flat();
    let w = fx.world();
    let mut ctx = Ctx::new();
    let mut m = mob(&ai(r#"{"type":"swim","priority":0}"#), at(0.5, 0.5));
    assert!(m.ai.goals.has("Float"));
    select(&mut m, &w, &mut ctx);
    assert!(!m.ai.goals.is_running("Float"), "dry land");
    m.fluids.in_lava = true;
    select(&mut m, &w, &mut ctx);
    assert!(m.ai.goals.is_running("Float"));
    let mut jumped = false;
    for _ in 0..20 {
        select(&mut m, &w, &mut ctx);
        jumped |= m.ai.jump;
    }
    assert!(jumped, "it swims up");
}

#[test]
fn wander_walks_somewhere() {
    let fx = flat();
    let w = fx.world();
    let mut ctx = Ctx::new();
    let mut m = mob(
        &ai(r#"{"type":"wander","priority":5,"speed":1.0,"interval":1}"#),
        at(0.5, 0.5),
    );
    assert!(m.ai.goals.has("Stroll"));
    run(&mut m, &w, &mut ctx, 100);
    let moved = (m.position.x - 0.5).hypot(m.position.z - 0.5);
    assert!(moved > 1.0, "moved {moved}");
}

#[test]
fn stroll_is_the_same_goal_under_its_new_name() {
    let m = mob(&ai(r#"{"type":"stroll","priority":5}"#), at(0.5, 0.5));
    assert!(m.ai.goals.has("Stroll"));
}

#[test]
fn random_swim_and_fly_wander_use_the_engines_swimming_and_flying_strolls() {
    let swimmer = mob(
        r#"{"navigator":"swim","goals":[{"type":"random_swim","priority":4,"interval":10}]}"#,
        at(0.5, 0.5),
    );
    assert!(swimmer.ai.goals.has("RandomSwimming"));
    // The flat world has no water to pick: the goal's pick is the engine's, tested there. The
    // flying stroll finds an air spot over the floor.
    let fx = flat();
    let w = fx.world();
    let mut ctx = Ctx::new();
    let mut flyer = mob(
        r#"{"navigator":"fly","goals":[{"type":"fly_wander","priority":4,"interval":1}]}"#,
        Vec3::new(0.5, 3.0, 0.5),
    );
    assert!(flyer.ai.goals.has("RandomStroll"));
    let started = select_until(&mut flyer, &w, &mut ctx, 50, |m| {
        m.ai.goals.is_running("RandomStroll")
    });
    assert!(started.is_some(), "a flying mob picks an air spot");
}

#[test]
fn look_at_player_and_random_look_start_by_chance() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = view(&w0, &others, Some(at(0.5, 4.5)));
    let mut ctx = Ctx::new();
    let mut m = mob(
        &ai(r#"{"type":"look_at_player","priority":7,"range":8},{"type":"random_look","priority":8}"#),
        at(0.5, 0.5),
    );
    assert!(select_until(&mut m, &w, &mut ctx, 600, |m| m.ai.goals.is_running("LookAtPlayer")).is_some());
    let mut m = mob(&ai(r#"{"type":"random_look","priority":8}"#), at(0.5, 0.5));
    assert!(select_until(&mut m, &w, &mut ctx, 600, |m| m.ai.goals.is_running("RandomLookAround")).is_some());
}

#[test]
fn look_at_player_respects_its_range() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = view(&w0, &others, Some(at(0.5, 20.5)));
    let mut ctx = Ctx::new();
    let mut m = mob(&ai(r#"{"type":"look_at_player","priority":7,"range":8}"#), at(0.5, 0.5));
    assert!(select_until(&mut m, &w, &mut ctx, 600, |m| m.ai.goals.is_running("LookAtPlayer")).is_none());
}

#[test]
fn look_at_target_looks_while_the_target_is_near() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let mut ctx = Ctx::new();
    let mut m = mob(&ai(r#"{"type":"look_at_target","priority":6,"range":6}"#), at(0.5, 0.5));
    let near = view(&w0, &others, Some(at(0.5, 4.5)));
    select(&mut m, &near, &mut ctx);
    assert!(!m.ai.goals.is_running("LookAtTarget"), "no target");
    m.ai.target = Some(Who::Player);
    select(&mut m, &near, &mut ctx);
    assert!(m.ai.goals.is_running("LookAtTarget"));
    let far = view(&w0, &others, Some(at(0.5, 30.5)));
    select(&mut m, &far, &mut ctx);
    assert!(!m.ai.goals.is_running("LookAtTarget"), "out of range");
}

#[test]
fn panic_runs_from_a_recent_hit() {
    let fx = flat();
    let w = fx.world();
    let mut ctx = Ctx::new();
    let mut m = mob(&ai(r#"{"type":"panic","priority":1,"speed":1.5}"#), at(0.5, 0.5));
    select(&mut m, &w, &mut ctx);
    assert!(!m.ai.goals.is_running("Panic"));
    assert!(m.damage(1.0, Damage::mob(9, [4.5, 1.0, 0.5])));
    assert!(select_until(&mut m, &w, &mut ctx, 30, |m| m.ai.goals.is_running("Panic")).is_some());
    assert!(m.ai.panicking);
}

#[test]
fn tempt_follows_a_player_holding_one_of_its_items() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let mut w = view(&w0, &others, Some(at(0.5, 5.5)));
    let mut ctx = Ctx::new();
    let mut m = mob(
        &ai(r#"{"type":"tempt","priority":3,"speed":1.1,"items":["minecraft:wheat"]}"#),
        at(0.5, 0.5),
    );
    select(&mut m, &w, &mut ctx);
    assert!(!m.ai.goals.is_running("Tempt"), "empty hand");
    if let Some(p) = w.player.as_mut() {
        p.main_hand = Some("minecraft:wheat".into());
    }
    select(&mut m, &w, &mut ctx);
    assert!(m.ai.goals.is_running("Tempt"));
}

#[test]
fn avoid_keeps_away_from_the_player() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = view(&w0, &others, Some(at(0.5, 3.5)));
    let mut ctx = Ctx::new();
    let mut m = mob(
        &ai(r#"{"type":"avoid","priority":2,"from":["minecraft:player"],"distance":8}"#),
        at(0.5, 0.5),
    );
    let mut seen = false;
    for _ in 0..30 {
        select(&mut m, &w, &mut ctx);
        seen |= m.ai.goals.is_running("AvoidEntity");
    }
    assert!(seen);
    // `flee`, the old name, is the same goal.
    let m = mob(&ai(r#"{"type":"flee","priority":2}"#), at(0.5, 0.5));
    assert!(m.ai.goals.has("AvoidEntity"));
}

#[test]
fn retreat_when_hurt_waits_for_low_health() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = view(&w0, &others, Some(at(0.5, 3.5)));
    let mut ctx = Ctx::new();
    let mut m = mob(
        &ai(r#"{"type":"retreat_when_hurt","priority":1,"health_below":0.3}"#),
        at(0.5, 0.5),
    );
    m.ai.target = Some(Who::Player);
    select(&mut m, &w, &mut ctx);
    assert!(!m.ai.goals.is_running("RetreatWhenHurt"), "healthy");
    m.health = 4.0;
    let mut ran = false;
    for _ in 0..30 {
        select(&mut m, &w, &mut ctx);
        ran |= m.ai.goals.is_running("RetreatWhenHurt");
    }
    assert!(ran, "20 health, 4 left, a target: it runs");
}

#[test]
fn go_to_block_walks_to_a_listed_block() {
    let fx = flat();
    let w = fx.world();
    let mut ctx = Ctx::new();
    let mut m = mob(
        &ai(r#"{"type":"go_to_block","priority":4,"blocks":["minecraft:stone"],"range":4}"#),
        at(0.5, 0.5),
    );
    assert!(m.ai.goals.has("GoToBlock"));
    // A goal made the same way, driven by hand to read where it ends.
    let mut g = super::goals::GoToBlock::new(1.0, vec!["minecraft:stone".into()], 4, 3, 1);
    let mut cx = ctx.cx(&w);
    assert!(g.can_use(&mut m, &mut cx), "stone is under its feet");
    g.start(&mut m, &mut cx);
    g.tick(&mut m, &mut cx);
    assert!(g.reached());
    let mut none = super::goals::GoToBlock::new(1.0, vec!["minecraft:diamond_block".into()], 4, 3, 1);
    assert!(!none.can_use(&mut m, &mut cx), "no such block around");
}

#[test]
fn stay_near_home_walks_back_and_patrol_stays_around() {
    let fx = flat();
    let w = fx.world();
    let mut ctx = Ctx::new();
    let mut m = mob(
        &ai(r#"{"type":"stay_near_home","priority":3,"radius":4}"#),
        at(0.5, 0.5),
    );
    assert_eq!(m.ai.home, Some(([0, 1, 0], 4)));
    select(&mut m, &w, &mut ctx);
    assert!(!m.ai.goals.is_running("StayNearHome"), "at home");
    m.position = at(8.5, 0.5);
    let back = select_until(&mut m, &w, &mut ctx, 10, |m| m.ai.goals.is_running("StayNearHome"));
    assert!(back.is_some(), "far from home: it heads back");

    let mut m = mob(
        &ai(r#"{"type":"patrol_around","priority":5,"radius":4,"interval":1}"#),
        at(0.5, 0.5),
    );
    assert_eq!(m.ai.home, Some(([0, 1, 0], 4)));
    let mut far = 0.0f64;
    for _ in 0..8 {
        run(&mut m, &w, &mut ctx, 50);
        far = far.max((m.position.x - 0.5).hypot(m.position.z - 0.5));
    }
    assert!(far > 0.5, "it patrols");
    assert!(far <= 7.0, "and stays around its post: {far}");
}

#[test]
fn leap_at_target_jumps_at_a_target_in_range() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = view(&w0, &others, Some(at(0.5, 3.5)));
    let mut ctx = Ctx::new();
    let mut m = mob(&ai(r#"{"type":"leap_at_target","priority":3,"yd":0.5}"#), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    m.on_ground = true;
    let jumped = select_until(&mut m, &w, &mut ctx, 60, |m| m.ai.goals.is_running("LeapAtTarget"));
    assert!(jumped.is_some());
    assert_eq!(m.velocity.y, 0.5);
    assert!(m.velocity.z > 0.3, "toward the target");
}

// ---- target selectors ----------------------------------------------------------------------

#[test]
fn target_player_picks_the_player_in_sight() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = view(&w0, &others, Some(at(0.5, 6.5)));
    let mut ctx = Ctx::new();
    let mut m = mob(&ai_targets(r#"{"type":"target_player","priority":2}"#), at(0.5, 0.5));
    assert!(select_until(&mut m, &w, &mut ctx, 100, |m| m.ai.target == Some(Who::Player)).is_some());
}

#[test]
fn nearest_attackable_takes_listed_types_in_range() {
    let fx = flat();
    let w0 = fx.world();
    let mut ctx = Ctx::new();
    let chicken = [seen(2, Species::Chicken, at(3.5, 0.5))];
    let cow = [seen(3, Species::Cow, at(3.5, 0.5))];
    let row = r#"{"type":"nearest_attackable","priority":2,"types":["minecraft:chicken"]}"#;
    let mut m = mob(&ai_targets(row), at(0.5, 0.5));
    let w = view(&w0, &chicken, None);
    assert!(select_until(&mut m, &w, &mut ctx, 100, |m| m.ai.target == Some(Who::Mob(2))).is_some());
    // Another type is not a target.
    let mut m = mob(&ai_targets(row), at(0.5, 0.5));
    let w = view(&w0, &cow, None);
    assert!(select_until(&mut m, &w, &mut ctx, 100, |m| m.ai.target.is_some()).is_none());
    // `range` bounds the search: the chicken is 3 away.
    let near = r#"{"type":"nearest_target","priority":2,"types":["minecraft:chicken"],"range":2}"#;
    let mut m = mob(&ai_targets(near), at(0.5, 0.5));
    let w = view(&w0, &chicken, None);
    assert!(select_until(&mut m, &w, &mut ctx, 100, |m| m.ai.target.is_some()).is_none());
    // Player and mob types together make two selectors.
    let both = r#"{"type":"nearest_attackable","priority":2,"types":["minecraft:player","minecraft:chicken"]}"#;
    let mut m = mob(&ai_targets(both), at(0.5, 0.5));
    let w = view(&w0, &chicken, Some(at(0.5, 5.5)));
    assert!(select_until(&mut m, &w, &mut ctx, 200, |m| m.ai.target.is_some()).is_some());
    // A type the engine cannot tell apart is left out (and logged); the rest still works.
    let odd = r#"{"type":"nearest_attackable","priority":2,"types":["demo:other","minecraft:chicken"]}"#;
    let mut m = mob(&ai_targets(odd), at(0.5, 0.5));
    let w = view(&w0, &chicken, Some(at(0.5, 5.5)));
    assert!(select_until(&mut m, &w, &mut ctx, 100, |m| m.ai.target == Some(Who::Mob(2))).is_some());
}

#[test]
fn hurt_by_takes_the_attacker_and_call_for_help_alerts_its_kind() {
    let fx = flat();
    let w0 = fx.world();
    let mut ctx = Ctx::new();
    let attacker = seen(5, Species::Zombie, at(4.5, 0.5));
    let mut kin = seen(6, Species::Custom, at(2.5, 2.5));
    kin.target = None;
    let others = [attacker, kin];
    let w = view(&w0, &others, None);
    let mut m = mob(&ai_targets(r#"{"type":"hurt_by","priority":1}"#), at(0.5, 0.5));
    select(&mut m, &w, &mut ctx);
    assert_eq!(m.ai.target, None);
    m.ai.last_hurt_by = Some(Who::Mob(5));
    m.ai.last_hurt_by_time = 10;
    select(&mut m, &w, &mut ctx);
    assert_eq!(m.ai.target, Some(Who::Mob(5)));
    assert_eq!(count(&ctx.events, |e| matches!(e, MobEvent::SetTarget { .. })), 0, "no alert");

    let mut m = mob(&ai_targets(r#"{"type":"call_for_help","priority":1}"#), at(0.5, 0.5));
    m.ai.last_hurt_by = Some(Who::Mob(5));
    m.ai.last_hurt_by_time = 10;
    select(&mut m, &w, &mut ctx);
    assert_eq!(m.ai.target, Some(Who::Mob(5)));
    assert!(any(&ctx.events, |e| matches!(e, MobEvent::SetTarget { mob: 6, target: Who::Mob(5) })));
    // `ignore` names attackers the selector leaves alone.
    let mut m = mob(
        &ai_targets(r#"{"type":"hurt_by","priority":1,"ignore":["minecraft:zombie"]}"#),
        at(0.5, 0.5),
    );
    m.ai.last_hurt_by = Some(Who::Mob(5));
    m.ai.last_hurt_by_time = 10;
    select(&mut m, &w, &mut ctx);
    assert_eq!(m.ai.target, None);
}

// ---- control flags -------------------------------------------------------------------------

#[test]
fn a_lower_priority_number_takes_over_a_flag_and_equal_ones_do_not() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let mut w = view(&w0, &others, Some(at(0.5, 1.5)));
    let mut ctx = Ctx::new();
    // The stroll (MOVE, 5) runs; a target makes the attack (MOVE and LOOK, 3) replace it.
    let goals = ai(r#"{"type":"wander","priority":5,"interval":1},
                     {"type":"melee_attack","priority":3},
                     {"type":"look_at_target","priority":6}"#);
    let mut m = mob(&goals, at(0.5, 0.5));
    m.on_ground = true;
    assert!(select_until(&mut m, &w, &mut ctx, 50, |m| m.ai.goals.is_running("Stroll")).is_some());
    assert!(!m.ai.goals.is_running("CustomMelee"));
    m.ai.target = Some(Who::Player);
    // (The engine's attack goal looks for a path once every 20 ticks.)
    w.game_time += 20;
    select(&mut m, &w, &mut ctx);
    assert!(m.ai.goals.is_running("CustomMelee"), "{:?}", m.ai.goals.running_names());
    assert!(!m.ai.goals.is_running("Stroll"), "MOVE was taken");
    // LOOK is held by the attack too, so the look goal (6) cannot start.
    assert!(!m.ai.goals.is_running("LookAtTarget"));

    // Equal priority never replaces: the first to run keeps MOVE.
    let goals = ai(r#"{"type":"wander","priority":4,"interval":1},{"type":"melee_attack","priority":4}"#);
    let mut m = mob(&goals, at(0.5, 0.5));
    m.on_ground = true;
    assert!(select_until(&mut m, &w, &mut ctx, 50, |m| m.ai.goals.is_running("Stroll")).is_some());
    m.ai.target = Some(Who::Player);
    w.game_time += 20;
    select(&mut m, &w, &mut ctx);
    assert!(m.ai.goals.is_running("Stroll") && !m.ai.goals.is_running("CustomMelee"));
}

#[test]
fn goals_with_disjoint_flags_run_together() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = view(&w0, &others, Some(at(0.5, 3.5)));
    let mut ctx = Ctx::new();
    let goals = ai(r#"{"type":"wander","priority":5,"interval":1},
                     {"type":"look_at_target","priority":6,"range":8}"#);
    let mut m = mob(&goals, at(0.5, 0.5));
    m.on_ground = true;
    m.ai.target = Some(Who::Player);
    let both = select_until(&mut m, &w, &mut ctx, 50, |m| {
        m.ai.goals.is_running("Stroll") && m.ai.goals.is_running("LookAtTarget")
    });
    assert!(both.is_some());
}

#[test]
fn float_takes_the_jump_flag_from_a_leap() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = view(&w0, &others, Some(at(0.5, 3.5)));
    let mut ctx = Ctx::new();
    let goals = ai(r#"{"type":"swim","priority":0},{"type":"leap_attack","priority":3,"windup":20}"#);
    let mut m = mob(&goals, at(0.5, 0.5));
    m.on_ground = true;
    m.ai.target = Some(Who::Player);
    assert!(select_until(&mut m, &w, &mut ctx, 60, |m| m.ai.goals.is_running("LeapAttack")).is_some());
    m.fluids.in_lava = true;
    select(&mut m, &w, &mut ctx);
    assert!(m.ai.goals.is_running("Float"));
    assert!(!m.ai.goals.is_running("LeapAttack"), "JUMP went to the lower number");
}

#[test]
fn a_target_selector_with_a_lower_number_replaces_a_running_one() {
    let fx = flat();
    let w0 = fx.world();
    let mut ctx = Ctx::new();
    let others = [seen(5, Species::Zombie, at(4.5, 0.5))];
    let w = view(&w0, &others, Some(at(0.5, 5.5)));
    let targets = ai_targets(r#"{"type":"hurt_by","priority":1},{"type":"target_player","priority":3}"#);
    let mut m = mob(&targets, at(0.5, 0.5));
    assert!(select_until(&mut m, &w, &mut ctx, 100, |m| m.ai.target == Some(Who::Player)).is_some());
    m.ai.last_hurt_by = Some(Who::Mob(5));
    m.ai.last_hurt_by_time = 10;
    select(&mut m, &w, &mut ctx);
    assert_eq!(m.ai.target, Some(Who::Mob(5)), "the revenge selector (1) takes TARGET from the player's (3)");
}

// ---- navigators ----------------------------------------------------------------------------

#[test]
fn each_navigator_picks_the_engines_navigation_and_controls() {
    let nav = |name: &str| {
        mob(&format!(r#"{{"navigator":"{name}"}}"#), at(0.5, 0.5))
    };
    let m = nav("ground");
    assert_eq!(m.ai.nav.kind, NavKind::Ground);
    assert_eq!(m.ai.mov.kind, MoveKind::Ground);
    let m = nav("climb");
    assert_eq!(m.ai.nav.kind, NavKind::WallClimber);
    assert_eq!(m.ai.mov.kind, MoveKind::Ground);
    let m = nav("swim");
    assert_eq!(m.ai.nav.kind, NavKind::Water { breach: false });
    assert!(matches!(m.ai.mov.kind, MoveKind::SmoothSwimming { gravity: true, .. }));
    let m = nav("fly");
    assert_eq!(m.ai.nav.kind, NavKind::Flying);
    assert!(matches!(m.ai.mov.kind, MoveKind::Flying { hovers: false, .. }));
    assert!(m.ai.nav.can_float && !m.ai.nav.can_open_doors);
    let m = nav("amphibious");
    assert_eq!(m.ai.nav.kind, NavKind::Amphibious { prefers_shallow: false });
    assert!(matches!(m.ai.mov.kind, MoveKind::SmoothSwimming { gravity: false, .. }));
}

#[test]
fn a_path_is_found_for_the_ground_and_the_flying_navigator() {
    let fx = flat();
    let w = fx.world();
    for (nav, from) in [("ground", at(0.5, 0.5)), ("fly", Vec3::new(0.5, 3.0, 0.5))] {
        let mut m = mob(&format!(r#"{{"navigator":"{nav}"}}"#), from);
        m.on_ground = nav == "ground";
        let path = m.create_path_to(&w, [8, 1, 0], 0);
        assert!(path.is_some(), "{nav}: a path over the flat floor");
    }
}

// ---- the path budget -----------------------------------------------------------------------

#[test]
fn a_custom_mobs_path_search_is_capped_and_a_vanilla_mobs_is_not() {
    // A follow range of 128 is 2048 nodes in the engine; a custom mob may visit 1024.
    let json = r#"{"template":"yoake:mob","size":{"width":0.6,"height":1.0},
        "attributes":{"follow_range":128},"ai":{}}"#;
    let file = horizon_mods::custom_mob::CustomMobFile::parse("demo:far", json.as_bytes()).unwrap();
    let t = std::sync::Arc::new(super::super::CustomType::new(file, None).unwrap());
    let kind = std::sync::Arc::new(super::super::build::mob_type(&t));
    let m = crate::mobs::Mob::new(1, kind, at(0.5, 0.5), 0.0);
    assert_eq!(m.ai.nav.max_visited_nodes, 2048);
    assert_eq!(nav_select::max_visited_nodes(&m, 2048), nav_select::MAX_VISITED_NODES);
    assert_eq!(nav_select::max_visited_nodes(&m, 256), 256, "a small range is untouched");
    let cow = crate::mobs::Mob::new(2, crate::mobs::sensor_scan_tests::kind(), at(0.5, 0.5), 0.0);
    assert_eq!(nav_select::max_visited_nodes(&cow, 4096), 4096, "vanilla mobs are not capped");
}

#[test]
fn a_search_to_an_unreachable_target_gives_up_in_bounded_work() {
    let fx = flat();
    let w = fx.world();
    // 200 blocks away: beyond the follow range, so the search is refused or cut short, never
    // an unbounded walk across the 81 by 81 floor.
    let mut m = mob("{}", at(0.5, 0.5));
    m.on_ground = true;
    let started = std::time::Instant::now();
    for _ in 0..50 {
        m.ai.nav.path = None;
        m.ai.nav.target_pos = None;
        let _ = m.create_path_to(&w, [200, 1, 0], 0);
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
}

// ---- attacks -------------------------------------------------------------------------------

fn player_view<'a>(w0: &crate::mobs::MobWorld<'a>, others: &'a [crate::mobs::Seen], at: Vec3) -> crate::mobs::MobWorld<'a> {
    view(w0, others, Some(at))
}

#[test]
fn a_plain_melee_attack_is_the_engines_goal_tick_for_tick() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = player_view(&w0, &others, at(0.5, 1.4));
    let mut a = mob(&ai(r#"{"type":"melee_attack","priority":3,"speed":1.0}"#), at(0.5, 0.5));
    let mut b = mob("{}", at(0.5, 0.5));
    b.ai.goals.add(3, MeleeAttack::new(1.0, false));
    let (mut ca, mut cb) = (Ctx::new(), Ctx::new());
    a.ai.target = Some(Who::Player);
    b.ai.target = Some(Who::Player);
    run(&mut a, &w, &mut ca, 90);
    run(&mut b, &w, &mut cb, 90);
    assert!(hits(&ca.events) >= 3, "it attacks: {}", hits(&ca.events));
    assert_eq!(hits(&ca.events), hits(&cb.events));
    assert_eq!(ca.events.len(), cb.events.len());
    assert_eq!((a.position.x, a.position.z), (b.position.x, b.position.z));
}

#[test]
fn a_melee_attack_waits_for_its_damage_frame_and_asks_for_the_trigger() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = player_view(&w0, &others, at(0.5, 1.4));
    let row = r#"{"type":"melee_attack","priority":3,"trigger":"bite","damage_frame":"hit","damage_delay":40}"#;
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    let mut ctx = Ctx::new();
    run(&mut m, &w, &mut ctx, 6);
    assert_eq!(hits(&ctx.events), 0, "the blow waits for the frame");
    assert_eq!(attacks::take_triggers(&mut m), ["bite"], "the animation was asked for once");
    assert!(attacks::take_triggers(&mut m).is_empty());
    assert!(attacks::deliver_frame(&mut m, "hit"));
    run(&mut m, &w, &mut ctx, 1);
    assert_eq!(hits(&ctx.events), 1, "the frame lands the blow");
    // Another event's name is not the frame.
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    let mut ctx = Ctx::new();
    run(&mut m, &w, &mut ctx, 4);
    attacks::deliver_frame(&mut m, "step");
    run(&mut m, &w, &mut ctx, 4);
    assert_eq!(hits(&ctx.events), 0);
}

#[test]
fn a_damage_frame_that_never_comes_falls_back_to_the_delay() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = player_view(&w0, &others, at(0.5, 1.4));
    let row = r#"{"type":"melee_attack","priority":3,"damage_frame":"hit","damage_delay":12}"#;
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    let mut ctx = Ctx::new();
    run(&mut m, &w, &mut ctx, 8);
    assert_eq!(hits(&ctx.events), 0);
    run(&mut m, &w, &mut ctx, 10);
    assert_eq!(hits(&ctx.events), 1, "no server animation: the fallback offset lands it");
    // Without a delay of its own a named frame waits the default number of ticks.
    let row = r#"{"type":"melee_attack","priority":3,"damage_frame":"hit"}"#;
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    let mut ctx = Ctx::new();
    run(&mut m, &w, &mut ctx, attacks::DEFAULT_FRAME_DELAY as usize - 3);
    assert_eq!(hits(&ctx.events), 0);
    run(&mut m, &w, &mut ctx, 8);
    assert_eq!(hits(&ctx.events), 1);
}

#[test]
fn the_blow_needs_the_target_still_in_reach_when_it_lands() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let near = player_view(&w0, &others, at(0.5, 1.4));
    let row = r#"{"type":"melee_attack","priority":3,"damage_frame":"hit","damage_delay":30}"#;
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    let mut ctx = Ctx::new();
    run(&mut m, &near, &mut ctx, 4);
    attacks::deliver_frame(&mut m, "hit");
    // The target stepped away before the frame.
    let gone = player_view(&w0, &others, at(0.5, 9.5));
    run(&mut m, &gone, &mut ctx, 1);
    assert_eq!(hits(&ctx.events), 0, "a miss");
}

#[test]
fn a_part_that_sticks_out_lets_a_melee_attack_reach_further() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = player_view(&w0, &others, at(0.5, 2.5));
    let neck = r#","hitboxes":[{"name":"head","size":[0.4,0.4],"offset":[0,0.2,1.5]}]"#;
    let goal = ai(r#"{"type":"melee_attack","priority":3}"#);
    let mut with = mob_with("creature", &goal, neck, at(0.5, 0.5));
    let mut without = mob(&goal, at(0.5, 0.5));
    for m in [&mut with, &mut without] {
        m.ai.target = Some(Who::Player);
        m.on_ground = true;
    }
    let mut ctx = Ctx::new();
    let mut strikes = |m: &mut crate::mobs::Mob| {
        let mut g = ChaseStrike::new(Strike::Melee, 1.0, false, 20, Timing::default());
        let mut cx = ctx.cx(&w);
        assert!(g.can_use(m, &mut cx));
        g.start(m, &mut cx);
        g.tick(m, &mut cx);
        drop(cx);
        hits(&ctx.events)
    };
    assert_eq!(strikes(&mut without), 0, "the box alone does not reach");
    assert_eq!(strikes(&mut with), 1, "the head does");
}

#[test]
fn area_attack_hits_what_is_in_its_shape() {
    let fx = flat();
    let w0 = fx.world();
    let others = [
        seen(2, Species::Chicken, at(0.5, 2.0)),
        seen(3, Species::Cow, at(2.0, 0.5)),
        seen(4, Species::Chicken, at(0.5, 9.0)),
    ];
    let w = player_view(&w0, &others, at(1.5, 0.5));
    let row = r#"{"type":"area_attack","priority":3,"radius":3,"damage":5,"knockback":0.4,
                  "types":["minecraft:player","minecraft:chicken"]}"#;
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    m.on_ground = true;
    let mut ctx = Ctx::new();
    run(&mut m, &w, &mut ctx, 3);
    assert_eq!(count(&ctx.events, |e| matches!(e, MobEvent::AttackPlayer(a, _) if *a == 5.0)), 1);
    assert!(any(&ctx.events, |e| matches!(e, MobEvent::AttackMob { target: 2, amount, .. } if *amount == 5.0)));
    assert!(!any(&ctx.events, |e| matches!(e, MobEvent::AttackMob { target: 3, .. })), "a cow is not listed");
    assert!(!any(&ctx.events, |e| matches!(e, MobEvent::AttackMob { target: 4, .. })), "out of radius");
    assert_eq!(count(&ctx.events, |e| matches!(e, MobEvent::Knockback { .. })), 2);
}

#[test]
fn a_cone_area_attack_only_hits_in_front() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    // At yaw 0 the mob faces +z.
    for (z, hit) in [(2.0, true), (-2.0, false)] {
        let w = player_view(&w0, &others, at(0.5, 0.5 + z));
        let mut m = mob("{}", at(0.5, 0.5));
        m.ai.target = Some(Who::Player);
        m.on_ground = true;
        let mut ctx = Ctx::new();
        let area = attacks::Area {
            radius: 3.0,
            range: 3.0,
            damage: 4.0,
            knockback: 0.0,
            cone: Some(90.0),
            species: vec![],
            player: true,
        };
        let mut g = ChaseStrike::new(Strike::Area(area), 1.0, false, 40, Timing::default());
        let mut cx = ctx.cx(&w);
        assert!(g.can_use(&mut m, &mut cx));
        g.start(&mut m, &mut cx);
        g.tick(&mut m, &mut cx);
        drop(cx);
        assert_eq!(hits(&ctx.events), usize::from(hit), "z {z}");
    }
}

fn shots(events: &[MobEvent]) -> Vec<&crate::projectiles::Projectile> {
    events
        .iter()
        .filter_map(|e| match e {
            MobEvent::Projectile(p) => Some(p),
            _ => None,
        })
        .collect()
}

#[test]
fn ranged_attack_shoots_its_projectile_at_the_target() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = player_view(&w0, &others, at(0.5, 9.5));
    for (name, check) in [
        ("arrow", 2.5),
        ("snowball", 0.0),
        ("small_fireball", 0.0),
    ] {
        let row = format!(
            r#"{{"type":"ranged_attack","priority":3,"projectile":"{name}","damage":{check},"interval_min":10,"interval_max":20,"radius":12}}"#
        );
        let mut m = mob(&ai(&row), at(0.5, 0.5));
        m.ai.target = Some(Who::Player);
        let mut ctx = Ctx::new();
        run(&mut m, &w, &mut ctx, 60);
        let shot = shots(&ctx.events);
        assert!(!shot.is_empty(), "{name}: it shot");
        assert!(shot[0].velocity.z > 0.0, "{name}: toward the player");
        assert_eq!(shot[0].owner, Some(Who::Mob(1)));
        if name == "arrow" {
            assert!(matches!(
                shot[0].kind,
                crate::projectiles::ProjectileKind::Arrow { base_damage, .. } if base_damage == 2.5
            ));
        }
    }
    // An unknown projectile leaves the goal out.
    let m = mob(&ai(r#"{"type":"ranged_attack","priority":3,"projectile":"meteor"}"#), at(0.5, 0.5));
    assert!(!m.ai.goals.has("CustomRanged"));
}

#[test]
fn a_ranged_attack_shoots_at_its_damage_frame() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = player_view(&w0, &others, at(0.5, 9.5));
    let row = r#"{"type":"ranged_attack","priority":3,"projectile":"snowball","trigger":"spit",
                  "damage_frame":"release","damage_delay":200,"interval_min":10,"interval_max":20,"radius":12}"#;
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    let mut ctx = Ctx::new();
    run(&mut m, &w, &mut ctx, 50);
    assert!(shots(&ctx.events).is_empty(), "nothing flies before the frame");
    assert_eq!(attacks::take_triggers(&mut m), ["spit"]);
    attacks::deliver_frame(&mut m, "release");
    run(&mut m, &w, &mut ctx, 1);
    assert_eq!(shots(&ctx.events).len(), 1);
}

#[test]
fn leap_attack_winds_up_jumps_and_strikes_once() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = player_view(&w0, &others, at(0.5, 3.0));
    let row = r#"{"type":"leap_attack","priority":3,"windup":4,"impulse":0.5,"forward":0.4,
                  "min_range":1,"max_range":6,"trigger":"pounce"}"#;
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    m.on_ground = true;
    let mut ctx = Ctx::new();
    assert!(select_until(&mut m, &w, &mut ctx, 60, |m| m.ai.goals.is_running("LeapAttack")).is_some());
    assert_eq!(m.velocity.y, 0.0, "winding up");
    assert_eq!(attacks::take_triggers(&mut m), ["pounce"]);
    for _ in 0..4 {
        select(&mut m, &w, &mut ctx);
    }
    assert_eq!(m.velocity.y, 0.5, "the jump");
    assert!(m.velocity.z > 0.3);
    // In the air and in reach of the player: one blow, once.
    m.on_ground = false;
    m.position = at(0.5, 2.4);
    for _ in 0..6 {
        select(&mut m, &w, &mut ctx);
    }
    assert_eq!(hits(&ctx.events), 1);
    // Landed: the goal ends and the cooldown holds it back.
    m.on_ground = true;
    select(&mut m, &w, &mut ctx);
    select(&mut m, &w, &mut ctx);
    assert!(!m.ai.goals.is_running("LeapAttack"));
    assert!(select_until(&mut m, &w, &mut ctx, 20, |m| m.ai.goals.is_running("LeapAttack")).is_none());
}

#[test]
fn leap_attack_strikes_at_its_damage_frame() {
    let fx = flat();
    let w0 = fx.world();
    let others = [];
    let w = player_view(&w0, &others, at(0.5, 2.0));
    let row = r#"{"type":"leap_attack","priority":3,"windup":0,"min_range":0,"max_range":6,
                  "damage_frame":"land","damage_delay":100}"#;
    let mut m = mob(&ai(row), at(0.5, 0.5));
    m.ai.target = Some(Who::Player);
    m.on_ground = true;
    let mut ctx = Ctx::new();
    assert!(select_until(&mut m, &w, &mut ctx, 60, |m| m.ai.goals.is_running("LeapAttack")).is_some());
    m.on_ground = false;
    m.position = at(0.5, 1.6);
    for _ in 0..5 {
        select(&mut m, &w, &mut ctx);
    }
    assert_eq!(hits(&ctx.events), 0, "in reach, but the frame has not come");
    attacks::deliver_frame(&mut m, "land");
    select(&mut m, &w, &mut ctx);
    assert_eq!(hits(&ctx.events), 1);
}

#[test]
fn only_a_custom_mob_takes_frames() {
    let mut cow = crate::mobs::Mob::new(2, crate::mobs::sensor_scan_tests::kind(), at(0.5, 0.5), 0.0);
    assert!(!attacks::deliver_frame(&mut cow, "hit"));
    assert!(attacks::take_triggers(&mut cow).is_empty());
}

// ---- climb ---------------------------------------------------------------------------------

/// A wall of stone along `x = 2`, `z` from -6 to 6 and three high, on the flat world.
fn walled() -> crate::mobs::mob_hook_bench::BenchWorld {
    let mut fx = flat();
    for z in -6..=6 {
        for y in 1..=3 {
            fx.set_stone(2, y, z);
        }
    }
    fx
}

#[test]
fn a_climb_row_gives_the_engines_climber_physics_and_a_path_cost() {
    let m = mob(&ai(r#"{"type":"climb","priority":2,"cost":1.5}"#), at(0.5, 0.5));
    assert!(m.kind.flags.climber, "the spider's wall climbing");
    assert_eq!(m.ai.nav.climb_cost, Some(1.5));
    assert!(m.path_mob().climb_cost.is_some());
    let plain = mob("{}", at(0.5, 0.5));
    assert!(!plain.kind.flags.climber);
    assert_eq!(plain.ai.nav.climb_cost, None);
    let default = mob(&ai(r#"{"type":"climb"}"#), at(0.5, 0.5));
    assert_eq!(default.ai.nav.climb_cost, Some(0.5));
}

#[test]
fn the_navigator_routes_up_a_wall_only_for_a_mob_with_a_climb_row() {
    let fx = walled();
    let w = fx.world();
    let mut climber = mob(&ai(r#"{"type":"climb","cost":0.2}"#), at(0.5, 0.5));
    let path = climber.create_path_to(&w, [2, 4, 0], 0).expect("a path");
    assert!(path.reached, "over the wall: {:?}", path.nodes);
    assert!(path.nodes.iter().any(|n| n.y >= 4), "it goes up the face");
    let mut walker = mob("{}", at(0.5, 0.5));
    let path = walker.create_path_to(&w, [2, 4, 0], 0);
    assert!(!path.is_some_and(|p| p.reached), "a plain ground mob cannot get onto the wall");
}

#[test]
fn the_climb_cost_decides_between_the_wall_and_the_way_round() {
    let fx = walled();
    let w = fx.world();
    let top = |m: &mut crate::mobs::Mob| {
        m.create_path_to(&w, [4, 1, 0], 0)
            .map(|p| (p.reached, p.nodes.iter().map(|n| n.y).max().unwrap_or(0)))
    };
    let mut cheap = mob(&ai(r#"{"type":"climb","cost":0.1}"#), at(0.5, 0.5));
    cheap.attrs.follow_range = 32.0;
    let (reached, highest) = top(&mut cheap).expect("a path");
    assert!(reached && highest >= 4, "a cheap wall is climbed (highest {highest})");
    let mut dear = mob(&ai(r#"{"type":"climb","cost":50}"#), at(0.5, 0.5));
    dear.attrs.follow_range = 32.0;
    let (reached, highest) = top(&mut dear).expect("a path");
    assert!(reached && highest == 1, "a dear wall is walked round (highest {highest})");
}

#[test]
fn a_climber_gets_up_the_wall_by_pressing_against_it() {
    let fx = walled();
    let w = fx.world();
    let mut ctx = Ctx::new();
    let mut m = mob(&ai(r#"{"type":"climb","cost":0.2}"#), at(0.5, 0.5));
    assert!(m.nav_move_to(&w, [2.5, 4.0, 0.5], 1.0), "a path to the top of the wall");
    run(&mut m, &w, &mut ctx, 200);
    assert!(
        m.position.y >= 3.9 && m.position.x > 1.9,
        "on top of the wall, at {:?}",
        m.position
    );
}
