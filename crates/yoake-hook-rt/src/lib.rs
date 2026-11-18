use std::any::Any;
use std::cmp::Reverse;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};

pub const MAX_SLOTS: usize = 4096;
const MAX_ENTRIES_PER_SLOT: usize = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Order {
    pub priority: i32,
    pub mod_load_order: u32,
}

#[derive(Clone)]
struct Entry {
    order: Order,
    sequence: usize,
    callback: Arc<dyn Any + Send + Sync>,
}

struct Chain {
    entries: Vec<Entry>,
}

#[repr(transparent)]
pub struct Slot(AtomicPtr<Chain>);

impl Default for Slot {
    fn default() -> Self {
        Self::new()
    }
}

impl Slot {
    pub const fn new() -> Self {
        Self(AtomicPtr::new(std::ptr::null_mut()))
    }

    #[inline(always)]
    pub fn load(&'static self) -> Option<Snapshot> {
        let pointer = self.0.load(Ordering::Acquire);
        if pointer.is_null() {
            None
        } else {
            // SAFETY: `pointer` is published with Release from `Box::into_raw`
            // in `register`, and old `Chain` snapshots are intentionally leaked
            // for the process lifetime so lock-free readers never observe freed memory.
            Some(Snapshot(unsafe { &(*pointer).entries }))
        }
    }
}

const EMPTY: Slot = Slot::new();
pub static SLOTS: [Slot; MAX_SLOTS] = [EMPTY; MAX_SLOTS];

static REGISTER_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Copy)]
pub struct Snapshot(&'static [Entry]);

impl Snapshot {
    #[cold]
    #[inline(never)]
    pub fn next<Callback: Copy + 'static>(&self) -> Option<(Callback, Self)> {
        let mut rest = self.0;
        while let Some((first, tail)) = rest.split_first() {
            rest = tail;
            if let Some(callback) = first.callback.downcast_ref::<Callback>() {
                return Some((*callback, Self(tail)));
            }
        }
        None
    }
}

pub fn register<Callback: Copy + Send + Sync + 'static>(
    id: usize,
    callback: Callback,
    order: Order,
) -> Result<(), &'static str> {
    if id >= MAX_SLOTS {
        return Err("hook slot id exceeds MAX_SLOTS");
    }
    if std::mem::size_of::<Callback>() == 0 {
        return Err("Callback must be a fn(...) pointer type, not a zero-sized fn item or closure");
    }
    let _guard = REGISTER_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let slot = &SLOTS[id];
    let mut entries = slot.load().map(|snapshot| snapshot.0.to_vec()).unwrap_or_default();
    let sequence = entries.len();
    if sequence >= MAX_ENTRIES_PER_SLOT {
        return Err("S2 spike limits each slot to 64 registrations");
    }
    entries.push(Entry {
        order,
        sequence,
        callback: Arc::new(callback),
    });
    entries.sort_by_key(|entry| {
        (
            Reverse(entry.order.priority),
            entry.order.mod_load_order,
            entry.sequence,
        )
    });
    // Old Chain allocations are intentionally leaked until process exit so
    // lock-free hot-path readers holding &'static [Entry] never race with a free.
    // Registration is bounded at 64 entries per slot and occurs at mod load time.
    let published = Box::into_raw(Box::new(Chain { entries }));
    slot.0.store(published, Ordering::Release);
    Ok(())
}
