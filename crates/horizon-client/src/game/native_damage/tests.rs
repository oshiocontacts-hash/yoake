use std::sync::Arc;

use horizon_client::mobs::{Anim, Brain, Mob, MobType, Part, Species};

use super::*;

pub(in crate::game) mod server;

fn tick(sequence: u64) -> (NativeWorldEpoch, NativeTickId) {
    (NativeWorldEpoch::new(1), NativeTickId::new(sequence))
}

fn mob(armor: f32) -> Mob {
    let kind = Arc::new(MobType {
        custom: None,
        id: "minecraft:pig".into(),
        species: Species::Pig,
        width: 0.9,
        height: 0.9,
        eye_height: 0.85,
        category: "creature".into(),
        max_health: 40.0,
        speed: 0.25,
        flying_speed: 0.02,
        attack_damage: 0.0,
        attack_knockback: 0.0,
        follow_range: 16.0,
        step_height: 0.6,
        armor,
        armor_toughness: 0.0,
        knockback_resistance: 0.0,
        jump_strength: 0.42,
        attributes: Default::default(),
        gravity: 0.08,
        safe_fall_distance: 3.0,
        fall_damage_multiplier: 1.0,
        water_efficiency: 0.0,
        water_slowdown: 0.8,
        air_speed: 0.02,
        baby_scale: 0.5,
        head_rot_speed: 10.0,
        max_head_x_rot: 40.0,
        max_head_y_rot: 75.0,
        ambient_interval: 80,
        flags: Default::default(),
        model: Arc::new(Part {
            name: "root".into(),
            pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
            polygons: Vec::new(),
            children: Vec::new(),
        }),
        layers: Default::default(),
        texture: String::new(),
        brain: Brain::Passive,
        anim: Anim::Quadruped,
        sounds: ["", "", ""],
        step_sound: None,
        food: Default::default(),
        tempt: Default::default(),
    });
    Mob::new(71, kind, Vec3::ZERO, 0.0)
}

fn source(kind: NativeDamageKind) -> Damage {
    Damage {
        kind: kind.mob_kind(),
        attacker: Some(Who::Player),
        from: Some([1.0, 0.0, 0.0]),
    }
}

fn close(actual: f32, expected: f32) {
    assert!((actual - expected).abs() < 0.0001, "{actual} != {expected}");
}

#[test]
fn mob_armor_and_magic_use_the_vanilla_reductions() {
    let mut physical = mob(20.0);
    let mut magic = mob(20.0);
    assert!(physical.damage_with(10.0, source(NativeDamageKind::PlayerAttack), None));
    assert!(magic.damage_with(10.0, source(NativeDamageKind::Magic), None));
    close(physical.health, 36.0);
    close(magic.health, 30.0);
}

#[test]
fn mob_window_refuses_equal_hits_and_accepts_only_the_stronger_difference() {
    let mut victim = mob(0.0);
    let damage = source(NativeDamageKind::Magic);
    assert!(victim.damage_with(4.0, damage, None));
    close(victim.health, 36.0);
    assert!(!victim.damage_with(4.0, damage, None));
    close(victim.health, 36.0);
    assert!(victim.damage_with(8.0, damage, None));
    close(victim.health, 32.0);
    victim.invulnerable = 0;
    assert!(victim.damage_with(4.0, damage, None));
    close(victim.health, 28.0);
}

#[test]
fn mob_resistance_and_absorption_are_not_generic_health_subtraction() {
    let mut resistant = mob(0.0);
    resistant
        .extra
        .effects
        .push(("minecraft:resistance".into(), 100, 1));
    assert!(resistant.damage_with(10.0, source(NativeDamageKind::Magic), None));
    close(resistant.health, 34.0);

    let mut absorbed = mob(0.0);
    absorbed.absorption = 10.0;
    assert!(absorbed.damage_with(4.0, source(NativeDamageKind::Magic), None));
    close(absorbed.health, 40.0);
    close(absorbed.absorption, 6.0);
}

#[test]
fn lethal_mob_hit_retains_player_damage_and_recent_kill_credit() {
    let mut victim = mob(0.0);
    assert!(victim.damage_with(100.0, source(NativeDamageKind::PlayerAttack), None));
    assert!(victim.health <= 0.0);
    assert_eq!(victim.last_damage.unwrap().0.attacker, Some(Who::Player));
    assert_eq!(victim.ai.last_hurt_by_player, 100);
}

#[test]
fn mapped_mob_delivery_matches_unmodified_damage_entry_point() {
    for kind in [
        NativeDamageKind::Magic,
        NativeDamageKind::PlayerAttack,
        NativeDamageKind::Explosion,
    ] {
        let mut native = mob(12.0);
        let mut vanilla = mob(12.0);
        native.absorption = 3.0;
        vanilla.absorption = 3.0;
        for amount in [4.0, 4.0, 9.0, 200.0] {
            let damage = source(kind);
            assert_eq!(
                native.damage_with(amount, damage, None),
                vanilla.damage(amount, damage)
            );
            close(native.health, vanilla.health);
            close(native.absorption, vanilla.absorption);
            close(native.last_hurt, vanilla.last_hurt);
            assert_eq!(native.invulnerable, vanilla.invulnerable);
            assert_eq!(
                native.ai.last_hurt_by_player,
                vanilla.ai.last_hurt_by_player
            );
        }
    }
}

#[test]
fn damage_constructor_and_explosion_types_preserve_attribution() {
    let damage = NativeDamage::new(
        NativeDamageKind::Explosion,
        NativeDamageActor::Player(0x1234),
        Some([1.0, 2.0, 3.0]),
    );
    assert_eq!(damage.actor, NativeDamageActor::Player(0x1234));
    assert_eq!(damage.position, Some([1.0, 2.0, 3.0]));
    assert_eq!(damage.kind.damage_type(false), "minecraft:explosion");
    assert_eq!(damage.kind.damage_type(true), "minecraft:player_explosion");
    assert_eq!(
        NativeDamageKind::PlayerAttack.damage_type(true),
        "minecraft:player_attack"
    );
}

#[test]
fn shared_operation_limit_refuses_without_an_extra_impulse() {
    let mut budgets = DamageBudgets::default();
    let mut velocity = 0.0;
    for _ in 0..MAX_OPERATIONS {
        budgets.reserve(tick(1), 2, false).unwrap();
        velocity += 0.25;
    }
    let result = budgets.reserve(tick(1), 2, false).map(|()| {
        velocity += 0.25;
    });
    assert!(result.is_err());
    assert_eq!(velocity, 64.0);
    budgets.reserve(tick(1), 3, false).unwrap();
    budgets.reserve(tick(2), 2, false).unwrap();
    budgets
        .reserve((NativeWorldEpoch::new(2), NativeTickId::new(1)), 2, false)
        .unwrap();
}

#[test]
fn explosion_limit_is_separate_and_resets_lazily() {
    let mut budgets = DamageBudgets::default();
    let mut damaged_blocks = Vec::new();
    for block in 0..MAX_EXPLOSIONS {
        budgets.reserve(tick(1), 0, true).unwrap();
        damaged_blocks.push(block);
    }
    let result = budgets.reserve(tick(1), 0, true).map(|()| {
        damaged_blocks.push(99);
    });
    assert!(result.is_err());
    assert_eq!(damaged_blocks, [0, 1, 2, 3]);
    budgets.reserve(tick(1), 0, false).unwrap();
    budgets.reserve(tick(1), 1, true).unwrap();
    budgets.reserve(tick(2), 0, true).unwrap();
}

#[test]
fn nested_depth_refuses_before_delivery_and_unwinds() {
    let mut guards = Vec::new();
    let mut health = 20;
    for _ in 0..MAX_DELIVERY_DEPTH {
        guards.push(DeliveryGuard::enter().unwrap());
        health -= 1;
    }
    let result = DeliveryGuard::enter().map(|_guard| {
        health -= 1;
    });
    assert!(result.is_err());
    assert_eq!(health, 16);
    drop(guards);
    let _guard = DeliveryGuard::enter().unwrap();
    assert_eq!(DELIVERY_DEPTH.with(Cell::get), 1);
}
