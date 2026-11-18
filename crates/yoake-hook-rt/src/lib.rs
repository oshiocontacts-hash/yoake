use std::any::Any;
use std::cmp::Reverse;
use std::marker::PhantomData;
use std::ops::ControlFlow;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::{Arc, Mutex};

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

#[derive(Clone, Default)]
struct Chain {
    head: Vec<Entry>,
    returned: Vec<Entry>,
}

pub struct Slot {
    chain: AtomicPtr<Chain>,
    publication: Mutex<()>,
}

impl Default for Slot {
    fn default() -> Self {
        Self::new()
    }
}

impl Slot {
    pub const fn new() -> Self {
        Self {
            chain: AtomicPtr::new(std::ptr::null_mut()),
            publication: Mutex::new(()),
        }
    }

    #[inline(always)]
    pub fn load(&'static self) -> Option<Snapshot> {
        let pointer = self.chain.load(Ordering::Acquire);
        if pointer.is_null() {
            None
        } else {
            Some(Snapshot(unsafe { &*pointer }))
        }
    }

    fn register<Callback: Copy + Send + Sync + 'static>(
        &'static self,
        callback: Callback,
        order: Order,
        at_head: bool,
    ) -> Result<(), &'static str> {
        let _guard = self
            .publication
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut chain = self
            .load()
            .map(|snapshot| snapshot.0.clone())
            .unwrap_or_default();
        let sequence = chain.head.len() + chain.returned.len();
        if sequence >= 64 {
            return Err("S2 spike limits each slot to 64 registrations");
        }
        let entries = if at_head {
            &mut chain.head
        } else {
            &mut chain.returned
        };
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
        self.chain
            .store(Box::into_raw(Box::new(chain)), Ordering::Release);
        Ok(())
    }
}

#[derive(Clone, Copy)]
pub struct Snapshot(&'static Chain);

impl Snapshot {
    #[cold]
    #[inline(never)]
    fn head<Callback: Copy + 'static, ResultValue>(
        &self,
        mut invoke: impl FnMut(Callback) -> ControlFlow<ResultValue>,
    ) -> ControlFlow<ResultValue> {
        for entry in &self.0.head {
            if let Some(callback) = entry.callback.downcast_ref::<Callback>() {
                invoke(*callback)?;
            }
        }
        ControlFlow::Continue(())
    }

    #[cold]
    #[inline(never)]
    fn returned<Callback: Copy + 'static>(&self, mut invoke: impl FnMut(Callback)) {
        for entry in &self.0.returned {
            if let Some(callback) = entry.callback.downcast_ref::<Callback>() {
                invoke(*callback);
            }
        }
    }
}

pub struct Site<Head, Returned> {
    slot: &'static Slot,
    marker: PhantomData<fn(Head, Returned)>,
}

impl<Head: Copy + Send + Sync + 'static, Returned: Copy + Send + Sync + 'static>
    Site<Head, Returned>
{
    pub const fn new(slot: &'static Slot) -> Self {
        Self {
            slot,
            marker: PhantomData,
        }
    }

    #[inline(always)]
    pub fn load(&self) -> Option<Snapshot> {
        self.slot.load()
    }

    #[inline(always)]
    pub fn head<ResultValue>(
        &self,
        snapshot: &Snapshot,
        invoke: impl FnMut(Head) -> ControlFlow<ResultValue>,
    ) -> ControlFlow<ResultValue> {
        snapshot.head(invoke)
    }

    #[inline(always)]
    pub fn returned(&self, snapshot: &Snapshot, invoke: impl FnMut(Returned)) {
        snapshot.returned(invoke);
    }

    pub fn register_head(&self, callback: Head, order: Order) -> Result<(), &'static str> {
        self.slot.register(callback, order, true)
    }

    pub fn register_return(&self, callback: Returned, order: Order) -> Result<(), &'static str> {
        self.slot.register(callback, order, false)
    }
}
