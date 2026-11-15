use std::{
    io::{self, Read, Write},
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};
use horizon_friends::social_tls::ControlChannel;

type Channel = Box<dyn ControlChannel>;

/// How long one forwarding read waits before the other direction gets its turn.
const READ_TICK: Duration = Duration::from_millis(1);

/// Message adapter for `PairedConnection`. A read that finds no whole message in
/// one tick is `WouldBlock` (partial frames stay in the channel). A write sends a
/// whole message or fails the connection; it never reports `WouldBlock`, so bytes
/// are never offered twice.
#[derive(Clone)]
pub(super) struct MessageStream {
    channel: Arc<Mutex<Channel>>,
    buffer: Vec<u8>,
    offset: usize,
}

impl MessageStream {
    pub(super) fn new(channel: Channel) -> Self {
        Self { channel: Arc::new(Mutex::new(channel)), buffer: Vec::new(), offset: 0 }
    }

    fn lock(&self) -> MutexGuard<'_, Channel> {
        self.channel.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn send_from_relay(&self, message: &horizon_friends::social::FromRelay) -> io::Result<()> {
        let bytes = serde_json::to_vec(message).map_err(io::Error::other)?;
        self.lock().send_message(&bytes)
    }

    pub(super) fn shutdown(&self) { self.lock().shutdown(); }
}

impl Read for MessageStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        while self.offset >= self.buffer.len() {
            // An empty message carries nothing (and `Ok(0)` would read as EOF): skip it.
            let Some(message) = self.lock().try_recv_message(READ_TICK)? else {
                return Err(io::ErrorKind::WouldBlock.into());
            };
            self.buffer = message;
            self.offset = 0;
        }
        let count = output.len().min(self.buffer.len() - self.offset);
        output[..count].copy_from_slice(&self.buffer[self.offset..self.offset + count]);
        self.offset += count;
        Ok(count)
    }
}

impl Write for MessageStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() { return Ok(0); }
        self.lock().send_message(bytes)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
