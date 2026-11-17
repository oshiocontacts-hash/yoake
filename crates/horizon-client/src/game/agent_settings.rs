//! Agent control as a setting (Horizon Settings > AI Agent): when the control socket is open, who
//! is connected, and the details an agent needs. The socket opens when the game was started for
//! an agent (`--control`, `--control-socket`: horizon-ctl launch, the launcher's MCP `play`) or
//! when the player turned Agent Control on, and never under No AI (`horizon_core::no_ai`).
//! Turning either switch in the game opens or closes it at once.
use super::Game;
use horizon_client::{Result, control};
use std::path::PathBuf;

impl Game<'_> {
    /// The endpoint: the one the command line gave, else the default horizon-ctl looks for.
    fn agent_socket_path(&self) -> PathBuf {
        self.options
            .control
            .clone()
            .unwrap_or_else(control::default_path)
    }

    /// At startup: open the socket if the game was started for an agent or Agent Control is on.
    pub(super) fn start_agent_control(&mut self) -> Result<()> {
        self.agent_control_on = self.settings.extras.agent_control;
        if self.options.benchmark.is_some() && self.options.control.is_none() {
            return Ok(());
        }
        if self.settings.extras.no_ai {
            if self.options.control.is_some() {
                log::warn!(
                    "Horizon: control socket not opened: {}",
                    horizon_core::no_ai::REFUSAL
                );
            }
            return Ok(());
        }
        if let Some(path) = &self.options.control {
            // Asked for on the command line: failing to listen is an error.
            let server = control::Server::start(path)
                .map_err(|e| format!("control socket {}: {e}", path.display()))?;
            log::info!("Horizon: control socket {}", server.path().display());
            self.control = Some(server);
        } else if self.agent_control_on {
            self.open_agent_socket();
        }
        Ok(())
    }

    /// The setting's socket: a failure (another game holds the default endpoint) is logged and
    /// shown on the AI Agent page, not fatal.
    fn open_agent_socket(&mut self) {
        let path = self.agent_socket_path();
        match control::Server::start(&path) {
            Ok(server) => {
                log::info!("Horizon: control socket {}", server.path().display());
                self.control = Some(server);
            }
            Err(e) => log::warn!("Horizon: control socket {}: {e}", path.display()),
        }
    }

    /// After a settings change: No AI closes the socket; Agent Control turned on or off opens or
    /// closes it.
    pub(super) fn sync_agent_control(&mut self) {
        let wanted = self.settings.extras.agent_control;
        let changed = wanted != self.agent_control_on;
        self.agent_control_on = wanted;
        let open = self
            .control
            .as_ref()
            .is_some_and(|c| !c.path().as_os_str().is_empty());
        if self.settings.extras.no_ai || (changed && !wanted) {
            if open {
                log::info!("Horizon: control socket closed");
                self.control = None;
            }
        } else if changed && wanted && self.control.is_none() {
            self.open_agent_socket();
        }
    }

    /// An agent's command arrived.
    pub(super) fn agent_command_seen(&mut self) {
        self.agent_last = Some(std::time::Instant::now());
    }

    /// The AI Agent page's lines: the state, then where agents connect.
    pub(super) fn agent_status_lines(&self) -> Vec<String> {
        let server = self
            .control
            .as_ref()
            .filter(|c| !c.path().as_os_str().is_empty())
            .map(|s| Socket {
                clients: s.clients(),
                name: s
                    .path()
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            });
        let mut lines = status_lines(
            server,
            self.agent_last.map(|t| t.elapsed().as_secs()),
            self.options.control.is_some() && !self.agent_control_on,
        );
        if let Some(line) = self.repro.agent_line(self.input_trace.events()) {
            lines.push(line);
        }
        lines
    }

    /// What Copy Connection Details puts on the clipboard: the socket, a horizon-ctl line to try,
    /// and an MCP client's config for this game.
    pub(super) fn agent_details(&self) -> String {
        let socket = self.agent_socket_path();
        let ctl = std::env::current_exe()
            .unwrap_or_default()
            .with_file_name(if cfg!(windows) {
                "horizon-ctl.exe"
            } else {
                "horizon-ctl"
            });
        let config = serde_json::json!({ "mcpServers": { "horizon-game": {
            "command": ctl, "args": ["--socket", socket, "mcp"] } } });
        format!(
            "Horizon agent control (this computer only)\nSocket: {}\nTry it: \"{}\" --socket \"{}\" state\nMCP config:\n{}\n",
            socket.display(),
            ctl.display(),
            socket.display(),
            serde_json::to_string_pretty(&config).unwrap_or_default()
        )
    }
}

/// The control socket as the AI Agent page tells of it.
struct Socket {
    /// Agents connected now.
    clients: usize,
    /// The socket's file name; the full path is too wide for the page (Copy Connection Details
    /// has it).
    name: String,
}

/// The AI Agent page's lines in the selected language: the state, then where agents connect.
/// `last_command` is the seconds since an agent's last command, `session_only` whether the
/// program that started the game opened the socket and the player's switch did not.
fn status_lines(
    server: Option<Socket>,
    last_command: Option<u64>,
    session_only: bool,
) -> Vec<String> {
    use horizon_client::lang::{get, tr};
    let mut lines = vec![match &server {
        Some(s) if s.clients > 0 => {
            let (one, many) = match last_command {
                Some(_) => ("one_ago", "many_ago"),
                None => ("one", "many"),
            };
            let key = format!(
                "horizon.agent.connected.{}",
                if s.clients == 1 { one } else { many }
            );
            let mut args: Vec<&dyn std::fmt::Display> = Vec::new();
            if s.clients != 1 {
                args.push(&s.clients);
            }
            if let Some(ago) = &last_command {
                args.push(ago);
            }
            tr(&key, &args)
        }
        Some(_) => get("horizon.agent.waiting"),
        None => get("horizon.agent.off"),
    }];
    if let Some(s) = &server {
        if session_only {
            lines.push(get("horizon.agent.session_opened"));
        }
        lines.push(tr("horizon.agent.socket", &[&s.name]));
    }
    lines.push(get("horizon.agent.local_only"));
    lines
}

#[cfg(test)]
mod tests {
    use super::{Socket, status_lines};
    use horizon_client::lang::{Language, scoped};

    fn lines(code: &str, clients: usize, ago: Option<u64>, session_only: bool) -> Vec<String> {
        let socket = Socket {
            clients,
            name: "ctl.sock".into(),
        };
        let (lines, missed) = scoped(Language::load(&[], code), || {
            status_lines(Some(socket), ago, session_only)
        });
        assert!(missed.is_empty(), "{code}: {missed:?}");
        lines
    }

    /// The AI Agent page translates its state in Japanese (#1927).
    #[test]
    fn agent_page_lines_follow_the_language() {
        assert_eq!(
            lines("ja_jp", 2, Some(5), false),
            [
                "\u{63a5}\u{7d9a}\u{4e2d}: \u{30a8}\u{30fc}\u{30b8}\u{30a7}\u{30f3}\u{30c8}2\u{4ef6}\u{3001}\u{6700}\u{5f8c}\u{306e}\u{64cd}\u{4f5c}\u{304b}\u{3089}5\u{79d2}",
                "\u{30bd}\u{30b1}\u{30c3}\u{30c8}: ctl.sock (\u{5b8c}\u{5168}\u{306a}\u{30d1}\u{30b9}\u{306f}\u{63a5}\u{7d9a}\u{60c5}\u{5831}\u{3092}\u{30b3}\u{30d4}\u{30fc}\u{3057}\u{3066}\u{304f}\u{3060}\u{3055}\u{3044})",
                "\u{30ed}\u{30fc}\u{30ab}\u{30eb}\u{9650}\u{5b9a}: \u{3053}\u{306e}\u{30b3}\u{30f3}\u{30d4}\u{30e5}\u{30fc}\u{30bf}\u{30fc}\u{4e0a}\u{306e}\u{30a8}\u{30fc}\u{30b8}\u{30a7}\u{30f3}\u{30c8}\u{306e}\u{307f}\u{63a5}\u{7d9a}\u{3067}\u{304d}\u{3001}\u{30cd}\u{30c3}\u{30c8}\u{30ef}\u{30fc}\u{30af}\u{306b}\u{306f}\u{516c}\u{958b}\u{3055}\u{308c}\u{307e}\u{305b}\u{3093}\u{3002}",
            ]
        );
        let (off, missed) =
            scoped(Language::load(&[], "ja_jp"), || status_lines(None, None, false));
        assert!(missed.is_empty(), "{missed:?}");
        assert_eq!(
            off,
            [
                "\u{30aa}\u{30d5}: \u{30a8}\u{30fc}\u{30b8}\u{30a7}\u{30f3}\u{30c8}\u{306f}\u{3053}\u{306e}\u{30b2}\u{30fc}\u{30e0}\u{306b}\u{63a5}\u{7d9a}\u{3067}\u{304d}\u{307e}\u{305b}\u{3093}\u{3002}",
                "\u{30ed}\u{30fc}\u{30ab}\u{30eb}\u{9650}\u{5b9a}: \u{3053}\u{306e}\u{30b3}\u{30f3}\u{30d4}\u{30e5}\u{30fc}\u{30bf}\u{30fc}\u{4e0a}\u{306e}\u{30a8}\u{30fc}\u{30b8}\u{30a7}\u{30f3}\u{30c8}\u{306e}\u{307f}\u{63a5}\u{7d9a}\u{3067}\u{304d}\u{3001}\u{30cd}\u{30c3}\u{30c8}\u{30ef}\u{30fc}\u{30af}\u{306b}\u{306f}\u{516c}\u{958b}\u{3055}\u{308c}\u{307e}\u{305b}\u{3093}\u{3002}",
            ]
        );
    }

    /// English keeps the sentences it always had: the count, its plural, the time since the last
    /// command, and the wait for a first agent.
    #[test]
    fn agent_page_lines_in_english() {
        assert_eq!(
            lines("en_us", 2, Some(5), false)[0],
            "Connected: 2 agent connections, last command 5 s ago"
        );
        assert_eq!(
            lines("en_us", 1, None, false)[0],
            "Connected: 1 agent connection"
        );
        assert_eq!(
            lines("en_us", 1, Some(0), false)[0],
            "Connected: 1 agent connection, last command 0 s ago"
        );
        assert_eq!(
            lines("en_us", 3, None, false)[0],
            "Connected: 3 agent connections"
        );
        assert_eq!(lines("en_us", 0, None, true)[0], "Waiting for an agent.");
    }
}
