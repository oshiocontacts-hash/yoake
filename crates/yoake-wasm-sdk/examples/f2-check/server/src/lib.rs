//! FR-REM05, main instance: the file checks of the F2 freedoms. It writes a file in its own data folder
//! (`/data`, which is `mod-data/f2-check/` in the game folder) and reads it back, and writes one in the game
//! folder (`/game`). Results go to the game log as `f2-check files ...`. The network part is in the worker.
use yoake_wasm_sdk::full::server::ServerMod;

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
    fn init(&mut self) {
        let data = check("/data/x.txt", "hello-data");
        let game = check("/game/f2-check-game.txt", "hello-game");
        println!("f2-check files data={data} game={game}");
    }
}

yoake_wasm_sdk::export_server_full!(Guest);
