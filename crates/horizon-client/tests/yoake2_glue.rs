//! MX3G1: the generated binding glue of the items and entities modules, compiled into the real engine
//! (feature `yoake2-slice`) and called on real engine objects through the slice runtime's activation
//! handles. Each call is compared with the same call made directly in Rust.
//!
//!   cargo test --offline -j 4 -p horizon-client-lib --features yoake2-slice --test yoake2_glue

use horizon_client_lib::entities::{self, ArmorStand, ItemEntity, StandMedium, StandStep};
use horizon_client_lib::items::{self, ItemStack};
use horizon_core::geometry::Vec3;
use std::sync::Arc;
use yoake_slice::activation::{self, Status};
use yoake_slice::codec::{CallError, Decode, Decoder, Encode};

fn id_of(path: &str) -> u32 {
    entities::yoake_glue::BINDINGS
        .iter()
        .chain(items::yoake_glue::BINDINGS)
        .find(|(_, p)| *p == path)
        .unwrap_or_else(|| panic!("no binding {path}"))
        .0
}

/// Ids are disjoint across modules (one lock file), so the first module that knows the id runs it.
fn call(handle: u64, path: &str, args: &[u8]) -> Result<Vec<u8>, CallError> {
    let id = id_of(path);
    let mut out = Vec::new();
    match entities::yoake_glue::call(handle, id, args, &mut out) {
        Err(CallError::UnknownBinding(_)) => items::yoake_glue::call(handle, id, args, &mut out)?,
        other => other?,
    }
    Ok(out)
}

fn bytes(f: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut v = Vec::new();
    f(&mut v);
    v
}

fn decode<T: Decode>(b: &[u8]) -> T {
    let mut d = Decoder::new(b);
    let v = T::decode(&mut d).unwrap();
    d.finish().unwrap();
    v
}

fn vec3(v: Vec3) -> Vec<u8> {
    bytes(|b| {
        v.x.encode(b);
        v.y.encode(b);
        v.z.encode(b);
    })
}

fn decode_vec3(b: &[u8]) -> Vec3 {
    let (x, y, z) = decode::<(f64, f64, f64)>(b);
    Vec3::new(x, y, z)
}

#[test]
fn an_armor_stand_through_its_handle() {
    let mut stand = ArmorStand::new(7, Vec3::new(1.0, 2.0, 3.0), 90.0);
    stand.previous_yaw = 10.0;
    let direct = stand.clone();
    let expected_yaw = direct.render_yaw(0.5);
    let expected_pos = direct.render_position(0.25);

    activation::scope(&mut stand, &entities::yoake_glue::FIELDS_crate__entities__ArmorStand, &mut |_| 0, |h| {
        // `&self` methods with scalar and struct results
        assert_eq!(decode::<f32>(&call(h, "entities::ArmorStand::render_yaw", &bytes(|b| 0.5f32.encode(b))).unwrap()), expected_yaw);
        assert_eq!(decode_vec3(&call(h, "entities::ArmorStand::render_position", &bytes(|b| 0.25f64.encode(b))).unwrap()), expected_pos);
        // a private scalar field, set and read back through the generated accessors and the FieldTable
        call(h, "entities::ArmorStand.disabled_slots#set", &bytes(|b| 0b101i32.encode(b))).unwrap();
        assert_eq!(decode::<i32>(&call(h, "entities::ArmorStand.disabled_slots#get", &[]).unwrap()), 0b101);
        for slot in 0..6usize {
            let wire = call(h, "entities::ArmorStand::is_disabled", &bytes(|b| slot.encode(b))).unwrap();
            assert_eq!(decode::<bool>(&wire), direct_is_disabled(0b101, slot), "slot {slot}");
        }
        // a struct argument (`Vec3`) and an `Option<(Vec3, f32)>` argument
        call(h, "entities::ArmorStand::snap", &bytes(|b| {
            vec3(Vec3::new(5.0, 6.0, 7.0)).iter().for_each(|byte| b.push(*byte));
            45.0f32.encode(b);
        }))
        .unwrap();
        assert_eq!(decode_vec3(&call(h, "entities::ArmorStand.position#get", &[]).unwrap()), Vec3::new(5.0, 6.0, 7.0));
        call(h, "entities::ArmorStand::interpolate", &bytes(|b| {
            1u8.encode(b);
            vec3(Vec3::new(8.0, 6.0, 7.0)).iter().for_each(|byte| b.push(*byte));
            50.0f32.encode(b);
        }))
        .unwrap();
        assert_eq!(decode_vec3(&call(h, "entities::ArmorStand.previous#get", &[]).unwrap()), Vec3::new(5.0, 6.0, 7.0));
        assert_eq!(decode_vec3(&call(h, "entities::ArmorStand.position#get", &[]).unwrap()), Vec3::new(8.0, 6.0, 7.0));
        // the scalar `FieldTable` sees the same object
        assert_eq!(activation::set(h, 1, 33), Ok(()));
        assert_eq!(decode::<i32>(&call(h, "entities::ArmorStand.fire_ticks#get", &[]).unwrap()), 33);
        // a handle to another type is refused, nothing runs
        assert_eq!(
            call(h, "entities::ItemEntity::has_infinite_pickup_delay", &[]),
            Err(CallError::Handle(Status::WrongType))
        );
        // a malformed call is an error, not a trap
        assert_eq!(call(h, "entities::ArmorStand::render_yaw", &[1, 2]), Err(CallError::UnexpectedEnd));
        assert_eq!(call(h, "entities::ArmorStand::render_yaw", &[0; 5]), Err(CallError::TrailingBytes));
    });
}

#[test]
fn a_glued_tick_settle_does_what_the_rust_one_does() {
    for (medium_wire, medium) in [
        (bytes(|b| 0u32.encode(b)), StandMedium::Air),
        (bytes(|b| 1u32.encode(b)), StandMedium::Water),
        (bytes(|b| {
            2u32.encode(b); // StandMedium::Lava { shallow }
            true.encode(b);
        }), StandMedium::Lava { shallow: true }),
    ] {
        let mut stand = ArmorStand::new(7, Vec3::new(1.0, 2.0, 3.0), 90.0);
        stand.velocity = Vec3::new(0.1, -0.3, 0.2);
        let mut control = stand.clone();
        control.tick_settle(medium, StandStep::for_hook_bench(true, 0.6));
        let step_wire = bytes(|b| {
            true.encode(b);
            0.6f32.encode(b);
        });
        let args: Vec<u8> = medium_wire.iter().chain(step_wire.iter()).copied().collect();
        activation::scope(&mut stand, &entities::yoake_glue::FIELDS_crate__entities__ArmorStand, &mut |_| 0, |h| {
            call(h, "entities::ArmorStand::tick_settle", &args).unwrap();
        });
        assert_eq!(stand.velocity, control.velocity, "{medium:?}");
        assert_eq!(stand.position, control.position, "{medium:?}");
        assert_ne!(stand.velocity, Vec3::new(0.1, -0.3, 0.2), "the tick must have changed something");
    }
}

fn direct_is_disabled(disabled_slots: i32, slot: usize) -> bool {
    // `ArmorStand::is_disabled` reads bit `filterFlag(slot)`; the engine function itself is the oracle
    let mut stand = ArmorStand::new(0, Vec3::ZERO, 0.0);
    stand.disabled_slots = disabled_slots;
    stand.is_disabled(slot)
}

fn dropped_item(count: u32, age: i32) -> ItemEntity {
    let at = Vec3::new(0.5, 64.0, 0.5);
    ItemEntity {
        join: Default::default(),
        no_gravity: false,
        id: 0,
        thrower: None,
        target: None,
        stack: ItemStack { item: Arc::from("minecraft:stone"), count, damage: 0, extra: None },
        health: 5,
        fire_ticks: 0,
        position: at,
        previous: at,
        velocity: Vec3::ZERO,
        on_ground: true,
        vertical_collision: false,
        horizontal_collision: false,
        age,
        pickup_delay: 0,
        bob_offset: 0.0,
        ticks: 0,
        needs_sync: false,
        stuck: Vec3::ZERO,
    }
}

#[test]
fn an_item_entity_through_its_handle() {
    let mut item = dropped_item(10, 100);
    activation::scope(&mut item, &entities::yoake_glue::FIELDS_crate__entities__ItemEntity, &mut |_| 0, |h| {
        let mergeable = |age: i32, max: u32| {
            decode::<bool>(&call(h, "entities::ItemEntity::mergeable", &bytes(|b| {
                age.encode(b);
                max.encode(b);
            }))
            .unwrap())
        };
        assert!(mergeable(100, 64));
        assert!(!mergeable(100, 10), "a full stack is not mergeable");
        assert!(!mergeable(6000, 64), "an item at its lifetime is not mergeable");
        // an `i64` private-ish field and the engine method that reads it
        call(h, "entities::ItemEntity.pickup_delay#set", &bytes(|b| i64::from(i16::MAX).encode(b))).unwrap();
        assert!(decode::<bool>(&call(h, "entities::ItemEntity::has_infinite_pickup_delay", &[]).unwrap()));
        assert!(!mergeable(100, 64), "an infinite pickup delay blocks merging");
        // `Option<[i32; 4]>` fields
        let thrower = bytes(|b| Some([1i32, 2, 3, 4]).encode(b));
        // `thrower` is an entity reference: readable, never writable (its setter id is reserved)
        let set = id_of("entities::ItemEntity.thrower#set");
        assert_eq!(call(h, "entities::ItemEntity.thrower#set", &thrower), Err(CallError::ReadOnly(set)));
        assert_eq!(decode::<Option<[i32; 4]>>(&call(h, "entities::ItemEntity.thrower#get", &[]).unwrap()), None);
        // a method that changes `self`
        call(h, "entities::ItemEntity::honey_block_inside", &bytes(|b| [0i32, 0, 0].encode(b))).unwrap();
        // the trait method
        assert!(!decode::<bool>(&call(h, "entities::<ItemEntity as EntityRules>::is_no_gravity", &[]).unwrap()));
        assert_eq!(decode::<f64>(&call(h, "entities::<ItemEntity as EntityRules>::get_default_gravity", &[]).unwrap()), 0.04);
    });
    assert_eq!(item.thrower, None);
    assert_eq!(item.pickup_delay, i64::from(i16::MAX));
}

#[test]
fn free_functions_and_the_items_module() {
    // free functions of the entities module (no receiver, any handle)
    assert_eq!(decode::<u8>(&call(0, "entities::dye_index", &bytes(|b| "minecraft:red".encode(b))).unwrap()), entities::dye_index("minecraft:red"));
    let wire = call(0, "entities::slime_step_on_scale", &bytes(|b| (-0.3f64).encode(b))).unwrap();
    assert_eq!(decode::<Option<f64>>(&wire), entities::slime_step_on_scale(-0.3));
    let wire = call(0, "entities::move_applies", &bytes(|b| {
        vec3(Vec3::new(1.0, 0.0, 0.0)).iter().for_each(|byte| b.push(*byte));
        vec3(Vec3::new(0.0, 0.0, 0.0)).iter().for_each(|byte| b.push(*byte));
    }))
    .unwrap();
    assert_eq!(decode::<bool>(&wire), entities::move_applies(Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO));

    // the items module: a free function and the accessors of `ItemStack`
    let wire = call(0, "items::villager_food", &bytes(|b| "minecraft:bread".encode(b))).unwrap();
    assert_eq!(decode::<Option<i32>>(&wire), items::villager_food("minecraft:bread"));
    let mut stack = ItemStack { item: Arc::from("minecraft:stone"), count: 3, damage: 1, extra: None };
    activation::scope(&mut stack, &items::yoake_glue::FIELDS_crate__items__ItemStack, &mut |_| 0, |h| {
        assert_eq!(decode::<u32>(&call(h, "items::ItemStack.count#get", &[]).unwrap()), 3);
        call(h, "items::ItemStack.count#set", &bytes(|b| 9u32.encode(b))).unwrap();
        // the item id is frozen: refused, unchanged
        let set = id_of("items::ItemStack.item#set");
        assert_eq!(call(h, "items::ItemStack.item#set", &bytes(|b| "minecraft:dirt".encode(b))), Err(CallError::ReadOnly(set)));
        assert_eq!(decode::<String>(&call(h, "items::ItemStack.item#get", &[]).unwrap()), "minecraft:stone");
        assert_eq!(activation::get(h, 1), Ok(9), "count through the scalar FieldTable");
    });
    assert_eq!(stack.count, 9);
    assert_eq!(&*stack.item, "minecraft:stone");
}

#[test]
fn bindings_have_unique_ids_across_both_modules() {
    let mut seen = std::collections::BTreeMap::new();
    for (id, path) in entities::yoake_glue::BINDINGS.iter().chain(items::yoake_glue::BINDINGS) {
        assert_eq!(seen.insert(*id, *path), None, "id {id} is used twice");
    }
    assert!(seen.len() > 300, "only {} bindings", seen.len());
}
