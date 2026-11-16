use crate::social_tls::{ClientChannel, ControlChannel, MAX_MESSAGE_SIZE};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

struct Shared {
    channel: Mutex<ClientChannel>,
    pending: Mutex<VecDeque<u8>>,
    socket: Arc<TcpStream>,
    closed: AtomicBool,
    expires: u64,
}

impl Drop for Shared {
    fn drop(&mut self) {
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}

pub struct Bridge {
    shared: Arc<Shared>,
    timeout: Mutex<Option<Duration>>,
}

impl Bridge {
    pub(crate) fn new(channel: ClientChannel, socket: Arc<TcpStream>, expires: u64) -> Self {
        Self {
            shared: Arc::new(Shared {
                channel: Mutex::new(channel),
                pending: Mutex::new(VecDeque::new()),
                socket,
                closed: AtomicBool::new(false),
                expires,
            }),
            timeout: Mutex::new(None),
        }
    }

    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            shared: Arc::clone(&self.shared),
            timeout: Mutex::new(
                *self
                    .timeout
                    .lock()
                    .map_err(|_| io::Error::other("social stream closed"))?,
            ),
        })
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        *self
            .timeout
            .lock()
            .map_err(|_| io::Error::other("social stream closed"))? = timeout;
        Ok(())
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.shared.closed.store(true, Ordering::Release);
        self.shared.socket.shutdown(Shutdown::Both)
    }

    fn closed(&self) -> bool {
        self.shared.closed.load(Ordering::Acquire)
            || (self.shared.expires != 0 && now() >= self.shared.expires)
    }

    fn drain(&self, output: &mut [u8]) -> io::Result<usize> {
        let mut pending = self
            .shared
            .pending
            .lock()
            .map_err(|_| io::Error::other("social stream closed"))?;
        let count = pending.len().min(output.len());
        for destination in &mut output[..count] {
            *destination = pending.pop_front().unwrap();
        }
        Ok(count)
    }
}

impl Read for Bridge {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        let timeout = *self
            .timeout
            .lock()
            .map_err(|_| io::Error::other("social stream closed"))?;
        let started = Instant::now();
        loop {
            if self.closed() {
                let _ = self.shutdown();
                return Ok(0);
            }
            let count = self.drain(output)?;
            if count != 0 {
                return Ok(count);
            }
            let wait = timeout.map_or(Duration::from_millis(16), |timeout| {
                timeout
                    .saturating_sub(started.elapsed())
                    .min(Duration::from_millis(16))
            });
            let received = self
                .shared
                .channel
                .lock()
                .map_err(|_| io::Error::other("social stream closed"))?
                .try_recv_message(wait);
            match received {
                Ok(Some(bytes)) => {
                    self.shared
                        .pending
                        .lock()
                        .map_err(|_| io::Error::other("social stream closed"))?
                        .extend(bytes);
                }
                Ok(None) => {
                    if timeout.is_some_and(|timeout| started.elapsed() >= timeout) {
                        return Err(io::ErrorKind::WouldBlock.into());
                    }
                }
                Err(error) => {
                    let _ = self.shutdown();
                    if matches!(
                        error.kind(),
                        io::ErrorKind::UnexpectedEof
                            | io::ErrorKind::ConnectionReset
                            | io::ErrorKind::ConnectionAborted
                    ) {
                        return Ok(0);
                    }
                    return Err(io::Error::new(error.kind(), "social stream closed"));
                }
            }
            std::thread::yield_now();
        }
    }
}

impl Write for Bridge {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.closed() {
            let _ = self.shutdown();
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let count = bytes.len().min(MAX_MESSAGE_SIZE);
        let result = self
            .shared
            .channel
            .lock()
            .map_err(|_| io::Error::other("social stream closed"))?
            .send_message(&bytes[..count]);
        if let Err(error) = result {
            let _ = self.shutdown();
            return Err(io::Error::new(error.kind(), "social stream closed"));
        }
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
