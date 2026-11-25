//! Launch-argument `Options` parsed from the command line (`--resources`, `--world`, ...).
//! Player persisted settings live in `horizon_client_render::settings::Settings` (`Game.settings`).

use super::{execution_role, launch_api, social_relay};
use horizon_client::Result;
use std::{
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

#[derive(Clone)]
pub(super) struct Options {
    pub(super) resources: PathBuf,
    pub(super) distance: i32,
    pub(super) distance_override: Option<i32>,
    pub(super) seed: i64,
    pub(super) creative: bool,
    pub(super) test_terrain: bool,
    pub(super) frames: Option<u64>,
    /// A splitscreen player: the split window paces every player's frames, not each its own.
    pub(super) paced_by_split: bool,
    /// A splitscreen player other than P1: its windows are the split's (P1 alone decides
    /// fullscreen; this game neither asks for it nor adopts it into its options).
    pub(super) split_guest: bool,
    /// A splitscreen guest drawn in Window 1, the game's own window: its F11 leaves that
    /// window to P1.
    pub(super) split_in_main_window: bool,
    /// `--world-frames N`: exit after N frames drawn in the world (smoke runs; the world may load slowly).
    pub(super) world_frames: Option<u64>,
    pub(super) screenshot_after: Option<u64>,
    pub(super) timeout: Duration,
    pub(super) report: Option<PathBuf>,
    pub(super) world: Option<String>,
    pub(super) look: Option<(f32, f32)>,
    pub(super) time: Option<i64>,
    pub(super) weather: Option<String>,
    pub(super) dimension: Option<String>,
    /// Options.pauseOnLostFocus off (F3+P), for scripted testing.
    pub(super) no_pause: bool,
    /// Control socket path (`--control`), for `horizon-ctl`.
    pub(super) control: Option<PathBuf>,
    /// `HORIZON_SPIKES=MS`: print the timed spans of frames slower than MS.
    pub(super) spikes: Option<f32>,
    /// Render into a texture instead of the window (`--offscreen`), so a
    /// GPU the display cannot present from can still be benchmarked. The
    /// window stays hidden and never grabs the system pointer.
    pub(super) offscreen: bool,
    /// `--no-focus` or `HORIZON_NO_FOCUS=1`: the window opens behind the others without taking
    /// the keyboard focus (an automated run recorded by a capture tool while someone works).
    /// It grabs the pointer only after the player clicks it to the front, as any unfocused window.
    pub(super) no_focus: bool,
    /// `--headless` (#4951): no display, no window and no renderer. The game runs its own
    /// loop (`game/headless.rs`) instead of winit's, so it starts where no display server
    /// is (an SSH session, a server). It counts as `offscreen` for everything else (an
    /// automated run that never shows a window or grabs the pointer).
    pub(super) headless: bool,
    /// `--width` and `--height` (vanilla's own arguments): the window's
    /// starting size, 1280x720 by default.
    pub(super) window_size: [u32; 2],
    /// Multiplayer: `--server HOST[:PORT]` joins at start; `--name` is the offline player name.
    pub(super) server: Option<String>,
    /// `--name`, `HORIZON_PLAYER_NAME` or the signed-in account's name; else
    /// [`Options::player_name`] uses the kept offline name.
    pub(super) player_name: Option<String>,
    /// `--dedicated`: run as a dedicated server; vanilla's server arguments
    /// (`--nogui`, `--port`, `--world`, `--universe`, `--initSettings`) follow.
    pub(super) dedicated: Option<Vec<String>>,
    /// `--benchmark SCENARIO`: run the benchmark, write its report and quit.
    pub(super) benchmark: Option<String>,
    /// `--dev-mod PATH` (repeatable): content mods always on.
    pub(super) dev_mods: Vec<PathBuf>,
    /// What this Game executes; independent of its current server connection.
    pub(super) execution_role: execution_role::ExecutionRole,
    /// `--fullscreen` (vanilla's): the window opens fullscreen whatever options.txt says,
    /// as vanilla's DisplayData does, without saving it there.
    pub(super) fullscreen: bool,
    /// `--game-dir DIR`: the folder `read` makes the working directory (saves, options.txt...).
    pub(super) game_dir: Option<PathBuf>,
    /// `--quickPlaySingleplayer`, checked once the game folder is known.
    pub(super) quick_play: Option<String>,
    pub(super) session_file: Option<String>,
    /// `--session-stdin`: the session file's JSON comes on standard input instead.
    pub(super) session_stdin: bool,
    /// `--yoake-test-run` and its companions: this process is a worker of `yoake test` (`yoake_test.rs`).
    pub(super) yoake_test: yoake_test_runner::WorkerArgs,
    pub(super) social_handoff: Option<PathBuf>,
    pub(super) social_trust: Option<PathBuf>,
    pub(super) social_control_stdin: bool,
    pub(super) social_startup: Arc<std::sync::Mutex<Option<std::result::Result<social_relay::Startup, ()>>>>,
}

impl Options {
    /// The name to join servers with: the one given, else the offline name kept
    /// next to options.txt (created on first use).
    pub(super) fn player_name(&self) -> String {
        self.player_name.clone().unwrap_or_else(|| {
            std::env::current_dir().map_or_else(
                |_| horizon_client::offline_name::generate(0),
                |_| {
                    horizon_client::offline_name::load_or_create(
                        &horizon_client::logging::player_dir(),
                    )
                },
            )
        })
    }
    /// The command line, then what it asks of the process: the game folder, the account.
    /// A bad command line is a [`launch_api::UsageError`] (exit code 2, no crash report).
    pub(super) fn read() -> Result<Option<Self>> {
        let Some(mut o) = Self::parse(std::env::args().skip(1))
            .map_err(|e| launch_api::UsageError(e.to_string()))?
        else {
            return Ok(None);
        };
        let social = social_relay::Startup::read(&mut o);
        let social_guest = social
            .as_ref()
            .is_some_and(|startup| startup.as_ref().is_ok_and(social_relay::Startup::is_guest));
        o.social_startup = Arc::new(std::sync::Mutex::new(social));
        if social_guest {
            o.world = None;
            o.server = None;
            o.quick_play = None;
        }
        // The launcher's account: online-mode servers need it. A bad file
        // only leaves the game offline.
        let session = if o.session_stdin {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| e.to_string())
                .and_then(|_| horizon_client::auth::AuthSession::parse(&text))
                .map(Some)
        } else {
            o.session_file
                .take()
                .filter(|p| !p.is_empty())
                .map(|path| horizon_client::auth::AuthSession::load(path.as_ref()))
                .transpose()
        };
        match session {
            Ok(Some(session)) => {
                o.player_name = Some(session.name.clone());
                horizon_client::auth::set_current(session);
                // The chat key pair, ready before the first join.
                horizon_client::auth::prepare_chat_keys(true);
            }
            Ok(None) => {}
            Err(e) => eprintln!("Horizon: not signed in: {e}"),
        }
        // A test worker's paths are made absolute before the working directory changes below.
        if o.yoake_test.is_active() {
            if !crate::yoake_test::is_worker() {
                return Err(launch_api::UsageError("--yoake-test-run is only for the workers of yoake test".into()).into());
            }
            o.yoake_test.module = o.yoake_test.module.map(std::path::absolute).transpose()?;
            o.yoake_test.result = o.yoake_test.result.map(std::path::absolute).transpose()?;
        }
        if let Some(dir) = o.game_dir.take() {
            // Paths given on the command line stay relative to where the game was started.
            o.resources = std::path::absolute(&o.resources)?;
            o.report = o.report.map(std::path::absolute).transpose()?;
            o.control = o.control.map(std::path::absolute).transpose()?;
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::env::set_current_dir(&dir))
                .map_err(|e| format!("game folder {}: {e}", dir.display()))?;
        }
        // Vanilla's quick play (Main, QuickPlay.joinSingleplayerWorld): only a
        // world that exists opens; else the title screen. --world also creates one.
        if let Some(world) = o.quick_play.take() {
            if std::env::current_dir()?
                .join("saves")
                .join(&world)
                .join("level.dat")
                .is_file()
            {
                o.world = Some(world);
            } else {
                eprintln!("Quick play: there is no world {world} in saves/");
            }
        }
        Ok(Some(o))
    }
    /// The command line alone: no file is read and nothing changes. `None` for `--help` and
    /// `--launch-api`, which print and end the run.
    pub(super) fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Self>> {
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or("missing executable directory")?
            .join("resources");
        let mut o = Self {
            resources: if sibling.join("horizon-resources.json").exists() {
                sibling
            } else {
                std::path::Path::new(".local/horizon-client").join(horizon_core::VANILLA_VERSION)
            },
            distance: 12,
            distance_override: None,
            seed: 0,
            creative: false,
            test_terrain: false,
            frames: None,
            paced_by_split: false,
            split_guest: false,
            split_in_main_window: false,
            world_frames: None,
            screenshot_after: None,
            timeout: Duration::from_secs(120),
            report: None,
            world: None,
            look: None,
            time: None,
            weather: None,
            dimension: None,
            no_pause: false,
            control: None,
            offscreen: false,
            no_focus: std::env::var_os("HORIZON_NO_FOCUS").is_some_and(|v| v == "1"),
            headless: false,
            window_size: [1280, 720],
            spikes: std::env::var("HORIZON_SPIKES")
                .ok()
                .and_then(|v| v.parse().ok()),
            server: None,
            player_name: std::env::var("HORIZON_PLAYER_NAME")
                .ok()
                .filter(|n| !n.is_empty()),
            dedicated: None,
            benchmark: None,
            dev_mods: Vec::new(),
            execution_role: execution_role::ExecutionRole::Client,
            fullscreen: false,
            game_dir: None,
            quick_play: None,
            session_file: std::env::var("HORIZON_SESSION_FILE").ok(),
            session_stdin: false,
            yoake_test: yoake_test_runner::WorkerArgs::default(),
            social_handoff: None,
            social_trust: None,
            social_control_stdin: false,
            social_startup: Arc::new(std::sync::Mutex::new(None)),
        };
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "Horizon — Minecraft Java Edition {} in Rust\n\n  --resources PATH        Prepared resources (horizon-prepare)\n  --render-distance N     Chunks, 2..128 (default 12)\n  --world NAME            World folder under saves/ (default \"New World\")\n  --seed N                World seed for new worlds (default 0)\n  --width N, --height N   Window size (default 1280 x 720)\n  --creative              Creative mode (default survival)\n  --test-terrain          Simple test terrain instead of vanilla generation\n  --frames N              Exit after N frames (testing)\n  --world-frames N        Exit after N frames drawn in the world (testing)\n  --smoke-report PATH     Write a JSON report on exit\n  --benchmark SCENARIO    Run a benchmark (standard, village, forest, ocean, mountain), write its report to benchmarks/ and quit\n  --control               Accept horizon-ctl commands on a local socket\n  --control-socket PATH   Control socket path (default $XDG_RUNTIME_DIR/horizon-control.sock)\n  --server HOST[:PORT]    Join a multiplayer server (offline mode) at start\n  --quickPlaySingleplayer WORLD  Open this world of saves/ at start, only if it exists (vanilla's)\n  --quickPlayMultiplayer HOST[:PORT]  The same as --server (vanilla's name)\n  --name NAME             Player name for multiplayer (default: generated once, kept in horizon-player.txt)\n  --session-file PATH     Signed-in account from the launcher (read once, then deleted)\n  --session-stdin         The same account as JSON on standard input\n  --game-dir DIR          Game folder: saves, options.txt, packs, mods (default: the working directory)\n  --home DIR              Logs and crash reports (default $HORIZON_HOME, else the platform data folder)\n  --player-dir DIR        This player's own files when several play in one game folder: options.txt, logs, horizon-player.txt, servers.dat, screenshots\n  --fullscreen            Open the window fullscreen\n  --launch-api            Print the Launch API version and options as JSON (docs/api/launch.md)\n  --dev-mod PATH          A content mod (.hmod or folder) to develop: always on\n  --pregen                Add Horizon's /pregen command (docs/pregeneration.md)\n\nControls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,\nleft click break, right click place/use, 1-9/scroll hotbar, E inventory,\nQ drop, F3 debug, Esc pause.",
                        horizon_core::VANILLA_VERSION
                    );
                    return Ok(None);
                }
                "--launch-api" => {
                    println!("{}", launch_api::reply(&horizon_client::crash::version()));
                    return Ok(None);
                }
                "--fullscreen" => o.fullscreen = true,
                "--session-stdin" => o.session_stdin = true,
                "--yoake-social-control-stdin" => o.social_control_stdin = true,
                "--dedicated" => {
                    o.execution_role = execution_role::ExecutionRole::DedicatedServer;
                    o.dedicated = Some(args.by_ref().collect());
                    break;
                }
                "--creative" => o.creative = true,
                "--test-terrain" => o.test_terrain = true,
                "--no-pause" => o.no_pause = true,
                "--offscreen" => o.offscreen = true,
                "--no-focus" => o.no_focus = true,
                // MX3JN1: read from the process arguments (`join_mods`); accepted here so the game starts.
                "--join-mod-sync" | "--full-access-dev" => {}
                "--headless" => {
                    o.headless = true;
                    o.offscreen = true;
                }
                "--control" => o.control = Some(horizon_client::control::default_path()),
                // Single player always runs on its server thread: a launcher's saved arguments
                // may still name the old switches.
                "--server-thread" => {}
                "--in-frame" | "--no-server-thread" => eprintln!(
                    "Horizon: {arg} was removed: single player runs on its server thread."
                ),
                // Horizon's own commands (/pregen), off by default as on vanilla.
                "--pregen" => horizon_client::command::vanilla::set_horizon_commands(true),
                _ => {
                    let value = args
                        .next()
                        .ok_or_else(|| format!("missing value after {arg}"))?;
                    // A launcher shows the error to the player: it names the option.
                    let read = || -> Result<()> {
                        match arg.as_str() {
                            "--resources" => o.resources = value.into(),
                            "--render-distance" | "--radius" => {
                                o.distance = value.parse()?;
                                o.distance_override = Some(o.distance);
                                let max = horizon_client::options_file::MAX_RENDER_DISTANCE;
                                if !(2..=max).contains(&o.distance) {
                                    return Err(format!("render distance must be 2..{max}").into());
                                }
                            }
                            "--seed" => {
                                o.seed = value.parse().unwrap_or_else(|_| {
                                    // Text seeds hash like String.hashCode.
                                    value
                                        .encode_utf16()
                                        .fold(0i32, |h, c| {
                                            h.wrapping_mul(31).wrapping_add(i32::from(c))
                                        })
                                        .into()
                                })
                            }
                            "--frames" => o.frames = Some(value.parse()?),
                            "--world-frames" => {
                                o.world_frames = Some(value.parse()?);
                                // A frame-limited run in every other respect; --timeout-seconds still applies.
                                o.frames.get_or_insert(u64::MAX);
                            }
                            "--timeout-seconds" => o.timeout = Duration::from_secs(value.parse()?),
                            "--smoke-report" => o.report = Some(value.into()),
                            "--screenshot-after" => o.screenshot_after = Some(value.parse()?),
                            "--width" => o.window_size[0] = value.parse::<u32>()?.max(1),
                            "--height" => o.window_size[1] = value.parse::<u32>()?.max(1),
                            // Vanilla opens only a level id that is one folder of saves/. --world, for
                            // harnesses, also takes a path (tools/bundle_tooltip_e2e.py).
                            "--world" => o.world = Some(value),
                            "--quickPlaySingleplayer" => {
                                let mut parts = std::path::Path::new(&value).components();
                                if !matches!(
                                    (parts.next(), parts.next()),
                                    (Some(std::path::Component::Normal(_)), None)
                                ) {
                                    return Err("not a folder name of saves/".into());
                                }
                                o.quick_play = Some(value);
                            }
                            "--game-dir" | "--gameDir" => o.game_dir = Some(value.into()),
                            // MX3JN1 / MX3L4: the launcher's mod arguments, read from the process arguments by
                            // `join_mods` and `yoake2_mods`, not through `Options`.
                            "--remote-mods" | "--full-access-approved" => {}
                            // main has made it the game home already (logging::set_game_home).
                            "--home" => {}
                            // ...and this player's own folder (logging::set_player_dir).
                            "--player-dir" => {}
                            "--dev-mod" => o.dev_mods.push(std::path::absolute(&value)?),
                            "--benchmark" => {
                                if horizon_client::benchmark::scenes(&value).is_none() {
                                    return Err(format!(
                                    "unknown benchmark {value}: standard, village, forest, ocean or mountain"
                                )
                                .into());
                                }
                                o.benchmark = Some(value);
                            }
                            "--server" | "--quickPlayMultiplayer" => o.server = Some(value),
                            "--name" => o.player_name = Some(value),
                            "--session-file" => o.session_file = Some(value),
                            "--yoake-social-handoff" => o.social_handoff = Some(value.into()),
                            "--yoake-social-trust" => o.social_trust = Some(value.into()),
                            "--control-socket" => o.control = Some(value.into()),
                            flag if yoake_test_runner::WorkerArgs::is_flag(flag) => {
                                o.yoake_test.set(flag, value)?;
                            }
                            "--time" => o.time = Some(value.parse()?),
                            "--weather" => o.weather = Some(value),
                            "--dimension" => {
                                o.dimension = Some(if value.contains(':') {
                                    value
                                } else {
                                    format!("minecraft:{value}")
                                })
                            }
                            "--look" => {
                                let (y, p) =
                                    value.split_once(',').ok_or("--look takes YAW,PITCH")?;
                                o.look = Some((y.parse()?, p.parse()?));
                            }
                            _ => return Err("unknown option".into()),
                        }
                        Ok(())
                    };
                    read().map_err(|e| format!("{arg}: {e}"))?;
                }
            }
        }
        // Test mode is a single-player worker: never a dedicated server (which also takes the rest of the line).
        o.yoake_test.validate()?;
        if o.yoake_test.is_active() && o.dedicated.is_some() {
            return Err("--yoake-test-run cannot be used with --dedicated".into());
        }
        if o.session_stdin && o.social_control_stdin {
            return Err(
                "--session-stdin and --yoake-social-control-stdin cannot share stdin".into(),
            );
        }
        Ok(Some(o))
    }
}

// REM-04: binding glue generated by yoake-bindgen, compiled only with the feature.
#[cfg(feature = "yoake2-slice")]
include!("../yoake_glue/launch_options.rs");

#[cfg(all(test, feature = "yoake2-slice"))]
mod yoake_options_tests {
    use super::*;
    use horizon_client_render::world::{self, World};
    use horizon_world::registry::BlockStateId;
    use std::path::Path;
    use yoake_slice::activation::{self, Accessor, ArgSet, PathRoot, PathRootRef, Status};
    use yoake_slice::codec::CallError;
    use yoake_slice::runtime::{self, ModConfig};

    #[path = "../../../../../yoake-guest-sdk/tests/support/build.rs"]
    mod guest_build;

    struct GameRoot {
        world: World,
        options: Options,
    }

    impl PathRoot for GameRoot {
        fn resolve(&mut self, path: &[Accessor], f: &mut dyn FnMut(&mut dyn std::any::Any)) -> std::result::Result<(), Status> {
            match path {
                [Accessor::Field(9)] => {
                    f(&mut self.world);
                    Ok(())
                }
                [Accessor::Field(11)] => {
                    f(&mut self.options);
                    Ok(())
                }
                _ => Err(Status::BadField),
            }
        }
    }

    impl PathRootRef for GameRoot {
        fn resolve_ref(&self, path: &[Accessor], f: &mut dyn FnMut(&dyn std::any::Any)) -> std::result::Result<(), Status> {
            match path {
                [Accessor::Field(9)] => {
                    f(&self.world);
                    Ok(())
                }
                [Accessor::Field(11)] => {
                    f(&self.options);
                    Ok(())
                }
                _ => Err(Status::BadField),
            }
        }
    }

    fn dispatch(handle: u64, id: u32, args: &[u8], out: &mut Vec<u8>) -> std::result::Result<(), CallError> {
        let modules = [
            super::super::native_client_items::yoake_glue::call,
            yoake_glue::call,
            world::yoake_glue::call,
        ];
        for call in modules {
            match call(handle, id, args, out) {
                Err(CallError::UnknownBinding(_)) => {}
                other => return other,
            }
        }
        Err(CallError::UnknownBinding(id))
    }

    fn id_of(bindings: &[(u32, &str)], key: &str) -> u32 {
        bindings.iter().find(|(_, k)| *k == key).unwrap_or_else(|| panic!("no binding {key}")).0
    }

    fn slot(reports: &[(i32, i64)], n: i32) -> Vec<i64> {
        reports.iter().filter(|(s, _)| *s == n).map(|(_, v)| *v).collect()
    }

    #[test]
    fn guest_reads_game_options_and_w10_world_fields_and_setters_are_read_only() {
        let distance_set = id_of(yoake_glue::BINDINGS, "launch_options::Options.distance#set");
        let day_time_set = id_of(world::yoake_glue::BINDINGS, "render_world::World.day_time#set");

        let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../yoake-guest-sdk");
        let scratch = std::env::temp_dir().join("yoake2_launch_options_guest");
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).unwrap();
        let guest_src = scratch.join("guest.rs");
        std::fs::write(
            &guest_src,
            format!(
                r#"#![no_std]
extern crate alloc;
use yoake_guest_sdk::stubs::game_items::{{game_items_game_options_handle, game_items_game_world_handle}};
use yoake_guest_sdk::stubs::launch_options::{{
    launch_options_options_creative_get, launch_options_options_distance_get,
    launch_options_options_execution_role_get, launch_options_options_seed_get,
    types::ExecutionRole,
}};
use yoake_guest_sdk::stubs::render_world::render_world_world_day_time_get;
use yoake_guest_sdk::{{call, encode, hooks, original, report, status, Error, Handle, Registrar, export_guest}};

fn init(r: &mut Registrar) {{
    r.hook(hooks::SERVER_TICK, 0).unwrap();
    r.hook(hooks::CLIENT_TICK, 0).unwrap();
}}

fn hook(_id: u32, root: Handle) -> i64 {{
    if let Ok(opt) = game_items_game_options_handle(root) {{
        if let Ok(dist) = launch_options_options_distance_get(opt) {{
            report(1, i64::from(dist));
        }}
        if let Ok(creative) = launch_options_options_creative_get(opt) {{
            report(2, i64::from(creative));
        }}
        if let Ok(seed) = launch_options_options_seed_get(opt) {{
            report(3, seed);
        }}
        if let Ok(role) = launch_options_options_execution_role_get(opt) {{
            report(4, match role {{
                ExecutionRole::Client => 0,
                ExecutionRole::IntegratedServer => 1,
                ExecutionRole::DedicatedServer => 2,
            }});
        }}
        if let Err(Error::Status(s)) = call(opt, {distance_set}, &encode(&8i32)) {{
            report(5, i64::from(s));
        }}
    }}
    if let Ok(w) = game_items_game_world_handle(root) {{
        if let Ok(day) = render_world_world_day_time_get(w) {{
            report(6, day);
        }}
        if let Err(Error::Status(s)) = call(w, {day_time_set}, &encode(&12000i64)) {{
            report(7, i64::from(s));
        }}
    }}
    let _ = original(root);
    0
}}

export_guest!(init, hook);
"#
            ),
        )
        .unwrap();
        let rlib = guest_build::build_sdk(&sdk, &scratch);
        let wasm = guest_build::build_guest(&rlib, &guest_src, &scratch, "options_guest");

        let _ = runtime::uninstall();
        runtime::set_thread_engine_dispatch(Some(dispatch)); // per thread: parallel guest tests must not swap each other's dispatcher
        runtime::install(&wasm, ModConfig::new("")).unwrap();

        let mut options = Options::parse(["--render-distance", "16", "--creative", "--seed", "42"].into_iter().map(str::to_owned))
            .unwrap()
            .unwrap();
        options.execution_role = execution_role::ExecutionRole::IntegratedServer;
        let mut world = World::new(BlockStateId(0), 42);
        world.day_time = 6000;
        let mut root = GameRoot { world, options };

        // 1. Mutable Game root (`SERVER_TICK`): guest reads Game.options and Game.world, setters refused ReadOnly (-4).
        runtime::dispatch_root_id(runtime::HOOK_SERVER_TICK, &mut root, &mut |_| 0);
        let r = runtime::take_reports().unwrap();
        assert_eq!(slot(&r, 1), [16]);
        assert_eq!(slot(&r, 2), [1]);
        assert_eq!(slot(&r, 3), [42]);
        assert_eq!(slot(&r, 4), [1]);
        assert_eq!(slot(&r, 5), [i64::from(yoake_slice::runtime::call_status::READ_ONLY)]);
        assert_eq!(slot(&r, 6), [6000]);
        assert_eq!(slot(&r, 7), [i64::from(yoake_slice::runtime::call_status::READ_ONLY)]);
        assert_eq!(root.options.distance, 16);
        assert_eq!(root.world.day_time, 6000);

        // 2. Shared `&Game` root (`CLIENT_TICK`): read-only path resolution also reaches Game.options and Game.world.
        runtime::dispatch_n_id(runtime::HOOK_CLIENT_TICK, ArgSet::new().root_shared(&root), &mut |_| 0);
        let r = runtime::take_reports().unwrap();
        assert_eq!(slot(&r, 1), [16]);
        assert_eq!(slot(&r, 6), [6000]);

        // 3. Generated `FieldTable` / `RefFieldTable` on Options directly.
        activation::scope(&mut root.options, &yoake_glue::FIELDS_crate__game__launch_options__Options, &mut |_| 0, |h| {
            assert_eq!(activation::get(h, 1), Ok(16));
            assert_eq!(activation::get(h, 3), Ok(42));
            assert_eq!(activation::get(h, 4), Ok(1));
            // REM-11: the table has setters now (access-transformer parity); `distance` is plain data
            assert_eq!(activation::set(h, 1, 8), Ok(()));
            assert_eq!(activation::get(h, 1), Ok(8));
            assert_eq!(activation::set(h, 2, 8), Err(Status::BadField), "index 2 is not a scalar field");
        });
        assert_eq!(root.options.distance, 8);

        let stats = runtime::stats();
        assert!(!stats.disabled, "{:?}", stats.trap);
        assert_eq!(stats.entries, 2);
        assert_eq!(activation::live_activations(), 0);
        assert_eq!(activation::live_path_handles(), 0);
        runtime::set_thread_engine_dispatch(None);
        let _ = runtime::uninstall();
    }
}
