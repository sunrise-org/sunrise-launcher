//! The launcher's state set up (`Launcher::new`), its settings saved, page changes, the
//! launcher's own updates and the release list.
use super::*;

impl Launcher {
    pub(crate) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        crate::eta::preload_timings();
        // A press on something that then leaves the page (a row that opens its page, a tab)
        // would leave the keyboard on nothing, and Tab dead until the next click.
        cx.on_focus_lost(window, |this, window, cx| {
            window.focus(&this.root_focus, cx)
        })
        .detach();
        let (tx, rx) = wake::channel();
        // A build that reaches native mods nobody reviewed waits on the window's warning.
        let asks = tx.clone();
        native_mods::ask_unreviewed_with(move |found| {
            let (reply, answer) = std::sync::mpsc::channel();
            asks.send(Msg::ConfirmUnreviewed(found.to_vec(), reply))
                .is_ok()
                && answer.recv().unwrap_or(false)
        });
        let appearance = cx.observe_window_appearance(window, |this, window, cx| {
            this.theme = this.settings_theme(window);
            cx.notify();
        });
        let window_handle = window.window_handle();
        #[cfg(feature = "frame-stats")]
        perf::frame_overlay(window);
        // The pump: on its tick, or at once when a worker says something (`wake.rs`).
        cx.spawn(async move |this, cx| loop {
            let tick = cx.background_executor().timer(wake::TICK);
            futures_util::future::select(std::pin::pin!(tick), std::pin::pin!(wake::woken())).await;
            if this
                .update(cx, |this, cx| this.drain(window_handle, cx))
                .is_err()
            {
                break;
            }
            cx.background_executor().timer(wake::SETTLE).await;
        })
        .detach();
        let mut settings = Settings::load();
        i18n::set(settings.language());
        if settings.minecraft_dir.is_none() {
            settings.minecraft_dir = backend::default_minecraft_dir();
        }
        // Release builds on Linux put their entry back in the applications menu when there is
        // none or it starts a program that is gone (a moved or deleted binary), never over one
        // that works; `SUNRISE_LAUNCHER_DESKTOP_ENTRY=1` does it in a development build.
        if cfg!(target_os = "linux")
            && settings.desktop_entry()
            && (!cfg!(debug_assertions)
                || std::env::var_os("SUNRISE_LAUNCHER_DESKTOP_ENTRY").is_some())
        {
            std::thread::spawn(|| {
                let _ = sunrise_launcher::desktop::repair(backend::exe_path());
            });
        }
        let (installed, known_nightly) = backend::play_installations(settings.selected.as_deref());
        if settings
            .selected
            .as_ref()
            .is_none_or(|s| !installed.contains(s))
        {
            settings.selected = known_nightly.clone().or_else(|| {
                installed
                    .iter()
                    .find(|tag| sunrise_launcher::nightly::date_of(tag).is_none())
                    .cloned()
            });
        }
        // Pin the window's chrome (macOS title bar) to the choice before the theme reads
        // `window.appearance()`, so both draw the same look.
        titlebar::apply_appearance(window, settings.appearance);
        let (location_error, location_unavailable) = location_page::startup_location();
        let mut this = Self {
            ui: Default::default(),
            mods_state: Default::default(),
            theme: crate::theme::from_settings(&settings, window.appearance()),
            page: Page::from_env(),
            behind: Page::Play,
            switches: 0,
            content_scroll: Scrollbar::default(),
            settings_pane: gpui::ScrollHandle::new(),
            settings_query: line_edit::LineEdit::new(String::new(), 80, 40),
            settings_nav_open: false,
            mods_sticky: sticky::Sticky::default(),
            settings_sticky: sticky::Sticky::default(),
            updates: Updates::default(),
            checked: None,
            checking_updates: false,
            launcher_checked_now: None,
            // The installer the last Update started could not put the new launcher in place.
            self_update: sunrise_launcher::update_handoff::take_failure()
                .map_or(SelfUpdate::Idle, |e| SelfUpdate::Stuck(Some(e))),
            restart_for_launcher_update: false,
            architecture_reinstall: None,
            instance_install: None,
            install_next: None,
            bar: (0., 0.),
            releases: Load::Loading,
            installed,
            known_nightly,
            installing: None,
            install_stop: None,
            prepare_stop: None,
            update_stop: None,
            jobs: Default::default(),
            removing: Vec::new(),
            storage_builds: Default::default(),
            storage_report: None,
            storage_pending: false,
            storage_clearing: false,
            storage_confirm: None,
            storage_error: None,
            install_error: None,
            importing_build: false,
            native_step: None,
            native_progress: None,
            native_cancellation: None,
            first_build: None,
            play_after_build: None,
            native_error: None,
            native_failed_mods: Vec::new(),
            native_worlds: None,
            native_for: None,
            native_built_mods: Vec::new(),
            native_builds: Default::default(),
            native_log: Default::default(),
            // The first draw takes the launcher theme from Settings.
            location_error,
            location_unavailable,
            location_note: location_page::startup_note(),
            location_target: None,
            args_input: line_edit::LineEdit::new(settings.game_args.clone(), 1024, 38),
            args_focused: false,
            args_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            proxy_input: line_edit::LineEdit::new(
                settings.proxy.clone().unwrap_or_default(),
                256,
                44,
            ),
            proxy_focused: false,
            corners_input: line_edit::LineEdit::new(settings.corner_radius().to_string(), 2, 3),
            corners_focused: false,
            corners_slider: crate::slider::Slider::default(),
            corners_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            proxy_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            settings,
            settings_save_error: None,
            links: backend::Links::default(),
            preparing: false,
            prepare_log: VecDeque::new(),
            log_revision: 0,
            prepare_error: None,
            client: None,
            finishing_game: 0,
            client_error: None,
            play_admission: None,
            play_retry: None,
            play_install_recovery: None,
            play_registry_notices: Vec::new(),
            client_started: None,
            client_home: None,
            crash: None,
            servers: servers_page::Servers::new(),
            internet: internet::Internet::new(),
            root_focus: cx.focus_handle(),
            toast_seen: None,
            accounts: Accounts::load(),
            auth_status: HashMap::new(),
            auth_progress: HashMap::new(),
            profile_heads: profile_heads::Heads::default(),
            skins: skins_page::SkinsPages::default(),
            account_menu: account_chip::Menu::default(),
            search: command_search::Search::default(),
            auth_checked: None,
            sign_in_screen: launcher_play::SignInScreen::default(),
            auth_busy: HashMap::new(),
            auth_next: 0,
            sign_in: None,
            sign_in_state: None,
            keychain: None,
            owner: None,
            starting_client: None,
            play_step: None,
            play_download: None,
            starting_play: None,
            play_when_prepared: None,
            friend_code: line_edit::LineEdit::new(String::new(), 256, 32),
            friend_focused: false,
            window_width: 0.,
            window_height: 0.,
            window_long_side: 0.,
            scale_factor: 1.,
            friend_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            client_run: None,
            collection: collection::Collection::load(),
            playing: None,
            tx,
            rx,
            _appearance: appearance,
            // Coming back to the window, the profiles are read again: `sunrise-launcher
            // instance rename` and the other commands change `instances.json` behind it.
            _activation: cx.observe_window_activation(window, |this, window, cx| {
                if window.is_window_active() {
                    this.refresh_instances_list();
                }
                cx.notify()
            }),
            perf: perf::Perf::new("session"),
            install_started: None,
            prepare_started: None,
            download_seen: None,
            play_pressed: None,
            bench: perf::Bench::from_env(),
            starting_pack: None,
            mods: mods_page::Mods::new(),
            github: github_mods_page::Github::new(),
            instances: instances_page::Instances::default(),
            worlds: worlds_page::Worlds::default(),
            multiplayer: multiplayer_page::Multiplayer::default(),
            screenshots: screenshots_page::Screenshots::default(),
            pictures: Default::default(),
            game_packs: packs_page::Packs::default(),
            world_packs: datapacks_page::WorldPacks::default(),
            launch: None,
            aside: while_playing::Aside::new(window.window_handle()),
            side_servers: sunrise_launcher::servers::list(),
            scale: window.scale_factor(),
        };
        // A data folder that did not open, or a move that failed or finished, is dealt with in
        // Settings › Storage, where its message and the recoveries are.
        if let Some(section) =
            settings::startup_section(this.location_error.is_some(), this.location_note.is_some())
        {
            this.ui.page = frame::Page::Settings;
            this.ui.settings.section = section;
        }
        if !this.location_unavailable {
            let interrupted = instances::interrupted_deletions();
            if !interrupted.is_empty() {
                this.ui.sheets.profile_action =
                    Some(profile_delete::Action::Interrupted(interrupted));
            }
        }
        if let Some(error) = sunrise_launcher::carry_over::startup_error() {
            this.perf.keep_local_log(false);
            this.open_notice(
                trf(
                    "Could not copy the earlier launcher's data: {error}\n\nFix the earlier launcher's data folder or reconnect its drive, then restart Sunrise to try again.",
                    &[("error", &error)],
                ),
                None,
            );
        }
        this.reload_instances();
        // SUNRISE_LAUNCHER_PAGE opens a page at start (tools and screenshots of a page).
        if let Ok(asked) = std::env::var("SUNRISE_LAUNCHER_PAGE") {
            this.open_named_page(&asked);
        }
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
        this.check_updates(UpdateCheck::Startup);
        this.fetch_links();
        telemetry::send_in_background();
        telemetry::watch_playing(this.tx.clone());
        this
    }

    /// Keeps the current choices in memory if writing fails; every page offers the same retry.
    pub(crate) fn save_settings(&mut self) -> bool {
        self.settings_save_error = self.settings.save().err();
        self.settings_save_error.is_none()
    }

    /// The banner at the top of the window while the settings could not be written: the short
    /// reason in its line, the whole error (with `~` for the home folder) behind Details.
    pub(crate) fn settings_save_notice(&self, cx: &mut Context<Self>) -> Option<Div> {
        use crate::widgets::feedback::{
            banner_button, banner_link, os_reason, tilde_in, Banner, Tone,
        };
        let error = self.settings_save_error.as_ref()?;
        let mut strip = Banner::new(Tone::Danger, tr("Settings weren't saved"));
        if let Some(reason) = os_reason(error) {
            strip = strip.detail(reason);
        }
        let full = tilde_in(error);
        let strip = strip
            .action(
                banner_link("settings-save-details", tr("Details")).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        let lead = tr("Your changes are still in this window. Retry to save them.");
                        this.open_notice(format!("{lead}\n{full}"), None);
                        cx.notify();
                    },
                )),
            )
            .action(
                banner_button("settings-save-retry", tr("Retry")).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        this.save_settings();
                        cx.notify();
                    },
                )),
            );
        Some(
            div()
                .flex_none()
                .px(px(theme::PAGE_GUTTER))
                .py(px(12.))
                .child(strip.build()),
        )
    }

    pub(crate) fn go(&mut self, page: Page) {
        // The CLI or another window may have changed the accounts: read them again (a small
        // file, once a switch).
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
        // Settings opens on its first page, its list folded and its search empty.
        if page == Page::Settings && self.page != Page::Settings {
            self.settings_nav_open = false;
            self.settings_pane.scroll_to_item(0);
            self.settings_query.set(String::new());
        }
        // The glass look's Settings is a dialog over the page: opening or closing it keeps the
        // page under it as it was (its scroll, no fade).
        let dialog = self.theme.glass
            && (page == Page::Settings || (self.page == Page::Settings && page == self.behind));
        if page != Page::Settings {
            self.behind = page;
        }
        if page != self.page && dialog {
            self.page = page;
        } else if page != self.page {
            self.page = page;
            self.switches += 1;
            self.content_scroll = Scrollbar::default();
            self.mods_sticky.reset();
            self.settings_sticky.reset();
        }
    }

    pub(crate) fn check_updates(&mut self, check: UpdateCheck) {
        if self.checking_updates {
            return;
        }
        self.checking_updates = true;
        if !matches!(check, UpdateCheck::ManualLauncher) {
            self.checked = Some(Instant::now());
        }
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let message = match check {
                UpdateCheck::Startup => {
                    let mut final_releases = None;
                    let updates = backend::releases_and_updates(|releases| {
                        if let Ok(list) = &releases {
                            final_releases = Some(list.clone());
                        }
                        let _ = tx.send(Msg::Releases(releases));
                    });
                    if let Some(list) = &final_releases {
                        if let Err(error) = backend::note_play_catalogue(list) {
                            eprintln!("Could not remember the current installed Nightly: {error}");
                        }
                    }
                    Msg::Updates(Box::new(updates))
                }
                UpdateCheck::Scheduled => Msg::Updates(Box::new(backend::check_updates())),
                UpdateCheck::ManualLauncher => {
                    Msg::LauncherChecked(Box::new(backend::refresh_launcher_update()))
                }
            };
            let _ = tx.send(message);
        });
    }

    /// Recovery text for a launcher update that can't go in: why, or where the copy has to be.
    pub(crate) fn launcher_update_help(&self, error: Option<&str>) -> String {
        if let Some(error) = error {
            return trf("the update failed: {error}", &[("error", error)]);
        }
        if backend::package_managed_launcher() {
            return tr("Update this launcher with your package manager.").to_string();
        }
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| std::path::PathBuf::from(tr("Downloads")));
        trf(
            "Close this launcher, move it to {folder}, and open it there to enable updates.",
            &[("folder", &folder.display().to_string())],
        )
    }

    pub(crate) fn launcher_download_available(&self) -> bool {
        !backend::package_managed_launcher()
    }

    /// What installed native build `version` was made from (`native_status::Build`), read once.
    pub(crate) fn native_build_of(&self, version: &str) -> Option<native_status::Build> {
        if !native_mods::is_native(version) {
            return None;
        }
        self.native_builds
            .borrow_mut()
            .entry(version.to_owned())
            .or_insert_with(|| native_status::Build::read(version))
            .clone()
    }

    pub(crate) fn fetch_links(&self) {
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(Msg::Links(backend::links()));
        });
    }
}
