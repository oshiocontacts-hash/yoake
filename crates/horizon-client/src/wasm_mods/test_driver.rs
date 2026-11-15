//! The operations of the engine test driver (test mode only, `--yoake-test`; `yoake_test.rs`).
//!
//! They run on the integrated server thread, between two ticks, as if they were a hook of a server mod: the
//! game's `Host` is lent to the same thread-local scope the server services read, and each operation calls
//! the very services a `server-full` mod calls (`server_services.rs`). So a test sees the world through
//! the code paths a mod does, and the commands it runs go through the same denylist (`command_gate`).
//! Nothing here is reachable from a mod: the only caller is the pump of the server loop, which exists
//! only in a worker process that was started with `--yoake-test-run`.
use super::full::{GameServerHost, uuid_text};
use super::*;
use yoake_test_runner::bridge::{
    Answer, EntityRow, MAX_ENTITIES, Op, PlayerRow, Reply, break_block_command, mannequin_uuid, place_block_command, player_name,
    same_state, state_parts, summon_command,
};
use yoake_wasm::{ServerHost, server_types as st};

/// The package id the test driver calls other modules under (what a callee's `handle-call` sees as the caller).
const CALLER: &str = "yoake-test";
/// The test world is the overworld.
const DIMENSION: &str = "minecraft:overworld";
/// How far from where it was summoned a new entity is looked for (it may fall or be pushed a little).
const SPAWN_SEARCH: f64 = 8.0;

fn block_pos(position: [i32; 3]) -> st::BlockPos {
    st::BlockPos { x: position[0], y: position[1], z: position[2] }
}

fn vec3(position: [f64; 3]) -> st::Vec3 {
    st::Vec3 { x: position[0], y: position[1], z: position[2] }
}

/// Runs `op` with `host` as the game. `Op::Advance` is the bridge's own and never gets here.
pub(in crate::entry) fn execute(host: &mut dyn Host, op: Op) -> Answer {
    let mut invocation = Invocation { host: HostAccess::Write(host), batch: None, batch_start: 0 };
    let _scope = Scope::enter(&mut invocation);
    // The game's native entity queries (and the guard around native mods) want a mod call to be active: the
    // driver is one, under a mod index no real mod has.
    let _call = crate::crash::enter_native_mod(crate::crash::NativeModCall { id: Some(CALLER), index: usize::MAX, hook: "yoake_test" });
    let mut services = GameServerHost { id: CALLER.to_owned(), commands: true };
    run(&mut services, op)
}

fn command(services: &mut GameServerHost, line: String, what: &str) -> Result<(), String> {
    if services.run_command(line.clone())? {
        Ok(())
    } else {
        Err(format!("{what} did not succeed (command: {line})"))
    }
}

fn run(services: &mut GameServerHost, op: Op) -> Answer {
    Ok(match op {
        Op::Advance(_) => return Err("advance-ticks is the bridge's".into()),
        Op::PlaceBlock { position, block } => {
            let line = place_block_command(position, &block)?;
            // `/setblock` of the state a block already has "fails": that is a block that is there, not an error.
            if !services.run_command(line.clone())? {
                // The `block` service names the block without its properties: the id must be the wanted one and
                // each wanted property must read back (`block-property`) with the wanted value.
                let actual = services.block(block_pos(position))?;
                let (wanted_id, wanted_properties) = state_parts(&block);
                let mut same = state_parts(&actual).0 == wanted_id;
                for (key, value) in &wanted_properties {
                    same = same && services.block_property(block_pos(position), key.clone())?.as_deref() == Some(value.as_str());
                }
                if !same {
                    return Err(format!("could not place {block} at {position:?}: the block there is {actual} (is the chunk loaded?)"));
                }
            }
            Reply::Done
        }
        Op::BreakBlock(position) => {
            if !services.run_command(break_block_command(position))? {
                // Breaking air does not succeed either.
                let actual = services.block(block_pos(position))?;
                if !same_state(&actual, "minecraft:air") {
                    return Err(format!("could not break the block at {position:?}: it is {actual} (is the chunk loaded?)"));
                }
            }
            Reply::Done
        }
        Op::BlockAt(position) => Reply::Text(services.block(block_pos(position))?),
        Op::SpawnEntity { kind, position } => {
            // The command does not say which entity it made: the entity of that kind that is new afterwards. A
            // second new one (another mob spawned meanwhile) makes the answer ambiguous, so it is an error.
            let known = |services: &mut GameServerHost| -> Result<std::collections::HashSet<u32>, String> {
                Ok(services
                    .entities_in(vec3(position), DIMENSION.to_owned(), SPAWN_SEARCH)?
                    .into_iter()
                    .filter(|entity| entity.kind == kind)
                    .map(|entity| entity.handle)
                    .collect())
            };
            let before = known(services)?;
            command(services, summon_command(&kind, position)?, "summoning the entity")?;
            let new: Vec<u32> = known(services)?.difference(&before).copied().collect();
            match new[..] {
                [id] => Reply::Id(id),
                [] => return Err(format!("the summoned {kind} was not found near {position:?}")),
                _ => return Err(format!("{} new {kind} entities appeared near {position:?}: cannot tell which one was summoned", new.len())),
            }
        }
        Op::SpawnMannequin { name, position } => {
            let uuid = uuid_text(mannequin_uuid(player_name(&name)?));
            match services.spawn_mannequin(uuid, "standing".into(), vec3(position), 0.0)? {
                Some(id) => Reply::Id(id),
                None => return Err("the server did not spawn the mannequin".into()),
            }
        }
        Op::EntitiesNear { center, radius } => Reply::Entities(
            services
                .entities_in(vec3(center), DIMENSION.to_owned(), radius)?
                .into_iter()
                .filter(|entity| entity.alive)
                .take(MAX_ENTITIES)
                .map(|entity| EntityRow { id: entity.handle, kind: entity.kind, position: [entity.position.x, entity.position.y, entity.position.z] })
                .collect(),
        ),
        Op::Players => {
            let owner = services.integrated_owner();
            let mut players = services.server_players()?;
            // The world's owner first, the others as the server lists them.
            players.sort_by_key(|player| Some(&player.uuid) != owner.as_ref());
            Reply::Players(
                players
                    .into_iter()
                    .map(|player| PlayerRow {
                        uuid: player.uuid,
                        name: player.name,
                        position: [player.position.x, player.position.y, player.position.z],
                        health: player.health,
                        game_mode: player.game_mode,
                    })
                    .collect(),
            )
        }
        Op::RunCommand(line) => Reply::Bool(services.run_command(line)?),
        Op::ModCall { target, method, payload } => Reply::Bytes(services.mod_call(method, payload, target)?),
    })
}
