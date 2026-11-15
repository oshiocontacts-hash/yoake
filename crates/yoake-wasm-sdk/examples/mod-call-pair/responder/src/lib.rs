//! The responding half of the mod-call pair. It exports no commands.

use yoake_wasm_sdk::export_server_full;
use yoake_wasm_sdk::full::server::ServerMod;

#[derive(Default)]
pub struct ResponderMod;

impl ServerMod for ResponderMod {
    fn handle_call(
        &mut self,
        caller: String,
        method: String,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        if method != "echo" {
            return Err(format!("unknown method: {method}"));
        }

        // Never assume that another mod's bytes are valid text. Returning Err
        // here makes the caller's services::mod_call return Err as well.
        let text = String::from_utf8(payload)
            .map_err(|_| "echo payload must be valid UTF-8".to_owned())?;
        Ok(format!("pong from {caller}: {text}").into_bytes())
    }
}

export_server_full!(ResponderMod);
