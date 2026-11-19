use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicUsize};

use super::*;
use crate::game::FallingBlock;
use crate::game::boats::Boat;
use crate::game::fireworks::Rocket;
use crate::game::minecarts::{CartKind, Minecart};
use crate::game::tnt::PrimedTnt;
use crate::game::trident::ThrownTrident;
use horizon_client::entities::{ArmorStand, ExperienceOrb, ItemEntity};
use horizon_client::items::ItemStack;
use horizon_client::lan::{Joined, PlayerSender};
use horizon_client::mobs::{Anim, Brain, Mob, MobType, Part, Species};
use horizon_client::projectiles::{Projectile, ProjectileKind};
use horizon_core::position::BlockPos;
use horizon_protocol::packets::configuration::ClientInformation;
use horizon_world::registry::BlockStateId;

fn mob(id: u32) -> Mob {
    let kind = Arc::new(MobType {
        custom: None,
        id: "minecraft:pig".into(),
        species: Species::Pig,
        width: 0.9,
        height: 0.9,
        eye_height: 0.85,
        category: "creature".into(),
        max_health: 10.0,
        speed: 0.25,
        flying_speed: 0.02,
        attack_damage: 0.0,
        attack_knockback: 0.0,
        follow_range: 16.0,
        step_height: 0.6,
        armor: 0.0,
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
    Mob::new(id, kind, Vec3::ZERO, 0.0)
}

fn guest(id: i32) -> Guest {
    let (sender, _receiver) = std::sync::mpsc::channel();
    let mut guest = Guest::new(
        Joined {
            conn: 0,
            uuid: 0x1234,
            name: "query-test".into(),
            properties: Vec::new(),
            info: ClientInformation::new(8),
            brand: String::new(),
            sender: PlayerSender::new(sender, Arc::new(AtomicUsize::new(0))),
            latency: Arc::new(AtomicI32::new(0)),
            address: "127.0.0.1:1".parse().unwrap(),
        },
        id,
        [0.0; 3],
        10,
    );
    guest.placed = true;
    guest.dimension = OVERWORLD.into();
    guest
}

#[test]
fn every_storage_family_is_enumerated_with_its_box_policy() {
    let mobs = [mob(1)];
    let dropped = [ItemEntity {
        join: Default::default(),
        health: 5,
        fire_ticks: 0,
        id: 2,
        stack: ItemStack::new("minecraft:stone", 1),
        thrower: None,
        target: None,
        position: Vec3::ZERO,
        previous: Vec3::ZERO,
        velocity: Vec3::ZERO,
        on_ground: false,
        age: 0,
        pickup_delay: 0,
        bob_offset: 0.0,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
        no_gravity: false,
        vertical_collision: false,
        horizontal_collision: false,
    }];
    let boats = [Boat::new(3, "oak".into(), Vec3::ZERO, 0.0)];
    let carts = [Minecart::new(4, CartKind::Rideable, Vec3::ZERO)];
    let armor_stands = [ArmorStand::new(5, Vec3::ZERO, 0.0)];
    let tnts = [PrimedTnt {
        join: Default::default(),
        id: 6,
        position: Vec3::ZERO,
        previous: Vec3::ZERO,
        velocity: Vec3::ZERO,
        fuse: 80,
        on_ground: false,
        owner: None,
        guest: None,
        no_gravity: false,
        power: 4.0,
    }];
    let falling = [FallingBlock {
        join: Default::default(),
        id: 7,
        state: BlockStateId(0),
        start_pos: BlockPos::new(0, 0, 0),
        position: Vec3::ZERO,
        previous: Vec3::ZERO,
        velocity: Vec3::ZERO,
        age: 0,
        no_gravity: false,
        portal: Default::default(),
        drop_item: true,
        fall_distance: 0.0,
        stuck: Vec3::ZERO,
        damage: Default::default(),
        saved: None,
    }];
    let orbs = [ExperienceOrb {
        join: Default::default(),
        id: 8,
        position: Vec3::ZERO,
        previous: Vec3::ZERO,
        velocity: Vec3::ZERO,
        value: 1,
        age: 0,
        on_ground: false,
        count: 1,
        ticks: 0,
        health: 5,
        no_gravity: false,
    }];
    let mut projectile = Projectile::new(ProjectileKind::Snowball, Vec3::ZERO, None);
    projectile.id = 9;
    let projectiles = [projectile];
    let rockets = [Rocket::remote(10, Vec3::ZERO, Vec3::ZERO)];
    let tridents = [ThrownTrident::remote(11, Vec3::ZERO, 0.0, 0.0)];
    let guests = [guest(12)];
    let others = [commands::OtherEntity {
        id: 13,
        kind: "minecraft:marker".into(),
        position: Vec3::ZERO,
        yaw: 0.0,
        pitch: 0.0,
    }];
    let cushions = [Cushion {
        id: 14,
        position: Vec3::ZERO,
        pos: [0; 3],
        yaw: 0.0,
        color: 0,
        check_interval: 0,
    }];
    let frames = [ItemFrame {
        id: 15,
        pos: [0; 3],
        facing: 2,
        glow: false,
        item: None,
        rotation: 0,
        drop_chance: 1.0,
        fixed: false,
        invisible: false,
        check_interval: 0,
    }];
    let paintings = [Painting {
        id: 16,
        pos: [0; 3],
        facing: 2,
        variant: "minecraft:kebab".into(),
        width: 1,
        height: 1,
        check_interval: 0,
    }];
    let mut potions = potions::PotionState::default();
    potions.add_remote_thrown(17, "minecraft:splash_potion", Vec3::ZERO, Vec3::ZERO);
    let mut end = end::EndState::default();
    end.add_remote_eye(18, [0.0; 3], [0.0; 3]);
    let tag = horizon_client::entity_nbt::vehicle_to_nbt(
        "minecraft:ender_dragon",
        Vec3::ZERO,
        Vec3::ZERO,
        0.0,
        [0; 4],
        None,
    );
    let mut dragon = dragon::Dragon::from_nbt(&tag, 0).unwrap();
    dragon.id = 20;
    let crystals = [dragon::Crystal {
        id: 21,
        position: Vec3::ZERO,
        show_bottom: true,
        time: 0,
        beam: None,
        invulnerable: false,
    }];
    let fireballs = [dragon::DragonFireball::remote(22, Vec3::ZERO, Vec3::ZERO)];
    let clouds = [dragon::Breath {
        id: 23,
        at: [0.0; 3],
        radius: 3.0,
        age: 0,
        duration: 100,
        from_dragon: true,
        potion: None,
    }];
    let knots = HashMap::from([([0; 3], 24)]);
    let fangs = [fangs::Fangs {
        id: 26,
        pos: [0.0; 3],
        yaw: 0.0,
        warmup: 0,
        life: 20,
        owner: 1,
        started: false,
    }];
    let sources = QuerySources {
        dimension: OVERWORLD,
        shared: EntityLists {
            mobs: &mobs,
            dropped: &dropped,
            boats: &boats,
            carts: &carts,
            armor_stands: &armor_stands,
            tnts: &tnts,
            falling: &falling,
            orbs: &orbs,
            projectiles: &projectiles,
            rockets: &rockets,
            tridents: &tridents,
        },
        guests: &guests,
        others: &others,
        cushions: &cushions,
        frames: &frames,
        paintings: &paintings,
        potions: &potions,
        include_transients: true,
        thrown_count: 1,
        end: &end,
        eye_count: 1,
        bolts: [(19, [0.0; 3])].into_iter(),
        dragon: Some(&dragon),
        crystals: &crystals,
        fireballs: &fireballs,
        clouds: &clouds,
        knots: &knots,
        spawners: [(25, [0.0; 3])].into_iter(),
        fangs: &fangs,
    };
    assert_eq!(sources.candidate_count(), 26);
    let result = query_entities(
        &mut QueryBudgets::default(),
        tick(1),
        0,
        OVERWORLD,
        Sphere::new([0.0; 3], 5.0).unwrap(),
        sources.candidate_count(),
        |visit| sources.visit(visit),
    )
    .unwrap();
    assert_eq!(
        result.iter().map(|entity| entity.id).collect::<Vec<_>>(),
        (1..=26).collect::<Vec<_>>()
    );
    for entity in &result {
        assert_eq!(entity.dimension, OVERWORLD);
        assert!(!entity.kind.is_empty());
        assert!(entity.alive);
        if matches!(entity.id, 1..=12 | 14..=16 | 21) {
            assert_ne!(entity.bounding_box[0], entity.bounding_box[1]);
        } else {
            assert_eq!(entity.bounding_box, [entity.position; 2]);
        }
    }
    assert_eq!(result[11].player_uuid, Some(0x1234));
    assert!(
        result
            .iter()
            .filter(|entity| entity.id != 12)
            .all(|entity| entity.player_uuid.is_none())
    );
}

#[test]
fn a_mob_53_blocks_from_the_center_intersects_the_level_tick_query_sphere() {
    let mut cow = mob(27);
    cow.position = Vec3::new(5.3, 0.0, 0.0);
    let candidate = Candidate::shared(&cow, OVERWORLD);
    let result = query_entities(
        &mut QueryBudgets::default(),
        tick(1),
        0,
        OVERWORLD,
        Sphere::new([0.0; 3], 5.0).unwrap(),
        1,
        |visit| visit(candidate),
    )
    .unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, 27);
    assert_eq!(result[0].position[0], 5.3);
}

#[test]
fn parked_levels_do_not_block_current_level_mob_queries_on_unscoped_transients() {
    let center = [-30.5, 101.62, -24.5];
    let mut cow = mob(27);
    Arc::make_mut(&mut cow.kind).id = "minecraft:cow".into();
    cow.position = Vec3::new(-25.5, 101.62, -24.5);
    let mobs = [cow];
    let mut owner = guest(12);
    owner.position = center;
    let guests = [owner];
    let mut potions = potions::PotionState::default();
    potions.add_remote_thrown(
        28,
        "minecraft:splash_potion",
        Vec3::new(center[0], center[1], center[2]),
        Vec3::ZERO,
    );
    let mut end = end::EndState::default();
    end.add_remote_eye(29, center, [0.0; 3]);
    let sources = QuerySources {
        dimension: OVERWORLD,
        shared: EntityLists {
            mobs: &mobs,
            dropped: &[],
            boats: &[],
            carts: &[],
            armor_stands: &[],
            tnts: &[],
            falling: &[],
            orbs: &[],
            projectiles: &[],
            rockets: &[],
            tridents: &[],
        },
        guests: &guests,
        others: &[],
        cushions: &[],
        frames: &[],
        paintings: &[],
        potions: &potions,
        include_transients: false,
        thrown_count: 0,
        end: &end,
        eye_count: 0,
        bolts: std::iter::empty(),
        dragon: None,
        crystals: &[],
        fireballs: &[],
        clouds: &[],
        knots: &HashMap::new(),
        spawners: std::iter::empty(),
        fangs: &[],
    };
    let result = query_entities(
        &mut QueryBudgets::default(),
        tick(1),
        0,
        OVERWORLD,
        Sphere::new(center, 8.0).unwrap(),
        sources.candidate_count(),
        |visit| sources.visit(visit),
    )
    .unwrap();

    assert_eq!(
        result
            .iter()
            .map(|entity| (entity.id, entity.kind.as_str()))
            .collect::<Vec<_>>(),
        [(12, "minecraft:player"), (27, "minecraft:cow")]
    );
}

#[test]
fn real_living_adapters_keep_dying_entities_and_exclude_removed_entities() {
    let mut dying = mob(1);
    dying.health = 0.0;
    let mut removed = mob(2);
    removed.removed = true;
    let mut stand = ArmorStand::new(3, Vec3::ZERO, 0.0);
    stand.health = 0.0;
    let result = run(
        &mut QueryBudgets::default(),
        1,
        0,
        &[
            Candidate::shared(&dying, OVERWORLD),
            Candidate::shared(&removed, OVERWORLD),
            Candidate::shared(&stand, OVERWORLD),
        ],
    )
    .unwrap();
    assert_eq!(result.len(), 2);
    assert!(result.iter().all(|entity| !entity.alive));
    assert_eq!(result[0].id, 1);
    assert_eq!(result[1].id, 3);
}

#[test]
fn player_state_controls_dimension_liveness_placement_and_uuid() {
    let mut dying = guest(1);
    dying.health = 0.0;
    let mut removed = guest(2);
    removed.placed = false;
    let mut elsewhere = guest(3);
    elsewhere.dimension = NETHER.into();
    let result = run(
        &mut QueryBudgets::default(),
        1,
        0,
        &[
            Candidate::guest(&dying).unwrap(),
            Candidate::guest(&removed).unwrap(),
            Candidate::guest(&elsewhere).unwrap(),
        ],
    )
    .unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, 1);
    assert!(!result[0].alive);
    assert_eq!(result[0].player_uuid, Some(dying.uuid));
}
