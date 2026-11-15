use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    sync::{Arc, Mutex, MutexGuard},
};
use horizon_friends::social_channel::{MAX_FRAME_SIZE, SecureChannel};

type Channel = SecureChannel<TcpStream>;

/// A nonblocking byte-stream view of encrypted messages for `PairedConnection`.
/// A complete frame is peeked before decryption, so a short socket read never
/// poisons the stage 1 channel. Writes temporarily use blocking mode.
#[derive(Clone)]
pub(super) struct MessageStream {
    channel: Arc<Mutex<Channel>>,
    buffer: Vec<u8>,
    offset: usize,
}

impl MessageStream {
    pub(super) fn new(channel: Channel) -> io::Result<Self> {
        channel.get_ref().set_nonblocking(true)?;
        Ok(Self { channel: Arc::new(Mutex::new(channel)), buffer: Vec::new(), offset: 0 })
    }

    fn lock(&self) -> MutexGuard<'_, Channel> {
        self.channel.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(super) fn send_from_relay(&self, message: &horizon_friends::social::FromRelay) -> io::Result<()> {
        let mut channel = self.lock();
        channel.get_ref().set_nonblocking(false)?;
        let result = channel.send_from_relay(message);
        let restored = channel.get_ref().set_nonblocking(true);
        result.and(restored)
    }

    pub(super) fn shutdown(&self) {
        let _ = self.lock().get_ref().shutdown(Shutdown::Both);
    }
}

impl Read for MessageStream {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.offset < self.buffer.len() { break; }
            let mut channel = self.lock();
            let mut wire = [0; MAX_FRAME_SIZE + 4];
            let available = match channel.get_ref().peek(&mut wire) {
                Ok(0) => return Ok(0),
                Ok(available) => available,
                Err(error) => return Err(error),
            };
            if available < 4 { return Err(io::ErrorKind::WouldBlock.into()); }
            let length = u32::from_be_bytes(wire[..4].try_into().unwrap()) as usize;
            if length <= MAX_FRAME_SIZE && available < length + 4 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let message = channel.recv_message()?;
            drop(channel);
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
        let mut channel = self.lock();
        channel.get_ref().set_nonblocking(false)?;
        let result = channel.send_message(bytes);
        let restored = channel.get_ref().set_nonblocking(true);
        result.and(restored)?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
