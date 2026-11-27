//! Sunrise launcher: installs Sunrise builds, prepares them from the player's own Minecraft,
//! and starts the client or a dedicated server.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

#[cfg(windows)]
use sunrise_launcher_scenery::coast;
use sunrise_launcher_scenery::{art, clock, glass};
mod account_chip;
mod app;
mod launcher_messages;
mod launcher_play;
mod updates;
pub(crate) use widgets::*;
mod backdrop;
mod blocks;
mod build;
mod build_mode;
mod cat;
mod cli;
mod cli_tools;
mod code_box;
mod collection;
mod command_search;
mod console;
mod console_page;
mod cull;
mod datapacks_page;
mod developer_play;
mod eta;
mod frame;
mod game_tools;
mod github_mods_page;
mod glass_ui;
mod home;
mod home_mods;
mod instance_icons;
mod instances_page;
mod internet;
mod jar_launch;
mod join;
mod language_page;
mod launch_page;
mod launch_state;
mod line_edit;
mod location_page;
mod markdown;
mod mcp;
mod microsoft;
mod mod_icons;
mod mods;
mod mods_page;
mod mods_project;
mod motion;
mod multiplayer_page;
mod native_log_page;
mod owner_page;
mod packs_page;
mod paint;
mod perf;
mod picture_cache;
mod plate;
mod profile_builds;
mod profile_delete;
mod profile_dialog;
mod profile_heads;
mod rail;
mod screenshots_page;
mod scrollbar;
mod selectable;
mod servers_page;
mod settings;
#[cfg(windows)]
mod setup_page;
mod sheets;
mod shell;
mod skins_page;
mod slider;
mod status_strip;
mod sticky;
mod storage_builds;
mod storage_report;
mod switch_page;
mod theme;
mod themes_page;
mod titlebar;
mod toast;
mod tokens;
mod wake;
mod while_playing;
mod widgets;
mod window_root;

use gpui::{
    actions, div, prelude::*, px, relative, size, AnimationExt, AnyElement, App, Bounds,
    ClickEvent, Context, Div, ElementId, FocusHandle, FontWeight, Hsla, KeyBinding, KeyDownEvent,
    Menu, MenuItem, PathPromptOptions, SharedString, Stateful, Subscription, Window, WindowBounds,
    WindowOptions,
};
use scrollbar::Scrollbar;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use sunrise_launcher::account::{self, Accounts, SignIn, SignInState};
use sunrise_launcher::backend::{self, Job, Msg, Release, Running, Settings, Updates};
use sunrise_launcher::i18n::{self, tr, trf};
use sunrise_launcher::instances;
use sunrise_launcher::native_mods;
use sunrise_launcher::native_status;
use sunrise_launcher::storage;
use sunrise_launcher::telemetry;
use sunrise_launcher::{build_import, identity};
use theme::Theme;

#[derive(Clone, Copy, PartialEq)]
/// The older page field (`Launcher::page`), which `go` sets and a few page-entry checks read.
/// The window draws `frame::Page` (`ui.page`) instead: navigate with `Launcher::navigate`.
enum Page {
    Play,
    Settings,
}

impl Page {
    /// Home; Settings when the data folder did not open, where the message and the two
    /// recoveries are.
    fn from_env() -> Page {
        page_after_startup(
            Page::Play,
            sunrise_launcher::data_location::startup_error().is_some(),
        )
    }
}

/// The launcher's own update, shared by the top bar and Settings.
enum SelfUpdate {
    Idle,
    Downloading(Release, f32),
    /// A rejected manifest invalidated the download; its worker still owns the stage.
    Discarding,
    /// Downloaded and checked. Only an explicit update request hands it to the installer.
    Ready(Release),
    /// A newer launcher can't go in here: this copy can't replace itself
    /// (`backend::cannot_replace_itself`), or the installer said why (the error). The player
    /// follows the recovery advice for its install location.
    Stuck(Option<String>),
    Failed(Release, String),
}

#[derive(Clone, Copy)]
enum UpdateCheck {
    Startup,
    Scheduled,
    ManualLauncher,
}

/// The last game the launcher started ended badly: what it left behind.
#[derive(Clone)]
struct Crash {
    /// One line on what went wrong: the report's error, else how the process ended.
    summary: String,
}

enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

const LOG_LINES: usize = 2000;

struct Launcher {
    ui: frame::State,
    /// The Mods page's own state, kept by the launcher rather than the frame so the frame
    /// draws the page without depending on the Mods module (`mods/state.rs`).
    mods_state: mods::ModsState,
    theme: Theme,
    page: Page,
    /// The page under the glass Settings dialog: the last one shown before it opened.
    behind: Page,
    /// How many times the page changed, for the page transition.
    switches: usize,
    content_scroll: Scrollbar,
    /// The glass Settings page's own scroll: its sections beside their list.
    settings_pane: gpui::ScrollHandle,
    /// The glass Settings page's search, its focus and whether it has it.
    settings_query: line_edit::LineEdit,
    /// Narrow Settings: the category list is dropped open from the switcher row; it starts folded and folds again on a pick.
    settings_nav_open: bool,
    /// A mod page's side panel, kept in view beside its description.
    mods_sticky: sticky::Sticky,
    /// Settings' categories, kept in view beside the section.
    settings_sticky: sticky::Sticky,
    updates: Updates,
    checked: Option<Instant>,
    checking_updates: bool,
    launcher_checked_now: Option<Instant>,
    self_update: SelfUpdate,
    restart_for_launcher_update: bool,
    /// An explicit same-build repair must not select or prune a build.
    architecture_reinstall: Option<String>,
    /// The Sunrise version being installed because an instance was made or saved to play it: it
    /// becomes the launcher's version only when the launcher has none.
    instance_install: Option<String>,
    /// One for an instance waiting for the install running now to end.
    install_next: Option<Release>,
    /// The install bar's previous and current fraction, so it moves rather than jumps.
    bar: (f32, f32),
    releases: Load<Vec<Release>>,
    installed: Vec<String>,
    known_nightly: Option<String>,
    installing: Option<(String, f32)>,
    /// Stops the game build download under way: its own, or the build switch's it serves.
    install_stop: Option<native_mods::Cancellation>,
    /// The status strip's list of work under way: open or folded, and the time left it measures.
    jobs: status_strip::Jobs,
    /// The confirmed builds whose removal is in progress.
    removing: Vec<String>,
    storage_builds: storage_builds::View,
    storage_report: Option<storage::Report>,
    storage_pending: bool,
    /// The storage work under way is a Clear, not a count.
    storage_clearing: bool,
    /// A Clear the player still has to confirm: `Some(None)` is Clear all.
    storage_confirm: Option<Option<storage::Category>>,
    storage_error: Option<String>,
    install_error: Option<(String, String)>,
    /// Storage is copying in a local game build.
    importing_build: bool,
    /// Native mods (`native_mods`, on the Mods page): the build's step while it runs, why it
    /// failed, and the warning before a build while it is open, with the mods it names.
    native_step: Option<String>,
    /// Where the build under way is, for the card's stages and bar.
    native_progress: Option<native_mods::Progress>,
    native_cancellation: Option<native_mods::Cancellation>,
    /// The first build's one-time download (the compiler), in bytes; `None` once it is there.
    first_build: Option<u64>,
    /// Play is waiting for a build, with the profile and account that pressed it.
    play_after_build: Option<launcher_play::PlayIntent>,
    native_error: Option<String>,
    /// The mods that stopped the last build (`native_mods::Failed::Mods`): their rows say Failed,
    /// the others keep their own state.
    native_failed_mods: Vec<String>,
    /// Mods the last build would have switched off that worlds were saved with: the choice
    /// between staying and backing those worlds up first, shown with `native_error`.
    native_worlds: Option<native_mods::AtStake>,
    /// The instance (id) the build under way, or the last one's error, is for.
    native_for: Option<String>,
    /// The native mods the build under way (or the last one) was given (`native_build.rs`).
    native_built_mods: Vec<String>,
    /// What each installed native build was made from, by version. A build's version names its
    /// contents, so an entry never goes stale.
    native_builds:
        std::cell::RefCell<std::collections::HashMap<String, Option<native_status::Build>>>,
    /// This attempt's build log (`native_log_page.rs`).
    native_log: native_log_page::NativeLog,
    /// What the Settings › Storage row says about the last move or choice.
    location_error: Option<String>,
    /// The data folder could not be opened at start: the only state that keeps Play and the
    /// edits that write there off. A move that failed or a folder that was refused leaves the
    /// current folder working, so those only say so in `location_error`.
    location_unavailable: bool,
    location_note: Option<String>,
    /// The empty folder picked for a data move, until it is confirmed or cancelled.
    location_target: Option<std::path::PathBuf>,
    settings: Settings,
    /// The last failed settings write, kept across page changes until a save succeeds.
    settings_save_error: Option<String>,
    /// The Discord and website links: built in until links.json on main has been read.
    links: backend::Links,
    preparing: bool,
    prepare_log: VecDeque<String>,
    /// Lines received from the game and the preparer since the launcher started: it only grows,
    /// so it moves when a line arrives even while `prepare_log` is full and its length stays.
    log_revision: u64,
    prepare_error: Option<String>,
    client: Option<Running>,
    /// Game exits still recording play time or running a profile's post-exit command.
    finishing_game: usize,
    client_error: Option<String>,
    play_admission: Option<launcher_play::PlayIntent>,
    play_retry: Option<launcher_play::PlayIntent>,
    play_install_recovery: Option<(std::path::PathBuf, String)>,
    /// The latest Play's registry warnings, shown until dismissed or a new Play is accepted.
    play_registry_notices: Vec<String>,
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
    /// The toast last drawn (`toast`), by its number.
    toast_seen: Option<(u64, bool)>,
    root_focus: FocusHandle,
    /// Microsoft accounts (accounts.json), the sign-in under way and what it last showed.
    accounts: Accounts,
    auth_status: HashMap<String, account::session::Status>,
    auth_progress: HashMap<String, account::session::Phase>,
    profile_heads: profile_heads::Heads,
    /// The Skins tab (`skins_page`).
    skins: skins_page::SkinsPages,
    /// The account chip's menu (`account_chip`).
    account_menu: account_chip::Menu,
    /// The launcher's search (`command_search`).
    search: command_search::Search,
    /// When page switches last checked every account's sign-in.
    auth_checked: Option<Instant>,
    sign_in_screen: launcher_play::SignInScreen,
    sign_in_copies: owner_page::SignInCopies,
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
    /// The game is waiting for its session (a token renewal), before it starts.
    starting_client: Option<String>,
    /// Which step the accepted Play is at, for the label while the game is starting.
    play_step: Option<backend::PlayStep>,
    /// How far the accepted Play's game build download is, while it is at that step.
    play_download: Option<f32>,
    /// The accepted Play the session worker is answering.
    starting_play: Option<launcher_play::PlayIntent>,
    /// Play was pressed on a version not prepared yet: it starts once this preparation ends well.
    play_when_prepared: Option<(String, launcher_play::PlayIntent)>,
    /// Join a friend: the code typed on the Play page.
    friend_code: line_edit::LineEdit,
    friend_focused: bool,
    friend_focus: FocusHandle,
    window_width: f32,
    window_height: f32,
    /// The window's longer side in points, and the screen's pixels per point: the size pictures
    /// are made at (`picture_cache`).
    window_long_side: f32,
    scale_factor: f32,
    /// The instance the running game plays and when it started: its play time grows on exit.
    client_run: Option<(instances::Instance, Instant)>,
    /// Settings' Game arguments field: the line, whether it has focus, and its focus handle.
    args_input: line_edit::LineEdit,
    args_focused: bool,
    args_focus: FocusHandle,
    /// Settings' Proxy field, the same way (`proxy`).
    proxy_input: line_edit::LineEdit,
    proxy_focused: bool,
    proxy_focus: FocusHandle,
    /// Settings › Appearance › Corner rounding, typed in px.
    corners_input: line_edit::LineEdit,
    corners_focused: bool,
    corners_focus: FocusHandle,
    corners_slider: slider::Slider,
    /// Opt-in data collection: the player's answer and Settings' section (`collection`).
    collection: collection::Collection,
    /// How many play Sunrise now, while the server answers ("N playing now" on the Play page).
    playing: Option<u64>,
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
    play_pressed: Option<Instant>,
    /// `sunrise-launcher benchmark` started this window to measure it.
    bench: Option<perf::Bench>,
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
    screenshots: screenshots_page::Screenshots,
    /// Pictures from disk made at their drawn size (`picture_cache`).
    pictures: picture_cache::Pictures,
    game_packs: packs_page::Packs,
    world_packs: datapacks_page::WorldPacks,
    launch: Option<launch_page::LaunchPanel>,
    /// What the window did when the game started (`while_playing.rs`).
    aside: while_playing::Aside,
    /// The player's servers, for the sidebar until the Servers page has read them.
    side_servers: Vec<sunrise_launcher::servers::Entry>,
    /// The window's scale, as of its last frame, for art drawn outside `render`'s reach.
    scale: f32,
}

actions!(sunrise_launcher, [Quit, CloseWindow]);

/// What closing the last window does: stop the servers this launcher started, send the telemetry,
/// exit. A game that is running keeps running. Cmd+Q goes through here too.
fn shut_down(cx: &mut App) {
    if profile_delete::defer_shutdown(cx) {
        return;
    }
    servers_page::stop_on_quit();
    telemetry::send_before_exit();
    sunrise_launcher::update_handoff::forget();
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

/// A data folder that did not open starts on Settings, where the message, Retry and
/// "Use the default folder" are; an explicit page still wins.
fn page_after_startup(page: Page, data_folder_failed: bool) -> Page {
    if page == Page::Play && data_folder_failed {
        Page::Settings
    } else {
        page
    }
}

/// The menu bar on macOS, where "Quit" belongs to the app menu.
fn menus() -> Vec<Menu> {
    vec![
        Menu {
            name: identity::DISPLAY_NAME.into(),
            items: vec![MenuItem::action(tr(identity::QUIT_LABEL), Quit)],
            disabled: false,
        },
        Menu {
            name: "Window".into(),
            items: vec![MenuItem::action("Close Window", CloseWindow)],
            disabled: false,
        },
    ]
}

mod startup;

fn main() {
    let mut args = std::env::args_os().skip(1);
    if let Some(mode) = args.next() {
        #[cfg(target_os = "linux")]
        if mode == sunrise_launcher::process::compiler_worker::ARG {
            std::process::exit(sunrise_launcher::process::compiler_worker::run(args));
        }
        if let Some(status) = sunrise_launcher::process::relay::private(&mode, args) {
            std::process::exit(status);
        }
    }
    startup::run();
}

#[cfg(test)]
mod tests {
    #[test]
    fn prepare_steps_run_in_their_order() {
        use sunrise_launcher::backend::PrepareStep::*;
        assert_eq!(
            sunrise_launcher::backend::PrepareStep::ALL,
            [Files, Java, Sounds, Prepare]
        );
        for s in sunrise_launcher::backend::PrepareStep::ALL {
            assert_eq!(sunrise_launcher::backend::PrepareStep::ALL[s as usize], s);
        }
    }

    #[test]
    fn an_unopenable_data_folder_starts_on_settings_instead_of_exiting() {
        use super::{page_after_startup, Page};
        assert!(matches!(
            page_after_startup(Page::Play, true),
            Page::Settings
        ));
        assert!(matches!(page_after_startup(Page::Play, false), Page::Play));
        assert!(matches!(
            page_after_startup(Page::Settings, false),
            Page::Settings
        ));
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
    /// first download and preparation run (#sunrise-bugs, 0.2.23: a white window opened).
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
        let completed = SignInState::Done(Ok(sunrise_launcher::account::Account {
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
        assert_eq!(menus[0].name, sunrise_launcher::identity::DISPLAY_NAME);
        assert!(matches!(
            &menus[0].items[0],
            gpui::MenuItem::Action { name, .. } if name == sunrise_launcher::identity::QUIT_LABEL
        ));
    }
}

mod native_build;
mod worlds_page;
