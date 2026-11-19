//! stdout/stderr of a worker or of a full-access module: complete lines go to the mod's log.

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use tokio::io::AsyncWrite;
use wasmtime_wasi::async_trait;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::{OutputStream as P2OutputStream, Pollable, StreamResult};

const MAX_LINE_CHARS: usize = 1024;
const MAX_UNTERMINATED: usize = 4096;

/// Where log lines go (one line per call, without newline).
#[derive(Clone)]
pub struct LogSink(Arc<dyn Fn(&str) + Send + Sync>);

impl LogSink {
    pub fn new(f: impl Fn(&str) + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// The default: the process's stderr (the game log echoes it).
    pub fn stderr() -> Self {
        Self::new(|line| eprintln!("{line}"))
    }

    pub fn log(&self, line: &str) {
        (self.0)(line);
    }
}

impl std::fmt::Debug for LogSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LogSink")
    }
}

/// A line-splitting output stream; each line is logged as `<prefix> [stdout|stderr]: <line>`
/// (the same shape as the sandboxed route: `wasm mod <id> [stdout]: ...`).
#[derive(Clone)]
pub struct LinePipe {
    buffer: Arc<Mutex<Vec<u8>>>,
    prefix: Arc<str>,
    stream: &'static str,
    sink: LogSink,
}

impl LinePipe {
    pub fn new(prefix: &str, stream: &'static str, sink: LogSink) -> Self {
        Self { buffer: Arc::default(), prefix: prefix.into(), stream, sink }
    }

    fn emit(&self, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let text: String = text
            .trim_end_matches('\r')
            .chars()
            .take(MAX_LINE_CHARS)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        self.sink.log(&format!("{} [{}]: {}", self.prefix, self.stream, text));
    }

    fn push(&self, bytes: &[u8]) {
        let mut lines: Vec<Vec<u8>> = Vec::new();
        {
            let mut buffer = self.buffer.lock().unwrap_or_else(|error| error.into_inner());
            buffer.extend_from_slice(bytes);
            while let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = buffer.drain(..=end).collect();
                lines.push(line[..line.len() - 1].to_vec());
            }
            if buffer.len() >= MAX_UNTERMINATED {
                lines.push(std::mem::take(&mut *buffer));
            }
        }
        for line in lines.into_iter().take(256) {
            self.emit(&line);
        }
    }
}

impl IsTerminal for LinePipe {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for LinePipe {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
    fn p2_stream(&self) -> Box<dyn P2OutputStream> {
        Box::new(self.clone())
    }
}

#[async_trait]
impl Pollable for LinePipe {
    async fn ready(&mut self) {}
}

impl P2OutputStream for LinePipe {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.push(&bytes);
        Ok(())
    }
    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }
    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(64 * 1024)
    }
}

impl AsyncWrite for LinePipe {
    fn poll_write(self: Pin<&mut Self>, _: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        self.push(bytes);
        Poll::Ready(Ok(bytes.len()))
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
