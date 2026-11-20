//! Right click on a custom mob: the `mob-interact` plumbing (FR-F4-MOBS.md sections 5 and 6.2,
//! F4M-4c).
//!
//! `mob-interact(mob, player, hand, item) -> consume | pass | success`. The WASM hook itself is
//! F4M-10; this is the native dispatch point it plugs into, and the data shortcuts that run when no
//! hook is installed or the hook passes:
//!
//! 1. the **hook** ([`Hook`], installed with [`set_hook`], or passed to [`dispatch`] by a test):
//!    `Consume` (handled, one of the held item is used up) and `Success` (handled, nothing used up)
//!    end the interaction; `Pass` goes on;
//! 2. the type's **`interact` rows**, in order: `feed`, `tame`, `ride`, `give_item`;
//! 3. the **implicit shortcuts** of the type's data, in this order: put on the `saddle_item`; `tame`
//!    with a `tameable.items` item; `feed` with `breedable.food`; the owner's click on a tamed mob
//!    that `sits_on_command` makes it sit or stand; `ride` when the type has seats and the player
//!    may sit on one.
//!
//! [`dispatch`] changes the mob (owner, love, sitting) and returns the [`Outcome`]: the reply and the
//! [`Action`]s only the game can do (use up the item, give an item, seat the player, show hearts).
//! `Pass` means nothing happened and the engine goes on with its own handlers (name tag, lead...).
use super::{breed, riding, tame};
use crate::mobs::Mob;
use horizon_mods::custom_mob::Interact;
use std::sync::{Arc, RwLock};

/// Which hand clicked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hand {
    Main,
    Off,
}

/// The answer of `mob-interact`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    /// Handled; one of the held item is used up.
    Consume,
    /// Not handled.
    Pass,
    /// Handled; nothing is used up.
    Success,
}

/// What the game must do after a handled interaction.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Put the saddle on (the game takes the stack from the player).
    Saddle,
    /// Seat the player on seat `seat`.
    Ride { seat: usize },
    /// Give the player `count` of `item`.
    Give { item: String, count: u32 },
    /// An entity event the clients show: 7 hearts (tamed), 6 smoke (the taming failed), 18 love
    /// hearts (fed).
    Event(u8),
    /// The eating sound.
    Eat,
}

/// What a right click is, as the dispatch needs it.
#[derive(Clone, Copy)]
pub struct Context<'a> {
    /// The player's uuid (what an owner is saved as).
    pub player: [i32; 4],
    pub hand: Hand,
    /// The held item's id; `None` for an empty hand.
    pub item: Option<&'a str>,
    pub sneaking: bool,
    /// Whether an item id matches a row's pattern (an id, or `#tag` resolved by the game).
    pub matches: &'a dyn Fn(&str, &str) -> bool,
    /// The seat the player would take now, or `None` when the player cannot ride it
    /// ([`riding::rideable`] and [`riding::free_seat`], worked out by the game, which has the world).
    pub ride_seat: Option<usize>,
}

/// The result of an interaction.
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub reply: Reply,
    pub actions: Vec<Action>,
}

impl Outcome {
    pub fn pass() -> Self {
        Self { reply: Reply::Pass, actions: Vec::new() }
    }
    fn done(reply: Reply, actions: Vec<Action>) -> Self {
        Self { reply, actions }
    }
}

/// The scripts' `mob-interact` (F4M-10). A hook is told the mob and the click and answers.
pub trait Hook: Send + std::marker::Sync {
    fn mob_interact(&self, mob: &mut Mob, ctx: &Context) -> Reply;
}

static HOOK: RwLock<Option<Arc<dyn Hook>>> = RwLock::new(None);

/// Installs (or, with `None`, removes) the `mob-interact` hook; returns the one it replaced.
pub fn set_hook(h: Option<Arc<dyn Hook>>) -> Option<Arc<dyn Hook>> {
    match HOOK.write() {
        Ok(mut slot) => std::mem::replace(&mut *slot, h),
        Err(_) => None,
    }
}

/// [`dispatch`] with the installed hook.
pub fn dispatch_global(m: &mut Mob, ctx: &Context) -> Outcome {
    let hook = HOOK.read().ok().and_then(|h| h.clone());
    dispatch(m, ctx, hook.as_deref())
}

/// The whole interaction: the hook, then the rows, then the implicit shortcuts.
pub fn dispatch(m: &mut Mob, ctx: &Context, hook: Option<&dyn Hook>) -> Outcome {
    let Some(t) = m.kind.custom.clone() else {
        return Outcome::pass();
    };
    if !m.alive() {
        return Outcome::pass();
    }
    if let Some(h) = hook {
        match h.mob_interact(m, ctx) {
            r @ (Reply::Consume | Reply::Success) => return Outcome::done(r, Vec::new()),
            Reply::Pass => {}
        }
    }
    for row in &t.file.interact {
        if row_applies(row, ctx)
            && let Some(o) = run_row(m, row, ctx)
        {
            return o;
        }
    }
    implicit(m, ctx)
}

fn row_applies(row: &Interact, ctx: &Context) -> bool {
    if row.sneaking.is_some_and(|s| s != ctx.sneaking) {
        return false;
    }
    match (&row.item, ctx.item) {
        (None, _) => true,
        (Some(p), Some(i)) => (ctx.matches)(p, i),
        (Some(_), None) => false,
    }
}

fn run_row(m: &mut Mob, row: &Interact, ctx: &Context) -> Option<Outcome> {
    // An explicit `item` overrides what the action takes by default.
    let explicit = row.item.is_some();
    let used = |default: bool| if row.consume.unwrap_or(default) { Reply::Consume } else { Reply::Success };
    match row.action.as_str() {
        "give_item" => {
            let item = row.give.clone()?;
            Some(Outcome::done(
                used(true),
                vec![Action::Give { item, count: row.count.unwrap_or(1).clamp(1, 64) }],
            ))
        }
        "tame" => {
            if tame::is_tamed(m) || !(explicit || tame::is_tame_item(m, ctx.item, ctx.matches)) {
                return None;
            }
            Some(tame_attempt(m, ctx, used(true)))
        }
        "feed" => {
            if !(explicit || breed::is_food(m, ctx.item, ctx.matches)) {
                return None;
            }
            feed_attempt(m, ctx, used(true))
        }
        "ride" => ride(ctx).map(|o| Outcome { reply: used(false), ..o }),
        _ => None,
    }
}

fn implicit(m: &mut Mob, ctx: &Context) -> Outcome {
    if riding::takes_saddle(m, ctx.item) {
        return Outcome::done(Reply::Consume, vec![Action::Saddle]);
    }
    if !tame::is_tamed(m) && tame::is_tame_item(m, ctx.item, ctx.matches) {
        return tame_attempt(m, ctx, Reply::Consume);
    }
    if breed::is_food(m, ctx.item, ctx.matches)
        && let Some(o) = feed_attempt(m, ctx, Reply::Consume)
    {
        return o;
    }
    // The owner's order to sit or stand: a click that is not food.
    if tame::is_tamed(m)
        && crate::mobs::tame::owned_by(m.extra.owner, ctx.player)
        && !ctx.sneaking
        && let Some(on) = tame::toggle_sit(m)
    {
        let _ = on;
        return Outcome::done(Reply::Success, Vec::new());
    }
    if !ctx.sneaking
        && let Some(o) = ride(ctx)
    {
        return o;
    }
    Outcome::pass()
}

fn tame_attempt(m: &mut Mob, ctx: &Context, reply: Reply) -> Outcome {
    let ok = tame::try_tame(m, ctx.player);
    Outcome::done(reply, vec![Action::Event(if ok { 7 } else { 6 })])
}

fn feed_attempt(m: &mut Mob, ctx: &Context, reply: Reply) -> Option<Outcome> {
    match breed::feed(m, Some(ctx.player)) {
        breed::Fed::Love => Some(Outcome::done(reply, vec![Action::Eat, Action::Event(18)])),
        breed::Fed::Grew => Some(Outcome::done(reply, vec![Action::Eat])),
        breed::Fed::No => None,
    }
}

fn ride(ctx: &Context) -> Option<Outcome> {
    ctx.ride_seat
        .map(|seat| Outcome::done(Reply::Success, vec![Action::Ride { seat }]))
}
