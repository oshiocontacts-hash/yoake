//! F4M-4d: several mob riders on a custom vehicle: boarding up to the seat count, the seat of each, the
//! driving seat, getting off, the save, the `seat-set` command, and the vanilla one-rider paths that must
//! not change. On the Cinder Fox fixture with seats, and plain chickens and cows as the riders.
use super::riding::{self, Refusal, RiderKind};
use crate::mobs::{
    Mob, MobEvent, Species,
    custom::{
        CustomType, SeatClaim,
        anim_sync::tests::{fox_mob, fox_type_with},
        persist,
        script::Command,
        script_apply,
    },
    equine, riders,
};
use horizon_core::geometry::Vec3;
use horizon_world::nbt::{Compound, Tag};
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};

const P1: [i32; 4] = [1, 1, 1, 1];
const P2: [i32; 4] = [2, 2, 2, 2];

fn at(x: f64, z: f64) -> Vec3 {
    Vec3::new(x, 1.0, z)
}

/// The fox type with `seats` (and no `tameable`, so anyone may ride it).
fn seated(seats: Value) -> Arc<CustomType> {
    fox_type_with(json!({"navigator": "ground", "goals": []}), |v| {
        v.as_object_mut().unwrap().remove("tameable");
        v["seats"] = seats;
    })
}

/// A driving seat, then two seats that only carry.
fn three() -> Value {
    json!([
        {"offset": [0.0, 1.0, 0.0], "controls": "player", "yaw_lock": true},
        {"offset": [0.0, 1.0, -0.8]},
        {"offset": [0.0, 1.0, -1.6]}
    ])
}

/// The vehicle, entity 1, at (10, 20).
fn vehicle(t: &Arc<CustomType>) -> Mob {
    let mut m = fox_mob(t, at(10.0, 20.0));
    m.id = 1;
    m
}

/// A vanilla mob of `species`, entity `id`.
fn vanilla(id: u32, species: Species) -> Mob {
    let mut kind = (*crate::mobs::sensor_scan_tests::kind()).clone();
    kind.species = species;
    Mob::new(id, Arc::new(kind), at(0.5, 0.5), 0.0)
}

fn chicken(id: u32) -> Mob {
    vanilla(id, Species::Chicken)
}

fn claims(m: &Mob) -> Vec<(u32, u8)> {
    m.extra.custom.as_ref().unwrap().seat_claims.iter().map(|c| (c.rider, c.seat)).collect()
}

fn sync(mobs: &mut [Mob]) {
    riders::sync_links(mobs, &HashSet::new());
}

fn player() -> equine::Rider {
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

// ---- boarding ---------------------------------------------------------------------------------------

/// Mobs board up to the type's seat count, each on the first free seat; the next one is refused.
#[test]
fn several_mobs_board_up_to_the_seat_count() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4), chicken(5)];
    assert_eq!(riding::board(&mut mobs, 2, 1, None, false), Ok(0));
    assert_eq!(riding::board(&mut mobs, 3, 1, None, false), Ok(1));
    assert_eq!(riding::board(&mut mobs, 4, 1, None, false), Ok(2));
    assert_eq!(riding::board(&mut mobs, 5, 1, None, false), Err(Refusal::Full));
    assert_eq!(mobs[4].extra.mount, None, "the refused one does not ride");
    assert_eq!((mobs[1].extra.mount, mobs[2].extra.mount, mobs[3].extra.mount), (Some(1), Some(1), Some(1)));
    assert_eq!(claims(&mobs[0]), [(2, 0), (3, 1), (4, 2)]);
    // `extra.rider` is the mob in the lowest seat.
    assert_eq!(mobs[0].extra.rider, Some(2));
    // The per-tick sync keeps all three and their seats.
    sync(&mut mobs);
    assert_eq!(claims(&mobs[0]), [(2, 0), (3, 1), (4, 2)]);
    assert_eq!(mobs[0].extra.rider, Some(2));
}

/// The player counts: with it aboard on a seat, mobs take the others.
#[test]
fn the_player_and_mobs_share_the_seats() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4)];
    mobs[0].rider = Some(player());
    assert_eq!(riding::claim_player(&mut mobs, 1, P1, None), Ok(0));
    assert_eq!(riding::player_seat(&mobs[0]).map(|(i, _)| i), Some(0));
    assert_eq!(riding::board(&mut mobs, 2, 1, None, false), Ok(1), "the player holds seat 0");
    assert_eq!(riding::board(&mut mobs, 3, 1, None, false), Ok(2));
    assert_eq!(riding::board(&mut mobs, 4, 1, None, false), Err(Refusal::Full));
    assert_eq!(riding::board(&mut mobs, 4, 1, Some(0), false), Err(Refusal::Taken), "not the player's seat");
}

/// An explicit seat, and every reason for a refusal.
#[test]
fn a_seat_can_be_asked_for_and_refusals_say_why() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4), vanilla(5, Species::Cow)];
    assert_eq!(riding::board(&mut mobs, 2, 1, Some(2), false), Ok(2));
    assert_eq!(claims(&mobs[0]), [(2, 2)]);
    // The next one without a choice takes the first free seat, not the one after.
    assert_eq!(riding::board(&mut mobs, 3, 1, None, false), Ok(0));
    assert_eq!(riding::board(&mut mobs, 4, 1, Some(2), false), Err(Refusal::Taken));
    assert_eq!(riding::board(&mut mobs, 4, 1, Some(9), false), Err(Refusal::NoSuchSeat));
    // Asking for the seat it already has is no change; another free one moves it.
    assert_eq!(riding::board(&mut mobs, 3, 1, Some(0), false), Ok(0));
    assert_eq!(riding::board(&mut mobs, 3, 1, Some(1), false), Ok(1));
    assert_eq!(claims(&mobs[0]), [(3, 1), (2, 2)]);
    // A mob does not ride itself, an unknown mob nothing, a chicken no vehicle without seats, and a vehicle
    // that rides the rider is no vehicle for it.
    assert_eq!(riding::board(&mut mobs, 1, 1, None, false), Err(Refusal::Itself));
    assert_eq!(riding::board(&mut mobs, 77, 1, None, false), Err(Refusal::Missing));
    assert_eq!(riding::board(&mut mobs, 2, 3, None, false), Err(Refusal::NoSeats), "a chicken has no seats");
    mobs[0].extra.mount = Some(4);
    assert_eq!(riding::board(&mut mobs, 4, 1, None, false), Err(Refusal::Loop));
    mobs[0].extra.mount = None;
    // The boarding cooldown stops the AI's boarding and not a command's.
    mobs[3].extra.boarding_cooldown = 5;
    assert_eq!(riding::board(&mut mobs, 4, 1, None, true), Err(Refusal::Cooldown));
    assert_eq!(riding::board(&mut mobs, 4, 1, None, false), Ok(0));
}

/// `allowed_types` decide who may sit where: a seat for chickens, one for the player, one for the type itself.
#[test]
fn allowed_types_pick_the_seat() {
    let t = seated(json!([
        {"offset": [0.0, 1.0, 0.0], "allowed_types": ["minecraft:player"]},
        {"offset": [0.0, 1.0, -0.8], "allowed_types": ["minecraft:chicken"]},
        {"offset": [0.0, 1.0, -1.6], "allowed_types": ["sample_mobs:cinder_fox"]}
    ]));
    let mut other = fox_mob(&t, at(0.5, 0.5));
    other.id = 4;
    let mut mobs = vec![vehicle(&t), chicken(2), vanilla(3, Species::Cow), other, chicken(5)];
    assert_eq!(riding::board(&mut mobs, 2, 1, None, false), Ok(1));
    assert_eq!(riding::board(&mut mobs, 3, 1, None, false), Err(Refusal::NotAllowed), "a cow sits nowhere");
    assert_eq!(riding::board(&mut mobs, 4, 1, None, false), Ok(2), "the fox takes its own seat");
    assert_eq!(riding::board(&mut mobs, 5, 1, Some(0), false), Err(Refusal::NotAllowed), "seat 0 is the player's");
    assert_eq!(riding::board(&mut mobs, 5, 1, None, false), Err(Refusal::Full), "the chicken seat is taken");
    // The player's seat stays free for the player.
    assert_eq!(riding::free_seat_for_player(&mobs[0], P1), Some(0));
}

// ---- where they sit ---------------------------------------------------------------------------------

/// Every rider sits at its seat's offset, turned with the vehicle.
#[test]
fn riders_sit_at_the_offset_of_their_seat() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4)];
    for id in 2..=4 {
        riding::board(&mut mobs, id, 1, None, false).unwrap();
    }
    let aboard: Vec<&Mob> = mobs.iter().filter(|m| m.extra.mount == Some(1)).collect();
    let offsets: Vec<[f64; 3]> =
        aboard.iter().map(|r| riding::mob_rider_attachment(&mobs[0], r, &aboard)).collect();
    assert_eq!(offsets, [[0.0, 1.0, 0.0], [0.0, 1.0, -0.8], [0.0, 1.0, -1.6]]);
    // Facing +z at yaw 0, behind is -z; the vehicle is at (10, 1, 20).
    let at = |i: usize, yaw: f32| riders::seat(offsets[i], 0.0, mobs[0].position, yaw);
    let p = at(2, 0.0);
    assert!((p.x - 10.0).abs() < 1e-9 && (p.y - 2.0).abs() < 1e-9 && (p.z - 18.4).abs() < 1e-9, "{p:?}");
    // A quarter turn swings the rear seat sideways.
    let p = at(2, 90.0);
    assert!(((p.x - 10.0).abs() - 1.6).abs() < 1e-6 && (p.z - 20.0).abs() < 1e-6, "{p:?}");
    // The seat that locks the yaw is the first one.
    assert!(riding::seat_locks_yaw(&mobs[0], 0) && !riding::seat_locks_yaw(&mobs[0], 1));
    // With the player aboard and one chicken off, the others keep their seats.
    mobs[1].extra.mount = None;
    sync(&mut mobs);
    mobs[0].rider = Some(player());
    riding::claim_player(&mut mobs, 1, P1, None).unwrap();
    let aboard: Vec<&Mob> = mobs.iter().filter(|m| m.extra.mount == Some(1)).collect();
    let seat_of = |r: &Mob| riding::mob_rider_seat(&mobs[0], r, &aboard);
    assert_eq!((seat_of(aboard[0]), seat_of(aboard[1])), (Some(1), Some(2)));
    assert_eq!(riding::player_seat(&mobs[0]).map(|(i, _)| i), Some(0), "the freed seat is the player's");
}

/// A rider keeps its seat when another gets off, and a new one fills the free seat.
#[test]
fn a_seat_is_kept_when_another_rider_gets_off() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4)];
    riding::board(&mut mobs, 2, 1, None, false).unwrap();
    riding::board(&mut mobs, 3, 1, None, false).unwrap();
    riders::stop_riding(&mut mobs[1], None);
    sync(&mut mobs);
    assert_eq!(claims(&mobs[0]), [(3, 1)], "the second rider stays on seat 1");
    assert_eq!(mobs[0].extra.rider, Some(3));
    assert_eq!(riding::board(&mut mobs, 4, 1, None, false), Ok(0), "the free seat is filled");
    assert_eq!(claims(&mobs[0]), [(4, 0), (3, 1)]);
    assert_eq!(mobs[0].extra.rider, Some(4));
}

// ---- the driving seat -------------------------------------------------------------------------------

/// The player steers from a `player` seat only, and a mob steers (toward its target) from one too; a mob on
/// a carrying seat is only carried.
#[test]
fn the_driving_seat_steers_and_the_others_carry() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3)];
    // A chicken on seat 1 (carries) does not steer; on seat 0 it does.
    riding::board(&mut mobs, 2, 1, Some(1), false).unwrap();
    assert_eq!(mobs[0].extra.rider, Some(2));
    assert!(!riding::mob_may_steer(&mobs[0]));
    riding::board(&mut mobs, 3, 1, Some(0), false).unwrap();
    assert_eq!(mobs[0].extra.rider, Some(3), "the lowest seat");
    assert!(riding::mob_may_steer(&mobs[0]));
    // The player: a mob in the driving seat leaves it a carrying seat, which does not steer ...
    mobs[0].rider = Some(player());
    assert_eq!(riding::claim_player(&mut mobs, 1, P1, None), Ok(2));
    assert_eq!(riding::player_seat(&mobs[0]).map(|(i, _)| i), Some(2));
    assert!(!equine::controlled(&mobs[0]), "seat 2 only carries");
    // ... and once the driving seat is free a player that boards takes it and steers.
    mobs[2].extra.mount = None;
    sync(&mut mobs);
    riding::release_player(&mut mobs[0], P1);
    assert_eq!(riding::claim_player(&mut mobs, 1, P1, None), Ok(0));
    assert_eq!(riding::player_seat(&mobs[0]).map(|(i, _)| i), Some(0));
    assert!(equine::controlled(&mobs[0]));
    assert_eq!(riding::locked_yaw(&mobs[0]), Some(mobs[0].yaw));
    // The seat the boarding click picked is the player's.
    riding::release_player(&mut mobs[0], P1);
    assert_eq!(riding::claim_player(&mut mobs, 1, P1, Some(2)), Ok(2));
    assert_eq!(riding::player_seat(&mobs[0]).map(|(i, _)| i), Some(2));
    assert!(!equine::controlled(&mobs[0]));
}

// ---- getting off and dying --------------------------------------------------------------------------

/// Every rider lets go when the vehicle dies; one that dies leaves the others in place.
#[test]
fn riders_get_off_when_the_vehicle_or_a_rider_dies() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4)];
    for id in 2..=4 {
        riding::board(&mut mobs, id, 1, None, false).unwrap();
    }
    // A rider dies: only it goes.
    mobs[2].health = 0.0;
    sync(&mut mobs);
    assert_eq!(mobs[2].extra.mount, None);
    assert_eq!(claims(&mobs[0]), [(2, 0), (4, 2)]);
    // The vehicle dies: everyone is off, nothing is claimed, nobody is the rider.
    mobs[0].health = 0.0;
    sync(&mut mobs);
    assert!(mobs.iter().all(|m| m.extra.mount.is_none() && !m.extra.passenger));
    assert!(claims(&mobs[0]).is_empty());
    assert_eq!(mobs[0].extra.rider, None);
    // A removed vehicle too.
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3)];
    riding::board(&mut mobs, 2, 1, None, false).unwrap();
    riding::board(&mut mobs, 3, 1, None, false).unwrap();
    mobs[0].removed = true;
    sync(&mut mobs);
    assert!(mobs[1].extra.mount.is_none() && mobs[2].extra.mount.is_none());
}

/// Riders that came without a seat (an old save, `/ride`) are seated by the sync, and those no seat is left
/// for are put off.
#[test]
fn the_sync_seats_riders_that_came_without_a_claim_and_drops_the_surplus() {
    let t = seated(json!([{"offset": [0.0, 1.0, 0.0]}, {"offset": [0.0, 1.0, -1.0]}]));
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4)];
    for id in 2..=4 {
        riders::link(&mut mobs, id, 1);
    }
    sync(&mut mobs);
    assert_eq!(claims(&mobs[0]), [(2, 0), (3, 1)]);
    assert_eq!(mobs[3].extra.mount, None, "no seat for the third");
    assert!(!mobs[3].extra.passenger);
    assert_eq!(mobs[0].extra.rider, Some(2));
}

// ---- vanilla and seatless vehicles keep the one rider ------------------------------------------------

/// The vanilla paths: a pig takes one rider by the old rule, the links of a vanilla vehicle with two riders
/// keep the last one as before, and nothing of a seat is ever written on a vanilla mob.
#[test]
fn a_vanilla_vehicle_keeps_its_one_rider_rules() {
    let mut mobs = vec![vanilla(1, Species::Pig), chicken(2), chicken(3)];
    assert!(riders::start_riding(&mut mobs, 2, 1));
    assert!(!riders::start_riding(&mut mobs, 3, 1), "a vehicle with a passenger takes no other");
    assert_eq!((mobs[1].extra.mount, mobs[2].extra.mount), (Some(1), None));
    assert_eq!(mobs[0].extra.rider, Some(2));
    // Two riders forced on it (`/ride`): the link as it always was, the last one in the list is its rider.
    riders::link(&mut mobs, 3, 1);
    sync(&mut mobs);
    assert_eq!((mobs[1].extra.mount, mobs[2].extra.mount), (Some(1), Some(1)));
    assert_eq!(mobs[0].extra.rider, Some(3));
    assert!(mobs[0].extra.custom.is_none());
    assert_eq!(riding::board(&mut mobs, 2, 1, None, false), Err(Refusal::NoSeats));
    // The vehicle dying lets both go.
    mobs[0].health = 0.0;
    sync(&mut mobs);
    assert!(mobs[1].extra.mount.is_none() && mobs[2].extra.mount.is_none());
    // A chicken on a boat or a minecart (an `other` vehicle id) stays while the id is there.
    let mut mobs = vec![chicken(2)];
    riders::link(&mut mobs, 2, 500);
    riders::sync_links(&mut mobs, &HashSet::from([500]));
    assert_eq!(mobs[0].extra.mount, Some(500));
    riders::sync_links(&mut mobs, &HashSet::new());
    assert_eq!(mobs[0].extra.mount, None);
}

/// A custom type without seats is a one-rider vehicle as before 4d (`/ride` works, a second rider is
/// refused by the AI's boarding), and the AI boards a seated vehicle by its seats and its cooldown.
#[test]
fn a_custom_type_without_seats_keeps_the_old_link_and_the_ai_boards_seats() {
    let none = fox_type_with(json!({"navigator": "ground", "goals": []}), |_| {});
    let mut v = fox_mob(&none, at(0.5, 0.5));
    v.id = 1;
    let mut mobs = vec![v, chicken(2), chicken(3)];
    assert!(!riding::has_seats(&mobs[0]));
    assert!(riders::start_riding(&mut mobs, 2, 1));
    assert!(!riders::start_riding(&mut mobs, 3, 1));
    sync(&mut mobs);
    assert_eq!(mobs[0].extra.rider, Some(2));
    assert!(mobs[0].extra.custom.as_ref().unwrap().seat_claims.is_empty());
    // With seats, the AI boards two and honours the cooldown.
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3)];
    mobs[1].extra.boarding_cooldown = 10;
    assert!(!riders::start_riding(&mut mobs, 2, 1), "the cooldown");
    mobs[1].extra.boarding_cooldown = 0;
    assert!(riders::start_riding(&mut mobs, 2, 1));
    assert!(riders::start_riding(&mut mobs, 3, 1), "a second rider boards");
    assert_eq!(claims(&mobs[0]), [(2, 0), (3, 1)]);
}

// ---- seat-set ---------------------------------------------------------------------------------------

/// What the game does with the events `apply_brain_events` leaves for it (`Game::script_seat_set`): plan,
/// ask the native mods (`veto`: the test double, told the rider, the vehicle and whether it boards), apply.
fn game_seat_set(mobs: &mut [Mob], event: MobEvent, veto: &mut dyn FnMut(u32, u32, bool) -> bool) {
    let MobEvent::SeatSet { vehicle, seat, rider } = event else { panic!("{event:?}") };
    let Some((who, mounting)) = riding::plan_seat_set(mobs, vehicle, seat as usize, rider) else { return };
    if veto(who, vehicle, mounting) {
        riding::seat_set(mobs, vehicle, seat as usize, rider);
    }
}

/// The `seat-set` command: the vehicle asks for the event, the engine seats (or frees) the rider.
#[test]
fn the_seat_set_command_seats_and_frees_a_rider() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3)];
    let mut events: Vec<MobEvent> = Vec::new();
    let mut yes = |_: u32, _: u32, _: bool| true;
    // Without an event queue the command waits.
    let cmd = Command::SeatSet { seat: 2, rider: Some(2) };
    assert!(!script_apply::apply_in(&mut mobs[0], None, None, cmd.clone()));
    assert!(script_apply::apply_in(&mut mobs[0], None, Some(&mut events), cmd));
    assert_eq!(events.len(), 1);
    // The brain pass leaves it for the game (which asks the native mods).
    crate::mobs::apply_brain_events(&mut mobs, &mut events, 0);
    assert_eq!(events.len(), 1, "the event stays for the game");
    game_seat_set(&mut mobs, events.remove(0), &mut yes);
    assert_eq!(claims(&mobs[0]), [(2, 2)]);
    assert_eq!(mobs[1].extra.mount, Some(1));
    // A taken seat changes nothing; another rider takes a free one.
    for (seat, rider) in [(2, 3), (1, 3)] {
        script_apply::apply_in(&mut mobs[0], None, Some(&mut events), Command::SeatSet { seat, rider: Some(rider) });
        crate::mobs::apply_brain_events(&mut mobs, &mut events, 0);
        game_seat_set(&mut mobs, events.remove(0), &mut yes);
    }
    assert_eq!(claims(&mobs[0]), [(3, 1), (2, 2)]);
    // `none` frees the seat: its rider gets off (and cannot board for 60 ticks), the other stays.
    script_apply::apply_in(&mut mobs[0], None, Some(&mut events), Command::SeatSet { seat: 2, rider: None });
    crate::mobs::apply_brain_events(&mut mobs, &mut events, 0);
    game_seat_set(&mut mobs, events.remove(0), &mut yes);
    assert_eq!(claims(&mobs[0]), [(3, 1)]);
    assert_eq!((mobs[1].extra.mount, mobs[1].extra.boarding_cooldown), (None, 60));
    // Freeing an empty seat does nothing.
    riding::seat_set(&mut mobs, 1, 0, None);
    assert_eq!(claims(&mobs[0]), [(3, 1)]);
}

/// A native mod that vetoes (`EntityMountEvent` canceled) stops `seat-set` boarding and freeing, and the AI
/// boarding of a seated vehicle; it is asked once and only for what would really happen.
#[test]
fn a_native_veto_stops_seat_set_and_ai_boarding() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3)];
    let asked = std::cell::RefCell::new(Vec::new());
    let mut no = |who: u32, v: u32, mounting: bool| {
        asked.borrow_mut().push((who, v, mounting));
        false
    };
    // seat-set boarding: vetoed.
    game_seat_set(&mut mobs, MobEvent::SeatSet { vehicle: 1, seat: 1, rider: Some(2) }, &mut no);
    assert!(claims(&mobs[0]).is_empty() && mobs[1].extra.mount.is_none());
    // Nothing would happen on a seat that does not exist: the mods are not asked.
    game_seat_set(&mut mobs, MobEvent::SeatSet { vehicle: 1, seat: 9, rider: Some(2) }, &mut no);
    assert_eq!(*asked.borrow(), [(2, 1, true)]);
    // AI boarding: the game plans, asks, and only then commits.
    let plan = riding::plan_board(&mobs, 3, 1, None, true).unwrap();
    assert_eq!(plan.seat, 0);
    assert!(!no(3, 1, true), "vetoed: no commit");
    assert!(claims(&mobs[0]).is_empty() && mobs[2].extra.mount.is_none());
    // Allowed: it boards. Then freeing its seat is vetoed too and it stays.
    riding::commit_board(&mut mobs, 3, 1, plan);
    assert_eq!(claims(&mobs[0]), [(3, 0)]);
    game_seat_set(&mut mobs, MobEvent::SeatSet { vehicle: 1, seat: 0, rider: None }, &mut no);
    assert_eq!(claims(&mobs[0]), [(3, 0)]);
    assert_eq!(asked.borrow().last(), Some(&(3, 1, false)));
    // Planning a refused boarding (cooldown) never asks.
    mobs[1].extra.boarding_cooldown = 5;
    assert_eq!(riding::plan_board(&mobs, 2, 1, None, true), Err(Refusal::Cooldown));
}

// ---- the save ----------------------------------------------------------------------------------------

/// The seats of the riders go into the `yoake` compound in the order of `Passengers`, and come back as the
/// claims of the riders as they load (ids change, the order does not).
#[test]
fn the_seats_of_the_riders_survive_a_save() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4)];
    riding::board(&mut mobs, 2, 1, Some(2), false).unwrap();
    riding::board(&mut mobs, 3, 1, None, false).unwrap();
    riding::board(&mut mobs, 4, 1, None, false).unwrap();
    assert_eq!(claims(&mobs[0]), [(3, 0), (4, 1), (2, 2)]);
    let mut tag = Tag::Compound(Compound::new());
    persist::write(&mut tag, &mobs[0]);
    persist::write_passenger_seats(&mut tag, &mobs[0], &[2, 3, 4]);
    assert_eq!(persist::read_passenger_seats(&tag), [2, 0, 1]);
    // A new world: the same riders under new ids, one that did not load.
    let mut loaded_vehicle = vehicle(&t);
    loaded_vehicle.id = 50;
    let read = persist::read_passenger_seats(&tag);
    let back = persist::claims_from(&read, &[Some(60), None, Some(62)]);
    assert_eq!(back, [SeatClaim { rider: 60, seat: 2 }, SeatClaim { rider: 62, seat: 1 }]);
    loaded_vehicle.extra.custom.as_mut().unwrap().set_seat_claims(back);
    assert_eq!(claims(&loaded_vehicle), [(62, 1), (60, 2)]);
    // They are riders of the loaded vehicle and the sync keeps their seats.
    let mut mobs = vec![loaded_vehicle, chicken(60), chicken(62)];
    mobs[1].extra.mount = Some(50);
    mobs[2].extra.mount = Some(50);
    sync(&mut mobs);
    assert_eq!(claims(&mobs[0]), [(62, 1), (60, 2)]);
    // A vehicle with nobody on it saves no list, and a stale one is removed.
    let mut empty = vehicle(&t);
    let mut tag = Tag::Compound(Compound::new());
    persist::write(&mut tag, &empty);
    persist::write_passenger_seats(&mut tag, &empty, &[]);
    assert!(persist::read_passenger_seats(&tag).is_empty());
    let mut stale = tag.clone();
    if let Some(Tag::Compound(y)) = stale.get_mut("yoake") {
        y.insert("passenger_seats".into(), Tag::List(vec![Tag::Int(1)]));
    }
    persist::write_passenger_seats(&mut stale, &empty, &[9]);
    assert!(persist::read_passenger_seats(&stale).is_empty());
    // A vanilla vehicle gets no `yoake` and no list.
    empty.extra.custom = None;
    let mut tag = Tag::Compound(Compound::new());
    persist::write_passenger_seats(&mut tag, &chicken(1), &[2]);
    assert_eq!(tag, Tag::Compound(Compound::new()));
}

/// The kinds the seats are chosen by: players are never a mob's kind, a custom mob is its type id.
#[test]
fn rider_kinds_name_the_type() {
    let t = seated(three());
    let f = vehicle(&t);
    assert!(matches!(riding::kind_of(&f), RiderKind::Mob { species: Species::Custom, custom: Some("sample_mobs:cinder_fox") }));
    assert!(matches!(riding::kind_of(&chicken(2)), RiderKind::Mob { species: Species::Chicken, custom: None }));
}

// ---- the host, the LAN guests and the mobs on one vehicle (reviewer decision 1) ---------------------

/// The host in the window, a LAN guest and mob riders never share a seat, whoever boards first.
#[test]
fn host_guest_and_mob_riders_never_share_a_seat() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3), chicken(4)];
    let held = |m: &Mob| -> Vec<u8> {
        let s = m.extra.custom.as_ref().unwrap();
        let mut v: Vec<u8> = s.seat_claims.iter().map(|c| c.seat).chain(s.player_claims.iter().map(|c| c.seat)).collect();
        v.sort_unstable();
        v
    };
    // The host takes the first free seat, a guest the next, then a chicken the last one; a fourth rider is refused.
    assert_eq!(riding::claim_player(&mut mobs, 1, P1, None), Ok(0));
    assert_eq!(riding::claim_player(&mut mobs, 1, P2, None), Ok(1));
    assert_eq!(riding::board(&mut mobs, 2, 1, None, false), Ok(2));
    assert_eq!(riding::board(&mut mobs, 3, 1, None, false), Err(Refusal::Full));
    assert_eq!(riding::claim_player(&mut mobs, 1, [3, 3, 3, 3], None), Err(Refusal::Full));
    assert_eq!(held(&mobs[0]), [0, 1, 2]);
    // Asking for a seat somebody holds is refused, for a player as for a mob.
    assert_eq!(riding::claim_player(&mut mobs, 1, [3, 3, 3, 3], Some(1)), Err(Refusal::Taken));
    assert_eq!(riding::board(&mut mobs, 3, 1, Some(0), false), Err(Refusal::Taken));
    // A player boarding again keeps its seat (nothing is given twice).
    assert_eq!(riding::claim_player(&mut mobs, 1, P2, None), Ok(1));
    assert_eq!(held(&mobs[0]), [0, 1, 2]);
    // The mobs are settled around the players every tick: the claims stay a partition.
    mobs[2].extra.mount = Some(1);
    sync(&mut mobs);
    assert_eq!(held(&mobs[0]).len(), 3, "the third chicken is put off, the three seats stay with their riders");
    let seats = held(&mobs[0]);
    assert!(seats.windows(2).all(|w| w[0] != w[1]), "{seats:?}");
    // The guest gets off: its seat is free for the next rider, and the host keeps its own.
    riding::release_player(&mut mobs[0], P2);
    assert_eq!(riding::seat_of_player(&mobs[0], P1), Some(0));
    assert_eq!(riding::board(&mut mobs, 3, 1, None, false), Ok(1));
}

/// A guest that disconnects (or dies, or gets off) frees its seat the same tick the game sees it is gone;
/// a rider that came without a claim takes the first free one, and one no seat is left for is sent off.
#[test]
fn a_disconnect_frees_the_seat() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t), chicken(2), chicken(3)];
    mobs[0].rider = Some(player());
    // The host and a guest ride; two chickens take the rest.
    assert!(riding::sync_players(&mut mobs, &[(P1, 1), (P2, 1)]).is_empty());
    assert_eq!(riding::seat_of_player(&mobs[0], P1), Some(0));
    assert_eq!(riding::seat_of_player(&mobs[0], P2), Some(1));
    assert_eq!(riding::board(&mut mobs, 2, 1, None, false), Ok(2));
    assert_eq!(riding::board(&mut mobs, 3, 1, None, false), Err(Refusal::Full));
    // The guest is gone: its seat is free, the host and the chicken keep theirs.
    assert!(riding::sync_players(&mut mobs, &[(P1, 1)]).is_empty());
    assert_eq!(riding::seat_of_player(&mobs[0], P2), None);
    assert_eq!(riding::seat_of_player(&mobs[0], P1), Some(0));
    assert_eq!(claims(&mobs[0]), [(2, 2)]);
    assert_eq!(riding::board(&mut mobs, 3, 1, None, false), Ok(1), "the freed seat");
    // Everyone gone: no claim is left and nothing steers.
    assert!(riding::sync_players(&mut mobs, &[]).is_empty());
    assert!(mobs[0].extra.custom.as_ref().unwrap().player_claims.is_empty());
    assert!(mobs[0].rider.is_none());
    // A player put on a full vehicle by something that did not ask is sent off, not squeezed in.
    sync(&mut mobs);
    assert_eq!(riding::sync_players(&mut mobs, &[(P1, 1), (P2, 1)]), [(P2, 1)]);
}

/// Two players cannot claim the same seat, also with `allowed_types` that keep the player out of some seats.
#[test]
fn players_obey_allowed_types_and_each_other() {
    let t = seated(json!([
        {"offset": [0.0, 1.0, 0.0], "controls": "player", "allowed_types": ["minecraft:chicken"]},
        {"offset": [0.0, 1.0, -0.8], "allowed_types": ["minecraft:player"]},
        {"offset": [0.0, 1.0, -1.6]}
    ]));
    let mut mobs = vec![vehicle(&t)];
    assert_eq!(riding::claim_player(&mut mobs, 1, P1, None), Ok(1), "seat 0 does not allow a player");
    assert_eq!(riding::claim_player(&mut mobs, 1, P2, None), Ok(2));
    assert_eq!(riding::claim_player(&mut mobs, 1, [3, 3, 3, 3], None), Err(Refusal::Full));
    assert_eq!(riding::claim_player(&mut mobs, 1, [3, 3, 3, 3], Some(0)), Err(Refusal::NotAllowed));
    assert_eq!(riding::claim_player(&mut mobs, 1, [3, 3, 3, 3], Some(7)), Err(Refusal::NoSuchSeat));
    // Only the seat that controls steers: neither player sits there.
    assert!(!riding::player_steers(&mobs[0], P1) && !riding::player_steers(&mobs[0], P2));
}

/// The seat of a player riding a saved vehicle comes back, and a claim made of a save does not take a
/// seat somebody holds now.
#[test]
fn the_seat_of_a_player_survives_a_save() {
    let t = seated(three());
    let mut mobs = vec![vehicle(&t)];
    riding::claim_player(&mut mobs, 1, P1, Some(1)).unwrap();
    riding::claim_player(&mut mobs, 1, P2, Some(2)).unwrap();
    let mut tag = Tag::Compound(Compound::new());
    persist::write(&mut tag, &mobs[0]);
    let saved = tag.get("yoake").and_then(|y| y.get("player_seats")).cloned();
    assert!(matches!(saved, Some(Tag::List(ref l)) if l.len() == 2), "{saved:?}");
    let _ = &mut tag;
    // Loaded again, nobody is aboard yet; the saved seats are what the players get when they board.
    let mut loaded = vehicle(&t);
    persist::read(&mut loaded, &tag);
    assert!(loaded.extra.custom.as_ref().unwrap().player_claims.is_empty());
    let mut mobs = vec![loaded];
    // P2 boards first and gets its own seat, not the first free one; P1 then gets seat 1.
    assert_eq!(riding::claim_player(&mut mobs, 1, P2, None), Ok(2));
    assert_eq!(riding::claim_player(&mut mobs, 1, P1, None), Ok(1));
}
