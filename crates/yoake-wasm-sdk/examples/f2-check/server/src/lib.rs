//! FR-REM05, main instance: the file checks of the F2 freedoms, and the jobs of the network part. It writes a file in
//! its own data folder (`/data`, which is `mod-data/f2-check/` in the game folder) and reads it back, and writes one in
//! the game folder (`/game`). Results go to the game log as `f2-check files ...`.
//!
//! It sends the three network jobs to its worker itself (`worker::send`, never blocking) and logs each answer the
//! worker sends back as `f2-check job <answer>`, polled from the server tick. The endpoints are the ones
//! `tools/f2_e2e.py` starts on 127.0.0.1.
use yoake_wasm_sdk::full::hook;
use yoake_wasm_sdk::full::server::{ServerMod, worker};

/// The HTTP server of the e2e (`/echo` and `/slow`).
const HTTP: &str = "http://127.0.0.1:17631";
/// The TCP echo server of the e2e.
const TCP: &str = "127.0.0.1:17632";

#[derive(Default)]
struct Guest;

fn check(path: &str, text: &str) -> String {
    if let Err(error) = std::fs::write(path, text) {
        return format!("error:write:{error}");
    }
    match std::fs::read_to_string(path) {
        Ok(back) if back == text => "ok".to_owned(),
        Ok(back) => format!("error:mismatch:{back}"),
        Err(error) => format!("error:read:{error}"),
    }
}

impl ServerMod for Guest {
    fn subscriptions() -> &'static [&'static str] {
        &[hook::TICK]
    }

    fn init(&mut self) {
        let data = check("/data/x.txt", "hello-data");
        let game = check("/game/f2-check-game.txt", "hello-game");
        println!("f2-check files data={data} game={game}");
        let jobs = [
            format!("1 post {HTTP}/echo hello-f2"),
            format!("2 tcp {TCP} hello-tcp"),
            format!("3 post {HTTP}/slow hello-slow"),
        ];
        for job in jobs {
            if let Err(error) = worker::send(job.into_bytes()) {
                println!("f2-check send error: {error}");
            }
        }
    }

    fn tick(&mut self) {
        while let Some(reply) = worker::recv() {
            println!("f2-check job {}", String::from_utf8_lossy(&reply));
        }
    }
}

yoake_wasm_sdk::export_server_full!(Guest);
