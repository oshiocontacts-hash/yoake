//! A bounded queue of byte messages between a mod and its worker.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};

/// Most messages one queue holds.
pub const MAX_MESSAGES: usize = 1024;
/// Most bytes one queue holds.
pub const MAX_QUEUE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendError {
    /// The queue is full; nothing was queued (a send never blocks).
    Full,
    /// The message alone is larger than the queue's byte limit.
    TooLarge,
    /// The other end is gone (no worker, or it stopped).
    Closed,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Full => "queue-full",
            Self::TooLarge => "too-large",
            Self::Closed => "closed",
        })
    }
}

struct State {
    items: VecDeque<Vec<u8>>,
    bytes: usize,
    closed: bool,
}

pub struct Queue {
    state: Mutex<State>,
    ready: Condvar,
    max_messages: usize,
    max_bytes: usize,
}

impl Queue {
    pub fn new() -> Self {
        Self::with_limits(MAX_MESSAGES, MAX_QUEUE_BYTES)
    }

    pub fn with_limits(max_messages: usize, max_bytes: usize) -> Self {
        Self {
            state: Mutex::new(State { items: VecDeque::new(), bytes: 0, closed: false }),
            ready: Condvar::new(),
            max_messages,
            max_bytes,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Queues a message without waiting.
    pub fn push(&self, message: Vec<u8>) -> Result<(), SendError> {
        if message.len() > self.max_bytes {
            return Err(SendError::TooLarge);
        }
        let mut state = self.lock();
        if state.closed {
            return Err(SendError::Closed);
        }
        if state.items.len() >= self.max_messages || state.bytes + message.len() > self.max_bytes {
            return Err(SendError::Full);
        }
        state.bytes += message.len();
        state.items.push_back(message);
        drop(state);
        self.ready.notify_one();
        Ok(())
    }

    /// The oldest message, without waiting.
    pub fn try_pop(&self) -> Option<Vec<u8>> {
        let mut state = self.lock();
        let message = state.items.pop_front()?;
        state.bytes -= message.len();
        Some(message)
    }

    /// The length of the oldest message.
    pub fn peek_len(&self) -> Option<usize> {
        self.lock().items.front().map(Vec::len)
    }

    /// The oldest message; waits until one arrives. `None` once the queue is closed and empty.
    pub fn pop_blocking(&self) -> Option<Vec<u8>> {
        let mut state = self.lock();
        loop {
            if let Some(message) = state.items.pop_front() {
                state.bytes -= message.len();
                return Some(message);
            }
            if state.closed {
                return None;
            }
            state = self.ready.wait(state).unwrap_or_else(|error| error.into_inner());
        }
    }

    /// No more pushes; waiting receivers wake up. Queued messages can still be read.
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }
}

impl Default for Queue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_queue_returns_an_error_and_never_blocks() {
        let queue = Queue::with_limits(3, 100);
        for i in 0..3u8 {
            queue.push(vec![i]).unwrap();
        }
        assert_eq!(queue.push(vec![9]), Err(SendError::Full));
        assert_eq!(queue.try_pop(), Some(vec![0]));
        queue.push(vec![9]).unwrap();
    }

    #[test]
    fn byte_limit_and_oversize() {
        let queue = Queue::with_limits(10, 8);
        queue.push(vec![0; 5]).unwrap();
        assert_eq!(queue.push(vec![0; 5]), Err(SendError::Full));
        assert_eq!(queue.push(vec![0; 9]), Err(SendError::TooLarge));
    }

    #[test]
    fn close_wakes_a_blocked_receiver() {
        let queue = std::sync::Arc::new(Queue::new());
        let q = queue.clone();
        let waiter = std::thread::spawn(move || q.pop_blocking());
        std::thread::sleep(std::time::Duration::from_millis(30));
        queue.close();
        assert_eq!(waiter.join().unwrap(), None);
        assert_eq!(queue.push(vec![1]), Err(SendError::Closed));
    }
}
