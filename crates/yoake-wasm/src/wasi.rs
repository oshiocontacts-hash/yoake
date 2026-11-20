//! The sandboxed WASI preview 2 subset a mod may import so it can use normal Rust `std`.
//!
//! Provided: stdout and stderr (collected for the mod's log), an empty stdin, an empty
//! environment, no arguments, wall and monotonic clocks, random, and `exit` (which traps the
//! mod). A guest can never block a host thread: the monotonic clock's `subscribe-duration` and
//! `subscribe-instant` are our own ([`BoundedClock`]): a wait that is not already (nearly) due
//! traps the mod instead of making `pollable.block()` or `thread::sleep` sleep the host thread, and
//! every other pollable a guest can hold (the output streams, the empty stdin) is always ready. The mod gets `/data`, `/game` and optionally `/host/<letter>` preopens (`yoake_mod_io::files`).
//! Sockets and `wasi:http` are linked, but the main instance may not use the network (the context denies it); network goes
//! through the mod's worker (`yoake_mod_io::worker`), which has full WASI p2.

use std::io;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use tokio::io::AsyncWrite;
use wasmtime::component::{HasData, Linker, Resource};
use wasmtime_wasi::async_trait;
use wasmtime_wasi::clocks::WasiClocksCtxView;
use wasmtime_wasi::p2::bindings::clocks::monotonic_clock;
use wasmtime_wasi::p2::{DynPollable, subscribe};
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
    "sockets/network",
    "sockets/instance-network",
    "sockets/ip-name-lookup",
    "sockets/tcp",
    "sockets/tcp-create-socket",
    "sockets/udp",
    "sockets/udp-create-socket",
    "http/types",
    "http/outgoing-handler",
];

/// `name` is the import name without its `wasi:` prefix, such as `cli/stdout@0.2.6`.
pub(crate) fn allowed_import(name: &str) -> bool {
    let Some((interface, version)) = name.rsplit_once('@') else { return false };
    version.starts_with("0.2.") && WASI_IMPORT_INTERFACES.contains(&interface)
}

/// The longest wait a guest may subscribe to; a longer one traps the mod (nothing sleeps).
pub(crate) const MAX_CLOCK_WAIT_NANOS: u64 = 2_000_000;

/// A pollable that is always ready: the only kind of clock pollable a guest can get.
struct Ready;

#[async_trait]
impl Pollable for Ready {
    async fn ready(&mut self) {}
}

/// `HasData` of the bounded `wasi:clocks/monotonic-clock`.
struct BoundedClock;

impl HasData for BoundedClock {
    type Data<'a> = BoundedClockView<'a>;
}

struct BoundedClockView<'a>(WasiClocksCtxView<'a>);

impl BoundedClockView<'_> {
    fn wait(&mut self, nanos: u64) -> wasmtime::Result<Resource<DynPollable>> {
        if nanos > MAX_CLOCK_WAIT_NANOS {
            wasmtime::bail!(
                "the mod waited {} ms on a clock; waiting is not allowed (a hook must not block)",
                nanos / 1_000_000
            );
        }
        let ready = self.0.table.push(Ready)?;
        subscribe(self.0.table, ready)
    }
}

impl monotonic_clock::Host for BoundedClockView<'_> {
    fn now(&mut self) -> wasmtime::Result<monotonic_clock::Instant> {
        monotonic_clock::Host::now(&mut self.0)
    }

    fn resolution(&mut self) -> wasmtime::Result<monotonic_clock::Instant> {
        monotonic_clock::Host::resolution(&mut self.0)
    }

    fn subscribe_instant(&mut self, when: monotonic_clock::Instant) -> wasmtime::Result<Resource<DynPollable>> {
        let now = monotonic_clock::Host::now(&mut self.0)?;
        self.wait(when.saturating_sub(now))
    }

    fn subscribe_duration(&mut self, duration: monotonic_clock::Duration) -> wasmtime::Result<Resource<DynPollable>> {
        self.wait(duration)
    }
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

/// `wasi:http` hooks of the main instance: outgoing requests are refused (a request can wait for
/// seconds, and the main instance must never block the game thread). Network goes through the
/// mod's worker. [`Sandbox::set_main_network`](crate::Sandbox::set_main_network) lifts this.
pub(crate) struct MainHttpHooks;

impl wasmtime_wasi_http::WasiHttpHooks for MainHttpHooks {
    fn send_request(
        &mut self,
        _request: http::Request<wasmtime_wasi_http::WasiBody>,
        _options: Option<wasmtime_wasi_http::RequestOptions>,
        _fut: Box<dyn std::future::Future<Output = wasmtime_wasi_http::Result<()>> + Send>,
    ) -> Box<
        dyn std::future::Future<
                Output = wasmtime_wasi_http::Result<(
                    http::Response<wasmtime_wasi_http::WasiBody>,
                    Box<dyn std::future::Future<Output = wasmtime_wasi_http::Result<()>> + Send>,
                )>,
            > + Send,
    > {
        Box::new(async { Err(wasmtime_wasi_http::Error::HttpRequestDenied) })
    }
}

/// Links exactly the allowed interfaces: everything in [`WASI_IMPORT_INTERFACES`], nothing else.
pub(crate) fn add_to_linker<T: WasiView + wasmtime_wasi_http::WasiHttpView>(linker: &mut Linker<T>) -> wasmtime::Result<()> {
    use wasmtime_wasi::cli::{WasiCli, WasiCliView as _};
    use wasmtime_wasi::filesystem::{WasiFilesystem, WasiFilesystemView as _};
    use wasmtime_wasi::p2::bindings::{cli, filesystem, random, sockets, sync};
    use wasmtime_wasi::random::WasiRandom;
    use wasmtime_wasi::clocks::WasiClocksView as _;
    use wasmtime_wasi::sockets::{WasiSockets, WasiSocketsView as _};

    // io (sync), clocks, random, stdin/stdout/stderr: the `wasi:http/proxy` subset of WASI.
    wasmtime_wasi::p2::add_to_linker_proxy_interfaces_sync(linker)?;
    // Replace the stock monotonic clock (whose pollables really sleep) with the bounded one.
    linker.allow_shadowing(true);
    monotonic_clock::add_to_linker::<T, BoundedClock>(linker, |t| BoundedClockView(t.clocks()))?;
    linker.allow_shadowing(false);
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
    // Sockets and http are linked so a component that imports them loads; whether the main instance may
    // use them is the WASI context's decision (denied unless `Sandbox::set_main_network(true)`).
    let options = sync::LinkOptions::default();
    sockets::tcp_create_socket::add_to_linker::<T, WasiSockets>(linker, T::sockets)?;
    sockets::instance_network::add_to_linker::<T, WasiSockets>(linker, T::sockets)?;
    sockets::network::add_to_linker::<T, WasiSockets>(linker, &(&options).into(), T::sockets)?;
    sockets::ip_name_lookup::add_to_linker::<T, WasiSockets>(linker, T::sockets)?;
    sync::sockets::tcp::add_to_linker::<T, WasiSockets>(linker, T::sockets)?;
    sync::sockets::udp::add_to_linker::<T, WasiSockets>(linker, T::sockets)?;
    sync::sockets::udp_create_socket::add_to_linker::<T, WasiSockets>(linker, T::sockets)?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_sync(linker)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_documented_wasi_subset_is_importable() {
        for name in ["cli/stdout@0.2.6", "clocks/monotonic-clock@0.2.0", "random/random@0.2.6", "filesystem/preopens@0.2.6", "io/streams@0.2.6", "cli/exit@0.2.3", "sockets/tcp@0.2.6", "sockets/network@0.2.6", "sockets/ip-name-lookup@0.2.6", "http/types@0.2.6", "http/outgoing-handler@0.2.6"] {
            assert!(allowed_import(name), "{name}");
        }
        for name in [
            "sockets/tcp@0.3.0",
            "http/types@0.3.0",
            "http/handler@0.2.6",
            "cli/command@0.2.6",
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
