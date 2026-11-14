//! The sandboxed WASI preview 2 subset a mod may import so it can use normal Rust `std`.
//!
//! Provided: stdout and stderr (collected for the mod's log), an empty stdin, an empty
//! environment, no arguments, wall and monotonic clocks, random, and `exit` (which traps the
//! mod). The filesystem interfaces are linked (std imports them unconditionally) but the guest
//! has **no preopened directory**, so there is nothing it can open. Sockets are neither
//! importable nor linked.

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use tokio::io::AsyncWrite;
use wasmtime::component::Linker;
use wasmtime_wasi::async_trait;
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::p2::{OutputStream as P2OutputStream, Pollable, StreamResult};
use wasmtime_wasi::WasiView;

/// WASI preview 2 interfaces a component may import (any `0.2.x` version).
pub const WASI_IMPORT_INTERFACES: &[&str] = &[
    "cli/environment",
    "cli/exit",
    "cli/stdin",
    "cli/stdout",
    "cli/stderr",
    "cli/terminal-input",
    "cli/terminal-output",
    "cli/terminal-stdin",
    "cli/terminal-stdout",
    "cli/terminal-stderr",
    "clocks/monotonic-clock",
    "clocks/wall-clock",
    "filesystem/types",
    "filesystem/preopens",
    "io/error",
    "io/poll",
    "io/streams",
    "random/random",
    "random/insecure",
    "random/insecure-seed",
];

/// `name` is the import name without its `wasi:` prefix, such as `cli/stdout@0.2.6`.
pub(crate) fn allowed_import(name: &str) -> bool {
    let Some((interface, version)) = name.rsplit_once('@') else { return false };
    version.starts_with("0.2.") && WASI_IMPORT_INTERFACES.contains(&interface)
}

const MAX_PENDING_BYTES: usize = 64 * 1024;
const MAX_LINE_CHARS: usize = 1024;
const MAX_LINES_PER_DRAIN: usize = 256;
const MAX_UNTERMINATED_BYTES: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutputLine {
    pub stream: OutputStream,
    pub text: String,
}

#[derive(Default)]
pub(crate) struct Output {
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Output {
    pub(crate) fn stdout(&self) -> Pipe {
        Pipe(Arc::clone(&self.stdout))
    }

    pub(crate) fn stderr(&self) -> Pipe {
        Pipe(Arc::clone(&self.stderr))
    }

    pub(crate) fn drain(&self) -> Vec<OutputLine> {
        let mut lines = Vec::new();
        split_lines(&self.stdout, OutputStream::Stdout, &mut lines);
        split_lines(&self.stderr, OutputStream::Stderr, &mut lines);
        lines
    }
}

fn split_lines(buffer: &Mutex<Vec<u8>>, stream: OutputStream, lines: &mut Vec<OutputLine>) {
    let mut buffer = buffer.lock().unwrap_or_else(|error| error.into_inner());
    if buffer.is_empty() {
        return;
    }
    let mut start = 0;
    let mut produced = 0;
    while let Some(end) = buffer[start..].iter().position(|byte| *byte == b'\n') {
        if produced < MAX_LINES_PER_DRAIN {
            lines.push(line(stream, &buffer[start..start + end]));
            produced += 1;
        }
        start += end + 1;
    }
    if buffer.len() - start >= MAX_UNTERMINATED_BYTES {
        if produced < MAX_LINES_PER_DRAIN {
            lines.push(line(stream, &buffer[start..]));
        }
        start = buffer.len();
    }
    buffer.drain(..start);
}

fn line(stream: OutputStream, bytes: &[u8]) -> OutputLine {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_end_matches('\r');
    OutputLine { stream, text: text.chars().take(MAX_LINE_CHARS).collect() }
}

/// A WASI output stream that appends to a bounded buffer; output beyond the bound is dropped.
#[derive(Clone)]
pub(crate) struct Pipe(Arc<Mutex<Vec<u8>>>);

impl Pipe {
    fn push(&self, bytes: &[u8]) {
        let mut buffer = self.0.lock().unwrap_or_else(|error| error.into_inner());
        let room = MAX_PENDING_BYTES.saturating_sub(buffer.len());
        buffer.extend_from_slice(&bytes[..bytes.len().min(room)]);
    }
}

impl IsTerminal for Pipe {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for Pipe {
    fn async_stream(&self) -> Box<dyn AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }

    fn p2_stream(&self) -> Box<dyn P2OutputStream> {
        Box::new(self.clone())
    }
}

#[async_trait]
impl Pollable for Pipe {
    async fn ready(&mut self) {}
}

impl P2OutputStream for Pipe {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.push(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(MAX_PENDING_BYTES)
    }
}

impl AsyncWrite for Pipe {
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

/// Links exactly the allowed interfaces: everything in [`WASI_IMPORT_INTERFACES`], nothing else
/// (no sockets, no name lookup).
pub(crate) fn add_to_linker<T: WasiView>(linker: &mut Linker<T>) -> wasmtime::Result<()> {
    use wasmtime_wasi::cli::{WasiCli, WasiCliView as _};
    use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemView as _};
    use wasmtime_wasi::p2::bindings::{cli, filesystem, random, sync};
    use wasmtime_wasi::random::WasiRandom;

    // io (sync), clocks, random, stdin/stdout/stderr: the `wasi:http/proxy` subset of WASI.
    wasmtime_wasi::p2::add_to_linker_proxy_interfaces_sync(linker)?;
    random::insecure::add_to_linker::<T, WasiRandom>(linker, |t| t.ctx().ctx.random())?;
    random::insecure_seed::add_to_linker::<T, WasiRandom>(linker, |t| t.ctx().ctx.random())?;
    cli::exit::add_to_linker::<T, WasiCli>(linker, T::cli)?;
    cli::environment::add_to_linker::<T, WasiCli>(linker, T::cli)?;
    cli::terminal_input::add_to_linker::<T, WasiCli>(linker, T::cli)?;
    cli::terminal_output::add_to_linker::<T, WasiCli>(linker, T::cli)?;
    cli::terminal_stdin::add_to_linker::<T, WasiCli>(linker, T::cli)?;
    cli::terminal_stdout::add_to_linker::<T, WasiCli>(linker, T::cli)?;
    cli::terminal_stderr::add_to_linker::<T, WasiCli>(linker, T::cli)?;
    filesystem::preopens::add_to_linker::<T, WasiFilesystem>(linker, T::filesystem)?;
    sync::filesystem::types::add_to_linker::<T, WasiFilesystem>(linker, T::filesystem)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_documented_wasi_subset_is_importable() {
        for name in ["cli/stdout@0.2.6", "clocks/monotonic-clock@0.2.0", "random/random@0.2.6", "filesystem/preopens@0.2.6", "io/streams@0.2.6", "cli/exit@0.2.3"] {
            assert!(allowed_import(name), "{name}");
        }
        for name in [
            "sockets/tcp@0.2.6",
            "sockets/network@0.2.6",
            "sockets/ip-name-lookup@0.2.6",
            "http/types@0.2.6",
            "cli/stdout@0.3.0",
            "cli/stdout",
            "cli/run@0.2.6",
        ] {
            assert!(!allowed_import(name), "{name}");
        }
    }

    #[test]
    fn output_is_split_into_complete_bounded_lines() {
        let output = Output::default();
        output.stdout().push(b"one\r\ntwo\npart");
        output.stderr().push(b"warn\n");
        let lines = output.drain();
        let texts: Vec<_> = lines.iter().map(|l| (l.stream, l.text.as_str())).collect();
        assert_eq!(texts, [(OutputStream::Stdout, "one"), (OutputStream::Stdout, "two"), (OutputStream::Stderr, "warn")]);
        output.stdout().push(b"ial\n");
        assert_eq!(output.drain()[0].text, "partial");
        output.stdout().push(&vec![b'x'; MAX_PENDING_BYTES * 2]);
        let lines = output.drain();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].text.len(), MAX_LINE_CHARS);
    }
}
