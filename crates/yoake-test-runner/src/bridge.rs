//! The bridge between a worker's test thread and the game's server thread.
//!
//! The test module runs on its own thread and calls the driver synchronously. Each call becomes an [`Op`]
//! that waits in a queue until the server thread's loop reaches [`Bridge::pump`] (once per iteration,
//! between two ticks); the server thread executes it with the game's own services and answers. `advance-ticks`
//! is the one operation that does not answer at once: the bridge counts the ticks the server loop
//! reports through [`Bridge::tick_done`] and answers after the last, and while one is pending
//! [`Bridge::ticking_fast`] tells the loop to skip its sleep, as vanilla's `/tick sprint` does.
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use sha2::{Digest, Sha256};
use yoake_wasm::TestHost;
use yoake_wasm::test_driver::{BlockPos, EntityInfo, PlayerInfo, Vec3};

/// One thing the test module asked of the world.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// Run this many more server ticks.
    Advance(u32),
    PlaceBlock { position: [i32; 3], block: String },
    BreakBlock([i32; 3]),
    BlockAt([i32; 3]),
    SpawnEntity { kind: String, position: [f64; 3] },
    SpawnMannequin { name: String, position: [f64; 3] },
    EntitiesNear { center: [f64; 3], radius: f64 },
    Players,
    RunCommand(String),
    ModCall { target: String, method: String, payload: Vec<u8> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityRow {
    pub id: u32,
    pub kind: String,
    pub position: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlayerRow {
    pub uuid: String,
    pub name: String,
    pub position: [f64; 3],
    pub health: f32,
    pub game_mode: i32,
}

/// What the server thread answers.
#[derive(Clone, Debug, PartialEq)]
pub enum Reply {
    Done,
    Bool(bool),
    Text(String),
    Id(u32),
    Entities(Vec<EntityRow>),
    Players(Vec<PlayerRow>),
    Bytes(Vec<u8>),
}

pub type Answer = Result<Reply, String>;

struct Request {
    op: Op,
    reply: SyncSender<Answer>,
}

/// The server thread's end.
pub struct Bridge {
    inbox: Mutex<Receiver<Request>>,
    advance: Mutex<Option<(u32, SyncSender<Answer>)>>,
    fast: AtomicBool,
    ticks: Arc<AtomicU64>,
    /// Set once the world is frozen: its ticks then run only for `advance-ticks`.
    frozen: Arc<AtomicBool>,
    /// The server loop has seen the freeze: no tick is in flight.
    acked: Arc<AtomicBool>,
    signal: Arc<Signal>,
}

/// "A request arrived": lets the server thread sleep until one does, without losing a wake-up.
#[derive(Default)]
struct Signal {
    arrived: Mutex<bool>,
    changed: Condvar,
}

impl Signal {
    fn notify(&self) {
        *self.arrived.lock().unwrap_or_else(|error| error.into_inner()) = true;
        self.changed.notify_one();
    }

    /// Waits up to `limit` for a notification; whether there was one.
    fn wait(&self, limit: Duration) -> bool {
        let mut arrived = self.arrived.lock().unwrap_or_else(|error| error.into_inner());
        if !*arrived {
            arrived = self.changed.wait_timeout(arrived, limit).unwrap_or_else(|error| error.into_inner()).0;
        }
        std::mem::take(&mut *arrived)
    }
}

/// The test thread's end. Cloning it shares the queue and the tick counter.
#[derive(Clone)]
pub struct Client {
    outbox: Sender<Request>,
    ticks: Arc<AtomicU64>,
    frozen: Arc<AtomicBool>,
    acked: Arc<AtomicBool>,
    signal: Arc<Signal>,
    wait: Duration,
}

/// A bridge and its client; a request that waits longer than `wait` for the server thread fails.
pub fn channel(wait: Duration) -> (Bridge, Client) {
    let (outbox, inbox) = mpsc::channel();
    let ticks = Arc::new(AtomicU64::new(0));
    let frozen = Arc::new(AtomicBool::new(false));
    let acked = Arc::new(AtomicBool::new(false));
    let signal = Arc::new(Signal::default());
    (
        Bridge {
            inbox: Mutex::new(inbox),
            advance: Mutex::new(None),
            fast: AtomicBool::new(false),
            ticks: Arc::clone(&ticks),
            frozen: Arc::clone(&frozen),
            acked: Arc::clone(&acked),
            signal: Arc::clone(&signal),
        },
        Client { outbox, ticks, frozen, acked, signal, wait },
    )
}

impl Bridge {
    /// Executes every queued request with `execute`. Called by the server loop once per iteration, on the
    /// server thread, before the tick. While an `advance-ticks` is pending nothing else is taken (the test
    /// thread is blocked on it anyway).
    pub fn pump(&self, execute: &mut dyn FnMut(Op) -> Answer) {
        loop {
            if self.advance.lock().unwrap_or_else(|error| error.into_inner()).is_some() {
                return;
            }
            let request = match self.inbox.lock().unwrap_or_else(|error| error.into_inner()).try_recv() {
                Ok(request) => request,
                Err(_) => return,
            };
            match request.op {
                Op::Advance(ticks) => {
                    *self.advance.lock().unwrap_or_else(|error| error.into_inner()) = Some((ticks, request.reply));
                    self.fast.store(true, Ordering::Relaxed);
                    return;
                }
                op => {
                    // The test thread may have given up (its wait ran out): nobody to answer then.
                    let _ = request.reply.send(execute(op));
                }
            }
        }
    }

    /// The server loop ran one tick.
    pub fn tick_done(&self) {
        self.ticks.fetch_add(1, Ordering::Relaxed);
        let mut advance = self.advance.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((left, _)) = advance.as_mut() {
            *left = left.saturating_sub(1);
            if *left == 0 {
                // The world stands still again before the test hears the ticks are done.
                self.fast.store(false, Ordering::Relaxed);
                if let Some((_, reply)) = advance.take() {
                    let _ = reply.send(Ok(Reply::Done));
                }
            }
        }
    }

    /// Whether the loop should run its next tick at once instead of waiting out the 50 ms.
    pub fn ticking_fast(&self) -> bool {
        self.fast.load(Ordering::Relaxed)
    }

    /// Whether the world stands still now: the test froze it and no `advance-ticks` is running. The server
    /// loop then only services its connections and the bridge; no tick of the world runs.
    pub fn frozen(&self) -> bool {
        self.frozen.load(Ordering::Relaxed) && !self.fast.load(Ordering::Relaxed)
    }

    /// The server loop's serving step: executes the queued requests and, while the world is frozen, goes on
    /// waiting for more (up to `idle` since the last one), so a test's calls cost what they cost and not
    /// a tick each. Returns once the world should tick (an `advance-ticks` arrived), or nothing came for
    /// `idle` (the loop then services its connections).
    pub fn serve(&self, execute: &mut dyn FnMut(Op) -> Answer, idle: Duration) {
        // The loop is back at its top: a tick that was running when the world was frozen is over.
        if self.frozen.load(Ordering::Relaxed) {
            self.acked.store(true, Ordering::SeqCst);
        }
        self.pump(execute);
        while self.frozen() {
            if !self.signal.wait(idle) {
                return;
            }
            self.pump(execute);
        }
    }

    /// Ticks run since the bridge was made.
    pub fn ticks(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }
}

impl Client {
    /// Freezes the world: from now on it ticks only for `advance-ticks`. The test thread does this once the
    /// world is up and has had its warm-up.
    pub fn freeze(&self) {
        self.frozen.store(true, Ordering::Relaxed);
        self.signal.notify();
        // Wait until the server loop has seen it, so the tick that may be running now is counted before the
        // test reads `tick()`. A loop that never answers (no world) does not hold the test up for long.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while !self.acked.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    pub fn request(&self, op: Op) -> Answer {
        self.request_within(op, self.wait)
    }

    pub fn request_within(&self, op: Op, wait: Duration) -> Answer {
        let (reply, answer) = mpsc::sync_channel(1);
        self.outbox.send(Request { op, reply }).map_err(|_| "the test world is gone".to_owned())?;
        self.signal.notify();
        match answer.recv_timeout(wait) {
            Ok(answer) => answer,
            Err(RecvTimeoutError::Timeout) => Err(format!("the test world did not answer within {} ms", wait.as_millis())),
            Err(RecvTimeoutError::Disconnected) => Err("the test world is gone".to_owned()),
        }
    }

    pub fn ticks(&self) -> u64 {
        self.ticks.load(Ordering::Relaxed)
    }
}

// ---------------------------------------------------------------------------------------------
// Commands the operations become (checked here, so the game's executor has nothing to guess)
// ---------------------------------------------------------------------------------------------

/// A block state as `/setblock` takes it: `[namespace:]id` and optional `[property=value,...]`, plain
/// characters only (nothing that could start a second command argument or a tag/NBT part).
pub fn block_state(block: &str) -> Result<&str, String> {
    let ok = !block.is_empty()
        && block.len() <= 256
        && block.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"_:.[]=,/-".contains(&byte))
        && block.bytes().next().is_some_and(|byte| byte.is_ascii_alphabetic());
    if ok { Ok(block) } else { Err(format!("not a block state: {:.64}", block)) }
}

/// A `namespace:path` registry id (an entity kind).
pub fn registry_id(id: &str) -> Result<&str, String> {
    let part = |text: &str, extra: &[u8]| {
        !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || extra.contains(&byte))
    };
    match id.split_once(':') {
        Some((namespace, path)) if id.len() <= 128 && part(namespace, b"_.-") && part(path, b"_./-") => Ok(id),
        _ => Err(format!("not a registry id (namespace:path): {:.64}", id)),
    }
}

pub fn place_block_command(position: [i32; 3], block: &str) -> Result<String, String> {
    let [x, y, z] = position;
    Ok(format!("setblock {x} {y} {z} {}", block_state(block)?))
}

pub fn break_block_command(position: [i32; 3]) -> String {
    let [x, y, z] = position;
    format!("setblock {x} {y} {z} minecraft:air destroy")
}

pub fn summon_command(kind: &str, position: [f64; 3]) -> Result<String, String> {
    let [x, y, z] = position;
    if !position.iter().all(|value| value.is_finite()) {
        return Err("positions must be finite".into());
    }
    Ok(format!("summon {} {x} {y} {z}", registry_id(kind)?))
}

/// A block state as `(id, sorted properties)`: `stone` and `minecraft:stone[]` are the same, and so are
/// `a[x=1,y=2]` and `minecraft:a[y=2,x=1]`.
fn normalized_state(text: &str) -> (String, Vec<(String, String)>) {
    let (id, properties) = match text.split_once('[') {
        Some((id, rest)) => (id, rest.trim_end_matches(']')),
        None => (text, ""),
    };
    let id = if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") };
    let mut properties: Vec<(String, String)> = properties
        .split(',')
        .filter(|pair| !pair.is_empty())
        .map(|pair| pair.split_once('=').map_or((pair.to_owned(), String::new()), |(key, value)| (key.to_owned(), value.to_owned())))
        .collect();
    properties.sort();
    (id, properties)
}

/// A block state text as `(namespaced id, properties sorted)`.
pub fn state_parts(text: &str) -> (String, Vec<(String, String)>) {
    normalized_state(text)
}

/// Whether the block state text `actual` (what the game reports) is the state `wanted` (what a test asked
/// for): `/setblock` of the state a block already has does not succeed, and that is not a failure here.
pub fn same_state(actual: &str, wanted: &str) -> bool {
    normalized_state(actual) == normalized_state(wanted)
}

/// A Minecraft player name: 1 to 16 letters, digits and underscores.
pub fn player_name(name: &str) -> Result<&str, String> {
    let ok = (1..=16).contains(&name.len()) && name.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if ok { Ok(name) } else { Err(format!("not a player name (1 to 16 letters, digits, _): {:.32}", name)) }
}

/// The UUID of a mannequin: a version-4 shaped hash of its name, the same for every run.
pub fn mannequin_uuid(name: &str) -> u128 {
    let digest = Sha256::digest(format!("yoake-test:mannequin:{name}").as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = bytes[6] & 0x0f | 0x40;
    bytes[8] = bytes[8] & 0x3f | 0x80;
    u128::from_be_bytes(bytes)
}

// ---------------------------------------------------------------------------------------------
// The driver, as the test module sees it
// ---------------------------------------------------------------------------------------------

/// Most entities one `entities-near` answers.
pub const MAX_ENTITIES: usize = 512;

/// The `test-driver` host of a worker: every call is an [`Op`] for the server thread.
pub struct ChannelHost {
    client: Client,
}

impl ChannelHost {
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    fn done(&self, op: Op) -> Result<(), String> {
        match self.client.request(op)? {
            Reply::Done | Reply::Bool(true) => Ok(()),
            Reply::Bool(false) => Err("the server said no".into()),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }
}

fn array(position: &Vec3) -> [f64; 3] {
    [position.x, position.y, position.z]
}

fn vec3(position: [f64; 3]) -> Vec3 {
    Vec3 { x: position[0], y: position[1], z: position[2] }
}

fn block_array(position: &BlockPos) -> [i32; 3] {
    [position.x, position.y, position.z]
}

impl TestHost for ChannelHost {
    fn advance_ticks(&mut self, ticks: u32) -> Result<(), String> {
        self.done(Op::Advance(ticks))
    }

    fn tick(&mut self) -> u64 {
        self.client.ticks()
    }

    fn place_block(&mut self, position: BlockPos, block: &str) -> Result<(), String> {
        block_state(block)?;
        self.done(Op::PlaceBlock { position: block_array(&position), block: block.to_owned() })
    }

    fn break_block(&mut self, position: BlockPos) -> Result<(), String> {
        self.done(Op::BreakBlock(block_array(&position)))
    }

    fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
        match self.client.request(Op::BlockAt(block_array(&position)))? {
            Reply::Text(block) => Ok(block),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn spawn_entity(&mut self, kind: &str, position: Vec3) -> Result<u32, String> {
        registry_id(kind)?;
        match self.client.request(Op::SpawnEntity { kind: kind.to_owned(), position: array(&position) })? {
            Reply::Id(id) => Ok(id),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn spawn_mannequin(&mut self, name: &str, position: Vec3) -> Result<u32, String> {
        player_name(name)?;
        match self.client.request(Op::SpawnMannequin { name: name.to_owned(), position: array(&position) })? {
            Reply::Id(id) => Ok(id),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn entities_near(&mut self, center: Vec3, radius: f64) -> Result<Vec<EntityInfo>, String> {
        match self.client.request(Op::EntitiesNear { center: array(&center), radius })? {
            Reply::Entities(rows) => Ok(rows
                .into_iter()
                .take(MAX_ENTITIES)
                .map(|row| EntityInfo { id: row.id, kind: row.kind, position: vec3(row.position) })
                .collect()),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn players(&mut self) -> Result<Vec<PlayerInfo>, String> {
        match self.client.request(Op::Players)? {
            Reply::Players(rows) => Ok(rows
                .into_iter()
                .map(|row| PlayerInfo {
                    uuid: row.uuid,
                    name: row.name,
                    position: vec3(row.position),
                    health: row.health,
                    game_mode: row.game_mode,
                })
                .collect()),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn run_command(&mut self, command: &str) -> Result<bool, String> {
        match self.client.request(Op::RunCommand(command.to_owned()))? {
            Reply::Bool(succeeded) => Ok(succeeded),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn mod_call(&mut self, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        match self.client.request(Op::ModCall { target: target.to_owned(), method: method.to_owned(), payload: payload.to_vec() })? {
            Reply::Bytes(bytes) => Ok(bytes),
            other => Err(format!("unexpected answer {other:?}")),
        }
    }

    fn log(&mut self, message: &str) {
        eprintln!("yoake-test: {}", message.chars().filter(|character| !character.is_control()).take(1024).collect::<String>());
    }

    fn client_count(&mut self) -> u32 {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Instant;

    /// A server thread: runs the loop `pump`, `tick_done` until told to stop; answers `RunCommand` with
    /// success and `BlockAt` with the position, so the tests can see the order and the thread.
    fn server(bridge: Arc<Bridge>, stop: Arc<AtomicBool>) -> thread::JoinHandle<Vec<String>> {
        thread::spawn(move || {
            let mut seen = Vec::new();
            while !stop.load(Ordering::Relaxed) {
                bridge.pump(&mut |op| {
                    seen.push(format!("{op:?}"));
                    match op {
                        Op::RunCommand(_) => Ok(Reply::Bool(true)),
                        Op::BlockAt([x, y, z]) => Ok(Reply::Text(format!("block {x},{y},{z}"))),
                        Op::PlaceBlock { .. } => Err("not loaded".into()),
                        _ => Ok(Reply::Done),
                    }
                });
                bridge.tick_done();
                if !bridge.ticking_fast() {
                    thread::sleep(Duration::from_millis(2));
                }
            }
            seen
        })
    }

    #[test]
    fn requests_are_answered_in_order_by_the_server_thread() {
        let (bridge, client) = channel(Duration::from_secs(5));
        let (bridge, stop) = (Arc::new(bridge), Arc::new(AtomicBool::new(false)));
        let handle = server(Arc::clone(&bridge), Arc::clone(&stop));
        let mut host = ChannelHost::new(client);
        assert_eq!(host.run_command("say hi"), Ok(true));
        assert_eq!(host.block_at(BlockPos { x: 1, y: -60, z: 2 }), Ok("block 1,-60,2".to_owned()));
        // The server's refusal is the test's error, unchanged.
        assert_eq!(host.place_block(BlockPos { x: 0, y: 0, z: 0 }, "minecraft:stone"), Err("not loaded".to_owned()));
        stop.store(true, Ordering::Relaxed);
        let seen = handle.join().unwrap();
        assert_eq!(
            seen,
            ["RunCommand(\"say hi\")", "BlockAt([1, -60, 2])", "PlaceBlock { position: [0, 0, 0], block: \"minecraft:stone\" }"]
        );
    }

    #[test]
    fn advance_answers_after_exactly_that_many_ticks_and_runs_fast() {
        let (bridge, client) = channel(Duration::from_secs(5));
        let (bridge, stop) = (Arc::new(bridge), Arc::new(AtomicBool::new(false)));
        let handle = server(Arc::clone(&bridge), Arc::clone(&stop));
        let mut host = ChannelHost::new(client);
        let before = host.tick();
        let began = Instant::now();
        assert_eq!(host.advance_ticks(200), Ok(()));
        // 200 real ticks would take 10 s: the bridge made the loop skip its wait.
        assert!(began.elapsed() < Duration::from_secs(8), "{:?}", began.elapsed());
        let after = host.tick();
        assert!(after - before >= 200, "{before} -> {after}");
        assert!(!bridge.ticking_fast(), "the loop goes back to 20 ticks a second");
        stop.store(true, Ordering::Relaxed);
        handle.join().unwrap();
    }

    #[test]
    fn a_pending_advance_holds_the_next_request_until_its_ticks_are_done() {
        let (bridge, client) = channel(Duration::from_secs(5));
        let advancing = {
            let client = client.clone();
            thread::spawn(move || client.request(Op::Advance(2)))
        };
        let mut executed = Vec::new();
        // The advance is taken (it executes nothing) and starts the fast ticks.
        let deadline = Instant::now() + Duration::from_secs(5);
        while !bridge.ticking_fast() && Instant::now() < deadline {
            bridge.pump(&mut |op| {
                executed.push(op);
                Ok(Reply::Done)
            });
            thread::yield_now();
        }
        assert!(bridge.ticking_fast());
        let commanding = thread::spawn(move || client.request(Op::RunCommand("x".into())));
        thread::sleep(Duration::from_millis(50));
        bridge.pump(&mut |op| {
            executed.push(op);
            Ok(Reply::Bool(true))
        });
        assert!(executed.is_empty(), "nothing runs while the advance is pending: {executed:?}");
        bridge.tick_done();
        assert!(bridge.ticking_fast(), "one tick of two");
        bridge.tick_done();
        assert!(!bridge.ticking_fast());
        assert_eq!(advancing.join().unwrap(), Ok(Reply::Done));
        bridge.pump(&mut |op| {
            executed.push(op);
            Ok(Reply::Bool(true))
        });
        assert_eq!(commanding.join().unwrap(), Ok(Reply::Bool(true)));
        assert_eq!(executed, [Op::RunCommand("x".into())]);
        assert_eq!(bridge.ticks(), 2);
    }

    #[test]
    fn a_frozen_world_serves_many_calls_without_waiting_for_ticks() {
        let (bridge, client) = channel(Duration::from_secs(20));
        let (bridge, stop) = (Arc::new(bridge), Arc::new(AtomicBool::new(false)));
        let server = {
            let (bridge, stop) = (Arc::clone(&bridge), Arc::clone(&stop));
            thread::spawn(move || {
                let mut ticked_while_frozen = 0;
                while !stop.load(Ordering::Relaxed) {
                    // The server loop: serve (this blocks while frozen and idle), then a tick or just the connections.
                    bridge.serve(&mut |_| Ok(Reply::Done), Duration::from_millis(50));
                    if bridge.frozen() {
                        continue;
                    }
                    if bridge.ticking_fast() {
                        bridge.tick_done();
                    } else {
                        // Not frozen yet: the world ticks at its own pace.
                        thread::sleep(Duration::from_millis(5));
                        bridge.tick_done();
                        ticked_while_frozen += usize::from(client_frozen(&bridge));
                    }
                }
                ticked_while_frozen
            })
        };
        fn client_frozen(bridge: &Bridge) -> bool {
            bridge.frozen.load(Ordering::Relaxed)
        }
        client.freeze();
        // freeze() returns only once no tick is in flight: the counter does not move afterwards.
        let frozen_at = client.ticks();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(client.ticks(), frozen_at, "a tick ran after freeze() returned");
        let began = Instant::now();
        for _ in 0..2000 {
            assert_eq!(client.request(Op::Players), Ok(Reply::Done));
        }
        // At one call per 50 ms tick these would take 100 s.
        assert!(began.elapsed() < Duration::from_secs(10), "{:?}", began.elapsed());
        let ticks = client.ticks();
        assert_eq!(client.request(Op::Advance(3)), Ok(Reply::Done));
        assert_eq!(client.ticks() - ticks, 3, "a frozen world ticks exactly as often as it was told to");
        assert!(bridge.frozen(), "frozen again once the ticks are done");
        stop.store(true, Ordering::Relaxed);
        client.freeze();
        assert_eq!(server.join().unwrap(), 0);
    }

    #[test]
    fn block_states_compare_by_meaning() {
        assert!(same_state("minecraft:stone", "stone"));
        assert!(same_state("minecraft:oak_stairs[facing=north,half=top]", "oak_stairs[half=top,facing=north]"));
        assert!(same_state("minecraft:stone[]", "minecraft:stone"));
        assert!(!same_state("minecraft:dirt", "minecraft:stone"));
        assert!(!same_state("minecraft:oak_stairs[facing=south]", "minecraft:oak_stairs[facing=north]"));
        assert!(!same_state("minecraft:oak_stairs[facing=south]", "minecraft:oak_stairs"));
        let (id, properties) = state_parts("oak_stairs[half=top,facing=north]");
        assert_eq!(id, "minecraft:oak_stairs");
        assert_eq!(properties, [("facing".to_owned(), "north".to_owned()), ("half".to_owned(), "top".to_owned())]);
    }

    #[test]
    fn a_request_nobody_serves_times_out_with_a_clear_error() {
        let (_bridge, client) = channel(Duration::from_millis(50));
        let error = client.request(Op::Players).unwrap_err();
        assert!(error.contains("did not answer within 50 ms"), "{error}");
    }

    #[test]
    fn a_request_to_a_dead_world_fails_at_once() {
        let (bridge, client) = channel(Duration::from_secs(60));
        drop(bridge);
        let began = Instant::now();
        assert!(client.request(Op::Players).unwrap_err().contains("gone"));
        assert!(began.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_driver_checks_names_before_it_asks_the_world() {
        let (_bridge, client) = channel(Duration::from_millis(10));
        let mut host = ChannelHost::new(client);
        let here = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
        // These fail on the check, not on the (absent) server's silence.
        assert!(host.place_block(BlockPos { x: 0, y: 0, z: 0 }, "stone 1 1 1").unwrap_err().starts_with("not a block state"));
        assert!(host.spawn_entity("Pig", here).unwrap_err().starts_with("not a registry id"));
        assert!(host.spawn_mannequin("two words", here).unwrap_err().starts_with("not a player name"));
    }

    #[test]
    fn commands_are_built_from_checked_parts() {
        assert_eq!(place_block_command([1, -60, 2], "minecraft:oak_stairs[facing=north,half=top]").unwrap(), "setblock 1 -60 2 minecraft:oak_stairs[facing=north,half=top]");
        assert_eq!(break_block_command([0, 5, -3]), "setblock 0 5 -3 minecraft:air destroy");
        assert_eq!(summon_command("minecraft:pig", [1.5, -60.0, 2.5]).unwrap(), "summon minecraft:pig 1.5 -60 2.5");
        for bad in ["", "stone replace", "stone\nstop", "stone{a:1}", "#minecraft:logs", "9stone", &"a".repeat(257)] {
            assert!(block_state(bad).is_err(), "{bad:?}");
        }
        for bad in ["pig", "Minecraft:pig", "minecraft:", ":pig", "minecraft:pig 1", "a:b\nc"] {
            assert!(registry_id(bad).is_err(), "{bad:?}");
        }
        assert!(summon_command("minecraft:pig", [f64::NAN, 0.0, 0.0]).is_err());
        assert!(player_name("Tester_1").is_ok() && player_name("").is_err() && player_name("seventeen_chars_x").is_err() && player_name("a b").is_err());
    }

    #[test]
    fn a_mannequin_has_a_stable_version_4_uuid() {
        let one = mannequin_uuid("Tester");
        assert_eq!(one, mannequin_uuid("Tester"));
        assert_ne!(one, mannequin_uuid("Other"));
        let text = format!("{one:032x}");
        assert_eq!(&text[12..13], "4", "version nibble: {text}");
        assert!(matches!(&text[16..17], "8" | "9" | "a" | "b"), "variant nibble: {text}");
    }
}
