//! The framed TLS channel: explicit pump steps, `Instant` deadlines, no threads.
//!
//! The socket is nonblocking and waits are short sleeps. Kernel send/receive
//! timeouts are avoided on purpose: on Windows a timed-out send leaves the socket
//! in an indeterminate state and may already have sent part of the bytes.
use std::{
    io::{self, Write},
    net::{Shutdown, TcpStream},
    ops::DerefMut,
    time::{Duration, Instant},
};

use rustls::{ConnectionCommon, SideData};

use super::{CHANNEL_BINDING_LABEL, ChannelTimeouts, ControlChannel, MAX_MESSAGE_SIZE};

#[cfg(test)]
mod tests;

/// Polling granularity of the handshake and of `shutdown`'s flush budget.
const TICK: Duration = Duration::from_millis(50);
/// Longest nap between polls of an idle socket.
const MAX_NAP: Duration = Duration::from_millis(10);
/// How long `shutdown` lets the peer finish before closing, so unread data does not turn the close into an RST.
const DRAIN_MAX: Duration = Duration::from_millis(100);

/// `now + wait`, clamped instead of overflowing for absurd durations.
fn after(wait: Duration) -> Instant {
    let now = Instant::now();
    now.checked_add(wait)
        .unwrap_or_else(|| now + Duration::from_secs(86_400 * 365))
}

pub struct TlsChannel<C> {
    conn: C,
    sock: TcpStream,
    incoming: Vec<u8>,
    pending_send: Option<PendingSend>,
    timeouts: ChannelTimeouts,
    broken: bool,
}

struct PendingSend {
    frame: Vec<u8>,
    offset: usize,
    deadline: Instant,
}

fn is_timeout(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

pub(super) fn handshake<C, S>(
    conn: C,
    sock: TcpStream,
    handshake_timeout: Duration,
    timeouts: ChannelTimeouts,
) -> io::Result<TlsChannel<C>>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
{
    sock.set_nonblocking(true)?;
    let _ = sock.set_nodelay(true);
    let mut channel = TlsChannel {
        conn,
        sock,
        incoming: Vec::new(),
        pending_send: None,
        timeouts,
        broken: false,
    };
    let deadline = after(handshake_timeout);
    while channel.conn.is_handshaking() {
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "TLS handshake deadline elapsed",
            ));
        }
        if channel.conn.wants_write() {
            if !channel.write_step()? {
                std::thread::sleep(Duration::from_millis(1));
            }
        } else {
            channel.read_step(TICK)?;
        }
    }
    channel.flush_until(deadline)?;
    Ok(channel)
}

impl<C, S> TlsChannel<C>
where
    C: DerefMut<Target = ConnectionCommon<S>>,
    S: SideData,
{
    /// TLS 1.3 exporter output for the later end-to-end layer; no context is used.
    /// It differs for every TLS session, so the two hops of a bridge never share it.
    pub fn channel_binding(&self) -> io::Result<[u8; 32]> {
        self.conn
            .export_keying_material([0; 32], CHANNEL_BINDING_LABEL, None)
            .map_err(io::Error::other)
    }

    pub fn set_timeouts(&mut self, timeouts: ChannelTimeouts) {
        self.timeouts = timeouts;
    }

    pub(crate) fn queue_message(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.broken || bytes.len() > MAX_MESSAGE_SIZE {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if self.pending_send.is_some() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let mut frame = Vec::with_capacity(4 + bytes.len());
        frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        frame.extend_from_slice(bytes);
        self.conn.writer().write_all(&frame)
    }

    pub(crate) fn try_flush(&mut self) -> io::Result<bool> {
        if self.broken {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        if self.pending_send.is_some() {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        if self.conn.wants_write() {
            self.write_step()?;
        }
        Ok(!self.conn.wants_write())
    }

    /// One `write_tls` call. `Ok(false)`: the socket is full, nothing was consumed.
    fn write_step(&mut self) -> io::Result<bool> {
        match self.conn.write_tls(&mut self.sock) {
            Ok(written) => Ok(written != 0),
            Err(error) if is_timeout(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn send_step(&mut self, bytes: &[u8]) -> io::Result<(bool, bool)> {
        if bytes.len() > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "social TLS message is too large",
            ));
        }
        if self.broken {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "social TLS channel is closed",
            ));
        }
        if let Some(pending) = &self.pending_send {
            if pending.frame[4..] != *bytes {
                self.broken = true;
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a different social TLS message is still pending",
                ));
            }
        } else {
            let mut frame = Vec::with_capacity(bytes.len() + 4);
            frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            frame.extend_from_slice(bytes);
            self.pending_send = Some(PendingSend {
                frame,
                offset: 0,
                deadline: after(self.timeouts.write),
            });
        }
        let pending = self.pending_send.as_mut().unwrap();
        if pending.offset == pending.frame.len() && !self.conn.wants_write() {
            self.pending_send = None;
            return Ok((true, true));
        }
        if Instant::now() >= pending.deadline {
            self.broken = true;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "social TLS write deadline elapsed",
            ));
        }
        let mut progressed = false;
        if pending.offset < pending.frame.len() {
            let written = self
                .conn
                .writer()
                .write(&pending.frame[pending.offset..])
                .inspect_err(|_| self.broken = true)?;
            pending.offset += written;
            progressed = written != 0;
        }
        let queued = pending.offset == pending.frame.len();
        if self.conn.wants_write() {
            progressed |= self.write_step().inspect_err(|_| self.broken = true)?;
        }
        if queued && !self.conn.wants_write() {
            self.pending_send = None;
            Ok((true, progressed))
        } else {
            Ok((false, progressed))
        }
    }

    /// Wait up to `wait` for TLS bytes, feed them to rustls. `Ok(false)` on timeout.
    fn read_step(&mut self, wait: Duration) -> io::Result<bool> {
        let deadline = after(wait);
        let mut nap = Duration::from_millis(1);
        loop {
            match self.conn.read_tls(&mut self.sock) {
                Ok(0) if self.conn.is_handshaking() => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "TLS peer closed during the handshake",
                    ));
                }
                Ok(0) => {
                    // Truncation: rustls reports UnexpectedEof from its reader unless close_notify came first.
                    self.conn
                        .process_new_packets()
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
                    return Ok(true);
                }
                Ok(_) => {
                    if let Err(error) = self.conn.process_new_packets() {
                        let _ = self.conn.write_tls(&mut self.sock); // best-effort alert
                        return Err(io::Error::new(io::ErrorKind::InvalidData, error));
                    }
                    return Ok(true);
                }
                Err(error) if is_timeout(&error) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Ok(false);
                    }
                    std::thread::sleep(nap.min(deadline - now));
                    nap = (nap * 2).min(MAX_NAP);
                }
                Err(error) => return Err(error),
            }
        }
    }

    /// Everything rustls has queued goes to the socket, or the deadline is fatal.
    fn flush_until(&mut self, deadline: Instant) -> io::Result<()> {
        let mut nap = Duration::from_millis(1);
        while self.conn.wants_write() {
            if Instant::now() >= deadline {
                self.broken = true;
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "social TLS write deadline elapsed",
                ));
            }
            if self.write_step().inspect_err(|_| self.broken = true)? {
                nap = Duration::from_millis(1);
            } else {
                std::thread::sleep(nap);
                nap = (nap * 2).min(MAX_NAP);
            }
        }
        Ok(())
    }

    fn take_frame(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.incoming.len() < 4 {
            return Ok(None);
        }
        let length = u32::from_be_bytes([
            self.incoming[0],
            self.incoming[1],
            self.incoming[2],
            self.incoming[3],
        ]) as usize;
        if length > MAX_MESSAGE_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "social TLS frame is too large",
            ));
        }
        if self.incoming.len() < length + 4 {
            return Ok(None);
        }
        let frame = self.incoming[4..length + 4].to_vec();
        self.incoming.drain(..length + 4);
        Ok(Some(frame))
    }

    /// Move decrypted plaintext into `incoming`. `Ok(false)`: nothing more yet.
    fn drain_plaintext(&mut self) -> io::Result<bool> {
        use io::Read;
        let mut bytes = [0; 8192];
        match self.conn.reader().read(&mut bytes) {
            Ok(0) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "TLS peer closed the channel (close_notify)",
            )),
            Ok(count) => {
                self.incoming.extend_from_slice(&bytes[..count]);
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "TLS peer closed the channel without close_notify",
            )),
            Err(error) => Err(error),
        }
    }

    fn recv_within(&mut self, wait: Duration) -> io::Result<Option<Vec<u8>>> {
        if self.broken {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "social TLS channel is closed",
            ));
        }
        let deadline = after(wait);
        let result: io::Result<Option<Vec<u8>>> = (|| loop {
            if let Some(frame) = self.take_frame()? {
                return Ok(Some(frame));
            }
            if self.drain_plaintext()? {
                continue;
            }
            if self.conn.wants_write() {
                self.write_step()?;
            } // tickets, alerts, key updates
            // At least one read attempt, even with a zero wait.
            let remaining = deadline.saturating_duration_since(Instant::now());
            if !self.read_step(remaining)? && Instant::now() >= deadline {
                return Ok(None);
            }
        })();
        if result.is_err() {
            self.broken = true;
        }
        result
    }
}

impl<C, S> ControlChannel for TlsChannel<C>
where
    C: DerefMut<Target = ConnectionCommon<S>> + Send,
    S: SideData,
{
    fn send_message(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut nap = Duration::from_millis(1);
        loop {
            let (complete, progressed) = self.send_step(bytes)?;
            if complete {
                return Ok(());
            }
            if progressed {
                nap = Duration::from_millis(1);
            } else {
                std::thread::sleep(nap);
                nap = (nap * 2).min(MAX_NAP);
            }
        }
    }

    fn try_send_message(&mut self, bytes: &[u8]) -> io::Result<bool> {
        self.send_step(bytes).map(|(complete, _)| complete)
    }

    fn recv_message(&mut self) -> io::Result<Vec<u8>> {
        let wait = self.timeouts.recv;
        self.recv_within(wait)?
            .ok_or_else(|| io::Error::new(io::ErrorKind::WouldBlock, "no social TLS message yet"))
    }

    fn try_recv_message(&mut self, wait: Duration) -> io::Result<Option<Vec<u8>>> {
        self.recv_within(wait)
    }

    fn channel_binding(&self) -> io::Result<[u8; 32]> {
        TlsChannel::channel_binding(self)
    }

    fn shutdown(&mut self) {
        let clean = !self.broken;
        if clean {
            self.conn.send_close_notify();
            let _ = self.flush_until(after(2 * TICK));
        }
        self.broken = true;
        if clean {
            // FIN after close_notify, then let the peer finish: closing with unread
            // incoming data would reset the connection and could lose the close_notify.
            let _ = self.sock.shutdown(Shutdown::Write);
            let deadline = after(DRAIN_MAX);
            let mut sink = [0; 4096];
            while Instant::now() < deadline {
                match io::Read::read(&mut self.sock, &mut sink) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if is_timeout(&error) => {
                        std::thread::sleep(Duration::from_millis(2))
                    }
                    Err(_) => break,
                }
            }
        }
        let _ = self.sock.shutdown(Shutdown::Both);
    }
}
