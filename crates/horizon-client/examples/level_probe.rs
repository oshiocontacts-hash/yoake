//! A protocol-level client for the levels e2e (`tools/levels_e2e.py`, F3-7): it joins a LAN host as a plain
//! multiplayer client would, with no game, no window and no Horizon-only feature used, and reports as JSON
//! lines what the server's protocol told it about the world's levels:
//!
//!   level_probe ADDRESS NAME RESOURCES "execute in mypack:moon run tp @s 8 80 8" [seconds]
//!
//! It prints the `minecraft:dimension_type` registry entries it was sent, the login packet's level keys and level,
//! then runs the command (a guest's chat command, as a stock client would send it) and prints the respawn it is
//! sent (the level and the type index) and what the chunks of that level look like (how many sections each holds). It never answers anything but keep-alives and the player-loaded packet; it exits non-zero when the
//! server disconnects it.
use horizon_client::net::{Event, Session, load_language};
use horizon_protocol::packets::{configuration::ClientInformation, play};
use serde_json::json;
use std::{path::PathBuf, sync::Arc, time::{Duration, Instant}};

/// How many sections a chunk's buffer holds: sections are read one after another until the buffer is used up. The
/// block palette's global bit width depends on the registry, so a few widths are tried; the one that reads the buffer
/// to its end exactly gives the count (`None` when none does).
fn sections_in(buffer: &[u8], biome_bits: u8) -> Option<usize> {
    use horizon_protocol::{buf::Reader, chunk::Section};
    for block_bits in 13..=16u8 {
        let mut r = Reader::new(buffer);
        let mut count = 0;
        while r.remaining() > 0 {
            if Section::read(&mut r, block_bits, biome_bits).is_err() {
                count = 0;
                break;
            }
            count += 1;
        }
        if count > 0 {
            return Some(count);
        }
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (address, name, resources, command) = (&args[1], &args[2], PathBuf::from(&args[3]), &args[4]);
    let seconds: u64 = args.get(5).and_then(|s| s.parse().ok()).unwrap_or(40);
    let lang = Arc::new(load_language(&resources));
    let session = Session::connect_lan(
        address,
        name,
        None,
        ClientInformation::new(4),
        lang,
        resources,
        false,
    );
    let say = |v: serde_json::Value| println!("{v}");
    let mut types: Vec<String> = Vec::new();
    let mut biome_bits = 0u8;
    let mut sent = false;
    let mut chunks_after = 0usize;
    let mut section_counts = std::collections::BTreeSet::<i64>::new();
    let mut respawn_at = None::<Instant>;
    let end = Instant::now() + Duration::from_secs(seconds);
    while Instant::now() < end {
        for event in session.poll() {
            match event {
                Event::Brand(b) => say(json!({"brand": b})),
                Event::Joined { registries, .. } => {
                    biome_bits = horizon_protocol::registry::ceil_log2(registries.len("minecraft:worldgen/biome"));
                    types = registries
                        .registries
                        .get("minecraft:dimension_type")
                        .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
                        .unwrap_or_default();
                    say(json!({"dimension_types": types}));
                }
                Event::Packet(packet) => match *packet {
                    play::Clientbound::Login(login) => {
                        say(json!({"login": {"levels": login.levels, "dimension": login.spawn.dimension,
                            "type": types.get(login.spawn.dimension_type as usize), "sea_level": login.spawn.sea_level}}));
                        session.send(play::Serverbound::PlayerLoaded(horizon_protocol::packets::Empty));
                    }
                    play::Clientbound::Respawn(r) => {
                        say(json!({"respawn": {"dimension": r.spawn.dimension,
                            "type": types.get(r.spawn.dimension_type as usize), "sea_level": r.spawn.sea_level}}));
                        session.send(play::Serverbound::PlayerLoaded(horizon_protocol::packets::Empty));
                        chunks_after = 0;
                        section_counts.clear();
                        respawn_at = Some(Instant::now());
                    }
                    play::Clientbound::LevelChunkWithLight(c) => {
                        if respawn_at.is_some() {
                            chunks_after += 1;
                            section_counts.insert(sections_in(&c.chunk.buffer, biome_bits).map_or(-1, |n| n as i64));
                        }
                    }
                    play::Clientbound::PlayerPosition(_) if !sent => {
                        sent = true;
                        session.send(play::Serverbound::ChatCommand(play::ChatCommand {
                            command: command.clone(),
                        }));
                        say(json!({"sent": command}));
                    }
                    _ => {}
                },
                Event::Disconnected { reason, .. } => {
                    say(json!({"disconnected": reason}));
                    std::process::exit(2);
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(20));
        if respawn_at.is_some_and(|t| t.elapsed() > Duration::from_secs(8)) {
            break;
        }
    }
    say(json!({"chunks_after_respawn": chunks_after, "sections": section_counts}));
}
