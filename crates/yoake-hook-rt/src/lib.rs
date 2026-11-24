use std::any::Any;
use std::cmp::Reverse;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};

pub const SLOT_CAPACITY: usize = 1 << 20;
pub const TEST_SLOTS: usize = 64;
pub const TEST_ID_BASE: usize = SLOT_CAPACITY - TEST_SLOTS;

// Keep runtime slots below the engine-synthetic ids defined by
// docs/yoake2/ENGINE_CALL_ABI.md on yoake2/slice.
const _: () = assert!(SLOT_CAPACITY as u64 <= 0x8000_0000);

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

    /// The hot-path entry: every stamped function and call site reaches its slot here.
    #[inline(always)]
    pub fn load(&'static self) -> Option<Snapshot> {
        #[cfg(feature = "hit-count")]
        hit_count::record(self);
        self.peek()
    }

    /// `load` without counting a hit (registration reads the chain this way).
    #[inline(always)]
    fn peek(&'static self) -> Option<Snapshot> {
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
    assert!(
        id != 0,
        "hook id 0 is reserved for the legacy single-export guest API"
    );
    let mut low = 0;
    let mut high = keys.len();
    while low < high {
        let middle = low + (high - low) / 2;
        if keys[middle].0 < id {
            low = middle + 1;
        } else {
            high = middle;
        }
    }
    if low < keys.len() && keys[low].0 == id {
        let expected = keys[low].1.as_bytes();
        let actual = key.as_bytes();
        if expected.len() != actual.len()
            || !regions_equal(expected, 0, actual, 0, expected.len())
        {
            panic!("hook id/key pair does not match the crate ledger");
        }
        return;
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
    let mut entries = slot.peek().map(|snapshot| snapshot.0.to_vec()).unwrap_or_default();
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

/// Per-slot hit counters for `--features hit-count`: which hook slots a run actually calls.
#[cfg(feature = "hit-count")]
mod hit_count {
    use super::{Slot, SLOTS, SLOT_CAPACITY};
    use std::cmp::Reverse;
    use std::io::{self, Write};
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};

    // Sized to the whole slot space rather than the ledger's `next_id` (12,210 at 0.1), because
    // the test slots sit at the top of it. Zero-initialised, so it costs address space only.
    const ZERO: AtomicU64 = AtomicU64::new(0);
    static HITS: [AtomicU64; SLOT_CAPACITY] = [ZERO; SLOT_CAPACITY];

    /// Counts one call. Every entry into a slot goes through `Slot::load`, which passes the slot
    /// here; `SLOTS` is one array, so the slot's index is its offset from the array's start.
    #[inline(always)]
    pub(super) fn record(slot: &'static Slot) {
        let offset = (slot as *const Slot as usize).wrapping_sub(SLOTS.as_ptr() as usize);
        let index = offset / std::mem::size_of::<Slot>();
        if index < SLOT_CAPACITY {
            HITS[index].fetch_add(1, Ordering::Relaxed);
        }
    }

    /// The non-zero counts as `(slot id, calls)`, most calls first (ties by slot id).
    pub fn hit_counts() -> Vec<(usize, u64)> {
        let mut rows: Vec<(usize, u64)> = HITS
            .iter()
            .enumerate()
            .filter_map(|(slot, count)| {
                let calls = count.load(Ordering::Relaxed);
                (calls > 0).then_some((slot, calls))
            })
            .collect();
        rows.sort_by_key(|&(slot, calls)| (Reverse(calls), slot));
        rows
    }

    /// Writes `<slot>\t<calls>` lines in the order given.
    pub fn write_hit_counts(out: &mut impl Write, rows: &[(usize, u64)]) -> io::Result<()> {
        for (slot, calls) in rows {
            writeln!(out, "{slot}\t{calls}")?;
        }
        Ok(())
    }

    /// Writes the current counts to `path`, replacing it.
    pub fn dump_hit_counts(path: &Path) -> io::Result<()> {
        let rows = hit_counts();
        let mut out = io::BufWriter::new(std::fs::File::create(path)?);
        write_hit_counts(&mut out, &rows)?;
        out.flush()
    }

    /// Dumps to the path in `YOAKE_HOOK_HITS`; does nothing when it is unset. A failed write is
    /// reported on stderr and never stops the game's exit.
    pub fn dump_hit_counts_from_env() {
        let Some(path) = std::env::var_os("YOAKE_HOOK_HITS") else {
            return;
        };
        if let Err(error) = dump_hit_counts(Path::new(&path)) {
            eprintln!("YOAKE_HOOK_HITS: cannot write {}: {error}", Path::new(&path).display());
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::TEST_ID_BASE;

        fn calls_of(slot: usize) -> u64 {
            hit_counts().iter().find(|(id, _)| *id == slot).map_or(0, |&(_, calls)| calls)
        }

        #[test]
        fn load_counts_each_entry_under_its_slot_id() {
            let slot = TEST_ID_BASE + 7;
            let other = TEST_ID_BASE + 8;
            let (before, other_before) = (calls_of(slot), calls_of(other));
            for _ in 0..3 {
                assert!(SLOTS[slot].load().is_none());
            }
            assert!(SLOTS[other].load().is_none());
            assert_eq!(calls_of(slot) - before, 3);
            assert_eq!(calls_of(other) - other_before, 1);
        }

        #[test]
        fn dump_format_is_tab_separated_per_slot() {
            let mut bytes = Vec::new();
            write_hit_counts(&mut bytes, &[(42, 9), (3, 1)]).unwrap();
            assert_eq!(String::from_utf8(bytes).unwrap(), "42\t9\n3\t1\n");
        }

        #[test]
        fn dump_writes_only_nonzero_slots_most_calls_first() {
            let busy = TEST_ID_BASE + 20;
            let quiet = TEST_ID_BASE + 21;
            for _ in 0..5 {
                let _ = SLOTS[busy].load();
            }
            let _ = SLOTS[quiet].load();
            let path = std::env::temp_dir()
                .join(format!("yoake-hook-hits-test-{}.txt", std::process::id()));
            dump_hit_counts(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::remove_file(&path).unwrap();
            let rows: Vec<(usize, u64)> = text
                .lines()
                .map(|line| {
                    let (slot, calls) = line.split_once('\t').unwrap();
                    (slot.parse().unwrap(), calls.parse().unwrap())
                })
                .collect();
            assert!(rows.iter().all(|&(_, calls)| calls > 0));
            assert!(rows.windows(2).all(|pair| pair[0].1 >= pair[1].1));
            let at = |slot| rows.iter().position(|&(id, _)| id == slot).unwrap();
            assert!(at(busy) < at(quiet));
            assert_eq!(rows[at(busy)].1, 5);
            assert_eq!(rows[at(quiet)].1, 1);
        }
    }
}

#[cfg(feature = "hit-count")]
pub use hit_count::{dump_hit_counts, dump_hit_counts_from_env, hit_counts, write_hit_counts};
