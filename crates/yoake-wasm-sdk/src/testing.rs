use alloc::{format, string::String, vec::Vec};

/// Test modules run by the engine (`yoake test`): the host-backed [`TestDriver`] and `export_tests!`.
/// Needs the SDK feature `test`.
#[cfg(feature = "test")]
pub mod engine;

pub type TestResult = Result<(), String>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BlockPos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

/// What a test may do to its world. [`engine::HostDriver`] (feature `test`) runs it in the game; a
/// mock implements it for `cargo test`. The methods after `run_ticks` have defaults that answer
/// "unsupported", so a mock implements only what its tests use.
pub trait TestDriver {
    fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult;
    fn block_at(&mut self, position: BlockPos) -> Result<String, String>;
    fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String>;
    fn run_ticks(&mut self, ticks: u32) -> TestResult;

    /// Breaks the block as a player would (drops included).
    fn break_block(&mut self, position: BlockPos) -> TestResult {
        let _ = position;
        Err(String::from("unsupported: break_block"))
    }
    /// Spawns a mannequin (not a player: it is not in `player_names`) for `name` and returns its entity id.
    fn spawn_mannequin(&mut self, name: &str, position: BlockPos) -> Result<u64, String> {
        let _ = (name, position);
        Err(String::from("unsupported: spawn_mannequin"))
    }
    /// Runs a server command as the console; whether it succeeded.
    fn run_command(&mut self, command: &str) -> Result<bool, String> {
        let _ = command;
        Err(String::from("unsupported: run_command"))
    }
    /// Calls another loaded server module's `handle-call` (the mod under test, usually).
    fn mod_call(&mut self, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        let _ = (target, method, payload);
        Err(String::from("unsupported: mod_call"))
    }
    /// The kinds of the entities within `radius` blocks of `position`.
    fn entities_near(&mut self, position: BlockPos, radius: f64) -> Result<Vec<String>, String> {
        let _ = (position, radius);
        Err(String::from("unsupported: entities_near"))
    }
    /// The names of the players on the server, the world's owner first.
    fn player_names(&mut self) -> Result<Vec<String>, String> {
        Err(String::from("unsupported: player_names"))
    }
    /// The server ticks the world has run since the test began.
    fn tick(&mut self) -> u64 {
        0
    }
    /// A line for the test log.
    fn log(&mut self, message: &str) {
        let _ = message;
    }
    /// How many clients take part: 1, or 2 in a two-client test (client 0 is the world's owner).
    fn client_count(&mut self) -> u32 {
        1
    }
    /// Sends one command line of the game's control protocol to client `index` (1); the JSON reply.
    fn client_call(&mut self, index: u32, command: &str) -> Result<String, String> {
        let _ = (index, command);
        Err(String::from("unsupported: client_call"))
    }
    /// Repeats `command` on client `index` until its reply contains `expect`, for at most `timeout_ms`.
    fn client_wait(&mut self, index: u32, command: &str, expect: &str, timeout_ms: u32) -> Result<String, String> {
        let _ = (index, command, expect, timeout_ms);
        Err(String::from("unsupported: client_wait"))
    }
    /// The names of the players on the world client `index` joined, as the server lists them.
    fn client_player_names(&mut self, index: u32) -> Result<Vec<String>, String> {
        let _ = index;
        Err(String::from("unsupported: client_player_names"))
    }
}

pub struct TestWorld<'driver> {
    driver: &'driver mut dyn TestDriver,
}

impl<'driver> TestWorld<'driver> {
    pub fn new(driver: &'driver mut dyn TestDriver) -> Self {
        Self { driver }
    }

    pub fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult {
        self.driver.place_block(position, block)
    }

    pub fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
        self.driver.block_at(position)
    }

    pub fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String> {
        self.driver.spawn_entity(kind, position)
    }

    pub fn run_ticks(&mut self, ticks: u32) -> TestResult {
        self.driver.run_ticks(ticks)
    }

    pub fn break_block(&mut self, position: BlockPos) -> TestResult {
        self.driver.break_block(position)
    }

    pub fn spawn_mannequin(&mut self, name: &str, position: BlockPos) -> Result<u64, String> {
        self.driver.spawn_mannequin(name, position)
    }

    pub fn run_command(&mut self, command: &str) -> Result<bool, String> {
        self.driver.run_command(command)
    }

    /// Runs `command` and fails the test when the server says it did not succeed.
    pub fn assert_command(&mut self, command: &str) -> TestResult {
        if self.driver.run_command(command)? {
            Ok(())
        } else {
            Err(format!("command did not succeed: {command}"))
        }
    }

    pub fn mod_call(&mut self, target: &str, method: &str, payload: &[u8]) -> Result<Vec<u8>, String> {
        self.driver.mod_call(target, method, payload)
    }

    pub fn entities_near(&mut self, position: BlockPos, radius: f64) -> Result<Vec<String>, String> {
        self.driver.entities_near(position, radius)
    }

    pub fn player_names(&mut self) -> Result<Vec<String>, String> {
        self.driver.player_names()
    }

    pub fn tick(&mut self) -> u64 {
        self.driver.tick()
    }

    /// How many clients take part (2 in a two-client test).
    pub fn client_count(&mut self) -> u32 {
        self.driver.client_count()
    }

    /// Client `index` of a two-client test (1 is the second game, which joined the world over loopback).
    pub fn client(&mut self, index: u32) -> ClientView<'_, 'driver> {
        ClientView { world: self, index }
    }

    pub fn log(&mut self, message: &str) {
        self.driver.log(message)
    }

    /// Fails unless exactly `count` entities of `kind` are within `radius` blocks of `position`.
    pub fn assert_entity_count(&mut self, kind: &str, position: BlockPos, radius: f64, count: usize) -> TestResult {
        let actual = self.driver.entities_near(position, radius)?.iter().filter(|found| *found == kind).count();
        if actual == count {
            Ok(())
        } else {
            Err(format!("entities {kind:?} near {position:?}: expected {count}, got {actual}"))
        }
    }

    pub fn assert_block(&mut self, position: BlockPos, expected: &str) -> TestResult {
        let actual = self.block_at(position)?;
        if actual == expected {
            Ok(())
        } else {
            Err(format!("block at {position:?}: expected {expected:?}, got {actual:?}"))
        }
    }
}

/// The text of the string field `name` of a JSON reply (`{"block":"minecraft:stone"}`), without unescaping:
/// block ids and player names have nothing to escape.
fn json_text_field(reply: &str, name: &str) -> Option<String> {
    let key = format!("\"{name}\":\"");
    let start = reply.find(&key)? + key.len();
    let end = reply[start..].find('"')?;
    Some(String::from(&reply[start..start + end]))
}

/// A second client of a two-client test, seen through its control endpoint.
pub struct ClientView<'world, 'driver> {
    world: &'world mut TestWorld<'driver>,
    index: u32,
}

impl ClientView<'_, '_> {
    /// Sends a control command line (`getblock`, `chat`, `chat-log`, `status`, ...) and returns the JSON reply.
    pub fn call(&mut self, command: &str) -> Result<String, String> {
        self.world.driver.client_call(self.index, command)
    }

    /// The block this client has at `position` (its own copy of the world, as the server sent it).
    pub fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
        let reply = self.call(&format!("getblock {} {} {}", position.x, position.y, position.z))?;
        json_text_field(&reply, "block").ok_or_else(|| format!("no block in the client's answer: {reply}"))
    }

    /// Waits (in real time, up to `timeout_ms`) until this client has `expected` at `position`: the way a test
    /// waits for a server change to arrive. `expected` is an id (`minecraft:stone`); properties may follow it.
    pub fn wait_block(&mut self, position: BlockPos, expected: &str, timeout_ms: u32) -> TestResult {
        let command = format!("getblock {} {} {}", position.x, position.y, position.z);
        let expect = format!("\"block\":\"{expected}");
        self.world.driver.client_wait(self.index, &command, &expect, timeout_ms).map(|_| ())
    }

    /// Makes this client say `text` in chat (one line of plain text, no leading slash).
    pub fn chat(&mut self, text: &str) -> TestResult {
        self.call(&format!("chat {text}")).map(|_| ())
    }

    /// Waits until `text` is among this client's last 30 chat lines.
    pub fn wait_chat(&mut self, text: &str, timeout_ms: u32) -> TestResult {
        self.world.driver.client_wait(self.index, "chat-log 30", text, timeout_ms).map(|_| ())
    }

    /// The names of the players on the world this client joined, as the server lists them.
    pub fn player_names(&mut self) -> Result<Vec<String>, String> {
        self.world.driver.client_player_names(self.index)
    }
}

pub struct TestCase {
    pub name: &'static str,
    pub run: for<'driver> fn(&mut TestWorld<'driver>) -> TestResult,
}

impl TestCase {
    pub fn run(&self, driver: &mut dyn TestDriver) -> TestResult {
        (self.run)(&mut TestWorld::new(driver))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[derive(Default)]
    struct MockDriver {
        block: Option<(BlockPos, String)>,
        spawned: Option<(String, BlockPos)>,
        ticks: u32,
    }

    impl TestDriver for MockDriver {
        fn place_block(&mut self, position: BlockPos, block: &str) -> TestResult {
            self.block = Some((position, block.to_string()));
            Ok(())
        }

        fn block_at(&mut self, position: BlockPos) -> Result<String, String> {
            self.block
                .as_ref()
                .filter(|(at, _)| *at == position)
                .map(|(_, block)| block.clone())
                .ok_or_else(|| "missing block".to_string())
        }

        fn spawn_entity(&mut self, kind: &str, position: BlockPos) -> Result<u64, String> {
            self.spawned = Some((kind.to_string(), position));
            Ok(17)
        }

        fn run_ticks(&mut self, ticks: u32) -> TestResult {
            self.ticks += ticks;
            Ok(())
        }
    }

    fn scenario(world: &mut TestWorld<'_>) -> TestResult {
        let position = BlockPos { x: 1, y: 2, z: 3 };
        world.place_block(position, "minecraft:stone")?;
        if world.spawn_entity("minecraft:pig", position)? != 17 {
            return Err("unexpected entity id".to_string());
        }
        world.run_ticks(2)?;
        world.assert_block(position, "minecraft:stone")
    }

    #[test]
    fn executes_registered_test_against_driver() {
        let mut driver = MockDriver::default();
        let case = TestCase { name: "scenario", run: scenario };
        assert_eq!(case.run(&mut driver), Ok(()));
        assert_eq!(driver.spawned, Some(("minecraft:pig".to_string(), BlockPos { x: 1, y: 2, z: 3 })));
        assert_eq!(driver.ticks, 2);
    }

    #[test]
    fn a_json_field_is_read_without_a_parser() {
        assert_eq!(json_text_field("{\"block\":\"minecraft:stone\"}", "block").as_deref(), Some("minecraft:stone"));
        assert_eq!(json_text_field("{\"ok\":true}", "block"), None);
        assert_eq!(json_text_field("{\"a\":\"x\",\"block\":\"y[z=1]\"}", "block").as_deref(), Some("y[z=1]"));
    }

    #[test]
    fn a_client_view_goes_through_the_driver_with_its_index() {
        #[derive(Default)]
        struct Calls(Vec<String>);
        impl TestDriver for Calls {
            fn place_block(&mut self, _: BlockPos, _: &str) -> TestResult { Ok(()) }
            fn block_at(&mut self, _: BlockPos) -> Result<String, String> { Ok(String::new()) }
            fn spawn_entity(&mut self, _: &str, _: BlockPos) -> Result<u64, String> { Ok(0) }
            fn run_ticks(&mut self, _: u32) -> TestResult { Ok(()) }
            fn client_count(&mut self) -> u32 { 2 }
            fn client_call(&mut self, index: u32, command: &str) -> Result<String, String> {
                self.0.push(format!("{index}:{command}"));
                Ok(String::from("{\"block\":\"minecraft:stone\"}"))
            }
            fn client_wait(&mut self, index: u32, command: &str, expect: &str, timeout_ms: u32) -> Result<String, String> {
                self.0.push(format!("{index}:{command}:{expect}:{timeout_ms}"));
                Ok(String::new())
            }
        }
        let mut driver = Calls::default();
        let mut world = TestWorld::new(&mut driver);
        assert_eq!(world.client_count(), 2);
        let at = BlockPos { x: 1, y: -60, z: 2 };
        assert_eq!(world.client(1).block_at(at).unwrap(), "minecraft:stone");
        world.client(1).wait_block(at, "minecraft:stone", 500).unwrap();
        world.client(1).chat("hello").unwrap();
        world.client(1).wait_chat("hello", 700).unwrap();
        assert_eq!(
            driver.0,
            ["1:getblock 1 -60 2", "1:getblock 1 -60 2:\"block\":\"minecraft:stone:500", "1:chat hello", "1:chat-log 30:hello:700"]
        );
    }

    #[test]
    fn block_assertion_reports_expected_and_actual() {
        let position = BlockPos { x: 1, y: 2, z: 3 };
        let mut driver = MockDriver {
            block: Some((position, "minecraft:dirt".to_string())),
            ..Default::default()
        };
        assert_eq!(
            TestWorld::new(&mut driver).assert_block(position, "minecraft:stone"),
            Err("block at BlockPos { x: 1, y: 2, z: 3 }: expected \"minecraft:stone\", got \"minecraft:dirt\"".to_string())
        );
    }

    #[test]
    fn block_assertion_propagates_driver_errors() {
        let mut driver = MockDriver::default();
        assert_eq!(
            TestWorld::new(&mut driver).assert_block(BlockPos { x: 1, y: 2, z: 3 }, "minecraft:stone"),
            Err("missing block".to_string())
        );
    }
}
