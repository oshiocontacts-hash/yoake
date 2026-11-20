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
//! * **Who sits where, with no stored state.** A seat is taken by what rides the mob now: the
//!   player (`Mob.rider`, which the game sets every tick the player rides) takes the first seat that
//!   allows the player, mobs that boarded it (`extra.mount`) take the next free seats that allow
//!   them, in id order. Nothing goes stale when a rider leaves or the mob dies. The engine links one
//!   mob rider per vehicle (`extra.rider`), so a type can seat the player and one mob at a time;
//!   more seats are for the layout and for the player-or-mob choice (open question in the report).
//! * **`allowed_types`**: entity type ids (`minecraft:player`, vanilla mobs, custom type ids); an
//!   empty list allows anything.
//! * **Controls**: a `player` seat steers the mob with the rider's input (the engine's ridden
//!   movement, see [`controlled`], wired into `equine::controlled`); `none` only carries.
//! * **Rideable**: the type has a free seat for the rider, the `saddle_item` (when the type names
//!   one) was put on it, a tameable type is tamed and the rider is its owner, and `rideable_when`
//!   (the goal rows' safe expression) holds.
use super::{TypeList, expr::When, type_list};
use crate::mobs::{
    Mob, MobWorld, Species,
    custom::CustomType,
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
    let mut taken = vec![false; d.seats.len()];
    let mut take = |kind: RiderKind| {
        let seat = d.seats.iter().enumerate().position(|(i, s)| !taken[i] && s.allows(kind));
        if let Some(i) = seat {
            taken[i] = true;
        }
        seat
    };
    let p = player.then(|| take(RiderKind::Player)).flatten();
    let rest = mobs.iter().map(|k| take(*k)).collect();
    (p, rest)
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

/// The rider's seat (the player's, by the rules above) and its definition.
pub fn player_seat(m: &Mob) -> Option<(usize, &SeatDef)> {
    let d = def(m)?;
    let (p, _) = assign(d, true, &[]);
    p.map(|i| (i, &d.seats[i]))
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
/// order, `rider` one of them) when the player is or is not aboard: its offset, for
/// `passengers.rs`.
pub fn mob_rider_attachment(vehicle: &Mob, rider: &Mob, aboard: &[&Mob], player_aboard: bool) -> [f64; 3] {
    let Some(d) = def(vehicle) else {
        return attachment(vehicle);
    };
    let kinds: Vec<RiderKind> = aboard.iter().map(|r| kind_of(r)).collect();
    let (_, seats) = assign(d, player_aboard, &kinds);
    let index = aboard.iter().position(|r| r.id == rider.id).unwrap_or(0);
    seats
        .get(index)
        .copied()
        .flatten()
        .map_or_else(|| attachment(vehicle), |s| d.seats[s].offset)
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
