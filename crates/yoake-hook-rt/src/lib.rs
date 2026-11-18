use std::any::Any;
use std::cmp::Reverse;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};

pub const SLOT_CAPACITY: usize = 1 << 20;
pub const TEST_SLOTS: usize = 64;
pub const TEST_ID_BASE: usize = SLOT_CAPACITY - TEST_SLOTS;

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
            // in `__register_typed`, and old `Chain` snapshots are intentionally leaked
            // for the process lifetime so lock-free readers never observe freed memory.
            Some(Snapshot(unsafe { &(*pointer).entries }))
        }
    }
}

const EMPTY: Slot = Slot::new();
pub static SLOTS: [Slot; SLOT_CAPACITY] = [EMPTY; SLOT_CAPACITY];

const fn regions_equal(
    left: &[u8],
    left_start: usize,
    right: &[u8],
    right_start: usize,
    length: usize,
) -> bool {
    let mut index = 0;
    while index < length {
        if left[left_start + index] != right[right_start + index] {
            return false;
        }
        index += 1;
    }
    true
}

pub const fn assert_ledger_key(keys: &[(usize, &str)], id: usize, key: &str) {
    let mut index = 0;
    while index < keys.len() {
        if keys[index].0 == id {
            let expected = keys[index].1.as_bytes();
            let actual = key.as_bytes();
            if expected.len() != actual.len()
                || !regions_equal(expected, 0, actual, 0, expected.len())
            {
                panic!("hook id/key pair does not match the crate ledger");
            }
            return;
        }
        index += 1;
    }
    panic!("hook id/key pair is absent from the crate ledger");
}

const fn trailing_cfg_start(key: &[u8], end: usize) -> usize {
    if end == 0 || key[end - 1] != b']' {
        return end;
    }
    let mut depth = 0;
    let mut index = end;
    while index > 0 {
        index -= 1;
        if key[index] == b']' {
            depth += 1;
        } else if key[index] == b'[' {
            if depth == 0 {
                return end;
            }
            depth -= 1;
            if depth == 0 {
                if index + 5 <= end
                    && key[index + 1] == b'c'
                    && key[index + 2] == b'f'
                    && key[index + 3] == b'g'
                    && key[index + 4] == b'('
                {
                    return index;
                }
                return end;
            }
        }
    }
    end
}

pub const fn assert_hook_name(key: &str, fn_name: &str) {
    let key = key.as_bytes();
    let mut key_end = key.len();
    loop {
        let cfg_start = trailing_cfg_start(key, key_end);
        if cfg_start == key_end {
            break;
        }
        key_end = cfg_start;
    }
    let mut key_start = 0;
    let mut angle_depth: usize = 0;
    let mut square_depth: usize = 0;
    let mut paren_depth: usize = 0;
    let mut index = 0;
    while index < key_end {
        match key[index] {
            b'<' => angle_depth += 1,
            b'>' => angle_depth = angle_depth.saturating_sub(1),
            b'[' => square_depth += 1,
            b']' => square_depth = square_depth.saturating_sub(1),
            b'(' => paren_depth += 1,
            b')' => paren_depth = paren_depth.saturating_sub(1),
            b':' if index + 1 < key_end && key[index + 1] == b':' => {
                if angle_depth == 0 && square_depth == 0 && paren_depth == 0 {
                    key_start = index + 2;
                }
                index += 1;
            }
            _ => {}
        }
        index += 1;
    }
    let key_length = key_end - key_start;
    let name = fn_name.as_bytes();
    let prefix = b"__yoake_trait_";
    let mut helper = name.len() > prefix.len();
    index = 0;
    while helper && index < prefix.len() {
        helper = name[index] == prefix[index];
        index += 1;
    }
    let name_start = if helper {
        if name.len() <= key_length || name[name.len() - key_length - 1] != b'_' {
            panic!("hook key does not match the trait helper method");
        }
        name.len() - key_length
    } else {
        if name.len() != key_length {
            panic!("hook key does not match the function name");
        }
        0
    };
    if !regions_equal(name, name_start, key, key_start, key_length) {
        panic!("hook key does not match the function name");
    }
}

static REGISTER_LOCK: Mutex<()> = Mutex::new(());

static SLOT_TYPES: Mutex<Vec<(usize, std::any::TypeId)>> = Mutex::new(Vec::new());

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

#[doc(hidden)]
pub fn __register_typed<Callback: Copy + Send + Sync + 'static>(
    id: usize,
    expected_type: std::any::TypeId,
    callback: Callback,
    order: Order,
) -> Result<(), &'static str> {
    if id >= SLOT_CAPACITY {
        return Err("hook slot id exceeds SLOT_CAPACITY");
    }
    if std::mem::size_of::<Callback>() == 0 {
        return Err("Callback must be a fn(...) pointer type, not a zero-sized fn item or closure");
    }
    let callback_type = std::any::TypeId::of::<Callback>();
    if expected_type != callback_type {
        return Err("expected callback TypeId does not match the callback");
    }
    let _guard = REGISTER_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    {
        let mut recorded = SLOT_TYPES.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((_, existing)) = recorded.iter().find(|(slot, _)| *slot == id) {
            if *existing != expected_type {
                return Err("hook slot already has a different callback type");
            }
        } else {
            recorded.push((id, expected_type));
        }
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
