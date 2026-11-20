//! The path budget of the custom mobs (FR-F4-MOBS.md section 8: "path search budgets").
//!
//! One search of a custom mob visits at most [`nav_select::MAX_VISITED_NODES`](super::nav_select)
//! nodes. On top of that, all custom mobs together may visit at most [`limit`] nodes in one game
//! tick (default [`DEFAULT_PATH_BUDGET`] = 8192, the server setting `horizon-custom-mob-path-budget`
//! of `server.properties`, `custom_mob_path_budget` of `server.toml`). A search that does not fit in
//! what is left of the tick is not made: the mob gets no path this tick (its goal asks again by its
//! own rules) and is put first in line for the next one.
//!
//! Fairness is round robin by refusal: the mobs refused in one tick are *waiting* in the next and are
//! served first; a mob that is not waiting may only search while enough is left for every waiting mob
//! that has not searched yet, and one that was served the two ticks before gives way while anybody
//! waits, so the turns go round and mobs late in the tick order are not starved by the early ones.
//! A search is charged the nodes it really visited.
//!
//! A refused mob does not ask again at once: it is put off by one tick plus a small jitter of 0 to
//! [`RETRY_JITTER`] more ticks, taken deterministically from its id and the tick (the same mobs in
//! the same ticks always get the same delays, so a test can rely on it), so that a crowd refused
//! in one tick does not come back, and get refused again, all in the same tick. While it is put
//! off its requests are refused without cost (`PathStats::backed_off`); when the delay is over it
//! is first in line like any mob refused last tick.
//!
//! The state is per thread (the server tick runs on one thread); the engine asks it through
//! [`request`] and [`charge`] from `Mob::query_path_raw`, only for custom mobs. [`stats`] reads
//! the counters for a debug view. Vanilla mobs never touch it.
use std::{
    cell::RefCell,
    sync::atomic::{AtomicI32, Ordering},
};

/// Nodes all custom mobs may visit in one tick, unless the server setting says otherwise.
pub const DEFAULT_PATH_BUDGET: i32 = 8192;
/// The most ticks a refused mob is put off beyond the one tick it always is.
pub const RETRY_JITTER: u32 = 3;
/// The server.properties key (Horizon's own; `server.toml`'s `custom_mob_path_budget`).
pub const PROPERTY: &str = "horizon-custom-mob-path-budget";
/// The most the setting may be.
pub const MAX_PATH_BUDGET: i32 = 1 << 20;

static LIMIT: AtomicI32 = AtomicI32::new(DEFAULT_PATH_BUDGET);

/// The budget of one tick, as set.
pub fn limit() -> i32 {
    LIMIT.load(Ordering::Relaxed)
}

/// Sets the budget of one tick (cut to 1 through [`MAX_PATH_BUDGET`]).
pub fn set_limit(nodes: i32) {
    LIMIT.store(nodes.clamp(1, MAX_PATH_BUDGET), Ordering::Relaxed);
}

/// [`PROPERTY`]'s value: [`DEFAULT_PATH_BUDGET`] when absent or unreadable, else cut to range.
pub fn limit_property(value: Option<&str>) -> i32 {
    value
        .and_then(|v| v.trim().parse::<i32>().ok())
        .map_or(DEFAULT_PATH_BUDGET, |n| n.clamp(1, MAX_PATH_BUDGET))
}

/// What the budget has done (a debug view reads this).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PathStats {
    /// Ticks in which a custom mob asked for a search.
    pub ticks: u64,
    /// Searches let through.
    pub granted: u64,
    /// Searches put off to the next tick.
    pub waited: u64,
    /// Nodes the granted searches visited.
    pub nodes: u64,
    /// The most nodes spent in one tick.
    pub busiest_tick: i32,
    /// Nodes spent in the tick that last asked.
    pub this_tick: i32,
    /// Requests refused without a look because the mob was still put off by a jittered delay.
    pub backed_off: u64,
}

/// The extra ticks (0 through `max`) a mob refused at `tick` waits: splitmix64 of the id and the tick.
pub fn jitter_for(id: u32, tick: i64, max: u32) -> u32 {
    let mut z = ((u64::from(id) << 32) ^ tick as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z % (u64::from(max) + 1)) as u32
}

/// The state of the budget: one tick's allowance and who waits.
#[derive(Clone, Debug)]
pub struct PathBudget {
    limit: i32,
    /// Take [`limit`] again at each tick (the global setting), or keep `limit`.
    follows_setting: bool,
    tick: Option<i64>,
    remaining: i32,
    /// Mobs refused last tick that have not searched yet this tick.
    waiting: Vec<u32>,
    /// Mobs refused this tick.
    refused: Vec<u32>,
    /// Mobs granted last tick, with how many ticks in a row they have been.
    granted_before: Vec<(u32, u32)>,
    /// Mobs granted this tick, likewise.
    granted_now: Vec<(u32, u32)>,
    /// Somebody was waiting when the tick began.
    contended: bool,
    /// Mobs put off by a jittered delay, with the first tick they may ask again.
    backoff: Vec<(u32, i64)>,
    /// The most extra ticks of delay.
    jitter: u32,
    stats: PathStats,
}

impl Default for PathBudget {
    fn default() -> Self {
        Self {
            limit: limit(),
            follows_setting: true,
            tick: None,
            remaining: 0,
            waiting: Vec::new(),
            refused: Vec::new(),
            granted_before: Vec::new(),
            granted_now: Vec::new(),
            contended: false,
            backoff: Vec::new(),
            jitter: RETRY_JITTER,
            stats: PathStats::default(),
        }
    }
}

impl PathBudget {
    /// A budget of its own (the setting does not change it).
    pub fn new(limit: i32) -> Self {
        Self {
            limit: limit.clamp(1, MAX_PATH_BUDGET),
            follows_setting: false,
            ..Self::default()
        }
    }

    /// The same budget with at most `max` extra ticks of retry delay (0: the next tick, always).
    pub fn with_jitter(mut self, max: u32) -> Self {
        self.jitter = max;
        self
    }

    pub fn limit(&self) -> i32 {
        self.limit
    }
    /// The first tick mob `id` may ask again, if it is put off now.
    pub fn retry_at(&self, id: u32) -> Option<i64> {
        self.backoff.iter().find(|(b, _)| *b == id).map(|(_, t)| *t)
    }
    pub fn remaining(&self) -> i32 {
        self.remaining
    }
    pub fn stats(&self) -> PathStats {
        self.stats
    }
    /// Mobs that are first in line this tick.
    pub fn waiting(&self) -> &[u32] {
        &self.waiting
    }

    fn roll(&mut self, tick: i64) {
        if self.tick == Some(tick) {
            return;
        }
        if self.follows_setting {
            self.limit = limit();
        }
        // Only the mobs refused in the tick just before wait (the tick may have been skipped
        // when nobody asked in it); older refusals are forgotten.
        let consecutive = self.tick.is_some_and(|t| tick - t <= 2);
        self.waiting = if consecutive {
            std::mem::take(&mut self.refused)
        } else {
            self.refused.clear();
            Vec::new()
        };
        self.granted_before = if consecutive {
            std::mem::take(&mut self.granted_now)
        } else {
            self.granted_now.clear();
            Vec::new()
        };
        // Mobs whose delay is over are first in line, as if they had been refused last tick.
        let mut i = 0;
        while i < self.backoff.len() {
            if self.backoff[i].1 <= tick {
                let (id, _) = self.backoff.swap_remove(i);
                if !self.waiting.contains(&id) {
                    self.waiting.push(id);
                }
            } else {
                i += 1;
            }
        }
        self.contended = !self.waiting.is_empty();
        self.tick = Some(tick);
        self.remaining = self.limit;
        self.stats.ticks += 1;
        self.stats.this_tick = 0;
    }

    /// Whether mob `id` may make a search of at most `cap` nodes at game time `tick`. A yes
    /// must be followed by [`Self::charge`] with the nodes it really visited.
    pub fn request(&mut self, id: u32, tick: i64, cap: i32) -> bool {
        self.roll(tick);
        if self.backoff.iter().any(|(b, _)| *b == id) {
            self.stats.backed_off += 1;
            return false;
        }
        let need = cap.clamp(1, self.limit);
        let first = self.waiting.iter().position(|w| *w == id);
        // What the mobs still waiting for their turn are owed.
        let owed = self.waiting.len() - usize::from(first.is_some());
        let reserve = i64::from(need) * owed as i64;
        let streak = self
            .granted_before
            .iter()
            .find(|(g, _)| *g == id)
            .map_or(0, |(_, n)| *n);
        // Mobs served two ticks running give way while others are waiting, so the turns go round.
        let yields = first.is_none() && self.contended && streak >= 2;
        let fits = if first.is_some() {
            self.remaining >= need
        } else {
            !yields && i64::from(self.remaining) - i64::from(need) >= reserve
        };
        if let Some(i) = first {
            self.waiting.remove(i);
        }
        if fits {
            self.granted_now.push((id, streak + 1));
            self.stats.granted += 1;
            true
        } else {
            self.stats.waited += 1;
            let extra = jitter_for(id, tick, self.jitter);
            if extra == 0 {
                if !self.refused.contains(&id) {
                    self.refused.push(id);
                }
            } else {
                self.backoff.push((id, tick + 1 + i64::from(extra)));
            }
            false
        }
    }

    /// Takes the nodes a granted search visited off this tick's allowance.
    pub fn charge(&mut self, nodes: i32) {
        let nodes = nodes.max(0);
        self.remaining = (self.remaining - nodes).max(0);
        self.stats.nodes += nodes as u64;
        self.stats.this_tick += nodes;
        self.stats.busiest_tick = self.stats.busiest_tick.max(self.stats.this_tick);
    }
}

/// What one custom type's mobs did with the budget (a debug view reads this).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TypeSearchStats {
    /// Searches let through.
    pub granted: u64,
    /// Searches put off to a later tick.
    pub waited: u64,
    /// Requests refused while the mob was put off by its jittered delay.
    pub backed_off: u64,
    /// Nodes the granted searches visited.
    pub nodes: u64,
}

thread_local! {
    static BUDGET: RefCell<PathBudget> = RefCell::new(PathBudget::default());
    static BY_TYPE: RefCell<std::collections::BTreeMap<String, TypeSearchStats>> = const { RefCell::new(std::collections::BTreeMap::new()) };
}

fn add_to_type(kind: &str, f: impl FnOnce(&mut TypeSearchStats)) {
    BY_TYPE.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(s) = m.get_mut(kind) {
            f(s);
        } else {
            let mut s = TypeSearchStats::default();
            f(&mut s);
            m.insert(kind.to_owned(), s);
        }
    });
}

/// [`request`] for a mob of custom type `kind`, counted for that type too.
pub fn request_for(kind: &str, id: u32, tick: i64, cap: i32) -> bool {
    let before = stats();
    let ok = request(id, tick, cap);
    let after = stats();
    add_to_type(kind, |s| {
        s.granted += after.granted - before.granted;
        s.waited += after.waited - before.waited;
        s.backed_off += after.backed_off - before.backed_off;
    });
    ok
}

/// [`charge`] for a mob of custom type `kind`, counted for that type too.
pub fn charge_for(kind: &str, nodes: i32) {
    charge(nodes);
    add_to_type(kind, |s| s.nodes += nodes.max(0) as u64);
}

/// This thread's counters by custom type id.
pub fn stats_by_type() -> std::collections::BTreeMap<String, TypeSearchStats> {
    BY_TYPE.with(|m| m.borrow().clone())
}

/// [`PathBudget::request`] on this thread's budget.
pub fn request(id: u32, tick: i64, cap: i32) -> bool {
    BUDGET.with(|b| b.borrow_mut().request(id, tick, cap))
}

/// [`PathBudget::charge`] on this thread's budget.
pub fn charge(nodes: i32) {
    BUDGET.with(|b| b.borrow_mut().charge(nodes));
}

/// This thread's counters.
pub fn stats() -> PathStats {
    BUDGET.with(|b| b.borrow().stats())
}

/// Sets this thread's most extra ticks of retry delay (tests).
pub fn set_retry_jitter(max: u32) {
    BUDGET.with(|b| b.borrow_mut().jitter = max);
}

/// Gives this thread a fresh budget of its own size (tests; the server uses the setting).
pub fn reset_for_thread(own_limit: Option<i32>) {
    BY_TYPE.with(|m| m.borrow_mut().clear());
    BUDGET.with(|b| {
        *b.borrow_mut() = own_limit.map_or_else(PathBudget::default, PathBudget::new);
    });
}
