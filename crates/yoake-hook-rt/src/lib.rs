use std::any::Any;
use std::cmp::Reverse;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};

include!(concat!(env!("OUT_DIR"), "/ledger.rs"));

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
pub static SLOTS: [Slot; MAX_SLOTS + TEST_SLOTS] = [EMPTY; MAX_SLOTS + TEST_SLOTS];

pub const fn assert_ledger_key(id: usize, key: &str) {
    if id >= MAX_SLOTS {
        panic!("hook id is outside the engine ledger");
    }
    let expected = LEDGER_KEYS[id].as_bytes();
    if expected.is_empty() {
        panic!("hook id is unused or tombstoned in the ledger");
    }
    let actual = key.as_bytes();
    if expected.len() != actual.len() {
        panic!("hook id/key pair does not match the ledger");
    }
    let mut index = 0;
    while index < expected.len() {
        if expected[index] != actual[index] {
            panic!("hook id/key pair does not match the ledger");
        }
        index += 1;
    }
}

static REGISTER_LOCK: Mutex<()> = Mutex::new(());

#[cfg(debug_assertions)]
static SLOT_TYPES: [Mutex<Option<std::any::TypeId>>; MAX_SLOTS + TEST_SLOTS] =
    [const { Mutex::new(None) }; MAX_SLOTS + TEST_SLOTS];

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

pub type CallsiteHook<Args, Return> = for<'original> fn(
    &'original mut (dyn FnMut(Args) -> Return + 'original),
    Args,
) -> Return;

#[doc(hidden)]
#[cold]
#[inline(never)]
pub fn callsite_dispatch<Args: 'static, Return: 'static>(
    snapshot: Snapshot,
    args: Args,
    original: &mut dyn FnMut(Args) -> Return,
) -> Return {
    if let Some((callback, rest)) = snapshot.next::<CallsiteHook<Args, Return>>() {
        let mut next = |args| callsite_dispatch(rest, args, original);
        callback(&mut next, args)
    } else {
        original(args)
    }
}

#[macro_export]
macro_rules! callsite {
    (test_id = $id:expr, $callee:path, ()) => {
        $crate::callsite!(@dispatch $id, $callee, (); ())
    };
    (test_id = $id:expr, $callee:path, ($arg0:expr $(,)?)) => {
        $crate::callsite!(@dispatch $id, $callee, ($arg0); (__arg0))
    };
    (test_id = $id:expr, $callee:path, ($arg0:expr, $arg1:expr $(,)?)) => {
        $crate::callsite!(@dispatch $id, $callee, ($arg0, $arg1); (__arg0, __arg1))
    };
    (test_id = $id:expr, $callee:path, ($arg0:expr, $arg1:expr, $arg2:expr $(,)?)) => {
        $crate::callsite!(
            @dispatch $id, $callee, ($arg0, $arg1, $arg2); (__arg0, __arg1, __arg2)
        )
    };
    (test_id = $id:expr, $callee:path, ($arg0:expr, $arg1:expr, $arg2:expr, $arg3:expr $(,)?)) => {
        $crate::callsite!(
            @dispatch $id, $callee, ($arg0, $arg1, $arg2, $arg3);
            (__arg0, __arg1, __arg2, __arg3)
        )
    };
    (@dispatch $id:expr, $callee:path, ($($arg:expr),*); ($($name:ident),*)) => {{
        const __YOAKE_CALLSITE_TEST_ID: usize = $id;
        const _: () = assert!(__YOAKE_CALLSITE_TEST_ID < 64);
        let __yoake_slot = $crate::TEST_ID_BASE + __YOAKE_CALLSITE_TEST_ID;
        if let Some(__yoake_chain) = $crate::SLOTS[__yoake_slot].load() {
            let __yoake_args = ($($arg,)*);
            let mut __yoake_original = |($($name,)*)| $callee($($name),*);
            $crate::callsite_dispatch(__yoake_chain, __yoake_args, &mut __yoake_original)
        } else {
            $callee($($arg),*)
        }
    }};
}

pub fn register<Callback: Copy + Send + Sync + 'static>(
    id: usize,
    callback: Callback,
    order: Order,
) -> Result<(), &'static str> {
    if id >= MAX_SLOTS + TEST_SLOTS {
        return Err("hook slot id exceeds MAX_SLOTS + TEST_SLOTS");
    }
    if std::mem::size_of::<Callback>() == 0 {
        return Err("Callback must be a fn(...) pointer type, not a zero-sized fn item or closure");
    }
    let _guard = REGISTER_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    #[cfg(debug_assertions)]
    {
        let callback_type = std::any::TypeId::of::<Callback>();
        let mut recorded = SLOT_TYPES[id].lock().unwrap_or_else(|error| error.into_inner());
        if recorded.is_some_and(|existing| existing != callback_type) {
            return Err("hook slot already has a different callback type");
        }
        *recorded = Some(callback_type);
    }
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
