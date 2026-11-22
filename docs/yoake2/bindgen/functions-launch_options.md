# `launch_options`: every function (horizon-client/src/game/launch_options.rs)

| function | line | receiver | category | reasons |
|---|---:|---|---|---|
| `launch_options::<Options as Clone>::clone` | 12 | &self | derived (not counted) | std type `std::path::PathBuf` has no codec yet [return value] |
| `launch_options::Options::parse` | 186 | - | excluded | `impl Trait` argument requires a listed instantiation; `impl Trait` requires a listed instantiation; generic type alias `Result` |
| `launch_options::Options::player_name` | 96 | &self | bound (a) | - |
| `launch_options::Options::read` | 110 | - | excluded | generic type alias `Result` |

## Fields

| field | status |
|---|---|
| `launch_options::Options.resources` | no accessor: std type `std::path::PathBuf` has no codec yet |
| `launch_options::Options.distance` | get + set |
| `launch_options::Options.distance_override` | get + set |
| `launch_options::Options.seed` | get + set |
| `launch_options::Options.creative` | get + set |
| `launch_options::Options.test_terrain` | get + set |
| `launch_options::Options.frames` | get + set |
| `launch_options::Options.paced_by_split` | get + set |
| `launch_options::Options.split_guest` | get + set |
| `launch_options::Options.split_in_main_window` | get + set |
| `launch_options::Options.world_frames` | get + set |
| `launch_options::Options.screenshot_after` | get + set |
| `launch_options::Options.timeout` | no accessor: std type `core::time::Duration` has no codec yet |
| `launch_options::Options.report` | no accessor: std type `std::path::PathBuf` has no codec yet |
| `launch_options::Options.world` | get + set |
| `launch_options::Options.look` | get + set |
| `launch_options::Options.time` | get + set |
| `launch_options::Options.weather` | get + set |
| `launch_options::Options.dimension` | get + set |
| `launch_options::Options.no_pause` | get + set |
| `launch_options::Options.control` | no accessor: std type `std::path::PathBuf` has no codec yet |
| `launch_options::Options.spikes` | get + set |
| `launch_options::Options.offscreen` | get + set |
| `launch_options::Options.no_focus` | get + set |
| `launch_options::Options.headless` | get + set |
| `launch_options::Options.window_size` | get + set |
| `launch_options::Options.server` | get + set |
| `launch_options::Options.player_name` | get + set |
| `launch_options::Options.dedicated` | get + set |
| `launch_options::Options.benchmark` | get + set |
| `launch_options::Options.dev_mods` | no accessor: std type `std::path::PathBuf` has no codec yet |
| `launch_options::Options.execution_role` | get + set |
| `launch_options::Options.fullscreen` | get + set |
| `launch_options::Options.game_dir` | no accessor: std type `std::path::PathBuf` has no codec yet |
| `launch_options::Options.quick_play` | get + set |
| `launch_options::Options.session_file` | get + set |
| `launch_options::Options.session_stdin` | get + set |
| `launch_options::Options.yoake_test` | no accessor: external type `yoake_test_runner::plan::WorkerArgs` (crate not loaded) |
| `launch_options::Options.social_handoff` | no accessor: std type `std::path::PathBuf` has no codec yet |
| `launch_options::Options.social_trust` | no accessor: std type `std::path::PathBuf` has no codec yet |
| `launch_options::Options.social_control_stdin` | get + set |
| `launch_options::Options.social_startup` | no accessor: std type `std::sync::poison::mutex::Mutex` has no codec yet |
