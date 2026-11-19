//! Horizon launcher: installs Horizon builds, prepares them from the player's own Minecraft,
//! and starts the client or a dedicated server.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod agents_page;

#[cfg(test)]
#[path = "../../tests/support/temp_dir.rs"]
mod test_support;
mod art;
mod blocks;
mod cli;
mod cli_tools;
mod collection;
mod console;
mod console_page;
mod cull;
mod datapacks_page;
mod dev_options;
mod export_page;
mod frame;
mod game_tools;
mod github_mods_page;
mod friends_page;
mod friends_view;
mod glass;
mod glass_home;
mod glass_home_lists;
mod glass_home_mods;
mod glass_home_packs;
mod glass_home_tabs;
mod glass_instance;
mod glass_instance_java;
mod glass_ui;
mod home;
mod home_tabs;
mod import_page;
mod instance_icons;
mod instance_mods;
mod instances_page;
mod internet;
mod language_page;
mod launch_page;
mod line_edit;
mod markdown;
mod mcp;
mod microsoft;
mod modpacks_page;
mod mods_page;
mod mods_project;
mod multiplayer_page;
mod native_log_page;
mod new_instance;
mod owner_page;
mod packs_page;
mod partner;
mod perf;
mod plate;
mod profile_heads;
mod scene;
mod screenshot_mode;
mod screenshots_page;
mod scrollbar;
mod servers_page;
#[cfg(windows)]
mod setup_page;
mod shell;
mod shortcut_page;
mod skin_page;
mod sky;
mod social;
mod social_mock;
mod specimen;
mod switch_page;
mod theme;
mod themes_page;
mod titlebar;
mod wardrobe_page;
mod while_playing;
mod worlds_page;
pub use horizon_launcher::{yoake_auth, yoake_feed, yoake_install, yoake_update};
mod yoake_configs_page;
mod yoake_launcher_update;
pub mod yoake_login;
mod yoake_mods_page;
mod yoake_page;
mod yoake_self_update;

use gpui::{
    actions, div, ease_out_quint, prelude::*, px, relative, size, Animation, AnimationExt,
    AnyElement, App, Bounds, ClickEvent, Context, Div, ElementId, FocusHandle, FontWeight, Hsla,
    KeyBinding, KeyDownEvent, Menu, MenuItem, MouseDownEvent, PathPromptOptions, Pixels, Point,
    SharedString, Stateful, Subscription, Window, WindowBounds, WindowOptions,
};
use horizon_launcher::account::{self, Accounts, SignIn, SignInState};
use horizon_launcher::backend::{self, Job, Msg, Release, Running, Settings, Updates};
use horizon_launcher::build_import;
use horizon_launcher::i18n::{self, tr, trf};
use horizon_launcher::instances;
use horizon_launcher::native_mods;
use horizon_launcher::native_status;
use horizon_launcher::telemetry;
use scrollbar::Scrollbar;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use theme::Theme;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    Play,
    Versions,
    /// Mods, resource packs and modpacks (imported ones and the browser's).
    Mods,
    Servers,
    /// What the player wears (`wardrobe_page.rs`), with the cosmetics flag on.
    Wardrobe,
    Notes,
    /// Friends and groups from the owner's portal (`friends_page.rs`), in Yoake builds.
    Friends,
    /// The group's modpacks and the portal sign-in (`yoake_page.rs`), in Yoake builds.
    Yoake,
    Settings,
}

impl Page {
    /// `HORIZON_LAUNCHER_PAGE=mods` (or `modpacks`, `resourcepacks`, `shaders`, `servers`, `versions`, `notes`,
    /// `settings`) opens on that page, for screenshots; Home otherwise (`getting-ready`: Home
    /// preparing, `home::getting_ready_preview`; `sign-in`: the owner gate's sheet with a
    /// made-up code, `owner_page::sign_in_preview`; `new-instance`: Home with the New instance
    /// dialog open on Modded; `edit-instance`: the selected one's Edit dialog; `worlds`, `packs`...: Home on that tab, `HomeTab::from_env`).
    fn from_env() -> Page {
        match std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() {
            Ok("mods" | "modpacks" | "resourcepacks" | "shaders" | "datapacks") => Page::Mods,
            Ok("servers") => Page::Servers,
            Ok("wardrobe") if horizon_cosmetics::enabled() => Page::Wardrobe,
            Ok("versions") => Page::Versions,
            Ok("notes") => Page::Notes,
            Ok("yoake") => Page::Yoake,
            Ok("friends" | "rooms") => Page::Friends,
            Ok("settings") => Page::Settings,
            _ => Page::Play,
        }
    }

    /// Every page, with its name and icon: the top bar shows all but Settings, which has its
    /// own button.
    const ALL: [(Page, &'static str, &'static str); 9] = [
        (Page::Play, "Home", "home"),
        (Page::Yoake, "Yoake", "user-multiple-02"),
        (Page::Friends, "Friends", "user"),
        (Page::Mods, "Mods", "package"),
        (Page::Servers, "Servers", "server"),
        (Page::Wardrobe, "Wardrobe", "user"),
        (Page::Versions, "Versions", "archive"),
        (Page::Notes, "Patch notes", "notes"),
        (Page::Settings, "Settings", "sliders"),
    ];

    /// The pages the top bar shows: all but Settings, and the Wardrobe only with cosmetics on.
    fn in_top_bar() -> impl Iterator<Item = (Page, &'static str, &'static str)> {
        Self::ALL.into_iter().filter(|(page, ..)| match page {
            Page::Settings => false,
            Page::Wardrobe => horizon_cosmetics::enabled(),
            Page::Yoake | Page::Friends => yoake_page::enabled(),
            _ => true,
        })
    }
}

/// The launcher's own update, as the footer and Settings show it.
#[derive(Clone, PartialEq)]
enum SelfUpdate {
    Idle,
    Downloading(String, f32),
    Ready(String),
    Failed(String),
}

/// The last game the launcher started ended badly: what it left behind.
#[derive(Clone)]
struct Crash {
    /// The crash report the run wrote, if it wrote one.
    report: Option<std::path::PathBuf>,
    /// One line on what went wrong: the report's error, else how the process ended.
    summary: String,
    copied: bool,
}

/// The opening: the wordmark holds at least `OPENING_HOLD` (until the release
/// list is in, at most `OPENING_MAX`), then the launcher appears over `OPENING_REVEAL`.
const OPENING_HOLD: Duration = horizon_theme::OPENING_HOLD;
const OPENING_MAX: Duration = horizon_theme::OPENING_MAX;
const OPENING_REVEAL: Duration = horizon_theme::OPENING_REVEAL;

/// The phase of the opening at one moment: how far the reveal has gone, 0 to 1.
struct Opening {
    /// Set on the first frame, so a slow start does not eat into the hold.
    started: Option<Instant>,
    reveal: Option<Instant>,
    focus: FocusHandle,
}

fn ease_out_cubic(x: f32) -> f32 {
    horizon_theme::ease_out_cubic(x)
}

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

const LOG_LINES: usize = 2000;

struct Launcher {
    theme: Theme,
    yoake_login_task: Option<yoake_login::LoginTask>,
    yoake_login_event: Option<yoake_login::LoginEvent>,
    yoake_user_code: Option<String>,
    yoake_update_task: Option<yoake_update::UpdateTask>,
    yoake_update_states: HashMap<std::path::PathBuf, yoake_update::UpdateState>,
    yoake: yoake_page::YoakePage,
    page: Page,
    /// How many times the page changed, for the page transition.
    switches: usize,
    content_scroll: Scrollbar,
    /// The rows of the page's long list (Versions, Patch notes) last drawn.
    long_list: cull::Cull,
    opening: Option<Opening>,
    updates: Updates,
    checked: Option<Instant>,
    update_error: Option<String>,
    self_update: SelfUpdate,
    /// The Horizon version being installed as an update, and whether it becomes the selected one.
    update_install: Option<(String, bool)>,
    /// The install bar's previous and current fraction, so it moves rather than jumps.
    bar: (f32, f32),
    releases: Load<Vec<Release>>,
    installed: Vec<String>,
    installing: Option<(String, f32)>,
    /// The versions being deleted on background threads. Each is already out of `installed`, so
    /// nothing can launch, select or prepare it, and its row can't install or remove it again.
    /// More than one may run at once: a row's Remove stays usable while another version goes.
    removing: Vec<String>,
    /// What each installed version takes on disk, measured on a thread for the Versions page, and
    /// the installed list it was measured for.
    disk: Vec<(String, u64)>,
    disk_for: Vec<String>,
    install_error: Option<(String, String)>,
    /// Import a game build (`build_import`, on the Versions page): whether one is being copied in,
    /// and why the last one failed.
    importing_build: bool,
    import_error: Option<String>,
    /// Native mods (`native_mods`, on the Mods page): the build's step while it runs, why it
    /// failed, and the warning before a build while it is open, with the mods it names.
    native_step: Option<String>,
    /// When the running native build started, and the whole seconds last drawn (the status shows
    /// the time it has taken).
    native_started: Option<(Instant, u64)>,
    /// How the last native build ended ("Built in 42 s (fast, incremental)", or "Up to date").
    native_done: Option<String>,
    native_error: Option<String>,
    native_confirm: Option<Result<Vec<native_mods::Bundle>, String>>,
    /// The instance (id) the build under way, or the last one's error, is for.
    native_for: Option<String>,
    /// The newer Horizon the open warning's build moves to (Update), else None: a build stays
    /// on the Horizon the instance plays.
    native_update: Option<String>,
    /// What each installed native build was made from, by version. A build's version names its
    /// contents, so an entry never goes stale.
    native_builds:
        std::cell::RefCell<std::collections::HashMap<String, Option<native_status::Build>>>,
    /// The mods of a build made for other Horizon versions (`native_mods::MISMATCH`): the build
    /// waits for "Build anyway".
    native_mismatch: Option<String>,
    /// The top bar's Build log panel (`native_log_page.rs`).
    native_log: native_log_page::NativeLog,
    confirm_remove: Option<String>,
    picker_open: bool,
    /// Where the accent picker's hue strip is on screen (`themes_page.rs`), to map a click to a
    /// hue.
    hue_area: std::rc::Rc<std::cell::Cell<Option<Bounds<gpui::Pixels>>>>,
    /// A drag on that strip is in flight, so a release anywhere writes the accent down.
    hue_dragging: bool,
    /// Settings > Language's dropdown (`language_page.rs`).
    language_menu: bool,
    /// The top bar's quick switch of who plays (`switch_page.rs`).
    account_menu: bool,
    /// The glass sidebar's ⋮ menu (`frame.rs`).
    row_menu: frame::RowMenu,
    /// The glass look's colours for the player's accent (`horizon_theme::glass`).
    look: horizon_theme::glass::Glass,
    /// Where the mouse went down to close a menu by pressing outside it, so that the
    /// same press, ending on the menu's own button, doesn't open it again.
    menu_closed_at: Option<Point<Pixels>>,
    /// Settings > Account's skin panel for an offline name (`skin_page.rs`).
    skin: Option<skin_page::SkinPanel>,
    /// The Wardrobe page's account and outfit (`wardrobe_page.rs`).
    wardrobe: Option<wardrobe_page::Wardrobe>,
    /// Ask the service again on the next draw of the Wardrobe (set by each visit).
    wardrobe_reload: bool,
    /// Work the look out again on the next frame (a launcher theme put on or taken off).
    restyle: bool,
    /// Why the last picture for Home's background was refused.
    background_error: Option<String>,
    settings: Settings,
    /// The last failed settings write, kept across page changes until a save succeeds.
    settings_save_error: Option<String>,
    /// The Discord, sponsor and website links: built in until links.json on main has been read.
    links: backend::Links,
    preparing: bool,
    prepare_log: VecDeque<String>,
    prepare_error: Option<String>,
    client: Option<Running>,
    client_error: Option<String>,
    /// When the running client started, to tell its crash report from older ones.
    client_started: Option<std::time::SystemTime>,
    /// The folder the running game keeps its settings in: the game home, or a modpack's.
    client_home: Option<std::path::PathBuf>,
    crash: Option<Crash>,
    /// The Servers page (`servers_page.rs`).
    servers: servers_page::Servers,
    /// Its Friends over the internet panel (`internet.rs`).
    internet: internet::Internet,
    /// The window's own focus, so Tab reaches the controls when nothing else has the keyboard.
    root_focus: FocusHandle,
    /// Microsoft accounts (accounts.json), the sign-in under way and what it last showed.
    accounts: Accounts,
    auth_status: HashMap<String, account::session::Status>,
    auth_progress: HashMap<String, account::session::Phase>,
    profile_heads: profile_heads::Heads,
    /// When page switches last checked every account's sign-in.
    auth_checked: Option<Instant>,
    sign_in_target: Option<account::Account>,
    auth_busy: HashMap<String, u64>,
    auth_next: u64,
    sign_in: Option<SignIn>,
    sign_in_state: Option<SignInState>,
    /// What to say when the keychain doesn't keep sign-ins ([`account::no_keychain`]).
    keychain: Option<String>,
    /// Whether an account here owns the game, by the keychain's last answers (the owner gate);
    /// none until the keychain has been read.
    owner: Option<bool>,
    /// Play was refused (or Home's Sign in pressed): the owner gate's screen is open.
    owner_screen_open: bool,
    /// Screenshot mode only: no first-run sheets over the page being drawn. Lives in memory; it is
    /// never part of the saved settings.
    quiet_sheets: bool,
    /// Clicks on the sidebar's launcher version, and whether they revealed the developer options
    /// (`dev_options`) for this run.
    dev_clicks: u8,
    dev_options: bool,
    dev_unsigned_dll_confirmation: bool,
    dev_unsigned_wasm_confirmation: bool,
    /// Why the chosen full-access dev folder was refused (Settings, developer options).
    dev_folder_error: Option<String>,
    /// The game is waiting for its session (a token renewal), or a Java instance for its files
    /// and Java (`start_java`), before it starts.
    starting_client: Option<String>,
    starting_yoake_home: Option<std::path::PathBuf>,
    /// Play was pressed on a version not prepared yet: it starts once this preparation ends well.
    play_when_prepared: Option<String>,
    /// Join a friend: the code typed on the Play page, and the one the game
    /// starting now joins.
    friend_code: line_edit::LineEdit,
    friend_focused: bool,
    friend_focus: FocusHandle,
    /// The window's width at the last frame: Home's head lays out by it, as the mockup's `xl`.
    window_width: f32,
    joining: Option<String>,
    /// The file that hands a room to the game starting now (`social::handoff`), and when it was
    /// asked for: its path is the game's `--yoake-social-handoff` argument.
    social_handoff: Option<SocialHandoff>,
    /// Where the next start goes at once: `--world <folder>` or `--server <address>`.
    quick_play: Option<(&'static str, String)>,
    /// The instance the running game plays and when it started: its play time grows on exit.
    client_run: Option<(String, Instant)>,
    /// The running Yoake game's list of approved full-access mods (a run folder); dropped when the game exits.
    client_full_access: Option<horizon_launcher::full_access::RunFolder>,
    /// MX3JN2: the game folder of the running Yoake game that was started with `--join-mod-sync`; its exit code 75 means
    /// "I fetched mods for a host" (`full_access::handle_join_exit`).
    join_sync_home: Option<std::path::PathBuf>,
    /// MX3JN2: the host key of the social room the next guest game joins (set by `friends_join_room`).
    friend_join_host: Option<String>,
    /// MX3JN2: a friend's host principal whose room the launcher could not rejoin by itself after a sync: the page offers
    /// "Join again" (`join_again_host`, `join_again`).
    join_again: Option<String>,
    /// The running game's instance variables, for its post-exit command.
    client_vars: horizon_launcher::launch::Vars,
    /// Settings' Game arguments field: the line, whether it has focus, and its focus handle.
    args_input: line_edit::LineEdit,
    args_focused: bool,
    args_focus: FocusHandle,
    /// Settings' Proxy field, the same way (`proxy`).
    proxy_input: line_edit::LineEdit,
    proxy_focused: bool,
    proxy_focus: FocusHandle,
    /// Opt-in data collection: the player's answer and Settings' section (`collection`).
    collection: collection::Collection,
    /// Settings' "Connect an AI agent" (`agents_page`).
    agents: agents_page::AgentsUi,
    /// How many play Horizon now, while the server answers ("N playing now" on the Play page).
    playing: Option<u64>,
    /// How many play on the official server now, while it answers ("M on the official SMP").
    official_players: Option<u64>,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    _appearance: Subscription,
    /// Redraws when the window gains or loses focus: the sky moves only in front.
    _activation: Subscription,
    /// The session's measurements (`perf.rs`), and when the steps they time began.
    perf: perf::Perf,
    install_started: Option<Instant>,
    prepare_started: Option<Instant>,
    /// The download under way as first seen (when, files done), for the time left.
    download_seen: Option<(Instant, usize)>,
    /// The tab open under Home's hero (`home_tabs.rs`).
    home_tab: home_tabs::HomeTab,
    /// Home's Play or folder menu, while open (glass look).
    home_menu: glass_home::HomeMenu,
    /// Whether the pointer is over Home's folder button, which then shows its ⋯.
    folder_hovered: bool,
    /// The list row under the pointer in Home's glass tabs (a world, a server): it shows its
    /// actions then.
    list_hover: Option<SharedString>,
    /// A Home list row's ⋯ menu, while open, and whether its Delete… was clicked once.
    list_menu: Option<(SharedString, bool)>,
    /// The size of Home's tab strip, measured as it is laid out: the glass panel's plate cuts
    /// its corner around it.
    home_strip: std::rc::Rc<std::cell::Cell<Option<gpui::Size<Pixels>>>>,
    /// When the running game was first seen running, for Home's "Playing for…".
    client_since: Option<Instant>,
    /// The selected instance's mods, for Home's Mods tab (`instance_mods.rs`).
    instance_mods: instance_mods::InstanceMods,
    yak_mods: yoake_mods_page::State,
    yoake_configs: yoake_configs_page::State,
    /// Modrinth projects' icon URLs (`instance_mods.rs`).
    project_icons: instance_mods::ProjectIcons,
    play_pressed: Option<Instant>,
    /// `horizon-launcher benchmark` started this window to measure it.
    bench: Option<perf::Bench>,
    /// The Modpacks page: the imported packs (read when the page first opens), the one shown,
    /// the report's filter, and the pipeline step running now with its log.
    packs: Vec<horizon_launcher::modpacks::Imported>,
    /// Modpacks installed from Horizon's registry, each its own profile.
    registry_packs: Vec<horizon_launcher::browser::horizon::pack::Installed>,
    packs_loaded: Option<Instant>,
    pack_open: Option<String>,
    pack_filter: Option<horizon_port_pack::report::Status>,
    pack_busy: bool,
    pack_log: VecDeque<String>,
    pack_error: Option<String>,
    /// The game starting now plays this pack's profile (its work folder).
    starting_pack: Option<std::path::PathBuf>,
    /// The Mods page: the browser's search, installs and updates (`mods_page.rs`).
    mods: mods_page::Mods,
    github: github_mods_page::Github,
    /// The instances, for the sidebar and Play, and their dialog (`instances_page.rs`).
    instances: instances_page::Instances,
    /// Home's Worlds section, of the selected instance (`worlds_page.rs`).
    worlds: worlds_page::Worlds,
    multiplayer: multiplayer_page::Multiplayer,
    /// The Friends page: friends, requests, blocks and groups of the portal (`friends_page.rs`).
    friends: friends_page::Friends,
    screenshots: screenshots_page::Screenshots,
    game_packs: packs_page::Packs,
    world_packs: datapacks_page::WorldPacks,
    export: Option<export_page::ExportPanel>,
    launch: Option<launch_page::LaunchPanel>,
    /// The memory row on Home.
    memory_slider: launch_page::MemorySlider,
    importing: import_page::Importing,
    console: console_page::Console,
    /// What the window did when the game started (`while_playing.rs`).
    aside: while_playing::Aside,
    /// The selected instance's desktop shortcut (`shortcut_page.rs`).
    shortcuts: shortcut_page::Shortcuts,
    /// The name the game made up for an offline player (`horizon-player.txt`), for the top bar.
    offline_name: Option<String>,
    /// The player's servers, for the sidebar until the Servers page has read them.
    side_servers: Vec<horizon_launcher::servers::Entry>,
    /// The pixel art made for this window (`art.rs`), and pictures downloaded by URL.
    sky: sky::Sky,
    /// The pixel sunset of the new design (dark appearance): its own view, so a frame of it
    /// alone redraws nothing else.
    scene: gpui::Entity<scene::Scene>,
    /// When the sky next asks for a frame, and the timer that will draw it.
    sky_wake: Option<(Instant, gpui::Task<()>)>,
    /// The window's scale, as of its last frame, for art drawn outside `render`'s reach.
    scale: f32,
}

impl Launcher {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (tx, rx) = channel();
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.theme = this.settings_theme(window);
            cx.notify();
        });
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(80))
                .await;
            if this.update(cx, |this, cx| this.drain(cx)).is_err() {
                break;
            }
        })
        .detach();
        // Counts the time a partner banner is on screen, until it may be hidden.
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(10))
                    .await;
                let counting = this.update(cx, |this, cx| {
                    let s = &mut this.settings;
                    if s.partner_hidden || s.partner_seen_secs >= partner::SEEN_BEFORE_HIDE {
                        return false;
                    }
                    // On screen on every page: the card in the sidebar, the banner on Servers.
                    s.partner_seen_secs += 10;
                    if s.partner_seen_secs % 60 == 0
                        || s.partner_seen_secs >= partner::SEEN_BEFORE_HIDE
                    {
                        this.save_settings();
                        cx.notify();
                    }
                    true
                });
                if !matches!(counting, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        let mut settings = Settings::load();
        i18n::set(settings.language());
        if settings.minecraft_dir.is_none() {
            settings.minecraft_dir = backend::default_minecraft_dir();
        }
        // Release builds on Linux put their entry back in the applications menu when there is
        // none or it starts a program that is gone (a moved or deleted binary), never over one
        // that works; `HORIZON_LAUNCHER_DESKTOP_ENTRY=1` does it in a development build.
        if cfg!(target_os = "linux")
            && settings.desktop_entry()
            && (!cfg!(debug_assertions)
                || std::env::var_os("HORIZON_LAUNCHER_DESKTOP_ENTRY").is_some())
        {
            std::thread::spawn(|| {
                let _ = horizon_launcher::desktop::repair(backend::exe_path());
            });
        }
        let installed = backend::installed();
        let yoake_update_task = match yoake_feed::FeedClient::configured() {
            Ok(Some(client))
                if matches!(
                    yoake_auth::YoakeAuth::authorization(&yoake_auth::TokenFileAuth::new(
                        &backend::data_dir()
                    )),
                    Ok(Some(_))
                ) =>
            {
                let homes = instances::list()
                    .into_iter()
                    .map(|instance| instance.home)
                    .collect();
                Some(yoake_update::start_update_check(client, homes))
            }
            _ => None,
        };
        if settings
            .selected
            .as_ref()
            .is_none_or(|s| !installed.contains(s))
        {
            settings.selected = installed.first().cloned();
        }
        // Pin the window's chrome (macOS title bar) to the choice before the theme reads
        // `window.appearance()`, so both draw the same look.
        titlebar::apply_appearance(window, settings.appearance);
        let mut this = Self {
            theme: Theme::for_appearance(settings.appearance, settings.accent, window.appearance()),
            yoake_login_task: None,
            yoake_login_event: None,
            yoake_user_code: None,
            yoake_update_task,
            yoake_update_states: HashMap::new(),
            yoake: yoake_page::YoakePage::new(cx),
            page: Page::from_env(),
            switches: 0,
            content_scroll: Scrollbar::default(),
            long_list: cull::Cull::default(),
            opening: (!settings.skip_opening).then(|| {
                let focus = cx.focus_handle();
                window.focus(&focus, cx);
                Opening {
                    started: None,
                    reveal: None,
                    focus,
                }
            }),
            updates: Updates::default(),
            checked: None,
            update_error: None,
            self_update: SelfUpdate::Idle,
            update_install: None,
            bar: (0., 0.),
            releases: Load::Loading,
            installed,
            installing: None,
            removing: Vec::new(),
            disk: Vec::new(),
            disk_for: Vec::new(),
            install_error: None,
            importing_build: false,
            import_error: None,
            native_step: None,
            native_started: None,
            native_done: None,
            native_error: None,
            native_confirm: None,
            native_for: None,
            native_update: None,
            native_builds: Default::default(),
            native_mismatch: None,
            native_log: Default::default(),
            confirm_remove: None,
            picker_open: false,
            hue_area: std::rc::Rc::new(std::cell::Cell::new(None)),
            hue_dragging: false,
            language_menu: false,
            account_menu: false,
            row_menu: Default::default(),
            look: horizon_theme::glass::Glass::default_look(),
            menu_closed_at: None,
            skin: None,
            wardrobe: None,
            wardrobe_reload: false,
            restyle: horizon_cosmetics::enabled(),
            background_error: None,
            args_input: line_edit::LineEdit::new(settings.game_args.clone(), 1024, 38),
            args_focused: false,
            args_focus: cx.focus_handle(),
            proxy_input: line_edit::LineEdit::new(
                settings.proxy.clone().unwrap_or_default(),
                256,
                44,
            ),
            proxy_focused: false,
            proxy_focus: cx.focus_handle(),
            settings,
            settings_save_error: None,
            links: backend::Links::default(),
            preparing: false,
            prepare_log: VecDeque::new(),
            prepare_error: None,
            client: None,
            client_error: None,
            client_started: None,
            client_home: None,
            crash: None,
            servers: servers_page::Servers::new(),
            internet: internet::Internet::new(),
            root_focus: cx.focus_handle(),
            accounts: Accounts::load(),
            auth_status: HashMap::new(),
            auth_progress: HashMap::new(),
            profile_heads: profile_heads::Heads::default(),
            auth_checked: None,
            sign_in_target: None,
            auth_busy: HashMap::new(),
            auth_next: 0,
            sign_in: None,
            sign_in_state: None,
            keychain: None,
            owner: None,
            owner_screen_open: false,
            quiet_sheets: false,
            dev_clicks: 0,
            dev_options: false,
            dev_unsigned_dll_confirmation: false,
            dev_unsigned_wasm_confirmation: false,
            dev_folder_error: None,
            starting_client: None,
            starting_yoake_home: None,
            play_when_prepared: None,
            friend_code: line_edit::LineEdit::new(String::new(), 256, 32),
            friend_focused: false,
            window_width: 0.,
            friend_focus: cx.focus_handle(),
            joining: None,
            social_handoff: None,
            quick_play: None,
            client_run: None,
            client_full_access: None,
            join_sync_home: None,
            friend_join_host: None,
            join_again: None,
            client_vars: Vec::new(),
            collection: collection::Collection::load(),
            agents: agents_page::AgentsUi::default(),
            playing: None,
            official_players: None,
            tx,
            rx,
            _appearance: appearance,
            _activation: cx.observe_window_activation(window, |_, _, cx| cx.notify()),
            perf: perf::Perf::new("session"),
            install_started: None,
            prepare_started: None,
            download_seen: None,
            home_tab: home_tabs::HomeTab::from_env(),
            home_menu: Default::default(),
            folder_hovered: false,
            list_hover: None,
            list_menu: None,
            home_strip: Default::default(),
            client_since: None,
            instance_mods: Default::default(),
            yak_mods: Default::default(),
            yoake_configs: yoake_configs_page::State::new(cx),
            project_icons: Default::default(),
            play_pressed: None,
            bench: perf::Bench::from_env(),
            packs: Vec::new(),
            registry_packs: Vec::new(),
            packs_loaded: None,
            pack_open: None,
            pack_filter: None,
            pack_busy: false,
            pack_log: VecDeque::new(),
            pack_error: None,
            starting_pack: None,
            mods: mods_page::Mods::new(cx),
            github: github_mods_page::Github::new(cx),
            instances: instances_page::Instances::default(),
            worlds: worlds_page::Worlds::default(),
            multiplayer: multiplayer_page::Multiplayer::default(),
            friends: friends_page::Friends::from_env(),
            screenshots: screenshots_page::Screenshots::default(),
            game_packs: packs_page::Packs::default(),
            world_packs: datapacks_page::WorldPacks::default(),
            export: None,
            launch: None,
            memory_slider: Default::default(),
            importing: Default::default(),
            console: Default::default(),
            aside: while_playing::Aside::new(window.window_handle()),
            shortcuts: Default::default(),
            offline_name: offline_name(),
            side_servers: horizon_launcher::servers::list(),
            sky: sky::Sky::default(),
            scene: cx.new(|_| scene::Scene::new()),
            sky_wake: None,
            scale: window.scale_factor(),
        };
        this.reload_instances();
        if this.bench.is_some() {
            this.perf.keep_local_log(false);
        }
        let tx = this.tx.clone();
        let keychain_tx = this.tx.clone();
        // Asking the keychain can block (D-Bus), so not on this thread.
        let accounts = this.accounts.clone();
        std::thread::spawn(move || {
            let store = account::store();
            let _ = keychain_tx.send(Msg::Keychain(account::no_keychain()));
            let _ = keychain_tx.send(Msg::Owner(accounts.has_owner(store)));
        });
        std::thread::spawn(move || {
            let reused = backend::reuse_prepared_everywhere();
            if !reused.is_empty() {
                let _ = tx.send(Msg::Reused(reused));
            }
        });
        this.load_profile_heads();
        this.load_auth_statuses(true);
        this.fetch_startup_releases();
        this.fetch_links();
        if home::getting_ready_preview() {
            this.preparing = true;
            this.play_when_prepared = this.selected().cloned();
        }
        // `HORIZON_LAUNCHER_PAGE=new-instance`: the dialog open on Modded, for screenshots
        // (`new-instance-horizon`, `new-instance-vanilla`: on those).
        let kind = match std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() {
            Ok("new-instance") => Some(new_instance::NewKind::Modded),
            Ok("new-instance-horizon") => Some(new_instance::NewKind::Horizon),
            Ok("new-instance-vanilla") => Some(new_instance::NewKind::Vanilla),
            _ => None,
        };
        if let Some(kind) = kind {
            this.open_instance_dialog(None);
            this.pick_kind(kind, cx);
        }
        if std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() == Ok("edit-instance") {
            this.open_instance_dialog(this.selected_instance().cloned());
        }
        if cfg!(debug_assertions)
            && std::env::var("HORIZON_LAUNCHER_PAGE").as_deref() == Ok("wardrobe")
        {
            this.wardrobe_preview();
        }
        // What the game and the launcher kept, if the player allows it and there is a server.
        telemetry::send_in_background();
        telemetry::watch_playing(this.tx.clone());
        telemetry::watch_official(this.tx.clone());
        this
    }

    /// Keeps the current choices in memory if writing fails; every page offers the same retry.
    fn save_settings(&mut self) -> bool {
        self.settings_save_error = self.settings.save().err();
        self.settings_save_error.is_none()
    }

    fn settings_save_notice(&self, cx: &mut Context<Self>) -> Option<Div> {
        let error = self.settings_save_error.as_ref()?;
        let t = self.theme;
        Some(
            div()
                .flex_none()
                .px(px(theme::PAGE_GUTTER))
                .py(px(12.))
                .child(
                    crate::plate::plate(&t.night)
                        .flex()
                        .items_center()
                        .gap(px(16.))
                        .p(px(16.))
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_w_0()
                                .child(
                                    shell::text(theme::night::UI, t.danger)
                                        .child(tr("Settings were not saved.")),
                                )
                                .child(
                                    shell::text(theme::night::CAPTION, t.ink_soft).child(trf(
                                        "Your changes are still in this window. Retry to save them. {error}",
                                        &[("error", error)],
                                    )),
                                ),
                        )
                        .child(
                            button("settings-save-retry", "Retry", Kind::Secondary, &t)
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.save_settings();
                                    cx.notify();
                                })),
                        ),
                ),
        )
    }

    fn go(&mut self, page: Page) {
        // The game may have made up a name since, and the CLI or another window may have
        // changed the accounts: read them again (small files, once a switch).
        self.offline_name = offline_name();
        self.accounts = Accounts::load();
        // Each check reads the keychain (a prompt on macOS): page switches check every account
        // at most once a minute, and an account added meanwhile at once.
        if self
            .auth_checked
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(60))
            || (self.accounts.accounts.iter()).any(|a| !self.auth_status.contains_key(&a.uuid))
        {
            self.load_auth_statuses(false);
        }
        self.load_profile_heads();
        self.wardrobe_reload |= page == Page::Wardrobe;
        if page != self.page {
            self.page = page;
            self.switches += 1;
            if page == Page::Friends {
                self.friends_opened();
            }
            self.content_scroll = Scrollbar::default();
            self.long_list = cull::Cull::default();
            if page == Page::Mods {
                self.mods.reset_lists();
                // Full-access mods: files put in by hand, the last launch's problem.
                self.reload_full_access();
            }
        }
        self.picker_open = false;
        self.language_menu = false;
        self.mods.browse.sort_menu = false;
        self.account_menu = false;
    }

    fn check_updates(&mut self) {
        self.checked = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Updates(backend::check_updates().map(Box::new)));
        });
    }

    fn update_launcher(&mut self, release: Release) {
        if !backend::self_update_allowed()
            || matches!(
                self.self_update,
                SelfUpdate::Downloading(..) | SelfUpdate::Ready(_)
            )
        {
            return;
        }
        let version = backend::launcher_version(&release.tag)
            .unwrap_or_default()
            .to_string();
        self.self_update = SelfUpdate::Downloading(version, 0.);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::LauncherProgress(p));
            };
            let _ = tx.send(Msg::LauncherUpdated(backend::self_update(
                &release, progress,
            )));
        });
    }

    /// Installs the Horizon version the last check found, keeping the player's choice of
    /// version unless they were on the newest one.
    fn install_update(&mut self) {
        let Some(release) = self.updates.horizon.clone() else {
            return;
        };
        if self.installing.is_some() || self.removing.contains(&release.tag) {
            return;
        }
        let select =
            backend::select_after_update(self.settings.selected.as_deref(), &self.installed);
        self.update_install = Some((release.tag.clone(), select));
        self.install(&release);
    }

    /// The native mods turned on in the instance Play starts, unpacked and checked, with their
    /// bundles: what the warning names and the build compiles.
    fn native_mods_to_build(
        &self,
    ) -> Result<(std::path::PathBuf, Vec<native_mods::Bundle>), String> {
        let folder = native_mods::installed_mods(&instances::selected().home)?;
        let found = native_mods::bundles(&folder)?;
        Ok((folder, found))
    }

    /// What installed native build `version` was made from (`native_status::Build`), read once.
    fn native_build_of(&self, version: &str) -> Option<native_status::Build> {
        if !native_mods::is_native(version) {
            return None;
        }
        self.native_builds
            .borrow_mut()
            .entry(version.to_owned())
            .or_insert_with(|| native_status::Build::read(version))
            .clone()
    }

    /// Builds the selected instance's native mods, once the player accepted the warning. The
    /// build stays on the Horizon the instance plays (`native_mods::base_for`), unless the
    /// warning was Update's: then it moves to that newer Horizon.
    /// `anyway`: the player agreed to build mods made for other Horizon versions together.
    fn build_native(&mut self, anyway: bool) {
        if self.native_step.is_some() {
            return;
        }
        self.native_confirm = None;
        self.native_error = None;
        self.native_mismatch = None;
        // This attempt's own log, starting empty: an open panel shows it rather than the last
        // attempt's (`native_log_page`).
        self.native_log_reset();
        let instance = instances::selected();
        self.native_for = Some(instance.id.clone());
        let plays = self.version_of(&instance).cloned();
        let base = match self.native_update.take() {
            Some(newer) => Some(newer),
            None => native_mods::base_for(plays.as_deref(), &self.installed, false),
        };
        let Some(base) = base else {
            let error = tr("Install Horizon first.").to_string();
            self.native_log_preflight(&error);
            self.native_error = Some(error);
            return;
        };
        let folder = match self.native_mods_to_build() {
            Ok((folder, _)) => folder,
            Err(e) => {
                self.native_log_preflight(&e);
                self.native_error = Some(e);
                return;
            }
        };
        self.native_step = Some(tr("Starting the build").into());
        self.native_started = Some((Instant::now(), 0));
        self.native_done = None;
        let profile = native_mods::Profile::chosen(&self.settings);
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let result = native_mods::build(&base, &folder, &instance, anyway, profile, |step| {
                let _ = tx.send(Msg::NativeProgress(step.label()));
            });
            let _ = tx.send(Msg::Native(result));
        });
    }

    fn fetch_releases(&mut self) {
        self.releases = Load::Loading;
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Releases(backend::releases()));
        });
    }

    fn fetch_startup_releases(&mut self) {
        self.checked = Some(Instant::now());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let updates = backend::releases_and_updates(|releases| {
                let _ = tx.send(Msg::Releases(releases));
            });
            let _ = tx.send(Msg::Updates(updates.map(Box::new)));
        });
    }

    fn fetch_links(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Links(backend::links()));
        });
    }

    fn drain(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        let login_events: Vec<_> = self
            .yoake_login_task
            .as_ref()
            .map(|task| task.events.try_iter().collect())
            .unwrap_or_default();
        let mut login_finished = false;
        let mut login_succeeded = false;
        for event in login_events {
            match &event {
                yoake_login::LoginEvent::VerificationRequired { user_code, url } => {
                    self.yoake_user_code = Some(user_code.clone());
                    cx.open_url(url);
                }
                yoake_login::LoginEvent::AuthorizationPending
                | yoake_login::LoginEvent::SlowDown { .. } => {}
                yoake_login::LoginEvent::Success(_) => {
                    login_finished = true;
                    login_succeeded = true;
                    self.yoake_user_code = None;
                }
                _ => {
                    login_finished = true;
                    self.yoake_user_code = None;
                }
            }
            self.yoake_login_event = Some(event);
            changed = true;
        }
        if login_finished {
            self.yoake_login_task = None;
        }
        if login_succeeded {
            self.yoake_refresh();
        }
        changed |= self.drain_yoake_page();
        changed |= self.drain_friends();
        changed |= self.drain_yak_mods();
        changed |= self.drain_yoake_configs();
        let mut update_finished = false;
        if let Some(task) = &self.yoake_update_task {
            loop {
                match task.events.try_recv() {
                    Ok(update) => {
                        self.yoake_update_states
                            .insert(update.instance_dir, update.state);
                        changed = true;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        update_finished = true;
                        break;
                    }
                }
            }
        }
        if update_finished {
            self.yoake_update_task = None;
            changed = true;
        }
        self.perf.tick();
        match self.bench.as_mut().map(|b| b.tick(&self.perf)) {
            Some(perf::Step::Go(page)) => {
                let pages: Vec<Page> = (Page::ALL.iter().map(|p| p.0))
                    .filter(|p| *p != Page::Wardrobe || horizon_cosmetics::enabled())
                    .collect();
                self.go(pages[page % pages.len()]);
                changed = true;
            }
            Some(perf::Step::Scroll) => {
                self.go(Page::Versions);
                changed = true;
            }
            Some(perf::Step::Quit) => cx.quit(),
            Some(perf::Step::Wait) | None => {}
        }
        // The running build's clock moves on each second.
        if let Some((started, shown)) = &mut self.native_started {
            if started.elapsed().as_secs() != *shown {
                *shown = started.elapsed().as_secs();
                changed = true;
            }
        }
        while let Ok(msg) = self.rx.try_recv() {
            changed = true;
            match msg {
                Msg::Releases(r) => {
                    if let (Ok(_), Some(checked)) = (&r, self.checked) {
                        self.perf.record_since("launcher.release_list_ms", checked);
                    }
                    self.releases = r.map_or_else(Load::Failed, Load::Ready)
                }
                Msg::Links(links) => {
                    // The official server may have moved: its list entry and its count follow.
                    self.multiplayer.stale();
                    telemetry::set_official_server(&links.official_server);
                    self.links = links;
                }
                // The page reads is_prepared when drawn; `changed` redraws it.
                Msg::Reused(_) => self.installed = backend::installed(),
                Msg::Keychain(said) => self.keychain = said,
                Msg::Owner(owns) => self.owner = Some(owns),
                Msg::AccountHead(uuid, url, face, failed) => {
                    self.profile_heads.loaded(&uuid, &url, face, failed)
                }
                Msg::AccountProgress(uuid, generation, phase) => {
                    if self.auth_busy.get(&uuid) == Some(&generation) {
                        self.auth_progress.insert(uuid, phase);
                    }
                }
                Msg::AccountStatus(uuid, generation, status) => {
                    if self.auth_busy.get(&uuid) != Some(&generation) {
                        continue;
                    }
                    self.auth_busy.remove(&uuid);
                    self.auth_progress.remove(&uuid);
                    if self.accounts.find(&uuid).is_some() {
                        if status.state == account::session::State::Ready {
                            self.owner = Some(true);
                        }
                        self.auth_status.insert(uuid, status);
                        // A refresh may have saved a new name or skin: show it now.
                        let saved = Accounts::load();
                        if saved != self.accounts {
                            self.accounts = saved;
                            self.load_profile_heads();
                        }
                    }
                }
                Msg::AccountRemoved(uuid, generation, result) => {
                    if self.auth_busy.get(&uuid) != Some(&generation) {
                        continue;
                    }
                    self.auth_busy.remove(&uuid);
                    self.auth_progress.remove(&uuid);
                    match result {
                        Ok(owns) => {
                            self.accounts = Accounts::load();
                            self.auth_status.remove(&uuid);
                            self.owner = Some(owns);
                        }
                        Err(e) => self.sign_in_state = Some(SignInState::Done(Err(e))),
                    }
                }
                Msg::Playing(playing) => self.playing = playing,
                Msg::OfficialPlayers(players) => self.official_players = players,
                Msg::JavaGame(id, home, cmd) => {
                    self.starting_client = None;
                    match yoake_install::prepare_launch(&home, &backend::data_dir()) {
                        Ok(None) => {}
                        Ok(Some(_)) => {
                            self.client_error = Some(
                                "The instance changed to Yoake while starting; retry the launch."
                                    .to_string(),
                            );
                            continue;
                        }
                        Err(error) => {
                            self.client_error =
                                Some(format!("Yoake launch preparation failed: {error}"));
                            continue;
                        }
                    }
                    let spawned = cmd.and_then(|(cmd, vars)| {
                        self.client_vars = vars;
                        backend::spawn(Job::Client, cmd, self.tx.clone())
                    });
                    match spawned {
                        Ok(r) => {
                            self.client = Some(r);
                            self.client_home = Some(home);
                            self.client_run = Some((id, Instant::now()));
                            self.client_error = account::take_unsaved();
                            self.step_aside(cx);
                        }
                        Err(e) => {
                            if e == account::NoOwner.to_string() {
                                self.owner = Some(false);
                                self.owner_screen_open = true;
                            }
                            self.client_error = Some(e);
                        }
                    }
                }
                Msg::Session(session) => {
                    self.load_auth_statuses(false);
                    if let Some(tag) = self.starting_client.take() {
                        let instance_home = self.start_instance_home();
                        let expected_home = self.starting_yoake_home.take();
                        if expected_home
                            .as_ref()
                            .is_some_and(|home| home != &instance_home)
                        {
                            self.client_error = Some(
                                "The selected Yoake instance changed; retry the launch."
                                    .to_string(),
                            );
                            self.starting_pack = None;
                            self.joining = None;
                            self.drop_social_handoff();
                            self.quick_play = None;
                            continue;
                        }
                        let expected_yoake = expected_home.is_some()
                            || yoake_install::has_manifest_or_pending(&instance_home);
                        let (mut cmd, yoake) = match yoake_install::prepare_launch(
                            &instance_home,
                            &backend::data_dir(),
                        ) {
                            Ok(Some(program)) => {
                                if program.resource_tag != tag
                                    || !backend::is_prepared(&program.resource_tag)
                                {
                                    self.client_error = Some(if program.resource_tag != tag {
                                        tr("Yoake's base version changed; retry the launch.")
                                            .to_string()
                                    } else {
                                        trf(
                                            "Yoake requires prepared resources for Horizon {version}; prepare that version and retry.",
                                            &[("version", &program.resource_tag)],
                                        )
                                    });
                                    self.starting_pack = None;
                                    self.joining = None;
                                    self.drop_social_handoff();
                                    self.quick_play = None;
                                    continue;
                                }
                                (
                                    backend::client_command_for_program(
                                        &program.resource_tag,
                                        &program.executable,
                                    ),
                                    true,
                                )
                            }
                            Ok(None)
                                if expected_yoake
                                    || (self.starting_pack.is_none()
                                        && instances::selected().java.is_some()) =>
                            {
                                self.client_error = Some(
                                    "The Yoake installation was recovered without a pack; retry the instance."
                                        .to_string(),
                                );
                                self.starting_pack = None;
                                self.joining = None;
                                self.drop_social_handoff();
                                self.quick_play = None;
                                continue;
                            }
                            Ok(None) => (backend::client_command(&tag), false),
                            Err(error) => {
                                self.client_error = Some(format!("Cannot launch Yoake: {error}"));
                                self.starting_pack = None;
                                self.joining = None;
                                self.drop_social_handoff();
                                self.quick_play = None;
                                continue;
                            }
                        };
                        let (file, offline) = match session {
                            Ok(pass) => pass,
                            Err(account::NoOwner) => {
                                self.owner = Some(false);
                                self.owner_screen_open = true;
                                self.client_error = Some(format!("{}.", tr(account::NEEDS_OWNER)));
                                self.starting_pack = None;
                                self.joining = None;
                                self.drop_social_handoff();
                                self.quick_play = None;
                                continue;
                            }
                        };
                        // The host's game carries the player's session: with none, no room.
                        if file.is_none()
                            && self.social_handoff.as_ref().is_some_and(|h| h.hosting.is_some())
                        {
                            self.client_error = Some(tr(friends_page::NEEDS_SESSION).to_string());
                            self.starting_pack = None;
                            self.joining = None;
                            self.drop_social_handoff();
                            self.quick_play = None;
                            continue;
                        }
                        // Past the gate by the developer option alone is no owner.
                        if !account::dev_offline_enabled(self.settings.dev_offline) {
                            self.owner = Some(true);
                        }
                        if let Some(file) = file {
                            cmd.arg("--session-file").arg(file);
                        }
                        // Still play, offline, and say why.
                        if let Some(e) = offline {
                            self.client_error = Some(format!("Playing offline: {e}"));
                        } else if let Some(e) = account::take_unsaved() {
                            self.client_error = Some(e);
                        }
                        self.accounts = Accounts::load();
                        // An offline name the player picked (the quick switch).
                        if let Some(name) = self.accounts.name_arg() {
                            cmd.arg("--name").arg(name);
                        }
                        cmd.envs(self.accounts.look_env());
                        // A pack's Play button plays that pack; Play plays the selected instance.
                        let (profile, run) = match self.starting_pack.take() {
                            Some(work) => {
                                let instance = instances::list()
                                    .into_iter()
                                    .find(|instance| instance.work == work);
                                let id = instance.as_ref().map(|instance| instance.id.clone());
                                let applied = if yoake {
                                    if let Some(mut instance) = instance {
                                        instance.kind = instances::Kind::Own;
                                        instance.java = None;
                                        instances::apply(&mut cmd, &instance)
                                    } else {
                                        Ok(())
                                    }
                                } else {
                                    horizon_launcher::modpacks::apply_profile(&mut cmd, &work)
                                };
                                (applied, id)
                            }
                            None => {
                                let mut instance = instances::selected();
                                if yoake {
                                    instance.kind = instances::Kind::Own;
                                    instance.java = None;
                                }
                                (instances::apply(&mut cmd, &instance), Some(instance.id))
                            }
                        };
                        if let Err(e) = profile {
                            self.client_error = Some(e);
                            self.drop_social_handoff();
                            continue;
                        }
                        if yoake {
                            cmd.current_dir(&instance_home)
                                .env("HORIZON_HOME", &instance_home);
                        }
                        if let Some(code) = self.joining.take() {
                            if let Err(e) = backend::join_friend(&mut cmd, &code) {
                                self.client_error = Some(e);
                            }
                        }
                        // MX3JN2: the host this launch joins, for `--remote-mods` and the host-scoped list
                        let mut join_host: Option<String> = None;
                        if let Some((flag, value)) = self.quick_play.take() {
                            if flag == "--server" {
                                join_host = horizon_launcher::full_access::server_host_key(&value);
                            }
                            cmd.arg(flag).arg(value);
                        }
                        let friend_host = self.friend_join_host.take();
                        // A room: the game gets the path of its private file and nothing else.
                        let handoff = self.take_social_handoff();
                        if matches!(&handoff, Some((_, None))) {
                            join_host = join_host.or(friend_host);
                        }
                        if let Some((path, hosting)) = &handoff {
                            cmd.arg(social::handoff::HANDOFF_ARG).arg(path);
                            // The host's game reads the launcher's lines on its stdin.
                            if hosting.is_some() {
                                cmd.arg(social::handoff::CONTROL_STDIN_ARG);
                            }
                        }
                        let hosting = handoff.as_ref().and_then(|(_, hosting)| hosting.clone());
                        backend::append_game_args(&mut cmd);
                        // Last, around every argument: the instance's pre-launch command,
                        // wrapper and variables.
                        let instance = run
                            .as_ref()
                            .and_then(|id| instances::list().into_iter().find(|i| &i.id == id))
                            .map(|mut instance| {
                                if yoake {
                                    instance.java = None;
                                }
                                instance
                            });
                        self.client_vars = instance
                            .as_ref()
                            .map(|i| i.vars(cmd.get_program()))
                            .unwrap_or_default();
                        cmd.envs(self.client_vars.iter().map(|(k, v)| (k, v)));
                        if let Err(error) = backend::check_client_args(&cmd) {
                            self.client_error = Some(error);
                            self.fail_social_handoff(handoff);
                            continue;
                        }
                        let launch = instance.map(|i| i.launch).unwrap_or_default();
                        horizon_launcher::launch::limit_memory(&mut cmd, &launch);
                        // MX3U1: the approved full-access list for this launch (the problem, if any, is kept for
                        // `full_access::last_launch_problem`).
                        let mut join_sync: Option<std::path::PathBuf> = None;
                        if yoake {
                            let prepared = horizon_launcher::full_access::prepare_joining(
                                &instance_home,
                                join_host.as_deref(),
                                self.settings.join_mod_sync,
                            );
                            if prepared.join_mod_sync {
                                join_sync = Some(instance_home.clone());
                            }
                            prepared.apply(&mut cmd);
                            self.client_full_access = prepared.run;
                            self.reload_full_access();
                        }
                        let mut cmd = horizon_launcher::launch::wrap(cmd, &launch);
                        if yoake {
                            cmd.env("HORIZON_HOME", &instance_home);
                        }
                        horizon_launcher::telemetry::protect_command(&mut cmd);
                        self.client_home = cmd.get_current_dir().map(|d| d.to_path_buf());
                        match backend::spawn(Job::Client, cmd, self.tx.clone()) {
                            Ok(r) => {
                                self.client = Some(r);
                                self.join_sync_home = join_sync;
                                self.client_run = run.map(|id| (id, Instant::now()));
                                self.step_aside(cx);
                                if let Some(room) = hosting {
                                    // The game's stdin goes to the room's driver.
                                    let stdin =
                                        self.client.as_mut().and_then(|c| c.stdin.take());
                                    self.friends_start_host(room, stdin);
                                }
                            }
                            Err(e) => {
                                self.client_full_access = None;
                                self.client_error = Some(e);
                                self.fail_social_handoff(handoff);
                            }
                        }
                    }
                }
                Msg::Progress(tag, p) => {
                    self.bar = (self.bar.1, p);
                    self.installing = Some((tag, p));
                }
                Msg::Installed(tag, result) => {
                    self.installing = None;
                    self.bar = (0., 0.);
                    self.installed = backend::installed();
                    let as_update = self.update_install.take().filter(|(t, _)| *t == tag);
                    if let (Ok(()), Some(started)) = (&result, self.install_started.take()) {
                        let name = if as_update.is_some() {
                            "launcher.update_ms"
                        } else {
                            "launcher.install_ms"
                        };
                        self.perf.record_since(name, started);
                    }
                    match result {
                        Ok(()) => {
                            if self.updates.horizon.as_ref().is_some_and(|r| r.tag == tag) {
                                self.updates.horizon = None;
                            }
                            if as_update.is_none_or(|(_, select)| select) {
                                self.settings.selected = Some(tag);
                            }
                            // Pruning reads settings from disk: protect the selection only once
                            // it has been saved there. A failed save leaves all versions in place.
                            if self.save_settings() {
                                let tx = self.tx.clone();
                                std::thread::spawn(move || {
                                    let _ = tx.send(Msg::Pruned(backend::prune_old_versions()));
                                });
                            }
                        }
                        Err(e) => self.install_error = Some((tag, e)),
                    }
                }
                Msg::Removed(tag, result) => {
                    self.removing.retain(|t| *t != tag);
                    self.installed = backend::installed();
                    let selected = if self.settings_save_error.is_some() {
                        self.settings.selected.take()
                    } else {
                        Settings::load().selected
                    };
                    self.settings.selected = selection_after_removal(&self.installed, selected);
                    if let Err(e) = result {
                        self.install_error = Some((tag, e));
                    }
                }
                Msg::DiskUse(disk) => self.disk = disk,
                Msg::Imported(result) => {
                    self.importing_build = false;
                    self.installed = backend::installed();
                    match result {
                        // `build_import::import` saved the selection; this keeps the window's copy.
                        Ok(tag) => self.settings.selected = Some(tag),
                        Err(e) => self.import_error = Some(e),
                    }
                }
                Msg::Pruned(gone) => {
                    if !gone.is_empty() {
                        self.installed = backend::installed();
                    }
                }
                Msg::Updates(Ok(updates)) => {
                    self.update_error = None;
                    self.updates = *updates;
                    if self.settings.auto_install() {
                        self.install_update();
                    }
                    if let Some(release) = self.updates.launcher.clone() {
                        self.update_launcher(release);
                    }
                }
                Msg::Updates(Err(e)) => self.update_error = Some(e),
                Msg::NativeProgress(step) => self.native_step = Some(step),
                Msg::Native(result) => {
                    // The last step of a build that worked says how it went.
                    let last = self.native_step.take();
                    self.native_started = None;
                    self.native_done = result.is_ok().then_some(last).flatten();
                    self.installed = backend::installed();
                    self.reload_instances();
                    // A build attempt ended: an open panel reads the log it left.
                    self.native_log.stale = true;
                    match result.map_err(|e| match e.strip_prefix(native_mods::MISMATCH) {
                        Some(list) => Ok(list.to_owned()),
                        None => Err(e),
                    }) {
                        Err(Ok(list)) => self.native_mismatch = Some(list),
                        Err(Err(e)) => self.native_error = Some(e),
                        Ok(_) => {}
                    }
                }
                Msg::LauncherProgress(p) => {
                    if let SelfUpdate::Downloading(_, f) = &mut self.self_update {
                        *f = p;
                    }
                }
                Msg::LauncherUpdated(result) => {
                    let version = match &self.self_update {
                        SelfUpdate::Downloading(v, _) => v.clone(),
                        _ => String::new(),
                    };
                    self.self_update = match result {
                        Ok(()) => SelfUpdate::Ready(version),
                        Err(e) => SelfUpdate::Failed(e),
                    };
                }
                Msg::PackLine(line) => {
                    self.pack_log.push_back(line);
                    if self.pack_log.len() > LOG_LINES {
                        self.pack_log.pop_front();
                    }
                }
                Msg::PackDone(result) => self.pack_done(result),
                // Old versions log their session; it never reaches the window.
                Msg::Line(_, line) if line.contains("Session ID is") => {}
                Msg::Line(job, line) => {
                    if job == Job::Client && line.starts_with(perf::GAME_FIRST_FRAME) {
                        if let Some(pressed) = self.play_pressed.take() {
                            self.perf
                                .record_since("launcher.play_to_game_frame_ms", pressed);
                        }
                    }
                    // The MCP server keeps its own server's log; the launcher's are the preparer's
                    // and the game's.
                    if job != Job::Server {
                        self.prepare_log.push_back(line);
                        if self.prepare_log.len() > LOG_LINES {
                            self.prepare_log.pop_front();
                        }
                    }
                }
                Msg::JoinSync(result) => match result {
                    Ok(done) => {
                        self.client_error = None;
                        self.join_again = None;
                        match done.rejoin {
                            horizon_launcher::full_access::Rejoin::Server(address) => {
                                self.quick_play("--server", address)
                            }
                            horizon_launcher::full_access::Rejoin::Friend(principal) => {
                                self.rejoin_friend(principal)
                            }
                        }
                    }
                    Err(e) => {
                        self.client_error = Some(format!("Could not get the host's mods: {e}"));
                    }
                },
                Msg::Exit(job, result) => {
                    let mut error = result.err();
                    // MX3JN2: exit code 75 of a game started with `--join-mod-sync` is not a crash: install what it
                    // fetched (a worker thread; the control lock), then join the same host again.
                    if job == Job::Client {
                        let home = self.join_sync_home.take();
                        if let Some(home) = home.filter(|_| {
                            error
                                .as_deref()
                                .is_some_and(horizon_launcher::full_access::is_join_exit_text)
                        }) {
                            error = None;
                            let tx = self.tx.clone();
                            std::thread::spawn(move || {
                                let _ = tx.send(Msg::JoinSync(
                                    horizon_launcher::full_access::handle_join_exit(&home),
                                ));
                            });
                        }
                    }
                    match job {
                        Job::Prepare => {
                            self.preparing = false;
                            // Play pressed before the version was prepared plays now.
                            if let Some(tag) = self.play_when_prepared.take() {
                                if error.is_none() && backend::is_prepared(&tag) {
                                    self.starting_pack = None;
                                    self.start_client(tag);
                                }
                            }
                            if let (None, Some(started)) = (&error, self.prepare_started.take()) {
                                self.perf.record_since("launcher.prepare_ms", started);
                            }
                            // A preparer process's exit status says little, so its last line
                            // follows; the launcher's own errors are full sentences already.
                            self.prepare_error = error.map(|e| {
                                let tail = self
                                    .prepare_log
                                    .iter()
                                    .rev()
                                    .find(|l| !l.trim().is_empty())
                                    .filter(|_| e.starts_with("Exited with"));
                                tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                            });
                            self.finish_yoake_base_prepare();
                        }
                        Job::Client => {
                            self.client = None;
                            self.client_full_access = None;
                            self.reload_full_access();
                            self.friends_game_ended();
                            self.come_back(cx);
                            // The game saved: last played, sizes and pictures changed.
                            self.worlds.stale();
                            self.multiplayer.stale();
                            self.screenshots.stale();
                            self.game_packs.stale();
                            let vars = std::mem::take(&mut self.client_vars);
                            if let Some((id, started)) = self.client_run.take() {
                                if let Err(e) = instances::record_play(&id, started.elapsed()) {
                                    eprintln!("Play time not saved: {e}");
                                }
                                if let Some(i) = instances::list().into_iter().find(|i| i.id == id)
                                {
                                    horizon_launcher::launch::run_after_exit(
                                        &i.launch, &i.home, &vars,
                                    );
                                }
                                self.reload_instances();
                            }
                            // The run's own game folder: an instance's or a pack's keeps its reports.
                            let home = self.client_home.clone().unwrap_or_else(backend::game_home);
                            let report = self
                                .client_started
                                .take()
                                .and_then(|since| backend::crash_report_since(&home, since));
                            if error.is_some() && report.is_none() && self.switched_to_dx12() {
                                self.client_error = Some(
                                    horizon_launcher::brand::text(
                                        "Horizon's Vulkan renderer crashed while starting on this \
                                     graphics card, so the game now uses DirectX 12. Press Play \
                                     to start it again. Video Settings > Graphics API switches \
                                     back.",
                                    )
                                    .into_owned(),
                                );
                            } else if error.is_some() || report.is_some() {
                                let summary = report
                                    .as_ref()
                                    .and_then(|p| std::fs::read_to_string(p).ok())
                                    .and_then(|text| backend::crash_summary(&text))
                                    .or_else(|| {
                                        let tail = self
                                            .prepare_log
                                            .iter()
                                            .rev()
                                            .find(|l| !l.trim().is_empty());
                                        error.as_ref().map(|e| {
                                            tail.map_or(e.clone(), |t| format!("{e}: {t}"))
                                        })
                                    })
                                    .unwrap_or_default();
                                self.crash = Some(Crash {
                                    report,
                                    summary,
                                    copied: false,
                                });
                            }
                        }
                        Job::Server => {}
                    }
                }
            }
        }
        changed |= self.servers_tick();
        changed |= self.mods_tick();
        changed |= self.github_tick();
        changed |= self.native_log_read(cx);
        if let Some(sign_in) = &self.sign_in {
            if let Some(state) = sign_in_change(&mut self.sign_in_state, sign_in.state()) {
                if let SignInState::Done(Ok(ref account)) = state {
                    self.auth_busy.remove(&account.uuid);
                    self.sign_in_target = None;
                    self.accounts = Accounts::load();
                    self.load_profile_heads();
                    self.load_auth_statuses(false);
                    // Sign-in refuses accounts that don't own the game.
                    self.owner = Some(true);
                    self.owner_screen_open = false;
                    self.client_error = None;
                    if !self.settings.owner_notice_seen {
                        self.settings.owner_notice_seen = true;
                        self.save_settings();
                    }
                }
                changed = true;
            }
        }
        if self
            .checked
            .is_some_and(|at| at.elapsed() > backend::CHECK_EVERY)
        {
            self.check_updates();
        }
        if changed {
            cx.notify();
        }
    }

    /// After a crash with no report: when the game's output ends while its Vulkan renderer
    /// started, a native crash in the graphics driver (#746), switches the game to DirectX 12
    /// (Windows only; only where the player left the graphics API on Auto). True when it did.
    fn switched_to_dx12(&self) -> bool {
        if !cfg!(windows) || !backend::vulkan_start_crashed(&self.prepare_log) {
            return false;
        }
        // The GPU is the same for every profile: the game home's setting and the crashed run's.
        let home = backend::game_home();
        let mut homes = vec![home.clone()];
        homes.extend(self.client_home.clone().filter(|h| *h != home));
        let mut switched = false;
        for home in homes {
            match backend::switch_to_dx12(&home) {
                Ok(changed) => switched |= changed,
                Err(e) => eprintln!(
                    "{}: could not switch to DirectX 12: {e}",
                    horizon_launcher::brand::text("Horizon launcher")
                ),
            }
        }
        switched
    }

    /// The version Play starts: the selected instance's own, else the launcher's.
    fn selected(&self) -> Option<&String> {
        self.selected_instance()
            .and_then(|i| self.version_of(i))
            .or_else(|| self.launcher_version())
    }

    fn instance_play_tag(&self, instance: &instances::Instance) -> Option<String> {
        let yoake_tag = yoake_page::enabled()
            .then(|| yoake_install::read_manifest(&instance.home).ok().flatten())
            .flatten()
            .and_then(|manifest| manifest.rollback_index)
            .and_then(|index| index.bases.into_iter().next())
            .map(|base| base.horizon_release);
        yoake_tag.or_else(|| {
            self.version_of(instance)
                .or_else(|| self.launcher_version())
                .cloned()
        })
    }

    fn selected_play_tag(&self) -> Option<String> {
        self.selected_instance()
            .and_then(|instance| self.instance_play_tag(instance))
            .or_else(|| self.selected().cloned())
            .or_else(|| {
                if !yoake_page::enabled() {
                    return None;
                }
                self.selected_instance()
                    .filter(|instance| yoake_install::has_manifest_or_pending(&instance.home))
                    .map(|_| String::new())
            })
    }

    /// The launcher's version: what instances without one of their own play.
    fn launcher_version(&self) -> Option<&String> {
        self.settings
            .selected
            .as_ref()
            .filter(|s| self.installed.contains(s))
    }

    fn install(&mut self, release: &Release) {
        if release.asset.is_none()
            || self.removing.contains(&release.tag)
            || (yoake_page::enabled() && self.yoake_busy())
        {
            return;
        }
        let (release, tx) = (release.clone(), self.tx.clone());
        let tag = release.tag.clone();
        self.installing = Some((tag.clone(), 0.0));
        self.install_started = Some(Instant::now());
        self.bar = (0., 0.);
        self.install_error = None;
        std::thread::spawn(move || {
            let progress = |p| {
                let _ = tx.send(Msg::Progress(tag.clone(), p));
            };
            let result = backend::install(&release, progress);
            let _ = tx.send(Msg::Installed(tag, result));
        });
    }

    /// Deletes an installed version on a background thread: a version is thousands of files and
    /// the window must keep drawing meanwhile. `Msg::Removed` ends it.
    fn remove(&mut self, tag: &str) {
        self.confirm_remove = None;
        if yoake_page::enabled() && self.yoake_busy() {
            return;
        }
        let installing = self.installing.as_ref().map(|(t, _)| t.as_str());
        if !removable(&self.installed, installing, &self.removing, tag) {
            return;
        }
        self.install_error = None;
        self.removing.push(tag.to_string());
        self.installed.retain(|t| t != tag);
        self.settings.selected =
            selection_after_removal(&self.installed, self.settings.selected.take());
        let (tag, tx) = (tag.to_string(), self.tx.clone());
        std::thread::spawn(move || {
            let result = backend::remove_version(&tag);
            let _ = tx.send(Msg::Removed(tag, result));
        });
    }

    /// Asks for a game program (or a zip holding one) and installs it as its own version on a
    /// thread: a version is thousands of files. `Msg::Imported` ends it.
    fn import_build(&mut self, cx: &mut Context<Self>) {
        // A removal still running could take the folder of the same program imported again.
        if self.importing_build || !self.removing.is_empty() {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("Import").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(mut paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.pop() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                // Another picker's import, or a removal, may have started meanwhile.
                if this.importing_build || !this.removing.is_empty() {
                    return;
                }
                this.importing_build = true;
                this.import_error = None;
                let tx = this.tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(Msg::Imported(build_import::import(&path, None)));
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Prepares the selected version, its Minecraft files from the player's install or, for an
    /// owner who asks, from Mojang.
    fn prepare(&mut self, from: backend::MinecraftFrom) {
        if yoake_page::enabled() && self.yoake_busy() {
            return;
        }
        let Some(tag) = self.selected_play_tag() else {
            return;
        };
        self.prepare_tag(tag, from);
    }

    fn prepare_tag(&mut self, tag: String, from: backend::MinecraftFrom) {
        horizon_launcher::mojang::CANCEL.store(false, Ordering::Relaxed);
        let minecraft = self
            .settings
            .minecraft_dir
            .clone()
            .unwrap_or_else(|| horizon_prepare::artifacts::minecraft_dir(None));
        self.prepare_log.clear();
        self.prepare_error = None;
        self.prepare_log
            .push_back("Checking Java and your Minecraft install".into());
        let tx = self.tx.clone();
        let yoake_cancel = self.yoake_prepare_cancel();
        // The checks may download the server jar, so they run on the thread too.
        std::thread::spawn(move || {
            let log = |line| {
                let _ = tx.send(Msg::Line(Job::Prepare, line));
            };
            let result = backend::preparer(&tag, &minecraft, from, &log)
                .inspect(|(_, minecraft)| {
                    if !yoake_cancel
                        .as_ref()
                        .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
                    {
                        backend::sounds(&tag, minecraft, &log);
                    }
                })
                .map(|(p, _)| {
                    backend::set_prepare_step(backend::PrepareStep::Prepare);
                    p
                })
                .and_then(|p| {
                    if yoake_cancel
                        .as_ref()
                        .is_some_and(|cancel| cancel.load(Ordering::Relaxed))
                    {
                        return Err("Cancelled.".to_string());
                    }
                    match p {
                        backend::Preparer::Process(cmd) => {
                            let running = backend::spawn(Job::Prepare, cmd, tx.clone())?;
                            if let Some(cancel) = yoake_cancel {
                                let child = std::sync::Arc::clone(&running.child);
                                std::thread::spawn(move || loop {
                                    let mut child = child.lock().unwrap();
                                    if cancel.load(Ordering::Relaxed) {
                                        let _ = child.kill();
                                        break;
                                    }
                                    if !matches!(child.try_wait(), Ok(None)) {
                                        break;
                                    }
                                    drop(child);
                                    std::thread::sleep(std::time::Duration::from_millis(100));
                                });
                            }
                            Ok(false)
                        }
                        backend::Preparer::InProcess(options) => {
                            backend::prepare_in_process(&options, &log).map(|_| true)
                        }
                    }
                });
            match result {
                // A process sends its own Exit when it ends.
                Ok(false) => {}
                Ok(true) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Ok(())));
                }
                Err(e) => {
                    let _ = tx.send(Msg::Exit(Job::Prepare, Err(e)));
                }
            }
        });
        self.preparing = true;
        self.prepare_started = Some(Instant::now());
        self.download_seen = None;
    }

    fn play(&mut self) {
        if yoake_page::enabled() {
            if let Some(home) = self
                .selected_instance()
                .filter(|instance| yoake_install::has_manifest_or_pending(&instance.home))
                .map(|instance| instance.home.clone())
            {
                if self.client.is_none() && self.starting_client.is_none() {
                    self.starting_pack = None;
                    self.begin_yoake_base(home, true);
                }
                return;
            }
        }
        let java = self
            .selected_instance()
            .filter(|i| i.java.is_some() && !yoake_install::has_manifest_or_pending(&i.home))
            .cloned();
        if let Some(instance) = java {
            // Mojang's game: no Horizon version or preparation, only no game already running.
            // It joins a server; a friend's code and opening a world are Horizon's.
            self.joining = None;
            self.drop_social_handoff();
            let server = self
                .quick_play
                .take()
                .filter(|(flag, _)| *flag == "--server")
                .map(|(_, ip)| ip);
            if self.client.is_some() || self.starting_client.is_some() {
                return;
            }
            self.starting_pack = None;
            self.start_java(instance, server);
            return;
        }
        let Some(tag) = self.selected_play_tag() else {
            return;
        };
        if !self.can_start(&tag) {
            return;
        }
        self.starting_pack = None;
        self.start_client(tag);
    }

    /// Starts a Java instance's Minecraft, joining `server` if given: its files and Java first
    /// (downloaded the first time, their lines in the log), then `drain` starts the game when
    /// the command arrives.
    fn start_java(&mut self, instance: instances::Instance, server: Option<String>) {
        self.client_error = None;
        self.crash = None;
        self.prepare_log.clear();
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        self.starting_client = Some(instance.id.clone());
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let log = |line: String| {
                let _ = tx.send(Msg::Line(Job::Client, line));
            };
            let cmd = backend::java_game(&instance, server.as_deref(), &log).map(|mut cmd| {
                let vars = instance.vars(cmd.get_program());
                cmd.envs(vars.iter().map(|(k, v)| (k, v)));
                (horizon_launcher::launch::wrap(cmd, &instance.launch), vars)
            });
            let _ = tx.send(Msg::JavaGame(instance.id, instance.home, cmd));
        });
    }

    /// Whether `tag` can start now ([`starts_now`]): prepared, no preparation under way, no game
    /// running or starting.
    pub(crate) fn can_start(&self, tag: &str) -> bool {
        let running = self.client.is_some() || self.starting_client.is_some();
        starts_now(
            backend::is_prepared(tag),
            self.preparing || (yoake_page::enabled() && self.yoake_busy()),
            running,
        )
    }

    /// Whether the selected instance's version can start now, from a shortcut (a server's Join,
    /// a world's Play, a friend's code): Horizon's only, never a Java instance.
    pub(crate) fn can_start_selected(&self) -> bool {
        self.selected_instance()
            .is_none_or(|i| i.java.is_none() || yoake_install::has_manifest_or_pending(&i.home))
            && self
                .selected_play_tag()
                .is_some_and(|tag| self.can_start(&tag))
    }

    /// Whether a server's Join starts the selected instance now: a Java instance too, as Play
    /// would (no game running or starting).
    pub(crate) fn can_join_selected(&self) -> bool {
        match self.selected_instance() {
            Some(i) if i.java.is_some() && !yoake_install::has_manifest_or_pending(&i.home) => {
                self.client.is_none() && self.starting_client.is_none()
            }
            _ => self.can_start_selected(),
        }
    }

    fn start_instance_home(&self) -> std::path::PathBuf {
        match &self.starting_pack {
            Some(work) => instances::list()
                .into_iter()
                .find(|instance| instance.work == *work)
                .map(|instance| instance.home)
                .unwrap_or_else(|| horizon_launcher::modpacks::profile_home(work)),
            None => instances::selected().home,
        }
    }

    /// Starts the game: the session first (a network round trip when the token needs
    /// renewing), then `drain` starts the process when it arrives.
    fn start_client(&mut self, tag: String) {
        self.client_error = None;
        self.crash = None;
        self.prepare_log.clear();
        let instance_home = self.start_instance_home();
        let expected_yoake = yoake_install::has_manifest_or_pending(&instance_home);
        self.starting_yoake_home = expected_yoake.then_some(instance_home);
        // A little earlier than the spawn: file times can be coarse.
        self.client_started =
            Some(std::time::SystemTime::now() - std::time::Duration::from_secs(2));
        // What the resources lack (built-in packs, panorama, sounds) and the session (a network
        // round trip when the token needs renewing) come first; `drain` starts the game when the
        // session arrives.
        self.starting_client = Some(tag.clone());
        self.play_pressed = Some(Instant::now());
        let tx = self.tx.clone();
        let offline_without_owner = account::dev_offline_enabled(self.settings.dev_offline);
        std::thread::spawn(move || {
            if !expected_yoake || backend::is_prepared(&tag) {
                backend::complete_assets(&tag);
            }
            let session = account::session_for_game(&backend::game_home(), offline_without_owner);
            let _ = tx.send(Msg::Session(session));
        });
    }

    fn load_profile_heads(&mut self) {
        let resources = self
            .selected()
            .or_else(|| self.installed.first())
            .map(|tag| backend::resources(tag))
            .unwrap_or_default();
        self.profile_heads
            .request(&self.accounts.accounts, resources, &self.tx);
    }

    fn begin_auth_job(&mut self, uuid: &str) -> Option<u64> {
        if self.auth_busy.contains_key(uuid) {
            return None;
        }
        self.auth_next += 1;
        self.auth_busy.insert(uuid.into(), self.auth_next);
        self.auth_progress
            .insert(uuid.into(), account::session::Phase::Queued);
        Some(self.auth_next)
    }

    /// Only workers read the keychain or contact authentication services. Cached public status
    /// keeps drawing independent of keychain prompts, network timeouts and refresh operations.
    fn load_auth_statuses(&mut self, renew_due: bool) {
        self.auth_checked = Some(Instant::now());
        for account in self.accounts.accounts.clone() {
            let Some(generation) = self.begin_auth_job(&account.uuid) else {
                continue;
            };
            let tx = self.tx.clone();
            std::thread::spawn(move || {
                let progress = |phase| {
                    let _ = tx.send(Msg::AccountProgress(
                        account.uuid.clone(),
                        generation,
                        phase,
                    ));
                };
                progress(account::session::Phase::Keychain);
                let store = account::store();
                let mut status =
                    account::session::status_with_progress(store, &account.uuid, &progress);
                if renew_due && status.state == account::session::State::NeedsRefresh {
                    if let Ok(renewed) =
                        account::session::refresh_with_progress(&account.uuid, &progress)
                    {
                        status = renewed;
                    }
                }
                let _ = tx.send(Msg::AccountStatus(account.uuid, generation, status));
            });
        }
    }

    fn refresh_account(&mut self, uuid: String) {
        let Some(generation) = self.begin_auth_job(&uuid) else {
            return;
        };
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let progress = |phase| {
                let _ = tx.send(Msg::AccountProgress(uuid.clone(), generation, phase));
            };
            progress(account::session::Phase::Keychain);
            if let Ok(status) = account::session::refresh_with_progress(&uuid, &progress) {
                let _ = tx.send(Msg::AccountStatus(uuid, generation, status));
            } else {
                // It may have been removed through the CLI while this window was open.
                let _ = tx.send(Msg::AccountRemoved(
                    uuid,
                    generation,
                    Ok(Accounts::load().has_owner(account::store())),
                ));
            }
        });
    }

    /// Generic Add account and targeted re-sign-in share the launcher-owned device-code flow.
    fn start_sign_in(&mut self) {
        self.begin_sign_in(None);
    }

    fn re_sign_in(&mut self, uuid: &str) {
        if let Some(account) = self.accounts.find(uuid).cloned() {
            self.begin_sign_in(Some(account));
        }
    }

    fn begin_sign_in(&mut self, target: Option<account::Account>) {
        self.cancel_sign_in();
        self.sign_in = Some(match &target {
            Some(account) => SignIn::start_for(account.clone()),
            None => SignIn::start(),
        });
        self.sign_in_target = target;
        self.sign_in_state = Some(SignInState::Starting);
    }

    fn cancel_sign_in(&mut self) {
        if let Some(old) = self.sign_in.take() {
            old.cancel();
        }
        self.sign_in_state = None;
        self.sign_in_target = None;
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(mut paths))) = paths.await {
                if let Some(path) = paths.pop() {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.minecraft_dir = Some(path);
                        this.save_settings();
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }
}

// ----- Building blocks ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Primary,
    Secondary,
    Ghost,
}

/// The element under the pointer and the one it just left, with when that happened. Hover
/// backgrounds ease in and out from these rather than snapping.
struct HoverState {
    on: Option<(SharedString, Instant)>,
    off: Option<(SharedString, Instant)>,
}

static HOVER: Mutex<HoverState> = Mutex::new(HoverState {
    on: None,
    off: None,
});

fn track_hover(el: Stateful<Div>, id: SharedString) -> Stateful<Div> {
    el.on_hover(move |hovered, window, _| note_hover(&id, *hovered, window))
}

/// Whether `id` is the element [`hover_fade`] shows hovered now.
fn hover_is(id: &SharedString) -> bool {
    HOVER
        .lock()
        .unwrap()
        .on
        .as_ref()
        .is_some_and(|(h, _)| h == id)
}

/// Records that `id` is now hovered or not, for [`hover_fade`]: an element whose own hover
/// listener does more calls it.
fn note_hover(id: &SharedString, hovered: bool, window: &mut Window) {
    let mut state = HOVER.lock().unwrap();
    let is_on = state.on.as_ref().is_some_and(|(h, _)| h == id);
    if hovered && !is_on {
        state.off = state.on.take();
        state.on = Some((id.clone(), Instant::now()));
    } else if !hovered && is_on {
        state.off = Some((id.clone(), Instant::now()));
        state.on = None;
    }
    window.refresh();
}

/// A layer under an element's content that fades `color` in while it is hovered. `bleed` lets
/// it reach past the element on the left and right, as the hover plate of a list row does.
fn hover_layer(id: &SharedString, color: Hsla, radius: f32, bleed: f32) -> AnyElement {
    let layer = || {
        div()
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-bleed))
            .right(px(-bleed))
            .rounded(px(radius))
    };
    hover_fade(id, layer, move |d, a| d.bg(color.opacity(color.a * a)))
}

/// The `layer` made while the element `id` (see [`track_hover`]) is hovered, eased in and out:
/// `paint` draws it at a strength from 0 (not shown) to 1. An element not hovered draws nothing,
/// so a long list costs no layers.
fn hover_fade(
    id: &SharedString,
    layer: impl FnOnce() -> Div,
    paint: impl Fn(Div, f32) -> Div + 'static,
) -> AnyElement {
    let state = HOVER.lock().unwrap();
    // Each transition gets its own animation id, from when it started, so it plays once.
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let start = *START.get_or_init(Instant::now);
    let key = |dir: &str, at: Instant| {
        ElementId::named_usize(format!("{id}-{dir}"), (at - start).as_micros() as usize)
    };
    let ease = || Animation::new(theme::QUICK).with_easing(ease_out_quint());
    if let Some((_, at)) = state.on.as_ref().filter(|(h, _)| h == id) {
        return layer()
            .with_animation(key("in", *at), ease(), paint)
            .into_any_element();
    }
    if let Some((_, at)) = state
        .off
        .as_ref()
        .filter(|(h, at)| h == id && at.elapsed() < theme::QUICK)
    {
        return layer()
            .with_animation(key("out", *at), ease(), move |d, t| paint(d, 1. - t))
            .into_any_element();
    }
    // Not hovered: nothing to draw (out of the flow, so it takes no gap either).
    div().absolute().into_any_element()
}

/// Text set in one step of the type scale: headings (heading, title, display) in the pixel face.
fn text(step: theme::Step) -> Div {
    let d = div().text_size(px(step.px)).line_height(px(step.line_px()));
    let d = if step.mono {
        d.font_family(theme::MONO)
    } else if step.px >= theme::HEADING.px {
        d.font_family(theme::PIXEL)
    } else {
        d
    };
    if step.weight >= 500 {
        d.font_weight(FontWeight::MEDIUM)
    } else {
        d
    }
}

/// A release's tag as shown: a Nightly by its title, an import by its label, else the tag
/// without its `v`.
fn version_label(tag: &str) -> String {
    horizon_launcher::nightly::label(tag)
        .or_else(|| build_import::label(tag))
        .unwrap_or_else(|| tag.trim_start_matches('v').to_string())
}

/// Horizon's mark: two stems, their reflection below, and the horizon across in the accent.
fn mark(size: f32, t: &Theme) -> Div {
    let u = size / 16.;
    let stem = |x: f32, y: f32, color: Hsla| {
        div()
            .absolute()
            .left(px(x * u))
            .top(px(y * u))
            .w(px(2. * u))
            .h(px(4. * u))
            .bg(color)
    };
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .child(stem(4., 3., t.ink))
        .child(stem(10., 3., t.ink))
        .child(stem(4., 8., t.ink.opacity(0.36)))
        .child(stem(10., 8., t.ink.opacity(0.36)))
        .child(
            div()
                .absolute()
                .left(px(u))
                .top(px(7. * u))
                .w(px(14. * u))
                .h(px(u))
                .bg(t.accent),
        )
}

/// The last thing the player did was press Tab: focused controls show a ring (a click focuses them
/// too, without one).
static KEYBOARD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Makes a control reachable with Tab, pressed with Enter or Space, with a ring when the keyboard
/// put it in focus.
fn keyboard(el: Stateful<Div>, t: &Theme) -> Stateful<Div> {
    let (ring, gap) = (t.accent, t.page);
    // A ring with a gap of page colour, so it shows around filled buttons too. GPUI draws nothing
    // for a shadow without blur: half a pixel keeps it crisp.
    let shadow = |color, spread: f32| gpui::BoxShadow {
        color,
        offset: gpui::point(px(0.), px(0.)),
        blur_radius: px(0.5),
        spread_radius: px(spread),
        inset: false,
    };
    el.tab_index(0).focus(move |s| {
        if KEYBOARD.load(Ordering::Relaxed) {
            s.shadow(vec![shadow(ring, 4.), shadow(gap, 1.5)])
        } else {
            s
        }
    })
}

/// A button: a block with a light bevel on top and a darker lip below, its label in the pixel
/// face. Primary (the one main action) is the violet face; Secondary a raised block; Ghost only
/// its label until the pointer is over it.
fn button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    button_label(id, tr(&label).to_string(), kind, t)
}

fn button_label(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    kind: Kind,
    t: &Theme,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label: SharedString = label.into();
    if glass_mode() {
        return match kind {
            Kind::Primary => glass_ui::solid(id, label, 36., 14., t),
            Kind::Secondary | Kind::Ghost => glass_ui::quiet(id, label, 36., t),
        };
    }
    let t = *t;
    let hover = match kind {
        Kind::Primary => t.on_face.opacity(0.10),
        Kind::Secondary => t.bevel.opacity(if t.dark { 0.10 } else { 0.35 }),
        Kind::Ghost => t.raised,
    };
    let base = div()
        .id(id.clone())
        .relative()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(theme::CONTROL))
        .px(px(14.))
        .font_family(theme::PIXEL)
        .text_size(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .child(hover_layer(&id, hover, 0., 0.));
    let base = match kind {
        Kind::Primary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.face_bevel, t.face_lip, theme::LIP))
            .bg(t.face)
            .text_color(t.on_face),
        Kind::Secondary => base
            .border_2()
            .border_color(t.outline)
            .children(blocks::edges(t.button_bevel, t.lip, theme::LIP))
            .bg(t.button)
            .text_color(t.ink),
        Kind::Ghost => base.text_color(t.muted).hover(move |s| s.text_color(t.ink)),
    };
    // The label sits on the block's face, above its lip; pressing pushes it down.
    let base = base.child(
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(8.))
            .pb(px(if kind == Kind::Ghost { 0. } else { 2. }))
            .child(label),
    );
    keyboard(track_hover(base, id), &t).active(|s| s.opacity(theme::PRESSED).top(px(1.)))
}

/// Copies to the clipboard, or into `HORIZON_LAUNCHER_CLIPBOARD` when a test run sets it
/// (`horizon_launcher::desktop_seams`).
fn copy_to_clipboard(cx: &mut App, item: gpui::ClipboardItem) {
    if let Some(file) = horizon_launcher::desktop_seams::clipboard_file() {
        let bytes = match item.entries().first() {
            Some(gpui::ClipboardEntry::Image(image)) => image.bytes.clone(),
            _ => item.text().unwrap_or_default().into_bytes(),
        };
        if let Err(e) = std::fs::write(&file, bytes) {
            eprintln!("{}: {e}", file.display());
        }
        return;
    }
    cx.write_to_clipboard(item.clone());
    #[cfg(target_os = "linux")]
    if let Some(text) = item.text() {
        sync_system_clipboard(text);
    }
}

/// GPUI's Wayland clipboard sends the last key press's serial, which a compositor ignores
/// after a mouse click: hand the text to the desktop's own tool too, the first one present
/// for this session (`wl-copy` on Wayland, `xclip` or `xsel` on X11).
#[cfg(target_os = "linux")]
fn sync_system_clipboard(text: String) {
    use std::io::Write;
    use std::process::{Command, Stdio};
    const TOOLS: [(&str, &str, &[&str]); 3] = [
        ("WAYLAND_DISPLAY", "wl-copy", &[]),
        ("DISPLAY", "xclip", &["-selection", "clipboard"]),
        ("DISPLAY", "xsel", &["--clipboard", "--input"]),
    ];
    if text.is_empty() {
        return;
    }
    // Off the UI thread: a tool can take its time.
    std::thread::spawn(move || {
        for (session, tool, args) in TOOLS {
            if std::env::var_os(session).is_none() {
                continue;
            }
            let Ok(mut child) = Command::new(tool)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            else {
                continue;
            };
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            if child.wait().is_ok_and(|status| status.success()) {
                return;
            }
        }
    });
}

/// An on/off switch: a square knob sliding in a sunk track that turns violet when on.
fn switch(id: impl Into<SharedString>, on: bool, t: &Theme) -> Stateful<Div> {
    if glass_mode() {
        return glass_ui::toggle(id, on, horizon_theme::glass::ACCENT, 0xffffff, t);
    }
    let t = *t;
    let id: SharedString = id.into();
    let knob = if on { t.on_face } else { t.muted };
    keyboard(div().id(id.clone()), &t)
        .relative()
        .flex_none()
        .w(px(44.))
        .h(px(24.))
        .border_2()
        .border_color(t.outline)
        .cursor_pointer()
        .bg(if on { t.face } else { t.well })
        .child(
            div()
                .absolute()
                .top(px(2.))
                .size(px(16.))
                .bg(knob)
                .child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(2.))
                        .bg(if on { t.face_lip } else { t.lip }),
                )
                .with_animation(
                    ElementId::named_usize(id, on as usize),
                    Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                    move |d, p| d.left(px(if on { 2. + 20. * p } else { 22. - 20. * p })),
                ),
        )
}

/// A section's title inside a page: a pixel icon in the accent and the name in the pixel face.
fn section_title(label: &'static str, icon: &'static str, t: &Theme) -> Div {
    if glass_mode() {
        use horizon_theme::glass::ink;
        let _ = (icon, t);
        return glass_ui::text(18., FontWeight::SEMIBOLD, ink::FULL)
            .pt(px(8.))
            .child(tr(label).to_string());
    }
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(blocks::icon(icon, 20., t.accent))
        .child(
            blocks::pixel(18., FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(tr(label)),
        )
}

/// A segmented control: a sunk track, the chosen segment a raised block.
fn segments<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    current: T,
    t: &Theme,
    on_pick: impl Fn(T, &mut Window, &mut App) + Clone + 'static,
) -> Stateful<Div> {
    if glass_mode() {
        return div()
            .id(id)
            .flex()
            .flex_none()
            .p(px(3.))
            .gap(px(2.))
            .rounded(px(horizon_theme::glass::radius::ROW))
            .bg(glass_ui::white(horizon_theme::glass::fill::FIELD))
            .children(options.iter().map(|&(value, label)| {
                let pick = on_pick.clone();
                glass_ui::tab(format!("{id}-{label}"), tr(label), current == value, t)
                    .on_click(move |_: &ClickEvent, window, cx| pick(value, window, cx))
            }));
    }
    let t = *t;
    div()
        .id(id)
        .flex()
        .flex_none()
        .p(px(2.))
        .gap(px(2.))
        .border_2()
        .border_color(t.outline)
        .bg(t.well)
        .children(options.iter().map(|&(value, label)| {
            let active = current == value;
            let pick = on_pick.clone();
            keyboard(div().id(SharedString::from(format!("{id}-{label}"))), &t)
                .relative()
                .flex()
                .items_center()
                .h(px(theme::SEGMENT))
                .px(px(14.))
                .pb(px(2.))
                .font_family(theme::PIXEL)
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .when(active, |d| {
                    d.bg(t.button).text_color(t.ink).children(blocks::edges(
                        t.button_bevel,
                        t.lip,
                        3.,
                    ))
                })
                .when(!active, |d| {
                    d.text_color(t.muted).hover(move |s| s.text_color(t.ink))
                })
                .child(div().relative().child(tr(label)))
                .on_click(move |_: &ClickEvent, window, cx| pick(value, window, cx))
        }))
}

/// What the Play button does for the selected version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlayPress {
    Play,
    /// Not prepared, for an account that owns the game: prepare it (from the player's install,
    /// else downloaded from Mojang for that account), then play, as the official launcher does.
    PrepareThenPlay,
    /// Not prepared and no owner (signing in comes first), or busy preparing or playing.
    Nothing,
}

/// Account state uses the same semantic colors as the launcher's other notices. Keep a
/// distinct icon and the full label so color is never the only way to recognize a state.
fn account_status_line(
    status: Option<&account::session::Status>,
    progress: Option<&str>,
    t: &Theme,
) -> Div {
    use account::session::State;
    let (icon, color, label) = if let Some(progress) = progress {
        ("clock", t.muted, progress)
    } else {
        let (icon, color) = match status.map(|s| s.state) {
            Some(State::Ready) => ("check", t.success),
            Some(State::NeedsRefresh | State::Retry) => ("reload", t.warn),
            Some(State::SignInRequired) => ("user", t.danger),
            Some(State::OwnershipRequired) => ("info-box", t.danger),
            None => ("info-box", t.muted),
        };
        (
            icon,
            color,
            status.map_or("Sign-in not checked", |s| s.label()),
        )
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_color(color)
        .child(blocks::icon(icon, 14., color))
        .child(tr(label).to_owned())
}

/// Polling must handle each sign-in transition once. Replaying a completed sign-in
/// restarts every account's status worker and leaves the UI perpetually checking.
fn sign_in_change(shown: &mut Option<SignInState>, next: SignInState) -> Option<SignInState> {
    if shown.as_ref() == Some(&next) {
        return None;
    }
    *shown = Some(next.clone());
    Some(next)
}

/// `owner`: whether an account here owns the game, by the keychain (`Launcher::owner`).
fn play_press(prepared: bool, owner: Option<bool>, busy: bool) -> PlayPress {
    match (prepared, owner) {
        _ if busy => PlayPress::Nothing,
        (true, _) => PlayPress::Play,
        (false, Some(true)) => PlayPress::PrepareThenPlay,
        (false, _) => PlayPress::Nothing,
    }
}

/// Whether a shortcut that starts the game at once (a server's Join, a world's Play, a friend's
/// code, a pack's Play) starts it now: only when Play would play. Never while a preparation runs:
/// the game would open on half-written files.
fn starts_now(prepared: bool, preparing: bool, running: bool) -> bool {
    play_press(prepared, None, preparing || running) == PlayPress::Play
}

/// Whether `tag` can start being deleted now: it is installed, no install is writing it, and it
/// is not already going. Another version deleting at the same time does not stop it.
fn removable(
    installed: &[String],
    installing: Option<&str>,
    removing: &[String],
    tag: &str,
) -> bool {
    installing != Some(tag)
        && installed.iter().any(|t| t == tag)
        && !removing.iter().any(|t| t == tag)
}

/// The selected version once one has been removed: the saved choice if it is still installed,
/// else the newest left.
fn selection_after_removal(installed: &[String], saved: Option<String>) -> Option<String> {
    saved
        .filter(|s| installed.contains(s))
        .or_else(|| installed.first().cloned())
}

fn disabled(b: Stateful<Div>) -> Stateful<Div> {
    b.opacity(theme::DISABLED).cursor_default().tab_stop(false)
}

/// A glass page's head: the title large in Bricolage with its line under it, as the Friends
/// page has it.
fn glass_page_head(title: &str, description: &str) -> Div {
    use horizon_theme::glass::ink;
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(
            div()
                .truncate()
                .font_family(horizon_theme::glass::DISPLAY_FONT)
                .font_weight(FontWeight::SEMIBOLD)
                .text_size(px(56.))
                .line_height(px(56.))
                .text_color(glass_ui::white(ink::FULL))
                .child(tr(title).to_string()),
        )
        .child(
            glass_ui::text(15., FontWeight::NORMAL, ink::NOTE)
                // Kana and kanji fill the title's line to its edges: more air.
                .mt(px(if i18n::current() == i18n::Language::Japanese {
                    18.
                } else {
                    8.
                }))
                .max_w(px(520.))
                .child(tr(description).to_string()),
        )
}

/// Whether the pages, the top bar and the sidebar are drawn in the glass look in both themes.
/// (The light theme's sky stays the dawn one.)
const GLASS_PAGES: bool = true;

thread_local! {
    /// Set while a page that has moved to the glass look but still builds with the pixel look's
    /// helpers (`button`, `slot_row`, `notice`, ...) is drawn: they then give their glass form.
    /// Everything else draws as before. See [`GlassMode`].
    static GLASS_MODE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Whether the page being built is in the glass look (see [`GlassMode`]).
fn glass_mode() -> bool {
    GLASS_MODE.with(|m| m.get())
}

/// Puts the pixel look's helpers into their glass form until it is dropped. The Mods page holds
/// one while it is built (its rows are built eagerly, in the same call).
struct GlassMode(bool);

impl GlassMode {
    fn enter(on: bool) -> Self {
        Self(GLASS_MODE.with(|m| m.replace(on)))
    }
}

impl Drop for GlassMode {
    fn drop(&mut self) {
        GLASS_MODE.with(|m| m.set(self.0));
    }
}

/// A small rounded label (22 px tall): the glass look's badge, `fg` on a tint of itself.
fn glass_pill(label: impl Into<SharedString>, fg: Hsla, tint: f32) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(22.))
        .px(px(8.))
        .rounded_full()
        .bg(fg.opacity(tint))
        .child(
            glass_ui::text(12., FontWeight::MEDIUM, 1.)
                .text_color(fg)
                .child(label.into()),
        )
}

/// A page's big title inside its content: the pixel look's `theme::TITLE`, or 24 px semibold
/// Geist in the glass form.
fn mode_title() -> Div {
    if glass_mode() {
        glass_ui::text(24., FontWeight::SEMIBOLD, horizon_theme::glass::ink::FULL)
            .line_height(px(30.))
    } else {
        text(theme::TITLE)
    }
}

/// `t` with the glass look's white inks, for text drawn by helpers that take a Theme (markdown).
fn glass_ink_theme(t: &Theme) -> Theme {
    use horizon_theme::glass::ink;
    let mut g = *t;
    g.ink = glass_ui::white(ink::FULL);
    g.ink_soft = glass_ui::white(ink::SOFT);
    g.muted = glass_ui::white(ink::DIM);
    g.plate_soft = glass_ui::white(0.08);
    g.rule = glass_ui::white(0.12);
    g
}

/// A glass panel for one block of a page (a release, a notice): round at 20, content after it.
fn glass_card() -> Div {
    glass::plate(horizon_theme::glass::radius::PANEL)
}

/// A section of a glass page: its title (an icon and 18 px text) with an optional action at the
/// right, over one glass panel that holds `body` (see [`glass_body`]).
fn glass_section(
    label: &str,
    icon: &'static str,
    right: Option<AnyElement>,
    body: Div,
) -> Div {
    use horizon_theme::glass::ink;
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(glass_ui::icon(icon, 20., glass_ui::white(ink::NOTE)))
                        .child(
                            glass_ui::text(18., FontWeight::SEMIBOLD, ink::FULL)
                                .child(tr(label).to_string()),
                        ),
                )
                .children(right),
        )
        .child(glass_card().child(body))
}

/// The inside of a glass panel that lists things: 8 px around its rows.
fn glass_body() -> Div {
    div().relative().flex().flex_col().p(px(8.))
}

/// A row of a glass panel that is only read, with no hover or Tab stop of its own: a name over
/// its detail, a value or control at the right. The glass `index_row`.
fn glass_index_row(
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
) -> Div {
    glass_slot_row(None, name, detail, right)
}

/// [`glass_index_row`] with a picture or letter slot before the name.
fn glass_slot_row(
    slot: Option<Div>,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
) -> Div {
    use horizon_theme::glass::{ink, radius};
    div()
        .relative()
        .flex()
        .items_center()
        .gap(px(12.))
        .w_full()
        .px(px(12.))
        .py(px(10.))
        .rounded(px(radius::ROW))
        .when(glass_mode(), |d| {
            d.bg(glass_ui::white(horizon_theme::glass::fill::HOVER))
                .mb(px(8.))
                .px(px(16.))
                .py(px(12.))
        })
        .children(slot)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    glass_ui::text(14., FontWeight::MEDIUM, ink::FULL)
                        .line_height(px(21.))
                        .child(name),
                )
                .children(detail.map(|d| {
                    glass_ui::text(13., FontWeight::NORMAL, ink::QUIET)
                        .line_height(px(19.5))
                        .child(d)
                })),
        )
        .children(right.map(|r| div().flex().flex_none().items_center().child(r)))
}

/// A line of text inside a glass panel (`px-3 pb-2`): white at 60 %, or the danger red.
fn glass_note(text_: impl Into<SharedString>, bad: bool) -> Div {
    use horizon_theme::glass::ink;
    div().px(px(12.)).pb(px(8.)).child(
        glass_ui::text(13., FontWeight::NORMAL, ink::NOTE).when(bad, |d| {
            d.text_color(glass_ui::color(horizon_theme::glass::DANGER))
        })
        .child(text_.into()),
    )
}

/// A sub-heading inside a glass panel, over a group of its rows.
fn glass_subhead(label: impl Into<SharedString>) -> Div {
    use horizon_theme::glass::ink;
    glass_ui::text(14., FontWeight::SEMIBOLD, ink::FULL)
        .px(px(12.))
        .pt(px(16.))
        .pb(px(8.))
        .child(label.into())
}

/// A page's title in the pixel face and one line on what it is for.
fn page_header(title: &str, description: &str, t: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            blocks::pixel(36., FontWeight::SEMIBOLD)
                .text_color(t.ink)
                .child(tr(title).to_string()),
        )
        .child(
            text(theme::UI)
                .max_w(px(520.))
                .text_color(t.muted)
                .child(tr(description).to_string()),
        )
}

/// One row of a list, as its own panel: a name in the pixel face with an optional detail under
/// it, then a value or a control on the right. Rows were numbered in the quiet look; `_n` keeps
/// the call sites as they are.
/// How an available update is named: a Nightly by its title ("Nightly 2026-10-28 · 0c02067"), a
/// release by its tag.
fn update_name(release: &Release) -> &str {
    match release.nightly {
        Some(_) => &release.name,
        None => &release.tag,
    }
}

fn index_row(
    _n: usize,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    slot_row(None, name, detail, right, t)
}

/// A row with an icon slot (`blocks::slot`) on its left: a project in the Mods page's lists.
fn slot_row(
    slot: Option<Div>,
    name: impl IntoElement,
    detail: Option<AnyElement>,
    right: Option<AnyElement>,
    t: &Theme,
) -> Div {
    if glass_mode() {
        return glass_slot_row(slot, name, detail, right);
    }
    blocks::panel(t)
        .flex()
        .items_center()
        .gap(px(16.))
        .min_h(px(theme::ROW))
        .mb(px(8.))
        .px(px(16.))
        .py(px(12.))
        .children(slot)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
                .child(
                    blocks::pixel(16., FontWeight::MEDIUM)
                        .text_color(t.ink)
                        .child(name),
                )
                .children(detail.map(|d| {
                    text(theme::CAPTION)
                        .text_color(t.muted)
                        .child(d)
                        .into_any_element()
                })),
        )
        .children(right)
}

/// A plate that stands apart from the page: notices, the console, the agents note.
fn plate(t: &Theme) -> Div {
    if glass_mode() {
        // A tile on the page's glass: white at 6 %.
        return div()
            .relative()
            .rounded(px(horizon_theme::glass::radius::ROW))
            .bg(glass_ui::white(horizon_theme::glass::fill::FIELD))
            .text_size(px(14.))
            .text_color(glass_ui::white(horizon_theme::glass::ink::FULL));
    }
    blocks::panel(t)
}

fn tab_action(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    t: &Theme,
) -> Stateful<Div> {
    let label: SharedString = label.into();
    shell::action(id, shell::Action::Raised, t).child(shell::label(tr(&label).to_string()))
}

/// Floating text with no background
fn quiet_note(text_: impl Into<SharedString>, t: &Theme) -> Div {
    plate(t)
        .px(px(12.))
        .py(px(8.))
        .child(text(theme::CAPTION).text_color(t.muted).child(text_.into()))
}

/// Something went wrong: a dot in the danger colour, then what happened.
fn notice(text_: String, t: &Theme) -> Div {
    if glass_mode() {
        use horizon_theme::glass::{ink, radius};
        let _ = t;
        let danger = glass_ui::color(horizon_theme::glass::DANGER);
        return div()
            .flex()
            .gap(px(12.))
            .px(px(16.))
            .py(px(12.))
            .rounded(px(radius::ROW))
            .bg(gpui::rgba(horizon_theme::glass::DANGER_HOVER << 8 | 0x1f))
            .child(glass_ui::icon("information-circle", 18., danger))
            .child(
                glass_ui::text(14., FontWeight::NORMAL, ink::FULL)
                    .flex_1()
                    .min_w_0()
                    .child(text_),
            );
    }
    plate(t)
        .flex()
        .gap(px(12.))
        .p(px(16.))
        .text_size(px(theme::UI.px))
        .line_height(px(21.))
        .text_color(t.ink)
        .border_l(px(6.))
        .child(
            div()
                .absolute()
                .top(px(-2.))
                .bottom(px(-2.))
                .left(px(-6.))
                .w(px(6.))
                .bg(t.danger),
        )
        .child(blocks::icon("info-box", 20., t.danger))
        .child(div().flex_1().min_w_0().child(text_))
}

/// An empty state: a short sentence, and the one thing to do about it.
fn centered(text_: &str, t: &Theme) -> Div {
    if glass_mode() {
        use horizon_theme::glass::ink;
        let _ = t;
        return div()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(16.))
            .py(px(32.))
            .child(glass_ui::text(14., FontWeight::NORMAL, ink::NOTE).child(text_.to_string()));
    }
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap(px(16.))
        .py(px(48.))
        .child(text(theme::UI).text_color(t.muted).child(text_.to_string()))
}

/// The name the game made up for a player without an account, once it has run.
fn offline_name() -> Option<String> {
    player_name(&std::fs::read_to_string(backend::game_home().join("horizon-player.txt")).ok()?)
}

/// The name in `horizon-player.txt`: the game writes `name:Player12345` (horizon-client's
/// offline_name.rs).
fn player_name(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("name:"))
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_owned)
}

/// `1234` as `1,234`.
fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A mono line in upper case: the status above a title, the footer's links.
fn caps(s: impl AsRef<str>) -> Div {
    if glass_mode() {
        return glass_ui::text(13., FontWeight::MEDIUM, horizon_theme::glass::ink::MUTED)
            .child(s.as_ref().to_string());
    }
    text(theme::MICRO).child(s.as_ref().to_uppercase())
}

fn short(path: &std::path::Path) -> String {
    let home = dirs::home_dir().unwrap_or_default();
    match path.strip_prefix(&home) {
        Ok(rest) if !home.as_os_str().is_empty() => format!("~/{}", rest.display()),
        _ => path.display().to_string(),
    }
}

fn megabytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1_000_000.0)
}

// ----- Pages -------------------------------------------------------------------------------------

impl Launcher {
    /// The top bar: the mark and name over the sidebar (in the title bar on Windows and Linux
    /// client-side decorations), the pages with their icons, then the account and Settings.
    /// The top bar over the pages: their tabs, then the account chip and Settings. No plate: the
    /// sky shows through, under a 1 px rule.
    fn topbar(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let n = t.night;
        let mut nav = div()
            .flex()
            .flex_1()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .gap(px(4.));
        for (page, label, _) in Page::in_top_bar() {
            nav = nav.child(
                shell::tab(format!("nav-{label}"), tr(label), self.page == page, &t).on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.go(page);
                        cx.notify();
                    }),
                ),
            );
        }
        // Only a signed-in account says "Microsoft account"; otherwise an offline name, or the
        // one the game made up. A signed-in one shows its face (its skin, else its UUID's default).
        let (name, how) = self.who_plays();
        let face = match self.accounts.player() {
            account::Player::Microsoft(_) => self.accounts.selected().map(|a| a.uuid.clone()),
            _ => None,
        };
        let picture = match face {
            Some(uuid) => (self.profile_heads)
                .draw(Some(&uuid), 28., self.scale, &t)
                .into_any_element(),
            None => div()
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
                .child(blocks::icon("user", 16., shell::c(n.muted)))
                .into_any_element(),
        };
        let account = shell::chip("account", &t)
            .max_w(px(200.))
            .child(div().relative().child(picture))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        shell::text(theme::night::UI, shell::c(n.ink))
                            .font_weight(FontWeight::SEMIBOLD)
                            .line_height(px(16.))
                            .truncate()
                            .child(name),
                    )
                    .child(
                        shell::text(theme::night::CAPTION, shell::c(n.muted))
                            .truncate()
                            .child(how),
                    ),
            )
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if this.closed_by_this_press(event) {
                    return;
                }
                this.account_menu = !this.account_menu;
                if this.account_menu {
                    // Who can play may have changed elsewhere since the window opened.
                    this.accounts = Accounts::load();
                    this.load_profile_heads();
                }
                cx.notify();
            }));
        let account = div()
            .relative()
            .child(account)
            .children(self.account_menu(cx));
        let settings = shell::icon_button(
            "nav-Settings",
            "sliders",
            40.,
            self.page == Page::Settings,
            &t,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
            this.go(Page::Settings);
            cx.notify();
        }));
        // The last native build's log, when there is one (`native_log_page`).
        let build_log = self.native_log.available.then(|| {
            tab_action("topbar-native-log", "Build log", &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.native_log_open();
                    cx.notify();
                },
            ))
        });
        div()
            .flex()
            .flex_none()
            .items_center()
            .min_h(px(theme::TOPBAR))
            .py(px(8.))
            .flex_wrap()
            .min_w_0()
            .pl(px(theme::PAGE_GUTTER - 14.))
            .pr(px(12.))
            .border_b_1()
            .border_color(plate::alpha(n.stroke, n.rule_alpha))
            .child(nav)
            .child(
                div()
                    .ml_auto()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(8.))
                    .children(build_log)
                    .child(account)
                    .child(settings),
            )
    }

    /// The sidebar, from the top of the window: the wordmark, Horizon and the imported modpacks,
    /// the player's servers, then the launcher's own update, the community links and its version.
    fn sidebar(&mut self, window: &Window, cx: &mut Context<Self>) -> Stateful<Div> {
        let t = self.theme;
        let n = t.night;
        self.load_packs();
        let heading = |label: String| shell::heading(label, &t);
        let add = shell::icon_button("add-instance", "plus", 24., false, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.open_instance_dialog(None);
                cx.notify();
            },
        ));
        let items = self.instance_items(heading, cx);
        let mut rows = items.len();
        let list = div()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(heading(tr("INSTANCES").into()).child(add))
            .children(items);
        let servers: Vec<_> = self
            .servers
            .sidebar_list()
            .map(<[_]>::to_vec)
            .unwrap_or_else(|| self.side_servers.clone());
        let mut list = list;
        if !servers.is_empty() {
            list = list.child(heading(tr("SERVERS").into()));
        }
        rows += servers.len();
        let headings = 1 + usize::from(!servers.is_empty());
        for e in servers {
            let up = self.servers.is_up(&e.dir);
            let open = self.page == Page::Servers && self.servers.open.as_ref() == Some(&e.dir);
            let dir = e.dir.clone();
            list = list.child(
                shell::row(format!("side-server-{}", e.dir.display()), open, &t)
                    .child(
                        div()
                            .relative()
                            .flex()
                            .flex_none()
                            .items_center()
                            .justify_center()
                            .size(px(36.))
                            .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
                            .child(blocks::icon("server", 18., shell::c(n.ink_soft))),
                    )
                    .child(shell::row_text(
                        e.name.clone(),
                        tr(if up { "Running" } else { "Stopped" }).into(),
                        Some(shell::c(if up { n.success } else { n.muted })),
                        &t,
                    ))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.open_server(dir.clone());
                        this.go(Page::Servers);
                        cx.notify();
                    })),
            );
        }
        let link = |id: &'static str, label: &'static str, icon: &'static str, url: String| {
            let id = SharedString::from(id);
            track_hover(
                div()
                    .id(id.clone())
                    .relative()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .h(px(32.))
                    .px(px(8.))
                    .cursor_pointer()
                    .child(hover_layer(&id, shell::c(n.card_hi).opacity(0.5), 0., 0.))
                    .child(
                        div()
                            .relative()
                            .child(blocks::icon(icon, 16., shell::c(n.muted))),
                    )
                    .child(
                        shell::text(theme::night::UI, shell::c(n.ink_soft))
                            .relative()
                            .child(label),
                    )
                    .on_click(move |_, _, cx| cx.open_url(&url)),
                id,
            )
        };
        let mut foot = div().mt_auto().flex().flex_col().gap(px(2.)).pt(px(12.));
        let update = |line: String| {
            plate::plate(&n)
                .flex()
                .flex_col()
                .gap(px(8.))
                .p(px(12.))
                .mb(px(6.))
                .child(
                    shell::text(theme::night::UI, shell::c(n.ink))
                        .relative()
                        .child(line),
                )
        };
        match &self.self_update {
            SelfUpdate::Downloading(version, f) => {
                foot = foot.child(
                    update(trf(
                        "Launcher {version}  {percent}%",
                        &[
                            ("version", version),
                            ("percent", &format!("{:.0}", f * 100.)),
                        ],
                    ))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .child(progress_bar(Some((*f, *f)), &t)),
                    ),
                )
            }
            SelfUpdate::Ready(version) => {
                foot = foot.child(
                    update(trf(
                        "Launcher updated to {version}",
                        &[("version", version)],
                    ))
                    .child(div().relative().flex().child(
                        button("restart", "Restart", Kind::Primary, &t).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.self_update = SelfUpdate::Failed(backend::restart());
                                cx.notify();
                            },
                        )),
                    )),
                )
            }
            _ => {}
        }
        // Sculk's card above the links: seen without scrolling, on every page but Servers, which shows
        // its wide banner. Tall where the sidebar has the height for it besides the wordmark, its rows
        // (52 high each, under headings of 40) and the links, compact otherwise. After a while on
        // screen a "Hide" beside its label puts it away for good.
        if !yoake_page::enabled() && self.page != Page::Servers && !self.settings.partner_hidden {
            let free = f32::from(window.viewport_size().height)
                - theme::TOPBAR
                - 40. * headings as f32
                - 52. * rows as f32
                - 110.;
            let mut side = div().relative().mb(px(8.)).child(partner::card(
                "partner-side",
                theme::SIDEBAR - 20.,
                free < partner::TALL_HEIGHT,
                &t,
            ));
            if self.settings.partner_seen_secs >= partner::SEEN_BEFORE_HIDE {
                let hide = partner::hide_button("partner-hide", &t).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.settings.partner_hidden = true;
                        this.save_settings();
                        cx.notify();
                    },
                ));
                side = side.child(hide);
            }
            foot = foot.child(side);
        }
        if !yoake_page::enabled() && !self.links.sponsor.is_empty() {
            foot = foot.child(link(
                "sponsor",
                tr("Sponsor"),
                "heart",
                self.links.sponsor.clone(),
            ));
        }
        if !yoake_page::enabled() && !self.links.discord.is_empty() {
            foot = foot.child(link(
                "discord",
                "Discord",
                "message",
                self.links.discord.clone(),
            ));
        }
        if !yoake_page::enabled() && !self.links.website.is_empty() {
            foot = foot.child(link(
                "website",
                tr("Website"),
                "globe",
                self.links.website.clone(),
            ));
        }
        foot = foot.child(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(8.))
                .px(px(8.))
                .pt(px(6.))
                .child(
                    shell::text(theme::night::CAPTION, shell::c(n.muted))
                        .id("launcher-version")
                        .child(trf("Launcher {version}", &[("version", backend::VERSION)]))
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.version_clicked();
                            cx.notify();
                        })),
                )
                .when(self.settings.dev_unsigned_dll || self.settings.dev_unsigned_wasm, |row| {
                    row.child(
                        shell::text(theme::night::CAPTION, shell::c(n.warn))
                            .child(tr("Developer mode")),
                    )
                }),
        );
        // Custom title bars carry the mark and name already.
        let brand = (!titlebar::draws_brand(window)).then(|| {
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.))
                .h(px(theme::TOPBAR))
                .px(px(8.))
                .child(mark(24., &t))
                .child(
                    shell::text(theme::night::TITLE, shell::c(n.ink))
                        .font_family(theme::DISPLAY_FONT)
                        .child(horizon_launcher::brand::name()),
                )
        });
        shell::sidebar_plate(&t)
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_none()
            .w(px(theme::SIDEBAR))
            .h_full()
            .px(px(10.))
            .pb(px(12.))
            .overflow_y_scroll()
            .children(brand)
            .child(list)
            .child(foot)
    }

    /// Whether `event` is the press that just closed a menu from outside it (on the menu's
    /// own button): the button then leaves the menu closed instead of opening it again.
    fn closed_by_this_press(&mut self, event: &ClickEvent) -> bool {
        closed_by_press(self.menu_closed_at.take(), event)
    }

    fn picker(&self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        // "Nightly · ba6344a", a release's tag: the menu lists the full tags.
        let label = self.selected().map_or_else(
            || tr("No version").into(),
            |tag| {
                horizon_launcher::nightly::label(tag)
                    .or_else(|| build_import::label(tag))
                    .unwrap_or_else(|| tag.clone())
            },
        );
        // As tall as Play: "Version" over the tag, and a chevron.
        let n = t.night;
        let mut wrap = div().relative().child(
            keyboard(div().id("picker"), &t)
                .relative()
                .flex()
                .items_center()
                .gap(px(14.))
                .h(px(theme::night::PLAY))
                .pl(px(14.))
                .pr(px(12.))
                .rounded(px(theme::night::RADIUS_CONTROL))
                .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
                .border_1()
                .border_color(plate::alpha(n.stroke, n.stroke_alpha))
                .cursor_pointer()
                .hover(move |s| s.border_color(plate::alpha(n.stroke, 3. * n.stroke_alpha)))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(
                            shell::text(theme::night::CAPTION, t.muted)
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(tr("Version").to_string()),
                        )
                        .child(shell::text(theme::night::UI, t.ink).child(label)),
                )
                .child(blocks::icon("chevron-down", 20., t.muted))
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    if !this.closed_by_this_press(event) {
                        this.picker_open = !this.picker_open;
                    }
                    cx.notify();
                })),
        );
        if self.picker_open {
            let mut menu = div()
                .absolute()
                .top(px(theme::night::PLAY + 6.))
                .left_0()
                .w(px(340.))
                .p(px(6.))
                .rounded(px(theme::night::RADIUS_CONTROL))
                .border_1()
                .border_color(plate::alpha(n.stroke, 2. * n.stroke_alpha))
                .bg(t.surface)
                .flex()
                .flex_col()
                .on_mouse_down_out(cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.picker_open = false;
                    this.menu_closed_at = Some(event.position);
                    cx.notify();
                }));
            // Picking a version keeps the selected instance on it; the launcher's own version
            // (newest unless chosen otherwise on Versions) follows updates.
            let follows = self.launcher_version().cloned();
            for tag in &self.installed {
                let selected = self.selected() == Some(tag);
                let tag = tag.clone();
                let latest = follows.as_ref() == Some(&tag);
                menu = menu.child(
                    div()
                        .id(SharedString::from(format!("pick-{tag}")))
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .h(px(32.))
                        .px(px(10.))
                        .rounded(px(theme::RADIUS))
                        .font_family(theme::MONO)
                        .text_size(px(theme::CAPTION.px))
                        .text_color(t.ink)
                        .cursor_pointer()
                        .hover(move |s| s.bg(t.plate_soft))
                        .child(div().min_w_0().truncate().child(tag.clone()))
                        .child(
                            div()
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(8.))
                                .when(latest, |d| {
                                    d.child(caps(tr("auto-update")).flex_none().text_color(t.muted))
                                })
                                .when(selected, |d| {
                                    d.child(div().size(px(6.)).rounded_full().bg(t.accent))
                                }),
                        )
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.pin_version((!latest).then(|| tag.clone()));
                            this.picker_open = false;
                            cx.notify();
                        })),
                );
            }
            menu = menu.child(div().my(px(6.)).h(px(1.)).bg(t.rule)).child(
                div()
                    .id("manage")
                    .flex()
                    .items_center()
                    .h(px(32.))
                    .px(px(10.))
                    .rounded(px(theme::RADIUS))
                    .text_size(px(theme::LABEL.px))
                    .text_color(t.muted)
                    .cursor_pointer()
                    .hover(move |s| s.bg(t.plate_soft).text_color(t.ink))
                    .child(tr("Manage versions"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.go(Page::Versions);
                        this.picker_open = false;
                        cx.notify();
                    })),
            );
            wrap = wrap.child(gpui::deferred(menu).with_priority(1));
        }
        wrap
    }

    fn play_page(&mut self, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        // A Java instance plays Mojang's game: no Horizon version, nothing of Horizon's to prepare.
        let java = self.selected_instance().and_then(|i| {
            (!yoake_install::has_manifest_or_pending(&i.home))
                .then(|| i.java.clone())
                .flatten()
        });
        let yoake = yoake_page::enabled()
            && self
                .selected_instance()
                .is_some_and(|instance| yoake_install::has_manifest_or_pending(&instance.home));
        let Some(tag) = self
            .selected_play_tag()
            .or_else(|| java.is_some().then(String::new))
        else {
            // No hero without a version: the page starts below the top bar like the others.
            let glass = GLASS_PAGES;
            let memory = self.memory_row(glass, cx);
            if glass {
                use horizon_theme::glass::ink;
                return div()
                    .flex()
                    .flex_col()
                    .gap(px(28.))
                    .pt(px(36.))
                    .child(glass_page_head(
                        "Play",
                        "Start the game, or run a server for others to join.",
                    ))
                    .child(
                        glass_card().child(
                            div()
                                .relative()
                                .flex()
                                .flex_col()
                                .items_start()
                                .gap(px(16.))
                                .p(px(20.))
                                .child(
                                    glass_ui::text(14., FontWeight::NORMAL, ink::NOTE)
                                        .child(tr("No version installed yet.")),
                                )
                                .child(
                                    glass_ui::solid(
                                        "go-versions",
                                        tr("Choose a version"),
                                        36.,
                                        14.,
                                        &t,
                                    )
                                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.go(Page::Versions);
                                        cx.notify();
                                    })),
                                ),
                        ),
                    )
                    .children(memory);
            }
            return div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .pt(px(28.))
                .child(page_header(
                    "Play",
                    "Start the game, or run a server for others to join.",
                    &t,
                ))
                .child(centered(tr("No version installed yet."), &t).child(
                    button("go-versions", "Choose a version", Kind::Secondary, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.go(Page::Versions);
                            cx.notify();
                        }),
                    ),
                ))
                .children(memory);
        };
        let prepared = backend::is_prepared(&tag);
        // Prepared without sounds (#1035): Prepare again is the fix, so it stands out.
        let silent = prepared && !backend::has_sounds(&tag);
        let version = java
            .as_ref()
            .map_or_else(|| backend::game_version(&tag), |j| j.minecraft.clone());
        let left = self.download_left();
        let steps = self.preparing.then(|| self.steps(&version, left));

        // Not prepared: an owner can have Horizon download Minecraft from Mojang; without an
        // owner there is nothing to play, so signing in comes first.
        let first = (!prepared && !self.preparing)
            .then_some(self.owner_shown())
            .flatten();
        // No launcher folder with Minecraft on this computer: an owner needs none.
        let no_install = self.settings.minecraft_dir.is_none();
        let resources_value = match first {
            _ if self.preparing => tr("Preparing…").to_string(),
            Some(true) if no_install => trf(
                "Not prepared. Play downloads Minecraft {version} from Mojang for your account and prepares it.",
                &[("version", &version)],
            ),
            Some(true) => trf(
                "Not prepared. Horizon can download Minecraft {version} from Mojang for your account, or use your own install.",
                &[("version", &version)],
            ),
            Some(false) => trf(
                "Not prepared. Sign in with the Microsoft account that owns Minecraft {version} first.",
                &[("version", &version)],
            ),
            None => backend::resources_summary(prepared, !silent, &version),
        };
        let mut prepare = button(
            "prepare",
            if prepared { "Prepare again" } else { "Prepare" },
            if prepared && !silent {
                Kind::Ghost
            } else {
                Kind::Secondary
            },
            &t,
        );
        if self.preparing {
            prepare = disabled(prepare);
        } else {
            prepare = prepare.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.prepare(backend::MinecraftFrom::Install);
                cx.notify();
            }));
        }
        let prepare = match first {
            Some(true) if no_install => div().child(
                button(
                    "prepare-download",
                    "Download and prepare",
                    Kind::Secondary,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.prepare(backend::MinecraftFrom::Mojang);
                    cx.notify();
                })),
            ),
            Some(true) => div()
                .flex()
                .gap(px(8.))
                .child(
                    button("prepare-install", "From my install", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.prepare(backend::MinecraftFrom::Install);
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    button(
                        "prepare-download",
                        "Download and prepare",
                        Kind::Secondary,
                        &t,
                    )
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.prepare(backend::MinecraftFrom::Mojang);
                        cx.notify();
                    })),
                ),
            Some(false) => div().child(microsoft::sign_in_button("prepare-sign-in", &t).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.sign_in_to_play();
                    cx.notify();
                }),
            )),
            None => div().child(prepare),
        };
        let owner = self.owner == Some(true);
        let minecraft = self.settings.minecraft_dir.as_deref().map_or_else(
            || {
                div()
                    .child(if owner {
                        tr("None needed: Horizon downloads Minecraft from Mojang for your account.")
                    } else {
                        tr("Not found. Choose the folder where your launcher keeps Minecraft.")
                    })
                    .into_any_element()
            },
            |p| {
                div()
                    .font_family(theme::MONO)
                    .child(short(p))
                    .into_any_element()
            },
        );
        let mut rows = div()
            .flex()
            .flex_col()
            // The glass look's sidebar ⋮ menu holds what the Instance row offers.
            .children((!GLASS_PAGES).then(|| self.instance_row(1, cx)).flatten())
            .children(self.export_panel(cx))
            .children(self.launch_panel(cx))
            .children(self.memory_row(false, cx));
        rows = match &java {
            Some(java) => rows.child(index_row(
                2,
                tr("Game"),
                Some(
                    div()
                        .child(trf(
                            "Minecraft {version} (Java Edition). Play downloads it from Mojang for your account the first time, with the Java it needs.",
                            &[("version", &java.minecraft)],
                        ))
                        .into_any_element(),
                ),
                None,
                &t,
            )),
            None => rows
                .child(index_row(
                    2,
                    tr("Resources"),
                    Some(div().child(resources_value).into_any_element()),
                    Some(prepare.into_any_element()),
                    &t,
                ))
                .child(index_row(
                3,
                tr("Minecraft folder"),
                Some(minecraft),
                Some(
                    button("choose-mc", "Choose…", Kind::Ghost, &t)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.choose_folder(cx)))
                        .into_any_element(),
                ),
                &t,
            )),
        };
        {
            // The top bar shows who plays; a row only when there is something to do about it:
            // no account that owns the game (Play needs one), or offline.
            if self.owner_shown() == Some(false) {
                rows = rows.child(index_row(
                    5,
                    tr("Microsoft account"),
                    Some(
                        div()
                            .child(format!("{}.", tr(account::NEEDS_OWNER)))
                            .into_any_element(),
                    ),
                    Some(
                        microsoft::sign_in_button("gate-sign-in", &t)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.sign_in_to_play();
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                    &t,
                ));
            } else if self.accounts.selected().is_none() && account::client_id().is_some() {
                rows = rows.child(index_row(
                    5,
                    tr("Playing offline"),
                    Some(
                        div()
                            .child(tr("Online servers need a Microsoft account."))
                            .into_any_element(),
                    ),
                    Some(
                        button("accounts", "Accounts…", Kind::Ghost, &t)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.go(Page::Settings);
                                cx.notify();
                            }))
                            .into_any_element(),
                    ),
                    &t,
                ));
            }
        }

        // The glass look's head (the mockup's `HomeView`) replaces the banner; the preparation's
        // steps stay right under it, where the player looks after pressing Play.
        let banner = if GLASS_PAGES {
            div()
                .flex()
                .flex_col()
                .gap(px(24.))
                .child(self.glass_home_head(&tag, prepared || yoake, cx))
                .children(steps)
        } else {
            let hero = match &java {
                // Its own game: the Minecraft version and the loader, never Horizon's.
                Some(j) => {
                    let game = match &j.loader {
                        Some(l) => format!("{version} · {} {}", l.kind.name(), l.version),
                        None => version.clone(),
                    };
                    self.hero("", &game, true, cx)
                }
                None => self.hero(&tag, &version, prepared, cx),
            };
            let actions =
                self.home_actions(prepared || java.is_some() || yoake, java.is_some(), cx);
            // The banner: the hero and the Play row over the sky (`sky.rs`), or over the player's
            // own picture under a veil of the page colour. It reaches the content's edges and the top
            // bar.
            let veil = |angle: f32, from: f32, to: f32, at: f32| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(gpui::linear_gradient(
                        angle,
                        gpui::linear_color_stop(t.page.opacity(from), 0.),
                        gpui::linear_color_stop(t.page.opacity(to), at),
                    ))
            };
            let picture = self.home_background().map(|path| {
                [
                    gpui::img(path)
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .object_fit(gpui::ObjectFit::Cover)
                        .into_any_element(),
                    veil(90., 0.82, 0., 0.62).into_any_element(),
                    veil(180., 0., 0.75, 1.).into_any_element(),
                ]
            });
            div()
                .relative()
                .flex()
                .flex_col()
                .justify_end()
                .min_h(px(340.))
                .mx(px(-theme::PAGE_GUTTER))
                .px(px(theme::PAGE_GUTTER))
                .pb(px(28.))
                .gap(px(24.))
                .overflow_hidden()
                .children(picture.into_iter().flatten())
                .child(
                    div()
                        .relative()
                        .flex()
                        .items_end()
                        .gap(px(32.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(28.))
                                .child(hero)
                                .child(actions)
                                // The preparation's steps right under Play, where the player looks
                                // after pressing it, not below the fold (#666).
                                .children(steps),
                        )
                        // Notes and friends are Horizon's.
                        .children(java.is_none().then(|| self.home_card(&tag, cx))),
                )
        };
        let mut page = div().flex().flex_col().gap(px(28.)).child(banner);
        if let Some(e) = self.prepare_error.clone() {
            page = page.child(notice(e, &t));
        }
        page = page.child(if GLASS_PAGES {
            self.glass_home_tabs(rows, cx)
        } else {
            self.home_tabs(rows, cx)
        });
        if let Some(e) = self.client_error.clone() {
            page = page.child(notice(e, &t));
        }
        if let Some(crash) = self.crash.clone() {
            page = page.child(self.crash_notice(crash, cx));
        }
        page
    }

    /// "Horizon crashed": the report and the logs, and where to report it.
    fn crash_notice(&self, crash: Crash, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let mut actions = div().flex().flex_wrap().gap(px(8.)).mt(px(8.));
        if let Some(report) = crash.report.clone() {
            let open = report.clone();
            actions = actions
                .child(
                    button("crash-open", "Open report", Kind::Secondary, &t)
                        .bg(t.plate)
                        .on_click(move |_, _, cx| cx.open_with_system(&open)),
                )
                .child(
                    button(
                        "crash-copy",
                        if crash.copied {
                            "Copied"
                        } else {
                            "Copy report"
                        },
                        Kind::Secondary,
                        &t,
                    )
                    .bg(t.plate)
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| {
                            if let Ok(text) = std::fs::read_to_string(&report) {
                                copy_to_clipboard(cx, gpui::ClipboardItem::new_string(text));
                                if let Some(c) = &mut this.crash {
                                    c.copied = true;
                                }
                            }
                            cx.notify();
                        },
                    )),
                );
        }
        actions = actions.child(
            button("crash-logs", "Open logs folder", Kind::Secondary, &t)
                .bg(t.plate)
                .on_click(|_, _, cx| {
                    let dir = backend::logs_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    cx.open_with_system(&dir);
                }),
        );
        let discord = self.links.discord.clone();
        if !yoake_page::enabled() && !discord.is_empty() {
            actions = actions.child(
                button("crash-discord", "Report on Discord", Kind::Secondary, &t)
                    .bg(t.plate)
                    .on_click(move |_, _, cx| cx.open_url(&discord)),
            );
        }
        actions = actions.child(
            button("crash-dismiss", "Dismiss", Kind::Ghost, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.crash = None;
                    cx.notify();
                },
            )),
        );
        let help = tr(if crash.report.is_some() {
            "When you report it on Discord, attach the crash report file (or paste it with Copy report) and say what you were doing."
        } else {
            "The game ended without a crash report. When you report it on Discord, attach latest.log from the logs folder and say what you were doing."
        });
        let mut body = div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                text(theme::HEADING)
                    .text_color(t.ink)
                    .child(tr("Horizon crashed")),
            );
        if !crash.summary.is_empty() {
            body = body.child(div().text_color(t.ink_soft).child(crash.summary));
        }
        if let Some(report) = &crash.report {
            body = body.child(
                text(theme::MICRO)
                    .text_color(t.muted)
                    .truncate()
                    .child(short(report)),
            );
        }
        body = body.child(actions).child(
            text(theme::CAPTION)
                .mt(px(4.))
                .text_color(t.muted)
                .child(help),
        );
        plate(&t)
            .flex()
            .gap(px(14.))
            .p(px(24.))
            .text_size(px(theme::UI.px))
            .line_height(px(21.))
            .text_color(t.ink)
            .child(
                div()
                    .flex_none()
                    .mt(px(11.))
                    .size(px(6.))
                    .rounded_full()
                    .bg(t.danger),
            )
            .child(body)
    }

    fn check_yoake_updates(&mut self) {
        if let Some(task) = self.yoake_update_task.take() {
            task.cancel();
        }
        self.yoake_update_states.clear();
        if let Ok(Some(client)) = yoake_feed::FeedClient::configured() {
            let homes = instances::list()
                .into_iter()
                .map(|instance| instance.home)
                .collect();
            self.yoake_update_task = Some(yoake_update::start_update_check(client, homes));
        }
    }

    fn settings_page(&mut self, cx: &mut Context<Self>) -> Div {
        // Rows, buttons and switches built below take their glass form while this is held.
        let _mode = GlassMode::enter(GLASS_PAGES);
        let t = self.theme;
        let toggle = switch("auto-install", self.settings.auto_install(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.install_new_versions = Some(!this.settings.auto_install());
                this.save_settings();
                if this.settings.auto_install() {
                    this.install_update();
                }
                cx.notify();
            }),
        );
        let prune = switch("auto-prune", self.settings.auto_prune(), &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.prune_old_versions = Some(!this.settings.auto_prune());
                this.save_settings();
                cx.notify();
            },
        ));
        let opening = switch("opening", !self.settings.skip_opening, &t).on_click(cx.listener(
            |this, _: &ClickEvent, _, cx| {
                this.settings.skip_opening = !this.settings.skip_opening;
                this.save_settings();
                cx.notify();
            },
        ));
        // Linux: the launcher's entry in the applications menu (`horizon_launcher::desktop`).
        let menu_entry = switch("desktop-entry", self.settings.desktop_entry(), &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                let on = !this.settings.desktop_entry();
                this.settings.desktop_entry = Some(on);
                this.save_settings();
                let _ = if on {
                    horizon_launcher::desktop::install(backend::exe_path()).map(|_| ())
                } else {
                    horizon_launcher::desktop::remove()
                };
                cx.notify();
            }),
        );
        let horizon = match (&self.updates.horizon, &self.update_error) {
            (Some(r), _) => trf("{version} available", &[("version", update_name(r))]),
            (None, Some(e)) => e.clone(),
            (None, None) if self.checked.is_some() => tr("Up to date").to_string(),
            _ => tr("Checking…").to_string(),
        };
        let horizon_action = self
            .updates
            .horizon
            .as_ref()
            .filter(|_| self.installing.is_none())
            .map(|_| {
                button("settings-install", "Install", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.install_update();
                        cx.notify();
                    }))
                    .into_any_element()
            });
        let launcher = match &self.self_update {
            SelfUpdate::Downloading(v, p) => {
                let done = format!("{:.0}", p * 100.);
                let said = trf(
                    "downloading {version}, {percent}%",
                    &[("version", v), ("percent", &done)],
                );
                format!("{}  ·  {said}", backend::VERSION)
            }
            SelfUpdate::Ready(v) => trf(
                "Updated to {version}. Restart the launcher to use it.",
                &[("version", v)],
            ),
            SelfUpdate::Failed(e) => format!(
                "{}  ·  {}",
                backend::VERSION,
                trf("the update failed: {error}", &[("error", e)])
            ),
            SelfUpdate::Idle => match &self.updates.launcher {
                Some(r) => format!(
                    "{}  ·  {}",
                    backend::VERSION,
                    trf(
                        "{version} available",
                        &[(
                            "version",
                            backend::launcher_version(&r.tag).unwrap_or_default()
                        )]
                    )
                ),
                None if self.checked.is_some() && self.update_error.is_none() => {
                    format!("{}  ·  {}", backend::VERSION, tr("up to date"))
                }
                None => backend::VERSION.to_string(),
            },
        };
        let launcher = if horizon_launcher::brand::store_distribution() {
            tr("The Microsoft Store updates Yoake.").to_string()
        } else {
            launcher
        };
        let launcher_action = match &self.self_update {
            SelfUpdate::Ready(_) => Some(
                button("restart-settings", "Restart", Kind::Secondary, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.self_update = SelfUpdate::Failed(backend::restart());
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
            SelfUpdate::Downloading(..) => None,
            _ => Some(
                button("check", "Check now", Kind::Ghost, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.check_updates();
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
        };
        let launcher_action =
            launcher_action.filter(|_| !horizon_launcher::brand::store_distribution());
        let mono = |s: String| div().font_family(theme::MONO).child(s).into_any_element();
        let plain = |s: &str| div().child(tr(s).to_string()).into_any_element();
        let (accounts, n) = self.account_rows(cx);
        let (offline, n) = self.offline_rows(n, cx);
        let appearance = segments(
            "appearance",
            &[
                (backend::Appearance::Dark, "Dark"),
                (backend::Appearance::Light, "Light"),
                (backend::Appearance::System, "System"),
            ],
            self.settings.appearance,
            &t,
            {
                let this = cx.entity().downgrade();
                move |choice, window, cx| {
                    let _ = this.update(cx, |this, cx| {
                        this.settings.appearance = choice;
                        this.save_settings();
                        titlebar::apply_appearance(window, choice);
                        this.theme = this.settings_theme(window);
                        cx.notify();
                    });
                }
            },
        );
        // One glass panel per group of rows, under its title.
        let group = |label: &'static str, icon: &'static str, rows: Vec<AnyElement>| {
            glass_section(label, icon, None, glass_body().children(rows))
        };
        let account = group(
            "Account",
            "user",
            vec![accounts.into_any_element(), offline.into_any_element()],
        );
        let updates = group(
            "Updates",
            "refresh",
            vec![
                index_row(
                    n + 1,
                    tr("Install new versions automatically"),
                    Some(plain(
                        "Your selected version only changes if you were playing the newest one.",
                    )),
                    Some(toggle.into_any_element()),
                    &t,
                )
                .into_any_element(),
                index_row(
                    n + 2,
                    tr("Delete old versions automatically"),
                    Some(
                        div()
                            .child(trf(
                                "Keeps the {count} newest, the one in use and the ones your instances use. Deleted versions install again from the Versions page.",
                                &[("count", &self.settings.keep_newest().to_string())],
                            ))
                            .into_any_element(),
                    ),
                    Some(prune.into_any_element()),
                    &t,
                )
                .into_any_element(),
            ],
        );
        let launcher_group = group(
            "Launcher",
            "computer",
            vec![
                index_row(
                    n + 3,
                    tr("Appearance"),
                    Some(plain(
                        "Dark by default. System follows your computer's light or dark.",
                    )),
                    Some(appearance.into_any_element()),
                    &t,
                )
                .into_any_element(),
                self.accent_row(cx).into_any_element(),
                self.home_background_row(cx).into_any_element(),
                self.language_row(cx).into_any_element(),
                self.while_playing_row(n + 3, cx).into_any_element(),
                index_row(
                    n + 3,
                    tr("Show the opening"),
                    Some(plain("The wordmark, while the release list loads.")),
                    Some(opening.into_any_element()),
                    &t,
                )
                .into_any_element(),
            ],
        );
        let network = group(
            "Network",
            "globe-02",
            vec![self.proxy_row(n + 4, cx).into_any_element()],
        );
        let join_sync = switch("join-mod-sync", self.settings.join_mod_sync, &t).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| {
                this.settings.join_mod_sync = !this.settings.join_mod_sync;
                this.save_settings();
                cx.notify();
            }),
        );
        let game = group(
            "Game",
            "game-controller-01",
            vec![
                self.game_args_row(n + 5, cx).into_any_element(),
                index_row(
                    n + 5,
                    tr("Get missing mods when joining"),
                    Some(plain(
                        "When a host has mods you lack, Yoake fetches them, installs them for that host only and joins again. This includes full-access mods, which get the game's own power: turn it off if you join hosts you do not trust.",
                    )),
                    Some(join_sync.into_any_element()),
                    &t,
                )
                .into_any_element(),
            ],
        );
        let ai = group(
            "AI",
            "computer-terminal-01",
            vec![self.no_ai_row(n + 6, cx).into_any_element()],
        );
        let mut about = vec![
            index_row(
                n + 7,
                horizon_launcher::brand::name(),
                Some(plain(&horizon)),
                horizon_action,
                &t,
            )
            .into_any_element(),
            index_row(
                n + 8,
                tr("Launcher"),
                Some(plain(&launcher)),
                launcher_action,
                &t,
            )
            .into_any_element(),
            index_row(
                n + 9,
                tr("Data folder"),
                Some(mono(short(&backend::data_dir()))),
                None,
                &t,
            )
            .into_any_element(),
            index_row(
                n + 10,
                tr("Logs"),
                Some(mono(format!(
                    "{}  ·  {}",
                    short(&backend::logs_dir()),
                    tr("crash reports beside it")
                ))),
                Some(
                    button("open-logs", "Open", Kind::Secondary, &t)
                        .on_click(|_, _, cx| {
                            let dir = backend::logs_dir();
                            let _ = std::fs::create_dir_all(&dir);
                            cx.open_with_system(&dir);
                        })
                        .into_any_element(),
                ),
                &t,
            )
            .into_any_element(),
        ];
        if cfg!(target_os = "linux") {
            about.push(
                index_row(
                    n + 11,
                    tr("Applications menu"),
                    Some(plain(if self.settings.desktop_entry() {
                        "Horizon and its icon are in your desktop's menu."
                    } else {
                        "Not in your desktop's menu."
                    })),
                    Some(menu_entry.into_any_element()),
                    &t,
                )
                .into_any_element(),
            );
        }
        let about = group("About", "information-circle", about);
        div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .pt(px(36.))
            .child(glass_page_head(
                "Settings",
                "Accounts, updates, and the launcher itself.",
            ))
            .child(account)
            .child(updates)
            .child(launcher_group)
            .child(network)
            .child(game)
            .child(ai)
            .child(about)
            .when(yoake_page::enabled(), |page| {
                use horizon_theme::glass::ink;
                page.child(glass_section(
                    "About / licences",
                    "information-circle",
                    None,
                    glass_body()
                        .gap(px(12.))
                        .p(px(16.))
                        .child(
                            glass_ui::text(14., FontWeight::NORMAL, ink::FULL)
                                .relative()
                                .child(tr("Yoake Launcher is built on Horizon (licence below)")),
                        )
                        .when(!horizon_launcher::brand::store_distribution(), |card| {
                            card.child(
                                button(
                                    "yoake-check-launcher",
                                    "Check for launcher updates",
                                    Kind::Secondary,
                                    &t,
                                )
                                .relative()
                                .self_start()
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.yoake.check_launcher_update(true);
                                    this.go(Page::Yoake);
                                    cx.notify();
                                })),
                            )
                        })
                        .child(
                            glass_ui::text(13., FontWeight::NORMAL, ink::NOTE)
                                .relative()
                                .child(include_str!("../../LICENSE")),
                        ),
                ))
            })
            .when(!horizon_launcher::brand::yoake(), |view| {
                view.child(self.collection_section(cx))
            })
            .children(self.agents_section(cx))
            .children(self.dev_options_section(cx))
    }

    /// The warning before a native build: what the mods can do and which mods. The build starts
    /// only from its button.
    fn native_warning(
        &self,
        mods: &Result<Vec<native_mods::Bundle>, String>,
        cx: &mut Context<Self>,
    ) -> Div {
        let t = self.theme;
        let list = match mods {
            Ok(found) => found
                .iter()
                .map(|b| format!("{} {}", b.id, b.version))
                .collect::<Vec<_>>()
                .join(", "),
            Err(e) => e.clone(),
        };
        let panel = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mt(px(8.))
            .p(px(12.))
            .bg(t.well)
            .child(div().text_color(t.danger).child(tr("These mods will run with full access to your computer").to_string()))
            .child(tr("They can read, change or delete your files, and use your accounts and network, like any program you run. Build them only if you trust their authors: the Mods page names them.").to_string())
            .child(trf("Mods: {mods}", &[("mods", &list)]))
            .child(tr("The build compiles the game: 20 to 30 minutes and a few GB of disk the first time (it downloads Rust), a few minutes after that.").to_string())
            .children(self.native_update.as_ref().map(|newer| {
                div().text_color(t.warn).child(trf(
                    "This moves the instance to Horizon {version}. It compiles the game from scratch, so it takes longer than a normal rebuild: up to 30 minutes.",
                    &[("version", &native_status::horizon_label(newer))],
                ))
            }));
        let mut buttons = div().flex().gap(px(8.)).mt(px(4.));
        if mods.is_ok() {
            buttons = buttons.child(
                button(
                    "native-trust",
                    "I trust these mods: build",
                    Kind::Primary,
                    &t,
                )
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.build_native(false);
                    cx.notify();
                })),
            );
        }
        panel.child(buttons.child(
            button("native-cancel", "Cancel", Kind::Secondary, &t).on_click(cx.listener(
                |this, _: &ClickEvent, _, cx| {
                    this.native_confirm = None;
                    this.native_update = None;
                    cx.notify();
                },
            )),
        ))
    }

    /// Mods made for other Horizon versions: which, and the version the build uses. They often
    /// still work, so the player may build anyway.
    fn native_mismatch_panel(&self, list: &str, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .mt(px(8.))
            .p(px(12.))
            .bg(t.well)
            .child(div().text_color(t.danger).child(tr("These mods were made for different Horizon versions").to_string()))
            .child(list.to_owned())
            .child(tr("They often still work together. If the build fails, install versions made for the same Nightly.").to_string())
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .mt(px(4.))
                    .child(
                        button("native-anyway", "Build anyway", Kind::Primary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.build_native(true);
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        button("native-mismatch-cancel", "Cancel", Kind::Secondary, &t).on_click(
                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.native_mismatch = None;
                                cx.notify();
                            }),
                        ),
                    ),
            )
    }

    /// Settings' Game arguments row: the extra arguments the game starts with, kept in
    /// `settings.json` and appended to the launcher's own (quotes group a word with spaces).
    fn game_args_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.args_focused;
        let field = div()
            .id("game-args")
            .track_focus(&self.args_focus)
            .flex()
            .items_center()
            .w(px(300.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.args_input,
                &t,
                focused,
                false,
                false,
                tr("For example --server-thread"),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.args_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.args_input, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => window.blur(cx),
                    line_edit::Key::Edited => {
                        this.settings.game_args = this.args_input.text().to_string();
                        this.save_settings();
                    }
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.args_input);
        index_row(
            n,
            tr("Game arguments"),
            Some(
                div()
                    .child(tr("Extra arguments for the game, passed through as-is (quotes group a word with spaces). Applies the next time the game starts."))
                    .into_any_element(),
            ),
            Some(field.into_any_element()),
            &t,
        )
    }

    /// Settings' Proxy row: the HTTP proxy every download and sign-in goes through, kept in
    /// `settings.json` and read back by `proxy.rs` on each request.
    fn proxy_row(&self, n: usize, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let focused = self.proxy_focused;
        let field = div()
            .id("proxy")
            .track_focus(&self.proxy_focus)
            .flex()
            .items_center()
            .w(px(300.))
            .h(px(30.))
            .px(px(12.))
            .overflow_hidden()
            .border_2()
            .border_color(if focused { t.accent } else { t.outline })
            .bg(t.well)
            .font_family(theme::MONO)
            .text_size(px(12.))
            .cursor_text()
            .child(line_edit::content(
                &self.proxy_input,
                &t,
                focused,
                false,
                false,
                tr("For example http://127.0.0.1:7890"),
            ))
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                window.focus(&this.proxy_focus, cx);
                cx.notify();
            }))
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                match line_edit::handle(&mut this.proxy_input, ev, cx) {
                    line_edit::Key::Pass => return,
                    line_edit::Key::Escape => window.blur(cx),
                    line_edit::Key::Edited => this.save_proxy(),
                    _ => {}
                }
                cx.stop_propagation();
                cx.notify();
            }));
        let field = line_edit::mouse(field, cx, |this| &mut this.proxy_input);
        // A half-typed address is no reason to complain; one left behind is.
        let detail = match (
            focused,
            horizon_launcher::proxy::refuse(self.proxy_input.text()),
        ) {
            (false, Some(why)) => div().text_color(t.danger).child(why).into_any_element(),
            _ => div()
                .child(tr(
                    "Downloads, sign-ins and updates go through this HTTP proxy \
                     (http://host:port). Empty means none: HTTPS_PROXY is used instead when the \
                     launcher was started with one.",
                ))
                .into_any_element(),
        };
        index_row(
            n,
            tr("Proxy"),
            Some(detail),
            Some(field.into_any_element()),
            &t,
        )
    }

    /// Keeps the Proxy field in `settings.json`, and tells the parts that build their own agents
    /// (`horizon-port-pack`) which proxy to use.
    fn save_proxy(&mut self) {
        let value = self.proxy_input.text().trim().to_owned();
        self.settings.proxy = (!value.is_empty()).then_some(value);
        self.save_settings();
        horizon_launcher::proxy::announce(self.settings.proxy.clone());
    }

    /// Settings' Microsoft accounts: who is signed in and plays, the sign-in with its code. The
    /// rows are numbered from 01; returns them and how many there are.
    fn account_rows(&mut self, cx: &mut Context<Self>) -> (Div, usize) {
        let t = self.theme;
        let mut rows = div().flex().flex_col();
        let mut n = 0;
        let selected = self.accounts.selected.clone();
        for (i, a) in self.accounts.accounts.clone().into_iter().enumerate() {
            let plays = selected.as_ref() == Some(&a.uuid);
            let uuid = a.uuid.clone();
            let mut actions = div().flex().gap(px(4.));
            if plays {
                actions = actions.child(
                    button(format!("offline-{i}"), "Play offline", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            let mut accounts = Accounts::load();
                            accounts.selected = None;
                            if accounts.save().is_ok() {
                                this.accounts = accounts;
                            }
                            cx.notify();
                        }),
                    ),
                );
            } else {
                let uuid = uuid.clone();
                actions = actions.child(
                    button(format!("select-{i}"), "Play as", Kind::Secondary, &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.play_as(account::Player::Microsoft(uuid.clone()));
                            cx.notify();
                        }),
                    ),
                );
            }
            let sign_in_progress = self
                .sign_in_target
                .as_ref()
                .filter(|a| a.uuid == uuid)
                .and_then(|_| match self.sign_in_state.as_ref() {
                    Some(SignInState::Starting) => Some("Asking Microsoft…"),
                    Some(SignInState::Code(_)) => Some("Waiting for browser sign-in…"),
                    Some(SignInState::Working(phase)) => Some(phase.label()),
                    _ => None,
                });
            let progress = sign_in_progress
                .or_else(|| self.auth_progress.get(&uuid).map(|phase| phase.label()));
            let busy = self.auth_busy.contains_key(&uuid) || sign_in_progress.is_some();
            let status = self.auth_status.get(&uuid);
            let needs_sign_in =
                status.is_some_and(|s| s.state == account::session::State::SignInRequired);
            let refresh_id = uuid.clone();
            let refresh = button(
                format!("refresh-{i}"),
                if busy {
                    "Checking…"
                } else if needs_sign_in {
                    "Sign in again"
                } else {
                    "Refresh"
                },
                Kind::Ghost,
                &t,
            );
            actions = actions.child(if busy {
                disabled(refresh)
            } else {
                refresh.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if needs_sign_in {
                        this.re_sign_in(&refresh_id);
                    } else {
                        this.refresh_account(refresh_id.clone());
                    }
                    cx.notify();
                }))
            });
            let sign_out = button(format!("sign-out-{i}"), "Sign out", Kind::Ghost, &t);
            actions = actions.child(if busy {
                disabled(sign_out)
            } else {
                sign_out.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    let Some(generation) = this.begin_auth_job(&uuid) else {
                        return;
                    };
                    let (tx, uuid) = (this.tx.clone(), uuid.clone());
                    std::thread::spawn(move || {
                        let mut accounts = Accounts::load();
                        let _ = tx.send(Msg::AccountProgress(
                            uuid.clone(),
                            generation,
                            account::session::Phase::SigningOut,
                        ));
                        let result = accounts
                            .remove(account::store(), &uuid)
                            .and_then(|()| accounts.save())
                            .map(|()| accounts.has_owner(account::store()));
                        let _ = tx.send(Msg::AccountRemoved(uuid, generation, result));
                    });
                    cx.notify();
                }))
            });
            n += 1;
            rows = rows.child(slot_row(
                Some(self.profile_heads.draw(Some(&a.uuid), 32., self.scale, &t)),
                a.name.clone(),
                Some(
                    div()
                        .child(tr(if plays {
                            "Microsoft account · plays"
                        } else {
                            "Microsoft account"
                        }))
                        .child(account_status_line(status, progress, &t))
                        .when(!busy, |row| {
                            row.when_some(status.filter(|s| s.message != s.label()), |row, s| {
                                row.child(div().child(tr(&s.message).to_string()))
                            })
                        })
                        .into_any_element(),
                ),
                Some(actions.into_any_element()),
                &t,
            ));
        }
        let configured = account::client_id().is_some();
        let busy = matches!(
            self.sign_in_state,
            Some(SignInState::Starting | SignInState::Code(_) | SignInState::Working(_))
        );
        let detail = match &self.sign_in_state {
            _ if !configured => div().child(tr(account::NOT_CONFIGURED)),
            Some(SignInState::Starting) => div().child(tr("Asking Microsoft…")),
            Some(SignInState::Working(phase)) => div().child(tr(phase.label())),
            Some(SignInState::Code(code)) => div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(trf(
                    "Open {link} in your browser and enter this code:",
                    &[("link", &code.verification_uri)],
                ))
                .child(
                    glass_ui::text(22., FontWeight::SEMIBOLD, horizon_theme::glass::ink::FULL)
                        .font_family(theme::MONO)
                        .child(code.user_code.clone()),
                ),
            Some(SignInState::Done(Ok(a))) => {
                div().child(trf("Signed in as {name}.", &[("name", &a.name)]))
            }
            Some(SignInState::Done(Err(e))) => div()
                .text_color(glass_ui::color(horizon_theme::glass::DANGER))
                .child(e.clone()),
            None if self.accounts.accounts.is_empty() => div().child(tr(
                "Horizon needs a Microsoft account that owns Minecraft: Java Edition, like other launchers. It also plays online servers with your name and skin.",
            )),
            None => div().child(tr("Add another account.")),
        };
        let detail = if let Some(target) = &self.sign_in_target {
            div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(trf(
                    "Use the Microsoft account for {name}.",
                    &[("name", &target.name)],
                ))
                .child(detail)
        } else {
            detail
        };
        let detail = match self.keychain.clone() {
            Some(said) if !self.accounts.accounts.is_empty() => div()
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(detail)
                .child(div().child(said)),
            _ => detail,
        };
        let action = if !configured {
            None
        } else if let Some(SignInState::Code(code)) = &self.sign_in_state {
            let (uri, user_code) = (code.verification_uri.clone(), code.user_code.clone());
            Some(
                div()
                    .flex()
                    .gap(px(4.))
                    .child(
                        button("sign-in-copy", "Copy code", Kind::Secondary, &t).on_click(
                            move |_, _, cx| {
                                copy_to_clipboard(
                                    cx,
                                    gpui::ClipboardItem::new_string(user_code.clone()),
                                )
                            },
                        ),
                    )
                    .child(
                        button("sign-in-open", "Open link", Kind::Secondary, &t)
                            .on_click(move |_, _, cx| cx.open_url(&uri)),
                    )
                    .child(
                        button("sign-in-cancel", "Cancel", Kind::Ghost, &t).on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.cancel_sign_in();
                                cx.notify();
                            },
                        )),
                    )
                    .into_any_element(),
            )
        } else if busy {
            Some(
                button("sign-in-cancel-working", "Cancel", Kind::Ghost, &t)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.cancel_sign_in();
                        cx.notify();
                    }))
                    .into_any_element(),
            )
        } else {
            let target = self.sign_in_target.as_ref().map(|a| a.uuid.clone());
            let targeted = target.is_some();
            // Re-signing one account names it; adding one is Microsoft's own button.
            let sign_in = if targeted {
                button("sign-in", "Sign in again", Kind::Secondary, &t)
            } else {
                microsoft::sign_in_button("sign-in", &t)
            };
            let mut actions = div().flex().gap(px(4.)).child(sign_in.on_click(cx.listener(
                move |this, _: &ClickEvent, _, cx| {
                    if let Some(uuid) = &target {
                        this.re_sign_in(uuid);
                    } else {
                        this.start_sign_in();
                    }
                    cx.notify();
                },
            )));
            if targeted {
                actions = actions.child(
                    button("sign-in-cancel-target", "Cancel", Kind::Ghost, &t).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.cancel_sign_in();
                            cx.notify();
                        }),
                    ),
                );
            }
            Some(actions.into_any_element())
        };
        n += 1;
        let title = match &self.sign_in_target {
            Some(target) => trf("Sign in again as {name}", &[("name", &target.name)]),
            None if self.accounts.accounts.is_empty() => tr("Microsoft account").to_owned(),
            None => tr("Another account").to_owned(),
        };
        rows = rows.child(index_row(
            n,
            title,
            Some(detail.into_any_element()),
            action,
            &t,
        ));
        (rows, n)
    }

    /// Join a friend: a code field and Join, which starts the game straight
    /// into the friend's world (errors show in the game).
    /// Plays the selected instance straight into a world (`--world`) or a server (`--server`):
    /// flags every Horizon release takes.
    pub(crate) fn quick_play(&mut self, flag: &'static str, value: String) {
        self.quick_play = Some((flag, value));
        self.play();
        if self.starting_client.is_none()
            && !(yoake_page::enabled() && self.yoake_base_pending_play())
        {
            self.quick_play = None;
        }
    }

    /// Asks for the hand-off file at `path` to go to the game that starts next; `hosting` is
    /// the room that game opens.
    pub(crate) fn set_social_handoff(&mut self, path: std::path::PathBuf, hosting: Option<String>) {
        self.drop_social_handoff();
        self.social_handoff = Some(SocialHandoff { path, asked: Instant::now(), hosting });
    }

    /// The game will not read the pending hand-off file: delete it, and close the room the
    /// game was to host.
    pub(crate) fn drop_social_handoff(&mut self) {
        if let Some(handoff) = self.social_handoff.take() {
            self.fail_social_handoff(Some((handoff.path, handoff.hosting)));
        }
    }

    /// A launch with a hand-off file failed: delete it, and close the room it was for.
    fn fail_social_handoff(&mut self, handoff: Option<(std::path::PathBuf, Option<String>)>) {
        let Some((path, hosting)) = handoff else {
            return;
        };
        social::handoff::discard(&path);
        if let Some(room) = hosting {
            self.friends_close_room_after_failure(room);
        }
    }

    /// The pending hand-off file for the game starting now, and the room it hosts. One that has
    /// waited as long as the portal's grant lasts is stale: it is deleted, not passed.
    fn take_social_handoff(&mut self) -> Option<(std::path::PathBuf, Option<String>)> {
        let handoff = self.social_handoff.take()?;
        if handoff.asked.elapsed() > SOCIAL_HANDOFF_TTL {
            self.fail_social_handoff(Some((handoff.path, handoff.hosting)));
            self.friends.note = Some((
                false,
                tr("The room's pass ran out before the game started. Try again.").to_string(),
            ));
            return None;
        }
        Some((handoff.path, handoff.hosting))
    }

    /// MX3JN2: after a join sync, joins the friend's room again: a fresh grant through the usual room join. When the
    /// launcher has no room of that host in its list (it may be stale), "Join again" is left for the page
    /// (`join_again_host`).
    fn rejoin_friend(&mut self, principal: String) {
        let room = self
            .friends
            .rooms
            .iter()
            .find(|room| room.host.principal == principal)
            .map(|room| room.session_id.clone());
        match room {
            Some(session_id) => self.friends_join_room(session_id),
            None => self.join_again = Some(principal),
        }
    }

    /// MX3JN2: the friend host (portal user id) the launcher could not rejoin by itself after fetching its mods: show
    /// "Join again", whose click calls [`Self::join_again`].
    pub(crate) fn join_again_host(&self) -> Option<&str> {
        self.join_again.as_deref()
    }

    /// MX3JN2: "Join again": the same room join as the automatic rejoin, once the host's room is listed.
    pub(crate) fn join_again(&mut self) {
        if let Some(principal) = self.join_again.clone() {
            if self.friends.rooms.iter().any(|room| room.host.principal == principal) {
                self.join_again = None;
                self.rejoin_friend(principal);
            }
        }
    }

    fn join_friend(&mut self) {
        if horizon_friends::invite::parse(self.friend_code.text()).is_none() {
            return;
        }
        self.joining = Some(self.friend_code.text().to_string());
        self.play();
        if self.starting_client.is_none()
            && !(yoake_page::enabled() && self.yoake_base_pending_play())
        {
            self.joining = None;
        }
    }

    /// Use and Remove for an installed version; Remove asks once more before it deletes.
    fn version_actions(&self, tag: &str, selected: bool, cx: &mut Context<Self>) -> Div {
        let t = self.theme;
        let confirming = self.confirm_remove.as_deref() == Some(tag);
        let (t1, t2) = (tag.to_string(), tag.to_string());
        let remove_id = SharedString::from(format!("remove-{tag}"));
        let remove = cx.listener(move |this, _: &ClickEvent, _, cx| {
            if this.confirm_remove.as_ref() == Some(&t2) {
                this.remove(&t2);
            } else {
                this.confirm_remove = Some(t2.clone());
            }
            cx.notify();
        });
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(4.))
            .when(!selected && !confirming, |d| {
                d.child(
                    glass_ui::quiet(format!("use-{tag}"), tr("Use"), 36., &t).on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.settings.selected = Some(t1.clone());
                            this.save_settings();
                            cx.notify();
                        }),
                    ),
                )
            })
            .child(if confirming {
                keyboard(
                    glass_home_lists::danger_button(remove_id, tr("Remove for good").into()),
                    &t,
                )
                .on_click(remove)
                .into_any_element()
            } else {
                glass_ui::quiet(remove_id, tr("Remove"), 36., &t)
                    .on_click(remove)
                    .into_any_element()
            })
    }

    /// A glass row's progress line (installing, removing, importing): the bar, then its text.
    fn glass_progress(&self, bar: Div, label: String) -> Div {
        use horizon_theme::glass::ink;
        div()
            .relative()
            .flex()
            .items_center()
            .gap(px(10.))
            .w(px(180.))
            .child(bar)
            .child(glass_ui::text(13., FontWeight::NORMAL, ink::DIM).child(label))
    }

    /// One installed build made here on the Versions page, a native build or an imported one:
    /// what it was made from, and Use and Remove.
    fn local_build_row(
        &self,
        tag: &str,
        on_disk: &dyn Fn(&String) -> Option<u64>,
        cx: &mut Context<Self>,
    ) -> Div {
        use horizon_theme::glass::ink;
        let t = self.theme;
        let selected = self.selected().map(String::as_str) == Some(tag);
        let mut meta = Vec::new();
        if let Some(about) = native_mods::about(tag) {
            meta.push(trf("Built on {base}", &[("base", &about.base)]));
        }
        if let Some(about) = build_import::about(tag) {
            let base = horizon_launcher::nightly::label(&about.base).unwrap_or(about.base);
            meta.push(trf("Game files from {base}", &[("base", &base)]));
            meta.push(about.file);
        }
        if let Some(n) = on_disk(&tag.to_string()) {
            meta.push(trf("{size} on disk", &[("size", &megabytes(n))]));
        }
        let right = if self.removing.iter().any(|r| r == tag) {
            self.glass_progress(progress_bar(None, &t), "Removing…".to_string())
        } else {
            self.version_actions(tag, selected, cx)
        };
        let error = self
            .install_error
            .as_ref()
            .filter(|(e, _)| e == tag)
            .map(|(_, e)| e.clone());
        let key = SharedString::from(format!("local-{tag}"));
        let hovered = self.list_hover.as_ref() == Some(&key);
        let hover = self.list_hover_listener(key.clone(), cx);
        div()
            .flex()
            .flex_col()
            .child(
                glass_ui::list_row_static(key, hovered, hover)
                    .child(glass_ui::list_text(version_label(tag), meta.join("  ·  ")))
                    .children(selected.then(|| {
                        glass_ui::text(13., FontWeight::MEDIUM, ink::FULL)
                            .relative()
                            .flex_none()
                            .text_color(glass_ui::accent_soft(self.look.accent_soft))
                            .child(tr("In use"))
                    }))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .justify_end()
                            .w(px(190.))
                            .flex_none()
                            .child(right),
                    ),
            )
            .children(error.map(glass_home_lists::danger_line))
    }

    /// The installed builds of one kind made here (`is`: native or imported) under their caption,
    /// with a notice above them; None when there are none.
    fn local_builds(
        &self,
        title: &str,
        is: fn(&str) -> bool,
        notice_: Option<&str>,
        on_disk: &dyn Fn(&String) -> Option<u64>,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        use horizon_theme::glass::ink;
        let tags: Vec<&String> = self
            .installed
            .iter()
            .chain(&self.removing)
            .filter(|v| is(v))
            .collect();
        (!tags.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .child(glass_ui::list_head(tr(title)).pt(px(12.)))
                .children(notice_.map(|n| {
                    div().px(px(12.)).pb(px(8.)).child(
                        glass_ui::text(13., FontWeight::NORMAL, ink::NOTE)
                            .text_color(glass_ui::color(horizon_theme::glass::WARN))
                            .child(n.to_string()),
                    )
                }))
                .children(
                    tags.into_iter()
                        .map(|tag| self.local_build_row(tag, on_disk, cx)),
                )
        })
    }

    fn versions_page(&mut self, cx: &mut Context<Self>) -> Div {
        use horizon_theme::glass::ink;
        let t = self.theme;
        let header = glass_page_head(
            "Versions",
            "Builds from GitHub. Each one installs into its own folder.",
        );
        // Measured again when the installed versions change.
        if self.disk_for != self.installed {
            self.disk_for = self.installed.clone();
            let (tags, tx) = (self.installed.clone(), self.tx.clone());
            std::thread::spawn(move || {
                let disk = tags.into_iter().map(|t| {
                    let n = backend::disk_use(&t);
                    (t, n)
                });
                let _ = tx.send(Msg::DiskUse(disk.collect()));
            });
        }
        let on_disk = |tag: &String| {
            let found = self.disk.iter().find(|(t, _)| t == tag);
            found
                .filter(|_| self.installed.contains(tag))
                .map(|(_, n)| *n)
        };
        let used: u64 = self.installed.iter().filter_map(on_disk).sum();
        // The list's own action at the end of the line above it: a build the player has.
        let import: AnyElement = if self.importing_build {
            self.glass_progress(progress_bar(None, &t), tr("Importing…").to_string())
                .into_any_element()
        } else {
            glass_ui::quiet("import-build", tr("Import a game build…"), 36., &t)
                .relative()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.import_build(cx)))
                .into_any_element()
        };
        let page = div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .pt(px(36.))
            .child(header);
        // The page's one panel: the line over the list, then what is installed, then the releases.
        let mut body = div()
            .relative()
            .flex()
            .flex_col()
            .min_h(px(200.))
            .p(px(8.))
            .child(
                glass_ui::list_head(if used > 0 {
                    trf(
                        "Installed versions use {size}.",
                        &[("size", &megabytes(used))],
                    )
                } else {
                    String::new()
                })
                .pt(px(8.))
                .child(import),
            )
            .children(
                self.import_error
                    .clone()
                    .map(glass_home_lists::danger_line),
            );
        // Builds made here, not downloaded, are listed apart from the releases.
        body = body
            .children(self.local_builds(
                "Native mod builds",
                native_mods::is_native,
                None,
                &on_disk,
                cx,
            ))
            .children(self.local_builds(
                "Imported builds",
                build_import::is_imported,
                Some(tr(
                    "Unsigned: you picked these programs yourself, and Horizon can't \
                    check where they came from. Updates never replace them.",
                )),
                &on_disk,
                cx,
            ));
        let releases = match &self.releases {
            Load::Loading => {
                return page.child(
                    glass_card().child(body.child(glass_ui::empty().child(tr("Loading releases…")))),
                )
            }
            Load::Failed(e) => {
                return page.child(
                    glass_card().child(
                        body.child(glass_ui::empty().pb(px(16.)).child(trf(
                            "Couldn't load releases. {error}",
                            &[("error", e)],
                        )))
                        .child(
                            div().px(px(12.)).pb(px(24.)).child(
                                glass_ui::quiet("retry", tr("Try again"), 36., &t).on_click(
                                    cx.listener(|this, _: &ClickEvent, _, cx| {
                                        this.fetch_releases();
                                        cx.notify();
                                    }),
                                ),
                            ),
                        ),
                    ),
                );
            }
            Load::Ready(r) if r.is_empty() => {
                return page.child(
                    glass_card().child(body.child(glass_ui::empty().child(tr("No releases yet.")))),
                );
            }
            Load::Ready(r) => r,
        };
        // A hundred releases: only the rows near the screen are drawn.
        let keep = |i: usize| {
            let tag = &releases[i].tag;
            self.installing.as_ref().is_some_and(|(t, _)| t == tag)
                || self.install_error.as_ref().is_some_and(|(t, _)| t == tag)
                || self.confirm_remove.as_ref() == Some(tag)
                || self.removing.contains(tag)
        };
        let list = self.long_list.clone().column(
            &self.content_scroll.handle,
            releases.len(),
            2.,
            keep,
            |i| {
                let release = releases[i].clone();
                let tag = release.tag.clone();
                let installed = self.installed.contains(&tag);
                let selected = self.selected() == Some(&tag);
                let mut meta = vec![release.date.clone()];
                if let Some((_, size)) = release.asset {
                    meta.push(megabytes(size));
                }
                if let Some(n) = on_disk(&tag) {
                    meta.push(trf("{size} on disk", &[("size", &megabytes(n))]));
                }
                let summary = release
                    .name
                    .split_once(": ")
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or_else(|| release.name.clone());
                let right: gpui::AnyElement = if let Some((_, p)) =
                    self.installing.as_ref().filter(|(t, _)| *t == tag)
                {
                    self.glass_progress(
                        progress_bar(Some(self.bar), &t),
                        format!("{:.0}%", p * 100.),
                    )
                    .into_any_element()
                } else if self.removing.contains(&tag) {
                    self.glass_progress(progress_bar(None, &t), "Removing…".to_string())
                        .into_any_element()
                } else if installed {
                    self.version_actions(&tag, selected, cx).into_any_element()
                } else if release.asset.is_none() {
                    glass_ui::text(13., FontWeight::NORMAL, ink::DIM)
                        .relative()
                        .child(tr("No build for this platform"))
                        .into_any_element()
                } else {
                    // Eight white buttons in a column are heavy: only the newest row's is solid.
                    let mut b = if i == 0 {
                        glass_ui::solid(format!("install-{tag}"), tr("Install"), 36., 14., &t)
                    } else {
                        glass_ui::quiet(format!("install-{tag}"), tr("Install"), 36., &t)
                    };
                    if self.installing.is_some() {
                        b = disabled(b);
                    } else {
                        b = b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.install(&release);
                            cx.notify();
                        }));
                    }
                    b.into_any_element()
                };
                let status = if selected {
                    Some(glass_pill(
                        tr("In use"),
                        glass_ui::accent_soft(self.look.accent_soft),
                        0.18,
                    ))
                } else if installed {
                    Some(glass_pill(tr("Installed"), glass_ui::white(ink::BODY), 0.1))
                } else {
                    None
                };
                let error = self
                    .install_error
                    .as_ref()
                    .filter(|(t, _)| *t == tag)
                    .map(|(_, e)| e.clone());
                let key = SharedString::from(format!("release-{tag}"));
                let hovered = self.list_hover.as_ref() == Some(&key);
                let hover = self.list_hover_listener(key.clone(), cx);
                div()
                    .flex()
                    .flex_col()
                    .child(
                        glass_ui::list_row_static(key, hovered, hover)
                            .child(
                                // `ui::list_text`, with the state's badge after the name.
                                div()
                                    .relative()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(px(8.))
                                            .min_w_0()
                                            .child(
                                                glass_ui::text(14., FontWeight::MEDIUM, ink::FULL)
                                                    .line_height(px(21.))
                                                    .truncate()
                                                    .child(version_label(&tag)),
                                            )
                                            .children(status),
                                    )
                                    .child(
                                        glass_ui::text(13., FontWeight::NORMAL, ink::QUIET)
                                            .line_height(px(19.5))
                                            .truncate()
                                            .child(summary),
                                    ),
                            )
                            .child(
                                glass_ui::text(13., FontWeight::NORMAL, ink::DIM)
                                    .relative()
                                    .flex_none()
                                    .whitespace_nowrap()
                                    .child(meta.join("  ·  ")),
                            )
                            .child(
                                div()
                                    .relative()
                                    .flex()
                                    .justify_end()
                                    .w(px(190.))
                                    .flex_none()
                                    .child(right),
                            ),
                    )
                    .children(error.map(glass_home_lists::danger_line))
            },
        );
        page.child(glass_card().child(body.child(list)))
    }

    fn notes_page(&mut self, cx: &mut Context<Self>) -> Div {
        use horizon_theme::glass::ink;
        let t = self.theme;
        let sponsor = (!yoake_page::enabled() && !self.links.sponsor.is_empty()).then(|| {
            glass_card()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(16.))
                .p(px(24.))
                .child(
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .min_w_0()
                        .child(
                            glass_ui::text(16., FontWeight::SEMIBOLD, ink::FULL)
                                .child(tr("Support the work below")),
                        )
                        .child(glass_ui::text(14., FontWeight::NORMAL, ink::NOTE).child(tr(
                            "Horizon is built in the open. Sponsors fund the next release.",
                        ))),
                )
                .child(
                    glass_ui::quiet("sponsor", tr("Sponsor on GitHub"), 36., &t)
                        .relative()
                        .on_click({
                            let url = self.links.sponsor.clone();
                            move |_: &ClickEvent, _, cx| cx.open_url(&url)
                        }),
                )
        });
        let page = div()
            .flex()
            .flex_col()
            .gap(px(28.))
            .pt(px(36.))
            .child(glass_page_head(
                "Patch notes",
                "What changed in each release, newest first.",
            ))
            .children(sponsor);
        let releases = match &self.releases {
            Load::Loading => {
                return page.child(
                    glass_card()
                        .child(glass_ui::empty().relative().child(tr("Loading patch notes…"))),
                )
            }
            Load::Failed(e) => {
                return page.child(
                    glass_card().child(
                        div()
                            .relative()
                            .flex()
                            .flex_col()
                            .items_start()
                            .child(glass_ui::empty().pb(px(16.)).child(trf(
                                "Couldn't load patch notes. {error}",
                                &[("error", e)],
                            )))
                            .child(
                                div().px(px(12.)).pb(px(32.)).child(
                                    glass_ui::quiet("retry-notes", tr("Try again"), 36., &t)
                                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                            this.fetch_releases();
                                            cx.notify();
                                        })),
                                ),
                            ),
                    ),
                );
            }
            Load::Ready(r) => r,
        };
        // The notes' markdown in the glass look's white at a few opacities.
        let mut g = t;
        g.ink = glass_ui::white(ink::FULL);
        g.ink_soft = glass_ui::white(ink::SOFT);
        g.muted = glass_ui::white(ink::DIM);
        g.plate_soft = glass_ui::white(0.08);
        g.rule = glass_ui::white(0.12);
        let list = self.long_list.clone().column(
            &self.content_scroll.handle,
            releases.len(),
            12.,
            |_| false,
            |i| {
                let release = &releases[i];
                let summary = release
                    .name
                    .split_once(": ")
                    .map(|(_, rest)| rest.to_string())
                    .unwrap_or_default();
                glass_card()
                    .flex()
                    .flex_col()
                    .gap(px(16.))
                    .p(px(24.))
                    .mb(px(12.))
                    .child(
                        div()
                            .relative()
                            .flex()
                            .items_center()
                            .gap(px(16.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .gap(px(2.))
                                    .child(
                                        glass_ui::text(22., FontWeight::SEMIBOLD, ink::FULL)
                                            .line_height(px(30.))
                                            .child(version_label(&release.tag)),
                                    )
                                    .when(!summary.is_empty(), |d| {
                                        d.child(
                                            glass_ui::text(14., FontWeight::NORMAL, ink::NOTE)
                                                .child(summary),
                                        )
                                    }),
                            )
                            .child(
                                glass_ui::text(13., FontWeight::NORMAL, ink::DIM)
                                    .flex_none()
                                    .child(release.date.clone()),
                            ),
                    )
                    .child(
                        div()
                            .relative()
                            .max_w(px(640.))
                            .font_family(horizon_theme::glass::FONT)
                            .child(if release.body.trim().is_empty() {
                                glass_ui::text(14., FontWeight::NORMAL, ink::NOTE)
                                    .child(tr("No notes for this release."))
                                    .into_any_element()
                            } else {
                                markdown::render(&release.body, &g)
                            }),
                    )
            },
        );
        page.child(list)
    }
}

/// A thin bar. A known fraction eases from its previous value; an unknown one shuttles slowly.
/// Progress: a plate-coloured track filling with the accent; work of unknown length shuttles.
fn progress_bar(fraction: Option<(f32, f32)>, t: &Theme) -> Div {
    // Night: a thin accent line on a plate's colour, square like the art.
    let n = t.night;
    let bar = div()
        .absolute()
        .top_0()
        .bottom_0()
        .bg(plate::alpha(n.accent, 1.));
    let track = div()
        .relative()
        .flex_1()
        .h(px(6.))
        .bg(plate::alpha(n.card_hi, n.card_hi_alpha))
        .overflow_hidden();
    match fraction {
        Some((from, to)) => {
            let (from, to) = (from.clamp(0.02, 1.), to.clamp(0.02, 1.));
            track.child(bar.left_0().with_animation(
                ElementId::named_usize("bar", (to * 1000.) as usize),
                Animation::new(theme::MOVE).with_easing(ease_out_quint()),
                move |d, p| d.w(relative(from + (to - from) * p)),
            ))
        }
        None => track.child(
            bar.w(relative(0.3)).with_animation(
                "shuttle",
                Animation::new(Duration::from_millis(1600))
                    .repeat()
                    .with_easing(gpui::ease_in_out),
                |d, p| d.left(relative(-0.3 + 1.3 * p)),
            ),
        ),
    }
}

impl Render for Launcher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        if specimen::wanted() {
            return div()
                .flex()
                .flex_col()
                .size_full()
                .bg(t.side)
                .children(titlebar::render(&t, window))
                .child(specimen::render(&t));
        }
        if std::mem::take(&mut self.restyle) {
            self.theme = self.settings_theme(window);
        }
        // The glass look's inks, fills and plates follow the theme (see `glass_ui::white`).
        glass_ui::set_light(!self.theme.dark);
        self.scale = window.scale_factor();
        self.perf.frame();
        if self.bench.as_ref().is_some_and(perf::Bench::scrolling) {
            self.bench_scroll(window);
        }
        self.servers.note_focus(window, cx);
        self.friend_focused = self.friend_focus.is_focused(window);
        self.mods.note_focus(window);
        self.github.note_focus(window);
        self.args_focused = self.args_focus.is_focused(window);
        self.proxy_focused = self.proxy_focus.is_focused(window);
        let size = window.viewport_size();
        self.window_width = f32::from(size.width);
        let content = match self.page {
            Page::Play => self.play_page(cx),
            Page::Versions => self.versions_page(cx),
            Page::Mods => self.mods_page(cx),
            Page::Servers => self.servers_page(cx),
            Page::Wardrobe => self.wardrobe_page(cx),
            Page::Notes => self.notes_page(cx),
            Page::Yoake => self.yoake_page(window, cx),
            Page::Friends => self.friends_page(window, cx),
            Page::Settings => self.settings_page(cx),
        };
        self.sky.free_replaced(window);
        // The new page fades in and rises a few pixels.
        let content = div()
            .relative()
            .w_full()
            // Let the page shrink inside the padded flex row so long notices can wrap.
            .min_w_0()
            .child(content)
            .with_animation(
                ElementId::named_usize("page", self.switches),
                Animation::new(theme::ENTER).with_easing(ease_out_quint()),
                |d, p| d.opacity(p).top(px(theme::RISE * (1. - p))),
            );
        let (reveal, held) = self.opening_phase(window);
        let ui = ease_out_cubic((reveal - 0.3) / 0.7);
        if self.opening.is_none() && window.focused(cx).is_none() {
            window.focus(&self.root_focus, cx);
        }
        // Dark: the new design's scene; Light keeps the dawn sky until the design has a light
        // look.
        glass::begin_build(window);
        let sky = if t.dark {
            self.scene.clone().into_any_element()
        } else {
            let (sky, next) = self.sky.render(
                &t.night,
                (size.width.into(), size.height.into()),
                window.scale_factor(),
                window.is_window_active(),
                Instant::now(),
            );
            if let Some(next) = next {
                self.wake_sky(next, cx);
            }
            sky.into_any_element()
        };
        // Dark: the glass frame over the scene; Light keeps the night one over the dawn.
        let (topbar, sidebar) = if GLASS_PAGES {
            (
                self.glass_topbar(cx),
                self.glass_sidebar(window, cx).into_any_element(),
            )
        } else {
            (self.topbar(cx), self.sidebar(window, cx).into_any_element())
        };
        let row_menu = if GLASS_PAGES {
            self.row_menu(window, cx)
        } else {
            None
        };
        let mut root = div()
            .id("root")
            .track_focus(&self.root_focus)
            .on_key_down(cx.listener(|this, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                match k.key.as_str() {
                    "tab" => {
                        KEYBOARD.store(true, Ordering::Relaxed);
                        if k.modifiers.shift {
                            window.focus_prev(cx);
                        } else {
                            window.focus_next(cx);
                        }
                    }
                    "escape" if this.native_log_escape() => {}
                    "escape" if this.servers_escape() => {}
                    "escape" if this.instance_escape() => {}
                    "escape" if this.screenshot_escape() => {}
                    "left" if this.screenshot_step(false) => {}
                    "right" if this.screenshot_step(true) => {}
                    _ => return,
                }
                cx.stop_propagation();
                cx.notify();
            }))
            .capture_any_mouse_down(|_, _, _| KEYBOARD.store(false, Ordering::Relaxed))
            .on_mouse_move(cx.listener(|this, ev: &gpui::MouseMoveEvent, window, cx| {
                let size = window.viewport_size();
                this.scene
                    .update(cx, |scene, cx| scene.point(ev.position, size, cx));
            }))
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .font_family(theme::FONT)
            .text_color(t.ink)
            .overflow_hidden()
            .child(
                div()
                    .relative()
                    .flex()
                    .size_full()
                    .opacity(ui)
                    .top(px(theme::RISE * (1. - ui)))
                    .child(sidebar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(topbar)
                            .child(
                                self.content_scroll
                                    .wrap(
                                        div()
                                            .id("content")
                                            .relative()
                                            .flex_1()
                                            .min_h_0()
                                            .overflow_y_scroll()
                                            .child(
                                                div()
                                                    .flex()
                                                    .w_full()
                                                    .min_w_0()
                                                    .px(px(theme::PAGE_GUTTER))
                                                    .pt(px(if self.page == Page::Play {
                                                        0.
                                                    } else {
                                                        28.
                                                    }))
                                                    .pb(px(48.))
                                                    .child(content),
                                            ),
                                        &t,
                                    )
                                    .flex_1()
                                    .min_h_0(),
                            ),
                    ),
            );
        root = root.children(row_menu);
        if self.page == Page::Servers {
            if let Some(dialog) = self.create_dialog(cx) {
                root = root.child(dialog);
            }
        }
        root = self.import_drop_target(root, cx);
        root = root.children(self.import_note(cx));
        if let Some(dialog) = self.instance_dialog(cx) {
            root = root.child(dialog);
        }
        if let Some(panel) = self.native_log_panel(cx) {
            root = root.child(panel);
        }
        if self.page == Page::Play {
            if let Some(viewer) = self.screenshot_viewer(cx) {
                root = root.child(viewer);
            }
        }
        if self.asking_consent() {
            root = root.child(self.consent_screen(cx));
        }
        if self.asking_owner() {
            root = root.child(self.owner_screen(cx));
        }
        if let Some(opening) = &self.opening {
            // The wordmark arrives letter by letter, then gives way to the launcher; a click or
            // a key skips ahead.
            let gone = 1. - ease_out_cubic(reveal / 0.6);
            let letters = if horizon_launcher::brand::yoake() {
                "YOAKE"
            } else {
                "HORIZON"
            };
            let word = div()
                .flex()
                .font_family(theme::PIXEL)
                .text_size(px(theme::WORDMARK.px * 0.5))
                .line_height(px(theme::WORDMARK.px * 0.5))
                .font_weight(FontWeight::BOLD)
                .text_color(t.ink)
                .children(letters.chars().enumerate().map(|(i, c)| {
                    let at = held - i as f32 * theme::STAGGER.as_secs_f32() * 1.5;
                    let p = ease_out_cubic(at / theme::ENTER.as_secs_f32());
                    div()
                        .relative()
                        .opacity(p)
                        .top(px(theme::RISE * 2. * (1. - p)))
                        .child(c.to_string())
                }));
            root = root.child(
                div()
                    .id("opening")
                    .track_focus(&opening.focus)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(t.page.opacity(gone))
                    .when(reveal == 0., |d| d.cursor_pointer())
                    .child(
                        div()
                            .relative()
                            .top(px(-24. * (1. - gone)))
                            .opacity(gone)
                            .child(word),
                    )
                    .when(reveal == 0., |d| {
                        d.on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                            this.skip_opening();
                            cx.notify();
                        }))
                        .on_key_down(cx.listener(
                            |this, _: &KeyDownEvent, _, cx| {
                                this.skip_opening();
                                cx.notify();
                            },
                        ))
                    }),
            );
        }
        div()
            .flex()
            .flex_col()
            .relative()
            .size_full()
            .bg(t.side)
            .child(sky)
            .children(titlebar::render(&t, window))
            .children(self.settings_save_notice(cx))
            .child(root.flex_1().min_h_0())
            .children(titlebar::resize_handles(window))
    }
}

impl Launcher {
    /// Where the opening is: the reveal's progress (1 when there is no opening) and how long
    /// the wordmark has been showing, in seconds.
    fn opening_phase(&mut self, window: &mut Window) -> (f32, f32) {
        let loaded = !matches!(self.releases, Load::Loading);
        let Some(opening) = &mut self.opening else {
            return (1., 10.);
        };
        let held = opening.started.get_or_insert_with(Instant::now).elapsed();
        if opening.reveal.is_none() && ((held >= OPENING_HOLD && loaded) || held >= OPENING_MAX) {
            opening.reveal = Some(Instant::now());
        }
        let reveal = opening.reveal.map_or(0., |at| {
            at.elapsed().as_secs_f32() / OPENING_REVEAL.as_secs_f32()
        });
        if reveal >= 1. {
            self.opening = None;
            return (1., 10.);
        }
        window.request_animation_frame();
        (reveal, held.as_secs_f32())
    }

    /// `horizon-launcher benchmark`: scroll the page a little on every frame, back to the top at
    /// the bottom, so the frame times measure scrolling.
    fn bench_scroll(&mut self, window: &mut Window) {
        let handle = &self.content_scroll.handle;
        let mut offset = handle.offset();
        offset.y -= px(24.);
        if -offset.y > handle.max_offset().y {
            offset.y = px(0.);
        }
        handle.set_offset(offset);
        window.request_animation_frame();
    }

    /// Draws the window again in `after`, when the sky next changes, unless a frame comes sooner.
    fn wake_sky(&mut self, after: Duration, cx: &mut Context<Self>) {
        let at = Instant::now() + after;
        if self.sky_wake.as_ref().is_some_and(|(when, _)| *when <= at) {
            return;
        }
        let timer = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(after).await;
            let _ = this.update(cx, |this, cx| {
                this.sky_wake = None;
                cx.notify();
            });
        });
        self.sky_wake = Some((at, timer));
    }

    fn skip_opening(&mut self) {
        if let Some(opening) = &mut self.opening {
            opening.reveal.get_or_insert_with(Instant::now);
        }
    }
}

actions!(horizon_launcher, [Quit, CloseWindow]);

/// What closing the last window does: stop the servers this launcher started, send the telemetry,
/// exit. A game that is running keeps running. Cmd+Q goes through here too.
fn shut_down(cx: &mut App) {
    // A room the player hosts is closed first (the game is told, then the portal).
    social::host::close_on_quit();
    servers_page::stop_on_quit();
    telemetry::send_before_exit();
    cx.quit()
}

/// The macOS shortcuts: Cmd+Q quits, Cmd+W closes the window.
#[cfg(target_os = "macos")]
fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
    ]
}

#[cfg(not(target_os = "macos"))]
fn key_bindings() -> Vec<KeyBinding> {
    Vec::new()
}

/// The menu bar on macOS, where "Quit" belongs to the app menu.
fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: horizon_launcher::brand::launcher_name().into(),
            items: vec![MenuItem::action(
                horizon_launcher::brand::text("Quit Horizon Launcher").into_owned(),
                Quit,
            )],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![MenuItem::action("Close Window", CloseWindow)],
            disabled: false,
        },
    ]
}

/// A hand-off file asked for, the time it was asked, and the room the game is to host (to close
/// it again when the game does not start).
struct SocialHandoff {
    path: std::path::PathBuf,
    asked: Instant,
    hosting: Option<String>,
}

/// A hand-off file older than this is not passed on: the portal's grant lasts 240 seconds from
/// the moment it is asked for, and the file is written right after.
const SOCIAL_HANDOFF_TTL: Duration = Duration::from_secs(230);

fn main() {
    // Signing out (or in as another account) first closes the room this launcher hosts, while
    // the old sign-in can still delete it.
    yoake_auth::set_before_change(social::host::close_on_quit);
    if let Err(error) = console::attach_for_cli() {
        eprintln!("Could not restore the launcher's terminal streams: {error}");
        std::process::exit(1);
    }
    // Windows setup: `setup --quiet` / `uninstall --quiet` run here without a window, and never
    // reach the CLI parser below (so neither the MCP tools nor the completions list them).
    #[cfg(windows)]
    if let Some(code) = horizon_launcher::installer::run_quiet() {
        std::process::exit(code);
    }
    perf::process_started();
    backend::exe_path();
    #[cfg(windows)]
    backend::remove_replaced_binaries();
    backend::cleanup_launcher_updates();
    // Room files the game never read (it did not start, or the launcher was closed first).
    social::handoff::sweep(
        &social::handoff::folder(&backend::data_dir()),
        social::handoff::MAX_AGE,
    );
    // Approved-list folders of games that were never closed properly (a crashed launcher).
    horizon_launcher::full_access::remove_stale_runs();
    // What the launcher downloads goes through the player's proxy, if they set one: the parts
    // that build their own agents (`horizon-port-pack`) are told before anything starts.
    horizon_launcher::proxy::announce(horizon_launcher::backend::Settings::load().proxy);
    if let Some(code) = cli::run() {
        std::process::exit(code);
    }
    gpui_platform::application()
        .with_assets(blocks::Assets)
        .run(|cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(horizon_theme::fonts::GEIST_MONO),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_REGULAR),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_MEDIUM),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_SEMIBOLD),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_BOLD),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_DISPLAY_BOLD),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_DISPLAY_SEMIBOLD),
                    Cow::Borrowed(horizon_theme::fonts::BRICOLAGE_HEADING_SEMIBOLD),
                    Cow::Borrowed(horizon_theme::fonts::GEIST_REGULAR),
                    Cow::Borrowed(horizon_theme::fonts::GEIST_MEDIUM),
                    Cow::Borrowed(horizon_theme::fonts::GEIST_SEMIBOLD),
                ])
                .expect("bundled fonts");
            cx.bind_keys(key_bindings());
            cx.on_action(|_: &Quit, cx| shut_down(cx));
            // The key press is being handled inside the window, so the window can only be
            // removed once that update has finished.
            cx.on_action(|_: &CloseWindow, cx| {
                cx.defer(|cx| {
                    for window in cx.windows() {
                        let _ = window.update(cx, |_, window, _| window.remove_window());
                    }
                })
            });
            cx.set_menus(menus());
            // The setup exe (and `horizon-launcher.exe uninstall`) opens its own window instead.
            #[cfg(windows)]
            if let Some(mode) = horizon_launcher::installer::window_mode() {
                setup_page::open(mode, cx);
                return;
            }
            // `HORIZON_LAUNCHER_SCREENSHOT=<png>`: a hidden window, drawn once off screen, saved
            // and closed (`screenshot_mode.rs`). Nothing is shown or focused.
            let shot = screenshot_mode::wanted();
            let bounds = Bounds::centered(None, size(px(1180.), px(760.)), cx);
            let handle = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        window_min_size: Some(size(px(960.), px(600.))),
                        titlebar: Some(titlebar::options()),
                        focus: shot.is_none(),
                        show: shot.is_none(),
                        #[cfg(target_os = "linux")]
                        window_decorations: Some(gpui::WindowDecorations::Client),
                        app_id: Some("horizon-launcher".into()),
                        // GPUI draws a window without focus at most every 33 ms to save energy. A
                        // benchmark's window often starts without it (KDE keeps focus from windows
                        // a background process opens), and it measures drawing, not that limit.
                        inactive_frame_interval: if perf::Bench::wanted() {
                            None
                        } else {
                            WindowOptions::default().inactive_frame_interval
                        },
                        ..Default::default()
                    },
                    |window, cx| {
                        cx.new(|cx| {
                            let mut launcher = Launcher::new(window, cx);
                            if shot.is_some() {
                                // No first-run sheets over the page being drawn.
                                launcher.quiet_sheets = true;
                                // `HORIZON_LAUNCHER_SCREENSHOT_LANG=en|ja|…`
                                if let Some(language) = std::env::var("HORIZON_LAUNCHER_SCREENSHOT_LANG")
                                    .ok()
                                    .and_then(|code| i18n::Language::parse(&code))
                                {
                                    i18n::set(language);
                                }
                            }
                            launcher
                        })
                    },
                )
                .expect("window");
            cx.on_window_closed(|cx, _| shut_down(cx)).detach();
            if let Some(shot) = screenshot_mode::wanted() {
                screenshot_mode::drive(handle.into(), shot, cx);
                return;
            }
            cx.activate(true);
        });
}

/// Whether the click `event` began with the mouse-down that closed a menu at `closed_at`.
/// A keyboard click never did.
fn closed_by_press(closed_at: Option<Point<Pixels>>, event: &ClickEvent) -> bool {
    matches!(event, ClickEvent::Mouse(click) if closed_at == Some(click.down.position))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_menu_button_stays_closed_after_the_press_that_closed_its_menu() {
        use super::closed_by_press;
        use gpui::{point, px, ClickEvent, KeyboardClickEvent, MouseClickEvent, MouseDownEvent};
        let click = |x: f32| {
            ClickEvent::Mouse(MouseClickEvent {
                down: MouseDownEvent {
                    position: point(px(x), px(20.)),
                    ..MouseDownEvent::default()
                },
                ..MouseClickEvent::default()
            })
        };
        let closed_at = Some(point(px(10.), px(20.)));
        // The press on the button that closed the open menu: it stays closed.
        assert!(closed_by_press(closed_at, &click(10.)));
        // A later press, or one with no menu closed, toggles as before.
        assert!(!closed_by_press(closed_at, &click(11.)));
        assert!(!closed_by_press(None, &click(10.)));
        let keyboard = ClickEvent::Keyboard(KeyboardClickEvent::default());
        assert!(!closed_by_press(closed_at, &keyboard));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cmd_q_quits_and_cmd_w_closes_the_window() {
        use super::{key_bindings, CloseWindow, Quit};
        use gpui::{Keymap, Keystroke};
        let keymap = Keymap::new(key_bindings());
        let key = |s: &'static str| [Keystroke::parse(s).unwrap()];
        let (hit, _) = keymap.bindings_for_input(&key("cmd-q"), &[]);
        assert!(hit.iter().any(|b| b.action().partial_eq(&Quit)));
        let (hit, _) = keymap.bindings_for_input(&key("cmd-w"), &[]);
        assert!(hit.iter().any(|b| b.action().partial_eq(&CloseWindow)));
    }

    #[test]
    fn a_second_version_can_be_removed_while_another_deletes() {
        let installed = ["v0.2.97".to_string(), "v0.2.76".to_string()];
        let removing = ["v0.2.97".to_string()];
        assert!(super::removable(&installed, None, &removing, "v0.2.76"));
        assert!(!super::removable(&installed, None, &removing, "v0.2.97"));
        assert!(!super::removable(
            &installed,
            Some("v0.2.76"),
            &[],
            "v0.2.76"
        ));
        assert!(!super::removable(&installed, None, &[], "v0.2.85"));
    }

    #[test]
    fn the_selection_survives_a_removal_or_moves_to_the_newest_left() {
        let left = ["v0.2.49".to_string(), "v0.2.48".to_string()];
        let pick =
            |saved: Option<&str>| super::selection_after_removal(&left, saved.map(String::from));
        assert_eq!(pick(Some("v0.2.48")).as_deref(), Some("v0.2.48"));
        assert_eq!(pick(Some("v0.2.47")).as_deref(), Some("v0.2.49"));
        assert_eq!(pick(None).as_deref(), Some("v0.2.49"));
        assert_eq!(
            super::selection_after_removal(&[], Some("v0.2.47".into())),
            None
        );
    }

    /// An owner's Play works on a version not prepared yet: it prepares it (downloading
    /// Minecraft from Mojang when this computer has no install), then plays. Without an owner
    /// it stays off: signing in comes first.
    #[test]
    fn play_prepares_first_for_an_owner() {
        use super::{play_press, PlayPress};
        assert_eq!(play_press(true, Some(true), false), PlayPress::Play);
        assert_eq!(play_press(true, None, false), PlayPress::Play);
        assert_eq!(
            play_press(false, Some(true), false),
            PlayPress::PrepareThenPlay
        );
        assert_eq!(play_press(false, Some(false), false), PlayPress::Nothing);
        assert_eq!(play_press(false, None, false), PlayPress::Nothing);
        assert_eq!(play_press(false, Some(true), true), PlayPress::Nothing);
        assert_eq!(play_press(true, Some(true), true), PlayPress::Nothing);
    }

    /// A server's Join (and every other shortcut) waits like Play: not while the version's
    /// first download and preparation run (#horizon-bugs, 0.2.23: a white window opened).
    #[test]
    fn shortcuts_start_only_when_play_would() {
        use super::starts_now;
        assert!(starts_now(true, false, false));
        assert!(
            !starts_now(false, true, false),
            "first-time setup under way"
        );
        assert!(!starts_now(true, true, false), "prepared again under way");
        assert!(!starts_now(false, false, false), "not prepared");
        assert!(!starts_now(true, false, true), "already running");
    }

    #[test]
    fn a_completed_sign_in_is_handled_once_across_ui_polls() {
        use super::{sign_in_change, SignInState};
        let mut shown = None;
        let completed = SignInState::Done(Ok(horizon_launcher::account::Account {
            uuid: "11111111111111111111111111111111".into(),
            name: "TestAccount".into(),
            skin: None,
            cape: None,
        }));
        assert!(sign_in_change(&mut shown, SignInState::Starting).is_some());
        assert!(sign_in_change(&mut shown, SignInState::Starting).is_none());
        let mut account_reloads = 0;
        for _ in 0..100 {
            if matches!(
                sign_in_change(&mut shown, completed.clone()),
                Some(SignInState::Done(Ok(_)))
            ) {
                account_reloads += 1;
            }
        }
        assert_eq!(
            account_reloads, 1,
            "completed sign-in must not restart account checks every frame"
        );
        let failed = SignInState::Done(Err("Sign-in cancelled".into()));
        assert!(sign_in_change(&mut shown, failed.clone()).is_some());
        assert!(sign_in_change(&mut shown, failed).is_none());
    }

    #[test]
    fn the_app_menu_has_quit() {
        let menus = super::menus();
        assert_eq!(menus[0].name, horizon_launcher::brand::launcher_name());
        assert!(matches!(
            &menus[0].items[0],
            gpui::MenuItem::Action { name, .. }
                if name == horizon_launcher::brand::text("Quit Horizon Launcher").as_ref()
        ));
    }

    #[test]
    fn the_offline_name_is_read_from_its_key() {
        use super::player_name;
        assert_eq!(
            player_name("name:Player63010\n").as_deref(),
            Some("Player63010")
        );
        assert_eq!(player_name("name:Steve\r\n").as_deref(), Some("Steve"));
        assert_eq!(player_name("Player63010\n"), None);
        assert_eq!(player_name(""), None);
    }

    #[test]
    fn counts_get_thousands_separators() {
        let shown: Vec<String> = [0, 7, 999, 1000, 12_300, 1_234_500]
            .into_iter()
            .map(super::thousands)
            .collect();
        assert_eq!(shown, ["0", "7", "999", "1,000", "12,300", "1,234,500"]);
    }
}
