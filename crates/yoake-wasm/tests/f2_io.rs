//! FR-F2: files, network (through a worker) and the worker channel, end to end with real components
//! (`crates/yoake-wasm-sdk/examples/f2-io`). No game is started.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yoake_mod_io::{FileAccess, LogSink, ModIo};
use yoake_wasm::test_guests::{F2_FILES, F2_MAIN, F2_WORKER, load};
use yoake_wasm::{ModInstance, Sandbox, TimeLimits};

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yoake-f2-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A loaded CI machine can pause a thread for longer than a hook's 20 ms budget; these tests are about files and the
/// worker, not about the time budget (the bounded clock is what `a_wait_in_the_main_instance_still_traps` checks).
fn relaxed(mut module: ModInstance<()>) -> ModInstance<()> {
    module.set_limits(TimeLimits { call: Duration::from_secs(10), aggregate: Duration::from_secs(10) });
    module
}

fn run(module: &mut ModInstance<()>, op: &str) -> Option<String> {
    module.call("run", |store, instance| {
        let run = instance
            .get_typed_func::<(String,), (String,)>(&mut *store, "run")
            .map_err(|error| error.to_string())?;
        run.call(&mut *store, (op.to_owned(),)).map(|(text,)| text).map_err(|error| format!("{error:#}"))
    })
}

fn files_module(game: &Path, id: &str, host_filesystem: bool, bytes: &[u8]) -> ModInstance<()> {
    let mut sandbox = Sandbox::new();
    sandbox.set_mod_io(id, ModIo::new(FileAccess::new(game, id, host_filesystem)));
    relaxed(sandbox.compile(bytes).unwrap().instantiate(id, (), |_| Ok(())).unwrap())
}

#[test]
fn component_writes_and_reads_its_data_folder_and_the_game_folder() {
    let Some(bytes) = load(&F2_FILES) else { return };
    let game = scratch("files");
    let mut module = files_module(&game, "f2files", false, &bytes);
    assert_eq!(run(&mut module, "write").as_deref(), Some("ok"));
    assert_eq!(std::fs::read_to_string(game.join("mod-data/f2files/x.txt")).unwrap(), "hello-data");
    assert_eq!(run(&mut module, "read").as_deref(), Some("hello-data"));
    assert_eq!(run(&mut module, "game").as_deref(), Some("ok"));
    assert_eq!(std::fs::read_to_string(game.join("f2-probe.txt")).unwrap(), "g");
    // no /host without `host_filesystem = true`
    assert!(run(&mut module, "host").unwrap().starts_with("error"));
    // stdout reaches the mod's log
    assert_eq!(run(&mut module, "print").as_deref(), Some("printed"));
    let lines: Vec<String> = module.drain_output().into_iter().map(|line| line.text).collect();
    assert!(lines.contains(&"files guest stdout line".to_owned()), "{lines:?}");
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn host_filesystem_preopens_the_drives_only_when_asked() {
    let Some(bytes) = load(&F2_FILES) else { return };
    let game = scratch("host");
    let mut module = files_module(&game, "f2host", true, &bytes);
    let count = run(&mut module, "host").unwrap();
    assert!(count.parse::<usize>().is_ok_and(|n| n >= 1), "{count}");
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn without_a_configured_mod_there_are_no_files() {
    let Some(bytes) = load(&F2_FILES) else { return };
    let mut module = relaxed(Sandbox::new().compile(&bytes).unwrap().instantiate("plain", (), |_| Ok(())).unwrap());
    assert!(run(&mut module, "write").unwrap().starts_with("error"));
}

#[test]
fn a_wait_in_the_main_instance_still_traps() {
    let Some(bytes) = load(&F2_FILES) else { return };
    let game = scratch("sleep");
    let mut module = files_module(&game, "f2sleep", false, &bytes);
    assert_eq!(run(&mut module, "sleep"), None);
    assert!(module.is_disabled());
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn the_main_instance_may_not_use_the_network() {
    let Some(bytes) = load(&F2_FILES) else { return };
    let game = scratch("net");
    let mut module = files_module(&game, "f2net", false, &bytes);
    let answer = run(&mut module, "net").unwrap();
    assert!(answer.starts_with("error"), "{answer}");
    let _ = std::fs::remove_dir_all(game);
}

#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<String>>>);

impl Log {
    fn sink(&self) -> LogSink {
        let lines = Arc::clone(&self.0);
        LogSink::new(move |line| lines.lock().unwrap().push(line.to_owned()))
    }

    fn contains(&self, needle: &str) -> bool {
        self.0.lock().unwrap().iter().any(|line| line.contains(needle))
    }

    fn wait_for(&self, needle: &str) -> bool {
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            if self.contains(needle) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }
}

fn worker_module(game: &Path, id: &str, log: &Log) -> Option<ModInstance<()>> {
    let main = load(&F2_MAIN)?;
    let worker = load(&F2_WORKER)?;
    let mut sandbox = Sandbox::new();
    let mut io = ModIo::new(FileAccess::new(game, id, false));
    io.worker = Some(worker);
    io.log = log.sink();
    sandbox.set_mod_io(id, io);
    Some(relaxed(sandbox.compile(&main).unwrap().instantiate(id, (), |_| Ok(())).unwrap()))
}

/// Polls the mod's `recv` until a message arrives (the worker answers on its own thread).
fn receive(module: &mut ModInstance<()>) -> String {
    let until = Instant::now() + Duration::from_secs(10);
    while Instant::now() < until {
        let answer = run(module, "recv").expect("main instance still runs");
        if answer != "none" {
            return answer;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("no message from the worker");
}

#[test]
fn worker_echoes_through_the_channel_and_main_never_blocks() {
    let log = Log::default();
    let game = scratch("echo");
    let Some(mut module) = worker_module(&game, "f2echo", &log) else { return };
    assert_eq!(run(&mut module, "recv").as_deref(), Some("none"));
    assert_eq!(run(&mut module, "send:echo:hi there").as_deref(), Some("ok"));
    assert_eq!(receive(&mut module), "hi there");
    // blocking waits are fine in the worker
    assert_eq!(run(&mut module, "send:sleep").as_deref(), Some("ok"));
    assert_eq!(receive(&mut module), "slept");
    // files, and its output lines, work in the worker too
    assert_eq!(run(&mut module, "send:files").as_deref(), Some("ok"));
    assert_eq!(receive(&mut module), "files:true");
    assert_eq!(std::fs::read_to_string(game.join("mod-data/f2echo/w.txt")).unwrap(), "from-worker");
    assert_eq!(run(&mut module, "send:eprint").as_deref(), Some("ok"));
    assert_eq!(receive(&mut module), "printed");
    assert!(log.contains("wasm mod f2echo worker [stdout]: worker started"));
    assert!(log.contains("wasm mod f2echo worker [stderr]: worker stderr line"));
    // unloading the mod stops the worker
    drop(module);
    assert!(log.contains("finished") || log.contains("stopped"), "{:?}", log.0.lock().unwrap());
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn worker_connects_tcp_to_a_local_listener() {
    let log = Log::default();
    let game = scratch("tcp");
    let Some(mut module) = worker_module(&game, "f2tcp", &log) else { return };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut ping = [0u8; 4];
        stream.read_exact(&mut ping).unwrap();
        assert_eq!(&ping, b"ping");
        stream.write_all(b"pong").unwrap();
    });
    assert_eq!(run(&mut module, &format!("send:tcp:127.0.0.1:{port}")).as_deref(), Some("ok"));
    assert_eq!(receive(&mut module), "tcp:pong");
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn worker_gets_an_http_page_from_a_local_server() {
    let log = Log::default();
    let game = scratch("http");
    let Some(mut module) = worker_module(&game, "f2http", &log) else { return };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0u8; 1];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8_lossy(&request).into_owned();
        assert!(request.starts_with("GET /hello?x=1 HTTP/1.1"), "{request}");
        stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\nworld").unwrap();
    });
    assert_eq!(run(&mut module, &format!("send:http:127.0.0.1:{port}/hello?x=1")).as_deref(), Some("ok"));
    assert_eq!(receive(&mut module), "http:200:world");
    server.join().unwrap();
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn a_full_queue_returns_an_error_instead_of_blocking() {
    let log = Log::default();
    let game = scratch("flood");
    let Some(mut module) = worker_module(&game, "f2flood", &log) else { return };
    // keep the worker busy so it does not drain the queue while it fills
    assert_eq!(run(&mut module, "send:sleep").as_deref(), Some("ok"));
    let began = Instant::now();
    let answer = run(&mut module, "flood:5000").unwrap();
    assert!(began.elapsed() < Duration::from_secs(2), "send must not wait");
    let (sent, error) = answer.split_once(':').unwrap();
    assert_eq!(error, "queue-full", "{answer}");
    let sent: usize = sent.parse().unwrap();
    assert!((1000..=1100).contains(&sent), "{answer}");
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn a_worker_trap_disables_only_the_worker() {
    let log = Log::default();
    let game = scratch("trap");
    let Some(mut module) = worker_module(&game, "f2trap", &log) else { return };
    assert_eq!(run(&mut module, "send:trap").as_deref(), Some("ok"));
    assert!(log.wait_for("the worker is disabled"), "{:?}", log.0.lock().unwrap());
    // the main instance carries on; sends now fail
    assert_eq!(run(&mut module, "recv").as_deref(), Some("none"));
    assert!(!module.is_disabled());
    assert_eq!(run(&mut module, "send:echo:x").as_deref(), Some("error: closed"));
    let _ = std::fs::remove_dir_all(game);
}

#[test]
fn a_mod_without_a_worker_gets_no_worker_errors() {
    let Some(main) = load(&F2_MAIN) else { return };
    let mut module = relaxed(Sandbox::new().compile(&main).unwrap().instantiate("noworker", (), |_| Ok(())).unwrap());
    assert_eq!(run(&mut module, "send:x").as_deref(), Some("error: no-worker"));
    assert_eq!(run(&mut module, "recv").as_deref(), Some("none"));
}
