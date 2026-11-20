//! Seats, saddle and rideability of a custom mob (FR-F4-MOBS.md section 5, F4M-4c).
//!
//! `seats [{ offset | locator, controls: none|player, allowed_types, yaw_lock }]`, `saddle_item`
//! and `rideable_when` of the type file become a [`RideDef`].
//!
//! * **Where a rider sits (server).** The server uses the seat's static `offset` (blocks from the
//!   mob's feet, `+z` ahead, turned with the mob's yaw): it is where the rider's *attachment point*
//!   sits, as vanilla's `passengerAttachments` are, so the engine subtracts the rider's own
//!   attachment (0.6 for a player). A seat with a `locator` and no offset sits at the locator's rest
//!   position in the model, worked out once when the type loads. The client's smooth seat (the
//!   locator of the animated pose) is F4M-6: [`seat_position`] takes the client's pose-driven
//!   position as `client_locator` and uses it instead of the static offset when given.
//! * **Who sits where (F4M-4d).** Any number of mobs may ride a vehicle with seats, up to its seat
//!   count (a vehicle of a type with no seats keeps the old one-rider link). Each mob rider's seat is
//!   a [`SeatClaim`] in the vehicle's `CustomState`: [`board`] gives it the seat asked for, or the
//!   first free seat that allows the rider, and [`settle`] (run every tick from
//!   `riders::sync_links`) turns riders that came without a claim (an old save, the `/ride` command)
//!   into claims and puts off riders no seat is left for. The player (`Mob.rider`, which the game
//!   sets every tick the player rides) takes its `player_seat` (what the boarding click picked) or the
//!   first seat nobody holds that allows the player. A rider keeps its seat when another gets off.
//!   `extra.rider` of the vehicle is the mob in the lowest seat (what the rest of the engine reads
//!   as "ridden"). Vanilla vehicles never get here: their one-rider links are untouched.
//! * **`allowed_types`**: entity type ids (`minecraft:player`, vanilla mobs, custom type ids); an
//!   empty list allows anything.
//! * **Controls**: a `player` seat steers the mob with the rider's input (the engine's ridden
//!   movement, see [`controlled`], wired into `equine::controlled`); `none` only carries. A mob
//!   rider steers (`riders::drive`, toward its own target) only from a `player` seat, see
//!   [`mob_may_steer`]; the mobs of the other seats are carried.
//! * **Rideable**: the type has a free seat for the rider, the `saddle_item` (when the type names
//!   one) was put on it, a tameable type is tamed and the rider is its owner, and `rideable_when`
//!   (the goal rows' safe expression) holds.
use super::{TypeList, expr::When, type_list};
use crate::mobs::{
    Mob, MobWorld, Species,
    custom::{CustomType, PlayerClaim, SeatClaim},
    riders,
};
use horizon_mods::custom_mob::{LoadedModel, Seat as SeatRow};

/// What a seat lets its rider do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Controls {
    /// The rider is only carried.
    None,
    /// A player on the seat steers.
    Player,
}

/// One seat, resolved.
#[derive(Clone, Debug)]
pub struct SeatDef {
    /// Where the rider's attachment point sits relative to the mob's feet (blocks; unturned).
    pub offset: [f64; 3],
    /// The model locator the client's seat follows, kept for F4M-6.
    pub locator: Option<String>,
    pub controls: Controls,
    /// Empty: anything may sit.
    pub allowed: TypeList,
    /// The rider faces where the mob faces.
    pub yaw_lock: bool,
}

/// What rides (or wants to ride) a mob.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiderKind<'a> {
    Player,
    Mob { species: Species, custom: Option<&'a str> },
}

impl SeatDef {
    /// Whether `rider` may sit here.
    pub fn allows(&self, rider: RiderKind) -> bool {
        if self.allowed.is_empty() {
            return true;
        }
        match rider {
            RiderKind::Player => self.allowed.player,
            RiderKind::Mob { species, custom } => {
                if let Some(c) = custom {
                    self.allowed.custom.iter().any(|t| &**t == c)
                } else {
                    self.allowed.species.contains(&species)
                }
            }
        }
    }
}

/// The riding data of a type.
#[derive(Debug, Default)]
pub struct RideDef {
    pub seats: Vec<SeatDef>,
    pub saddle_item: Option<String>,
    pub when: Option<When>,
}

impl RideDef {
    /// Resolves the file's `seats`, `saddle_item` and `rideable_when` (an expression that does not
    /// compile is an error of the file). `model` gives the rest position of a seat's locator.
    pub fn new(t: &CustomType, model: Option<&LoadedModel>) -> Result<Self, String> {
        let f = &t.file;
        let mut seats = Vec::new();
        for (i, s) in f.seats.iter().enumerate() {
            seats.push(resolve(i, s, t, model)?);
        }
        let when = f
            .rideable_when
            .as_deref()
            .map(|src| When::compile(t, src).map_err(|e| format!("rideable_when: {e}")))
            .transpose()?;
        Ok(Self {
            seats,
            saddle_item: f.saddle_item.clone(),
            when,
        })
    }
    pub fn is_rideable_type(&self) -> bool {
        !self.seats.is_empty()
    }
}

fn resolve(
    i: usize,
    s: &SeatRow,
    t: &CustomType,
    model: Option<&LoadedModel>,
) -> Result<SeatDef, String> {
    let offset = match (s.offset, &s.locator) {
        (Some(o), _) => o,
        (None, Some(name)) => match model.and_then(|m| m.locator_rest_position(name)) {
            Some(p) => crate::mobs::custom::hitbox::model_to_mob(p),
            None => [0.0, t.file.size.height, 0.0],
        },
        (None, None) => return Err(format!("seat {i}: an offset or a locator")),
    };
    Ok(SeatDef {
        offset,
        locator: s.locator.clone(),
        controls: if s.controls.as_deref() == Some("player") {
            Controls::Player
        } else {
            Controls::None
        },
        allowed: type_list(&s.allowed_types, &format!("seat {i} allowed_types")),
        yaw_lock: s.yaw_lock,
    })
}

fn def(m: &Mob) -> Option<&RideDef> {
    m.kind.custom.as_deref().map(|t| &t.ride).filter(|r| r.is_rideable_type())
}

/// The seat each rider takes: the player (when `player`) first, on the first seat that allows it,
/// then `mobs` in the order given on the next free seats that allow them. `None` for a rider with
/// no seat.
pub fn assign(d: &RideDef, player: bool, mobs: &[RiderKind]) -> (Option<usize>, Vec<Option<usize>>) {
    let mobs: Vec<(RiderKind, Option<usize>)> = mobs.iter().map(|k| (*k, None)).collect();
    let players: &[Option<usize>] = if player { &[None] } else { &[] };
    let (p, rest) = assign_with(d, players, &mobs);
    (p.first().copied().flatten(), rest)
}

/// [`assign`] with the seats riders already hold (F4M-4d) and any number of players (the host in the
/// window and the LAN guests). Every player comes with the seat it claimed (`None`: none yet), every mob
/// likewise. Claims are settled first, the mobs' and then the players': one that is out of range, taken or
/// not allowed for its rider is ignored. Riders without a seat then take the first free seat that allows
/// them, the players before the mobs (with no claims at all this is exactly [`assign`]). No seat is ever
/// given twice.
pub fn assign_with(
    d: &RideDef,
    players: &[Option<usize>],
    mobs: &[(RiderKind, Option<usize>)],
) -> (Vec<Option<usize>>, Vec<Option<usize>>) {
    let n = d.seats.len();
    let mut taken = vec![false; n];
    let mut seats: Vec<Option<usize>> = vec![None; mobs.len()];
    let mut ps: Vec<Option<usize>> = vec![None; players.len()];
    let settle = |taken: &mut Vec<bool>, kind: RiderKind, claim: Option<usize>| {
        let i = claim.filter(|&i| i < n && !taken[i] && d.seats[i].allows(kind))?;
        taken[i] = true;
        Some(i)
    };
    for (seat, (kind, claim)) in seats.iter_mut().zip(mobs) {
        *seat = settle(&mut taken, *kind, *claim);
    }
    for (seat, claim) in ps.iter_mut().zip(players) {
        *seat = settle(&mut taken, RiderKind::Player, *claim);
    }
    let first_free = |taken: &mut Vec<bool>, kind: RiderKind| {
        let i = d.seats.iter().enumerate().position(|(i, s)| !taken[i] && s.allows(kind))?;
        taken[i] = true;
        Some(i)
    };
    for seat in ps.iter_mut().filter(|s| s.is_none()) {
        *seat = first_free(&mut taken, RiderKind::Player);
    }
    for (seat, (kind, _)) in seats.iter_mut().zip(mobs) {
        if seat.is_none() {
            *seat = first_free(&mut taken, *kind);
        }
    }
    (ps, seats)
}

/// The seat a new rider of `kind` would get on `m` now, given the mob riders already aboard
/// (`aboard`, in id order) and whether the player is aboard. `None` when it has none.
pub fn free_seat(m: &Mob, kind: RiderKind, player_aboard: bool, aboard: &[RiderKind]) -> Option<usize> {
    let d = def(m)?;
    let mut all: Vec<RiderKind> = aboard.to_vec();
    let player = player_aboard || kind == RiderKind::Player;
    if kind != RiderKind::Player {
        all.push(kind);
    }
    let (p, rest) = assign(d, player, &all);
    if kind == RiderKind::Player {
        p
    } else {
        rest.last().copied().flatten()
    }
}

/// Whether `rider` (the player with this uuid, or a mob) may be put on `m` at all, apart from a free
/// seat: the saddle, ownership of a tameable type and `rideable_when`.
pub fn rideable(m: &Mob, w: &MobWorld, player: Option<[i32; 4]>) -> bool {
    let (Some(d), Some(t)) = (def(m), m.kind.custom.as_deref()) else {
        return false;
    };
    if !m.alive() {
        return false;
    }
    if d.saddle_item.is_some() && !m.extra.saddled {
        return false;
    }
    // A tameable type is ridden by its owner (a horse is not ridden wild).
    if t.tame.is_some() {
        match player {
            Some(uuid) if m.extra.tame && crate::mobs::tame::owned_by(m.extra.owner, uuid) => {}
            Some(_) => return false,
            // A mob rider: a tamed mount is no one's but its owner's.
            None if m.extra.tame => return false,
            None => {}
        }
    }
    d.when.as_ref().is_none_or(|e| e.eval(m, w))
}

/// Whether `item` is the saddle this type is saddled with and the mob lacks it.
pub fn takes_saddle(m: &Mob, item: Option<&str>) -> bool {
    let Some(d) = def(m) else { return false };
    d.saddle_item.as_deref().zip(item).is_some_and(|(s, i)| s == i) && !m.extra.saddled && m.alive()
}

/// The seat of the player riding `m` that steers it if one does, else the first player's, else the seat
/// a player would take now (a rider whose claim is not made yet), and its definition.
pub fn player_seat(m: &Mob) -> Option<(usize, &SeatDef)> {
    let d = def(m)?;
    let claims = m.extra.custom.as_ref().map_or(&[][..], |s| s.player_claims.as_slice());
    let i = claims
        .iter()
        .map(|c| usize::from(c.seat))
        .find(|&i| d.seats.get(i).is_some_and(|s| s.controls == Controls::Player))
        .or_else(|| claims.first().map(|c| usize::from(c.seat)))
        .or_else(|| free_player_seat(m, d, None))?;
    Some((i, d.seats.get(i)?))
}

/// The seat `player` sits on in `m`, by its claim.
pub fn seat_of_player(m: &Mob, player: [i32; 4]) -> Option<usize> {
    m.extra.custom.as_ref()?.player_claims.iter().find(|c| c.player == player).map(|c| usize::from(c.seat))
}

/// The seat `player` takes on `m` (or keeps): its claim, else the seat a loaded save gave it while that
/// is free, else the first seat nobody holds that allows a player. `None` when there is none. With no
/// `player` the claims of everyone aboard count.
fn free_player_seat(m: &Mob, d: &RideDef, player: Option<[i32; 4]>) -> Option<usize> {
    if let Some(i) = player.and_then(|p| seat_of_player(m, p)) {
        return Some(i);
    }
    let state = m.extra.custom.as_ref();
    let n = d.seats.len();
    let mut taken = vec![false; n];
    for i in state
        .iter()
        .flat_map(|s| s.seat_claims.iter().map(|c| c.seat).chain(s.player_claims.iter().map(|c| c.seat)))
    {
        if let Some(t) = taken.get_mut(usize::from(i)) {
            *t = true;
        }
    }
    let free = |i: usize| !taken[i] && d.seats[i].allows(RiderKind::Player);
    player
        .and_then(|p| state?.saved_player_seats.iter().find(|c| c.player == p))
        .map(|c| usize::from(c.seat))
        .filter(|&i| i < n && free(i))
        .or_else(|| (0..n).find(|&i| free(i)))
}

/// The seat `player` would get on `m` when it gets on now, the mobs and the players aboard counted by
/// their claims (the click that boards asks this; [`free_seat`] only knows the kinds of the riders).
pub fn free_seat_for_player(m: &Mob, player: [i32; 4]) -> Option<usize> {
    free_player_seat(m, def(m)?, Some(player))
}

/// Puts `player` on seat `seat` of the custom vehicle `vehicle` (the first free seat that allows it when
/// `seat` is `None`) and returns the seat. A player already aboard keeps its claim (unless it asks
/// for another free seat). No seat is ever given twice: the mobs aboard and the other players hold
/// theirs.
pub fn claim_player(mobs: &mut [Mob], vehicle: u32, player: [i32; 4], seat: Option<usize>) -> Result<usize, Refusal> {
    let Some(v) = mobs.iter_mut().find(|m| m.id == vehicle) else { return Err(Refusal::Missing) };
    let Some(d) = def(v) else { return Err(Refusal::NoSeats) };
    let n = d.seats.len();
    let mut taken = vec![false; n];
    if let Some(state) = v.extra.custom.as_ref() {
        for c in &state.seat_claims {
            if let Some(t) = taken.get_mut(usize::from(c.seat)) {
                *t = true;
            }
        }
        for c in state.player_claims.iter().filter(|c| c.player != player) {
            if let Some(t) = taken.get_mut(usize::from(c.seat)) {
                *t = true;
            }
        }
    }
    let own = seat_of_player(v, player);
    let chosen = match seat {
        Some(s) if s >= n => return Err(Refusal::NoSuchSeat),
        Some(s) if !d.seats[s].allows(RiderKind::Player) => return Err(Refusal::NotAllowed),
        Some(s) if taken[s] => return Err(Refusal::Taken),
        Some(s) => s,
        None if own.is_some() => own.unwrap_or(0),
        None => match free_player_seat(v, d, Some(player)) {
            Some(i) => i,
            None if (0..n).any(|i| d.seats[i].allows(RiderKind::Player)) => return Err(Refusal::Full),
            None => return Err(Refusal::NotAllowed),
        },
    };
    let Some(state) = v.extra.custom.as_mut() else { return Err(Refusal::NoSeats) };
    let mut claims: Vec<PlayerClaim> = state.player_claims.iter().copied().filter(|c| c.player != player).collect();
    claims.push(PlayerClaim { player, seat: u8::try_from(chosen).map_err(|_| Refusal::NoSuchSeat)? });
    state.saved_player_seats.retain(|c| c.player != player);
    state.set_player_claims(claims);
    Ok(chosen)
}

/// Makes the player claims of every custom vehicle with seats match who rides (`riders`: each player's UUID
/// and the vehicle it rides; the game's `riding_players`): the claim of a player that does not ride the
/// vehicle any more (it got off, disconnected, died) is freed, a rider without a claim takes one, and a vehicle
/// nobody steers from forgets its rider input. Returns the players no seat was left for (they must get
/// off).
pub fn sync_players(mobs: &mut [Mob], riders: &[([i32; 4], u32)]) -> Vec<([i32; 4], u32)> {
    let mut evict = Vec::new();
    let seated: Vec<u32> = mobs.iter().filter(|m| has_seats(m)).map(|m| m.id).collect();
    for id in seated {
        let aboard: Vec<[i32; 4]> = riders.iter().filter(|r| r.1 == id).map(|r| r.0).collect();
        let Some(vi) = mobs.iter().position(|m| m.id == id) else { continue };
        let held: Vec<[i32; 4]> = mobs[vi]
            .extra
            .custom
            .as_ref()
            .map_or_else(Vec::new, |s| s.player_claims.iter().map(|c| c.player).collect());
        for p in held.into_iter().filter(|p| !aboard.contains(p)) {
            release_player(&mut mobs[vi], p);
        }
        for p in aboard {
            if seat_of_player(&mobs[vi], p).is_none() && claim_player(mobs, id, p, None).is_err() {
                evict.push((p, id));
            }
        }
        // The mob takes its input from a player that steers; with none it has no rider input.
        let m = &mut mobs[vi];
        let steered = m
            .extra
            .custom
            .as_ref()
            .is_some_and(|s| s.player_claims.iter().any(|c| seat_of_player(m, c.player).is_some() && player_steers(m, c.player)));
        if m.rider.is_some() && !steered {
            m.rider = None;
        }
    }
    evict
}

/// Frees the seat `player` held on `m`.
pub fn release_player(m: &mut Mob, player: [i32; 4]) {
    if let Some(state) = m.extra.custom.as_mut()
        && state.player_claims.iter().any(|c| c.player == player)
    {
        let claims = state.player_claims.iter().copied().filter(|c| c.player != player).collect();
        state.set_player_claims(claims);
    }
}

/// Whether `player` sits (or would sit) on a seat of `m` that steers (`controls: player`).
pub fn player_steers(m: &Mob, player: [i32; 4]) -> bool {
    let Some(d) = def(m) else { return true };
    let i = seat_of_player(m, player).or_else(|| free_player_seat(m, d, Some(player)));
    i.and_then(|i| d.seats.get(i)).is_some_and(|s| s.controls == Controls::Player)
}

/// The seat claimed by mob `rider` on `vehicle`.
pub fn claim_of(vehicle: &Mob, rider: u32) -> Option<usize> {
    vehicle
        .extra
        .custom
        .as_ref()?
        .seat_claims
        .iter()
        .find(|c| c.rider == rider)
        .map(|c| usize::from(c.seat))
}

/// Whether `m` is a custom mob whose type has seats: the vehicles F4M-4d seats several mobs on.
pub fn has_seats(m: &Mob) -> bool {
    def(m).is_some()
}

/// Whether the mob riding `vehicle` (`extra.rider`) steers it: a mob steers from a `player` seat
/// only, and a rider whose seat is not known yet (the tick it boarded) as before.
pub fn mob_may_steer(vehicle: &Mob) -> bool {
    let (Some(d), Some(rider)) = (def(vehicle), vehicle.extra.rider) else {
        return true;
    };
    claim_of(vehicle, rider).and_then(|i| d.seats.get(i)).is_none_or(|s| s.controls == Controls::Player)
}

/// Where a rider's attachment point sits for seat `seat`: the mob's position plus the seat's
/// offset turned with `yaw` (the engine's `Entity.positionRider` then subtracts the rider's own
/// attachment). `client_locator` is the smooth, pose-driven seat position in the mob's frame
/// (blocks, unturned) that F4M-6's renderer works out from the animated skeleton; the server passes
/// `None` and gets the static offset.
pub fn seat_position(
    d: &RideDef,
    seat: usize,
    at: horizon_core::geometry::Vec3,
    yaw: f32,
    client_locator: Option<[f64; 3]>,
) -> horizon_core::geometry::Vec3 {
    let off = client_locator.unwrap_or_else(|| d.seats.get(seat).map_or([0.0; 3], |s| s.offset));
    riders::seat(off, 0.0, at, yaw)
}

/// `riders::passenger_attachment` of a custom mob: the first seat's static offset, or the top of
/// its box when it has no seat.
pub fn attachment(m: &Mob) -> [f64; 3] {
    match def(m).and_then(|d| d.seats.first()) {
        Some(s) => s.offset,
        None => [0.0, f64::from(m.height()), 0.0],
    }
}

/// The attachment of seat `i` as a passenger mob would use it (`game/passengers.rs`).
pub fn attachment_at(m: &Mob, seat: usize) -> [f64; 3] {
    match def(m).and_then(|d| d.seats.get(seat)) {
        Some(s) => s.offset,
        None => attachment(m),
    }
}

/// The seat of the mob rider `rider` among the mobs aboard `vehicle` (`aboard`, the riders in id
/// order, `rider` one of them): its offset, for `passengers.rs`. The players aboard hold their claims.
pub fn mob_rider_attachment(vehicle: &Mob, rider: &Mob, aboard: &[&Mob]) -> [f64; 3] {
    let Some(d) = def(vehicle) else {
        return attachment(vehicle);
    };
    mob_rider_seat(vehicle, rider, aboard)
        .map_or_else(|| attachment(vehicle), |s| d.seats[s].offset)
}

/// The seats the players aboard `m` claimed, in claim order.
fn player_seats(m: &Mob) -> Vec<Option<usize>> {
    m.extra.custom.as_ref().map_or_else(Vec::new, |s| s.player_claims.iter().map(|c| Some(usize::from(c.seat))).collect())
}

/// The seat of mob `rider` among the mobs `aboard` `vehicle` (in list order): the seat it claimed, or
/// the first free one that allows it, the players (by their claims) counted.
pub fn mob_rider_seat(vehicle: &Mob, rider: &Mob, aboard: &[&Mob]) -> Option<usize> {
    let d = def(vehicle)?;
    let kinds: Vec<(RiderKind, Option<usize>)> =
        aboard.iter().map(|r| (kind_of(r), claim_of(vehicle, r.id))).collect();
    let (_, seats) = assign_with(d, &player_seats(vehicle), &kinds);
    let index = aboard.iter().position(|r| r.id == rider.id).unwrap_or(0);
    seats.get(index).copied().flatten()
}

/// Whether a rider on seat `seat` of `vehicle` faces where the vehicle faces (`yaw_lock`).
pub fn seat_locks_yaw(vehicle: &Mob, seat: usize) -> bool {
    def(vehicle).and_then(|d| d.seats.get(seat)).is_some_and(|s| s.yaw_lock)
}

/// Why [`board`] put nobody on a seat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The rider or the vehicle is not in the list.
    Missing,
    /// The vehicle is no custom mob with seats.
    NoSeats,
    /// A mob does not ride itself.
    Itself,
    /// The vehicle rides the rider (through any number of vehicles).
    Loop,
    /// The rider got off something less than 60 ticks ago (only asked for when the rider boards by
    /// its own will).
    Cooldown,
    /// The seat index is past the type's seats.
    NoSuchSeat,
    /// The seat's `allowed_types` do not list the rider (or no seat does).
    NotAllowed,
    /// The seat asked for is taken.
    Taken,
    /// Every seat that allows the rider is taken.
    Full,
}

/// Puts mob `rider` on seat `seat` of the custom vehicle `vehicle` (the first free seat that allows it
/// when `seat` is `None`) and returns the seat. The players and the mobs already aboard
/// keep their seats. A rider that rides another vehicle is taken off it by the link, as `/ride`'s
/// forced mount does; a rider already aboard moves to the seat asked for. `honour_cooldown` refuses a
/// rider that got off something less than 60 ticks ago (the AI's boarding; commands and scripts force it).
pub fn board(
    mobs: &mut [Mob],
    rider: u32,
    vehicle: u32,
    seat: Option<usize>,
    honour_cooldown: bool,
) -> Result<usize, Refusal> {
    let plan = plan_board(mobs, rider, vehicle, seat, honour_cooldown)?;
    Ok(commit_board(mobs, rider, vehicle, plan))
}

/// What [`plan_board`] worked out: the seat and the claims of the riders already aboard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardPlan {
    pub seat: usize,
    claims: Vec<SeatClaim>,
}

/// The checks of [`board`] without changing anything: the game asks the native mods (`native_mob_may_ride`)
/// between this and [`commit_board`], so a boarding no mod may veto is the one that is allowed.
pub fn plan_board(
    mobs: &[Mob],
    rider: u32,
    vehicle: u32,
    seat: Option<usize>,
    honour_cooldown: bool,
) -> Result<BoardPlan, Refusal> {
    if rider == vehicle {
        return Err(Refusal::Itself);
    }
    let find = |mobs: &[Mob], id: u32| mobs.iter().position(|m| m.id == id);
    let (Some(ri), Some(vi)) = (find(mobs, rider), find(mobs, vehicle)) else {
        return Err(Refusal::Missing);
    };
    if !has_seats(&mobs[vi]) {
        return Err(Refusal::NoSeats);
    }
    // Nothing that rides the rider (through any number of vehicles) can be its vehicle.
    let mut up = mobs[vi].extra.mount;
    for _ in 0..64 {
        let Some(v) = up else { break };
        if v == rider {
            return Err(Refusal::Loop);
        }
        up = mobs.iter().find(|m| m.id == v).and_then(|m| m.extra.mount);
    }
    if honour_cooldown && mobs[ri].extra.boarding_cooldown > 0 {
        return Err(Refusal::Cooldown);
    }
    let (chosen, mut claims) = {
        let v = &mobs[vi];
        let Some(d) = def(v) else { return Err(Refusal::NoSeats) };
        let n = d.seats.len();
        let aboard: Vec<&Mob> = mobs
            .iter()
            .filter(|m| m.extra.mount == Some(vehicle) && m.id != rider && m.alive())
            .collect();
        let existing: Vec<(RiderKind, Option<usize>)> =
            aboard.iter().map(|m| (kind_of(m), claim_of(v, m.id))).collect();
        let (p, rest) = assign_with(d, &player_seats(v), &existing);
        let mut taken = vec![false; n];
        for i in p.into_iter().flatten().chain(rest.iter().copied().flatten()) {
            taken[i] = true;
        }
        let kind = kind_of(&mobs[ri]);
        let chosen = match seat {
            Some(s) if s >= n => return Err(Refusal::NoSuchSeat),
            Some(s) if !d.seats[s].allows(kind) => return Err(Refusal::NotAllowed),
            Some(s) if taken[s] => return Err(Refusal::Taken),
            Some(s) => s,
            None => match (0..n).find(|&i| !taken[i] && d.seats[i].allows(kind)) {
                Some(i) => i,
                None if (0..n).any(|i| d.seats[i].allows(kind)) => return Err(Refusal::Full),
                None => return Err(Refusal::NotAllowed),
            },
        };
        let claims: Vec<SeatClaim> = aboard
            .iter()
            .zip(&rest)
            .filter_map(|(m, s)| Some(SeatClaim { rider: m.id, seat: u8::try_from((*s)?).ok()? }))
            .collect();
        (chosen, claims)
    };
    claims.push(SeatClaim { rider, seat: u8::try_from(chosen).map_err(|_| Refusal::NoSuchSeat)? });
    Ok(BoardPlan { seat: chosen, claims })
}

/// Does what [`plan_board`] planned (nothing may have changed in between) and returns the seat.
pub fn commit_board(mobs: &mut [Mob], rider: u32, vehicle: u32, plan: BoardPlan) -> usize {
    if let Some(ri) = mobs.iter().position(|m| m.id == rider) {
        mobs[ri].extra.mount = Some(vehicle);
        mobs[ri].extra.passenger = true;
    }
    if let Some(vi) = mobs.iter().position(|m| m.id == vehicle) {
        store_claims(&mut mobs[vi], plan.claims);
    }
    plan.seat
}

/// What a script's `seat-set` is about to do, for the mods to veto first: the mob that boards (`true`) or
/// is taken off (`false`) and the vehicle. `None` when it would change nothing (the seat is taken, the
/// rider is not allowed, nobody sits there ...).
pub fn plan_seat_set(mobs: &[Mob], vehicle: u32, seat: usize, rider: Option<u32>) -> Option<(u32, bool)> {
    match rider {
        Some(r) => plan_board(mobs, r, vehicle, Some(seat), false).ok().map(|_| (r, true)),
        None => seat_occupant(mobs, vehicle, seat).map(|r| (r, false)),
    }
}

/// The mob that sits on seat `seat` of `vehicle`.
pub fn seat_occupant(mobs: &[Mob], vehicle: u32, seat: usize) -> Option<u32> {
    mobs.iter()
        .find(|m| m.id == vehicle)?
        .extra
        .custom
        .as_ref()?
        .seat_claims
        .iter()
        .find(|c| usize::from(c.seat) == seat)
        .map(|c| c.rider)
}

/// Sets the claims of `vehicle` and its `extra.rider` (the mob on the lowest seat, or none).
fn store_claims(vehicle: &mut Mob, claims: Vec<SeatClaim>) {
    let Some(state) = vehicle.extra.custom.as_mut() else { return };
    state.set_seat_claims(claims);
    vehicle.extra.rider = state.seat_claims.first().map(|c| c.rider);
}

/// Brings the seats of the custom vehicle at `vi` in order: riders without a claim get one (the first free
/// seat that allows them), riders no seat is left for or that no seat allows get off, claims of what no
/// longer rides it go, `extra.rider` is the mob on the lowest seat. The player (`Mob.rider`) is counted.
/// Does nothing for a vehicle without seats.
pub fn settle(mobs: &mut [Mob], vi: usize) {
    if !has_seats(&mobs[vi]) {
        return;
    }
    let vehicle = mobs[vi].id;
    let (riders, seats, players) = {
        let v = &mobs[vi];
        let Some(d) = def(v) else { return };
        let aboard: Vec<usize> = (0..mobs.len()).filter(|&j| j != vi && mobs[j].extra.mount == Some(vehicle)).collect();
        let kinds: Vec<(RiderKind, Option<usize>)> =
            aboard.iter().map(|&j| (kind_of(&mobs[j]), claim_of(v, mobs[j].id))).collect();
        let (pseats, seats) = assign_with(d, &player_seats(v), &kinds);
        let who = v.extra.custom.as_ref().map_or_else(Vec::new, |s| s.player_claims.iter().map(|c| c.player).collect::<Vec<_>>());
        let players: Vec<PlayerClaim> = who
            .into_iter()
            .zip(pseats)
            .filter_map(|(player, s)| Some(PlayerClaim { player, seat: u8::try_from(s?).ok()? }))
            .collect();
        (aboard, seats, players)
    };
    let mut claims = Vec::new();
    for (j, seat) in riders.into_iter().zip(seats) {
        match seat.and_then(|s| u8::try_from(s).ok()) {
            Some(seat) => claims.push(SeatClaim { rider: mobs[j].id, seat }),
            None => {
                // No seat for it: off (it was loaded with more riders than seats, or the type changed).
                mobs[j].extra.mount = None;
                mobs[j].extra.passenger = false;
            }
        }
    }
    if let Some(state) = mobs[vi].extra.custom.as_mut() {
        state.set_player_claims(players);
    }
    store_claims(&mut mobs[vi], claims);
}

/// [`settle`] for every custom vehicle with seats in `mobs`.
pub fn settle_all(mobs: &mut [Mob]) {
    for vi in 0..mobs.len() {
        if mobs[vi].kind.custom.is_some() {
            settle(mobs, vi);
        }
    }
}

/// What a script's `seat-set(vehicle, seat, rider)` does: `Some(rider)` puts that mob on the seat, `None`
/// takes whoever sits there off. A refusal is logged at debug level and changes nothing.
pub fn seat_set(mobs: &mut [Mob], vehicle: u32, seat: usize, rider: Option<u32>) {
    let Some(vi) = mobs.iter().position(|m| m.id == vehicle) else { return };
    match rider {
        Some(rider) => {
            if let Err(why) = board(mobs, rider, vehicle, Some(seat), false) {
                log::debug!("seat-set: mob {rider} cannot sit on seat {seat} of {vehicle}: {why:?}");
            }
        }
        None => {
            let Some(who) = mobs[vi]
                .extra
                .custom
                .as_ref()
                .and_then(|c| c.seat_claims.iter().find(|c| usize::from(c.seat) == seat).map(|c| c.rider))
            else {
                return;
            };
            if let Some(m) = mobs.iter_mut().find(|m| m.id == who) {
                riders::stop_riding(m, None);
            }
            settle(mobs, vi);
        }
    }
}

/// What a mob is as a rider.
pub fn kind_of(m: &Mob) -> RiderKind<'_> {
    RiderKind::Mob {
        species: m.species(),
        custom: m.kind.custom.as_deref().map(|t| t.id.as_str()),
    }
}

/// Whether the player riding `m` steers it: a rider is aboard (`Mob.rider`) on a seat that
/// controls, and the saddle (when the type has one) is on. Wired into `equine::controlled`.
pub fn controlled(m: &Mob) -> bool {
    m.rider.is_some()
        && m.extra.custom.is_some()
        && player_seat(m).is_some_and(|(_, s)| s.controls == Controls::Player)
        && def(m).is_some_and(|d| d.saddle_item.is_none() || m.extra.saddled)
}

/// The yaw the player riding `m` is locked to (its seat's `yaw_lock`), if any.
pub fn locked_yaw(m: &Mob) -> Option<f32> {
    player_seat(m).filter(|(_, s)| s.yaw_lock).map(|_| m.yaw)
}

/// Leaving the mob forgets nothing here (the seats are derived); this only says whether `m` is
/// the kind of mob the dismount branch of the game should look at.
pub fn is_rideable_mob(m: &Mob) -> bool {
    def(m).is_some()
}
